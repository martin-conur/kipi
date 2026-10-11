//! `kipi init` (create a store and mount it, or mount an existing one) and
//! `kipi stores --unmount` (roadmap §7, §11 second-machine flow).

use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, Item, Value};

use super::scope::{self, ScopeOptions};
use super::toml::{push, table_mut};
use super::{Change, Op, Outcome, absolute, file_write, tilde};
use crate::config::{self, BINDINGS_FILE, DEFAULT_STORE, Env};
use crate::error::{Error, Result};
use crate::model::{Config, Machine, StoreSet, check_name};
use crate::resolve::resolve;
use crate::templates::BINDINGS_TOML;

/// The directories of a store's layout (roadmap §4), created when missing.
const STORE_DIRS: [&str; 4] = ["skills", "repos", "conventions", "generated"];
/// What a new store's `.gitignore` must hold: registries are per-machine output.
const GITIGNORE_LINE: &str = "generated/";

/// The arguments of `init`.
#[derive(Debug, Clone, Copy, Default)]
pub struct InitOptions<'a> {
    /// `--store <path>`; default `~/.agents/store`.
    pub path: Option<&'a Path>,
    /// `--name <n>`; default from the directory name ([`default_store_name`]).
    pub name: Option<&'a str>,
    /// `--git`.
    pub git: bool,
    /// `--scope`: declare this scope in the store, unless it already has it.
    pub scope: Option<&'a str>,
    /// `--root`: the roots of `--scope`.
    pub roots: &'a [PathBuf],
}

/// The name a new store at `root` gets without `--name`: `personal` for the default store,
/// else its directory name without a leading `.` and a `kipi-` prefix
/// (`~/work/.kipi-acme` → `acme`).
pub fn default_store_name(root: &Path, env: &Env) -> String {
    if root == env.home.join(DEFAULT_STORE) {
        return crate::resolve::FALLBACK_SCOPE.to_owned();
    }
    let dir = root.file_name().unwrap_or_default().to_string_lossy();
    let dir = dir.trim_start_matches('.');
    dir.strip_prefix("kipi-").unwrap_or(dir).to_owned()
}

