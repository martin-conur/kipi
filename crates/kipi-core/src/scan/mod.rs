//! Repo discovery under scope roots, plus deterministic summary/alias drafting
//! (roadmap §5.2, §7 `scan`, §9).
//!
//! [`scan`] walks the scope roots (after `machine.toml [roots]`) or the given paths for
//! git checkouts, identifies each against the mounted stores' entries, and computes the
//! edits: `machine.toml [repos]` lines, new `[repos.<name>]` entries (with `remote` and
//! drafted `summary` / `aliases`) in the store owning the inferred scope, `remote` filled
//! on entries lacking one, and with `--refresh-drafts` re-drafted values still marked
//! `# kipi:draft`. It returns the edits as [`FileWrite`]s (comment-preserving, nothing
//! written yet) and a report; [`write_files`] applies them. Nothing is ever removed and
//! reviewed values are never touched. Ambiguity is reported, never guessed.

mod discover;
mod draft;
pub(crate) mod write;

pub use discover::{SKIPPED_DIRS, find_checkouts};
pub use draft::{
    GENERIC_WORDS, SUMMARY_CHARS, draft_aliases, draft_summary, first_paragraph,
    manifest_description, readme_summary,
};
pub use write::{DRAFT_MARK, FileWrite, write_files};

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, Item, Value};

use crate::config::BINDINGS_FILE;
use crate::error::Result;
use crate::git;
use crate::model::{
    Checkout, Machine, MatchedBy, RemoteUrl, RepoEntry, RepoMatch, Scope, Store, StoreSet,
    check_name, match_checkout,
};
use crate::plan::alias_key;
use crate::resolve::remote_pattern_matches;

/// What to scan and how.
#[derive(Debug, Clone, Default)]
pub struct ScanOptions {
    /// Directories to walk instead of every scope root (absolute).
    pub paths: Vec<PathBuf>,
    /// Re-draft `summary` / `aliases` values still marked `# kipi:draft`.
    pub refresh_drafts: bool,
    /// Place checkouts that no scope root places in this scope (`--scope`), writing an
    /// explicit `scope =` on their new entries. The caller checks that it exists.
    pub scope: Option<String>,
}

/// The outcome of a scan: the report and the files to write.
#[derive(Debug, Clone, Default)]
pub struct Scan {
    /// Directories walked.
    pub roots: Vec<PathBuf>,
    /// Roots (or given paths) that do not exist on this machine.
    pub missing_roots: Vec<PathBuf>,
    /// Checkouts found, in path order.
    pub checkouts: Vec<PathBuf>,
    /// New `[repos.<name>]` entries.
    pub added: Vec<Added>,
    /// `machine.toml [repos]` lines written: new ones and moved checkouts.
    pub recorded: Vec<Recorded>,
    /// `remote` filled on existing entries that lacked one.
    pub remotes_filled: Vec<RemoteFilled>,
    /// Drafted values rewritten by `--refresh-drafts`.
    pub refreshed: Vec<Refreshed>,
    /// Things a human should look at; nothing was written for them.
    pub warnings: Vec<Warning>,
    /// Entries of the mounted stores with no checkout on this machine, by name.
    pub not_found: Vec<String>,
    /// The edited files, only those whose text changes.
    pub writes: Vec<FileWrite>,
}

/// A new entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Added {
    /// Entry name.
    pub name: String,
    /// Store it was added to.
    pub store: String,
    /// Its scope: inferred from the path, or `--scope` (then written as `scope =`).
    pub scope: String,
    /// Whether `scope =` was written (the checkout lies outside every root).
    pub explicit_scope: bool,
    /// The checkout.
    pub path: PathBuf,
    /// Its normalized remote.
    pub remote: Option<RemoteUrl>,
    /// Drafted summary.
    pub summary: Option<String>,
    /// Drafted aliases.
    pub aliases: Vec<String>,
}

/// A `machine.toml [repos]` line written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recorded {
    /// Entry name.
    pub entry: String,
    /// The checkout.
    pub path: PathBuf,
    /// The path it replaced, which no longer exists (the checkout moved).
    pub previous: Option<PathBuf>,
}

