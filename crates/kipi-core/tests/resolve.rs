//! Resolution (roadmap §6): the example stores end to end, then each rule on in-memory
//! stores built with `kipi_test_support::builders`.

use std::path::{Path, PathBuf};

use kipi_core::config::{self, Env};
use kipi_core::error::Error;
use kipi_core::model::{AgentId, GrantMode, Machine, StoreSet};
use kipi_core::plan::{AlwaysOn, GrantKind, Plan, RepoPlan};
use kipi_core::resolve::{remote_pattern_matches, resolve};
use kipi_test_support::builders::{
    MachineBuilder, StoreBuilder, draft, remote, reviewed, skill_refs, store_set,
};
use kipi_test_support::stores::ExampleStore;
use kipi_test_support::workspace_root;

const HOME: &str = "/HOME";

fn env() -> Env {
    let mut env = Env::from_lookup(|_| None, Some(PathBuf::from(HOME))).unwrap();
    env.config_dir = workspace_root().join("examples");
    env
}

fn examples(stores: &[ExampleStore]) -> (StoreSet, Machine) {
    let paths: Vec<PathBuf> = stores.iter().map(|s| s.source_path()).collect();
    let set = config::load_stores(&paths, &env()).unwrap_or_else(|e| panic!("{}", e.report()));
    let machine = config::load_machine(&env()).unwrap_or_else(|e| panic!("{}", e.report()));
    (set, machine)
}

fn ok(set: &StoreSet, machine: &Machine) -> Plan {
    resolve(set, machine).unwrap_or_else(|e| panic!("{}", e.report()))
}

/// The resolution error, checked for a location and a hint.
fn err(set: &StoreSet, machine: &Machine) -> Error {
    let err = resolve(set, machine).expect_err("resolution fails");
    assert!(err.location().is_some(), "error has a location: {err}");
    assert!(!err.hint().trim().is_empty(), "error has a hint: {err}");
    err
}

fn skills(repo: &RepoPlan) -> Vec<String> {
    repo.skills
        .iter()
        .map(|s| s.reference.to_string())
        .collect()
}

fn granted_repos(repo: &RepoPlan) -> Vec<&str> {
    repo.grant
        .dirs
        .iter()
        .filter_map(|d| match &d.kind {
            GrantKind::Repo { name } => Some(name.as_str()),
            _ => None,
        })
        .collect()
}

fn scope(repo: &RepoPlan) -> Option<&str> {
    repo.scope.value.as_deref()
}

// ---------------------------------------------------------------------------------
// Example stores

#[test]
fn every_example_repo_resolves_with_both_stores_mounted() {
    let (set, machine) = examples(&ExampleStore::ALL);
    let plan = ok(&set, &machine);
    let scopes: Vec<(&str, Option<&str>)> = plan
        .repos
        .iter()
        .map(|r| (r.name.as_str(), scope(r)))
        .collect();
    assert_eq!(
        scopes,
        [
            ("acme-api", Some("acme")),
            ("api", Some("personal")),
            ("billing-api", Some("acme")),
            ("blog", Some("personal")),
            ("dotfiles", Some("personal")),
            ("events-pipeline", Some("acme-ml")),
            ("feature-store", Some("acme")),
            ("ml-platform", Some("acme-ml")),
            ("side-tool", Some("personal")),
            ("web-dashboard", Some("acme")),
        ]
    );
    let registries: Vec<&str> = plan.registries.iter().map(|r| r.scope.as_str()).collect();
    assert_eq!(registries, ["acme", "personal"]);
}

