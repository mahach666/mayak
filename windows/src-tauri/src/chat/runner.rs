//! Starting a CLI and the plumbing around its pipes: the process group that goes with Mayak, the
//! bounded line reader, the stderr tail and the short probes (`claude auth status`). std threads
//! and blocking reads only: nothing here polls, so an idle chat costs no CPU.

use std::io::{self, BufRead, Read};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use super::agent::{ChatError, Probe};

/// The longest line read from a CLI. Its init line is the biggest (tens of KB); a line past this is
/// skipped, not buffered, so a runaway process cannot take the app's memory.
pub const MAX_LINE: usize = 8 << 20;
/// What is kept of a process's stderr: the end of it, for the message of a failed run.
const STDERR_KEEP: usize = 4_096;

// ── The process group ──────────────────────────────────────────────────────────

/// Every process a chat run starts (the CLI and what it spawns: shells, MCP servers) belongs to one
/// group that ends together: `kill` ends it now, and on Windows the job object also ends it when
/// Mayak exits or crashes, so no agent outlives the app that started it.
pub struct ProcGroup(imp::Group);

impl ProcGroup {
    pub fn new() -> io::Result<Self> {
        imp::Group::new().map(Self)
    }

    pub fn add(&self, child: &Child) -> io::Result<()> {
        self.0.add(child)
    }

    pub fn kill(&self) {
        self.0.kill();
    }
}

#[cfg(windows)]
mod imp {
    use std::io;
    use std::mem::size_of;
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
        TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    pub struct Group {
        job: HANDLE,
        /// The process added, and whether the job really took it: when it did not, `kill` falls
        /// back to ending the process tree by its id.
        pid: AtomicU32,
        assigned: AtomicBool,
    }

    // A job handle is a plain kernel handle: every call on it is thread safe.
    unsafe impl Send for Group {}
    unsafe impl Sync for Group {}

    impl Group {
        pub fn new() -> io::Result<Self> {
            // SAFETY: the calls below only read the structures they are given, which outlive them.
            unsafe {
                let job = CreateJobObjectW(None, PCWSTR::null()).map_err(io::Error::from)?;
                let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let set = SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
                if let Err(err) = set {
                    let _ = CloseHandle(job);
                    return Err(io::Error::from(err));
                }
                Ok(Group { job, pid: AtomicU32::new(0), assigned: AtomicBool::new(false) })
            }
        }

        pub fn add(&self, child: &Child) -> io::Result<()> {
            self.pid.store(child.id(), Ordering::SeqCst);
            // SAFETY: both handles are valid for the duration of the call.
            let added = unsafe { AssignProcessToJobObject(self.job, HANDLE(child.as_raw_handle())) };
            added.map_err(io::Error::from)?;
            self.assigned.store(true, Ordering::SeqCst);
            Ok(())
        }

        pub fn kill(&self) {
            if self.assigned.load(Ordering::SeqCst) {
                // SAFETY: the job handle is owned by `self`.
                unsafe {
                    let _ = TerminateJobObject(self.job, 1);
                }
                return;
            }
            // The process never joined the job: end it and what it started by its id.
            let pid = self.pid.load(Ordering::SeqCst);
            if pid != 0 {
                let mut cmd = std::process::Command::new("taskkill");
                cmd.args(["/PID", &pid.to_string(), "/T", "/F"]);
                crate::platform::no_console(&mut cmd);
                let _ = cmd.output();
            }
        }
    }

    impl Drop for Group {
        fn drop(&mut self) {
            // Closing the last handle of a KILL_ON_JOB_CLOSE job ends whatever is still in it.
            // SAFETY: the handle is ours and not used again.
            unsafe {
                let _ = CloseHandle(self.job);
            }
        }
    }
}

#[cfg(unix)]
mod imp {
    use std::io;
    use std::process::Child;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// The process group of the CLI (`process_group(0)` made it its own leader).
    pub struct Group(AtomicU32);

    impl Group {
        pub fn new() -> io::Result<Self> {
            Ok(Group(AtomicU32::new(0)))
        }

        pub fn add(&self, child: &Child) -> io::Result<()> {
            self.0.store(child.id(), Ordering::SeqCst);
            Ok(())
        }

        pub fn kill(&self) {
            let pgid = self.0.load(Ordering::SeqCst);
            if pgid > 1 {
                // SAFETY: a plain signal to the group this run created. Called only while the run
                // is alive, so the id has not been reused.
                unsafe {
                    libc::kill(-(pgid as i32), libc::SIGKILL);
                }
            }
        }
    }
}

// ── Starting a process ─────────────────────────────────────────────────────────

pub struct Started {
    pub child: Child,
    pub group: Arc<ProcGroup>,
}

