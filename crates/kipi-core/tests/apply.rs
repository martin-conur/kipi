//! The apply engine (roadmap §8 steps 4–6, §13 `apply` row): integration tests in a
//! tempdir holding copies of both example stores and fake checkouts of every repo.
//! Idempotence, clean removal, non-destruction.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use kipi_core::apply::{self, Change, EXCLUDE_BEGIN, MANIFEST, Op, Options, Report, Target, What};
use kipi_core::config::{self, Env};
use kipi_core::manifest::Manifest;
use kipi_core::plan::{Action, Plan, repo_actions};
use kipi_core::render::block;
use kipi_core::resolve::resolve;
use kipi_test_support::builders::MachineBuilder;
use kipi_test_support::repos::{example_repos, lay_out};
use kipi_test_support::stores::{ExampleStore, copy_example_store_to};
use tempfile::TempDir;
use walkdir::WalkDir;

/// A simulated machine: the example stores and every example checkout under one home.
struct World {
    _tmp: TempDir,
    home: PathBuf,
    repos: Vec<(String, PathBuf)>,
}

impl World {
    fn new() -> World {
        let tmp = tempfile::tempdir().unwrap();
        // Canonical, so paths compare equal to what the OS reports (macOS /var → /private/var).
        let home = tmp.path().canonicalize().unwrap().join("home");
        for store in ExampleStore::ALL {
            copy_example_store_to(store, &home.join(store.dir_name())).unwrap();
        }
        let repos = lay_out(&home, &example_repos(&ExampleStore::ALL).unwrap()).unwrap();
        World {
            _tmp: tmp,
            home,
            repos,
        }
    }

    fn store(&self, store: ExampleStore) -> PathBuf {
        self.home.join(store.dir_name())
    }

    fn dir(&self, repo: &str) -> &Path {
        &self.repos.iter().find(|(e, _)| e == repo).unwrap().1
    }

    /// Replace `from` with `to` in a store's `bindings.toml`.
    fn edit_bindings(&self, store: ExampleStore, from: &str, to: &str) {
        let path = self.store(store).join("bindings.toml");
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains(from), "{from:?} not in bindings");
        fs::write(&path, text.replacen(from, to, 1)).unwrap();
    }

    fn plan(&self) -> Plan {
        let env = Env::from_lookup(|_| None, Some(self.home.clone())).unwrap();
        let paths: Vec<PathBuf> = ExampleStore::ALL.iter().map(|&s| self.store(s)).collect();
        let set = config::load_stores(&paths, &env).unwrap_or_else(|e| panic!("{}", e.report()));
        let machine = self
            .repos
            .iter()
            .fold(MachineBuilder::new(), |m, (e, p)| m.repo(e, p))
            .build();
        resolve(&set, &machine).unwrap_or_else(|e| panic!("{}", e.report()))
    }

    /// The actions sync wants in `repo`, with the checkout read from disk.
    fn actions(&self, plan: &Plan, repo: &str) -> Vec<Action> {
        let rp = plan.repo(repo).unwrap();
        let conventions = block::load_conventions(rp).unwrap();
        let block = block::render(plan, rp, &conventions, Some(&self.home));
        let checkout = apply::checkout(self.dir(repo), false).unwrap();
        repo_actions(rp, &block, &checkout)
            .unwrap()
            .actions()
            .cloned()
            .collect()
    }

    fn sync(&self, repo: &str, opts: Options) -> Report {
        let plan = self.plan();
        self.apply(repo, &self.actions(&plan, repo), opts)
    }

    fn apply(&self, repo: &str, actions: &[Action], opts: Options) -> Report {
        apply::run(&Target::repo(self.dir(repo)), actions, opts)
            .unwrap_or_else(|e| panic!("{}", e.report()))
    }

    fn sync_all(&self) -> Vec<Report> {
        let plan = self.plan();
        let mut reports: Vec<Report> = self
            .repos
            .iter()
            .map(|(name, _)| self.apply(name, &self.actions(&plan, name), Options::default()))
            .collect();
        for store in ExampleStore::ALL {
            reports.push(self.registries(&plan, store, Options::default()));
        }
        reports
    }

    fn registries(&self, plan: &Plan, store: ExampleStore, opts: Options) -> Report {
        let dir = self.store(store);
        let actions: Vec<Action> = plan
            .registries
            .iter()
            .filter(|r| r.store == store.name())
            .map(|r| apply::registry_action(&dir, r, Some(&self.home)))
            .collect();
        apply::run(&Target::generated(&dir), &actions, opts).unwrap()
    }
}