#[test]
fn events_pipeline_resolves_as_the_spec_describes() {
    let (set, machine) = examples(&ExampleStore::ALL);
    let plan = ok(&set, &machine);
    let events = plan.repo("events-pipeline").unwrap();

    assert_eq!(scope(events), Some("acme-ml"));
    assert_eq!(events.scope.source.rule, "repos.events-pipeline.scope");
    assert_eq!(events.scope.source.store(), Some("acme"));
    assert_eq!(events.chain, ["acme", "acme-ml"]);
    assert_eq!(
        skills(events),
        [
            "code-review",
            "personal/commit-style",
            "dbt-conventions",
            "stream-recipes"
        ]
    );
    assert_eq!(events.excluded.len(), 1);
    assert_eq!(events.excluded[0].name, "pr-conventions");
    assert_eq!(
        events.excluded[0].source.rule,
        "repos.events-pipeline.exclude"
    );
    assert_eq!(events.skills[0].source.rule, "scopes.acme.skills");
    assert_eq!(
        set.provenance(events.skills[0].source.at.as_ref().unwrap()),
        "acme:bindings.toml:13"
    );

    assert_eq!(events.agents.value, [AgentId::Claude]);
    assert_eq!(events.budget_tokens.value, 4000);
    assert_eq!(events.budget_tokens.source.rule, "defaults.budget_tokens");
    assert_eq!(events.grant.mode.value, GrantMode::Registry);

    // Both related repos are granted (and the rest of the acme registry, as grant = registry).
    let related: Vec<&str> = events.related.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(related, ["billing-api", "ml-platform"]);
    for r in &events.related {
        assert!(
            events.grant.contains(r.path.as_ref().unwrap()),
            "{} granted",
            r.name
        );
    }
    assert_eq!(
        granted_repos(events),
        ["acme-api", "billing-api", "ml-platform", "web-dashboard"]
    );
    let acme = ExampleStore::Acme.source_path();
    let personal = ExampleStore::Personal.source_path();
    assert!(events.grant.contains(&acme.join("repos")));
    assert!(events.grant.contains(&acme.join("skills")));
    assert!(
        events.grant.contains(&personal.join("skills")),
        "a shared skill grants its store's skills/ dir"
    );
    assert!(!events.grant.contains(&personal.join("repos")));

    assert_eq!(
        events.always,
        [
            AlwaysOn::File {
                path: acme.join("conventions/acme.md"),
                scope: "acme".into(),
                source: events.always_source(0),
            },
            AlwaysOn::Registry {
                scope: "acme".into(),
                source: plan.registry("acme").unwrap().source.clone(),
            },
            AlwaysOn::ThisRepo,
        ]
    );
    assert_eq!(plan.registry_of(events).unwrap().scope, "acme");
}

/// Shorthand for the source of an `AlwaysOn::File` entry.
trait AlwaysSource {
    fn always_source(&self, i: usize) -> kipi_core::plan::Source;
}

impl AlwaysSource for RepoPlan {
    fn always_source(&self, i: usize) -> kipi_core::plan::Source {
        match &self.always[i] {
            AlwaysOn::File { source, .. } | AlwaysOn::Registry { source, .. } => source.clone(),
            AlwaysOn::ThisRepo => panic!("no source"),
        }
    }
}

#[test]
fn example_scopes_come_from_roots_patterns_and_the_personal_fallback() {
    let (set, machine) = examples(&ExampleStore::ALL);
    let plan = ok(&set, &machine);
    let rule = |name: &str| plan.repo(name).unwrap().scope.source.rule.clone();
    assert_eq!(rule("billing-api"), "machine.toml [roots] acme");
    assert_eq!(rule("ml-platform"), "machine.toml [roots] acme-ml");
    assert_eq!(
        rule("feature-store"),
        "scopes.acme.remote_patterns (not cloned here)"
    );
    assert_eq!(rule("side-tool"), "machine.toml [roots] personal");

    // Without that override no root holds `/srv/code/side-tool`: rule 4 places it.
    let mut machine = machine;
    machine.roots.retain(|r| r.scope != "personal");
    let plan = ok(&set, &machine);
    let side_tool = &plan.repo("side-tool").unwrap().scope;
    assert_eq!(side_tool.value.as_deref(), Some("personal"));
    assert_eq!(side_tool.source.rule, "fallback: scope `personal`");
}

#[test]
fn example_registries_list_uncloned_repos_and_drafts() {
    let (set, machine) = examples(&ExampleStore::ALL);
    let plan = ok(&set, &machine);
    let acme = plan.registry("acme").unwrap();
    assert_eq!(acme.scopes, ["acme", "acme-ml"]);
    let names: Vec<&str> = acme.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "acme-api",
            "billing-api",
            "events-pipeline",
            "feature-store",
            "ml-platform",
            "web-dashboard"
        ]
    );
    let entry = |n: &str| acme.entries.iter().find(|e| e.name == n).unwrap();
    assert_eq!(entry("feature-store").local, None, "not cloned here");
    assert!(entry("feature-store").summary_draft && entry("feature-store").aliases_draft);
    assert!(entry("billing-api").summary_draft);
    assert!(entry("billing-api").notes.is_some());
    assert_eq!(
        entry("billing-api").keys,
        ["billing-api", "billing", "invoices"]
    );
    assert_eq!(entry("web-dashboard").summary, None);
    assert_eq!(
        entry("ml-platform").local.as_deref(),
        Some(Path::new("/Volumes/work/acme/ml/ml-platform"))
    );
    assert!(acme.collisions.is_empty());

    let personal = plan.registry("personal").unwrap();
    assert_eq!(personal.entries.len(), 4);
    assert!(personal.entries.iter().all(|e| e.local.is_some()));
}

