//! `--attr` on `kipi bind`, `kipi unbind` and `kipi scope`: set or remove entries of a
//! repo's or scope's `attributes` table. A key is found however it is written (a quoted
//! `"tracker.project"`, nested `tracker.project`, or a `[….attributes.tracker]` table), so
//! an edit changes the line that is there instead of adding a second spelling; a new key
//! is written quoted into `attributes = { … }`.

use toml_edit::{InlineTable, Item, TableLike, Value};

use super::{Change, Op};
use crate::error::{Error, Result};
use crate::model::{check_attribute_key, check_attribute_value};

/// The table key.
const KEY: &str = "attributes";

/// Parse each `--attr key=value` of `bind` / `kipi scope` (a later value for the same key
/// wins). `command` is the example the hint shows.
pub(super) fn parse_sets(
    flag: &str,
    raw: &[String],
    command: &str,
) -> Result<Vec<(String, String)>> {
    let mut out: Vec<(String, String)> = Vec::with_capacity(raw.len());
    for r in raw {
        let Some((key, value)) = r.split_once('=') else {
            return Err(Error::usage(
                format!("{flag} `{r}` has no value"),
                format!("pass `key=value`, e.g. `{command} {flag} tracker.project=PROJ`"),
            ));
        };
        let key = key.trim();
        check_key(flag, key)?;
        check_attribute_value(value).map_err(|why| {
            Error::usage(
                format!("invalid {flag} `{r}`: the value {why}"),
                "give it a one-line value; to remove the attribute, unset it instead",
            )
        })?;
        out.retain(|(k, _)| k != key);
        out.push((key.to_owned(), value.to_owned()));
    }
    Ok(out)
}

/// Parse each key to remove (`kipi unbind --attr key`, `kipi scope --remove-attr key`).
pub(super) fn parse_keys<'a>(flag: &str, raw: &'a [String]) -> Result<Vec<&'a str>> {
    let mut out: Vec<&str> = Vec::with_capacity(raw.len());
    for r in raw {
        if r.contains('=') {
            return Err(Error::usage(
                format!("{flag} `{r}` names a value"),
                format!(
                    "pass the key alone: `{flag} {}`",
                    r.split('=').next().unwrap_or(r)
                ),
            ));
        }
        check_key(flag, r)?;
        if !out.contains(&r.as_str()) {
            out.push(r);
        }
    }
    Ok(out)
}

fn check_key(flag: &str, key: &str) -> Result<()> {
    check_attribute_key(key).map_err(|why| {
        Error::usage(
            format!("invalid {flag} key `{key}`: the key {why}"),
            "use dotted segments of letters, digits, `-` and `_`, e.g. `tracker.project`",
        )
    })
}

/// Set `key` to `value` in the `attributes` of `table`.
pub(super) fn set(table: &mut dyn TableLike, key: &str, value: &str) -> Change {
    let shown = format!("{key}={value}");
    match get(table, key) {
        Some(old) if old == value => Change::new(Op::Keep, KEY, shown).note("already so"),
        old => {
            write(table, key, value);
            let change = Change::new(Op::Add, KEY, shown);
            match old {
                Some(old) => change.note(format!("was `{old}`")),
                None => change,
            }
        }
    }
}

/// Remove `key` from the `attributes` of `table`; a table left empty goes too.
pub(super) fn unset(table: &mut dyn TableLike, key: &str) -> Change {
    match get(table, key) {
        Some(old) => {
            remove(table, key);
            Change::new(Op::Remove, KEY, format!("{key}={old}"))
        }
        None => Change::new(Op::Keep, KEY, key).note("not there"),
    }
}

