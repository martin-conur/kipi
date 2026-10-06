//! The apply engine (roadmap §8 steps 4–6): the only code that changes consumer repos.
//!
//! [`run`] diffs the desired [`Action`]s against a target's manifest and its disk, then
//! changes the disk: anything the manifest lists but no action wants is removed, anything
//! on disk that kipi did not write is never touched (reported as foreign), and the new
//! manifest records what is kipi's now. With [`Options::dry_run`] it only reports.
//!
//! Safety rules: writes are atomic (temp file in the same directory, then rename),
//! symlink targets are absolute, deletion never follows a symlink, a regular file or
//! directory where a link goes is replaced only with [`Options::force`], nothing is
//! written through a symlinked parent directory, and only empty directories kipi itself
//! created are removed. [`Lock`] serializes whole syncs across processes.
//!
//! Read grants (`MergeGrant`) are merged into a settings file with [`json_merge`]: only
//! the entries the manifest records as kipi's are ever removed, and a file that is
//! malformed, tracked by git, or a symlink is reported and left untouched.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};

use crate::adapters;
use crate::error::{Error, Location, Result};
use crate::git::{self, TrackedCache};
use crate::json_merge::{self, Outcome};
use crate::manifest::{self, BlockEntry, ExcludeEntry, FileEntry, GrantEntry, LinkEntry, Manifest};
use crate::model::AgentId;
use crate::plan::{Action, Checkout, Registry};
use crate::render::{registry, splice};

/// A repo's manifest, relative to the checkout.
pub const MANIFEST: &str = ".kipi/manifest.toml";
/// A store's manifest for its generated files, relative to `generated/`.
pub const GENERATED_MANIFEST: &str = ".kipi-manifest.toml";
/// The global lock, relative to the config directory (`~/.config/kipi`).
pub const LOCK_FILE: &str = "lock";
/// The line opening kipi's section of `.git/info/exclude`.
pub const EXCLUDE_BEGIN: &str = "# kipi:begin (managed by `kipi sync`; do not edit)";
/// The line closing kipi's section of `.git/info/exclude`.
pub const EXCLUDE_END: &str = "# kipi:end";

const TMP_SUFFIX: &str = ".kipi-tmp";

/// Where [`run`] applies actions: everything it writes stays under `root`, and the
/// manifest at `manifest` records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// The directory every action path lives under.
    pub root: PathBuf,
    /// The manifest file.
    pub manifest: PathBuf,
}

impl Target {
    /// A consumer repo checkout; its manifest is [`MANIFEST`].
    pub fn repo(dir: &Path) -> Target {
        Target {
            root: dir.to_path_buf(),
            manifest: dir.join(MANIFEST),
        }
    }

    /// The `generated/` directory of the store at `store_dir`; its manifest is
    /// [`GENERATED_MANIFEST`].
    pub fn generated(store_dir: &Path) -> Target {
        let root = store_dir.join(registry::GENERATED_DIR);
        Target {
            manifest: root.join(GENERATED_MANIFEST),
            root,
        }
    }
}

/// How [`run`] behaves.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Options {
    /// Report what would change; touch nothing.
    pub dry_run: bool,
    /// Replace a regular file or directory standing where a link or an owned file goes.
    pub force: bool,
}

/// What [`run`] did (or, in a dry run, would do) to one target.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// The target root.
    pub root: PathBuf,
    /// Changes, in the order they were made.
    pub changes: Vec<Change>,
    /// Actions not carried out, each with what the user can do about it.
    pub skipped: Vec<Skipped>,
    /// Entries kipi did not write found beside what it manages (in skill directories).
    pub foreign: Vec<PathBuf>,
    /// Each settings file whose grant changed, with its text before and after (for
    /// `--dry-run` diffs).
    pub grants: Vec<GrantDiff>,
}

/// A change to the read grant in one settings file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantDiff {
    /// The settings file (absolute).
    pub path: PathBuf,
    /// Entries kipi added.
    pub added: Vec<String>,
    /// Entries kipi removed (only ever its own).
    pub removed: Vec<String>,
    /// The file's text before; empty when it did not exist.
    pub before: String,
    /// The file's text after; empty when it is deleted.
    pub after: String,
}

impl Report {
    /// Whether nothing changed.
    pub fn is_unchanged(&self) -> bool {
        self.changes.is_empty()
    }
}

/// One change to the disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// What happened.
    pub op: Op,
    /// To what.
    pub what: What,
    /// The path (absolute).
    pub path: PathBuf,
}

/// What a [`Change`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Op {
    /// Created something that did not exist.
    Create,
    /// Rewrote something kipi owns.
    Update,
    /// Replaced something in the way (a symlink with another target, or with `--force`).
    Replace,
    /// Removed something kipi no longer wants.
    Remove,
}

/// What a [`Change`] touched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum What {
    /// A symlink.
    Link,
    /// A generated block inside a file.
    Block,
    /// A whole file.
    File,
    /// A directory kipi created.
    Dir,
    /// kipi's section of `.git/info/exclude`.
    Exclude,
    /// kipi's read-grant entries in a settings file.
    Grant,
}

impl Op {
    /// `create`, `update`, `replace` or `remove`.
    pub fn as_str(self) -> &'static str {
        match self {
            Op::Create => "create",
            Op::Update => "update",
            Op::Replace => "replace",
            Op::Remove => "remove",
        }
    }
}

