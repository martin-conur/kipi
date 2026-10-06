//! K017: a `machine.toml [repos]` checkout that no longer exists. Safe fix: remove the line.

use super::{Code, Finding, Fix, Inputs};

pub(super) fn check(inputs: &Inputs<'_>, out: &mut Vec<Finding>) {
    for (entry, path) in &inputs.checkouts.missing {
        if inputs.only.is_some_and(|o| o != entry) {
            continue;
        }
        let at = inputs
            .machine
            .repos
            .iter()
            .find(|r| r.entry == *entry)
            .map(|r| r.at.clone());
        out.push(
            Finding::new(
                Code::CheckoutGone,
                Some(entry),
                at,
                format!(
                    "machine.toml places `{entry}` at {}, which is no longer a directory",
                    inputs.shown(path)
                ),
                "if the repo moved, `kipi scan` records its new path; if it is gone, \
                 `kipi doctor --fix` removes the line",
            )
            .with_fix(Fix::RemoveCheckout {
                entry: entry.clone(),
                path: path.clone(),
            }),
        );
    }
}
