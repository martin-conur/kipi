//! Thin wrapper over the `git` binary (no libgit2).

use std::path::Path;
use std::process::Command;

use crate::error::{Error, Result};

/// The raw `remote.origin.url` of the checkout at `repo`, or `None` when it has no
/// `origin` remote.
pub fn origin_url(repo: &Path) -> Result<Option<String>> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["config", "--get", "remote.origin.url"])
        .output()
        .map_err(|e| Error::io(repo, e, "install git and make sure it is on PATH"))?;
    match out.status.code() {
        Some(0) => Ok(Some(String::from_utf8_lossy(&out.stdout).trim().to_owned())),
        // `git config --get` exits 1 when the key is unset.
        Some(1) => Ok(None),
        _ => Err(Error::io(
            repo,
            std::io::Error::other(String::from_utf8_lossy(&out.stderr).trim().to_owned()),
            "check that the directory is a readable git checkout",
        )),
    }
}
