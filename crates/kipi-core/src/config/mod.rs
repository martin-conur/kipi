//! Load and validate each store's `bindings.toml`, `machine.toml` and `config.toml`;
//! cross-store checks (toml_edit + serde, `deny_unknown_fields`).
//!
//! Loading is strict: unknown keys are errors naming the closest valid key, and every
//! error points at `file:line` with a hint. Each store loads on its own; [`load_stores`]
//! then checks the mounted set (unique names, nothing crossing stores). Whether a
//! reference within a store names something that exists is checked by resolution.

mod bindings;
mod machine;
mod paths;
mod settings;
mod skills;
mod source;
mod stores;

pub use bindings::load_store;
pub use machine::{load_machine, parse_machine_text};
pub use paths::{
    DEFAULT_STORE, ENV_CACHE_DIR, ENV_CONFIG_DIR, ENV_STORE, Env, expand_tilde, store_paths,
};
pub use settings::{TOKENIZERS, load_config, parse_config_text};
pub use skills::SKILL_FILE;
pub use stores::load_stores;

/// A store's bindings file.
pub const BINDINGS_FILE: &str = "bindings.toml";
/// Per-machine paths, in the config dir.
pub const MACHINE_FILE: &str = "machine.toml";
/// Mounted stores and preferences, in the config dir.
pub const CONFIG_FILE: &str = "config.toml";

/// Hint for an invalid store, scope, repo or skill name.
const NAME_HINT: &str =
    "names use letters, digits, `-`, `_` and `.`, starting with a letter or digit";
