//! kipi-core: the library behind the `kipi` CLI.
//!
//! No terminal or network dependencies; I/O is limited to the filesystem and the
//! `git` binary. Data flow (roadmap §12): `config::load` → `resolve::resolve` →
//! `Plan` → `apply::run` | `doctor::run` | the CLI's output layer.

pub mod adapters;
pub mod apply;
pub mod config;
pub mod doctor;
pub mod error;
pub mod frontmatter;
pub mod git;
pub mod json_merge;
pub mod manifest;
pub mod model;
pub mod plan;
pub mod render;
pub mod resolve;
pub mod scan;
pub mod search;
pub mod sync;
pub mod templates;
pub mod tokens;
