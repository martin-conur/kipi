//! `machine.toml` (roadmap §5.2): this machine's repo paths and root overrides.

use std::path::Path;

use serde::Deserialize;
use toml_edit::{Array, DocumentMut, Item, Value};

use super::NAME_HINT;
use super::paths::{ABSOLUTE_PATH_HINT, Env, absolute_path};
use super::source::{Entries, Source};
use crate::error::{Error, Result};
use crate::model::{Machine, MachineRepo, MachineRoots, check_name};
use crate::scan::{FileWrite, write_files};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMachine {
    hostname: Option<String>,
    #[serde(default)]
    roots: Entries<Vec<String>>,
    #[serde(default)]
    repos: Entries<String>,
    #[serde(default)]
    approved_commands: Vec<String>,
}

/// Every key `machine.toml` accepts (see [`super::schema`]).
pub(super) fn schema_keys() -> Vec<String> {
    super::schema::fields::<RawMachine>()
        .iter()
        .map(|&key| match key {
            "roots" => "roots.<scope>".to_owned(),
            "repos" => "repos.<repo>".to_owned(),
            other => other.to_owned(),
        })
        .collect()
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
    for key in &raw.approved_commands {
        let hex = key.strip_prefix("sha256:").unwrap_or_default();
        if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(Error::invalid(
                src.at(&["approved_commands"]),
                format!("`{key}` is not a sync-command approval key"),
                "remove it; `kipi sync --allow-commands` writes the keys (`sha256:<hex>`)",
            ));
        }
    }
    Ok(Machine {
        hostname: raw.hostname,
        roots,
        repos,
        approved_commands: raw.approved_commands,
        source: Some(src.into_doc()),
    })
}

/// Add `keys` to `machine.toml`'s `approved_commands` (comments and order kept), creating
/// the file when there is none. Keys already there are not repeated.
pub fn approve_commands(env: &Env, machine: &Machine, keys: &[String]) -> Result<()> {
    let mut doc: DocumentMut = machine
        .source
        .as_ref()
        .map(|s| s.doc.clone())
        .unwrap_or_default();
    let item = doc
        .entry("approved_commands")
        .or_insert_with(|| Item::Value(Value::Array(Array::new())));
    let Some(list) = item.as_array_mut() else {
        return Err(Error::invalid(
            crate::error::Location::file(env.machine_file()),
            "`approved_commands` is not an array",
            "write it as `approved_commands = []`, or remove it",
        ));
    };
    for key in keys {
        if !list.iter().any(|v| v.as_str() == Some(key)) {
            list.push(key.as_str());
        }
    }
    if list.len() > 1 {
        for v in list.iter_mut() {
            v.decor_mut().set_prefix("\n    ");
        }
        list.set_trailing("\n");
        list.set_trailing_comma(true);
    }
    let path = env.machine_file();
    let before = std::fs::read_to_string(&path).ok();
    write_files(&[FileWrite {
        path,
        before,
        after: doc.to_string(),
    }])
}
