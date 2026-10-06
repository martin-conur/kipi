//! Regex search over store files and local checkouts, scope-filtered and gitignore-aware.
//!
//! The agent-facing lookup channel (roadmap §3, channel 4): [`lookup`] resolves an alias
//! to a repo entry, [`search_knowledge`] greps the store's conventions, notes and skills
//! and, optionally, the checkouts of the repos in view. Both work on a [`View`]: what a
//! repo's scope chain may see (the leak rule of roadmap §6), or everything mounted.

use std::collections::BTreeSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use regex::Regex;

use crate::model::StoreSet;
use crate::plan::{Plan, RepoPlan, alias_key};

/// Results returned when no limit is given.
pub const DEFAULT_LIMIT: usize = 100;
/// Files larger than this are skipped.
pub const MAX_FILESIZE: u64 = 1024 * 1024;
/// Longer matching lines are cut to this many characters.
pub const MAX_LINE_CHARS: usize = 300;
/// A file with a NUL byte in its first this-many bytes is binary and skipped.
const BINARY_PROBE: usize = 8192;

/// What a lookup or search may see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    /// What the view is, for messages: `scope chain of acme-ml`, `every mounted store`.
    pub label: String,
    /// Visible repo entries, sorted.
    pub repos: BTreeSet<String>,
    /// Visible scopes with their store, in store then file order.
    pub scopes: Vec<(String, String)>,
    /// Visible skill directories, without duplicates, in binding order.
    pub skills: Vec<SkillDir>,
}

/// A skill directory in view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillDir {
    /// The store holding it.
    pub store: String,
    /// Its directory name.
    pub name: String,
    /// The directory.
    pub dir: PathBuf,
}

impl View {
    /// Every repo, scope and skill of every mounted store.
    pub fn everything(stores: &StoreSet) -> Self {
        let mut view = View::empty("every mounted store");
        for store in stores.stores() {
            view.repos
                .extend(store.repos.iter().map(|r| r.name.clone()));
            view.scopes.extend(
                store
                    .scopes
                    .iter()
                    .map(|s| (store.name.clone(), s.name.clone())),
            );
            for skill in &store.skills {
                view.push_skill(&store.name, &skill.name, &skill.dir);
            }
        }
        view
    }

    /// The scope chain of `scope`: the scope, its ancestors and its descendants (the
    /// same rule `related` obeys, roadmap §6), their repos, and every skill those scopes
    /// and repos bind (shared skills included). `None` when no mounted store has `scope`.
    pub fn scope(stores: &StoreSet, plan: &Plan, scope: &str) -> Option<Self> {
        let (store, _) = stores.scope(scope)?;
        let mut view = View::empty(&format!("scope chain of {scope}"));
        let own = chain(stores, scope);
        let family: Vec<&str> = store
            .scopes
            .iter()
            .map(|s| s.name.as_str())
            .filter(|&s| own.contains(&s) || chain(stores, s).contains(&scope))
            .collect();
        for &name in &family {
            view.scopes.push((store.name.clone(), name.to_owned()));
            let Some(declared) = store.scope(name) else {
                continue;
            };
            for skill in &declared.skills {
                if let Some((owner, s)) = stores.skill(&store.name, skill) {
                    view.push_skill(&owner.name, &s.name, &s.dir);
                }
            }
        }
        for repo in &plan.repos {
            if repo.store == store.name
                && repo
                    .scope
                    .value
                    .as_deref()
                    .is_some_and(|s| family.contains(&s))
            {
                view.add_repo(repo);
            }
        }
        Some(view)
    }

    /// What `repo` may see: its scope's chain, or, when it is unscoped, the unscoped
    /// repos of its own store (they count as one chain, roadmap §6).
    pub fn repo(stores: &StoreSet, plan: &Plan, repo: &RepoPlan) -> Self {
        if let Some(scope) = &repo.scope.value
            && let Some(view) = Self::scope(stores, plan, scope)
        {
            return view;
        }
        let mut view = View::empty(&format!("unscoped repos of store {}", repo.store));
        for other in &plan.repos {
            if other.store == repo.store && other.scope.value.is_none() {
                view.add_repo(other);
            }
        }
        view
    }

    fn empty(label: &str) -> Self {
        View {
            label: label.to_owned(),
            repos: BTreeSet::new(),
            scopes: Vec::new(),
            skills: Vec::new(),
        }
    }

    fn add_repo(&mut self, repo: &RepoPlan) {
        self.repos.insert(repo.name.clone());
        for skill in &repo.skills {
            self.push_skill(&skill.store, &skill.name, &skill.dir);
        }
    }

    fn push_skill(&mut self, store: &str, name: &str, dir: &Path) {
        if !self.skills.iter().any(|s| s.dir == dir) {
            self.skills.push(SkillDir {
                store: store.to_owned(),
                name: name.to_owned(),
                dir: dir.to_path_buf(),
            });
        }
    }
}

