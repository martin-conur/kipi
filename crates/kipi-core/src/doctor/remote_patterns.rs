//! K001: a repo whose remote matches a scope's `remote_patterns` but that resolved to a
//! scope outside that scope's chain (e.g. a work repo cloned under `~/code`).

use crate::model::{RemoteUrl, Scope, Store};
use crate::resolve::remote_pattern_matches;

use super::{Code, Finding, Inputs};

/// The scope of `store` with the longest `remote_patterns` entry matching `remote`,
/// with that pattern.
pub(super) fn best_match<'s>(store: &'s Store, remote: &RemoteUrl) -> Option<(&'s Scope, &'s str)> {
    store
        .scopes
        .iter()
        .filter_map(|scope| {
            let pattern = scope
                .remote_patterns
                .iter()
                .filter(|p| remote_pattern_matches(p, remote.as_str()))
                .max_by_key(|p| p.len())?;
            Some((scope, pattern.as_str()))
        })
        .max_by_key(|(_, p)| p.len())
}

pub(super) fn check(inputs: &Inputs<'_>, out: &mut Vec<Finding>) {
    for repo in inputs.repos() {
        let (Some(remote), Some(store)) =
            (inputs.remote_of(repo), inputs.stores.store(&repo.store))
        else {
            continue;
        };
        let Some((scope, pattern)) = best_match(store, remote) else {
            continue;
        };
        // Any matching scope in the chain (the scope itself or an ancestor) is fine.
        let in_chain = store.scopes.iter().any(|s| {
            repo.chain.contains(&s.name)
                && s.remote_patterns
                    .iter()
                    .any(|p| remote_pattern_matches(p, remote.as_str()))
        });
        if in_chain {
            continue;
        }
        let resolved = match &repo.scope.value {
            Some(s) => format!("scope `{s}` ({})", repo.scope.source.rule),
            None => "no scope".to_owned(),
        };
        out.push(Finding::new(
            Code::RemoteScopeMismatch,
            Some(&repo.name),
            Some(repo.at.clone()),
            format!(
                "repo `{}` has remote `{remote}`, which matches `{pattern}` in \
                 scopes.{}.remote_patterns, but it resolved to {resolved}",
                repo.name, scope.name
            ),
            format!(
                "add `scope = \"{}\"` to [repos.{}]; if the repo really belongs elsewhere, \
                 narrow scopes.{}.remote_patterns",
                scope.name, repo.name, scope.name
            ),
        ));
    }
}
