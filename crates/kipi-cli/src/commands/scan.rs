//! `kipi scan [path...]`: find checkouts, record them in `machine.toml`, and draft
//! entries for new repos (roadmap §7, §9). `--dry-run` prints the TOML diff instead.

use std::path::PathBuf;

use kipi_core::config::{self, Env};
use kipi_core::model::StoreSet;
use kipi_core::scan::{self, Scan, ScanOptions};

use crate::error::Failure;
use crate::output::Output;
use crate::output::{diff, json};

/// The command's arguments.
pub struct Args<'a> {
    /// Directories to walk instead of the scope roots.
    pub paths: &'a [PathBuf],
    /// `--refresh-drafts`.
    pub refresh_drafts: bool,
    /// `--store <name>`.
    pub store: Option<&'a str>,
    /// `--scope <scope>`.
    pub scope: Option<&'a str>,
    /// `--dry-run`.
    pub dry_run: bool,
}

/// Scan, write (unless `--dry-run`) and report.
pub fn run(env: &Env, store_flags: &[PathBuf], args: &Args, out: &Output) -> Result<(), Failure> {
    let cfg = config::load_config(env)?;
    let stores = config::load_stores(&config::store_paths(store_flags, env, &cfg), env)?;
    let machine = config::load_machine(env)?;
    if let Some(scope) = args.scope {
        check_target(&stores, args.store, scope)?;
    }
    let paths = args
        .paths
        .iter()
        .map(|p| {
            std::path::absolute(p).map_err(|e| {
                Failure::user(
                    format!("cannot resolve {}: {e}", p.display()),
                    "pass an existing directory",
                )
            })
        })
        .collect::<Result<_, _>>()?;
    let opts = ScanOptions {
        paths,
        refresh_drafts: args.refresh_drafts,
        scope: args.scope.map(str::to_owned),
    };
    let result = scan::scan(&stores, &machine, &env.machine_file(), &opts)?;
    if !args.dry_run {
        scan::write_files(&result.writes)?;
    }
    if out.json {
        let text = json::scan(&result, args.dry_run).map_err(|e| Failure::json("the scan", e))?;
        return Ok(out.print(&text)?);
    }
    out.print(&human(&result, args.dry_run, out))?;
    Ok(())
}

/// `--scope` must name a mounted scope, owned by `--store` when that is given.
fn check_target(stores: &StoreSet, store: Option<&str>, scope: &str) -> Result<(), Failure> {
    let Some((owner, _)) = stores.scope(scope) else {
        let names = stores
            .stores()
            .iter()
            .flat_map(|s| &s.scopes)
            .map(|s| s.name.as_str());
        let hint = match kipi_core::error::closest(scope, names) {
            Some(near) => format!("did you mean `{near}`?"),
            None => "`kipi resolve` shows the scopes of the mounted stores".to_owned(),
        };
        return Err(Failure::user(
            format!("no scope `{scope}` in the mounted stores"),
            hint,
        ));
    };
    match store {
        Some(name) if name != owner.name => Err(Failure::user(
            format!(
                "scope `{scope}` belongs to store `{}`, not `{name}`",
                owner.name
            ),
            format!(
                "pass `--store {}`, or a scope of store `{name}`",
                owner.name
            ),
        )),
        _ => Ok(()),
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn human(scan: &Scan, dry_run: bool, out: &Output) -> String {
    let p = &out.paint;
    let mut text = format!(
        "Scanned {}: {}.\n",
        plural(scan.roots.len(), "directory", "directories"),
        plural(scan.checkouts.len(), "checkout", "checkouts")
    );
    for root in &scan.missing_roots {
        text.push_str(&p.dim(format!("  skipped {} (does not exist)\n", out.path(root))));
    }
    let draft = |s: &str| format!("{s} {}", p.dim("(draft)"));

    if !scan.added.is_empty() {
        text.push_str(&p.heading(format!(
            "\nAdded {}:\n",
            plural(scan.added.len(), "entry", "entries")
        )));
        for a in &scan.added {
            let scope = if a.explicit_scope {
                format!("scope = \"{}\"", a.scope)
            } else {
                format!("scope {}", a.scope)
            };
            text.push_str(&format!(
                "  + {}  store {}, {scope}  {}\n",
                p.heading(&a.name),
                a.store,
                out.path(&a.path)
            ));
            if let Some(r) = &a.remote {
                text.push_str(&format!("      remote:  {r}\n"));
            }
            if let Some(s) = &a.summary {
                text.push_str(&format!("      summary: {}\n", draft(s)));
            }
            if !a.aliases.is_empty() {
                text.push_str(&format!(
                    "      aliases: {}\n",
                    draft(&a.aliases.join(", "))
                ));
            }
        }
    }
    if !scan.recorded.is_empty() {
        text.push_str(&p.heading("\nRecorded in machine.toml:\n"));
        for r in &scan.recorded {
            let moved = r
                .previous
                .as_ref()
                .map(|old| p.dim(format!("  (moved from {})", out.path(old))))
                .unwrap_or_default();
            text.push_str(&format!("  {}  {}{moved}\n", r.entry, out.path(&r.path)));
        }
    }
    if !scan.remotes_filled.is_empty() {
        text.push_str(&p.heading("\nFilled remote:\n"));
        for r in &scan.remotes_filled {
            text.push_str(&format!(
                "  {}  {}  {}\n",
                r.entry,
                r.remote,
                p.dim(&r.store)
            ));
        }
    }
    if !scan.refreshed.is_empty() {
        text.push_str(&p.heading("\nRefreshed drafts:\n"));
        for r in &scan.refreshed {
            text.push_str(&format!("  {}.{} = {}\n", r.entry, r.field, r.value));
        }
    }
    for w in &scan.warnings {
        text.push_str(&format!(
            "\n{} {}\n  {} {}\n",
            p.warn("warning:"),
            w.message(|path| out.path(path)),
            p.hint("hint:"),
            w.hint(|path| out.path(path))
        ));
    }
    if !scan.not_found.is_empty() {
        text.push_str(&format!(
            "\nNot on this machine: {}\n",
            scan.not_found.join(", ")
        ));
    }

    text.push('\n');
    if scan.writes.is_empty() {
        text.push_str("Nothing to write.\n");
    } else if dry_run {
        text.push_str(&format!(
            "Dry run: would write {}:\n",
            plural(scan.writes.len(), "file", "files")
        ));
        for w in &scan.writes {
            let before = w.before.as_deref().unwrap_or_default();
            text.push_str(&diff::unified(&out.path(&w.path), before, &w.after, p));
        }
    } else {
        for w in &scan.writes {
            text.push_str(&format!("Wrote {}\n", out.path(&w.path)));
        }
    }
    text
}