#[test]
fn personal_repos_never_see_acme_content() {
    let (set, machine) = examples(&ExampleStore::ALL);
    let plan = ok(&set, &machine);
    let acme_root = ExampleStore::Acme.source_path();
    let acme_names: Vec<&str> = set
        .store("acme")
        .unwrap()
        .repos
        .iter()
        .map(|r| r.name.as_str())
        .collect();
    for repo in plan.repos.iter().filter(|r| r.store == "personal") {
        assert_eq!(scope(repo), Some("personal"));
        assert!(
            repo.skills.iter().all(|s| s.store == "personal"),
            "{}",
            repo.name
        );
        assert!(
            repo.grant
                .dirs
                .iter()
                .all(|d| !d.path.starts_with(&acme_root))
        );
        assert!(
            repo.grant
                .dirs
                .iter()
                .all(|d| !d.path.starts_with("/Volumes/work"))
        );
        assert!(
            repo.related
                .iter()
                .all(|r| !acme_names.contains(&r.name.as_str()))
        );
        let registry = plan.registry_of(repo).unwrap();
        assert_eq!(registry.scope, "personal");
        assert!(
            registry
                .entries
                .iter()
                .all(|e| !acme_names.contains(&e.name.as_str()))
        );
    }
    let side_tool = plan.repo("side-tool").unwrap();
    assert_eq!(skills(side_tool), ["rust-style", "commit-style"]);
    assert_eq!(granted_repos(side_tool), ["api"]);
}

#[test]
fn personal_store_alone_resolves_to_the_same_personal_plans() {
    let (both, machine) = examples(&ExampleStore::ALL);
    let (alone, _) = examples(&[ExampleStore::Personal]);
    let with_work = ok(&both, &machine);
    let without = ok(&alone, &machine);
    let personal: Vec<&RepoPlan> = with_work
        .repos
        .iter()
        .filter(|r| r.store == "personal")
        .collect();
    assert_eq!(without.repos.iter().collect::<Vec<_>>(), personal);
    assert_eq!(
        without.registries,
        [with_work.registry("personal").unwrap().clone()]
    );
}

// ---------------------------------------------------------------------------------
// Hinted errors

fn leak_fixture() -> StoreSet {
    store_set(vec![
        StoreBuilder::new("s", "/st")
            .scope("a", |_| {})
            .scope("b", |_| {})
            .repo("x", |r| {
                r.scope = Some("a".into());
                r.related = vec!["y".into()];
            })
            .repo("y", |r| r.scope = Some("b".into()))
            .build(),
    ])
}

#[test]
fn related_outside_the_scope_chain_is_a_leak_error() {
    let e = err(&leak_fixture(), &Machine::default());
    let report = e.report();
    assert!(
        report.contains(
            "repo `x` (scope `a`) is related to `y` (scope `b`), outside its scope chain"
        ),
        "{report}"
    );
    assert!(matches!(e, Error::Conflict { .. }), "names both entries");
}

#[test]
fn related_from_scoped_to_unscoped_is_a_leak_error() {
    let set = store_set(vec![
        StoreBuilder::new("s", "/st")
            .scope("a", |_| {})
            .repo("x", |r| {
                r.scope = Some("a".into());
                r.related = vec!["y".into()];
            })
            .repo("y", |_| {})
            .build(),
    ]);
    assert!(
        err(&set, &Machine::default())
            .to_string()
            .contains("(unscoped)")
    );
}

#[test]
fn missing_skill_is_an_error_suggesting_the_closest_name() {
    let set = store_set(vec![
        StoreBuilder::new("s", "/st")
            .scope("a", |s| s.skills = skill_refs(&["code-reveiw"]))
            .skill("code-review", None)
            .build(),
    ]);
    let e = err(&set, &Machine::default());
    assert!(
        e.to_string()
            .contains("skill `code-reveiw` (in `scopes.a.skills`) is not in store `s`")
    );
    assert_eq!(e.hint(), "did you mean `code-review`?");

    let set = store_set(vec![
        StoreBuilder::new("s", "/st")
            .repo("x", |r| r.exclude = skill_refs(&["nothing-like-it"]))
            .build(),
    ]);
    let e = err(&set, &Machine::default());
    assert!(
        e.hint().contains("kipi add skill nothing-like-it"),
        "{}",
        e.hint()
    );
}

