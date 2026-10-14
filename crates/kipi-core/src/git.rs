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
///
/// Names compare case-insensitively: a tracked `CLAUDE.MD` or `claude.md` counts as a
/// tracked `CLAUDE.md`. On a case-insensitive filesystem (macOS, Windows) those are one
/// file, so writing "`CLAUDE.md`" would dirty the tracked one (#60); on a case-sensitive
/// one the answer only errs toward leaving the file alone.
pub fn tracked(repo: &Path, paths: &[PathBuf]) -> Result<BTreeSet<PathBuf>> {
    let rel: Vec<(&PathBuf, String)> = paths
        .iter()
        .filter_map(|p| Some((p, p.strip_prefix(repo).ok()?.to_str()?.to_owned())))
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
            .args(rel.iter().map(|(_, r)| format!(":(icase){r}"))),
    )?;
    let listed: BTreeSet<String> = out
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(str::to_lowercase)
        .collect();
    Ok(rel
        .into_iter()
        .filter(|(_, r)| listed.contains(&r.to_lowercase()))
        .map(|(p, _)| p.clone())
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
/// 2: answers became case-insensitive (#60), so version-1 answers are dropped.
const TRACKED_VERSION: u32 = 2;

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

/// The linked worktrees of the checkout at `repo` (`git worktree add`), as `git worktree
/// list` names them: not the main working tree, not `repo` itself, and none whose
/// directory is gone (a worktree deleted without `git worktree remove`). A checkout with
/// a `.git` directory holding no `worktrees/` is answered without starting `git`; a
/// directory that is not a git checkout has none.
pub fn worktrees(repo: &Path) -> Result<Vec<PathBuf>> {
    let dot_git = repo.join(".git");
    if !dot_git.exists() || (dot_git.is_dir() && !dot_git.join("worktrees").is_dir()) {
        return Ok(Vec::new());
    }
    let out = run(
        repo,
        Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["worktree", "list", "--porcelain", "-z"]),
    )?;
    let real = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let this = real(repo);
    // Records are NUL-separated lines, each worktree ending with an empty line; the main
    // working tree comes first.
    let mut found = Vec::new();
    for record in out.split("\0\0").skip(1) {
        let mut lines = record.split('\0');
        let Some(path) = lines.next().and_then(|l| l.strip_prefix("worktree ")) else {
            continue;
        };
        let path = PathBuf::from(path);
        let unusable = lines.any(|l| l == "bare" || l.starts_with("prunable"));
        if !unusable && path.is_dir() && real(&path) != this {
            found.push(path);
        }
    }
    Ok(found)
}

/// The working tree holding `dir` and its repository's common git directory (the main
/// checkout's `.git`, shared by every linked worktree), both canonical; `None` when `dir`
/// is not inside a git working tree. A directory with no `.git` in it or any ancestor is
/// answered without starting `git`.
pub fn working_tree(dir: &Path) -> Option<(PathBuf, PathBuf)> {
    if !dir
        .ancestors()
        .any(|d| fs::symlink_metadata(d.join(".git")).is_ok())
    {
        return None;
    }
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "rev-parse",
            "--path-format=absolute",
            "--show-toplevel",
            "--git-common-dir",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines = text.lines();
    let top = Path::new(lines.next()?).canonicalize().ok()?;
    let common = Path::new(lines.next()?).canonicalize().ok()?;
    Some((top, common))
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
    clone_with(url, dest, &["--depth", "1"])
}

/// Clone `url` into `dest` with its full history (a store is pulled and pushed later).
/// `dest` must not exist yet, or be an empty directory.
pub fn clone_full(url: &str, dest: &Path) -> Result<()> {
    clone_with(url, dest, &[])
}

fn clone_with(url: &str, dest: &Path, extra: &[&str]) -> Result<()> {
    let out = Command::new("git")
        .args(["clone", "--quiet"])
        .args(extra)
        .args(["--", url])
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

/// The git state of a directory, for `kipi stores` and `kipi tree`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirStatus {
    /// No uncommitted change under the directory.
    pub clean: bool,
    /// The repository has at least one remote.
    pub has_remote: bool,
    /// The current branch's upstream, as of the last fetch; `None` when the branch has
    /// none (or HEAD is detached, or the upstream branch does not exist yet).
    pub upstream: Option<Tracking>,
}

