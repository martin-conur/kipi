//! `kipi add repo`: register a repo entry by hand, in the store owning its scope.

use std::path::Path;

use toml_edit::{Item, Value};

use super::{Change, Op, Outcome, absolute, bindings_write, file_write, find_scope, revalidate};
use crate::config::{self, Env};
use crate::error::{Error, Result};
use crate::model::{Machine, RemoteUrl, StoreSet, check_name};
use crate::scan::write::{add_repo as add_entry, set_machine_repo};
use crate::scan::{Placement, place};

/// The arguments of `add repo`.
#[derive(Debug, Clone, Copy, Default)]
pub struct RepoOptions<'a> {
    /// The entry name.
    pub name: &'a str,
    /// `--scope`.
    pub scope: Option<&'a str>,
    /// `--path`: its checkout on this machine.
    pub path: Option<&'a Path>,
    /// `--summary`.
    pub summary: Option<&'a str>,
}

/// Add `[repos.<name>]` (with `remote` read from the checkout, `scope` when the path does
/// not already place it there, and `summary`) to the store owning its scope, and record
/// `--path` in `machine.toml`. Both edits are validated together before anything is written.
pub fn add_repo(
    stores: &StoreSet,
    machine: &Machine,
    env: &Env,
    opts: &RepoOptions,
) -> Result<Outcome> {
    let name = opts.name;
    check_name(name).map_err(|why| {
        Error::usage(
            format!("invalid repo entry name `{name}`: the name {why}"),
            "use letters, digits, `-`, `_` and `.`, starting with a letter or digit",
        )
    })?;
    if let Some((store, _)) = stores.repo(name) {
        return Err(Error::usage(
            format!(
                "repo entry `{name}` already exists in store `{}`",
                store.name
            ),
            format!("edit it with `kipi bind {name} ...`, or pick another name"),
        ));
    }
    if let Some(summary) = opts.summary
        && (summary.trim().is_empty() || summary.contains('\n'))
    {
        return Err(Error::usage(
            "--summary must be one non-empty line",
            "longer knowledge belongs in repos/<name>/notes.md",
        ));
    }

    let path = opts.path.map(|p| absolute(p, env)).transpose()?;
    let mut remote = None;
    if let Some(path) = &path {
        if !path.is_dir() {
            return Err(Error::usage(
                format!("{} is not a directory", path.display()),
                "pass the repo's checkout with --path",
            ));
        }
        if let Some(other) = machine.repos.iter().find(|r| r.path == *path) {
            return Err(Error::usage(
                format!(
                    "{} is already recorded for `{}` in machine.toml",
                    path.display(),
                    other.entry
                ),
                format!(
                    "that checkout is `{}`; edit it with `kipi bind`",
                    other.entry
                ),
            ));
        }
        remote = crate::git::origin_url(path)?.and_then(|u| RemoteUrl::parse(&u).ok());
        if let Some((_, other)) = remote
            .as_ref()
            .and_then(|r| stores.repos().find(|(_, e)| e.remote.as_ref() == Some(r)))
        {
            return Err(Error::usage(
                format!(
                    "the checkout's remote {} already belongs to entry `{}`",
                    remote.as_ref().map(RemoteUrl::as_str).unwrap_or_default(),
                    other.name
                ),
                format!(
                    "it is the same repository; `kipi scan {}` records it as `{}`",
                    path.display(),
                    other.name
                ),
            ));
        }
    }

    let placed = path.as_deref().map(|p| place(stores.stores(), machine, p));
    let (store, explicit) = match opts.scope {
        Some(s) => {
            let (owner, scope) = find_scope(stores, s)?;
            let placed_here =
                matches!(&placed, Some(Placement::Scope(_, sc)) if sc.name == scope.name);
            (owner, (!placed_here).then_some(scope.name.as_str()))
        }
        None => match placed {
            Some(Placement::Scope(owner, _)) => (owner, None),
            Some(Placement::Tie(names)) => {
                return Err(Error::usage(
                    format!(
                        "the checkout lies under roots of scopes {} at the same depth",
                        names.join(", ")
                    ),
                    "pass --scope <scope> to choose",
                ));
            }
            _ => match stores.stores() {
                [only] => (only, None),
                _ => {
                    return Err(Error::usage(
                        "several stores are mounted and no scope root holds the repo",
                        "pass --scope <scope>; the entry goes to the store owning it",
                    ));
                }
            },
        },
    };

    let mut changes = vec![Change::new(
        Op::Add,
        format!("store {}", store.name),
        format!("[repos.{name}]"),
    )];
    let mut items: Vec<(&str, Item)> = Vec::new();
    if let Some(r) = &remote {
        items.push(("remote", Item::Value(Value::from(r.as_str()))));
        changes.push(Change::new(Op::Add, "remote", r.as_str()));
    }
    if let Some(scope) = explicit {
        items.push(("scope", Item::Value(Value::from(scope))));
        changes.push(Change::new(Op::Add, "scope", scope));
    }
    if let Some(summary) = opts.summary {
        items.push(("summary", Item::Value(Value::from(summary))));
        changes.push(Change::new(Op::Add, "summary", summary));
    }
    let mut doc = store.bindings.doc.clone();
    add_entry(&mut doc, name, items);
    let text = doc.to_string();

    let machine_file = env.machine_file();
    let mut machine_text = None;
    let mut edited_machine = None;
    if let Some(path) = &path {
        let Some(raw) = path.to_str() else {
            return Err(Error::usage(
                format!("{} is not valid UTF-8", path.display()),
                "machine.toml cannot hold the path; rename the directory",
            ));
        };
        let mut mdoc = machine
            .source
            .as_ref()
            .map(|s| s.doc.clone())
            .unwrap_or_default();
        set_machine_repo(&mut mdoc, name, raw);
        let mtext = mdoc.to_string();
        edited_machine = Some(
            config::parse_machine_text(&machine_file, mtext.clone(), env)
                .map_err(Error::rejected)?,
        );
        machine_text = Some(mtext);
        changes.push(Change::new(
            Op::Add,
            "machine.toml",
            path.display().to_string(),
        ));
    }
    revalidate(
        stores,
        edited_machine.as_ref().unwrap_or(machine),
        env,
        &[(&store.name, text.clone())],
        None,
    )?;

    let mut writes: Vec<_> = bindings_write(store, &text).into_iter().collect();
    writes.extend(machine_text.and_then(|t| file_write(machine_file, t)));
    Ok(Outcome {
        store: Some(store.name.clone()),
        changes,
        writes,
        ..Outcome::default()
    })
}