/// `remote` filled on an existing entry (factual, so not a draft).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFilled {
    /// Entry name.
    pub entry: String,
    /// Its store.
    pub store: String,
    /// The remote written.
    pub remote: RemoteUrl,
}

/// A drafted value that `--refresh-drafts` rewrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refreshed {
    /// Entry name.
    pub entry: String,
    /// Its store.
    pub store: String,
    /// `summary` or `aliases`.
    pub field: &'static str,
    /// The new value, as written in TOML.
    pub value: String,
}

/// Something `scan` noticed and did not act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Warning {
    /// The checkout's remote matches `scopes.<pattern_scope>.remote_patterns`, but its
    /// path (or explicit `scope`) places it in a scope outside that one.
    ScopeConflict {
        /// The checkout.
        path: PathBuf,
        /// Its entry (existing or just added).
        entry: String,
        /// The store holding the entry.
        store: String,
        /// The scope it resolves to; `None` when no root places it.
        scope: Option<String>,
        /// The scope whose `remote_patterns` match.
        pattern_scope: String,
        /// The store owning that scope.
        pattern_store: String,
    },
    /// A checkout recorded for an entry has a different remote than the entry's.
    RemoteDiffers {
        /// The checkout.
        path: PathBuf,
        /// The entry.
        entry: String,
        /// The entry's `remote`.
        expected: RemoteUrl,
        /// The checkout's origin, normalized.
        found: RemoteUrl,
    },
    /// Several checkouts match one entry (or are clones of one new repo).
    Duplicate {
        /// The entry; `None` for clones of a repo with no entry yet (none was added).
        entry: Option<String>,
        /// Every checkout, in path order.
        paths: Vec<PathBuf>,
        /// The one `machine.toml` keeps, if any.
        kept: Option<PathBuf>,
    },
    /// Several entries match the checkout under the same rule.
    Ambiguous {
        /// The checkout.
        path: PathBuf,
        /// The matching rule.
        by: MatchedBy,
        /// The candidate entries.
        candidates: Vec<String>,
    },
    /// A new checkout that could not be placed or named; nothing was added.
    Unplaced {
        /// The checkout.
        path: PathBuf,
        /// Why.
        reason: String,
        /// What to do.
        hint: String,
    },
    /// A `machine.toml [repos]` path that no longer exists (kept: scan removes nothing).
    Vanished {
        /// The entry.
        entry: String,
        /// The recorded path.
        path: PathBuf,
    },
}