/// How far a branch is from its upstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Tracking {
    /// Commits on the branch that the upstream lacks.
    pub ahead: u32,
    /// Commits on the upstream that the branch lacks.
    pub behind: u32,
}

/// The git state of `dir`, or `None` when it is not inside a git work tree. A directory
/// with no `.git` in it or any ancestor is answered without starting `git`. Nothing is
/// fetched: ahead and behind are as of the last fetch.
pub fn status(dir: &Path) -> Option<DirStatus> {
    if !in_work_tree(dir) {
        return None;
    }
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
    let text = git(&["status", "--porcelain=v2", "--branch", "--", "."])?;
    let remotes = git(&["remote"]).unwrap_or_default();
    let (mut has_upstream, mut tracking) = (false, None);
    let mut clean = true;
    for line in text.lines() {
        if let Some(header) = line.strip_prefix("# ") {
            if header.starts_with("branch.upstream ") {
                has_upstream = true;
            } else if let Some(ab) = header.strip_prefix("branch.ab ") {
                tracking = parse_ab(ab);
            }
        } else if !line.is_empty() {
            clean = false;
        }
    }
    Some(DirStatus {
        clean,
        has_remote: !remotes.trim().is_empty(),
        upstream: tracking.filter(|_| has_upstream),
    })
}

/// `+3 -2` (porcelain v2 `branch.ab`) as ahead 3, behind 2.
fn parse_ab(ab: &str) -> Option<Tracking> {
    let (ahead, behind) = ab.split_once(' ')?;
    Some(Tracking {
        ahead: ahead.strip_prefix('+')?.parse().ok()?,
        behind: behind.strip_prefix('-')?.parse().ok()?,
    })
}

/// Whether `dir` or an ancestor holds a `.git` (a cheap test before starting `git`).
fn in_work_tree(dir: &Path) -> bool {
    dir.ancestors()
        .any(|d| fs::symlink_metadata(d.join(".git")).is_ok())
}

/// `git fetch` the current branch's remote in the repository holding `dir`.
pub fn fetch(dir: &Path) -> Result<()> {
    run_hinting(
        dir,
        &["fetch", "--quiet"],
        "check that the store's remote is reachable (try `git fetch` in it by hand)",
    )
    .map(drop)
}

/// Fast-forward the current branch to its upstream (`git merge --ff-only @{u}`).
pub fn fast_forward(dir: &Path) -> Result<()> {
    run_hinting(
        dir,
        &["merge", "--ff-only", "--quiet", "@{u}"],
        "commit or stash the changes git names above, then pull again",
    )
    .map(drop)
}

/// The paths under `dir` (relative to it, `/`-separated) with an uncommitted change,
/// untracked ones included, `generated/` left out.
pub fn changed_paths(dir: &Path) -> Result<Vec<String>> {
    let out = run_hinting(
        dir,
        &[
            "-c",
            "status.relativePaths=true",
            "status",
            "--porcelain=v1",
            "-z",
            "--no-renames",
            "--untracked-files=all",
            "--",
            ".",
            STORE_EXCLUDE,
        ],
        "check that the store is a readable git checkout",
    )?;
    Ok(out
        .split('\0')
        .filter(|entry| entry.len() > 3)
        .map(|entry| entry[3..].to_owned())
        .collect())
}

/// The committed contents of `rel` (relative to `dir`) at `HEAD`; `None` when it is not
/// in `HEAD` (or there is no commit yet).
pub fn head_file(dir: &Path, rel: &str) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["show", &format!("HEAD:./{rel}")])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The pathspec that keeps a store's per-machine `generated/` out of every commit, even
/// when its `.gitignore` lost the line.
const STORE_EXCLUDE: &str = ":(exclude)generated";

/// Stage and commit every change under `dir` but `generated/`, with `message`. Changes
/// elsewhere in the repository (a store kept in a dotfiles repo) are neither staged nor
/// committed.
pub fn commit_dir(dir: &Path, message: &str) -> Result<()> {
    let hint = "check that the store is a writable git checkout";
    // An exclude pathspec naming an ignored directory makes `git add` fail, so stage
    // everything, then unstage `generated/` (staged only when no longer ignored).
    run_hinting(dir, &["add", "--all", "--", "."], hint)?;
    run_hinting(dir, &["reset", "--quiet", "--", "generated"], hint)?;
    run_hinting(
        dir,
        &["commit", "--quiet", "-m", message, "--", ".", STORE_EXCLUDE],
        "set your identity with `git config --global user.name` and `user.email`, then push again",
    )
    .map(drop)
}