/// The TOML key path of `key` under `table`: `["tracker.project"]`, `["tracker",
/// "project"]`, …
fn locate(table: &dyn TableLike, key: &str) -> Option<Vec<String>> {
    for (k, item) in table.iter() {
        if k == key && item.is_str() {
            return Some(vec![k.to_owned()]);
        }
        if let Some(rest) = key.strip_prefix(k).and_then(|r| r.strip_prefix('.'))
            && let Some(inner) = item.as_table_like()
            && let Some(mut path) = locate(inner, rest)
        {
            path.insert(0, k.to_owned());
            return Some(path);
        }
    }
    None
}

fn attributes(table: &dyn TableLike) -> Option<&dyn TableLike> {
    table.get(KEY).and_then(Item::as_table_like)
}

fn attributes_mut(table: &mut dyn TableLike) -> Option<&mut dyn TableLike> {
    table.get_mut(KEY).and_then(Item::as_table_like_mut)
}

/// The value of `key`, if set.
fn get(table: &dyn TableLike, key: &str) -> Option<String> {
    let attrs = attributes(table)?;
    let path = locate(attrs, key)?;
    let mut item = attrs.get(&path[0])?;
    for k in &path[1..] {
        item = item.as_table_like()?.get(k)?;
    }
    item.as_str().map(str::to_owned)
}

/// Write `value` where `key` already is (keeping its comments and spacing), else as a new
/// quoted key of `attributes`, created as an inline table when absent.
fn write(table: &mut dyn TableLike, key: &str, value: &str) {
    if attributes(table).is_none() {
        table.insert(KEY, Item::Value(Value::InlineTable(InlineTable::new())));
    }
    let Some(attrs) = attributes_mut(table) else {
        return;
    };
    let Some(mut path) = locate(attrs, key) else {
        let mut new = Value::from(value);
        if let Some(inline) = inline_mut(table) {
            // The space before `}` stays at the end: it moves to the new last value.
            if let Some((_, last)) = inline.iter_mut().last() {
                // A value kipi just wrote has default decor, which renders that space.
                let tail = last.decor().suffix().cloned().unwrap_or_else(|| " ".into());
                last.decor_mut().set_suffix("");
                new.decor_mut().set_suffix(tail);
            }
            inline.insert(key, new);
        } else if let Some(attrs) = attributes_mut(table) {
            attrs.insert(key, Item::Value(new));
        }
        return;
    };
    let last = path.pop().unwrap_or_default();
    let mut at: &mut dyn TableLike = attrs;
    for k in &path {
        let Some(next) = at.get_mut(k).and_then(Item::as_table_like_mut) else {
            return;
        };
        at = next;
    }
    if let Some(old) = at.get_mut(&last).and_then(Item::as_value_mut) {
        let decor = old.decor().clone();
        *old = Value::from(value);
        *old.decor_mut() = decor;
    }
}

/// `attributes = { … }`, when it is an inline table.
fn inline_mut(table: &mut dyn TableLike) -> Option<&mut InlineTable> {
    table.get_mut(KEY)?.as_value_mut()?.as_inline_table_mut()
}

/// Remove `key`, then every table the removal left empty, `attributes` included.
fn remove(table: &mut dyn TableLike, key: &str) {
    let Some(path) = attributes(table).and_then(|a| locate(a, key)) else {
        return;
    };
    // The last value of an inline table hands the space before `}` to the one before it.
    if let ([only], Some(inline)) = (path.as_slice(), inline_mut(table))
        && inline.iter().last().is_some_and(|(k, _)| k == only)
        && let Some(tail) = inline.get(only).and_then(|v| v.decor().suffix().cloned())
    {
        inline.remove(only);
        if let Some((_, last)) = inline.iter_mut().last() {
            last.decor_mut().set_suffix(tail);
        }
        if inline.is_empty() {
            table.remove(KEY);
        }
        return;
    }
    let mut full = vec![KEY.to_owned()];
    full.extend(path);
    remove_path(table, &full);
}

