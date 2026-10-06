//! Token estimates (tiktoken-rs, `cl100k_base`) with an (path, mtime, size) cache.
//!
//! Until the tokenizer lands (Stage 3), [`estimate`] approximates by characters.

/// Characters per token assumed by [`estimate`]: the usual rule of thumb for English
/// prose and code under `cl100k_base`.
pub const CHARS_PER_TOKEN: usize = 4;

/// An estimate of the tokens in `text`: its characters divided by
/// [`CHARS_PER_TOKEN`], rounded up.
pub fn estimate(text: &str) -> u32 {
    let tokens = text.chars().count().div_ceil(CHARS_PER_TOKEN);
    u32::try_from(tokens).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimates_round_up_by_characters() {
        assert_eq!(estimate(""), 0);
        assert_eq!(estimate("abc"), 1);
        assert_eq!(estimate("abcd"), 1);
        assert_eq!(estimate("abcde"), 2);
        assert_eq!(estimate("ééééé"), 2, "characters, not bytes");
    }
}