#[test]
fn extends_cycle_is_an_error_naming_the_cycle() {
    let set = store_set(vec![
        StoreBuilder::new("s", "/st")
            .scope("a", |s| s.extends = Some("c".into()))
            .scope("b", |s| s.extends = Some("a".into()))
            .scope("c", |s| s.extends = Some("b".into()))
            .build(),
    ]);
    let e = err(&set, &Machine::default());
    assert!(
        e.to_string()
            .contains("`extends` forms a cycle: a → c → b → a"),
        "{e}"
    );

    let set = store_set(vec![
        StoreBuilder::new("s", "/st")
            .scope("a", |s| s.extends = Some("a".into()))
            .build(),
    ]);
    assert!(
        err(&set, &Machine::default())
            .to_string()
            .contains("cycle: a → a")
    );
}

#[test]
fn extends_unknown_parent_is_an_error() {
    let set = store_set(vec![
        StoreBuilder::new("s", "/st")
            .scope("acme", |_| {})
            .scope("acme-ml", |s| s.extends = Some("acm".into()))
            .build(),
    ]);
    let e = err(&set, &Machine::default());
    assert!(
        e.to_string()
            .contains("scope `acme-ml` extends `acm`, which is not a scope in store `s`")
    );
    assert_eq!(e.hint(), "did you mean `acme`?");
}

#[test]
fn unknown_references_are_errors() {
    let unknown_scope = store_set(vec![
        StoreBuilder::new("s", "/st")
            .scope("work", |_| {})
            .repo("x", |r| r.scope = Some("zzz".into()))
            .build(),
    ]);
    let e = err(&unknown_scope, &Machine::default());
    assert!(e.to_string().contains("sets `scope = \"zzz\"`"));
    assert_eq!(e.hint(), "known scopes: `work`");

    let no_scopes = store_set(vec![
        StoreBuilder::new("s", "/st")
            .repo("x", |r| r.scope = Some("zzz".into()))
            .build(),
    ]);
    assert_eq!(
        err(&no_scopes, &Machine::default()).hint(),
        "declare [scopes.zzz] or remove the reference"
    );

    let unknown_related = store_set(vec![
        StoreBuilder::new("s", "/st")
            .repo("billing", |r| r.related = vec!["biling".into()])
            .build(),
    ]);
    let e = err(&unknown_related, &Machine::default());
    assert!(
        e.to_string()
            .contains("related to `biling`, which is not a repo entry")
    );
    assert_eq!(e.hint(), "did you mean `billing`?");

    let itself = store_set(vec![
        StoreBuilder::new("s", "/st")
            .repo("x", |r| r.related = vec!["x".into()])
            .build(),
    ]);
    assert!(
        err(&itself, &Machine::default())
            .to_string()
            .contains("lists itself")
    );
}

#[test]
fn skills_stay_within_their_x_kipi_scope() {
    let base = || {
        StoreBuilder::new("s", "/st")
            .scope("acme", |_| {})
            .scope("acme-ml", |s| s.extends = Some("acme".into()))
            .scope("other", |_| {})
            .skill("dbt", Some("acme"))
            .skill("ml-only", Some("acme-ml"))
    };
    // An ancestor-owned skill bound in a descendant is fine; the reverse is not.
    let fine = store_set(vec![
        base()
            .scope("x", |s| {
                s.extends = Some("acme-ml".into());
                s.skills = skill_refs(&["dbt", "ml-only"]);
            })
            .build(),
    ]);
    ok(&fine, &Machine::default());

    let up = store_set(vec![
        base()
            .repo("r", |r| {
                r.scope = Some("acme".into());
                r.skills = skill_refs(&["ml-only"]);
            })
            .build(),
    ]);
    let e = err(&up, &Machine::default());
    assert!(e.to_string().contains("belongs to scope `acme-ml` (x-kipi.scope) but `repos.r.skills` binds it to scope `acme`"), "{e}");

    let unscoped = store_set(vec![
        base()
            .repo("r", |r| r.skills = skill_refs(&["dbt"]))
            .build(),
    ]);
    assert!(
        err(&unscoped, &Machine::default())
            .to_string()
            .contains("binds it to an unscoped repo")
    );

    let sideways = store_set(vec![
        base()
            .scope("y", |s| {
                s.extends = Some("other".into());
                s.skills = skill_refs(&["dbt"]);
            })
            .build(),
    ]);
    assert!(
        err(&sideways, &Machine::default())
            .to_string()
            .contains("`scopes.y.skills` binds it to scope `y`")
    );

    // Excluding a skill is never a leak.
    let exclude = store_set(vec![
        base()
            .repo("r", |r| {
                r.scope = Some("other".into());
                r.exclude = skill_refs(&["ml-only"]);
            })
            .build(),
    ]);
    ok(&exclude, &Machine::default());
}

