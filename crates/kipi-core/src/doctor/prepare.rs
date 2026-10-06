//! Before anything resolves: report and remove the references that would stop loading
//! or resolution (K012, K005, K003 for `related`), so the remaining checks still run.

use crate::error::Result;
use crate::model::{Machine, RefSite, Store, StoreSet};
use crate::plan::Plan;
use crate::resolve::resolve;

use super::{Finding, cross_store, missing_skills, scope_crossing};

/// The stores without the references doctor reported, and their plan.
#[derive(Debug)]
pub struct Prepared {
    /// The stores, checked as a set.
    pub stores: StoreSet,
    /// The plan resolved from them.
    pub plan: Plan,
    /// One finding per removed reference.
    pub findings: Vec<Finding>,
}

/// A bad reference: the finding to report and the reference to remove.
pub(super) struct Removal {
    pub store: String,
    pub site: RefSite,
    pub finding: Finding,
}

/// Report and remove, in turn: references into another store and qualified skills that
/// do not exist (as the set check sees them), skills missing from their own store, and
/// `related` repos outside the scope chain. Then resolve what is left.
pub fn prepare(stores: Vec<Store>, machine: &Machine) -> Result<Prepared> {
    let mut findings = Vec::new();
    let (set, problems) = StoreSet::checked(stores)?;
    let removals = problems.into_iter().map(cross_store::removal).collect();
    let set = remove(set, removals, &mut findings)?;
    let removals = missing_skills::removals(&set);
    let set = remove(set, removals, &mut findings)?;
    let removals = scope_crossing::related_removals(&set, machine)?;
    let set = remove(set, removals, &mut findings)?;
    let plan = resolve(&set, machine)?;
    Ok(Prepared {
        stores: set,
        plan,
        findings,
    })
}

fn remove(set: StoreSet, removals: Vec<Removal>, findings: &mut Vec<Finding>) -> Result<StoreSet> {
    if removals.is_empty() {
        return Ok(set);
    }
    let mut stores = set.into_stores();
    for r in removals {
        if let Some(store) = stores.iter_mut().find(|s| s.name == r.store) {
            r.site.remove_from(store);
        }
        findings.push(r.finding);
    }
    StoreSet::new(stores)
}
