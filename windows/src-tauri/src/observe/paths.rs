// Path identity for sessions and projects (spec §5.7, §13 point 2).
//
// Pure: no environment variables and no file system access, except `git_probe`, which
// callers pass to `project_of` / `ProjectCache::get` and tests replace with a closure.

use std::collections::HashMap;
use std::path::Path;

/// The separator of normalized paths: the platform's own.
#[cfg(windows)]
pub const SEP: char = '\\';
#[cfg(not(windows))]
pub const SEP: char = '/';

/// A normalized path. `display` keeps the original letter case (drive letter
/// uppercased); `key` is its lowercase form, used for identity and comparisons.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NormPath {
    pub display: String,
    pub key: String,
}

/// Spec §5.7 order: trim → strip "file:///" (ASCII case-insensitive) → "/c:/" → "c:/" →
/// '/' → '\' → strip "\\?\" → drive letter uppercase → collapse repeated '\' → resolve "."
/// and ".." lexically (".." at the root is dropped) → strip the trailing '\' except "X:\"
/// (bare "X:" → "X:\") → key = display.to_lowercase(). No percent-decoding, no UNC (D8).
#[cfg(windows)]
pub fn normalize(raw: &str) -> NormPath {
    let mut s = raw.trim();
    if let Some(head) = s.get(..8) {
        if head.eq_ignore_ascii_case("file:///") {
            s = &s[8..];
        }
    }
    let b = s.as_bytes();
    if b.len() >= 3 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b':' {
        s = &s[1..];
    }
    let slashed = s.replace('/', "\\");
    let text = slashed.strip_prefix(r"\\?\").unwrap_or(&slashed);

    let (drive, rest) = match text.as_bytes() {
        [d, b':', ..] if d.is_ascii_alphabetic() => (Some(d.to_ascii_uppercase() as char), &text[2..]),
        _ => (None, text),
    };
    // A drive path is always absolute here: drive-relative "C:foo" does not occur in agent
    // working directories.
    let rooted = drive.is_some() || rest.starts_with('\\');
    let mut parts: Vec<&str> = Vec::new();
    for seg in rest.split('\\') {
        match seg {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|p| *p != "..") {
                    parts.pop();
                } else if !rooted {
                    parts.push("..");
                }
            }
            _ => parts.push(seg),
        }
    }
    let joined = parts.join("\\");
    let display = match drive {
        Some(d) => format!("{d}:\\{joined}"),
        None if rooted => format!("\\{joined}"),
        None => joined,
    };
    let key = display.to_lowercase();
    NormPath { display, key }
}

/// The same steps on a '/' path: trim → strip "file://" (ASCII case-insensitive) → collapse
/// repeated '/' → resolve "." and ".." lexically → strip the trailing '/' except "/" → key =
/// display.to_lowercase() (the default macOS volume ignores case too).
#[cfg(not(windows))]
pub fn normalize(raw: &str) -> NormPath {
    let mut s = raw.trim();
    if let Some(head) = s.get(..7) {
        if head.eq_ignore_ascii_case("file://") {
            s = &s[7..];
        }
    }
    let rooted = s.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for seg in s.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|p| *p != "..") {
                    parts.pop();
                } else if !rooted {
                    parts.push("..");
                }
            }
            _ => parts.push(seg),
        }
    }
    let joined = parts.join("/");
    let display = if rooted { format!("/{joined}") } else { joined };
    let key = display.to_lowercase();
    NormPath { display, key }
}

/// A project: the git root found by walking up from a path, or the folder itself.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProjectRef {
    pub key: String,
    /// `NormPath.display` of the root.
    pub root: String,
}

/// First of normalize(raw) and its ancestors (self included) where probe(dir\.git) is true;
/// none → normalize(raw) itself. The drive root "C:\" probes "C:\.git".
pub fn project_of(raw: &str, probe: impl Fn(&Path) -> bool) -> ProjectRef {
    let norm = normalize(raw);
    if norm.display.is_empty() {
        return ProjectRef { key: String::new(), root: String::new() };
    }
    let mut dir = norm.display.as_str();
    loop {
        let git = if dir.ends_with(SEP) { format!("{dir}.git") } else { format!("{dir}{SEP}.git") };
        if probe(Path::new(&git)) {
            return ProjectRef { key: dir.to_lowercase(), root: dir.to_string() };
        }
        match parent(dir) {
            Some(up) => dir = up,
            None => break,
        }
    }
    ProjectRef { key: norm.key, root: norm.display }
}

