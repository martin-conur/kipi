//! In-memory builders for kipi-core's model, so resolution can be unit-tested without
//! touching the filesystem.
//!
//! Every declaration gets its own line in its store's `bindings.toml` (counted from 1 in
//! the order it is added) and is tagged with the store name, like the real loader does.
//!
//! ```
//! use kipi_test_support::builders::{MachineBuilder, StoreBuilder, store_set};
//!
//! let work = StoreBuilder::new("work", "/stores/work")
//!     .scope("work", |s| s.roots = vec!["/w".into()])
//!     .skill("review", Some("work"))
//!     .repo("api", |_| {})
//!     .build();
//! let set = store_set(vec![work]);
//! let machine = MachineBuilder::new().repo("api", "/w/api").build();
//! assert_eq!(set.stores().len(), 1);
//! assert_eq!(machine.repo_path("api"), Some(std::path::Path::new("/w/api")));
//! ```

use std::path::{Path, PathBuf};

use kipi_core::config::{BINDINGS_FILE, MACHINE_FILE, SKILL_FILE};
use kipi_core::error::Location;
use kipi_core::model::{
    Defaults, Drafted, Machine, MachineRepo, MachineRoots, RemoteUrl, RepoEntry, Scope, Skill,
    SkillRef, Store, StoreSet, TomlDoc,
};
use toml_edit::DocumentMut;

/// Builds one [`Store`].
#[derive(Debug, Clone)]
pub struct StoreBuilder {
    store: Store,
    line: usize,
}

impl StoreBuilder {
    /// An empty store `name` rooted at `root`, with `[store]` on line 1.
    pub fn new(name: &str, root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let bindings = root.join(BINDINGS_FILE);
        StoreBuilder {
            store: Store {
                name: name.to_owned(),
                name_at: Location::at(&bindings, 1).in_store(name),
                version: kipi_core::model::STORE_VERSION,
                share_skills: false,
                defaults: Defaults::default(),
                scopes: Vec::new(),
                repos: Vec::new(),
                skills: Vec::new(),
                bindings: TomlDoc {
                    path: bindings,
                    doc: DocumentMut::new(),
                },
                root,
            },
            line: 1,
        }
    }

    fn next_at(&mut self) -> Location {
        self.line += 1;
        Location::at(&self.store.bindings.path, self.line).in_store(&self.store.name)
    }

    /// Set `share_skills`.
    pub fn share_skills(mut self, share: bool) -> Self {
        self.store.share_skills = share;
        self
    }

    /// Edit `[defaults]` (declared on the next line).
    pub fn defaults(mut self, edit: impl FnOnce(&mut Defaults)) -> Self {
        self.store.defaults.at = Some(self.next_at());
        edit(&mut self.store.defaults);
        self
    }

    /// Add `[scopes.<name>]`, edited by `edit`.
    pub fn scope(mut self, name: &str, edit: impl FnOnce(&mut Scope)) -> Self {
        let mut scope = scope(name, self.next_at());
        edit(&mut scope);
        self.store.scopes.push(scope);
        self
    }

    /// Add `[repos.<name>]`, edited by `edit`.
    pub fn repo(mut self, name: &str, edit: impl FnOnce(&mut RepoEntry)) -> Self {
        let mut repo = repo(name, self.next_at());
        edit(&mut repo);
        self.store.repos.push(repo);
        self
    }

    /// Add `skills/<name>/SKILL.md` with `x-kipi.scope` set to `scope`.
    pub fn skill(mut self, name: &str, scope: Option<&str>) -> Self {
        let dir = self.store.root.join("skills").join(name);
        self.store.skills.push(Skill {
            name: name.to_owned(),
            description: Some(format!("Use when testing {name}.")),
            scope: scope.map(str::to_owned),
            tags: Vec::new(),
            at: Location::file(dir.join(SKILL_FILE)).in_store(&self.store.name),
            dir,
        });
        self.store.skills.sort_by(|a, b| a.name.cmp(&b.name));
        self
    }

