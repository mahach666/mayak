//! Claude Desktop is an MSIX package: the paths it reports for its own files (the `cwd` of a
//! scratch chat) are virtualised for the package. A process outside the package finds the same
//! folder under `%LOCALAPPDATA%\Packages\Claude_<publisher id>\LocalCache\Roaming\…`
//! (quick wins §3.2). Pure apart from the `Packages` listing and the `exists` probe it is given.

use std::path::{Component, Path, PathBuf};

use crate::launcher::{package_family, CLAUDE_PACKAGE};

/// The real folder of `path`, a path the package sees under `roaming` (%APPDATA%): the same
/// remainder under `localappdata\Packages\<Claude family>\LocalCache\Roaming`, when `exists` says
/// it is there. `None` when `path` is not strictly under `roaming` (compared by components,
/// without case), has a `..` or other non-name part after it, the Claude package is not
/// installed, or nothing exists at the mapped path.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn real_path(path: &Path, roaming: &Path, localappdata: &Path, exists: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    let rest = rest_under(path, roaming)?;
    let packages = localappdata.join("Packages");
    let family = package_family(&packages, CLAUDE_PACKAGE)?;
    let mapped = packages.join(family).join("LocalCache").join("Roaming").join(rest);
    exists(&mapped).then_some(mapped)
}

/// What follows `base` in `path`, `None` unless it is a non-empty run of plain names.
fn rest_under(path: &Path, base: &Path) -> Option<PathBuf> {
    let key = |c: Component| c.as_os_str().to_string_lossy().to_lowercase();
    let mut path_parts = path.components();
    for want in base.components() {
        if key(path_parts.next()?) != key(want) {
            return None;
        }
    }
    let rest: PathBuf = path_parts.clone().collect();
    let plain = path_parts.all(|c| matches!(c, Component::Normal(_)));
    (plain && !rest.as_os_str().is_empty()).then_some(rest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    const FAMILY: &str = "Claude_pzs8sxrjxfjjc";

    /// A machine with `Packages\Claude_…\LocalCache\Roaming\Claude\scratch\a` on disk:
    /// (temp dir, roaming, localappdata).
    fn machine() -> (TempDir, PathBuf, PathBuf) {
        let tmp = TempDir::new("msix");
        let roaming = tmp.path.join("Roaming");
        let local = tmp.path.join("Local");
        std::fs::create_dir_all(roaming.join("Mayak")).unwrap();
        std::fs::create_dir_all(local.join("Packages").join(FAMILY).join("LocalCache").join("Roaming").join("Claude").join("scratch").join("a")).unwrap();
        (tmp, roaming, local)
    }

    fn is_dir(p: &Path) -> bool {
        p.is_dir()
    }

    fn mapped(local: &Path, rest: &[&str]) -> PathBuf {
        let mut p = local.join("Packages").join(FAMILY).join("LocalCache").join("Roaming");
        p.extend(rest);
        p
    }

    #[test]
    fn a_path_under_roaming_maps_into_the_package_cache() {
        let (_tmp, roaming, local) = machine();
        let path = roaming.join("Claude").join("scratch").join("a");
        assert_eq!(real_path(&path, &roaming, &local, is_dir), Some(mapped(&local, &["Claude", "scratch", "a"])));
    }

    #[test]
    fn the_comparison_ignores_case() {
        let (_tmp, roaming, local) = machine();
        let upper = PathBuf::from(roaming.to_string_lossy().to_uppercase()).join("CLAUDE").join("scratch").join("a");
        // the remainder keeps the case it was reported with; the disk decides whether it exists
        let found = real_path(&upper, &roaming, &local, |_| true);
        assert_eq!(found, Some(mapped(&local, &["CLAUDE", "scratch", "a"])));
    }

    #[test]
    fn a_path_outside_roaming_or_equal_to_it_gives_none() {
        let (tmp, roaming, local) = machine();
        for path in [tmp.path.join("elsewhere").join("Claude"), tmp.path.join("RoamingX").join("Claude"), roaming.clone(), tmp.path.clone()] {
            assert_eq!(real_path(&path, &roaming, &local, |_| true), None, "{}", path.display());
        }
    }

    #[test]
    fn a_parent_part_never_escapes_the_cache() {
        let (_tmp, roaming, local) = machine();
        let path = roaming.join("Claude").join("..").join("..").join("Secrets");
        assert_eq!(real_path(&path, &roaming, &local, |_| true), None);
    }

    #[test]
    fn nothing_at_the_mapped_path_gives_none() {
        let (_tmp, roaming, local) = machine();
        let path = roaming.join("Claude").join("scratch").join("gone");
        assert_eq!(real_path(&path, &roaming, &local, is_dir), None);
    }

    #[test]
    fn without_the_claude_package_the_answer_is_none() {
        let (tmp, roaming, _) = machine();
        let bare = tmp.path.join("NoPackages");
        std::fs::create_dir_all(bare.join("Packages").join("OpenAI.Codex_2p2nqsd0c76g0")).unwrap();
        assert_eq!(real_path(&roaming.join("Claude"), &roaming, &bare, |_| true), None);
        assert_eq!(real_path(&roaming.join("Claude"), &roaming, &tmp.path.join("missing"), |_| true), None);
    }

    #[test]
    fn another_package_with_a_similar_name_is_not_claude() {
        let (tmp, roaming, _) = machine();
        let local = tmp.path.join("Other");
        std::fs::create_dir_all(local.join("Packages").join("ClaudeBeta_pzs8sxrjxfjjc")).unwrap();
        assert_eq!(real_path(&roaming.join("Claude"), &roaming, &local, |_| true), None);
    }
}
