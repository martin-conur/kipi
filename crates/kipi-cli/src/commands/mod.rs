//! One module per subcommand (roadmap §7). Every command after loading is a function
//! of the [`Plan`]; nothing reads config twice (roadmap §12).

mod explain;
mod resolve;
mod scan;
mod status;
mod sync;

use std::path::PathBuf;

use kipi_core::config::{self, Env};
use kipi_core::error::closest;
use kipi_core::model::{Machine, StoreSet};
use kipi_core::plan::{Plan, RepoPlan};
use kipi_core::resolve::resolve as resolve_plan;

use crate::cli::{Command, Global};
use crate::error::Failure;
use crate::output::Output;

/// Everything a read-only command needs: the mounted stores, this machine and the plan.
pub struct Loaded {
    /// The mounted stores.
    pub stores: StoreSet,
    /// `machine.toml`.
    pub machine: Machine,
    /// The resolved plan.
    pub plan: Plan,
}

impl Loaded {
    /// `config::load` → `resolve::resolve` (roadmap §12, data flow).
    pub fn load(env: &Env, store_flags: &[PathBuf], out: &Output) -> Result<Self, Failure> {
        let cfg = config::load_config(env)?;
        let paths = config::store_paths(store_flags, env, &cfg);
        let stores = config::load_stores(&paths, env)?;
        for store in stores.stores() {
            out.info(&format!(
                "mounted store `{}` at {}",
                store.name,
                out.path(&store.root)
            ));
        }
        let machine = config::load_machine(env)?;
        let plan = resolve_plan(&stores, &machine)?;
        Ok(Loaded {
            stores,
            machine,
            plan,
        })
    }

    /// The plan of repo entry `name`, or a user error naming the closest entry.
    pub fn repo(&self, name: &str) -> Result<&RepoPlan, Failure> {
        if let Some(repo) = self.plan.repo(name) {
            return Ok(repo);
        }
        let names = self.plan.repos.iter().map(|r| r.name.as_str());
        let by_alias = self
            .plan
            .registries
            .iter()
            .flat_map(|r| &r.entries)
            .find(|e| e.keys.contains(&kipi_core::plan::alias_key(name)));
        let hint = match (closest(name, names), by_alias) {
            (_, Some(entry)) => format!(
                "`{name}` is an alias of `{}`; pass the entry name",
                entry.name
            ),
            (Some(near), None) => format!("did you mean `{near}`?"),
            (None, None) => {
                "`kipi resolve` lists every repo entry of the mounted stores".to_owned()
            }
        };
        Err(Failure::user(
            format!("no repo entry `{name}` in the mounted stores"),
            hint,
        ))
    }
}

/// Run `command`.
pub fn run(command: &Command, global: &Global, out: &Output) -> Result<(), Failure> {
    if command.dry_run().is_some_and(|d| d.dry_run) {
        out.info("dry run: nothing will be written");
    }
    let env = || Env::from_process().map_err(Failure::from);
    let load = |env: &Env| Loaded::load(env, &global.stores, out);
    match command {
        Command::Resolve { repo } => resolve::run(&load(&env()?)?, repo.as_deref(), out),
        Command::Explain { repo } => explain::run(&load(&env()?)?, repo, out),
        Command::Sync {
            repos,
            all,
            agents,
            force,
            allow_tracked,
            dry_run,
        } => {
            let env = env()?;
            let args = sync::Args {
                repos,
                all: *all,
                agents,
                force: *force,
                allow_tracked: *allow_tracked,
                dry_run: dry_run.dry_run,
            };
            sync::run(&env, &load(&env)?, &args, out)
        }
        Command::Status { repo } => {
            let env = env()?;
            status::run(&env, &load(&env)?, repo.as_deref(), out)
        }
        Command::Scan {
            paths,
            refresh_drafts,
            store,
            scope,
            dry_run,
        } => scan::run(
            &env()?,
            &global.stores,
            &scan::Args {
                paths,
                refresh_drafts: *refresh_drafts,
                store: store.as_deref(),
                scope: scope.as_deref(),
                dry_run: dry_run.dry_run,
            },
            out,
        ),
        other => Err(not_implemented(other)),
    }
}

/// The error for a declared but unimplemented subcommand.
fn not_implemented(command: &Command) -> Failure {
    let name = command.name();
    let stage = match command {
        Command::Tree { .. }
        | Command::Where { .. }
        | Command::Grep { .. }
        | Command::Edit { .. }
        | Command::Doctor { .. } => "Stage 3",
        Command::Completions { .. } => "Stage 4",
        _ => "Stage 2",
    };
    Failure::user(
        format!("`kipi {name}` is not implemented yet"),
        format!(
            "it lands in {stage} (roadmap §14); `kipi resolve` and `kipi explain <repo>` work today"
        ),
    )
}
