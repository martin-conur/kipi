//! The authoring commands against copies of the example stores in a temporary home:
//! comment-preserving edits, byte-identical bind/unbind round trips, and edits rejected
//! (nothing written) when the edited stores no longer resolve.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use kipi_core::authoring::{
    self, Binding, InitOptions, Op, Outcome, RemoveOptions, RepoOptions, SkillOptions,
};
use kipi_core::config::{self, Env};
use kipi_core::model::{Machine, StoreSet};
use kipi_test_support::repos::{FakeRepo, ManifestKind};
use kipi_test_support::stores::{ExampleStore, copy_example_store_to};
use tempfile::TempDir;

struct Home {
    dir: TempDir,
}

impl Home {
    fn new() -> Self {
        let home = Home {
            dir: tempfile::tempdir().unwrap(),
        };
        for s in ExampleStore::ALL {
            copy_example_store_to(s, &home.store(s)).unwrap();
        }
        home
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn store(&self, s: ExampleStore) -> PathBuf {
        self.path().join("stores").join(s.dir_name())
    }

    fn bindings(&self, s: ExampleStore) -> String {
        fs::read_to_string(self.store(s).join("bindings.toml")).unwrap()
    }

    fn env(&self) -> Env {
        Env::from_lookup(|_| None, Some(self.path().to_path_buf())).unwrap()
    }

    fn stores(&self) -> StoreSet {
        let paths: Vec<PathBuf> = ExampleStore::ALL.iter().map(|&s| self.store(s)).collect();
        config::load_stores(&paths, &self.env()).unwrap()
    }

    fn machine(&self) -> Machine {
        config::load_machine(&self.env()).unwrap()
    }

    fn bind(&self, b: &Binding, adding: bool) -> kipi_core::error::Result<Outcome> {
        let (stores, machine, env) = (self.stores(), self.machine(), self.env());
        if adding {
            authoring::bind(&stores, &machine, &env, b)
        } else {
            authoring::unbind(&stores, &machine, &env, b)
        }
    }
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| (*v).to_owned()).collect()
}

#[test]
fn bind_then_unbind_is_byte_identical_for_every_example_repo() {
    let home = Home::new();
    let originals: Vec<String> = ExampleStore::ALL
        .iter()
        .map(|&s| home.bindings(s))
        .collect();
    let stores = home.stores();
    for (store, repo) in stores.repos() {
        let skill = store
            .skills
            .iter()
            .find(|s| !repo.skills.iter().any(|r| r.name == s.name))
            .map(|s| s.name.clone())
            .unwrap();
        let related = store
            .repos
            .iter()
            .find(|r| r.name != repo.name && !repo.related.contains(&r.name))
            .map(|r| r.name.clone())
            .unwrap();
        let exclude = store.skills[0].name.clone();
        let (skills, related, exclude) = (vec![skill], vec![related], vec![exclude]);
        let aliases = if repo.aliases.is_draft {
            Vec::new()
        } else {
            vec![format!("zz-{}", repo.name)]
        };
        let b = Binding {
            repo: &repo.name,
            skills: &skills,
            related: &related,
            exclude: if repo.exclude.is_empty() {
                &exclude
            } else {
                &[]
            },
            aliases: &aliases,
        };
        let bound = home.bind(&b, true).unwrap();
        assert_eq!(bound.writes.len(), 1, "{}", repo.name);
        bound.write().unwrap();
        home.bind(&b, false).unwrap().write().unwrap();
        let now: Vec<String> = ExampleStore::ALL
            .iter()
            .map(|&s| home.bindings(s))
            .collect();
        assert_eq!(now, originals, "round trip of {}", repo.name);
    }
}

