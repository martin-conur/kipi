//! `kipi add repo`: register a repo entry by hand, in the store owning its scope, or record
//! the checkout of an entry that has none. `kipi remove repo`: drop an entry.

use std::path::Path;

use toml_edit::{DocumentMut, Item, Value};

use super::toml::remove_table;
use super::{
    Change, Op, Outcome, absolute, bindings_write, file_write, find_repo, find_scope, revalidate,
};
use crate::apply::Target;
use crate::config::{self, Env};
use crate::error::{Error, Result};
use crate::model::{Machine, RemoteUrl, RepoEntry, Store, StoreSet, check_name};
use crate::scan::write::{add_repo as add_entry, set_machine_repo, set_repo_key};
use crate::scan::{Placement, draft_missing, foreign_pattern, place, taken_keys};

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
///
/// For an existing entry with no checkout on this machine, `--path` records one instead
/// (see [`fill_path`]).
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
    if let Some(summary) = opts.summary
        && (summary.trim().is_empty() || summary.contains('\n'))
    {
        return Err(Error::usage(
            "--summary must be one non-empty line",
            "longer knowledge belongs in repos/<name>/notes.md",
        ));
    }
    if let Some((store, entry)) = stores.repo(name) {
        return fill_path(stores, machine, env, opts, store, entry);
    }

    let path = opts.path.map(|p| absolute(p, env)).transpose()?;
    let remote = match &path {
        Some(path) => checkout(stores, machine, path, None)?,
        None => None,
    };

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

    let edited = path
        .as_deref()
        .map(|p| record(machine, env, name, p))
        .transpose()?;
    if let Some(path) = &path {
        changes.push(Change::new(
            Op::Add,
            "machine.toml",
            path.display().to_string(),
        ));
    }
    revalidate(
        stores,
        edited.as_ref().map_or(machine, |(_, m)| m),
        env,
        &[(&store.name, text.clone())],
        None,
    )?;
    Ok(outcome(store, &text, edited.map(|(t, _)| t), env, changes))
}

