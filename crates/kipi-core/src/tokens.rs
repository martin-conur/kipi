//! Token counts with `tiktoken-rs` (roadmap §10), cached on disk so `tree` and `status`
//! never build a tokenizer when nothing changed.
//!
//! Counts are estimates: Claude's tokenizer is not public, so the configured tiktoken
//! encoding (`config.toml` `tokenizer`, default `cl100k_base`) stands in for it.
//!
//! The cache lives in `<cache dir>/tokens.<encoding>.json` (see [`crate::cache`]). A file
//! is keyed by (path, mtime, size), so an unchanged file is never read again; generated
//! text (a rendered block, a registry, a skill description) has no file of its own and is
//! keyed by its SHA-256.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tiktoken_rs::CoreBPE;

use crate::cache::{self, Stamp};
use crate::error::Result;

/// A tiktoken encoding `config.toml` may name.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Tokenizer {
    /// `cl100k_base`, the default.
    #[default]
    Cl100kBase,
    /// `o200k_base`.
    O200kBase,
    /// `p50k_base`.
    P50kBase,
    /// `p50k_edit`.
    P50kEdit,
    /// `r50k_base`.
    R50kBase,
}

impl Tokenizer {
    /// Every encoding, the default first.
    pub const ALL: [Tokenizer; 5] = [
        Tokenizer::Cl100kBase,
        Tokenizer::O200kBase,
        Tokenizer::P50kBase,
        Tokenizer::P50kEdit,
        Tokenizer::R50kBase,
    ];

    /// The encoding's name, as written in `config.toml`.
    pub fn name(self) -> &'static str {
        match self {
            Tokenizer::Cl100kBase => "cl100k_base",
            Tokenizer::O200kBase => "o200k_base",
            Tokenizer::P50kBase => "p50k_base",
            Tokenizer::P50kEdit => "p50k_edit",
            Tokenizer::R50kBase => "r50k_base",
        }
    }

    /// The encoding called `name`.
    pub fn from_name(name: &str) -> Option<Tokenizer> {
        Tokenizer::ALL.into_iter().find(|t| t.name() == name)
    }

    /// The encoder, built on first use (that is the slow part: tens of milliseconds).
    fn bpe(self) -> &'static CoreBPE {
        match self {
            Tokenizer::Cl100kBase => tiktoken_rs::cl100k_base_singleton(),
            Tokenizer::O200kBase => tiktoken_rs::o200k_base_singleton(),
            Tokenizer::P50kBase => tiktoken_rs::p50k_base_singleton(),
            Tokenizer::P50kEdit => tiktoken_rs::p50k_edit_singleton(),
            Tokenizer::R50kBase => tiktoken_rs::r50k_base_singleton(),
        }
    }

    /// The tokens in `text`, counted without a cache. Special-token syntax such as
    /// `<|endoftext|>` is counted as the plain text it is in a file.
    pub fn count(self, text: &str) -> u32 {
        u32::try_from(self.bpe().count_ordinary(text)).unwrap_or(u32::MAX)
    }
}

/// The cache file format. Bump `version` on a change; another version reads as empty.
const CACHE_VERSION: u32 = 1;

#[derive(Debug, Default, Serialize, Deserialize)]
struct CacheFile {
    version: u32,
    /// Path → (its stamp, tokens).
    files: BTreeMap<String, (Stamp, u32)>,
    /// `sha256` hex of the text → tokens.
    texts: BTreeMap<String, u32>,
}

#[derive(Debug, Default)]
struct Cache {
    file: CacheFile,
    used_files: BTreeSet<String>,
    used_texts: BTreeSet<String>,
    dirty: bool,
}

/// Counts tokens with one [`Tokenizer`], through an on-disk cache. Shareable across
/// threads; call [`Counter::save`] once at the end of a command.
#[derive(Debug)]
pub struct Counter {
    tokenizer: Tokenizer,
    /// The cache file; `None` keeps the cache in memory only.
    path: Option<PathBuf>,
    cache: Mutex<Cache>,
}

impl Counter {
    /// A counter caching in `<cache_dir>/tokens.<encoding>.json`, loading what is there.
    pub fn new(tokenizer: Tokenizer, cache_dir: &Path) -> Counter {
        let path = cache_dir.join(format!("tokens.{}.json", tokenizer.name()));
        let file = cache::load(&path, |f: &CacheFile| f.version, CACHE_VERSION);
        Counter {
            tokenizer,
            path: Some(path),
            cache: Mutex::new(Cache {
                file,
                ..Cache::default()
            }),
        }
    }

    /// A counter that remembers counts for its own lifetime only.
    pub fn in_memory(tokenizer: Tokenizer) -> Counter {
        Counter {
            tokenizer,
            path: None,
            cache: Mutex::new(Cache::default()),
        }
    }

    /// The encoding.
    pub fn tokenizer(&self) -> Tokenizer {
        self.tokenizer
    }

    /// The tokens in `text`, cached by its content.
    pub fn count(&self, text: &str) -> u32 {
        if text.is_empty() {
            return 0;
        }
        let key = hex(&Sha256::digest(text.as_bytes()));
        {
            let mut cache = self.lock();
            if let Some(&n) = cache.file.texts.get(&key) {
                cache.used_texts.insert(key);
                return n;
            }
        }
        let n = self.tokenizer.count(text);
        let mut cache = self.lock();
        cache.file.texts.insert(key.clone(), n);
        cache.used_texts.insert(key);
        cache.dirty = true;
        n
    }

