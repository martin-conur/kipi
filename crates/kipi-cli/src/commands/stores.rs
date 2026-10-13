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
        let git = match r.git {
            None => "local, no git".to_owned(),
            Some(g) => format!(
                "{}, {}",
                if g.clean { "clean" } else { "dirty" },
                if g.has_remote { "remote" } else { "no remote" }
            ),
        };
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
