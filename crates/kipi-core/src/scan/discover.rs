//! Finding git checkouts under a directory.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::error::{Error, Result};

/// Directories never descended into: dependency and build trees.
pub const SKIPPED_DIRS: &[&str] = &["node_modules", "target", ".venv"];

/// Every directory under `root` (itself included) holding a `.git` *directory*, in
/// file-name order. Found checkouts are not descended into. Hidden directories and
/// [`SKIPPED_DIRS`] are skipped, and so is a directory whose `.git` is a file (a
/// worktree or submodule: it belongs to a checkout found elsewhere). Symlinks are not
/// followed; unreadable directories below `root` are skipped.
pub fn find_checkouts(root: &Path) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    let mut walk = WalkDir::new(root)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter();
    while let Some(entry) = walk.next() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) if e.depth() == 0 => {
                let io = e
                    .into_io_error()
                    .unwrap_or_else(|| std::io::Error::other("cannot walk"));
                return Err(Error::io(root, io, "check that the directory is readable"));
            }
            Err(_) => continue,
        };
        if !entry.file_type().is_dir() {
            continue;
        }
        if entry.depth() > 0 && skipped(entry.file_name()) {
            walk.skip_current_dir();
            continue;
        }
        let git = entry.path().join(".git");
        if git.is_dir() {
            found.push(entry.into_path());
            walk.skip_current_dir();
        } else if git.exists() {
            walk.skip_current_dir();
        }
    }
    Ok(found)
}

fn skipped(name: &OsStr) -> bool {
    let name = name.to_string_lossy();
    name.starts_with('.') || SKIPPED_DIRS.contains(&name.as_ref())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn finds_checkouts_and_skips_noise() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        for dir in [
            "a/.git",
            "a/nested/.git",
            "group/b/.git",
            "node_modules/dep/.git",
            "x/target/t/.git",
            "x/.venv/v/.git",
            ".hidden/h/.git",
            "plain/sub",
        ] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        fs::create_dir_all(root.join("wt")).unwrap();
        fs::write(root.join("wt/.git"), "gitdir: elsewhere\n").unwrap();
        fs::create_dir_all(root.join("wt/inner/.git")).unwrap();
        let found = find_checkouts(root).unwrap();
        assert_eq!(found, [root.join("a"), root.join("group/b")]);
    }

    #[test]
    fn a_root_that_is_a_checkout_is_found_itself() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join(".git")).unwrap();
        fs::create_dir_all(tmp.path().join("sub/.git")).unwrap();
        assert_eq!(find_checkouts(tmp.path()).unwrap(), [tmp.path()]);
    }
}
