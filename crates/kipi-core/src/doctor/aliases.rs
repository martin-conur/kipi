//! K004: an alias (or entry name) claimed by two repos of one registry, which makes it
//! ambiguous for the agent (roadmap §6, Aliases).

use super::{Code, Finding, Inputs};

pub(super) fn check(inputs: &Inputs<'_>, out: &mut Vec<Finding>) {
    for registry in &inputs.plan.registries {
        for collision in &registry.collisions {
            let repo = match inputs.only {
                Some(only) if collision.repos.iter().any(|r| r == only) => only,
                Some(_) => continue,
                None => collision.repos[0].as_str(),
            };
            let at = inputs.plan.repo(repo).map(|r| r.at.clone());
            let names: Vec<String> = collision.repos.iter().map(|r| format!("`{r}`")).collect();
            out.push(Finding::new(
                Code::AliasCollision,
                Some(repo),
                at,
                format!(
                    "alias `{}` is claimed by {} in registry `{}`",
                    collision.key,
                    names.join(" and "),
                    registry.scope
                ),
                "aliases (and entry names) must be unique within a registry; drop or rename \
                 the alias in all but one of those entries",
            ));
        }
    }
}
