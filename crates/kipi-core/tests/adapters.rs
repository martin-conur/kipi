//! Adapters (roadmap §3, §12, §13): one golden snapshot per adapter per example repo
//! (rendered block, action list, grant), the tracked-file policy, and the size rule.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use kipi_core::adapters;
use kipi_core::config::{self, Env};
use kipi_core::model::AgentId;
use kipi_core::plan::{Action, Checkout, Plan, RepoActions, repo_actions};
use kipi_core::render::block::{self, RenderedBlock};
use kipi_core::render::display_path;
use kipi_core::resolve::resolve;
use kipi_test_support::builders::{MachineBuilder, StoreBuilder, store_set};
use kipi_test_support::stores::ExampleStore;
use kipi_test_support::workspace_root;

/// `examples/` doubles as the home directory, as in `tests/render.rs`.
fn home() -> PathBuf {
    workspace_root().join("examples")
}

fn examples() -> Plan {
    let mut env = Env::from_lookup(|_| None, Some(home())).unwrap();
    env.config_dir = home();
    let paths: Vec<PathBuf> = ExampleStore::ALL.iter().map(|s| s.source_path()).collect();
    let set = config::load_stores(&paths, &env).unwrap_or_else(|e| panic!("{}", e.report()));
    let machine = config::load_machine(&env).unwrap_or_else(|e| panic!("{}", e.report()));
    resolve(&set, &machine).unwrap_or_else(|e| panic!("{}", e.report()))
}

fn rendered(plan: &Plan, repo: &str) -> RenderedBlock {
    let repo = plan.repo(repo).unwrap();
    let conventions = block::load_conventions(repo).unwrap_or_else(|e| panic!("{}", e.report()));
    block::render(plan, repo, &conventions, Some(&home()))
}

/// One adapter's part of `actions`, as text: block target and text, links, grant,
/// exclude patterns and notices.
fn describe(actions: &RepoActions, id: AgentId) -> String {
    let home = home();
    let p = |path: &Path| display_path(path, Some(&home));
    let rel = |path: &Path| {
        path.strip_prefix(&actions.dir)
            .unwrap()
            .display()
            .to_string()
    };
    let Some(agent) = actions.agents.iter().find(|a| a.agent == id) else {
        return format!("{id} is not enabled for {}\n", actions.repo);
    };
    let mut out = format!("repo: {} at {}\n", actions.repo, p(&actions.dir));
    let mut block = String::new();
    for action in &agent.actions {
        match action {
            Action::WriteBlock { file, text } => {
                writeln!(out, "write block → {}", rel(file)).unwrap();
                block = text.clone();
            }
            Action::Link { link, target } => {
                writeln!(out, "link {} → {}", rel(link), p(target)).unwrap();
            }
            Action::MergeGrant { file, key, entries } => {
                writeln!(out, "grant → {} {}", rel(file), key.join(".")).unwrap();
                for e in entries {
                    writeln!(out, "  + {}", p(e)).unwrap();
                }
                if entries.is_empty() {
                    writeln!(out, "  (no entries)").unwrap();
                }
            }
            other => panic!("adapters never emit {other:?}"),
        }
    }
    if !agent
        .actions
        .iter()
        .any(|a| matches!(a, Action::MergeGrant { .. }))
    {
        writeln!(out, "grant: none (no grant mechanism)").unwrap();
    }
    if let Action::Exclude { patterns, .. } = &actions.exclude {
        writeln!(out, "exclude: {}", patterns.join(" ")).unwrap();
    }
    for n in &agent.notices {
        writeln!(out, "notice: {} (hint: {})", n.message, n.hint).unwrap();
    }
    writeln!(out, "\n{block}").unwrap();
    out
}

#[test]
fn snapshots_for_every_adapter_and_example_repo() {
    let plan = examples();
    assert_eq!(plan.repos.len(), 10);
    for repo in &plan.repos {
        let block = rendered(&plan, &repo.name);
        let actions = repo_actions(repo, &block, &Checkout::default());
        for id in AgentId::ALL {
            let text = match &actions {
                Some(a) => describe(a, id),
                None => format!("{} is not cloned on this machine: no actions\n", repo.name),
            };
            insta::assert_snapshot!(format!("{id}-{}", repo.name), text);
        }
    }
}

