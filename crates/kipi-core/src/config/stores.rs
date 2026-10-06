//! Mount several stores (roadmap §6, Multiple stores).

use std::path::PathBuf;

use super::bindings::load_store;
use super::paths::Env;
use crate::error::Result;
use crate::model::StoreSet;

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