/// `add repo <existing> --path <dir>`: record the checkout of an entry that has none on
/// this machine, filling its `remote` when it lacks one and drafting the `summary` /
/// `aliases` it does not set (as `scan` does). Every other flag must agree with the entry
/// as it is; a conflict is refused.
fn fill_path(
    stores: &StoreSet,
    machine: &Machine,
    env: &Env,
    opts: &RepoOptions,
    store: &Store,
    entry: &RepoEntry,
) -> Result<Outcome> {
    let name = opts.name;
    let exists = || {
        Error::usage(
            format!(
                "repo entry `{name}` already exists in store `{}`",
                store.name
            ),
            format!(
                "edit it with `kipi bind {name} ...`, record its checkout with \
                 `kipi add repo {name} --path <dir>`, or pick another name"
            ),
        )
    };
    let Some(raw) = opts.path else {
        return Err(exists());
    };
    let path = absolute(raw, env)?;
    let recorded = machine.repos.iter().find(|r| r.entry == name);
    if let Some(r) = recorded.filter(|r| r.path.is_dir()) {
        if r.path == path && opts.scope.is_none() && opts.summary.is_none() {
            return Ok(Outcome {
                store: Some(store.name.clone()),
                changes: vec![
                    Change::new(Op::Keep, "machine.toml", path.display().to_string())
                        .note("already recorded"),
                ],
                ..Outcome::default()
            });
        }
        if r.path != path {
            return Err(Error::usage(
                format!(
                    "repo entry `{name}` is already cloned at {}",
                    r.path.display()
                ),
                format!("if the checkout moved, change `{name}` in machine.toml [repos]"),
            ));
        }
        return Err(exists());
    }
    let summary = entry.summary.as_ref().map(|s| s.value.as_str());
    if let Some(wanted) = opts.summary
        && summary != Some(wanted)
    {
        return Err(Error::usage(
            format!("--summary differs from the summary of entry `{name}`"),
            format!("drop --summary, or change the summary with `kipi edit {name}`"),
        ));
    }
    let wanted_scope = opts.scope.map(|s| find_scope(stores, s)).transpose()?;

    let remote = checkout(stores, machine, &path, Some(name))?;
    let mut changes = vec![Change::new(
        Op::Keep,
        format!("store {}", store.name),
        format!("[repos.{name}]"),
    )];
    let mut doc = store.bindings.doc.clone();
    match (&entry.remote, &remote) {
        (Some(have), Some(found)) if have != found => {
            return Err(Error::usage(
                format!("the checkout's remote {found} is not the remote {have} of entry `{name}`"),
                "pass the checkout of this repository, or `kipi add repo <new-name> --path` \
                 for another one",
            ));
        }
        (None, Some(found)) => {
            set_repo_key(&mut doc, name, "remote", Value::from(found.as_str()));
            changes.push(Change::new(Op::Add, "remote", found.as_str()));
        }
        _ => {}
    }
    // Like `scan`: never copy a README summary into the entry when another store's
    // `remote_patterns` claim the checkout.
    let foreign = remote
        .as_ref()
        .is_some_and(|r| foreign_pattern(stores, store, r).is_some());
    if !foreign {
        let remote = entry.remote.as_ref().or(remote.as_ref());
        let drafts = draft_missing(&mut doc, entry, remote, &path, &taken_keys(stores, name));
        if let Some(summary) = drafts.summary {
            changes.push(Change::new(Op::Add, "summary", summary).note("draft"));
        }
        if !drafts.aliases.is_empty() {
            changes.push(Change::new(Op::Add, "aliases", drafts.aliases.join(", ")).note("draft"));
        }
    }
    let text = doc.to_string();
    let (machine_text, edited) = record(machine, env, name, &path)?;
    changes.push(Change::new(
        Op::Add,
        "machine.toml",
        path.display().to_string(),
    ));
    let plan = revalidate(stores, &edited, env, &[(&store.name, text.clone())], None)?;
    if let Some((_, scope)) = wanted_scope {
        let placed = plan.repo(name).and_then(|r| r.scope.value.clone());
        if placed.as_deref() != Some(scope.name.as_str()) {
            return Err(Error::usage(
                format!(
                    "entry `{name}` resolves to {}, not scope `{}`",
                    placed.map_or("no scope".to_owned(), |s| format!("scope `{s}`")),
                    scope.name
                ),
                format!("drop --scope, or set the entry's `scope` with `kipi edit {name}`"),
            ));
        }
    }
    Ok(outcome(store, &text, Some(machine_text), env, changes))
}