/// Runs `bin` (an absolute path found on PATH) with `args` in `cwd`, all three pipes taken. No shell
/// is involved, and on Windows the current folder is not searched for the programs a `.cmd` shim runs.
pub fn spawn(bin: &Path, args: &[String], env: &[(String, String)], cwd: &Path) -> Result<Started, ChatError> {
    let mut cmd = Command::new(bin);
    cmd.args(args).current_dir(cwd).envs(env.iter().map(|(k, v)| (k, v))).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    prepare(&mut cmd);
    let group = ProcGroup::new().map_err(|e| ChatError::new("spawnFailed", format!("Could not start the CLI ({:?}).", e.kind())))?;
    let child = cmd.spawn().map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => ChatError::new("cliMissing", "The CLI was not found."),
        // std refuses an argument it cannot pass to a .cmd or .bat file safely (a line break, say).
        io::ErrorKind::InvalidInput => ChatError::new("promptNotPassable", "That message cannot be passed to this CLI's command line."),
        kind => ChatError::new("spawnFailed", format!("Could not start the CLI ({kind:?}).")),
    })?;
    if let Err(err) = group.add(&child) {
        crate::log::line(format!("chat: process group: {:?}", err.kind()));
    }
    Ok(Started { child, group: Arc::new(group) })
}

#[cfg(windows)]
fn prepare(cmd: &mut Command) {
    crate::platform::no_console(cmd);
    cmd.env("NoDefaultCurrentDirectoryInExePath", "1");
}

#[cfg(unix)]
fn prepare(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    cmd.process_group(0);
}

// ── Reading ────────────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq, Eq)]
pub enum Line {
    /// A complete line, without its newline (and without a `\r` before it).
    Text(String),
    /// A line longer than `MAX_LINE`: skipped to its end.
    TooLong,
    Eof,
}

/// The next line of `reader`, at most `max` bytes of it kept. Invalid UTF-8 is replaced, never an error.
pub fn read_line<R: BufRead>(reader: &mut R, max: usize) -> io::Result<Line> {
    let mut buf: Vec<u8> = Vec::new();
    let mut too_long = false;
    loop {
        let available = match reader.fill_buf() {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if available.is_empty() {
            // End of input: a last line without a newline still counts.
            if too_long {
                return Ok(Line::TooLong);
            }
            return Ok(if buf.is_empty() { Line::Eof } else { Line::Text(finish(buf)) });
        }
        let (chunk, found) = match available.iter().position(|b| *b == b'\n') {
            Some(i) => (&available[..i], Some(i + 1)),
            None => (available, None),
        };
        if !too_long {
            if buf.len() + chunk.len() > max {
                too_long = true;
                buf = Vec::new();
            } else {
                buf.extend_from_slice(chunk);
            }
        }
        let used = found.unwrap_or(chunk.len());
        reader.consume(used);
        if found.is_some() {
            return Ok(if too_long { Line::TooLong } else { Line::Text(finish(buf)) });
        }
    }
}

fn finish(mut buf: Vec<u8>) -> String {
    if buf.last() == Some(&b'\r') {
        buf.pop();
    }
    String::from_utf8_lossy(&buf).into_owned()
}

/// The end of a process's output, kept while it is drained. A CLI that fills an unread pipe
/// blocks, so a pipe is always read to the end, whatever is done with it.
#[derive(Clone)]
pub struct Tail {
    buf: Arc<Mutex<Vec<u8>>>,
    limit: usize,
}

impl Default for Tail {
    fn default() -> Self {
        Self::with_limit(STDERR_KEEP)
    }
}

impl Tail {
    pub fn with_limit(limit: usize) -> Self {
        Self { buf: Arc::new(Mutex::new(Vec::new())), limit }
    }

    pub fn push(&self, bytes: &[u8]) {
        let mut buf = self.buf.lock().unwrap_or_else(PoisonError::into_inner);
        buf.extend_from_slice(bytes);
        if buf.len() > self.limit {
            let cut = buf.len() - self.limit;
            buf.drain(..cut);
        }
    }

    /// The last non-empty lines, as text, at most `lines` of them.
    pub fn text(&self, lines: usize) -> String {
        let buf = self.buf.lock().unwrap_or_else(PoisonError::into_inner);
        let text = String::from_utf8_lossy(&buf);
        let mut kept: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
        if kept.len() > lines {
            kept.drain(..kept.len() - lines);
        }
        kept.join("
")
    }

    /// Everything kept.
    pub fn all(&self) -> String {
        String::from_utf8_lossy(&self.buf.lock().unwrap_or_else(PoisonError::into_inner)).into_owned()
    }
}

/// Reads `reader` to its end into `tail`.
pub fn drain_into<R: Read>(mut reader: R, tail: &Tail) {
    let mut chunk = [0u8; 2_048];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => return,
            Ok(n) => tail.push(&chunk[..n]),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return,
        }
    }
}

