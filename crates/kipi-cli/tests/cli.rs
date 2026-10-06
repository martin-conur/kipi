//! End to end: the `kipi` binary against the example stores, mounted in a temporary
//! home with the example `machine.toml`. Output is snapshotted with the home replaced
//! by `[HOME]`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use kipi_test_support::many::many_repos;
use kipi_test_support::repos::{FakeRepo, ManifestKind, example_repos, lay_out};
use kipi_test_support::stores::{ExampleStore, copy_example_store_to};
use kipi_test_support::workspace_root;
use predicates::prelude::*;
use tempfile::TempDir;
use walkdir::WalkDir;

/// Every repo entry of the example stores.
const EXAMPLE_REPOS: [&str; 10] = [
    "acme-api",
    "api",
    "billing-api",
    "blog",
    "dotfiles",
    "events-pipeline",
    "feature-store",
    "ml-platform",
    "side-tool",
    "web-dashboard",
];

/// A home directory holding copies of the example stores under `stores/` and the
/// example `machine.toml` under `.config/kipi/`.
struct Home {
    dir: TempDir,
}

impl Home {
    /// A home with nothing in it.
    fn empty() -> Self {
        Home {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        for store in ExampleStore::ALL {
            copy_example_store_to(store, &dir.path().join("stores").join(store.dir_name()))
                .unwrap();
        }
        let config = dir.path().join(".config/kipi");
        fs::create_dir_all(&config).unwrap();
        fs::copy(
            workspace_root().join("examples/machine.toml"),
            config.join("machine.toml"),
        )
        .unwrap();
        Home { dir }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn store(&self, store: ExampleStore) -> PathBuf {
        self.path().join("stores").join(store.dir_name())
    }

    /// `kipi` with this home and no store mounted (no `--store`, no config.toml).
    fn bare(&self) -> Command {
        let mut cmd = Command::cargo_bin("kipi").expect("kipi binary is built");
        cmd.env("HOME", self.path())
            .env_remove("KIPI_CONFIG_DIR")
            .env_remove("KIPI_CACHE_DIR")
            .env_remove("KIPI_STORE")
            .env_remove("NO_COLOR");
        cmd
    }

    /// `kipi --store <personal> --store <acme>`.
    fn kipi(&self) -> Command {
        self.mounting(&ExampleStore::ALL)
    }

    fn mounting(&self, stores: &[ExampleStore]) -> Command {
        let mut cmd = self.bare();
        for &store in stores {
            cmd.arg("--store").arg(self.store(store));
        }
        cmd
    }

    /// `text` with this home shown as `[HOME]`.
    fn scrub(&self, text: &[u8]) -> String {
        String::from_utf8(text.to_vec())
            .unwrap()
            .replace(&self.path().display().to_string(), "[HOME]")
    }
}

fn stdout_of(home: &Home, cmd: &mut Command) -> String {
    let out = cmd.assert().success().get_output().stdout.clone();
    home.scrub(&out)
}

/// `(exit code, scrubbed stderr)` of a failing run.
fn failure_of(home: &Home, cmd: &mut Command) -> (i32, String) {
    let out = cmd.assert().failure().get_output().clone();
    assert!(out.stdout.is_empty(), "nothing on stdout when failing");
    (out.status.code().unwrap(), home.scrub(&out.stderr))
}

#[test]
fn version_prints_crate_version() {
    Home::new()
        .bare()
        .arg("--version")
        .assert()
        .success()
        .stdout(format!("kipi {}\n", env!("CARGO_PKG_VERSION")));
}

#[test]
fn help_documents_the_whole_v1_surface() {
    let home = Home::new();
    let help = stdout_of(&home, home.bare().arg("--help"));
    insta::assert_snapshot!("help", help);
    for command in [
        "init",
        "stores",
        "scan",
        "add",
        "bind",
        "unbind",
        "resolve",
        "explain",
        "sync",
        "status",
        "tree",
        "where",
        "grep",
        "edit",
        "doctor",
        "completions",
    ] {
        assert!(
            help.contains(&format!("\n  {command} ")),
            "{command} in --help"
        );
    }
}

#[test]
fn explain_every_example_repo() {
    let home = Home::new();
    for repo in EXAMPLE_REPOS {
        let out = stdout_of(&home, home.kipi().args(["explain", repo]));
        insta::assert_snapshot!(format!("explain-{repo}"), out);
    }
}

#[test]
fn explain_events_pipeline_shows_every_decision() {
    let home = Home::new();
    let out = stdout_of(&home, home.kipi().args(["explain", "events-pipeline"]));
    for expected in [
        "acme-ml  ← repos.events-pipeline.scope (acme:bindings.toml:34)",
        "chain: acme → acme-ml",
        "○ code-review            ← scopes.acme.skills (acme:bindings.toml:13)",
        "● stream-recipes         ← repos.events-pipeline.skills (acme:bindings.toml:34)",
        "✗ pr-conventions         ← repos.events-pipeline.exclude (acme:bindings.toml:34)",
        "6 repos in scopes acme, acme-ml",
        "billing-api  acme     /Volumes/work/acme/billing-api",
        "Grant: registry  ← repos.events-pipeline.grant (acme:bindings.toml:34)",
        "~/stores/personal-store/skills",
        "budget_tokens  4000      ← defaults.budget_tokens (acme:bindings.toml:8)",
    ] {
        assert!(out.contains(expected), "missing {expected:?} in\n{out}");
    }
}

#[test]
fn explain_works_with_the_personal_store_alone() {
    let home = Home::new();
    let out = stdout_of(
        &home,
        home.mounting(&[ExampleStore::Personal])
            .args(["explain", "side-tool"]),
    );
    insta::assert_snapshot!("explain-personal-alone-side-tool", out);
    assert!(!out.contains("acme"), "no work content: {out}");
}

#[test]
fn resolve_prints_the_plan() {
    let home = Home::new();
    insta::assert_snapshot!("resolve", stdout_of(&home, home.kipi().arg("resolve")));
    insta::assert_snapshot!(
        "resolve-side-tool",
        stdout_of(&home, home.kipi().args(["resolve", "side-tool"]))
    );
}

#[test]
fn resolve_json_is_valid_and_stable() {
    let home = Home::new();
    let out = stdout_of(&home, home.kipi().args(["resolve", "--json"]));
    let doc: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
    assert_eq!(doc["version"], 1);
    assert_eq!(doc["repos"].as_array().unwrap().len(), EXAMPLE_REPOS.len());
    let names: Vec<&str> = doc["repos"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, EXAMPLE_REPOS, "sorted by name");
    insta::assert_snapshot!("resolve-json", out);

    let again = stdout_of(&home, home.kipi().args(["resolve", "--json"]));
    assert_eq!(out, again, "deterministic");
}

#[test]
fn resolve_json_for_one_repo_keeps_the_shape() {
    let home = Home::new();
    let out = stdout_of(
        &home,
        home.kipi().args(["resolve", "events-pipeline", "--json"]),
    );
    let doc: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(doc["repos"].as_array().unwrap().len(), 1);
    assert_eq!(doc["registries"][0]["scope"], "acme");
    let repo = &doc["repos"][0];
    assert_eq!(repo["scope"]["value"], "acme-ml");
    assert_eq!(
        repo["scope"]["source"]["at"], "acme:bindings.toml:34",
        "provenance is store:file:line"
    );
    assert_eq!(repo["skills"][0]["inherited_from"], "acme");
    assert_eq!(repo["skills"][3]["inherited_from"], serde_json::Value::Null);
    assert_eq!(repo["excluded"][0]["name"], "pr-conventions");
    assert_eq!(repo["grant"]["mode"]["value"], "registry");

    let explain = stdout_of(
        &home,
        home.kipi().args(["explain", "events-pipeline", "--json"]),
    );
    assert_eq!(explain, out, "explain --json is resolve <repo> --json");
}

#[test]
fn exit_0_on_success() {
    let home = Home::new();
    home.kipi().arg("resolve").assert().code(0);
}

#[test]
fn exit_1_on_unknown_repo_with_a_hint() {
    let home = Home::new();
    let (code, err) = failure_of(&home, home.kipi().args(["explain", "billing-apu"]));
    assert_eq!(code, 1);
    assert_eq!(
        err,
        "error: no repo entry `billing-apu` in the mounted stores\n  hint: did you mean `billing-api`?\n"
    );
    let (code, err) = failure_of(&home, home.kipi().args(["explain", "Billing"]));
    assert_eq!(code, 1);
    assert!(
        err.contains("`Billing` is an alias of `billing-api`"),
        "{err}"
    );
}

#[test]
fn exit_1_on_usage_errors() {
    let home = Home::new();
    let (code, err) = failure_of(&home, home.bare().arg("--no-such-flag"));
    assert_eq!(
        code, 1,
        "not clap's default 2, which is reserved for doctor"
    );
    assert!(err.starts_with("error: unexpected argument"), "{err}");
    let (code, _) = failure_of(&home, home.bare().arg("explain"));
    assert_eq!(code, 1, "missing required argument");
    let (code, _) = failure_of(&home, home.bare().args(["-v", "-q", "resolve"]));
    assert_eq!(code, 1, "conflicting flags");
}

#[test]
fn exit_1_on_unimplemented_commands() {
    let home = Home::new();
    let (code, err) = failure_of(&home, home.kipi().args(["completions", "zsh"]));
    assert_eq!(code, 1);
    assert_eq!(
        err,
        "error: `kipi completions` is not implemented yet\n  hint: it lands in Stage 4 \
         (roadmap §14); `kipi resolve` and `kipi explain <repo>` work today\n"
    );
}

#[test]
fn exit_1_on_an_invalid_store() {
    let home = Home::new();
    let (code, err) = failure_of(&home, home.bare().arg("resolve"));
    assert_eq!(code, 1, "the default store does not exist");
    assert_eq!(
        err,
        "error: [HOME]/.agents/store/bindings.toml: no bindings.toml: not a kipi store\n  \
         hint: create a store with `kipi init --store <path>`, or point --store / KIPI_STORE at \
         an existing one\n"
    );
    let (code, err) = failure_of(&home, home.mounting(&[ExampleStore::Acme]).arg("resolve"));
    assert_eq!(
        code, 1,
        "the work store needs the personal store it shares from"
    );
    assert!(err.contains("which is not mounted"), "{err}");
}

#[test]
fn exit_3_on_io_failure() {
    let home = Home::new();
    let store = home.path().join("broken");
    fs::create_dir_all(store.join("bindings.toml")).unwrap();
    let (code, err) = failure_of(&home, home.bare().arg("--store").arg(&store).arg("resolve"));
    assert_eq!(code, 3, "{err}");
    assert!(
        err.starts_with("error: [HOME]/broken/bindings.toml: "),
        "{err}"
    );
    assert!(err.contains("\n  hint: "), "{err}");
}

#[test]
fn stores_come_from_kipi_store_and_config_toml() {
    let home = Home::new();
    let personal = home.store(ExampleStore::Personal);
    let acme = home.store(ExampleStore::Acme);
    let list = std::env::join_paths([&personal, &acme]).unwrap();
    home.bare()
        .env("KIPI_STORE", &list)
        .args(["explain", "events-pipeline"])
        .assert()
        .success();

    let config = "stores = [\"~/stores/personal-store\", \"~/stores/acme-store\"]\n";
    fs::write(home.path().join(".config/kipi/config.toml"), config).unwrap();
    home.bare()
        .args(["explain", "events-pipeline"])
        .assert()
        .success();
}

#[test]
fn verbose_names_mounted_stores_and_quiet_stays_silent() {
    let home = Home::new();
    home.kipi()
        .args(["resolve", "-v"])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "mounted store `acme` at ~/stores/acme-store",
        ));
    home.kipi()
        .args(["resolve", "-q"])
        .assert()
        .success()
        .stderr("");
}

