//! Every mounted store, checked as a set (roadmap §6, Multiple stores).
//!
//! Each store is loaded on its own; [`StoreSet::new`] then enforces what only the set
//! can see: store, scope and repo entry names are unique across stores, and nothing
//! references into another store except a qualified `<store>/<skill>` to a store that
//! sets `share_skills = true`. Whether a reference within a store names something that
//! exists is left to resolution.

use std::path::Path;

use super::{RepoEntry, Scope, Skill, SkillRef, Store};
use crate::error::{Error, Location, Result};

/// The mounted stores, in mount order.
#[derive(Debug, Clone)]
pub struct StoreSet {
    stores: Vec<Store>,
}

impl StoreSet {
    /// Validate `stores` as a set. Collisions and cross-store references are errors
    /// naming both locations.
    pub fn new(stores: Vec<Store>) -> Result<Self> {
        let (set, problems) = Self::checked(stores)?;
        match problems.into_iter().next() {
            Some(problem) => Err(problem.error),
            None => Ok(set),
        }
    }

    /// Like [`StoreSet::new`], but every bad reference is returned instead of failing on
    /// the first, so `doctor` can report them all. Name collisions are still errors: with
    /// two stores claiming a name, no lookup in the set means anything.
    pub fn checked(stores: Vec<Store>) -> Result<(Self, Vec<RefProblem>)> {
        let set = StoreSet { stores };
        set.check_unique_names()?;
        let mut problems = Vec::new();
        for store in &set.stores {
            set.check_references(store, &mut problems);
        }
        Ok((set, problems))
    }

    /// The stores, consumed, in mount order.
    pub fn into_stores(self) -> Vec<Store> {
        self.stores
    }

    /// The stores, in mount order.
    pub fn stores(&self) -> &[Store] {
        &self.stores
    }

    /// The store named `name`.
    pub fn store(&self, name: &str) -> Option<&Store> {
        self.stores.iter().find(|s| s.name == name)
    }

    /// The scope named `name`, with its store.
    pub fn scope(&self, name: &str) -> Option<(&Store, &Scope)> {
        self.stores
            .iter()
            .find_map(|st| st.scope(name).map(|s| (st, s)))
    }

    /// The repo entry named `name`, with its store.
    pub fn repo(&self, name: &str) -> Option<(&Store, &RepoEntry)> {
        self.stores
            .iter()
            .find_map(|st| st.repo(name).map(|r| (st, r)))
    }

    /// Every repo entry with its store, in mount then file order.
    pub fn repos(&self) -> impl Iterator<Item = (&Store, &RepoEntry)> {
        self.stores
            .iter()
            .flat_map(|st| st.repos.iter().map(move |r| (st, r)))
    }

    /// The skill `skill` names when referenced from store `from`: a bare name in `from`
    /// itself, a qualified one in the named store if that store shares its skills (or
    /// is `from`).
    pub fn skill(&self, from: &str, skill: &SkillRef) -> Option<(&Store, &Skill)> {
        let owner = match skill.store.as_deref() {
            None => from,
            Some(name) => name,
        };
        let store = self.store(owner)?;
        if owner != from && !store.share_skills {
            return None;
        }
        store.skill(&skill.name).map(|s| (store, s))
    }

    /// `at` as `store:path-in-store:line` (`acme:bindings.toml:12`) when it lies in a
    /// mounted store, else as `path:line`.
    pub fn provenance(&self, at: &Location) -> String {
        let rel = at.store.as_deref().and_then(|name| {
            let store = self.store(name)?;
            let rel = at.file.strip_prefix(&store.root).ok()?;
            Some((name, rel))
        });
        match (rel, at.line) {
            (Some((name, rel)), Some(line)) => format!("{name}:{}:{line}", rel.display()),
            (Some((name, rel)), None) => format!("{name}:{}", rel.display()),
            (None, _) => at.to_string(),
        }
    }

