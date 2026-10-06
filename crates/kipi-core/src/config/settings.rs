//! `config.toml` (roadmap §5.5): mounted stores, editor, tokenizer.

use std::path::Path;

use serde::Deserialize;

use super::paths::{ABSOLUTE_PATH_HINT, Env, absolute_path};
use super::source::Source;
use crate::error::{Error, Result, closest};
use crate::model::Config;

/// Encodings `tokenizer` may name (the ones tiktoken-rs ships).
pub const TOKENIZERS: &[&str] = &[
    "cl100k_base",
    "o200k_base",
    "p50k_base",
    "p50k_edit",
    "r50k_base",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    #[serde(default)]
    stores: Vec<String>,
    editor: Option<String>,
    tokenizer: Option<String>,
}

/// Load `config.toml` from the config dir; an absent file is an empty [`Config`].
pub fn load_config(env: &Env) -> Result<Config> {
    match Source::read_optional(&env.config_file())? {
        Some(src) => parse_config(src, &env.home),
        None => Ok(Config::default()),
    }
}

/// Parse a `config.toml` read from `path`.
pub fn parse_config_text(path: &Path, text: String, env: &Env) -> Result<Config> {
    parse_config(Source::parse(path, text)?, &env.home)
}

fn parse_config(src: Source, home: &Path) -> Result<Config> {
    let raw: RawConfig = src.deserialize("change the value to the expected type")?;
    let stores = raw
        .stores
        .iter()
        .map(|s| {
            absolute_path(s, home)
                .map_err(|m| Error::invalid(src.at(&["stores"]), m, ABSOLUTE_PATH_HINT))
        })
        .collect::<Result<_>>()?;
    if raw.editor.as_deref().is_some_and(|e| e.trim().is_empty()) {
        return Err(Error::invalid(
            src.at(&["editor"]),
            "empty editor",
            "name an editor command such as `editor = \"hx\"`, or remove the key",
        ));
    }
    if let Some(t) = raw.tokenizer.as_deref().filter(|t| !TOKENIZERS.contains(t)) {
        let valid = TOKENIZERS
            .iter()
            .map(|t| format!("`{t}`"))
            .collect::<Vec<_>>();
        let valid = valid.join(", ");
        return Err(Error::invalid(
            src.at(&["tokenizer"]),
            format!("unknown tokenizer `{t}`"),
            match closest(t, TOKENIZERS.iter().copied()) {
                Some(s) => format!("did you mean `{s}`? Valid values: {valid}"),
                None => format!("valid values: {valid}"),
            },
        ));
    }
    Ok(Config {
        stores,
        editor: raw.editor,
        tokenizer: raw.tokenizer,
        source: Some(src.into_doc()),
    })
}
