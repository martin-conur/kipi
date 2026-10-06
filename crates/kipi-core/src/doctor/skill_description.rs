//! K009: a skill whose `description` is missing or longer than agents accept. Agents
//! read only the description to decide when to load a skill.

use super::{Code, Finding, Inputs};

/// The longest `description` the Agent Skills format allows, in characters.
pub const MAX_DESCRIPTION_CHARS: usize = 1024;

pub(super) fn check(inputs: &Inputs<'_>, out: &mut Vec<Finding>) {
    if !inputs.store_level() {
        return;
    }
    for store in inputs.stores.stores() {
        for skill in &store.skills {
            let desc = skill.description.as_deref().map(str::trim).unwrap_or("");
            let len = desc.chars().count();
            let (message, hint) = if desc.is_empty() {
                (
                    format!(
                        "skill `{}` of store `{}` has no `description`",
                        skill.name, store.name
                    ),
                    "add `description:` to its SKILL.md frontmatter: agents read it to decide \
                     when to load the skill"
                        .to_owned(),
                )
            } else if len > MAX_DESCRIPTION_CHARS {
                (
                    format!(
                        "the `description` of skill `{}` of store `{}` is {len} characters, \
                         over the {MAX_DESCRIPTION_CHARS} limit",
                        skill.name, store.name
                    ),
                    "shorten it to say when to use the skill; move detail into the body".to_owned(),
                )
            } else {
                continue;
            };
            out.push(Finding::new(
                Code::SkillDescription,
                None,
                Some(skill.at.clone()),
                message,
                hint,
            ));
        }
    }
}
