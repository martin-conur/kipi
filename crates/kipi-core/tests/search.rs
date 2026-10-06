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
    let fixture = copy_example_stores(&ExampleStore::ALL).unwrap();
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