/// Create the store at `opts.path` (dirs, `bindings.toml` skeleton, `.gitignore`, with
/// `--git` a repository) unless it exists, and add it to `config.toml` `stores` unless it
/// is listed. With `--scope`, declare that scope in the store unless it has it. Never
/// overwrites a file other than by that edit. The new mount set is loaded and resolved
/// first.
pub fn init(env: &Env, cfg: &Config, machine: &Machine, opts: &InitOptions) -> Result<Outcome> {
    let root = match opts.path {
        Some(p) => absolute(p, env)?,
        None => env.home.join(DEFAULT_STORE),
    };
    let scope_opts = opts.scope.map(|name| ScopeOptions {
        name,
        roots: opts.roots,
        ..ScopeOptions::default()
    });
    let roots = scope_opts
        .as_ref()
        .map(|s| scope::check(s, env))
        .transpose()?;
    let mut out = Outcome::default();
    let bindings = root.join(BINDINGS_FILE);
    let new_text = if bindings.is_file() {
        let store = config::load_store(&root, env)?;
        if let Some(name) = opts.name.filter(|n| *n != store.name) {
            return Err(Error::usage(
                format!(
                    "the store at {} is named `{}`, not `{name}`",
                    root.display(),
                    store.name
                ),
                format!(
                    "drop --name to mount it as `{}`; a store's name is `[store] name` in its \
                     bindings.toml",
                    store.name
                ),
            ));
        }
        out.changes.push(
            Change::new(
                Op::Keep,
                "store",
                format!("{} at {}", store.name, root.display()),
            )
            .note("exists; mounted as is"),
        );
        let mut edited = None;
        if let (Some(s), Some(roots)) = (&scope_opts, &roots) {
            if store.scope(s.name).is_some() {
                out.changes.push(
                    Change::new(Op::Keep, "scope", format!("[scopes.{}]", s.name))
                        .note("exists; left as is"),
                );
            } else {
                let mut doc = store.bindings.doc.clone();
                out.changes
                    .extend(scope::declare(&mut doc, &store.name, s, roots));
                let text = doc.to_string();
                out.writes
                    .extend(file_write(bindings.clone(), text.clone()));
                edited = Some(text);
            }
        }
        out.store = Some(store.name);
        edited
    } else {
        if root.is_file() {
            return Err(Error::usage(
                format!("{} is a file, not a store directory", root.display()),
                "pass a directory for --store",
            ));
        }
        let name = opts
            .name
            .map_or_else(|| default_store_name(&root, env), str::to_owned);
        check_name(&name).map_err(|why| {
            Error::usage(
                format!("invalid store name `{name}`: the name {why}"),
                "pass a name with --name: letters, digits, `-`, `_` and `.`",
            )
        })?;
        let mut text = skeleton(&name);
        let mut scope_changes = Vec::new();
        if let (Some(s), Some(roots)) = (&scope_opts, &roots) {
            let mut doc: DocumentMut = text.parse().expect("the skeleton is valid TOML");
            scope_changes = scope::declare(&mut doc, &name, s, roots);
            text = doc.to_string();
        }
        // A store name clash with a mounted store is caught when mounting below.
        config::load_store_text(&root, text.clone(), env).map_err(Error::rejected)?;
        out.dirs = std::iter::once(root.clone())
            .chain(STORE_DIRS.iter().map(|d| root.join(d)))
            .filter(|d| !d.is_dir())
            .collect();
        out.writes.extend(file_write(bindings, text.clone()));
        let gitignore = root.join(".gitignore");
        let ignored = std::fs::read_to_string(&gitignore).unwrap_or_default();
        if !ignored.lines().any(|l| l.trim() == GITIGNORE_LINE) {
            let mut after = ignored;
            if !after.is_empty() && !after.ends_with('\n') {
                after.push('\n');
            }
            after.push_str(GITIGNORE_LINE);
            after.push('\n');
            out.writes.extend(file_write(gitignore, after));
        }
        out.changes.push(Change::new(
            Op::Add,
            "store",
            format!("{name} at {}", root.display()),
        ));
        out.changes.extend(scope_changes);
        out.store = Some(name);
        Some(text)
    };

    if opts.git {
        if root.join(".git").exists() {
            out.changes
                .push(Change::new(Op::Keep, "git", "git init").note("already a git repository"));
        } else {
            out.git_init = Some(root.clone());
            out.changes.push(Change::new(Op::Add, "git", "git init"));
        }
    }

    mount(env, cfg, machine, &root, new_text, &mut out)?;
    Ok(out)
}

/// The `bindings.toml` skeleton for store `name`, its trailing comments kept in a column.
fn skeleton(name: &str) -> String {
    let placeholder = "\"{{ name }}\"";
    BINDINGS_TOML
        .lines()
        .map(|line| match line.split_once(placeholder) {
            Some((key, rest)) => {
                let value = format!("{key}\"{name}\"");
                let comment = rest.trim_start();
                let width = key.len() + placeholder.len() + (rest.len() - comment.len());
                format!(
                    "{value:<width$}{comment}\n",
                    width = width.max(value.len() + 1)
                )
            }
            None => format!("{line}\n"),
        })
        .collect()
}

