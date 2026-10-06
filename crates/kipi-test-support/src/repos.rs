//! Fake git checkouts for the example stores' repo entries.
//!
//! Every example entry has a fixed path relative to a *machine root* (a stand-in
//! for `$HOME`): acme repos under `work/acme/` (ML ones under `work/acme/ml/`),
//! personal repos under `code/`. Lay the same set out under different roots to
//! simulate several machines; tweak [`FakeRepo::dir`] to simulate a repo cloned
//! under another directory name.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use toml_edit::DocumentMut;

use crate::stores::ExampleStore;

/// A manifest file to drop into a fake repo, used by `scan`'s summary drafting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestKind {
    /// `Cargo.toml` with `package.description`.
    Cargo,
    /// `package.json` with `description`.
    PackageJson,
    /// `pyproject.toml` with `project.description`.
    Pyproject,
    /// No manifest; drafting falls back to the README.
    None,
}

/// Where each example entry lives under a machine root, and what it is built from.
/// Remotes and summaries are read from the example `bindings.toml`, not repeated here.
pub const EXAMPLE_LAYOUT: &[(ExampleStore, &str, &str, ManifestKind)] = &[
    (
        ExampleStore::Acme,
        "billing-api",
        "work/acme/billing-api",
        ManifestKind::Cargo,
    ),
    (
        ExampleStore::Acme,
        "acme-api",
        "work/acme/api",
        ManifestKind::PackageJson,
    ),
    (
        ExampleStore::Acme,
        "web-dashboard",
        "work/acme/web-dashboard",
        ManifestKind::None,
    ),
    (
        ExampleStore::Acme,
        "events-pipeline",
        "work/acme/ml/events-pipeline",
        ManifestKind::Cargo,
    ),
    (
        ExampleStore::Acme,
        "ml-platform",
        "work/acme/ml/ml-platform",
        ManifestKind::Pyproject,
    ),
    (
        ExampleStore::Acme,
        "feature-store",
        "work/acme/ml/feature-store",
        ManifestKind::Pyproject,
    ),
    (
        ExampleStore::Personal,
        "side-tool",
        "code/side-tool",
        ManifestKind::Cargo,
    ),
    (
        ExampleStore::Personal,
        "api",
        "code/api",
        ManifestKind::PackageJson,
    ),
    (
        ExampleStore::Personal,
        "blog",
        "code/blog",
        ManifestKind::None,
    ),
    (
        ExampleStore::Personal,
        "dotfiles",
        "code/dotfiles",
        ManifestKind::None,
    ),
];

/// A fake checkout to create on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeRepo {
    /// The `[repos.<name>]` entry this checkout belongs to.
    pub entry: String,
    /// Path relative to the machine root.
    pub dir: PathBuf,
    /// Value for `remote.origin.url`; `None` creates a repo without a remote.
    pub origin_url: Option<String>,
    /// `README.md` contents; `None` writes no README.
    pub readme: Option<String>,
    /// Manifest kind; its description is the entry's summary (if any).
    pub manifest: ManifestKind,
    /// Description written into the manifest.
    pub description: Option<String>,
}

impl FakeRepo {
    /// Create this repo under `root`, returning its absolute path.
    pub fn create_under(&self, root: &Path) -> io::Result<PathBuf> {
        let path = root.join(&self.dir);
        create_fake_repo(&path, self)?;
        Ok(path)
    }
}

/// The fake repos for every entry of the given example stores, in [`EXAMPLE_LAYOUT`] order.
///
/// `origin_url` is derived from the entry's normalized `remote`: acme repos use the
/// scp-like SSH form (`git@host:owner/name.git`), personal repos HTTPS
/// (`https://host/owner/name.git`), so both normalization paths get exercised.
pub fn example_repos(stores: &[ExampleStore]) -> io::Result<Vec<FakeRepo>> {
    let mut docs = Vec::new();
    for &store in stores {
        docs.push((store, read_bindings(store)?));
    }
    let mut repos = Vec::new();
    for &(store, entry, dir, manifest) in EXAMPLE_LAYOUT {
        let Some((_, doc)) = docs.iter().find(|(s, _)| *s == store) else {
            continue;
        };
        let item = doc
            .get("repos")
            .and_then(|r| r.get(entry))
            .ok_or_else(|| io::Error::other(format!("{}: no [repos.{entry}]", store.name())))?;
        let field = |k: &str| item.get(k).and_then(|v| v.as_str()).map(str::to_owned);
        let (remote, summary) = (field("remote"), field("summary"));
        let origin_url = remote.map(|r| match store {
            ExampleStore::Acme => ssh_url(&r),
            ExampleStore::Personal => https_url(&r),
        });
        let readme = Some(match &summary {
            Some(s) => format!("# {entry}\n\n{s}.\n"),
            None => format!("# {entry}\n"),
        });
        repos.push(FakeRepo {
            entry: entry.to_owned(),
            dir: PathBuf::from(dir),
            origin_url,
            readme,
            manifest,
            description: summary,
        });
    }
    Ok(repos)
}

/// Create every repo under `root` (one simulated machine), returning `(entry, absolute path)`.
pub fn lay_out(root: &Path, repos: &[FakeRepo]) -> io::Result<Vec<(String, PathBuf)>> {
    repos
        .iter()
        .map(|r| Ok((r.entry.clone(), r.create_under(root)?)))
        .collect()
}