impl Warning {
    /// A stable kebab-case code for JSON.
    pub fn code(&self) -> &'static str {
        match self {
            Warning::ScopeConflict { .. } => "scope-conflict",
            Warning::RemoteDiffers { .. } => "remote-differs",
            Warning::Duplicate { .. } => "duplicate-checkouts",
            Warning::Ambiguous { .. } => "ambiguous-match",
            Warning::Unplaced { .. } => "unplaced",
            Warning::Vanished { .. } => "vanished",
        }
    }

    /// The checkout or recorded path the warning is about (the first, for duplicates).
    pub fn path(&self) -> &Path {
        match self {
            Warning::ScopeConflict { path, .. }
            | Warning::RemoteDiffers { path, .. }
            | Warning::Ambiguous { path, .. }
            | Warning::Unplaced { path, .. }
            | Warning::Vanished { path, .. } => path,
            Warning::Duplicate { paths, .. } => &paths[0],
        }
    }

    /// The entry the warning is about, if any.
    pub fn entry(&self) -> Option<&str> {
        match self {
            Warning::ScopeConflict { entry, .. }
            | Warning::RemoteDiffers { entry, .. }
            | Warning::Vanished { entry, .. } => Some(entry),
            Warning::Duplicate { entry, .. } => entry.as_deref(),
            Warning::Ambiguous { .. } | Warning::Unplaced { .. } => None,
        }
    }

    /// One line saying what is wrong; paths are shown through `show`.
    pub fn message(&self, show: impl Fn(&Path) -> String) -> String {
        match self {
            Warning::ScopeConflict {
                path,
                entry,
                scope,
                pattern_scope,
                ..
            } => format!(
                "`{entry}` ({}) resolves to {}, but its remote matches \
                 scopes.{pattern_scope}.remote_patterns",
                show(path),
                scope
                    .as_ref()
                    .map_or("no scope".to_owned(), |s| format!("scope `{s}`")),
            ),
            Warning::RemoteDiffers {
                path,
                entry,
                expected,
                found,
            } => format!(
                "{} is recorded for `{entry}` (remote {expected}) but its origin is {found}",
                show(path)
            ),
            Warning::Duplicate { entry, paths, .. } => {
                let list: Vec<String> = paths.iter().map(|p| show(p)).collect();
                match entry {
                    Some(e) => {
                        format!("{} checkouts match `{e}`: {}", paths.len(), list.join(", "))
                    }
                    None => format!(
                        "{} checkouts of one new repo: {}",
                        paths.len(),
                        list.join(", ")
                    ),
                }
            }
            Warning::Ambiguous {
                path,
                by,
                candidates,
            } => format!(
                "{} matches several entries by {}: {}",
                show(path),
                match by {
                    MatchedBy::Remote => "remote",
                    MatchedBy::DirName => "directory name",
                    MatchedBy::RemoteBasename => "remote basename",
                },
                candidates.join(", ")
            ),
            Warning::Unplaced { path, reason, .. } => format!("{}: {reason}", show(path)),
            Warning::Vanished { entry, path } => format!(
                "`{entry}` is recorded at {}, which no longer exists",
                show(path)
            ),
        }
    }

    /// What to do about it.
    pub fn hint(&self) -> String {
        match self {
            Warning::ScopeConflict {
                entry,
                store,
                pattern_scope,
                pattern_store,
                ..
            } if store == pattern_store => format!(
                "add `scope = \"{pattern_scope}\"` to [repos.{entry}] in {store}:{BINDINGS_FILE}"
            ),
            Warning::ScopeConflict {
                entry,
                store,
                pattern_scope,
                pattern_store,
                ..
            } => format!(
                "the entry lives in store `{store}`; move [repos.{entry}] to store \
                 `{pattern_store}` and add `scope = \"{pattern_scope}\"`"
            ),
            Warning::RemoteDiffers { entry, .. } => format!(
                "if the checkout is a fork or mirror, fix `remote` in [repos.{entry}]; \
                 otherwise correct its machine.toml line"
            ),
            Warning::Duplicate { kept: Some(k), .. } => format!(
                "machine.toml keeps {}; remove the other checkouts or point the line at one",
                k.display()
            ),
            Warning::Duplicate { entry: Some(e), .. } => format!(
                "nothing was recorded; add the right one to machine.toml as `\"{e}\" = \"<path>\"`"
            ),
            Warning::Duplicate { entry: None, .. } => {
                "nothing was added; scan one of them alone with `kipi scan <path>`".to_owned()
            }
            Warning::Ambiguous { candidates, .. } => format!(
                "nothing was recorded; set a distinct `remote` on {}, or record the checkout \
                 in machine.toml by hand",
                candidates.join(" / ")
            ),
            Warning::Unplaced { hint, .. } => hint.clone(),
            Warning::Vanished { entry, .. } => format!(
                "re-clone it, or remove the `\"{entry}\"` line from machine.toml (scan never \
                 removes entries)"
            ),
        }
    }
}

