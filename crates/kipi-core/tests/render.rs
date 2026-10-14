//! Rendering (roadmap §8): registries and generated blocks for every example repo
//! (snapshots), the token budget of the §3 walkthrough, edge cases on in-memory stores,
//! and splicing the block into existing files.

use std::fs;
use std::path::{Path, PathBuf};

use kipi_core::config::{self, Env};
use kipi_core::error::{Error, ErrorKind};
use kipi_core::model::{GrantMode, Machine, Related, StoreSet};
use kipi_core::plan::Plan;
use kipi_core::render::block::{self, BEGIN, Conventions, END, RenderedBlock};
use kipi_core::render::registry;
use kipi_core::render::splice::splice;
use kipi_core::resolve::resolve;
use kipi_test_support::builders::{MachineBuilder, StoreBuilder, store_set};
use kipi_test_support::stores::ExampleStore;
use kipi_test_support::workspace_root;

/// `examples/` doubles as the home directory, so store paths render as `~/acme-store/…`
/// on every machine and the snapshots stay stable.
fn home() -> PathBuf {
    workspace_root().join("examples")
}

fn env() -> Env {
    let mut env = Env::from_lookup(|_| None, Some(home())).unwrap();
    env.config_dir = workspace_root().join("examples");
    env
}

fn examples(stores: &[ExampleStore]) -> Plan {
    let paths: Vec<PathBuf> = stores.iter().map(|s| s.source_path()).collect();
    let set = config::load_stores(&paths, &env()).unwrap_or_else(|e| panic!("{}", e.report()));
    let machine = config::load_machine(&env()).unwrap_or_else(|e| panic!("{}", e.report()));
    ok(&set, &machine)
}

fn ok(set: &StoreSet, machine: &Machine) -> Plan {
    resolve(set, machine).unwrap_or_else(|e| panic!("{}", e.report()))
}

/// `repo`'s block as the file content sync would create.
fn block_file(plan: &Plan, repo: &str, home: Option<&Path>) -> String {
    let repo = plan.repo(repo).unwrap();
    let conventions = block::load_conventions(repo).unwrap_or_else(|e| panic!("{}", e.report()));
    let block = block::render(plan, repo, &conventions, home);
    splice(Path::new("CLAUDE.md"), None, &block).unwrap()
}

// ---------------------------------------------------------------------------------
// Example stores

#[test]
fn registry_snapshots() {
    let plan = examples(&ExampleStore::ALL);
    let scopes: Vec<&str> = plan.registries.iter().map(|r| r.scope.as_str()).collect();
    assert_eq!(scopes, ["acme", "personal"]);
    for reg in &plan.registries {
        insta::assert_snapshot!(
            format!("registry-{}", reg.scope),
            registry::render(reg, Some(&home()))
        );
    }
}

#[test]
fn block_snapshots_for_every_example_repo() {
    let plan = examples(&ExampleStore::ALL);
    assert_eq!(plan.repos.len(), 10);
    for repo in &plan.repos {
        insta::assert_snapshot!(
            format!("block-{}", repo.name),
            block_file(&plan, &repo.name, Some(&home()))
        );
    }
}

#[test]
fn personal_store_alone_renders_the_same_personal_blocks() {
    let both = examples(&ExampleStore::ALL);
    let alone = examples(&[ExampleStore::Personal]);
    for repo in &alone.repos {
        assert_eq!(
            block_file(&alone, &repo.name, Some(&home())),
            block_file(&both, &repo.name, Some(&home())),
            "{}",
            repo.name
        );
    }
}

#[test]
fn no_absolute_home_path_reaches_the_rendered_text() {
    let plan = examples(&ExampleStore::ALL);
    let home = home().display().to_string();
    for reg in &plan.registries {
        assert!(!registry::render(reg, Some(Path::new(&home))).contains(&home));
    }
    for repo in &plan.repos {
        let text = block_file(&plan, &repo.name, Some(Path::new(&home)));
        assert!(!text.contains(&home), "{}:\n{text}", repo.name);
    }
}

