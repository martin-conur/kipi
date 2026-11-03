//! Sync commands (`on_sync`) end to end in temporary directories: a store on disk, fake
//! git checkouts, `resolve` then `sync::run`, and what ran, what was recorded in
//! `.kipi/on_sync.toml` and what was left behind.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use kipi_core::cache::Caches;
use kipi_core::config::{self, Env};
use kipi_core::model::{Machine, StoreSet};
use kipi_core::on_sync::State;
use kipi_core::plan::{Plan, SkipReason};
use kipi_core::resolve::resolve;
use kipi_core::sync::{self, CommandState, Outcome, RepoOutcome, Selection};
use kipi_core::tokens::Tokenizer;
use kipi_test_support::builders::MachineBuilder;
use kipi_test_support::repos::{FakeRepo, ManifestKind, create_fake_repo};
use tempfile::TempDir;

/// Writes `$1` (the attribute) to `out.txt` in the working tree, as one argument.
const WRITE_BOARD: &str =
    r#"["sh", "-c", "printf '%s' \"$1\" > out.txt", "x", "{attr.tracker.board}"]"#;

struct Fx {
    dir: TempDir,
    /// Extra approval keys, beyond what [`Fx::approve_all`] approved.
    approved: Vec<String>,
}

impl Fx {
    /// A store `work` whose `[repos]` are `repos` (TOML after the `[store]` table), and a
    /// fake checkout under `code/` for each name in `checkouts`.
    fn new(repos: &str, checkouts: &[&str]) -> Fx {
        let fx = Fx {
            dir: tempfile::tempdir().unwrap(),
            approved: Vec::new(),
        };
        fs::create_dir_all(fx.store()).unwrap();
        fx.bindings(repos);
        for name in checkouts {
            let repo = FakeRepo {
                entry: (*name).to_owned(),
                dir: PathBuf::from(name),
                origin_url: None,
                readme: Some(format!("# {name}\n")),
                manifest: ManifestKind::None,
                description: None,
            };
            create_fake_repo(&fx.checkout(name), &repo).unwrap();
        }
        fx
    }

    fn store(&self) -> PathBuf {
        self.dir.path().join("store")
    }

    fn checkout(&self, name: &str) -> PathBuf {
        self.dir.path().join("code").join(name)
    }

    fn bindings(&self, repos: &str) {
        let text = format!("[store]\nname = \"work\"\nversion = 1\n\n{repos}");
        fs::write(self.store().join("bindings.toml"), text).unwrap();
    }

    fn env(&self) -> Env {
        Env::from_lookup(|_| None, Some(self.dir.path().to_path_buf())).unwrap()
    }

    fn stores(&self) -> StoreSet {
        let store = config::load_store(&self.store(), &self.env())
            .unwrap_or_else(|e| panic!("{}", e.report()));
        StoreSet::new(vec![store]).unwrap()
    }

    /// Every checkout under `code/` recorded, plus the approvals so far.
    fn machine(&self) -> Machine {
        let mut m = MachineBuilder::new();
        for entry in fs::read_dir(self.dir.path().join("code")).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            m = m.repo(&name, path);
        }
        for key in &self.approved {
            m = m.approve(key.clone());
        }
        m.build()
    }

    fn plan(&self) -> Plan {
        resolve(&self.stores(), &self.machine()).unwrap_or_else(|e| panic!("{}", e.report()))
    }

    /// Approve every command the current bindings plan.
    fn approve_all(&mut self) {
        let keys: Vec<String> = self
            .plan()
            .repos
            .iter()
            .flat_map(|r| r.on_sync.iter().map(|c| c.approval.clone()))
            .collect();
        self.approved.extend(keys);
    }

    fn run(&self, selection: &Selection, dry_run: bool, force: bool) -> Outcome {
        let opts = sync::Options {
            apply: kipi_core::apply::Options { dry_run, force },
            worktrees: true,
            run_commands: true,
            ..sync::Options::default()
        };
        let caches = Caches::in_memory(Tokenizer::default());
        let (set, machine) = (self.stores(), self.machine());
        let plan = resolve(&set, &machine).unwrap_or_else(|e| panic!("{}", e.report()));
        sync::run(&set, &machine, &plan, selection, &opts, &caches)
            .unwrap_or_else(|e| panic!("{}", e.report()))
    }

    fn sync(&self) -> Outcome {
        self.run(&Selection::All, false, false)
    }

    fn out(&self, tree: &Path) -> Option<String> {
        fs::read_to_string(tree.join("out.txt")).ok()
    }
}

/// The working tree of `repo` at `path` in `outcome`.
fn tree<'o>(outcome: &'o Outcome, path: &Path) -> &'o RepoOutcome {
    let real = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    outcome
        .repos
        .iter()
        .find(|r| r.path.as_deref().map(real) == Some(real(path)))
        .unwrap_or_else(|| panic!("no outcome for {}", path.display()))
}

