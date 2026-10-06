//! `kipi doctor [repo]`: every check of roadmap §10 that needs only the stores, this
//! machine's `machine.toml` and the plan (docs/doctor.md). Exits 2 when any finding is
//! an error.

use kipi_core::config::{self, Env};
use kipi_core::doctor::{self, Checkouts, Finding, Level};
use kipi_core::model::Store;

use super::Loaded;
use crate::error::Failure;
use crate::output::{Output, json};

/// Load each store on its own (doctor reports what would stop loading them as a set),
/// diagnose, print.
pub fn run(
    env: &Env,
    store_flags: &[std::path::PathBuf],
    repo: Option<&str>,
    fix: bool,
    out: &Output,
) -> Result<(), Failure> {
    if fix {
        return Err(Failure::user(
            "`kipi doctor --fix` is not implemented yet",
            "it lands with the filesystem checks (Stage 3, #13); each finding's hint says \
             what to change by hand",
        ));
    }
    let cfg = config::load_config(env)?;
    let paths = config::store_paths(store_flags, env, &cfg);
    let stores = paths
        .iter()
        .map(|p| config::load_store(p, env))
        .collect::<Result<Vec<Store>, _>>()?;
    for store in &stores {
        out.info(&format!(
            "mounted store `{}` at {}",
            store.name,
            out.path(&store.root)
        ));
    }
    let machine = config::load_machine(env)?;
    let checkouts = Checkouts::read(&machine);
    let diagnosis = doctor::diagnose(stores, &machine, &checkouts, &env.home, repo)?;
    let errors = diagnosis
        .findings
        .iter()
        .filter(|f| f.level == Level::Error)
        .count();
    let loaded = Loaded {
        stores: diagnosis.stores,
        machine,
        plan: diagnosis.plan,
    };
    if let Some(name) = repo {
        loaded.repo(name)?;
    }
    if out.json {
        let text = json::doctor(&loaded.stores, &diagnosis.findings)
            .map_err(|e| Failure::json("the findings", e))?;
        out.print(&text)?;
    } else {
        out.print(&human(&loaded, &diagnosis.findings, out))?;
    }
    if errors > 0 {
        return Err(Failure::Findings(errors));
    }
    Ok(())
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
