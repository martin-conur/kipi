//! String-array edits on a toml_edit table that keep the array's own formatting, so that
//! adding values and then removing the same values gives back the original bytes; a
//! `related` item's `via`, switching it between a string and an inline table in place;
//! and removing a whole table without losing the comments above it.

use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, TableLike, Value};

use crate::scan::DRAFT_MARK;

/// The `[a.b]` table at `keys` of `doc`, if it exists.
pub(super) fn table_mut<'a>(
    doc: &'a mut toml_edit::DocumentMut,
    keys: &[&str],
) -> Option<&'a mut dyn TableLike> {
    let mut table: &mut dyn TableLike = doc.as_table_mut();
    for key in keys {
        table = table.get_mut(key)?.as_table_like_mut()?;
    }
    Some(table)
}

/// The name an array element stands for: a string, or the `repo` of a `related` table.
fn element_name(value: &Value) -> Option<&str> {
    match value {
        Value::InlineTable(t) => t.get("repo").and_then(Value::as_str),
        v => v.as_str(),
    }
}

/// Whether the array `key` of `table` holds an element whose name is `same`.
pub(super) fn contains(table: &dyn TableLike, key: &str, same: impl Fn(&str) -> bool) -> bool {
    table
        .get(key)
        .and_then(Item::as_array)
        .is_some_and(|a| a.iter().any(|v| element_name(v).is_some_and(&same)))
}

/// The `via` of the element named `name` in the array `key` of `table`.
pub(super) fn via(table: &dyn TableLike, key: &str, name: &str) -> Option<String> {
    let array = table.get(key).and_then(Item::as_array)?;
    let element = array.iter().find(|v| element_name(v) == Some(name))?;
    let via = element.as_inline_table()?.get("via")?.as_str()?;
    Some(via.to_owned())
}

/// Set (`Some`) or drop (`None`) the `via` of the element named `name` in the array
/// `key` of `table`. A string becomes `{ repo = "…", via = "…" }` in place, and a table
/// left with only its `repo` becomes a string again; the element keeps its decor (its
/// position, line and comments), so setting and then dropping gives back the original
/// bytes.
pub(super) fn set_via(table: &mut dyn TableLike, key: &str, name: &str, via: Option<&str>) {
    let Some(array) = table.get_mut(key).and_then(Item::as_array_mut) else {
        return;
    };
    let Some(i) = array.iter().position(|v| element_name(v) == Some(name)) else {
        return;
    };
    match (array.get_mut(i), via) {
        (Some(Value::InlineTable(t)), Some(via)) => {
            match t.get_mut("via") {
                Some(old) => {
                    let decor = old.decor().clone();
                    *old = Value::from(via);
                    *old.decor_mut() = decor;
                }
                None => {
                    t.insert("via", Value::from(via));
                    t.fmt();
                }
            }
            return;
        }
        (Some(Value::InlineTable(t)), None) => {
            t.remove("via");
            if t.len() > 1 {
                return;
            }
        }
        (Some(_), None) | (None, _) => return,
        (Some(_), Some(_)) => {}
    }
    let new = match via {
        Some(via) => {
            let mut t = InlineTable::new();
            t.insert("repo", Value::from(name));
            t.insert("via", Value::from(via));
            Value::InlineTable(t)
        }
        None => Value::from(name),
    };
    array.replace(i, new);
}

/// Append `value` to the string array `key` of `table`, creating `key = ["value"]` when
/// absent. A new element copies the decor of the last one, so a multi-line array stays
/// one value per line.
pub(super) fn push(table: &mut dyn TableLike, key: &str, value: &str) {
    let Some(array) = table.get_mut(key).and_then(Item::as_array_mut) else {
        let array: Array = std::iter::once(value).collect();
        table.insert(key, Item::Value(Value::Array(array)));
        return;
    };
    let multiline = array.iter().any(|v| {
        v.decor()
            .prefix()
            .and_then(|p| p.as_str())
            .is_some_and(|p| p.contains('\n'))
    });
    match array.len() {
        0 => array.push(value),
        1 if !multiline => array.push(value),
        n => {
            let mut new = Value::from(value);
            let decor = array
                .get(n - 1)
                .map(|v| v.decor().clone())
                .unwrap_or_default();
            *new.decor_mut() = decor;
            if new
                .decor()
                .suffix()
                .and_then(|s| s.as_str())
                .is_some_and(|s| s.contains('#'))
            {
                new.decor_mut().set_suffix("");
            }
            array.push_formatted(new);
        }
    }
}

