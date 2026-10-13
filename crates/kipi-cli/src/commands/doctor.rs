//! `kipi doctor [repo] [--fix]`: every check of roadmap §10 (docs/doctor.md). Exits 2
//! when any finding is an error.
//!
//! `--fix` applies the safe fixes ([`Repairs`]): the config edits and broken-link
//! removals first, then a re-sync of the repos that need one on a plan resolved after
//! those edits, under the sync lock. Doctor then runs again and reports what remains.
//! With `--dry-run` it lists the fixes and changes nothing.

use std::path::{Path, PathBuf};

use kipi_core::apply;
use kipi_core::cache::Caches;
use kipi_core::config::{self, Env};
use kipi_core::doctor::fix::{Applied, Repairs, Skipped, resync_skipped};
use kipi_core::doctor::{self, Checkouts, Diagnosis, Finding, Fix, Level};
use kipi_core::model::{Machine, Store};
use kipi_core::sync::{self, RepoState, Selection};

use super::Loaded;
use crate::error::Failure;
use crate::output::{Output, json};

/// The command's arguments.
pub struct Args<'a> {
    /// Only findings about this repo entry.
    pub repo: Option<&'a str>,
    /// `--fix`.
    pub fix: bool,
    /// `--dry-run` (with `--fix`).
    pub dry_run: bool,
}

/// Load each store on its own (doctor reports what would stop loading them as a set),
/// diagnose, fix if asked, print.
pub fn run(env: &Env, store_flags: &[PathBuf], args: &Args, out: &Output) -> Result<(), Failure> {
    let cfg = config::load_config(env)?;
    let paths = config::store_paths(store_flags, env, &cfg);
    config::check_config(store_flags, env, &cfg, &paths)?;
    let caches = super::caches(env, &cfg);
    let first = Run::load(env, &paths, args.repo, &caches, Some(out))?;
    let mut lines = Vec::new();
    let mut fixed: Option<Applied> = None;
    let last = if args.fix {
        let repairs = Repairs::plan(
            &first.diagnosis.findings,
            &first.stores,
            &first.machine,
            &env.machine_file(),
        );
        if repairs.is_empty() {
            lines.push("nothing to fix".to_owned());
            fixed = Some(Applied::default());
            first
        } else if args.dry_run {
            for f in &repairs.fixes {
                lines.push(format!("would {}", f.describe(Some(&env.home))));
            }
            fixed = Some(Applied::default());
            first
        } else {
            let applied = repair(env, &paths, args.repo, &caches, &repairs, &mut lines)?;
            fixed = Some(applied);
            Run::load(env, &paths, args.repo, &caches, None)?
        }
    } else {
        first
    };
    let Run {
        machine, diagnosis, ..
    } = last;
    let errors = diagnosis
        .findings
        .iter()
        .filter(|f| f.level == Level::Error)
        .count();
    let loaded = Loaded {
        config: cfg,
        stores: diagnosis.stores,
        machine,
        plan: diagnosis.plan,
        caches,
    };
    loaded.save_caches(out);
    if let Some(name) = args.repo {
        loaded.repo(name)?;
    }
    if out.json {
        let text = json::doctor(&loaded.stores, &diagnosis.findings, fixed.as_ref())
            .map_err(|e| Failure::json("the findings", e))?;
        out.print(&text)?;
    } else {
        let mut text = String::new();
        for line in &lines {
            text.push_str(line);
            text.push('\n');
        }
        if !lines.is_empty() {
            text.push('\n');
        }
        text.push_str(&human(&loaded, &diagnosis.findings, out));
        out.print(&text)?;
    }
    if errors > 0 {
        return Err(Failure::Findings(errors));
    }
    Ok(())
}

/// The stores as loaded, `machine.toml`, and what doctor found in them.
struct Run {
    stores: Vec<Store>,
    machine: Machine,
    diagnosis: Diagnosis,
}

impl Run {
    /// Load and diagnose; `announce` prints the mounted stores (with `-v`).
    fn load(
        env: &Env,
        paths: &[PathBuf],
        repo: Option<&str>,
        caches: &Caches,
        announce: Option<&Output>,
    ) -> Result<Run, Failure> {
        let stores = paths
            .iter()
            .map(|p| config::load_store(p, env))
            .collect::<Result<Vec<Store>, _>>()?;
        if let Some(out) = announce {
            for store in &stores {
                out.info(&format!(
                    "mounted store `{}` at {}",
                    store.name,
                    out.path(&store.root)
                ));
            }
        }
        let machine = config::load_machine(env)?;
        let checkouts = Checkouts::read(&machine);
        let diagnosis = doctor::diagnose(
            stores.clone(),
            &machine,
            &checkouts,
            &env.home,
            repo,
            caches,
        )?;
        Ok(Run {
            stores,
            machine,
            diagnosis,
        })
    }
}