/// Real probe: `.git` exists as a folder (repository) or a file (worktree, submodule).
pub fn git_probe(p: &Path) -> bool {
    p.exists()
}

/// Lifetime cache owned by one source thread (spec §5.7): one probe walk per distinct path.
#[derive(Default)]
pub struct ProjectCache {
    by_key: HashMap<String, ProjectRef>,
}

impl ProjectCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&mut self, raw: &str, probe: impl Fn(&Path) -> bool) -> ProjectRef {
        let key = normalize(raw).key;
        if let Some(found) = self.by_key.get(&key) {
            return found.clone();
        }
        let project = project_of(raw, probe);
        self.by_key.insert(key, project.clone());
        project
    }
}

/// Lexical parent of a normalized display path: "C:\a\b" → "C:\a", "C:\a" → "C:\",
/// "\a" → "\"; roots and single relative segments → None.
fn parent(display: &str) -> Option<&str> {
    if is_root(display) {
        return None;
    }
    let idx = display.rfind(SEP)?;
    let head = &display[..idx];
    if head.is_empty() {
        return Some(&display[..1]);
    }
    let h = head.as_bytes();
    if cfg!(windows) && h.len() == 2 && h[0].is_ascii_alphabetic() && h[1] == b':' {
        return Some(&display[..idx + 1]);
    }
    Some(head)
}

fn is_root(display: &str) -> bool {
    let b = display.as_bytes();
    display.len() == 1 && display.starts_with(SEP)
        || (cfg!(windows) && b.len() == 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\')
}

/// Segment glob over lowercase '\'-separated keys. Pattern segments: "**" = zero or more
/// segments, "*" = exactly one segment, "abc*" = one segment starting with "abc", else
/// literal. Empty segments (a trailing '\') are ignored on both sides.
pub fn glob_match(key: &str, pattern: &str) -> bool {
    let k: Vec<&str> = key.split(SEP).filter(|s| !s.is_empty()).collect();
    let p: Vec<&str> = pattern.split(SEP).filter(|s| !s.is_empty()).collect();
    match_segments(&k, &p)
}

fn match_segments(key: &[&str], pattern: &[&str]) -> bool {
    match pattern.split_first() {
        None => key.is_empty(),
        Some((&"**", rest)) => (0..=key.len()).any(|skip| match_segments(&key[skip..], rest)),
        Some((seg, rest)) => match key.split_first() {
            Some((k, key_rest)) => segment_matches(k, seg) && match_segments(key_rest, rest),
            None => false,
        },
    }
}

fn segment_matches(segment: &str, pattern: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => segment.starts_with(prefix),
        None => segment == pattern,
    }
}

/// Last component of a display path; "C:\" → "C:".
pub fn basename(display: &str) -> String {
    let trimmed = display.trim_end_matches(SEP);
    trimmed.rsplit(SEP).next().unwrap_or("").to_string()
}

/// Basename of the parent; None for a drive root (or a path without a parent).
pub fn parent_basename(display: &str) -> Option<String> {
    parent(display).map(basename)
}

#[cfg(all(test, not(windows)))]
mod unix_tests {
    use super::*;

