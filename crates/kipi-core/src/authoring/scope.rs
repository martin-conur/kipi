//! `kipi add scope`: declare `[scopes.<name>]` in a store, so onboarding needs no
//! hand-edited TOML. `kipi init --scope` shares [`insert`]. `kipi scope`: change an
//! existing scope ([`edit_scope`]).

use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, Item, Table, TableLike, Value};

use super::bind::{check_skill_refs, dedup};
use super::toml::{contains, push, remove, table_mut};
use super::{
    Change, Op, Outcome, absolute, bindings_write, find_scope, find_store, revalidate, tilde,
};
use crate::config::{self, Env};
use crate::error::{Error, Result};
use crate::model::{Machine, RemoteUrl, Scope, Store, StoreSet, check_name};
use crate::scan::write::string_array;

/// The arguments of `add scope`.
#[derive(Debug, Clone, Copy, Default)]
pub struct ScopeOptions<'a> {
    /// The scope name.
    pub name: &'a str,
    /// `--root`: directories whose repos belong to the scope.
    pub roots: &'a [PathBuf],
    /// `--extends`: the parent scope, in the same store.
    pub extends: Option<&'a str>,
    /// `--registry` (`Some(true)`) or `--no-registry` (`Some(false)`); `None` builds a
    /// registry unless the scope inherits one through `extends` ([`registry_default`]).
    pub registry: Option<bool>,
    /// `--skill`: skill refs every repo in the scope gets.
    pub skills: &'a [String],
    /// `--remote-pattern`: `remote_patterns` prefixes (`host/owner/`).
    pub remote_patterns: &'a [String],
    /// `--store`: the store to declare it in (by name).
    pub store: Option<&'a str>,
}

/// Add `[scopes.<name>]` to the store named by `--store`, else the store owning
/// `--extends`, else the only mounted store. The edited store set is validated before
/// anything is written.
pub fn add_scope(
    stores: &StoreSet,
    machine: &Machine,
    env: &Env,
    opts: &ScopeOptions,
) -> Result<Outcome> {
    let roots = check(opts, env)?;
    let patterns = check_patterns(opts.remote_patterns, "--remote-pattern")?;
    let name = opts.name;
    if let Some((store, _)) = stores.scope(name) {
        return Err(Error::usage(
            format!("scope `{name}` already exists in store `{}`", store.name),
            format!("pick another name, or change it with `kipi scope {name} ...`"),
        ));
    }
    let store = target(stores, opts)?;
    let mut doc = store.bindings.doc.clone();
    let inherited = registry_default(opts, store);
    let registry = opts.registry.unwrap_or(inherited.is_none());
    let mut changes = declare(&mut doc, &store.name, opts, &roots, &patterns, registry);
    if let Some(parent) = inherited {
        changes.push(
            Change::new(Op::Keep, "registry", "false")
                .note(format!("inherits the registry of scope `{parent}`")),
        );
    }
    let text = doc.to_string();
    revalidate(stores, machine, env, &[(&store.name, text.clone())], None)?;
    Ok(Outcome {
        store: Some(store.name.clone()),
        changes,
        writes: bindings_write(store, &text).into_iter().collect(),
        ..Outcome::default()
    })
}

/// The store the scope goes in. `extends` resolves only within a store, so a parent in
/// another store than `--store` is refused.
fn target<'a>(stores: &'a StoreSet, opts: &ScopeOptions) -> Result<&'a Store> {
    let explicit = opts.store.map(|s| find_store(stores, s)).transpose()?;
    let parent = opts.extends.map(|p| find_scope(stores, p)).transpose()?;
    match (explicit, parent) {
        (Some(store), Some((owner, parent))) if owner.name != store.name => Err(Error::usage(
            format!(
                "scope `{}` is in store `{}`, not `{}`",
                parent.name, owner.name, store.name
            ),
            format!(
                "`extends` resolves only within a store: pass `--store {}`, or extend a \
                     scope of store `{}`",
                owner.name, store.name
            ),
        )),
        (Some(store), _) | (None, Some((store, _))) => Ok(store),
        (None, None) => match stores.stores() {
            [only] => Ok(only),
            [] => Err(Error::usage(
                "no store is mounted",
                "create one with `kipi init`, or pass `kipi init --scope <name> --root <path>`",
            )),
            _ => Err(Error::usage(
                "several stores are mounted",
                "pass --store <name> to choose the store that owns the scope",
            )),
        },
    }
}

