//! `kipi hook install|uninstall [repo...|--all]`: the git `post-checkout` hook that runs
//! `kipi sync` in a new worktree ([`kipi_core::hook`]). One line per repo; a repo the
//! hook cannot go into is a warning, and the command fails once the rest are done.

use kipi_core::hook::{self, Installed, Uninstalled};
use kipi_core::plan::RepoPlan;

use super::Loaded;
use crate::error::Failure;
use crate::output::Output;

/// The command's arguments.
pub struct Args<'a> {
    /// `install` (else `uninstall`).
    pub install: bool,
    /// Repo entries.
    pub repos: &'a [String],
    /// `--all`: every repo cloned here.
    pub all: bool,
    /// `--dry-run`.
    pub dry_run: bool,
}

/// Install or uninstall the hook in each selected repo, then report.
pub fn run(loaded: &Loaded, args: &Args, out: &Output) -> Result<(), Failure> {
    let verb = if args.install { "install" } else { "uninstall" };
    let repos: Vec<&RepoPlan> = if args.all {
        loaded
            .plan
            .repos
            .iter()
            .filter(|r| r.path.is_some())
            .collect()
    } else if args.repos.is_empty() {
        return Err(Failure::user(
            format!("no repo to {verb} the hook in"),
            format!("name repo entries (`kipi hook {verb} <repo>`) or pass `--all`"),
        ));
    } else {
        args.repos
            .iter()
            .map(|n| loaded.repo(n))
            .collect::<Result<_, _>>()?
    };
    let p = &out.paint;
    let mut text = String::new();
    let mut failed = 0;
    for repo in repos {
        let Some(dir) = &repo.path else {
            text.push_str(&format!(
                "{}\n{}",
                p.heading(&repo.name),
                p.dim("  skipped: not cloned on this machine\n")
            ));
            continue;
        };
        text.push_str(&format!("{}  {}\n", p.heading(&repo.name), out.path(dir)));
        let done = if args.install {
            hook::install(dir, args.dry_run).map(|i| installed(i, args.dry_run))
        } else {
            hook::uninstall(dir, args.dry_run).map(|u| uninstalled(u, args.dry_run))
        };
        match done {
            Ok(line) => text.push_str(&format!("  {line}\n")),
            Err(e) => {
                failed += 1;
                text.push_str(&format!(
                    "  {} {e}\n    {} {}\n",
                    p.warn("warning:"),
                    p.hint("hint:"),
                    e.hint()
                ));
            }
        }
    }
    out.print(&text)?;
    if failed > 0 {
        return Err(Failure::user(
            format!(
                "the hook could not be {verb}ed in {failed} repo{}",
                if failed == 1 { "" } else { "s" }
            ),
            "see the warning under each repo above",
        ));
    }
    Ok(())
}

fn installed(i: Installed, dry_run: bool) -> &'static str {
    match (i, dry_run) {
        (Installed::Created, false) => "post-checkout hook installed",
        (Installed::Created, true) => "would install the post-checkout hook",
        (Installed::Chained, false) => {
            "post-checkout hook installed; the hook already there is kept as post-checkout.pre-kipi and runs first"
        }
        (Installed::Chained, true) => {
            "would install the post-checkout hook, keeping the one already there as post-checkout.pre-kipi"
        }
        (Installed::Updated, false) => "post-checkout hook updated",
        (Installed::Updated, true) => "would update the post-checkout hook",
        (Installed::Unchanged, _) => "post-checkout hook already installed",
    }
}

fn uninstalled(u: Uninstalled, dry_run: bool) -> &'static str {
    match (u, dry_run) {
        (Uninstalled::Removed, false) => "post-checkout hook removed",
        (Uninstalled::Removed, true) => "would remove the post-checkout hook",
        (Uninstalled::Restored, false) => {
            "post-checkout hook removed; the hook it had replaced is back"
        }
        (Uninstalled::Restored, true) => {
            "would remove the post-checkout hook and put back the one it had replaced"
        }
        (Uninstalled::NotInstalled, _) => "no kipi post-checkout hook here",
    }
}
