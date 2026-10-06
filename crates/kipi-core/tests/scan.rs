//! `scan` end to end over real (fake) git checkouts: the example stores in a temporary
//! home, with the example repos laid out under it as on a machine.

use std::fs;
use std::path::{Path, PathBuf};

use kipi_core::config::{self, Env};
use kipi_core::model::RemoteUrl;
use kipi_core::scan::{self, Scan, ScanOptions, Warning, draft_aliases, draft_summary};
use kipi_test_support::repos::{FakeRepo, ManifestKind, example_repos, https_url, lay_out};
use kipi_test_support::stores::{ExampleStore, copy_example_store_to};
use kipi_test_support::workspace_root;
use tempfile::TempDir;

/// A simulated machine: a home with copies of the stores and the example checkouts.
struct Machine {
    home: TempDir,
    stores: Vec<ExampleStore>,
}

impl Machine {
    /// The stores as shipped, every example repo checked out, no machine.toml.
    fn new(stores: &[ExampleStore]) -> Self {
        let m = Machine {
            home: tempfile::tempdir().unwrap(),
            stores: stores.to_vec(),
        };
        for &s in stores {
            copy_example_store_to(s, &m.store(s)).unwrap();
        }
        lay_out(m.path(), &example_repos(stores).unwrap()).unwrap();
        m
    }

    /// Like [`Machine::new`], with every `[repos.*]` removed from the stores.
    fn without_entries(stores: &[ExampleStore]) -> Self {
        let m = Machine::new(stores);
        for &s in stores {
            let file = m.store(s).join("bindings.toml");
            let mut doc: toml_edit::DocumentMut =
                fs::read_to_string(&file).unwrap().parse().unwrap();
            doc.remove("repos");
            fs::write(&file, doc.to_string()).unwrap();
        }
        m
    }

    fn path(&self) -> &Path {
        self.home.path()
    }

    fn store(&self, s: ExampleStore) -> PathBuf {
        self.path().join("stores").join(s.dir_name())
    }

    fn env(&self) -> Env {
        Env::from_lookup(|_| None, Some(self.path().to_path_buf())).unwrap()
    }

    fn scan(&self, opts: &ScanOptions) -> Scan {
        let env = self.env();
        let paths: Vec<PathBuf> = self.stores.iter().map(|&s| self.store(s)).collect();
        let set = config::load_stores(&paths, &env).unwrap_or_else(|e| panic!("{}", e.report()));
        let machine = config::load_machine(&env).unwrap_or_else(|e| panic!("{}", e.report()));
        scan::scan(&set, &machine, &env.machine_file(), opts).unwrap()
    }

    /// Scan and write.
    fn apply(&self, opts: &ScanOptions) -> Scan {
        let out = self.scan(opts);
        scan::write_files(&out.writes).unwrap();
        out
    }

    fn bindings(&self, s: ExampleStore) -> String {
        fs::read_to_string(self.store(s).join("bindings.toml")).unwrap()
    }

    fn machine_toml(&self) -> String {
        fs::read_to_string(self.env().machine_file())
            .unwrap()
            .replace(&self.path().display().to_string(), "[HOME]")
    }

    fn checkout(&self, repo: FakeRepo) -> PathBuf {
        repo.create_under(self.path()).unwrap()
    }
}

fn fake(dir: &str, origin: Option<&str>, readme: Option<&str>) -> FakeRepo {
    FakeRepo {
        entry: String::new(),
        dir: dir.into(),
        origin_url: origin.map(str::to_owned),
        readme: readme.map(str::to_owned),
        manifest: ManifestKind::None,
        description: None,
    }
}

fn refresh() -> ScanOptions {
    ScanOptions {
        refresh_drafts: true,
        ..ScanOptions::default()
    }
}

fn codes(scan: &Scan) -> Vec<(&'static str, Option<&str>)> {
    scan.warnings
        .iter()
        .map(|w| (w.code(), w.entry()))
        .collect()
}