/// Everything under `dir` as path → contents (`-> target` for symlinks, `/` for
/// directories), skipping `.git` except its `info/exclude`.
fn tree(dir: &Path) -> BTreeMap<PathBuf, String> {
    let mut out = BTreeMap::new();
    let walk = WalkDir::new(dir)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| e.file_name() != ".git");
    for entry in walk {
        let entry = entry.unwrap();
        let rel = entry.path().strip_prefix(dir).unwrap().to_path_buf();
        let value = if entry.path_is_symlink() {
            format!("-> {}", fs::read_link(entry.path()).unwrap().display())
        } else if entry.file_type().is_dir() {
            "/".to_owned()
        } else {
            fs::read_to_string(entry.path()).unwrap()
        };
        out.insert(rel, value);
    }
    let exclude = dir.join(".git/info/exclude");
    out.insert(
        ".git/info/exclude".into(),
        fs::read_to_string(exclude).unwrap_or_default(),
    );
    out
}

fn changes(report: &Report, what: What, op: Op) -> Vec<PathBuf> {
    report
        .changes
        .iter()
        .filter(|c| c.what == what && c.op == op)
        .map(|c| c.path.clone())
        .collect()
}

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

#[test]
fn applying_the_same_plan_twice_changes_nothing_the_second_time() {
    let world = World::new();
    let first = world.sync_all();
    assert!(first.iter().all(|r| !r.is_unchanged()), "{first:#?}");
    assert!(first.iter().all(|r| r.skipped.is_empty()), "{first:#?}");

    let before: Vec<_> = world.repos.iter().map(|(_, d)| tree(d)).collect();
    let second = world.sync_all();
    for report in &second {
        assert!(report.is_unchanged(), "{:#?}", report.changes);
        assert!(report.skipped.is_empty(), "{:#?}", report.skipped);
    }
    let after: Vec<_> = world.repos.iter().map(|(_, d)| tree(d)).collect();
    assert_eq!(before, after);
}

#[test]
fn a_synced_repo_holds_blocks_links_excludes_and_a_manifest() {
    let world = World::new();
    world.sync_all();
    let dir = world.dir("billing-api");
    for file in ["CLAUDE.md", "AGENTS.md"] {
        let text = fs::read_to_string(dir.join(file)).unwrap();
        assert!(text.starts_with(block::BEGIN), "{file}: {text}");
    }
    for skills in [".claude/skills", ".agents/skills"] {
        let link = dir.join(skills).join("code-review");
        let target = fs::read_link(&link).unwrap();
        assert!(target.is_absolute());
        assert_eq!(
            target,
            world.store(ExampleStore::Acme).join("skills/code-review")
        );
        // The shared skill links into the personal store.
        let shared = fs::read_link(dir.join(skills).join("commit-style")).unwrap();
        assert!(shared.starts_with(world.store(ExampleStore::Personal)));
    }
    let exclude = fs::read_to_string(dir.join(".git/info/exclude")).unwrap();
    assert!(
        exclude.contains(&format!(
            "{EXCLUDE_BEGIN}\n.claude/skills/\n.claude/settings.local.json\n.agents/\n.kipi/\n"
        )),
        "{exclude}"
    );
    let manifest = Manifest::load(&dir.join(MANIFEST)).unwrap();
    assert_eq!(manifest.blocks.len(), 2);
    assert!(manifest.blocks.iter().all(|b| b.created));
    assert_eq!(
        manifest.dirs,
        [".claude", ".claude/skills", ".agents", ".agents/skills"].map(PathBuf::from)
    );
    // Registries land in each store's generated/ with their own manifest.
    let generated = world.store(ExampleStore::Acme).join("generated");
    assert!(generated.join("registry.acme.md").is_file());
    assert!(generated.join(apply::GENERATED_MANIFEST).is_file());
}

