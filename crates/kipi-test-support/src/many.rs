//! A generated store with many repos, for timing `tree` and `status` (roadmap §10: under
//! ~100 ms for 50 repos with a warm token cache).
//!
//! [`many_repos`] lays out a whole home directory: a store `bench` at `~/store` with a
//! root scope (registry, conventions, two skills) and four team scopes extending it, one
//! git checkout per repo under `~/code/team-<k>/`, and `config.toml` + `machine.toml`
//! under `~/.config/kipi`. Run `kipi` with `HOME` set to [`ManyRepos::home`].

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::repos::{FakeRepo, ManifestKind, create_fake_repo};

/// Team scopes the repos are spread over.
pub const TEAMS: usize = 4;

/// A home laid out by [`many_repos`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManyRepos {
    /// The home directory.
    pub home: PathBuf,
    /// The store, `~/store`.
    pub store: PathBuf,
    /// The repo entry names, in order.
    pub repos: Vec<String>,
}

/// The entry name of repo `i`.
pub fn repo_name(i: usize) -> String {
    format!("repo-{i:02}")
}

/// Lay out `n` repos under the home directory `home` (created if missing).
pub fn many_repos(home: &Path, n: usize) -> io::Result<ManyRepos> {
    let store = home.join("store");
    write(&store.join("conventions/bench.md"), CONVENTIONS)?;
    let mut skills = vec!["review".to_owned(), "testing".to_owned()];
    skills.extend((0..TEAMS).map(|k| format!("team-{k}-recipes")));
    for skill in &skills {
        write(
            &store.join(format!("skills/{skill}/SKILL.md")),
            &format!(
                "---\nname: {skill}\ndescription: Use when working on {skill} tasks in a bench \
                 repo; covers the steps, the checks to run and the files to touch.\n---\n\n\
                 # {skill}\n\nSteps go here.\n"
            ),
        )?;
    }

    let mut bindings = String::from(
        "[store]\nname = \"bench\"\nversion = 1\n\n\
         [scopes.bench]\nroots = [\"~/code\"]\nskills = [\"review\", \"testing\"]\n\
         always = [\"conventions/bench.md\"]\nregistry = true\n",
    );
    for k in 0..TEAMS {
        write!(
            bindings,
            "\n[scopes.team-{k}]\nextends = \"bench\"\nroots = [\"~/code/team-{k}\"]\n\
             skills = [\"team-{k}-recipes\"]\n"
        )
        .expect("writing to a String");
    }
    let mut machine = String::from("hostname = \"bench\"\n\n[repos]\n");
    let mut names = Vec::with_capacity(n);
    for i in 0..n {
        let name = repo_name(i);
        let team = i % TEAMS;
        let dir = home.join(format!("code/team-{team}/{name}"));
        write!(
            bindings,
            "\n[repos.{name}]\nremote = \"example.com/bench/{name}\"\n\
             summary = \"Service {i} of team {team}: handles part {i} of the bench workload\"\n\
             aliases = [\"r{i}\", \"part-{i}\"]\n"
        )
        .expect("writing to a String");
        if i + TEAMS < n {
            // The next repo of the same team: related repos stay in one scope.
            writeln!(bindings, "related = [\"{}\"]", repo_name(i + TEAMS))
                .expect("writing to a String");
        }
        if i % 5 == 0 {
            write(
                &store.join(format!("repos/{name}/notes.md")),
                "## Gotchas\n\nNever call the v1 endpoint.\n",
            )?;
        }
        create_fake_repo(
            &dir,
            &FakeRepo {
                entry: name.clone(),
                dir: PathBuf::from(&name),
                origin_url: Some(format!("https://example.com/bench/{name}.git")),
                readme: Some(format!("# {name}\n")),
                manifest: ManifestKind::None,
                description: None,
            },
        )?;
        // A commit gives the checkout an index, as every real clone has.
        commit_all(&dir)?;
        writeln!(machine, "\"{name}\" = \"{}\"", dir.display()).expect("writing to a String");
        names.push(name);
    }
    write(&store.join("bindings.toml"), &bindings)?;
    let config = home.join(".config/kipi");
    write(&config.join("config.toml"), "stores = [\"~/store\"]\n")?;
    write(&config.join("machine.toml"), &machine)?;
    Ok(ManyRepos {
        home: home.to_path_buf(),
        store,
        repos: names,
    })
}

fn commit_all(dir: &Path) -> io::Result<()> {
    for args in [
        &["add", "--all"][..],
        &[
            "-c",
            "user.name=bench",
            "-c",
            "user.email=bench@example.com",
            "commit",
            "--quiet",
            "--no-gpg-sign",
            "-m",
            "init",
        ],
    ] {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .status()?
            .success();
        if !ok {
            return Err(io::Error::other(format!(
                "git {args:?} failed in {}",
                dir.display()
            )));
        }
    }
    Ok(())
}

const CONVENTIONS: &str = "Write small, reviewed changes. Every service exposes /health and \
logs as JSON. Tests run with `make test`; never merge on red.\n";

fn write(path: &Path, text: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lays_out_a_store_checkouts_and_config() {
        let dir = tempfile::tempdir().unwrap();
        let many = many_repos(dir.path(), 6).unwrap();
        assert_eq!(many.repos.len(), 6);
        assert!(many.store.join("bindings.toml").is_file());
        assert!(dir.path().join("code/team-1/repo-05/.git/index").is_file());
        assert!(many.store.join("repos/repo-05/notes.md").is_file());
        let machine = fs::read_to_string(dir.path().join(".config/kipi/machine.toml")).unwrap();
        assert_eq!(machine.matches(" = \"").count(), 7, "hostname + 6 repos");
    }
}
