//! K013: a repo whose remote matches a scope's `remote_patterns` in another store, and
//! none in its own: e.g. a work repo recorded in the personal store.

use super::remote_patterns::best_match;
use super::{Code, Finding, Inputs};

pub(super) fn check(inputs: &Inputs<'_>, out: &mut Vec<Finding>) {
    for repo in inputs.repos() {
        let Some(remote) = inputs.remote_of(repo) else {
            continue;
        };
        let mut stores = inputs.stores.stores().iter();
        if stores
            .clone()
            .any(|s| s.name == repo.store && best_match(s, remote).is_some())
        {
            continue;
        }
        let Some((other, scope, pattern)) = stores.find_map(|s| {
            let (scope, pattern) = best_match(s, remote).filter(|_| s.name != repo.store)?;
            Some((s, scope, pattern))
        }) else {
            continue;
        };
        out.push(Finding::new(
            Code::WrongStore,
            Some(&repo.name),
            Some(repo.at.clone()),
            format!(
                "repo `{}` is recorded in store `{}`, but its remote `{remote}` matches \
                 `{pattern}` in scopes.{}.remote_patterns of store `{}`",
                repo.name, repo.store, scope.name, other.name
            ),
            format!(
                "move [repos.{}] (and repos/{}/ if it has notes) to store `{}`; if the repo \
                 is really `{}`'s, narrow scopes.{}.remote_patterns",
                repo.name, repo.name, other.name, repo.store, scope.name
            ),
        ));
    }
}
