//! `kipi edit <repo|skill|scope:<name>|notes:<repo>|conventions:<scope>>`: open the right
//! file in `$EDITOR` (else config.toml `editor`), at the table's line for a repo or scope; create notes or
//! conventions that do not exist yet; after the editor exits, mark every drafted value
//! the user changed as reviewed and reload the stores to report what no longer
//! validates (roadmap §7, §9).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use kipi_core::apply::write_atomic;
use kipi_core::config::Env;
use kipi_core::edit::{editor_command, review_changed_drafts};
use kipi_core::error::{Error, closest};
use kipi_core::model::{Scope, SkillRef, Store, StoreSet};
use kipi_core::templates::NOTES_MD;

use super::Loaded;
use crate::error::Failure;
use crate::output::Output;

/// The forms `edit` accepts, for hints.
const FORMS: &str = "pass a repo entry, a skill, `scope:<name>`, `notes:<repo>` or \
                     `conventions:<scope>` \
                     (`repo:<name>` / `skill:<name>` when a name is both)";

/// The file to open.
#[derive(Debug)]
struct Target {
    path: PathBuf,
    line: Option<usize>,
    /// Contents to create the file with when it is missing.
    template: Option<String>,
    /// A conventions file no `always` lists (so sync does not render it): its scope and
    /// its path in the store.
    unlisted: Option<(String, String)>,
}

/// Edit `target`, then validate.
pub fn run(
    env: &Env,
    store_flags: &[PathBuf],
    loaded: &Loaded,
    target: &str,
    out: &Output,
) -> Result<(), Failure> {
    let target = resolve(&loaded.stores, target)?;
    let editor = std::env::var("EDITOR")
        .ok()
        .filter(|e| !e.trim().is_empty())
        .or_else(|| loaded.config.editor.clone())
        .ok_or_else(|| {
            Failure::user(
                "no editor configured",
                "set $EDITOR, or `editor = \"hx\"` in ~/.config/kipi/config.toml",
            )
        })?;
    let mut created = false;
    if let Some(template) = &target.template
        && !target.path.exists()
    {
        if let Some(dir) = target.path.parent() {
            fs::create_dir_all(dir)
                .map_err(|e| Error::io(dir, e, "check the store is writable"))?;
        }
        write_atomic(&target.path, template)?;
        created = true;
    }
    let before = read(&target.path)?;
    open(&editor, &target)?;
    let after = read(&target.path)?;
    let mut reviewed = Vec::new();
    if let Some((text, keys)) = review_changed_drafts(&before, &after) {
        if text != after {
            write_atomic(&target.path, &text)?;
        }
        reviewed = keys;
    }
    // Validate by loading and resolving again, exactly as every other command will.
    Loaded::load(env, store_flags, out)?;
    let mut text = String::new();
    if created {
        text.push_str(&format!("created {}\n", target.path.display()));
    }
    for key in &reviewed {
        text.push_str(&format!("reviewed {key} (draft marker removed)\n"));
    }
    text.push_str(&format!("{}: valid\n", target.path.display()));
    out.print(&text)?;
    if let Some((scope, rel)) = target.unlisted {
        eprintln!(
            "{} {rel} is not in `always` of [scopes.{scope}], so sync does not render it\n  {} add `always = [\"{rel}\"]` to [scopes.{scope}] in the store's bindings.toml",
            out.paint_err.hint("note:"),
            out.paint_err.hint("hint:"),
        );
    }
    Ok(())
}

fn read(path: &Path) -> Result<String, Failure> {
    fs::read_to_string(path)
        .map_err(|e| Error::io(path, e, "check the file exists and is readable").into())
}

/// Run the editor and wait for it.
fn open(editor: &str, target: &Target) -> Result<(), Failure> {
    let argv = editor_command(editor, &target.path, target.line).ok_or_else(|| {
        Failure::user(
            "the editor command is blank",
            "set $EDITOR, or `editor` in config.toml",
        )
    })?;
    let status = Command::new(&argv[0]).args(&argv[1..]).status();
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(Failure::user(
            format!("the editor `{editor}` exited with {s}"),
            "nothing was validated; fix the editor problem and run `kipi edit` again",
        )),
        Err(e) => Err(Failure::user(
            format!("cannot run the editor `{editor}`: {e}"),
            "check that $EDITOR (or config.toml `editor`) names an installed command",
        )),
    }
}

