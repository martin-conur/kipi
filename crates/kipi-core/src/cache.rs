//! What kipi remembers between runs in `~/.cache/kipi` (or `$KIPI_CACHE_DIR`), so `tree`
//! and `status` stay fast (roadmap §10): token counts ([`Counter`]) and which files git
//! tracks in each checkout ([`TrackedCache`]).
//!
//! Every entry is keyed by the stamp (mtime, size) of the file it was derived from, so a
//! changed file is never answered from the cache. The caches are an optimization only:
//! an unreadable or malformed cache file counts as empty, and only [`Caches::save`] can
//! fail because of one.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::apply::write_atomic;
use crate::error::{Error, Result};
use crate::git::TrackedCache;
use crate::tokens::{Counter, Tokenizer};

/// Both caches, opened together and saved together.
#[derive(Debug)]
pub struct Caches {
    /// Token counts.
    pub tokens: Counter,
    /// Files git tracks, per checkout.
    pub tracked: TrackedCache,
}

impl Caches {
    /// The caches in `cache_dir`, loading what is there; counts use `tokenizer`.
    pub fn open(cache_dir: &Path, tokenizer: Tokenizer) -> Caches {
        Caches {
            tokens: Counter::new(tokenizer, cache_dir),
            tracked: TrackedCache::new(cache_dir),
        }
    }

    /// Caches that live for this process only.
    pub fn in_memory(tokenizer: Tokenizer) -> Caches {
        Caches {
            tokens: Counter::in_memory(tokenizer),
            tracked: TrackedCache::default(),
        }
    }

    /// Write back whatever this run added. Each cache is saved on its own, so one that
    /// cannot be written never keeps the other from being saved; the result holds one
    /// error per cache that failed (empty when both were saved or had nothing new).
    pub fn save(&self) -> Vec<Error> {
        [self.tokens.save(), self.tracked.save()]
            .into_iter()
            .filter_map(Result::err)
            .collect()
    }
}

/// A map in a cache file keeps at most this many entries; past it, saving keeps only
/// the ones the run used, so churn does not grow the file forever.
pub(crate) const MAX_ENTRIES: usize = 4096;

/// The (mtime in ns since the epoch, size in bytes) a cache entry is keyed by.
pub(crate) type Stamp = (u64, u64);

/// The stamp of a file's metadata. An mtime before the epoch (or none) is `0`.
pub(crate) fn stamp(meta: &fs::Metadata) -> Stamp {
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX));
    (mtime, meta.len())
}

/// Past [`MAX_ENTRIES`], keep only the entries used this run.
pub(crate) fn prune<V>(map: &mut BTreeMap<String, V>, used: &BTreeSet<String>) {
    if map.len() > MAX_ENTRIES {
        map.retain(|k, _| used.contains(k));
    }
}

/// A cache file's contents, or the default when it is absent, unreadable, malformed or
/// another `version`.
pub(crate) fn load<T: DeserializeOwned + Default>(
    path: &Path,
    version: impl Fn(&T) -> u32,
    want: u32,
) -> T {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<T>(&text).ok())
        .filter(|t| version(t) == want)
        .unwrap_or_default()
}

/// Write a cache file atomically, creating the cache directory.
pub(crate) fn store<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let text = serde_json::to_string(value).expect("cache files serialize");
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| {
            Error::io(
                dir,
                e,
                "check that the cache directory ($KIPI_CACHE_DIR, default ~/.cache/kipi) is writable",
            )
        })?;
    }
    write_atomic(path, &text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_maps_keep_what_the_run_used() {
        let mut map: BTreeMap<String, u32> =
            (0..=MAX_ENTRIES).map(|i| (i.to_string(), 1)).collect();
        prune(&mut map, &BTreeSet::from(["7".to_owned()]));
        assert_eq!(map.keys().collect::<Vec<_>>(), ["7"]);
        let mut small: BTreeMap<String, u32> = BTreeMap::from([("a".into(), 1)]);
        prune(&mut small, &BTreeSet::new());
        assert_eq!(small.len(), 1);
    }

    /// A cache with something new to write: a counted text, a tracked-file answer.
    fn dirty(tokens_dir: &Path, tracked_dir: &Path, repo: &Path) -> Caches {
        let caches = Caches {
            tokens: Counter::new(Tokenizer::default(), tokens_dir),
            tracked: TrackedCache::new(tracked_dir),
        };
        caches.tokens.count("hello world");
        caches
            .tracked
            .tracked(repo, &[repo.join("README.md")])
            .unwrap();
        caches
    }

    #[test]
    fn saving_tries_each_cache_and_reports_every_failure() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        fs::create_dir(&repo).unwrap();
        let git = |args: &[&str]| {
            let ok = std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .status()
                .unwrap()
                .success();
            assert!(ok, "git {args:?}");
        };
        git(&["init", "--quiet"]);
        fs::write(repo.join("README.md"), "x").unwrap();
        git(&["add", "README.md"]);
        // A file where the cache directory should be cannot be written into.
        let blocked = dir.path().join("blocked");
        fs::write(&blocked, "").unwrap();
        let good = dir.path().join("good");

        let errors = dirty(&blocked, &good, &repo).save();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(
            good.join("tracked.json").is_file(),
            "saved despite the token failure"
        );

        let errors = dirty(&good, &blocked, &repo).save();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(good.join("tokens.cl100k_base.json").is_file());

        let errors = dirty(&blocked, &blocked, &repo).save();
        assert_eq!(errors.len(), 2, "both failures reported: {errors:?}");
        assert!(Caches::in_memory(Tokenizer::default()).save().is_empty());
    }

    #[test]
    fn stamps_change_with_size() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("f");
        fs::write(&f, "a").unwrap();
        let a = stamp(&fs::metadata(&f).unwrap());
        fs::write(&f, "ab").unwrap();
        let b = stamp(&fs::metadata(&f).unwrap());
        assert_eq!((a.1, b.1), (1, 2));
        assert!(a.0 > 0);
    }
}