    fn check_unique_names(&self) -> Result<()> {
        for (i, later) in self.stores.iter().enumerate() {
            let earlier = &self.stores[..i];
            if let Some(first) = earlier.iter().find(|s| s.name == later.name) {
                return Err(if same_dir(&first.root, &later.root) {
                    Error::conflict(
                        later.name_at.clone(),
                        first.name_at.clone(),
                        format!("store `{}` is mounted twice", later.name),
                        "list each store once across --store, KIPI_STORE and config.toml `stores`",
                    )
                } else {
                    Error::conflict(
                        later.name_at.clone(),
                        first.name_at.clone(),
                        format!("store name `{}` is used by two mounted stores", later.name),
                        "store names are unique among mounted stores; change `[store] name` \
                         in one of them (and any `<store>/<skill>` reference to it)",
                    )
                });
            }
            for scope in &later.scopes {
                if let Some((other, s)) = find_in(earlier, |st| st.scope(&scope.name)) {
                    return Err(Error::conflict(
                        scope.at.clone(),
                        s.at.clone(),
                        format!(
                            "scope `{}` is declared in stores `{}` and `{}`",
                            scope.name, other.name, later.name
                        ),
                        "scope names are unique across mounted stores (machine.toml [roots] is \
                         keyed by them); rename one of the scopes",
                    ));
                }
            }
            for repo in &later.repos {
                if let Some((other, r)) = find_in(earlier, |st| st.repo(&repo.name)) {
                    return Err(Error::conflict(
                        repo.at.clone(),
                        r.at.clone(),
                        format!(
                            "repo entry `{}` is declared in stores `{}` and `{}`",
                            repo.name, other.name, later.name
                        ),
                        "entry names are unique across mounted stores (machine.toml [repos] is \
                         keyed by them); rename one, e.g. to `<owner>-<name>`, and keep its \
                         `remote` so checkouts still match",
                    ));
                }
            }
        }
        Ok(())
    }

    /// No scope, repo or bare skill reference of `store` lands in another store; every
    /// qualified skill reference names a mounted store that shares the skill. Each
    /// violation is pushed to `problems`, in declaration order.
    fn check_references(&self, store: &Store, problems: &mut Vec<RefProblem>) {
        let others: Vec<&Store> = self
            .stores
            .iter()
            .filter(|s| s.name != store.name)
            .collect();
        let foreign = |what: &str, name: &str, at: &Location, other: &Store, theirs: &Location| {
            Error::conflict(
                at.clone(),
                theirs.clone(),
                format!(
                    "{what} `{name}` belongs to store `{}`, not to store `{}`",
                    other.name, store.name
                ),
                format!(
                    "nothing crosses stores: {what} references resolve only within the store \
                     that owns them; declare `{name}` in store `{}` or remove the reference",
                    store.name
                ),
            )
        };
        let mut push = |site: RefSite, kind: RefKind, error: Error| {
            problems.push(RefProblem {
                store: store.name.clone(),
                site,
                kind,
                error,
            });
        };
        let scope_ref = |name: &str, at: &Location| -> Option<Error> {
            if store.scope(name).is_some() {
                return None;
            }
            find_in(&others, |st| st.scope(name))
                .map(|(other, s)| foreign("scope", name, at, other, &s.at))
        };

        for scope in &store.scopes {
            if let Some(parent) = &scope.extends
                && let Some(e) = scope_ref(parent, &scope.at)
            {
                let site = RefSite::ScopeExtends {
                    scope: scope.name.clone(),
                };
                push(site, RefKind::CrossStore, e);
            }
            for skill in &scope.skills {
                if let Some((kind, e)) = self.check_skill_ref(store, &others, skill, &scope.at) {
                    let site = RefSite::ScopeSkill {
                        scope: scope.name.clone(),
                        skill: skill.clone(),
                    };
                    push(site, kind, e);
                }
            }
        }
        for repo in &store.repos {
            if let Some(scope) = &repo.scope
                && let Some(e) = scope_ref(scope, &repo.at)
            {
                let site = RefSite::RepoScope {
                    repo: repo.name.clone(),
                };
                push(site, RefKind::CrossStore, e);
            }
            for rel in &repo.related {
                if store.repo(rel).is_none()
                    && let Some((other, r)) = find_in(&others, |st| st.repo(rel))
                {
                    let site = RefSite::RepoRelated {
                        repo: repo.name.clone(),
                        name: rel.clone(),
                    };
                    push(
                        site,
                        RefKind::CrossStore,
                        foreign("repo", rel, &repo.at, other, &r.at),
                    );
                }
            }
            for (skill, exclude) in (repo.skills.iter().map(|s| (s, false)))
                .chain(repo.exclude.iter().map(|s| (s, true)))
            {
                if let Some((kind, e)) = self.check_skill_ref(store, &others, skill, &repo.at) {
                    let (repo, skill) = (repo.name.clone(), skill.clone());
                    let site = if exclude {
                        RefSite::RepoExclude { repo, skill }
                    } else {
                        RefSite::RepoSkill { repo, skill }
                    };
                    push(site, kind, e);
                }
            }
        }
    }