#[test]
fn drafts_an_entry_for_every_example_checkout() {
    let m = Machine::without_entries(&ExampleStore::ALL);
    let out = m.apply(&ScanOptions::default());
    assert_eq!(out.checkouts.len(), 10);
    assert_eq!(out.added.len(), 10, "{:?}", out.warnings);
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    let added: Vec<(&str, &str, &str)> = out
        .added
        .iter()
        .map(|a| (a.name.as_str(), a.store.as_str(), a.scope.as_str()))
        .collect();
    assert_eq!(
        added,
        [
            ("api", "personal", "personal"),
            ("blog", "personal", "personal"),
            ("dotfiles", "personal", "personal"),
            ("side-tool", "personal", "personal"),
            // `api` is taken by the personal checkout: `<owner>-<name>`.
            ("acme-org-api", "acme", "acme"),
            ("billing-api", "acme", "acme"),
            ("events-pipeline", "acme", "acme-ml"),
            ("feature-store", "acme", "acme-ml"),
            ("ml-platform", "acme", "acme-ml"),
            ("web-dashboard", "acme", "acme"),
        ]
    );
    insta::assert_snapshot!("drafted-acme", m.bindings(ExampleStore::Acme));
    insta::assert_snapshot!("drafted-personal", m.bindings(ExampleStore::Personal));
    insta::assert_snapshot!("drafted-machine", m.machine_toml());

    // The new entries load, and every one is found on this machine.
    assert!(out.not_found.is_empty());
    let again = m.scan(&ScanOptions::default());
    assert!(again.writes.is_empty(), "re-running scan is a no-op");
    assert!(again.added.is_empty() && again.recorded.is_empty());
    let refreshed = m.scan(&refresh());
    assert!(refreshed.writes.is_empty(), "{:?}", refreshed.refreshed);
}

#[test]
fn the_example_stores_drafts_are_what_scan_drafts() {
    let m = Machine::new(&ExampleStore::ALL);
    let out = m.apply(&refresh());
    assert!(out.added.is_empty(), "every checkout matches an entry");
    assert!(out.remotes_filled.is_empty());
    assert!(out.refreshed.is_empty(), "{:?}", out.refreshed);
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    assert_eq!(out.recorded.len(), 10);
    assert_eq!(out.writes.len(), 1, "only machine.toml is written");
    for s in ExampleStore::ALL {
        let shipped = fs::read_to_string(s.source_path().join("bindings.toml")).unwrap();
        assert_eq!(m.bindings(s), shipped);
    }
}

#[test]
fn refresh_rewrites_drafts_and_never_reviewed_values() {
    let m = Machine::new(&ExampleStore::ALL);
    m.apply(&ScanOptions::default());
    // billing-api's summary is a draft; ml-platform's is reviewed.
    for (dir, file, key) in [
        ("work/acme/billing-api", "Cargo.toml", "package"),
        ("work/acme/ml/ml-platform", "pyproject.toml", "project"),
    ] {
        let path = m.path().join(dir).join(file);
        let mut doc: toml_edit::DocumentMut = fs::read_to_string(&path).unwrap().parse().unwrap();
        doc[key]["description"] = toml_edit::value("Rewritten upstream");
        fs::write(&path, doc.to_string()).unwrap();
    }
    let plain = m.scan(&ScanOptions::default());
    assert!(
        plain.writes.is_empty(),
        "without --refresh-drafts nothing changes"
    );

    let out = m.apply(&refresh());
    let refreshed: Vec<(&str, &str, &str)> = out
        .refreshed
        .iter()
        .map(|r| (r.entry.as_str(), r.field, r.value.as_str()))
        .collect();
    assert_eq!(
        refreshed,
        [("billing-api", "summary", "\"Rewritten upstream\"")]
    );
    let text = m.bindings(ExampleStore::Acme);
    assert!(
        text.contains("summary = \"Rewritten upstream\"  # kipi:draft\n"),
        "{text}"
    );
    assert!(text.contains("summary = \"Shared training and feature tooling\"\n"));
    assert!(
        m.scan(&refresh()).writes.is_empty(),
        "stable once refreshed"
    );
}

#[test]
fn two_machines_get_the_same_entries_and_their_own_machine_toml() {
    let a = Machine::without_entries(&ExampleStore::ALL);
    let b = Machine::without_entries(&ExampleStore::ALL);
    // Machine B keeps its work checkouts somewhere else, via a root override.
    fs::rename(b.path().join("work"), b.path().join("elsewhere")).unwrap();
    let cfg = b.env().config_dir;
    fs::create_dir_all(&cfg).unwrap();
    let elsewhere = b.path().join("elsewhere/acme");
    fs::write(
        cfg.join("machine.toml"),
        format!(
            "hostname = \"laptop-b\"\n\n[roots]\nacme = [\"{0}\"]\nacme-ml = [\"{0}/ml\"]\n",
            elsewhere.display()
        ),
    )
    .unwrap();
    a.apply(&ScanOptions::default());
    b.apply(&ScanOptions::default());
    for s in ExampleStore::ALL {
        assert_eq!(a.bindings(s), b.bindings(s), "{}", s.name());
    }
    let (ma, mb) = (a.machine_toml(), b.machine_toml());
    assert_ne!(ma, mb);
    assert!(mb.starts_with("hostname = \"laptop-b\""), "{mb}");
    assert!(
        mb.contains("billing-api = \"[HOME]/elsewhere/acme/billing-api\""),
        "{mb}"
    );
    assert!(
        ma.contains("billing-api = \"[HOME]/work/acme/billing-api\""),
        "{ma}"
    );
}