/// `git init` a repo at `path` (created if missing) with the given remote, README and manifest.
pub fn create_fake_repo(path: &Path, repo: &FakeRepo) -> io::Result<()> {
    fs::create_dir_all(path)?;
    git(path, &["init", "--quiet"])?;
    if let Some(url) = &repo.origin_url {
        git(path, &["remote", "add", "origin", url])?;
    }
    if let Some(readme) = &repo.readme {
        fs::write(path.join("README.md"), readme)?;
    }
    let name = repo
        .dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(&repo.entry);
    if let Some((file, contents)) = manifest_file(repo.manifest, name, repo.description.as_deref())
    {
        fs::write(path.join(file), contents)?;
    }
    Ok(())
}

/// `github.com/owner/name` → `git@github.com:owner/name.git`.
pub fn ssh_url(remote: &str) -> String {
    match remote.split_once('/') {
        Some((host, rest)) => format!("git@{host}:{rest}.git"),
        None => format!("git@{remote}.git"),
    }
}

/// `github.com/owner/name` → `https://github.com/owner/name.git`.
pub fn https_url(remote: &str) -> String {
    format!("https://{remote}.git")
}

fn read_bindings(store: ExampleStore) -> io::Result<DocumentMut> {
    let text = fs::read_to_string(store.source_path().join("bindings.toml"))?;
    text.parse::<DocumentMut>()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

fn manifest_file(
    kind: ManifestKind,
    name: &str,
    description: Option<&str>,
) -> Option<(&'static str, String)> {
    let desc = description.unwrap_or_default();
    match kind {
        ManifestKind::Cargo => Some((
            "Cargo.toml",
            format!(
                "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\ndescription = \"{desc}\"\n"
            ),
        )),
        ManifestKind::PackageJson => Some((
            "package.json",
            format!(
                "{{\n  \"name\": \"{name}\",\n  \"version\": \"0.1.0\",\n  \"description\": \"{desc}\"\n}}\n"
            ),
        )),
        ManifestKind::Pyproject => Some((
            "pyproject.toml",
            format!(
                "[project]\nname = \"{name}\"\nversion = \"0.1.0\"\ndescription = \"{desc}\"\n"
            ),
        )),
        ManifestKind::None => None,
    }
}

fn git(dir: &Path, args: &[&str]) -> io::Result<String> {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output()?;
    if !out.status.success() {
        return Err(io::Error::other(format!(
            "git {} failed in {}: {}",
            args.join(" "),
            dir.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_forms() {
        assert_eq!(
            ssh_url("github.com/acme-org/api"),
            "git@github.com:acme-org/api.git"
        );
        assert_eq!(
            https_url("github.com/example-user/api"),
            "https://github.com/example-user/api.git"
        );
    }

    #[test]
    fn example_repos_cover_every_layout_entry_of_the_requested_stores() {
        let all = example_repos(&ExampleStore::ALL).unwrap();
        assert_eq!(all.len(), EXAMPLE_LAYOUT.len());
        let personal = example_repos(&[ExampleStore::Personal]).unwrap();
        assert_eq!(personal.len(), 4);
        assert!(personal.iter().all(|r| r.dir.starts_with("code")));
        assert!(
            all.iter().all(|r| r.origin_url.is_some()),
            "every example entry has a remote"
        );
    }

    #[test]
    fn lays_out_real_git_repos_with_matching_remotes() {
        let root = tempfile::tempdir().unwrap();
        let repos = example_repos(&ExampleStore::ALL).unwrap();
        let laid = lay_out(root.path(), &repos).unwrap();
        assert_eq!(laid.len(), repos.len());
        for (repo, (entry, path)) in repos.iter().zip(&laid) {
            assert_eq!(&repo.entry, entry);
            assert!(path.join(".git").is_dir(), "{entry} is a git repo");
            assert!(path.join("README.md").is_file());
            let url = git(path, &["config", "--get", "remote.origin.url"]).unwrap();
            assert_eq!(Some(url), repo.origin_url);
        }
        let billing = root.path().join("work/acme/billing-api/Cargo.toml");
        let manifest = fs::read_to_string(billing).unwrap();
        assert!(manifest.contains("description = \"Invoicing and payment orchestration"));
    }

    #[test]
    fn same_repo_set_lays_out_under_two_machine_roots() {
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let mut repos = example_repos(&[ExampleStore::Personal]).unwrap();
        lay_out(a.path(), &repos).unwrap();
        // Machine B clones side-tool under another directory name.
        repos[0].dir = PathBuf::from("src/st");
        let laid = lay_out(b.path(), &repos).unwrap();
        assert_eq!(laid[0], ("side-tool".to_owned(), b.path().join("src/st")));
        assert!(a.path().join("code/side-tool/.git").is_dir());
    }

    #[test]
    fn repo_without_remote_has_no_origin() {
        let root = tempfile::tempdir().unwrap();
        let repo = FakeRepo {
            entry: "scratch".into(),
            dir: "code/scratch".into(),
            origin_url: None,
            readme: None,
            manifest: ManifestKind::None,
            description: None,
        };
        let path = repo.create_under(root.path()).unwrap();
        assert!(git(&path, &["config", "--get", "remote.origin.url"]).is_err());
        assert!(!path.join("README.md").exists());
    }
}
