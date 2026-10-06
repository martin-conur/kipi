//! `kipi add skill`: a new skill from the template, or an imported `SKILL.md` tree (a
//! local directory such as `~/.claude/skills/<name>`, or a git URL cloned to a temp dir).

use std::fs;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use super::toml::{contains, push, table_mut};
use super::{
    Change, LinkBack, Op, Outcome, SkillCopy, absolute, bindings_write, find_scope, find_store,
    revalidate,
};
use crate::config::{Env, SKILL_FILE};
use crate::error::{Error, Location, Result};
use crate::frontmatter::{Frontmatter, MAX_NAME_LEN, SkillMeta, is_skill_name};
use crate::model::{Machine, Skill, Store, StoreSet};
use crate::templates::SKILL_MD;

/// The arguments of `add skill`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SkillOptions<'a> {
    /// The skill's directory name.
    pub name: &'a str,
    /// `--scope`: add it to this scope's `skills`.
    pub scope: Option<&'a str>,
    /// `--store`: the store to create it in, by name.
    pub store: Option<&'a str>,
    /// `--from`: a git URL or a directory holding `SKILL.md`.
    pub from: Option<&'a str>,
    /// `--link-back`: replace the `--from` directory with a symlink into the store.
    pub link_back: bool,
}

/// Whether `--from` names a git URL rather than a directory: `scheme://…` or `user@host:…`.
pub fn is_git_url(from: &str) -> bool {
    from.contains("://")
        || from
            .split_once(':')
            .is_some_and(|(user_host, _)| user_host.contains('@') && !user_host.contains('/'))
}

/// Create skill `opts.name` in its store and, with `--scope`, add it to that scope's
/// `skills` (as `<store>/<name>` when the scope lives in another store, which must share
/// its skills). The skill's frontmatter is validated and the stores re-resolved first.
pub fn add_skill(
    stores: &StoreSet,
    machine: &Machine,
    env: &Env,
    opts: &SkillOptions,
) -> Result<Outcome> {
    let name = opts.name;
    if !is_skill_name(name) {
        return Err(Error::usage(
            format!("invalid skill name `{name}`"),
            format!(
                "use lowercase letters, digits and `-`, at most {MAX_NAME_LEN} characters (the \
                 Agent Skills format)"
            ),
        ));
    }
    let scope = opts.scope.map(|s| find_scope(stores, s)).transpose()?;
    let store = match (opts.store, scope) {
        (Some(n), _) => find_store(stores, n)?,
        (None, Some((owner, _))) => owner,
        (None, None) => match stores.stores() {
            [only] => only,
            _ => {
                return Err(Error::usage(
                    "several stores are mounted; which one gets the skill?",
                    "pass --store <name>, or --scope <scope> to use the store owning it",
                ));
            }
        },
    };
    let dest = store.root.join("skills").join(name);
    if store.skill(name).is_some() || fs::symlink_metadata(&dest).is_ok() {
        return Err(Error::usage(
            format!(
                "skill `{name}` already exists in store `{}` ({})",
                store.name,
                dest.display()
            ),
            "pick another name; kipi never overwrites a skill",
        ));
    }

    let mut out = Outcome {
        store: Some(store.name.clone()),
        ..Outcome::default()
    };
    // The scope's store, when it is the skill's own: `x-kipi.scope` names scopes of the
    // skill's store only.
    let own_scope = scope
        .filter(|(owner, _)| owner.name == store.name)
        .map(|(_, s)| s.name.as_str());
    let (skill, origin) = match opts.from {
        None => {
            if opts.link_back {
                return Err(link_back_needs_a_directory());
            }
            let text = template(name, own_scope);
            let file = dest.join(SKILL_FILE);
            let skill = parse_skill(&file, &text, name, &dest)?;
            out.writes.extend(super::file_write(file, text));
            (skill, "from the template".to_owned())
        }
        Some(from) => {
            let (src, origin) = if is_git_url(from) {
                if opts.link_back {
                    return Err(link_back_needs_a_directory());
                }
                let tmp = tempfile::tempdir()
                    .map_err(|e| Error::io(Path::new("."), e, "check $TMPDIR is writable"))?;
                let checkout = tmp.path().join("repo");
                crate::git::clone(from, &checkout)?;
                let src = find_in_clone(&checkout, name, from)?;
                out.scratch = Some(tmp);
                (src, format!("from {from}"))
            } else {
                let src = absolute(Path::new(from), env)?;
                check_source_dir(&src, opts.link_back)?;
                (src.clone(), format!("from {}", src.display()))
            };
            let file = src.join(SKILL_FILE);
            let text = fs::read_to_string(&file)
                .map_err(|e| Error::io(&file, e, "check the file's permissions"))?;
            let skill = parse_skill(&file, &text, name, &dest)?;
            out.skill = Some(SkillCopy {
                files: tree(&src)?,
                from: src.clone(),
                to: dest.clone(),
            });
            if opts.link_back {
                out.link_back = Some(LinkBack {
                    original: src,
                    target: dest.clone(),
                });
            }
            (skill, origin)
        }
    };
    out.changes.push(Change::new(
        Op::Add,
        "skill",
        format!("{name} in store {} ({origin})", store.name),
    ));

    let mut edited = Vec::new();
    if let Some((owner, scope)) = scope {
        let reference = if owner.name == store.name {
            name.to_owned()
        } else {
            format!("{}/{name}", store.name)
        };
        let what = format!("scopes.{}.skills", scope.name);
        let mut doc = owner.bindings.doc.clone();
        let table = table_mut(&mut doc, &["scopes", &scope.name])
            .ok_or_else(|| not_a_table(owner, &scope.name))?;
        if contains(table, "skills", |v| v == reference) {
            out.changes
                .push(Change::new(Op::Keep, what, reference).note("already there"));
        } else {
            push(table, "skills", &reference);
            out.changes.push(Change::new(Op::Add, what, reference));
            edited.push((owner, doc.to_string()));
        }
    }
    let texts: Vec<(&str, String)> = edited
        .iter()
        .map(|(s, t)| (s.name.as_str(), t.clone()))
        .collect();
    revalidate(stores, machine, env, &texts, Some((&store.name, &skill)))?;
    for (owner, text) in &edited {
        out.writes.extend(bindings_write(owner, text));
    }
    if let Some(link) = &out.link_back {
        out.changes.push(Change::new(
            Op::Add,
            "link-back",
            format!("{} -> {}", link.original.display(), link.target.display()),
        ));
    }
    Ok(out)
}

