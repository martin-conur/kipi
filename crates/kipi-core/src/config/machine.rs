//! `machine.toml` (roadmap §5.2): this machine's repo paths and root overrides.

use std::path::Path;

use serde::Deserialize;

use super::NAME_HINT;
use super::paths::{ABSOLUTE_PATH_HINT, Env, absolute_path};
use super::source::{Entries, Source};
use crate::error::{Error, Result};
use crate::model::{Machine, MachineRepo, MachineRoots, check_name};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMachine {
    hostname: Option<String>,
    #[serde(default)]
    roots: Entries<Vec<String>>,
    #[serde(default)]
    repos: Entries<String>,
}

/// Load `machine.toml` from the config dir; an absent file is an empty [`Machine`].
pub fn load_machine(env: &Env) -> Result<Machine> {
    match Source::read_optional(&env.machine_file())? {
        Some(src) => parse_machine(src, &env.home),
        None => Ok(Machine::default()),
    }
}

/// Parse a `machine.toml` read from `path`.
pub fn parse_machine_text(path: &Path, text: String, env: &Env) -> Result<Machine> {
    parse_machine(Source::parse(path, text)?, &env.home)
}

fn parse_machine(src: Source, home: &Path) -> Result<Machine> {
    let raw: RawMachine = src.deserialize("change the value to the expected type")?;
    let name = |keys: &[&str], value: &str, what: &str| {
        check_name(value).map_err(|why| {
            Error::invalid(
                src.at(keys),
                format!("invalid {what} name `{value}`: {why}"),
                NAME_HINT,
            )
        })
    };
    let path = |keys: &[&str], raw: &str| {
        absolute_path(raw, home).map_err(|m| Error::invalid(src.at(keys), m, ABSOLUTE_PATH_HINT))
    };

    let mut roots = Vec::with_capacity(raw.roots.0.len());
    for (scope, paths) in raw.roots.0 {
        let keys = ["roots", scope.as_str()];
        name(&keys, &scope, "scope")?;
        roots.push(MachineRoots {
            roots: paths
                .iter()
                .map(|p| path(&keys, p))
                .collect::<Result<_>>()?,
            at: src.at(&keys),
            scope,
        });
    }
    let mut repos = Vec::with_capacity(raw.repos.0.len());
    for (entry, p) in raw.repos.0 {
        let keys = ["repos", entry.as_str()];
        name(&keys, &entry, "repo")?;
        repos.push(MachineRepo {
            path: path(&keys, &p)?,
            at: src.at(&keys),
            entry,
        });
    }
    Ok(Machine {
        hostname: raw.hostname,
        roots,
        repos,
        source: Some(src.into_doc()),
    })
}