#[test]
fn x_kipi_scope_must_name_a_scope_of_the_store() {
    let set = store_set(vec![
        StoreBuilder::new("s", "/st")
            .scope("acme", |_| {})
            .skill("dbt", Some("acmee"))
            .build(),
    ]);
    let e = err(&set, &Machine::default());
    assert!(
        e.to_string()
            .contains("sets `x-kipi.scope: acmee`, but store `s` has no scope `acmee`")
    );
    assert_eq!(e.hint(), "did you mean `acme`?");
}

#[test]
fn two_roots_at_the_same_depth_are_ambiguous() {
    let set = store_set(vec![
        StoreBuilder::new("s", "/st")
            .scope("a", |s| s.roots = vec!["/w".into()])
            .scope("b", |s| s.roots = vec!["/w".into()])
            .repo("x", |_| {})
            .build(),
    ]);
    let machine = MachineBuilder::new().repo("x", "/w/x").build();
    let e = err(&set, &machine);
    assert!(
        e.to_string()
            .contains("under roots of scopes `a`, `b` at the same depth"),
        "{e}"
    );
    assert!(e.hint().contains("[repos.x]"));
}

#[test]
fn every_scope_tied_at_the_deepest_root_is_named() {
    let set = store_set(vec![
        StoreBuilder::new("s", "/st")
            .scope("c", |s| s.roots = vec!["/w".into()])
            .scope("shallow", |s| s.roots = vec!["/".into()])
            .scope("a", |s| s.roots = vec!["/w".into()])
            .scope("b", |s| s.roots = vec!["/w".into()])
            .repo("x", |_| {})
            .build(),
    ]);
    let machine = MachineBuilder::new().repo("x", "/w/x").build();
    let e = err(&set, &machine);
    assert!(
        e.to_string()
            .contains("under roots of scopes `a`, `b`, `c` at the same depth"),
        "{e}"
    );

    // A deeper root clears an earlier tie.
    let set = store_set(vec![
        StoreBuilder::new("s", "/st")
            .scope("a", |s| s.roots = vec!["/w".into()])
            .scope("b", |s| s.roots = vec!["/w".into()])
            .scope("deep", |s| s.roots = vec!["/w/x".into()])
            .repo("x", |_| {})
            .build(),
    ]);
    let machine = MachineBuilder::new().repo("x", "/w/x/y").build();
    assert_eq!(scope(ok(&set, &machine).repo("x").unwrap()), Some("deep"));
}

// ---------------------------------------------------------------------------------
// Rules

fn one_store(build: impl FnOnce(StoreBuilder) -> StoreBuilder) -> StoreSet {
    store_set(vec![build(StoreBuilder::new("s", "/st")).build()])
}

#[test]
fn scope_precedence_explicit_then_deepest_root_then_personal_then_unscoped() {
    let set = one_store(|s| {
        s.scope("top", |s| s.roots = vec!["/w".into(), "/elsewhere".into()])
            .scope("nested", |s| {
                s.extends = Some("top".into());
                s.roots = vec!["/w/deep".into()];
            })
            .repo("explicit", |r| r.scope = Some("top".into()))
            .repo("deep", |_| {})
            .repo("shallow", |_| {})
            .repo("outside", |_| {})
            .repo("prefix-not-ancestor", |_| {})
    });
    let machine = MachineBuilder::new()
        .repo("explicit", "/w/deep/explicit")
        .repo("deep", "/w/deep/x")
        .repo("shallow", "/w/x")
        .repo("outside", "/tmp/x")
        .repo("prefix-not-ancestor", "/wx/y")
        .build();
    let plan = ok(&set, &machine);
    let s = |n: &str| scope(plan.repo(n).unwrap());
    assert_eq!(s("explicit"), Some("top"));
    assert_eq!(s("deep"), Some("nested"));
    assert_eq!(s("shallow"), Some("top"));
    assert_eq!(s("outside"), None);
    assert_eq!(
        s("prefix-not-ancestor"),
        None,
        "roots match whole path components"
    );
    let outside = plan.repo("outside").unwrap();
    assert!(outside.scope.source.at.is_none());
    assert!(outside.chain.is_empty() && outside.registry.is_none());
    assert_eq!(outside.always, [AlwaysOn::ThisRepo]);

    let with_personal = one_store(|s| s.scope("personal", |_| {}).repo("outside", |_| {}));
    let plan = ok(&with_personal, &machine);
    assert_eq!(scope(plan.repo("outside").unwrap()), Some("personal"));
}