/// Check the scope's name and roots; the roots as written into `bindings.toml` (`~/…`
/// under the home directory, never an absolute home path).
pub(super) fn check(opts: &ScopeOptions, env: &Env) -> Result<Vec<String>> {
    let name = opts.name;
    check_name(name).map_err(|why| {
        Error::usage(
            format!("invalid scope name `{name}`: the name {why}"),
            "use letters, digits, `-`, `_` and `.`, starting with a letter or digit",
        )
    })?;
    if opts.roots.is_empty() {
        return Err(Error::usage(
            format!("scope `{name}` needs a root"),
            "pass --root <path>: the directory holding the scope's repos",
        ));
    }
    let mut roots: Vec<String> = Vec::with_capacity(opts.roots.len());
    for raw in opts.roots {
        let path = absolute(raw, env)?;
        if !path.is_dir() {
            return Err(Error::usage(
                format!("{} is not a directory", path.display()),
                "pass an existing directory with --root; `kipi scan` walks only directories",
            ));
        }
        let written = tilde(&path, env);
        if !roots.contains(&written) {
            roots.push(written);
        }
    }
    Ok(roots)
}

/// Each remote pattern normalized ([`RemoteUrl::pattern`]), duplicates dropped; `flag`
/// names the option in the error.
fn check_patterns(raw: &[String], flag: &str) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::with_capacity(raw.len());
    for r in raw {
        let pattern = RemoteUrl::pattern(r).map_err(|why| {
            Error::usage(
                format!("{flag}: {why}"),
                "pass a remote prefix without a scheme, ending at the owner: \
                 `github.com/<owner>/`",
            )
        })?;
        if !out.contains(&pattern) {
            out.push(pattern);
        }
    }
    Ok(out)
}

/// Without `--registry` / `--no-registry`, a scope gets a registry, unless a scope it
/// extends already builds one: its repos are listed there, and a registry of its own
/// would show them only each other (the nearest registry wins, roadmap §6). Returns that
/// ancestor when the default leaves the registry off.
fn registry_default<'a>(opts: &ScopeOptions, store: &'a Store) -> Option<&'a str> {
    if opts.registry.is_some() {
        return None;
    }
    let mut next = opts.extends;
    let mut seen = Vec::new();
    while let Some(name) = next.filter(|n| !seen.contains(n)) {
        seen.push(name);
        let scope = store.scope(name)?;
        if scope.registry {
            return Some(&scope.name);
        }
        next = scope.extends.as_deref();
    }
    None
}

/// [`insert`], reported as a new table of store `store`. `registry` is the flag after
/// defaults; `roots` and `patterns` are checked ([`check`], [`check_patterns`]).
pub(super) fn declare(
    doc: &mut DocumentMut,
    store: &str,
    opts: &ScopeOptions,
    roots: &[String],
    patterns: &[String],
    registry: bool,
) -> Vec<Change> {
    let header = Change::new(
        Op::Add,
        format!("store {store}"),
        format!("[scopes.{}]", opts.name),
    );
    std::iter::once(header)
        .chain(insert(doc, opts, roots, patterns, registry))
        .collect()
}

