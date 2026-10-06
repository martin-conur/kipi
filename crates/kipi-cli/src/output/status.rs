//! What `kipi status` reports per repo, shared by its table and its `--json` form.

use kipi_core::plan::RepoPlan;
use kipi_core::sync::{RepoOutcome, RepoState};

/// Where a repo stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Disk and manifest match the plan.
    Synced,
    /// Synced before, but a sync would change something.
    Stale,
    /// Cloned here and never synced: nothing kipi writes is there.
    Missing,
    /// Not cloned on this machine.
    NotCloned,
    /// `machine.toml` names a checkout that is gone.
    CheckoutMissing,
}

impl State {
    /// The name used in output and JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            State::Synced => "synced",
            State::Stale => "stale",
            State::Missing => "missing",
            State::NotCloned => "not_cloned",
            State::CheckoutMissing => "checkout_missing",
        }
    }

    /// `state` of a dry-run sync of the repo.
    pub fn of(r: &RepoOutcome) -> State {
        match &r.state {
            RepoState::NotCloned => State::NotCloned,
            RepoState::CheckoutMissing => State::CheckoutMissing,
            RepoState::Applied(_) | RepoState::Orphan(_) if !r.had_manifest => State::Missing,
            RepoState::Applied(report) | RepoState::Orphan(report) if report.is_unchanged() => {
                State::Synced
            }
            RepoState::Applied(_) | RepoState::Orphan(_) => State::Stale,
        }
    }
}

/// One repo's status line.
pub struct Row<'a> {
    /// The plan.
    pub plan: &'a RepoPlan,
    /// The dry-run sync.
    pub outcome: RepoOutcome,
    /// Its state.
    pub state: State,
    /// Its summary is still marked `# kipi:draft`.
    pub draft_summary: bool,
}