impl What {
    /// `link`, `block`, `file`, `dir`, `exclude` or `grant`.
    pub fn as_str(self) -> &'static str {
        match self {
            What::Link => "link",
            What::Block => "block",
            What::File => "file",
            What::Dir => "dir",
            What::Exclude => "exclude",
            What::Grant => "grant",
        }
    }
}

impl fmt::Display for Change {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {} {}",
            self.op.as_str(),
            self.what.as_str(),
            self.path.display()
        )
    }
}

/// An action [`run`] did not carry out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    /// The path concerned.
    pub path: PathBuf,
    /// Why.
    pub message: String,
    /// What to do.
    pub hint: String,
}

impl Skipped {
    fn new(path: &Path, message: impl Into<String>, hint: impl Into<String>) -> Self {
        Skipped {
            path: path.to_path_buf(),
            message: message.into(),
            hint: hint.into(),
        }
    }
}

/// The global sync lock (`~/.config/kipi/lock`), held until dropped.
#[derive(Debug)]
pub struct Lock {
    _file: File,
}

impl Lock {
    /// Take the lock in `config_dir`, creating the directory and the lock file as needed.
    /// Fails at once, with a hint, when another process holds it.
    pub fn acquire(config_dir: &Path) -> Result<Lock> {
        let hint = "check that the config directory is writable";
        fs::create_dir_all(config_dir).map_err(|e| Error::io(config_dir, e, hint))?;
        let path = config_dir.join(LOCK_FILE);
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(|e| Error::io(&path, e, hint))?;
        match file.try_lock() {
            Ok(()) => Ok(Lock { _file: file }),
            Err(TryLockError::WouldBlock) => Err(Error::invalid(
                Location::file(&path),
                "another kipi process is applying changes (the lock is held)",
                "wait for it to finish and re-run; the lock is released when that process exits",
            )),
            Err(TryLockError::Error(e)) => Err(Error::io(&path, e, hint)),
        }
    }
}

/// Write `text` to `path` atomically: a temp file beside it, then a rename. The parent
/// directory must exist. Renaming replaces a symlink at `path`, never its target.
pub fn write_atomic(path: &Path, text: &str) -> Result<()> {
    let hint = "check that the directory exists and is writable";
    let tmp = tmp_path(path);
    fs::write(&tmp, text).map_err(|e| Error::io(&tmp, e, hint))?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        Error::io(path, e, hint)
    })
}

fn tmp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(TMP_SUFFIX);
    path.with_file_name(name)
}

/// The action writing `registry` into `generated/registry.<scope>.md` of the store at
/// `store_dir`; apply it with [`Target::generated`]. `home` is shown as `~`.
pub fn registry_action(store_dir: &Path, registry: &Registry, home: Option<&Path>) -> Action {
    Action::WriteFile {
        path: store_dir.join(registry::relative_path(&registry.scope)),
        text: registry::render(registry, home),
    }
}

/// The facts about the checkout at `dir` that [`crate::plan::repo_actions`] needs:
/// which always-on files git tracks, and which hold user content (present, and not
/// created by kipi per the manifest, or created by kipi and since given text outside
/// the block).
pub fn checkout(dir: &Path, allow_tracked: bool, tracked: &TrackedCache) -> Result<Checkout> {
    let manifest = Manifest::load(&Target::repo(dir).manifest)?;
    let mut candidates: Vec<PathBuf> = Vec::new();
    for id in AgentId::ALL {
        let adapter = adapters::adapter(id);
        let files = [
            Some(adapter.always_on_file(dir)),
            adapter.local_always_on_file(dir),
            adapter.reads_when_absent().map(|name| dir.join(name)),
        ];
        for file in files.into_iter().flatten() {
            if !candidates.contains(&file) {
                candidates.push(file);
            }
        }
    }
    let mut user_files = BTreeSet::new();
    for file in &candidates {
        let Ok(meta) = fs::symlink_metadata(file) else {
            continue;
        };
        let created = file
            .strip_prefix(dir)
            .ok()
            .and_then(|rel| manifest.block(rel))
            .is_some_and(|b| b.created);
        let user = !created
            || !meta.is_file()
            || fs::read_to_string(file)
                .map_err(|_| ())
                .and_then(|text| splice::has_text_outside_block(file, &text).map_err(|_| ()))
                .unwrap_or(true);
        if user {
            user_files.insert(file.clone());
        }
    }
    Ok(Checkout {
        user_files,
        tracked: tracked.tracked(dir, &candidates)?,
        allow_tracked,
    })
}

/// One filesystem operation, planned before anything is touched.
#[derive(Debug)]
enum Step {
    /// Create one directory (its parent exists by then).
    MkDir(PathBuf),
    /// Create the directory and its parents; not recorded (e.g. `.git/info`).
    MkDirAll(PathBuf),
    /// Remove whatever is at the path first (a file or symlink; a directory recursively,
    /// without following symlinks inside it), then link.
    Symlink {
        link: PathBuf,
        target: PathBuf,
        clear: Option<Kind>,
    },
    /// Write a file atomically, removing a directory in the way first (`--force`).
    Write {
        path: PathBuf,
        text: String,
        clear_dir: bool,
    },
    /// Remove a file or a symlink (never its target).
    Delete(PathBuf),
    /// Remove a directory if it is empty.
    RmDir(PathBuf),
}

