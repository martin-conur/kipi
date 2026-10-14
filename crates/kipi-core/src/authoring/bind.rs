//! `kipi bind` / `kipi unbind`: add or remove skills, related repos, exclusions and
//! aliases on a repo entry, and describe a relation (`--via`).

use super::toml::{clear_draft, contains, push, remove, set_via, table_mut, via};
use super::{Change, Op, Outcome, bindings_write, find_repo, revalidate};
use crate::config::Env;
use crate::error::{Error, Result};
use crate::model::{Machine, SkillRef, StoreSet, check_name, check_via};
use crate::plan::{SkillOrigin, alias_key};

/// The arguments of `bind` and `unbind`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Binding<'a> {
    /// The repo entry.
    pub repo: &'a str,
    /// `--skill`.
    pub skills: &'a [String],
    /// `--related`.
    pub related: &'a [String],
    /// `--exclude`.
    pub exclude: &'a [String],
    /// `--alias`.
    pub aliases: &'a [String],
    /// `--via`: the description of the one `--related` relation; `""` removes it.
    pub via: Option<&'a str>,
}

impl Binding<'_> {
    /// `(key, values)` in the order they are edited and reported.
    fn keys(&self) -> [(&'static str, &[String]); 4] {
        [
            ("skills", self.skills),
            ("related", self.related),
            ("exclude", self.exclude),
            ("aliases", self.aliases),
        ]
    }
}

/// Add the values of `b` to its repo entry (values already there are reported as kept).
/// Adding an alias clears the aliases' `# kipi:draft` marker.
pub fn bind(stores: &StoreSet, machine: &Machine, env: &Env, b: &Binding) -> Result<Outcome> {
    edit(stores, machine, env, b, true)
}

/// Remove the values of `b` from its repo entry (values not there are reported as kept);
/// a list left empty is removed. Removing an alias clears the draft marker.
pub fn unbind(stores: &StoreSet, machine: &Machine, env: &Env, b: &Binding) -> Result<Outcome> {
    edit(stores, machine, env, b, false)
}

fn edit(
    stores: &StoreSet,
    machine: &Machine,
    env: &Env,
    b: &Binding,
    adding: bool,
) -> Result<Outcome> {
    let command = if adding { "bind" } else { "unbind" };
    check_via_flag(b, adding)?;
    if b.keys().iter().all(|(_, values)| values.is_empty()) {
        return Err(Error::usage(
            format!("nothing to {command}"),
            format!(
                "pass at least one of --skill, --related, --exclude or --alias, e.g. \
                 `kipi {command} {} --skill <skill>`",
                b.repo
            ),
        ));
    }
    let (store, repo) = find_repo(stores, b.repo)?;
    validate(b)?;

    let mut doc = store.bindings.doc.clone();
    let table = table_mut(&mut doc, &["repos", &repo.name]).ok_or_else(|| {
        Error::usage(
            format!(
                "cannot edit [repos.{}] in {}",
                repo.name,
                store.bindings.path.display()
            ),
            "declare the entry as a `[repos.<name>]` table",
        )
    })?;
    let mut changes = Vec::new();
    let mut added_aliases = Vec::new();
    for (key, values) in b.keys() {
        let mut edited = false;
        for value in dedup(values) {
            let same = |v: &str| same_value(key, v, value);
            let present = contains(table, key, same);
            let change = match (adding, present) {
                (true, false) => {
                    push(table, key, value);
                    if key == "aliases" {
                        added_aliases.push(alias_key(value));
                    }
                    Change::new(Op::Add, key, value)
                }
                (false, true) => {
                    remove(table, key, same);
                    Change::new(Op::Remove, key, value)
                }
                (true, true) => Change::new(Op::Keep, key, value).note("already there"),
                (false, false) => Change::new(Op::Keep, key, value).note("not there"),
            };
            edited |= change.op != Op::Keep;
            changes.push(change);
            if key == "related"
                && adding
                && let Some(wanted) = b.via
            {
                changes.push(describe(table, value, wanted));
            }
        }
        if key == "aliases" && edited {
            clear_draft(table, key);
        }
    }

    let text = doc.to_string();
    let plan = revalidate(stores, machine, env, &[(&store.name, text.clone())], None)?;
    if let Some(collision) = plan
        .registries
        .iter()
        .flat_map(|r| r.collisions.iter().map(move |c| (r, c)))
        .find(|(_, c)| c.repos.contains(&repo.name) && added_aliases.contains(&c.key))
    {
        let (registry, c) = collision;
        let others: Vec<&str> = c
            .repos
            .iter()
            .filter(|r| **r != repo.name)
            .map(String::as_str)
            .collect();
        return Err(Error::usage(
            format!(
                "alias `{}` of `{}` is already a name or alias of `{}` in the registry of scope \
                 `{}`",
                c.key,
                repo.name,
                others.join("`, `"),
                registry.scope
            ),
            "aliases must be unique within a registry so the agent can resolve them; pick \
             another alias",
        )
        .rejected());
    }

    // An unbound skill the repo still gets from its scope needs `--exclude` instead.
    if !adding && let Some(planned) = plan.repo(&repo.name) {
        for change in changes.iter_mut().filter(|c| c.what == "skills") {
            let inherited = planned.skills.iter().find_map(|s| match &s.origin {
                SkillOrigin::Scope(scope) if s.reference.to_string() == change.value => Some(scope),
                _ => None,
            });
            if let Some(scope) = inherited {
                let note = format!(
                    "still inherited from scope `{scope}`; `kipi bind {} --exclude {}` removes it",
                    repo.name, change.value
                );
                *change = change.clone().note(note);
            }
        }
    }

    Ok(Outcome {
        store: Some(store.name.clone()),
        changes,
        writes: bindings_write(store, &text).into_iter().collect(),
        ..Outcome::default()
    })
}