// ── Probes ─────────────────────────────────────────────────────────────────────

/// What a probe printed and how it ended; `exit` is None when it was killed at its timeout.
pub struct ProbeOutput {
    pub exit: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// The most a probe's stdout keeps: `codex debug models` prints the whole catalog (600 KB).
const PROBE_STDOUT_KEEP: usize = 4 << 20;

/// Runs a short probe command and waits for it, at most `probe.timeout`. Blocking: call it off the
/// UI thread. stdin is closed so nothing waits for input.
pub fn run_probe(bin: &Path, probe: &Probe, cwd: &Path) -> Result<ProbeOutput, ChatError> {
    let mut cmd = Command::new(bin);
    cmd.args(&probe.args).current_dir(cwd).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    prepare(&mut cmd);
    let group = ProcGroup::new().map_err(|e| ChatError::new("spawnFailed", format!("Could not start the CLI ({:?}).", e.kind())))?;
    let mut child = cmd.spawn().map_err(|e| ChatError::new("spawnFailed", format!("Could not start the CLI ({:?}).", e.kind())))?;
    let _ = group.add(&child);
    let out = Tail::with_limit(PROBE_STDOUT_KEEP);
    let err = Tail::with_limit(16 << 10);
    let mut threads = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        let t = out.clone();
        threads.push(std::thread::spawn(move || drain_into(stdout, &t)));
    }
    if let Some(stderr) = child.stderr.take() {
        let t = err.clone();
        threads.push(std::thread::spawn(move || drain_into(stderr, &t)));
    }
    let deadline = Instant::now() + probe.timeout;
    let exit = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code(),
            Ok(None) if Instant::now() >= deadline => {
                group.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(_) => break None,
        }
    };
    for t in threads {
        let _ = t.join();
    }
    Ok(ProbeOutput { exit, stdout: out.all(), stderr: err.all() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn lines(input: &[u8], max: usize) -> Vec<Line> {
        let mut reader = Cursor::new(input.to_vec());
        let mut out = Vec::new();
        loop {
            match read_line(&mut reader, max).unwrap() {
                Line::Eof => return out,
                other => out.push(other),
            }
        }
    }

    #[test]
    fn lines_split_on_newlines_and_a_last_line_without_one_counts() {
        assert_eq!(
            lines(b"one\r\ntwo\n\nlast", 100),
            [Line::Text("one".into()), Line::Text("two".into()), Line::Text(String::new()), Line::Text("last".into())]
        );
        assert!(lines(b"", 100).is_empty());
    }

    #[test]
    fn a_line_past_the_limit_is_skipped_and_the_next_one_is_read() {
        assert_eq!(lines(b"short\n0123456789\nafter\n", 8), [Line::Text("short".into()), Line::TooLong, Line::Text("after".into())]);
        assert_eq!(lines(b"0123456789", 8), [Line::TooLong], "also at the end of the input");
        assert_eq!(lines(b"01234567\n", 8), [Line::Text("01234567".into())], "exactly the limit is fine");
    }

    #[test]
    fn a_line_spanning_many_small_reads_is_assembled() {
        struct Dribble<'a>(&'a [u8]);
        impl Read for Dribble<'_> {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                let n = self.0.len().min(1).min(buf.len());
                buf[..n].copy_from_slice(&self.0[..n]);
                self.0 = &self.0[n..];
                Ok(n)
            }
        }
        let mut reader = io::BufReader::with_capacity(2, Dribble(b"{\"a\":1}\nnext\n"));
        assert_eq!(read_line(&mut reader, 100).unwrap(), Line::Text("{\"a\":1}".into()));
        assert_eq!(read_line(&mut reader, 100).unwrap(), Line::Text("next".into()));
        assert_eq!(read_line(&mut reader, 100).unwrap(), Line::Eof);
    }

    #[test]
    fn invalid_utf8_is_replaced_not_an_error() {
        assert_eq!(lines(b"a\xFFb\n", 100), [Line::Text("a\u{FFFD}b".into())]);
    }

    #[test]
    fn the_tail_keeps_the_end_and_gives_the_last_lines() {
        let tail = Tail::default();
        tail.push(b"first\n\n  second  \nthird\n");
        assert_eq!(tail.text(2), "second\nthird");
        assert_eq!(tail.text(10), "first\nsecond\nthird");
        let big = Tail::default();
        big.push(&vec![b'x'; STDERR_KEEP + 500]);
        big.push(b"\nthe end\n");
        assert!(big.text(1) == "the end" && big.all().len() <= STDERR_KEEP);
    }

    #[test]
    fn draining_reads_to_the_end() {
        let tail = Tail::default();
        drain_into(Cursor::new(b"a\nb\n".to_vec()), &tail);
        assert_eq!(tail.text(5), "a\nb");
    }
}