#[test]
fn piped_output_has_no_color_codes() {
    let home = Home::new();
    home.kipi()
        .args(["explain", "events-pipeline"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\u{1b}[").not());
}

/// A home with the example stores and every example repo checked out under it, but no
/// `machine.toml` yet; the personal store lacks `[repos.dotfiles]` and blog's `remote`.
fn scan_home() -> Home {
    let home = Home::new();
    fs::remove_file(home.path().join(".config/kipi/machine.toml")).unwrap();
    lay_out(home.path(), &example_repos(&ExampleStore::ALL).unwrap()).unwrap();
    let file = home.store(ExampleStore::Personal).join("bindings.toml");
    let text = fs::read_to_string(&file).unwrap();
    let text = text
        .replace("remote = \"github.com/example-user/blog\"\n", "")
        .replace(
            "\n[repos.dotfiles]                   # no summary yet\n\
             remote = \"github.com/example-user/dotfiles\"\n",
            "",
        );
    fs::write(&file, text).unwrap();
    home
}

#[test]
fn scan_dry_run_prints_the_diff_and_writes_nothing() {
    let home = scan_home();
    let bindings = home.store(ExampleStore::Personal).join("bindings.toml");
    let before = fs::read_to_string(&bindings).unwrap();
    let out = stdout_of(&home, home.kipi().args(["scan", "--dry-run"]));
    insta::assert_snapshot!("scan-dry-run", out);
    assert_eq!(fs::read_to_string(&bindings).unwrap(), before);
    assert!(!home.path().join(".config/kipi/machine.toml").exists());
}

#[test]
fn scan_writes_then_is_a_no_op() {
    let home = scan_home();
    let out = stdout_of(&home, home.kipi().arg("scan"));
    insta::assert_snapshot!("scan", out);
    let machine = fs::read_to_string(home.path().join(".config/kipi/machine.toml")).unwrap();
    assert!(machine.contains("dotfiles = "), "{machine}");
    // The stores still load and resolve with the new entries.
    let explain = stdout_of(&home, home.kipi().args(["explain", "dotfiles"]));
    assert!(explain.contains("~/code/dotfiles"), "{explain}");

    let again = stdout_of(&home, home.kipi().arg("scan"));
    assert!(again.ends_with("\nNothing to write.\n"), "{again}");
    assert!(
        !again.contains("Added") && !again.contains("Recorded"),
        "{again}"
    );
}

#[test]
fn scan_json_reports_everything() {
    let home = scan_home();
    let out = stdout_of(&home, home.kipi().args(["scan", "--json", "--dry-run"]));
    let doc: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
    assert_eq!(doc["version"], 1);
    assert_eq!(doc["dry_run"], true);
    assert_eq!(doc["checkouts"].as_array().unwrap().len(), 10);
    assert_eq!(doc["added"][0]["name"], "dotfiles");
    assert_eq!(doc["added"][0]["store"], "personal");
    assert_eq!(doc["remotes_filled"][0]["entry"], "blog");
    assert_eq!(doc["recorded"].as_array().unwrap().len(), 10);
    assert_eq!(doc["files"].as_array().unwrap().len(), 2);
    insta::assert_snapshot!("scan-json", out);
}

#[test]
fn scan_reports_checkouts_outside_every_root() {
    let home = scan_home();
    let repo = FakeRepo {
        entry: String::new(),
        dir: "src/scratch".into(),
        origin_url: Some("https://git.example.com/me/scratch".into()),
        readme: Some("Throwaway experiments.\n".into()),
        manifest: ManifestKind::None,
        description: None,
    };
    let path = repo.create_under(home.path()).unwrap();
    let out = stdout_of(&home, home.kipi().arg("scan").arg(&path));
    assert!(
        out.contains(
            "warning: ~/src/scratch: lies outside every scope root\n  hint: add it with \
             `kipi scan [HOME]/src/scratch --store <name> --scope <scope>`"
        ),
        "{out}"
    );
    let out = stdout_of(
        &home,
        home.kipi()
            .arg("scan")
            .arg(&path)
            .args(["--store", "personal", "--scope", "personal"]),
    );
    assert!(
        out.contains("+ scratch  store personal, scope = \"personal\""),
        "{out}"
    );
    let text = fs::read_to_string(home.store(ExampleStore::Personal).join("bindings.toml"));
    assert!(text.unwrap().contains(
        "[repos.scratch]\nremote = \"git.example.com/me/scratch\"\nscope = \"personal\"\n"
    ));
}

#[test]
fn scan_checks_store_and_scope() {
    let home = scan_home();
    let (code, err) = failure_of(&home, home.kipi().args(["scan", "--scope", "acme-m"]));
    assert_eq!(code, 1);
    assert_eq!(
        err,
        "error: no scope `acme-m` in the mounted stores\n  hint: did you mean `acme-ml`?\n"
    );
    let (code, err) = failure_of(
        &home,
        home.kipi()
            .args(["scan", "--store", "acme", "--scope", "personal"]),
    );
    assert_eq!(code, 1);
    assert_eq!(
        err,
        "error: scope `personal` belongs to store `personal`, not `acme`\n  hint: pass \
         `--store personal`, or a scope of store `acme`\n"
    );
    let (code, _) = failure_of(&home, home.kipi().args(["scan", "--store", "acme"]));
    assert_eq!(code, 1, "--store needs --scope");
}

/// A home with the example stores mounted with `kipi init --store <existing store>` and
/// every example repo checked out under it, scanned into `machine.toml`.
fn onboarded_home() -> Home {
    let home = Home::new();
    fs::remove_file(home.path().join(".config/kipi/machine.toml")).unwrap();
    lay_out(home.path(), &example_repos(&ExampleStore::ALL).unwrap()).unwrap();
    for store in ExampleStore::ALL {
        let path = home.store(store);
        home.bare()
            .arg("init")
            .arg("--store")
            .arg(path)
            .assert()
            .success();
    }
    let config = fs::read_to_string(home.path().join(".config/kipi/config.toml")).unwrap();
    assert_eq!(
        config,
        "stores = [\"~/stores/personal-store\", \"~/stores/acme-store\"]\n"
    );
    home.bare().arg("scan").assert().success();
    home
}

/// Everything under `dir` as path → contents (`-> target` for symlinks), skipping
/// `.git` except its `info/exclude`, and the token cache in `.cache/`.
fn files_under(dir: &Path) -> BTreeMap<PathBuf, String> {
    // Inside `.git`, descend only into `info/` and keep only `info/exclude`.
    let keep = |e: &walkdir::DirEntry| {
        let parent = e.path().parent().and_then(Path::file_name);
        let grandparent = e
            .path()
            .parent()
            .and_then(Path::parent)
            .and_then(Path::file_name);
        let name = e.file_name();
        if name == ".cache" {
            false
        } else if parent == Some(".git".as_ref()) {
            name == "info"
        } else if parent == Some("info".as_ref()) && grandparent == Some(".git".as_ref()) {
            name == "exclude"
        } else {
            true
        }
    };
    let mut out = BTreeMap::new();
    let walk = WalkDir::new(dir)
        .follow_links(false)
        .into_iter()
        .filter_entry(keep);
    for entry in walk {
        let entry = entry.unwrap();
        let value = if entry.path_is_symlink() {
            format!("-> {}", fs::read_link(entry.path()).unwrap().display())
        } else if entry.file_type().is_dir() {
            continue;
        } else {
            fs::read_to_string(entry.path()).unwrap_or_default()
        };
        out.insert(entry.path().to_path_buf(), value);
    }
    out
}

fn json_of(home: &Home, cmd: &mut Command) -> serde_json::Value {
    serde_json::from_str(&stdout_of(home, cmd)).expect("valid JSON")
}

/// Replace `from` with `to` in the `bindings.toml` of `store`.
fn edit_bindings(home: &Home, store: ExampleStore, from: &str, to: &str) {
    let path = home.store(store).join("bindings.toml");
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains(from), "{from:?} not in bindings");
    fs::write(&path, text.replacen(from, to, 1)).unwrap();
}

#[test]
fn onboarding_init_scan_sync_then_sync_is_a_no_op() {
    let home = onboarded_home();
    let first = stdout_of(&home, home.bare().args(["sync", "--all"]));
    insta::assert_snapshot!("sync-all-first", first);
    let synced = files_under(home.path());

    let second = stdout_of(&home, home.bare().args(["sync", "--all"]));
    insta::assert_snapshot!("sync-all-second", second);
    assert_eq!(
        files_under(home.path()),
        synced,
        "the second sync wrote nothing"
    );

    let doc = json_of(&home, home.bare().args(["sync", "--all", "--json"]));
    assert_eq!(doc["dry_run"], false);
    for repo in doc["repos"].as_array().unwrap() {
        assert_eq!(repo["status"], "unchanged", "{repo}");
        let report = &repo["report"];
        assert_eq!(report["changes"], serde_json::json!([]), "{repo}");
        assert_eq!(
            report["links"],
            serde_json::json!({"added": 0, "removed": 0})
        );
        assert_eq!(
            report["blocks"],
            serde_json::json!({"written": 0, "removed": 0})
        );
        assert_eq!(report["grants"], serde_json::json!([]));
        assert_eq!(report["exclude_changed"], false);
    }
    for store in doc["registries"].as_array().unwrap() {
        assert_eq!(store["report"]["changes"], serde_json::json!([]), "{store}");
    }
}

#[test]
fn sync_writes_blocks_into_claude_md_and_agents_md_and_the_exclude_file() {
    let home = onboarded_home();
    home.bare().args(["sync", "--all"]).assert().success();
    let dir = home.path().join("work/acme/billing-api");
    for file in ["CLAUDE.md", "AGENTS.md"] {
        let text = fs::read_to_string(dir.join(file)).unwrap();
        assert!(text.starts_with("<!-- kipi:begin"), "{file}: {text}");
        assert!(text.contains("## Known repos (acme)"), "{file}: {text}");
    }
    let exclude = fs::read_to_string(dir.join(".git/info/exclude")).unwrap();
    for pattern in [
        ".claude/skills/",
        ".claude/settings.local.json",
        ".agents/",
        ".kipi/",
    ] {
        assert!(
            exclude.contains(&format!("\n{pattern}\n")),
            "{pattern} in {exclude}"
        );
    }
    assert!(dir.join(".claude/skills/code-review").is_symlink());
    assert!(dir.join(".agents/skills/code-review").is_symlink());
    assert!(
        home.store(ExampleStore::Acme)
            .join("generated/registry.acme.md")
            .is_file()
    );
}

#[test]
fn removing_a_related_entry_removes_its_grant_and_its_mention() {
    let home = onboarded_home();
    home.bare().args(["sync", "--all"]).assert().success();
    let dir = home.path().join("code/side-tool");
    let api = home.path().join("code/api").display().to_string();
    let api_json = "[HOME]/code/api";
    let settings = || fs::read_to_string(dir.join(".claude/settings.local.json")).unwrap();
    let block = || fs::read_to_string(dir.join("CLAUDE.md")).unwrap();
    assert!(settings().contains(&format!("\"{api}\"")), "{}", settings());
    assert!(
        block().contains("Related: api (read access granted)."),
        "{}",
        block()
    );

    edit_bindings(&home, ExampleStore::Personal, "related = [\"api\"]\n", "");
    let doc = json_of(&home, home.bare().args(["sync", "side-tool", "--json"]));
    let repos = doc["repos"].as_array().unwrap();
    assert_eq!(repos.len(), 1, "only the named repo");
    assert_eq!(
        repos[0]["report"]["grants"][0]["removed"],
        serde_json::json!([api_json])
    );
    assert!(
        !settings().contains(&format!("\"{api}\"")),
        "{}",
        settings()
    );
    assert!(!block().contains("Related:"), "{}", block());
    assert!(
        !fs::read_to_string(dir.join("AGENTS.md"))
            .unwrap()
            .contains("Related:")
    );
    let again = stdout_of(&home, home.bare().args(["sync", "side-tool"]));
    assert!(
        again.contains("side-tool  ~/code/side-tool\n  up to date"),
        "{again}"
    );
}

#[test]
fn dry_run_lists_the_changes_and_writes_nothing() {
    let home = onboarded_home();
    let before = files_under(home.path());
    let out = stdout_of(&home, home.bare().args(["sync", "side-tool", "--dry-run"]));
    insta::assert_snapshot!("sync-dry-run", out);
    assert_eq!(files_under(home.path()), before);
    let doc = json_of(
        &home,
        home.bare().args(["sync", "--all", "--dry-run", "--json"]),
    );
    assert_eq!(doc["dry_run"], true);
    assert_eq!(doc["repos"][0]["status"], "changed");
    assert_eq!(files_under(home.path()), before);
}

#[test]
fn repos_not_cloned_here_are_skipped_with_a_note() {
    let home = onboarded_home();
    let machine = home.path().join(".config/kipi/machine.toml");
    let text = fs::read_to_string(&machine).unwrap();
    let blog = text.lines().find(|l| l.starts_with("blog = ")).unwrap();
    fs::write(&machine, text.replace(&format!("{blog}\n"), "")).unwrap();
    fs::remove_dir_all(home.path().join("code/api")).unwrap();
    let out = stdout_of(&home, home.bare().args(["sync", "blog", "api"]));
    assert_eq!(
        out,
        "blog\n  skipped: not cloned on this machine\n\
         api  ~/code/api\n  warning: the checkout is gone; skipped\n    \
         hint: re-run `kipi scan` to record where it lives now\n\
         store personal  ~/stores/personal-store/generated  1 registry written\n\
         store acme  ~/stores/acme-store/generated  1 registry written\n\
         \nSynced 2 repos: 0 changed, 0 up to date, 2 skipped.\n"
    );
    let doc = json_of(&home, home.bare().args(["sync", "blog", "api", "--json"]));
    assert_eq!(doc["repos"][0]["status"], "not_cloned");
    assert_eq!(doc["repos"][0]["report"], serde_json::Value::Null);
    assert_eq!(doc["repos"][1]["status"], "checkout_missing");
}

#[test]
fn an_entry_gone_from_every_store_is_cleaned_up_by_sync_all() {
    let home = onboarded_home();
    home.bare().args(["sync", "--all"]).assert().success();
    let dir = home.path().join("code/blog");
    assert!(dir.join("CLAUDE.md").is_file());
    edit_bindings(
        &home,
        ExampleStore::Personal,
        "[repos.blog]\nremote = \"github.com/example-user/blog\"\n\
         summary = \"Static site; posts in content/, theme in themes/plain\"\n\
         aliases = [\"site\"]\n",
        "",
    );
    let out = stdout_of(&home, home.bare().args(["sync", "--all"]));
    assert!(
        out.contains("blog  ~/code/blog\n  -4 links, 2 blocks removed, -2 grants, exclude updated; its entry is in no mounted store\n"),
        "{out}"
    );
    for gone in ["CLAUDE.md", "AGENTS.md", ".claude", ".agents", ".kipi"] {
        assert!(!dir.join(gone).exists(), "{gone} left behind");
    }
    let exclude = fs::read_to_string(dir.join(".git/info/exclude")).unwrap_or_default();
    assert!(!exclude.contains("kipi"), "{exclude}");
    let again = stdout_of(&home, home.bare().args(["sync", "--all"]));
    assert!(!again.contains("blog"), "nothing left to clean: {again}");
}

#[test]
fn agents_overrides_the_plan_and_unknown_agents_are_user_errors() {
    let home = onboarded_home();
    let size = |doc: &serde_json::Value| doc["repos"][0]["always_on"].clone();
    let planned = json_of(
        &home,
        home.bare()
            .args(["sync", "billing-api", "--dry-run", "--json"]),
    );
    let doc = json_of(
        &home,
        home.bare()
            .args(["sync", "billing-api", "--agents", "generic", "--json"]),
    );
    let dir = home.path().join("work/acme/billing-api");
    assert!(dir.join("AGENTS.md").is_file());
    assert!(!dir.join("CLAUDE.md").exists());
    assert!(!dir.join(".claude").exists());
    assert_eq!(
        size(&doc),
        size(&planned),
        "always-on is the cost to one agent, whichever adapters write it"
    );
    assert!(size(&doc)["tokens"].as_u64().unwrap() > 0);
    let (code, err) = failure_of(
        &home,
        home.bare().args(["sync", "--all", "--agents", "claud"]),
    );
    assert_eq!(code, 1);
    assert_eq!(
        err,
        "error: unknown agent `claud`\n  hint: did you mean `claude`?\n"
    );
}

/// With a user-written AGENTS.md and both adapters on, Claude Code keeps reading
/// AGENTS.md, so sync writes the block there once and creates no CLAUDE.md.
#[test]
fn a_user_agents_md_carries_the_one_block_and_no_claude_md_is_created() {
    let home = onboarded_home();
    let dir = home.path().join("work/acme/billing-api");
    fs::write(dir.join("AGENTS.md"), "# Ours\n").unwrap();
    let out = stdout_of(&home, home.bare().args(["sync", "billing-api"]));
    assert!(
        out.contains("  note: CLAUDE.md not created: the repo's own AGENTS.md"),
        "{out}"
    );
    assert!(!dir.join("CLAUDE.md").exists());
    assert!(!dir.join("CLAUDE.local.md").exists());
    let agents = fs::read_to_string(dir.join("AGENTS.md")).unwrap();
    assert!(agents.starts_with("# Ours\n\n<!-- kipi:begin"), "{agents}");
    assert_eq!(agents.matches("<!-- kipi:begin").count(), 1, "{agents}");
    let again = stdout_of(&home, home.bare().args(["sync", "billing-api"]));
    assert!(again.contains("  up to date"), "{again}");
}

/// A tracked CLAUDE.md is never dirtied: the block goes to CLAUDE.local.md (excluded
/// from git), until `--allow-tracked` moves it into CLAUDE.md.
#[test]
fn a_tracked_claude_md_gets_its_block_in_claude_local_md_unless_allow_tracked() {
    let home = onboarded_home();
    let dir = home.path().join("work/acme/billing-api");
    fs::write(dir.join("CLAUDE.md"), "# Team rules\n").unwrap();
    let added = std::process::Command::new("git")
        .arg("-C")
        .arg(&dir)
        .args(["add", "CLAUDE.md"])
        .status()
        .unwrap();
    assert!(added.success());

    home.bare().args(["sync", "billing-api"]).assert().success();
    assert_eq!(
        fs::read_to_string(dir.join("CLAUDE.md")).unwrap(),
        "# Team rules\n"
    );
    let local = fs::read_to_string(dir.join("CLAUDE.local.md")).unwrap();
    assert!(local.starts_with("<!-- kipi:begin"), "{local}");
    let exclude = fs::read_to_string(dir.join(".git/info/exclude")).unwrap();
    assert!(exclude.contains("\nCLAUDE.local.md\n"), "{exclude}");

    home.bare()
        .args(["sync", "billing-api", "--allow-tracked"])
        .assert()
        .success();
    let claude = fs::read_to_string(dir.join("CLAUDE.md")).unwrap();
    assert!(
        claude.starts_with("# Team rules\n\n<!-- kipi:begin"),
        "{claude}"
    );
    assert!(
        !dir.join("CLAUDE.local.md").exists(),
        "the kipi-created side file goes"
    );
    let exclude = fs::read_to_string(dir.join(".git/info/exclude")).unwrap();
    assert!(!exclude.contains("CLAUDE.local.md"), "{exclude}");
}

#[test]
fn sync_needs_repos_or_all() {
    let home = onboarded_home();
    let (code, err) = failure_of(&home, home.bare().arg("sync"));
    assert_eq!(code, 1);
    assert_eq!(
        err,
        "error: no repo to sync\n  hint: name repo entries (`kipi sync billing-api`) or pass `--all`\n"
    );
    let (code, err) = failure_of(&home, home.bare().args(["sync", "billing-apu"]));
    assert_eq!(code, 1);
    assert!(err.contains("did you mean `billing-api`?"), "{err}");
    let (code, _) = failure_of(&home, home.bare().args(["sync", "blog", "--all"]));
    assert_eq!(code, 1, "repos and --all conflict");
}

#[test]
fn status_tracks_missing_synced_and_stale_without_writing() {
    let home = onboarded_home();
    let before = files_under(home.path());
    let out = stdout_of(&home, home.bare().arg("status"));
    insta::assert_snapshot!("status-before-sync", out);
    assert_eq!(files_under(home.path()), before, "status writes nothing");

    home.bare().args(["sync", "--all"]).assert().success();
    let out = stdout_of(&home, home.bare().arg("status"));
    insta::assert_snapshot!("status-synced", out);

    edit_bindings(&home, ExampleStore::Personal, "related = [\"api\"]\n", "");
    let synced = files_under(home.path());
    let doc = json_of(&home, home.bare().args(["status", "side-tool", "--json"]));
    assert_eq!(files_under(home.path()), synced);
    insta::assert_snapshot!(
        "status-stale-json",
        serde_json::to_string_pretty(&doc).unwrap()
    );
    let repo = &doc["repos"][0];
    assert_eq!(repo["state"], "stale");
    assert_eq!(repo["scope"], "personal");
    assert!(!repo["pending"].as_array().unwrap().is_empty());
    assert_eq!(doc["repos"].as_array().unwrap().len(), 1);

    let doc = json_of(&home, home.bare().args(["status", "--json"]));
    assert_eq!(doc["draft_summaries"], 3, "api's, and the two scan drafted");
    let api = doc["repos"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "api")
        .unwrap();
    assert_eq!(api["draft_summary"], true);
    assert_eq!(api["state"], "synced");
}

#[test]
fn status_of_repos_not_cloned_here() {
    let home = Home::new();
    let out = stdout_of(&home, home.kipi().args(["status", "events-pipeline"]));
    assert!(
        out.contains("events-pipeline  acme-ml  checkout gone"),
        "the example machine.toml points at /Volumes: {out}"
    );
}

#[test]
fn a_renamed_entry_keeps_its_checkout_synced_not_cleaned_up() {
    let home = onboarded_home();
    home.bare().args(["sync", "--all"]).assert().success();
    edit_bindings(
        &home,
        ExampleStore::Personal,
        "[repos.blog]",
        "[repos.site-blog]",
    );
    let machine = home.path().join(".config/kipi/machine.toml");
    let text = fs::read_to_string(&machine).unwrap();
    let line = text.lines().find(|l| l.starts_with("blog = ")).unwrap();
    // The old name stays in machine.toml beside the new one, pointing at the same checkout.
    fs::write(
        &machine,
        format!("{text}{}\n", line.replacen("blog", "site-blog", 1)),
    )
    .unwrap();
    let out = stdout_of(&home, home.bare().args(["sync", "--all"]));
    assert!(out.contains("site-blog  ~/code/blog\n"), "{out}");
    assert!(!out.contains("no mounted store"), "{out}");
    let dir = home.path().join("code/blog");
    assert!(dir.join("CLAUDE.md").is_file());
    assert!(dir.join(".claude/skills/rust-style").is_symlink());
}

#[test]
fn doctor_on_the_example_stores_exits_0() {
    let home = Home::new();
    let out = stdout_of(&home, home.kipi().arg("doctor"));
    insta::assert_snapshot!("doctor", out);
    assert!(out.ends_with("0 errors, 0 warnings, 5 info\n"), "{out}");
    let json = stdout_of(&home, home.kipi().args(["doctor", "--json"]));
    insta::assert_snapshot!("doctor-json", json);
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["summary"]["error"], 0);
    let one = stdout_of(&home, home.kipi().args(["doctor", "dotfiles"]));
    assert!(
        one.starts_with("info[K008] dotfiles: repo `dotfiles` has no summary"),
        "{one}"
    );
    assert!(one.ends_with("0 errors, 0 warnings, 1 info\n"), "{one}");
    let clean = stdout_of(&home, home.kipi().args(["doctor", "side-tool"]));
    assert_eq!(clean, "no problems found\n");
}

#[test]
fn doctor_exits_2_on_an_error_and_still_prints_every_finding() {
    let home = Home::new();
    let bindings = home.store(ExampleStore::Acme).join("bindings.toml");
    let text = fs::read_to_string(&bindings)
        .unwrap()
        .replace("aliases = [\"ml\"]", "aliases = [\"ml\", \"Billing\"]");
    fs::write(&bindings, text).unwrap();
    let out = home
        .kipi()
        .arg("doctor")
        .assert()
        .code(2)
        .get_output()
        .clone();
    let stdout = home.scrub(&out.stdout);
    assert!(
        stdout.starts_with(
            "error[K004] billing-api: alias `billing` is claimed by `billing-api` and \
             `ml-platform` in registry `acme`\n  --> acme:bindings.toml:25\n"
        ),
        "{stdout}"
    );
    assert!(
        stdout.ends_with("1 error, 0 warnings, 5 info\n"),
        "{stdout}"
    );
    assert_eq!(
        home.scrub(&out.stderr),
        "error: doctor found 1 error-level problem\n  hint: fix each as its hint says, then run \
         `kipi doctor` again\n"
    );
    let out = home
        .kipi()
        .args(["doctor", "--json"])
        .assert()
        .code(2)
        .get_output()
        .clone();
    let parsed: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(parsed["findings"][0]["code"], "K004");
    assert_eq!(parsed["findings"][0]["check"], "alias-collision");
    assert_eq!(parsed["summary"]["error"], 1);
}

#[test]
fn doctor_reports_a_cross_store_reference_instead_of_failing() {
    let home = Home::new();
    let bindings = home.store(ExampleStore::Personal).join("bindings.toml");
    let text = fs::read_to_string(&bindings)
        .unwrap()
        .replace("share_skills = true", "share_skills = false");
    fs::write(&bindings, text).unwrap();
    // Everything else refuses to load these stores...
    let (code, _) = failure_of(&home, home.kipi().arg("resolve"));
    assert_eq!(code, 1);
    // ...doctor names the problem and checks the rest.
    let out = home
        .kipi()
        .arg("doctor")
        .assert()
        .code(2)
        .get_output()
        .clone();
    let stdout = home.scrub(&out.stdout);
    assert!(
        stdout.starts_with(
            "error[K012] skill `personal/commit-style` is in store `personal`, which does not \
             share its skills\n"
        ),
        "{stdout}"
    );
    assert!(
        stdout.ends_with("1 error, 0 warnings, 5 info\n"),
        "{stdout}"
    );
}

#[test]
fn doctor_user_errors_exit_1() {
    let home = Home::new();
    let (code, err) = failure_of(&home, home.kipi().args(["doctor", "billing-apu"]));
    assert_eq!(code, 1);
    assert!(err.contains("did you mean `billing-api`?"), "{err}");
    let (code, err) = failure_of(&home, home.kipi().args(["doctor", "--fix"]));
    assert_eq!(code, 1);
    assert!(
        err.starts_with("error: `kipi doctor --fix` is not implemented yet"),
        "{err}"
    );
}

/// A skill directory as a user keeps it under `~/.claude/skills/<name>`.
fn user_skill(home: &Home, name: &str) -> PathBuf {
    let dir = home.path().join(".claude/skills").join(name);
    fs::create_dir_all(dir.join("references")).unwrap();
    fs::write(
        dir.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: Use when reviewing a change.\n---\n\n# {name}\n"),
    )
    .unwrap();
    fs::write(dir.join("references/checklist.md"), "- tests\n").unwrap();
    dir
}

#[test]
fn authoring_from_an_empty_home_to_a_synced_repo() {
    let home = Home::empty();
    for (dir, url) in [
        ("code/app", "https://github.com/example-user/app.git"),
        (
            "code/lib-core",
            "https://github.com/example-user/lib-core.git",
        ),
    ] {
        let repo = FakeRepo {
            entry: String::new(),
            dir: dir.into(),
            origin_url: Some(url.into()),
            readme: Some("A small example project.\n".into()),
            manifest: ManifestKind::None,
            description: None,
        };
        repo.create_under(home.path()).unwrap();
    }

    let out = stdout_of(&home, home.bare().args(["init", "--git"]));
    assert!(
        out.contains("+ store: personal at ~/.agents/store"),
        "{out}"
    );
    let store = home.path().join(".agents/store");
    assert!(store.join(".git").is_dir());
    // The user declares a scope by hand, as the skeleton's commented example shows.
    let bindings = store.join("bindings.toml");
    let mut text = fs::read_to_string(&bindings).unwrap();
    text.push_str("\n[scopes.personal]\nroots = [\"~/code\"]\nregistry = true\n");
    fs::write(&bindings, text).unwrap();

    home.bare().arg("scan").assert().success();
    let src = user_skill(&home, "review");
    let out = stdout_of(
        &home,
        home.bare()
            .args([
                "add",
                "skill",
                "review",
                "--scope",
                "personal",
                "--link-back",
                "--from",
            ])
            .arg(&src),
    );
    assert!(out.contains("+ scopes.personal.skills: review"), "{out}");
    assert_eq!(fs::read_link(&src).unwrap(), store.join("skills/review"));

    let before_bind = fs::read_to_string(&bindings).unwrap();
    let out = stdout_of(
        &home,
        home.bare().args(["bind", "app", "--related", "lib-core"]),
    );
    assert!(out.contains("+ related: lib-core"), "{out}");
    home.bare().args(["sync", "--all"]).assert().success();

    let app = home.path().join("code/app");
    assert_eq!(
        fs::read_link(app.join(".claude/skills/review")).unwrap(),
        store.join("skills/review")
    );
    let block = fs::read_to_string(app.join("CLAUDE.md")).unwrap();
    assert!(
        block.contains("Related: lib-core (read access granted)."),
        "{block}"
    );
    let lib = home.path().join("code/lib-core").display().to_string();
    let settings = || fs::read_to_string(app.join(".claude/settings.local.json")).unwrap();
    assert!(settings().contains(&lib), "{}", settings());

    home.bare()
        .args(["unbind", "app", "--related", "lib-core"])
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(&bindings).unwrap(),
        before_bind,
        "byte-identical"
    );
    home.bare().args(["sync", "--all"]).assert().success();
    assert!(!settings().contains(&lib), "{}", settings());
}

