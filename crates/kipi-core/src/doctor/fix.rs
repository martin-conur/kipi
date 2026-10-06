//! `doctor --fix`: the safe fixes of a set of findings as [`Repairs`], applied in a fixed
//! order so each step sees the result of the one before. Config edits (`bindings.toml`
//! `remote`, `machine.toml [repos]` lines) are comment-preserving `toml_edit` writes;
//! broken links kipi did not create are removed with [`apply::remove_link`]; everything
//! else is a re-sync of the repo through the apply engine, which the caller runs on a plan
//! resolved after the config edits (so a removed checkout's grant goes too).

use std::path::{Path, PathBuf};

use toml_edit::{Item, Value};

use crate::apply;
use crate::error::Result;
use crate::model::{Machine, Store};
use crate::scan::{self, FileWrite};

use super::{Finding, Fix};

/// What `--fix` will do, in order: write the config files, remove the links, re-sync.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Repairs {
    /// The fixes, without duplicates, in finding order.
    pub fixes: Vec<Fix>,
    /// Edited `bindings.toml` and `machine.toml` files (only those whose text changes).
    pub writes: Vec<FileWrite>,
    /// Broken links to remove: the link and the target it must still point at.
    pub unlink: Vec<(PathBuf, PathBuf)>,
    /// Repo entries to re-sync, in finding order.
    pub resync: Vec<String>,
}

impl Repairs {
    /// The repairs for every fix in `findings`. `stores` are the stores as loaded (their
    /// `bindings.toml` documents are edited); `machine_file` is where `machine.toml` lives.
    pub fn plan(
        findings: &[Finding],
        stores: &[Store],
        machine: &Machine,
        machine_file: &Path,
    ) -> Repairs {
        let mut fixes: Vec<Fix> = Vec::new();
        for fix in findings.iter().filter_map(|f| f.fix.as_ref()) {
            if !fixes.contains(fix) {
                fixes.push(fix.clone());
            }
        }
        let mut repairs = Repairs::default();
        let mut bindings: Vec<(&Store, toml_edit::DocumentMut)> = Vec::new();
        let mut machine_doc = machine.source.as_ref().map(|s| s.doc.clone());
        for fix in &fixes {
            match fix {
                Fix::FillRemote {
                    store,
                    repo,
                    remote,
                } => {
                    let Some(owner) = stores.iter().find(|s| s.name == *store) else {
                        continue;
                    };
                    let i = match bindings.iter().position(|(s, _)| s.name == *store) {
                        Some(i) => i,
                        None => {
                            bindings.push((owner, owner.bindings.doc.clone()));
                            bindings.len() - 1
                        }
                    };
                    let doc = &mut bindings[i].1;
                    scan::set_repo_key(doc, repo, "remote", Value::from(remote.as_str()));
                }
                Fix::RemoveCheckout { entry, .. } => {
                    if let Some(repos) = machine_doc
                        .as_mut()
                        .and_then(|d| d.get_mut("repos"))
                        .and_then(Item::as_table_like_mut)
                    {
                        repos.remove(entry);
                    }
                }
                Fix::RemoveLink { link, target } => {
                    repairs.unlink.push((link.clone(), target.clone()));
                }
                Fix::Resync { repo } => {
                    if !repairs.resync.contains(repo) {
                        repairs.resync.push(repo.clone());
                    }
                }
            }
        }
        let changed = |path: PathBuf, before: Option<String>, after: String| {
            (before.as_deref() != Some(after.as_str())).then_some(FileWrite {
                path,
                before,
                after,
            })
        };
        for (store, doc) in bindings {
            let path = store.bindings.path.clone();
            let before = store.bindings.doc.to_string();
            repairs
                .writes
                .extend(changed(path, Some(before), doc.to_string()));
        }
        if let (Some(source), Some(doc)) = (&machine.source, machine_doc) {
            let before = source.doc.to_string();
            let path = machine_file.to_path_buf();
            repairs
                .writes
                .extend(changed(path, Some(before), doc.to_string()));
        }
        repairs.fixes = fixes;
        repairs
    }

    /// Whether there is nothing to repair.
    pub fn is_empty(&self) -> bool {
        self.fixes.is_empty()
    }

    /// Write the config files and remove the broken links: everything but the re-sync,
    /// which needs the stores reloaded. Returns the links actually removed; one that
    /// changed since doctor looked is left alone.
    pub fn apply_files(&self) -> Result<Vec<PathBuf>> {
        scan::write_files(&self.writes)?;
        let mut removed = Vec::new();
        for (link, target) in &self.unlink {
            if apply::remove_link(link, target)? {
                removed.push(link.clone());
            }
        }
        Ok(removed)
    }
}
