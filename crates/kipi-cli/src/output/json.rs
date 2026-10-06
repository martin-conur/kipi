//! The stable `--json` form of a [`Plan`] (documented in `docs/json.md`).
//!
//! The JSON is built from these view structs rather than by deriving `Serialize` on
//! kipi-core's types, so the internal model can change without changing the output.
//! Field order is declaration order; lists keep the plan's deterministic order.

use std::path::{Path, PathBuf};

use kipi_core::model::StoreSet;
use kipi_core::plan::{
    AlwaysOn, Decided, GrantKind, Plan, PlannedSkill, Registry, RepoPlan, SkillOrigin, Source,
};
use serde::Serialize;

/// Bumped on any breaking change to the shape below.
pub const JSON_VERSION: u32 = 1;

/// `plan` (or the part of it named by `only`) as pretty-printed JSON with a trailing
/// newline. Fails only on a path that is not valid UTF-8.
pub fn plan(set: &StoreSet, plan: &Plan, only: Option<&RepoPlan>) -> serde_json::Result<String> {
    let repos: Vec<&RepoPlan> = match only {
        Some(r) => vec![r],
        None => plan.repos.iter().collect(),
    };
    let registries: Vec<&Registry> = match only {
        Some(r) => plan.registry_of(r).into_iter().collect(),
        None => plan.registries.iter().collect(),
    };
    let doc = PlanJson {
        version: JSON_VERSION,
        repos: repos.into_iter().map(|r| repo(set, r)).collect(),
        registries: registries.into_iter().map(|r| registry(set, r)).collect(),
    };
    let mut out = serde_json::to_string_pretty(&doc)?;
    out.push('\n');
    Ok(out)
}

#[derive(Serialize)]
struct PlanJson<'a> {
    version: u32,
    repos: Vec<RepoJson<'a>>,
    registries: Vec<RegistryJson<'a>>,
}

#[derive(Serialize)]
struct SourceJson<'a> {
    rule: &'a str,
    at: Option<String>,
}

#[derive(Serialize)]
struct DecidedJson<'a, T: Serialize> {
    value: T,
    source: SourceJson<'a>,
}

#[derive(Serialize)]
struct RepoJson<'a> {
    name: &'a str,
    store: &'a str,
    at: String,
    path: Option<&'a Path>,
    scope: DecidedJson<'a, Option<&'a str>>,
    chain: &'a [String],
    skills: Vec<SkillJson<'a>>,
    excluded: Vec<SkillJson<'a>>,
    always: Vec<AlwaysJson<'a>>,
    registry: Option<DecidedJson<'a, &'a str>>,
    related: Vec<RelatedJson<'a>>,
    grant: GrantJson<'a>,
    agents: DecidedJson<'a, Vec<&'static str>>,
    budget_tokens: DecidedJson<'a, u32>,
}

#[derive(Serialize)]
struct SkillJson<'a> {
    name: &'a str,
    store: &'a str,
    reference: String,
    dir: &'a Path,
    /// The scope whose `skills` named it, or `null` for the repo's own list.
    inherited_from: Option<&'a str>,
    source: SourceJson<'a>,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum AlwaysJson<'a> {
    File {
        path: &'a Path,
        scope: &'a str,
        source: SourceJson<'a>,
    },
    Registry {
        scope: &'a str,
        source: SourceJson<'a>,
    },
    ThisRepo,
}

#[derive(Serialize)]
struct RelatedJson<'a> {
    name: &'a str,
    scope: Option<&'a str>,
    path: Option<&'a Path>,
    notes: Option<&'a Path>,
    source: SourceJson<'a>,
}

#[derive(Serialize)]
struct GrantJson<'a> {
    mode: DecidedJson<'a, &'static str>,
    dirs: Vec<GrantDirJson<'a>>,
}

#[derive(Serialize)]
struct GrantDirJson<'a> {
    path: &'a Path,
    kind: &'static str,
    /// The store, for `store-repos` and `store-skills`.
    store: Option<&'a str>,
    /// The repo entry, for `repo`.
    repo: Option<&'a str>,
    source: SourceJson<'a>,
}

