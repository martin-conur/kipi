//! The clap command tree: every §7 subcommand is declared, so `--help` documents the
//! whole v1 surface even before a command is implemented.

use std::path::PathBuf;

use clap::{ArgAction, Args, Parser, Subcommand, ValueHint};
use clap_complete::ArgValueCandidates;

use crate::complete;

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
    #[arg(long = "store", value_name = "PATH", value_hint = ValueHint::DirPath)]
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
        #[arg(long, value_name = "PATH", value_hint = ValueHint::DirPath)]
        store: Option<PathBuf>,
        /// The store's name (`[store] name`).
        #[arg(long)]
        name: Option<String>,
        /// Also `git init` the store.
        #[arg(long)]
        git: bool,
        /// Declare this scope in the store (see `kipi add scope`).
        #[arg(long, requires = "roots")]
        scope: Option<String>,
        /// The --scope's roots: directories holding its repos.
        #[arg(long = "root", value_name = "PATH", num_args = 1.., requires = "scope", value_hint = ValueHint::DirPath)]
        roots: Vec<PathBuf>,
        /// Declare the --scope without a registry.
        #[arg(long, requires = "scope")]
        no_registry: bool,
        #[command(flatten)]
        dry_run: DryRun,
    },
    /// List mounted stores, or unmount one.
    Stores {
        /// Remove this store (by name or path) from config.toml.
        #[arg(long, value_name = "NAME", add = ArgValueCandidates::new(complete::stores))]
        unmount: Option<String>,
        #[command(flatten)]
        dry_run: DryRun,
    },
    /// Find git repos under scope roots (or PATHs) and record them.
    Scan {
        /// Directories to walk instead of the scope roots.
        #[arg(value_hint = ValueHint::DirPath)]
        paths: Vec<PathBuf>,
        /// Rewrite values still marked `# kipi:draft`.
        #[arg(long)]
        refresh_drafts: bool,
        /// Store (by name) for checkouts outside every root; must own --scope.
        #[arg(long, value_name = "NAME", requires = "scope", add = ArgValueCandidates::new(complete::stores))]
        store: Option<String>,
        /// Scope for checkouts outside every root (written as `scope =`).
        #[arg(long, add = ArgValueCandidates::new(complete::scopes))]
        scope: Option<String>,
        #[command(flatten)]
        dry_run: DryRun,
    },
    /// Add a skill, a repo entry or a scope.
    #[command(subcommand)]
    Add(Add),
    /// Remove a repo entry.
    #[command(subcommand)]
    Remove(Remove),
    /// Change a scope: its registry, roots, parent, skills or always-on files.
    Scope(ScopeArgs),
    /// Add skills, relations, exclusions or aliases to a repo entry.
    Bind(Binding),
    /// Remove skills, relations, exclusions or aliases from a repo entry.
    Unbind(Binding),
    /// Print the resolved plan (no side effects).
    Resolve {
        /// Only this repo entry.
        #[arg(add = ArgValueCandidates::new(complete::repos))]
        repo: Option<String>,
    },
    /// Show where every decision for a repo comes from.
    Explain {
        /// The repo entry.
        #[arg(add = ArgValueCandidates::new(complete::repos))]
        repo: String,
    },
    /// Materialize the plan into repos (and their linked worktrees).
    Sync {
        /// Repo entries to sync (default: the repo or worktree you are in).
        #[arg(add = ArgValueCandidates::new(complete::repos))]
        repos: Vec<String>,
        /// Sync every repo cloned on this machine.
        #[arg(long, conflicts_with = "repos")]
        all: bool,
        /// Adapters to render instead of the plan's (comma-separated: claude, generic).
        #[arg(long, value_delimiter = ',')]
        agents: Vec<String>,
        /// Replace files kipi did not write.
        #[arg(long)]
        force: bool,
        /// Write the generated block into a tracked CLAUDE.md / AGENTS.md.
        #[arg(long)]
        allow_tracked: bool,
        /// Run by the post-checkout hook: do nothing outside a known repo.
        #[arg(long, hide = true, conflicts_with_all = ["repos", "all"])]
        hook: bool,
        #[command(flatten)]
        dry_run: DryRun,
    },
    /// Per-repo sync state and always-on tokens vs budget.
    Status {
        /// Only this repo entry.
        #[arg(add = ArgValueCandidates::new(complete::repos))]
        repo: Option<String>,
    },
    /// Stores → scopes → repos → skills, with provenance markers.
    Tree {
        /// Only this repo entry.
        #[arg(long, add = ArgValueCandidates::new(complete::repos))]
        repo: Option<String>,
        /// Only repos using this skill.
        #[arg(long, add = ArgValueCandidates::new(complete::skills))]
        skill: Option<String>,
        /// Only this store (by name).
        #[arg(long, value_name = "NAME", add = ArgValueCandidates::new(complete::stores))]
        store: Option<String>,
        /// Show each repo's always-on tokens against its budget.
        #[arg(long)]
        budget: bool,
        /// Show this machine's paths.
        #[arg(long)]
        machine: bool,
    },
    /// Resolve an alias to a repo, its summary and paths.
    Where {
        /// A repo name or alias.
        #[arg(add = ArgValueCandidates::new(complete::aliases))]
        alias: String,
        /// Print the repo's notes.
        #[arg(long)]
        notes: bool,
        /// Look in every mounted store, not just the current repo's scope chain.
        #[arg(long)]
        any_scope: bool,
    },
    /// Search skills, notes and conventions (and checkouts with --repos).
    Grep {
        /// A regular expression.
        pattern: String,
        /// Only this scope.
        #[arg(long, add = ArgValueCandidates::new(complete::scopes))]
        scope: Option<String>,
        /// Also search the checkouts of every repo in the scope chain.
        #[arg(long)]
        repos: bool,
        /// Print at most this many matching lines.
        #[arg(long, value_name = "N", default_value_t = kipi_core::search::DEFAULT_LIMIT)]
        limit: usize,
    },
    /// Open a repo entry, skill, scope:<name>, notes:<repo> or conventions:<scope> in $EDITOR.
    Edit {
        /// A repo entry, a skill, scope:<name>, notes:<repo> or conventions:<scope>.
        #[arg(add = ArgValueCandidates::new(complete::edit_targets))]
        target: String,
    },
    /// Check stores, repos and grants for problems.
    Doctor {
        /// Only this repo entry.
        #[arg(add = ArgValueCandidates::new(complete::repos))]
        repo: Option<String>,
        /// Apply the fixes marked safe.
        #[arg(long)]
        fix: bool,
        #[command(flatten)]
        dry_run: DryRun,
    },
    /// Install or remove the git hook that syncs each new worktree.
    #[command(subcommand)]
    Hook(Hook),
    /// Print shell completions.
    Completions {
        /// The shell to print the registration script for.
        #[arg(value_parser = complete::SHELL_NAMES)]
        shell: String,
    },
}

