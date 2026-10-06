//! K006: an absolute home path in a store's `bindings.toml`, notes or conventions.
//! Stores travel between machines (and some are shared), so paths in them are written
//! `~/…`; absolute paths belong in `machine.toml`, which never leaves the machine.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;

use crate::error::{Error, Location, Result};
use crate::model::Store;

use super::{Code, Finding, Inputs};

/// `/Users/<name>` or `/home/<name>`, not inside a longer word or after `~`.
static HOME_PATH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:^|[^A-Za-z0-9._~/-])(/(?:Users|home)/[A-Za-z0-9._-]+)")
        .expect("home path regex is valid")
});

pub(super) fn check(inputs: &Inputs<'_>, out: &mut Vec<Finding>) -> Result<()> {
    for store in inputs.stores.stores() {
        for (repo, file) in files(store, inputs)? {
            let text = fs::read_to_string(&file)
                .map_err(|e| Error::io(&file, e, "check the file's permissions"))?;
            let Some((line, found)) = first_home_path(&text, inputs.home) else {
                continue;
            };
            let at = Location::at(&file, line).in_store(&store.name);
            let shown = inputs.stores.provenance(&at);
            out.push(Finding::new(
                Code::HomePath,
                repo.as_deref(),
                Some(at),
                format!("{shown} holds the absolute home path `{found}`"),
                "write it as `~/…`: stores move between machines; absolute paths belong in \
                 machine.toml",
            ));
        }
    }
    Ok(())
}

/// The files of `store` to scan, each with the repo it belongs to: `bindings.toml`,
/// `conventions/*.md` and every scope's `always` file (store-level), and
/// `repos/<name>/notes.md`.
fn files(store: &Store, inputs: &Inputs<'_>) -> Result<Vec<(Option<String>, PathBuf)>> {
    let mut out: Vec<(Option<String>, PathBuf)> = Vec::new();
    if inputs.store_level() {
        out.push((None, store.bindings.path.clone()));
        let mut conventions = list(&store.root.join("conventions"))?;
        conventions.retain(|p| p.extension().is_some_and(|e| e == "md") && p.is_file());
        let always = store.scopes.iter().flat_map(|s| &s.always);
        conventions.extend(always.map(|f| store.root.join(f)).filter(|p| p.is_file()));
        for path in conventions {
            if !out.iter().any(|(_, p)| *p == path) {
                out.push((None, path));
            }
        }
    }
    for dir in list(&store.root.join("repos"))? {
        let notes = dir.join("notes.md");
        let name = dir.file_name().map(|n| n.to_string_lossy().into_owned());
        let wanted = inputs.only.is_none_or(|o| name.as_deref() == Some(o));
        if wanted && notes.is_file() {
            out.push((name, notes));
        }
    }
    Ok(out)
}

/// The entries of `dir`, sorted; none when it does not exist.
fn list(dir: &Path) -> Result<Vec<PathBuf>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Error::io(dir, e, "check the directory's permissions")),
    };
    let mut out = entries
        .map(|e| e.map(|e| e.path()))
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(|e| Error::io(dir, e, "check the directory's permissions"))?;
    out.sort();
    Ok(out)
}

/// The 1-based line and text of the first absolute home path in `text`: a
/// `/Users/<name>` or `/home/<name>` path, or this machine's home directory.
pub fn first_home_path(text: &str, home: &Path) -> Option<(usize, String)> {
    let home = home
        .to_str()
        .filter(|h| h.len() > 1)
        .map(|h| h.trim_end_matches('/'));
    text.lines().enumerate().find_map(|(i, line)| {
        if let Some(m) = HOME_PATH.captures(line).and_then(|c| c.get(1)) {
            return Some((i + 1, m.as_str().to_owned()));
        }
        let home = home?;
        let found = line.match_indices(home).any(|(at, _)| {
            let before = line[..at].chars().next_back();
            let after = line[at + home.len()..].chars().next();
            !before.is_some_and(|c| c.is_alphanumeric() || "._~/-".contains(c))
                && !after.is_some_and(|c| c.is_alphanumeric() || "._-".contains(c))
        });
        found.then(|| (i + 1, home.to_owned()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_home_paths_but_not_tilde_or_other_absolute_paths() {
        let home = Path::new("/srv/me");
        let users = format!("/{}/{}", "Users", "someone");
        let hit = |text: &str| first_home_path(text, home);
        assert_eq!(
            hit(&format!("a\nroots = [\"{users}/w\"]\n")),
            Some((2, users.clone()))
        );
        let linux = format!("/{}/{}", "home", "dev");
        assert_eq!(hit(&format!("see {linux}/x")), Some((1, linux)));
        assert_eq!(
            hit("roots = [\"/srv/me/code\"]"),
            Some((1, "/srv/me".into()))
        );
        assert_eq!(hit("roots = [\"/srv/me\"]"), Some((1, "/srv/me".into())));
        assert_eq!(hit("roots = [\"~/code\"]"), None);
        assert_eq!(hit("roots = [\"/Volumes/work\"]"), None);
        assert_eq!(hit("example.com/home/page and ./home/x"), None);
        assert_eq!(hit("/srv/meadow"), None, "a longer name is not the home");
        assert_eq!(
            first_home_path("/x/y", Path::new("/")),
            None,
            "home `/` matches nothing"
        );
    }
}