/// Remove every element of the array `key` of `table` whose name matches `matches`;
/// returns how many were removed. A key left empty is removed with its line.
pub(super) fn remove(
    table: &mut dyn TableLike,
    key: &str,
    matches: impl Fn(&str) -> bool,
) -> usize {
    let Some(array) = table.get_mut(key).and_then(Item::as_array_mut) else {
        return 0;
    };
    let (mut i, mut removed) = (0, 0);
    while i < array.len() {
        if !array.get(i).and_then(element_name).is_some_and(&matches) {
            i += 1;
            continue;
        }
        let gone = array.remove(i);
        removed += 1;
        // The new first element takes the old first's leading whitespace (`["a", "b"]`
        // minus `a` is `["b"]`, not `[ "b"]`).
        if i == 0
            && let Some(first) = array.get_mut(0)
        {
            let prefix = gone.decor().prefix().cloned().unwrap_or_default();
            first.decor_mut().set_prefix(prefix);
        }
    }
    if array.is_empty() {
        table.remove(key);
    }
    removed
}

/// Drop a trailing `# kipi:draft` comment from the value at `key` (a human edited it).
pub(super) fn clear_draft(table: &mut dyn TableLike, key: &str) {
    let Some(value) = table.get_mut(key).and_then(Item::as_value_mut) else {
        return;
    };
    let mark = DRAFT_MARK.trim_start_matches("# ");
    let drafted = value
        .decor()
        .suffix()
        .and_then(|s| s.as_str())
        .is_some_and(|s| {
            s.trim()
                .strip_prefix('#')
                .and_then(|c| c.split_whitespace().next())
                == Some(mark)
        });
    if drafted {
        value.decor_mut().set_suffix("");
    }
}

/// Remove the table `[<parent>.<name>]` from `doc`; returns whether it was there. Its own
/// lines go (header comment and body included), but a comment above its header may be
/// about the section rather than the entry, so it moves to the next table (or the end of
/// the document): removing what `add repo` appended gives back the original bytes.
pub(super) fn remove_table(doc: &mut DocumentMut, parent: &str, name: &str) -> bool {
    let Some(gone) = doc
        .get_mut(parent)
        .and_then(Item::as_table_like_mut)
        .and_then(|t| t.remove(name))
    else {
        return false;
    };
    let Some(table) = gone.as_table() else {
        return true;
    };
    let prefix = table
        .decor()
        .prefix()
        .and_then(|p| p.as_str())
        .unwrap_or("");
    if !prefix.contains('#') {
        return true;
    }
    let prefix = prefix.to_owned();
    let next = table.position().and_then(|pos| {
        let mut found = Vec::new();
        positions(doc.as_table(), &mut Vec::new(), &mut found);
        found.into_iter().filter(|(p, _)| *p > pos).min()
    });
    match next.and_then(|(_, keys)| table_at(doc, &keys)) {
        Some(next) => {
            // The moved comment already carries the blank line before it, so the next
            // table's own blank lines go; its own comment keeps them, so the two comment
            // blocks stay apart.
            let own = next.decor().prefix().and_then(|p| p.as_str()).unwrap_or("");
            let own = if own.contains('#') { own } else { "" };
            let joined = format!("{prefix}{own}");
            next.decor_mut().set_prefix(joined);
        }
        None => {
            let trailing = doc.trailing().as_str().unwrap_or("").to_owned();
            doc.set_trailing(format!("{prefix}{trailing}"));
        }
    }
    true
}