fn states(r: &RepoOutcome) -> Vec<(&str, &CommandState)> {
    r.commands
        .iter()
        .map(|c| (c.name.as_str(), &c.state))
        .collect()
}

const RAN: CommandState = CommandState::Ran {
    exit: Some(0),
    ok: true,
};

fn board_repo(board: &str, extra: &str) -> String {
    format!(
        "[repos.app]\nattributes = {{ \"tracker.board\" = {board:?} }}\n\n\
         [[repos.app.on_sync]]\nname = \"board\"\nrun = {WRITE_BOARD}\n{extra}"
    )
}

#[test]
fn an_unapproved_command_does_not_run_until_approved() {
    let mut fx = Fx::new(&board_repo("B", ""), &["app"]);
    let app = fx.checkout("app");
    let first = fx.sync();
    assert_eq!(
        states(tree(&first, &app)),
        [("board", &CommandState::Unapproved)]
    );
    assert_eq!(fx.out(&app), None);
    fx.approve_all();
    assert_eq!(states(tree(&fx.sync(), &app)), [("board", &RAN)]);
    assert_eq!(fx.out(&app).as_deref(), Some("B"));
}

#[test]
fn runs_once_with_the_value_as_one_argument_then_is_idempotent() {
    let value = "Payments; rm -rf / 'quoted' \"$HOME\" $(id)";
    let mut fx = Fx::new(&board_repo(value, ""), &["app"]);
    fx.approve_all();
    let app = fx.checkout("app");
    assert_eq!(states(tree(&fx.sync(), &app)), [("board", &RAN)]);
    assert_eq!(fx.out(&app).as_deref(), Some(value));
    let state = State::load(&app).unwrap();
    assert_eq!(state.runs.len(), 1);
    assert!(state.runs[0].ok);
    assert_eq!(state.runs[0].exit, Some(0));

    // A second sync is a no-op: the command does not run again.
    fs::remove_file(app.join("out.txt")).unwrap();
    assert_eq!(
        states(tree(&fx.sync(), &app)),
        [("board", &CommandState::UpToDate)]
    );
    assert_eq!(fx.out(&app), None);

    // `--force` reruns it.
    let forced = fx.run(&Selection::All, false, true);
    assert_eq!(states(tree(&forced, &app)), [("board", &RAN)]);
    assert_eq!(fx.out(&app).as_deref(), Some(value));
}

#[test]
fn a_changed_value_reruns_without_approval_but_a_changed_run_needs_it() {
    let mut fx = Fx::new(&board_repo("One", ""), &["app"]);
    fx.approve_all();
    let app = fx.checkout("app");
    fx.sync();
    assert_eq!(fx.out(&app).as_deref(), Some("One"));

    fx.bindings(&board_repo("Two", ""));
    assert_eq!(states(tree(&fx.sync(), &app)), [("board", &RAN)]);
    assert_eq!(fx.out(&app).as_deref(), Some("Two"));

    let changed = board_repo("Two", "").replace("printf '%s'", "printf '%s!'");
    fx.bindings(&changed);
    assert_eq!(
        states(tree(&fx.sync(), &app)),
        [("board", &CommandState::Unapproved)]
    );
    assert_eq!(fx.out(&app).as_deref(), Some("Two"));
}

#[test]
fn a_missing_attribute_skips_with_a_reason_and_when_skips_silently() {
    let repos = format!(
        "[repos.app]\n\n\
         [[repos.app.on_sync]]\nname = \"board\"\nrun = {WRITE_BOARD}\n\n\
         [[repos.app.on_sync]]\nname = \"gated\"\nrun = {WRITE_BOARD}\nwhen = \"attr.tracker.board\"\n"
    );
    let mut fx = Fx::new(&repos, &["app"]);
    fx.approve_all();
    let app = fx.checkout("app");
    let outcome = fx.sync();
    assert_eq!(
        states(tree(&outcome, &app)),
        [
            (
                "board",
                &CommandState::Skipped {
                    reason: SkipReason::MissingAttribute,
                    key: "tracker.board".into()
                }
            ),
            (
                "gated",
                &CommandState::Skipped {
                    reason: SkipReason::When,
                    key: "tracker.board".into()
                }
            ),
        ]
    );
    assert_eq!(fx.out(&app), None);
    assert!(!State::path(&app).exists(), "nothing ran, nothing recorded");
}

