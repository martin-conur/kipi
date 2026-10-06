//! Deterministic, offline drafting of a repo's `summary` and `aliases` (roadmap §9).
//!
//! Summary: the manifest description (`Cargo.toml`, `pyproject.toml`, `package.json`,
//! `go.mod`'s leading comment), else the first prose paragraph of the README with
//! badges, headings, HTML and links stripped, cut to ~140 chars on a word boundary;
//! then key-file hints ("OpenAPI in api/openapi.yaml", "dbt project") are appended.
//! Aliases: the name split on `-` / `_` / `.`, generic words dropped, deduplicated
//! against names and aliases other repos already use.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

/// Drafted summaries are cut to about this many characters.
pub const SUMMARY_CHARS: usize = 140;

/// Name parts too generic to be a useful alias on their own.
pub const GENERIC_WORDS: &[&str] = &[
    "api", "apis", "app", "apps", "backend", "cli", "client", "core", "frontend", "go", "js",
    "lib", "libs", "py", "rs", "sdk", "server", "service", "services", "svc", "the", "tool",
    "tools", "ts", "util", "utils", "web", "www",
];

/// README file names, in preference order (matched case-insensitively).
const READMES: &[&str] = &["readme.md", "readme.markdown", "readme", "readme.txt"];

/// Files whose presence is worth a hint, with the hint text. A hint is skipped when
/// the summary already mentions its needle.
const KEY_FILES: &[(&str, &str, &str)] = &[
    ("api/openapi.yaml", "OpenAPI in api/openapi.yaml", "openapi"),
    ("api/openapi.yml", "OpenAPI in api/openapi.yml", "openapi"),
    ("api/openapi.json", "OpenAPI in api/openapi.json", "openapi"),
    ("openapi.yaml", "OpenAPI in openapi.yaml", "openapi"),
    ("openapi.yml", "OpenAPI in openapi.yml", "openapi"),
    ("openapi.json", "OpenAPI in openapi.json", "openapi"),
    ("dbt_project.yml", "dbt project", "dbt"),
];

/// The drafted summary of the checkout at `repo`, if anything describes it.
pub fn draft_summary(repo: &Path) -> Option<String> {
    let base = manifest_description(repo).or_else(|| readme_summary(repo));
    let mut parts: Vec<String> = base.into_iter().collect();
    let mut needles: Vec<&str> = Vec::new();
    for &(file, hint, needle) in KEY_FILES {
        let mentioned = parts.iter().any(|p| p.to_lowercase().contains(needle));
        if !mentioned && !needles.contains(&needle) && repo.join(file).is_file() {
            parts.push(hint.to_owned());
            needles.push(needle);
        }
    }
    (!parts.is_empty()).then(|| parts.join("; "))
}

/// Aliases drafted from the entry `name`: its parts split on `-`, `_` and `.`,
/// lowercased, without generic words, one-letter, numeric or version (`v2`) parts, and any key in
/// `taken` (names and aliases of other repos). A one-part name gets none, since the
/// name itself already resolves.
pub fn draft_aliases(name: &str, taken: &BTreeSet<String>) -> Vec<String> {
    let parts: Vec<String> = name
        .split(['-', '_', '.'])
        .filter(|p| !p.is_empty())
        .map(str::to_lowercase)
        .collect();
    if parts.len() < 2 {
        return Vec::new();
    }
    let mut out: Vec<String> = Vec::new();
    for part in parts {
        let useless = part.chars().count() < 2
            || part
                .trim_start_matches('v')
                .chars()
                .all(|c| c.is_ascii_digit())
            || GENERIC_WORDS.contains(&part.as_str());
        if !useless && part != name.to_lowercase() && !taken.contains(&part) && !out.contains(&part)
        {
            out.push(part);
        }
    }
    out
}

/// The first manifest description found: `Cargo.toml` `package.description`,
/// `pyproject.toml` `project.description` (or `tool.poetry.description`),
/// `package.json` `description`, then the comment block right above `go.mod`'s `module`.
pub fn manifest_description(repo: &Path) -> Option<String> {
    let toml_str = |file: &str, keys: &[&[&str]]| -> Option<String> {
        let doc: toml_edit::DocumentMut = read(&repo.join(file))?.parse().ok()?;
        keys.iter().find_map(|path| {
            let mut item = doc.as_item();
            for key in *path {
                item = item.get(key)?;
            }
            item.as_str().map(str::to_owned)
        })
    };
    let json = || -> Option<String> {
        let v: serde_json::Value = serde_json::from_str(&read(&repo.join("package.json"))?).ok()?;
        v.get("description")?.as_str().map(str::to_owned)
    };
    let go = || -> Option<String> {
        let text = read(&repo.join("go.mod"))?;
        let mut comment: Vec<&str> = Vec::new();
        for line in text.lines().map(str::trim) {
            if let Some(c) = line.strip_prefix("//") {
                comment.push(c.trim());
            } else if line.starts_with("module ") || line == "module" {
                break;
            } else {
                comment.clear();
            }
        }
        (!comment.is_empty()).then(|| comment.join(" "))
    };
    [
        toml_str("Cargo.toml", &[&["package", "description"]]),
        toml_str(
            "pyproject.toml",
            &[
                &["project", "description"],
                &["tool", "poetry", "description"],
            ],
        ),
        json(),
        go(),
    ]
    .into_iter()
    .flatten()
    .find_map(|d| tidy(&collapse(&d)))
}