#[test]
fn dry_run_reports_the_changes_and_touches_nothing() {
    let world = World::new();
    let dir = world.dir("billing-api");
    let before = tree(dir);
    let dry = world.sync(
        "billing-api",
        Options {
            dry_run: true,
            ..Options::default()
        },
    );
    assert_eq!(tree(dir), before, "a dry run wrote something");
    assert!(!dir.join(MANIFEST).exists());
    let real = world.sync("billing-api", Options::default());
    assert_eq!(dry.changes, real.changes);

    // A dry run of the removal also matches the real removal.
    let dry = world.apply(
        "billing-api",
        &[],
        Options {
            dry_run: true,
            ..Options::default()
        },
    );
    let real = world.apply("billing-api", &[], Options::default());
    assert_eq!(dry.changes, real.changes);
}

#[test]
fn unbinding_a_skill_removes_exactly_its_links_and_foreign_entries_survive() {
    let world = World::new();
    world.sync_all();
    let dir = world.dir("events-pipeline").to_path_buf();
    // Foreign: a skill the user dropped in by hand, and a link of their own.
    let mine = dir.join(".claude/skills/my-own");
    fs::create_dir(&mine).unwrap();
    fs::write(mine.join("SKILL.md"), "mine").unwrap();
    std::os::unix::fs::symlink("/elsewhere", dir.join(".claude/skills/theirs")).unwrap();

    world.edit_bindings(ExampleStore::Acme, "skills = [\"stream-recipes\"]\n", "");
    let report = world.sync("events-pipeline", Options::default());
    assert_eq!(
        changes(&report, What::Link, Op::Remove),
        vec![dir.join(".claude/skills/stream-recipes")]
    );
    // The block does not list skills, so only the links went.
    assert!(
        report
            .changes
            .iter()
            .all(|c| c.op == Op::Remove && c.what == What::Link),
        "{:#?}",
        report.changes
    );
    assert_eq!(
        report.foreign,
        vec![mine.clone(), dir.join(".claude/skills/theirs")]
    );
    assert_eq!(fs::read_to_string(mine.join("SKILL.md")).unwrap(), "mine");
    assert!(fs::symlink_metadata(dir.join(".claude/skills/theirs")).is_ok());
    // The skill itself is intact in the store.
    assert!(
        world
            .store(ExampleStore::Acme)
            .join("skills/stream-recipes/SKILL.md")
            .is_file()
    );
}

#[test]
fn removing_everything_restores_the_checkout_and_keeps_user_content() {
    let world = World::new();
    let dir = world.dir("billing-api").to_path_buf();
    // User content: an AGENTS.md of their own and a note.
    fs::write(dir.join("AGENTS.md"), "# Our agents\n").unwrap();
    fs::write(dir.join("NOTES.txt"), "keep me").unwrap();
    let pristine = tree(&dir);

    world.sync("billing-api", Options::default());
    let synced = fs::read_to_string(dir.join("AGENTS.md")).unwrap();
    assert!(
        synced.starts_with("# Our agents\n\n<!-- kipi:begin"),
        "{synced}"
    );

    // With the user's AGENTS.md carrying the block, Claude Code reads it: no CLAUDE.md.
    assert!(!dir.join("CLAUDE.md").exists());

    // The repo is no longer bound: no desired actions at all.
    let report = world.apply("billing-api", &[], Options::default());
    assert!(report.skipped.is_empty(), "{:#?}", report.skipped);
    assert_eq!(
        changes(&report, What::Block, Op::Remove),
        vec![dir.join("AGENTS.md")]
    );
    assert_eq!(tree(&dir), pristine);
    assert!(!dir.join(".kipi").exists());

    let again = world.apply("billing-api", &[], Options::default());
    assert!(again.is_unchanged());
}

#[test]
fn a_regular_file_at_a_link_location_needs_force() {
    let world = World::new();
    let dir = world.dir("billing-api").to_path_buf();
    let link = dir.join(".claude/skills/code-review");
    fs::create_dir_all(link.parent().unwrap()).unwrap();
    fs::write(&link, "not a link").unwrap();

    let report = world.sync("billing-api", Options::default());
    let skipped: Vec<_> = report.skipped.iter().map(|s| &s.path).collect();
    assert_eq!(skipped, vec![&link]);
    assert!(report.skipped[0].hint.contains("--force"));
    assert_eq!(fs::read_to_string(&link).unwrap(), "not a link");
    // Everything else was still applied.
    assert!(dir.join(".agents/skills/code-review").is_symlink());

    let forced = world.sync(
        "billing-api",
        Options {
            force: true,
            ..Options::default()
        },
    );
    assert_eq!(
        changes(&forced, What::Link, Op::Replace),
        vec![link.clone()]
    );
    assert!(link.is_symlink());
    assert!(world.sync("billing-api", Options::default()).is_unchanged());
}

