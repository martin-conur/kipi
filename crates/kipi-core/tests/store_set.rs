//! Multiple mounted stores (roadmap §6) and repo identity by remote (§5.1, §5.2), on
//! the example stores and fake checkouts.

use std::fs;
use std::path::{Path, PathBuf};

use kipi_core::config::{self, Env};
use kipi_core::error::Error;
use kipi_core::git;
use kipi_core::model::{Checkout, MatchedBy, RemoteUrl, RepoMatch, StoreSet, match_checkout};
use kipi_test_support::repos::{example_repos, lay_out};
use kipi_test_support::stores::{ExampleStore, StoreFixture, copy_example_stores};
use walkdir::WalkDir;

const HOME: &str = "/HOME";

fn env() -> Env {
    Env::from_lookup(|_| None, Some(PathBuf::from(HOME))).unwrap()
}

fn load(paths: &[&Path]) -> kipi_core::error::Result<StoreSet> {
    let paths: Vec<PathBuf> = paths.iter().map(|p| p.to_path_buf()).collect();
    config::load_stores(&paths, &env())
}

/// Both example stores, copied, with `edit` applied to `store`'s `bindings.toml`.
fn both_with(store: ExampleStore, edit: impl Fn(String) -> String) -> StoreFixture {
    let fx = copy_example_stores(&ExampleStore::ALL).unwrap();
    let path = fx.path(store).join("bindings.toml");
    let text = fs::read_to_string(&path).unwrap();
    let edited = edit(text.clone());
    assert_ne!(edited, text, "the edit applies");
    fs::write(&path, edited).unwrap();
    fx
}

/// The error of loading `fx`'s stores, checked for a location and a hint, redacted.
fn error_of(fx: &StoreFixture) -> String {
    let err = load(&fx.paths()).expect_err("the store set is rejected");
    report(&err, fx.root())
}

fn report(err: &Error, root: &Path) -> String {
    assert!(err.location().is_some(), "error has a location: {err}");
    assert!(!err.hint().trim().is_empty(), "error has a hint: {err}");
    err.report().replace(&root.display().to_string(), "<dir>")
}

/// Assert `err` is a conflict naming two different files or lines.
fn assert_two_locations(fx: &StoreFixture) {
    match load(&fx.paths()).err().unwrap() {
        Error::Conflict { at, other, .. } => assert_ne!(at, *other),
        other => panic!("expected a conflict, got {other:?}"),
    }
}

#[test]
fn example_stores_load_together() {
    let set = load(&[
        &ExampleStore::Personal.source_path(),
        &ExampleStore::Acme.source_path(),
    ])
    .unwrap_or_else(|e| panic!("{}", e.report()));
    let names: Vec<&str> = set.stores().iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["personal", "acme"]);
    assert_eq!(set.repos().count(), 10);
    assert_eq!(set.scope("acme-ml").unwrap().0.name, "acme");
    assert_eq!(set.repo("api").unwrap().0.name, "personal");
    assert_eq!(set.repo("acme-api").unwrap().0.name, "acme");

    // The shared skill resolves into the sharing store; bare names stay local.
    let shared = "personal/commit-style".parse().unwrap();
    let (owner, skill) = set.skill("acme", &shared).unwrap();
    assert_eq!(
        (owner.name.as_str(), skill.name.as_str()),
        ("personal", "commit-style")
    );
    assert!(
        set.skill("acme", &"commit-style".parse().unwrap())
            .is_none()
    );
    assert!(
        set.skill("personal", &"acme/code-review".parse().unwrap())
            .is_none()
    );
    assert!(
        set.skill("acme", &"acme/code-review".parse().unwrap())
            .is_some()
    );

    // Every span carries its store.
    let (_, events) = set.repo("events-pipeline").unwrap();
    assert_eq!(events.at.store.as_deref(), Some("acme"));
    let line = events.at.line.unwrap();
    assert_eq!(
        set.provenance(&events.at),
        format!("acme:bindings.toml:{line}")
    );
    assert_eq!(
        set.provenance(&skill.at),
        "personal:skills/commit-style/SKILL.md"
    );
    for store in set.stores() {
        assert_eq!(store.name_at.store.as_deref(), Some(store.name.as_str()));
        assert!(
            store
                .scopes
                .iter()
                .all(|s| s.at.store.as_deref() == Some(store.name.as_str()))
        );
        assert!(
            store
                .skills
                .iter()
                .all(|s| s.at.store.as_deref() == Some(store.name.as_str()))
        );
    }
}

