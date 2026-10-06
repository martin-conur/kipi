//! Property tests for resolution (roadmap §13): deterministic under input reordering,
//! `exclude` always wins, nothing crosses scopes or stores except shared skills, and
//! unmounting the work store leaves a plan with no work content.
//!
//! Each case generates two stores: `work` (does not share) and `home` (shares its
//! skills). Generated configs are often invalid (a leaking `related`, a skill bound
//! outside its `x-kipi.scope`); the properties then only require that the error does
//! not depend on input order.

use std::path::{Path, PathBuf};

use kipi_core::model::{GrantMode, Machine, SkillRef, Store, StoreSet};
use kipi_core::plan::{GrantKind, Plan, RepoPlan};
use kipi_core::resolve::resolve;
use kipi_test_support::builders::{MachineBuilder, StoreBuilder};
use proptest::prelude::*;

const SKILLS: usize = 3;

#[derive(Debug, Clone)]
struct World {
    work: StoreSpec,
    home: StoreSpec,
}

#[derive(Debug, Clone)]
struct StoreSpec {
    scopes: Vec<ScopeSpec>,
    /// `x-kipi.scope` of each skill, as a scope index.
    skill_scopes: Vec<Option<usize>>,
    repos: Vec<RepoSpec>,
}

#[derive(Debug, Clone)]
struct ScopeSpec {
    parent: Option<usize>,
    registry: bool,
    skills: Vec<Pick>,
    grant: Option<GrantMode>,
}

#[derive(Debug, Clone)]
struct RepoSpec {
    place: Place,
    skills: Vec<Pick>,
    exclude: Vec<Pick>,
    related: Vec<usize>,
    grant: Option<GrantMode>,
}

/// A skill reference: one of the store's own skills, or (from `work` only) a shared
/// skill of `home`, written qualified.
#[derive(Debug, Clone, Copy)]
enum Pick {
    Own(usize),
    Shared(usize),
}

#[derive(Debug, Clone, Copy)]
enum Place {
    Explicit(usize),
    Under(usize),
    Outside,
    NotCloned,
}

fn pick() -> impl Strategy<Value = Pick> {
    prop_oneof![
        3 => (0..SKILLS).prop_map(Pick::Own),
        1 => (0..SKILLS).prop_map(Pick::Shared),
    ]
}

fn grant() -> impl Strategy<Value = Option<GrantMode>> {
    prop_oneof![
        Just(None),
        Just(Some(GrantMode::Related)),
        Just(Some(GrantMode::Registry)),
        Just(Some(GrantMode::None)),
    ]
}

fn store_spec() -> impl Strategy<Value = StoreSpec> {
    (1usize..4, 1usize..6).prop_flat_map(|(n_scopes, n_repos)| {
        let scope = (
            proptest::option::of(0..n_scopes),
            any::<bool>(),
            proptest::collection::vec(pick(), 0..3),
            grant(),
        )
            .prop_map(|(parent, registry, skills, grant)| ScopeSpec {
                parent,
                registry,
                skills,
                grant,
            });
        let place = prop_oneof![
            3 => (0..n_scopes).prop_map(Place::Explicit),
            3 => (0..n_scopes).prop_map(Place::Under),
            1 => Just(Place::Outside),
            1 => Just(Place::NotCloned),
        ];
        let repo = (
            place,
            proptest::collection::vec(pick(), 0..3),
            proptest::collection::vec(pick(), 0..2),
            proptest::collection::vec(0..n_repos, 0..2),
            grant(),
        )
            .prop_map(|(place, skills, exclude, related, grant)| RepoSpec {
                place,
                skills,
                exclude,
                related,
                grant,
            });
        (
            proptest::collection::vec(scope, n_scopes),
            proptest::collection::vec(
                prop_oneof![8 => Just(None), 1 => (0..n_scopes).prop_map(Some)],
                SKILLS,
            ),
            proptest::collection::vec(repo, n_repos),
        )
            .prop_map(|(mut scopes, skill_scopes, repos)| {
                // Parents only point backwards, so chains are acyclic (cycles are
                // covered by the unit tests).
                for (i, s) in scopes.iter_mut().enumerate() {
                    s.parent = s.parent.filter(|&p| p < i);
                }
                StoreSpec {
                    scopes,
                    skill_scopes,
                    repos,
                }
            })
    })
}

fn world() -> impl Strategy<Value = World> {
    (store_spec(), store_spec()).prop_map(|(work, home)| World { work, home })
}

const STORES: [(&str, bool); 2] = [("work", false), ("home", true)];

fn root(store: &str) -> PathBuf {
    PathBuf::from(format!("/stores/{store}"))
}

fn scope_name(store: &str, i: usize) -> String {
    format!("{store}-s{i}")
}

fn repo_name(store: &str, i: usize) -> String {
    format!("{store}-r{i}")
}

/// A scope's root: nested under its parent's, so deeper scopes have deeper roots.
fn scope_root(store: &str, spec: &StoreSpec, i: usize) -> PathBuf {
    match spec.scopes[i].parent {
        Some(p) => scope_root(store, spec, p).join(format!("s{i}")),
        None => PathBuf::from(format!("/m/{store}/s{i}")),
    }
}

