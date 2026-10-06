//! K007: a repo with no scope gets no conventions, skills or registry.

use super::{Code, Finding, Inputs};

pub(super) fn check(inputs: &Inputs<'_>, out: &mut Vec<Finding>) {
    for repo in inputs.repos().filter(|r| r.scope.value.is_none()) {
        out.push(Finding::new(
            Code::Unscoped,
            Some(&repo.name),
            Some(repo.at.clone()),
            format!(
                "repo `{}` has no scope ({}), so it gets no conventions, skills or registry",
                repo.name, repo.scope.source.rule
            ),
            format!(
                "set `scope = \"…\"` in [repos.{}], add a root that holds its checkout, or \
                 declare a `personal` scope in store `{}`",
                repo.name, repo.store
            ),
        ));
    }
}
