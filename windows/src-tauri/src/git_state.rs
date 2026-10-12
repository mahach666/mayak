// Git state of a project in the sessions list (ideas #5): branch, ahead / behind its upstream,
// changed / untracked / conflicted files, +N −M lines against HEAD and a merge or rebase in
// progress. Read-only and local: `git status --porcelain=v2 --branch -z` and, only when something
// changed, `git diff HEAD --numstat`, run in the project root with optional locks off, the
// fsmonitor off, no external diff and no textconv, under one deadline. Never a fetch. Asked by the
// island only while its session list is on screen (when it opens and when a turn ends), so nothing
// runs while the island is hidden. Results are cached per root for FRESH, which absorbs bursts of
// snapshots. Nothing here logs a path, a branch name or any output.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;

/// All the git calls of one root share this deadline (ideas #5: 2–3 s).
pub const TIMEOUT: Duration = Duration::from_millis(2500);
/// A result younger than this is given again without running git.
pub const FRESH: Duration = Duration::from_millis(1500);
/// At most this many projects per call.
pub const MAX_ROOTS: usize = 16;
/// A git output past this is refused (a tree with a huge untracked list).
const MAX_OUTPUT: usize = 8 * 1024 * 1024;

/// An operation that stopped half-way and waits for the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Operation {
    Rebase,
    Merge,
    CherryPick,
    Revert,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitState {
    /// The checked-out branch; None when HEAD is detached.
    pub branch: Option<String>,
    /// The first 7 characters of HEAD's commit; None before the first commit.
    pub commit: Option<String>,
    /// The branch has an upstream (ahead / behind are counted against it).
    pub upstream: bool,
    pub ahead: u32,
    pub behind: u32,
    /// Tracked files with a staged or unstaged change (conflicts not included).
    pub changed: u32,
    pub untracked: u32,
    pub conflicts: u32,
    /// Lines added / removed in tracked files against HEAD (staged and unstaged).
    pub insertions: u32,
    pub deletions: u32,
    pub operation: Option<Operation>,
}

// ── Parsing ───────────────────────────────────────────────────────────────────

