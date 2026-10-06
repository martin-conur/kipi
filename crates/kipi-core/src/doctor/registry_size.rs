//! K011: a registry of more than 40 repos, which every repo of the scope pays for on
//! every turn.

use super::{Code, Finding, Inputs};

/// Registries listing more repos than this are reported.
pub const MAX_REGISTRY_REPOS: usize = 40;

pub(super) fn check(inputs: &Inputs<'_>, out: &mut Vec<Finding>) {
    if !inputs.store_level() {
        return;
    }
    for registry in &inputs.plan.registries {
        let n = registry.entries.len();
        if n <= MAX_REGISTRY_REPOS {
            continue;
        }
        out.push(Finding::new(
            Code::RegistrySize,
            None,
            registry.source.at.clone(),
            format!(
                "registry `{}` lists {n} repos, over {MAX_REGISTRY_REPOS}",
                registry.scope
            ),
            format!(
                "consider splitting scope `{}`: move groups of repos into scopes of their own, \
                 so each repo's always-on registry stays short",
                registry.scope
            ),
        ));
    }
}