#[test]
fn bind_keeps_comments_order_and_blank_lines() {
    let home = Home::new();
    let b = Binding {
        repo: "events-pipeline",
        skills: &strings(&["code-review"]),
        related: &strings(&["feature-store"]),
        exclude: &strings(&["dbt-conventions"]),
        aliases: &strings(&["kafka"]),
    };
    let outcome = home.bind(&b, true).unwrap();
    let ops: Vec<(Op, &str, &str)> = outcome
        .changes
        .iter()
        .map(|c| (c.op, c.what.as_str(), c.value.as_str()))
        .collect();
    assert_eq!(
        ops,
        [
            (Op::Add, "skills", "code-review"),
            (Op::Add, "related", "feature-store"),
            (Op::Add, "exclude", "dbt-conventions"),
            (Op::Add, "aliases", "kafka"),
        ]
    );
    insta::assert_snapshot!("bind-events-pipeline", outcome.writes[0].after);

    // Adding a new key appends it to the entry; removing it takes its line away again.
    let b = Binding {
        repo: "acme-api",
        skills: &strings(&["code-review", "personal/rust-style"]),
        ..Binding::default()
    };
    let outcome = home.bind(&b, true).unwrap();
    insta::assert_snapshot!("bind-acme-api-new-key", outcome.writes[0].after);
}

#[test]
fn values_already_there_or_missing_change_nothing() {
    let home = Home::new();
    let b = Binding {
        repo: "events-pipeline",
        related: &strings(&["billing-api"]),
        aliases: &strings(&["EVENTS"]),
        ..Binding::default()
    };
    let outcome = home.bind(&b, true).unwrap();
    assert!(outcome.is_empty());
    assert!(outcome.changes.iter().all(|c| c.op == Op::Keep));

    let b = Binding {
        repo: "events-pipeline",
        skills: &strings(&["code-review"]),
        ..Binding::default()
    };
    let outcome = home.bind(&b, false).unwrap();
    assert!(outcome.is_empty());
    let note = outcome.changes[0].note.as_deref().unwrap();
    assert!(
        note.contains("inherited from scope `acme`") && note.contains("--exclude code-review"),
        "{note}"
    );
}

#[test]
fn editing_aliases_clears_the_draft_marker() {
    let home = Home::new();
    let b = Binding {
        repo: "feature-store",
        aliases: &strings(&["features"]),
        ..Binding::default()
    };
    let text = home.bind(&b, true).unwrap().writes.remove(0).after;
    assert!(
        text.contains("aliases = [\"feature\", \"store\", \"features\"]\n"),
        "{text}"
    );
    assert!(
        text.contains("models/\"  # kipi:draft\n"),
        "summary stays a draft"
    );
}

#[test]
fn a_bind_crossing_stores_or_scopes_is_rejected() {
    let home = Home::new();
    let before = home.bindings(ExampleStore::Personal);
    let b = Binding {
        repo: "side-tool",
        related: &strings(&["billing-api"]),
        ..Binding::default()
    };
    let err = home.bind(&b, true).unwrap_err().report();
    assert!(err.contains("(rejected: nothing was written)"), "{err}");
    assert!(err.contains("billing-api"), "{err}");
    assert_eq!(home.bindings(ExampleStore::Personal), before);

    // Two unrelated scopes of one store: the leak rule within a store.
    let file = home.store(ExampleStore::Acme).join("bindings.toml");
    let mut text = home.bindings(ExampleStore::Acme);
    text.push_str("\n[scopes.lab]\nroots = [\"~/lab\"]\n\n[repos.lab-notes]\nscope = \"lab\"\n");
    fs::write(&file, &text).unwrap();
    let b = Binding {
        repo: "lab-notes",
        related: &strings(&["billing-api"]),
        ..Binding::default()
    };
    let err = home.bind(&b, true).unwrap_err().report();
    assert!(err.contains("rejected"), "{err}");
    assert_eq!(home.bindings(ExampleStore::Acme), text);
}

