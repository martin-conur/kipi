//! K016: a repo's always-on text (the generated block plus its skills' descriptions)
//! over its `budget_tokens`, naming the largest contributors.

use crate::render::{block, registry};
use crate::sync::always_on_size;
use crate::tokens::human;

use super::{Code, Finding, Inputs};

/// How many contributors the message names.
const LARGEST: usize = 3;

pub(super) fn check(inputs: &Inputs<'_>, out: &mut Vec<Finding>) {
    let home = Some(inputs.home);
    for repo in inputs.repos() {
        // An unreadable conventions file stops `sync` with its own error and hint.
        let Ok(conventions) = block::load_conventions(repo) else {
            continue;
        };
        let text = block::render(inputs.plan, repo, &conventions, home).text();
        let size = always_on_size(inputs.stores, repo, &text, inputs.tokens);
        if !size.over() {
            continue;
        }
        let root = inputs.stores.store(&repo.store).map(|s| s.root.as_path());
        let mut parts: Vec<(String, u32)> = Vec::new();
        for c in &conventions {
            let path = root.and_then(|r| c.path.strip_prefix(r).ok());
            let name = path.unwrap_or(&c.path).display();
            parts.push((format!("conventions {name}"), inputs.tokens.count(&c.text)));
        }
        if let Some(r) = inputs.plan.registry_of(repo) {
            let n = inputs.tokens.count(&registry::render(r, home));
            let label = format!("registry `{}` ({} repos)", r.scope, r.entries.len());
            parts.push((label, n));
        }
        // The rest of the block: the this-repo section, headings and markers.
        let counted = parts.iter().map(|(_, n)| *n).fold(0, u32::saturating_add);
        let rest = inputs.tokens.count(&text).saturating_sub(counted);
        parts.push(("this-repo section".to_owned(), rest));
        for s in &repo.skills {
            let description = inputs
                .stores
                .store(&s.store)
                .and_then(|st| st.skill(&s.name))
                .and_then(|sk| sk.description.as_deref());
            if let Some(d) = description {
                let label = format!("skill `{}` description", s.name);
                parts.push((label, inputs.tokens.count(d)));
            }
        }
        parts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let largest: Vec<String> = parts
            .iter()
            .take(LARGEST)
            .map(|(label, n)| format!("{label} ~{} tok", human(*n)))
            .collect();
        out.push(Finding::new(
            Code::Budget,
            Some(&repo.name),
            Some(repo.at.clone()),
            format!(
                "always-on text is ~{} tokens, over the budget of {}; largest: {}",
                human(size.tokens),
                size.budget,
                largest.join(", ")
            ),
            "shorten the largest parts (move detail into notes or a skill), `exclude` skills \
             the repo does not need, narrow the registry's scope, or raise `budget_tokens`",
        ));
    }
}