/// What stands at a path, read without following symlinks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Symlink,
    File,
    Dir,
}

fn kind(path: &Path) -> Result<Option<Kind>> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => Ok(Some(Kind::Symlink)),
        Ok(m) if m.is_dir() => Ok(Some(Kind::Dir)),
        Ok(_) => Ok(Some(Kind::File)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::io(path, e, "check the permissions of the path")),
    }
}

fn read(path: &Path) -> Result<String> {
    fs::read_to_string(path).map_err(|e| Error::io(path, e, "check that the file is readable"))
}

/// Apply `actions` to `target` (roadmap §8 steps 4–5) and record the result in its
/// manifest. Errors are for I/O failures and actions outside the target root; anything
/// kipi declines to do is reported in [`Report::skipped`] and the rest still happens.
///
pub fn run(
    target: &Target,
    actions: &[Action],
    opts: Options,
    tracked: &TrackedCache,
) -> Result<Report> {
    let old = Manifest::load(&target.manifest)?;
    let mut diff = Diff::new(target, &old, opts, tracked);
    for action in actions {
        diff.desired(action)?;
    }
    diff.undesired()?;
    diff.dirs()?;
    diff.foreign()?;
    let Diff {
        steps,
        mut report,
        new,
        ..
    } = diff;
    if opts.dry_run {
        report.changes = steps.into_iter().filter_map(|(_, c)| c).collect();
        return Ok(report);
    }
    // Record everything that is about to exist before touching the disk, so an
    // interrupted run leaves nothing unaccounted for.
    if !steps.is_empty() {
        save_manifest(target, &union(&old, &new))?;
    }
    for (step, change) in steps {
        if execute(&step)? {
            report.changes.extend(change);
        }
    }
    let mut new = new;
    new.dirs.retain(|d| target.root.join(d).is_dir());
    save_manifest(target, &new)?;
    Ok(report)
}

/// The diff of one target: the steps to take and the manifest that results.
struct Diff<'a> {
    root: &'a Path,
    old: &'a Manifest,
    opts: Options,
    /// Answers which files git tracks.
    tracked: &'a TrackedCache,
    new: Manifest,
    steps: Vec<(Step, Option<Change>)>,
    report: Report,
    /// Every path an action wants, relative to the root.
    wanted: BTreeSet<PathBuf>,
    /// Directories planned for creation (absolute), so each is created once.
    planned_dirs: BTreeSet<PathBuf>,
    /// Paths planned for deletion (absolute), for predicting empty directories.
    deleted: BTreeSet<PathBuf>,
    exclude_seen: bool,
    /// The root is missing and planned for creation (a store's `generated/`).
    root_planned: bool,
}

impl<'a> Diff<'a> {
    fn new(
        target: &'a Target,
        old: &'a Manifest,
        opts: Options,
        tracked: &'a TrackedCache,
    ) -> Self {
        Diff {
            root: &target.root,
            old,
            opts,
            tracked,
            new: Manifest::new(),
            steps: Vec::new(),
            report: Report {
                root: target.root.clone(),
                ..Report::default()
            },
            wanted: BTreeSet::new(),
            planned_dirs: BTreeSet::new(),
            deleted: BTreeSet::new(),
            exclude_seen: false,
            root_planned: false,
        }
    }

    fn step(&mut self, step: Step, change: Option<(Op, What, &Path)>) {
        let change = change.map(|(op, what, path)| Change {
            op,
            what,
            path: path.to_path_buf(),
        });
        self.steps.push((step, change));
    }

    fn skip(&mut self, path: &Path, message: impl Into<String>, hint: impl Into<String>) {
        self.report.skipped.push(Skipped::new(path, message, hint));
    }

    /// `path` relative to the root; an action outside it is a bug in the caller.
    fn rel(&self, path: &Path) -> Result<PathBuf> {
        match path.strip_prefix(self.root) {
            Ok(rel) if !rel.as_os_str().is_empty() => Ok(rel.to_path_buf()),
            _ => Err(Error::invalid(
                Location::file(path),
                format!("action outside the target {}", self.root.display()),
                "this is a bug in kipi; please report it",
            )),
        }
    }

    /// Plan the parent directories of `path` (absolute, under the root). `false` when a
    /// parent is a symlink or not a directory: nothing is written through or over it.
    fn parents(&mut self, path: &Path) -> Result<bool> {
        if !self.root_planned && !self.root.is_dir() {
            self.root_planned = true;
            self.step(Step::MkDirAll(self.root.to_path_buf()), None);
        }
        let mut missing = Vec::new();
        let mut dir = path.parent();
        while let Some(d) = dir {
            if d == self.root || !d.starts_with(self.root) {
                break;
            }
            // A planned dir was planned by a walk that checked every ancestor up to the
            // root, so there is nothing left to check above it.
            if self.planned_dirs.contains(d) {
                break;
            }
            match kind(d)? {
                None => missing.push(d.to_path_buf()),
                Some(Kind::Dir) => {}
                Some(Kind::Symlink) => {
                    self.skip(
                        path,
                        format!(
                            "{} is a symlink; kipi does not write through it",
                            d.display()
                        ),
                        "replace the symlink with a real directory, then re-run",
                    );
                    return Ok(false);
                }
                Some(Kind::File) => {
                    self.skip(
                        path,
                        format!("{} is a file, not a directory", d.display()),
                        "move the file away, then re-run",
                    );
                    return Ok(false);
                }
            }
            dir = d.parent();
        }
        for d in missing.into_iter().rev() {
            self.planned_dirs.insert(d.clone());
            let rel = self.rel(&d)?;
            if !self.new.dirs.contains(&rel) {
                self.new.dirs.push(rel);
            }
            self.step(Step::MkDir(d.clone()), Some((Op::Create, What::Dir, &d)));
        }
        Ok(true)
    }

