//! One module per subcommand (roadmap §7). Every command after loading is a function
//! of the [`Plan`]; nothing reads config twice (roadmap §12).

mod authoring;
mod doctor;
mod edit;
pub mod explain;
mod grep;
mod resolve;
mod scan;
mod status;
pub mod stores;
mod sync;
mod tree;
mod where_;

use std::path::PathBuf;

use kipi_core::cache::Caches;
use kipi_core::config::{self, Env};
use kipi_core::error::closest;
use kipi_core::model::{Config, Machine, StoreSet};
use kipi_core::plan::{Plan, RepoPlan};
use kipi_core::resolve::resolve as resolve_plan;
use kipi_core::tokens::Tokenizer;

use crate::cli::{Add, Command, Global};
use crate::error::Failure;
use crate::output::Output;

/// Everything a read-only command needs: the mounted stores, this machine and the plan.
pub struct Loaded {
    /// `config.toml`.
    pub config: Config,
    /// The mounted stores.
    pub stores: StoreSet,
    /// `machine.toml`.
    pub machine: Machine,
    /// The resolved plan.
    pub plan: Plan,
    /// Token counts and git's tracked files, remembered in `~/.cache/kipi`.
    pub caches: Caches,
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
            caches: caches(env, &cfg),
            config: cfg,
            stores,
            machine,
            plan,
        })
    }

    /// Write the caches back. A failure only costs speed next time, so it is a note on
    /// stderr (with `-v`), not an error.
    pub fn save_caches(&self, out: &Output) {
        for e in self.caches.save() {
            out.info(&format!("cache not saved: {e}"));
        }
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

/// The caches under `~/.cache/kipi` (or `$KIPI_CACHE_DIR`), counting tokens with
/// `config.toml`'s `tokenizer` (default `cl100k_base`).
pub fn caches(env: &Env, cfg: &Config) -> Caches {
    let tokenizer = cfg
        .tokenizer
        .as_deref()
        .and_then(Tokenizer::from_name)
        .unwrap_or_default();
    Caches::open(&env.cache_dir, tokenizer)
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
        Command::Where {
            alias,
            notes,
            any_scope,
        } => where_::run(&load(&env()?)?, alias, *notes, *any_scope, out),
        Command::Grep {
            pattern,
            scope,
            repos,
            limit,
        } => {
            let args = grep::Args {
                pattern,
                scope: scope.as_deref(),
                repos: *repos,
                limit: *limit,
            };
            grep::run(&load(&env()?)?, &args, out)
        }
        Command::Edit { target } => {
            let env = env()?;
            edit::run(&env, &global.stores, &load(&env)?, target, out)
        }
        Command::Tree {
            repo,
            skill,
            store,
            budget,
            machine,
        } => {
            let env = env()?;
            let args = tree::Args {
                repo: repo.as_deref(),
                skill: skill.as_deref(),
                store: store.as_deref(),
                budget: *budget,
                machine: *machine,
            };
            tree::run(&env, &load(&env)?, &args, out)
        }
        Command::Doctor { repo, fix, .. } => {
            doctor::run(&env()?, &global.stores, repo.as_deref(), *fix, out)
        }
        Command::Init {
            store,
            name,
            git,
            dry_run,
        } => authoring::init(
            &env()?,
            &kipi_core::authoring::InitOptions {
                path: store.as_deref(),
                name: name.as_deref(),
                git: *git,
            },
            dry_run.dry_run,
            out,
        ),
        Command::Stores { unmount, dry_run } => match unmount {
            Some(which) => authoring::unmount(&env()?, which, dry_run.dry_run, out),
            None => stores::run(&env()?, &global.stores, out),
        },
        Command::Add(Add::Skill {
            name,
            scope,
            store,
            from,
            link_back,
            dry_run,
        }) => authoring::add_skill(
            &env()?,
            &global.stores,
            &kipi_core::authoring::SkillOptions {
                name,
                scope: scope.as_deref(),
                store: store.as_deref(),
                from: from.as_deref(),
                link_back: *link_back,
            },
            dry_run.dry_run,
            out,
        ),
        Command::Add(Add::Repo {
            name,
            scope,
            path,
            summary,
            dry_run,
        }) => authoring::add_repo(
            &env()?,
            &global.stores,
            &kipi_core::authoring::RepoOptions {
                name,
                scope: scope.as_deref(),
                path: path.as_deref(),
                summary: summary.as_deref(),
            },
            dry_run.dry_run,
            out,
        ),
        Command::Bind(b) | Command::Unbind(b) => authoring::bind(
            &env()?,
            &global.stores,
            &kipi_core::authoring::Binding {
                repo: &b.repo,
                skills: &b.skills,
                related: &b.related,
                exclude: &b.exclude,
                aliases: &b.aliases,
            },
            matches!(command, Command::Bind(_)),
            b.dry_run.dry_run,
            out,
        ),
        other => Err(not_implemented(other)),
    }
}

/// The repo entry whose checkout holds the working directory, if any.
pub fn cwd_repo(loaded: &Loaded) -> Option<&RepoPlan> {
    let cwd = std::env::current_dir().ok()?;
    kipi_core::search::repo_at(&loaded.plan, &cwd)
}

/// The error for a declared but unimplemented subcommand.
fn not_implemented(command: &Command) -> Failure {
    let name = command.name();
    let stage = "Stage 4";
    Failure::user(
        format!("`kipi {name}` is not implemented yet"),
        format!(
            "it lands in {stage} (roadmap §14); `kipi resolve` and `kipi explain <repo>` work today"
        ),
    )
}