#[test]
fn bind_dry_run_prints_the_diff_and_writes_nothing() {
    let home = Home::new();
    let bindings = home.store(ExampleStore::Acme).join("bindings.toml");
    let before = fs::read_to_string(&bindings).unwrap();
    let out = stdout_of(
        &home,
        home.kipi().args([
            "bind",
            "events-pipeline",
            "--skill",
            "code-review",
            "--related",
            "billing-api",
            "--alias",
            "kafka",
            "--dry-run",
        ]),
    );
    insta::assert_snapshot!("bind-dry-run", out);
    assert_eq!(fs::read_to_string(&bindings).unwrap(), before);

    let doc = json_of(
        &home,
        home.kipi().args([
            "unbind",
            "events-pipeline",
            "--exclude",
            "pr-conventions",
            "--json",
        ]),
    );
    assert_eq!(doc["command"], "unbind");
    assert_eq!(doc["store"], "acme");
    assert_eq!(doc["dry_run"], false);
    assert_eq!(
        doc["changes"],
        serde_json::json!([{"op": "remove", "what": "exclude", "value": "pr-conventions", "note": null}])
    );
    assert!(!fs::read_to_string(&bindings).unwrap().contains("exclude ="));
}

#[test]
fn a_bind_breaking_the_leak_rule_is_rejected_without_writing() {
    let home = Home::new();
    let bindings = home.store(ExampleStore::Personal).join("bindings.toml");
    let before = fs::read_to_string(&bindings).unwrap();
    let (code, err) = failure_of(
        &home,
        home.kipi()
            .args(["bind", "side-tool", "--related", "billing-api"]),
    );
    assert_eq!(code, 1);
    assert!(err.contains("(rejected: nothing was written)"), "{err}");
    assert!(err.contains("\n  hint: "), "{err}");
    assert_eq!(fs::read_to_string(&bindings).unwrap(), before);
}