    fn desired(&mut self, action: &Action) -> Result<()> {
        match action {
            Action::Link { link, target } => self.link(link, target),
            Action::WriteBlock { file, text } => self.block(file, text),
            Action::WriteFile { path, text } => self.file(path, text),
            Action::Exclude { repo, patterns } => self.exclude(repo, Some(patterns)),
            Action::MergeGrant { file, key, entries } => self.grant(file, key, entries),
            Action::Remove { path } => {
                // Removal is derived from the manifest; an explicit one is only checked.
                self.rel(path)?;
                Ok(())
            }
        }
    }

    fn link(&mut self, link: &Path, target: &Path) -> Result<()> {
        let rel = self.rel(link)?;
        if !self.wanted.insert(rel.clone()) {
            return Ok(());
        }
        if !target.is_absolute() {
            return Err(Error::invalid(
                Location::file(link),
                format!("link target {} is not absolute", target.display()),
                "this is a bug in kipi; please report it",
            ));
        }
        if !self.parents(link)? {
            return Ok(());
        }
        let entry = LinkEntry {
            path: rel,
            target: target.to_path_buf(),
        };
        let clear = match kind(link)? {
            None => None,
            Some(Kind::Symlink) => {
                let current = fs::read_link(link)
                    .map_err(|e| Error::io(link, e, "check the permissions of the path"))?;
                if current == target {
                    self.new.links.push(entry);
                    return Ok(());
                }
                Some(Kind::Symlink)
            }
            Some(k) if self.opts.force => Some(k),
            Some(k) => {
                let what = if k == Kind::Dir {
                    "a directory"
                } else {
                    "a regular file"
                };
                self.skip(
                    link,
                    format!("{what} stands where kipi links {}", target.display()),
                    "move it away, or re-run with `--force` to replace it",
                );
                return Ok(());
            }
        };
        let op = if clear.is_some() {
            Op::Replace
        } else {
            Op::Create
        };
        self.new.links.push(entry);
        self.step(
            Step::Symlink {
                link: link.to_path_buf(),
                target: target.to_path_buf(),
                clear,
            },
            Some((op, What::Link, link)),
        );
        Ok(())
    }

    fn block(&mut self, file: &Path, text: &str) -> Result<()> {
        let rel = self.rel(file)?;
        if !self.wanted.insert(rel.clone()) {
            return Ok(());
        }
        if !self.parents(file)? {
            return Ok(());
        }
        let old = self.old.block(&rel).cloned();
        let existing = match kind(file)? {
            None => None,
            Some(Kind::File) => Some(read(file)?),
            Some(k) => {
                let what = if k == Kind::Dir {
                    "a directory"
                } else {
                    "a symlink"
                };
                self.skip(
                    file,
                    format!(
                        "{} is {what}; kipi writes the generated block only into a regular file",
                        file.display()
                    ),
                    "make it a regular file, or drop the agent from `agents`",
                );
                self.new.blocks.extend(old);
                return Ok(());
            }
        };
        let spliced = match splice::splice_text(file, existing.as_deref(), text) {
            Ok(s) => s,
            Err(e) => {
                self.skip(file, e.to_string(), e.hint());
                self.new.blocks.extend(old);
                return Ok(());
            }
        };
        self.new.blocks.push(BlockEntry {
            path: rel,
            created: existing.is_none() || old.is_some_and(|o| o.created),
            hash: manifest::hash(text),
        });
        if existing.as_deref() != Some(spliced.as_str()) {
            let op = if existing.is_some() {
                Op::Update
            } else {
                Op::Create
            };
            self.step(
                Step::Write {
                    path: file.to_path_buf(),
                    text: spliced,
                    clear_dir: false,
                },
                Some((op, What::Block, file)),
            );
        }
        Ok(())
    }

    fn file(&mut self, path: &Path, text: &str) -> Result<()> {
        let rel = self.rel(path)?;
        if !self.wanted.insert(rel.clone()) {
            return Ok(());
        }
        if !self.parents(path)? {
            return Ok(());
        }
        let ours = self.old.file(&rel).is_some();
        let entry = FileEntry {
            path: rel,
            hash: manifest::hash(text),
        };
        let (op, clear_dir) = match kind(path)? {
            None => (Op::Create, false),
            Some(Kind::File) if read(path)? == text => {
                self.new.files.push(entry);
                return Ok(());
            }
            Some(Kind::File) if ours => (Op::Update, false),
            Some(k) if self.opts.force => (Op::Replace, k == Kind::Dir),
            Some(_) => {
                self.skip(
                    path,
                    "a file kipi did not write is in the way",
                    "move it away, or re-run with `--force` to replace it",
                );
                return Ok(());
            }
        };
        self.new.files.push(entry);
        self.step(
            Step::Write {
                path: path.to_path_buf(),
                text: text.to_owned(),
                clear_dir,
            },
            Some((op, What::File, path)),
        );
        Ok(())
    }

