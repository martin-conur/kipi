//! `kipi where <alias> [--notes]`: resolve an alias to a repo entry, its summary, its
//! checkout and its notes (roadmap §3 channel 4, §7). Written for an agent reading it:
//! one fact per line, absolute paths, no decoration. Inside a bound repo only the
//! repo's scope chain is searched; `--any-scope` searches every mounted store.

use std::fs;

use kipi_core::error::{Error, closest};
use kipi_core::search::{self, Lookup, RepoInfo, View};

use super::{Loaded, cwd_repo};
use crate::error::Failure;
use crate::output::{Output, json};

/// Look `alias` up and print what it names.
pub fn run(
    loaded: &Loaded,
    alias: &str,
    notes: bool,
    any_scope: bool,
    out: &Output,
) -> Result<(), Failure> {
    let here = if any_scope { None } else { cwd_repo(loaded) };
    let view = match here {
        Some(repo) => View::repo(&loaded.stores, &loaded.plan, repo),
        None => View::everything(&loaded.stores),
    };
    let info = match search::lookup(&loaded.stores, &loaded.plan, &view, alias) {
        Lookup::Found(info) => info,
        Lookup::Ambiguous(infos) => {
            let names: Vec<&str> = infos.iter().map(|i| i.name.as_str()).collect();
            return Err(Failure::user(
                format!("`{alias}` is ambiguous: {}", names.join(", ")),
                format!(
                    "pass an entry name (`kipi where {}`); `kipi doctor` reports the alias collision",
                    names[0]
                ),
            ));
        }
        Lookup::NotFound => return Err(not_found(loaded, &view, alias, here.is_some())),
    };
    let notes_text = if notes {
        Some(read_notes(&info)?)
    } else {
        None
    };
    if out.json {
        let text = json::lookup(&info, notes_text.as_deref())
            .map_err(|e| Failure::json("the repo entry", e))?;
        return Ok(out.print(&text)?);
    }
    let mut text = describe(&info);
    if let Some(notes) = notes_text {
        text.push('\n');
        text.push_str(&notes);
        if !notes.ends_with('\n') {
            text.push('\n');
        }
    }
    out.print(&text)?;
    Ok(())
}

/// One fact per line: name and summary, aliases, scope, checkout, notes.
fn describe(info: &RepoInfo) -> String {
    let draft = |d: bool| if d { " (draft)" } else { "" };
    let summary = info.summary.as_deref().unwrap_or("(no summary)");
    let mut text = format!("{} — {summary}{}\n", info.name, draft(info.summary_draft));
    if !info.aliases.is_empty() {
        text.push_str(&format!(
            "aliases: {}{}\n",
            info.aliases.join(", "),
            draft(info.aliases_draft)
        ));
    }
    text.push_str(&format!(
        "scope: {} (store {})\n",
        info.scope.as_deref().unwrap_or("unscoped"),
        info.store
    ));
    let local = info.local.as_ref().map_or_else(
        || "(not cloned here)".to_owned(),
        |p| p.display().to_string(),
    );
    text.push_str(&format!("local: {local}\n"));
    let notes = info
        .notes
        .as_ref()
        .map_or_else(|| "none".to_owned(), |p| p.display().to_string());
    text.push_str(&format!("notes: {notes}\n"));
    text
}

fn read_notes(info: &RepoInfo) -> Result<String, Failure> {
    let Some(path) = &info.notes else {
        return Err(Failure::user(
            format!("repo `{}` has no notes", info.name),
            format!(
                "drop `--notes`, or write some with `kipi edit notes:{}`",
                info.name
            ),
        ));
    };
    fs::read_to_string(path).map_err(|e| Error::io(path, e, "check the file's permissions").into())
}

/// No entry in view: suggest a near name or alias from the view only, so nothing
/// outside the scope chain is ever named.
fn not_found(loaded: &Loaded, view: &View, alias: &str, scoped: bool) -> Failure {
    let mut keys: Vec<&str> = Vec::new();
    for name in &view.repos {
        keys.push(name);
        if let Some((_, entry)) = loaded.stores.repo(name) {
            keys.extend(entry.aliases.value.iter().map(String::as_str));
        }
    }
    let hint = match (closest(alias, keys), scoped) {
        (Some(near), _) => format!("did you mean `{near}`?"),
        (None, true) => format!(
            "only {} is searched from here; `kipi where --any-scope {alias}` searches every \
             mounted store",
            view.label
        ),
        (None, false) => "`kipi resolve` lists every repo entry of the mounted stores".to_owned(),
    };
    let within = if scoped {
        format!(" in {}", view.label)
    } else {
        String::new()
    };
    Failure::user(format!("no repo named or aliased `{alias}`{within}"), hint)
}