/// Add `[scopes.<name>]` (`extends`, `roots`, `remote_patterns`, `skills`, `registry`, in
/// that order) to `doc`, after its last scope, else before its first repo entry. Returns
/// what it added.
fn insert(
    doc: &mut DocumentMut,
    opts: &ScopeOptions,
    roots: &[String],
    patterns: &[String],
    registry: bool,
) -> Vec<Change> {
    let mut changes = Vec::new();
    let mut table = Table::new();
    table.decor_mut().set_prefix("\n");
    if let Some(parent) = opts.extends {
        table.insert("extends", Item::Value(Value::from(parent)));
        changes.push(Change::new(Op::Add, "extends", parent));
    }
    table.insert("roots", Item::Value(string_array(roots)));
    changes.extend(roots.iter().map(|r| Change::new(Op::Add, "roots", r)));
    if !patterns.is_empty() {
        table.insert("remote_patterns", Item::Value(string_array(patterns)));
        changes.extend(
            patterns
                .iter()
                .map(|p| Change::new(Op::Add, "remote_patterns", p)),
        );
    }
    if !opts.skills.is_empty() {
        table.insert("skills", Item::Value(string_array(opts.skills)));
        changes.extend(
            opts.skills
                .iter()
                .map(|s| Change::new(Op::Add, "skills", s)),
        );
    }
    if registry {
        table.insert("registry", Item::Value(Value::from(true)));
        changes.push(Change::new(Op::Add, "registry", "true"));
    }
    // A new table without a position is printed after the last table before it in the
    // document; with no scope yet, `[scopes]` would land after every repo entry, so place
    // it just before the first one instead.
    if !doc.contains_key("scopes") {
        let first_repo = doc.get("repos").and_then(Item::as_table).and_then(|repos| {
            repos
                .iter()
                .filter_map(|(_, t)| t.as_table().and_then(Table::position))
                .min()
        });
        // `[store]` precedes every `[repos.*]` in a loaded store, so `p` is at least 1.
        table.set_position(first_repo.map(|p| p.saturating_sub(1)));
        let mut scopes = Table::new();
        scopes.set_implicit(true);
        doc.insert("scopes", Item::Table(scopes));
    }
    if let Some(scopes) = doc.get_mut("scopes").and_then(Item::as_table_mut) {
        scopes.insert(opts.name, Item::Table(table));
    }
    changes
}

/// The arguments of `kipi scope`.
#[derive(Debug, Clone, Copy, Default)]
pub struct ScopeEdit<'a> {
    /// The scope.
    pub name: &'a str,
    /// `--registry` (`Some(true)`) or `--no-registry` (`Some(false)`).
    pub registry: Option<bool>,
    /// `--add-root`.
    pub add_roots: &'a [PathBuf],
    /// `--remove-root`.
    pub remove_roots: &'a [PathBuf],
    /// `--extends <s>` (`Some(Some(s))`) or `--no-extends` (`Some(None)`).
    pub extends: Option<Option<&'a str>>,
    /// `--skill`.
    pub skills: &'a [String],
    /// `--remove-skill`.
    pub remove_skills: &'a [String],
    /// `--always`: always-on files, relative to the store root.
    pub always: &'a [String],
    /// `--remove-always`.
    pub remove_always: &'a [String],
    /// `--add-remote-pattern`.
    pub add_remote_patterns: &'a [String],
    /// `--remove-remote-pattern`.
    pub remove_remote_patterns: &'a [String],
}

