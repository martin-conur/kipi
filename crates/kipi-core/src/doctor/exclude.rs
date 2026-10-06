//! K021: patterns kipi keeps in a synced checkout's `.git/info/exclude` (as its manifest
//! records them) that the file no longer holds, so git shows kipi's files as untracked.
//! Safe fix: re-sync the repo, which rewrites kipi's section.

use std::fs;

use crate::git;

use super::{Code, Finding, Fix, Inputs};

pub(super) fn check(inputs: &Inputs<'_>, out: &mut Vec<Finding>) {
    for s in inputs.synced() {
        let Some(recorded) = &s.manifest.exclude else {
            continue;
        };
        // Without a git directory there is nothing to exclude from; sync says so itself.
        let Ok(path) = git::info_exclude(s.dir) else {
            continue;
        };
        let text = fs::read_to_string(&path).unwrap_or_default();
        let missing: Vec<String> = recorded
            .patterns
            .iter()
            .filter(|p| !text.lines().any(|l| l.trim() == p.as_str()))
            .map(|p| format!("`{p}`"))
            .collect();
        if missing.is_empty() {
            continue;
        }
        let name = &s.repo.name;
        out.push(
            Finding::new(
                Code::MissingExclude,
                Some(name),
                None,
                format!(
                    ".git/info/exclude lacks {}, so git shows kipi's files as untracked",
                    missing.join(", ")
                ),
                format!("`kipi doctor --fix` (or `kipi sync {name}`) restores kipi's section"),
            )
            .with_fix(Fix::Resync { repo: name.clone() }),
        );
    }
}