#[test]
fn bad_values_are_rejected() {
    let home = Home::new();
    let cases: [(Binding, &str); 5] = [
        (
            Binding {
                repo: "blog",
                skills: &strings(&["no-such-skill"]),
                ..Binding::default()
            },
            "rejected",
        ),
        (
            Binding {
                repo: "ml-platform",
                aliases: &strings(&["Billing"]),
                ..Binding::default()
            },
            "alias `billing` of `ml-platform` is already a name or alias of `billing-api` in the \
             registry of scope `acme` (rejected: nothing was written)",
        ),
        (
            Binding {
                repo: "blog",
                skills: &strings(&["a/b/c"]),
                ..Binding::default()
            },
            "invalid --skill `a/b/c`",
        ),
        (
            Binding {
                repo: "billing",
                skills: &strings(&["x"]),
                ..Binding::default()
            },
            "`billing` is an alias of `billing-api`",
        ),
        (
            Binding {
                repo: "blog",
                ..Binding::default()
            },
            "nothing to bind",
        ),
    ];
    for (b, expected) in cases {
        let err = home.bind(&b, true).unwrap_err().report();
        assert!(err.contains(expected), "{expected:?} not in {err}");
    }
}

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
        .args(args)
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

fn skill_dir(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    fs::create_dir_all(path.join("references")).unwrap();
    fs::write(
        path.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: Use when testing imports.\n---\n\n# {name}\n"),
    )
    .unwrap();
    fs::write(path.join("references/deep.md"), "deep\n").unwrap();
    path
}

#[test]
fn add_skill_from_the_template_binds_it_to_its_scope() {
    let home = Home::new();
    let opts = SkillOptions {
        name: "incident-review",
        scope: Some("acme-ml"),
        ..SkillOptions::default()
    };
    let outcome = authoring::add_skill(&home.stores(), &home.machine(), &home.env(), &opts);
    let outcome = outcome.unwrap();
    assert_eq!(outcome.store.as_deref(), Some("acme"));
    outcome.write().unwrap();
    let skill = fs::read_to_string(
        home.store(ExampleStore::Acme)
            .join("skills/incident-review/SKILL.md"),
    );
    let skill = skill.unwrap();
    assert!(skill.starts_with("---\nname: incident-review\n"), "{skill}");
    assert!(skill.contains("  scope: acme-ml\n"), "{skill}");
    assert!(
        home.bindings(ExampleStore::Acme)
            .contains("skills = [\"dbt-conventions\", \"incident-review\"]\n")
    );
    // The edited store loads and resolves.
    let plan = kipi_core::resolve::resolve(&home.stores(), &home.machine()).unwrap();
    let repo = plan.repo("events-pipeline").unwrap();
    assert!(repo.skills.iter().any(|s| s.name == "incident-review"));
}

#[test]
fn add_skill_imports_a_directory_and_links_back() {
    let home = Home::new();
    let user_skills = home.path().join(".claude/skills");
    let src = skill_dir(&user_skills, "triage");
    let opts = SkillOptions {
        name: "triage",
        store: Some("personal"),
        scope: Some("acme"),
        from: Some(src.to_str().unwrap()),
        link_back: true,
    };
    let outcome = authoring::add_skill(&home.stores(), &home.machine(), &home.env(), &opts);
    let outcome = outcome.unwrap();
    assert_eq!(
        outcome.skill.as_ref().unwrap().files,
        [
            PathBuf::from("SKILL.md"),
            PathBuf::from("references/deep.md")
        ]
    );
    outcome.write().unwrap();
    let dest = home.store(ExampleStore::Personal).join("skills/triage");
    assert_eq!(
        fs::read_to_string(dest.join("references/deep.md")).unwrap(),
        "deep\n"
    );
    assert_eq!(fs::read_link(&src).unwrap(), dest, "linked back");
    assert!(
        fs::read_dir(&user_skills).unwrap().count() == 1,
        "no leftovers"
    );
    // A shared skill of another store is referenced qualified.
    assert!(
        home.bindings(ExampleStore::Acme)
            .contains("\"personal/triage\"]")
    );
    kipi_core::resolve::resolve(&home.stores(), &home.machine()).unwrap();
}

