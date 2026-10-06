//! K003: a `related` repo or a granted path crossing unrelated scopes (the leak rule,
//! roadmap §6). Resolution refuses such a `related` entry, so it is reported (and
//! removed) before resolving; grants are checked on the plan as a safety net.

use crate::error::Result;
use crate::model::{Machine, RefSite, StoreSet};
use crate::plan::{GrantKind, RepoPlan};
use crate::resolve::resolve;

use super::prepare::Removal;
use super::{Code, Finding, Inputs};

/// Every `related` entry whose repo is outside the referencing repo's scope chain.
/// Scopes come from resolving the stores with every `related` list emptied (relations
/// never decide a scope). Unknown names and self-references are left to resolution.
pub(super) fn related_removals(set: &StoreSet, machine: &Machine) -> Result<Vec<Removal>> {
    let unrelated: Vec<_> = set
        .stores()
        .iter()
        .cloned()
        .map(|mut store| {
            store.repos.iter_mut().for_each(|r| r.related.clear());
            store
        })
        .collect();
    let plan = resolve(&StoreSet::new(unrelated)?, machine)?;
    let mut out = Vec::new();
    for (store, entry) in set.repos() {
        let Some(repo) = plan.repo(&entry.name) else {
            continue;
        };
        for name in &entry.related {
            let Some(target) = plan.repo(name).filter(|t| t.store == store.name) else {
                continue;
            };
            if target.name == repo.name || same_chain(repo, target) {
                continue;
            }
            let (_, target_entry) = set.repo(name).expect("planned repos are entries");
            let finding = Finding::new(
                Code::ScopeCrossing,
                Some(&repo.name),
                Some(entry.at.clone()),
                format!(
                    "repo `{}` ({}) is related to `{name}` ({}), outside its scope chain",
                    repo.name,
                    describe(repo),
                    describe(target)
                ),
                format!(
                    "related repos must share a scope chain (same scope, an ancestor or a \
                     descendant); remove `{name}` from `related`, or set `scope = \"…\"` on \
                     one of the entries if it is misclassified (declared at {})",
                    target_entry.at
                ),
            );
            out.push(Removal {
                store: store.name.clone(),
                site: RefSite::RepoRelated {
                    repo: repo.name.clone(),
                    name: name.clone(),
                },
                finding,
            });
        }
    }
    Ok(out)
}

/// Every granted path belongs to the repo's own store, a store a skill it uses comes
/// from, or a repo of its store in its scope chain.
pub(super) fn check_grants(inputs: &Inputs<'_>, out: &mut Vec<Finding>) {
    for repo in inputs.repos() {
        for dir in &repo.grant.dirs {
            let why = match &dir.kind {
                GrantKind::StoreRepos => {
                    // The plan was resolved from these stores, so its owner is mounted.
                    let own = (inputs.stores.store(&repo.store))
                        .expect("a planned repo's store is in the set it was resolved from");
                    (dir.path != own.root.join("repos"))
                        .then(|| "the `repos/` dir of another store".to_owned())
                }
                GrantKind::StoreSkills { store } => (*store != repo.store
                    && !repo.skills.iter().any(|s| s.store == *store))
                .then(|| format!("the skills of store `{store}`, which it uses none of")),
                GrantKind::Repo { name } => match inputs.plan.repo(name) {
                    Some(t) if t.store == repo.store && same_chain(repo, t) => None,
                    Some(t) => Some(format!("repo `{name}` ({})", describe(t))),
                    None => Some(format!("`{name}`, which is not a repo entry")),
                },
            };
            if let Some(why) = why {
                out.push(Finding::new(
                    Code::ScopeCrossing,
                    Some(&repo.name),
                    Some(repo.at.clone()),
                    format!(
                        "repo `{}` ({}) is granted {} at {}, outside its scope chain",
                        repo.name,
                        describe(repo),
                        why,
                        dir.path.display()
                    ),
                    "this is a bug in kipi's resolution; please report it, and do not sync \
                     this repo until it is fixed",
                ));
            }
        }
    }
}

/// Same scope, or one an ancestor of the other; two unscoped repos count as one chain.
fn same_chain(a: &RepoPlan, b: &RepoPlan) -> bool {
    match (a.scope.value.as_deref(), b.scope.value.as_deref()) {
        (None, None) => true,
        (Some(x), Some(y)) => a.chain.iter().any(|s| s == y) || b.chain.iter().any(|s| s == x),
        _ => false,
    }
}

fn describe(repo: &RepoPlan) -> String {
    match &repo.scope.value {
        Some(s) => format!("scope `{s}`"),
        None => "unscoped".to_owned(),
    }
}