#[test]
fn work_content_never_reaches_a_personal_block() {
    let plan = examples(&ExampleStore::ALL);
    for repo in plan.repos.iter().filter(|r| r.store == "personal") {
        let text = block_file(&plan, &repo.name, Some(&home()));
        for work in ["acme", "billing", "events-pipeline", "Volumes"] {
            assert!(
                !text.contains(work),
                "{} mentions {work}:\n{text}",
                repo.name
            );
        }
    }
}

/// Token counts stay within the §3 walkthrough (and §7 `tree` sketch) estimates:
/// acme conventions ~300, acme registry ~350, personal registry ~210, and the
/// events-pipeline block (conventions + registry + this repo) ≈ 1.3k minus ~600 for
/// skill descriptions.
#[test]
fn example_rendering_stays_within_the_walkthrough_token_estimates() {
    let bpe = tiktoken_rs::cl100k_base().unwrap();
    let tokens = |text: &str| bpe.encode_with_special_tokens(text).len();
    let plan = examples(&ExampleStore::ALL);
    let home = home();
    let reg = |scope: &str| registry::render(plan.registry(scope).unwrap(), Some(&home));
    let events = plan.repo("events-pipeline").unwrap();
    let conventions = block::load_conventions(events).unwrap();
    let acme_conventions: String = conventions.iter().map(|c| c.text.as_str()).collect();
    let block = block::render(&plan, events, &conventions, Some(&home)).text();

    let counts = [
        ("acme conventions", tokens(&acme_conventions), 300),
        ("acme registry", tokens(&reg("acme")), 350),
        ("personal registry", tokens(&reg("personal")), 210),
        ("events-pipeline block", tokens(&block), 700),
    ];
    for (what, count, estimate) in counts {
        assert!(count <= estimate, "{what}: {count} tokens > {estimate}");
    }
}

// ---------------------------------------------------------------------------------
// Edge cases on in-memory stores

/// Scope `team` (registry) extends `org`; `me` relates to a repo in `org` (outside the
/// registry), one not cloned here, and one in the registry.
fn mixed_related(grant: Option<GrantMode>) -> (StoreSet, Machine) {
    mixed_related_with(grant, None)
}

/// [`mixed_related`], with `via` on the relation to `up`.
fn mixed_related_with(grant: Option<GrantMode>, via: Option<&str>) -> (StoreSet, Machine) {
    let set = store_set(vec![
        StoreBuilder::new("s", "/st")
            .defaults(|d| d.grant = grant)
            .scope("org", |_| {})
            .scope("team", |s| {
                s.extends = Some("org".into());
                s.registry = true;
            })
            .repo("me", |r| {
                r.scope = Some("team".into());
                r.related = vec!["up".into(), "gone".into(), "peer".into()];
                r.related[0].via = via.map(str::to_owned);
            })
            .repo("up", |r| r.scope = Some("org".into()))
            .repo("gone", |r| r.scope = Some("team".into()))
            .repo("peer", |r| r.scope = Some("team".into()))
            .notes("up")
            .build(),
    ]);
    let machine = MachineBuilder::new()
        .repo("me", "/h/c/me")
        .repo("up", "/h/c/up")
        .repo("peer", "/h/c/peer")
        .build();
    (set, machine)
}

#[test]
fn related_statuses_differ_and_unlisted_related_repos_get_their_own_line() {
    let (set, machine) = mixed_related(None);
    let plan = ok(&set, &machine);
    let me = plan.repo("me").unwrap();
    let body = block::render(&plan, me, &[], Some(Path::new("/h")))
        .body()
        .to_owned();
    assert_eq!(
        body,
        "## Known repos (team)\n\
         - gone · (not cloned here)\n\
         - me · ~/c/me\n\
         - peer · ~/c/peer\n\
         \n\
         ## This repo\n\
         Scope: team. Related: up (read access granted), gone (not cloned here), peer (read access granted).\n\
         - up · ~/c/up · notes: /st/repos/up/notes.md\n\
         For any repo above, read its code at the listed path; check its notes first if listed.\n\
         Lookup: `kipi where <alias>`, `kipi grep <term> --repos`.\n"
    );
}

