//! The example stores must match the spec (roadmap §4, §5) and stay internally
//! consistent until real config loading lands in Stage 1.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use kipi_test_support::repos::EXAMPLE_LAYOUT;
use kipi_test_support::stores::ExampleStore;
use kipi_test_support::workspace_root;
use toml_edit::{DocumentMut, Item, Table};

fn bindings(store: ExampleStore) -> DocumentMut {
    let path = store.source_path().join("bindings.toml");
    fs::read_to_string(&path)
        .unwrap()
        .parse()
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn table<'a>(doc: &'a DocumentMut, key: &str) -> &'a Table {
    doc[key]
        .as_table()
        .unwrap_or_else(|| panic!("[{key}] is a table"))
}

static NONE: Item = Item::None;

/// `item[key]`, or `Item::None` when absent (immutable indexing panics on a missing key).
fn opt<'a>(item: &'a Item, key: &str) -> &'a Item {
    item.get(key).unwrap_or(&NONE)
}

fn strings(item: &Item) -> Vec<String> {
    item.as_array()
        .map(|a| a.iter().map(|v| v.as_str().unwrap().to_owned()).collect())
        .unwrap_or_default()
}

/// The repo names of a `related` array: strings, or the `repo` of `{ repo, via }` tables.
fn related(item: &Item) -> Vec<String> {
    item.as_array()
        .map(|a| {
            a.iter()
                .map(|v| match v.as_inline_table() {
                    Some(t) => t["repo"].as_str().unwrap().to_owned(),
                    None => v.as_str().unwrap().to_owned(),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn is_draft(item: &Item) -> bool {
    item.as_value()
        .and_then(|v| v.decor().suffix())
        .and_then(|s| s.as_str())
        .is_some_and(|s| s.contains("# kipi:draft"))
}

/// Frontmatter of a SKILL.md as (name, description, x-kipi scope).
fn skill_frontmatter(path: &Path) -> (String, String, String) {
    let text = fs::read_to_string(path).unwrap();
    let rest = text
        .strip_prefix("---\n")
        .expect("SKILL.md starts with frontmatter");
    let (front, body) = rest.split_once("\n---\n").expect("frontmatter is closed");
    assert!(!body.trim().is_empty(), "{} has a body", path.display());
    let field = |prefix: &str| {
        front
            .lines()
            .find_map(|l| l.strip_prefix(prefix))
            .unwrap_or_else(|| panic!("{} lacks `{prefix}`", path.display()))
            .trim()
            .to_owned()
    };
    assert!(
        front.lines().any(|l| l == "x-kipi:"),
        "{} has x-kipi",
        path.display()
    );
    (field("name: "), field("description: "), field("  scope: "))
}

#[test]
fn acme_store_mirrors_spec_5_1() {
    let doc = bindings(ExampleStore::Acme);
    let store = table(&doc, "store");
    assert_eq!(store["name"].as_str(), Some("acme"));
    assert_eq!(store["version"].as_integer(), Some(1));
    assert_eq!(store["share_skills"].as_bool(), Some(false));

    let defaults = table(&doc, "defaults");
    assert_eq!(defaults["budget_tokens"].as_integer(), Some(4000));
    assert_eq!(strings(&defaults["agents"]), ["claude", "generic"]);
    assert_eq!(defaults["grant"].as_str(), Some("related"));

    let scopes = table(&doc, "scopes");
    assert_eq!(
        scopes.iter().map(|(k, _)| k).collect::<Vec<_>>(),
        ["acme", "acme-ml"]
    );
    let acme = &scopes["acme"];
    assert_eq!(strings(&acme["roots"]), ["~/work/acme"]);
    assert_eq!(
        strings(opt(acme, "skills")),
        ["code-review", "pr-conventions", "personal/commit-style"]
    );
    assert_eq!(strings(opt(acme, "always")), ["conventions/acme.md"]);
    assert_eq!(acme["registry"].as_bool(), Some(true));
    assert_eq!(strings(&acme["remote_patterns"]), ["github.com/acme-org/"]);
    let ml = &scopes["acme-ml"];
    assert_eq!(ml["extends"].as_str(), Some("acme"));
    assert_eq!(strings(&ml["roots"]), ["~/work/acme/ml"]);
    assert_eq!(strings(opt(ml, "skills")), ["dbt-conventions"]);

    let repos = table(&doc, "repos");
    let names: BTreeSet<&str> = repos.iter().map(|(k, _)| k).collect();
    assert_eq!(names.len(), 6);
    for name in ["billing-api", "events-pipeline", "ml-platform", "acme-api"] {
        assert!(names.contains(name), "acme has {name}");
    }
    let billing = &repos["billing-api"];
    assert_eq!(
        billing["remote"].as_str(),
        Some("github.com/acme-org/billing-api")
    );
    assert!(
        is_draft(opt(billing, "summary")),
        "billing-api summary is a draft (§5.1)"
    );
    assert_eq!(strings(opt(billing, "aliases")), ["billing", "invoices"]);
    assert_eq!(
        repos["acme-api"]["remote"].as_str(),
        Some("github.com/acme-org/api")
    );
    let events = &repos["events-pipeline"];
    assert_eq!(events["scope"].as_str(), Some("acme-ml"));
    assert_eq!(
        related(opt(events, "related")),
        ["billing-api", "ml-platform"]
    );
    assert_eq!(strings(opt(events, "skills")), ["stream-recipes"]);
    assert_eq!(strings(&events["exclude"]), ["pr-conventions"]);
    assert_eq!(strings(&events["agents"]), ["claude"]);
    assert_eq!(events["grant"].as_str(), Some("registry"));
}

#[test]
fn personal_store_shape() {
    let doc = bindings(ExampleStore::Personal);
    let store = table(&doc, "store");
    assert_eq!(store["name"].as_str(), Some("personal"));
    assert_eq!(store["share_skills"].as_bool(), Some(true));
    let scopes = table(&doc, "scopes");
    assert_eq!(
        scopes.iter().map(|(k, _)| k).collect::<Vec<_>>(),
        ["personal"]
    );
    assert_eq!(strings(&scopes["personal"]["roots"]), ["~/code"]);
    let repos = table(&doc, "repos");
    assert_eq!(repos.len(), 4);
    assert!(repos.contains_key("side-tool") && repos.contains_key("api"));
    let skills: Vec<_> = fs::read_dir(ExampleStore::Personal.source_path().join("skills"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    assert_eq!(skills, ["commit-style", "rust-style"]);
}

#[test]
fn acme_store_has_four_skills() {
    let dir = ExampleStore::Acme.source_path().join("skills");
    let skills: BTreeSet<_> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(
        skills.into_iter().collect::<Vec<_>>(),
        [
            "code-review",
            "dbt-conventions",
            "pr-conventions",
            "stream-recipes"
        ]
    );
}

#[test]
fn cross_store_requirements_hold() {
    let docs: Vec<_> = ExampleStore::ALL
        .iter()
        .map(|&s| (s, bindings(s)))
        .collect();
    let mut entries = BTreeSet::new();
    let (mut drafts, mut missing_summary, mut relations) = (0, 0, 0);
    let mut remotes_by_dir: Vec<(String, String)> = Vec::new();
    for (store, doc) in &docs {
        let repos = table(doc, "repos");
        for (name, entry) in repos.iter() {
            assert!(
                entries.insert(name.to_owned()),
                "entry {name} is unique across stores"
            );
            let remote = entry["remote"]
                .as_str()
                .unwrap_or_else(|| panic!("{}:{name} has a remote", store.name()));
            let dir = remote.rsplit('/').next().unwrap().to_owned();
            remotes_by_dir.push((dir, remote.to_owned()));
            drafts +=
                usize::from(is_draft(opt(entry, "summary")) || is_draft(opt(entry, "aliases")));
            missing_summary += usize::from(entry.get("summary").is_none());
            for rel in related(opt(entry, "related")) {
                relations += 1;
                assert!(
                    repos.contains_key(&rel),
                    "{name} relates to {rel} in its own store"
                );
            }
        }
    }
    assert!(drafts >= 1, "at least one `# kipi:draft` value");
    assert!(missing_summary >= 1, "at least one repo without a summary");
    assert!(relations >= 1, "at least one `related`");

    // The directory-name clash: two checkouts named `api`, told apart by remote.
    let api: BTreeSet<_> = remotes_by_dir
        .iter()
        .filter(|(d, _)| d == "api")
        .map(|(_, r)| r.as_str())
        .collect();
    assert_eq!(
        api.into_iter().collect::<Vec<_>>(),
        ["github.com/acme-org/api", "github.com/example-user/api"]
    );
    assert!(entries.contains("acme-api") && entries.contains("api"));
}

#[test]
fn every_referenced_file_exists() {
    for store in ExampleStore::ALL {
        let root = store.source_path();
        let doc = bindings(store);
        for (scope, def) in table(&doc, "scopes").iter() {
            for always in strings(opt(def, "always")) {
                assert!(root.join(&always).is_file(), "{scope}: {always} exists");
            }
            let mut skills = strings(opt(def, "skills"));
            for (_, entry) in table(&doc, "repos").iter() {
                skills.extend(strings(opt(entry, "skills")));
            }
            for skill in skills {
                let path = match skill.split_once('/') {
                    Some((other, name)) => {
                        let owner = ExampleStore::ALL.into_iter().find(|s| s.name() == other);
                        let owner = owner.unwrap_or_else(|| panic!("store {other} exists"));
                        assert_eq!(
                            table(&bindings(owner), "store")["share_skills"].as_bool(),
                            Some(true),
                            "{other} shares skills"
                        );
                        owner.source_path().join("skills").join(name)
                    }
                    None => root.join("skills").join(&skill),
                };
                assert!(
                    path.join("SKILL.md").is_file(),
                    "{skill} resolves to a skill dir"
                );
            }
        }
        for dir in fs::read_dir(root.join("repos")).unwrap() {
            let name = dir.unwrap().file_name().into_string().unwrap();
            assert!(
                table(&doc, "repos").contains_key(&name),
                "notes for known repo {name}"
            );
        }
    }
}

#[test]
fn skills_follow_5_4() {
    for store in ExampleStore::ALL {
        let scopes: BTreeSet<String> = table(&bindings(store), "scopes")
            .iter()
            .map(|(k, _)| k.to_owned())
            .collect();
        for dir in fs::read_dir(store.source_path().join("skills")).unwrap() {
            let dir = dir.unwrap().path();
            let (name, description, scope) = skill_frontmatter(&dir.join("SKILL.md"));
            assert_eq!(name, dir.file_name().unwrap().to_str().unwrap());
            assert!(!description.is_empty() && description.len() <= 1024);
            assert!(
                scopes.contains(&scope),
                "{name}: x-kipi.scope {scope} is a scope of its store"
            );
        }
    }
}

#[test]
fn layout_matches_example_entries() {
    let mut expected = BTreeSet::new();
    for store in ExampleStore::ALL {
        for (name, _) in table(&bindings(store), "repos").iter() {
            expected.insert((store.name(), name.to_owned()));
        }
    }
    let layout: BTreeSet<_> = EXAMPLE_LAYOUT
        .iter()
        .map(|(s, e, _, _)| (s.name(), (*e).to_owned()))
        .collect();
    assert_eq!(layout, expected);
}

#[test]
fn machine_and_config_examples_parse() {
    let examples = workspace_root().join("examples");
    let machine: DocumentMut = fs::read_to_string(examples.join("machine.toml"))
        .unwrap()
        .parse()
        .unwrap();
    let entries: BTreeSet<String> = ExampleStore::ALL
        .iter()
        .flat_map(|&s| {
            table(&bindings(s), "repos")
                .iter()
                .map(|(k, _)| k.to_owned())
                .collect::<Vec<_>>()
        })
        .collect();
    for (name, path) in table(&machine, "repos").iter() {
        assert!(
            entries.contains(name),
            "machine.toml repo {name} is an entry"
        );
        assert!(
            path.as_str().unwrap().starts_with('/'),
            "machine paths are absolute"
        );
    }
    assert!(
        !table(&machine, "repos").contains_key("feature-store"),
        "one repo not cloned here"
    );

    let config: DocumentMut = fs::read_to_string(examples.join("config.toml"))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        strings(&config["stores"]),
        ["~/.agents/store", "~/work/.kipi-acme"]
    );
}