/// Map `raw` to a file.
fn resolve(stores: &StoreSet, raw: &str) -> Result<Target, Failure> {
    let file = |path: PathBuf, line| Target {
        path,
        line,
        template: None,
        unlisted: None,
    };
    if let Some(name) = raw.strip_prefix("notes:") {
        let (store, _) = repo(stores, name)?;
        return Ok(Target {
            template: Some(NOTES_MD.replace("{{ repo }}", name)),
            ..file(store.join("repos").join(name).join("notes.md"), None)
        });
    }
    if let Some(name) = raw.strip_prefix("scope:") {
        let (store, scope) = scope(stores, name)?;
        return Ok(file(store.bindings.path.clone(), scope.at.line));
    }
    if let Some(name) = raw.strip_prefix("conventions:") {
        let (store, scope) = scope(stores, name)?;
        let listed = scope.always.iter().find(|p| p.starts_with("conventions"));
        let rel = listed
            .cloned()
            .unwrap_or_else(|| Path::new("conventions").join(format!("{name}.md")));
        return Ok(Target {
            template: Some(String::new()),
            unlisted: listed
                .is_none()
                .then(|| (name.to_owned(), rel.display().to_string())),
            ..file(store.root.join(rel), None)
        });
    }
    if let Some(name) = raw.strip_prefix("repo:") {
        let (path, line) = repo(stores, name)?.1;
        return Ok(file(path, line));
    }
    if let Some(name) = raw.strip_prefix("skill:") {
        return skill(stores, name)?.ok_or_else(|| no_skill(stores, name));
    }
    let as_repo = repo(stores, raw).ok();
    let as_skill = skill(stores, raw)?;
    match (as_repo, as_skill) {
        (Some(_), Some(_)) => Err(Failure::user(
            format!("`{raw}` is both a repo entry and a skill"),
            format!("pass `repo:{raw}` or `skill:{raw}`"),
        )),
        (Some((_, (path, line))), None) => Ok(file(path, line)),
        (None, Some(target)) => Ok(target),
        (None, None) => {
            let names = stores.stores().iter().flat_map(|s| {
                (s.repos.iter().map(|r| r.name.as_str()))
                    .chain(s.skills.iter().map(|k| k.name.as_str()))
            });
            Err(unknown("repo entry or skill", raw, closest(raw, names)))
        }
    }
}

/// The scope `name` and the store declaring it.
fn scope<'a>(stores: &'a StoreSet, name: &str) -> Result<(&'a Store, &'a Scope), Failure> {
    stores.scope(name).ok_or_else(|| {
        let names = stores
            .stores()
            .iter()
            .flat_map(|s| s.scopes.iter().map(|s| s.name.as_str()));
        unknown("scope", name, closest(name, names))
    })
}

/// The repo entry `name`: its store's root, and its `[repos.<name>]` line.
fn repo(stores: &StoreSet, name: &str) -> Result<(PathBuf, (PathBuf, Option<usize>)), Failure> {
    match stores.repo(name) {
        Some((store, entry)) => Ok((
            store.root.clone(),
            (store.bindings.path.clone(), entry.at.line),
        )),
        None => {
            let names = stores.repos().map(|(_, r)| r.name.as_str());
            Err(unknown("repo entry", name, closest(name, names)))
        }
    }
}

/// The skill `name` (`skill` or `store/skill`): its `SKILL.md`. A bare name found in
/// several stores is ambiguous.
fn skill(stores: &StoreSet, name: &str) -> Result<Option<Target>, Failure> {
    let Ok(reference) = name.parse::<SkillRef>() else {
        return Ok(None);
    };
    let found: Vec<_> = stores
        .stores()
        .iter()
        .filter(|s| reference.store.as_deref().is_none_or(|n| n == s.name))
        .filter_map(|s| s.skill(&reference.name).map(|k| (s, k)))
        .collect();
    match found.as_slice() {
        [] => Ok(None),
        [(_, skill)] => Ok(Some(Target {
            path: skill.at.file.clone(),
            line: None,
            template: None,
            unlisted: None,
        })),
        several => {
            let names: Vec<String> = several
                .iter()
                .map(|(s, k)| format!("`{}/{}`", s.name, k.name))
                .collect();
            Err(Failure::user(
                format!("skill `{name}` is in several stores"),
                format!("pass one of {}", names.join(", ")),
            ))
        }
    }
}

fn no_skill(stores: &StoreSet, name: &str) -> Failure {
    let names = stores
        .stores()
        .iter()
        .flat_map(|s| s.skills.iter().map(|k| k.name.as_str()));
    unknown("skill", name, closest(name, names))
}

fn unknown(what: &str, name: &str, near: Option<&str>) -> Failure {
    let hint = match near {
        Some(near) => format!("did you mean `{near}`? ({FORMS})"),
        None => FORMS.to_owned(),
    };
    Failure::user(format!("no {what} `{name}` in the mounted stores"), hint)
}
