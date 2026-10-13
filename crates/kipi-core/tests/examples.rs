//! The example stores and machine/config files load through the real loaders.

use std::fs;
use std::path::{Path, PathBuf};

use kipi_core::config::{self, Env};
use kipi_core::error::Error;
use kipi_core::model::{AgentId, GrantMode, SkillRef, Store};
use kipi_test_support::stores::{ExampleStore, copy_example_stores};
use kipi_test_support::workspace_root;

const HOME: &str = "/HOME";

fn env() -> Env {
    let mut env = Env::from_lookup(|_| None, Some(PathBuf::from(HOME))).unwrap();
    env.config_dir = workspace_root().join("examples");
    env
}

fn load(store: ExampleStore) -> Store {
    config::load_store(&store.source_path(), &env()).unwrap_or_else(|e| panic!("{}", e.report()))
}

/// `(entry, field)` for every value still marked `# kipi:draft`.
fn drafts(store: &Store) -> Vec<(String, &'static str)> {
    let mut out = Vec::new();
    for repo in &store.repos {
        if repo.summary_is_draft() {
            out.push((repo.name.clone(), "summary"));
        }
        if repo.aliases_is_draft() {
            out.push((repo.name.clone(), "aliases"));
        }
    }
    out
}

fn names<T>(items: &[T], name: impl Fn(&T) -> &str) -> Vec<&str> {
    items.iter().map(name).collect()
}

#[test]
fn acme_store_loads_with_drafts_detected() {
    let store = load(ExampleStore::Acme);
    assert_eq!(store.name, "acme");
    assert!(!store.share_skills);
    assert_eq!(names(&store.scopes, |s| &s.name), ["acme", "acme-ml"]);
    assert_eq!(
        names(&store.repos, |r| &r.name),
        [
            "billing-api",
            "acme-api",
            "events-pipeline",
            "ml-platform",
            "feature-store",
            "web-dashboard"
        ]
    );
    assert_eq!(
        names(&store.skills, |s| &s.name),
        [
            "code-review",
            "dbt-conventions",
            "pr-conventions",
            "stream-recipes"
        ]
    );
    assert_eq!(
        drafts(&store),
        [
            ("billing-api".to_owned(), "summary"),
            ("feature-store".to_owned(), "summary"),
            ("feature-store".to_owned(), "aliases"),
            ("web-dashboard".to_owned(), "aliases"),
        ]
    );

    let acme = store.scope("acme").unwrap();
    assert_eq!(acme.roots, [PathBuf::from("/HOME/work/acme")]);
    assert_eq!(
        acme.skills.last(),
        Some(&"personal/commit-style".parse::<SkillRef>().unwrap())
    );
    assert!(acme.registry);
    assert_eq!(acme.at.line, Some(13));
    assert_eq!(
        store.scope("acme-ml").unwrap().extends.as_deref(),
        Some("acme")
    );

    let events = store.repo("events-pipeline").unwrap();
    assert_eq!(events.scope.as_deref(), Some("acme-ml"));
    assert_eq!(events.related, ["billing-api", "ml-platform"]);
    assert_eq!(events.exclude, ["pr-conventions".parse().unwrap()]);
    assert_eq!(events.agents.as_deref(), Some(&[AgentId::Claude][..]));
    assert_eq!(events.grant, Some(GrantMode::Registry));
    assert!(events.notes.is_some());
    assert_eq!(
        events.at.file,
        ExampleStore::Acme.source_path().join("bindings.toml")
    );
    assert!(store.repo("billing-api").unwrap().notes.is_some());
    assert!(store.repo("ml-platform").unwrap().notes.is_none());
    assert!(store.repo("web-dashboard").unwrap().summary.is_none());
    assert_eq!(
        store.skill("dbt-conventions").unwrap().scope.as_deref(),
        Some("acme")
    );
}

#[test]
fn personal_store_loads() {
    let store = load(ExampleStore::Personal);
    assert_eq!(store.name, "personal");
    assert!(store.share_skills);
    assert_eq!(
        names(&store.skills, |s| &s.name),
        ["commit-style", "rust-style"]
    );
    assert_eq!(drafts(&store), [("api".to_owned(), "summary")]);
    assert_eq!(store.defaults.grant(), GrantMode::Related);
    assert_eq!(
        store.scope("personal").unwrap().roots,
        [PathBuf::from("/HOME/code")]
    );
}

#[test]
fn example_machine_and_config_load() {
    let machine = config::load_machine(&env()).unwrap_or_else(|e| panic!("{}", e.report()));
    assert_eq!(machine.hostname.as_deref(), Some("laptop-a"));
    assert_eq!(
        machine.repo_path("events-pipeline"),
        Some(Path::new("/Volumes/work/acme/ml/events-pipeline"))
    );
    assert_eq!(machine.repo_path("feature-store"), None, "not cloned here");
    assert_eq!(
        machine.roots_for("acme").unwrap().roots,
        [PathBuf::from("/Volumes/work/acme")]
    );

    let cfg = config::load_config(&env()).unwrap_or_else(|e| panic!("{}", e.report()));
    assert_eq!(
        cfg.stores,
        [
            PathBuf::from("/HOME/.agents/store"),
            PathBuf::from("/HOME/work/.kipi-acme")
        ]
    );
    assert_eq!(cfg.tokenizer.as_deref(), Some("cl100k_base"));
    assert_eq!(
        config::store_paths(&[], &env(), &cfg),
        cfg.stores,
        "config.toml wins when no flag or KIPI_STORE is given"
    );
}

#[test]
fn typod_key_names_file_line_and_closest_key() {
    let fx = copy_example_stores(&[ExampleStore::Acme]).unwrap();
    let root = fx.path(ExampleStore::Acme);
    let path = root.join("bindings.toml");
    let text = fs::read_to_string(&path).unwrap();
    let typo = "skils = [\"stream-recipes\"]";
    let text = text.replace("skills = [\"stream-recipes\"]", typo);
    fs::write(&path, &text).unwrap();
    let line = text.lines().position(|l| l == typo).unwrap() + 1;

    let err = config::load_store(root, &env()).unwrap_err();
    let Error::UnknownKey { at, key, table, .. } = &err else {
        panic!("expected an unknown key, got {err:?}");
    };
    assert_eq!(at.file, path);
    assert_eq!(at.line, Some(line));
    assert_eq!(key, "skils");
    assert_eq!(table, "[repos.events-pipeline]");
    assert_eq!(err.suggestion(), Some("skills"));
    let report = err.report();
    assert!(
        report.starts_with(&format!(
            "error: {}:{line}: unknown key `skils`",
            path.display()
        )),
        "{report}"
    );
    assert!(report.contains("did you mean `skills`?"), "{report}");
}

#[test]
fn untouched_example_skills_round_trip_byte_stable() {
    use kipi_core::frontmatter::Frontmatter;
    for store in ExampleStore::ALL {
        for skill in &load(store).skills {
            let text = fs::read_to_string(&skill.at.file).unwrap();
            let fm = Frontmatter::parse(&skill.at.file, &text).unwrap();
            assert_eq!(fm.render(), text, "{}", skill.at.file.display());
        }
    }
}
