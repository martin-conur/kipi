//! End to end: keeping a store in step across two machines through a local bare remote
//! (`kipi init --clone`, `kipi push`, `kipi pull`, `kipi stores`). Each machine is its
//! own temporary home: its own config dir, store checkout and repo checkouts.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as Process;

use assert_cmd::Command;
use kipi_test_support::repos::{FakeRepo, ManifestKind};
use tempfile::TempDir;
use walkdir::WalkDir;

/// Git's identity and default branch, so commits work under an empty home.
const GIT_ENV: [(&str, &str); 8] = [
    ("GIT_AUTHOR_NAME", "Test"),
    ("GIT_AUTHOR_EMAIL", "test@example.com"),
    ("GIT_COMMITTER_NAME", "Test"),
    ("GIT_COMMITTER_EMAIL", "test@example.com"),
    ("GIT_CONFIG_NOSYSTEM", "1"),
    ("GIT_CONFIG_COUNT", "1"),
    ("GIT_CONFIG_KEY_0", "init.defaultBranch"),
    ("GIT_CONFIG_VALUE_0", "main"),
];

/// `git -C <dir> <args>`, which must succeed; its stdout.
fn git(dir: &Path, args: &[&str]) -> String {
    let out = Process::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .envs(GIT_ENV)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

/// A bare repository to push to, under `dir`.
fn bare_remote(dir: &Path, name: &str) -> PathBuf {
    let remote = dir.join(name);
    fs::create_dir_all(&remote).unwrap();
    git(&remote, &["init", "--quiet", "--bare"]);
    remote
}

/// One machine: a home with its own config dir and checkouts.
struct Machine {
    home: TempDir,
}

impl Machine {
    fn new() -> Self {
        Machine {
            home: tempfile::tempdir().unwrap(),
        }
    }

    fn path(&self) -> &Path {
        self.home.path()
    }

    fn store(&self) -> PathBuf {
        self.path().join(".agents/store")
    }

    /// `kipi` on this machine.
    fn kipi(&self) -> Command {
        let mut cmd = Command::cargo_bin("kipi").expect("kipi binary is built");
        cmd.env("HOME", self.path())
            .env_remove("KIPI_CONFIG_DIR")
            .env_remove("KIPI_CACHE_DIR")
            .env_remove("KIPI_STORE")
            .env_remove("KIPI_COMPLETE")
            .envs(GIT_ENV);
        cmd
    }

    /// Run `kipi <args>`, which must succeed; its stdout.
    fn ok(&self, args: &[&str]) -> String {
        let out = self
            .kipi()
            .args(args)
            .assert()
            .success()
            .get_output()
            .clone();
        String::from_utf8(out.stdout).unwrap()
    }

    /// Run `kipi <args>`, which must fail; `(exit code, stdout, stderr)`.
    fn fails(&self, args: &[&str]) -> (i32, String, String) {
        let out = self
            .kipi()
            .args(args)
            .assert()
            .failure()
            .get_output()
            .clone();
        (
            out.status.code().unwrap(),
            String::from_utf8(out.stdout).unwrap(),
            String::from_utf8(out.stderr).unwrap(),
        )
    }

    /// A checkout under `code/<name>` with an origin on a placeholder host.
    fn checkout(&self, name: &str) -> PathBuf {
        FakeRepo {
            entry: String::new(),
            dir: PathBuf::from("code").join(name),
            origin_url: Some(format!("https://github.com/example-user/{name}.git")),
            readme: Some(format!("The {name} project.\n")),
            manifest: ManifestKind::None,
            description: None,
        }
        .create_under(self.path())
        .unwrap()
    }
}

/// Machine A: a personal store with scope `personal` over `~/code`, two scanned repos,
/// committed and pushed to `remote` by hand (the first push sets the upstream).
fn first_machine(remote: &Path) -> Machine {
    let a = Machine::new();
    a.checkout("app");
    a.checkout("lib");
    a.ok(&["init", "--git"]);
    let code = a.path().join("code");
    a.ok(&["add", "scope", "personal", "--root", code.to_str().unwrap()]);
    a.ok(&["scan"]);
    let store = a.store();
    git(
        &store,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git(&store, &["add", "-A"]);
    git(&store, &["commit", "--quiet", "-m", "initial store"]);
    git(&store, &["push", "--quiet", "-u", "origin", "main"]);
    a
}

/// Every file under `dir` but `.git/`, with its text.
fn files_under(dir: &Path) -> Vec<(PathBuf, String)> {
    WalkDir::new(dir)
        .into_iter()
        .filter_entry(|e| e.file_name() != ".git")
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .map(|e| {
            let text = fs::read_to_string(e.path()).unwrap_or_default();
            (e.path().strip_prefix(dir).unwrap().to_path_buf(), text)
        })
        .collect()
}

fn commit_count(dir: &Path) -> usize {
    git(dir, &["rev-list", "--count", "HEAD"])
        .trim()
        .parse()
        .unwrap()
}

#[test]
fn a_change_pushed_on_one_machine_is_pulled_and_synced_on_the_other() {
    let remotes = tempfile::tempdir().unwrap();
    let remote = bare_remote(remotes.path(), "store.git");
    let a = first_machine(&remote);

    // Machine B: its own home, config dir and checkouts; the store comes from the remote.
    let b = Machine::new();
    let b_app = b.checkout("app");
    b.checkout("lib");
    let b_store = b.store();
    let out = b.ok(&[
        "init",
        "--store",
        b_store.to_str().unwrap(),
        "--clone",
        remote.to_str().unwrap(),
    ]);
    assert!(out.contains("+ git clone:"), "{out}");
    assert!(
        out.contains("+ config.toml stores: ~/.agents/store"),
        "{out}"
    );
    assert!(
        out.contains("Next: `kipi scan`, then `kipi sync --all`."),
        "{out}"
    );
    assert!(
        !b_app.join("CLAUDE.md").exists(),
        "init --clone runs neither scan nor sync"
    );
    b.ok(&["scan"]);
    b.ok(&["sync", "--all"]);
    let stores = b.ok(&["stores"]);
    assert!(stores.contains("clean"), "{stores}");

    // Machine A edits a summary and adds a skill, then pushes.
    let bindings = a.store().join("bindings.toml");
    let text = fs::read_to_string(&bindings).unwrap();
    let edited = text.replacen(
        "[repos.lib]\n",
        "[repos.lib]\nsummary = \"Shared parsing helpers for app\"\n",
        1,
    );
    let edited = strip_lib_draft_summary(&edited);
    fs::write(&bindings, edited).unwrap();
    a.ok(&["add", "skill", "review", "--scope", "personal"]);
    let stores = a.ok(&["stores"]);
    assert!(stores.contains("dirty"), "{stores}");

    let out = a.ok(&["push"]);
    assert!(out.contains("pushed 1 commit"), "{out}");
    let message = git(&a.store(), &["log", "-1", "--format=%B"]);
    assert!(
        message
            .starts_with("kipi: update store personal: repos lib; skills review; scopes personal"),
        "{message}"
    );
    assert!(
        a.ok(&["stores"]).contains("clean"),
        "nothing left to commit or push"
    );

    // Machine B pulls: the store fast-forwards and its repos are synced.
    let out = b.ok(&["pull"]);
    assert!(out.contains("personal  pulled 1 commit"), "{out}");
    let block = fs::read_to_string(b_app.join("CLAUDE.md")).unwrap();
    assert!(block.contains("Shared parsing helpers for app"), "{block}");
    assert_eq!(
        fs::read_link(b_app.join(".claude/skills/review")).unwrap(),
        b_store.join("skills/review")
    );

    // Pulling again changes nothing.
    let out = b.ok(&["pull", "--no-sync"]);
    assert_eq!(out, "personal  up to date\n");

    // The pushed store holds no machine-only state.
    let inspect = remotes.path().join("inspect");
    git(
        remotes.path(),
        &[
            "clone",
            "--quiet",
            remote.to_str().unwrap(),
            inspect.to_str().unwrap(),
        ],
    );
    let files = files_under(&inspect);
    assert!(
        files
            .iter()
            .any(|(p, _)| p == Path::new("skills/review/SKILL.md"))
    );
    for (path, text) in &files {
        let name = path.file_name().unwrap().to_string_lossy();
        assert!(
            !["machine.toml", "config.toml", "manifest.toml"].contains(&name.as_ref()),
            "{} is machine-only",
            path.display()
        );
        assert!(!path.starts_with("generated"), "{}", path.display());
        for machine in [&a, &b] {
            let home = machine.path().display().to_string();
            assert!(
                !text.contains(&home),
                "{} holds a home path",
                path.display()
            );
        }
        for prefix in ["Users", "home"] {
            assert!(
                !text.contains(&format!("/{prefix}/")),
                "{} holds an absolute home path",
                path.display()
            );
        }
        // machine.toml's own tables: `[repos]` maps entries to paths, `[roots]` overrides.
        assert!(!text.contains("\n[roots]"), "{}", path.display());
        assert!(
            !text.lines().any(|l| l.trim() == "[repos]"),
            "{}",
            path.display()
        );
    }
}

/// `text` without a drafted `summary` for `lib` (scan drafts one from the README), so
/// the hand-written summary is the only one.
fn strip_lib_draft_summary(text: &str) -> String {
    let mut out = String::new();
    let mut in_lib = false;
    let mut seen = false;
    for line in text.lines() {
        if line.starts_with('[') {
            in_lib = line == "[repos.lib]";
        }
        if in_lib && line.starts_with("summary") {
            if seen {
                continue;
            }
            seen = true;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// A machine with two git stores, `one` and `two`, each pushed to its own remote.
fn two_store_machine(remotes: &Path) -> (Machine, [PathBuf; 2]) {
    let m = Machine::new();
    let mut paths = Vec::new();
    for name in ["one", "two"] {
        let store = m.path().join(format!("stores/{name}"));
        m.ok(&["init", "--store", store.to_str().unwrap(), "--git"]);
        let remote = bare_remote(remotes, &format!("{name}.git"));
        git(
            &store,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        git(&store, &["add", "-A"]);
        git(&store, &["commit", "--quiet", "-m", "initial"]);
        git(&store, &["push", "--quiet", "-u", "origin", "main"]);
        paths.push(store);
    }
    (m, paths.try_into().unwrap())
}

#[test]
fn a_diverged_store_stops_with_a_hint_and_the_others_still_pull() {
    let remotes = tempfile::tempdir().unwrap();
    let (a, [a_one, a_two]) = two_store_machine(remotes.path());
    let b = Machine::new();
    let mut b_stores = Vec::new();
    for name in ["one", "two"] {
        let store = b.path().join(format!("stores/{name}"));
        let remote = remotes.path().join(format!("{name}.git"));
        b.ok(&[
            "init",
            "--store",
            store.to_str().unwrap(),
            "--clone",
            remote.to_str().unwrap(),
        ]);
        b_stores.push(store);
    }
    let [b_one, b_two] = [&b_stores[0], &b_stores[1]];

    // A pushes to both stores; B commits to `one` on its own and leaves an edit uncommitted.
    for store in [&a_one, &a_two] {
        fs::write(store.join("conventions/shared.md"), "From machine A.\n").unwrap();
    }
    a.ok(&["push"]);
    // git keeps no empty directory, so the clone has no conventions/ yet.
    fs::create_dir_all(b_one.join("conventions")).unwrap();
    fs::write(b_one.join("conventions/local.md"), "From machine B.\n").unwrap();
    git(b_one, &["add", "-A"]);
    git(b_one, &["commit", "--quiet", "-m", "B's own change"]);
    fs::write(b_one.join("conventions/draft.md"), "Uncommitted.\n").unwrap();
    let head = git(b_one, &["rev-parse", "HEAD"]);

    let (code, stdout, stderr) = b.fails(&["pull"]);
    assert_eq!(code, 1);
    assert!(
        stdout.contains("one  diverged from its upstream (1 ahead, 1 behind); nothing was changed"),
        "{stdout}"
    );
    assert!(stdout.contains("git pull --rebase"), "{stdout}");
    assert!(stdout.contains("two  pulled 1 commit"), "{stdout}");
    assert!(stderr.contains("pull stopped for store `one`"), "{stderr}");
    assert!(stderr.contains("hint:"), "{stderr}");

    assert_eq!(git(b_one, &["rev-parse", "HEAD"]), head, "one is untouched");
    assert!(!b_one.join("conventions/shared.md").exists());
    assert_eq!(
        fs::read_to_string(b_one.join("conventions/draft.md")).unwrap(),
        "Uncommitted.\n"
    );
    assert_eq!(
        fs::read_to_string(b_two.join("conventions/shared.md")).unwrap(),
        "From machine A.\n"
    );

    // --store limits the pull; --json reports each store.
    let out = b
        .kipi()
        .args(["pull", "--store", "two", "--no-sync", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let doc: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(doc["stores"].as_array().unwrap().len(), 1);
    assert_eq!(doc["stores"][0]["name"], "two");
    assert_eq!(doc["stores"][0]["result"], "up_to_date");
    assert_eq!(doc["sync"], serde_json::Value::Null);
}

#[test]
fn push_is_refused_without_an_upstream_and_on_a_doctor_error() {
    let m = Machine::new();
    m.checkout("app");
    m.ok(&["init", "--git"]);
    let code = m.path().join("code");
    m.ok(&["add", "scope", "personal", "--root", code.to_str().unwrap()]);
    m.ok(&["scan"]);
    let store = m.store();

    // No upstream: named, the store is refused; implicit, it is skipped. Nothing commits.
    let (code, stdout, stderr) = m.fails(&["push", "--store", "personal"]);
    assert_eq!(code, 1);
    assert!(stdout.contains("its branch has no upstream"), "{stdout}");
    assert!(stderr.contains("push -u origin main"), "{stderr}");
    let out = m.ok(&["push"]);
    assert!(out.contains("skipped: its branch has no upstream"), "{out}");
    assert!(
        Process::new("git")
            .arg("-C")
            .arg(&store)
            .args(["rev-parse", "--verify", "--quiet", "HEAD"])
            .output()
            .unwrap()
            .stdout
            .is_empty(),
        "nothing was committed"
    );

    // With an upstream, an absolute home path in the store (K006) still blocks the push.
    let remotes = tempfile::tempdir().unwrap();
    let remote = bare_remote(remotes.path(), "store.git");
    git(
        &store,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git(&store, &["add", "-A"]);
    git(&store, &["commit", "--quiet", "-m", "initial"]);
    git(&store, &["push", "--quiet", "-u", "origin", "main"]);
    let leaked = format!("Clone into /{}/{}/code.\n", "Users", "someone");
    fs::write(store.join("conventions/personal.md"), leaked).unwrap();
    let (code, stdout, _) = m.fails(&["push"]);
    assert_eq!(code, 2, "doctor errors exit 2");
    assert!(
        stdout.contains("doctor found error-level problems"),
        "{stdout}"
    );
    assert!(stdout.contains("K006"), "{stdout}");
    assert_eq!(commit_count(&store), 1, "nothing was committed");
    assert_eq!(
        git(&store, &["rev-parse", "@{u}"]),
        git(&store, &["rev-parse", "HEAD"])
    );

    // Fixed, it goes through; the dry run first shows the commit and changes nothing.
    fs::write(
        store.join("conventions/personal.md"),
        "Clone into ~/code.\n",
    )
    .unwrap();
    let out = m.ok(&["push", "--dry-run", "-m", "Add conventions"]);
    assert!(out.contains("would push 1 commit"), "{out}");
    assert!(out.contains("would commit: Add conventions"), "{out}");
    assert_eq!(commit_count(&store), 1);
    let out = m.ok(&["push", "-m", "Add conventions"]);
    assert!(out.contains("pushed 1 commit"), "{out}");
    assert_eq!(
        git(&store, &["log", "-1", "--format=%s"]),
        "Add conventions\n"
    );
    assert_eq!(m.ok(&["push"]), "personal  up to date\n");
}

#[test]
fn generated_files_are_never_committed() {
    let remotes = tempfile::tempdir().unwrap();
    let remote = bare_remote(remotes.path(), "store.git");
    let a = first_machine(&remote);
    let store = a.store();
    a.ok(&["sync", "--all"]);
    assert!(store.join("generated").read_dir().unwrap().next().is_some());
    // Even with the .gitignore line gone, generated/ stays out of the commit.
    fs::write(store.join(".gitignore"), "").unwrap();
    a.ok(&["push"]);
    let tracked = git(&store, &["ls-files"]);
    assert!(!tracked.contains("generated/"), "{tracked}");
    assert!(tracked.contains(".gitignore"), "{tracked}");
}

#[test]
fn init_clone_needs_an_empty_dir_a_store_and_a_free_name() {
    let remotes = tempfile::tempdir().unwrap();
    let remote = bare_remote(remotes.path(), "store.git");
    let a = first_machine(&remote);

    // A non-empty directory is refused before anything is cloned.
    let b = Machine::new();
    let target = b.path().join("store");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("x"), "x").unwrap();
    let (code, _, stderr) = b.fails(&[
        "init",
        "--store",
        target.to_str().unwrap(),
        "--clone",
        remote.to_str().unwrap(),
    ]);
    assert_eq!(code, 1);
    assert!(stderr.contains("is not empty"), "{stderr}");

    // A repository that is not a store is refused, and the clone is kept.
    let other = remotes.path().join("other");
    fs::create_dir_all(&other).unwrap();
    git(&other, &["init", "--quiet"]);
    fs::write(other.join("README.md"), "x").unwrap();
    git(&other, &["add", "-A"]);
    git(&other, &["commit", "--quiet", "-m", "x"]);
    let dest = b.path().join("not-a-store");
    let (_, _, stderr) = b.fails(&[
        "init",
        "--store",
        dest.to_str().unwrap(),
        "--clone",
        other.to_str().unwrap(),
    ]);
    assert!(stderr.contains("it is not a kipi store"), "{stderr}");
    assert!(dest.join("README.md").is_file());

    // A clone whose name collides with a mounted store is not mounted.
    let dest = a.path().join("second-copy");
    let (_, _, stderr) = a.fails(&[
        "init",
        "--store",
        dest.to_str().unwrap(),
        "--clone",
        remote.to_str().unwrap(),
    ]);
    assert!(stderr.contains("personal"), "{stderr}");
    assert!(stderr.contains("was kept and is not mounted"), "{stderr}");
    let config = fs::read_to_string(a.path().join(".config/kipi/config.toml")).unwrap_or_default();
    assert!(!config.contains("second-copy"), "{config}");

    // --dry-run clones nothing.
    let dest = b.path().join("dry");
    let out = b.ok(&[
        "init",
        "--store",
        dest.to_str().unwrap(),
        "--clone",
        remote.to_str().unwrap(),
        "--dry-run",
    ]);
    assert!(out.contains("+ git clone:"), "{out}");
    assert!(!dest.exists());
}

#[test]
fn stores_shows_each_store_s_git_state() {
    let remotes = tempfile::tempdir().unwrap();
    let (m, [one, _]) = two_store_machine(remotes.path());
    let plain = m.path().join("stores/plain");
    m.ok(&["init", "--store", plain.to_str().unwrap()]);
    fs::write(one.join("conventions/x.md"), "x\n").unwrap();
    git(&one, &["add", "-A"]);
    git(&one, &["commit", "--quiet", "-m", "local"]);
    let out = m.ok(&["stores"]);
    assert!(out.contains("clean, 1 ahead"), "{out}");
    assert!(out.contains("not a git repo"), "{out}");

    let out = m
        .kipi()
        .args(["stores", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let doc: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let git_of = |name: &str| {
        doc["stores"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["name"] == name)
            .unwrap()["git"]
            .clone()
    };
    assert_eq!(
        git_of("one"),
        serde_json::json!({"clean": true, "remote": true, "upstream": true, "ahead": 1, "behind": 0})
    );
    assert_eq!(git_of("plain"), serde_json::Value::Null);
}
