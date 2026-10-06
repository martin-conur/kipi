//! Adapters (roadmap §3, §12, §13): one golden snapshot per adapter per example repo
//! (rendered block, action list, grant), the tracked-file policy, and the size rule.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use kipi_core::adapters;
use kipi_core::config::{self, Env};
use kipi_core::model::AgentId;
use kipi_core::plan::{Action, Checkout, Plan, RepoActions, repo_actions};
use kipi_core::render::block::{self, BEGIN, RenderedBlock};
use kipi_core::render::display_path;
use kipi_core::render::splice::splice_text;
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
        ..Checkout::default()
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

// ---------------------------------------------------------------------------------
// Claude Code reads AGENTS.md only while no CLAUDE.md / CLAUDE.local.md exists

/// A repo with the user's own `files` (name, text), synced once with `generic` on or off.
fn synced(files: &[(&str, &str)], generic: bool) -> (BTreeMap<String, String>, RepoActions) {
    let agents = if generic {
        AgentId::ALL.to_vec()
    } else {
        vec![AgentId::Claude]
    };
    let set = store_set(vec![
        StoreBuilder::new("s", "/st")
            .defaults(|d| d.agents = Some(agents))
            .repo("a", |_| {})
            .build(),
    ]);
    let machine = MachineBuilder::new().repo("a", "/c/a").build();
    let plan = resolve(&set, &machine).unwrap();
    let repo = plan.repo("a").unwrap();
    let dir = PathBuf::from("/c/a");
    let checkout = Checkout {
        user_files: files.iter().map(|(name, _)| dir.join(name)).collect(),
        ..Checkout::default()
    };
    let actions = repo_actions(repo, &block::render(&plan, repo, &[], None), &checkout).unwrap();
    let mut disk: BTreeMap<String, String> = files
        .iter()
        .map(|(n, t)| (n.to_string(), t.to_string()))
        .collect();
    for action in actions.actions() {
        if let Action::WriteBlock { file, text } = action {
            let name = file.strip_prefix(&dir).unwrap().display().to_string();
            let new = splice_text(file, disk.get(&name).map(String::as_str), text).unwrap();
            disk.insert(name, new);
        }
    }
    (disk, actions)
}

/// What Claude Code loads by default (`claude-md-or-agents-md`): `CLAUDE.md` then
/// `CLAUDE.local.md`, `@file` lines expanded, when either exists; else `AGENTS.md`.
fn claude_loads(disk: &BTreeMap<String, String>) -> String {
    let own: Vec<&String> = ["CLAUDE.md", "CLAUDE.local.md"]
        .iter()
        .filter_map(|n| disk.get(*n))
        .collect();
    if own.is_empty() {
        return disk.get("AGENTS.md").cloned().unwrap_or_default();
    }
    own.iter()
        .flat_map(|t| t.lines())
        .map(|l| match l.strip_prefix('@') {
            Some(f) => disk.get(f).cloned().unwrap_or_default(),
            None => format!("{l}\n"),
        })
        .collect()
}

const USER_CLAUDE: &str = "# my claude notes\n";
const USER_AGENTS: &str = "# my agents notes\n";

/// Claude keeps everything it loaded before the sync and gains exactly one block; with
/// `generic` on, AGENTS.md agents get the user's AGENTS.md text and one block too.
fn check(files: &[(&str, &str)], generic: bool) -> RepoActions {
    let before: BTreeMap<String, String> = files
        .iter()
        .map(|(n, t)| (n.to_string(), t.to_string()))
        .collect();
    let (disk, actions) = synced(files, generic);
    let loads = claude_loads(&disk);
    let case = format!("{files:?} generic={generic}:\n{loads}");
    assert_eq!(loads.matches(BEGIN).count(), 1, "{case}");
    for line in claude_loads(&before).lines() {
        assert!(loads.contains(line), "lost {line:?} in {case}");
    }
    if generic {
        let agents = disk.get("AGENTS.md").cloned().unwrap_or_default();
        assert_eq!(agents.matches(BEGIN).count(), 1, "{case}");
        if let Some((_, text)) = files.iter().find(|(n, _)| *n == "AGENTS.md") {
            assert!(agents.contains(text), "{case}");
        }
    }
    actions
}

#[test]
fn agents_md_only_with_generic_on_leaves_claude_reading_agents_md() {
    let actions = check(&[("AGENTS.md", USER_AGENTS)], true);
    let claude = &actions.agents[0];
    assert!(
        !claude
            .actions
            .iter()
            .any(|a| matches!(a, Action::WriteBlock { .. }))
    );
    assert_eq!(claude.notices.len(), 1);
    assert!(claude.notices[0].message.contains("CLAUDE.md not created"));
}

#[test]
fn agents_md_only_with_generic_off_imports_agents_md_from_a_new_claude_md() {
    check(&[("AGENTS.md", USER_AGENTS)], false);
    let (disk, _) = synced(&[("AGENTS.md", USER_AGENTS)], false);
    assert!(
        disk["CLAUDE.md"].contains("\n@AGENTS.md\n"),
        "{}",
        disk["CLAUDE.md"]
    );
    assert_eq!(disk["AGENTS.md"], USER_AGENTS);
}

#[test]
fn a_tracked_agents_md_without_a_block_is_imported_too() {
    let (set, dir) = (
        store_set(vec![
            StoreBuilder::new("s", "/st").repo("a", |_| {}).build(),
        ]),
        PathBuf::from("/c/a"),
    );
    let plan = resolve(&set, &MachineBuilder::new().repo("a", "/c/a").build()).unwrap();
    let repo = plan.repo("a").unwrap();
    let agents = dir.join("AGENTS.md");
    let checkout = Checkout {
        user_files: [agents.clone()].into(),
        tracked: [agents].into(),
        allow_tracked: false,
    };
    let actions = repo_actions(repo, &block::render(&plan, repo, &[], None), &checkout).unwrap();
    let Some(Action::WriteBlock { file, text }) = actions.agents[0].actions.first() else {
        panic!("{actions:?}")
    };
    assert_eq!(file, &dir.join("CLAUDE.md"));
    assert!(text.contains("\n@AGENTS.md\n"), "{text}");
}

#[test]
fn claude_md_only_gets_the_block_in_place() {
    for generic in [true, false] {
        let (disk, _) = synced(&[("CLAUDE.md", USER_CLAUDE)], generic);
        assert!(!disk["CLAUDE.md"].contains("@AGENTS.md"));
        check(&[("CLAUDE.md", USER_CLAUDE)], generic);
    }
}

#[test]
fn both_files_get_their_own_block() {
    for generic in [true, false] {
        check(
            &[("CLAUDE.md", USER_CLAUDE), ("AGENTS.md", USER_AGENTS)],
            generic,
        );
    }
}

#[test]
fn neither_file_creates_claude_md_without_an_import() {
    for generic in [true, false] {
        check(&[], generic);
        let (disk, _) = synced(&[], generic);
        assert!(!disk["CLAUDE.md"].contains("@AGENTS.md"));
        assert_eq!(disk.contains_key("AGENTS.md"), generic);
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
