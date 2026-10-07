//! `(stores, machine) -> Plan`. Pure; the only place policy lives (roadmap §6).
//!
//! Resolution runs in three passes over the [`StoreSet`]: validate each store's scopes
//! (`extends` parents and cycles, scope `skills`, `x-kipi.scope`), decide every repo's
//! scope, then build the registries and one [`RepoPlan`] per entry. Everything resolves
//! within the store owning the repo; the only thing that crosses stores is a qualified
//! `<store>/<skill>` from a store that shares its skills (already checked by
//! [`StoreSet::new`]). Every decision records its [`Source`].

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::error::{Error, Location, Result, closest};
use crate::model::{
    DEFAULT_AGENTS, DEFAULT_BUDGET_TOKENS, DEFAULT_GRANT, GrantMode, Machine, RepoEntry, Scope,
    SkillRef, Store, StoreSet,
};
use crate::plan::{
    AliasCollision, AlwaysOn, Decided, Grant, GrantDir, GrantKind, Plan, PlannedSkill, Registry,
    RegistryEntry, RelatedRepo, RepoPlan, SkillOrigin, Source, alias_key,
};

/// The scope a repo falls back to when nothing else places it (roadmap §6, rule 4).
pub const FALLBACK_SCOPE: &str = "personal";

/// Resolve every repo entry of every mounted store against this machine.
///
/// Errors (each with a hint): a scope extending an unknown scope or itself (a cycle),
/// an unknown explicit `scope`, a checkout under two scopes' roots at the same depth,
/// a skill missing from its store or used outside its `x-kipi.scope`, and a `related`
/// repo that is unknown, the repo itself, or outside the repo's scope chain (the leak rule).
pub fn resolve(stores: &StoreSet, machine: &Machine) -> Result<Plan> {
    let mut ctxs = Vec::with_capacity(stores.stores().len());
    for store in stores.stores() {
        ctxs.push(StoreCtx::new(stores, machine, store)?);
    }
    let mut plan = Plan {
        repos: Vec::new(),
        registries: Vec::new(),
    };
    for ctx in &ctxs {
        for scope in ctx.store.scopes.iter().filter(|s| s.registry) {
            plan.registries.push(ctx.registry(scope));
        }
        for repo in &ctx.store.repos {
            plan.repos.push(ctx.repo_plan(repo)?);
        }
    }
    plan.repos.sort_by(|a, b| a.name.cmp(&b.name));
    plan.registries.sort_by(|a, b| a.scope.cmp(&b.scope));
    Ok(plan)
}

/// Whether `pattern` (a `remote_patterns` entry) matches the normalized `remote`.
/// Patterns are prefixes of the normalized form: `github.com/acme-org/`.
pub fn remote_pattern_matches(pattern: &str, remote: &str) -> bool {
    remote.starts_with(pattern)
}

/// One store, with its scope chains and every repo's resolved scope.
struct StoreCtx<'a> {
    set: &'a StoreSet,
    machine: &'a Machine,
    store: &'a Store,
    /// Scope name → its `extends` chain, top-most ancestor first, ending with itself.
    chains: BTreeMap<&'a str, Vec<&'a str>>,
    /// Repo entry name → its resolved scope.
    scopes: BTreeMap<&'a str, Decided<Option<String>>>,
    /// Scope name → its own `skills`, resolved.
    scope_skills: BTreeMap<&'a str, Vec<PlannedSkill>>,
}

