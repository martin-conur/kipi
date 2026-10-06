//! K018 and K020: what stands in a synced checkout's skill directories. A link pointing
//! outside every mounted store, or at nothing, is a warning (K018); any other link the
//! manifest does not list is info (K020). Each link gets at most one finding.
//!
//! Fixes: kipi's own links are re-synced, which replaces or removes them; a broken link
//! kipi did not create is removed. A working link of the user's is never touched.

use std::collections::BTreeSet;
use std::fs;

use crate::adapters;
use crate::model::AgentId;

use super::{Code, Finding, Fix, Inputs};

pub(super) fn check(inputs: &Inputs<'_>, out: &mut Vec<Finding>) {
    for s in inputs.synced() {
        let name = &s.repo.name;
        let mut dirs = BTreeSet::new();
        for id in AgentId::ALL {
            let dir = adapters::adapter(id).skills_dir(s.dir);
            if !dirs.insert(dir.clone()) {
                continue;
            }
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            let mut links: Vec<_> = entries
                .flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_symlink()))
                .map(|e| e.path())
                .collect();
            links.sort();
            for link in links {
                let Ok(target) = fs::read_link(&link) else {
                    continue;
                };
                let rel = link.strip_prefix(s.dir).unwrap_or(&link);
                let ours = s.manifest.link(rel).is_some();
                let broken = fs::metadata(&link).is_err();
                let resolved = link.parent().map_or(target.clone(), |p| p.join(&target));
                let outside = !broken && !inputs.in_store(&resolved);
                let shown = inputs.shown(&target);
                let what = if broken {
                    format!("{shown}, which does not exist")
                } else {
                    format!("{shown}, outside every mounted store")
                };
                let finding = match (ours, broken || outside) {
                    (true, true) => Finding::new(
                        Code::StrayLink,
                        Some(name),
                        None,
                        format!("kipi's link {} points at {what}", rel.display()),
                        format!(
                            "`kipi doctor --fix` (or `kipi sync {name}`) re-links or removes it"
                        ),
                    )
                    .with_fix(Fix::Resync { repo: name.clone() }),
                    (false, true) if broken => Finding::new(
                        Code::StrayLink,
                        Some(name),
                        None,
                        format!("link {} points at {what}", rel.display()),
                        "kipi did not create it; `kipi doctor --fix` removes the broken link",
                    )
                    .with_fix(Fix::RemoveLink {
                        link: link.clone(),
                        target: target.clone(),
                    }),
                    (false, true) => Finding::new(
                        Code::StrayLink,
                        Some(name),
                        None,
                        format!("link {} points at {what}", rel.display()),
                        "kipi did not create it and leaves it alone; remove it if agents in \
                         this repo should not load that skill, or move the skill into a store \
                         and bind it",
                    ),
                    (true, false) => continue,
                    (false, false) => Finding::new(
                        Code::ForeignLink,
                        Some(name),
                        None,
                        format!("link {} → {shown} is not in kipi's manifest", rel.display()),
                        format!(
                            "kipi leaves it alone; to have sync manage it, bind the skill \
                             (`kipi bind {name} --skill <skill>`): the next sync adopts a link \
                             it would create anyway"
                        ),
                    ),
                };
                out.push(finding);
            }
        }
    }
}