/// `SKILL.md` from the template; without a scope, `x-kipi.scope` is left out.
fn template(name: &str, scope: Option<&str>) -> String {
    let text = SKILL_MD.replace("{{ name }}", name);
    match scope {
        Some(scope) => text.replace("{{ scope }}", scope),
        None => text
            .lines()
            .filter(|l| !l.contains("{{ scope }}"))
            .map(|l| format!("{l}\n"))
            .collect(),
    }
}

/// Validate `text` (the `SKILL.md` at `file`) as skill `name`, to live in `dest`.
fn parse_skill(file: &Path, text: &str, name: &str, dest: &Path) -> Result<Skill> {
    let fm = Frontmatter::parse(file, text)?;
    let meta = SkillMeta::from_frontmatter(file, &fm)?;
    if meta.name != name {
        return Err(Error::invalid(
            Location::maybe_at(file, fm.line_of(&["name"])),
            format!("the skill's `name` is `{}`, not `{name}`", meta.name),
            format!(
                "the Agent Skills format requires `name` to equal the directory name; run \
                 `kipi add skill {}`, or change `name` in the frontmatter",
                meta.name
            ),
        ));
    }
    Ok(Skill {
        name: meta.name,
        description: meta.description,
        scope: meta.scope,
        tags: meta.tags,
        dir: dest.to_path_buf(),
        at: Location::file(dest.join(SKILL_FILE)),
    })
}

fn check_source_dir(src: &Path, link_back: bool) -> Result<()> {
    let meta = fs::symlink_metadata(src).map_err(|e| {
        Error::usage(
            format!("cannot read {}: {e}", src.display()),
            "pass a directory holding SKILL.md, or a git URL",
        )
    })?;
    if link_back && meta.file_type().is_symlink() {
        return Err(Error::usage(
            format!("{} is already a symlink", src.display()),
            "drop --link-back, or pass the directory it points to",
        ));
    }
    if !src.join(SKILL_FILE).is_file() {
        return Err(Error::usage(
            format!("{} has no {SKILL_FILE}", src.display()),
            "pass the skill's own directory (the one holding SKILL.md)",
        ));
    }
    Ok(())
}

/// The skill directory inside a clone: its root, `skills/<name>/` or `<name>/`.
fn find_in_clone(checkout: &Path, name: &str, url: &str) -> Result<PathBuf> {
    [
        checkout.to_path_buf(),
        checkout.join("skills").join(name),
        checkout.join(name),
    ]
    .into_iter()
    .find(|dir| dir.join(SKILL_FILE).is_file())
    .ok_or_else(|| {
        Error::usage(
            format!("{url} has no {SKILL_FILE} at its root, in skills/{name}/ or in {name}/"),
            "point --from at a repository holding the skill, or clone it and pass the directory",
        )
    })
}

/// Every file and symlink under `dir`, relative and sorted, skipping `.git`.
fn tree(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let walk = WalkDir::new(dir)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| e.file_name() != ".git");
    for entry in walk {
        let entry = entry.map_err(|e| {
            let path = e.path().unwrap_or(dir).to_path_buf();
            Error::io(&path, e.into(), "check the directory's permissions")
        })?;
        if !entry.file_type().is_dir() {
            let rel = entry.path().strip_prefix(dir).unwrap_or(entry.path());
            files.push(rel.to_path_buf());
        }
    }
    Ok(files)
}

fn link_back_needs_a_directory() -> Error {
    Error::usage(
        "--link-back needs --from <directory>",
        "it replaces an imported local skill directory with a symlink into the store",
    )
}

fn not_a_table(store: &Store, scope: &str) -> Error {
    Error::usage(
        format!(
            "cannot edit [scopes.{scope}] in {}",
            store.bindings.path.display()
        ),
        "declare the scope as a `[scopes.<name>]` table",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_urls_are_told_from_paths() {
        for url in [
            "https://example.com/me/skills.git",
            "file:///tmp/skill",
            "git@example.com:me/skill.git",
            "ssh://git@example.com/me/skill",
        ] {
            assert!(is_git_url(url), "{url}");
        }
        for path in [
            "~/.claude/skills/x",
            "/abs/dir",
            "rel/dir",
            "./a:b",
            "/x/y@z:w",
        ] {
            assert!(!is_git_url(path), "{path}");
        }
    }

    #[test]
    fn template_without_scope_has_no_scope_line() {
        let text = template("x", None);
        assert!(!text.contains("scope:"), "{text}");
        assert!(text.contains("name: x\n"));
        assert!(template("x", Some("acme")).contains("  scope: acme\n"));
    }
}
