//! `kipi hook install|uninstall`: the git `post-checkout` hook that syncs a new working
//! tree. `git worktree add` checks the new tree out from the all-zero commit id, and the
//! hook then runs `kipi sync --hook` there ([`crate::templates::POST_CHECKOUT`]).
//!
//! Hooks live in the repository's common git directory, so one install covers the
//! checkout and every linked worktree. A hook that was already there is kept, renamed to
//! [`CHAINED`], and run first; uninstalling renames it back, byte for byte. kipi's own
//! hook is recognized by its [`MARKER`] line, so installing twice changes nothing.

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::apply::write_atomic;
use crate::error::{Error, Location, Result};
use crate::git;
use crate::templates::POST_CHECKOUT;

/// The hook's file name.
pub const HOOK: &str = "post-checkout";
/// Where a hook kipi's replaced is kept, beside kipi's.
pub const CHAINED: &str = "post-checkout.pre-kipi";
/// The line marking the hook as kipi's.
pub const MARKER: &str = "# kipi:post-checkout";

/// What [`install`] did (or, in a dry run, would do).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Installed {
    /// No hook was there; kipi's is.
    Created,
    /// Another hook was there: kept as [`CHAINED`], and kipi's runs it first.
    Chained,
    /// An older version of kipi's hook was rewritten.
    Updated,
    /// kipi's hook was already there.
    Unchanged,
}

/// What [`uninstall`] did (or, in a dry run, would do).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Uninstalled {
    /// kipi's hook was removed.
    Removed,
    /// kipi's hook was removed and the hook it had replaced put back.
    Restored,
    /// kipi's hook was not there.
    NotInstalled,
}

/// The hooks directory of the checkout `repo`: `hooks/` in its common git directory.
/// An error, with a hint, when `core.hooksPath` points git elsewhere.
pub fn hooks_dir(repo: &Path) -> Result<PathBuf> {
    let hooks_path = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["config", "--get", "core.hooksPath"])
        .output()
        .map_err(|e| Error::io(repo, e, "install git and make sure it is on PATH"))?;
    if hooks_path.status.success() {
        let dir = String::from_utf8_lossy(&hooks_path.stdout)
            .trim()
            .to_owned();
        return Err(Error::invalid(
            Location::file(repo),
            format!("git runs hooks from `core.hooksPath` ({dir}), shared beyond this repo"),
            format!(
                "have that hook run `kipi sync --hook` when its first argument is all zeros \
                 (see {HOOK} in kipi's templates/)"
            ),
        ));
    }
    match git::working_tree(repo) {
        Some((_, common)) => Ok(common.join("hooks")),
        None => Err(Error::invalid(
            Location::file(repo),
            "not a git checkout",
            "clone the repo there, or run `kipi scan` to record where it lives",
        )),
    }
}

/// Install kipi's `post-checkout` hook in the checkout `repo`, chaining any hook already
/// there. With `dry_run`, only say what would happen.
pub fn install(repo: &Path, dry_run: bool) -> Result<Installed> {
    let dir = hooks_dir(repo)?;
    let (hook, chained) = (dir.join(HOOK), dir.join(CHAINED));
    let done = match read_hook(&hook)? {
        Some(Hook::Kipi(text)) if text == POST_CHECKOUT => return Ok(Installed::Unchanged),
        Some(Hook::Kipi(_)) => Installed::Updated,
        Some(Hook::Other) if exists(&chained) => {
            return Err(Error::invalid(
                Location::file(&hook),
                format!("{HOOK} is not kipi's, and {CHAINED} is taken too"),
                "merge or move one of them, then re-run `kipi hook install`",
            ));
        }
        Some(Hook::Other) => Installed::Chained,
        None => Installed::Created,
    };
    if dry_run {
        return Ok(done);
    }
    let hint = "check that the git directory is writable";
    fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e, hint))?;
    if done == Installed::Chained {
        fs::rename(&hook, &chained).map_err(|e| Error::io(&hook, e, hint))?;
    }
    write_atomic(&hook, POST_CHECKOUT)?;
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755))
        .map_err(|e| Error::io(&hook, e, hint))?;
    Ok(done)
}