    fn grant(&mut self, file: &Path, key: &[&str], entries: &[PathBuf]) -> Result<()> {
        let rel = self.rel(file)?;
        if !self.wanted.insert(rel.clone()) {
            return Ok(());
        }
        let mut want = Vec::new();
        for entry in entries {
            match entry.to_str() {
                Some(s) => want.push(s.to_owned()),
                None => self.skip(
                    file,
                    format!("{} is not valid UTF-8; not granted", entry.display()),
                    "rename the directory, or grant it by hand",
                ),
            }
        }
        if want.is_empty() {
            // Nothing to add, so no directory to create; as in removal, nothing below a
            // directory that is gone or turned symlink is touched.
            if self.behind_symlink(file) {
                return Ok(());
            }
        } else if !self.parents(file)? {
            self.new.grants.extend(self.old.grant(&rel).cloned());
            return Ok(());
        }
        let key: Vec<String> = key.iter().map(|k| k.to_string()).collect();
        self.merge_grant(file, rel, key, &want)
    }

    /// Bring kipi's entries at `key` of the settings file `file` (`rel` to the root) to
    /// exactly `want`, recording what it owns afterwards. Anything that stops it keeps the
    /// old manifest entry, so a later run can still remove what kipi added.
    fn merge_grant(
        &mut self,
        file: &Path,
        rel: PathBuf,
        key: Vec<String>,
        want: &[String],
    ) -> Result<()> {
        let old = self.old.grant(&rel).cloned();
        let keep_old = |diff: &mut Self| diff.new.grants.extend(old.clone());
        let existing = match kind(file)? {
            None => None,
            Some(Kind::File) => Some(read(file)?),
            Some(k) => {
                let what = if k == Kind::Dir {
                    "a directory"
                } else {
                    "a symlink"
                };
                self.skip(
                    file,
                    format!(
                        "{} is {what}; kipi merges the read grant only into a regular file",
                        file.display()
                    ),
                    "make it a regular file, then re-run",
                );
                keep_old(self);
                return Ok(());
            }
        };
        // A tracked settings.local.json makes Claude Code ask for workspace trust before
        // applying it (docs/adapters.md), so the grant would silently not take effect.
        if existing.is_some()
            && self
                .tracked
                .tracked(self.root, &[file.to_path_buf()])?
                .contains(file)
        {
            self.skip(
                file,
                "tracked by git; Claude Code applies it only once the folder is trusted, so kipi does not write the read grant there",
                format!("untrack it (`git rm --cached {}`), then re-run", rel.display()),
            );
            keep_old(self);
            return Ok(());
        }
        let owned: Vec<String> = old
            .iter()
            .flat_map(|g| &g.entries)
            .filter_map(|e| e.to_str().map(str::to_owned))
            .collect();
        let created = old.as_ref().map_or(existing.is_none(), |g| g.created);
        let merged = match json_merge::merge(existing.as_deref(), &key, want, &owned, created) {
            Ok(m) => m,
            Err(e) => {
                self.skip(file, e.to_string(), e.hint());
                keep_old(self);
                return Ok(());
            }
        };
        if !merged.owned.is_empty() {
            self.new.grants.push(GrantEntry {
                path: rel,
                key,
                created,
                entries: merged.owned.iter().map(PathBuf::from).collect(),
            });
        }
        let (op, step, after) = match merged.outcome {
            Outcome::Unchanged => return Ok(()),
            Outcome::Write(text) => {
                let op = if existing.is_none() {
                    Op::Create
                } else {
                    Op::Update
                };
                let step = Step::Write {
                    path: file.to_path_buf(),
                    text: text.clone(),
                    clear_dir: false,
                };
                (op, step, text)
            }
            Outcome::Delete => {
                self.deleted.insert(file.to_path_buf());
                (Op::Remove, Step::Delete(file.to_path_buf()), String::new())
            }
        };
        self.step(step, Some((op, What::Grant, file)));
        self.report.grants.push(GrantDiff {
            path: file.to_path_buf(),
            added: merged.added,
            removed: merged.removed,
            before: existing.unwrap_or_default(),
            after,
        });
        Ok(())
    }