#[test]
fn add_repo_registers_an_entry_and_its_checkout() {
    let home = Home::new();
    let repo = FakeRepo {
        entry: String::new(),
        dir: "code/notes-app".into(),
        origin_url: Some("https://github.com/example-user/notes-app.git".into()),
        readme: None,
        manifest: ManifestKind::None,
        description: None,
    };
    let path = repo.create_under(home.path()).unwrap();
    let out = stdout_of(
        &home,
        home.kipi()
            .args([
                "add",
                "repo",
                "notes-app",
                "--scope",
                "personal",
                "--summary",
                "Notes, synced",
                "--path",
            ])
            .arg(&path),
    );
    insta::assert_snapshot!("add-repo", out);
    let explain = stdout_of(&home, home.kipi().args(["explain", "notes-app"]));
    assert!(explain.contains("~/code/notes-app"), "{explain}");
}

#[test]
fn stores_lists_and_unmount_cleans_up_everything_a_store_contributed() {
    let home = onboarded_home();
    let work = home.path().join("work");
    let pristine = files_under(&work);
    insta::assert_snapshot!("stores", stdout_of(&home, home.bare().arg("stores")));
    let doc = json_of(&home, home.bare().args(["stores", "--json"]));
    assert_eq!(doc["stores"][1]["name"], "acme");
    assert_eq!(doc["stores"][1]["repos"], 6);
    assert_eq!(doc["stores"][0]["share_skills"], true);

    home.bare().args(["sync", "--all"]).assert().success();
    assert_ne!(files_under(&work), pristine);
    let out = stdout_of(&home, home.bare().args(["stores", "--unmount", "acme"]));
    assert!(
        out.contains("- config.toml stores: acme (~/stores/acme-store)"),
        "{out}"
    );
    home.bare().args(["sync", "--all"]).assert().success();
    assert_eq!(files_under(&work), pristine, "no work content left");
    for (path, text) in files_under(&home.path().join("code")) {
        assert!(!text.contains("acme"), "{}: {text}", path.display());
    }
    let out = stdout_of(&home, home.bare().arg("stores"));
    assert!(!out.contains("acme"), "{out}");
}

