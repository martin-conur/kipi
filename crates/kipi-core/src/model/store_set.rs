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
        let set = StoreSet { stores };
        set.check_unique_names()?;
        for store in &set.stores {
            set.check_references(store)?;
        }
        Ok(set)
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
    /// qualified skill reference names a mounted store that shares the skill.
    fn check_references(&self, store: &Store) -> Result<()> {
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
        let scope_ref = |name: &str, what: &str, at: &Location| -> Result<()> {
            if store.scope(name).is_some() {
                return Ok(());
            }
            match find_in(&others, |st| st.scope(name)) {
                Some((other, s)) => Err(foreign(what, name, at, other, &s.at)),
                None => Ok(()),
            }
        };

        for scope in &store.scopes {
            if let Some(parent) = &scope.extends {
                scope_ref(parent, "scope", &scope.at)?;
            }
            for skill in &scope.skills {
                self.check_skill_ref(store, &others, skill, &scope.at)?;
            }
        }
        for repo in &store.repos {
            if let Some(scope) = &repo.scope {
                scope_ref(scope, "scope", &repo.at)?;
            }
            for rel in &repo.related {
                if store.repo(rel).is_none()
                    && let Some((other, r)) = find_in(&others, |st| st.repo(rel))
                {
                    return Err(foreign("repo", rel, &repo.at, other, &r.at));
                }
            }
            for skill in repo.skills.iter().chain(&repo.exclude) {
                self.check_skill_ref(store, &others, skill, &repo.at)?;
            }
        }
        Ok(())
    }

    fn check_skill_ref(
        &self,
        store: &Store,
        others: &[&Store],
        skill: &SkillRef,
        at: &Location,
    ) -> Result<()> {
        let target = match skill.store.as_deref() {
            Some(name) if name != store.name => name,
            // Bare (or self-qualified): resolves in `store`. A name that exists only in
            // another store is a cross-store reference.
            _ => {
                if store.skill(&skill.name).is_some() {
                    return Ok(());
                }
                let Some((other, s)) = find_in(others, |st| st.skill(&skill.name)) else {
                    return Ok(());
                };
                let fix = if other.share_skills {
                    format!("reference it as `{}/{}`", other.name, skill.name)
                } else {
                    format!(
                        "store `{}` does not share its skills; copy the skill into store `{}` \
                         or remove the reference",
                        other.name, store.name
                    )
                };
                return Err(Error::conflict(
                    at.clone(),
                    s.at.clone(),
                    format!(
                        "skill `{}` is not in store `{}`; it belongs to store `{}`",
                        skill.name, store.name, other.name
                    ),
                    format!("bare skill names resolve within their own store; {fix}"),
                ));
            }
        };
        let Some(other) = self.store(target) else {
            return Err(Error::invalid(
                at.clone(),
                format!("skill `{skill}` names store `{target}`, which is not mounted"),
                format!(
                    "mount store `{target}` (--store, KIPI_STORE or config.toml `stores`) or \
                     remove the reference"
                ),
            ));
        };
        if !other.share_skills {
            return Err(Error::conflict(
                at.clone(),
                other.name_at.clone(),
                format!("skill `{skill}` is in store `{target}`, which does not share its skills"),
                format!(
                    "set `share_skills = true` in store `{target}`'s [store] table, or copy the \
                     skill into store `{}`",
                    store.name
                ),
            ));
        }
        if other.skill(&skill.name).is_none() {
            return Err(Error::conflict(
                at.clone(),
                Location::file(other.root.join("skills").join(&skill.name)).in_store(&other.name),
                format!("store `{target}` has no skill `{}`", skill.name),
                format!(
                    "check the name against store `{target}`'s skills/ directory, or remove the \
                     reference"
                ),
            ));
        }
        Ok(())
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