    fn git_at<'a>(paths: &'a [&'static str]) -> impl Fn(&Path) -> bool + 'a {
        move |p: &Path| paths.iter().any(|g| p.to_str() == Some(*g))
    }

    #[test]
    fn every_spelling_of_one_folder_gives_one_key() {
        for raw in ["/Users/x", "/Users/x/", "file:///Users/x", "FILE:///Users/x", "  //Users///x  ", "/Users/./y/../x"] {
            let n = normalize(raw);
            assert_eq!(n.display, "/Users/x", "display of {raw:?}");
            assert_eq!(n.key, "/users/x", "key of {raw:?}");
        }
        assert_eq!(normalize("/..").display, "/");
        assert_eq!(normalize("/").display, "/");
    }

    #[test]
    fn project_root_is_the_nearest_folder_with_git_and_the_root_is_probed_last() {
        let p = project_of("/Users/me/repo/src/deep", git_at(&["/Users/me/repo/.git"]));
        assert_eq!(p.root, "/Users/me/repo");
        assert_eq!(p.key, "/users/me/repo");
        assert_eq!(project_of("/a/b", git_at(&["/.git"])).root, "/");
        assert_eq!(project_of("/a/b", git_at(&[])).root, "/a/b");
    }

    #[test]
    fn basenames_parents_and_globs() {
        assert_eq!(basename("/Users/me/repo"), "repo");
        assert_eq!(parent_basename("/Users/me/repo").as_deref(), Some("me"));
        assert_eq!(parent_basename("/"), None);
        assert!(glob_match("/users/me/repo", "/users/**/repo"));
        assert!(glob_match("/users/me/repo", "/users/*/re*"));
        assert!(!glob_match("/users/me/repo", "/users/*"));
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// A probe that answers true exactly for the given `.git` paths.
    fn git_at(paths: &[&str]) -> impl Fn(&Path) -> bool {
        let paths: Vec<String> = paths.iter().map(|p| p.to_string()).collect();
        move |p: &Path| paths.iter().any(|g| p.to_str() == Some(g.as_str()))
    }

    #[test]
    fn every_spelling_of_one_folder_gives_one_key() {
        let spellings = [
            r"c:/Users/x",
            r"C:\Users\x\",
            r"/c:/Users/x",
            r"file:///c:/Users/x",
            r"FILE:///C:/Users/x",
            r"\\?\C:\Users\x",
            r"  C:\\Users\\\x  ",
            r"C:\Users\.\y\..\x",
        ];
        for raw in spellings {
            let n = normalize(raw);
            assert_eq!(n.display, r"C:\Users\x", "display of {raw:?}");
            assert_eq!(n.key, r"c:\users\x", "key of {raw:?}");
        }
    }

    #[test]
    fn mixed_case_and_cyrillic_share_the_lowercase_key() {
        let a = normalize(r"C:\Users\HP\Проекты\Маяк");
        let b = normalize(r"c:/users/hp/ПРОЕКТЫ/маяк/");
        assert_eq!(a.display, r"C:\Users\HP\Проекты\Маяк");
        assert_eq!(b.display, r"C:\users\hp\ПРОЕКТЫ\маяк");
        assert_eq!(a.key, r"c:\users\hp\проекты\маяк");
        assert_eq!(a.key, b.key);
    }

    #[test]
    fn drive_roots_and_dot_dot_at_the_root() {
        assert_eq!(normalize("c:").display, r"C:\");
        assert_eq!(normalize(r"c:\").display, r"C:\");
        assert_eq!(normalize("/d:/").display, r"D:\");
        assert_eq!(normalize(r"C:\..\..\Windows").display, r"C:\Windows");
        assert_eq!(normalize(r"C:\a\b\..\..\..").display, r"C:\");
        assert_eq!(normalize(r"C:\a\b\..\..\..").key, r"c:\");
        assert_eq!(normalize("").display, "");
    }

    #[test]
    fn project_root_is_the_nearest_folder_with_git() {
        let p = project_of(r"c:/work/repo/src/ui", git_at(&[r"C:\work\repo\.git"]));
        assert_eq!(p, ProjectRef { key: r"c:\work\repo".into(), root: r"C:\work\repo".into() });
    }

    #[test]
    fn a_nested_git_file_is_its_own_project() {
        // A submodule or worktree has a `.git` *file*; the probe does not care which.
        let probe = git_at(&[r"C:\work\repo\.git", r"C:\work\repo\vendor\lib\.git"]);
        assert_eq!(project_of(r"C:\work\repo\vendor\lib\src", &probe).root, r"C:\work\repo\vendor\lib");
        assert_eq!(project_of(r"C:\work\repo\docs", &probe).root, r"C:\work\repo");
    }

    #[test]
    fn without_git_the_folder_itself_is_the_project() {
        let p = project_of(r"C:\Users\dev\scratch\", git_at(&[]));
        assert_eq!(p, ProjectRef { key: r"c:\users\dev\scratch".into(), root: r"C:\Users\dev\scratch".into() });
    }

    #[test]
    fn an_empty_path_is_never_probed() {
        let p = project_of("  ", |_: &Path| panic!("an empty path must not be probed"));
        assert_eq!(p, ProjectRef { key: String::new(), root: String::new() });
    }

    #[test]
    fn the_drive_root_is_probed_last() {
        let probed = std::cell::RefCell::new(Vec::new());
        let p = project_of(r"C:\a\b", |path: &Path| {
            probed.borrow_mut().push(path.to_str().unwrap().to_string());
            path.to_str() == Some(r"C:\.git")
        });
        assert_eq!(p, ProjectRef { key: r"c:\".into(), root: r"C:\".into() });
        assert_eq!(*probed.borrow(), vec![r"C:\a\b\.git", r"C:\a\.git", r"C:\.git"]);
    }

    #[test]
    fn the_cache_walks_each_path_once() {
        let calls = Cell::new(0);
        let probe = |p: &Path| {
            calls.set(calls.get() + 1);
            p.to_str() == Some(r"C:\work\repo\.git")
        };
        let mut cache = ProjectCache::new();
        let first = cache.get(r"C:\work\repo\src", probe);
        let walked = calls.get();
        let second = cache.get(r"c:/WORK/repo/src/", probe);
        assert_eq!(first, second);
        assert_eq!(calls.get(), walked, "a cached path must not be probed again");
        assert_eq!(first.root, r"C:\work\repo");
    }

    #[test]
    fn git_probe_accepts_a_git_folder_and_a_git_file() {
        // The only test that touches the disk: the real probe decides "folder or file" (§5.7).
        let tmp = crate::testutil::TempDir::new("git-probe");
        let repo = tmp.path.join("a");
        let worktree = tmp.path.join("b");
        let plain = tmp.path.join("c");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::write(worktree.join(".git"), b"gitdir: C:/elsewhere/.git/worktrees/b\n").unwrap();
        std::fs::create_dir_all(&plain).unwrap();
        assert!(git_probe(&repo.join(".git")), "a .git folder (repository)");
        assert!(git_probe(&worktree.join(".git")), "a .git file (worktree)");
        assert!(!git_probe(&plain.join(".git")), "no .git");
        let root = normalize(repo.to_str().unwrap()).display;
        assert_eq!(project_of(repo.join("src").to_str().unwrap(), git_probe).root, root);
    }

    #[test]
    fn glob_segments() {
        let desktop = r"c:\users\dev\appdata\local\packages\claude_pzs8sxrjxfjjc\localcache\roaming\claude\claude-code\2.1.286\claude.exe";
        assert!(glob_match(desktop, r"**\packages\claude_*\localcache\roaming\claude\claude-code\*\claude.exe"));
        assert!(!glob_match(desktop, r"**\packages\claude_*\localcache\roaming\claude\*\claude.exe"));
        let cursor = r"c:\users\dev\.cursor\extensions\anthropic.claude-code-2.1.286-win32-x64\resources\native-binary\claude.exe";
        assert!(glob_match(cursor, r"**\.cursor\extensions\anthropic.claude-code-*\**\claude.exe"));
        assert!(!glob_match(cursor, r"**\.vscode\extensions\anthropic.claude-code-*\**\claude.exe"));
        // "**" may match zero segments; "*" matches exactly one.
        assert!(glob_match(r"c:\a\claude.exe", r"c:\**\a\claude.exe"));
        assert!(glob_match(r"c:\a\b", r"c:\*\b"));
        assert!(!glob_match(r"c:\a\x\b", r"c:\*\b"));
        assert!(!glob_match(r"c:\a", r"c:\a\*"));
        assert!(glob_match(r"c:\", r"c:\**"));
    }

    #[test]
    fn basenames() {
        assert_eq!(basename(r"C:\work\repo"), "repo");
        assert_eq!(basename(r"C:\"), "C:");
        assert_eq!(parent_basename(r"C:\work\repo"), Some("work".to_string()));
        assert_eq!(parent_basename(r"C:\repo"), Some("C:".to_string()));
        assert_eq!(parent_basename(r"C:\"), None);
    }
}
