//! `kipi resolve [repo]`: print the plan; `--json` is the stable form (docs/json.md).

use kipi_core::plan::RepoPlan;

use super::Loaded;
use crate::error::Failure;
use crate::output::Output;
use crate::output::color::styles;
use crate::output::json;
use crate::output::table::Table;

/// Print the whole plan, or only `repo`'s part of it.
pub fn run(loaded: &Loaded, repo: Option<&str>, out: &Output) -> Result<(), Failure> {
    let only = repo.map(|name| loaded.repo(name)).transpose()?;
    if out.json {
        let text = json::plan(&loaded.stores, &loaded.plan, only).map_err(|e| {
            Failure::user(
                format!("cannot print the plan as JSON: {e}"),
                "JSON output needs every path to be valid UTF-8; rename the offending directory",
            )
        })?;
        return Ok(out.print(&text)?);
    }
    out.print(&human(loaded, only, out))?;
    Ok(())
}

/// One row per repo, then one per registry.
fn human(loaded: &Loaded, only: Option<&RepoPlan>, out: &Output) -> String {
    let plan = &loaded.plan;
    let p = &out.paint;
    let repos: Vec<&RepoPlan> = match only {
        Some(r) => vec![r],
        None => plan.repos.iter().collect(),
    };
    let cell = |s: String| (s, styles::none());
    let mut t = Table::new(0);
    t.row(
        [
            "REPO", "STORE", "SCOPE", "SKILLS", "RELATED", "GRANT", "CHECKOUT",
        ]
        .map(|h| (h.to_owned(), styles::name())),
    );
    for r in &repos {
        let list = |items: Vec<String>| {
            if items.is_empty() {
                "-".to_owned()
            } else {
                items.join(",")
            }
        };
        t.row([
            (r.name.clone(), styles::name()),
            cell(r.store.clone()),
            cell(r.scope.value.clone().unwrap_or_else(|| "(unscoped)".into())),
            cell(list(
                r.skills.iter().map(|s| s.reference.to_string()).collect(),
            )),
            cell(list(r.related.iter().map(|x| x.name.clone()).collect())),
            cell(format!(
                "{} ({} dirs)",
                r.grant.mode.value,
                r.grant.dirs.len()
            )),
            cell(match &r.path {
                Some(path) => out.path(path),
                None => "(not cloned here)".into(),
            }),
        ]);
    }
    let mut text = t.render(p);

    let registries: Vec<_> = match only {
        Some(r) => plan.registry_of(r).into_iter().collect(),
        None => plan.registries.iter().collect(),
    };
    if !registries.is_empty() {
        let mut t = Table::new(0);
        t.row(
            ["REGISTRY", "STORE", "SCOPES", "REPOS", "ALIAS COLLISIONS"]
                .map(|h| (h.to_owned(), styles::name())),
        );
        for reg in registries {
            let collisions = if reg.collisions.is_empty() {
                "-".to_owned()
            } else {
                reg.collisions
                    .iter()
                    .map(|c| format!("{} ({})", c.key, c.repos.join(", ")))
                    .collect::<Vec<_>>()
                    .join("; ")
            };
            t.row([
                (reg.scope.clone(), styles::name()),
                cell(reg.store.clone()),
                cell(reg.scopes.join(",")),
                cell(reg.entries.len().to_string()),
                cell(collisions),
            ]);
        }
        text.push('\n');
        text.push_str(&t.render(p));
    }
    text
}