    /// Keep `patterns` (or, with `None`, no section) in the repo's `.git/info/exclude`.
    fn exclude(&mut self, repo: &Path, patterns: Option<&[String]>) -> Result<()> {
        if repo != self.root {
            return Err(Error::invalid(
                Location::file(repo),
                format!(
                    "exclude for a repo other than the target {}",
                    self.root.display()
                ),
                "this is a bug in kipi; please report it",
            ));
        }
        if patterns.is_some() {
            if self.exclude_seen {
                return Ok(());
            }
            self.exclude_seen = true;
        }
        let path = match git::info_exclude(repo) {
            Ok(path) => path,
            Err(e) => {
                self.skip(
                    repo,
                    format!("cannot locate .git/info/exclude: {e}"),
                    e.hint(),
                );
                self.new.exclude = self.old.exclude.clone();
                return Ok(());
            }
        };
        let existing = match kind(&path)? {
            Some(Kind::File) => Some(read(&path)?),
            _ => None,
        };
        let current = existing.as_deref().unwrap_or("");
        let updated = match patterns {
            Some(p) => set_exclude_section(current, p),
            None => {
                remove_exclude_section(current).map(|t| t.unwrap_or_else(|| current.to_owned()))
            }
        };
        let updated = match updated {
            Ok(t) => t,
            Err(message) => {
                self.skip(
                    &path,
                    message,
                    format!("keep exactly one `{EXCLUDE_BEGIN}` … `{EXCLUDE_END}` pair, or delete both lines"),
                );
                self.new.exclude = self.old.exclude.clone();
                return Ok(());
            }
        };
        self.new.exclude = patterns.map(|p| ExcludeEntry {
            patterns: p.to_vec(),
        });
        if updated != current {
            let op = match (patterns, self.old.exclude.is_some()) {
                (None, _) => Op::Remove,
                (Some(_), true) => Op::Update,
                (Some(_), false) => Op::Create,
            };
            if let Some(dir) = path.parent()
                && !dir.is_dir()
            {
                self.step(Step::MkDirAll(dir.to_path_buf()), None);
            }
            self.step(
                Step::Write {
                    path: path.clone(),
                    text: updated,
                    clear_dir: false,
                },
                Some((op, What::Exclude, &path)),
            );
        }
        Ok(())
    }

    /// Whether a directory between the root and `path` is no longer a real directory
    /// (the user replaced it with a symlink, say): then nothing below it is kipi's to
    /// remove, since deleting there would act on whatever the symlink points at.
    fn behind_symlink(&self, path: &Path) -> bool {
        path.ancestors()
            .skip(1)
            .take_while(|d| *d != self.root && d.starts_with(self.root))
            .any(|d| kind(d).ok().flatten() != Some(Kind::Dir))
    }

    /// Plan the removal of everything the old manifest lists that no action wants.
    fn undesired(&mut self) -> Result<()> {
        let old = self.old;
        for entry in &old.links {
            if self.wanted.contains(&entry.path) {
                continue;
            }
            let link = self.root.join(&entry.path);
            if self.behind_symlink(&link) {
                continue;
            }
            match kind(&link)? {
                None => {}
                Some(Kind::Symlink)
                    if fs::read_link(&link).ok().as_ref() == Some(&entry.target) =>
                {
                    self.deleted.insert(link.clone());
                    self.step(
                        Step::Delete(link.clone()),
                        Some((Op::Remove, What::Link, &link)),
                    );
                }
                Some(_) => self.skip(
                    &link,
                    "changed since kipi created it; left in place",
                    "remove it yourself if you no longer want it",
                ),
            }
        }
        for entry in &old.blocks {
            if self.wanted.contains(&entry.path) {
                continue;
            }
            let file = self.root.join(&entry.path);
            if self.behind_symlink(&file) {
                continue;
            }
            if kind(&file)? != Some(Kind::File) {
                continue;
            }
            match splice::remove_block(&file, &read(&file)?) {
                Ok(None) => {}
                Ok(Some(rest)) if entry.created && rest.trim().is_empty() => {
                    self.deleted.insert(file.clone());
                    self.step(
                        Step::Delete(file.clone()),
                        Some((Op::Remove, What::File, &file)),
                    );
                }
                Ok(Some(rest)) => self.step(
                    Step::Write {
                        path: file.clone(),
                        text: rest,
                        clear_dir: false,
                    },
                    Some((Op::Remove, What::Block, &file)),
                ),
                Err(e) => {
                    self.skip(&file, e.to_string(), e.hint());
                    self.new.blocks.push(entry.clone());
                }
            }
        }
        for entry in &old.files {
            if self.wanted.contains(&entry.path) {
                continue;
            }
            let path = self.root.join(&entry.path);
            if self.behind_symlink(&path) {
                continue;
            }
            match kind(&path)? {
                Some(Kind::File) if manifest::hash(&read(&path)?) == entry.hash => {
                    self.deleted.insert(path.clone());
                    self.step(
                        Step::Delete(path.clone()),
                        Some((Op::Remove, What::File, &path)),
                    );
                }
                None => {}
                Some(_) => self.skip(
                    &path,
                    "changed since kipi wrote it; left in place",
                    "remove it yourself if you no longer want it",
                ),
            }
        }
        for entry in &old.grants {
            if self.wanted.contains(&entry.path) {
                continue;
            }
            let file = self.root.join(&entry.path);
            if self.behind_symlink(&file) {
                continue;
            }
            self.merge_grant(&file, entry.path.clone(), entry.key.clone(), &[])?;
        }
        if old.exclude.is_some() && !self.exclude_seen {
            let root = self.root.to_path_buf();
            self.exclude(&root, None)?;
        }
        Ok(())
    }

