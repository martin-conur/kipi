//! K008: a repo with no summary (the registry lists it by name only), or a summary
//! `scan` drafted that no human has reviewed.

use super::{Code, Finding, Inputs};

pub(super) fn check(inputs: &Inputs<'_>, out: &mut Vec<Finding>) {
    for repo in inputs.repos() {
        let Some((_, entry)) = inputs.stores.repo(&repo.name) else {
            continue;
        };
        let (message, hint) = match &entry.summary {
            None => (
                format!(
                    "repo `{}` has no summary; the registry lists it by name only",
                    repo.name
                ),
                format!(
                    "add `summary = \"…\"` to [repos.{}] (one line: what it is, where to look)",
                    repo.name
                ),
            ),
            Some(s) if s.is_draft => (
                format!("the summary of repo `{}` is still a draft", repo.name),
                "review it, then remove its `# kipi:draft` marker".to_owned(),
            ),
            Some(_) => continue,
        };
        out.push(Finding::new(
            Code::Summary,
            Some(&repo.name),
            Some(repo.at.clone()),
            message,
            hint,
        ));
    }
}
