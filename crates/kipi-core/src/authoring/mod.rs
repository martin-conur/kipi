//! The authoring commands (roadmap §7 `init`, `stores --unmount`, `add skill|repo|scope`,
//! `bind`, `unbind`): edits to `bindings.toml`, `config.toml` and `machine.toml` that keep
//! comments, ordering and blank lines (toml_edit, roadmap §1 principle 6).
//!
//! Every command computes an [`Outcome`] first and writes nothing. Before an edited
//! `bindings.toml` or `machine.toml` is accepted, the mounted stores are re-loaded with it
//! and re-resolved ([`revalidate`]); any error (a leak-rule violation, a missing skill, a
//! name clash) rejects the edit. [`Outcome::write`] then applies it; `--dry-run` prints
//! it instead.

mod bind;
mod init;
mod repo;
mod scope;
mod skill;
mod toml;

pub use bind::{Binding, bind, unbind};
pub use init::{InitOptions, default_store_name, init, unmount};
pub use repo::{RepoOptions, add_repo};
pub use scope::{ScopeOptions, add_scope};
pub use skill::{SkillOptions, add_skill, is_git_url};

use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use crate::config::{self, BINDINGS_FILE, Env};
use crate::error::{Error, Result, closest};
use crate::model::{Machine, RepoEntry, Skill, Store, StoreSet};
use crate::plan::{Plan, alias_key};
use crate::resolve::resolve;
use crate::scan::{FileWrite, write_files};

/// What an authoring command changes. Nothing is written until [`Outcome::write`].
#[derive(Debug, Default)]
pub struct Outcome {
    /// The store the command edits, by name (`None` for config-only changes).
    pub store: Option<String>,
    /// What changed, in order, for the report.
    pub changes: Vec<Change>,
    /// Directories to create (a new store's layout).
    pub dirs: Vec<PathBuf>,
    /// A skill directory to install into a store.
    pub skill: Option<SkillCopy>,
    /// TOML (and other text) files to write, only those whose text changes.
    pub writes: Vec<FileWrite>,
    /// A user-level skill directory to replace with a symlink into the store.
    pub link_back: Option<LinkBack>,
    /// A directory to `git init`.
    pub git_init: Option<PathBuf>,
    /// Keeps a cloned skill's temp dir alive until it is installed.
    scratch: Option<TempDir>,
}

/// One line of an authoring report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// Added, removed or left as is.
    pub op: Op,
    /// What it is about: `skills`, `related`, `store`, `config.toml stores`, ...
    pub what: String,
    /// The value.
    pub value: String,
    /// Why nothing changed, or what follows from the change.
    pub note: Option<String>,
}

/// The kind of a [`Change`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    /// Added or created.
    Add,
    /// Removed.
    Remove,
    /// Already as asked; nothing to do.
    Keep,
}

impl Op {
    /// `add`, `remove` or `keep`, for JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            Op::Add => "add",
            Op::Remove => "remove",
            Op::Keep => "keep",
        }
    }
}

impl Change {
    fn new(op: Op, what: impl Into<String>, value: impl Into<String>) -> Self {
        Change {
            op,
            what: what.into(),
            value: value.into(),
            note: None,
        }
    }

    fn note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }
}

/// A skill directory tree to copy into a store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillCopy {
    /// The source directory (holding `SKILL.md`).
    pub from: PathBuf,
    /// The skill's directory in the store (`<store>/skills/<name>`); must not exist.
    pub to: PathBuf,
    /// Every file and symlink under `from`, relative, `.git` excluded, sorted.
    pub files: Vec<PathBuf>,
}

impl SkillCopy {
    /// Copy into a hidden staging directory beside `to`, then rename it into place, so
    /// a failed copy never leaves a half-written skill the loader would pick up.
    fn install(&self) -> Result<()> {
        let parent = self.to.parent().unwrap_or(&self.to);
        let name = self.to.file_name().unwrap_or_default().to_string_lossy();
        let staging = parent.join(format!(".{name}.kipi-tmp"));
        let hint = "check that the store's skills/ directory is writable";
        if staging.exists() {
            fs::remove_dir_all(&staging).map_err(|e| Error::io(&staging, e, hint))?;
        }
        fs::create_dir_all(&staging).map_err(|e| Error::io(&staging, e, hint))?;
        for rel in &self.files {
            let (src, dest) = (self.from.join(rel), staging.join(rel));
            if let Some(dir) = dest.parent() {
                fs::create_dir_all(dir).map_err(|e| Error::io(dir, e, hint))?;
            }
            let meta = fs::symlink_metadata(&src).map_err(|e| Error::io(&src, e, hint))?;
            if meta.file_type().is_symlink() {
                let target = fs::read_link(&src).map_err(|e| Error::io(&src, e, hint))?;
                std::os::unix::fs::symlink(&target, &dest)
                    .map_err(|e| Error::io(&dest, e, hint))?;
            } else {
                fs::copy(&src, &dest).map_err(|e| Error::io(&src, e, hint))?;
            }
        }
        fs::rename(&staging, &self.to).map_err(|e| Error::io(&self.to, e, hint))
    }
}

