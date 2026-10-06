// Incremental reader of an append-only JSONL file (spec §5.7): Claude transcripts and Codex
// rollouts. Each read opens, reads and closes the file with full sharing, so the agent that
// writes it is never blocked and can rename or delete it at any time.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom};
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

/// First read of an existing transcript looks at its last 256 KiB only.
pub const WINDOW_BYTES: u64 = 256 * 1024;

/// FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE.
const SHARE_ALL: u32 = 0x1 | 0x2 | 0x4;

pub struct JsonlTail {
    path: PathBuf,
    /// Bytes of the file consumed so far (complete lines + `partial`).
    read_to: u64,
    /// The incomplete last line, held until its '\n' arrives.
    partial: Vec<u8>,
    /// Discard bytes up to and including the next '\n' (we started inside a line).
    skip_first: bool,
}

impl JsonlTail {
    /// Offset 0: the first `read_new` returns the whole file.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into(), read_to: 0, partial: Vec::new(), skip_first: false }
    }

    /// offset = current length. Missing file → offset 0, Ok(0). If the file does not end with
    /// '\n', the rest of that line (it started before us) is discarded by the next read.
    pub fn start_at_eof(&mut self) -> io::Result<u64> {
        self.restart();
        let Some(mut file) = open_shared(&self.path)? else { return Ok(0) };
        let len = file.metadata()?.len();
        if len > 0 {
            file.seek(SeekFrom::Start(len - 1))?;
            let mut last = [0u8; 1];
            file.read_exact(&mut last)?;
            self.skip_first = last[0] != b'\n';
        }
        self.read_to = len;
        Ok(len)
    }

    /// offset = max(0, len - bytes); if > 0 the bytes up to and including the first '\n' are
    /// discarded by the next read (spec §5.5: the window starts inside a line). Missing file →
    /// offset 0.
    pub fn start_with_window(&mut self, bytes: u64) -> io::Result<()> {
        self.restart();
        let Some(file) = open_shared(&self.path)? else { return Ok(()) };
        let len = file.metadata()?.len();
        self.read_to = len.saturating_sub(bytes);
        self.skip_first = self.read_to > 0;
        Ok(())
    }

    /// Complete new lines as (start offset of the line, text without the line ending); blank
    /// lines are skipped and the incomplete last line is kept for the next call. Second value =
    /// `eof()` after this read. A file shorter than what was consumed is read again from 0; a
    /// missing file gives no lines and restarts at 0 when it reappears.
    pub fn read_new(&mut self) -> io::Result<(Vec<(u64, String)>, u64)> {
        let Some(mut file) = open_shared(&self.path)? else {
            self.restart();
            return Ok((Vec::new(), 0));
        };
        let len = file.metadata()?.len();
        if len < self.read_to {
            self.restart();
        }
        if len == self.read_to {
            return Ok((Vec::new(), self.eof()));
        }
        file.seek(SeekFrom::Start(self.read_to))?;
        let mut fresh = Vec::new();
        file.read_to_end(&mut fresh)?;
        drop(file);

        let fresh_at = self.read_to;
        self.read_to += fresh.len() as u64;
        let mut from = 0usize;
        if self.skip_first {
            match fresh.iter().position(|&b| b == b'\n') {
                Some(nl) => {
                    from = nl + 1;
                    self.skip_first = false;
                }
                None => return Ok((Vec::new(), self.eof())),
            }
        }

        let mut data = std::mem::take(&mut self.partial);
        let data_at = fresh_at + from as u64 - data.len() as u64;
        data.extend_from_slice(&fresh[from..]);

        let mut lines = Vec::new();
        let mut start = 0usize;
        while let Some(rel) = data[start..].iter().position(|&b| b == b'\n') {
            let end = start + rel;
            let mut line = &data[start..end];
            if let Some(stripped) = line.strip_suffix(b"\r") {
                line = stripped;
            }
            if !line.iter().all(u8::is_ascii_whitespace) {
                lines.push((data_at + start as u64, String::from_utf8_lossy(line).into_owned()));
            }
            start = end + 1;
        }
        self.partial = data[start..].to_vec();
        Ok((lines, self.eof()))
    }

    /// First byte not yet consumed (as a returned or blank line, or as skipped pre-existing
    /// bytes): the end of the last complete line, or a mid-line offset while the rest of a line
    /// that a start landed inside is being skipped.
    pub fn eof(&self) -> u64 {
        self.read_to - self.partial.len() as u64
    }

    fn restart(&mut self) {
        self.read_to = 0;
        self.partial.clear();
        self.skip_first = false;
    }
}