#[test]
fn personal_store_loads_alone_and_holds_no_work_data() {
    let set =
        load(&[&ExampleStore::Personal.source_path()]).unwrap_or_else(|e| panic!("{}", e.report()));
    assert_eq!(set.stores().len(), 1);
    assert_eq!(set.repos().count(), 4);

    let work = config::load_store(&ExampleStore::Acme.source_path(), &env()).unwrap();
    let patterns: Vec<&String> = work
        .scopes
        .iter()
        .flat_map(|s| &s.remote_patterns)
        .collect();
    let mut needles: Vec<String> = vec![work.name.clone()];
    needles.extend(work.repos.iter().map(|r| r.name.clone()));
    needles.extend(work.scopes.iter().map(|s| s.name.clone()));
    needles.extend(patterns.iter().map(|p| p.to_string()));
    for entry in WalkDir::new(ExampleStore::Personal.source_path()) {
        let entry = entry.unwrap();
        if !entry.file_type().is_file() {
            continue;
        }
        let text = fs::read_to_string(entry.path()).unwrap().to_lowercase();
        for needle in &needles {
            assert!(
                !text.contains(&needle.to_lowercase()),
                "{} mentions work data `{needle}`",
                entry.path().display()
            );
        }
    }
    for (_, repo) in set.repos() {
        let remote = repo.remote.as_ref().unwrap().as_str();
        assert!(
            patterns.iter().all(|p| !remote.starts_with(p.as_str())),
            "{remote} is a work remote"
        );
    }
}

