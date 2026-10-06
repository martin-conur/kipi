//! K002: a cloned repo with no explicit `scope` placed in a scope none of whose roots
//! (after `machine.toml [roots]`) holds its checkout: the `personal` fallback.

use crate::render::display_path;

use super::{Code, Finding, Inputs};

pub(super) fn check(inputs: &Inputs<'_>, out: &mut Vec<Finding>) {
    for repo in inputs.repos() {
        let (Some(path), Some(scope)) = (&repo.path, &repo.scope.value) else {
            continue;
        };
        let explicit = inputs
            .stores
            .repo(&repo.name)
            .is_some_and(|(_, e)| e.scope.is_some());
        let Some((_, decl)) = inputs.stores.scope(scope) else {
            continue;
        };
        if explicit {
            continue;
        }
        let roots = inputs
            .machine
            .roots_for(scope)
            .map_or(decl.roots.as_slice(), |r| r.roots.as_slice());
        if roots.iter().any(|root| path.starts_with(root)) {
            continue;
        }
        out.push(Finding::new(
            Code::RootNotAncestor,
            Some(&repo.name),
            Some(repo.at.clone()),
            format!(
                "repo `{}` is cloned at {}, outside every root of scope `{scope}` it was \
                 placed in ({})",
                repo.name,
                display_path(path, Some(inputs.home)),
                repo.scope.source.rule
            ),
            format!(
                "if `{scope}` is right, say so with `scope = \"{scope}\"` in [repos.{}]; \
                 otherwise add a root that holds the checkout (scopes.<name>.roots, or \
                 machine.toml [roots] for this machine)",
                repo.name
            ),
        ));
    }
}