/// Set (or, for `""`, drop) the `via` of the relation to `target`.
fn describe(table: &mut dyn toml_edit::TableLike, target: &str, wanted: &str) -> Change {
    let old = via(table, "related", target);
    let wanted = (!wanted.is_empty()).then_some(wanted);
    if old.as_deref() == wanted {
        let note = if wanted.is_some() {
            "already there"
        } else {
            "not there"
        };
        return Change::new(Op::Keep, "via", target).note(note);
    }
    set_via(table, "related", target, wanted);
    match wanted {
        Some(text) => Change::new(Op::Add, "via", format!("{target} ({text})")),
        None => Change::new(Op::Remove, "via", target),
    }
}

/// Whether the array value `existing` is the same `key` value as `wanted`: aliases match
/// after case-folding (as the registry matches them), everything else exactly.
fn same_value(key: &str, existing: &str, wanted: &str) -> bool {
    if key == "aliases" {
        alias_key(existing) == alias_key(wanted)
    } else {
        existing == wanted
    }
}

pub(super) fn dedup(values: &[String]) -> Vec<&str> {
    let mut seen = Vec::new();
    for v in values {
        if !seen.contains(&v.as_str()) {
            seen.push(v.as_str());
        }
    }
    seen
}

/// Every value of `flag` parses as a skill ref.
pub(super) fn check_skill_refs(flag: &str, values: &[String]) -> Result<()> {
    for v in values {
        v.parse::<SkillRef>().map_err(|why| {
            Error::usage(
                format!("invalid {flag} `{v}`: {why}"),
                "name a skill of the store (`code-review`) or a shared skill of another \
                 store (`personal/commit-style`)",
            )
        })?;
    }
    Ok(())
}

/// `--via` comes with `bind` and exactly one `--related`, and is one short line (or `""`).
fn check_via_flag(b: &Binding, adding: bool) -> Result<()> {
    if let Some(via) = b.via {
        if !adding {
            return Err(Error::usage(
                "--via only applies to `kipi bind`",
                format!(
                    "`kipi unbind {} --related <repo>` removes the relation with its \
                     description; `kipi bind {} --related <repo> --via \"\"` keeps the \
                     relation and drops the description",
                    b.repo, b.repo
                ),
            ));
        }
        if b.related.len() != 1 {
            return Err(Error::usage(
                "--via describes one relation",
                format!(
                    "pass exactly one --related with it, e.g. `kipi bind {} --related <repo> \
                     --via \"<how they connect>\"`",
                    b.repo
                ),
            ));
        }
        if !via.is_empty() {
            check_via(via).map_err(|why| {
                Error::usage(
                    format!("invalid --via: the description {why}"),
                    "keep it to one short line (at most 120 characters); pass `--via \"\"` \
                     to remove it",
                )
            })?;
        }
    }
    Ok(())
}

fn validate(b: &Binding) -> Result<()> {
    for (flag, values) in [("--skill", b.skills), ("--exclude", b.exclude)] {
        check_skill_refs(flag, values)?;
    }
    for v in b.related {
        check_name(v).map_err(|why| {
            Error::usage(
                format!("invalid --related `{v}`: the name {why}"),
                "pass the entry name of a repo in the same store",
            )
        })?;
    }
    for v in b.aliases {
        if v.trim().is_empty() || v.contains('\n') {
            return Err(Error::usage(
                format!("invalid --alias {v:?}"),
                "an alias is a short, non-empty, single-line name",
            ));
        }
    }
    Ok(())
}
