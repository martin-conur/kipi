//! Thin wrapper over the `git` binary (no libgit2).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

use crate::cache::{self, Stamp};
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

/// Remembers which files git tracks in each checkout, keyed by the stamp of its
/// `.git/index` (the file `git ls-files` reads), so `status` and `tree` do not start one
/// `git` per repo while no index changed. Saved in `<cache dir>/tracked.json`; the
/// default value remembers for this process only.
///
/// A checkout whose `.git` is a file (a worktree or a submodule) keeps its index
/// elsewhere and is always asked; one with no index yet tracks nothing.
#[derive(Debug, Default)]
pub struct TrackedCache {
    path: Option<PathBuf>,
    state: Mutex<TrackedState>,
}

#[derive(Debug, Default)]
struct TrackedState {
    file: TrackedFile,
    used: BTreeSet<String>,
    dirty: bool,
}

/// The cache file format. Bump `version` on a change; another version reads as empty.
const TRACKED_VERSION: u32 = 1;

#[derive(Debug, Default, Serialize, Deserialize)]
struct TrackedFile {
    version: u32,
    /// Checkout → what its index said.
    repos: BTreeMap<String, TrackedEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
struct TrackedEntry {
    /// The stamp of `.git/index` the answers hold for.
    index: Stamp,
    /// Path relative to the checkout → tracked.
    paths: BTreeMap<String, bool>,
}

impl TrackedCache {
    /// The cache in `cache_dir`, loading what is there.
    pub fn new(cache_dir: &Path) -> TrackedCache {
        let path = cache_dir.join("tracked.json");
        let file = cache::load(&path, |f: &TrackedFile| f.version, TRACKED_VERSION);
        TrackedCache {
            path: Some(path),
            state: Mutex::new(TrackedState {
                file,
                ..TrackedState::default()
            }),
        }
    }

    /// [`tracked`], answered from the cache while the checkout's index is unchanged.
    pub fn tracked(&self, repo: &Path, paths: &[PathBuf]) -> Result<BTreeSet<PathBuf>> {
        let dot_git = repo.join(".git");
        if !dot_git.is_dir() {
            return tracked(repo, paths);
        }
        let stamp = match fs::metadata(dot_git.join("index")) {
            Ok(meta) => cache::stamp(&meta),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(BTreeSet::new()),
            Err(_) => return tracked(repo, paths),
        };
        let rels: Vec<String> = paths
            .iter()
            .filter_map(|p| p.strip_prefix(repo).ok()?.to_str().map(str::to_owned))
            .collect();
        let Some(key) = repo.to_str().filter(|_| rels.len() == paths.len()) else {
            return tracked(repo, paths);
        };
        {
            let mut state = self.lock();
            if let Some(entry) = state.file.repos.get(key)
                && entry.index == stamp
                && rels.iter().all(|r| entry.paths.contains_key(r))
            {
                let found = paths
                    .iter()
                    .zip(&rels)
                    .filter(|(_, r)| entry.paths[*r])
                    .map(|(p, _)| p.clone())
                    .collect();
                state.used.insert(key.to_owned());
                return Ok(found);
            }
        }
        let found = tracked(repo, paths)?;
        let mut state = self.lock();
        let entry = state
            .file
            .repos
            .entry(key.to_owned())
            .or_insert_with(|| TrackedEntry {
                index: stamp,
                paths: BTreeMap::new(),
            });
        if entry.index != stamp {
            entry.index = stamp;
            entry.paths.clear();
        }
        for (p, r) in paths.iter().zip(rels) {
            entry.paths.insert(r, found.contains(p));
        }
        state.used.insert(key.to_owned());
        state.dirty = true;
        Ok(found)
    }

    /// Write the cache back if this run added to it.
    pub fn save(&self) -> Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let mut state = self.lock();
        if !state.dirty {
            return Ok(());
        }
        let TrackedState { file, used, .. } = &mut *state;
        cache::prune(&mut file.repos, used);
        file.version = TRACKED_VERSION;
        cache::store(path, &*file)?;
        state.dirty = false;
        Ok(())
    }

    fn lock(&self) -> MutexGuard<'_, TrackedState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
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
    fn the_tracked_cache_follows_the_index() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("r");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "--quiet"]);
        let (a, b) = (repo.join("CLAUDE.md"), repo.join("AGENTS.md"));
        std::fs::write(&a, "x").unwrap();
        std::fs::write(&b, "x").unwrap();
        let cache_dir = dir.path().join("cache");
        let cache = TrackedCache::new(&cache_dir);
        let asked = [a.clone(), b.clone()];
        assert!(
            cache.tracked(&repo, &asked).unwrap().is_empty(),
            "no index yet"
        );
        git(&repo, &["add", "CLAUDE.md"]);
        assert_eq!(
            cache.tracked(&repo, &asked).unwrap(),
            BTreeSet::from([a.clone()])
        );
        cache.save().unwrap();

        // A warm cache answers from the file: doctor it to prove git is not asked.
        let saved = cache_dir.join("tracked.json");
        let text = std::fs::read_to_string(&saved).unwrap();
        std::fs::write(
            &saved,
            text.replace("\"AGENTS.md\":false", "\"AGENTS.md\":true"),
        )
        .unwrap();
        let warm = TrackedCache::new(&cache_dir);
        assert_eq!(warm.tracked(&repo, &asked).unwrap().len(), 2);
        assert!(!warm.lock().dirty);

        // A changed index is asked again.
        git(&repo, &["rm", "--cached", "--quiet", "CLAUDE.md"]);
        assert!(warm.tracked(&repo, &asked).unwrap().is_empty());
        assert!(warm.lock().dirty);
        // A path not asked before is asked.
        let c = repo.join("CLAUDE.local.md");
        assert!(warm.tracked(&repo, &[c]).unwrap().is_empty());
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