#[test]
fn work_store_alone_needs_the_sharing_store() {
    let fx = copy_example_stores(&[ExampleStore::Acme]).unwrap();
    insta::assert_snapshot!(error_of(&fx), @r"
    error: <dir>/acme-store/bindings.toml:13: skill `personal/commit-style` names store `personal`, which is not mounted
      hint: mount store `personal` (--store, KIPI_STORE or config.toml `stores`) or remove the reference
    ");
}

#[test]
fn colliding_store_names() {
    let fx = copy_example_stores(&ExampleStore::ALL).unwrap();
    let second = fx.root().join("personal-copy");
    kipi_test_support::stores::copy_example_store_to(ExampleStore::Personal, &second).unwrap();
    let mut paths = fx.paths();
    paths.push(&second);
    let err = load(&paths).err().unwrap();
    insta::assert_snapshot!(report(&err, fx.root()), @r"
    error: <dir>/personal-copy/bindings.toml:4: store name `personal` is used by two mounted stores
      also at: <dir>/personal-store/bindings.toml:4
      hint: store names are unique among mounted stores; change `[store] name` in one of them (and any `<store>/<skill>` reference to it)
    ");
}

#[test]
fn same_store_mounted_twice() {
    let fx = copy_example_stores(&[ExampleStore::Personal]).unwrap();
    let p = fx.path(ExampleStore::Personal);
    let err = load(&[p, p]).err().unwrap();
    insta::assert_snapshot!(report(&err, fx.root()), @r"
    error: <dir>/personal-store/bindings.toml:4: store `personal` is mounted twice
      also at: <dir>/personal-store/bindings.toml:4
      hint: list each store once across --store, KIPI_STORE and config.toml `stores`
    ");
}

#[test]
fn colliding_scope_names() {
    let fx = both_with(ExampleStore::Acme, |t| {
        t + "\n[scopes.personal]\nroots = [\"~/work/personal\"]\n"
    });
    assert_two_locations(&fx);
    insta::assert_snapshot!(error_of(&fx), @r"
    error: <dir>/acme-store/bindings.toml:59: scope `personal` is declared in stores `personal` and `acme`
      also at: <dir>/personal-store/bindings.toml:13
      hint: scope names are unique across mounted stores (machine.toml [roots] is keyed by them); rename one of the scopes
    ");
}

#[test]
fn colliding_repo_entry_names() {
    // §5.1: acme's `api` checkout must not reuse the personal entry name `api`.
    let fx = both_with(ExampleStore::Acme, |t| {
        t.replace("[repos.acme-api]", "[repos.api]")
    });
    assert_two_locations(&fx);
    insta::assert_snapshot!(error_of(&fx), @r"
    error: <dir>/acme-store/bindings.toml:30: repo entry `api` is declared in stores `personal` and `acme`
      also at: <dir>/personal-store/bindings.toml:26
      hint: entry names are unique across mounted stores (machine.toml [repos] is keyed by them); rename one, e.g. to `<owner>-<name>`, and keep its `remote` so checkouts still match
    ");
}

#[test]
fn scope_references_into_another_store() {
    let extends = both_with(ExampleStore::Acme, |t| {
        t.replace("extends = \"acme\"", "extends = \"personal\"")
    });
    assert_two_locations(&extends);
    insta::assert_snapshot!(error_of(&extends), @r"
    error: <dir>/acme-store/bindings.toml:20: scope `personal` belongs to store `personal`, not to store `acme`
      also at: <dir>/personal-store/bindings.toml:13
      hint: nothing crosses stores: scope references resolve only within the store that owns them; declare `personal` in store `acme` or remove the reference
    ");

    let scope = both_with(ExampleStore::Personal, |t| {
        t.replace(
            "related = [\"api\"]",
            "related = [\"api\"]\nscope = \"acme\"",
        )
    });
    assert_two_locations(&scope);
    insta::assert_snapshot!(error_of(&scope), @r"
    error: <dir>/personal-store/bindings.toml:20: scope `acme` belongs to store `acme`, not to store `personal`
      also at: <dir>/acme-store/bindings.toml:13
      hint: nothing crosses stores: scope references resolve only within the store that owns them; declare `acme` in store `personal` or remove the reference
    ");
}

#[test]
fn related_into_another_store() {
    let fx = both_with(ExampleStore::Personal, |t| {
        t.replace(
            "related = [\"api\"]",
            "related = [\"api\", \"billing-api\"]",
        )
    });
    assert_two_locations(&fx);
    insta::assert_snapshot!(error_of(&fx), @r"
    error: <dir>/personal-store/bindings.toml:20: repo `billing-api` belongs to store `acme`, not to store `personal`
      also at: <dir>/acme-store/bindings.toml:25
      hint: nothing crosses stores: repo references resolve only within the store that owns them; declare `billing-api` in store `personal` or remove the reference
    ");
}

#[test]
fn bare_skill_names_resolve_within_their_store() {
    // A sharing store's skill must be qualified.
    let shared = both_with(ExampleStore::Acme, |t| {
        t.replace("\"personal/commit-style\"]", "\"commit-style\"]")
    });
    assert_two_locations(&shared);
    insta::assert_snapshot!(error_of(&shared), @r"
    error: <dir>/acme-store/bindings.toml:13: skill `commit-style` is not in store `acme`; it belongs to store `personal`
      also at: <dir>/personal-store/skills/commit-style/SKILL.md
      hint: bare skill names resolve within their own store; reference it as `personal/commit-style`
    ");

    // A non-sharing store's skill cannot be reached at all; `exclude` is checked too.
    let private = both_with(ExampleStore::Personal, |t| {
        t.replace(
            "related = [\"api\"]",
            "related = [\"api\"]\nexclude = [\"code-review\"]",
        )
    });
    insta::assert_snapshot!(error_of(&private), @r"
    error: <dir>/personal-store/bindings.toml:20: skill `code-review` is not in store `personal`; it belongs to store `acme`
      also at: <dir>/acme-store/skills/code-review/SKILL.md
      hint: bare skill names resolve within their own store; store `acme` does not share its skills; copy the skill into store `personal` or remove the reference
    ");
}

#[test]
fn qualified_skill_to_a_non_sharing_store() {
    let fx = both_with(ExampleStore::Personal, |t| {
        t.replace(
            "skills = [\"rust-style\", \"commit-style\"]",
            "skills = [\"rust-style\", \"commit-style\", \"acme/code-review\"]",
        )
    });
    assert_two_locations(&fx);
    insta::assert_snapshot!(error_of(&fx), @r"
    error: <dir>/personal-store/bindings.toml:13: skill `acme/code-review` is in store `acme`, which does not share its skills
      also at: <dir>/acme-store/bindings.toml:4
      hint: set `share_skills = true` in store `acme`'s [store] table, or copy the skill into store `personal`
    ");
}

#[test]
fn qualified_skill_missing_in_the_sharing_store() {
    let fx = both_with(ExampleStore::Acme, |t| {
        t.replace("\"personal/commit-style\"]", "\"personal/commit-styles\"]")
    });
    assert_two_locations(&fx);
    insta::assert_snapshot!(error_of(&fx), @r"
    error: <dir>/acme-store/bindings.toml:13: store `personal` has no skill `commit-styles`
      also at: <dir>/personal-store/skills/commit-styles
      hint: check the name against store `personal`'s skills/ directory, or remove the reference
    ");
}

/// Match every checkout under `root` against `set`, returning `(dir, entry)`.
fn match_all(set: &StoreSet, checkouts: &[(String, PathBuf)]) -> Vec<(PathBuf, String)> {
    checkouts
        .iter()
        .map(|(_, path)| {
            let raw = git::origin_url(path)
                .unwrap()
                .expect("example repos have an origin");
            let remote = RemoteUrl::parse(&raw).unwrap();
            let checkout = Checkout {
                dir_name: path.file_name().unwrap().to_str().unwrap(),
                remote: Some(&remote),
            };
            match match_checkout(checkout, set.repos().map(|(_, r)| r)) {
                RepoMatch::Unique { entry, by } => {
                    assert_eq!(by, MatchedBy::Remote, "{}", path.display());
                    (path.clone(), entry.name.clone())
                }
                other => panic!("{}: {other:?}", path.display()),
            }
        })
        .collect()
}

#[test]
fn checkouts_match_by_remote_across_paths_and_directory_names() {
    let set = load(&[
        &ExampleStore::Personal.source_path(),
        &ExampleStore::Acme.source_path(),
    ])
    .unwrap();
    let mut repos = example_repos(&ExampleStore::ALL).unwrap();

    // Machine A: the example layout. acme's `work/acme/api` and personal `code/api`
    // share a directory name; their remotes tell them apart.
    let a = tempfile::tempdir().unwrap();
    let laid = lay_out(a.path(), &repos).unwrap();
    for ((entry, _), (_, matched)) in laid.iter().zip(match_all(&set, &laid)) {
        assert_eq!(&matched, entry);
    }

    // Machine B: every repo under another path and another directory name.
    for (i, r) in repos.iter_mut().enumerate() {
        r.dir = PathBuf::from(format!("src/{i}/clone-{i}"));
    }
    let b = tempfile::tempdir().unwrap();
    let laid = lay_out(b.path(), &repos).unwrap();
    for ((entry, _), (_, matched)) in laid.iter().zip(match_all(&set, &laid)) {
        assert_eq!(&matched, entry);
    }
}

#[test]
fn origin_url_of_a_repo_without_remote_is_none() {
    let dir = tempfile::tempdir().unwrap();
    let status = std::process::Command::new("git")
        .args(["init", "--quiet"])
        .arg(dir.path())
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(git::origin_url(dir.path()).unwrap(), None);
    assert!(git::origin_url(&dir.path().join("missing")).is_err());
}
