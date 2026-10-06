//! Colors via owo-colors, only when the stream is a terminal and neither `--no-color`
//! nor `NO_COLOR` says otherwise.

use std::fmt::Display;
use std::io::IsTerminal;

use owo_colors::{OwoColorize, Style};

/// Whether `NO_COLOR` asks for no color: set and non-empty (no-color.org).
pub fn no_color_env(var: Option<std::ffi::OsString>) -> bool {
    var.is_some_and(|v| !v.is_empty())
}

/// Paints text for one output stream, or passes it through unchanged.
#[derive(Debug, Clone, Copy)]
pub struct Painter {
    enabled: bool,
}

impl Painter {
    /// Color when `stream` is a terminal and color is not turned off.
    pub fn detect(stream: &impl IsTerminal, no_color: bool) -> Self {
        Painter {
            enabled: !no_color && stream.is_terminal(),
        }
    }

    /// Never color.
    #[cfg_attr(not(test), allow(dead_code))] // for output that is never colored, e.g. files.
    pub fn plain() -> Self {
        Painter { enabled: false }
    }

    /// `text` in `style`, or as is when color is off.
    pub fn paint(&self, text: impl Display, style: Style) -> String {
        if self.enabled {
            text.style(style).to_string()
        } else {
            text.to_string()
        }
    }

    /// `error:`.
    pub fn error(&self, text: impl Display) -> String {
        self.paint(text, Style::new().red().bold())
    }

    /// `hint:`.
    pub fn hint(&self, text: impl Display) -> String {
        self.paint(text, Style::new().cyan().bold())
    }

    /// Section headings.
    pub fn heading(&self, text: impl Display) -> String {
        self.paint(text, Style::new().bold())
    }

    /// Provenance and other secondary text.
    pub fn dim(&self, text: impl Display) -> String {
        self.paint(text, Style::new().dimmed())
    }
}

/// Styles for table cells.
pub mod styles {
    use owo_colors::Style;

    /// No styling.
    pub fn none() -> Style {
        Style::new()
    }

    /// Names (repos, skills, scopes).
    pub fn name() -> Style {
        Style::new().bold()
    }

    /// Provenance.
    pub fn source() -> Style {
        Style::new().dimmed()
    }

    /// Something removed or missing.
    pub fn removed() -> Style {
        Style::new().red()
    }

    /// Something added explicitly.
    pub fn explicit() -> Style {
        Style::new().green()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_color_needs_a_non_empty_value() {
        assert!(!no_color_env(None));
        assert!(!no_color_env(Some("".into())));
        assert!(no_color_env(Some("1".into())));
    }

    #[test]
    fn plain_passes_text_through() {
        assert_eq!(Painter::plain().error("error:"), "error:");
        let on = Painter { enabled: true };
        assert_ne!(on.error("error:"), "error:");
        assert!(on.error("error:").contains("error:"));
    }
}