/// `kipi hook`.
#[derive(Debug, Subcommand)]
pub enum Hook {
    /// Install the post-checkout hook (an existing hook is kept and still runs).
    Install(HookTargets),
    /// Remove kipi's post-checkout hook, putting back the one it replaced.
    Uninstall(HookTargets),
}

/// The repos `kipi hook` acts on.
#[derive(Debug, Args)]
pub struct HookTargets {
    /// Repo entries.
    #[arg(add = ArgValueCandidates::new(complete::repos))]
    pub repos: Vec<String>,
    /// Every repo cloned on this machine.
    #[arg(long, conflicts_with = "repos")]
    pub all: bool,
    #[command(flatten)]
    pub dry_run: DryRun,
}

/// `kipi add`.
#[derive(Debug, Subcommand)]
pub enum Add {
    /// Create a skill from the template, or import an existing SKILL.md tree.
    Skill {
        /// The skill's directory name.
        name: String,
        /// Bind it to this scope.
        #[arg(long, add = ArgValueCandidates::new(complete::scopes))]
        scope: Option<String>,
        /// The store to create it in (by name).
        #[arg(long, value_name = "NAME", add = ArgValueCandidates::new(complete::stores))]
        store: Option<String>,
        /// A git URL or path to import from.
        #[arg(long, value_name = "GIT-URL|PATH")]
        from: Option<String>,
        /// Replace the --from directory with a symlink to the imported copy.
        #[arg(long, requires = "from")]
        link_back: bool,
        #[command(flatten)]
        dry_run: DryRun,
    },
    /// Register a repo by hand.
    Repo {
        /// The entry name.
        name: String,
        /// Its scope.
        #[arg(long, add = ArgValueCandidates::new(complete::scopes))]
        scope: Option<String>,
        /// Its checkout on this machine.
        #[arg(long, value_hint = ValueHint::DirPath)]
        path: Option<PathBuf>,
        /// Its one-line summary.
        #[arg(long)]
        summary: Option<String>,
        #[command(flatten)]
        dry_run: DryRun,
    },
    /// Declare a scope: the directories whose repos belong to it.
    Scope {
        /// The scope name.
        name: String,
        /// Directories holding the scope's repos.
        #[arg(long = "root", value_name = "PATH", num_args = 1.., required = true, value_hint = ValueHint::DirPath)]
        roots: Vec<PathBuf>,
        /// The parent scope, in the same store.
        #[arg(long, value_name = "SCOPE", add = ArgValueCandidates::new(complete::scopes))]
        extends: Option<String>,
        /// Build a registry for the scope (the default, unless a scope it extends has one).
        #[arg(long, conflicts_with = "no_registry")]
        registry: bool,
        /// Build no registry for the scope.
        #[arg(long)]
        no_registry: bool,
        /// Skills every repo in the scope gets.
        #[arg(long = "skill", value_name = "SKILL", num_args = 1.., add = ArgValueCandidates::new(complete::skills))]
        skills: Vec<String>,
        /// The store to declare it in (by name).
        #[arg(long, value_name = "NAME", add = ArgValueCandidates::new(complete::stores))]
        store: Option<String>,
        #[command(flatten)]
        dry_run: DryRun,
    },
}

