//! `kipi add scope`: declare `[scopes.<name>]` in a store, so onboarding needs no
//! hand-edited TOML. `kipi init --scope` shares [`insert`].

use std::path::PathBuf;

use toml_edit::{DocumentMut, Item, Table, Value};

use super::{
    Change, Op, Outcome, absolute, bindings_write, find_scope, find_store, revalidate, tilde,
};
use crate::config::Env;
use crate::error::{Error, Result};
use crate::model::{Machine, Store, StoreSet, check_name};
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
    /// `--registry`.
    pub registry: bool,
    /// `--skill`: skill refs every repo in the scope gets.
    pub skills: &'a [String],
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
    let name = opts.name;
    if let Some((store, _)) = stores.scope(name) {
        return Err(Error::usage(
            format!("scope `{name}` already exists in store `{}`", store.name),
            format!(
                "pick another name, or edit [scopes.{name}] in {}",
                store.bindings.path.display()
            ),
        ));
    }
    let store = target(stores, opts)?;
    let mut doc = store.bindings.doc.clone();
    let changes = declare(&mut doc, &store.name, opts, &roots);
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

/// [`insert`], reported as a new table of store `store`.
pub(super) fn declare(
    doc: &mut DocumentMut,
    store: &str,
    opts: &ScopeOptions,
    roots: &[String],
) -> Vec<Change> {
    let header = Change::new(
        Op::Add,
        format!("store {store}"),
        format!("[scopes.{}]", opts.name),
    );
    std::iter::once(header)
        .chain(insert(doc, opts, roots))
        .collect()
}

/// Add `[scopes.<name>]` (`extends`, `roots`, `skills`, `registry`, in that order) to
/// `doc`, after its last scope, else before its first repo entry. Returns what it added.
fn insert(doc: &mut DocumentMut, opts: &ScopeOptions, roots: &[String]) -> Vec<Change> {
    let mut changes = Vec::new();
    let mut table = Table::new();
    table.decor_mut().set_prefix("\n");
    if let Some(parent) = opts.extends {
        table.insert("extends", Item::Value(Value::from(parent)));
        changes.push(Change::new(Op::Add, "extends", parent));
    }
    table.insert("roots", Item::Value(string_array(roots)));
    changes.extend(roots.iter().map(|r| Change::new(Op::Add, "roots", r)));
    if !opts.skills.is_empty() {
        table.insert("skills", Item::Value(string_array(opts.skills)));
        changes.extend(
            opts.skills
                .iter()
                .map(|s| Change::new(Op::Add, "skills", s)),
        );
    }
    if opts.registry {
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
        table.set_position(first_repo.map(|p| p - 1));
        let mut scopes = Table::new();
        scopes.set_implicit(true);
        doc.insert("scopes", Item::Table(scopes));
    }
    if let Some(scopes) = doc.get_mut("scopes").and_then(Item::as_table_mut) {
        scopes.insert(opts.name, Item::Table(table));
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add(text: &str, opts: &ScopeOptions, roots: &[&str]) -> String {
        let mut doc: DocumentMut = text.parse().unwrap();
        let roots: Vec<String> = roots.iter().map(|r| (*r).to_owned()).collect();
        insert(&mut doc, opts, &roots);
        doc.to_string()
    }

    #[test]
    fn a_scope_goes_after_the_last_scope() {
        let text = "[store]\nname = \"s\"\n\n[scopes.a]  # first\nroots = [\"~/a\"]\n\n\
                    # the repos\n[repos.x]\nremote = \"h/o/x\"\n";
        let opts = ScopeOptions {
            name: "b",
            extends: Some("a"),
            registry: true,
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