#[derive(Serialize)]
struct RegistryJson<'a> {
    scope: &'a str,
    store: &'a str,
    scopes: &'a [String],
    entries: Vec<RegistryEntryJson<'a>>,
    collisions: Vec<CollisionJson<'a>>,
    source: SourceJson<'a>,
}

#[derive(Serialize)]
struct RegistryEntryJson<'a> {
    name: &'a str,
    aliases: &'a [String],
    aliases_draft: bool,
    summary: Option<&'a str>,
    summary_draft: bool,
    scope: &'a str,
    local: Option<&'a Path>,
    notes: Option<&'a Path>,
    keys: &'a [String],
    source: SourceJson<'a>,
}

#[derive(Serialize)]
struct CollisionJson<'a> {
    key: &'a str,
    repos: &'a [String],
}

fn source<'a>(set: &StoreSet, s: &'a Source) -> SourceJson<'a> {
    SourceJson {
        rule: &s.rule,
        at: s.at.as_ref().map(|at| set.provenance(at)),
    }
}

fn decided<'a, T, U: Serialize>(
    set: &StoreSet,
    d: &'a Decided<T>,
    value: impl FnOnce(&'a T) -> U,
) -> DecidedJson<'a, U> {
    DecidedJson {
        value: value(&d.value),
        source: source(set, &d.source),
    }
}

fn skill<'a>(set: &StoreSet, s: &'a PlannedSkill) -> SkillJson<'a> {
    SkillJson {
        name: &s.name,
        store: &s.store,
        reference: s.reference.to_string(),
        dir: &s.dir,
        inherited_from: match &s.origin {
            SkillOrigin::Scope(scope) => Some(scope),
            SkillOrigin::Repo => None,
        },
        source: source(set, &s.source),
    }
}

fn repo<'a>(set: &StoreSet, r: &'a RepoPlan) -> RepoJson<'a> {
    RepoJson {
        name: &r.name,
        store: &r.store,
        at: set.provenance(&r.at),
        path: r.path.as_deref(),
        scope: decided(set, &r.scope, |v| v.as_deref()),
        chain: &r.chain,
        skills: r.skills.iter().map(|s| skill(set, s)).collect(),
        excluded: r.excluded.iter().map(|s| skill(set, s)).collect(),
        always: r
            .always
            .iter()
            .map(|a| match a {
                AlwaysOn::File {
                    path,
                    scope,
                    source: s,
                } => AlwaysJson::File {
                    path,
                    scope,
                    source: source(set, s),
                },
                AlwaysOn::Registry { scope, source: s } => AlwaysJson::Registry {
                    scope,
                    source: source(set, s),
                },
                AlwaysOn::ThisRepo => AlwaysJson::ThisRepo,
            })
            .collect(),
        registry: r.registry.as_ref().map(|d| decided(set, d, String::as_str)),
        related: r
            .related
            .iter()
            .map(|rel| RelatedJson {
                name: &rel.name,
                scope: rel.scope.as_deref(),
                path: rel.path.as_deref(),
                notes: rel.notes.as_deref(),
                source: source(set, &rel.source),
            })
            .collect(),
        grant: GrantJson {
            mode: decided(set, &r.grant.mode, |m| m.as_str()),
            dirs: r
                .grant
                .dirs
                .iter()
                .map(|d| {
                    let (kind, store, repo) = match &d.kind {
                        GrantKind::StoreRepos => ("store-repos", Some(r.store.as_str()), None),
                        GrantKind::StoreSkills { store } => {
                            ("store-skills", Some(store.as_str()), None)
                        }
                        GrantKind::Repo { name } => ("repo", None, Some(name.as_str())),
                    };
                    GrantDirJson {
                        path: &d.path,
                        kind,
                        store,
                        repo,
                        source: source(set, &d.source),
                    }
                })
                .collect(),
        },
        agents: decided(set, &r.agents, |a| a.iter().map(|a| a.as_str()).collect()),
        budget_tokens: decided(set, &r.budget_tokens, |b| *b),
    }
}