#[test]
fn a_failure_is_recorded_per_repo_and_retried() {
    let repos = format!(
        "{}\n[repos.bad]\n\n[[repos.bad.on_sync]]\nname = \"fails\"\nrun = \"false\"\n\n\
         [[repos.bad.on_sync]]\nname = \"missing\"\nrun = \"kipi-no-such-program --x\"\n",
        board_repo("B", "")
    );
    let mut fx = Fx::new(&repos, &["app", "bad"]);
    fx.approve_all();
    let (app, bad) = (fx.checkout("app"), fx.checkout("bad"));
    let outcome = fx.sync();
    assert_eq!(states(tree(&outcome, &app)), [("board", &RAN)]);
    let failed = states(tree(&outcome, &bad));
    assert_eq!(
        failed[0],
        (
            "fails",
            &CommandState::Ran {
                exit: Some(1),
                ok: false
            }
        )
    );
    assert!(
        matches!(failed[1], ("missing", CommandState::SpawnFailed(why)) if why.contains("kipi-no-such-program"))
    );
    let state = State::load(&bad).unwrap();
    let rec = state.get("fails").unwrap();
    assert_eq!((rec.ok, rec.exit), (false, Some(1)));
    let rec = state.get("missing").unwrap();
    assert_eq!((rec.ok, rec.exit), (false, None));

    // The next sync retries the failed ones and leaves the good one alone.
    let again = fx.sync();
    assert_eq!(
        states(tree(&again, &app)),
        [("board", &CommandState::UpToDate)]
    );
    assert!(states(tree(&again, &bad)).iter().all(|(_, s)| s.failed()));
}

#[test]
fn a_dry_run_reports_and_runs_nothing() {
    let mut fx = Fx::new(&board_repo("B", ""), &["app"]);
    fx.approve_all();
    let app = fx.checkout("app");
    let dry = fx.run(&Selection::All, true, false);
    assert_eq!(
        states(tree(&dry, &app)),
        [("board", &CommandState::WouldRun)]
    );
    assert_eq!(fx.out(&app), None);
    assert!(!app.join(".kipi").exists());

    // A run that does not execute commands (status, doctor) reports them the same way.
    let opts = sync::Options {
        worktrees: true,
        ..sync::Options::default()
    };
    let caches = Caches::in_memory(Tokenizer::default());
    let (set, machine) = (fx.stores(), fx.machine());
    let plan = resolve(&set, &machine).unwrap();
    let synced = sync::run(&set, &machine, &plan, &Selection::All, &opts, &caches).unwrap();
    assert_eq!(
        states(tree(&synced, &app)),
        [("board", &CommandState::WouldRun)]
    );
    assert_eq!(fx.out(&app), None);
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(["-c", "user.name=kipi", "-c", "user.email=kipi@example.com"])
        .args(args)
        .current_dir(dir)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

/// `app`'s checkout with one linked worktree, `app-wt` (outside `code/`).
fn with_worktree(fx: &Fx) -> PathBuf {
    let app = fx.checkout("app");
    git(&app, &["commit", "--quiet", "--allow-empty", "-m", "init"]);
    let wt = fx.dir.path().join("app-wt");
    git(&app, &["worktree", "add", "--quiet", wt.to_str().unwrap()]);
    wt
}

#[test]
fn linked_worktrees_run_only_commands_that_ask_for_it() {
    let mut fx = Fx::new(&board_repo("B", ""), &["app"]);
    fx.approve_all();
    let wt = with_worktree(&fx);
    let app = fx.checkout("app");
    let outcome = fx.sync();
    assert_eq!(states(tree(&outcome, &app)), [("board", &RAN)]);
    assert!(states(tree(&outcome, &wt)).is_empty());
    assert_eq!(fx.out(&wt), None);

    fx.bindings(&board_repo("B", "worktrees = true\n"));
    fx.approve_all();
    let outcome = fx.sync();
    // `worktrees` is part of the approval, so the checkout runs it again too.
    assert_eq!(states(tree(&outcome, &wt)), [("board", &RAN)]);
    assert_eq!(fx.out(&wt).as_deref(), Some("B"));

    // `sync --hook` from the worktree: just that tree.
    fs::remove_file(wt.join("out.txt")).unwrap();
    let hook = fx.run(
        &Selection::Tree {
            repo: "app".into(),
            dir: wt.clone(),
        },
        false,
        true,
    );
    assert_eq!(hook.repos.len(), 1);
    assert_eq!(states(&hook.repos[0]), [("board", &RAN)]);
    assert_eq!(fx.out(&wt).as_deref(), Some("B"));
}

#[test]
fn dropped_commands_and_unbound_repos_leave_nothing_behind() {
    let mut fx = Fx::new(&board_repo("B", ""), &["app"]);
    fx.approve_all();
    let app = fx.checkout("app");
    fx.sync();
    assert!(State::path(&app).exists());

    // The command goes away: its record goes too, the manifest stays.
    fx.bindings("[repos.app]\n");
    let outcome = fx.sync();
    assert!(tree(&outcome, &app).commands.is_empty());
    assert!(!State::path(&app).exists());
    assert!(app.join(".kipi/manifest.toml").exists());

    // Put it back and run it, then unbind the repo: no `.kipi/` is left.
    fx.bindings(&board_repo("B", ""));
    fx.sync();
    assert!(State::path(&app).exists());
    fx.bindings("");
    fx.sync();
    assert!(
        !app.join(".kipi").exists(),
        "{:?}",
        fs::read_dir(&app).unwrap().collect::<Vec<_>>()
    );
}
