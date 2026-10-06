//! K022: a file holding kipi's generated block that git tracks, so the block shows in
//! `git diff` (roadmap §4, gitignore policy). The user decides; there is no fix.

use std::path::PathBuf;

use super::{Code, Finding, Inputs};

pub(super) fn check(inputs: &Inputs<'_>, out: &mut Vec<Finding>) {
    for s in inputs.synced() {
        let files: Vec<PathBuf> = s
            .manifest
            .blocks
            .iter()
            .map(|b| s.dir.join(&b.path))
            .collect();
        if files.is_empty() {
            continue;
        }
        // A checkout git cannot read has nothing tracked to report.
        let Ok(tracked) = inputs.tracked.tracked(s.dir, &files) else {
            continue;
        };
        for file in files.iter().filter(|f| tracked.contains(*f)) {
            let rel = file.strip_prefix(s.dir).unwrap_or(file);
            out.push(Finding::new(
                Code::TrackedBlock,
                Some(&s.repo.name),
                None,
                format!(
                    "{} holds kipi's generated block and is tracked by git, so the block \
                     shows in `git diff`",
                    rel.display()
                ),
                "keep it if the block belongs in the repo; otherwise `kipi sync` without \
                 `--allow-tracked` moves it into an untracked side file",
            ));
        }
    }
}