/// Replace `original` (a user-level skill directory that was imported) with a symlink to
/// `target` (its copy in the store).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkBack {
    /// The imported directory, e.g. `~/.claude/skills/<name>`.
    pub original: PathBuf,
    /// The skill's directory in the store.
    pub target: PathBuf,
}

impl LinkBack {
    fn apply(&self) -> Result<()> {
        let hint = "the skill is in the store; replace the directory with a symlink by hand";
        let name = self
            .original
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        let parent = self.original.parent().unwrap_or(&self.original);
        let old = parent.join(format!(".{name}.kipi-old"));
        fs::rename(&self.original, &old).map_err(|e| Error::io(&self.original, e, hint))?;
        if let Err(e) = std::os::unix::fs::symlink(&self.target, &self.original) {
            let _ = fs::rename(&old, &self.original);
            return Err(Error::io(&self.original, e, hint));
        }
        fs::remove_dir_all(&old).map_err(|e| Error::io(&old, e, "remove it by hand"))
    }
}

impl Outcome {
    /// Every path the outcome creates or writes, for the report: directories, the skill
    /// directory, written files, the link-back symlink.
    pub fn paths(&self) -> Vec<&Path> {
        let mut paths: Vec<&Path> = self.dirs.iter().map(PathBuf::as_path).collect();
        paths.extend(self.skill.as_ref().map(|s| s.to.as_path()));
        paths.extend(self.writes.iter().map(|w| w.path.as_path()));
        paths.extend(self.link_back.as_ref().map(|l| l.original.as_path()));
        paths
    }

    /// Whether the outcome changes nothing on disk.
    pub fn is_empty(&self) -> bool {
        self.dirs.is_empty()
            && self.skill.is_none()
            && self.writes.is_empty()
            && self.link_back.is_none()
            && self.git_init.is_none()
    }

    /// Apply the outcome: directories, the skill, the files, the link-back, `git init`.
    pub fn write(&self) -> Result<()> {
        for dir in &self.dirs {
            fs::create_dir_all(dir)
                .map_err(|e| Error::io(dir, e, "check that the parent directory is writable"))?;
        }
        // Deliberately skill first, bindings second: a failure in between leaves an
        // unreferenced skill directory, never a store that references a missing skill.
        if let Some(skill) = &self.skill {
            skill.install()?;
        }
        if let Err(e) = write_files(&self.writes) {
            return Err(match (&self.skill, e) {
                (Some(skill), Error::Io { path, source, hint }) => Error::Io {
                    path,
                    source,
                    hint: unbound_skill_hint(&hint, &skill.to),
                },
                (_, e) => e,
            });
        }
        if let Some(link) = &self.link_back {
            link.apply()?;
        }
        if let Some(dir) = &self.git_init {
            crate::git::init(dir)?;
        }
        Ok(())
    }
}

/// The hint of a failed bindings write after skill `dir` was installed.
fn unbound_skill_hint(hint: &str, dir: &Path) -> String {
    format!(
        "{hint}; the skill was already installed at {} but is not bound: remove that \
         directory, or re-run without it in the way",
        dir.display()
    )
}

/// `after` as a write of `path`, or `None` when the file already holds exactly that.
fn file_write(path: PathBuf, after: String) -> Option<FileWrite> {
    let before = fs::read_to_string(&path).ok();
    (before.as_deref() != Some(after.as_str())).then_some(FileWrite {
        path,
        before,
        after,
    })
}

/// The write of a store's edited `bindings.toml`, if its text changed.
fn bindings_write(store: &Store, text: &str) -> Option<FileWrite> {
    file_write(store.root.join(BINDINGS_FILE), text.to_owned())
}

