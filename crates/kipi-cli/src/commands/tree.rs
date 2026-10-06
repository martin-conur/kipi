//! `kipi tree [--repo r] [--skill s] [--store n] [--budget] [--machine]`: every mounted
//! store → its scopes → their repos → skills and related repos, with provenance markers,
//! always-on tokens, registry sizes and sync state (roadmap §7). `--json` prints the same
//! structure (docs/json.md).

use std::io::IsTerminal;

use kipi_core::apply;
use kipi_core::config::Env;
use kipi_core::error::{Error, closest};
use kipi_core::git;
use kipi_core::model::{Scope, SkillRef, Store, StoreSet};
use kipi_core::plan::{PlannedSkill, RepoPlan, SkillOrigin};
use kipi_core::render::registry;
use kipi_core::sync::{self, Selection};

use super::Loaded;
use crate::error::Failure;
use crate::output::status::State;
use crate::output::tree::{
    self, Access, Display, Mark, RegistrySize, RelatedNode, RepoNode, ScopeNode, SkillNode,
    StoreNode, Tree,
};
use crate::output::{Output, json};

/// The command's arguments.
#[derive(Debug, Clone, Copy, Default)]
pub struct Args<'a> {
    /// `--repo`.
    pub repo: Option<&'a str>,
    /// `--skill`.
    pub skill: Option<&'a str>,
    /// `--store`.
    pub store: Option<&'a str>,
    /// `--budget`.
    pub budget: bool,
    /// `--machine`.
    pub machine: bool,
}

/// Build the tree, print it.
pub fn run(env: &Env, loaded: &Loaded, args: &Args, out: &Output) -> Result<(), Failure> {
    let tree = build(env, loaded, args)?;
    loaded.save_caches(out);
    if out.json {
        let text = json::tree(&tree).map_err(|e| Failure::json("the tree", e))?;
        return Ok(out.print(&text)?);
    }
    if tree.stores.is_empty()
        && let Some(skill) = args.skill
    {
        return Ok(out.print(&format!("no repo uses skill `{skill}`\n"))?);
    }
    let display = Display {
        ascii: !std::io::stdout().is_terminal(),
        budget: args.budget,
        machine: args.machine,
    };
    Ok(out.print(&tree::render(
        &tree,
        display,
        &out.paint,
        out.home.as_deref(),
    ))?)
}

/// What a filter keeps.
struct Filter<'a> {
    repo: Option<&'a str>,
    skill: Option<SkillRef>,
}

impl Filter<'_> {
    fn active(&self) -> bool {
        self.repo.is_some() || self.skill.is_some()
    }

    fn keeps(&self, repo: &RepoPlan) -> bool {
        self.repo.is_none_or(|r| r == repo.name)
            && self
                .skill
                .as_ref()
                .is_none_or(|s| repo.skills.iter().any(|p| uses(p, s)))
    }
}

/// `planned` is the skill `wanted` names: same name, and same store when qualified.
fn uses(planned: &PlannedSkill, wanted: &SkillRef) -> bool {
    planned.name == wanted.name && wanted.store.as_ref().is_none_or(|s| *s == planned.store)
}

/// The tree of `loaded` restricted by `args`. Validates the filters first.
pub fn build(env: &Env, loaded: &Loaded, args: &Args) -> Result<Tree, Failure> {
    let set = &loaded.stores;
    if let Some(name) = args.store {
        check_store(set, name)?;
    }
    if let Some(name) = args.repo {
        loaded.repo(name)?;
    }
    let filter = Filter {
        repo: args.repo,
        skill: args.skill.map(|s| check_skill(set, s)).transpose()?,
    };
    let stores: Vec<&Store> = set
        .stores()
        .iter()
        .filter(|s| args.store.is_none_or(|n| n == s.name))
        .collect();
    let shown: Vec<&RepoPlan> = loaded
        .plan
        .repos
        .iter()
        .filter(|r| stores.iter().any(|s| s.name == r.store) && filter.keeps(r))
        .collect();
    let states = sync_states(env, loaded, &shown)?;
    let mut tree = Tree {
        tokenizer: loaded.caches.tokens.tokenizer().name(),
        stores: Vec::new(),
    };
    for store in stores {
        let ctx = Ctx {
            env,
            loaded,
            store,
            shown: &shown,
            states: &states,
            filter: &filter,
        };
        let mut scopes = Vec::new();
        for scope in store.scopes.iter().filter(|s| s.extends.is_none()) {
            if let Some(node) = ctx.scope(&scope.name)? {
                scopes.push(node);
            }
        }
        let unscoped: Vec<RepoNode> = shown
            .iter()
            .filter(|r| r.store == store.name && r.scope.value.is_none())
            .map(|r| ctx.repo(r))
            .collect();
        if filter.active() && scopes.is_empty() && unscoped.is_empty() {
            continue;
        }
        tree.stores.push(StoreNode {
            name: store.name.clone(),
            root: store.root.clone(),
            git: git::status(&store.root),
            shares_skills: store.share_skills,
            scopes,
            unscoped,
        });
    }
    Ok(tree)
}

