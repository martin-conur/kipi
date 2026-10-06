//! The authoring commands: `init`, `stores --unmount`, `add skill|repo`, `bind`, `unbind`
//! (roadmap §7). Each computes a [`kipi_core::authoring::Outcome`], then writes it under
//! the lock, or with `--dry-run` prints it as a unified diff.

use std::path::PathBuf;

use kipi_core::apply::Lock;
use kipi_core::authoring::{self, Binding, InitOptions, Op, Outcome, RepoOptions, SkillOptions};
use kipi_core::config::{self, Env};
use kipi_core::model::{Config, Machine, StoreSet};

use crate::error::Failure;
use crate::output::{Output, diff, json};

/// `config.toml`, the mounted stores and `machine.toml`.
fn load(env: &Env, store_flags: &[PathBuf]) -> Result<(Config, StoreSet, Machine), Failure> {
    let cfg = config::load_config(env)?;
    let stores = config::load_stores(&config::store_paths(store_flags, env, &cfg), env)?;
    let machine = config::load_machine(env)?;
    Ok((cfg, stores, machine))
}

/// `kipi init`.
pub fn init(env: &Env, opts: &InitOptions, dry_run: bool, out: &Output) -> Result<(), Failure> {
    let cfg = config::load_config(env)?;
    let machine = config::load_machine(env)?;
    let outcome = authoring::init(env, &cfg, &machine, opts)?;
    finish("init", "init".to_owned(), &outcome, env, dry_run, out)
}

/// `kipi stores --unmount <name>`.
pub fn unmount(env: &Env, which: &str, dry_run: bool, out: &Output) -> Result<(), Failure> {
    let cfg = config::load_config(env)?;
    let outcome = authoring::unmount(env, &cfg, which)?;
    finish(
        "unmount",
        format!("unmount {which}"),
        &outcome,
        env,
        dry_run,
        out,
    )
}

/// `kipi add skill`.
pub fn add_skill(
    env: &Env,
    store_flags: &[PathBuf],
    opts: &SkillOptions,
    dry_run: bool,
    out: &Output,
) -> Result<(), Failure> {
    let (_, stores, machine) = load(env, store_flags)?;
    let outcome = authoring::add_skill(&stores, &machine, env, opts)?;
    let title = format!("add skill {}", opts.name);
    finish("add skill", title, &outcome, env, dry_run, out)
}

/// `kipi add repo`.
pub fn add_repo(
    env: &Env,
    store_flags: &[PathBuf],
    opts: &RepoOptions,
    dry_run: bool,
    out: &Output,
) -> Result<(), Failure> {
    let (_, stores, machine) = load(env, store_flags)?;
    let outcome = authoring::add_repo(&stores, &machine, env, opts)?;
    let title = format!("add repo {}", opts.name);
    finish("add repo", title, &outcome, env, dry_run, out)
}

/// `kipi bind` (`adding`) or `kipi unbind`.
pub fn bind(
    env: &Env,
    store_flags: &[PathBuf],
    binding: &Binding,
    adding: bool,
    dry_run: bool,
    out: &Output,
) -> Result<(), Failure> {
    let (_, stores, machine) = load(env, store_flags)?;
    let (command, outcome) = if adding {
        ("bind", authoring::bind(&stores, &machine, env, binding)?)
    } else {
        (
            "unbind",
            authoring::unbind(&stores, &machine, env, binding)?,
        )
    };
    let title = format!("{command} {}", binding.repo);
    finish(command, title, &outcome, env, dry_run, out)
}

/// Write `outcome` (unless `dry_run`) and report it.
fn finish(
    command: &str,
    title: String,
    outcome: &Outcome,
    env: &Env,
    dry_run: bool,
    out: &Output,
) -> Result<(), Failure> {
    if !dry_run && !outcome.is_empty() {
        let _lock = Lock::acquire(&env.config_dir)?;
        outcome.write()?;
    }
    if out.json {
        let text = json::authoring(command, outcome, dry_run)
            .map_err(|e| Failure::json("the report", e))?;
        return Ok(out.print(&text)?);
    }
    Ok(out.print(&human(&title, outcome, dry_run, out))?)
}

fn human(title: &str, outcome: &Outcome, dry_run: bool, out: &Output) -> String {
    let p = &out.paint;
    let store = outcome
        .store
        .as_ref()
        .map(|s| format!(" {}", p.dim(format!("(store {s})"))))
        .unwrap_or_default();
    let mut text = format!("{}{store}\n", p.heading(title));
    for c in &outcome.changes {
        let mark = match c.op {
            Op::Add => p.paint("+", owo_colors::Style::new().green()),
            Op::Remove => p.paint("-", owo_colors::Style::new().red()),
            Op::Keep => p.dim("="),
        };
        let note = c
            .note
            .as_ref()
            .map(|n| format!("  {}", p.dim(format!("({n})"))))
            .unwrap_or_default();
        // Paths in values are shown with `~`.
        let value = match out.home.as_deref() {
            Some(home) => c.value.replace(&home.display().to_string(), "~"),
            None => c.value.clone(),
        };
        text.push_str(&format!("  {mark} {}: {value}{note}\n", c.what));
    }
    text.push('\n');
    if outcome.is_empty() {
        text.push_str("Nothing to write.\n");
        return text;
    }
    let verb = if dry_run { "Would create" } else { "Created" };
    for dir in &outcome.dirs {
        text.push_str(&format!("{verb} {}/\n", out.path(dir)));
    }
    if let Some(skill) = &outcome.skill {
        text.push_str(&format!(
            "{} {}/ ({} {} from {})\n",
            if dry_run { "Would copy" } else { "Copied" },
            out.path(&skill.to),
            skill.files.len(),
            if skill.files.len() == 1 {
                "file"
            } else {
                "files"
            },
            out.path(&skill.from),
        ));
    }
    if dry_run && !outcome.writes.is_empty() {
        let n = outcome.writes.len();
        text.push_str(&format!(
            "Dry run: would write {n} {}:\n",
            if n == 1 { "file" } else { "files" }
        ));
        for w in &outcome.writes {
            let before = w.before.as_deref().unwrap_or_default();
            text.push_str(&diff::unified(&out.path(&w.path), before, &w.after, p));
        }
    } else {
        for w in &outcome.writes {
            text.push_str(&format!("Wrote {}\n", out.path(&w.path)));
        }
    }
    if let Some(link) = &outcome.link_back {
        text.push_str(&format!(
            "{} {} -> {}\n",
            if dry_run { "Would link" } else { "Linked" },
            out.path(&link.original),
            out.path(&link.target)
        ));
    }
    if let Some(dir) = &outcome.git_init {
        let verb = if dry_run { "Would run" } else { "Ran" };
        text.push_str(&format!("{verb} git init in {}\n", out.path(dir)));
    }
    text
}