#[test]
fn init_mounts_an_existing_store_without_touching_it() {
    let home = Home::new();
    let store = home.store(ExampleStore::Personal);
    let before = files_under(&store);
    let out = stdout_of(&home, home.bare().arg("init").arg("--store").arg(&store));
    assert!(
        out.contains("= store: personal at ~/stores/personal-store  (exists; mounted as is)"),
        "{out}"
    );
    assert_eq!(files_under(&store), before);
    let again = stdout_of(&home, home.bare().arg("init").arg("--store").arg(&store));
    assert!(again.ends_with("Nothing to write.\n"), "{again}");
    let (code, err) = failure_of(
        &home,
        home.bare()
            .arg("init")
            .arg("--store")
            .arg(&store)
            .args(["--name", "work"]),
    );
    assert_eq!(code, 1);
    assert!(err.contains("is named `personal`, not `work`"), "{err}");
}

// --- where / grep / edit (#11) ---

#[test]
fn where_resolves_an_alias_to_the_entry_and_its_paths() {
    let home = Home::new();
    let out = stdout_of(
        &home,
        home.kipi()
            .arg("where")
            .arg("billing")
            .current_dir(home.path()),
    );
    insta::assert_snapshot!("where-billing", out);
    assert!(
        out.starts_with("billing-api — Invoicing and payment orchestration"),
        "{out}"
    );

    let same = stdout_of(
        &home,
        home.kipi()
            .args(["where", "  INVOICES "])
            .current_dir(home.path()),
    );
    assert_eq!(same, out, "aliases fold case and whitespace");

    let out = stdout_of(
        &home,
        home.kipi()
            .args(["where", "feature-store"])
            .current_dir(home.path()),
    );
    assert!(out.contains("local: (not cloned here)\n"), "{out}");
    assert!(out.contains("notes: none\n"), "{out}");

    let out = stdout_of(
        &home,
        home.kipi()
            .args(["where", "events", "--notes"])
            .current_dir(home.path()),
    );
    insta::assert_snapshot!("where-events-notes", out);

    let out = stdout_of(
        &home,
        home.kipi()
            .args(["where", "billing", "--notes", "--json"])
            .current_dir(home.path()),
    );
    insta::assert_snapshot!("where-billing-json", out);
    let doc: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(doc["name"], "billing-api");
    assert_eq!(doc["summary_draft"], true);
    assert!(
        doc["notes_text"]
            .as_str()
            .unwrap()
            .contains("Idempotency-Key")
    );
}