/// Each shown repo's state, from a dry-run sync (the same diff `kipi status` runs).
fn sync_states(
    env: &Env,
    loaded: &Loaded,
    shown: &[&RepoPlan],
) -> Result<Vec<(String, State, sync::AlwaysOnSize)>, Failure> {
    let opts = sync::Options {
        apply: apply::Options {
            dry_run: true,
            force: false,
        },
        home: Some(env.home.clone()),
        ..sync::Options::default()
    };
    let names = shown.iter().map(|r| r.name.clone()).collect();
    let outcome = sync::run(
        &loaded.stores,
        &loaded.machine,
        &loaded.plan,
        &Selection::Only(names),
        &opts,
        &loaded.caches,
    )?;
    Ok(outcome
        .repos
        .iter()
        .filter_map(|o| Some((o.name.clone(), State::of(o), o.always_on?)))
        .collect())
}

struct Ctx<'a> {
    env: &'a Env,
    loaded: &'a Loaded,
    store: &'a Store,
    shown: &'a [&'a RepoPlan],
    states: &'a [(String, State, sync::AlwaysOnSize)],
    filter: &'a Filter<'a>,
}

impl Ctx<'_> {
    /// The node of scope `name` and the scopes extending it; `None` when a filter
    /// leaves nothing under it.
    fn scope(&self, name: &str) -> Result<Option<ScopeNode>, Failure> {
        let scope = self.store.scope(name).expect("scopes come from this store");
        let mut children = Vec::new();
        for child in self
            .store
            .scopes
            .iter()
            .filter(|s| s.extends.as_deref() == Some(name))
        {
            if let Some(node) = self.scope(&child.name)? {
                children.push(node);
            }
        }
        let repos: Vec<RepoNode> = self
            .shown
            .iter()
            .filter(|r| r.store == self.store.name && r.scope.value.as_deref() == Some(name))
            .map(|r| self.repo(r))
            .collect();
        if self.filter.active() && repos.is_empty() && children.is_empty() {
            return Ok(None);
        }
        let plan = &self.loaded.plan;
        let home = Some(self.env.home.as_path());
        let tokens = &self.loaded.caches.tokens;
        let registry = plan
            .registry(name)
            .filter(|_| scope.registry)
            .map(|r| RegistrySize {
                repos: r.entries.len(),
                tokens: tokens.count(&registry::render(r, home)),
            });
        Ok(Some(ScopeNode {
            name: name.to_owned(),
            roots: scope.roots.clone(),
            machine_roots: self.loaded.machine.roots_for(name).map(|r| r.roots.clone()),
            extends: scope.extends.clone(),
            always_on: self.scope_always_on(name)?,
            skills: scope.skills.iter().map(ToString::to_string).collect(),
            registry,
            scopes: children,
            repos,
        }))
    }

    /// The chain's conventions, the nearest registry and the chain's skill descriptions.
    fn scope_always_on(&self, name: &str) -> Result<u32, Failure> {
        let tokens = &self.loaded.caches.tokens;
        let chain = self.chain(name);
        let mut total = 0u32;
        for scope in &chain {
            for file in &scope.always {
                let path = self.store.root.join(file);
                let n = tokens.count_file(&path).map_err(|e| {
                    Error::io(
                        &path,
                        e,
                        format!(
                            "create the file or remove it from `scopes.{}.always`",
                            scope.name
                        ),
                    )
                })?;
                total = total.saturating_add(n);
            }
        }
        let home = Some(self.env.home.as_path());
        if let Some(r) = chain
            .iter()
            .rev()
            .find(|s| s.registry)
            .and_then(|s| self.loaded.plan.registry(&s.name))
        {
            total = total.saturating_add(tokens.count(&registry::render(r, home)));
        }
        let mut seen = Vec::new();
        for skill in chain.iter().flat_map(|s| &s.skills) {
            let Some((store, s)) = self.loaded.stores.skill(&self.store.name, skill) else {
                continue;
            };
            if seen.contains(&(&store.name, &s.name)) {
                continue;
            }
            seen.push((&store.name, &s.name));
            if let Some(d) = &s.description {
                total = total.saturating_add(tokens.count(d));
            }
        }
        Ok(total)
    }

    /// Scope `name`'s `extends` chain in this store, top-most first.
    fn chain(&self, name: &str) -> Vec<&Scope> {
        let mut chain = Vec::new();
        let mut next = self.store.scope(name);
        while let Some(scope) = next {
            if chain.iter().any(|s: &&Scope| s.name == scope.name) {
                break;
            }
            chain.push(scope);
            next = scope.extends.as_deref().and_then(|p| self.store.scope(p));
        }
        chain.reverse();
        chain
    }

    fn repo(&self, plan: &RepoPlan) -> RepoNode {
        let (_, state, always_on) = self
            .states
            .iter()
            .find(|(n, ..)| *n == plan.name)
            .expect("every shown repo was diffed");
        let entry = self.store.repo(&plan.name);
        RepoNode {
            name: plan.name.clone(),
            path: plan.path.clone(),
            always_on: *always_on,
            state: *state,
            notes: entry.and_then(|e| e.notes.clone()),
            draft_summary: entry.is_some_and(|e| e.summary_is_draft()),
            skills: self.skills(plan),
            related: plan
                .related
                .iter()
                .map(|r| RelatedNode {
                    name: r.name.clone(),
                    access: match &r.path {
                        None => Access::NotCloned,
                        Some(p) if plan.grant.contains(p) => Access::Granted,
                        Some(_) => Access::NotGranted,
                    },
                })
                .collect(),
            show_skills: self.filter.active()
                || !plan.excluded.is_empty()
                || plan.skills.iter().any(|s| s.origin == SkillOrigin::Repo),
        }
    }

    /// Effective and excluded skills in the order the chain and the repo bind them, so an
    /// excluded skill shows where it would have been.
    fn skills(&self, plan: &RepoPlan) -> Vec<SkillNode> {
        let set = &self.loaded.stores;
        let mut bound: Vec<&SkillRef> = self
            .chain(plan.scope.value.as_deref().unwrap_or_default())
            .iter()
            .flat_map(|s| &s.skills)
            .collect();
        if let Some(entry) = self.store.repo(&plan.name) {
            bound.extend(&entry.skills);
        }
        let all = plan.skills.iter().chain(&plan.excluded);
        let mut out: Vec<SkillNode> = Vec::new();
        let mut placed: Vec<&PlannedSkill> = Vec::new();
        let place = |s: &'_ PlannedSkill, out: &mut Vec<SkillNode>| {
            let mark = if plan.excluded.iter().any(|e| same(e, s)) {
                Mark::Excluded
            } else if s.origin == SkillOrigin::Repo {
                Mark::Explicit
            } else {
                Mark::Inherited
            };
            out.push(SkillNode {
                reference: s.reference.to_string(),
                mark,
            });
        };
        for r in bound {
            let Some((store, skill)) = set.skill(&self.store.name, r) else {
                continue;
            };
            let found = all
                .clone()
                .find(|p| p.store == store.name && p.name == skill.name);
            if let Some(p) = found
                && !placed.iter().any(|q| same(q, p))
            {
                placed.push(p);
                place(p, &mut out);
            }
        }
        for p in all {
            if !placed.iter().any(|q| same(q, p)) {
                placed.push(p);
                place(p, &mut out);
            }
        }
        out
    }
}