/// The position and key path of every table under `table` that has a header.
fn positions(table: &Table, keys: &mut Vec<String>, out: &mut Vec<(isize, Vec<String>)>) {
    for (key, item) in table.iter() {
        let Some(t) = item.as_table() else { continue };
        keys.push(key.to_owned());
        if !t.is_implicit()
            && let Some(p) = t.position()
        {
            out.push((p, keys.clone()));
        }
        positions(t, keys, out);
        keys.pop();
    }
}

/// The table at `keys` of `doc`.
fn table_at<'a>(doc: &'a mut DocumentMut, keys: &[String]) -> Option<&'a mut Table> {
    let mut table = doc.as_table_mut();
    for key in keys {
        table = table.get_mut(key)?.as_table_mut()?;
    }
    Some(table)
}

#[cfg(test)]
mod tests {
    use toml_edit::DocumentMut;

    use super::*;

    fn edit(text: &str, f: impl FnOnce(&mut dyn TableLike)) -> String {
        let mut doc: DocumentMut = text.parse().unwrap();
        f(table_mut(&mut doc, &["t"]).unwrap());
        doc.to_string()
    }

    #[test]
    fn push_then_remove_round_trips() {
        for text in [
            "[t]\nx = 1\n",
            "[t]\na = [\"p\"]  # keep\n",
            "[t]\na = [\"p\", \"q\"]\n\n[u]\n",
            "[t]\na = [ \"p\" ]\n",
            "[t]\na = [\n  \"p\",\n  \"q\",\n]\n",
            "[t]\na = [\n  \"p\",\n]\nb = 2 # after\n",
            "[t]\na = []\n# trailing comment\n",
        ] {
            let pushed = edit(text, |t| push(t, "a", "new"));
            assert_ne!(pushed, text);
            let back = edit(&pushed, |t| {
                remove(t, "a", |v| v == "new");
            });
            // An empty array that gets a value and loses it again is removed.
            let expected = text.replace("a = []\n", "");
            assert_eq!(back, expected, "{pushed}");
        }
    }

    #[test]
    fn push_formats_like_the_array() {
        assert_eq!(
            edit("[t]\nb = 1\n", |t| push(t, "a", "x")),
            "[t]\nb = 1\na = [\"x\"]\n"
        );
        assert_eq!(
            edit("[t]\na = [\"p\", \"q\"]\n", |t| push(t, "a", "x")),
            "[t]\na = [\"p\", \"q\", \"x\"]\n"
        );
        assert_eq!(
            edit("[t]\na = [\n  \"p\",\n]\n", |t| push(t, "a", "x")),
            "[t]\na = [\n  \"p\",\n  \"x\",\n]\n"
        );
    }

    #[test]
    fn remove_keeps_the_rest_tidy() {
        assert_eq!(
            edit("[t]\na = [\"p\", \"q\"]\n", |t| {
                assert_eq!(remove(t, "a", |v| v == "p"), 1);
            }),
            "[t]\na = [\"q\"]\n"
        );
        assert_eq!(
            edit("[t]\na = [\"p\"]\nb = 1\n", |t| {
                remove(t, "a", |v| v == "p");
            }),
            "[t]\nb = 1\n"
        );
        assert_eq!(
            edit("[t]\nb = 1\n", |t| {
                assert_eq!(remove(t, "a", |_| true), 0);
            }),
            "[t]\nb = 1\n"
        );
    }

    #[test]
    fn remove_table_keeps_the_comments_above_it() {
        let remove = |text: &str, name: &str| {
            let mut doc: DocumentMut = text.parse().unwrap();
            assert!(remove_table(&mut doc, "repos", name));
            doc.to_string()
        };
        let text = "[store]\nname = \"s\"\n\n# the repos\n[repos.a]  # mine\nx = 1\n\n\
                    [repos.b]\ny = 2\n";
        assert_eq!(
            remove(text, "a"),
            "[store]\nname = \"s\"\n\n# the repos\n[repos.b]\ny = 2\n"
        );
        assert_eq!(
            remove(text, "b"),
            "[store]\nname = \"s\"\n\n# the repos\n[repos.a]  # mine\nx = 1\n"
        );
        assert_eq!(
            remove("[repos.a]\nx = 1\n\n# last\n[repos.b]\ny = 2\n# end\n", "b"),
            "[repos.a]\nx = 1\n\n# last\n# end\n"
        );
        let mut doc: DocumentMut = "[repos.a]\n".parse().unwrap();
        assert!(!remove_table(&mut doc, "repos", "z"));
    }

