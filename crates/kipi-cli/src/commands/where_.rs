//! `kipi where <alias> [--notes]`: resolve an alias to a repo entry, its summary, its
//! checkout, its attributes and its notes (roadmap §3 channel 4, §7). Written for an
//! agent reading it: one fact per line, absolute paths, no decoration. `kipi where --attr
//! key=value` is the reverse: every repo whose resolved attributes match. Inside a bound
//! repo only the repo's scope chain is searched; `--any-scope` searches every mounted
//! store.

use std::fs;

use kipi_core::error::{Error, closest};
use kipi_core::model::check_attribute_key;
use kipi_core::search::{self, AttrQuery, Lookup, RepoInfo, View};

use super::{Loaded, cwd_repo};
use crate::error::Failure;
use crate::output::{Output, json};

/// What `where` may see from the working directory: the current repo's scope chain, or
/// everything with `--any-scope` or outside a bound repo. `true` when scoped.
fn view(loaded: &Loaded, any_scope: bool) -> (View, bool) {
    let here = if any_scope { None } else { cwd_repo(loaded) };
    match here {
        Some(repo) => (View::repo(&loaded.stores, &loaded.plan, repo), true),
        None => (View::everything(&loaded.stores), false),
    }
}

/// List every repo in view whose resolved attributes match every `--attr`.
pub fn by_attributes(
    loaded: &Loaded,
    raw: &[String],
    any_scope: bool,
    out: &Output,
) -> Result<(), Failure> {
    let mut query = Vec::with_capacity(raw.len());
    for r in raw {
        let q = AttrQuery::parse(r);
        check_attribute_key(&q.key).map_err(|why| {
            Failure::user(
                format!("invalid --attr `{r}`: the key {why}"),
                "pass `key=value` or `key`, e.g. `kipi where --attr tracker.project=PROJ`",
            )
        })?;
        query.push(q);
    }
    let (view, scoped) = view(loaded, any_scope);
    out.info(&format!("searching {}", view.label));
    let found = search::by_attributes(&loaded.stores, &loaded.plan, &view, &query);
    if out.json {
        let text = json::by_attributes(&view, &query, &found)
            .map_err(|e| Failure::json("the repo entries", e))?;
        return Ok(out.print(&text)?);
    }
    let text: Vec<String> = found.iter().map(describe).collect();
    out.print(&text.join("\n"))?;
    if found.is_empty() && out.verbosity >= 0 {
        let p = &out.paint_err;
        let more = if scoped {
            "; `--any-scope` searches every mounted store"
        } else {
            ""
        };
        eprintln!(
            "{} no repo in {} has {}\n  {} values match exactly; `kipi explain <repo>` shows a \
             repo's attributes{more}",
            p.hint("note:"),
            view.label,
            raw.join(" and "),
            p.hint("hint:")
        );
    }
    Ok(())
}

/// Look `alias` up and print what it names.
pub fn run(
    loaded: &Loaded,
    alias: &str,
    notes: bool,
    any_scope: bool,
    out: &Output,
) -> Result<(), Failure> {
    let (view, scoped) = view(loaded, any_scope);
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
        Lookup::NotFound => return Err(not_found(loaded, &view, alias, scoped)),
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

/// One fact per line: name and summary, aliases, scope, checkout, notes, attributes.
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
    for (key, value) in &info.attributes {
        text.push_str(&format!("attribute: {key} = {value}\n"));
    }
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