#[test]
fn machine_roots_override_the_store_roots() {
    let set = one_store(|s| {
        s.scope("work", |s| s.roots = vec!["/HOME/work".into()])
            .repo("x", |_| {})
    });
    let at_home = MachineBuilder::new().repo("x", "/HOME/work/x").build();
    assert_eq!(scope(ok(&set, &at_home).repo("x").unwrap()), Some("work"));

    let moved = MachineBuilder::new()
        .roots("work", &["/Volumes/work"])
        .repo("x", "/HOME/work/x")
        .build();
    assert_eq!(
        scope(ok(&set, &moved).repo("x").unwrap()),
        None,
        "the override replaces the roots"
    );
}

#[test]
fn uncloned_repos_are_placed_by_the_longest_remote_pattern() {
    let set = one_store(|s| {
        s.scope("org", |s| {
            s.remote_patterns = vec!["github.com/org/".into()]
        })
        .scope("team", |s| {
            s.remote_patterns = vec!["github.com/org/team-".into()]
        })
        .scope("twin", |s| {
            s.remote_patterns = vec!["github.com/org/team-".into(), "gitlab.com/".into()]
        })
        .scope("personal", |_| {})
        .scope("any-github", |s| {
            s.remote_patterns = vec!["github.com/".into()]
        })
        .repo("plain", |r| r.remote = Some(remote("github.com/org/plain")))
        .repo("tied", |r| r.remote = Some(remote("github.com/org/team-x")))
        .repo("gl", |r| r.remote = Some(remote("gitlab.com/a/b")))
        .repo("no-remote", |_| {})
        .repo("cloned", |r| {
            r.remote = Some(remote("github.com/org/cloned"))
        })
    });
    let machine = MachineBuilder::new()
        .repo("cloned", "/nowhere/cloned")
        .build();
    let plan = ok(&set, &machine);
    let s = |n: &str| scope(plan.repo(n).unwrap());
    assert_eq!(s("plain"), Some("org"));
    assert_eq!(s("tied"), Some("personal"), "a tie decides nothing");
    assert_eq!(s("gl"), Some("twin"));
    assert_eq!(s("no-remote"), Some("personal"));
    assert_eq!(
        s("cloned"),
        Some("personal"),
        "patterns never override a checkout's path"
    );
    assert!(remote_pattern_matches(
        "github.com/org/",
        "github.com/org/x"
    ));
    assert!(!remote_pattern_matches(
        "github.com/org/",
        "github.com/other/x"
    ));
}

#[test]
fn skills_union_top_down_dedupe_and_exclude_wins() {
    let set = store_set(vec![
        StoreBuilder::new("home", "/home-store")
            .share_skills(true)
            .skill("shared", None)
            .build(),
        StoreBuilder::new("s", "/st")
            .scope("top", |s| s.skills = skill_refs(&["a", "b", "home/shared"]))
            .scope("mid", |s| {
                s.extends = Some("top".into());
                s.skills = skill_refs(&["c", "a", "s/b"]);
            })
            .skill("a", None)
            .skill("b", None)
            .skill("c", Some("top"))
            .skill("d", Some("mid"))
            .repo("x", |r| {
                r.scope = Some("mid".into());
                r.skills = skill_refs(&["d", "c"]);
                r.exclude = skill_refs(&["b", "home/shared"]);
            })
            .build(),
    ]);
    let plan = ok(&set, &Machine::default());
    let x = plan.repo("x").unwrap();
    assert_eq!(skills(x), ["a", "c", "d"]);
    assert_eq!(
        x.skills[1].source.rule, "scopes.mid.skills",
        "first mention wins"
    );
    let excluded: Vec<String> = x.excluded.iter().map(|s| s.reference.to_string()).collect();
    assert_eq!(excluded, ["b", "home/shared"]);
    assert!(x.excluded[1].is_shared("s"));
    assert!(
        !x.grant.contains(Path::new("/home-store/skills")),
        "an excluded shared skill grants nothing"
    );
}

