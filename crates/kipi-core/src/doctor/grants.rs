//! K019: a read-grant entry kipi merged into a settings file that points at a path that
//! no longer exists. Safe fix: re-sync the repo, which drops an entry the plan no longer
//! grants (after `--fix` removes the `machine.toml` line of a checkout that is gone).
//!
//! A store's `repos/` or `skills/` the plan still grants is not reported when missing:
//! git keeps no empty directories, so a store without notes often has no `repos/`, and
//! granting it is harmless until it appears.

use std::fs;

use crate::plan::GrantKind;

use super::{Code, Finding, Fix, Inputs};

pub(super) fn check(inputs: &Inputs<'_>, out: &mut Vec<Finding>) {
    for s in inputs.synced() {
        let name = &s.repo.name;
        for grant in &s.manifest.grants {
            for entry in &grant.entries {
                if fs::metadata(entry).is_ok() {
                    continue;
                }
                let planned = s.repo.grant.dirs.iter().find(|d| d.path == *entry);
                let hint = match planned.map(|d| &d.kind) {
                    None => format!("`kipi doctor --fix` (or `kipi sync {name}`) drops the entry"),
                    Some(GrantKind::Repo { name: gone })
                        if inputs.checkouts.missing.contains_key(gone) =>
                    {
                        format!(
                            "`kipi doctor --fix` removes `{gone}` from machine.toml and \
                             re-syncs, which drops the entry"
                        )
                    }
                    Some(_) => continue,
                };
                out.push(
                    Finding::new(
                        Code::StaleGrant,
                        Some(name),
                        None,
                        format!(
                            "{} grants read access to {}, which does not exist",
                            grant.path.display(),
                            inputs.shown(entry)
                        ),
                        hint,
                    )
                    .with_fix(Fix::Resync { repo: name.clone() }),
                );
            }
        }
    }
}
