// Dropped files are copied into %LOCALAPPDATA%\Mayak\inbox so the original is
// never touched and the copy survives the drag source going away.
// The inbox is swept of anything older than a week, as on macOS.

use std::fs::File;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::Serialize;

use crate::settings;

const KEEP_FOR: Duration = Duration::from_secs(7 * 24 * 60 * 60);

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DroppedFile {
    pub name: String,
    pub path: String,
    pub size: u64,
}

pub fn inbox_dir() -> PathBuf {
    settings::local_dir().join("inbox")
}

pub fn ingest(source: &str) -> Result<DroppedFile, String> {
    // The real inbox lives in our private local folder; make sure it exists first.
    crate::platform::ensure_private_dir(&settings::local_dir()).map_err(|e| e.to_string())?;
    ingest_into(source, &inbox_dir())
}

/// `ingest` with the inbox passed in, so tests never touch the real one.
pub fn ingest_into(source: &str, inbox: &Path) -> Result<DroppedFile, String> {
    let src = Path::new(source);
    let meta = std::fs::metadata(src).map_err(|e| format!("cannot read {source}: {e}"))?;
    if meta.is_dir() {
        return Err("Folders can't be dropped yet.".into());
    }

    let dir = inbox.to_path_buf();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let name = src
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".into());
    let mut from = std::fs::File::open(src).map_err(|e| format!("cannot read {source}: {e}"))?;
    let (dest, mut to) = create_unique(&dir, &name)?;
    // A plain stream copy into the file we just created: unlike CopyFileEx it carries no
    // timestamps across, so a file last edited three years ago does not arrive already older
    // than the sweep window. The inbox ages from when *we* copied it.
    let copied = std::io::copy(&mut from, &mut to).map(|_| ());
    drop(to);
    discard_on_error(&dest, copied, "cannot copy")?;
    sweep(&dir);

    Ok(DroppedFile {
        name,
        path: dest.to_string_lossy().to_string(),
        size: meta.len(),
    })
}

/// Largest file the island takes by its contents. The page checks it before reading the file,
/// which is what bounds memory; this is the same limit on the bytes that did arrive.
pub const MAX_DROP_BYTES: u64 = 200 * 1024 * 1024;

fn too_big(len: u64) -> bool {
    len > MAX_DROP_BYTES
}

/// A file dropped on the island page, handed over by its contents: WebView2 gives an HTML5 drop
/// no path. Lands in the inbox exactly like `ingest`.
pub fn ingest_bytes(name: &str, bytes: &[u8]) -> Result<DroppedFile, String> {
    crate::platform::ensure_private_dir(&settings::local_dir()).map_err(|e| e.to_string())?;
    ingest_bytes_into(name, bytes, &inbox_dir())
}

/// `ingest_bytes` with the inbox passed in, so tests never touch the real one.
pub fn ingest_bytes_into(name: &str, bytes: &[u8], inbox: &Path) -> Result<DroppedFile, String> {
    if too_big(bytes.len() as u64) {
        return Err("That file is too big to drop (200 MB max).".into());
    }
    std::fs::create_dir_all(inbox).map_err(|e| e.to_string())?;

    let name = safe_name(name);
    let (dest, mut to) = create_unique(inbox, &name)?;
    let written = to.write_all(bytes);
    drop(to);
    discard_on_error(&dest, written, "cannot save")?;
    sweep(inbox);

    Ok(DroppedFile {
        name,
        path: dest.to_string_lossy().to_string(),
        size: bytes.len() as u64,
    })
}

/// Most names `create_unique` tries: `name`, then `name (2)` … `name (999)`.
const MAX_NAME_TRIES: u32 = 999;

/// Creates `name`, or `name (2)`, `name (3)`… next to it, and returns the new empty file. The
/// file is created with `create_new`, so the name is reserved by the same call that finds it free:
/// the inbox never overwrites a copy, even when two drops race. All names taken → an error.
fn create_unique(dir: &Path, name: &str) -> Result<(PathBuf, File), String> {
    let as_path = Path::new(name);
    let stem = as_path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let ext = as_path.extension().map(|s| format!(".{}", s.to_string_lossy())).unwrap_or_default();
    for i in 1..=MAX_NAME_TRIES {
        let candidate = if i == 1 { dir.join(name) } else { dir.join(format!("{stem} ({i}){ext}")) };
        match File::options().write(true).create_new(true).open(&candidate) {
            Ok(file) => return Ok((candidate, file)),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("cannot save: {e}")),
        }
    }
    Err(format!("The inbox already holds {MAX_NAME_TRIES} files named like {name}."))
}