/// `scope`'s `extends` chain within its store, the scope itself included. Resolution
/// has already rejected unknown parents and cycles; the walk stops on either anyway.
fn chain<'a>(stores: &'a StoreSet, scope: &'a str) -> Vec<&'a str> {
    let mut out = vec![scope];
    let Some((store, mut current)) = stores.scope(scope) else {
        return out;
    };
    while let Some(parent) = current.extends.as_deref() {
        match store.scope(parent) {
            Some(next) if !out.contains(&next.name.as_str()) => {
                out.push(&next.name);
                current = next;
            }
            _ => break,
        }
    }
    out
}

/// The repo entry whose checkout holds `dir` (the deepest one, for nested checkouts).
/// Paths are compared after resolving symlinks where possible.
pub fn repo_at<'p>(plan: &'p Plan, dir: &Path) -> Option<&'p RepoPlan> {
    let real = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let dir = real(dir);
    plan.repos
        .iter()
        .filter_map(|r| r.path.as_deref().map(|p| (r, real(p))))
        .filter(|(_, path)| dir.starts_with(path))
        .max_by_key(|(_, path)| path.components().count())
        .map(|(r, _)| r)
}

/// One repo entry as `where` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoInfo {
    /// The entry name.
    pub name: String,
    /// The store owning it.
    pub store: String,
    /// Its resolved scope.
    pub scope: Option<String>,
    /// `aliases`, as written.
    pub aliases: Vec<String>,
    /// The alias list is still a draft.
    pub aliases_draft: bool,
    /// `summary`.
    pub summary: Option<String>,
    /// The summary is still a draft.
    pub summary_draft: bool,
    /// The checkout on this machine; `None` when not cloned here.
    pub local: Option<PathBuf>,
    /// `repos/<name>/notes.md` in the store, when it exists.
    pub notes: Option<PathBuf>,
}

/// What an alias resolves to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lookup {
    /// Exactly one repo entry.
    Found(RepoInfo),
    /// Several entries claim the alias (a doctor error); sorted by name.
    Ambiguous(Vec<RepoInfo>),
    /// No entry in view has that name or alias.
    NotFound,
}

/// Resolve `alias` to a repo entry in `view`: an entry name wins outright, else every
/// entry listing the alias matches. Matching is exact after case-folding and whitespace
/// normalization (roadmap §6, Aliases); there is no fuzzy matching.
pub fn lookup(stores: &StoreSet, plan: &Plan, view: &View, alias: &str) -> Lookup {
    let key = alias_key(alias);
    let infos: Vec<RepoInfo> = plan
        .repos
        .iter()
        .filter(|r| view.repos.contains(&r.name))
        .filter_map(|r| repo_info(stores, r))
        .collect();
    if let Some(info) = infos.iter().find(|i| alias_key(&i.name) == key) {
        return Lookup::Found(info.clone());
    }
    let mut hits: Vec<RepoInfo> = infos
        .into_iter()
        .filter(|i| i.aliases.iter().any(|a| alias_key(a) == key))
        .collect();
    match hits.len() {
        0 => Lookup::NotFound,
        1 => Lookup::Found(hits.remove(0)),
        _ => Lookup::Ambiguous(hits),
    }
}

fn repo_info(stores: &StoreSet, plan: &RepoPlan) -> Option<RepoInfo> {
    let (_, entry) = stores.repo(&plan.name)?;
    Some(RepoInfo {
        name: plan.name.clone(),
        store: plan.store.clone(),
        scope: plan.scope.value.clone(),
        aliases: entry.aliases.value.clone(),
        aliases_draft: entry.aliases_is_draft(),
        summary: entry.summary.as_ref().map(|s| s.value.clone()),
        summary_draft: entry.summary_is_draft(),
        local: plan.path.as_deref().map(absolute),
        notes: entry.notes.as_deref().map(absolute),
    })
}

/// How to search.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchOptions {
    /// Also search the checkouts of every repo in view (`--repos`).
    pub repos: bool,
    /// Stop after this many hits.
    pub limit: usize,
    /// Skip files larger than this.
    pub max_filesize: u64,
}

impl Default for SearchOptions {
    fn default() -> Self {
        SearchOptions {
            repos: false,
            limit: DEFAULT_LIMIT,
            max_filesize: MAX_FILESIZE,
        }
    }
}

/// What a searched file is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HitSource {
    /// A scope's always-on file (`conventions/<scope>.md`).
    Conventions {
        /// The scope.
        scope: String,
    },
    /// `repos/<repo>/notes.md`.
    Notes {
        /// The repo entry.
        repo: String,
    },
    /// A file in a skill directory.
    Skill {
        /// The store holding the skill.
        store: String,
        /// The skill.
        name: String,
    },
    /// A file in a repo's checkout.
    Checkout {
        /// The repo entry.
        repo: String,
    },
}