#[test]
fn where_fails_on_ambiguous_and_unknown_aliases() {
    let home = Home::new();
    edit_bindings(
        &home,
        ExampleStore::Acme,
        "summary = \"Public REST gateway\"\n",
        "summary = \"Public REST gateway\"\naliases = [\"billing\"]\n",
    );
    let (code, err) = failure_of(
        &home,
        home.kipi()
            .args(["where", "billing"])
            .current_dir(home.path()),
    );
    assert_eq!(code, 1);
    assert_eq!(
        err,
        "error: `billing` is ambiguous: acme-api, billing-api\n  hint: pass an entry name \
         (`kipi where acme-api`); `kipi doctor` reports the alias collision\n"
    );
    let (code, err) = failure_of(
        &home,
        home.kipi()
            .args(["where", "billing-apu"])
            .current_dir(home.path()),
    );
    assert_eq!(code, 1);
    assert_eq!(
        err,
        "error: no repo named or aliased `billing-apu`\n  hint: did you mean `billing-api`?\n"
    );
    let (code, err) = failure_of(
        &home,
        home.kipi()
            .args(["where", "web-dashboard", "--notes"])
            .current_dir(home.path()),
    );
    assert_eq!(code, 1);
    assert!(
        err.contains("hint: drop `--notes`, or write some with `kipi edit notes:web-dashboard`"),
        "{err}"
    );
}

#[test]
fn where_inside_a_repo_stays_in_its_scope_chain() {
    let home = onboarded_home();
    let side_tool = home.path().join("code/side-tool");
    let (code, err) = failure_of(
        &home,
        home.bare()
            .args(["where", "billing"])
            .current_dir(side_tool.join("src").parent().unwrap()),
    );
    assert_eq!(code, 1);
    assert_eq!(
        err,
        "error: no repo named or aliased `billing` in the scope chain of personal\n  hint: only \
         the scope chain of personal is searched from here; `kipi where --any-scope billing` \
         searches every mounted store\n"
    );
    let out = stdout_of(
        &home,
        home.bare()
            .args(["where", "billing", "--any-scope"])
            .current_dir(&side_tool),
    );
    assert!(out.starts_with("billing-api — "), "{out}");
    // From a nested directory of events-pipeline (acme-ml), its ancestor scope is visible.
    let nested = home.path().join("work/acme/ml/events-pipeline/src");
    fs::create_dir_all(&nested).unwrap();
    let out = stdout_of(
        &home,
        home.bare().args(["where", "billing"]).current_dir(&nested),
    );
    assert!(
        out.contains("local: [HOME]/work/acme/billing-api\n"),
        "{out}"
    );
    let (code, _) = failure_of(
        &home,
        home.bare()
            .args(["where", "side-tool"])
            .current_dir(&nested),
    );
    assert_eq!(code, 1, "personal repos are invisible from a work repo");
}

