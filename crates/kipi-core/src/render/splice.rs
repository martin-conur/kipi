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
    match locate(path, text)? {
        None => {
            let sep = if text.ends_with("\n\n") {
                ""
            } else if text.ends_with('\n') {
                "\n"
            } else {
                "\n\n"
            };
            Ok(format!("{text}{sep}{block}\n"))
        }
        Some((start, end)) => Ok(format!("{}{block}{}", &text[..start], &text[end..])),
    }
}

/// `text` without its block, or `None` when it holds none: the inverse of [`splice_text`].
///
/// The block goes with the line break after it, and when it was the last thing in the
/// file, with the one blank line before it that appending added, so text that ended in
/// exactly one newline gets back byte for byte. Text that is blank once the block is
/// gone held nothing but the block.
pub fn remove_block(path: &Path, text: &str) -> Result<Option<String>> {
    let Some((start, end)) = locate(path, text)? else {
        return Ok(None);
    };
    let rest = &text[end..];
    let rest = rest
        .strip_prefix("\r\n")
        .or_else(|| rest.strip_prefix('\n'))
        .unwrap_or(rest);
    let mut before = &text[..start];
    if rest.is_empty() && before.ends_with("\n\n") {
        before = &before[..before.len() - 1];
    }
    Ok(Some(format!("{before}{rest}")))
}

/// Whether `text` holds anything besides its block: non-whitespace outside the markers.
pub fn has_text_outside_block(path: &Path, text: &str) -> Result<bool> {
    let outside = remove_block(path, text)?;
    Ok(!outside.as_deref().unwrap_or(text).trim().is_empty())
}

/// The block's byte range in `text`, from the begin marker's first byte to the end
/// marker's last; `None` when there are no markers, an error unless there is exactly one
/// begin before one end.
fn locate(path: &Path, text: &str) -> Result<Option<(usize, usize)>> {
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
        ([], []) => Ok(None),
        ([(start, _)], [(end, _)]) if start < end => Ok(Some((*start, end + END.len()))),
        ([_, (_, line), ..], _) => Err(invalid(*line, "duplicated kipi:begin marker")),
        (_, [_, (_, line), ..]) => Err(invalid(*line, "duplicated kipi:end marker")),
        ([(_, line)], []) => Err(invalid(*line, "kipi:begin marker without a kipi:end")),
        ([], [(_, line)]) => Err(invalid(*line, "kipi:end marker without a kipi:begin")),
        ([_], [(_, line)]) => Err(invalid(*line, "kipi:end marker before its kipi:begin")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLOCK: &str = "<!-- kipi:begin x -->\nbody\n<!-- kipi:end -->";

    fn p() -> &'static Path {
        Path::new("CLAUDE.md")
    }

    #[test]
    fn removing_undoes_appending() {
        for text in ["# Mine\n", "# Mine\r\n", "a\n\nb\n"] {
            let spliced = splice_text(p(), Some(text), BLOCK).unwrap();
            assert_eq!(remove_block(p(), &spliced).unwrap().as_deref(), Some(text));
        }
        let alone = splice_text(p(), None, BLOCK).unwrap();
        assert_eq!(remove_block(p(), &alone).unwrap().as_deref(), Some(""));
    }

    #[test]
    fn removing_keeps_text_after_the_block() {
        let text = format!("top\n{BLOCK}\nbottom\n");
        assert_eq!(
            remove_block(p(), &text).unwrap().as_deref(),
            Some("top\nbottom\n")
        );
        assert_eq!(remove_block(p(), "no block\n").unwrap(), None);
        assert!(remove_block(p(), "<!-- kipi:end -->\n").is_err());
    }

    #[test]
    fn text_outside_the_block() {
        let alone = splice_text(p(), None, BLOCK).unwrap();
        assert!(!has_text_outside_block(p(), &alone).unwrap());
        assert!(!has_text_outside_block(p(), &format!("\n  \n{alone}\n")).unwrap());
        assert!(has_text_outside_block(p(), &format!("# Mine\n{alone}")).unwrap());
        assert!(has_text_outside_block(p(), "no block").unwrap());
        assert!(!has_text_outside_block(p(), "").unwrap());
    }
}