/// A copy that failed half way must not stay in the inbox as a truncated file.
fn discard_on_error(dest: &Path, result: std::io::Result<()>, what: &str) -> Result<(), String> {
    let Err(err) = result else { return Ok(()) };
    match std::fs::remove_file(dest) {
        Ok(()) => Err(format!("{what}: {err}")),
        Err(cleanup) => Err(format!("{what}: {err} (and the partial copy could not be removed: {cleanup})")),
    }
}

/// Device names Windows reserves whatever the extension: `NUL.txt` is the null device too.
const RESERVED_STEMS: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// The name the page sent, reduced to one plain file name: only what follows the last `/` or
/// `\`, with the characters Windows refuses in a name replaced by `_`, the trailing dots and
/// spaces Windows would drop removed, and a reserved device name prefixed with `_`. A name with
/// nothing left is `file`, as for `ingest`. It must never steer the write out of the inbox.
fn safe_name(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let clean: String = last
        .chars()
        .map(|c| if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') { '_' } else { c })
        .collect();
    let clean = clean.trim_end_matches(['.', ' ']);
    if clean.is_empty() {
        return "file".into();
    }
    // Windows also ignores spaces before the extension: `CON .txt` is CON.
    let stem = clean.split('.').next().unwrap_or_default().trim_end_matches(' ').to_ascii_uppercase();
    if RESERVED_STEMS.contains(&stem.as_str()) {
        format!("_{clean}")
    } else {
        clean.to_string()
    }
}

/// The file name from the `x-file-name` header, which the page fills with
/// `encodeURIComponent(file.name)`; then `safe_name`. An escape that is not two hex digits is
/// kept as it is.
pub fn header_name(value: Option<&str>) -> String {
    let raw = value.unwrap_or_default().as_bytes();
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%' && i + 2 < raw.len() {
            if let (Some(hi), Some(lo)) = (hex(raw[i + 1]), hex(raw[i + 2])) {
                out.push(hi << 4 | lo);
                i += 3;
                continue;
            }
        }
        out.push(raw[i]);
        i += 1;
    }
    safe_name(&String::from_utf8_lossy(&out))
}

fn hex(b: u8) -> Option<u8> {
    (b as char).to_digit(16).map(|d| d as u8)
}

