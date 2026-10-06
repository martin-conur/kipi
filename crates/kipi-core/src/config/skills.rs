//! `skills/*/SKILL.md` of one store.

use std::fs;
use std::io;
use std::path::Path;

use crate::error::{Error, Location, Result};
use crate::frontmatter::{Frontmatter, SkillMeta};
use crate::model::Skill;

/// The skill file inside each skill directory.
pub const SKILL_FILE: &str = "SKILL.md";

/// Load every skill under `<root>/skills/`, sorted by directory name. Hidden entries
/// and plain files are skipped; a directory without `SKILL.md` is an error.
pub(crate) fn load_skills(root: &Path) -> Result<Vec<Skill>> {
    let dir = root.join("skills");
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Error::io(&dir, e, "check the directory's permissions")),
    };
    let mut dirs = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| Error::io(&dir, e, "check the directory's permissions"))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with('.') && entry.path().is_dir() {
            dirs.push((name, entry.path()));
        }
    }
    dirs.sort();

    let mut skills = Vec::with_capacity(dirs.len());
    for (dir_name, path) in dirs {
        let file = path.join(SKILL_FILE);
        let text = match fs::read_to_string(&file) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(Error::invalid(
                    Location::file(&path),
                    "skill directory has no SKILL.md",
                    "add one (`kipi add skill <name>` writes a template) or remove the directory",
                ));
            }
            Err(e) => return Err(Error::io(&file, e, "check the file's permissions")),
        };
        let fm = Frontmatter::parse(&file, &text)?;
        let meta = SkillMeta::from_frontmatter(&file, &fm)?;
        if meta.name != dir_name {
            return Err(Error::invalid(
                Location::maybe_at(&file, fm.line_of(&["name"])),
                format!(
                    "skill name `{}` does not match its directory `{dir_name}`",
                    meta.name
                ),
                "the Agent Skills format requires `name` to equal the directory name; rename one",
            ));
        }
        skills.push(Skill {
            name: meta.name,
            description: meta.description,
            scope: meta.scope,
            tags: meta.tags,
            dir: path,
            at: Location::file(file),
        });
    }
    Ok(skills)
}
