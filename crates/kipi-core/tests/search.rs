//! `search`: views obey the scope-chain leak rule; lookups and searches stay inside them.

use std::path::PathBuf;

use kipi_core::config::{self, Env};
use kipi_core::model::StoreSet;
use kipi_core::plan::Plan;
use kipi_core::resolve::resolve;
use kipi_core::search::{self, HitSource, Lookup, SearchOptions, View};
use kipi_test_support::stores::{ExampleStore, StoreFixture, copy_example_stores};
use kipi_test_support::workspace_root;
use regex::Regex;

fn load() -> (StoreFixture, StoreSet, Plan) {
    load_with(|_| {})
}

/// Copy the example stores, let `edit` change the copies, then load and resolve them.
fn load_with(edit: impl FnOnce(&StoreFixture)) -> (StoreFixture, StoreSet, Plan) {
    let fixture = copy_example_stores(&ExampleStore::ALL).unwrap();
    edit(&fixture);
    let mut env = Env::from_lookup(|_| None, Some(PathBuf::from("/HOME"))).unwrap();
    env.config_dir = workspace_root().join("examples");
    let paths: Vec<PathBuf> = fixture.paths().into_iter().map(PathBuf::from).collect();
    let stores = config::load_stores(&paths, &env).unwrap();
    let machine = config::load_machine(&env).unwrap();
    let plan = resolve(&stores, &machine).unwrap();
    (fixture, stores, plan)
}

fn names(view: &View) -> Vec<&str> {
    view.repos.iter().map(String::as_str).collect()
}

#[test]
fn a_scope_sees_its_ancestors_and_descendants_only() {
    let (_dir, stores, plan) = load();
    let ml = View::scope(&stores, &plan, "acme-ml").unwrap();
    let acme = View::scope(&stores, &plan, "acme").unwrap();
    let all_acme = [
        "acme-api",
        "billing-api",
        "events-pipeline",
        "feature-store",
        "ml-platform",
        "web-dashboard",
    ];
    assert_eq!(names(&ml), all_acme);
    assert_eq!(names(&acme), all_acme);
    let personal = View::scope(&stores, &plan, "personal").unwrap();
    assert_eq!(names(&personal), ["api", "blog", "dotfiles", "side-tool"]);
    assert!(View::scope(&stores, &plan, "nope").is_none());
    // The shared skill is visible from acme; nothing else of the personal store is.
    let skills: Vec<_> = acme
        .skills
        .iter()
        .map(|s| format!("{}/{}", s.store, s.name))
        .collect();
    assert!(
        skills.contains(&"personal/commit-style".to_owned()),
        "{skills:?}"
    );
    assert!(
        !skills.contains(&"personal/rust-style".to_owned()),
        "{skills:?}"
    );
}

#[test]
fn lookups_resolve_names_and_aliases_within_the_view() {
    let (_dir, stores, plan) = load();
    let personal = View::scope(&stores, &plan, "personal").unwrap();
    let everything = View::everything(&stores);
    let found = |view: &View, alias: &str| match search::lookup(&stores, &plan, view, alias) {
        Lookup::Found(info) => Some(info.name),
        _ => None,
    };
    assert_eq!(
        found(&everything, "Billing").as_deref(),
        Some("billing-api")
    );
    assert_eq!(
        found(&everything, "api").as_deref(),
        Some("api"),
        "a name beats aliases"
    );
    assert_eq!(found(&personal, "billing"), None, "the leak rule");
    assert_eq!(
        search::lookup(&stores, &plan, &everything, "bill"),
        Lookup::NotFound,
        "no fuzzy matching"
    );
}

#[test]
fn knowledge_search_stays_in_the_view() {
    let (_dir, stores, plan) = load();
    let personal = View::scope(&stores, &plan, "personal").unwrap();
    let any = Regex::new(".").unwrap();
    let found =
        search::search_knowledge(&stores, &plan, &personal, &any, &SearchOptions::default());
    assert!(!found.hits.is_empty());
    for hit in &found.hits {
        let path = hit.path.display().to_string();
        assert!(path.contains("personal-store"), "{path}");
        assert!(
            !matches!(hit.source, HitSource::Checkout { .. }),
            "no --repos"
        );
    }
    let capped = SearchOptions {
        limit: 2,
        ..SearchOptions::default()
    };
    let found = search::search_knowledge(&stores, &plan, &personal, &any, &capped);
    assert_eq!(found.hits.len(), 2);
    assert!(found.truncated);
}

#[test]
fn ambiguous_lookups_are_sorted_by_name_whatever_the_plan_order() {
    let (_dir, stores, mut plan) = load_with(|fx| {
        let file = fx.path(ExampleStore::Acme).join("bindings.toml");
        let text = std::fs::read_to_string(&file).unwrap();
        let text = text.replacen(
            "summary = \"Public REST gateway\"\n",
            "summary = \"Public REST gateway\"\naliases = [\"billing\"]\n",
            1,
        );
        std::fs::write(&file, text).unwrap();
    });
    plan.repos.reverse();
    let names: Vec<&str> = plan.repos.iter().map(|r| r.name.as_str()).collect();
    let (b, a) = (
        names.iter().position(|n| *n == "billing-api").unwrap(),
        names.iter().position(|n| *n == "acme-api").unwrap(),
    );
    assert!(b < a, "the plan lists billing-api first: {names:?}");
    let view = View::everything(&stores);
    match search::lookup(&stores, &plan, &view, "billing") {
        Lookup::Ambiguous(infos) => {
            let names: Vec<&str> = infos.iter().map(|i| i.name.as_str()).collect();
            assert_eq!(names, ["acme-api", "billing-api"]);
        }
        other => panic!("expected ambiguous, got {other:?}"),
    }
}

#[test]
fn checkout_hits_come_in_repo_name_order_whatever_the_plan_order() {
    let (_dir, stores, mut plan) = load();
    let checkouts = tempfile::tempdir().unwrap();
    for repo in plan.repos.iter_mut() {
        if ["api", "blog", "side-tool"].contains(&repo.name.as_str()) {
            let dir = checkouts.path().join(&repo.name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("README.md"), "needle\n").unwrap();
            repo.path = Some(dir);
        }
    }
    plan.repos.reverse();
    let personal = View::scope(&stores, &plan, "personal").unwrap();
    let options = SearchOptions {
        repos: true,
        ..SearchOptions::default()
    };
    let needle = Regex::new("^needle$").unwrap();
    let found = search::search_knowledge(&stores, &plan, &personal, &needle, &options);
    let repos: Vec<&str> = found
        .hits
        .iter()
        .map(|h| match &h.source {
            HitSource::Checkout { repo } => repo.as_str(),
            other => panic!("unexpected hit {other:?}"),
        })
        .collect();
    assert_eq!(repos, ["api", "blog", "side-tool"]);
}

#[test]
fn view_labels_read_as_noun_phrases() {
    let (_dir, stores, plan) = load();
    assert_eq!(View::everything(&stores).label, "every mounted store");
    assert_eq!(
        View::scope(&stores, &plan, "acme-ml").unwrap().label,
        "the scope chain of acme-ml"
    );
}
