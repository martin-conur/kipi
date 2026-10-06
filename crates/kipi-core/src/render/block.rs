//! The marker-delimited generated block for `CLAUDE.md` / `AGENTS.md` (roadmap §8):
//! conventions per scope top-down, the known repos, then the this-repo section.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::registry::RegistryView;
use super::{LineView, display_path, local};
use crate::error::{Error, Location, Result};
use crate::plan::{AlwaysOn, Plan, RepoPlan};

/// The line opening the generated block.
pub const BEGIN: &str = "<!-- kipi:begin (do not edit; regenerate with `kipi sync`) -->";
/// The line closing the generated block.
pub const END: &str = "<!-- kipi:end -->";
/// What every begin-marker line starts with; the rest of the line is free text.
pub(crate) const BEGIN_PREFIX: &str = "<!-- kipi:begin";

/// Whether `line` (without its terminator) is a begin marker.
pub(crate) fn is_begin(line: &str) -> bool {
    line.starts_with(BEGIN_PREFIX)
}

/// Whether `line` (without its terminator) is an end marker.
pub(crate) fn is_end(line: &str) -> bool {
    line.trim_end() == END
}

/// One scope's `always` file, read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conventions {
    /// The scope listing the file.
    pub scope: String,
    /// The file.
    pub path: PathBuf,
    /// Its contents.
    pub text: String,
}

/// Read every `always` file `repo` renders, in render order (top-most scope first).
///
/// A file containing a kipi marker line is an error: it would end the block early.
pub fn load_conventions(repo: &RepoPlan) -> Result<Vec<Conventions>> {
    let mut out = Vec::new();
    for always in &repo.always {
        let AlwaysOn::File { path, scope, .. } = always else {
            continue;
        };
        let text = fs::read_to_string(path).map_err(|e| {
            Error::io(
                path,
                e,
                format!("create the file or remove it from `scopes.{scope}.always`"),
            )
        })?;
        if let Some(i) = text.lines().position(|l| is_begin(l) || is_end(l)) {
            return Err(Error::invalid(
                Location::at(path, i + 1),
                "conventions contain a kipi marker line",
                "remove the `<!-- kipi:… -->` line; it would cut the generated block short",
            ));
        }
        out.push(Conventions {
            scope: scope.clone(),
            path: path.clone(),
            text,
        });
    }
    Ok(out)
}

/// A rendered generated block. [`RenderedBlock::text`] is what goes between (and
/// including) the markers; adapters write it, [`super::splice::splice`] places it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedBlock {
    body: String,
}

impl RenderedBlock {
    /// The content between the markers, ending with a newline.
    pub fn body(&self) -> &str {
        &self.body
    }

    /// The whole block, from the begin marker through the end marker (no trailing newline).
    pub fn text(&self) -> String {
        format!("{BEGIN}\n{}{END}", self.body)
    }
}

#[derive(Debug, Serialize)]
struct BlockView<'a> {
    conventions: Vec<ConventionsView<'a>>,
    registry: Option<RegistryView<'a>>,
    scope: Option<&'a str>,
    related: Vec<RelatedView<'a>>,
    /// Set when every related repo has the same status, printed once after the list.
    related_status: Option<&'static str>,
    /// Related repos the known-repos list does not show, as their own lines.
    unlisted: Vec<LineView<'a>>,
}

#[derive(Debug, Serialize)]
struct ConventionsView<'a> {
    scope: &'a str,
    text: String,
}

#[derive(Debug, Serialize)]
struct RelatedView<'a> {
    name: &'a str,
    status: &'static str,
}

/// Render `repo`'s generated block. `conventions` comes from [`load_conventions`];
/// `home` is shown as `~`.
pub fn render(
    plan: &Plan,
    repo: &RepoPlan,
    conventions: &[Conventions],
    home: Option<&Path>,
) -> RenderedBlock {
    // Consecutive files of one scope share a heading.
    let mut grouped: Vec<ConventionsView> = Vec::new();
    for c in conventions {
        let text = c.text.trim_end();
        if text.is_empty() {
            continue;
        }
        match grouped.last_mut() {
            Some(last) if last.scope == c.scope => {
                last.text.push_str("\n\n");
                last.text.push_str(text);
            }
            _ => grouped.push(ConventionsView {
                scope: &c.scope,
                text: text.to_owned(),
            }),
        }
    }

    let registry = plan.registry_of(repo);
    let listed = |name: &str| registry.is_some_and(|r| r.entries.iter().any(|e| e.name == name));
    let related: Vec<RelatedView> = repo
        .related
        .iter()
        .map(|r| RelatedView {
            name: &r.name,
            status: match &r.path {
                None => "not cloned here",
                Some(p) if repo.grant.contains(p) => "read access granted",
                Some(_) => "read access not granted",
            },
        })
        .collect();
    let related_status = match related.split_first() {
        Some((first, rest)) if rest.iter().all(|r| r.status == first.status) => Some(first.status),
        _ => None,
    };
    let unlisted = repo
        .related
        .iter()
        .filter(|r| !listed(&r.name))
        .map(|r| LineView {
            name: &r.name,
            aliases: &[],
            summary: None,
            local: local(r.path.as_deref(), home),
            notes: r.notes.as_deref().map(|p| display_path(p, home)),
        })
        .collect();

    let view = BlockView {
        conventions: grouped,
        registry: registry.map(|r| RegistryView::new(r, home)),
        scope: repo.scope.value.as_deref(),
        related,
        related_status,
        unlisted,
    };
    RenderedBlock {
        body: super::render("block.md.j2", view),
    }
}