    #[test]
    fn remove_table_keeps_sections_apart() {
        let remove = |text: &str| {
            let mut doc: DocumentMut = text.parse().unwrap();
            assert!(remove_table(&mut doc, "repos", "a"));
            doc.to_string()
        };
        // The next table's prefix is only blank lines: the moved comment block keeps its
        // blank line above and sits right on the next header, as it sat on the removed one.
        assert_eq!(
            remove(
                "[store]\nname = \"s\"\n\n# the repos\n# (by hand)\n[repos.a]\nx = 1\n\n\n[repos.b]\ny = 2\n"
            ),
            "[store]\nname = \"s\"\n\n# the repos\n# (by hand)\n[repos.b]\ny = 2\n"
        );
        // The next table has its own comment: both blocks keep their spacing.
        assert_eq!(
            remove(
                "[store]\nname = \"s\"\n\n# the repos\n[repos.a]\nx = 1\n\n# b, mostly\n[repos.b]\ny = 2\n"
            ),
            "[store]\nname = \"s\"\n\n# the repos\n\n# b, mostly\n[repos.b]\ny = 2\n"
        );
    }

    #[test]
    fn via_turns_a_string_into_a_table_and_back() {
        for text in [
            "[t]\na = [\"p\"]\n",
            "[t]\na = [\"p\", \"q\"]  # keep\n",
            "[t]\na = [ \"q\", \"p\" ]\n",
            "[t]\na = [\n  \"q\",\n  # about p\n  \"p\", # p\n]\n",
        ] {
            let set = edit(text, |t| set_via(t, "a", "p", Some("its tables")));
            assert!(
                set.contains("{ repo = \"p\", via = \"its tables\" }"),
                "{set}"
            );
            let replaced = edit(&set, |t| set_via(t, "a", "p", Some("its API")));
            assert_eq!(replaced, set.replace("its tables", "its API"));
            let mut doc: DocumentMut = replaced.parse().unwrap();
            let table = table_mut(&mut doc, &["t"]).unwrap();
            assert!(contains(table, "a", |v| v == "p"));
            assert_eq!(via(table, "a", "p").as_deref(), Some("its API"));
            assert_eq!(edit(&replaced, |t| set_via(t, "a", "p", None)), text);
            assert_eq!(
                edit(&replaced, |t| assert_eq!(remove(t, "a", |v| v == "p"), 1)),
                edit(text, |t| {
                    remove(t, "a", |v| v == "p");
                })
            );
        }
        // A hand-written table keeps its spacing and its other keys' order.
        assert_eq!(
            edit("[t]\na = [{repo = \"p\", via = \"x\"}]\n", |t| set_via(
                t,
                "a",
                "p",
                Some("y")
            )),
            "[t]\na = [{repo = \"p\", via = \"y\"}]\n"
        );
        assert_eq!(
            edit("[t]\na = [{ repo = \"p\" }]\n", |t| set_via(
                t,
                "a",
                "p",
                Some("y")
            )),
            "[t]\na = [{ repo = \"p\", via = \"y\" }]\n"
        );
    }

    #[test]
    fn draft_marker_is_cleared() {
        assert_eq!(
            edit("[t]\na = [\"p\"]  # kipi:draft\n", |t| clear_draft(t, "a")),
            "[t]\na = [\"p\"]\n"
        );
        assert_eq!(
            edit("[t]\na = [\"p\"]  # mine\n", |t| clear_draft(t, "a")),
            "[t]\na = [\"p\"]  # mine\n"
        );
    }
}
