//! Test fixtures for kipi. Dev-only: never a dependency of kipi-core or kipi-cli
//! outside `[dev-dependencies]`.
//!
//! - [`builders`]: build stores, store sets and machines in memory (for resolve tests).
//! - [`stores`]: copy `examples/acme-store` and/or `examples/personal-store` into a tempdir.
//! - [`repos`]: lay out fake git checkouts matching the example entries under any root.
//! - [`many`]: a generated home with one store and many repos, for timing `tree`.
//! - [`leak_guard`]: the denylist / home-path scanner behind the repo-wide leak test.

pub mod builders;
pub mod leak_guard;
pub mod many;
pub mod repos;
pub mod stores;

use std::path::{Path, PathBuf};

/// The workspace root (the directory holding `examples/` and `roadmap.md`).
pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crate lives at <root>/crates/<name>")
        .to_path_buf()
}