/// `git status --porcelain=v2 --branch -z`: the branch headers and one count per entry kind.
/// A rename / copy entry ("2 …") is followed by its original path, which is skipped.
pub fn parse_status(out: &[u8]) -> GitState {
    let mut state = GitState::default();
    let mut detached = false;
    let mut items = out.split(|b| *b == 0).filter(|item| !item.is_empty());
    while let Some(item) = items.next() {
        let line = String::from_utf8_lossy(item);
        if let Some(header) = line.strip_prefix("# ") {
            let (name, value) = header.split_once(' ').unwrap_or((header, ""));
            match name {
                "branch.oid" if value != "(initial)" => state.commit = Some(value.chars().take(7).collect()),
                "branch.head" if value == "(detached)" => detached = true,
                "branch.head" => state.branch = Some(value.to_string()),
                "branch.upstream" => state.upstream = true,
                "branch.ab" => {
                    for part in value.split(' ') {
                        if let Some(n) = part.strip_prefix('+') {
                            state.ahead = n.parse().unwrap_or(0);
                        } else if let Some(n) = part.strip_prefix('-') {
                            state.behind = n.parse().unwrap_or(0);
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        match item.first() {
            Some(b'1') => state.changed += 1,
            Some(b'2') => {
                state.changed += 1;
                items.next();
            }
            Some(b'u') => state.conflicts += 1,
            Some(b'?') => state.untracked += 1,
            _ => {}
        }
    }
    if detached {
        state.branch = None;
    }
    state
}

/// `git diff --numstat`: the added and removed lines summed; a binary file ("-\t-") counts none.
pub fn parse_numstat(out: &[u8]) -> (u32, u32) {
    let text = String::from_utf8_lossy(out);
    let mut added = 0u32;
    let mut removed = 0u32;
    for line in text.lines() {
        let mut fields = line.split('\t');
        let (Some(a), Some(d)) = (fields.next(), fields.next()) else { continue };
        added = added.saturating_add(a.parse().unwrap_or(0));
        removed = removed.saturating_add(d.parse().unwrap_or(0));
    }
    (added, removed)
}

/// The git directory named by a `.git` file (a worktree, a submodule): "gitdir: <path>".
pub fn gitdir_from_file(content: &str) -> Option<&str> {
    let path = content.lines().next()?.strip_prefix("gitdir:")?.trim();
    (!path.is_empty()).then_some(path)
}

/// The operation in progress, from the marker files git leaves in its directory.
pub fn operation(exists: impl Fn(&str) -> bool) -> Option<Operation> {
    if exists("rebase-merge") || exists("rebase-apply") {
        Some(Operation::Rebase)
    } else if exists("MERGE_HEAD") {
        Some(Operation::Merge)
    } else if exists("CHERRY_PICK_HEAD") {
        Some(Operation::CherryPick)
    } else if exists("REVERT_HEAD") {
        Some(Operation::Revert)
    } else {
        None
    }
}

/// The (project key, root) pairs asked for, in the order asked, each once, at most MAX_ROOTS.
/// Only projects of the snapshot: the island never names a folder itself.
pub fn pick_roots(projects: &[(String, String)], keys: &[String]) -> Vec<(String, String)> {
    let mut picked: Vec<(String, String)> = Vec::new();
    for key in keys {
        if picked.len() == MAX_ROOTS {
            break;
        }
        if picked.iter().any(|(k, _)| k == key) {
            continue;
        }
        if let Some((k, root)) = projects.iter().find(|(k, _)| k == key) {
            picked.push((k.clone(), root.clone()));
        }
    }
    picked
}

// ── Running git ───────────────────────────────────────────────────────────────

/// Arguments that keep a read from writing or running anything the repository configures.
const SAFE: [&str; 4] = ["--no-optional-locks", "-c", "core.fsmonitor=false", "--no-pager"];

/// Runs `git <SAFE> <args>` in `root` until `deadline`. Errors name no path and no output.
fn run(git: &Path, root: &Path, args: &[&str], deadline: Instant) -> Result<Vec<u8>, String> {
    let mut cmd = Command::new(git);
    cmd.args(SAFE)
        .args(args)
        .current_dir(root)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    crate::platform::no_console(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| format!("did not start ({:?})", e.kind()))?;
    let stdout = child.stdout.take().ok_or("no output pipe")?;
    let reader = std::thread::Builder::new()
        .name("git-state-out".into())
        .spawn(move || {
            let mut out = Vec::new();
            let read = stdout.take(MAX_OUTPUT as u64 + 1).read_to_end(&mut out);
            read.map(|_| out)
        })
        .map_err(|e| format!("reader did not start ({:?})", e.kind()))?;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Err("timed out".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Err(format!("wait failed ({:?})", e.kind()));
            }
        }
    };
    let out = reader
        .join()
        .map_err(|_| "reader failed".to_string())?
        .map_err(|e| format!("read failed ({:?})", e.kind()))?;
    if !status.success() {
        return Err(match status.code() {
            Some(code) => format!("exit code {code}"),
            None => "killed".into(),
        });
    }
    if out.len() > MAX_OUTPUT {
        return Err("output too large".into());
    }
    Ok(out)
}

/// The git directory of `root`: `.git` itself, or the one its `.git` file names. None without one.
fn git_dir(root: &Path) -> Option<PathBuf> {
    let dot = root.join(".git");
    let meta = std::fs::metadata(&dot).ok()?;
    if meta.is_dir() {
        return Some(dot);
    }
    if meta.len() > 4096 {
        return None;
    }
    let content = std::fs::read_to_string(&dot).ok()?;
    let named = PathBuf::from(gitdir_from_file(&content)?);
    Some(if named.is_absolute() { named } else { root.join(named) })
}

/// The state of one repository root. Err when `root` is no repository or git failed.
pub fn read(git: &Path, root: &Path) -> Result<GitState, String> {
    read_within(git, root, TIMEOUT)
}

/// `read` with its own deadline: the island waits at most `TIMEOUT`; a test that runs beside hundreds
/// of others (each spawning processes) gives git far longer, so a busy machine is not a failure.
fn read_within(git: &Path, root: &Path, timeout: Duration) -> Result<GitState, String> {
    let dir = git_dir(root).ok_or("not a repository")?;
    let deadline = Instant::now() + timeout;
    let status = run(git, root, &["status", "--porcelain=v2", "--branch", "-z", "--untracked-files=normal"], deadline)?;
    let mut state = parse_status(&status);
    if state.commit.is_some() && state.changed + state.conflicts > 0 {
        let diff = run(
            git,
            root,
            &["diff", "HEAD", "--numstat", "--no-renames", "--no-ext-diff", "--no-textconv", "--no-color"],
            deadline,
        )?;
        (state.insertions, state.deletions) = parse_numstat(&diff);
    }
    state.operation = operation(|name| dir.join(name).exists());
    Ok(state)
}

// ── Cache and the command's body ──────────────────────────────────────────────

struct Entry {
    at: Instant,
    result: Result<GitState, String>,
}

static CACHE: Mutex<Option<HashMap<String, Entry>>> = Mutex::new(None);

/// The cached result of `root` when younger than FRESH.
fn cached(root: &str, now: Instant) -> Option<Result<GitState, String>> {
    let cache = CACHE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let entry = cache.as_ref()?.get(root)?;
    (now.duration_since(entry.at) < FRESH).then(|| entry.result.clone())
}

/// Stores a fresh result. A failure is logged only when it differs from the last one of that root,
/// without the path.
fn store(root: &str, result: Result<GitState, String>, now: Instant) {
    let mut cache = CACHE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let map = cache.get_or_insert_with(HashMap::new);
    if let Err(err) = &result {
        let same = matches!(map.get(root), Some(Entry { result: Err(old), .. }) if old == err);
        if !same {
            crate::log::line(format!("git state: a project could not be read ({err})"));
        }
    }
    map.insert(root.to_string(), Entry { at: now, result });
}

/// The state of each (key, root) whose root is a repository, keyed by project key. Each root runs on
/// its own thread. Without git on PATH: nothing (the island shows no git block).
pub fn read_all(roots: Vec<(String, String)>) -> HashMap<String, GitState> {
    let mut states = HashMap::new();
    let roots: Vec<(String, String)> = roots
        .into_iter()
        .filter(|(_, root)| Path::new(root).is_absolute() && crate::observe::paths::git_probe(&Path::new(root).join(".git")))
        .collect();
    if roots.is_empty() {
        return states;
    }
    let Some(git) = crate::platform::find_on_path("git") else { return states };
    let results: Vec<(String, Result<GitState, String>)> = std::thread::scope(|scope| {
        let handles: Vec<_> = roots
            .iter()
            .map(|(key, root)| {
                let git = &git;
                scope.spawn(move || {
                    let now = Instant::now();
                    let result = match cached(root, now) {
                        Some(result) => result,
                        None => {
                            let result = read(git, Path::new(root));
                            store(root, result.clone(), Instant::now());
                            result
                        }
                    };
                    (key.clone(), result)
                })
            })
            .collect();
        handles.into_iter().filter_map(|h| h.join().ok()).collect()
    });
    for (key, result) in results {
        if let Ok(state) = result {
            states.insert(key, state);
        }
    }
    states
}

#[cfg(test)]
mod tests {
    use super::*;

    fn z(items: &[&str]) -> Vec<u8> {
        let mut out = Vec::new();
        for item in items {
            out.extend_from_slice(item.as_bytes());
            out.push(0);
        }
        out
    }

    const OID: &str = "0123456789abcdef0123456789abcdef01234567";

    #[test]
    fn a_clean_tree_on_a_branch_with_an_upstream() {
        let s = parse_status(&z(&[
            &format!("# branch.oid {OID}"),
            "# branch.head main",
            "# branch.upstream origin/main",
            "# branch.ab +0 -0",
        ]));
        assert_eq!(
            s,
            GitState { branch: Some("main".into()), commit: Some("0123456".into()), upstream: true, ..Default::default() }
        );
    }

    #[test]
    fn a_dirty_tree_counts_each_kind_and_skips_the_original_path_of_a_rename() {
        let s = parse_status(&z(&[
            &format!("# branch.oid {OID}"),
            "# branch.head feature/x",
            "# branch.upstream origin/feature/x",
            "# branch.ab +3 -12",
            "1 .M N... 100644 100644 100644 aaaa bbbb src/a.rs",
            "1 A. N... 000000 100644 100644 0000 cccc src/b.rs",
            "2 R. N... 100644 100644 100644 dddd dddd R100 src/new.rs",
            "1 looks-like-an-entry but is the original path",
            "? notes.txt",
            "? tmp/",
            "! target/",
        ]));
        assert_eq!(s.branch.as_deref(), Some("feature/x"));
        assert_eq!((s.ahead, s.behind), (3, 12));
        assert_eq!((s.changed, s.untracked, s.conflicts), (3, 2, 0));
    }

    #[test]
    fn a_detached_head_has_no_branch_but_its_commit() {
        let s = parse_status(&z(&[&format!("# branch.oid {OID}"), "# branch.head (detached)"]));
        assert_eq!(s.branch, None);
        assert_eq!(s.commit.as_deref(), Some("0123456"));
        assert!(!s.upstream);
    }

    #[test]
    fn a_conflicted_tree_counts_unmerged_entries_apart() {
        let s = parse_status(&z(&[
            &format!("# branch.oid {OID}"),
            "# branch.head main",
            "u UU N... 100644 100644 100644 100644 a b c src/lib.rs",
            "1 M. N... 100644 100644 100644 aaaa bbbb src/ok.rs",
        ]));
        assert_eq!((s.changed, s.conflicts), (1, 1));
    }

    #[test]
    fn a_repository_without_commits_has_no_commit() {
        let s = parse_status(&z(&["# branch.oid (initial)", "# branch.head main", "? a.txt"]));
        assert_eq!(s.commit, None);
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert_eq!(s.untracked, 1);
    }

    #[test]
    fn numstat_sums_lines_and_counts_binaries_as_none() {
        let out = b"10\t2\tsrc/a.rs\n-\t-\tlogo.png\n0\t7\t\"odd\\tname.rs\"\n";
        assert_eq!(parse_numstat(out), (10, 9));
        assert_eq!(parse_numstat(b""), (0, 0));
    }

    #[test]
    fn a_git_file_names_its_directory() {
        assert_eq!(gitdir_from_file("gitdir: C:/repo/.git/worktrees/x\n"), Some("C:/repo/.git/worktrees/x"));
        assert_eq!(gitdir_from_file("gitdir: ../.git/modules/lib"), Some("../.git/modules/lib"));
        assert_eq!(gitdir_from_file("something else"), None);
        assert_eq!(gitdir_from_file("gitdir:   "), None);
    }

    #[test]
    fn the_operation_in_progress_comes_from_its_marker() {
        let only = |names: &'static [&'static str]| move |n: &str| names.contains(&n);
        assert_eq!(operation(only(&[])), None);
        assert_eq!(operation(only(&["MERGE_HEAD"])), Some(Operation::Merge));
        assert_eq!(operation(only(&["rebase-merge"])), Some(Operation::Rebase));
        assert_eq!(operation(only(&["rebase-apply"])), Some(Operation::Rebase));
        assert_eq!(operation(only(&["CHERRY_PICK_HEAD"])), Some(Operation::CherryPick));
        assert_eq!(operation(only(&["REVERT_HEAD"])), Some(Operation::Revert));
        // A rebase that stopped on a merge commit leaves both: the rebase is what goes on.
        assert_eq!(operation(only(&["MERGE_HEAD", "rebase-merge"])), Some(Operation::Rebase));
    }

    #[test]
    fn only_projects_of_the_snapshot_are_read_each_once() {
        let projects = vec![("a".to_string(), r"C:\a".to_string()), ("b".to_string(), r"C:\b".to_string())];
        let keys = ["b", "zzz", "a", "b"].map(String::from);
        assert_eq!(
            pick_roots(&projects, &keys),
            vec![("b".to_string(), r"C:\b".to_string()), ("a".to_string(), r"C:\a".to_string())]
        );
        let many: Vec<(String, String)> = (0..40).map(|i| (format!("k{i}"), format!(r"C:\p{i}"))).collect();
        let keys: Vec<String> = many.iter().map(|(k, _)| k.clone()).collect();
        assert_eq!(pick_roots(&many, &keys).len(), MAX_ROOTS);
    }

    #[test]
    fn the_wire_names_are_camel_case() {
        let s = GitState { operation: Some(Operation::CherryPick), ..Default::default() };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["operation"], "cherryPick");
        assert!(v.get("insertions").is_some() && v.get("branch").unwrap().is_null());
    }

    /// How long the real-git test lets one read take: far beyond `TIMEOUT`, so load never fails it.
    const PATIENT: Duration = Duration::from_secs(60);

    /// Runs git for a test fixture; panics on failure.
    fn git_ok(git: &Path, dir: &Path, args: &[&str]) {
        let mut cmd = Command::new(git);
        cmd.args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "init.defaultBranch=main", "-c", "commit.gpgsign=false"])
            .args(args)
            .current_dir(dir)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        assert!(cmd.status().unwrap().success(), "git {args:?}");
    }

    /// The real git on this machine, on clean, dirty, conflicted and detached trees. Without git on
    /// PATH the test says so and checks nothing more (the parsers above still run).
    #[test]
    fn real_git_reads_clean_dirty_conflicted_and_detached_trees() {
        let Some(git) = crate::platform::find_on_path("git") else {
            eprintln!("git not on PATH: real-git test skipped");
            return;
        };
        let tmp = crate::testutil::TempDir::new("git-state");
        let repo = tmp.path.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git_ok(&git, &repo, &["init", "-q"]);
        std::fs::write(repo.join("a.txt"), "one\ntwo\n").unwrap();
        git_ok(&git, &repo, &["add", "a.txt"]);
        git_ok(&git, &repo, &["commit", "-q", "-m", "first"]);

        let clean = read_within(&git, &repo, PATIENT).unwrap();
        assert_eq!(clean.branch.as_deref(), Some("main"));
        assert_eq!((clean.changed, clean.untracked, clean.conflicts, clean.operation), (0, 0, 0, None));

        std::fs::write(repo.join("a.txt"), "one\nTWO\nthree\n").unwrap();
        std::fs::write(repo.join("b.txt"), "new\n").unwrap();
        let dirty = read_within(&git, &repo, PATIENT).unwrap();
        assert_eq!((dirty.changed, dirty.untracked), (1, 1));
        assert_eq!((dirty.insertions, dirty.deletions), (2, 1));

        // A conflict: the same line changed on two branches, then merged.
        git_ok(&git, &repo, &["commit", "-q", "-am", "main side"]);
        git_ok(&git, &repo, &["checkout", "-q", "-b", "other", "HEAD~1"]);
        std::fs::write(repo.join("a.txt"), "one\nOTHER\n").unwrap();
        git_ok(&git, &repo, &["commit", "-q", "-am", "other side"]);
        git_ok(&git, &repo, &["checkout", "-q", "main"]);
        let mut merge = Command::new(&git);
        merge.args(["-c", "user.name=t", "-c", "user.email=t@t", "merge", "other"]).current_dir(&repo);
        assert!(!merge.stdout(Stdio::null()).stderr(Stdio::null()).status().unwrap().success());
        let conflicted = read_within(&git, &repo, PATIENT).unwrap();
        assert_eq!(conflicted.conflicts, 1);
        assert_eq!(conflicted.operation, Some(Operation::Merge));
        git_ok(&git, &repo, &["merge", "--abort"]);

        git_ok(&git, &repo, &["checkout", "-q", "--detach", "HEAD~1"]);
        let detached = read_within(&git, &repo, PATIENT).unwrap();
        assert_eq!(detached.branch, None);
        assert_eq!(detached.commit.as_ref().map(String::len), Some(7));

        // Not a repository at all.
        let plain = tmp.path.join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        assert!(read_within(&git, &plain, PATIENT).is_err());
    }
}
