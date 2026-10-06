//! `.kipi/manifest.toml`: everything `sync` wrote into a repo, for clean removal
//! (roadmap §4, §8 step 4).
//!
//! Paths are relative to the target root (the repo checkout, or a store's `generated/`
//! directory), so the manifest reads the same wherever the checkout lives. A path is
//! only ever removed by kipi when the manifest lists it and the disk still holds what
//! kipi wrote there.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Error, Location, Result};

/// The manifest schema version this kipi reads and writes.
pub const VERSION: u32 = 1;

/// What one sync left in a target, as recorded in its manifest.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Schema version ([`VERSION`]).
    pub version: u32,
    /// Directories kipi created to hold what it wrote; removed once empty and unneeded.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dirs: Vec<PathBuf>,
    /// Symlinks kipi created.
    #[serde(default, rename = "link", skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<LinkEntry>,
    /// Files holding a generated block.
    #[serde(default, rename = "block", skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<BlockEntry>,
    /// Whole files kipi owns (generated registries).
    #[serde(default, rename = "file", skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<FileEntry>,
    /// The section kipi keeps in the repo's `.git/info/exclude`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude: Option<ExcludeEntry>,
    /// JSON array entries kipi merged into a settings file (filled by the grants engine).
    #[serde(default, rename = "grant", skip_serializing_if = "Vec::is_empty")]
    pub grants: Vec<GrantEntry>,
}

/// A symlink kipi created.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkEntry {
    /// The link, relative to the target root.
    pub path: PathBuf,
    /// What it points at (absolute).
    pub target: PathBuf,
}

/// A file holding a generated block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockEntry {
    /// The file, relative to the target root.
    pub path: PathBuf,
    /// kipi created the file (it did not exist before the block was first written).
    pub created: bool,
    /// [`hash`] of the block text as written, markers included.
    pub hash: String,
}

/// A whole file kipi owns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileEntry {
    /// The file, relative to the target root.
    pub path: PathBuf,
    /// [`hash`] of the contents as written.
    pub hash: String,
}

/// kipi's section of `.git/info/exclude`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExcludeEntry {
    /// The patterns inside the section.
    pub patterns: Vec<String>,
}

/// Entries kipi added to a JSON array in a settings file
/// (`.claude/settings.local.json` → `permissions.additionalDirectories`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrantEntry {
    /// The settings file, relative to the target root.
    pub path: PathBuf,
    /// The object keys down to the array.
    pub key: Vec<String>,
    /// The array entries kipi owns.
    pub entries: Vec<PathBuf>,
}

impl Manifest {
    /// An empty manifest at the current [`VERSION`].
    pub fn new() -> Self {
        Manifest {
            version: VERSION,
            ..Manifest::default()
        }
    }

    /// Whether it records nothing.
    pub fn is_empty(&self) -> bool {
        self.dirs.is_empty()
            && self.links.is_empty()
            && self.blocks.is_empty()
            && self.files.is_empty()
            && self.exclude.is_none()
            && self.grants.is_empty()
    }

    /// Read the manifest at `path`; an empty one when the file does not exist.
    pub fn load(path: &Path) -> Result<Manifest> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Manifest::new()),
            Err(e) => return Err(Error::io(path, e, "check that the file is readable")),
        };
        let manifest: Manifest = toml_edit::de::from_str(&text).map_err(|e| {
            Error::invalid(
                Location::file(path),
                format!("malformed manifest: {}", e.message()),
                "delete the manifest and re-run `kipi sync`; files it listed are then left \
                 in place as foreign",
            )
        })?;
        if manifest.version != VERSION {
            return Err(Error::invalid(
                Location::file(path),
                format!(
                    "manifest version {} is not supported (expected {VERSION})",
                    manifest.version
                ),
                "upgrade kipi, or delete the manifest and re-run `kipi sync`",
            ));
        }
        Ok(manifest)
    }

    /// The manifest as TOML.
    pub fn to_toml(&self) -> String {
        toml_edit::ser::to_string_pretty(self).expect("the manifest serializes to TOML")
    }

    /// The link entry for `path`.
    pub fn link(&self, path: &Path) -> Option<&LinkEntry> {
        self.links.iter().find(|l| l.path == path)
    }

    /// The block entry for `path`.
    pub fn block(&self, path: &Path) -> Option<&BlockEntry> {
        self.blocks.iter().find(|b| b.path == path)
    }

    /// The file entry for `path`.
    pub fn file(&self, path: &Path) -> Option<&FileEntry> {
        self.files.iter().find(|f| f.path == path)
    }
}

/// The content hash recorded for blocks and files: `sha256:<hex>`.
pub fn hash(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    let mut out = String::with_capacity(7 + 64);
    out.push_str("sha256:");
    for byte in digest.iter() {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_toml() {
        let manifest = Manifest {
            dirs: vec![".claude".into(), ".claude/skills".into()],
            links: vec![LinkEntry {
                path: ".claude/skills/x".into(),
                target: "/store/skills/x".into(),
            }],
            blocks: vec![BlockEntry {
                path: "CLAUDE.md".into(),
                created: true,
                hash: hash("block"),
            }],
            files: vec![FileEntry {
                path: "registry.s.md".into(),
                hash: hash("file"),
            }],
            exclude: Some(ExcludeEntry {
                patterns: vec![".kipi/".into()],
            }),
            grants: vec![GrantEntry {
                path: ".claude/settings.local.json".into(),
                key: vec!["permissions".into(), "additionalDirectories".into()],
                entries: vec!["/store/repos".into()],
            }],
            ..Manifest::new()
        };
        let text = manifest.to_toml();
        let back: Manifest = toml_edit::de::from_str(&text).unwrap();
        assert_eq!(back, manifest, "{text}");
        assert!(text.starts_with("version = 1\n"), "{text}");
    }

    #[test]
    fn empty_manifest_is_just_the_version() {
        assert_eq!(Manifest::new().to_toml(), "version = 1\n");
        assert!(Manifest::new().is_empty());
    }

    #[test]
    fn missing_file_loads_empty_and_bad_files_are_errors_with_hints() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("manifest.toml");
        assert_eq!(Manifest::load(&path).unwrap(), Manifest::new());

        fs::write(&path, "version = 1\nsurprise = true\n").unwrap();
        let err = Manifest::load(&path).unwrap_err();
        assert!(err.to_string().contains("malformed manifest"), "{err}");
        assert!(err.hint().contains("re-run `kipi sync`"));

        fs::write(&path, "version = 9\n").unwrap();
        let err = Manifest::load(&path).unwrap_err();
        assert!(err.to_string().contains("version 9"), "{err}");
    }

    #[test]
    fn hash_is_sha256_hex() {
        assert_eq!(
            hash(""),
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
