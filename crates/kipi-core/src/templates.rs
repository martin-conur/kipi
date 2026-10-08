//! Skeletons for files kipi creates, embedded at compile time from the repo's
//! `templates/` directory.

/// `SKILL.md` skeleton used by `kipi add skill`.
pub const SKILL_MD: &str = include_str!("../../../templates/SKILL.md");

/// `notes.md` skeleton used by `kipi edit notes:<repo>`.
pub const NOTES_MD: &str = include_str!("../../../templates/notes.md");

/// `bindings.toml` skeleton used by `kipi init`.
pub const BINDINGS_TOML: &str = include_str!("../../../templates/bindings.toml");

/// minijinja macros shared by the rendering templates (the registry line).
pub const MACROS_MD: &str = include_str!("../../../templates/macros.md.j2");

/// `generated/registry.<scope>.md`, rendered by `render::registry`.
pub const REGISTRY_MD: &str = include_str!("../../../templates/registry.md.j2");

/// The body of the generated block in `CLAUDE.md` / `AGENTS.md`, rendered by `render::block`.
pub const BLOCK_MD: &str = include_str!("../../../templates/block.md.j2");

/// The git `post-checkout` hook installed by `kipi hook install`.
pub const POST_CHECKOUT: &str = include_str!("../../../templates/post-checkout.sh");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_template_has_frontmatter_with_name_and_description() {
        let rest = SKILL_MD
            .strip_prefix("---\n")
            .expect("starts with frontmatter");
        let (front, _body) = rest.split_once("\n---\n").expect("frontmatter is closed");
        assert!(front.lines().any(|l| l.starts_with("name: ")));
        assert!(front.lines().any(|l| l.starts_with("description: ")));
        assert!(front.lines().any(|l| l == "x-kipi:"));
    }

    #[test]
    fn notes_template_has_recommended_sections() {
        for section in ["## Invariants", "## Gotchas", "## Where to look"] {
            assert!(NOTES_MD.contains(section), "missing {section}");
        }
    }

    #[test]
    fn bindings_template_declares_store_and_defaults() {
        assert!(BINDINGS_TOML.contains("[store]"));
        assert!(BINDINGS_TOML.contains("version = 1"));
        assert!(BINDINGS_TOML.contains("[defaults]"));
    }
}