/// Change the existing `[scopes.<name>]` in place (toml_edit, comments kept): toggle its
/// registry, add or remove roots, remote patterns, skills and always-on files, set or drop
/// `extends`.
/// Values already as asked are reported as kept. The edited store set is validated
/// before anything is written.
pub fn edit_scope(
    stores: &StoreSet,
    machine: &Machine,
    env: &Env,
    edit: &ScopeEdit,
) -> Result<Outcome> {
    let name = edit.name;
    let nothing = edit.registry.is_none()
        && edit.add_roots.is_empty()
        && edit.remove_roots.is_empty()
        && edit.extends.is_none()
        && edit.skills.is_empty()
        && edit.remove_skills.is_empty()
        && edit.always.is_empty()
        && edit.remove_always.is_empty()
        && edit.add_remote_patterns.is_empty()
        && edit.remove_remote_patterns.is_empty();
    if nothing {
        return Err(Error::usage(
            format!("nothing to change in scope `{name}`"),
            format!(
                "pass at least one of --registry, --no-registry, --add-root, --remove-root, \
                 --add-remote-pattern, --remove-remote-pattern, --extends, --no-extends, \
                 --skill, --remove-skill, --always or --remove-always, e.g. \
                 `kipi scope {name} --registry`"
            ),
        ));
    }
    let (store, scope) = find_scope(stores, name)?;
    check_skill_refs("--skill", edit.skills)?;
    check_skill_refs("--remove-skill", edit.remove_skills)?;
    let add_roots = check_roots(edit.add_roots, env)?;
    let add_patterns = check_patterns(edit.add_remote_patterns, "--add-remote-pattern")?;
    let always = check_always(edit.always, store, name)?;
    if let Some(Some(parent)) = edit.extends {
        check_parent(stores, store, name, parent)?;
    }

    let mut doc = store.bindings.doc.clone();
    let table = table_mut(&mut doc, &["scopes", name]).ok_or_else(|| {
        Error::usage(
            format!(
                "cannot edit [scopes.{name}] in {}",
                store.bindings.path.display()
            ),
            "declare the scope as a `[scopes.<name>]` table",
        )
    })?;
    let mut changes = Vec::new();
    if let Some(on) = edit.registry {
        changes.push(registry(table, scope, on));
    }
    if let Some(parent) = edit.extends {
        changes.push(extends(table, scope, parent));
    }
    for (path, written) in &add_roots {
        changes.push(if scope.roots.contains(path) {
            Change::new(Op::Keep, "roots", written).note("already there")
        } else {
            push(table, "roots", written);
            Change::new(Op::Add, "roots", written)
        });
    }
    for raw in edit.remove_roots {
        let path = absolute(raw, env)?;
        let same = |v: &str| config::expand_tilde(v, &env.home) == path;
        let shown = tilde(&path, env);
        changes.push(if contains(table, "roots", same) {
            remove(table, "roots", same);
            Change::new(Op::Remove, "roots", shown)
        } else {
            Change::new(Op::Keep, "roots", shown).note("not there")
        });
    }
    if table
        .get("roots")
        .and_then(Item::as_array)
        .is_none_or(|a| a.is_empty())
    {
        return Err(Error::usage(
            format!("scope `{name}` would be left without a root"),
            "add another with --add-root before removing the last one",
        ));
    }
    // Compared normalized, so `github.com/o` written by hand is `github.com/o/`; a
    // `--remove-remote-pattern` that does not normalize can still match a value verbatim.
    let normal = |v: &str| RemoteUrl::pattern(v).unwrap_or_else(|_| v.to_owned());
    for pattern in &add_patterns {
        let same = |v: &str| normal(v) == *pattern;
        changes.push(if contains(table, "remote_patterns", same) {
            Change::new(Op::Keep, "remote_patterns", pattern).note("already there")
        } else {
            push(table, "remote_patterns", pattern);
            Change::new(Op::Add, "remote_patterns", pattern)
        });
    }
    for raw in dedup(edit.remove_remote_patterns) {
        let wanted = normal(raw);
        let same = |v: &str| normal(v) == wanted;
        changes.push(if contains(table, "remote_patterns", same) {
            remove(table, "remote_patterns", same);
            Change::new(Op::Remove, "remote_patterns", &wanted)
        } else {
            Change::new(Op::Keep, "remote_patterns", &wanted).note("not there")
        });
    }
    for (key, values, adding) in [
        ("skills", edit.skills, true),
        ("skills", edit.remove_skills, false),
    ] {
        for value in dedup(values) {
            changes.push(array_edit(table, key, value, adding));
        }
    }
    for value in &always {
        changes.push(array_edit(table, "always", value, true));
    }
    let mut removed: Vec<PathBuf> = Vec::new();
    for raw in edit.remove_always {
        let wanted = store_relative(raw, store);
        if removed.contains(&wanted) {
            continue;
        }
        let same = |v: &str| store_relative(v, store) == wanted;
        let shown = wanted.display().to_string();
        changes.push(if contains(table, "always", same) {
            remove(table, "always", same);
            Change::new(Op::Remove, "always", shown)
        } else {
            Change::new(Op::Keep, "always", shown).note("not there")
        });
        removed.push(wanted);
    }

    let text = doc.to_string();
    revalidate(stores, machine, env, &[(&store.name, text.clone())], None)?;
    Ok(Outcome {
        store: Some(store.name.clone()),
        changes,
        writes: bindings_write(store, &text).into_iter().collect(),
        ..Outcome::default()
    })
}