#[test]
fn grant_none_says_access_is_not_granted() {
    let (set, machine) = mixed_related(Some(GrantMode::None));
    let plan = ok(&set, &machine);
    let body = block::render(&plan, plan.repo("me").unwrap(), &[], None);
    assert!(
        body.body().contains(
            "Related: up (read access not granted), gone (not cloned here), peer (read access not granted)."
        ),
        "{}",
        body.body()
    );
}

#[test]
fn via_follows_its_related_repo_with_or_without_a_shared_status() {
    // Statuses differ: each repo's parentheses hold its `via`, then its status.
    let (set, machine) = mixed_related_with(None, Some("shares its event schema"));
    let plan = ok(&set, &machine);
    let body = block::render(&plan, plan.repo("me").unwrap(), &[], None);
    insta::assert_snapshot!("block-via-mixed-status", body.body());

    // One status for all: names with their `via`, then the status after a dash; with
    // no `via` at all the line is as before.
    let etl = |via: Option<&str>| {
        let set = store_set(vec![
            StoreBuilder::new("s", "/st")
                .repo("etl", |r| {
                    r.related = vec![
                        Related {
                            name: "app".into(),
                            via: via.map(str::to_owned),
                        },
                        "billing-api".into(),
                    ];
                })
                .repo("app", |_| {})
                .repo("billing-api", |_| {})
                .build(),
        ]);
        let machine = MachineBuilder::new()
            .repo("etl", "/h/c/etl")
            .repo("app", "/h/c/app")
            .repo("billing-api", "/h/c/billing-api")
            .build();
        let plan = ok(&set, &machine);
        let etl = plan.repo("etl").unwrap();
        assert_eq!(etl.related[0].via.as_deref(), via);
        block::render(&plan, etl, &[], None).body().to_owned()
    };
    let described = etl(Some("marts.* tables consumed by app's loaders"));
    assert!(
        described.contains(
            "Related: app (marts.* tables consumed by app's loaders), billing-api — read access \
             granted."
        ),
        "{described}"
    );
    let plain = etl(None);
    assert!(
        plain.contains("Related: app, billing-api (read access granted)."),
        "{plain}"
    );
}

#[test]
fn unscoped_repo_gets_this_repo_section_only() {
    let set = store_set(vec![
        StoreBuilder::new("s", "/st").repo("a", |_| {}).build(),
    ]);
    let plan = ok(&set, &Machine::default());
    let block = block::render(&plan, plan.repo("a").unwrap(), &[], None);
    assert_eq!(
        block.text(),
        format!(
            "{BEGIN}\n## This repo\nScope: none (unscoped).\n\
             Lookup: `kipi where <alias>`, `kipi grep <term> --repos`.\n{END}"
        )
    );
}

#[test]
fn conventions_group_per_scope_top_down_and_skip_empty_files() {
    let set = store_set(vec![
        StoreBuilder::new("s", "/st")
            .scope("org", |_| {})
            .scope("team", |s| s.extends = Some("org".into()))
            .repo("a", |r| r.scope = Some("team".into()))
            .build(),
    ]);
    let plan = ok(&set, &Machine::default());
    let conv = |scope: &str, text: &str| Conventions {
        scope: scope.into(),
        path: PathBuf::from("/st/conventions/x.md"),
        text: text.into(),
    };
    let conventions = [
        conv("org", "- one\n\n"),
        conv("org", "- two"),
        conv("team", "  \n"),
        conv("team", "- three\n"),
    ];
    let body = block::render(&plan, plan.repo("a").unwrap(), &conventions, None);
    assert!(
        body.body().starts_with(
            "## Conventions (org)\n- one\n\n- two\n\n## Conventions (team)\n- three\n\n## This repo\n"
        ),
        "{}",
        body.body()
    );
}