    /// Plan the removal of directories kipi created that nothing wanted lives under,
    /// deepest first, when they will be empty.
    fn dirs(&mut self) -> Result<()> {
        let (needed, unneeded): (Vec<&PathBuf>, Vec<&PathBuf>) = self
            .old
            .dirs
            .iter()
            .partition(|d| self.wanted.iter().any(|w| w.starts_with(d)));
        for d in needed {
            if !self.new.dirs.contains(d) {
                self.new.dirs.push(d.clone());
            }
        }
        let mut dirs: Vec<PathBuf> = unneeded.into_iter().map(|d| self.root.join(d)).collect();
        dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
        for dir in dirs {
            if kind(&dir)? != Some(Kind::Dir) || self.behind_symlink(&dir) {
                continue;
            }
            // Recorded either way: `run` forgets the directories that are gone afterwards.
            self.new.dirs.push(self.rel(&dir)?);
            let entries = fs::read_dir(&dir)
                .map_err(|e| Error::io(&dir, e, "check the permissions of the directory"))?;
            let mut empty = true;
            for entry in entries {
                let entry = entry
                    .map_err(|e| Error::io(&dir, e, "check the permissions of the directory"))?;
                if !self.deleted.contains(&entry.path()) {
                    empty = false;
                    break;
                }
            }
            // One still holding something kipi did not write stays recorded, and goes once empty.
            if empty {
                self.deleted.insert(dir.clone());
                self.step(
                    Step::RmDir(dir.clone()),
                    Some((Op::Remove, What::Dir, &dir)),
                );
            }
        }
        Ok(())
    }

    /// Report what sits beside kipi's links without being kipi's.
    fn foreign(&mut self) -> Result<()> {
        let links: BTreeSet<PathBuf> = self
            .new
            .links
            .iter()
            .map(|l| l.path.clone())
            .chain(self.old.links.iter().map(|l| l.path.clone()))
            .collect();
        let parents: BTreeSet<PathBuf> = self
            .new
            .links
            .iter()
            .filter_map(|l| l.path.parent().map(Path::to_path_buf))
            .collect();
        for parent in parents {
            let dir = self.root.join(&parent);
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let rel = parent.join(entry.file_name());
                if !links.contains(&rel)
                    && !entry.file_name().to_string_lossy().ends_with(TMP_SUFFIX)
                {
                    self.report.foreign.push(entry.path());
                }
            }
        }
        self.report.foreign.sort();
        Ok(())
    }
}

/// Carry out one step; `false` when it turned out to have nothing to do (a directory
/// that was not empty after all).
fn execute(step: &Step) -> Result<bool> {
    let hint = "check the permissions of the path";
    match step {
        Step::MkDir(dir) => fs::create_dir(dir).map_err(|e| Error::io(dir, e, hint))?,
        Step::MkDirAll(dir) => fs::create_dir_all(dir).map_err(|e| Error::io(dir, e, hint))?,
        Step::Symlink {
            link,
            target,
            clear,
        } => {
            match clear {
                // Swapped in by an atomic rename below.
                None | Some(Kind::Symlink) | Some(Kind::File) => {}
                Some(Kind::Dir) => {
                    fs::remove_dir_all(link).map_err(|e| Error::io(link, e, hint))?
                }
            }
            let tmp = tmp_path(link);
            if fs::symlink_metadata(&tmp).is_ok() {
                fs::remove_file(&tmp).map_err(|e| Error::io(&tmp, e, hint))?;
            }
            std::os::unix::fs::symlink(target, &tmp).map_err(|e| Error::io(&tmp, e, hint))?;
            fs::rename(&tmp, link).map_err(|e| {
                let _ = fs::remove_file(&tmp);
                Error::io(link, e, hint)
            })?;
        }
        Step::Write {
            path,
            text,
            clear_dir,
        } => {
            if *clear_dir {
                fs::remove_dir_all(path).map_err(|e| Error::io(path, e, hint))?;
            }
            write_atomic(path, text)?;
        }
        Step::Delete(path) => fs::remove_file(path).map_err(|e| Error::io(path, e, hint))?,
        Step::RmDir(dir) => match fs::remove_dir(dir) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::DirectoryNotEmpty => return Ok(false),
            Err(e) => return Err(Error::io(dir, e, hint)),
        },
    }
    Ok(true)
}

/// Write `manifest` for `target`, or remove the manifest (and its directory, if it is
/// not the root and is left empty) when it records nothing.
fn save_manifest(target: &Target, manifest: &Manifest) -> Result<()> {
    let path = &target.manifest;
    let hint = "check that the directory is writable";
    if manifest.is_empty() {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(Error::io(path, e, hint)),
        }
        if let Some(dir) = path.parent()
            && dir != target.root
        {
            let _ = fs::remove_dir(dir);
        }
        return Ok(());
    }
    let text = manifest.to_toml();
    if fs::read_to_string(path).ok().as_deref() == Some(text.as_str()) {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| Error::io(dir, e, hint))?;
    }
    write_atomic(path, &text)
}

/// Everything either manifest records, `new` winning per path; grant entries are
/// combined per file, since entries being removed are still on disk until the write.
fn union(old: &Manifest, new: &Manifest) -> Manifest {
    fn merge<T: Clone>(old: &[T], new: &[T], key: impl Fn(&T) -> &Path) -> Vec<T> {
        let mut by: BTreeMap<&Path, &T> = old.iter().map(|e| (key(e), e)).collect();
        by.extend(new.iter().map(|e| (key(e), e)));
        by.into_values().cloned().collect()
    }
    let mut dirs = old.dirs.clone();
    dirs.extend(new.dirs.iter().filter(|d| !old.dirs.contains(d)).cloned());
    Manifest {
        version: manifest::VERSION,
        dirs,
        links: merge(&old.links, &new.links, |e| &e.path),
        blocks: merge(&old.blocks, &new.blocks, |e| &e.path),
        files: merge(&old.files, &new.files, |e| &e.path),
        exclude: new.exclude.clone().or_else(|| old.exclude.clone()),
        grants: union_grants(&old.grants, &new.grants),
    }
}

