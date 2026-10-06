//! The example stores shipped under `examples/`, and helpers to copy them into a
//! tempdir so tests can mutate them freely.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use tempfile::TempDir;
use walkdir::WalkDir;

use crate::workspace_root;

/// One of the example stores under `examples/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExampleStore {
    /// `examples/acme-store`: store `acme`, scopes `acme` + `acme-ml`.
    Acme,
    /// `examples/personal-store`: store `personal`, scope `personal`, shares skills.
    Personal,
}

impl ExampleStore {
    /// Both example stores, in mount order (personal first, as in the example `config.toml`).
    pub const ALL: [ExampleStore; 2] = [ExampleStore::Personal, ExampleStore::Acme];

    /// The `[store] name` declared in this store's `bindings.toml`.
    pub fn name(self) -> &'static str {
        match self {
            ExampleStore::Acme => "acme",
            ExampleStore::Personal => "personal",
        }
    }

    /// Directory name under `examples/`.
    pub fn dir_name(self) -> &'static str {
        match self {
            ExampleStore::Acme => "acme-store",
            ExampleStore::Personal => "personal-store",
        }
    }

    /// Path of the checked-in store. Read-only by convention: copy it before mutating.
    pub fn source_path(self) -> PathBuf {
        workspace_root().join("examples").join(self.dir_name())
    }
}

/// Example stores copied into a temporary directory, removed on drop.
#[derive(Debug)]
pub struct StoreFixture {
    dir: TempDir,
    stores: Vec<(ExampleStore, PathBuf)>,
}

impl StoreFixture {
    /// The tempdir holding the copied stores.
    pub fn root(&self) -> &Path {
        self.dir.path()
    }

    /// Path of a copied store. Panics if that store was not copied.
    pub fn path(&self, store: ExampleStore) -> &Path {
        self.stores
            .iter()
            .find(|(s, _)| *s == store)
            .map(|(_, p)| p.as_path())
            .unwrap_or_else(|| panic!("store {} was not copied into this fixture", store.name()))
    }

    /// Paths of every copied store, in the order requested.
    pub fn paths(&self) -> Vec<&Path> {
        self.stores.iter().map(|(_, p)| p.as_path()).collect()
    }
}

/// Copy the given example stores into a fresh tempdir, each under its `examples/` dir name.
pub fn copy_example_stores(stores: &[ExampleStore]) -> io::Result<StoreFixture> {
    let dir = tempfile::tempdir()?;
    let mut copied = Vec::with_capacity(stores.len());
    for &store in stores {
        let dest = dir.path().join(store.dir_name());
        copy_dir(&store.source_path(), &dest)?;
        copied.push((store, dest));
    }
    Ok(StoreFixture {
        dir,
        stores: copied,
    })
}

/// Copy one example store to `dest` (created if missing).
pub fn copy_example_store_to(store: ExampleStore, dest: &Path) -> io::Result<()> {
    copy_dir(&store.source_path(), dest)
}

/// Recursively copy `src` into `dest`, skipping `generated/` (it is per-machine output).
fn copy_dir(src: &Path, dest: &Path) -> io::Result<()> {
    let walker = WalkDir::new(src)
        .into_iter()
        .filter_entry(|e| e.depth() != 1 || e.file_name() != "generated");
    for entry in walker {
        let entry = entry.map_err(io::Error::other)?;
        let rel = entry
            .path()
            .strip_prefix(src)
            .expect("walkdir yields paths under its root");
        let target = dest.join(rel);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target)?;
        } else {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_both_stores_side_by_side() {
        let fx = copy_example_stores(&ExampleStore::ALL).unwrap();
        for store in ExampleStore::ALL {
            let path = fx.path(store);
            assert!(path.starts_with(fx.root()));
            assert!(path.join("bindings.toml").is_file());
            assert!(path.join("skills").is_dir());
        }
        assert_eq!(fx.paths().len(), 2);
    }

    #[test]
    fn copies_a_single_store() {
        let fx = copy_example_stores(&[ExampleStore::Personal]).unwrap();
        assert!(
            fx.path(ExampleStore::Personal)
                .join("bindings.toml")
                .is_file()
        );
        assert!(!fx.root().join("acme-store").exists());
    }

    #[test]
    fn copy_is_independent_of_the_source() {
        let fx = copy_example_stores(&[ExampleStore::Acme]).unwrap();
        let copied = fx.path(ExampleStore::Acme).join("bindings.toml");
        fs::write(&copied, "mutated").unwrap();
        let original = fs::read_to_string(ExampleStore::Acme.source_path().join("bindings.toml"));
        assert_ne!(original.unwrap(), "mutated");
    }
}
