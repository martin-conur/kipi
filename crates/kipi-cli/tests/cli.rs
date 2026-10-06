//! End to end: the `kipi` binary against the example stores, mounted in a temporary
//! home with the example `machine.toml`. Output is snapshotted with the home replaced
//! by `[HOME]`.

use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use kipi_test_support::repos::{FakeRepo, ManifestKind, example_repos, lay_out};
use kipi_test_support::stores::{ExampleStore, copy_example_store_to};
use kipi_test_support::workspace_root;
use predicates::prelude::*;
use tempfile::TempDir;

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
    let (code, err) = failure_of(&home, home.kipi().args(["sync", "--all", "--dry-run"]));
    assert_eq!(code, 1);
    assert_eq!(
        err,
        "error: `kipi sync` is not implemented yet\n  hint: it lands in Stage 2 (roadmap §14); \
         `kipi resolve` and `kipi explain <repo>` work today\n"
    );
    let (code, err) = failure_of(&home, home.kipi().args(["add", "skill", "x"]));
    assert_eq!(code, 1);
    assert!(err.starts_with("error: `kipi add skill` is not implemented yet"));
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
