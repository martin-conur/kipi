//! K010: notes over 1,500 tokens (estimated with the configured encoding, cached). Notes are read on demand, but long ones
//! cost the agent every time it does (roadmap §5.3: target 500).

use crate::error::{Error, Location, Result};

use super::{Code, Finding, Inputs};

/// Notes larger than this many tokens are reported.
pub const MAX_NOTES_TOKENS: u32 = 1500;

pub(super) fn check(inputs: &Inputs<'_>, out: &mut Vec<Finding>) -> Result<()> {
    for repo in inputs.repos() {
        let Some((store, entry)) = inputs.stores.repo(&repo.name) else {
            continue;
        };
        let Some(path) = &entry.notes else {
            continue;
        };
        let estimate = inputs
            .tokens
            .count_file(path)
            .map_err(|e| Error::io(path, e, "check the notes file's permissions"))?;
        if estimate <= MAX_NOTES_TOKENS {
            continue;
        }
        out.push(Finding::new(
            Code::NotesSize,
            Some(&repo.name),
            Some(Location::file(path).in_store(&store.name)),
            format!(
                "the notes of repo `{}` are ~{estimate} tokens, over {MAX_NOTES_TOKENS}",
                repo.name
            ),
            "keep notes to what the code cannot tell (invariants, gotchas, where to look); \
             aim for 500 tokens and link to docs in the repo for the rest",
        ));
    }
    Ok(())
}