    fn check_skill_ref(
        &self,
        store: &Store,
        others: &[&Store],
        skill: &SkillRef,
        at: &Location,
    ) -> Option<(RefKind, Error)> {
        let target = match skill.store.as_deref() {
            Some(name) if name != store.name => name,
            // Bare (or self-qualified): resolves in `store`. A name that exists only in
            // another store is a cross-store reference.
            _ => {
                if store.skill(&skill.name).is_some() {
                    return None;
                }
                let (other, s) = find_in(others, |st| st.skill(&skill.name))?;
                let fix = if other.share_skills {
                    format!("reference it as `{}/{}`", other.name, skill.name)
                } else {
                    format!(
                        "store `{}` does not share its skills; copy the skill into store `{}` \
                         or remove the reference",
                        other.name, store.name
                    )
                };
                return Some((
                    RefKind::CrossStore,
                    Error::conflict(
                        at.clone(),
                        s.at.clone(),
                        format!(
                            "skill `{}` is not in store `{}`; it belongs to store `{}`",
                            skill.name, store.name, other.name
                        ),
                        format!("bare skill names resolve within their own store; {fix}"),
                    ),
                ));
            }
        };
        let Some(other) = self.store(target) else {
            return Some((
                RefKind::MissingSkill,
                Error::invalid(
                    at.clone(),
                    format!("skill `{skill}` names store `{target}`, which is not mounted"),
                    format!(
                        "mount store `{target}` (--store, KIPI_STORE or config.toml `stores`) or \
                         remove the reference"
                    ),
                ),
            ));
        };
        if !other.share_skills {
            return Some((
                RefKind::CrossStore,
                Error::conflict(
                    at.clone(),
                    other.name_at.clone(),
                    format!(
                        "skill `{skill}` is in store `{target}`, which does not share its skills"
                    ),
                    format!(
                        "set `share_skills = true` in store `{target}`'s [store] table, or copy \
                         the skill into store `{}`",
                        store.name
                    ),
                ),
            ));
        }
        if other.skill(&skill.name).is_none() {
            return Some((
                RefKind::MissingSkill,
                Error::conflict(
                    at.clone(),
                    Location::file(other.root.join("skills").join(&skill.name))
                        .in_store(&other.name),
                    format!("store `{target}` has no skill `{}`", skill.name),
                    format!(
                        "check the name against store `{target}`'s skills/ directory, or remove \
                         the reference"
                    ),
                ),
            ));
        }
        None
    }
}

/// A reference [`StoreSet::checked`] rejects.
#[derive(Debug)]
pub struct RefProblem {
    /// The store holding the reference.
    pub store: String,
    /// Where the reference is.
    pub site: RefSite,
    /// What is wrong with it.
    pub kind: RefKind,
    /// The error [`StoreSet::new`] reports for it.
    pub error: Error,
}