/// Remove the item at `path` under `table`; returns whether `table` is now empty.
fn remove_path(table: &mut dyn TableLike, path: &[String]) -> bool {
    match path {
        [] => {}
        [last] => {
            table.remove(last);
        }
        [first, rest @ ..] => {
            let emptied = table
                .get_mut(first)
                .and_then(Item::as_table_like_mut)
                .is_some_and(|inner| remove_path(inner, rest));
            if emptied {
                table.remove(first);
            }
        }
    }
    table.is_empty()
}

#[cfg(test)]
mod tests {
    use toml_edit::DocumentMut;

    use super::super::toml::table_mut;
    use super::*;

    fn edit(text: &str, f: impl FnOnce(&mut dyn TableLike) -> Change) -> (String, Change) {
        let mut doc: DocumentMut = text.parse().unwrap();
        let change = f(table_mut(&mut doc, &["r"]).unwrap());
        (doc.to_string(), change)
    }

    #[test]
    fn set_creates_a_quoted_inline_table_and_unset_removes_it() {
        let text = "[r]\nsummary = \"s\"\n";
        let (set1, c) = edit(text, |t| set(t, "tracker.project", "PROJ"));
        assert_eq!(
            set1,
            "[r]\nsummary = \"s\"\nattributes = { \"tracker.project\" = \"PROJ\" }\n"
        );
        assert_eq!((c.op, c.value.as_str()), (Op::Add, "tracker.project=PROJ"));
        let (set2, _) = edit(&set1, |t| set(t, "tracker.board", "Team board"));
        assert_eq!(
            set2,
            "[r]\nsummary = \"s\"\nattributes = { \"tracker.project\" = \"PROJ\", \
             \"tracker.board\" = \"Team board\" }\n"
        );
        let (back, c) = edit(&set2, |t| unset(t, "tracker.board"));
        assert_eq!(c.value, "tracker.board=Team board");
        assert_eq!(back, set1);
        let (back, _) = edit(&back, |t| unset(t, "tracker.project"));
        assert_eq!(back, text);
    }

    #[test]
    fn set_changes_the_line_that_is_there() {
        let text = "[r]\n\n[r.attributes]\ntracker.project = \"OLD\"  # mine\nbase = \"main\"\n";
        let (out, c) = edit(text, |t| set(t, "tracker.project", "NEW"));
        assert_eq!(out, text.replace("OLD", "NEW"));
        assert_eq!(c.note.as_deref(), Some("was `OLD`"));
        let (_, c) = edit(text, |t| set(t, "base", "main"));
        assert_eq!((c.op, c.note.as_deref()), (Op::Keep, Some("already so")));
    }

    #[test]
    fn unset_prunes_emptied_nested_tables() {
        let text = "[r]\nattributes = { tracker = { project = \"P\" }, base = \"main\" }\n";
        let (out, _) = edit(text, |t| unset(t, "tracker.project"));
        assert_eq!(out, "[r]\nattributes = { base = \"main\" }\n");
        let (out, c) = edit(text, |t| unset(t, "tracker"));
        assert_eq!((c.op, c.note.as_deref()), (Op::Keep, Some("not there")));
        assert_eq!(out, text);
    }

    #[test]
    fn flags_parse_and_refuse_with_hints() {
        let raw = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        let sets = parse_sets("--attr", &raw(&["a.b=x=y", "c=1", "a.b=z"]), "kipi bind r").unwrap();
        assert_eq!(sets, [("c".into(), "1".into()), ("a.b".into(), "z".into())]);
        assert!(parse_sets("--attr", &raw(&["a.b"]), "kipi bind r").is_err());
        assert!(parse_sets("--attr", &raw(&["a..b=x"]), "kipi bind r").is_err());
        assert!(parse_sets("--attr", &raw(&["a="]), "kipi bind r").is_err());
        assert_eq!(parse_keys("--attr", &raw(&["a", "a"])).unwrap(), ["a"]);
        assert!(parse_keys("--attr", &raw(&["a=x"])).is_err());
    }
}
