//! K005: a skill named in `skills` or `exclude` that its store does not have.
//! (Qualified references into an unmounted store, or naming a skill the sharing store
//! lacks, come from the set check; see `cross_store`.)

use crate::error::closest;
use crate::model::{RefSite, SkillRef, Store, StoreSet};

use super::prepare::Removal;
use super::{Code, Finding};

/// Every reference in `set` naming a skill that does not exist.
pub(super) fn removals(set: &StoreSet) -> Vec<Removal> {
    let mut out = Vec::new();
    for store in set.stores() {
        let mut check = |skill: &SkillRef, rule: String, repo: Option<&str>, site: RefSite, at| {
            if set.skill(&store.name, skill).is_some() {
                return;
            }
            out.push(Removal {
                store: store.name.clone(),
                site,
                finding: Finding::new(
                    Code::MissingSkill,
                    repo,
                    Some(at),
                    format!(
                        "skill `{skill}` (in `{rule}`) is not in store `{}`",
                        store.name
                    ),
                    hint(store, skill),
                ),
            });
        };
        for scope in &store.scopes {
            for skill in &scope.skills {
                let site = RefSite::ScopeSkill {
                    scope: scope.name.clone(),
                    skill: skill.clone(),
                };
                let rule = format!("scopes.{}.skills", scope.name);
                check(skill, rule, None, site, scope.at.clone());
            }
        }
        for repo in &store.repos {
            for skill in &repo.skills {
                let site = RefSite::RepoSkill {
                    repo: repo.name.clone(),
                    skill: skill.clone(),
                };
                let rule = format!("repos.{}.skills", repo.name);
                check(skill, rule, Some(&repo.name), site, repo.at.clone());
            }
            for skill in &repo.exclude {
                let site = RefSite::RepoExclude {
                    repo: repo.name.clone(),
                    skill: skill.clone(),
                };
                let rule = format!("repos.{}.exclude", repo.name);
                check(skill, rule, Some(&repo.name), site, repo.at.clone());
            }
        }
    }
    out
}

fn hint(store: &Store, skill: &SkillRef) -> String {
    match closest(&skill.name, store.skills.iter().map(|s| s.name.as_str())) {
        Some(s) => format!("did you mean `{s}`?"),
        None => format!(
            "create skills/{}/SKILL.md in store `{}` (`kipi add skill {}`) or remove the reference",
            skill.name, store.name, skill.name
        ),
    }
}