fn registry<'a>(set: &StoreSet, r: &'a Registry) -> RegistryJson<'a> {
    RegistryJson {
        scope: &r.scope,
        store: &r.store,
        scopes: &r.scopes,
        entries: r
            .entries
            .iter()
            .map(|e| RegistryEntryJson {
                name: &e.name,
                aliases: &e.aliases,
                aliases_draft: e.aliases_draft,
                summary: e.summary.as_deref(),
                summary_draft: e.summary_draft,
                scope: &e.scope,
                local: e.local.as_deref(),
                notes: e.notes.as_deref(),
                keys: &e.keys,
                source: source(set, &e.source),
            })
            .collect(),
        collisions: r
            .collisions
            .iter()
            .map(|c| CollisionJson {
                key: &c.key,
                repos: &c.repos,
            })
            .collect(),
        source: source(set, &r.source),
    }
}

/// A `kipi scan` report as pretty-printed JSON with a trailing newline (`docs/json.md`,
/// "Scan"). Fails only on a path that is not valid UTF-8.
pub fn scan(scan: &kipi_core::scan::Scan, dry_run: bool) -> serde_json::Result<String> {
    let doc = ScanJson {
        version: JSON_VERSION,
        dry_run,
        roots: scan.roots.iter().map(PathBuf::as_path).collect(),
        missing_roots: scan.missing_roots.iter().map(PathBuf::as_path).collect(),
        checkouts: scan.checkouts.iter().map(PathBuf::as_path).collect(),
        added: scan
            .added
            .iter()
            .map(|a| AddedJson {
                name: &a.name,
                store: &a.store,
                scope: &a.scope,
                explicit_scope: a.explicit_scope,
                path: &a.path,
                remote: a.remote.as_ref().map(|r| r.as_str()),
                summary: a.summary.as_deref(),
                aliases: &a.aliases,
            })
            .collect(),
        recorded: scan
            .recorded
            .iter()
            .map(|r| RecordedJson {
                entry: &r.entry,
                path: &r.path,
                previous: r.previous.as_deref(),
            })
            .collect(),
        remotes_filled: scan
            .remotes_filled
            .iter()
            .map(|r| RemoteFilledJson {
                entry: &r.entry,
                store: &r.store,
                remote: r.remote.as_str(),
            })
            .collect(),
        refreshed: scan
            .refreshed
            .iter()
            .map(|r| RefreshedJson {
                entry: &r.entry,
                store: &r.store,
                field: r.field,
                value: &r.value,
            })
            .collect(),
        warnings: scan
            .warnings
            .iter()
            .map(|w| WarningJson {
                code: w.code(),
                entry: w.entry(),
                path: w.path(),
                message: w.message(|p| p.display().to_string()),
                hint: w.hint(),
            })
            .collect(),
        not_found: &scan.not_found,
        files: scan.writes.iter().map(|w| w.path.as_path()).collect(),
    };
    let mut out = serde_json::to_string_pretty(&doc)?;
    out.push('\n');
    Ok(out)
}

#[derive(Serialize)]
struct ScanJson<'a> {
    version: u32,
    dry_run: bool,
    roots: Vec<&'a Path>,
    missing_roots: Vec<&'a Path>,
    checkouts: Vec<&'a Path>,
    added: Vec<AddedJson<'a>>,
    recorded: Vec<RecordedJson<'a>>,
    remotes_filled: Vec<RemoteFilledJson<'a>>,
    refreshed: Vec<RefreshedJson<'a>>,
    warnings: Vec<WarningJson<'a>>,
    not_found: &'a [String],
    files: Vec<&'a Path>,
}

#[derive(Serialize)]
struct AddedJson<'a> {
    name: &'a str,
    store: &'a str,
    scope: &'a str,
    explicit_scope: bool,
    path: &'a Path,
    remote: Option<&'a str>,
    summary: Option<&'a str>,
    aliases: &'a [String],
}

#[derive(Serialize)]
struct RecordedJson<'a> {
    entry: &'a str,
    path: &'a Path,
    previous: Option<&'a Path>,
}

#[derive(Serialize)]
struct RemoteFilledJson<'a> {
    entry: &'a str,
    store: &'a str,
    remote: &'a str,
}

#[derive(Serialize)]
struct RefreshedJson<'a> {
    entry: &'a str,
    store: &'a str,
    field: &'a str,
    value: &'a str,
}

#[derive(Serialize)]
struct WarningJson<'a> {
    code: &'a str,
    entry: Option<&'a str>,
    path: &'a Path,
    message: String,
    hint: String,
}