/// The first prose paragraph of the checkout's README, cleaned and cut.
pub fn readme_summary(repo: &Path) -> Option<String> {
    let mut files: Vec<_> = fs::read_dir(repo)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    files.sort();
    let file = READMES.iter().find_map(|want| {
        files
            .iter()
            .find(|f| f.to_lowercase() == *want)
            .map(|f| repo.join(f))
    })?;
    first_paragraph(&read(&file)?)
}

/// The first paragraph of Markdown `text` that is prose once badges, headings, HTML,
/// links, code blocks, lists and tables are removed; cleaned and cut to ~140 chars.
pub fn first_paragraph(text: &str) -> Option<String> {
    let text = HTML_COMMENT.replace_all(text, "");
    let mut paragraphs: Vec<Vec<&str>> = vec![Vec::new()];
    let mut fence: Option<&str> = None;
    let mut prev: Option<&str> = None;
    for line in text.lines() {
        let t = line.trim();
        if let Some(f) = fence {
            if t.starts_with(f) {
                fence = None;
            }
            continue;
        }
        if let Some(f) = ["```", "~~~"].into_iter().find(|f| t.starts_with(f)) {
            fence = Some(f);
            paragraphs.push(Vec::new());
            continue;
        }
        let setext = !t.is_empty() && (t.chars().all(|c| c == '=') || t.chars().all(|c| c == '-'));
        let current = paragraphs.last_mut().expect("never empty");
        if setext && prev.is_some_and(|p| !p.trim().is_empty()) {
            // `Title\n=====`: the previous line was a heading.
            current.pop();
        } else if t.is_empty() || t.starts_with('#') || REF_DEF.is_match(t) {
            paragraphs.push(Vec::new());
        } else {
            current.push(t);
        }
        prev = Some(line);
    }
    paragraphs
        .into_iter()
        .filter(|p| !p.is_empty() && !is_list_or_table(p[0]))
        .find_map(|p| {
            let cleaned = clean_inline(&p.join(" "));
            let prose =
                cleaned.split_whitespace().count() >= 2 && cleaned.chars().any(char::is_alphabetic);
            if prose { tidy(&cleaned) } else { None }
        })
}

/// `s` on one line, cut to ~[`SUMMARY_CHARS`] on a word boundary (with `…`), and
/// without a final period. `None` when nothing is left.
pub fn tidy(s: &str) -> Option<String> {
    let s = collapse(s);
    let out = if s.chars().count() <= SUMMARY_CHARS {
        s.strip_suffix('.')
            .filter(|r| !r.ends_with('.'))
            .unwrap_or(&s)
            .trim_end()
            .to_owned()
    } else {
        let head: String = s.chars().take(SUMMARY_CHARS).collect();
        let cut = match head.rfind(char::is_whitespace) {
            Some(i) if i > 0 => &head[..i],
            _ => head.as_str(),
        };
        let cut = cut.trim_end_matches(|c: char| c.is_whitespace() || ",;:-—(".contains(c));
        format!("{cut}…")
    };
    (!out.trim().is_empty()).then_some(out)
}

fn is_list_or_table(first: &str) -> bool {
    first.starts_with(['|', '-', '*', '+']) || LIST_NUMBER.is_match(first)
}

/// Remove images, headings in HTML, tags and bare URLs; keep link and code text.
fn clean_inline(s: &str) -> String {
    let s = HTML_HEADING.replace_all(s, "");
    let s = IMAGE.replace_all(&s, "");
    let s = LINK.replace_all(&s, "$1");
    let s = TAG.replace_all(&s, "");
    let s = URL.replace_all(&s, "");
    let s = s
        .replace("**", "")
        .replace("__", "")
        .replace(['`', '*'], "")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"");
    let s = s.trim_start_matches(['>', ' ']);
    let s = collapse(s);
    // Separators orphaned by removed badges or links: "| |", "· ·".
    s.trim_matches(|c: char| c.is_whitespace() || "|·•-".contains(c))
        .to_owned()
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn read(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok()
}

macro_rules! regex {
    ($name:ident, $re:literal) => {
        static $name: LazyLock<Regex> =
            LazyLock::new(|| Regex::new($re).expect("a valid regex literal"));
    };
}

