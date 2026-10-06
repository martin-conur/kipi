//! Shell completions through `clap_complete`'s dynamic engine (`docs/decisions.md`,
//! "Stage 4: completions"). `kipi completions <shell>` prints a short registration
//! script; while completing, the shell runs `KIPI_COMPLETE=<shell> kipi -- <words>`,
//! which [`handle_request`] answers before any other work. Subcommands and flags come
//! from the clap tree; repo, alias, skill, scope and store names come from the stores
//! mounted by `config.toml` (or `$KIPI_STORE`), so they are always current.

use std::io::Write;

use clap::CommandFactory;
use clap_complete::env::{Bash, Elvish, Fish, Powershell, Shells, Zsh};
use clap_complete::{CompleteEnv, CompletionCandidate};
use kipi_core::config::{self, Env};
use kipi_core::model::StoreSet;

use crate::cli::Cli;
use crate::error::Failure;

/// The variable that turns a `kipi` run into a completion request.
pub const ENV_VAR: &str = "KIPI_COMPLETE";

/// The shells `kipi completions` supports.
pub const SHELL_NAMES: [&str; 5] = ["bash", "elvish", "fish", "powershell", "zsh"];

const SHELLS: Shells<'static> = Shells(&[&Bash, &Elvish, &Fish, &Powershell, &Zsh]);

/// Answer a completion request and exit, if this run is one; otherwise return.
pub fn handle_request() {
    CompleteEnv::with_factory(Cli::command)
        .var(ENV_VAR)
        .shells(SHELLS)
        .complete();
}

/// `kipi completions <shell>`: write the registration script for `shell`.
pub fn run(shell: &str) -> Result<(), Failure> {
    let completer = SHELLS.completer(shell).ok_or_else(|| {
        Failure::user(
            format!("no completions for shell `{shell}`"),
            format!("pick one of: {}", SHELL_NAMES.join(", ")),
        )
    })?;
    let mut buf = Vec::new();
    completer
        .write_registration(ENV_VAR, "kipi", "kipi", "kipi", &mut buf)
        .map_err(Failure::Output)?;
    std::io::stdout().write_all(&buf).map_err(Failure::Output)
}

/// The mounted stores, or `None` when anything fails: completion never prints errors.
fn mounted() -> Option<StoreSet> {
    let env = Env::from_process().ok()?;
    let cfg = config::load_config(&env).ok()?;
    let paths = config::store_paths(&[], &env, &cfg);
    config::load_stores(&paths, &env).ok()
}

fn candidates(f: impl Fn(&StoreSet, &mut Vec<CompletionCandidate>)) -> Vec<CompletionCandidate> {
    let mut out = Vec::new();
    if let Some(stores) = mounted() {
        f(&stores, &mut out);
    }
    out
}

fn repo_names(stores: &StoreSet, out: &mut Vec<CompletionCandidate>) {
    for repo in stores.stores().iter().flat_map(|s| &s.repos) {
        let summary = repo.summary.as_ref().map(|s| s.value.clone());
        out.push(CompletionCandidate::new(&repo.name).help(summary.map(Into::into)));
    }
}

/// Repo entry names.
pub fn repos() -> Vec<CompletionCandidate> {
    candidates(repo_names)
}

/// Repo entry names and their aliases (`kipi where`).
pub fn aliases() -> Vec<CompletionCandidate> {
    candidates(|stores, out| {
        repo_names(stores, out);
        for repo in stores.stores().iter().flat_map(|s| &s.repos) {
            for alias in &repo.aliases.value {
                out.push(
                    CompletionCandidate::new(alias).help(Some(format!("→ {}", repo.name).into())),
                );
            }
        }
    })
}

fn skill_names(stores: &StoreSet, out: &mut Vec<CompletionCandidate>) {
    for store in stores.stores() {
        for skill in &store.skills {
            let help = skill.description.clone().map(Into::into);
            out.push(CompletionCandidate::new(&skill.name).help(help.clone()));
            if store.share_skills {
                let qualified = format!("{}/{}", store.name, skill.name);
                out.push(CompletionCandidate::new(qualified).help(help));
            }
        }
    }
}

/// Skill names, plus `<store>/<skill>` for stores that share their skills.
pub fn skills() -> Vec<CompletionCandidate> {
    candidates(skill_names)
}

/// Scope names.
pub fn scopes() -> Vec<CompletionCandidate> {
    candidates(|stores, out| {
        for scope in stores.stores().iter().flat_map(|s| &s.scopes) {
            out.push(CompletionCandidate::new(&scope.name));
        }
    })
}

/// Store names.
pub fn stores() -> Vec<CompletionCandidate> {
    candidates(|stores, out| {
        for store in stores.stores() {
            out.push(CompletionCandidate::new(&store.name));
        }
    })
}

/// `kipi edit` targets: repo entries, skills, `notes:<repo>` and `conventions:<scope>`.
pub fn edit_targets() -> Vec<CompletionCandidate> {
    candidates(|stores, out| {
        repo_names(stores, out);
        for store in stores.stores() {
            for skill in &store.skills {
                out.push(CompletionCandidate::new(&skill.name));
            }
            for repo in &store.repos {
                out.push(CompletionCandidate::new(format!("notes:{}", repo.name)));
            }
            for scope in &store.scopes {
                out.push(CompletionCandidate::new(format!(
                    "conventions:{}",
                    scope.name
                )));
            }
        }
    })
}
