//! `kipi grep <pattern> [--scope s] [--repos]`: regex search over the conventions, notes
//! and skills in view and, with `--repos`, the checkouts of the repos in view (roadmap
//! §3 channel 4, §7). Ripgrep-style `path:line: text` lines with absolute paths, capped
//! at `--limit` with a hint on stderr when cut. The view is the scope chain of `--scope`,
//! else of the repo holding the working directory, else every mounted store.

use kipi_core::error::closest;
use kipi_core::search::{self, SearchOptions, View};
use regex::Regex;

use super::{Loaded, cwd_repo};
use crate::error::Failure;
use crate::output::{Output, json};

/// The command's arguments.
pub struct Args<'a> {
    /// The regular expression.
    pub pattern: &'a str,
    /// `--scope`.
    pub scope: Option<&'a str>,
    /// `--repos`.
    pub repos: bool,
    /// `--limit`.
    pub limit: usize,
}

/// Search, then print the hits.
pub fn run(loaded: &Loaded, args: &Args, out: &Output) -> Result<(), Failure> {
    let query = Regex::new(args.pattern).map_err(|e| {
        Failure::user(
            format!("invalid pattern `{}`: {e}", args.pattern),
            "patterns use Rust regex syntax (like ripgrep): escape metacharacters with `\\`, \
             prefix `(?i)` to ignore case",
        )
    })?;
    let view = view(loaded, args.scope)?;
    out.info(&format!("searching the {}", view.label));
    let options = SearchOptions {
        repos: args.repos,
        limit: args.limit,
        ..SearchOptions::default()
    };
    let found = search::search_knowledge(&loaded.stores, &loaded.plan, &view, &query, &options);
    if out.json {
        let text = json::grep(&view, &found).map_err(|e| Failure::json("the matches", e))?;
        return Ok(out.print(&text)?);
    }
    let mut text = String::new();
    for hit in &found.hits {
        text.push_str(&format!(
            "{}:{}: {}\n",
            hit.path.display(),
            hit.line,
            hit.text
        ));
    }
    out.print(&text)?;
    let p = &out.paint_err;
    if found.truncated {
        eprintln!(
            "{} stopped after {} matching lines\n  {} narrow the pattern, pass `--scope`, or raise `--limit`",
            p.hint("note:"),
            args.limit,
            p.hint("hint:")
        );
    } else if found.hits.is_empty() && out.verbosity >= 0 {
        let more = if args.repos {
            ""
        } else {
            "; `--repos` also searches the checkouts"
        };
        eprintln!(
            "{} no match in the {}\n  {} patterns are case-sensitive; prefix `(?i)` to ignore case{more}",
            p.hint("note:"),
            view.label,
            p.hint("hint:")
        );
    }
    Ok(())
}

fn view(loaded: &Loaded, scope: Option<&str>) -> Result<View, Failure> {
    let (stores, plan) = (&loaded.stores, &loaded.plan);
    if let Some(scope) = scope {
        return View::scope(stores, plan, scope).ok_or_else(|| {
            let names = stores
                .stores()
                .iter()
                .flat_map(|s| s.scopes.iter().map(|s| s.name.as_str()));
            let hint = match closest(scope, names) {
                Some(near) => format!("did you mean `{near}`?"),
                None => "`kipi resolve` shows every repo's scope".to_owned(),
            };
            Failure::user(format!("no scope `{scope}` in the mounted stores"), hint)
        });
    }
    Ok(match cwd_repo(loaded) {
        Some(repo) => View::repo(stores, plan, repo),
        None => View::everything(stores),
    })
}