#[test]
fn fills_missing_remotes_and_matches_by_directory_name() {
    let m = Machine::new(&[ExampleStore::Personal]);
    let file = m.store(ExampleStore::Personal).join("bindings.toml");
    let text = fs::read_to_string(&file).unwrap();
    fs::write(
        &file,
        text.replace("remote = \"github.com/example-user/blog\"\n", ""),
    )
    .unwrap();
    let out = m.apply(&ScanOptions::default());
    assert!(out.added.is_empty());
    assert_eq!(out.remotes_filled.len(), 1);
    assert_eq!(out.remotes_filled[0].entry, "blog");
    assert_eq!(
        out.remotes_filled[0].remote.as_str(),
        "github.com/example-user/blog"
    );
    let text = m.bindings(ExampleStore::Personal);
    assert!(
        text.contains("aliases = [\"site\"]\nremote = \"github.com/example-user/blog\"\n"),
        "factual, so no draft marker:\n{text}"
    );
}

#[test]
fn a_name_clash_with_another_remote_becomes_owner_name() {
    let m = Machine::new(&ExampleStore::ALL);
    m.checkout(fake(
        "code/forks/api",
        Some("git@git.example.com:someone/api.git"),
        Some("# api\n\nSomeone else's API.\n"),
    ));
    let out = m.apply(&ScanOptions::default());
    assert_eq!(out.added.len(), 1, "{:?}", out.warnings);
    let added = &out.added[0];
    assert_eq!(
        (added.name.as_str(), added.store.as_str()),
        ("someone-api", "personal")
    );
    assert_eq!(added.summary.as_deref(), Some("Someone else's API"));
    assert!(
        added.aliases.is_empty(),
        "drafted from `api`, not from the owner prefix"
    );
    let text = m.bindings(ExampleStore::Personal);
    assert!(
        text.ends_with(
            "\n[repos.someone-api]\nremote = \"git.example.com/someone/api\"\n\
             summary = \"Someone else's API\"  # kipi:draft\n"
        ),
        "{text}"
    );
}

#[test]
fn checkouts_outside_every_root_are_reported_or_placed_with_scope() {
    let m = Machine::new(&ExampleStore::ALL);
    let path = m.checkout(fake(
        "src/scratch",
        Some("https://github.com/example-user/scratch"),
        Some("Throwaway experiments.\n"),
    ));
    let paths = ScanOptions {
        paths: vec![m.path().join("src")],
        ..ScanOptions::default()
    };
    let out = m.scan(&paths);
    assert!(out.added.is_empty());
    assert!(out.writes.is_empty());
    let [Warning::Unplaced { hint, reason, .. }] = out.warnings.as_slice() else {
        panic!("{:?}", out.warnings)
    };
    assert_eq!(reason, "lies outside every scope root");
    assert_eq!(
        hint,
        &format!(
            "its remote matches scopes.personal.remote_patterns; add it with \
             `kipi scan {} --store personal --scope personal`",
            path.display()
        )
    );

    let out = m.apply(&ScanOptions {
        scope: Some("personal".into()),
        ..paths
    });
    assert_eq!(out.added.len(), 1);
    assert!(out.added[0].explicit_scope);
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    assert!(m.bindings(ExampleStore::Personal).ends_with(
        "\n[repos.scratch]\nremote = \"github.com/example-user/scratch\"\nscope = \"personal\"\n\
         summary = \"Throwaway experiments\"  # kipi:draft\n"
    ));
}

#[test]
fn a_remote_matching_another_scopes_patterns_is_a_conflict() {
    let m = Machine::new(&ExampleStore::ALL);
    m.checkout(fake(
        "code/payroll",
        Some("git@github.com:acme-org/payroll.git"),
        None,
    ));
    let out = m.scan(&ScanOptions::default());
    assert_eq!(out.added.len(), 1);
    assert_eq!(out.added[0].store, "personal", "placed by path");
    let [w @ Warning::ScopeConflict { .. }] = out.warnings.as_slice() else {
        panic!("{:?}", out.warnings)
    };
    assert_eq!(
        w.hint(),
        "the entry lives in store `personal`; move [repos.payroll] to store `acme` and add \
         `scope = \"acme\"`"
    );
    assert!(w.message(|p| p.display().to_string()).ends_with(
        "resolves to scope `personal`, but its remote matches scopes.acme.remote_patterns"
    ),);
}