/// Each `--add-root` as (absolute path, as written): an existing directory, `~/…` under
/// the home directory, duplicates dropped.
fn check_roots(raw: &[PathBuf], env: &Env) -> Result<Vec<(PathBuf, String)>> {
    let mut roots: Vec<(PathBuf, String)> = Vec::with_capacity(raw.len());
    for r in raw {
        let path = absolute(r, env)?;
        if !path.is_dir() {
            return Err(Error::usage(
                format!("{} is not a directory", path.display()),
                "pass an existing directory with --add-root; `kipi scan` walks only \
                 directories",
            ));
        }
        if !roots.iter().any(|(p, _)| *p == path) {
            let written = tilde(&path, env);
            roots.push((path, written));
        }
    }
    Ok(roots)
}

/// Each `--always` as written: a file of the store, relative to its root (an absolute path
/// inside the store is made relative).
fn check_always(raw: &[String], store: &Store, scope: &str) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::with_capacity(raw.len());
    for r in raw {
        let rel = store_relative(r, store);
        let inside = rel
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)));
        if !inside || rel.as_os_str().is_empty() || !store.root.join(&rel).is_file() {
            return Err(Error::usage(
                format!("--always `{r}` is not a file of store `{}`", store.name),
                format!(
                    "pass a file relative to {}, e.g. `conventions/{scope}.md`; \
                     `kipi edit conventions:{scope}` creates one",
                    store.root.display()
                ),
            ));
        }
        let written = rel.display().to_string();
        if !out.contains(&written) {
            out.push(written);
        }
    }
    Ok(out)
}

/// `raw` relative to the store root: an absolute path inside the store loses the root,
/// and `.` components go, so `./conventions/a.md` and `conventions/a.md` are one file.
fn store_relative(raw: &str, store: &Store) -> PathBuf {
    Path::new(raw)
        .strip_prefix(&store.root)
        .unwrap_or(Path::new(raw))
        .components()
        .filter(|c| !matches!(c, std::path::Component::CurDir))
        .collect()
}

/// `--extends <parent>`: a scope of the same store, other than the scope itself.
fn check_parent(stores: &StoreSet, store: &Store, name: &str, parent: &str) -> Result<()> {
    if parent == name {
        return Err(Error::usage(
            format!("scope `{name}` cannot extend itself"),
            "pass another scope of the same store, or --no-extends",
        ));
    }
    let (owner, _) = find_scope(stores, parent)?;
    if owner.name != store.name {
        return Err(Error::usage(
            format!(
                "scope `{parent}` is in store `{}`, not `{}`",
                owner.name, store.name
            ),
            "`extends` resolves only within a store: extend a scope of the same store",
        ));
    }
    Ok(())
}

/// `registry = true`, or no `registry` (it defaults to off). A `registry` line carrying a
/// comment is set to `false` instead of removed, so the comment stays.
fn registry(table: &mut dyn TableLike, scope: &Scope, on: bool) -> Change {
    let value = if on { "true" } else { "false" };
    if scope.registry == on {
        return Change::new(Op::Keep, "registry", value).note("already so");
    }
    let commented = table
        .get("registry")
        .and_then(Item::as_value)
        .is_some_and(|v| {
            [v.decor().prefix(), v.decor().suffix()]
                .into_iter()
                .flatten()
                .filter_map(|s| s.as_str())
                .any(|s| s.contains('#'))
        })
        || table
            .key("registry")
            .and_then(|k| k.leaf_decor().prefix())
            .and_then(|p| p.as_str())
            .is_some_and(|p| p.contains('#'));
    if on || commented {
        set(table, "registry", Value::from(on));
    } else {
        table.remove("registry");
    }
    let change = if on { Op::Add } else { Op::Remove };
    Change::new(change, "registry", "true")
}