fn same(a: &PlannedSkill, b: &PlannedSkill) -> bool {
    a.store == b.store && a.name == b.name
}

fn check_store(set: &StoreSet, name: &str) -> Result<(), Failure> {
    if set.store(name).is_some() {
        return Ok(());
    }
    let names = set.stores().iter().map(|s| s.name.as_str());
    let hint = match closest(name, names.clone()) {
        Some(near) => format!("did you mean `{near}`?"),
        None => format!(
            "pass a mounted store's name: {}",
            names.collect::<Vec<_>>().join(", ")
        ),
    };
    Err(Failure::user(format!("no mounted store `{name}`"), hint))
}

/// `raw` as a skill reference naming a skill some mounted store has.
fn check_skill(set: &StoreSet, raw: &str) -> Result<SkillRef, Failure> {
    let wanted: SkillRef = raw.parse().map_err(|_| {
        Failure::user(
            format!("`{raw}` is not a skill reference"),
            "pass a skill name (`code-review`) or `<store>/<skill>`",
        )
    })?;
    let exists = set.stores().iter().any(|st| {
        wanted.store.as_ref().is_none_or(|s| *s == st.name) && st.skill(&wanted.name).is_some()
    });
    if exists {
        return Ok(wanted);
    }
    let elsewhere = set
        .stores()
        .iter()
        .find(|st| st.skill(&wanted.name).is_some());
    let hint = if let Some(store) = elsewhere {
        let (name, store) = (&wanted.name, &store.name);
        format!("`{name}` is in store `{store}`: pass `{store}/{name}` or just `{name}`")
    } else {
        let names = set
            .stores()
            .iter()
            .flat_map(|st| st.skills.iter().map(|s| s.name.as_str()));
        match closest(&wanted.name, names) {
            Some(near) => format!("did you mean `{near}`?"),
            None => "`kipi resolve` lists the skills each repo uses".to_owned(),
        }
    };
    Err(Failure::user(
        format!("no mounted store has skill `{raw}`"),
        hint,
    ))
}