/// Remove kipi's `post-checkout` hook from the checkout `repo`, putting back the hook it
/// had replaced. With `dry_run`, only say what would happen.
pub fn uninstall(repo: &Path, dry_run: bool) -> Result<Uninstalled> {
    let dir = hooks_dir(repo)?;
    let (hook, chained) = (dir.join(HOOK), dir.join(CHAINED));
    let ours = matches!(read_hook(&hook)?, Some(Hook::Kipi(_)));
    let restore = exists(&chained) && (ours || !exists(&hook));
    if !ours && !restore {
        return Ok(Uninstalled::NotInstalled);
    }
    let hint = "check that the git directory is writable";
    if !dry_run {
        if ours {
            fs::remove_file(&hook).map_err(|e| Error::io(&hook, e, hint))?;
        }
        if restore {
            fs::rename(&chained, &hook).map_err(|e| Error::io(&chained, e, hint))?;
        }
    }
    Ok(if restore {
        Uninstalled::Restored
    } else {
        Uninstalled::Removed
    })
}

/// What stands at a hook path.
enum Hook {
    /// kipi's hook, with its text.
    Kipi(String),
    /// Anything else: another hook, a symlink, a directory.
    Other,
}

fn read_hook(path: &Path) -> Result<Option<Hook>> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Error::io(path, e, "check the permissions of the path")),
    };
    if !meta.is_file() {
        return Ok(Some(Hook::Other));
    }
    let text = fs::read(path).map_err(|e| Error::io(path, e, "check that the file is readable"))?;
    Ok(Some(match String::from_utf8(text) {
        Ok(text) if text.lines().any(|l| l.starts_with(MARKER)) => Hook::Kipi(text),
        _ => Hook::Other,
    }))
}

fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("r");
        fs::create_dir(&repo).unwrap();
        let ok = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["init", "--quiet"])
            .status()
            .unwrap()
            .success();
        assert!(ok);
        (dir, repo)
    }

    #[test]
    fn install_is_idempotent_and_uninstall_removes_it() {
        let (_dir, repo) = repo();
        let hook = hooks_dir(&repo).unwrap().join(HOOK);
        assert_eq!(install(&repo, true).unwrap(), Installed::Created);
        assert!(!hook.exists(), "a dry run writes nothing");
        assert_eq!(install(&repo, false).unwrap(), Installed::Created);
        assert_eq!(fs::read_to_string(&hook).unwrap(), POST_CHECKOUT);
        assert_eq!(
            fs::metadata(&hook).unwrap().permissions().mode() & 0o777,
            0o755
        );
        assert_eq!(install(&repo, false).unwrap(), Installed::Unchanged);
        fs::write(&hook, format!("{MARKER} (old)\n")).unwrap();
        assert_eq!(install(&repo, false).unwrap(), Installed::Updated);
        assert_eq!(uninstall(&repo, false).unwrap(), Uninstalled::Removed);
        assert!(!hook.exists());
        assert_eq!(uninstall(&repo, false).unwrap(), Uninstalled::NotInstalled);
    }

    #[test]
    fn a_hook_already_there_is_chained_and_restored_byte_for_byte() {
        let (_dir, repo) = repo();
        let dir = hooks_dir(&repo).unwrap();
        fs::create_dir_all(&dir).unwrap();
        let (hook, chained) = (dir.join(HOOK), dir.join(CHAINED));
        let theirs = b"#!/bin/sh\necho theirs \xff\n";
        fs::write(&hook, theirs).unwrap();
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(install(&repo, false).unwrap(), Installed::Chained);
        assert_eq!(fs::read(&chained).unwrap(), theirs);
        assert_eq!(install(&repo, false).unwrap(), Installed::Unchanged);
        assert_eq!(uninstall(&repo, false).unwrap(), Uninstalled::Restored);
        assert_eq!(fs::read(&hook).unwrap(), theirs);
        assert_eq!(
            fs::metadata(&hook).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert!(!chained.exists());
    }

    #[test]
    fn refusals_carry_hints() {
        let (_dir, repo) = repo();
        let dir = hooks_dir(&repo).unwrap();
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(HOOK), "#!/bin/sh\n").unwrap();
        fs::write(dir.join(CHAINED), "#!/bin/sh\n").unwrap();
        let err = install(&repo, false).unwrap_err();
        assert!(err.to_string().contains("is taken too"), "{err}");
        assert!(err.hint().contains("merge or move"));

        let ok = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["config", "core.hooksPath", "/elsewhere"])
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let err = install(&repo, false).unwrap_err();
        assert!(err.to_string().contains("core.hooksPath"), "{err}");
        assert!(err.hint().contains("kipi sync --hook"));
    }
}
