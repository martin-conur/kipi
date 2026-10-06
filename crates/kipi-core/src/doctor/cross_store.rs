//! K012: a reference into another store (`extends`, `scope`, `related`, `exclude`, a
//! bare skill of another store, or a qualified skill of a store without
//! `share_skills`). Qualified skills that do not exist are K005.

use crate::model::{RefKind, RefProblem};

use super::prepare::Removal;
use super::{Code, Finding};

/// The finding for a reference [`crate::model::StoreSet::checked`] rejected.
pub(super) fn removal(problem: RefProblem) -> Removal {
    let code = match problem.kind {
        RefKind::CrossStore => Code::CrossStore,
        RefKind::MissingSkill => Code::MissingSkill,
    };
    let finding = Finding::new(
        code,
        problem.site.repo(),
        problem.error.location().cloned(),
        problem.error.message(),
        problem.error.hint(),
    );
    Removal {
        store: problem.store,
        site: problem.site,
        finding,
    }
}