#[test]
fn add_skill_clones_a_git_url() {
    let home = Home::new();
    let repo = home.path().join("upstream");
    skill_dir(&repo.join("skills"), "lint-fix");
    git(&repo, &["init", "--quiet"]);
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "--quiet", "-m", "skill"]);
    let url = format!("file://{}", repo.display());
    let opts = SkillOptions {
        name: "lint-fix",
        store: Some("personal"),
        from: Some(&url),
        ..SkillOptions::default()
    };
    let outcome = authoring::add_skill(&home.stores(), &home.machine(), &home.env(), &opts);
    let outcome = outcome.unwrap();
    assert!(outcome.writes.is_empty(), "no scope, no bindings edit");
    outcome.write().unwrap();
    let dest = home.store(ExampleStore::Personal).join("skills/lint-fix");
    assert!(dest.join("SKILL.md").is_file());
    assert!(!dest.join(".git").exists());
}

#[test]
fn add_skill_refuses_clobbering_and_mismatched_names() {
    let home = Home::new();
    let run = |opts: &SkillOptions| {
        authoring::add_skill(&home.stores(), &home.machine(), &home.env(), opts)
            .unwrap_err()
            .report()
    };
    let err = run(&SkillOptions {
        name: "rust-style",
        store: Some("personal"),
        ..SkillOptions::default()
    });
    assert!(err.contains("already exists in store `personal`"), "{err}");
    let src = skill_dir(home.path(), "other-name");
    let err = run(&SkillOptions {
        name: "wanted",
        store: Some("personal"),
        from: Some(src.to_str().unwrap()),
        ..SkillOptions::default()
    });
    assert!(
        err.contains("the skill's `name` is `other-name`, not `wanted`"),
        "{err}"
    );
    let err = run(&SkillOptions {
        name: "x",
        ..SkillOptions::default()
    });
    assert!(err.contains("several stores are mounted"), "{err}");
    let err = run(&SkillOptions {
        name: "Bad_Name",
        store: Some("personal"),
        ..SkillOptions::default()
    });
    assert!(err.contains("invalid skill name"), "{err}");
}

#[test]
fn add_repo_goes_to_the_store_owning_its_scope() {
    let home = Home::new();
    let repo = FakeRepo {
        entry: "payments".into(),
        dir: "work/acme/payments".into(),
        origin_url: Some("git@github.com:acme-org/payments.git".into()),
        readme: None,
        manifest: ManifestKind::None,
        description: None,
    };
    let path = repo.create_under(home.path()).unwrap();
    let opts = RepoOptions {
        name: "payments",
        path: Some(&path),
        summary: Some("Card payments"),
        ..RepoOptions::default()
    };
    let outcome = authoring::add_repo(&home.stores(), &home.machine(), &home.env(), &opts);
    let outcome = outcome.unwrap();
    assert_eq!(outcome.store.as_deref(), Some("acme"));
    outcome.write().unwrap();
    assert!(home.bindings(ExampleStore::Acme).ends_with(
        "\n[repos.payments]\nremote = \"github.com/acme-org/payments\"\nsummary = \"Card payments\"\n"
    ));
    let machine = home.machine();
    assert_eq!(machine.repo_path("payments"), Some(path.as_path()));

    let err = authoring::add_repo(&home.stores(), &home.machine(), &home.env(), &opts);
    assert!(
        err.unwrap_err()
            .report()
            .contains("already exists in store `acme`")
    );
    let err = authoring::add_repo(
        &home.stores(),
        &home.machine(),
        &home.env(),
        &RepoOptions {
            name: "orphan",
            ..RepoOptions::default()
        },
    );
    assert!(err.unwrap_err().report().contains("pass --scope"));
    let outcome = authoring::add_repo(
        &home.stores(),
        &home.machine(),
        &home.env(),
        &RepoOptions {
            name: "notes-site",
            scope: Some("personal"),
            ..RepoOptions::default()
        },
    );
    assert!(
        outcome.unwrap().writes[0]
            .after
            .ends_with("[repos.notes-site]\nscope = \"personal\"\n")
    );
}