    /// Give `[repos.<name>]` a `repos/<name>/notes.md` (the store has one on disk).
    pub fn notes(mut self, name: &str) -> Self {
        let path = self.store.root.join("repos").join(name).join("notes.md");
        let repo = self
            .store
            .repos
            .iter_mut()
            .find(|r| r.name == name)
            .unwrap_or_else(|| panic!("no [repos.{name}] to attach notes to"));
        repo.notes = Some(path);
        self
    }

    /// The store.
    pub fn build(self) -> Store {
        self.store
    }
}

/// A `[scopes.<name>]` with nothing set, declared at `at`.
pub fn scope(name: &str, at: Location) -> Scope {
    Scope {
        name: name.to_owned(),
        extends: None,
        roots: Vec::new(),
        skills: Vec::new(),
        always: Vec::new(),
        registry: false,
        remote_patterns: Vec::new(),
        agents: None,
        grant: None,
        budget_tokens: None,
        at,
    }
}

/// A `[repos.<name>]` with nothing set, declared at `at`.
pub fn repo(name: &str, at: Location) -> RepoEntry {
    RepoEntry {
        name: name.to_owned(),
        remote: None,
        summary: None,
        aliases: Drafted::default(),
        related: Vec::new(),
        skills: Vec::new(),
        exclude: Vec::new(),
        scope: None,
        agents: None,
        grant: None,
        budget_tokens: None,
        notes: None,
        at,
    }
}

/// Parse skill references (`code-review`, `personal/commit-style`). Panics on a bad one.
pub fn skill_refs(refs: &[&str]) -> Vec<SkillRef> {
    refs.iter()
        .map(|r| {
            r.parse()
                .unwrap_or_else(|e| panic!("bad skill ref `{r}`: {e}"))
        })
        .collect()
}

/// A normalized remote (`github.com/owner/name`). Panics on a bad one.
pub fn remote(normalized: &str) -> RemoteUrl {
    let url = RemoteUrl::parse(&format!("https://{normalized}"))
        .unwrap_or_else(|e| panic!("bad remote `{normalized}`: {e}"));
    assert_eq!(url.as_str(), normalized, "remote is not normalized");
    url
}

/// A summary or alias value that no human has reviewed yet.
pub fn draft<T>(value: T) -> Drafted<T> {
    Drafted {
        value,
        is_draft: true,
    }
}

/// A reviewed summary or alias value.
pub fn reviewed<T>(value: T) -> Drafted<T> {
    Drafted {
        value,
        is_draft: false,
    }
}

/// `stores` validated as a set. Panics with the error report when they are invalid.
pub fn store_set(stores: Vec<Store>) -> StoreSet {
    StoreSet::new(stores).unwrap_or_else(|e| panic!("{}", e.report()))
}

/// Builds a [`Machine`] (`machine.toml`), one line per call.
#[derive(Debug, Clone, Default)]
pub struct MachineBuilder {
    machine: Machine,
    line: usize,
}

impl MachineBuilder {
    /// An empty machine.
    pub fn new() -> Self {
        Self::default()
    }

    fn next_at(&mut self) -> Location {
        self.line += 1;
        Location::at(Path::new("/config").join(MACHINE_FILE), self.line)
    }

    /// `[roots] "<scope>" = [roots...]`.
    pub fn roots(mut self, scope: &str, roots: &[&str]) -> Self {
        let at = self.next_at();
        self.machine.roots.push(MachineRoots {
            scope: scope.to_owned(),
            roots: roots.iter().map(PathBuf::from).collect(),
            at,
        });
        self
    }

    /// `[repos] "<entry>" = "<path>"`.
    pub fn repo(mut self, entry: &str, path: impl Into<PathBuf>) -> Self {
        let at = self.next_at();
        self.machine.repos.push(MachineRepo {
            entry: entry.to_owned(),
            path: path.into(),
            at,
        });
        self
    }

    /// The machine.
    pub fn build(self) -> Machine {
        self.machine
    }
}