#[test]
fn every_action_path_stays_inside_the_repo_and_links_point_into_a_store() {
    let plan = examples();
    let stores = [
        ExampleStore::Acme.source_path(),
        ExampleStore::Personal.source_path(),
    ];
    for repo in &plan.repos {
        let Some(actions) = repo_actions(repo, &rendered(&plan, &repo.name), &Checkout::default())
        else {
            continue;
        };
        for action in actions.actions() {
            match action {
                Action::Link { link, target } => {
                    assert!(link.starts_with(&actions.dir), "{link:?}");
                    assert!(target.is_absolute(), "{target:?}");
                    assert!(stores.iter().any(|s| target.starts_with(s)), "{target:?}");
                }
                Action::WriteBlock { file, .. } | Action::MergeGrant { file, .. } => {
                    assert!(file.starts_with(&actions.dir), "{file:?}")
                }
                Action::Exclude { repo, .. } => assert_eq!(repo, &actions.dir),
                Action::Remove { .. } => panic!("adapters never emit Remove"),
            }
        }
    }
}

fn tiny() -> (Plan, PathBuf) {
    let set = store_set(vec![
        StoreBuilder::new("s", "/st").repo("a", |_| {}).build(),
    ]);
    let machine = MachineBuilder::new().repo("a", "/c/a").build();
    (resolve(&set, &machine).unwrap(), PathBuf::from("/c/a"))
}

fn tracked(files: &[&str], allow_tracked: bool) -> RepoActions {
    let (plan, dir) = tiny();
    let repo = plan.repo("a").unwrap();
    let checkout = Checkout {
        tracked: files.iter().map(|f| dir.join(f)).collect(),
        allow_tracked,
    };
    repo_actions(repo, &block::render(&plan, repo, &[], None), &checkout).unwrap()
}

fn block_files(actions: &RepoActions) -> Vec<String> {
    actions
        .actions()
        .filter_map(|a| match a {
            Action::WriteBlock { file, .. } => Some(file.display().to_string()),
            _ => None,
        })
        .collect()
}

fn excluded(actions: &RepoActions) -> Vec<String> {
    let Action::Exclude { patterns, .. } = &actions.exclude else {
        unreachable!()
    };
    patterns.clone()
}

#[test]
fn untracked_files_get_the_block_in_place() {
    let a = tracked(&[], false);
    assert_eq!(block_files(&a), ["/c/a/CLAUDE.md", "/c/a/AGENTS.md"]);
    assert!(a.agents.iter().all(|x| x.notices.is_empty()));
    assert_eq!(
        excluded(&a),
        [
            ".claude/skills/",
            ".claude/settings.local.json",
            ".agents/",
            ".kipi/"
        ]
    );
}

#[test]
fn a_tracked_claude_md_moves_the_block_to_claude_local_md() {
    let a = tracked(&["CLAUDE.md"], false);
    assert_eq!(block_files(&a), ["/c/a/CLAUDE.local.md", "/c/a/AGENTS.md"]);
    assert!(excluded(&a).contains(&"CLAUDE.local.md".to_owned()));
}

#[test]
fn a_tracked_agents_md_gets_no_block_and_a_notice() {
    let a = tracked(&["AGENTS.md", "CLAUDE.md", "CLAUDE.local.md"], false);
    assert!(block_files(&a).is_empty());
    for agent in &a.agents {
        assert_eq!(agent.notices.len(), 1, "{agent:?}");
        assert!(agent.notices[0].message.contains("tracked"));
        assert!(agent.notices[0].hint.contains("--allow-tracked"));
    }
}

#[test]
fn allow_tracked_writes_into_tracked_files() {
    let a = tracked(&["AGENTS.md", "CLAUDE.md"], true);
    assert_eq!(block_files(&a), ["/c/a/CLAUDE.md", "/c/a/AGENTS.md"]);
}

#[test]
fn a_repo_not_cloned_here_has_no_actions() {
    let (plan, _) = tiny();
    let mut repo = plan.repo("a").unwrap().clone();
    repo.path = None;
    assert!(
        repo_actions(
            &repo,
            &block::render(&plan, &repo, &[], None),
            &Checkout::default()
        )
        .is_none()
    );
}

#[test]
fn adapters_render_the_same_block_text() {
    let (plan, _) = tiny();
    let block = block::render(&plan, plan.repo("a").unwrap(), &[], None);
    for id in AgentId::ALL {
        assert_eq!(adapters::adapter(id).render_block(&block), block.text());
        assert_eq!(adapters::adapter(id).id(), id);
    }
}

/// Roadmap §3 / repo `CLAUDE.md`: every adapter stays under 100 lines.
#[test]
fn every_adapter_is_under_100_lines() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/adapters");
    let mut checked = 0;
    for entry in fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.file_name().is_some_and(|n| n == "mod.rs") {
            continue;
        }
        let lines = fs::read_to_string(&path).unwrap().lines().count();
        assert!(lines < 100, "{} has {lines} lines", path.display());
        checked += 1;
    }
    assert_eq!(checked, AgentId::ALL.len());
}
