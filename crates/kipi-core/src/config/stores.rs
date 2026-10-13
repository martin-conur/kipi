//! Mount several stores (roadmap §6, Multiple stores).

use std::path::PathBuf;

use super::BINDINGS_FILE;
use super::bindings::load_store;
use super::paths::{ENV_CONFIG_DIR, ENV_STORE, Env, store_paths};
use crate::error::{Error, Result};
use crate::model::{Config, StoreSet};

/// The stores to mount ([`store_paths`]), loaded ([`load_stores`]); see [`check_config`].
pub fn mount(flags: &[PathBuf], env: &Env, config: &Config) -> Result<StoreSet> {
    let paths = store_paths(flags, env, config);
    check_config(flags, env, config, &paths)?;
    load_stores(&paths, env)
}

/// With no `--store`, no `KIPI_STORE` and no `config.toml`, the default store is all that
/// is left; when it is not a store either, the missing config is the real problem (a
/// first run, or a shell without `KIPI_CONFIG_DIR`), so say that rather than "not a kipi
/// store". `paths` is what [`store_paths`] returned.
pub fn check_config(
    flags: &[PathBuf],
    env: &Env,
    config: &Config,
    paths: &[PathBuf],
) -> Result<()> {
    let defaulted = flags.is_empty() && env.stores.is_empty() && config.source.is_none();
    if defaulted && paths.iter().any(|p| !p.join(BINDINGS_FILE).is_file()) {
        return Err(Error::usage(
            format!(
                "no kipi config at {}, and no store at the default {}",
                env.config_file().display(),
                paths[0].display()
            ),
            format!(
                "create a store and its config with `kipi init --store <path>`; if the config \
                 lives elsewhere, set {ENV_CONFIG_DIR} to its directory; or point --store / \
                 {ENV_STORE} at an existing store"
            ),
        ));
    }
    Ok(())
}

/// Load each store in `paths` independently, then validate them as a set: names
/// unique across stores, no reference crossing into another store except a qualified
/// skill of a store with `share_skills = true`.
pub fn load_stores(paths: &[PathBuf], env: &Env) -> Result<StoreSet> {
    let stores = paths
        .iter()
        .map(|p| load_store(p, env))
        .collect::<Result<_>>()?;
    StoreSet::new(stores)
}