/// One matching line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// The file, absolute.
    pub path: PathBuf,
    /// 1-based line number.
    pub line: usize,
    /// The line, without its line ending, cut to [`MAX_LINE_CHARS`].
    pub text: String,
    /// What the file is.
    pub source: HitSource,
}

/// The result of [`search_knowledge`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Search {
    /// Matching lines: conventions, notes, skills, then checkouts by repo name; files
    /// in path order.
    pub hits: Vec<Hit>,
    /// More lines matched than the limit allowed.
    pub truncated: bool,
}

/// Search `view`'s conventions, notes and skills (and, with `options.repos`, the local
/// checkouts of its repos) for `query`. Walks honor `.gitignore`, `.git/info/exclude` and
/// hidden files like ripgrep; binary files and files over `max_filesize` are skipped;
/// symlinks are not followed.
pub fn search_knowledge(
    stores: &StoreSet,
    plan: &Plan,
    view: &View,
    query: &Regex,
    options: &SearchOptions,
) -> Search {
    let mut roots: Vec<(PathBuf, HitSource)> = Vec::new();
    for (store, scope) in &view.scopes {
        let Some(declared) = stores.store(store).and_then(|s| s.scope(scope)) else {
            continue;
        };
        let root = &stores
            .store(store)
            .map(|s| s.root.clone())
            .unwrap_or_default();
        for file in &declared.always {
            let source = HitSource::Conventions {
                scope: scope.clone(),
            };
            roots.push((root.join(file), source));
        }
    }
    for name in &view.repos {
        if let Some(notes) = stores.repo(name).and_then(|(_, r)| r.notes.clone()) {
            roots.push((notes, HitSource::Notes { repo: name.clone() }));
        }
    }
    for skill in &view.skills {
        let source = HitSource::Skill {
            store: skill.store.clone(),
            name: skill.name.clone(),
        };
        roots.push((skill.dir.clone(), source));
    }
    if options.repos {
        for repo in plan.repos.iter().filter(|r| view.repos.contains(&r.name)) {
            if let Some(path) = repo.path.as_ref().filter(|p| p.is_dir()) {
                let source = HitSource::Checkout {
                    repo: repo.name.clone(),
                };
                roots.push((path.clone(), source));
            }
        }
    }
    let mut out = Search::default();
    for (root, source) in roots {
        if !root.exists() {
            continue;
        }
        if search_root(&root, &source, query, options, &mut out) {
            break;
        }
    }
    out
}

/// Search one file or directory; `true` once the limit is exceeded.
fn search_root(
    root: &Path,
    source: &HitSource,
    query: &Regex,
    options: &SearchOptions,
    out: &mut Search,
) -> bool {
    let walk = ignore::WalkBuilder::new(root)
        .require_git(false)
        .follow_links(false)
        .max_filesize(Some(options.max_filesize))
        .sort_by_file_name(|a, b| a.cmp(b))
        .build();
    for entry in walk {
        // Unreadable entries (permissions, races) are skipped, as ripgrep does.
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let Some(text) = read_text(entry.path()) else {
            continue;
        };
        for (i, line) in text.lines().enumerate() {
            if !query.is_match(line) {
                continue;
            }
            if out.hits.len() == options.limit {
                out.truncated = true;
                return true;
            }
            out.hits.push(Hit {
                path: absolute(entry.path()),
                line: i + 1,
                text: cut(line.trim_end_matches('\r')),
                source: source.clone(),
            });
        }
    }
    false
}

/// `path` made absolute against the working directory (a store mounted as
/// `--store examples/x` has relative paths); agents need paths that work from anywhere.
fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// The file's text, or `None` when it is binary or cannot be read.
fn read_text(path: &Path) -> Option<String> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .and_then(|mut f| f.read_to_end(&mut bytes))
        .ok()?;
    if bytes[..bytes.len().min(BINARY_PROBE)].contains(&0) {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn cut(line: &str) -> String {
    match line.char_indices().nth(MAX_LINE_CHARS) {
        Some((at, _)) => format!("{}…", &line[..at]),
        None => line.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_lines_are_cut() {
        assert_eq!(cut("short"), "short");
        let long = "é".repeat(MAX_LINE_CHARS + 5);
        let cut = cut(&long);
        assert_eq!(cut.chars().count(), MAX_LINE_CHARS + 1);
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn binary_files_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("x.bin");
        fs::write(&bin, b"abc\0def").unwrap();
        assert_eq!(read_text(&bin), None);
        let txt = dir.path().join("x.txt");
        fs::write(&txt, "abc\n").unwrap();
        assert_eq!(read_text(&txt).as_deref(), Some("abc\n"));
    }
}