/// Scan for checkouts and compute the edits; nothing is written (see [`write_files`]).
///
/// `machine_file` is where `machine.toml` lives (it may not exist yet).
pub fn scan(
    stores: &StoreSet,
    machine: &Machine,
    machine_file: &Path,
    opts: &ScanOptions,
) -> Result<Scan> {
    let mut ctx = Ctx::new(stores, machine);

    let roots = if opts.paths.is_empty() {
        scope_roots(stores, machine)
    } else {
        opts.paths.clone()
    };
    let mut paths = BTreeSet::new();
    for root in dedup(roots) {
        if root.is_dir() {
            paths.extend(find_checkouts(&root)?);
            ctx.out.roots.push(root);
        } else {
            ctx.out.missing_roots.push(root);
        }
    }
    let mut found = Vec::with_capacity(paths.len());
    for path in paths {
        let remote = git::origin_url(&path)?.and_then(|u| RemoteUrl::parse(&u).ok());
        ctx.out.checkouts.push(path.clone());
        found.push(Found { path, remote });
    }

    // Identify each checkout: its machine.toml line, else the shared matching rules.
    // A checkout recorded for an entry of an unmounted store is that entry's: it is
    // left alone rather than added to whichever mounted store's root holds it.
    let recorded: BTreeMap<&Path, &str> = machine
        .repos
        .iter()
        .map(|r| (r.path.as_path(), r.entry.as_str()))
        .collect();
    let mut known: BTreeMap<&str, Vec<&Found>> = BTreeMap::new();
    let mut new = Vec::new();
    for f in &found {
        if let Some(entry) = recorded.get(f.path.as_path()) {
            if stores.repo(entry).is_some() {
                known.entry(entry).or_default().push(f);
            }
            continue;
        }
        match identify(stores, f) {
            Identified::Entry(e) => known.entry(&e.name).or_default().push(f),
            Identified::Ambiguous(by, candidates) => ctx.out.warnings.push(Warning::Ambiguous {
                path: f.path.clone(),
                by,
                candidates,
            }),
            Identified::New => new.push(f),
        }
    }

    for (entry, checkouts) in known {
        ctx.known(entry, &checkouts, opts);
    }
    ctx.add_new(&new, opts);

    for r in &machine.repos {
        let moved = ctx.out.recorded.iter().any(|x| x.entry == r.entry);
        if !moved && !r.path.is_dir() {
            ctx.out.warnings.push(Warning::Vanished {
                entry: r.entry.clone(),
                path: r.path.clone(),
            });
        }
    }
    let recorded_now: BTreeSet<&str> = ctx.out.recorded.iter().map(|r| r.entry.as_str()).collect();
    let mut not_found: Vec<String> = stores
        .repos()
        .map(|(_, r)| r.name.as_str())
        .filter(|n| machine.repo_path(n).is_none() && !recorded_now.contains(n))
        .map(str::to_owned)
        .collect();
    not_found.sort();
    ctx.out.not_found = not_found;

    ctx.finish(machine_file)
}

/// A checkout found on disk.
struct Found {
    path: PathBuf,
    remote: Option<RemoteUrl>,
}

enum Identified<'a> {
    Entry(&'a RepoEntry),
    Ambiguous(MatchedBy, Vec<String>),
    New,
}

/// [`match_checkout`] over every mounted entry, except that a basename match only
/// counts for entries without a `remote`: an entry with a different remote is a
/// different repository that happens to share the name (it gets `<owner>-<name>`).
fn identify<'a>(stores: &'a StoreSet, f: &Found) -> Identified<'a> {
    let dir_name = f
        .path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let checkout = Checkout {
        dir_name,
        remote: f.remote.as_ref(),
    };
    let candidates = match match_checkout(checkout, stores.repos().map(|(_, r)| r)) {
        RepoMatch::Unique { entry, by } => (by, vec![entry]),
        RepoMatch::Ambiguous { by, candidates } => (by, candidates),
        RepoMatch::None => return Identified::New,
    };
    let (by, mut entries) = candidates;
    // Deliberate: a basename match on an entry that has a `remote` is not that entry.
    // Had the remotes been equal, the `Remote` rule would already have matched, so a
    // different remote means a different repository sharing the name. When every
    // basename candidate has a remote (unique or ambiguous), the checkout is therefore
    // `New` (and gets `<owner>-<name>`), not an ambiguity to report.
    if by == MatchedBy::RemoteBasename {
        entries.retain(|e| e.remote.is_none());
    }
    match entries.len() {
        0 => Identified::New,
        1 => Identified::Entry(entries[0]),
        _ => Identified::Ambiguous(by, entries.iter().map(|e| e.name.clone()).collect()),
    }
}