/// `kipi remove`.
#[derive(Debug, Subcommand)]
pub enum Remove {
    /// Delete a repo entry, its machine.toml path and what kipi wrote into its checkout.
    Repo {
        /// The entry name.
        #[arg(add = ArgValueCandidates::new(complete::repos))]
        name: String,
        /// Also delete its notes.md in the store.
        #[arg(long)]
        notes: bool,
        #[command(flatten)]
        dry_run: DryRun,
    },
}

/// The arguments of `kipi scope`.
#[derive(Debug, Args)]
pub struct ScopeArgs {
    /// The scope.
    #[arg(add = ArgValueCandidates::new(complete::scopes))]
    pub name: String,
    /// Build a registry for the scope and render it into the block.
    #[arg(long, conflicts_with = "no_registry")]
    pub registry: bool,
    /// Stop building a registry for the scope.
    #[arg(long)]
    pub no_registry: bool,
    /// Add a directory holding the scope's repos.
    #[arg(long = "add-root", value_name = "PATH", num_args = 1.., value_hint = ValueHint::DirPath)]
    pub add_roots: Vec<PathBuf>,
    /// Remove a root.
    #[arg(long = "remove-root", value_name = "PATH", num_args = 1.., value_hint = ValueHint::DirPath)]
    pub remove_roots: Vec<PathBuf>,
    /// Set the parent scope, in the same store.
    #[arg(long, value_name = "SCOPE", conflicts_with = "no_extends", add = ArgValueCandidates::new(complete::scopes))]
    pub extends: Option<String>,
    /// Drop the parent scope.
    #[arg(long)]
    pub no_extends: bool,
    /// Add a skill every repo in the scope gets.
    #[arg(long = "skill", value_name = "SKILL", num_args = 1.., add = ArgValueCandidates::new(complete::skills))]
    pub skills: Vec<String>,
    /// Remove a skill.
    #[arg(long = "remove-skill", value_name = "SKILL", num_args = 1.., add = ArgValueCandidates::new(complete::skills))]
    pub remove_skills: Vec<String>,
    /// Add an always-on file (relative to the store root).
    #[arg(long, value_name = "FILE", num_args = 1..)]
    pub always: Vec<String>,
    /// Remove an always-on file.
    #[arg(long = "remove-always", value_name = "FILE", num_args = 1..)]
    pub remove_always: Vec<String>,
    #[command(flatten)]
    pub dry_run: DryRun,
}

/// The arguments of `bind` and `unbind`.
#[derive(Debug, Args)]
pub struct Binding {
    /// The repo entry.
    #[arg(add = ArgValueCandidates::new(complete::repos))]
    pub repo: String,
    /// Skills.
    #[arg(long = "skill", value_name = "SKILL", num_args = 1.., add = ArgValueCandidates::new(complete::skills))]
    pub skills: Vec<String>,
    /// Related repos.
    #[arg(long, value_name = "REPO", num_args = 1.., add = ArgValueCandidates::new(complete::repos))]
    pub related: Vec<String>,
    /// Excluded skills.
    #[arg(long, value_name = "SKILL", num_args = 1.., add = ArgValueCandidates::new(complete::skills))]
    pub exclude: Vec<String>,
    /// Aliases.
    #[arg(long = "alias", value_name = "ALIAS", num_args = 1..)]
    pub aliases: Vec<String>,
    #[command(flatten)]
    pub dry_run: DryRun,
}

impl Command {
    /// The command's `--dry-run`, for commands that write.
    pub fn dry_run(&self) -> Option<DryRun> {
        match self {
            Command::Init { dry_run, .. }
            | Command::Stores { dry_run, .. }
            | Command::Scan { dry_run, .. }
            | Command::Add(
                Add::Skill { dry_run, .. } | Add::Repo { dry_run, .. } | Add::Scope { dry_run, .. },
            )
            | Command::Remove(Remove::Repo { dry_run, .. })
            | Command::Sync { dry_run, .. }
            | Command::Doctor { dry_run, .. } => Some(*dry_run),
            Command::Scope(s) => Some(s.dry_run),
            Command::Bind(b) | Command::Unbind(b) => Some(b.dry_run),
            Command::Hook(Hook::Install(h) | Hook::Uninstall(h)) => Some(h.dry_run),
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
