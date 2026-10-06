//! Rendering of registries and generated blocks (minijinja templates).
//!
//! Everything here is pure text in, text out: the only file reads are the scopes'
//! `always` files, loaded up front by [`block::load_conventions`]. Paths under the home
//! directory are written as `~/…`: shorter for the agent, and no username in the output.

pub mod block;
pub mod registry;
pub mod splice;

use std::path::Path;
use std::sync::LazyLock;

use minijinja::{AutoEscape, Environment, UndefinedBehavior};
use serde::Serialize;

use crate::plan::RegistryEntry;
use crate::templates;

/// What a registry line prints in place of the path of a repo not cloned on this machine.
pub const NOT_CLONED: &str = "(not cloned here)";

/// `path` with a leading `home` replaced by `~`.
pub fn display_path(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// The template environment, built once. Templates are embedded, so a render error is
/// a bug in kipi, not in the user's files.
static ENV: LazyLock<Environment<'static>> = LazyLock::new(|| {
    let mut env = Environment::new();
    env.set_auto_escape_callback(|_| AutoEscape::None);
    env.set_undefined_behavior(UndefinedBehavior::Strict);
    env.set_trim_blocks(true);
    env.set_lstrip_blocks(true);
    env.set_keep_trailing_newline(true);
    for (name, source) in [
        ("macros.md.j2", templates::MACROS_MD),
        ("registry.md.j2", templates::REGISTRY_MD),
        ("block.md.j2", templates::BLOCK_MD),
    ] {
        env.add_template(name, source)
            .unwrap_or_else(|e| panic!("embedded template {name} is invalid: {e}"));
    }
    env
});

/// Render embedded template `name` with `ctx`.
fn render(name: &str, ctx: impl Serialize) -> String {
    ENV.get_template(name)
        .and_then(|t| t.render(ctx))
        .unwrap_or_else(|e| panic!("embedded template {name} failed to render: {e:#}"))
}

/// One repo line, as the `repo_line` macro prints it.
#[derive(Debug, Serialize)]
struct LineView<'a> {
    name: &'a str,
    aliases: &'a [String],
    summary: Option<&'a str>,
    /// The checkout, or [`NOT_CLONED`].
    local: String,
    notes: Option<String>,
}

impl<'a> LineView<'a> {
    fn entry(e: &'a RegistryEntry, home: Option<&Path>) -> Self {
        LineView {
            name: &e.name,
            aliases: &e.aliases,
            summary: e.summary.as_deref(),
            local: local(e.local.as_deref(), home),
            notes: e.notes.as_deref().map(|p| display_path(p, home)),
        }
    }
}

fn local(path: Option<&Path>, home: Option<&Path>) -> String {
    path.map_or_else(|| NOT_CLONED.to_owned(), |p| display_path(p, home))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_paths_start_with_tilde() {
        let home = Some(Path::new("/h/me"));
        assert_eq!(display_path(Path::new("/h/me/work/a"), home), "~/work/a");
        assert_eq!(display_path(Path::new("/h/me"), home), "~");
        assert_eq!(display_path(Path::new("/h/meow"), home), "/h/meow");
        assert_eq!(display_path(Path::new("/srv/x"), None), "/srv/x");
    }

    #[test]
    fn every_embedded_template_compiles() {
        for name in ["macros.md.j2", "registry.md.j2", "block.md.j2"] {
            assert!(ENV.get_template(name).is_ok(), "{name}");
        }
    }
}
