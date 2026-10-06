//! The stable `--json` form of a [`Plan`] (documented in `docs/json.md`).
//!
//! The JSON is built from these view structs rather than by deriving `Serialize` on
//! kipi-core's types, so the internal model can change without changing the output.
//! Field order is declaration order; lists keep the plan's deterministic order.

use std::path::{Path, PathBuf};

use kipi_core::apply::Report;
use kipi_core::doctor::{Finding, Fix, Level};
use kipi_core::model::StoreSet;
use kipi_core::plan::{
    AlwaysOn, Decided, GrantKind, Plan, PlannedSkill, Registry, RepoPlan, SkillOrigin, Source,
};
use kipi_core::sync::{AlwaysOnSize, Outcome, RepoState, Tally};
use serde::Serialize;

use super::status::Row;

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

/// A `kipi sync` outcome as pretty-printed JSON with a trailing newline (`docs/json.md`,
/// "Sync"). Fails only on a path that is not valid UTF-8.
pub fn sync(outcome: &Outcome, dry_run: bool) -> serde_json::Result<String> {
    let doc = SyncJson {
        version: JSON_VERSION,
        dry_run,
        repos: outcome
            .repos
            .iter()
            .map(|r| SyncRepoJson {
                name: &r.name,
                store: r.store.as_deref(),
                path: r.path.as_deref(),
                status: match &r.state {
                    RepoState::Applied(report) if report.is_unchanged() => "unchanged",
                    RepoState::Applied(_) => "changed",
                    RepoState::NotCloned => "not_cloned",
                    RepoState::CheckoutMissing => "checkout_missing",
                    RepoState::Orphan(_) => "orphan",
                },
                always_on: r.always_on.map(always_on),
                notices: r
                    .notices
                    .iter()
                    .map(|n| NoticeJson {
                        message: &n.message,
                        hint: &n.hint,
                    })
                    .collect(),
                report: r.state.report().map(report),
            })
            .collect(),
        registries: outcome
            .registries
            .iter()
            .map(|s| StoreSyncJson {
                store: &s.store,
                report: report(&s.report),
            })
            .collect(),
    };
    let mut out = serde_json::to_string_pretty(&doc)?;
    out.push('\n');
    Ok(out)
}

/// `kipi doctor`'s findings as pretty-printed JSON with a trailing newline
/// (`docs/doctor.md`, "JSON").
pub fn doctor(set: &StoreSet, findings: &[Finding]) -> serde_json::Result<String> {
    let count = |level| findings.iter().filter(|f| f.level == level).count();
    let doc = DoctorJson {
        version: JSON_VERSION,
        findings: findings
            .iter()
            .map(|f| FindingJson {
                level: f.level.as_str(),
                code: f.code.as_str(),
                check: f.code.name(),
                repo: f.repo.as_deref(),
                at: f.at.as_ref().map(|at| set.provenance(at)),
                message: &f.message,
                hint: &f.hint,
                fix: f.fix.as_ref().map(|fix| match fix {
                    Fix::FillRemote {
                        store,
                        repo,
                        remote,
                    } => FixJson::FillRemote {
                        store,
                        repo,
                        remote: remote.as_str(),
                    },
                }),
            })
            .collect(),
        summary: DoctorSummaryJson {
            error: count(Level::Error),
            warn: count(Level::Warn),
            info: count(Level::Info),
        },
    };
    let mut out = serde_json::to_string_pretty(&doc)?;
    out.push('\n');
    Ok(out)
}

#[derive(Serialize)]
struct DoctorJson<'a> {
    version: u32,
    findings: Vec<FindingJson<'a>>,
    summary: DoctorSummaryJson,
}

#[derive(Serialize)]
struct FindingJson<'a> {
    level: &'static str,
    code: &'static str,
    check: &'static str,
    repo: Option<&'a str>,
    at: Option<String>,
    message: &'a str,
    hint: &'a str,
    fix: Option<FixJson<'a>>,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum FixJson<'a> {
    FillRemote {
        store: &'a str,
        repo: &'a str,
        remote: &'a str,
    },
}

#[derive(Serialize)]
struct DoctorSummaryJson {
    error: usize,
    warn: usize,
    info: usize,
}