/// Opens for reading with full sharing; Ok(None) when the file does not exist.
fn open_shared(path: &Path) -> io::Result<Option<File>> {
    match OpenOptions::new().read(true).share_mode(SHARE_ALL).open(path) {
        Ok(file) => Ok(Some(file)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use std::io::Write;

    fn append(path: &Path, bytes: &[u8]) {
        let mut f = OpenOptions::new().create(true).append(true).open(path).unwrap();
        f.write_all(bytes).unwrap();
    }

    fn lines(v: &[(u64, &str)]) -> Vec<(u64, String)> {
        v.iter().map(|(o, s)| (*o, s.to_string())).collect()
    }

    #[test]
    fn complete_lines_come_with_offsets_and_the_partial_line_waits() {
        let tmp = TempDir::new("tail-lines");
        let path = tmp.path.join("t.jsonl");
        append(&path, b"a\n{b}\r\n\n  \npart");
        let mut tail = JsonlTail::new(&path);
        assert_eq!(tail.eof(), 0);
        let (got, eof) = tail.read_new().unwrap();
        assert_eq!(got, lines(&[(0, "a"), (2, "{b}")]));
        assert_eq!(eof, 11);
        append(&path, b"ial\n");
        let (got, eof) = tail.read_new().unwrap();
        assert_eq!(got, lines(&[(11, "partial")]));
        assert_eq!(eof, 19);
        assert_eq!(tail.read_new().unwrap(), (Vec::new(), 19));
    }

    #[test]
    fn invalid_utf8_is_replaced_not_fatal() {
        let tmp = TempDir::new("tail-utf8");
        let path = tmp.path.join("t.jsonl");
        append(&path, b"\xff{}\nok\n");
        let mut tail = JsonlTail::new(&path);
        assert_eq!(tail.read_new().unwrap(), (lines(&[(0, "\u{fffd}{}"), (4, "ok")]), 7));
    }

    #[test]
    fn start_at_eof_skips_history() {
        let tmp = TempDir::new("tail-eof");
        let path = tmp.path.join("rollout.jsonl");
        append(&path, b"x\ny\n");
        let mut tail = JsonlTail::new(&path);
        assert_eq!(tail.start_at_eof().unwrap(), 4);
        append(&path, b"z\n");
        assert_eq!(tail.read_new().unwrap(), (lines(&[(4, "z")]), 6));
    }

    #[test]
    fn start_at_eof_inside_a_line_drops_the_rest_of_that_line() {
        let tmp = TempDir::new("tail-eof-mid");
        let path = tmp.path.join("rollout.jsonl");
        append(&path, b"x\nhal");
        let mut tail = JsonlTail::new(&path);
        assert_eq!(tail.start_at_eof().unwrap(), 5);
        assert_eq!(tail.eof(), 5, "a mid-line eof: everything before it is old");
        append(&path, b"f");
        assert_eq!(tail.read_new().unwrap(), (Vec::new(), 6), "still skipping the old line");
        append(&path, b"\nnext\n");
        assert_eq!(tail.read_new().unwrap(), (lines(&[(7, "next")]), 12));
    }

    #[test]
    fn a_missing_file_starts_at_zero() {
        let tmp = TempDir::new("tail-missing");
        let path = tmp.path.join("later.jsonl");
        let mut tail = JsonlTail::new(&path);
        assert_eq!(tail.start_at_eof().unwrap(), 0);
        tail.start_with_window(WINDOW_BYTES).unwrap();
        assert_eq!(tail.read_new().unwrap(), (Vec::new(), 0));
        append(&path, b"first\n");
        assert_eq!(tail.read_new().unwrap(), (lines(&[(0, "first")]), 6));
    }

    #[test]
    fn the_window_drops_its_incomplete_first_line() {
        let tmp = TempDir::new("tail-window");
        let path = tmp.path.join("s.jsonl");
        append(&path, b"0123456789\nsecond\nthird\n");
        let len = 24;
        let mut tail = JsonlTail::new(&path);
        tail.start_with_window(len - 3).unwrap(); // starts at offset 3, inside "0123456789"
        assert_eq!(tail.read_new().unwrap(), (lines(&[(11, "second"), (18, "third")]), 24));

        let mut whole = JsonlTail::new(&path);
        whole.start_with_window(WINDOW_BYTES).unwrap(); // window larger than the file → offset 0
        let (got, _) = whole.read_new().unwrap();
        assert_eq!(got.first(), Some(&(0, "0123456789".to_string())));
    }

    #[test]
    fn a_shorter_file_is_read_again_from_zero() {
        let tmp = TempDir::new("tail-shrink");
        let path = tmp.path.join("t.jsonl");
        append(&path, b"aaaa\nbbbb\n");
        let mut tail = JsonlTail::new(&path);
        assert_eq!(tail.read_new().unwrap().0.len(), 2);
        std::fs::write(&path, b"c\n").unwrap();
        assert_eq!(tail.read_new().unwrap(), (lines(&[(0, "c")]), 2));
    }

    #[test]
    fn reading_never_blocks_the_writer_and_holds_nothing_open() {
        let tmp = TempDir::new("tail-share");
        let path = tmp.path.join("live.jsonl");
        // The agent keeps its handle open and grants read+write sharing only.
        let mut writer = OpenOptions::new()
            .create(true)
            .append(true)
            .share_mode(0x1 | 0x2)
            .open(&path)
            .unwrap();
        writer.write_all(b"{\"n\":1}\n").unwrap();
        let mut tail = JsonlTail::new(&path);
        assert_eq!(tail.read_new().unwrap(), (lines(&[(0, "{\"n\":1}")]), 8));
        writer.write_all(b"{\"n\":2}\n").unwrap();
        assert_eq!(tail.read_new().unwrap(), (lines(&[(8, "{\"n\":2}")]), 16));
        drop(writer);
        // Between reads the tail holds no handle, so the file can be deleted.
        std::fs::remove_file(&path).unwrap();
        assert_eq!(tail.read_new().unwrap(), (Vec::new(), 0));
    }
}