fn skill_ref(store: &str, pick: Pick) -> SkillRef {
    match pick {
        Pick::Shared(i) if store == "work" => format!("home/k{i}").parse().unwrap(),
        Pick::Own(i) | Pick::Shared(i) => format!("k{i}").parse().unwrap(),
    }
}

fn build_store(store: &str, share: bool, spec: &StoreSpec, machine: &mut MachineBuilder) -> Store {
    let mut b = StoreBuilder::new(store, root(store)).share_skills(share);
    for (i, s) in spec.scopes.iter().enumerate() {
        b = b.scope(&scope_name(store, i), |scope| {
            scope.extends = s.parent.map(|p| scope_name(store, p));
            scope.roots = vec![scope_root(store, spec, i)];
            scope.registry = s.registry;
            scope.skills = s.skills.iter().map(|&p| skill_ref(store, p)).collect();
            scope.grant = s.grant;
        });
    }
    for (i, owner) in spec.skill_scopes.iter().enumerate() {
        b = b.skill(
            &format!("k{i}"),
            owner.map(|o| scope_name(store, o)).as_deref(),
        );
    }
    for (i, r) in spec.repos.iter().enumerate() {
        let name = repo_name(store, i);
        let path = match r.place {
            Place::Explicit(s) | Place::Under(s) => Some(scope_root(store, spec, s).join(&name)),
            Place::Outside => Some(PathBuf::from(format!("/elsewhere/{name}"))),
            Place::NotCloned => None,
        };
        if let Some(path) = path {
            *machine = std::mem::take(machine).repo(&name, path);
        }
        b = b.repo(&name, |repo| {
            if let Place::Explicit(s) = r.place {
                repo.scope = Some(scope_name(store, s));
            }
            repo.skills = r.skills.iter().map(|&p| skill_ref(store, p)).collect();
            repo.exclude = r.exclude.iter().map(|&p| skill_ref(store, p)).collect();
            repo.related = (r.related.iter())
                .filter(|&&j| j != i)
                .map(|&j| repo_name(store, j))
                .collect();
            repo.grant = r.grant;
        });
    }
    b.build()
}

/// The stores (in `STORES` order, filtered by `mounted`) and the machine.
fn build(world: &World, mounted: &[&str]) -> (Vec<Store>, Machine) {
    let mut machine = MachineBuilder::new();
    let mut stores = Vec::new();
    for (name, share) in STORES {
        let spec = if name == "work" {
            &world.work
        } else {
            &world.home
        };
        let store = build_store(name, share, spec, &mut machine);
        if mounted.contains(&name) {
            stores.push(store);
        }
    }
    (stores, machine.build())
}

/// Reorder every list whose order carries no meaning: stores, scopes, repos, skills and
/// machine entries. Each item keeps its own location, as it would in a reordered file.
fn shuffled(mut stores: Vec<Store>, mut machine: Machine, seed: u64) -> (Vec<Store>, Machine) {
    let mut rng = seed;
    fn shuffle<T>(items: &mut [T], rng: &mut u64) {
        for i in (1..items.len()).rev() {
            *rng = rng
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            items.swap(i, (*rng >> 33) as usize % (i + 1));
        }
    }
    shuffle(&mut stores, &mut rng);
    for store in &mut stores {
        shuffle(&mut store.scopes, &mut rng);
        shuffle(&mut store.repos, &mut rng);
        shuffle(&mut store.skills, &mut rng);
    }
    shuffle(&mut machine.repos, &mut rng);
    (stores, machine)
}

fn plan_of(stores: Vec<Store>, machine: &Machine) -> Option<Plan> {
    let set = StoreSet::new(stores).expect("generated stores only cross via shared skills");
    resolve(&set, machine).ok()
}

fn same_chain(plan: &Plan, a: &RepoPlan, b: &str) -> bool {
    let b = plan.repo(b).expect("granted and related repos have plans");
    match (a.scope.value.as_deref(), b.scope.value.as_deref()) {
        (None, None) => true,
        (Some(sa), Some(sb)) => a.chain.iter().any(|s| s == sb) || b.chain.iter().any(|s| s == sa),
        _ => false,
    }
}