/// `git push` the current branch to its upstream.
pub fn push(dir: &Path) -> Result<()> {
    run_hinting(
        dir,
        &["push", "--quiet"],
        "if the remote has commits you lack, run `kipi pull` first, then push again",
    )
    .map(drop)
}

/// The current branch's name; `None` on a detached HEAD.
pub fn branch(dir: &Path) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// `git -C <dir> <args>`, returning stdout; a failure is an I/O error carrying git's
/// message and `hint`.
fn run_hinting(dir: &Path, args: &[&str], hint: &str) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|e| Error::io(dir, e, "install git and make sure it is on PATH"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let why = if stderr.trim().is_empty() {
            stdout
        } else {
            stderr
        };
        return Err(Error::io(
            dir,
            std::io::Error::other(format!("git {} failed: {}", args[0], why.trim())),
            hint,
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
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
    fn tracked_ignores_case_in_file_names() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        git(repo, &["init", "--quiet"]);
        std::fs::write(repo.join("CLAUDE.MD"), "x").unwrap();
        std::fs::write(repo.join("agents.md"), "x").unwrap();
        git(repo, &["add", "CLAUDE.MD", "agents.md"]);
        let asked = [
            repo.join("CLAUDE.md"),
            repo.join("AGENTS.md"),
            repo.join("CLAUDE.local.md"),
        ];
        assert_eq!(
            tracked(repo, &asked).unwrap(),
            BTreeSet::from([repo.join("CLAUDE.md"), repo.join("AGENTS.md")]),
            "the asked names come back, matched against any case git tracks"
        );
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
    fn status_of_plain_and_git_directories() {
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("plain");
        std::fs::create_dir(&plain).unwrap();
        assert_eq!(status(&plain), None, "no .git anywhere above: no git run");
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "--quiet"]);
        let sub = repo.join("sub");
        std::fs::create_dir(&sub).unwrap();
        let clean = DirStatus {
            clean: true,
            has_remote: false,
            upstream: None,
        };
        assert_eq!(status(&sub), Some(clean), "inside a work tree");
        std::fs::write(sub.join("x"), "x").unwrap();
        assert!(!status(&sub).unwrap().clean);
    }

    #[test]
    fn branch_ab_parses_ahead_and_behind() {
        assert_eq!(
            parse_ab("+3 -2"),
            Some(Tracking {
                ahead: 3,
                behind: 2
            })
        );
        assert_eq!(parse_ab("+0 -0"), Some(Tracking::default()));
        assert_eq!(parse_ab("3 2"), None);
    }

    #[test]
    fn worktrees_lists_the_linked_ones_that_still_exist() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("main");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "--quiet"]);
        assert!(worktrees(&repo).unwrap().is_empty(), "no worktrees/ yet");
        assert!(worktrees(dir.path()).unwrap().is_empty(), "not a checkout");
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
        let (a, b) = (dir.path().join("a"), dir.path().join("b c"));
        for wt in [&a, &b] {
            git(
                &repo,
                &[
                    "worktree",
                    "add",
                    "--quiet",
                    "--detach",
                    wt.to_str().unwrap(),
                ],
            );
        }
        let real = |p: &Path| p.canonicalize().unwrap();
        let found: Vec<PathBuf> = worktrees(&repo).unwrap().iter().map(|p| real(p)).collect();
        assert_eq!(found, [real(&a), real(&b)]);
        assert_eq!(
            worktrees(&a).unwrap().len(),
            1,
            "from a worktree: the other one"
        );

        let (top, common) = working_tree(&b).unwrap();
        assert_eq!(top, real(&b));
        assert_eq!(common, real(&repo.join(".git")));
        assert_eq!(working_tree(dir.path()), None);

        std::fs::remove_dir_all(&b).unwrap();
        let found: Vec<PathBuf> = worktrees(&repo).unwrap().iter().map(|p| real(p)).collect();
        assert_eq!(found, [real(&a)], "a deleted worktree is left out");
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