/// Set or drop `extends`.
fn extends(table: &mut dyn TableLike, scope: &Scope, parent: Option<&str>) -> Change {
    match (parent, scope.extends.as_deref()) {
        (Some(p), Some(have)) if p == have => {
            Change::new(Op::Keep, "extends", p).note("already so")
        }
        (Some(p), _) => {
            set(table, "extends", Value::from(p));
            Change::new(Op::Add, "extends", p)
        }
        (None, Some(have)) => {
            table.remove("extends");
            Change::new(Op::Remove, "extends", have)
        }
        (None, None) => Change::new(Op::Keep, "extends", "(none)").note("not there"),
    }
}

/// Add (`adding`) or remove `value` in the string array `key`.
fn array_edit(table: &mut dyn TableLike, key: &str, value: &str, adding: bool) -> Change {
    let same = |v: &str| v == value;
    match (adding, contains(table, key, same)) {
        (true, false) => {
            push(table, key, value);
            Change::new(Op::Add, key, value)
        }
        (false, true) => {
            remove(table, key, same);
            Change::new(Op::Remove, key, value)
        }
        (true, true) => Change::new(Op::Keep, key, value).note("already there"),
        (false, false) => Change::new(Op::Keep, key, value).note("not there"),
    }
}

/// Set `key` to `value`, keeping the old value's comments and spacing.
fn set(table: &mut dyn TableLike, key: &str, mut value: Value) {
    if let Some(old) = table.get(key).and_then(Item::as_value) {
        *value.decor_mut() = old.decor().clone();
    }
    table.insert(key, Item::Value(value));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add(text: &str, opts: &ScopeOptions, roots: &[&str]) -> String {
        let mut doc: DocumentMut = text.parse().unwrap();
        let roots: Vec<String> = roots.iter().map(|r| (*r).to_owned()).collect();
        insert(&mut doc, opts, &roots, &[], opts.registry.unwrap_or(false));
        doc.to_string()
    }

    #[test]
    fn a_scope_goes_after_the_last_scope() {
        let text = "[store]\nname = \"s\"\n\n[scopes.a]  # first\nroots = [\"~/a\"]\n\n\
                    # the repos\n[repos.x]\nremote = \"h/o/x\"\n";
        let opts = ScopeOptions {
            name: "b",
            extends: Some("a"),
            registry: Some(true),
            ..ScopeOptions::default()
        };
        assert_eq!(
            add(text, &opts, &["~/a/b"]),
            "[store]\nname = \"s\"\n\n[scopes.a]  # first\nroots = [\"~/a\"]\n\n\
             [scopes.b]\nextends = \"a\"\nroots = [\"~/a/b\"]\nregistry = true\n\n\
             # the repos\n[repos.x]\nremote = \"h/o/x\"\n"
        );
    }

    #[test]
    fn the_first_scope_goes_before_the_repos() {
        let text = "[store]\nname = \"s\"\n\n[repos.x]\nremote = \"h/o/x\"\n\n# end\n";
        let skills = ["review".to_owned()];
        let opts = ScopeOptions {
            name: "p",
            skills: &skills,
            ..ScopeOptions::default()
        };
        assert_eq!(
            add(text, &opts, &["~/code", "/srv/code"]),
            "[store]\nname = \"s\"\n\n[scopes.p]\nroots = [\"~/code\", \"/srv/code\"]\n\
             skills = [\"review\"]\n\n[repos.x]\nremote = \"h/o/x\"\n\n# end\n"
        );
        let text = "[store]\nname = \"s\"\n\n# [scopes.<scope>]\n";
        assert_eq!(
            add(text, &opts, &["~/code"]),
            "[store]\nname = \"s\"\n\n[scopes.p]\nroots = [\"~/code\"]\n\
             skills = [\"review\"]\n\n# [scopes.<scope>]\n"
        );
    }
}