/// No skill, registry line, related repo or grant of any repo crosses its scope chain or
/// its store, except a shared skill (and that store's `skills/` dir).
fn assert_no_crossing(plan: &Plan, world: &World) {
    for repo in &plan.repos {
        let own = root(&repo.store);
        for skill in &repo.skills {
            if skill.store == repo.store {
                assert_eq!(skill.reference.store, None);
                let spec = if repo.store == "work" {
                    &world.work
                } else {
                    &world.home
                };
                let idx: usize = skill.name[1..].parse().unwrap();
                if let Some(owner) = spec.skill_scopes[idx] {
                    let owner = scope_name(&repo.store, owner);
                    assert!(
                        repo.chain.contains(&owner),
                        "{} uses {} outside {owner}",
                        repo.name,
                        skill.name
                    );
                }
            } else {
                assert_eq!(
                    (repo.store.as_str(), skill.store.as_str()),
                    ("work", "home")
                );
                assert_eq!(skill.reference.store.as_deref(), Some("home"));
            }
        }
        if let Some(registry) = plan.registry_of(repo) {
            assert_eq!(registry.store, repo.store);
            assert!(repo.chain.contains(&registry.scope));
            for entry in &registry.entries {
                assert!(registry.scopes.contains(&entry.scope));
                assert_eq!(plan.repo(&entry.name).unwrap().store, repo.store);
            }
        }
        for related in &repo.related {
            assert_eq!(plan.repo(&related.name).unwrap().store, repo.store);
            assert!(same_chain(plan, repo, &related.name));
        }
        if repo.grant.mode.value == GrantMode::None {
            assert!(repo.grant.dirs.is_empty());
        }
        for dir in &repo.grant.dirs {
            match &dir.kind {
                GrantKind::StoreRepos => assert_eq!(dir.path, own.join("repos")),
                GrantKind::StoreSkills { store } if *store == repo.store => {
                    assert_eq!(dir.path, own.join("skills"));
                }
                GrantKind::StoreSkills { store } => {
                    assert_eq!(dir.path, root(store).join("skills"));
                    assert!(repo.skills.iter().any(|s| s.store == *store));
                }
                GrantKind::Repo { name } => {
                    let target = plan.repo(name).unwrap();
                    assert_eq!(target.store, repo.store);
                    assert_ne!(target.name, repo.name);
                    assert_eq!(target.path.as_deref(), Some(dir.path.as_path()));
                    assert!(same_chain(plan, repo, name), "{} granted {name}", repo.name);
                }
            }
        }
    }
}

fn mentions_work(path: &Path) -> bool {
    path.starts_with("/stores/work")
        || path.starts_with("/m/work")
        || path.to_string_lossy().contains("work-")
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn deterministic_under_input_reordering(world in world(), seed in any::<u64>()) {
        let (stores, machine) = build(&world, &["work", "home"]);
        let (moved, moved_machine) = shuffled(stores.clone(), machine.clone(), seed);
        let set = StoreSet::new(stores).unwrap();
        let set_moved = StoreSet::new(moved).unwrap();
        match (resolve(&set, &machine), resolve(&set_moved, &moved_machine)) {
            (Ok(a), Ok(b)) => prop_assert_eq!(a, b),
            (Err(_), Err(_)) => {}
            (a, b) => prop_assert!(false, "ok-ness depends on order: {:?} vs {:?}", a.is_ok(), b.is_ok()),
        }
    }

    #[test]
    fn exclude_always_wins(world in world()) {
        let (stores, machine) = build(&world, &["work", "home"]);
        if let Some(plan) = plan_of(stores, &machine) {
            for repo in &plan.repos {
                for x in &repo.excluded {
                    prop_assert!(
                        !repo.skills.iter().any(|s| s.store == x.store && s.name == x.name),
                        "{} still uses excluded {}", repo.name, x.name
                    );
                }
            }
        }
    }

    #[test]
    fn nothing_crosses_scopes_or_stores(world in world()) {
        let (stores, machine) = build(&world, &["work", "home"]);
        if let Some(plan) = plan_of(stores, &machine) {
            assert_no_crossing(&plan, &world);
        }
    }

    #[test]
    fn unmounting_the_work_store_leaves_no_work_content(world in world()) {
        let (both, machine) = build(&world, &["work", "home"]);
        let (alone, _) = build(&world, &["home"]);
        let with_work = plan_of(both, &machine);
        let without = plan_of(alone, &machine);
        if with_work.is_some() {
            prop_assert!(without.is_some(), "the home store resolves on its own");
        }
        if let Some(plan) = &without {
            for repo in &plan.repos {
                prop_assert_eq!(&repo.store, "home");
                prop_assert!(!repo.grant.dirs.iter().any(|d| mentions_work(&d.path)));
                prop_assert!(!repo.skills.iter().any(|s| mentions_work(&s.dir)));
            }
            for registry in &plan.registries {
                prop_assert!(!registry.entries.iter().any(|e| e.name.starts_with("work-")));
            }
            if let Some(full) = &with_work {
                let home: Vec<&RepoPlan> = full.repos.iter().filter(|r| r.store == "home").collect();
                prop_assert_eq!(plan.repos.iter().collect::<Vec<_>>(), home);
            }
        }
    }
}

/// The properties above are vacuous for worlds that fail to resolve; keep the generator
/// producing a healthy share of valid ones.
#[test]
fn most_generated_worlds_resolve() {
    use proptest::strategy::ValueTree;
    use proptest::test_runner::TestRunner;
    let mut runner = TestRunner::deterministic();
    let samples = 400;
    let ok = (0..samples)
        .filter(|_| {
            let world = world().new_tree(&mut runner).unwrap().current();
            let (stores, machine) = build(&world, &["work", "home"]);
            plan_of(stores, &machine).is_some()
        })
        .count();
    eprintln!("{ok}/{samples} generated worlds resolve");
    assert!(
        ok * 4 >= samples,
        "only {ok}/{samples} generated worlds resolve"
    );
}