/// Every scope's roots on this machine (`machine.toml [roots]` replaces a scope's own).
fn scope_roots(stores: &StoreSet, machine: &Machine) -> Vec<PathBuf> {
    stores
        .stores()
        .iter()
        .flat_map(|s| &s.scopes)
        .flat_map(|scope| effective_roots(machine, scope).iter().cloned())
        .collect()
}

fn effective_roots<'a>(machine: &'a Machine, scope: &'a Scope) -> &'a [PathBuf] {
    machine
        .roots_for(&scope.name)
        .map_or(scope.roots.as_slice(), |r| r.roots.as_slice())
}

fn dedup(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    paths
        .into_iter()
        .filter(|p| seen.insert(p.clone()))
        .collect()
}

/// Where a path falls among the scopes' roots.
pub(crate) enum Placement<'a> {
    Scope(&'a Store, &'a Scope),
    Tie(Vec<String>),
    Outside,
}

/// The scope whose root is the deepest ancestor of `path`, among `stores`.
pub(crate) fn place<'a>(
    stores: impl IntoIterator<Item = &'a Store>,
    machine: &Machine,
    path: &Path,
) -> Placement<'a> {
    let mut best: Vec<(&Store, &Scope)> = Vec::new();
    let mut best_depth = 0;
    for store in stores {
        for scope in &store.scopes {
            let Some(depth) = effective_roots(machine, scope)
                .iter()
                .filter(|root| path.starts_with(root))
                .map(|root| root.components().count())
                .max()
            else {
                continue;
            };
            if depth > best_depth {
                best = vec![(store, scope)];
                best_depth = depth;
            } else if depth == best_depth {
                best.push((store, scope));
            }
        }
    }
    match best.as_slice() {
        [] => Placement::Outside,
        [(store, scope)] => Placement::Scope(store, scope),
        many => {
            let mut names: Vec<String> = many.iter().map(|(_, s)| s.name.clone()).collect();
            names.sort();
            Placement::Tie(names)
        }
    }
}

/// The scope with the longest `remote_patterns` entry matching `remote`; a tie
/// between scopes decides nothing.
fn pattern_scope<'a>(stores: &'a StoreSet, remote: &RemoteUrl) -> Option<(&'a Store, &'a Scope)> {
    let mut best: Vec<(&Store, &Scope)> = Vec::new();
    let mut best_len = 0;
    for store in stores.stores() {
        for scope in &store.scopes {
            let Some(len) = scope
                .remote_patterns
                .iter()
                .filter(|p| remote_pattern_matches(p, remote.as_str()))
                .map(String::len)
                .max()
            else {
                continue;
            };
            if len > best_len {
                best = vec![(store, scope)];
                best_len = len;
            } else if len == best_len {
                best.push((store, scope));
            }
        }
    }
    match best.as_slice() {
        [one] => Some(*one),
        _ => None,
    }
}

/// The part of an entry name aliases are drafted from: without the `<owner>-` prefix
/// that a name clash adds, so `acme-org-api` drafts like `api` (not `acme`, `org`).
fn alias_base<'n>(name: &'n str, remote: Option<&RemoteUrl>) -> &'n str {
    remote
        .and_then(|r| r.as_str().rsplit('/').nth(1))
        .and_then(|owner| name.strip_prefix(owner)?.strip_prefix('-'))
        .filter(|base| !base.is_empty())
        .unwrap_or(name)
}

/// `scope` is `ancestor` or extends it (within `store`).
fn within(store: &Store, scope: &str, ancestor: &str) -> bool {
    let mut current = Some(scope);
    for _ in 0..=store.scopes.len() {
        match current {
            Some(name) if name == ancestor => return true,
            Some(name) => current = store.scope(name).and_then(|s| s.extends.as_deref()),
            None => return false,
        }
    }
    false
}

struct Ctx<'a> {
    stores: &'a StoreSet,
    machine: &'a Machine,
    /// Edited `bindings.toml` per store name.
    bindings: BTreeMap<String, DocumentMut>,
    machine_doc: Option<DocumentMut>,
    /// Entry names in use, including entries added by this scan.
    names: BTreeSet<String>,
    /// Alias keys per entry name (name plus aliases), including drafts of this scan.
    keys: BTreeMap<String, BTreeSet<String>>,
    out: Scan,
}

