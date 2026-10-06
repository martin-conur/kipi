//! Output formatting: color, JSON, tables, and how paths are shown.

pub mod color;
pub mod diff;
pub mod json;
pub mod status;
pub mod table;
pub mod tree;

use std::io::{self, Write};
use std::path::Path;

use color::Painter;
pub use kipi_core::render::display_path;

/// How a command prints: format, colors and verbosity, from the global flags.
#[derive(Debug, Clone)]
pub struct Output {
    /// `--json`.
    pub json: bool,
    /// Colors for stdout.
    pub paint: Painter,
    /// Colors for stderr.
    pub paint_err: Painter,
    /// `-v` count; `-1` for `-q`.
    pub verbosity: i8,
    /// The home directory, shown as `~` in human output.
    pub home: Option<std::path::PathBuf>,
}

impl Output {
    /// Write `text` to stdout.
    pub fn print(&self, text: &str) -> io::Result<()> {
        let mut out = io::stdout().lock();
        out.write_all(text.as_bytes())?;
        out.flush()
    }

    /// A progress note on stderr, shown with `-v`.
    pub fn info(&self, text: &str) {
        if self.verbosity > 0 {
            eprintln!("{}", self.paint_err.dim(text));
        }
    }

    /// `path` for humans: under the home directory it starts with `~/`.
    pub fn path(&self, path: &Path) -> String {
        display_path(path, self.home.as_deref())
    }
}
