//! Helpers for `kipi edit` (roadmap §7, §9): the editor command line for a file and
//! line, and the supervision rule that a drafted value a human changed is reviewed.

use std::ffi::OsString;
use std::path::Path;

use toml_edit::{DocumentMut, Item, Value};

use crate::scan::DRAFT_MARK;

/// The command line that opens `file` (at `line`, when the editor has a syntax for it)
/// in `editor`, a command with optional arguments such as `code -w`. Helix and other
/// `file:line` editors get `file:line`, vi-likes `+line file`, VS Code-likes
/// `--goto file:line`; an editor kipi does not know gets the file alone. `None` when
/// `editor` is blank.
pub fn editor_command(editor: &str, file: &Path, line: Option<usize>) -> Option<Vec<OsString>> {
    let mut words = editor.split_whitespace();
    let program = words.next()?;
    let mut argv: Vec<OsString> = std::iter::once(program)
        .chain(words)
        .map(OsString::from)
        .collect();
    let name = Path::new(program)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(program);
    let with_line = |sep: &str| {
        let mut s = file.as_os_str().to_owned();
        if let Some(line) = line {
            s.push(format!("{sep}{line}"));
        }
        s
    };
    match (name, line) {
        ("hx" | "helix" | "subl" | "zed" | "micro", _) => argv.push(with_line(":")),
        ("vi" | "vim" | "nvim" | "nano" | "emacs" | "emacsclient" | "kak" | "mg", Some(l)) => {
            argv.push(format!("+{l}").into());
            argv.push(file.into());
        }
        ("code" | "code-insiders" | "codium" | "cursor", Some(_)) => {
            argv.push("--goto".into());
            argv.push(with_line(":"));
        }
        _ => argv.push(file.into()),
    }
    Some(argv)
}

/// Supervision after an edit (roadmap §9): every `[repos.*]` value that still carries
/// `# kipi:draft` in `after` but differs from its value in `before` was changed by a
/// human, so its marker is removed. Returns the new text and the keys reviewed
/// (`repos.billing-api.summary`), or `None` when either text is not valid TOML (loading
/// reports that).
pub fn review_changed_drafts(before: &str, after: &str) -> Option<(String, Vec<String>)> {
    let old: DocumentMut = before.parse().ok()?;
    let mut new: DocumentMut = after.parse().ok()?;
    let mut reviewed = Vec::new();
    if let Some(repos) = new.get_mut("repos").and_then(Item::as_table_like_mut) {
        for (name, entry) in repos.iter_mut() {
            let Some(entry) = entry.as_table_like_mut() else {
                continue;
            };
            for (key, item) in entry.iter_mut() {
                let Some(value) = item.as_value_mut().filter(|v| is_draft(v)) else {
                    continue;
                };
                let was = old
                    .get("repos")
                    .and_then(|r| r.get(name.get()))
                    .and_then(|r| r.get(key.get()))
                    .and_then(Item::as_value);
                if was.is_some_and(|w| same_value(w, value)) {
                    continue;
                }
                value.decor_mut().set_suffix("");
                reviewed.push(format!("repos.{}.{}", name.get(), key.get()));
            }
        }
    }
    let text = if reviewed.is_empty() {
        after.to_owned()
    } else {
        new.to_string()
    };
    Some((text, reviewed))
}

/// The value's trailing comment is the draft marker.
fn is_draft(value: &Value) -> bool {
    let mark = DRAFT_MARK.trim_start_matches('#').trim();
    value
        .decor()
        .suffix()
        .and_then(|s| s.as_str())
        .is_some_and(|c| {
            c.trim()
                .strip_prefix('#')
                .and_then(|c| c.split_whitespace().next())
                == Some(mark)
        })
}

/// Equal as data: quoting, spacing and comments do not count.
fn same_value(a: &Value, b: &Value) -> bool {
    canonical(a) == canonical(b)
}

fn canonical(value: &Value) -> String {
    match value {
        Value::String(s) => format!("{:?}", s.value()),
        Value::Array(items) => {
            let items: Vec<String> = items.iter().map(canonical).collect();
            format!("[{}]", items.join(","))
        }
        Value::InlineTable(t) => {
            let items: Vec<String> = t
                .iter()
                .map(|(k, v)| format!("{k:?}={}", canonical(v)))
                .collect();
            format!("{{{}}}", items.join(","))
        }
        other => {
            let mut other = other.clone();
            other.decor_mut().clear();
            other.to_string().trim().to_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn argv(editor: &str, line: Option<usize>) -> Vec<String> {
        editor_command(editor, &PathBuf::from("/s/bindings.toml"), line)
            .unwrap()
            .into_iter()
            .map(|a| a.into_string().unwrap())
            .collect()
    }

    #[test]
    fn editors_get_their_line_syntax() {
        assert_eq!(argv("hx", Some(25)), ["hx", "/s/bindings.toml:25"]);
        assert_eq!(
            argv("/usr/bin/nvim", Some(3)),
            ["/usr/bin/nvim", "+3", "/s/bindings.toml"]
        );
        assert_eq!(
            argv("code -w", Some(3)),
            ["code", "-w", "--goto", "/s/bindings.toml:3"]
        );
        assert_eq!(argv("ed", Some(3)), ["ed", "/s/bindings.toml"]);
        assert_eq!(argv("hx", None), ["hx", "/s/bindings.toml"]);
        assert_eq!(argv("vim", None), ["vim", "/s/bindings.toml"]);
        assert!(editor_command("  ", Path::new("x"), None).is_none());
    }

    const BEFORE: &str = "[repos.a]\nsummary = \"old\"  # kipi:draft\n\
                          aliases = [\"x\"]  # kipi:draft\n\n[repos.b]\n\
                          summary = \"keep\"  # kipi:draft\n";

    #[test]
    fn a_changed_draft_is_reviewed_and_the_rest_stay_drafts() {
        let after = BEFORE.replace("\"old\"", "\"new\"");
        let (text, reviewed) = review_changed_drafts(BEFORE, &after).unwrap();
        assert_eq!(reviewed, ["repos.a.summary"]);
        assert_eq!(
            text,
            "[repos.a]\nsummary = \"new\"\naliases = [\"x\"]  # kipi:draft\n\n[repos.b]\n\
             summary = \"keep\"  # kipi:draft\n"
        );
    }

    #[test]
    fn reformatting_is_not_a_change() {
        let after = BEFORE
            .replace("\"old\"", "'old'")
            .replace("[\"x\"]", "[ \"x\", ]");
        let (text, reviewed) = review_changed_drafts(BEFORE, &after).unwrap();
        assert!(reviewed.is_empty());
        assert_eq!(text, after, "untouched");
    }

    #[test]
    fn a_changed_alias_list_is_reviewed() {
        let after = BEFORE.replace("[\"x\"]", "[\"x\", \"y\"]");
        let (_, reviewed) = review_changed_drafts(BEFORE, &after).unwrap();
        assert_eq!(reviewed, ["repos.a.aliases"]);
    }

    #[test]
    fn invalid_toml_is_left_to_loading() {
        assert!(review_changed_drafts(BEFORE, "[repos.a\n").is_none());
    }
}