#[test]
fn add_repo_records_the_checkout_of_an_existing_entry() {
    let home = Home::new();
    let checkout = |dir: &str, url: &str| {
        FakeRepo {
            entry: String::new(),
            dir: dir.into(),
            origin_url: Some(url.into()),
            readme: None,
            manifest: ManifestKind::None,
            description: None,
        }
        .create_under(home.path())
        .unwrap()
    };
    let right = checkout(
        "work/acme/feature-store",
        "https://github.com/acme-org/feature-store.git",
    );
    let wrong = checkout("work/acme/other", "https://github.com/acme-org/other.git");
    let add = |path: &Path, scope: Option<&str>, summary: Option<&str>| {
        let opts = RepoOptions {
            name: "feature-store",
            path: Some(path),
            scope,
            summary,
        };
        authoring::add_repo(&home.stores(), &home.machine(), &home.env(), &opts)
    };
    let refused = |r: kipi_core::error::Result<Outcome>| r.unwrap_err().report();

    assert!(refused(add(&wrong, None, None)).contains("is not the remote"));
    assert!(refused(add(&right, None, Some("Other"))).contains("--summary differs"));
    assert!(
        refused(add(&right, Some("acme-ml"), None))
            .contains("resolves to scope `acme`, not scope `acme-ml`")
    );
    let summary = "Online and offline feature serving; dbt models in models/";
    let outcome = add(&right, Some("acme"), Some(summary)).unwrap();
    assert_eq!(
        outcome.writes.len(),
        1,
        "only machine.toml: the remote is there"
    );
    outcome.write().unwrap();
    assert_eq!(
        home.machine().repo_path("feature-store"),
        Some(right.as_path())
    );

    assert!(
        add(&right, None, None).unwrap().is_empty(),
        "already recorded"
    );
    assert!(refused(add(&wrong, None, None)).contains("is already cloned at"));
}

#[test]
fn remove_repo_deletes_the_entry_and_its_notes_only_when_asked() {
    let home = Home::new();
    let notes = home
        .store(ExampleStore::Personal)
        .join("repos/side-tool/notes.md");
    let remove = |notes: bool| {
        let opts = RemoveOptions {
            name: "side-tool",
            notes,
        };
        authoring::remove_repo(&home.stores(), &home.machine(), &home.env(), &opts)
    };
    let kept = remove(false).unwrap();
    assert!(kept.removes.is_empty());
    assert!(
        kept.changes
            .iter()
            .any(|c| c.what == "notes" && c.op == Op::Keep)
    );
    let outcome = remove(true).unwrap();
    assert_eq!(outcome.removes, std::slice::from_ref(&notes));
    outcome.write().unwrap();
    assert!(!notes.exists());
    assert!(
        !notes.parent().unwrap().exists(),
        "the emptied directory goes too"
    );
    assert!(
        !home
            .bindings(ExampleStore::Personal)
            .contains("[repos.side-tool]")
    );
    let err = authoring::remove_repo(
        &home.stores(),
        &home.machine(),
        &home.env(),
        &RemoveOptions {
            name: "photos",
            notes: false,
        },
    );
    assert!(err.unwrap_err().report().contains("no repo entry `photos`"));
}