impl<'a> StoreCtx<'a> {
    fn new(set: &'a StoreSet, machine: &'a Machine, store: &'a Store) -> Result<Self> {
        let mut ctx = StoreCtx {
            set,
            machine,
            store,
            chains: BTreeMap::new(),
            scopes: BTreeMap::new(),
            scope_skills: BTreeMap::new(),
        };
        for scope in &store.scopes {
            let chain = ctx.extends_chain(scope)?;
            ctx.chains.insert(&scope.name, chain);
        }
        for skill in &store.skills {
            if let Some(owner) = &skill.scope
                && store.scope(owner).is_none()
            {
                return Err(Error::invalid(
                    skill.at.clone(),
                    format!(
                        "skill `{}` sets `x-kipi.scope: {owner}`, but store `{}` has no scope `{owner}`",
                        skill.name, store.name
                    ),
                    unknown_hint(owner, &scope_names(store), "scope"),
                ));
            }
        }
        for scope in &store.scopes {
            let rule = format!("scopes.{}.skills", scope.name);
            let mut skills = Vec::with_capacity(scope.skills.len());
            for skill in &scope.skills {
                skills.push(ctx.skill(
                    skill,
                    &scope.at,
                    &rule,
                    Binder::Scope(Some(&scope.name)),
                    SkillOrigin::Scope(scope.name.clone()),
                )?);
            }
            ctx.scope_skills.insert(&scope.name, skills);
        }
        for repo in &store.repos {
            let scope = ctx.repo_scope(repo)?;
            ctx.scopes.insert(&repo.name, scope);
        }
        Ok(ctx)
    }

