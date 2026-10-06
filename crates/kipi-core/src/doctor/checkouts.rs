//! K014 and K015: checkouts on this machine against their entries. A checkout whose
//! origin differs from its entry's `remote`, or two checkouts matching one entry, is a
//! warning (K014); an entry without `remote` whose checkout has one is info with a safe
//! fix (K015).

use std::collections::BTreeMap;
use std::path::Path;

use crate::model::{Checkout, RepoMatch, match_checkout};
use crate::render::display_path;

use super::{Code, Finding, Fix, Inputs};

pub(super) fn check(inputs: &Inputs<'_>, out: &mut Vec<Finding>) {
    let shown = |p: &Path| display_path(p, Some(inputs.home));
    for repo in inputs.repos() {
        let (Some((_, entry)), Some(checkout)) = (
            inputs.stores.repo(&repo.name),
            inputs.checkouts.get(&repo.name),
        ) else {
            continue;
        };
        let Some(origin) = &checkout.origin else {
            continue;
        };
        match &entry.remote {
            Some(remote) if remote != origin => out.push(Finding::new(
                Code::CheckoutMismatch,
                Some(&repo.name),
                Some(repo.at.clone()),
                format!(
                    "the checkout of `{}` at {} has origin `{origin}`, but the entry's remote \
                     is `{remote}`",
                    repo.name,
                    shown(&checkout.path)
                ),
                format!(
                    "if the checkout is another repo, fix its machine.toml [repos] line \
                     (`kipi scan` rebuilds it); if the repo moved, update `remote` in [repos.{}]",
                    repo.name
                ),
            )),
            Some(_) => {}
            None => out.push(
                Finding::new(
                    Code::MissingRemote,
                    Some(&repo.name),
                    Some(repo.at.clone()),
                    format!(
                        "entry `{}` has no `remote`, but its checkout's origin is `{origin}`",
                        repo.name
                    ),
                    format!(
                        "add `remote = \"{origin}\"` to [repos.{}] so the repo is recognized \
                         by its remote on every machine (`kipi scan` fills it too)",
                        repo.name
                    ),
                )
                .with_fix(Fix::FillRemote {
                    store: repo.store.clone(),
                    repo: repo.name.clone(),
                    remote: origin.clone(),
                }),
            ),
        }
    }

    // Two checkouts matching one entry, by the rules `scan` uses.
    let entries: Vec<_> = inputs.stores.repos().map(|(_, e)| e).collect();
    let mut matched: BTreeMap<&str, Vec<(&str, &Path)>> = BTreeMap::new();
    for (key, checkout) in &inputs.checkouts.by_entry {
        let dir_name = checkout
            .path
            .file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default();
        let found = Checkout {
            dir_name: &dir_name,
            remote: checkout.origin.as_ref(),
        };
        if let RepoMatch::Unique { entry, .. } = match_checkout(found, entries.iter().copied()) {
            matched
                .entry(entry.name.as_str())
                .or_default()
                .push((key.as_str(), checkout.path.as_path()));
        }
    }
    for (entry, checkouts) in matched.into_iter().filter(|(_, c)| c.len() > 1) {
        if inputs
            .only
            .is_some_and(|o| o != entry && !checkouts.iter().any(|(k, _)| *k == o))
        {
            continue;
        }
        let Some(repo) = inputs.plan.repo(entry) else {
            continue;
        };
        let list: Vec<String> = checkouts
            .iter()
            .map(|(key, path)| format!("{} (machine.toml `{key}`)", shown(path)))
            .collect();
        out.push(Finding::new(
            Code::CheckoutMismatch,
            Some(entry),
            Some(repo.at.clone()),
            format!(
                "{} checkouts on this machine match entry `{entry}`: {}",
                checkouts.len(),
                list.join(", ")
            ),
            "keep one: point the other machine.toml [repos] lines at the entries they really \
             are, or remove them (`kipi scan` rebuilds the table)",
        ));
    }
}