#[test]
fn init_creates_mounts_and_never_clobbers() {
    let home = Home::new();
    let env = home.env();
    let init = |opts: &InitOptions| {
        let cfg = config::load_config(&env).unwrap();
        authoring::init(&env, &cfg, &Machine::default(), opts)
    };
    // The default store is named `personal` unless --name says otherwise.
    let outcome = init(&InitOptions::default()).unwrap();
    assert_eq!(outcome.store.as_deref(), Some("personal"));
    let outcome = init(&InitOptions {
        name: Some("mine"),
        ..InitOptions::default()
    })
    .unwrap();
    outcome.write().unwrap();
    let root = home.path().join(".agents/store");
    for dir in ["skills", "repos", "conventions", "generated"] {
        assert!(root.join(dir).is_dir(), "{dir}");
    }
    assert_eq!(
        fs::read_to_string(root.join(".gitignore")).unwrap(),
        "generated/\n"
    );
    let bindings = fs::read_to_string(root.join("bindings.toml")).unwrap();
    insta::assert_snapshot!("init-bindings", bindings);
    assert!(
        init(&InitOptions::default()).unwrap().is_empty(),
        "idempotent"
    );

    // Existing stores are mounted byte for byte, in order.
    for store in ExampleStore::ALL {
        let before = home.bindings(store);
        let path = home.store(store);
        let outcome = init(&InitOptions {
            path: Some(&path),
            ..InitOptions::default()
        });
        outcome.unwrap().write().unwrap();
        assert_eq!(home.bindings(store), before);
    }
    assert_eq!(
        fs::read_to_string(env.config_file()).unwrap(),
        "stores = [\"~/.agents/store\", \"~/stores/personal-store\", \"~/stores/acme-store\"]\n"
    );

    // A new store whose name clashes with a mounted one is rejected, and nothing is created.
    let other = home.path().join("other");
    let err = init(&InitOptions {
        path: Some(&other),
        name: Some("acme"),
        ..InitOptions::default()
    });
    let err = err.unwrap_err().report();
    assert!(
        err.contains("store name `acme` is used by two mounted stores"),
        "{err}"
    );
    assert!(err.contains("rejected"), "{err}");
    assert!(!other.exists());

    let acme = home.store(ExampleStore::Acme);
    let err = init(&InitOptions {
        path: Some(&acme),
        name: Some("work"),
        ..InitOptions::default()
    });
    assert!(
        err.unwrap_err()
            .report()
            .contains("is named `acme`, not `work`")
    );
}

#[test]
fn init_keeps_the_implicit_default_store_mounted() {
    let home = Home::new();
    let env = home.env();
    copy_example_store_to(ExampleStore::Personal, &home.path().join(".agents/store")).unwrap();
    let acme = home.store(ExampleStore::Acme);
    let cfg = config::load_config(&env).unwrap();
    let opts = InitOptions {
        path: Some(&acme),
        ..InitOptions::default()
    };
    let outcome = authoring::init(&env, &cfg, &Machine::default(), &opts).unwrap();
    assert_eq!(
        outcome.writes[0].after,
        "stores = [\"~/.agents/store\", \"~/stores/acme-store\"]\n"
    );
}

#[test]
fn unmount_removes_a_store_by_name_or_path() {
    let home = Home::new();
    let env = home.env();
    fs::create_dir_all(&env.config_dir).unwrap();
    fs::write(
        env.config_file(),
        "# mine\nstores = [\"~/stores/personal-store\", \"~/stores/acme-store\"]  # both\neditor = \"hx\"\n",
    )
    .unwrap();
    let cfg = config::load_config(&env).unwrap();
    let outcome = authoring::unmount(&env, &cfg, "acme").unwrap();
    assert_eq!(
        outcome.writes[0].after,
        "# mine\nstores = [\"~/stores/personal-store\"]  # both\neditor = \"hx\"\n"
    );
    let outcome = authoring::unmount(&env, &cfg, "~/stores/personal-store").unwrap();
    assert_eq!(
        outcome.writes[0].after,
        "# mine\nstores = [\"~/stores/acme-store\"]  # both\neditor = \"hx\"\n"
    );
    let err = authoring::unmount(&env, &cfg, "acm").unwrap_err().report();
    assert!(err.contains("did you mean `acme`?"), "{err}");
}