#[test]
fn a_directory_at_a_link_location_needs_force_too() {
    let world = World::new();
    let dir = world.dir("billing-api").to_path_buf();
    let link = dir.join(".agents/skills/code-review");
    fs::create_dir_all(&link).unwrap();
    fs::write(link.join("SKILL.md"), "local copy").unwrap();

    let report = world.sync("billing-api", Options::default());
    assert_eq!(report.skipped.len(), 1, "{:#?}", report.skipped);
    assert!(link.join("SKILL.md").is_file());

    world.sync(
        "billing-api",
        Options {
            force: true,
            ..Options::default()
        },
    );
    assert!(link.is_symlink());
}

#[test]
fn a_symlink_with_another_target_is_replaced() {
    let world = World::new();
    let dir = world.dir("billing-api").to_path_buf();
    let link = dir.join(".claude/skills/code-review");
    fs::create_dir_all(link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink("/somewhere/else", &link).unwrap();

    let report = world.sync("billing-api", Options::default());
    assert_eq!(
        changes(&report, What::Link, Op::Replace),
        vec![link.clone()]
    );
    assert_eq!(
        fs::read_link(&link).unwrap(),
        world.store(ExampleStore::Acme).join("skills/code-review")
    );
}

#[test]
fn deleting_a_symlink_to_a_directory_never_deletes_the_targets_contents() {
    let world = World::new();
    world.sync_all();
    let skill = world.store(ExampleStore::Acme).join("skills/code-review");
    let before = tree(&skill);
    assert!(before.contains_key(Path::new("SKILL.md")));
    // Unbind every repo that links the skill.
    for (name, _) in &world.repos {
        world.apply(name, &[], Options::default());
    }
    assert_eq!(tree(&skill), before);

    // And directly: a manifest-listed link to a directory full of files.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let target = root.join("target");
    fs::create_dir_all(target.join("nested")).unwrap();
    fs::write(target.join("nested/file.txt"), "precious").unwrap();
    let repo = root.join("repo");
    fs::create_dir(&repo).unwrap();
    let link = repo.join("link");
    let t = Target::repo(&repo);
    apply::run(
        &t,
        &[Action::Link {
            link: link.clone(),
            target: target.clone(),
        }],
        Options::default(),
    )
    .unwrap();
    assert!(link.join("nested/file.txt").is_file());
    let report = apply::run(&t, &[], Options::default()).unwrap();
    assert_eq!(
        report.changes,
        vec![Change {
            op: Op::Remove,
            what: What::Link,
            path: link.clone(),
        }]
    );
    assert!(fs::symlink_metadata(&link).is_err());
    assert_eq!(
        fs::read_to_string(target.join("nested/file.txt")).unwrap(),
        "precious"
    );
}

#[test]
fn a_link_the_user_changed_is_left_alone() {
    let world = World::new();
    world.sync("billing-api", Options::default());
    let dir = world.dir("billing-api").to_path_buf();
    let link = dir.join(".claude/skills/code-review");
    fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink("/mine", &link).unwrap();

    let report = world.apply("billing-api", &[], Options::default());
    assert!(report.skipped.iter().any(|s| s.path == link));
    assert_eq!(fs::read_link(&link).unwrap(), PathBuf::from("/mine"));
    // `.claude/skills` still holds it, so kipi keeps the directory (and its record).
    assert!(link.parent().unwrap().is_dir());
}

#[test]
fn a_tracked_claude_md_sends_the_block_to_claude_local_md() {
    let world = World::new();
    let dir = world.dir("billing-api").to_path_buf();
    fs::write(dir.join("CLAUDE.md"), "# Team rules\n").unwrap();
    git(&dir, &["add", "CLAUDE.md"]);

    let checkout = apply::checkout(&dir, false).unwrap();
    assert!(checkout.tracked.contains(&dir.join("CLAUDE.md")));
    assert!(!checkout.tracked.contains(&dir.join("AGENTS.md")));
    assert!(checkout.user_files.contains(&dir.join("CLAUDE.md")));
    assert!(!checkout.allow_tracked);
    assert!(apply::checkout(&dir, true).unwrap().allow_tracked);

    let report = world.sync("billing-api", Options::default());
    assert!(report.skipped.is_empty(), "{:#?}", report.skipped);
    assert_eq!(
        fs::read_to_string(dir.join("CLAUDE.md")).unwrap(),
        "# Team rules\n"
    );
    let local = fs::read_to_string(dir.join("CLAUDE.local.md")).unwrap();
    assert!(local.starts_with(block::BEGIN));
    let exclude = fs::read_to_string(dir.join(".git/info/exclude")).unwrap();
    assert!(exclude.contains("\nCLAUDE.local.md\n"), "{exclude}");
    assert!(world.sync("billing-api", Options::default()).is_unchanged());
}

/// The #33 transition: with a user AGENTS.md and only Claude enabled, kipi creates a
/// CLAUDE.md importing it; enabling the generic adapter moves the block into AGENTS.md
/// and the kipi-created CLAUDE.md goes away through the manifest.
#[test]
fn enabling_generic_removes_the_kipi_created_claude_md() {
    let world = World::new();
    let dir = world.dir("events-pipeline").to_path_buf();
    fs::write(dir.join("AGENTS.md"), "# Ours\n").unwrap();

    world.sync("events-pipeline", Options::default());
    let claude = fs::read_to_string(dir.join("CLAUDE.md")).unwrap();
    assert!(claude.contains("@AGENTS.md"), "{claude}");
    assert_eq!(
        fs::read_to_string(dir.join("AGENTS.md")).unwrap(),
        "# Ours\n"
    );
    let checkout = apply::checkout(&dir, false).unwrap();
    assert!(!checkout.user_files.contains(&dir.join("CLAUDE.md")));
    assert!(checkout.user_files.contains(&dir.join("AGENTS.md")));

    world.edit_bindings(
        ExampleStore::Acme,
        "agents = [\"claude\"]                # per-repo override",
        "agents = [\"claude\", \"generic\"]",
    );
    let report = world.sync("events-pipeline", Options::default());
    assert_eq!(
        changes(&report, What::File, Op::Remove),
        vec![dir.join("CLAUDE.md")]
    );
    assert!(!dir.join("CLAUDE.md").exists());
    let agents = fs::read_to_string(dir.join("AGENTS.md")).unwrap();
    assert!(agents.starts_with("# Ours\n\n<!-- kipi:begin"), "{agents}");
    assert!(
        world
            .sync("events-pipeline", Options::default())
            .is_unchanged()
    );
}

/// The same transition after the user wrote into the kipi-created CLAUDE.md: it is
/// theirs now, so it keeps a block (without the import) and is never deleted; when
/// Claude is dropped altogether only the block goes.
#[test]
fn a_kipi_created_file_the_user_edited_is_never_deleted() {
    let world = World::new();
    let dir = world.dir("events-pipeline").to_path_buf();
    fs::write(dir.join("AGENTS.md"), "# Ours\n").unwrap();
    world.sync("events-pipeline", Options::default());
    let claude = dir.join("CLAUDE.md");
    let text = fs::read_to_string(&claude).unwrap();
    fs::write(&claude, format!("# My notes\n\n{text}")).unwrap();
    assert!(
        apply::checkout(&dir, false)
            .unwrap()
            .user_files
            .contains(&claude)
    );

    world.edit_bindings(
        ExampleStore::Acme,
        "agents = [\"claude\"]                # per-repo override",
        "agents = [\"claude\", \"generic\"]  # enabled",
    );
    world.sync("events-pipeline", Options::default());
    let text = fs::read_to_string(&claude).unwrap();
    assert!(text.starts_with("# My notes\n\n<!-- kipi:begin"), "{text}");
    assert!(!text.contains("@AGENTS.md"), "{text}");

    world.edit_bindings(
        ExampleStore::Acme,
        "agents = [\"claude\", \"generic\"]  # enabled",
        "agents = [\"generic\"]",
    );
    let report = world.sync("events-pipeline", Options::default());
    assert_eq!(
        changes(&report, What::Block, Op::Remove),
        vec![claude.clone()]
    );
    assert_eq!(fs::read_to_string(&claude).unwrap(), "# My notes\n");
}

#[test]
fn a_symlinked_always_on_file_is_not_written_through() {
    let world = World::new();
    let dir = world.dir("billing-api").to_path_buf();
    fs::write(dir.join("AGENTS.md"), "# Shared\n").unwrap();
    std::os::unix::fs::symlink("AGENTS.md", dir.join("CLAUDE.md")).unwrap();

    let report = world.sync("billing-api", Options::default());
    let skipped: Vec<_> = report.skipped.iter().map(|s| s.path.clone()).collect();
    assert_eq!(skipped, vec![dir.join("CLAUDE.md")]);
    assert!(dir.join("CLAUDE.md").is_symlink());
    // AGENTS.md got its block from the generic adapter.
    assert!(
        fs::read_to_string(dir.join("AGENTS.md"))
            .unwrap()
            .contains(block::BEGIN)
    );
}

#[test]
fn registries_are_written_once_and_removed_when_no_longer_wanted() {
    let world = World::new();
    let plan = world.plan();
    let acme = world.store(ExampleStore::Acme);
    let generated = acme.join("generated");
    let first = world.registries(&plan, ExampleStore::Acme, Options::default());
    assert_eq!(
        changes(&first, What::File, Op::Create),
        vec![generated.join("registry.acme.md")]
    );
    let text = fs::read_to_string(generated.join("registry.acme.md")).unwrap();
    assert!(text.contains("billing-api"), "{text}");
    let again = world.registries(&plan, ExampleStore::Acme, Options::default());
    assert!(again.is_unchanged());

    world.edit_bindings(ExampleStore::Acme, "registry = true\n", "");
    let plan = world.plan();
    let gone = world.registries(&plan, ExampleStore::Acme, Options::default());
    assert_eq!(
        changes(&gone, What::File, Op::Remove),
        vec![generated.join("registry.acme.md")]
    );
    assert!(!generated.join(apply::GENERATED_MANIFEST).exists());
}

#[test]
fn a_registry_file_kipi_did_not_write_needs_force() {
    let world = World::new();
    let plan = world.plan();
    let generated = world.store(ExampleStore::Acme).join("generated");
    fs::create_dir_all(&generated).unwrap();
    fs::write(generated.join("registry.acme.md"), "hand-written").unwrap();
    let report = world.registries(&plan, ExampleStore::Acme, Options::default());
    assert_eq!(report.skipped.len(), 1);
    assert_eq!(
        fs::read_to_string(generated.join("registry.acme.md")).unwrap(),
        "hand-written"
    );
    let forced = world.registries(
        &plan,
        ExampleStore::Acme,
        Options {
            force: true,
            ..Options::default()
        },
    );
    assert_eq!(forced.changes.len(), 1, "{:#?}", forced.changes);
}

#[test]
fn nothing_is_written_through_a_symlinked_directory() {
    let world = World::new();
    let dir = world.dir("billing-api").to_path_buf();
    let elsewhere = world.home.join("elsewhere");
    fs::create_dir(&elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, dir.join(".agents")).unwrap();

    let report = world.sync("billing-api", Options::default());
    assert!(
        report
            .skipped
            .iter()
            .all(|s| s.path.starts_with(dir.join(".agents"))),
        "{:#?}",
        report.skipped
    );
    assert!(!report.skipped.is_empty());
    assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
}

#[test]
fn actions_outside_the_target_are_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    let err = apply::run(
        &Target::repo(&root),
        &[Action::WriteFile {
            path: tmp.path().join("outside.md"),
            text: String::new(),
        }],
        Options::default(),
    )
    .unwrap_err();
    assert!(err.to_string().contains("outside the target"), "{err}");
}

#[test]
fn removal_never_reaches_through_a_directory_turned_symlink() {
    let world = World::new();
    world.sync("billing-api", Options::default());
    let dir = world.dir("billing-api").to_path_buf();
    // The user moves `.claude` elsewhere and links it back: the skill links now live in
    // their directory, still pointing where kipi recorded.
    let moved = world.home.join("claude-config");
    fs::rename(dir.join(".claude"), &moved).unwrap();
    std::os::unix::fs::symlink(&moved, dir.join(".claude")).unwrap();

    world.apply("billing-api", &[], Options::default());
    assert!(moved.join("skills/code-review").is_symlink());
    assert!(dir.join(".claude").is_symlink());
    assert!(!dir.join(".agents").exists());
}
