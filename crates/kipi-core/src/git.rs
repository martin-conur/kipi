//! Thin wrapper over the `git` binary (no libgit2).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{Error, Result};

/// The raw `remote.origin.url` of the checkout at `repo`, or `None` when it has no
/// `origin` remote.
pub fn origin_url(repo: &Path) -> Result<Option<String>> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["config", "--get", "remote.origin.url"])
        .output()
        .map_err(|e| Error::io(repo, e, "install git and make sure it is on PATH"))?;
    match out.status.code() {
        Some(0) => Ok(Some(String::from_utf8_lossy(&out.stdout).trim().to_owned())),
        // `git config --get` exits 1 when the key is unset.
        Some(1) => Ok(None),
        _ => Err(Error::io(
            repo,
            std::io::Error::other(String::from_utf8_lossy(&out.stderr).trim().to_owned()),
            "check that the directory is a readable git checkout",
        )),
    }
}

/// Which of `paths` (absolute, inside `repo`) git tracks in the checkout at `repo`.
pub fn tracked(repo: &Path, paths: &[PathBuf]) -> Result<BTreeSet<PathBuf>> {
    let rel: Vec<&Path> = paths
        .iter()
        .filter_map(|p| p.strip_prefix(repo).ok())
        .collect();
    if rel.is_empty() {
        return Ok(BTreeSet::new());
    }
    let out = run(
        repo,
        Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["ls-files", "-z", "--"])
            .args(&rel),
    )?;
    Ok(out
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(|p| repo.join(p))
        .collect())
}

/// The checkout's `info/exclude` file: `.git/info/exclude`, or wherever git keeps it
/// for a worktree or a submodule (a `.git` file).
pub fn info_exclude(repo: &Path) -> Result<PathBuf> {
    let dot_git = repo.join(".git");
    if dot_git.is_dir() {
        return Ok(dot_git.join("info").join("exclude"));
    }
    let out = run(
        repo,
        Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["rev-parse", "--git-path", "info/exclude"]),
    )?;
    Ok(repo.join(out.trim()))
}

/// `git init` the directory `dir` (it must exist).
pub fn init(dir: &Path) -> Result<()> {
    run(
        dir,
        Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["init", "--quiet"]),
    )
    .map(drop)
}

/// Shallow-clone `url` into `dest` (which must not exist yet).
pub fn clone(url: &str, dest: &Path) -> Result<()> {
    let out = Command::new("git")
        .args(["clone", "--quiet", "--depth", "1", "--", url])
        .arg(dest)
        .output()
        .map_err(|e| Error::io(dest, e, "install git and make sure it is on PATH"))?;
    if !out.status.success() {
        return Err(Error::io(
            dest,
            std::io::Error::other(format!(
                "git clone {url} failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )),
            "check the URL and that git can reach it (try `git clone` by hand)",
        ));
    }
    Ok(())
}

/// The git state of a directory, for `kipi stores`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirStatus {
    /// No uncommitted change under the directory.
    pub clean: bool,
    /// The repository has at least one remote.
    pub has_remote: bool,
}

/// The git state of `dir`, or `None` when it is not inside a git work tree.
pub fn status(dir: &Path) -> Option<DirStatus> {
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    };
    let changes = git(&["status", "--porcelain", "--", "."])?;
    let remotes = git(&["remote"]).unwrap_or_default();
    Some(DirStatus {
        clean: changes.trim().is_empty(),
        has_remote: !remotes.trim().is_empty(),
    })
}

/// Run a git command, returning its stdout; a non-zero exit is an I/O error.
fn run(repo: &Path, cmd: &mut Command) -> Result<String> {
    let out = cmd
        .output()
        .map_err(|e| Error::io(repo, e, "install git and make sure it is on PATH"))?;
    if !out.status.success() {
        return Err(Error::io(
            repo,
            std::io::Error::other(String::from_utf8_lossy(&out.stderr).trim().to_owned()),
            "check that the directory is a readable git checkout",
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    }

    #[test]
    fn tracked_lists_only_files_in_the_index() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        git(repo, &["init", "--quiet"]);
        std::fs::write(repo.join("CLAUDE.md"), "x").unwrap();
        std::fs::write(repo.join("AGENTS.md"), "x").unwrap();
        git(repo, &["add", "CLAUDE.md"]);
        let asked = [
            repo.join("CLAUDE.md"),
            repo.join("AGENTS.md"),
            repo.join("CLAUDE.local.md"),
        ];
        let tracked = tracked(repo, &asked).unwrap();
        assert_eq!(tracked, BTreeSet::from([repo.join("CLAUDE.md")]));
        assert!(super::tracked(repo, &[]).unwrap().is_empty());
    }

    #[test]
    fn info_exclude_of_a_checkout_and_of_a_worktree() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("main");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "--quiet"]);
        assert_eq!(info_exclude(&repo).unwrap(), repo.join(".git/info/exclude"));
        git(
            &repo,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "commit",
                "--quiet",
                "--allow-empty",
                "-m",
                "init",
            ],
        );
        let wt = dir.path().join("wt");
        git(&repo, &["worktree", "add", "--quiet", wt.to_str().unwrap()]);
        let exclude = info_exclude(&wt).unwrap();
        assert!(exclude.ends_with("info/exclude"), "{}", exclude.display());
        assert!(
            !exclude.starts_with(&wt),
            "a worktree shares the main exclude file"
        );
    }
}
