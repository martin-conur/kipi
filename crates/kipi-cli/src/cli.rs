//! The clap command tree: every §7 subcommand is declared, so `--help` documents the
//! whole v1 surface even before a command is implemented.

use std::path::PathBuf;

use clap::{ArgAction, Args, Parser, Subcommand};

/// Give LLM coding agents a scoped map of your local repos and shared skills.
#[derive(Debug, Parser)]
#[command(name = "kipi", version, about, long_about = None)]
pub struct Cli {
    /// Flags every command accepts.
    #[command(flatten)]
    pub global: Global,
    /// The subcommand.
    #[command(subcommand)]
    pub command: Command,
}

/// Global flags (roadmap §7).
#[derive(Debug, Args)]
pub struct Global {
    /// Mount this store instead of the configured ones (repeatable; before the subcommand).
    #[arg(long = "store", value_name = "PATH")]
    pub stores: Vec<PathBuf>,
    /// Print machine-readable JSON (see docs/json.md).
    #[arg(long, global = true)]
    pub json: bool,
    /// Never color output (NO_COLOR is respected too).
    #[arg(long, global = true)]
    pub no_color: bool,
    /// Say more on stderr (repeatable).
    #[arg(short, long, action = ArgAction::Count, global = true, conflicts_with = "quiet")]
    pub verbose: u8,
    /// Say nothing on stderr but errors.
    #[arg(short, long, global = true)]
    pub quiet: bool,
}

/// `--dry-run`, shared by every command that writes.
#[derive(Debug, Clone, Copy, Args)]
pub struct DryRun {
    /// Print what would change and write nothing.
    #[arg(long)]
    pub dry_run: bool,
}

/// The subcommands (roadmap §7).
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create a store and mount it, or mount an existing one.
    Init {
        /// Where the store lives (default: ~/.agents/store).
        #[arg(long, value_name = "PATH")]
        store: Option<PathBuf>,
        /// The store's name (`[store] name`).
        #[arg(long)]
        name: Option<String>,
        /// Also `git init` the store.
        #[arg(long)]
        git: bool,
        #[command(flatten)]
        dry_run: DryRun,
    },
    /// List mounted stores, or unmount one.
    Stores {
        /// Remove this store from config.toml.
        #[arg(long, value_name = "NAME")]
        unmount: Option<String>,
        #[command(flatten)]
        dry_run: DryRun,
    },
    /// Find git repos under scope roots (or PATHs) and record them.
    Scan {
        /// Directories to walk instead of the scope roots.
        paths: Vec<PathBuf>,
        /// Rewrite values still marked `# kipi:draft`.
        #[arg(long)]
        refresh_drafts: bool,
        #[command(flatten)]
        dry_run: DryRun,
    },
    /// Add a skill or a repo entry.
    #[command(subcommand)]
    Add(Add),
    /// Add skills, relations, exclusions or aliases to a repo entry.
    Bind(Binding),
    /// Remove skills, relations, exclusions or aliases from a repo entry.
    Unbind(Binding),
    /// Print the resolved plan (no side effects).
    Resolve {
        /// Only this repo entry.
        repo: Option<String>,
    },
    /// Show where every decision for a repo comes from.
    Explain {
        /// The repo entry.
        repo: String,
    },
    /// Materialize the plan into repos.
    Sync {
        /// Repo entries to sync.
        repos: Vec<String>,
        /// Sync every repo cloned on this machine.
        #[arg(long)]
        all: bool,
        /// Adapters to render, overriding the plan.
        #[arg(long, value_delimiter = ',')]
        agents: Vec<String>,
        /// Replace files kipi did not write.
        #[arg(long)]
        force: bool,
        #[command(flatten)]
        dry_run: DryRun,
    },
    /// Per-repo sync state and always-on tokens vs budget.
    Status {
        /// Only this repo entry.
        repo: Option<String>,
    },
    /// Stores → scopes → repos → skills, with provenance markers.
    Tree {
        /// Only this repo entry.
        #[arg(long)]
        repo: Option<String>,
        /// Only repos using this skill.
        #[arg(long)]
        skill: Option<String>,
        /// Only this store (by name).
        #[arg(long, value_name = "NAME")]
        store: Option<String>,
        /// Show always-on token counts.
        #[arg(long)]
        budget: bool,
        /// Show this machine's paths.
        #[arg(long)]
        machine: bool,
    },
    /// Resolve an alias to a repo, its summary and paths.
    Where {
        /// A repo name or alias.
        alias: String,
        /// Print the repo's notes.
        #[arg(long)]
        notes: bool,
    },
    /// Search skills, notes and conventions (and checkouts with --repos).
    Grep {
        /// A regular expression.
        pattern: String,
        /// Only this scope.
        #[arg(long)]
        scope: Option<String>,
        /// Also search the checkouts of every repo in the scope chain.
        #[arg(long)]
        repos: bool,
    },
    /// Open a repo entry, skill, notes:<repo> or conventions:<scope> in $EDITOR.
    Edit {
        /// What to edit.
        target: String,
    },
    /// Check stores, repos and grants for problems.
    Doctor {
        /// Only this repo entry.
        repo: Option<String>,
        /// Apply the fixes marked safe.
        #[arg(long)]
        fix: bool,
        #[command(flatten)]
        dry_run: DryRun,
    },
    /// Print shell completions.
    Completions {
        /// bash, zsh, fish, elvish or powershell.
        shell: String,
    },
}