    /// `scope`'s chain, top-most ancestor first. Unknown parents and cycles are errors.
    fn extends_chain(&self, scope: &'a Scope) -> Result<Vec<&'a str>> {
        let mut chain = vec![scope.name.as_str()];
        let mut current = scope;
        while let Some(parent) = &current.extends {
            let Some(next) = self.store.scope(parent) else {
                return Err(Error::invalid(
                    current.at.clone(),
                    format!(
                        "scope `{}` extends `{parent}`, which is not a scope in store `{}`",
                        current.name, self.store.name
                    ),
                    unknown_hint(parent, &scope_names(self.store), "scope"),
                ));
            };
            if let Some(pos) = chain.iter().position(|n| *n == next.name) {
                let mut cycle = chain[pos..].to_vec();
                cycle.push(&next.name);
                return Err(Error::invalid(
                    next.at.clone(),
                    format!("`extends` forms a cycle: {}", cycle.join(" → ")),
                    "remove `extends` from one scope in the cycle; the top-most scope extends nothing",
                ));
            }
            chain.push(&next.name);
            current = next;
        }
        chain.reverse();
        Ok(chain)
    }

    fn chain(&self, scope: Option<&str>) -> &[&'a str] {
        scope
            .and_then(|s| self.chains.get(s))
            .map_or(&[], Vec::as_slice)
    }

    /// Two scopes are in one scope chain: equal, or one is an ancestor of the other.
    /// Two unscoped repos (of this store) count as one chain; unscoped and scoped do not.
    fn same_chain(&self, a: Option<&str>, b: Option<&str>) -> bool {
        match (a, b) {
            (None, None) => true,
            (Some(a), Some(b)) => {
                self.chain(Some(a)).contains(&b) || self.chain(Some(b)).contains(&a)
            }
            _ => false,
        }
    }

    fn scope_of(&self, repo: &str) -> Option<&str> {
        self.scopes.get(repo).and_then(|d| d.value.as_deref())
    }

    /// Explicit `scope` > deepest root containing the checkout > `remote_patterns` for a
    /// repo not cloned here > `personal` > unscoped.
    fn repo_scope(&self, repo: &RepoEntry) -> Result<Decided<Option<String>>> {
        let store = self.store;
        if let Some(name) = &repo.scope {
            if store.scope(name).is_none() {
                return Err(Error::invalid(
                    repo.at.clone(),
                    format!(
                        "repo `{}` sets `scope = \"{name}\"`, but store `{}` has no such scope",
                        repo.name, store.name
                    ),
                    unknown_hint(name, &scope_names(store), "scope"),
                ));
            }
            let source = Source::at(&repo.at, format!("repos.{}.scope", repo.name));
            return Ok(Decided::new(Some(name.clone()), source));
        }
        if let Some(path) = self.machine.repo_path(&repo.name) {
            if let Some(found) = self.scope_by_path(repo, path)? {
                return Ok(found);
            }
        } else if let Some(remote) = &repo.remote
            && let Some(found) = self.scope_by_remote(remote.as_str())
        {
            return Ok(found);
        }
        if let Some(scope) = store.scope(FALLBACK_SCOPE) {
            let source = Source::at(&scope.at, format!("fallback: scope `{FALLBACK_SCOPE}`"));
            return Ok(Decided::new(Some(scope.name.clone()), source));
        }
        Ok(Decided::new(
            None,
            Source::builtin(format!(
                "unscoped: no explicit scope, no root contains it, no `{FALLBACK_SCOPE}` scope"
            )),
        ))
    }

    /// The scope whose root (after `machine.toml [roots]`) is the deepest ancestor of `path`.
    fn scope_by_path(
        &self,
        repo: &RepoEntry,
        path: &Path,
    ) -> Result<Option<Decided<Option<String>>>> {
        let mut best: Option<(usize, &Scope, Source)> = None;
        let mut tied: Vec<&Scope> = Vec::new();
        for scope in &self.store.scopes {
            let (roots, source) = match self.machine.roots_for(&scope.name) {
                Some(r) => (
                    &r.roots,
                    Source::at(&r.at, format!("machine.toml [roots] {}", scope.name)),
                ),
                None => (
                    &scope.roots,
                    Source::at(&scope.at, format!("scopes.{}.roots", scope.name)),
                ),
            };
            let Some(depth) = roots
                .iter()
                .filter(|root| path.starts_with(root))
                .map(|root| root.components().count())
                .max()
            else {
                continue;
            };
            match &best {
                Some((d, _, _)) if *d > depth => {}
                Some((d, _, _)) if *d == depth => tied.push(scope),
                _ => {
                    best = Some((depth, scope, source));
                    tied.clear();
                }
            }
        }
        let Some((_, scope, source)) = best else {
            return Ok(None);
        };
        if !tied.is_empty() {
            let mut all: Vec<&str> = std::iter::once(scope)
                .chain(tied)
                .map(|s| s.name.as_str())
                .collect();
            all.sort_unstable();
            let all: Vec<String> = all.iter().map(|s| format!("`{s}`")).collect();
            return Err(Error::invalid(
                repo.at.clone(),
                format!(
                    "the checkout of `{}` at {} lies under roots of scopes {} at the same depth",
                    repo.name,
                    path.display(),
                    all.join(", ")
                ),
                format!(
                    "set `scope = \"…\"` in [repos.{}], or give the scopes different roots",
                    repo.name
                ),
            ));
        }
        Ok(Some(Decided::new(Some(scope.name.clone()), source)))
    }

    /// For a repo not cloned here: the one scope with the longest `remote_patterns`
    /// entry matching its remote. No match, or a tie between scopes, decides nothing.
    fn scope_by_remote(&self, remote: &str) -> Option<Decided<Option<String>>> {
        let mut best: Option<(usize, &Scope)> = None;
        let mut tied = false;
        for scope in &self.store.scopes {
            let Some(len) = scope
                .remote_patterns
                .iter()
                .filter(|p| remote_pattern_matches(p, remote))
                .map(String::len)
                .max()
            else {
                continue;
            };
            match best {
                Some((l, _)) if l > len => {}
                Some((l, _)) if l == len => tied = true,
                _ => {
                    best = Some((len, scope));
                    tied = false;
                }
            }
        }
        let (_, scope) = best.filter(|_| !tied)?;
        let rule = format!("scopes.{}.remote_patterns (not cloned here)", scope.name);
        Some(Decided::new(
            Some(scope.name.clone()),
            Source::at(&scope.at, rule),
        ))
    }

    /// Resolve a skill reference. A skill of this store with `x-kipi.scope` must be bound
    /// in that scope or a descendant of it (see [`Binder`]).
    fn skill(
        &self,
        reference: &SkillRef,
        at: &Location,
        rule: &str,
        binder: Binder<'_>,
        origin: SkillOrigin,
    ) -> Result<PlannedSkill> {
        let store = self.store;
        let Some((owner, skill)) = self.set.skill(&store.name, reference) else {
            // Qualified references were checked completely by `StoreSet::new`, so this
            // is a name missing from the repo's own store.
            return Err(Error::invalid(
                at.clone(),
                format!(
                    "skill `{reference}` (in `{rule}`) is not in store `{}`",
                    store.name
                ),
                match closest(
                    &reference.name,
                    store.skills.iter().map(|s| s.name.as_str()),
                ) {
                    Some(s) => format!("did you mean `{s}`?"),
                    None => format!(
                        "create {}/skills/{}/SKILL.md (`kipi add skill {}`) or remove the reference",
                        store.root.display(),
                        reference.name,
                        reference.name
                    ),
                },
            ));
        };
        let shared = owner.name != store.name;
        if !shared
            && let (Binder::Scope(binder_scope), Some(owner_scope)) = (binder, &skill.scope)
            && !self.chain(binder_scope).contains(&owner_scope.as_str())
        {
            let bound = match binder_scope {
                Some(s) => format!("scope `{s}`, which does not extend it"),
                None => "an unscoped repo".to_owned(),
            };
            return Err(Error::conflict(
                at.clone(),
                skill.at.clone(),
                format!(
                    "skill `{}` belongs to scope `{owner_scope}` (x-kipi.scope) but `{rule}` binds it to {bound}",
                    skill.name
                ),
                format!(
                    "bind it in scope `{owner_scope}` or a scope that extends it, or change its `x-kipi.scope`"
                ),
            ));
        }
        Ok(PlannedSkill {
            name: skill.name.clone(),
            store: owner.name.clone(),
            reference: SkillRef {
                store: shared.then(|| owner.name.clone()),
                name: skill.name.clone(),
            },
            dir: skill.dir.clone(),
            origin,
            source: Source::at(at, rule),
        })
    }

    fn registry(&self, scope: &Scope) -> Registry {
        let mut scopes: Vec<String> = self
            .store
            .scopes
            .iter()
            .filter(|s| self.chain(Some(&s.name)).contains(&scope.name.as_str()))
            .map(|s| s.name.clone())
            .collect();
        scopes.sort();
        let mut entries: Vec<RegistryEntry> = self
            .store
            .repos
            .iter()
            .filter_map(|repo| {
                let repo_scope = self.scope_of(&repo.name)?;
                scopes
                    .iter()
                    .any(|s| s == repo_scope)
                    .then(|| self.registry_entry(repo, repo_scope))
            })
            .collect();
        entries.sort_by(|a, b| a.name.cmp(&b.name));

        let mut claims: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        for entry in &entries {
            for key in &entry.keys {
                claims.entry(key).or_default().push(entry.name.clone());
            }
        }
        let collisions = claims
            .into_iter()
            .filter(|(_, repos)| repos.len() > 1)
            .map(|(key, repos)| AliasCollision {
                key: key.to_owned(),
                repos,
            })
            .collect();
        Registry {
            scope: scope.name.clone(),
            store: self.store.name.clone(),
            scopes,
            entries,
            collisions,
            source: Source::at(&scope.at, format!("scopes.{}.registry", scope.name)),
        }
    }

    fn registry_entry(&self, repo: &RepoEntry, scope: &str) -> RegistryEntry {
        let mut keys = Vec::new();
        for key in std::iter::once(&repo.name)
            .chain(&repo.aliases.value)
            .map(|a| alias_key(a))
        {
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        RegistryEntry {
            name: repo.name.clone(),
            aliases: repo.aliases.value.clone(),
            aliases_draft: repo.aliases.is_draft,
            summary: repo.summary.as_ref().map(|s| s.value.clone()),
            summary_draft: repo.summary_is_draft(),
            scope: scope.to_owned(),
            local: self.machine.repo_path(&repo.name).map(Path::to_path_buf),
            notes: repo.notes.clone(),
            keys,
            source: Source::at(&repo.at, format!("repos.{}", repo.name)),
        }
    }

    fn repo_plan(&self, repo: &RepoEntry) -> Result<RepoPlan> {
        let store = self.store;
        let scope = self.scopes[repo.name.as_str()].clone();
        let chain: Vec<&Scope> = self
            .chain(scope.value.as_deref())
            .iter()
            .filter_map(|s| store.scope(s))
            .collect();

        // Skills: the chain top-down, then the repo's own, minus `exclude`.
        let mut skills: Vec<PlannedSkill> = Vec::new();
        for s in &chain {
            for skill in &self.scope_skills[s.name.as_str()] {
                push_skill(&mut skills, skill.clone());
            }
        }
        let rule = format!("repos.{}.skills", repo.name);
        for r in &repo.skills {
            let binder = Binder::Scope(scope.value.as_deref());
            let skill = self.skill(r, &repo.at, &rule, binder, SkillOrigin::Repo)?;
            push_skill(&mut skills, skill);
        }
        let rule = format!("repos.{}.exclude", repo.name);
        let mut excluded: Vec<PlannedSkill> = Vec::new();
        for r in &repo.exclude {
            push_skill(
                &mut excluded,
                self.skill(r, &repo.at, &rule, Binder::Exclude, SkillOrigin::Repo)?,
            );
        }
        skills.retain(|s| !excluded.iter().any(|x| same_skill(s, x)));

        // Always-on: each scope's files top-down, the registry, the this-repo section.
        let registry = chain.iter().rev().find(|s| s.registry).map(|s| {
            Decided::new(
                s.name.clone(),
                Source::at(&s.at, format!("scopes.{}.registry", s.name)),
            )
        });
        let mut always = Vec::new();
        for s in &chain {
            for file in &s.always {
                always.push(AlwaysOn::File {
                    path: store.root.join(file),
                    scope: s.name.clone(),
                    source: Source::at(&s.at, format!("scopes.{}.always", s.name)),
                });
            }
        }
        if let Some(r) = &registry {
            always.push(AlwaysOn::Registry {
                scope: r.value.clone(),
                source: r.source.clone(),
            });
        }
        always.push(AlwaysOn::ThisRepo);

        let related = self.related(repo, scope.value.as_deref())?;
        let agents = self.setting(
            repo,
            &chain,
            "agents",
            |r| r.agents.clone(),
            |s| s.agents.clone(),
            |st| st.defaults.agents.clone(),
            DEFAULT_AGENTS.to_vec(),
        );
        let budget_tokens = self.setting(
            repo,
            &chain,
            "budget_tokens",
            |r| r.budget_tokens,
            |s| s.budget_tokens,
            |st| st.defaults.budget_tokens,
            DEFAULT_BUDGET_TOKENS,
        );
        let mode = self.setting(
            repo,
            &chain,
            "grant",
            |r| r.grant,
            |s| s.grant,
            |st| st.defaults.grant,
            DEFAULT_GRANT,
        );
        let grant = self.grant(
            repo,
            scope.value.as_deref(),
            mode,
            &skills,
            &related,
            registry.as_ref(),
        );

        Ok(RepoPlan {
            name: repo.name.clone(),
            store: store.name.clone(),
            at: repo.at.clone(),
            path: self.machine.repo_path(&repo.name).map(Path::to_path_buf),
            chain: chain.iter().map(|s| s.name.clone()).collect(),
            scope,
            skills,
            excluded,
            always,
            registry,
            related,
            grant,
            agents,
            budget_tokens,
        })
    }

    /// `related` entries: known repos of this store, not the repo itself, in its scope chain.
    fn related(&self, repo: &RepoEntry, scope: Option<&str>) -> Result<Vec<RelatedRepo>> {
        let store = self.store;
        let mut out = Vec::with_capacity(repo.related.len());
        for name in &repo.related {
            let Some(target) = store.repo(name) else {
                return Err(Error::invalid(
                    repo.at.clone(),
                    format!(
                        "repo `{}` is related to `{name}`, which is not a repo entry in store `{}`",
                        repo.name, store.name
                    ),
                    unknown_hint(name, &repo_names(store), "repo"),
                ));
            };
            if target.name == repo.name {
                return Err(Error::invalid(
                    repo.at.clone(),
                    format!("repo `{}` lists itself in `related`", repo.name),
                    format!("remove `{name}` from [repos.{name}] related"),
                ));
            }
            let target_scope = self.scope_of(name);
            if !self.same_chain(scope, target_scope) {
                return Err(Error::conflict(
                    repo.at.clone(),
                    target.at.clone(),
                    format!(
                        "repo `{}` ({}) is related to `{name}` ({}), outside its scope chain",
                        repo.name,
                        describe_scope(scope),
                        describe_scope(target_scope)
                    ),
                    format!(
                        "related repos must share a scope chain (same scope, an ancestor or a \
                         descendant), so nothing leaks across scopes; remove `{name}` from \
                         `related`, or set `scope = \"…\"` on one of the entries if it is misclassified"
                    ),
                ));
            }
            out.push(RelatedRepo {
                name: name.clone(),
                scope: target_scope.map(str::to_owned),
                path: self.machine.repo_path(name).map(Path::to_path_buf),
                notes: target.notes.clone(),
                source: Source::at(&repo.at, format!("repos.{}.related", repo.name)),
            });
        }
        Ok(out)
    }

    /// Precedence: the repo, then each scope from the repo's own up, then the store's
    /// `[defaults]`, then the built-in default.
    #[allow(clippy::too_many_arguments)]
    fn setting<T>(
        &self,
        repo: &RepoEntry,
        chain: &[&Scope],
        key: &str,
        on_repo: impl Fn(&RepoEntry) -> Option<T>,
        on_scope: impl Fn(&Scope) -> Option<T>,
        on_defaults: impl Fn(&Store) -> Option<T>,
        builtin: T,
    ) -> Decided<T> {
        if let Some(v) = on_repo(repo) {
            return Decided::new(
                v,
                Source::at(&repo.at, format!("repos.{}.{key}", repo.name)),
            );
        }
        for s in chain.iter().rev() {
            if let Some(v) = on_scope(s) {
                return Decided::new(v, Source::at(&s.at, format!("scopes.{}.{key}", s.name)));
            }
        }
        match (on_defaults(self.store), &self.store.defaults.at) {
            (Some(v), Some(at)) => Decided::new(v, Source::at(at, format!("defaults.{key}"))),
            (Some(v), None) => Decided::new(v, Source::builtin(format!("defaults.{key}"))),
            (None, _) => Decided::new(builtin, Source::builtin(format!("built-in default {key}"))),
        }
    }

    /// The read grant (roadmap §6): store dirs, shared skills' stores, then related
    /// repos (`related`) or every registry repo plus related repos (`registry`). Only
    /// repos cloned here and in the repo's scope chain are granted; `none` grants nothing.
    fn grant(
        &self,
        repo: &RepoEntry,
        scope: Option<&str>,
        mode: Decided<GrantMode>,
        skills: &[PlannedSkill],
        related: &[RelatedRepo],
        registry: Option<&Decided<String>>,
    ) -> Grant {
        let mut dirs: Vec<GrantDir> = Vec::new();
        if mode.value == GrantMode::None {
            return Grant { mode, dirs };
        }
        let store = self.store;
        let push = |dirs: &mut Vec<GrantDir>, dir: GrantDir| {
            if !dirs.iter().any(|d| d.path == dir.path) {
                dirs.push(dir);
            }
        };
        let own = Source::builtin(format!("grant: store `{}` dirs", store.name));
        push(
            &mut dirs,
            GrantDir {
                path: store.root.join("repos"),
                kind: GrantKind::StoreRepos,
                source: own.clone(),
            },
        );
        push(
            &mut dirs,
            GrantDir {
                path: store.root.join("skills"),
                kind: GrantKind::StoreSkills {
                    store: store.name.clone(),
                },
                source: own,
            },
        );
        for skill in skills.iter().filter(|s| s.is_shared(&store.name)) {
            let owner =
                (self.set.store(&skill.store)).expect("resolved skills come from mounted stores");
            push(
                &mut dirs,
                GrantDir {
                    path: owner.root.join("skills"),
                    kind: GrantKind::StoreSkills {
                        store: owner.name.clone(),
                    },
                    source: skill.source.clone(),
                },
            );
        }

        let mut repos: BTreeMap<&str, Source> = BTreeMap::new();
        for r in related {
            repos.entry(&r.name).or_insert_with(|| r.source.clone());
        }
        if mode.value == GrantMode::Registry
            && let Some(registry) = registry
        {
            let members: BTreeSet<&str> = self.chain_members(&registry.value);
            for name in members {
                repos.entry(name).or_insert_with(|| registry.source.clone());
            }
        }
        for (name, source) in repos {
            let Some(path) = self.machine.repo_path(name) else {
                continue;
            };
            if name == repo.name || !self.same_chain(scope, self.scope_of(name)) {
                continue;
            }
            push(
                &mut dirs,
                GrantDir {
                    path: path.to_path_buf(),
                    kind: GrantKind::Repo {
                        name: name.to_owned(),
                    },
                    source,
                },
            );
        }
        Grant { mode, dirs }
    }

    /// Repos whose scope is `scope` or one of its descendants.
    fn chain_members(&self, scope: &str) -> BTreeSet<&'a str> {
        self.store
            .repos
            .iter()
            .filter(|r| {
                self.scope_of(&r.name)
                    .is_some_and(|s| self.chain(Some(s)).contains(&scope))
            })
            .map(|r| r.name.as_str())
            .collect()
    }
}

/// Who a skill reference binds the skill to, for the `x-kipi.scope` check.
#[derive(Clone, Copy)]
enum Binder<'s> {
    /// A scope's `skills`, or a repo's `skills` (`None`: the repo is unscoped).
    Scope(Option<&'s str>),
    /// A repo's `exclude`: removing a skill never leaks it, so nothing is checked.
    Exclude,
}

fn same_skill(a: &PlannedSkill, b: &PlannedSkill) -> bool {
    a.store == b.store && a.name == b.name
}

/// Append unless the same skill (store and name) is already listed: first mention wins.
fn push_skill(list: &mut Vec<PlannedSkill>, skill: PlannedSkill) {
    if !list.iter().any(|s| same_skill(s, &skill)) {
        list.push(skill);
    }
}

fn describe_scope(scope: Option<&str>) -> String {
    match scope {
        Some(s) => format!("scope `{s}`"),
        None => "unscoped".to_owned(),
    }
}

fn scope_names(store: &Store) -> Vec<&str> {
    store.scopes.iter().map(|s| s.name.as_str()).collect()
}

fn repo_names(store: &Store) -> Vec<&str> {
    store.repos.iter().map(|r| r.name.as_str()).collect()
}

/// Hint for a reference to an unknown `what` (`scope`, `repo`): the closest name, else
/// the known names, else how to declare one.
fn unknown_hint(name: &str, candidates: &[&str], what: &str) -> String {
    match closest(name, candidates.iter().copied()) {
        Some(s) => format!("did you mean `{s}`?"),
        None if candidates.is_empty() => {
            format!("declare [{what}s.{name}] or remove the reference")
        }
        None => format!(
            "known {what}s: {}",
            candidates
                .iter()
                .map(|c| format!("`{c}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}