impl<'a> Ctx<'a> {
    fn new(stores: &'a StoreSet, machine: &'a Machine) -> Self {
        let mut keys = BTreeMap::new();
        for (_, r) in stores.repos() {
            let mut k: BTreeSet<String> = r.aliases.value.iter().map(|a| alias_key(a)).collect();
            k.insert(alias_key(&r.name));
            keys.insert(r.name.clone(), k);
        }
        Ctx {
            stores,
            machine,
            bindings: BTreeMap::new(),
            machine_doc: None,
            names: stores.repos().map(|(_, r)| r.name.clone()).collect(),
            keys,
            out: Scan::default(),
        }
    }

    fn bindings(&mut self, store: &Store) -> &mut DocumentMut {
        self.bindings
            .entry(store.name.clone())
            .or_insert_with(|| store.bindings.doc.clone())
    }

    fn record(&mut self, entry: &str, path: &Path, previous: Option<PathBuf>) {
        let Some(text) = path.to_str() else { return };
        let machine = self.machine;
        let doc = self.machine_doc.get_or_insert_with(|| {
            machine
                .source
                .as_ref()
                .map(|s| s.doc.clone())
                .unwrap_or_default()
        });
        write::set_machine_repo(doc, entry, text);
        self.out.recorded.push(Recorded {
            entry: entry.to_owned(),
            path: path.to_path_buf(),
            previous,
        });
    }

    /// Alias keys of every entry but `except`.
    fn taken(&self, except: &str) -> BTreeSet<String> {
        self.keys
            .iter()
            .filter(|(name, _)| name.as_str() != except)
            .flat_map(|(_, k)| k.iter().cloned())
            .collect()
    }

    /// An existing entry and the checkouts identified as it.
    fn known(&mut self, name: &str, checkouts: &[&Found], opts: &ScanOptions) {
        let (store, repo) = self.stores.repo(name).expect("identified entries exist");
        let recorded = self.machine.repo_path(name);
        let chosen: Option<&Found> = match (checkouts, recorded) {
            ([f], Some(p)) if f.path == p => Some(f),
            ([f], Some(p)) if !p.is_dir() => {
                self.record(name, &f.path, Some(p.to_path_buf()));
                Some(f)
            }
            ([f], None) => {
                self.record(name, &f.path, None);
                Some(f)
            }
            _ => {
                let mut paths: Vec<PathBuf> = checkouts.iter().map(|f| f.path.clone()).collect();
                let kept = recorded.filter(|p| p.is_dir()).map(Path::to_path_buf);
                if let Some(k) = kept.as_ref().filter(|k| !paths.contains(k)) {
                    paths.push(k.clone());
                }
                paths.sort();
                self.out.warnings.push(Warning::Duplicate {
                    entry: Some(name.to_owned()),
                    paths,
                    kept: kept.clone(),
                });
                kept.and_then(|k| checkouts.iter().copied().find(|f| f.path == k))
            }
        };
        let Some(f) = chosen else { return };

        match (&repo.remote, &f.remote) {
            (Some(expected), Some(found)) if expected != found => {
                self.out.warnings.push(Warning::RemoteDiffers {
                    path: f.path.clone(),
                    entry: name.to_owned(),
                    expected: expected.clone(),
                    found: found.clone(),
                });
            }
            (None, Some(found)) => {
                let doc = self.bindings(store);
                write::set_repo_key(doc, name, "remote", Value::from(found.as_str()));
                self.out.remotes_filled.push(RemoteFilled {
                    entry: name.to_owned(),
                    store: store.name.clone(),
                    remote: found.clone(),
                });
            }
            _ => {}
        }

        if opts.refresh_drafts {
            self.refresh(store, repo, &f.path);
        }

        let scope = repo.scope.clone().or_else(|| {
            match place(std::iter::once(store), self.machine, &f.path) {
                Placement::Scope(_, s) => Some(s.name.clone()),
                _ => None,
            }
        });
        self.check_patterns(f, name, store, scope.as_deref());
    }

