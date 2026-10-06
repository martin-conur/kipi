//! Placing a [`RenderedBlock`] into `CLAUDE.md` / `AGENTS.md` (roadmap §8).
//!
//! Exactly one block per file. Everything outside the markers is user-owned and kept
//! byte for byte: no file → the block alone; no markers → the block appended after a
//! blank line; one block → replaced in place; anything else → an error.

use std::path::Path;

use super::block::{END, RenderedBlock, is_begin, is_end};
use crate::error::{Error, Location, Result};

const FIX_HINT: &str = "keep exactly one `<!-- kipi:begin … -->` … `<!-- kipi:end -->` pair \
                        (or delete the partial block), then re-run `kipi sync`";

/// The new contents of the file at `path` (used only in errors), given its current
/// contents (`None` when it does not exist) and the block to place.
pub fn splice(path: &Path, existing: Option<&str>, block: &RenderedBlock) -> Result<String> {
    splice_text(path, existing, &block.text())
}

/// [`splice`] for block text an adapter already rendered
/// ([`crate::adapters::AgentAdapter::render_block`]): from the begin marker through the
/// end marker, without a trailing newline.
pub fn splice_text(path: &Path, existing: Option<&str>, block: &str) -> Result<String> {
    let text = match existing {
        None | Some("") => return Ok(format!("{block}\n")),
        Some(text) => text,
    };

    // (byte offset of the line, 1-based line number)
    let mut begins: Vec<(usize, usize)> = Vec::new();
    let mut ends: Vec<(usize, usize)> = Vec::new();
    let mut offset = 0;
    for (i, raw) in text.split_inclusive('\n').enumerate() {
        let line = raw.strip_suffix('\n').unwrap_or(raw);
        if is_begin(line) {
            begins.push((offset, i + 1));
        } else if is_end(line) {
            ends.push((offset, i + 1));
        }
        offset += raw.len();
    }

    let invalid =
        |line: usize, message: &str| Error::invalid(Location::at(path, line), message, FIX_HINT);
    match (begins.as_slice(), ends.as_slice()) {
        ([], []) => {
            let sep = if text.ends_with("\n\n") {
                ""
            } else if text.ends_with('\n') {
                "\n"
            } else {
                "\n\n"
            };
            Ok(format!("{text}{sep}{block}\n"))
        }
        ([(start, _)], [(end, _)]) if start < end => {
            let after = end + END.len();
            Ok(format!("{}{block}{}", &text[..*start], &text[after..]))
        }
        ([_, (_, line), ..], _) => Err(invalid(*line, "duplicated kipi:begin marker")),
        (_, [_, (_, line), ..]) => Err(invalid(*line, "duplicated kipi:end marker")),
        ([(_, line)], []) => Err(invalid(*line, "kipi:begin marker without a kipi:end")),
        ([], [(_, line)]) => Err(invalid(*line, "kipi:end marker without a kipi:begin")),
        ([_], [(_, line)]) => Err(invalid(*line, "kipi:end marker before its kipi:begin")),
    }
}