regex!(HTML_COMMENT, r"(?s)<!--.*?-->");
regex!(HTML_HEADING, r"(?is)<h[1-6][^>]*>.*?</h[1-6]>");
regex!(IMAGE, r"!\[[^\]]*\](?:\([^)]*\)|\[[^\]]*\])");
regex!(LINK, r"\[([^\]]*)\](?:\([^)]*\)|\[[^\]]*\])");
regex!(TAG, r"</?[A-Za-z][^>]*>");
regex!(URL, r"https?://\S+");
regex!(REF_DEF, r"^\[[^\]]+\]:\s*\S+");
regex!(LIST_NUMBER, r"^\d+[.)]\s");

#[cfg(test)]
mod tests {
    use super::*;

    fn taken(keys: &[&str]) -> BTreeSet<String> {
        keys.iter().map(|k| (*k).to_owned()).collect()
    }

    #[test]
    fn aliases_split_drop_generic_and_dedupe() {
        let none = taken(&[]);
        assert_eq!(
            draft_aliases("events-pipeline", &none),
            ["events", "pipeline"]
        );
        assert_eq!(draft_aliases("billing-api", &none), ["billing"]);
        assert_eq!(draft_aliases("web_dashboard", &none), ["dashboard"]);
        assert_eq!(draft_aliases("ml-platform", &none), ["ml", "platform"]);
        assert_eq!(draft_aliases("Side-Tool", &none), ["side"]);
        assert!(
            draft_aliases("blog", &none).is_empty(),
            "one part: the name"
        );
        assert!(draft_aliases("api-v2", &none).is_empty());
        assert!(draft_aliases("x-2", &none).is_empty());
        assert_eq!(draft_aliases("data-data-lake", &none), ["data", "lake"]);
        assert_eq!(
            draft_aliases("events-store", &taken(&["events"])),
            ["store"],
            "another repo already answers to `events`"
        );
    }

    #[test]
    fn go_mod_without_a_leading_comment_has_no_description() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("go.mod"),
            "// unrelated\n\nmodule example.com/m\n",
        )
        .unwrap();
        assert_eq!(manifest_description(tmp.path()), None);
        fs::write(
            tmp.path().join("go.mod"),
            "// Does things.\nmodule example.com/m\n",
        )
        .unwrap();
        assert_eq!(
            manifest_description(tmp.path()).as_deref(),
            Some("Does things")
        );
    }

    #[test]
    fn tidy_cuts_on_a_word_boundary() {
        assert_eq!(tidy("  One line.  ").as_deref(), Some("One line"));
        assert_eq!(
            tidy("Ends with ellipsis...").as_deref(),
            Some("Ends with ellipsis...")
        );
        assert_eq!(tidy("   "), None);
        let long = "word ".repeat(40);
        let cut = tidy(&long).unwrap();
        assert!(cut.ends_with("word…"), "{cut}");
        assert!(cut.chars().count() <= SUMMARY_CHARS + 1);
        let unbroken = "x".repeat(200);
        assert_eq!(tidy(&unbroken).unwrap().chars().count(), SUMMARY_CHARS + 1);
        let comma = format!("{}, and more words follow here", "a".repeat(135));
        assert_eq!(tidy(&comma).unwrap(), format!("{}…", "a".repeat(135)));
    }

    #[test]
    fn readme_badges_headings_and_links_are_stripped() {
        let text = "# billing-api\n\n\
            [![CI](https://ci.example.com/badge.svg)](https://ci.example.com) \
            ![license](https://img.example.com/l.svg)\n\n\
            Handles **invoices** via the [payments API](https://example.com/docs) \
            and `webhooks`. See <https://example.com>.\n\nSecond paragraph.\n";
        assert_eq!(
            first_paragraph(text).as_deref(),
            Some("Handles invoices via the payments API and webhooks. See")
        );
    }

    #[test]
    fn readme_skips_html_code_lists_and_setext_headings() {
        let text = "<p align=\"center\">\n  <img src=\"logo.png\">\n</p>\n\
            <h1 align=\"center\">Tool</h1>\n\n\
            Title\n=====\n\n\
            ```sh\ncargo install tool\n```\n\n\
            - a list item\n- another\n\n\
            | a | b |\n|---|---|\n\n\
            1. step one\n\n\
            > A quoted tagline that is prose.\n";
        assert_eq!(
            first_paragraph(text).as_deref(),
            Some("A quoted tagline that is prose")
        );
        assert_eq!(first_paragraph("# Only\n\n[![b](x)](y)\n"), None);
        // CommonMark: a lone `-` or `=` right under a line underlines it as a heading;
        // after a blank line it is not an underline (and not prose either).
        assert_eq!(
            first_paragraph("A heading by setext\n-\n\nThe actual prose.").as_deref(),
            Some("The actual prose")
        );
        assert_eq!(
            first_paragraph("Another heading\n=\n\nMore prose here.").as_deref(),
            Some("More prose here")
        );
        assert_eq!(
            first_paragraph("Prose before a rule.\n\n-\n\nAfter.").as_deref(),
            Some("Prose before a rule")
        );
        assert_eq!(first_paragraph(""), None);
        assert_eq!(
            first_paragraph("<!-- a\ncomment -->\nReal text here.").as_deref(),
            Some("Real text here")
        );
    }
}