    /// Re-draft `repo`'s values still marked draft from its checkout at `path`.
    fn refresh(&mut self, store: &Store, repo: &RepoEntry, path: &Path) {
        if let Some(old) = repo.summary.as_ref().filter(|s| s.is_draft)
            && let Some(new) = draft_summary(path).filter(|n| *n != old.value)
        {
            write::replace_value(
                self.bindings(store),
                &repo.name,
                "summary",
                Value::from(new.as_str()),
            );
            self.out.refreshed.push(Refreshed {
                entry: repo.name.clone(),
                store: store.name.clone(),
                field: "summary",
                value: Value::from(new.as_str()).to_string().trim().to_owned(),
            });
        }
        if repo.aliases.is_draft {
            let base = alias_base(&repo.name, repo.remote.as_ref());
            let new = draft_aliases(base, &self.taken(&repo.name));
            if !new.is_empty() && new != repo.aliases.value {
                let mut keys: BTreeSet<String> = new.iter().map(|a| alias_key(a)).collect();
                keys.insert(alias_key(&repo.name));
                self.keys.insert(repo.name.clone(), keys);
                let value = write::string_array(&new);
                self.out.refreshed.push(Refreshed {
                    entry: repo.name.clone(),
                    store: store.name.clone(),
                    field: "aliases",
                    value: value.to_string().trim().to_owned(),
                });
                write::replace_value(self.bindings(store), &repo.name, "aliases", value);
            }
        }
    }

    /// Warn when `f`'s remote matches a scope's `remote_patterns` but `scope` (where
    /// the entry resolves) is neither that scope nor one extending it.
    fn check_patterns(&mut self, f: &Found, entry: &str, store: &Store, scope: Option<&str>) {
        let Some(remote) = &f.remote else { return };
        let Some((pstore, pscope)) = pattern_scope(self.stores, remote) else {
            return;
        };
        let consistent =
            pstore.name == store.name && scope.is_some_and(|s| within(store, s, &pscope.name));
        if !consistent {
            self.out.warnings.push(Warning::ScopeConflict {
                path: f.path.clone(),
                entry: entry.to_owned(),
                store: store.name.clone(),
                scope: scope.map(str::to_owned),
                pattern_scope: pscope.name.clone(),
                pattern_store: pstore.name.clone(),
            });
        }
    }

    /// Checkouts matching no entry: add an entry to the store owning their scope.
    fn add_new(&mut self, new: &[&Found], opts: &ScanOptions) {
        let mut by_remote: BTreeMap<&RemoteUrl, Vec<&Found>> = BTreeMap::new();
        for f in new {
            if let Some(r) = &f.remote {
                by_remote.entry(r).or_default().push(f);
            }
        }
        for clones in by_remote.values().filter(|c| c.len() > 1) {
            self.out.warnings.push(Warning::Duplicate {
                entry: None,
                paths: clones.iter().map(|f| f.path.clone()).collect(),
                kept: None,
            });
        }
        for f in new {
            let cloned = f
                .remote
                .as_ref()
                .is_some_and(|r| by_remote.get(r).is_some_and(|c| c.len() > 1));
            if !cloned {
                self.add(f, opts);
            }
        }
    }