/// `kipi add`.
#[derive(Debug, Subcommand)]
pub enum Add {
    /// Create a skill from the template, or import an existing SKILL.md tree.
    Skill {
        /// The skill's directory name.
        name: String,
        /// Bind it to this scope.
        #[arg(long)]
        scope: Option<String>,
        /// The store to create it in (by name).
        #[arg(long, value_name = "NAME")]
        store: Option<String>,
        /// A git URL or path to import from.
        #[arg(long, value_name = "GIT-URL|PATH")]
        from: Option<String>,
        #[command(flatten)]
        dry_run: DryRun,
    },
    /// Register a repo by hand.
    Repo {
        /// The entry name.
        name: String,
        /// Its scope.
        #[arg(long)]
        scope: Option<String>,
        /// Its checkout on this machine.
        #[arg(long)]
        path: Option<PathBuf>,
        /// Its one-line summary.
        #[arg(long)]
        summary: Option<String>,
        #[command(flatten)]
        dry_run: DryRun,
    },
}

/// The arguments of `bind` and `unbind`.
#[derive(Debug, Args)]
pub struct Binding {
    /// The repo entry.
    pub repo: String,
    /// Skills.
    #[arg(long = "skill", value_name = "SKILL", num_args = 1..)]
    pub skills: Vec<String>,
    /// Related repos.
    #[arg(long, value_name = "REPO", num_args = 1..)]
    pub related: Vec<String>,
    /// Excluded skills.
    #[arg(long, value_name = "SKILL", num_args = 1..)]
    pub exclude: Vec<String>,
    /// Aliases.
    #[arg(long = "alias", value_name = "ALIAS", num_args = 1..)]
    pub aliases: Vec<String>,
    #[command(flatten)]
    pub dry_run: DryRun,
}

impl Command {
    /// The subcommand's name as typed (`add skill`, `sync`).
    pub fn name(&self) -> &'static str {
        match self {
            Command::Init { .. } => "init",
            Command::Stores { .. } => "stores",
            Command::Scan { .. } => "scan",
            Command::Add(Add::Skill { .. }) => "add skill",
            Command::Add(Add::Repo { .. }) => "add repo",
            Command::Bind(_) => "bind",
            Command::Unbind(_) => "unbind",
            Command::Resolve { .. } => "resolve",
            Command::Explain { .. } => "explain",
            Command::Sync { .. } => "sync",
            Command::Status { .. } => "status",
            Command::Tree { .. } => "tree",
            Command::Where { .. } => "where",
            Command::Grep { .. } => "grep",
            Command::Edit { .. } => "edit",
            Command::Doctor { .. } => "doctor",
            Command::Completions { .. } => "completions",
        }
    }

    /// The command's `--dry-run`, for commands that write.
    pub fn dry_run(&self) -> Option<DryRun> {
        match self {
            Command::Init { dry_run, .. }
            | Command::Stores { dry_run, .. }
            | Command::Scan { dry_run, .. }
            | Command::Add(Add::Skill { dry_run, .. } | Add::Repo { dry_run, .. })
            | Command::Sync { dry_run, .. }
            | Command::Doctor { dry_run, .. } => Some(*dry_run),
            Command::Bind(b) | Command::Unbind(b) => Some(b.dry_run),
            Command::Resolve { .. }
            | Command::Explain { .. }
            | Command::Status { .. }
            | Command::Tree { .. }
            | Command::Where { .. }
            | Command::Grep { .. }
            | Command::Edit { .. }
            | Command::Completions { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn command_tree_is_well_formed() {
        Cli::command().debug_assert();
    }

    #[test]
    fn writers_take_dry_run_and_readers_do_not() {
        let parse = |args: &[&str]| Cli::try_parse_from(args).unwrap().command;
        let sync = parse(&["kipi", "sync", "--all", "--dry-run"]);
        assert!(sync.dry_run().unwrap().dry_run);
        assert!(
            !parse(&["kipi", "bind", "x", "--skill", "s"])
                .dry_run()
                .unwrap()
                .dry_run
        );
        assert!(parse(&["kipi", "resolve"]).dry_run().is_none());
        assert!(Cli::try_parse_from(["kipi", "resolve", "--dry-run"]).is_err());
    }

    #[test]
    fn global_store_precedes_the_subcommand_and_init_has_its_own() {
        let cli = Cli::try_parse_from(["kipi", "--store", "/a", "--store", "/b", "resolve"]);
        assert_eq!(
            cli.unwrap().global.stores,
            [PathBuf::from("/a"), PathBuf::from("/b")]
        );
        let cli = Cli::try_parse_from(["kipi", "init", "--store", "/s"]).unwrap();
        assert!(cli.global.stores.is_empty());
        assert!(matches!(cli.command, Command::Init { store: Some(_), .. }));
        let cli = Cli::try_parse_from(["kipi", "resolve", "--json", "-vv"]).unwrap();
        assert!(cli.global.json);
        assert_eq!(cli.global.verbose, 2);
        assert!(Cli::try_parse_from(["kipi", "-v", "-q", "resolve"]).is_err());
    }
}