/// Drops anything copied here more than a week ago. `ingest` stamps every copy
/// with the time it landed, so this really is the age of the copy and not the
/// age of whatever the user happened to drag in.
fn sweep(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        let Ok(copied) = meta.modified() else { continue };
        if now.duration_since(copied).map(|age| age > KEEP_FOR).unwrap_or(false) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    #[test]
    fn ingest_copies_and_never_overwrites() {
        let tmp = TempDir::new("files");
        let inbox = tmp.path.join("inbox");
        let source = tmp.path.join("note.txt");
        std::fs::write(&source, b"hello").unwrap();

        let first = ingest_into(source.to_str().unwrap(), &inbox).unwrap();
        assert_eq!(first.name, "note.txt");
        assert!(
            Path::new(&first.path).starts_with(&inbox),
            "the copy must land in the inbox it was given, got {}",
            first.path
        );
        assert_eq!(std::fs::read(&first.path).unwrap(), b"hello");

        // A second drop of the same name must not clobber the first copy.
        std::fs::write(&source, b"second").unwrap();
        let second = ingest_into(source.to_str().unwrap(), &inbox).unwrap();
        assert_ne!(first.path, second.path);
        assert_eq!(std::fs::read(&first.path).unwrap(), b"hello");
        assert_eq!(std::fs::read(&second.path).unwrap(), b"second");

        // Folders are refused rather than silently ignored.
        assert!(ingest_into(tmp.path.to_str().unwrap(), &inbox).is_err());

        // An ancient source must not arrive already older than the sweep window.
        let old_source = tmp.path.join("ancient.txt");
        std::fs::write(&old_source, b"old").unwrap();
        let long_ago = SystemTime::now() - KEEP_FOR - Duration::from_secs(60 * 60);
        std::fs::File::options()
            .write(true)
            .open(&old_source)
            .unwrap()
            .set_modified(long_ago)
            .unwrap();
        let aged = ingest_into(old_source.to_str().unwrap(), &inbox).unwrap();
        assert!(
            Path::new(&aged.path).exists(),
            "a file copied just now was swept as if it were a week old"
        );
        // `tmp` removes the source files and the inbox when it is dropped.
    }

    #[test]
    fn ingest_bytes_lands_in_the_inbox_with_the_same_collision_rule_as_ingest() {
        let tmp = TempDir::new("files-bytes");
        let inbox = tmp.path.join("inbox");

        let first = ingest_bytes_into("note.txt", b"hello", &inbox).unwrap();
        assert_eq!(first.name, "note.txt");
        assert_eq!(first.size, 5);
        assert_eq!(Path::new(&first.path), inbox.join("note.txt"));
        assert_eq!(std::fs::read(&first.path).unwrap(), b"hello");

        // Same name again: `name (2).ext`, the first copy untouched.
        let second = ingest_bytes_into("note.txt", b"second", &inbox).unwrap();
        assert_eq!(Path::new(&second.path), inbox.join("note (2).txt"));
        assert_eq!(std::fs::read(&first.path).unwrap(), b"hello");
        assert_eq!(std::fs::read(&second.path).unwrap(), b"second");

        // A path drop of the same name follows on from the same numbering.
        let source = tmp.path.join("note.txt");
        std::fs::write(&source, b"third").unwrap();
        let third = ingest_into(source.to_str().unwrap(), &inbox).unwrap();
        assert_eq!(Path::new(&third.path), inbox.join("note (3).txt"));

        // And the other way round, for a name without an extension.
        std::fs::write(tmp.path.join("README"), b"a").unwrap();
        ingest_into(tmp.path.join("README").to_str().unwrap(), &inbox).unwrap();
        let readme = ingest_bytes_into("README", b"b", &inbox).unwrap();
        assert_eq!(Path::new(&readme.path), inbox.join("README (2)"));
    }

    #[test]
    fn ingest_bytes_sweeps_the_inbox_like_ingest() {
        let tmp = TempDir::new("files-bytes-sweep");
        let inbox = tmp.path.join("inbox");
        std::fs::create_dir_all(&inbox).unwrap();
        let stale = inbox.join("stale.txt");
        std::fs::write(&stale, b"old").unwrap();
        let long_ago = SystemTime::now() - KEEP_FOR - Duration::from_secs(60 * 60);
        std::fs::File::options().write(true).open(&stale).unwrap().set_modified(long_ago).unwrap();

        ingest_bytes_into("fresh.txt", b"new", &inbox).unwrap();
        assert!(!stale.exists(), "a week-old copy survived a drop");
        assert!(inbox.join("fresh.txt").exists());
    }

    #[test]
    fn ingest_bytes_refuses_more_than_200_mb() {
        assert_eq!(MAX_DROP_BYTES, 200 * 1024 * 1024);
        assert!(!too_big(MAX_DROP_BYTES));
        assert!(too_big(MAX_DROP_BYTES + 1));

        let tmp = TempDir::new("files-bytes-big");
        let inbox = tmp.path.join("inbox");
        // Zeroed pages are never touched, so this costs no real memory.
        let big = vec![0u8; MAX_DROP_BYTES as usize + 1];
        let err = ingest_bytes_into("big.bin", &big, &inbox).err().expect("a 200 MB + 1 drop was taken");
        assert!(err.contains("200 MB"), "unexpected message: {err}");
        assert!(!inbox.join("big.bin").exists());
    }

    #[test]
    fn a_dropped_name_is_one_plain_file_name() {
        assert_eq!(safe_name("note.txt"), "note.txt");
        assert_eq!(safe_name(""), "file");
        assert_eq!(safe_name("   "), "file");
        assert_eq!(safe_name("."), "file");
        assert_eq!(safe_name(".."), "file");
        assert_eq!(safe_name("..\\..\\evil.txt"), "evil.txt");
        assert_eq!(safe_name("../../evil.txt"), "evil.txt");
        assert_eq!(safe_name("C:\\Windows\\win.ini"), "win.ini");
        assert_eq!(safe_name("dir/"), "file");
        // Characters Windows refuses in a name (':' would also open an alternate stream).
        assert_eq!(safe_name("a:b*c?.txt"), "a_b_c_.txt");
        assert_eq!(safe_name("x\u{1}y.txt"), "x_y.txt");
        assert_eq!(safe_name("naïve résumé.pdf"), "naïve résumé.pdf");

        let tmp = TempDir::new("files-bytes-names");
        let inbox = tmp.path.join("inbox");
        let escaped = ingest_bytes_into("..\\..\\evil.txt", b"x", &inbox).unwrap();
        assert_eq!(escaped.name, "evil.txt");
        assert_eq!(Path::new(&escaped.path), inbox.join("evil.txt"));
        assert!(!tmp.path.join("evil.txt").exists());
        let empty = ingest_bytes_into("", b"y", &inbox).unwrap();
        assert_eq!(Path::new(&empty.path), inbox.join("file"));
    }

    #[test]
    fn reserved_windows_names_and_trailing_dots_are_made_safe() {
        assert_eq!(safe_name("CON"), "_CON");
        assert_eq!(safe_name("nul.txt"), "_nul.txt");
        assert_eq!(safe_name("COM1"), "_COM1");
        assert_eq!(safe_name("lpt9.tar.gz"), "_lpt9.tar.gz");
        assert_eq!(safe_name("Aux"), "_Aux");
        assert_eq!(safe_name("prn.."), "_prn");
        // Not reserved: other numbers, longer stems, the name only inside the stem.
        assert_eq!(safe_name("COM0"), "COM0");
        assert_eq!(safe_name("COM10"), "COM10");
        assert_eq!(safe_name("CONSOLE.txt"), "CONSOLE.txt");
        assert_eq!(safe_name("my.con"), "my.con");
        // Windows drops trailing dots and spaces itself: do it first, so the name we report is
        // the name on disk.
        assert_eq!(safe_name("a."), "a");
        assert_eq!(safe_name("a. . "), "a");
        assert_eq!(safe_name("..."), "file");
        assert_eq!(safe_name(". ."), "file");
        assert_eq!(safe_name("name .txt"), "name .txt");
        assert_eq!(safe_name("con .txt"), "_con .txt");
    }

    #[test]
    fn an_existing_name_gets_the_next_free_one_and_is_never_overwritten() {
        let tmp = TempDir::new("files-unique");
        let dir = &tmp.path;
        std::fs::write(dir.join("a.txt"), b"one").unwrap();
        std::fs::write(dir.join("a (2).txt"), b"two").unwrap();

        let (path, file) = create_unique(dir, "a.txt").unwrap();
        drop(file);
        assert_eq!(path, dir.join("a (3).txt"));
        assert!(path.exists(), "the free name must be reserved on disk");
        assert_eq!(std::fs::read(dir.join("a.txt")).unwrap(), b"one");
        assert_eq!(std::fs::read(dir.join("a (2).txt")).unwrap(), b"two");

        let (fresh, _file) = create_unique(dir, "b.txt").unwrap();
        assert_eq!(fresh, dir.join("b.txt"));
    }

    #[test]
    fn nine_hundred_ninety_nine_existing_names_give_an_error_not_an_overwrite() {
        let tmp = TempDir::new("files-full");
        let inbox = tmp.path.join("inbox");
        std::fs::create_dir_all(&inbox).unwrap();
        std::fs::write(inbox.join("x.txt"), b"first").unwrap();
        for i in 2..1000 {
            std::fs::write(inbox.join(format!("x ({i}).txt")), b"").unwrap();
        }

        assert!(create_unique(&inbox, "x.txt").is_err());
        assert!(ingest_bytes_into("x.txt", b"new", &inbox).is_err());
        let source = tmp.path.join("x.txt");
        std::fs::write(&source, b"new").unwrap();
        assert!(ingest_into(source.to_str().unwrap(), &inbox).is_err());

        assert_eq!(std::fs::read(inbox.join("x.txt")).unwrap(), b"first");
        assert_eq!(std::fs::read_dir(&inbox).unwrap().count(), 999);
    }

    #[test]
    fn the_header_name_is_uri_decoded() {
        // What encodeURIComponent makes of the names.
        assert_eq!(header_name(Some("note.txt")), "note.txt");
        assert_eq!(header_name(Some("my%20report%20(1).pdf")), "my report (1).pdf");
        assert_eq!(header_name(Some("na%C3%AFve%20r%C3%A9sum%C3%A9.pdf")), "naïve résumé.pdf");
        assert_eq!(header_name(Some("100%25.txt")), "100%.txt");
        // Not a valid escape: kept as it is, never a panic.
        assert_eq!(header_name(Some("50%.txt")), "50%.txt");
        assert_eq!(header_name(Some("%zz%4")), "%zz%4");
        assert_eq!(header_name(Some("%+1x")), "%+1x");
        // No header, or one that decodes to nothing usable.
        assert_eq!(header_name(None), "file");
        assert_eq!(header_name(Some("")), "file");
        // Decoding comes before sanitizing: an encoded separator cannot climb out.
        assert_eq!(header_name(Some("..%5C..%5Cevil.txt")), "evil.txt");
        assert_eq!(header_name(Some("..%2F..%2Fevil.txt")), "evil.txt");
    }
}