    fn add(&mut self, f: &Found, opts: &ScanOptions) {
        let unplaced = |reason: String, hint: String| Warning::Unplaced {
            path: f.path.clone(),
            reason,
            hint,
        };
        if f.path.to_str().is_none() {
            self.out.warnings.push(unplaced(
                "the path is not valid UTF-8, so machine.toml cannot hold it".into(),
                "rename the directory".into(),
            ));
            return;
        }
        let forced = || {
            let name = opts.scope.as_deref()?;
            self.stores.scope(name)
        };
        let (store, scope, explicit) = match place(self.stores.stores(), self.machine, &f.path) {
            Placement::Scope(store, scope) => (store, scope, false),
            other => match forced() {
                Some((store, scope)) => (store, scope, true),
                None => {
                    let reason = match other {
                        Placement::Tie(names) => format!(
                            "lies under roots of scopes {} at the same depth",
                            names.join(", ")
                        ),
                        _ => "lies outside every scope root".to_owned(),
                    };
                    let hint = match f
                        .remote
                        .as_ref()
                        .and_then(|r| pattern_scope(self.stores, r))
                    {
                        Some((store, scope)) => format!(
                            "its remote matches scopes.{}.remote_patterns; add it with \
                             `kipi scan {} --store {} --scope {}`",
                            scope.name,
                            f.path.display(),
                            store.name,
                            scope.name
                        ),
                        None => format!(
                            "add it with `kipi scan {} --store <name> --scope <scope>`, or add \
                             its parent directory to a scope's roots",
                            f.path.display()
                        ),
                    };
                    self.out.warnings.push(unplaced(reason, hint));
                    return;
                }
            },
        };

        let Some(name) = self.entry_name(f) else {
            let dir = f.path.file_name().map(|n| n.to_string_lossy().into_owned());
            self.out.warnings.push(unplaced(
                format!(
                    "no free entry name (`{}` is taken or invalid)",
                    dir.unwrap_or_default()
                ),
                "register it by hand with `kipi add repo <name> --path <path>`".into(),
            ));
            return;
        };

        let summary = draft_summary(&f.path);
        let aliases = draft_aliases(alias_base(&name, f.remote.as_ref()), &self.taken(&name));
        let mut items: Vec<(&str, Item)> = Vec::new();
        if let Some(r) = &f.remote {
            items.push(("remote", Item::Value(Value::from(r.as_str()))));
        }
        if explicit {
            items.push(("scope", Item::Value(Value::from(scope.name.as_str()))));
        }
        if let Some(s) = &summary {
            items.push(("summary", write::drafted(s.as_str())));
        }
        if !aliases.is_empty() {
            items.push(("aliases", write::drafted(write::string_array(&aliases))));
        }
        write::add_repo(self.bindings(store), &name, items);

        let mut keys: BTreeSet<String> = aliases.iter().map(|a| alias_key(a)).collect();
        keys.insert(alias_key(&name));
        self.keys.insert(name.clone(), keys);
        self.names.insert(name.clone());
        self.record(&name, &f.path, None);
        self.out.added.push(Added {
            name: name.clone(),
            store: store.name.clone(),
            scope: scope.name.clone(),
            explicit_scope: explicit,
            path: f.path.clone(),
            remote: f.remote.clone(),
            summary,
            aliases,
        });
        self.check_patterns(f, &name, store, Some(&scope.name));
    }

    /// The directory name (else the remote's basename); when another entry has it,
    /// `<owner>-<name>` with the remote's owner. `None` when neither is free and valid.
    fn entry_name(&self, f: &Found) -> Option<String> {
        let valid = |n: &str| check_name(n).is_ok();
        let dir = f
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .filter(|n| valid(n));
        let base = dir.or_else(|| {
            f.remote
                .as_ref()
                .map(RemoteUrl::basename)
                .filter(|n| valid(n))
        })?;
        if !self.names.contains(base) {
            return Some(base.to_owned());
        }
        let owner = f.remote.as_ref()?.as_str().rsplit('/').nth(1)?;
        let qualified = format!("{owner}-{base}");
        (valid(&qualified) && !self.names.contains(&qualified)).then_some(qualified)
    }

    fn finish(mut self, machine_file: &Path) -> Result<Scan> {
        let mut writes = Vec::new();
        for store in self.stores.stores() {
            if let Some(doc) = self.bindings.get(&store.name) {
                writes.push((store.root.join(BINDINGS_FILE), doc.to_string()));
            }
        }
        if let Some(doc) = &self.machine_doc {
            writes.push((machine_file.to_path_buf(), doc.to_string()));
        }
        for (path, after) in writes {
            let before = fs::read_to_string(&path).ok();
            if before.as_deref() != Some(after.as_str()) {
                self.out.writes.push(FileWrite {
                    path,
                    before,
                    after,
                });
            }
        }
        Ok(self.out)
    }
}