#[test]
fn settings_precedence_repo_then_scope_chain_then_defaults_then_builtin() {
    let set = one_store(|s| {
        s.defaults(|d| d.budget_tokens = Some(1000))
            .scope("top", |s| {
                s.agents = Some(vec![AgentId::Generic]);
                s.grant = Some(GrantMode::None);
            })
            .scope("mid", |s| {
                s.extends = Some("top".into());
                s.grant = Some(GrantMode::Registry);
            })
            .repo("x", |r| {
                r.scope = Some("mid".into());
                r.budget_tokens = Some(2000);
            })
            .repo("y", |r| r.scope = Some("top".into()))
    });
    let plan = ok(&set, &Machine::default());
    let x = plan.repo("x").unwrap();
    assert_eq!(
        (x.budget_tokens.value, x.budget_tokens.source.rule.as_str()),
        (2000, "repos.x.budget_tokens")
    );
    assert_eq!(
        (x.grant.mode.value, x.grant.mode.source.rule.as_str()),
        (GrantMode::Registry, "scopes.mid.grant")
    );
    assert_eq!(
        (x.agents.value.as_slice(), x.agents.source.rule.as_str()),
        (&[AgentId::Generic][..], "scopes.top.agents")
    );
    let y = plan.repo("y").unwrap();
    assert_eq!(
        (y.budget_tokens.value, y.budget_tokens.source.rule.as_str()),
        (1000, "defaults.budget_tokens")
    );
    assert!(y.budget_tokens.source.at.is_some());
    assert_eq!(y.grant.mode.value, GrantMode::None);
    assert!(y.grant.dirs.is_empty(), "grant = none grants nothing");

    let bare = one_store(|s| s.repo("z", |_| {}));
    let plan = ok(&bare, &Machine::default());
    let z = plan.repo("z").unwrap();
    assert_eq!(z.agents.value, [AgentId::Claude, AgentId::Generic]);
    assert_eq!(z.grant.mode.value, GrantMode::Related);
    assert_eq!(
        z.budget_tokens.source.rule,
        "built-in default budget_tokens"
    );
    assert!(z.budget_tokens.source.at.is_none());

    // A store whose [defaults] has no header location (built in memory) still names the key.
    let mut store = StoreBuilder::new("s", "/st")
        .scope("top", |s| s.budget_tokens = Some(300))
        .repo("in-top", |r| r.scope = Some("top".into()))
        .repo("z", |_| {})
        .build();
    store.defaults.grant = Some(GrantMode::None);
    store.defaults.agents = Some(vec![AgentId::Generic]);
    store.defaults.budget_tokens = Some(100);
    let plan = ok(&store_set(vec![store]), &Machine::default());
    let z = plan.repo("z").unwrap();
    for source in [
        &z.grant.mode.source,
        &z.agents.source,
        &z.budget_tokens.source,
    ] {
        assert!(
            source.rule.starts_with("defaults.") && source.at.is_none(),
            "{source:?}"
        );
    }
    let in_top = plan.repo("in-top").unwrap();
    assert_eq!(
        (
            in_top.budget_tokens.value,
            in_top.budget_tokens.source.rule.as_str()
        ),
        (300, "scopes.top.budget_tokens")
    );
}

#[test]
fn registry_grant_stays_in_the_scope_chain_and_on_this_machine() {
    let set = one_store(|s| {
        s.scope("org", |s| {
            s.registry = true;
            s.grant = Some(GrantMode::Registry);
        })
        .scope("left", |s| s.extends = Some("org".into()))
        .scope("right", |s| s.extends = Some("org".into()))
        .repo("root-repo", |r| r.scope = Some("org".into()))
        .repo("me", |r| r.scope = Some("left".into()))
        .repo("left-peer", |r| r.scope = Some("left".into()))
        .repo("sibling", |r| r.scope = Some("right".into()))
        .repo("uncloned", |r| r.scope = Some("left".into()))
    });
    let machine = MachineBuilder::new()
        .repo("root-repo", "/c/root-repo")
        .repo("me", "/c/me")
        .repo("left-peer", "/c/left-peer")
        .repo("sibling", "/c/sibling")
        .build();
    let plan = ok(&set, &machine);
    let me = plan.repo("me").unwrap();
    assert_eq!(granted_repos(me), ["left-peer", "root-repo"]);
    assert_eq!(me.grant.dirs[0].kind, GrantKind::StoreRepos);
    assert_eq!(
        me.grant.dirs[1].kind,
        GrantKind::StoreSkills { store: "s".into() }
    );
    // The registry itself lists every repo under `org`, including the sibling scope's.
    assert_eq!(plan.registry_of(me).unwrap().entries.len(), 5);
    let root = plan.repo("root-repo").unwrap();
    assert_eq!(
        granted_repos(root),
        ["left-peer", "me", "sibling"],
        "descendants are in the chain"
    );
}