#[test]
fn reports_duplicates_ambiguity_vanished_and_missing_repos() {
    let m = Machine::new(&ExampleStore::ALL);
    m.apply(&ScanOptions::default());
    // A second clone of side-tool; feature-store's checkout disappears; a recorded
    // checkout of blog moves.
    m.checkout(fake(
        "code/copies/side-tool",
        Some(&https_url("github.com/example-user/side-tool")),
        None,
    ));
    fs::remove_dir_all(m.path().join("work/acme/ml/feature-store")).unwrap();
    fs::rename(m.path().join("code/blog"), m.path().join("code/site")).unwrap();
    // Two clones of one new repo.
    for dir in ["code/new-a", "code/new-b"] {
        m.checkout(fake(dir, Some("https://github.com/example-user/new"), None));
    }

    let out = m.apply(&ScanOptions::default());
    assert_eq!(
        codes(&out),
        [
            ("duplicate-checkouts", Some("side-tool")),
            ("duplicate-checkouts", None),
            ("vanished", Some("feature-store")),
        ]
    );
    let Warning::Duplicate { kept, .. } = &out.warnings[0] else {
        unreachable!()
    };
    assert_eq!(
        kept.as_deref(),
        Some(m.path().join("code/side-tool").as_path())
    );
    let moved = &out.recorded[0];
    assert_eq!(moved.entry, "blog");
    assert_eq!(moved.previous, Some(m.path().join("code/blog")));
    assert!(out.added.is_empty());
    assert!(
        out.not_found.is_empty(),
        "feature-store is recorded, only vanished"
    );
    assert!(
        m.machine_toml()
            .contains("feature-store = \"[HOME]/work/acme/ml/feature-store\"")
    );

    // Remove the record by hand: now it is simply not on this machine.
    let file = m.env().machine_file();
    let text = fs::read_to_string(&file).unwrap();
    let kept: Vec<&str> = text
        .lines()
        .filter(|l| !l.starts_with("feature-store"))
        .collect();
    fs::write(&file, kept.join("\n") + "\n").unwrap();
    let out = m.scan(&ScanOptions::default());
    assert_eq!(out.not_found, ["feature-store"]);
}

#[test]
fn a_basename_match_fills_the_remote_of_an_entry_without_one() {
    let m = Machine::new(&[ExampleStore::Personal]);
    let file = m.store(ExampleStore::Personal).join("bindings.toml");
    let text = fs::read_to_string(&file).unwrap();
    fs::write(&file, format!("{text}\n[repos.tool]\n")).unwrap();
    m.checkout(fake(
        "code/x",
        Some("https://git.example.com/me/tool"),
        None,
    ));
    let out = m.scan(&ScanOptions::default());
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    assert!(out.added.is_empty());
    assert_eq!(out.remotes_filled[0].entry, "tool");
}

#[test]
fn two_entries_with_one_remote_are_ambiguous() {
    let m = Machine::new(&[ExampleStore::Personal]);
    let file = m.store(ExampleStore::Personal).join("bindings.toml");
    let text = fs::read_to_string(&file).unwrap();
    fs::write(
        &file,
        format!("{text}\n[repos.blog-copy]\nremote = \"github.com/example-user/blog\"\n"),
    )
    .unwrap();
    let out = m.scan(&ScanOptions::default());
    let [w @ Warning::Ambiguous { candidates, .. }] = out.warnings.as_slice() else {
        panic!("{:?}", out.warnings)
    };
    assert_eq!(candidates, &["blog", "blog-copy"]);
    assert!(
        w.message(|p| p.display().to_string())
            .ends_with("matches several entries by remote: blog, blog-copy")
    );
    assert!(!out.recorded.iter().any(|r| r.entry.starts_with("blog")));
    assert!(
        out.added.is_empty(),
        "never guessed into a new entry either"
    );
}

#[test]
fn drafting_fixture_repos() {
    let root = workspace_root().join("crates/kipi-core/tests/fixtures/repos");
    let mut dirs: Vec<PathBuf> = fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    dirs.sort();
    let mut out = String::new();
    let taken = ["openapi".to_owned()].into_iter().collect();
    for dir in dirs {
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        out.push_str(&format!(
            "[{name}]\nsummary = {:?}\naliases = {:?}\n\n",
            draft_summary(&dir),
            draft_aliases(&name, &taken),
        ));
    }
    insta::assert_snapshot!("drafting-fixtures", out);
    let again: String = draft_summary(&root.join("readme-long")).unwrap();
    assert_eq!(
        Some(again),
        draft_summary(&root.join("readme-long")),
        "deterministic"
    );
}

#[test]
fn remote_urls_of_new_entries_are_normalized() {
    let m = Machine::without_entries(&[ExampleStore::Personal]);
    let out = m.scan(&ScanOptions::default());
    let remotes: Vec<&str> = out
        .added
        .iter()
        .filter_map(|a| a.remote.as_ref().map(RemoteUrl::as_str))
        .collect();
    assert_eq!(
        remotes,
        [
            "github.com/example-user/api",
            "github.com/example-user/blog",
            "github.com/example-user/dotfiles",
            "github.com/example-user/side-tool",
        ]
    );
}