/// Which kind of bad reference a [`RefProblem`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    /// It lands in another store (roadmap §6, the cross-store leak rule).
    CrossStore,
    /// A qualified skill whose store is not mounted, or has no such skill.
    MissingSkill,
}

/// A single reference in a store's `bindings.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefSite {
    /// `scopes.<scope>.extends`.
    ScopeExtends {
        /// The scope.
        scope: String,
    },
    /// One entry of `scopes.<scope>.skills`.
    ScopeSkill {
        /// The scope.
        scope: String,
        /// The reference.
        skill: SkillRef,
    },
    /// `repos.<repo>.scope`.
    RepoScope {
        /// The repo entry.
        repo: String,
    },
    /// One entry of `repos.<repo>.related`.
    RepoRelated {
        /// The repo entry.
        repo: String,
        /// The related entry name.
        name: String,
    },
    /// One entry of `repos.<repo>.skills`.
    RepoSkill {
        /// The repo entry.
        repo: String,
        /// The reference.
        skill: SkillRef,
    },
    /// One entry of `repos.<repo>.exclude`.
    RepoExclude {
        /// The repo entry.
        repo: String,
        /// The reference.
        skill: SkillRef,
    },
}

impl RefSite {
    /// The repo entry the reference belongs to, if any.
    pub fn repo(&self) -> Option<&str> {
        match self {
            RefSite::ScopeExtends { .. } | RefSite::ScopeSkill { .. } => None,
            RefSite::RepoScope { repo }
            | RefSite::RepoRelated { repo, .. }
            | RefSite::RepoSkill { repo, .. }
            | RefSite::RepoExclude { repo, .. } => Some(repo),
        }
    }

    /// Delete the reference from `store`, so the rest of the store can still resolve.
    pub fn remove_from(&self, store: &mut Store) {
        fn scope<'s>(store: &'s mut Store, name: &str) -> Option<&'s mut Scope> {
            store.scopes.iter_mut().find(|s| s.name == name)
        }
        fn repo<'s>(store: &'s mut Store, name: &str) -> Option<&'s mut RepoEntry> {
            store.repos.iter_mut().find(|r| r.name == name)
        }
        match self {
            RefSite::ScopeExtends { scope: name } => {
                if let Some(s) = scope(store, name) {
                    s.extends = None;
                }
            }
            RefSite::ScopeSkill { scope: name, skill } => {
                if let Some(s) = scope(store, name) {
                    s.skills.retain(|r| r != skill);
                }
            }
            RefSite::RepoScope { repo: name } => {
                if let Some(r) = repo(store, name) {
                    r.scope = None;
                }
            }
            RefSite::RepoRelated {
                repo: name,
                name: rel,
            } => {
                if let Some(r) = repo(store, name) {
                    r.related.retain(|n| n != rel);
                }
            }
            RefSite::RepoSkill { repo: name, skill } => {
                if let Some(r) = repo(store, name) {
                    r.skills.retain(|s| s != skill);
                }
            }
            RefSite::RepoExclude { repo: name, skill } => {
                if let Some(r) = repo(store, name) {
                    r.exclude.retain(|s| s != skill);
                }
            }
        }
    }
}

/// The first store in `stores` where `get` finds something, with that thing.
fn find_in<'a, T: 'a, S: std::borrow::Borrow<Store>>(
    stores: &'a [S],
    get: impl Fn(&'a Store) -> Option<&'a T>,
) -> Option<(&'a Store, &'a T)> {
    stores.iter().find_map(|st| {
        let st = st.borrow();
        get(st).map(|t| (st, t))
    })
}

/// Two store roots name the same directory (after resolving symlinks, when possible).
fn same_dir(a: &Path, b: &Path) -> bool {
    a == b || matches!((a.canonicalize(), b.canonicalize()), (Ok(a), Ok(b)) if a == b)
}