    /// The tokens in the file at `path`, cached by (path, mtime, size): an unchanged
    /// file is not read.
    pub fn count_file(&self, path: &Path) -> io::Result<u32> {
        let stamp = cache::stamp(&fs::metadata(path)?);
        let key = path.to_str().map(str::to_owned);
        if let Some(key) = &key {
            let mut cache = self.lock();
            if let Some(&(s, n)) = cache.file.files.get(key)
                && s == stamp
            {
                cache.used_files.insert(key.clone());
                return Ok(n);
            }
        }
        let n = self.tokenizer.count(&fs::read_to_string(path)?);
        // A path that is not UTF-8 cannot be a JSON key; it is counted every time.
        if let Some(key) = key {
            let mut cache = self.lock();
            cache.file.files.insert(key.clone(), (stamp, n));
            cache.used_files.insert(key);
            cache.dirty = true;
        }
        Ok(n)
    }

    /// Write the cache back if this run added to it. Creates the cache directory.
    pub fn save(&self) -> Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let mut cache = self.lock();
        if !cache.dirty {
            return Ok(());
        }
        let Cache {
            file,
            used_files,
            used_texts,
            ..
        } = &mut *cache;
        cache::prune(&mut file.files, used_files);
        cache::prune(&mut file.texts, used_texts);
        file.version = CACHE_VERSION;
        cache::store(path, &*file)?;
        cache.dirty = false;
        Ok(())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Cache> {
        // A panic while holding the lock leaves at worst a partial map of correct counts.
        self.cache.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `n` tokens for humans: `620`, `1.3k`, `12.0k`.
pub fn human(n: u32) -> String {
    if n < 1000 {
        n.to_string()
    } else {
        format!("{:.1}k", f64::from(n) / 1000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_config_tokenizer_has_an_encoding() {
        for name in crate::config::TOKENIZERS {
            assert_eq!(Tokenizer::from_name(name).unwrap().name(), *name);
        }
        assert_eq!(Tokenizer::ALL.len(), crate::config::TOKENIZERS.len());
        assert_eq!(Tokenizer::default().name(), "cl100k_base");
        assert!(Tokenizer::from_name("gpt2").is_none());
    }

    #[test]
    fn counts_with_the_encoding() {
        let c = Counter::in_memory(Tokenizer::Cl100kBase);
        assert_eq!(c.count(""), 0);
        assert_eq!(c.count("hello world"), 2);
        assert_eq!(
            c.count("<|endoftext|>"),
            Tokenizer::Cl100kBase.count("<|endoftext|>")
        );
        assert!(c.count("<|endoftext|>") > 1, "special syntax is plain text");
    }

    #[test]
    fn human_counts() {
        assert_eq!(human(0), "0");
        assert_eq!(human(999), "999");
        assert_eq!(human(1000), "1.0k");
        assert_eq!(human(1349), "1.3k");
        assert_eq!(human(12_050), "12.1k");
    }

    #[test]
    fn a_warm_cache_answers_without_reading_or_tokenizing() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.md");
        fs::write(&file, "hello world").unwrap();
        let cache = dir.path().join("cache");
        let cold = Counter::new(Tokenizer::Cl100kBase, &cache);
        assert_eq!(cold.count_file(&file).unwrap(), 2);
        assert_eq!(cold.count("a b c"), 3);
        cold.save().unwrap();
        let saved = cache.join("tokens.cl100k_base.json");
        assert!(saved.is_file());

        // Doctor the cached values: a warm counter must return them untouched.
        let text = fs::read_to_string(&saved).unwrap();
        let mut doc: CacheFile = serde_json::from_str(&text).unwrap();
        doc.files.values_mut().for_each(|v| v.1 = 77);
        doc.texts.values_mut().for_each(|v| *v = 88);
        fs::write(&saved, serde_json::to_string(&doc).unwrap()).unwrap();
        let warm = Counter::new(Tokenizer::Cl100kBase, &cache);
        assert_eq!(warm.count_file(&file).unwrap(), 77);
        assert_eq!(warm.count("a b c"), 88);
        assert!(!warm.lock().dirty, "nothing new to write");

        // A change of size invalidates the entry.
        fs::write(&file, "hello world, again").unwrap();
        assert_eq!(warm.count_file(&file).unwrap(), 4);
        assert!(warm.lock().dirty);

        // Another encoding has its own file.
        let other = Counter::new(Tokenizer::O200kBase, &cache);
        assert_eq!(other.count("a b c"), 3);
    }

    #[test]
    fn a_broken_cache_counts_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("tokens.cl100k_base.json"), "{not json").unwrap();
        let c = Counter::new(Tokenizer::Cl100kBase, dir.path());
        assert_eq!(c.count("hello world"), 2);
        c.save().unwrap();
        let again = Counter::new(Tokenizer::Cl100kBase, dir.path());
        assert_eq!(again.lock().file.texts.len(), 1, "rewritten whole");
    }

    #[test]
    fn missing_files_are_io_errors() {
        let c = Counter::in_memory(Tokenizer::Cl100kBase);
        assert!(
            c.count_file(Path::new("/nonexistent/kipi/notes.md"))
                .is_err()
        );
        c.save().unwrap();
    }
}