/// The output line of a fix that did not run.
fn skipped_line(s: &Skipped, home: Option<&Path>) -> String {
    format!("skipped: {}: {}", s.fix.describe(home), s.reason)
}

/// Apply `repairs` under the sync lock, describing each in `lines`.
fn repair(
    env: &Env,
    paths: &[PathBuf],
    repo: Option<&str>,
    caches: &Caches,
    repairs: &Repairs,
    lines: &mut Vec<String>,
) -> Result<Applied, Failure> {
    let home = Some(env.home.as_path());
    let _lock = apply::Lock::acquire(&env.config_dir)?;
    let removed = repairs.apply_files()?;
    let mut applied = Applied::default();
    let skip = |applied: &mut Applied, lines: &mut Vec<String>, fix: &Fix, reason: String| {
        let s = Skipped {
            fix: fix.clone(),
            reason,
        };
        lines.push(skipped_line(&s, home));
        applied.skipped.push(s);
    };
    for fix in &repairs.fixes {
        match fix {
            Fix::Resync { .. } => continue,
            Fix::RemoveLink { link, .. } if !removed.contains(link) => {
                let reason = "it changed since doctor looked".to_owned();
                skip(&mut applied, lines, fix, reason);
                continue;
            }
            _ => {}
        }
        lines.push(format!("fixed: {}", fix.describe(home)));
        applied.fixed.push(fix.clone());
    }
    if repairs.resync.is_empty() {
        return Ok(applied);
    }
    // Resolved again, so the config edits above count (a removed checkout is no longer
    // granted); references doctor reports as errors stay out of the plan it syncs.
    let run = Run::load(env, paths, repo, caches, None)?;
    let opts = sync::Options {
        home: Some(env.home.clone()),
        ..sync::Options::default()
    };
    let in_plan: Vec<String> = repairs
        .resync
        .iter()
        .filter(|n| run.diagnosis.plan.repo(n).is_some())
        .cloned()
        .collect();
    let outcome = sync::run(
        &run.diagnosis.stores,
        &run.machine,
        &run.diagnosis.plan,
        &Selection::Only(in_plan),
        &opts,
        caches,
    )?;
    // In finding order, each repo the fixes named: synced, or why not.
    for name in &repairs.resync {
        let fix = Fix::Resync { repo: name.clone() };
        let state = outcome
            .repos
            .iter()
            .find(|r| r.name == *name)
            .map(|r| &r.state);
        let checkout_removed = applied
            .fixed
            .iter()
            .any(|f| matches!(f, Fix::RemoveCheckout { entry, .. } if entry == name));
        if let Some(reason) = resync_skipped(name, state, checkout_removed) {
            skip(&mut applied, lines, &fix, reason);
            continue;
        }
        let report = state.and_then(RepoState::report);
        let n = report.map_or(0, |r| r.changes.len());
        let mut line = format!(
            "fixed: {} ({n} change{})",
            fix.describe(home),
            if n == 1 { "" } else { "s" }
        );
        for s in report.iter().flat_map(|r| &r.skipped) {
            line.push_str(&format!("\n  skipped {}: {}", s.path.display(), s.message));
        }
        lines.push(line);
        applied.fixed.push(fix);
    }
    Ok(applied)
}

fn human(loaded: &Loaded, findings: &[Finding], out: &Output) -> String {
    let p = &out.paint;
    let mut text = String::new();
    for f in findings {
        let label = format!("{}[{}]", f.level, f.code);
        let label = match f.level {
            Level::Error => p.error(label),
            Level::Warn => p.warn(label),
            Level::Info => p.heading(label),
        };
        let repo = f
            .repo
            .as_deref()
            .map_or_else(String::new, |r| format!(" {r}:"));
        text.push_str(&format!("{label}{repo} {}\n", f.message));
        if let Some(at) = &f.at {
            let at = loaded.stores.provenance(at);
            text.push_str(&format!("  {} {}\n", p.dim("-->"), p.dim(at)));
        }
        text.push_str(&format!("  {} {}\n", p.hint("hint:"), f.hint));
    }
    let count = |level| findings.iter().filter(|f| f.level == level).count();
    let (e, w, i) = (count(Level::Error), count(Level::Warn), count(Level::Info));
    if findings.is_empty() {
        text.push_str("no problems found\n");
    } else {
        let plural =
            |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        text.push_str(&format!(
            "\n{}, {}, {} info\n",
            plural(e, "error", "errors"),
            plural(w, "warning", "warnings"),
            i
        ));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_skipped_fix_is_reported_with_its_reason() {
        let s = Skipped {
            fix: Fix::Resync { repo: "api".into() },
            reason: resync_skipped("api", Some(&RepoState::NotCloned), true).unwrap(),
        };
        assert_eq!(
            skipped_line(&s, None),
            "skipped: re-sync `api`: this run removed its machine.toml entry; nothing to re-sync"
        );
    }
}