/// Write `text` to `rel` under the home directory.
fn put(home: &Home, rel: &str, text: &[u8]) {
    let path = home.path().join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// An onboarded home whose checkouts mention idempotency, in work and personal repos.
fn grep_home() -> Home {
    let home = onboarded_home();
    put(
        &home,
        "work/acme/billing-api/src/idempotency.rs",
        b"// Idempotency keys live 24 h.\npub fn idempotency_key() {}\n",
    );
    put(&home, "work/acme/billing-api/.gitignore", b"target/\n");
    put(
        &home,
        "work/acme/billing-api/target/debug.log",
        b"idempotency in a build artifact\n",
    );
    put(
        &home,
        "work/acme/billing-api/blob.bin",
        b"idempotency\0binary\n",
    );
    put(
        &home,
        "code/api/src/main.ts",
        b"// idempotency for the personal api\n",
    );
    put(
        &home,
        "code/side-tool/README.md",
        b"no idempotency here either\n",
    );
    home
}

#[test]
fn grep_repos_from_events_pipeline_finds_billing_api_and_never_personal_repos() {
    let home = grep_home();
    let dir = home.path().join("work/acme/ml/events-pipeline");
    let out = stdout_of(
        &home,
        home.bare()
            .args(["grep", "(?i)idempotency", "--repos"])
            .current_dir(&dir),
    );
    insta::assert_snapshot!("grep-repos-events-pipeline", out);
    assert!(
        out.contains(
            "[HOME]/work/acme/billing-api/src/idempotency.rs:2: pub fn idempotency_key() {}\n"
        ),
        "{out}"
    );
    assert!(
        out.contains("[HOME]/stores/acme-store/repos/billing-api/notes.md:"),
        "{out}"
    );
    assert!(!out.contains("[HOME]/code/"), "no personal checkout: {out}");
    assert!(
        !out.contains("personal-store/repos"),
        "no personal notes: {out}"
    );
    assert!(!out.contains("target/"), "gitignored: {out}");
    assert!(!out.contains("blob.bin"), "binary: {out}");

    // Without --repos only the store is searched.
    let out = stdout_of(
        &home,
        home.bare()
            .args(["grep", "(?i)idempotency"])
            .current_dir(&dir),
    );
    assert!(!out.contains("[HOME]/work/"), "{out}");
    assert!(out.contains("notes.md"), "{out}");

    // From a personal repo, nothing from the work store or work checkouts.
    let out = stdout_of(
        &home,
        home.bare()
            .args(["grep", "(?i)idempotency", "--repos"])
            .current_dir(home.path().join("code/side-tool")),
    );
    assert!(out.contains("[HOME]/code/api/src/main.ts:1:"), "{out}");
    assert!(!out.contains("acme"), "{out}");
}

#[test]
fn grep_scope_json_and_truncation() {
    let home = grep_home();
    let out = stdout_of(
        &home,
        home.bare()
            .args(["grep", "idempotency", "--repos", "--scope", "personal"])
            .current_dir(home.path()),
    );
    assert_eq!(
        out,
        "[HOME]/code/api/src/main.ts:1: // idempotency for the personal api\n\
         [HOME]/code/side-tool/README.md:1: no idempotency here either\n"
    );
    let out = stdout_of(
        &home,
        home.bare()
            .args([
                "grep",
                "idempotency",
                "--repos",
                "--scope",
                "personal",
                "--json",
            ])
            .current_dir(home.path()),
    );
    insta::assert_snapshot!("grep-personal-json", out);

    let assert = home
        .bare()
        .args([
            "grep",
            "idempotency",
            "--repos",
            "--scope",
            "personal",
            "--limit",
            "1",
        ])
        .current_dir(home.path())
        .assert()
        .success();
    let output = assert.get_output();
    assert_eq!(
        home.scrub(&output.stdout),
        "[HOME]/code/api/src/main.ts:1: // idempotency for the personal api\n"
    );
    assert_eq!(
        home.scrub(&output.stderr),
        "note: stopped after 1 matching lines\n  hint: narrow the pattern, pass `--scope`, or raise `--limit`\n"
    );

    let assert = home
        .bare()
        .args(["grep", "idempotency", "-v"])
        .current_dir(home.path())
        .assert()
        .success();
    let err = home.scrub(&assert.get_output().stderr);
    assert!(err.contains("searching every mounted store\n"), "{err}");
    let assert = home
        .bare()
        .args(["grep", "no-such-text-anywhere", "-v", "--scope", "acme-ml"])
        .current_dir(home.path())
        .assert()
        .success();
    let err = home.scrub(&assert.get_output().stderr);
    assert!(
        err.contains("searching the scope chain of acme-ml\n"),
        "{err}"
    );
    assert!(
        err.contains("note: no match in the scope chain of acme-ml\n"),
        "{err}"
    );

    let (code, err) = failure_of(&home, home.bare().args(["grep", "x", "--scope", "acme-m"]));
    assert_eq!(code, 1);
    assert_eq!(
        err,
        "error: no scope `acme-m` in the mounted stores\n  hint: did you mean `acme-ml`?\n"
    );
    let (code, err) = failure_of(&home, home.bare().args(["grep", "(unclosed"]));
    assert_eq!(code, 1);
    assert!(
        err.starts_with("error: invalid pattern `(unclosed`: "),
        "{err}"
    );
}

/// A fake editor named `hx` (so kipi passes `file:line`): it records its argument in
/// `editor-args`, then replaces `from` with `to` in the file.
fn fake_hx(home: &Home, from: &str, to: &str) -> PathBuf {
    let bin = home.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    let script = bin.join("hx");
    let log = home.path().join("editor-args");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$1\" > '{}'\nf=\"${{1%:*}}\"\nsed 's|{from}|{to}|' \"$f\" > \"$f.tmp\" && mv \"$f.tmp\" \"$f\"\n",
            log.display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    script
}

#[test]
fn editing_a_drafted_summary_marks_it_reviewed() {
    let home = Home::new();
    let hx = fake_hx(
        &home,
        "Invoicing and payment orchestration",
        "Invoices, payments and refunds",
    );
    let out = stdout_of(
        &home,
        home.kipi().args(["edit", "billing-api"]).env("EDITOR", &hx),
    );
    insta::assert_snapshot!("edit-billing-api", out);
    let args = fs::read_to_string(home.path().join("editor-args")).unwrap();
    assert_eq!(
        home.scrub(args.as_bytes()),
        "[HOME]/stores/acme-store/bindings.toml:25\n",
        "opened at the entry's line"
    );
    let bindings =
        fs::read_to_string(home.store(ExampleStore::Acme).join("bindings.toml")).unwrap();
    assert!(
        bindings.contains(
            "summary = \"Invoices, payments and refunds; OpenAPI in api/openapi.yaml\"\naliases"
        ),
        "{bindings}"
    );
    assert!(
        bindings.contains("aliases = [\"feature\", \"store\"]  # kipi:draft"),
        "other drafts stay: {bindings}"
    );
    let status = json_of(
        &home,
        home.kipi()
            .args(["where", "billing-api", "--json"])
            .current_dir(home.path()),
    );
    assert_eq!(status["summary_draft"], false);
}

#[test]
fn edit_creates_notes_from_the_template_and_reports_invalid_edits() {
    let home = Home::new();
    let out = stdout_of(
        &home,
        home.kipi()
            .args(["edit", "notes:ml-platform"])
            .env("EDITOR", "true"),
    );
    let notes = home
        .store(ExampleStore::Acme)
        .join("repos/ml-platform/notes.md");
    assert_eq!(
        out,
        "created [HOME]/stores/acme-store/repos/ml-platform/notes.md\n\
         [HOME]/stores/acme-store/repos/ml-platform/notes.md: valid\n"
    );
    assert!(
        fs::read_to_string(notes)
            .unwrap()
            .starts_with("# ml-platform notes\n")
    );

    let out = stdout_of(
        &home,
        home.kipi()
            .args(["edit", "personal/commit-style"])
            .env("EDITOR", "true"),
    );
    assert_eq!(
        out,
        "[HOME]/stores/personal-store/skills/commit-style/SKILL.md: valid\n"
    );

    let (code, err) = failure_of(
        &home,
        home.kipi()
            .args(["edit", "biling-api"])
            .env("EDITOR", "true"),
    );
    assert_eq!(code, 1);
    assert!(err.starts_with("error: no repo entry or skill `biling-api` in the mounted stores\n  hint: did you mean `billing-api`?"), "{err}");
    let (code, err) = failure_of(
        &home,
        home.kipi()
            .args(["edit", "billing-api"])
            .env("EDITOR", "")
            .env_remove("EDITOR"),
    );
    assert_eq!(code, 1);
    assert!(err.starts_with("error: no editor configured\n"), "{err}");

    let hx = fake_hx(
        &home,
        "^summary = \"Shared training",
        "summry = \"Shared training",
    );
    let (code, err) = failure_of(
        &home,
        home.kipi().args(["edit", "ml-platform"]).env("EDITOR", &hx),
    );
    assert_eq!(code, 1);
    assert!(
        err.contains("unknown key `summry` in [repos.ml-platform]"),
        "{err}"
    );
    assert!(err.contains("hint: did you mean `summary`?"), "{err}");
}

/// An onboarded home after `kipi sync --all`.
fn synced_home() -> Home {
    let home = onboarded_home();
    home.bare().args(["sync", "--all"]).assert().success();
    home
}

#[test]
fn tree_of_both_example_stores_matches_the_sketch_layout() {
    let home = synced_home();
    let out = stdout_of(&home, home.bare().arg("tree"));
    insta::assert_snapshot!("tree", out);
    for expected in [
        "[store personal  ~/stores/personal-store  no git, shares skills]",
        "[store acme  ~/stores/acme-store  no git]",
        "|- code-review ● pr-conventions ● personal/commit-style ●",
        "|- registry (6 repos, ",
        "|- acme-ml  (~/work/acme/ml)  extends acme",
        "skills: code-review ○ pr-conventions ✗ personal/commit-style ○ dbt-conventions ○ \
         stream-recipes ●",
        "related: billing-api ✓ granted   ml-platform ✓ granted",
    ] {
        assert!(out.contains(expected), "missing {expected:?} in\n{out}");
    }
    assert!(
        !out.contains('├'),
        "box drawing falls back to ASCII in a pipe"
    );
    let lines: Vec<&str> = out.lines().filter(|l| l.contains("✓ synced")).collect();
    assert_eq!(lines.len(), 10, "every repo synced:\n{out}");
    let column = |l: &str| l.find(" tok").map(|i| l[..i].chars().count());
    assert!(
        lines.windows(2).all(|w| column(w[0]) == column(w[1])),
        "token counts share one column:\n{out}"
    );
}

#[test]
fn tree_filters_and_display_flags() {
    let home = synced_home();
    let cases: [(&str, &[&str]); 7] = [
        ("tree-repo", &["--repo", "events-pipeline"]),
        ("tree-skill", &["--skill", "commit-style"]),
        ("tree-skill-qualified", &["--skill", "acme/dbt-conventions"]),
        ("tree-store", &["--store", "personal"]),
        ("tree-budget", &["--budget"]),
        ("tree-machine", &["--machine"]),
        (
            "tree-repo-budget-machine",
            &["--repo", "side-tool", "--budget", "--machine"],
        ),
    ];
    for (name, args) in cases {
        let out = stdout_of(&home, home.bare().arg("tree").args(args));
        insta::assert_snapshot!(name, out);
    }
    let out = stdout_of(&home, home.bare().args(["tree", "--store", "personal"]));
    assert!(!out.contains("acme"), "{out}");
    let out = stdout_of(
        &home,
        home.bare().args(["tree", "--repo", "events-pipeline"]),
    );
    assert!(
        !out.contains("billing-api  "),
        "only the repo's own line: {out}"
    );
    assert!(!out.contains("personal  ("), "only the repo's store: {out}");
    let out = stdout_of(
        &home,
        home.bare().args(["tree", "--skill", "stream-recipes"]),
    );
    assert_eq!(
        out.matches("always-on:").count(),
        2,
        "acme and acme-ml: {out}"
    );
    assert_eq!(out.matches("✓ synced").count(), 1, "events-pipeline: {out}");
}

#[test]
fn tree_of_the_personal_store_alone() {
    let home = Home::new();
    let out = stdout_of(&home, home.mounting(&[ExampleStore::Personal]).arg("tree"));
    insta::assert_snapshot!("tree-personal-alone", out);
    assert!(!out.contains("acme"), "no work content: {out}");
    // The example machine.toml overrides the roots: `--machine` shows this machine's.
    let out = stdout_of(
        &home,
        home.mounting(&[ExampleStore::Personal])
            .args(["tree", "--machine"]),
    );
    insta::assert_snapshot!("tree-personal-alone-machine", out);
    assert!(out.contains("personal  (/srv/code)"), "{out}");
    assert!(out.contains("side-tool  /srv/code/side-tool"), "{out}");
}

#[test]
fn tree_json_has_the_same_structure() {
    let home = synced_home();
    let doc = json_of(&home, home.bare().args(["tree", "--json"]));
    insta::assert_snapshot!("tree-json", serde_json::to_string_pretty(&doc).unwrap());
    assert_eq!(doc["version"], 1);
    assert_eq!(doc["tokenizer"], "cl100k_base");
    let acme = &doc["stores"][1];
    assert_eq!(acme["name"], "acme");
    let ml = &acme["scopes"][0]["scopes"][0];
    assert_eq!(ml["name"], "acme-ml");
    let events = ml["repos"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "events-pipeline")
        .unwrap();
    assert_eq!(events["state"], "synced");
    let marks: Vec<String> = events["skills"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            format!(
                "{} {}",
                s["reference"].as_str().unwrap(),
                s["mark"].as_str().unwrap()
            )
        })
        .collect();
    assert_eq!(
        marks,
        [
            "code-review inherited",
            "pr-conventions excluded",
            "personal/commit-style inherited",
            "dbt-conventions inherited",
            "stream-recipes explicit"
        ]
    );
    assert_eq!(events["related"][0]["access"], "granted");
    let tokens = events["always_on"]["tokens"].as_u64().unwrap();
    let status = json_of(
        &home,
        home.bare().args(["status", "events-pipeline", "--json"]),
    );
    assert_eq!(
        status["repos"][0]["always_on"]["tokens"], tokens,
        "tree and status agree"
    );
}