/// A `kipi status` report as pretty-printed JSON with a trailing newline (`docs/json.md`,
/// "Status"). Fails only on a path that is not valid UTF-8.
pub fn status(rows: &[Row]) -> serde_json::Result<String> {
    let doc = StatusJson {
        version: JSON_VERSION,
        repos: rows
            .iter()
            .map(|r| StatusRepoJson {
                name: &r.plan.name,
                store: &r.plan.store,
                scope: r.plan.scope.value.as_deref(),
                path: r.plan.path.as_deref(),
                state: r.state.as_str(),
                pending: r.outcome.state.report().map(|rep| report(rep).changes),
                always_on: r.outcome.always_on.map(always_on),
                draft_summary: r.draft_summary,
            })
            .collect(),
        draft_summaries: rows.iter().filter(|r| r.draft_summary).count(),
    };
    let mut out = serde_json::to_string_pretty(&doc)?;
    out.push('\n');
    Ok(out)
}

fn always_on(s: AlwaysOnSize) -> AlwaysOnJson {
    AlwaysOnJson {
        tokens: s.tokens,
        budget: s.budget,
        over: s.over(),
    }
}

fn report(r: &Report) -> ReportJson<'_> {
    let t = Tally::of(r);
    ReportJson {
        changes: r
            .changes
            .iter()
            .map(|c| ChangeJson {
                op: c.op.as_str(),
                what: c.what.as_str(),
                path: &c.path,
            })
            .collect(),
        links: CountJson {
            added: t.links_added,
            removed: t.links_removed,
        },
        blocks: BlocksJson {
            written: t.blocks_written,
            removed: t.blocks_removed,
        },
        grants: r
            .grants
            .iter()
            .map(|g| GrantChangeJson {
                path: &g.path,
                added: &g.added,
                removed: &g.removed,
            })
            .collect(),
        exclude_changed: t.exclude_changed,
        skipped: r
            .skipped
            .iter()
            .map(|s| SkippedJson {
                path: &s.path,
                message: &s.message,
                hint: &s.hint,
            })
            .collect(),
        foreign: r.foreign.iter().map(PathBuf::as_path).collect(),
    }
}

#[derive(Serialize)]
struct SyncJson<'a> {
    version: u32,
    dry_run: bool,
    repos: Vec<SyncRepoJson<'a>>,
    registries: Vec<StoreSyncJson<'a>>,
}

#[derive(Serialize)]
struct SyncRepoJson<'a> {
    name: &'a str,
    store: Option<&'a str>,
    path: Option<&'a Path>,
    status: &'static str,
    always_on: Option<AlwaysOnJson>,
    notices: Vec<NoticeJson<'a>>,
    report: Option<ReportJson<'a>>,
}

#[derive(Serialize)]
struct StoreSyncJson<'a> {
    store: &'a str,
    report: ReportJson<'a>,
}

#[derive(Serialize)]
struct AlwaysOnJson {
    tokens: u32,
    budget: u32,
    over: bool,
}

#[derive(Serialize)]
struct NoticeJson<'a> {
    message: &'a str,
    hint: &'a str,
}

#[derive(Serialize)]
struct ReportJson<'a> {
    changes: Vec<ChangeJson<'a>>,
    links: CountJson,
    blocks: BlocksJson,
    grants: Vec<GrantChangeJson<'a>>,
    exclude_changed: bool,
    skipped: Vec<SkippedJson<'a>>,
    foreign: Vec<&'a Path>,
}

#[derive(Serialize)]
struct ChangeJson<'a> {
    op: &'static str,
    what: &'static str,
    path: &'a Path,
}

#[derive(Serialize)]
struct CountJson {
    added: usize,
    removed: usize,
}

#[derive(Serialize)]
struct BlocksJson {
    written: usize,
    removed: usize,
}

#[derive(Serialize)]
struct GrantChangeJson<'a> {
    path: &'a Path,
    added: &'a [String],
    removed: &'a [String],
}

#[derive(Serialize)]
struct SkippedJson<'a> {
    path: &'a Path,
    message: &'a str,
    hint: &'a str,
}

#[derive(Serialize)]
struct StatusJson<'a> {
    version: u32,
    repos: Vec<StatusRepoJson<'a>>,
    draft_summaries: usize,
}

#[derive(Serialize)]
struct StatusRepoJson<'a> {
    name: &'a str,
    store: &'a str,
    scope: Option<&'a str>,
    path: Option<&'a Path>,
    state: &'static str,
    pending: Option<Vec<ChangeJson<'a>>>,
    always_on: Option<AlwaysOnJson>,
    draft_summary: bool,
}