#[test]
fn related_grant_includes_ancestors_and_descendants_but_not_uncloned() {
    let set = one_store(|s| {
        s.scope("org", |_| {})
            .scope("team", |s| s.extends = Some("org".into()))
            .repo("me", |r| {
                r.scope = Some("team".into());
                r.related = vec!["up".into(), "gone".into(), "peer".into()];
            })
            .repo("up", |r| {
                r.scope = Some("org".into());
                r.related = vec!["me".into()];
            })
            .repo("gone", |r| r.scope = Some("team".into()))
            .repo("peer", |r| r.scope = Some("team".into()))
    });
    let machine = MachineBuilder::new()
        .repo("me", "/c/me")
        .repo("up", "/c/up")
        .repo("peer", "/c/peer")
        .build();
    let plan = ok(&set, &machine);
    let me = plan.repo("me").unwrap();
    assert_eq!(granted_repos(me), ["peer", "up"]);
    assert_eq!(me.related[1].path, None);
    assert_eq!(
        granted_repos(plan.repo("up").unwrap()),
        ["me"],
        "cycles are fine"
    );
}

#[test]
fn unscoped_repos_may_relate_to_each_other_only() {
    let set = one_store(|s| {
        s.repo("a", |r| r.related = vec!["b".into()])
            .repo("b", |r| r.related = vec!["a".into()])
    });
    let machine = MachineBuilder::new()
        .repo("a", "/c/a")
        .repo("b", "/c/b")
        .build();
    let plan = ok(&set, &machine);
    assert_eq!(granted_repos(plan.repo("a").unwrap()), ["b"]);
}

#[test]
fn alias_collisions_are_reported_as_data() {
    let set = one_store(|s| {
        s.scope("org", |s| s.registry = true)
            .repo("billing-api", |r| {
                r.scope = Some("org".into());
                r.aliases = reviewed(vec![
                    "Billing".into(),
                    "billing".into(),
                    "Pay  Ments".into(),
                ]);
                r.summary = Some(draft("Invoices".into()));
            })
            .repo("payments", |r| {
                r.scope = Some("org".into());
                r.aliases = draft(vec!["pay ments".into(), "BILLING ".into()]);
            })
            .repo("elsewhere", |_| {})
    });
    let plan = ok(&set, &Machine::default());
    let registry = plan.registry("org").unwrap();
    let billing = &registry.entries[0];
    assert_eq!(billing.keys, ["billing-api", "billing", "pay ments"]);
    assert_eq!(
        billing.aliases,
        ["Billing", "billing", "Pay  Ments"],
        "aliases kept as written"
    );
    assert!(billing.summary_draft && !billing.aliases_draft);
    let collisions: Vec<(&str, Vec<&str>)> = registry
        .collisions
        .iter()
        .map(|c| (c.key.as_str(), c.repos.iter().map(String::as_str).collect()))
        .collect();
    assert_eq!(
        collisions,
        [
            ("billing", vec!["billing-api", "payments"]),
            ("pay ments", vec!["billing-api", "payments"])
        ]
    );
    assert_eq!(
        registry.entries.len(),
        2,
        "unscoped repos are in no registry"
    );
}

#[test]
fn registry_is_the_nearest_enabled_scope_and_always_on_runs_top_down() {
    let set = one_store(|s| {
        s.scope("org", |s| {
            s.registry = true;
            s.always = vec!["conventions/org.md".into()];
        })
        .scope("team", |s| {
            s.extends = Some("org".into());
            s.registry = true;
            s.always = vec!["conventions/team.md".into(), "conventions/extra.md".into()];
        })
        .repo("in-org", |r| r.scope = Some("org".into()))
        .repo("in-team", |r| r.scope = Some("team".into()))
        .notes("in-team")
    });
    let plan = ok(&set, &Machine::default());
    let team = plan.repo("in-team").unwrap();
    assert_eq!(team.registry.as_ref().unwrap().value, "team");
    let files: Vec<&Path> = team
        .always
        .iter()
        .filter_map(|a| match a {
            AlwaysOn::File { path, .. } => Some(path.as_path()),
            _ => None,
        })
        .collect();
    assert_eq!(
        files,
        [
            Path::new("/st/conventions/org.md"),
            Path::new("/st/conventions/team.md"),
            Path::new("/st/conventions/extra.md")
        ]
    );
    assert_eq!(plan.registry("org").unwrap().entries.len(), 2);
    let team_registry = plan.registry("team").unwrap();
    assert_eq!(team_registry.entries.len(), 1);
    assert_eq!(
        team_registry.entries[0].notes.as_deref(),
        Some(Path::new("/st/repos/in-team/notes.md"))
    );
}
