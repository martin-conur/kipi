//! `kipi stores`: list the mounted stores (roadmap §7). `--unmount` is in `authoring`.

use std::path::PathBuf;

use kipi_core::config::{self, Env};
use kipi_core::git;
use owo_colors::Style;

use crate::error::Failure;
use crate::output::color::styles;
use crate::output::table::Table;
use crate::output::{Output, json};

/// One mounted store, as listed.
pub struct Row {
    /// `[store] name`.
    pub name: String,
    /// Its directory.
    pub path: PathBuf,
    /// Its git state; `None` when it is not in a git work tree.
    pub git: Option<git::DirStatus>,
    /// `share_skills`.
    pub share_skills: bool,
    /// Its scopes, in file order.
    pub scopes: Vec<String>,
    /// How many repo entries it holds.
    pub repos: usize,
}

/// `clean` or `dirty`, then how the branch stands: `3 ahead`, `2 behind`, `no upstream`
/// (as of the last fetch; `kipi stores` fetches nothing).
pub fn git_state(g: git::DirStatus) -> String {
    let mut parts = vec![if g.clean { "clean" } else { "dirty" }.to_owned()];
    match g.upstream {
        None => parts.push("no upstream".to_owned()),
        Some(t) => {
            if t.ahead > 0 {
                parts.push(format!("{} ahead", t.ahead));
            }
            if t.behind > 0 {
                parts.push(format!("{} behind", t.behind));
            }
        }
    }
    parts.join(", ")
}

/// List every mounted store.
pub fn run(env: &Env, store_flags: &[PathBuf], out: &Output) -> Result<(), Failure> {
    let cfg = config::load_config(env)?;
    let stores = config::mount(store_flags, env, &cfg)?;
    let rows: Vec<Row> = stores
        .stores()
        .iter()
        .map(|s| Row {
            name: s.name.clone(),
            path: s.root.clone(),
            git: git::status(&s.root),
            share_skills: s.share_skills,
            scopes: s.scopes.iter().map(|sc| sc.name.clone()).collect(),
            repos: s.repos.len(),
        })
        .collect();
    if out.json {
        let text = json::stores(&rows).map_err(|e| Failure::json("the stores", e))?;
        return Ok(out.print(&text)?);
    }
    let p = &out.paint;
    let mut table = Table::new(0);
    table.row([
        ("NAME".to_owned(), styles::none()),
        ("PATH".to_owned(), styles::none()),
        ("GIT".to_owned(), styles::none()),
        ("SCOPES".to_owned(), styles::none()),
        ("REPOS".to_owned(), styles::none()),
        (String::new(), styles::none()),
    ]);
    for r in &rows {
        let git = r.git.map_or_else(|| "not a git repo".to_owned(), git_state);
        table.row([
            (r.name.clone(), styles::name()),
            (out.path(&r.path), Style::new()),
            (git, Style::new()),
            (r.scopes.join(", "), Style::new()),
            (r.repos.to_string(), Style::new()),
            (
                if r.share_skills { "shares skills" } else { "" }.to_owned(),
                styles::source(),
            ),
        ]);
    }
    Ok(out.print(&table.render(p))?)
}