#[test]
fn conventions_errors_have_location_and_hint() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("conventions")).unwrap();
    let set = store_set(vec![
        StoreBuilder::new("s", root)
            .scope("a", |s| {
                s.always = vec![PathBuf::from("conventions/a.md")];
            })
            .repo("x", |r| r.scope = Some("a".into()))
            .build(),
    ]);
    let plan = ok(&set, &Machine::default());
    let repo = plan.repo("x").unwrap();

    let missing = block::load_conventions(repo).unwrap_err();
    assert_eq!(missing.kind(), ErrorKind::Io);
    assert!(
        missing.hint().contains("scopes.a.always"),
        "{}",
        missing.report()
    );

    fs::write(root.join("conventions/a.md"), format!("ok\n{END}\n")).unwrap();
    let marker = block::load_conventions(repo).unwrap_err();
    let Error::Invalid { at, .. } = &marker else {
        panic!("{marker:?}")
    };
    assert_eq!(at.line, Some(2));
    assert!(marker.hint().contains("kipi:"), "{}", marker.report());

    fs::write(root.join("conventions/a.md"), "- fine\n").unwrap();
    let loaded = block::load_conventions(repo).unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].text, "- fine\n");
}

// ---------------------------------------------------------------------------------
// Splicing

fn sample() -> RenderedBlock {
    let set = store_set(vec![
        StoreBuilder::new("s", "/st").repo("a", |_| {}).build(),
    ]);
    let plan = ok(&set, &Machine::default());
    block::render(&plan, plan.repo("a").unwrap(), &[], None)
}

fn spliced(existing: Option<&str>) -> String {
    splice(Path::new("CLAUDE.md"), existing, &sample()).unwrap()
}

#[test]
fn splice_creates_and_appends_after_a_blank_line() {
    let b = sample().text();
    assert_eq!(spliced(None), format!("{b}\n"));
    assert_eq!(spliced(Some("")), format!("{b}\n"));
    assert_eq!(spliced(Some("# Me")), format!("# Me\n\n{b}\n"));
    assert_eq!(spliced(Some("# Me\n")), format!("# Me\n\n{b}\n"));
    assert_eq!(spliced(Some("# Me\n\n")), format!("# Me\n\n{b}\n"));
}

#[test]
fn splice_replaces_exactly_the_marked_region() {
    let b = sample().text();
    let old = format!("intro\r\n{BEGIN}\r\nstale\r\n{END}  \r\noutro");
    assert_eq!(spliced(Some(&old)), format!("intro\r\n{b}  \r\noutro"));
    let custom_begin = format!("{}\n<!-- kipi:begin -->\nx\n{END}", "top");
    assert_eq!(spliced(Some(&custom_begin)), format!("top\n{b}"));
}

#[test]
fn splice_rejects_duplicated_or_unbalanced_markers() {
    let cases = [
        (
            format!("{BEGIN}\n{END}\n{BEGIN}\n{END}\n"),
            3,
            "duplicated kipi:begin",
        ),
        (format!("{BEGIN}\n{END}\n{END}\n"), 3, "duplicated kipi:end"),
        (format!("a\n{BEGIN}\nb\n"), 2, "without a kipi:end"),
        (format!("a\nb\n{END}\n"), 3, "without a kipi:begin"),
        (format!("{END}\n{BEGIN}\n"), 1, "before its kipi:begin"),
    ];
    for (text, line, message) in cases {
        let e = splice(Path::new("CLAUDE.md"), Some(&text), &sample()).unwrap_err();
        assert_eq!(e.location().unwrap().line, Some(line), "{text:?}");
        assert!(e.to_string().contains(message), "{e}");
        assert!(e.hint().contains("kipi sync"), "{}", e.report());
    }
}

#[test]
fn markers_only_count_at_the_start_of_a_line() {
    let text = format!("see `{BEGIN}` and `{END}` in the docs\n");
    let b = sample().text();
    assert_eq!(spliced(Some(&text)), format!("{text}\n{b}\n"));
}