/// Re-load the mounted stores with each `(store name, text)` of `bindings` replacing that
/// store's `bindings.toml` and `skill` added to its store, validate them as a set and
/// resolve them against `machine`. Any error rejects the edit (nothing is written).
fn revalidate(
    stores: &StoreSet,
    machine: &Machine,
    env: &Env,
    bindings: &[(&str, String)],
    skill: Option<(&str, &Skill)>,
) -> Result<Plan> {
    let check = || {
        let mut list = Vec::with_capacity(stores.stores().len());
        for store in stores.stores() {
            let mut store = match bindings.iter().find(|(name, _)| *name == store.name) {
                Some((_, text)) => config::load_store_text(&store.root, text.clone(), env)?,
                None => store.clone(),
            };
            if let Some((_, skill)) = skill.filter(|(name, _)| *name == store.name) {
                let mut skill = skill.clone();
                skill.at = skill.at.in_store(&store.name);
                store.skills.push(skill);
                store.skills.sort_by(|a, b| a.name.cmp(&b.name));
            }
            list.push(store);
        }
        resolve(&StoreSet::new(list)?, machine)
    };
    check().map_err(Error::rejected)
}

/// The repo entry `name` of the mounted stores, or a usage error naming the closest
/// entry, or the entry `name` is an alias of.
fn find_repo<'a>(stores: &'a StoreSet, name: &str) -> Result<(&'a Store, &'a RepoEntry)> {
    if let Some(found) = stores.repo(name) {
        return Ok(found);
    }
    let key = alias_key(name);
    let by_alias = stores
        .repos()
        .find(|(_, r)| r.aliases.value.iter().any(|a| alias_key(a) == key));
    let hint = match (
        by_alias,
        closest(name, stores.repos().map(|(_, r)| r.name.as_str())),
    ) {
        (Some((_, r)), _) => format!("`{name}` is an alias of `{}`; pass the entry name", r.name),
        (None, Some(near)) => format!("did you mean `{near}`?"),
        (None, None) => "`kipi resolve` lists every repo entry of the mounted stores".to_owned(),
    };
    Err(Error::usage(
        format!("no repo entry `{name}` in the mounted stores"),
        hint,
    ))
}

/// The scope `name` of the mounted stores, or a usage error naming the closest one.
fn find_scope<'a>(
    stores: &'a StoreSet,
    name: &str,
) -> Result<(&'a Store, &'a crate::model::Scope)> {
    stores.scope(name).ok_or_else(|| {
        let names = stores
            .stores()
            .iter()
            .flat_map(|s| &s.scopes)
            .map(|s| s.name.as_str());
        let hint = match closest(name, names) {
            Some(near) => format!("did you mean `{near}`?"),
            None => "`kipi stores` lists the scopes of the mounted stores".to_owned(),
        };
        Error::usage(format!("no scope `{name}` in the mounted stores"), hint)
    })
}

/// The mounted store `name`, or a usage error naming the closest one.
fn find_store<'a>(stores: &'a StoreSet, name: &str) -> Result<&'a Store> {
    stores.store(name).ok_or_else(|| {
        let names = stores.stores().iter().map(|s| s.name.as_str());
        let hint = match closest(name, names) {
            Some(near) => format!("did you mean `{near}`?"),
            None => "`kipi stores` lists the mounted stores".to_owned(),
        };
        Error::usage(format!("no mounted store named `{name}`"), hint)
    })
}

/// `raw` (`~` expanded) as an absolute path, against the working directory if relative.
fn absolute(raw: &Path, env: &Env) -> Result<PathBuf> {
    let expanded = config::expand_tilde(&raw.to_string_lossy(), &env.home);
    std::path::absolute(&expanded).map_err(|e| {
        Error::usage(
            format!("cannot resolve {}: {e}", raw.display()),
            "pass an absolute path",
        )
    })
}

/// `path` as written into `config.toml`: `~/…` under the home directory.
fn tilde(path: &Path, env: &Env) -> String {
    match path.strip_prefix(&env.home) {
        Ok(rel) if rel.as_os_str().is_empty() => "~".to_owned(),
        Ok(rel) => format!("~/{}", rel.display()),
        Err(_) => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unbound_skill_hint_names_the_directory() {
        assert_eq!(
            unbound_skill_hint("check it", Path::new("/s/skills/x")),
            "check it; the skill was already installed at /s/skills/x but is not bound: \
             remove that directory, or re-run without it in the way"
        );
    }
}