/// Add `root` to `config.toml` `stores` (after the implicit default store, when the file
/// lists none and that store exists), checking the new set loads and resolves. A store
/// already listed is checked too when `new_text` edits it.
fn mount(
    env: &Env,
    cfg: &Config,
    machine: &Machine,
    root: &Path,
    new_text: Option<String>,
    out: &mut Outcome,
) -> Result<()> {
    let mounted = cfg.stores.iter().any(|p| same_dir(p, root));
    let mut paths: Vec<PathBuf> = cfg.stores.clone();
    let default = env.home.join(DEFAULT_STORE);
    if paths.is_empty() && default != root && default.join(BINDINGS_FILE).is_file() {
        // Listing only the new store would silently unmount the implicit default one.
        paths.push(default);
    }
    let added: Vec<PathBuf> = if mounted {
        Vec::new()
    } else {
        paths
            .iter()
            .filter(|p| !cfg.stores.contains(p))
            .cloned()
            .chain(std::iter::once(root.to_path_buf()))
            .collect()
    };
    if !mounted {
        paths.push(root.to_path_buf());
    }

    let check = || {
        let stores = paths
            .iter()
            .map(|p| match (&new_text, same_dir(p, root)) {
                (Some(text), true) => config::load_store_text(p, text.clone(), env),
                _ => config::load_store(p, env),
            })
            .collect::<Result<Vec<_>>>()?;
        resolve(&StoreSet::new(stores)?, machine).map(drop)
    };
    if new_text.is_some() || !mounted {
        check().map_err(Error::rejected)?;
    }
    if mounted {
        out.changes.push(
            Change::new(Op::Keep, "config.toml stores", tilde(root, env)).note("already mounted"),
        );
        return Ok(());
    }

    let mut doc = cfg
        .source
        .as_ref()
        .map(|s| s.doc.clone())
        .unwrap_or_default();
    let top = table_mut(&mut doc, &[]).expect("the root table exists");
    for path in &added {
        let entry = tilde(path, env);
        push(top, "stores", &entry);
        out.changes
            .push(Change::new(Op::Add, "config.toml stores", entry));
    }
    out.writes
        .extend(file_write(env.config_file(), doc.to_string()));
    Ok(())
}

/// Remove the store named (or at the path) `which` from `config.toml` `stores`.
pub fn unmount(env: &Env, cfg: &Config, which: &str) -> Result<Outcome> {
    let config_file = env.config_file();
    if cfg.stores.is_empty() {
        return Err(Error::usage(
            format!("{} mounts no stores", config_file.display()),
            format!(
                "with no `stores` listed, the default store ~/{DEFAULT_STORE} is mounted \
                 implicitly; mount another with `kipi init --store <path>`"
            ),
        ));
    }
    let as_path = config::expand_tilde(which, &env.home);
    let mut names = Vec::new();
    let mut matched: Option<(PathBuf, Option<String>)> = None;
    for path in &cfg.stores {
        let name = config::load_store(path, env).ok().map(|s| s.name);
        if matched.is_none() && (name.as_deref() == Some(which) || same_dir(path, &as_path)) {
            matched = Some((path.clone(), name.clone()));
        }
        names.extend(name);
    }
    let Some((path, name)) = matched else {
        let hint = match crate::error::closest(which, names.iter().map(String::as_str)) {
            Some(near) => format!("did you mean `{near}`?"),
            None if names.is_empty() => "pass the store's path".to_owned(),
            None => format!("mounted stores: {}", names.join(", ")),
        };
        return Err(Error::usage(
            format!("no store `{which}` in {}", config_file.display()),
            hint,
        ));
    };

    let mut doc: DocumentMut = cfg
        .source
        .as_ref()
        .map(|s| s.doc.clone())
        .unwrap_or_default();
    if let Some(array) = doc.get_mut("stores").and_then(Item::as_array_mut) {
        let keep = |v: &Value| {
            v.as_str()
                .is_none_or(|s| !same_dir(&config::expand_tilde(s, &env.home), &path))
        };
        // Keep an empty `stores = []` rather than dropping the key: the file still
        // documents that it mounts nothing explicitly.
        array.retain(keep);
        if let Some(first) = array.get_mut(0) {
            first.decor_mut().set_prefix("");
        }
    }
    let label = match &name {
        Some(n) => format!("{n} ({})", tilde(&path, env)),
        None => tilde(&path, env),
    };
    let mut change = Change::new(Op::Remove, "config.toml stores", label)
        .note("`kipi sync --all` removes what it contributed to your repos");
    if cfg.stores.len() == 1 {
        change = change.note(format!(
            "no store is listed now, so the default store ~/{DEFAULT_STORE} is mounted; \
             `kipi sync --all` removes what the old one contributed"
        ));
    }
    Ok(Outcome {
        store: name,
        changes: vec![change],
        writes: file_write(config_file, doc.to_string())
            .into_iter()
            .collect(),
        ..Outcome::default()
    })
}

/// Whether `a` and `b` name the same directory (equal, or equal once canonicalized).
fn same_dir(a: &Path, b: &Path) -> bool {
    a == b
        || matches!(
            (a.canonicalize(), b.canonicalize()),
            (Ok(x), Ok(y)) if x == y
        )
}
