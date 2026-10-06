//! Doctor (roadmap §10): one fixture per check code that triggers exactly that finding,
//! then the example stores, which must come out without errors.

use std::fs;
use std::path::{Path, PathBuf};

use kipi_core::config::{self, Env};
use kipi_core::doctor::{
    self, CheckoutInfo, Checkouts, Code, Finding, Fix, Inputs, Level, diagnose,
};
use kipi_core::model::{Machine, Store};
use kipi_core::plan::{GrantDir, GrantKind, Source};
use kipi_test_support::builders::{
    MachineBuilder, StoreBuilder, draft, remote, reviewed, skill_refs,
};
use kipi_test_support::repos::{FakeRepo, ManifestKind, create_fake_repo};
use kipi_test_support::stores::ExampleStore;
use kipi_test_support::workspace_root;
use tempfile::TempDir;

const HOME: &str = "/HOME";
const WORK: &str = "git.example.com/work/";

/// Store roots in a tempdir, each with a `bindings.toml` on disk (the home-path check
/// reads it) whose content is up to the test.
struct Fx {
    dir: TempDir,
}

impl Fx {
    fn new() -> Self {
        Fx {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    /// The root of store `name`, created with a harmless `bindings.toml`.
    fn root(&self, name: &str) -> PathBuf {
        let root = self.dir.path().join(name);
        fs::create_dir_all(&root).unwrap();
        let bindings = root.join("bindings.toml");
        if !bindings.exists() {
            fs::write(
                &bindings,
                format!("[store]\nname = \"{name}\"\nversion = 1\n"),
            )
            .unwrap();
        }
        root
    }

    /// A store `work` with scope `work` (roots `/w`, a registry, `remote_patterns`
    /// [`WORK`]), a skill `review` the scope uses, and repo `api` with a reviewed summary,
    /// a remote and its checkout at `/w/api`. Doctor finds nothing in it.
    fn work(&self) -> StoreBuilder {
        StoreBuilder::new("work", self.root("work"))
            .scope("work", |s| {
                s.roots = vec!["/w".into()];
                s.registry = true;
                s.remote_patterns = vec![WORK.into()];
                s.skills = skill_refs(&["review"]);
            })
            .skill("review", Some("work"))
            .repo("api", |r| {
                r.summary = Some(reviewed("The API".into()));
                r.remote = Some(remote("git.example.com/work/api"));
            })
    }
}

/// Summary set, so K008 stays quiet.
fn summarized(r: &mut kipi_core::model::RepoEntry) {
    r.summary = Some(reviewed(format!("The {} repo", r.name)));
}

fn machine() -> MachineBuilder {
    MachineBuilder::new().repo("api", "/w/api")
}

fn run(stores: Vec<Store>, machine: &Machine, checkouts: &Checkouts) -> Vec<Finding> {
    diagnose(stores, machine, checkouts, Path::new(HOME), None)
        .unwrap_or_else(|e| panic!("{}", e.report()))
        .findings
}

/// The only finding, which must be of `code`, at its level, with a hint.
fn exactly_one(findings: Vec<Finding>, code: Code) -> Finding {
    assert_eq!(
        findings.len(),
        1,
        "expected exactly one {code} finding, got {findings:#?}"
    );
    let f = findings.into_iter().next().unwrap();
    assert_eq!(f.code, code, "{f:#?}");
    assert_eq!(f.level, code.level());
    assert!(
        !f.hint.trim().is_empty(),
        "every finding has a hint: {f:#?}"
    );
    assert!(!f.message.trim().is_empty());
    f
}

fn checkouts(list: &[(&str, &str, Option<&str>)]) -> Checkouts {
    Checkouts {
        by_entry: list
            .iter()
            .map(|(entry, path, origin)| {
                let info = CheckoutInfo {
                    path: PathBuf::from(path),
                    origin: origin.map(remote),
                };
                ((*entry).to_owned(), info)
            })
            .collect(),
    }
}

#[test]
fn the_base_fixture_is_clean() {
    let fx = Fx::new();
    let findings = run(
        vec![fx.work().build()],
        &machine().build(),
        &Checkouts::default(),
    );
    assert_eq!(findings, vec![]);
}

#[test]
fn codes_are_stable_and_unique() {
    let codes: Vec<&str> = Code::ALL.iter().map(|c| c.as_str()).collect();
    let want: Vec<String> = (1..=15).map(|n| format!("K{n:03}")).collect();
    assert_eq!(codes, want);
    let docs = fs::read_to_string(workspace_root().join("docs/doctor.md")).unwrap();
    for code in Code::ALL {
        assert!(
            docs.contains(&format!("## {} {}", code.as_str(), code.name())),
            "docs/doctor.md documents {code} as `## {} {}`",
            code.as_str(),
            code.name()
        );
    }
}

#[test]
fn k001_remote_matches_a_pattern_but_resolved_elsewhere() {
    let fx = Fx::new();
    let store = fx
        .work()
        .scope("side", |s| s.roots = vec!["/s".into()])
        .repo("tool", |r| {
            summarized(r);
            r.remote = Some(remote("git.example.com/work/tool"));
        })
        .build();
    let machine = machine().repo("tool", "/s/tool").build();
    let f = exactly_one(
        run(vec![store], &machine, &Checkouts::default()),
        Code::RemoteScopeMismatch,
    );
    assert_eq!(f.repo.as_deref(), Some("tool"));
    assert!(f.hint.contains("`scope = \"work\"`"), "{}", f.hint);
}

#[test]
fn k001_accepts_a_pattern_of_an_ancestor_scope() {
    let fx = Fx::new();
    let store = fx
        .work()
        .scope("work-ml", |s| {
            s.extends = Some("work".into());
            s.roots = vec!["/w/ml".into()];
        })
        .repo("model", |r| {
            summarized(r);
            r.remote = Some(remote("git.example.com/work/model"));
        })
        .build();
    let machine = machine().repo("model", "/w/ml/model").build();
    assert_eq!(run(vec![store], &machine, &Checkouts::default()), vec![]);
}

#[test]
fn k002_fallback_scope_whose_roots_miss_the_checkout() {
    let fx = Fx::new();
    let store = StoreBuilder::new("me", fx.root("me"))
        .scope("personal", |s| s.roots = vec!["/code".into()])
        .repo("tool", summarized)
        .build();
    let machine = MachineBuilder::new()
        .repo("tool", "/elsewhere/tool")
        .build();
    let f = exactly_one(
        run(vec![store], &machine, &Checkouts::default()),
        Code::RootNotAncestor,
    );
    assert!(f.message.contains("/elsewhere/tool"), "{}", f.message);
}

#[test]
fn k002_respects_machine_roots_and_explicit_scopes() {
    let fx = Fx::new();
    let store = StoreBuilder::new("me", fx.root("me"))
        .scope("personal", |s| s.roots = vec!["/code".into()])
        .repo("tool", summarized)
        .repo("pinned", |r| {
            summarized(r);
            r.scope = Some("personal".into());
        })
        .build();
    let machine = MachineBuilder::new()
        .roots("personal", &["/srv"])
        .repo("tool", "/srv/tool")
        .repo("pinned", "/elsewhere/pinned")
        .build();
    assert_eq!(run(vec![store], &machine, &Checkouts::default()), vec![]);
}

/// Store `work` with sibling scopes `a` and `b` (neither extends the other) and repos
/// `x` in `a`, `y` in `b`.
fn siblings(fx: &Fx, x: impl FnOnce(&mut kipi_core::model::RepoEntry)) -> (Store, Machine) {
    let store = StoreBuilder::new("work", fx.root("work"))
        .scope("a", |s| s.roots = vec!["/a".into()])
        .scope("b", |s| s.roots = vec!["/b".into()])
        .repo("x", |r| {
            summarized(r);
            x(r);
        })
        .repo("y", summarized)
        .build();
    let machine = MachineBuilder::new()
        .repo("x", "/a/x")
        .repo("y", "/b/y")
        .build();
    (store, machine)
}

#[test]
fn k003_related_repo_in_an_unrelated_scope() {
    let fx = Fx::new();
    let (store, machine) = siblings(&fx, |r| r.related = vec!["y".into()]);
    let diagnosis = diagnose(
        vec![store],
        &machine,
        &Checkouts::default(),
        Path::new(HOME),
        None,
    )
    .unwrap_or_else(|e| panic!("{}", e.report()));
    let f = exactly_one(diagnosis.findings, Code::ScopeCrossing);
    assert_eq!(f.repo.as_deref(), Some("x"));
    // The relation was removed so the rest resolved, and nothing was granted across.
    let x = diagnosis.plan.repo("x").unwrap();
    assert!(x.related.is_empty());
    assert!(!x.grant.contains(Path::new("/b/y")));
}

#[test]
fn k003_grant_crossing_scopes_in_a_plan() {
    let fx = Fx::new();
    let (store, machine) = siblings(&fx, |_| {});
    let checkouts = Checkouts::default();
    let diagnosis = diagnose(vec![store], &machine, &checkouts, Path::new(HOME), None).unwrap();
    assert_eq!(diagnosis.findings, vec![]);
    // Resolution never grants across scopes; doctor double-checks the plan anyway.
    let mut plan = diagnosis.plan.clone();
    let x = plan.repos.iter_mut().find(|r| r.name == "x").unwrap();
    x.grant.dirs.push(GrantDir {
        path: "/b/y".into(),
        kind: GrantKind::Repo { name: "y".into() },
        source: Source::builtin("tampered"),
    });
    let inputs = Inputs {
        stores: &diagnosis.stores,
        machine: &machine,
        plan: &plan,
        checkouts: &checkouts,
        home: Path::new(HOME),
        only: None,
    };
    let f = exactly_one(doctor::check(&inputs).unwrap(), Code::ScopeCrossing);
    assert!(f.message.contains("repo `y` (scope `b`)"), "{}", f.message);
}

#[test]
fn k004_alias_collision_in_a_registry() {
    let fx = Fx::new();
    let store = fx
        .work()
        .repo("web", |r| {
            summarized(r);
            r.aliases = reviewed(vec!["Front End".into()]);
        })
        .repo("web-old", |r| {
            summarized(r);
            r.aliases = draft(vec!["front  end".into()]);
        })
        .build();
    let machine = machine()
        .repo("web", "/w/web")
        .repo("web-old", "/w/web-old")
        .build();
    let f = exactly_one(
        run(vec![store], &machine, &Checkouts::default()),
        Code::AliasCollision,
    );
    assert!(f.message.contains("`front end`"), "{}", f.message);
    assert!(f.message.contains("`web` and `web-old`"), "{}", f.message);
}

#[test]
fn k005_skill_missing_from_its_store() {
    let fx = Fx::new();
    let store = fx
        .work()
        .scope("misc", |s| s.skills = skill_refs(&["revew"]))
        .build();
    let f = exactly_one(
        run(vec![store], &machine().build(), &Checkouts::default()),
        Code::MissingSkill,
    );
    assert_eq!(f.repo, None);
    assert_eq!(f.hint, "did you mean `review`?");
}

#[test]
fn k005_qualified_skill_missing_from_the_sharing_store() {
    let fx = Fx::new();
    let shared = StoreBuilder::new("me", fx.root("me"))
        .share_skills(true)
        .build();
    let store = fx
        .work()
        .repo("tool", |r| {
            summarized(r);
            r.skills = skill_refs(&["me/nope"]);
        })
        .build();
    let machine = machine().repo("tool", "/w/tool").build();
    let f = exactly_one(
        run(vec![shared, store], &machine, &Checkouts::default()),
        Code::MissingSkill,
    );
    assert_eq!(f.repo.as_deref(), Some("tool"));
    assert!(f.message.contains("has no skill `nope`"), "{}", f.message);
}

#[test]
fn k006_home_path_in_bindings() {
    let fx = Fx::new();
    let store = fx.work().build();
    let users = format!("/{}/{}", "Users", "someone");
    fs::write(
        &store.bindings.path,
        format!("[store]\nname = \"work\"\n# old root: {users}/w\n"),
    )
    .unwrap();
    let f = exactly_one(
        run(vec![store], &machine().build(), &Checkouts::default()),
        Code::HomePath,
    );
    assert_eq!(f.repo, None);
    assert_eq!(f.at.as_ref().and_then(|a| a.line), Some(3));
    assert!(
        f.message.starts_with("work:bindings.toml:3 holds"),
        "{}",
        f.message
    );
}

#[test]
fn k006_home_path_in_notes_and_conventions() {
    let fx = Fx::new();
    let root = fx.root("work");
    let store = fx
        .work()
        .scope("work-ml", |s| s.always = vec!["conventions/ml.md".into()])
        .build();
    fs::create_dir_all(root.join("repos/api")).unwrap();
    fs::write(
        root.join("repos/api/notes.md"),
        format!("Clone it to {HOME}/w/api.\n"),
    )
    .unwrap();
    let f = exactly_one(
        run(
            vec![store.clone()],
            &machine().build(),
            &Checkouts::default(),
        ),
        Code::HomePath,
    );
    assert_eq!(f.repo.as_deref(), Some("api"));
    fs::remove_file(root.join("repos/api/notes.md")).unwrap();

    fs::create_dir_all(root.join("conventions")).unwrap();
    fs::write(
        root.join("conventions/ml.md"),
        format!("Data in {}/{}/data\n", "/home", "dev"),
    )
    .unwrap();
    let f = exactly_one(
        run(vec![store], &machine().build(), &Checkouts::default()),
        Code::HomePath,
    );
    assert_eq!(f.repo, None);
    assert!(
        f.message.contains("work:conventions/ml.md:1"),
        "{}",
        f.message
    );
}

#[test]
fn k007_unscoped_repo() {
    let fx = Fx::new();
    let store = fx.work().repo("stray", summarized).build();
    let machine = machine().repo("stray", "/tmp/stray").build();
    let f = exactly_one(
        run(vec![store], &machine, &Checkouts::default()),
        Code::Unscoped,
    );
    assert_eq!(f.repo.as_deref(), Some("stray"));
}

#[test]
fn k008_missing_or_draft_summary() {
    let fx = Fx::new();
    let store = fx.work().repo("web", |_| {}).build();
    let machine = machine().repo("web", "/w/web").build();
    let f = exactly_one(
        run(vec![store], &machine, &Checkouts::default()),
        Code::Summary,
    );
    assert!(f.message.contains("no summary"), "{}", f.message);

    let store = fx
        .work()
        .repo("web", |r| r.summary = Some(draft("Drafted".into())))
        .build();
    let f = exactly_one(
        run(vec![store], &machine, &Checkouts::default()),
        Code::Summary,
    );
    assert!(f.message.contains("still a draft"), "{}", f.message);
}

#[test]
fn k009_skill_description_missing_or_too_long() {
    let fx = Fx::new();
    let mut store = fx.work().skill("lint", Some("work")).build();
    store
        .skills
        .iter_mut()
        .find(|s| s.name == "lint")
        .unwrap()
        .description = None;
    let f = exactly_one(
        run(
            vec![store.clone()],
            &machine().build(),
            &Checkouts::default(),
        ),
        Code::SkillDescription,
    );
    assert!(f.message.contains("no `description`"), "{}", f.message);

    store
        .skills
        .iter_mut()
        .find(|s| s.name == "lint")
        .unwrap()
        .description = Some("x".repeat(1024));
    assert_eq!(
        run(
            vec![store.clone()],
            &machine().build(),
            &Checkouts::default()
        ),
        vec![]
    );
    store
        .skills
        .iter_mut()
        .find(|s| s.name == "lint")
        .unwrap()
        .description = Some("x".repeat(1025));
    let f = exactly_one(
        run(vec![store], &machine().build(), &Checkouts::default()),
        Code::SkillDescription,
    );
    assert!(f.message.contains("1025 characters"), "{}", f.message);
}

#[test]
fn k010_notes_over_budget() {
    let fx = Fx::new();
    let root = fx.root("work");
    fs::create_dir_all(root.join("repos/api")).unwrap();
    let notes = root.join("repos/api/notes.md");
    let store = fx.work().notes("api").build();
    fs::write(&notes, "word ".repeat(1200)).unwrap(); // 6,000 chars: 1,500 tokens
    assert_eq!(
        run(
            vec![store.clone()],
            &machine().build(),
            &Checkouts::default()
        ),
        vec![]
    );
    fs::write(&notes, "word ".repeat(1201)).unwrap();
    let f = exactly_one(
        run(vec![store], &machine().build(), &Checkouts::default()),
        Code::NotesSize,
    );
    assert_eq!(f.repo.as_deref(), Some("api"));
}

#[test]
fn k011_registry_over_forty_repos() {
    let fx = Fx::new();
    let with = |extra: usize| {
        // Not cloned here: placed in `work` by `remote_patterns`.
        let repo = |b: StoreBuilder, i| {
            b.repo(&format!("svc{i:02}"), |r| {
                summarized(r);
                r.remote = Some(remote(&format!("{WORK}svc{i:02}")));
            })
        };
        (1..=extra).fold(fx.work(), repo).build()
    };
    let check = |store: Store| run(vec![store], &machine().build(), &Checkouts::default());
    assert_eq!(check(with(39)), vec![], "40 repos is fine");
    let f = exactly_one(check(with(40)), Code::RegistrySize);
    assert!(f.message.contains("41 repos"), "{}", f.message);
}

#[test]
fn k012_reference_into_another_store() {
    let fx = Fx::new();
    let personal = StoreBuilder::new("me", fx.root("me"))
        .scope("personal", |s| s.roots = vec!["/code".into()])
        .skill("style", None)
        .build();
    let store = fx
        .work()
        .repo("tool", |r| {
            summarized(r);
            r.skills = skill_refs(&["me/style"]);
        })
        .build();
    let machine = machine().repo("tool", "/w/tool").build();
    let diagnosis = diagnose(
        vec![personal, store],
        &machine,
        &Checkouts::default(),
        Path::new(HOME),
        None,
    )
    .unwrap_or_else(|e| panic!("{}", e.report()));
    let f = exactly_one(diagnosis.findings, Code::CrossStore);
    assert_eq!(f.repo.as_deref(), Some("tool"));
    assert!(
        f.message.contains("does not share its skills"),
        "{}",
        f.message
    );
    assert!(
        diagnosis
            .plan
            .repo("tool")
            .unwrap()
            .skills
            .iter()
            .all(|s| s.store == "work")
    );
}

#[test]
fn k012_bare_cross_store_related_and_extends() {
    let fx = Fx::new();
    let personal = StoreBuilder::new("me", fx.root("me"))
        .scope("personal", |s| s.roots = vec!["/code".into()])
        .repo("blog", summarized)
        .build();
    let store = fx
        .work()
        .scope("work-x", |s| s.extends = Some("personal".into()))
        .repo("tool", |r| {
            summarized(r);
            r.related = vec!["blog".into()];
        })
        .build();
    let machine = machine()
        .repo("tool", "/w/tool")
        .repo("blog", "/code/blog")
        .build();
    let findings = run(vec![personal, store], &machine, &Checkouts::default());
    let codes: Vec<(Code, Option<&str>)> = findings
        .iter()
        .map(|f| (f.code, f.repo.as_deref()))
        .collect();
    assert_eq!(
        codes,
        vec![(Code::CrossStore, None), (Code::CrossStore, Some("tool"))],
        "{findings:#?}"
    );
}

#[test]
fn k013_entry_in_the_wrong_store() {
    let fx = Fx::new();
    let personal = StoreBuilder::new("me", fx.root("me"))
        .scope("personal", |s| {
            s.roots = vec!["/code".into()];
            s.remote_patterns = vec!["git.example.com/me/".into()];
        })
        .repo("billing", |r| {
            summarized(r);
            r.remote = Some(remote("git.example.com/work/billing"));
        })
        .build();
    let machine = machine().repo("billing", "/code/billing").build();
    let f = exactly_one(
        run(
            vec![personal, fx.work().build()],
            &machine,
            &Checkouts::default(),
        ),
        Code::WrongStore,
    );
    assert_eq!(f.repo.as_deref(), Some("billing"));
    assert!(f.hint.contains("to store `work`"), "{}", f.hint);
}

#[test]
fn k014_checkout_origin_differs_from_the_entry() {
    let fx = Fx::new();
    let found = checkouts(&[("api", "/w/api", Some("git.example.com/fork/api"))]);
    let f = exactly_one(
        run(vec![fx.work().build()], &machine().build(), &found),
        Code::CheckoutMismatch,
    );
    assert!(
        f.message.contains("origin `git.example.com/fork/api`"),
        "{}",
        f.message
    );
}

#[test]
fn k014_two_checkouts_match_one_entry() {
    let fx = Fx::new();
    let store = StoreBuilder::new("work", fx.root("work"))
        .scope("work", |s| s.roots = vec!["/w".into()])
        .repo("a", summarized)
        .repo("b", summarized)
        .build();
    let machine = MachineBuilder::new()
        .repo("a", "/w/a")
        .repo("b", "/w/old/a")
        .build();
    let found = checkouts(&[("a", "/w/a", None), ("b", "/w/old/a", None)]);
    let f = exactly_one(run(vec![store], &machine, &found), Code::CheckoutMismatch);
    assert_eq!(f.repo.as_deref(), Some("a"));
    assert!(
        f.message
            .starts_with("2 checkouts on this machine match entry `a`"),
        "{}",
        f.message
    );
}

#[test]
fn k015_entry_without_remote_whose_checkout_has_one() {
    let fx = Fx::new();
    let store = fx.work().repo("web", summarized).build();
    let machine = machine().repo("web", "/w/web").build();
    let found = checkouts(&[("web", "/w/web", Some("git.example.com/work/web"))]);
    let f = exactly_one(run(vec![store], &machine, &found), Code::MissingRemote);
    assert_eq!(
        f.fix,
        Some(Fix::FillRemote {
            store: "work".into(),
            repo: "web".into(),
            remote: remote("git.example.com/work/web"),
        })
    );
}

#[test]
fn only_keeps_one_repos_findings() {
    let fx = Fx::new();
    let store = fx
        .work()
        .scope("misc", |s| s.skills = skill_refs(&["revew"]))
        .repo("web", |_| {})
        .repo("web2", |r| r.aliases = reviewed(vec!["web".into()]))
        .build();
    let machine = machine()
        .repo("web", "/w/web")
        .repo("web2", "/w/web2")
        .build();
    let only = |name| {
        diagnose(
            vec![store.clone()],
            &machine,
            &Checkouts::default(),
            Path::new(HOME),
            Some(name),
        )
        .unwrap()
        .findings
        .into_iter()
        .map(|f| (f.code, f.repo))
        .collect::<Vec<_>>()
    };
    let some = |s: &str| Some(s.to_owned());
    assert_eq!(only("api"), vec![]);
    assert_eq!(
        only("web"),
        vec![
            (Code::AliasCollision, some("web")),
            (Code::Summary, some("web"))
        ]
    );
    assert_eq!(
        only("web2"),
        vec![
            (Code::AliasCollision, some("web2")),
            (Code::Summary, some("web2"))
        ]
    );
}

#[test]
fn findings_sort_errors_first() {
    let fx = Fx::new();
    let store = fx
        .work()
        .scope("misc", |s| s.skills = skill_refs(&["revew"]))
        .repo("web", |_| {})
        .repo("stray", summarized)
        .build();
    let machine = machine()
        .repo("web", "/w/web")
        .repo("stray", "/tmp/stray")
        .build();
    let levels: Vec<Level> = run(vec![store], &machine, &Checkouts::default())
        .iter()
        .map(|f| f.level)
        .collect();
    assert_eq!(levels, vec![Level::Error, Level::Warn, Level::Info]);
}

#[test]
fn unrelated_resolution_errors_still_fail() {
    let fx = Fx::new();
    let store = fx
        .work()
        .repo("tool", |r| r.scope = Some("nope".into()))
        .build();
    let err = diagnose(
        vec![store],
        &machine().build(),
        &Checkouts::default(),
        Path::new(HOME),
        None,
    )
    .expect_err("an unknown scope has no check of its own");
    assert!(err.to_string().contains("no such scope"), "{err}");
}

#[test]
fn checkouts_are_read_from_git() {
    let dir = tempfile::tempdir().unwrap();
    let repo = |entry: &str, origin: Option<&str>| FakeRepo {
        entry: entry.into(),
        dir: entry.into(),
        origin_url: origin.map(str::to_owned),
        readme: None,
        manifest: ManifestKind::None,
        description: None,
    };
    for r in [
        repo("ssh", Some("git@git.example.com:work/api.git")),
        repo("none", None),
        repo("local", Some("/srv/git/x.git")),
    ] {
        create_fake_repo(&dir.path().join(&r.dir), &r).unwrap();
    }
    let mut machine = MachineBuilder::new();
    for name in ["ssh", "none", "local", "gone"] {
        machine = machine.repo(name, dir.path().join(name));
    }
    let found = Checkouts::read(&machine.build());
    let origins: Vec<(&str, Option<&str>)> = found
        .by_entry
        .iter()
        .map(|(k, c)| (k.as_str(), c.origin.as_ref().map(|o| o.as_str())))
        .collect();
    assert_eq!(
        origins,
        vec![
            ("local", None),
            ("none", None),
            ("ssh", Some("git.example.com/work/api"))
        ]
    );
}

#[test]
fn example_stores_have_no_errors() {
    let mut env = Env::from_lookup(|_| None, Some(PathBuf::from(HOME))).unwrap();
    env.config_dir = workspace_root().join("examples");
    let stores: Vec<Store> = ExampleStore::ALL
        .iter()
        .map(|s| config::load_store(&s.source_path(), &env).unwrap())
        .collect();
    let machine = config::load_machine(&env).unwrap();
    let diagnosis = diagnose(
        stores,
        &machine,
        &Checkouts::default(),
        Path::new(HOME),
        None,
    )
    .unwrap_or_else(|e| panic!("{}", e.report()));
    assert!(!diagnosis.has_errors(), "{:#?}", diagnosis.findings);
    let summary: Vec<String> = diagnosis
        .findings
        .iter()
        .map(|f| {
            format!(
                "{} {} {} {}",
                f.level,
                f.code,
                f.repo.as_deref().unwrap_or("-"),
                f.message
            )
        })
        .collect();
    insta::assert_snapshot!(summary.join("\n"));
}