#[test]
fn tree_filter_errors_are_user_errors_with_hints() {
    let home = Home::new();
    let (code, err) = failure_of(&home, home.kipi().args(["tree", "--store", "acmee"]));
    assert_eq!(code, 1);
    assert!(
        err.contains("no mounted store `acmee`") && err.contains("`acme`"),
        "{err}"
    );
    let (code, err) = failure_of(&home, home.kipi().args(["tree", "--skill", "code-reveiw"]));
    assert_eq!(code, 1);
    assert!(err.contains("did you mean `code-review`?"), "{err}");
    let (code, err) = failure_of(
        &home,
        home.kipi().args(["tree", "--skill", "acme/rust-style"]),
    );
    assert_eq!(code, 1);
    assert!(err.contains("`rust-style` is in store `personal`"), "{err}");
    let (code, err) = failure_of(&home, home.kipi().args(["tree", "--repo", "billing"]));
    assert_eq!(code, 1);
    assert!(err.contains("alias of `billing-api`"), "{err}");
    edit_bindings(
        &home,
        ExampleStore::Acme,
        "skills = [\"stream-recipes\"]\n",
        "",
    );
    let out = stdout_of(
        &home,
        home.kipi().args(["tree", "--skill", "stream-recipes"]),
    );
    assert_eq!(out, "no repo uses skill `stream-recipes`\n");
}

/// Roadmap §10 / §14: `tree` under 100 ms for 50 repos with a warm token cache. The bound
/// is generous for a release build; the best of five runs is printed for the record.
#[test]
fn tree_of_fifty_repos_is_fast_with_a_warm_cache() {
    let dir = tempfile::tempdir().unwrap();
    let many = many_repos(dir.path(), 50).unwrap();
    let kipi = || {
        let mut cmd = Command::cargo_bin("kipi").unwrap();
        cmd.env("HOME", &many.home)
            .env_remove("KIPI_CONFIG_DIR")
            .env_remove("KIPI_CACHE_DIR")
            .env_remove("KIPI_STORE");
        cmd
    };
    kipi().args(["sync", "--all"]).assert().success();
    let first = kipi()
        .arg("tree")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let first = String::from_utf8(first).unwrap();
    assert_eq!(first.matches("✓ synced").count(), 50, "{first}");
    assert!(
        dir.path()
            .join(".cache/kipi/tokens.cl100k_base.json")
            .is_file()
    );
    let best = (0..5)
        .map(|_| {
            let start = std::time::Instant::now();
            kipi().arg("tree").assert().success();
            start.elapsed()
        })
        .min()
        .unwrap();
    eprintln!("kipi tree, 50 repos, warm cache: best of 5 = {best:?}");
    assert!(
        best < std::time::Duration::from_millis(100),
        "kipi tree took {best:?} for 50 repos"
    );
}
