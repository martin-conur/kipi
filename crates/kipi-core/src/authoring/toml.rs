//! String-array edits on a toml_edit table that keep the array's own formatting, so that
//! adding values and then removing the same values gives back the original bytes.

use toml_edit::{Array, Item, TableLike, Value};

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

/// Whether the string array `key` of `table` holds a value `same` as `value`.
pub(super) fn contains(table: &dyn TableLike, key: &str, same: impl Fn(&str) -> bool) -> bool {
    table
        .get(key)
        .and_then(Item::as_array)
        .is_some_and(|a| a.iter().any(|v| v.as_str().is_some_and(&same)))
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

/// Remove every element of the string array `key` of `table` matching `matches`;
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
        if !array.get(i).and_then(Value::as_str).is_some_and(&matches) {
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