/// Check `path` as a checkout to record for entry `owner` (`None` for a new entry): an
/// existing directory no other entry records, whose remote no other entry has. Returns
/// that remote.
fn checkout(
    stores: &StoreSet,
    machine: &Machine,
    path: &Path,
    owner: Option<&str>,
) -> Result<Option<RemoteUrl>> {
    if !path.is_dir() {
        return Err(Error::usage(
            format!("{} is not a directory", path.display()),
            "pass the repo's checkout with --path",
        ));
    }
    if let Some(other) = machine
        .repos
        .iter()
        .find(|r| r.path == path && Some(r.entry.as_str()) != owner)
    {
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
    let remote = crate::git::origin_url(path)?.and_then(|u| RemoteUrl::parse(&u).ok());
    if let Some((_, other)) = remote.as_ref().and_then(|r| {
        stores
            .repos()
            .find(|(_, e)| e.remote.as_ref() == Some(r) && Some(e.name.as_str()) != owner)
    }) {
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
    Ok(remote)
}

/// `machine.toml` with `entry = "<path>"` in `[repos]`: its text and the parsed result.
fn record(machine: &Machine, env: &Env, entry: &str, path: &Path) -> Result<(String, Machine)> {
    let Some(raw) = path.to_str() else {
        return Err(Error::usage(
            format!("{} is not valid UTF-8", path.display()),
            "machine.toml cannot hold the path; rename the directory",
        ));
    };
    let mut doc = machine_doc(machine);
    set_machine_repo(&mut doc, entry, raw);
    parse_machine(doc, env)
}

fn machine_doc(machine: &Machine) -> DocumentMut {
    machine
        .source
        .as_ref()
        .map(|s| s.doc.clone())
        .unwrap_or_default()
}

fn parse_machine(doc: DocumentMut, env: &Env) -> Result<(String, Machine)> {
    let text = doc.to_string();
    let parsed = config::parse_machine_text(&env.machine_file(), text.clone(), env)
        .map_err(Error::rejected)?;
    Ok((text, parsed))
}

/// The outcome writing `text` into `store`'s `bindings.toml` and, if given, `machine.toml`.
fn outcome(
    store: &Store,
    text: &str,
    machine_text: Option<String>,
    env: &Env,
    changes: Vec<Change>,
) -> Outcome {
    let mut writes: Vec<_> = bindings_write(store, text).into_iter().collect();
    writes.extend(machine_text.and_then(|t| file_write(env.machine_file(), t)));
    Outcome {
        store: Some(store.name.clone()),
        changes,
        writes,
        ..Outcome::default()
    }
}

/// The arguments of `remove repo`.
#[derive(Debug, Clone, Copy, Default)]
pub struct RemoveOptions<'a> {
    /// The entry name.
    pub name: &'a str,
    /// `--notes`: delete its `notes.md` in the store too.
    pub notes: bool,
}

/// Remove `[repos.<name>]` from its store and its line from `machine.toml [repos]`, and
/// clean what kipi wrote into its checkout (and linked worktrees). Refused while another
/// entry's `related` names it. `notes.md` is kept unless `--notes`.
pub fn remove_repo(
    stores: &StoreSet,
    machine: &Machine,
    env: &Env,
    opts: &RemoveOptions,
) -> Result<Outcome> {
    let (store, entry) = find_repo(stores, opts.name)?;
    let name = entry.name.as_str();
    let referrers: Vec<&str> = stores
        .repos()
        .filter(|(_, e)| e.name != name && e.related.iter().any(|r| r == name))
        .map(|(_, e)| e.name.as_str())
        .collect();
    if let Some(first) = referrers.first() {
        return Err(Error::usage(
            format!(
                "repo entry `{name}` is related to by `{}`",
                referrers.join("`, `")
            ),
            format!("unbind it first: `kipi unbind {first} --related {name}`"),
        ));
    }

    let mut doc = store.bindings.doc.clone();
    remove_table(&mut doc, "repos", name);
    let text = doc.to_string();
    let mut changes = vec![
        Change::new(
            Op::Remove,
            format!("store {}", store.name),
            format!("[repos.{name}]"),
        )
        .note("the next `kipi sync --all` drops it from the registries"),
    ];

    let recorded = machine.repos.iter().find(|r| r.entry == name);
    let mut edited = None;
    if let Some(r) = recorded {
        let mut mdoc = machine_doc(machine);
        if let Some(repos) = mdoc.get_mut("repos").and_then(Item::as_table_like_mut) {
            repos.remove(name);
        }
        edited = Some(parse_machine(mdoc, env)?);
        changes.push(Change::new(
            Op::Remove,
            "machine.toml",
            r.path.display().to_string(),
        ));
    }
    revalidate(
        stores,
        edited.as_ref().map_or(machine, |(_, m)| m),
        env,
        &[(&store.name, text.clone())],
        None,
    )?;

    // `machine.toml` is how `sync --all` finds an orphaned checkout, so once its line is
    // gone no sync would; the files go now instead.
    let mut clean = Vec::new();
    if let Some(r) = recorded.filter(|r| r.path.is_dir()) {
        for (dir, _) in crate::sync::trees(&r.path, true) {
            if crate::sync::exists(&Target::repo(&dir).manifest) {
                changes.push(Change::new(
                    Op::Remove,
                    "kipi files",
                    dir.display().to_string(),
                ));
                clean.push(dir);
            }
        }
    }
    let mut removes = Vec::new();
    if let Some(notes) = &entry.notes {
        let value = notes.display().to_string();
        if opts.notes {
            changes.push(Change::new(Op::Remove, "notes", value));
            removes.push(notes.clone());
        } else {
            changes.push(Change::new(Op::Keep, "notes", value).note("pass --notes to delete it"));
        }
    }
    Ok(Outcome {
        clean,
        removes,
        ..outcome(store, &text, edited.map(|(t, _)| t), env, changes)
    })
}