fn union_grants(old: &[GrantEntry], new: &[GrantEntry]) -> Vec<GrantEntry> {
    let mut out: Vec<GrantEntry> = old.to_vec();
    for n in new {
        match out.iter_mut().find(|o| o.path == n.path) {
            Some(o) => {
                o.created |= n.created;
                for e in &n.entries {
                    if !o.entries.contains(e) {
                        o.entries.push(e.clone());
                    }
                }
            }
            None => out.push(n.clone()),
        }
    }
    out
}

/// `text` with kipi's exclude section holding exactly `patterns`: replaced in place, or
/// appended on its own lines.
fn set_exclude_section(text: &str, patterns: &[String]) -> std::result::Result<String, String> {
    let mut section = format!("{EXCLUDE_BEGIN}\n");
    for p in patterns {
        section.push_str(p);
        section.push('\n');
    }
    section.push_str(EXCLUDE_END);
    section.push('\n');
    match exclude_section(text)? {
        Some((start, end)) => Ok(format!("{}{section}{}", &text[..start], &text[end..])),
        None if text.is_empty() || text.ends_with('\n') => Ok(format!("{text}{section}")),
        None => Ok(format!("{text}\n{section}")),
    }
}

/// `text` without kipi's exclude section; `None` when it has none.
fn remove_exclude_section(text: &str) -> std::result::Result<Option<String>, String> {
    Ok(exclude_section(text)?.map(|(start, end)| format!("{}{}", &text[..start], &text[end..])))
}

/// The byte range of kipi's exclude section, line breaks included.
fn exclude_section(text: &str) -> std::result::Result<Option<(usize, usize)>, String> {
    let (mut begin, mut end) = (None, None);
    let mut offset = 0;
    for raw in text.split_inclusive('\n') {
        let line = raw.trim_end();
        if line.starts_with("# kipi:begin") {
            if begin.is_some() {
                return Err("duplicated kipi:begin line in the exclude file".into());
            }
            begin = Some(offset);
        } else if line == EXCLUDE_END {
            if end.is_some() {
                return Err("duplicated kipi:end line in the exclude file".into());
            }
            end = Some(offset + raw.len());
        }
        offset += raw.len();
    }
    match (begin, end) {
        (None, None) => Ok(None),
        (Some(b), Some(e)) if b < e => Ok(Some((b, e))),
        _ => Err("unbalanced kipi:begin / kipi:end lines in the exclude file".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclude_section_is_appended_replaced_and_removed() {
        let pats = |p: &[&str]| p.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let user = "# git ls-files --others --exclude-from=.git/info/exclude\n*.swp\n";
        let once = set_exclude_section(user, &pats(&[".kipi/"])).unwrap();
        assert_eq!(
            once,
            format!("{user}{EXCLUDE_BEGIN}\n.kipi/\n{EXCLUDE_END}\n")
        );
        assert_eq!(
            set_exclude_section(&once, &pats(&[".kipi/"])).unwrap(),
            once
        );
        let twice = set_exclude_section(&once, &pats(&[".agents/", ".kipi/"])).unwrap();
        assert!(twice.ends_with(&format!(
            "{EXCLUDE_BEGIN}\n.agents/\n.kipi/\n{EXCLUDE_END}\n"
        )));
        assert_eq!(
            remove_exclude_section(&twice).unwrap().as_deref(),
            Some(user)
        );
        assert_eq!(remove_exclude_section(user).unwrap(), None);
        assert_eq!(
            set_exclude_section("*.swp", &pats(&["x"])).unwrap(),
            format!("*.swp\n{EXCLUDE_BEGIN}\nx\n{EXCLUDE_END}\n")
        );
        assert!(set_exclude_section(&format!("{EXCLUDE_END}\n"), &pats(&["x"])).is_err());
    }

    #[test]
    fn lock_is_exclusive_until_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config");
        let held = Lock::acquire(&config).unwrap();
        let err = Lock::acquire(&config).unwrap_err();
        assert!(err.to_string().contains("lock is held"), "{err}");
        assert!(err.hint().contains("wait for it"));
        drop(held);
        Lock::acquire(&config).unwrap();
    }

    #[test]
    fn the_crash_safety_union_keeps_grant_entries_still_on_disk() {
        let entry = |entries: &[&str], created| GrantEntry {
            path: ".claude/settings.local.json".into(),
            key: vec!["permissions".into(), "additionalDirectories".into()],
            created,
            entries: entries.iter().map(PathBuf::from).collect(),
        };
        let old = Manifest {
            grants: vec![entry(&["/a", "/b"], true)],
            ..Manifest::new()
        };
        let new = Manifest {
            grants: vec![entry(&["/b", "/c"], false)],
            ..Manifest::new()
        };
        assert_eq!(union(&old, &new).grants, [entry(&["/a", "/b", "/c"], true)]);
        assert_eq!(union(&old, &Manifest::new()).grants, old.grants);
    }

    #[test]
    fn change_display() {
        let c = Change {
            op: Op::Replace,
            what: What::Link,
            path: "/r/.claude/skills/x".into(),
        };
        assert_eq!(c.to_string(), "replace link /r/.claude/skills/x");
    }
}
