//! Shared plumbing for the TOML loaders: parse with spans, deserialize with serde, and
//! turn every failure into an [`Error`] pointing at `file:line` with a hint.

use std::fs;
use std::io;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

use serde::de::{DeserializeOwned, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use toml_edit::{Document, Item, TableLike};

use crate::error::{Error, Location, Result, closest};
use crate::model::TomlDoc;

/// A TOML file parsed with spans, plus the text the spans index into.
pub(crate) struct Source {
    pub path: PathBuf,
    pub text: String,
    pub doc: Document<String>,
}

impl Source {
    /// Read and parse `path`. `Ok(None)` when it does not exist.
    pub fn read_optional(path: &Path) -> Result<Option<Self>> {
        match fs::read_to_string(path) {
            Ok(text) => Self::parse(path, text).map(Some),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Error::io(path, e, "check the file's permissions")),
        }
    }

    /// Parse `text`, read from `path`.
    pub fn parse(path: &Path, text: String) -> Result<Self> {
        let doc = Document::parse(text.clone()).map_err(|e| {
            Error::invalid(
                Location::maybe_at(path, e.span().map(|s| line_at(&text, s.start))),
                format!("invalid TOML: {}", e.message()),
                "fix the TOML syntax at that line",
            )
        })?;
        Ok(Source {
            path: path.to_path_buf(),
            text,
            doc,
        })
    }

    /// Deserialize the whole document. `schema_hint` is the hint for errors that are
    /// neither unknown keys nor unknown values (wrong types, missing keys).
    pub fn deserialize<T: DeserializeOwned>(&self, schema_hint: &str) -> Result<T> {
        toml_edit::de::from_str(&self.text).map_err(|e| self.de_error(&e, schema_hint))
    }

    /// `file:line` of the key at `keys` (e.g. `["scopes", "acme", "roots"]`): the key
    /// itself when it has a span, else the item, else its first child.
    pub fn at(&self, keys: &[&str]) -> Location {
        Location::maybe_at(
            &self.path,
            self.offset(keys).map(|o| line_at(&self.text, o)),
        )
    }

    /// The text of the trailing comment on the value at `keys`, if any.
    pub fn trailing_comment(&self, keys: &[&str]) -> Option<&str> {
        let (last, parents) = keys.split_last()?;
        let table = table_at(self.doc.as_item(), parents)?;
        let suffix = table.get(last)?.as_value()?.decor().suffix()?;
        suffix
            .as_str()
            .or_else(|| suffix.span().map(|s| &self.text[s]))
    }

    /// The document, despanned for comment-preserving write-back.
    pub fn into_doc(self) -> TomlDoc {
        TomlDoc {
            path: self.path,
            doc: self.doc.into_mut(),
        }
    }

    fn offset(&self, keys: &[&str]) -> Option<usize> {
        let Some((last, parents)) = keys.split_last() else {
            return Some(0);
        };
        let table = table_at(self.doc.as_item(), parents)?;
        let (key, item) = table.get_key_value(last)?;
        key.span()
            .map(|s| s.start)
            .or_else(|| item.span().map(|s| s.start))
            .or_else(|| first_child_offset(item))
    }

    fn de_error(&self, e: &toml_edit::de::Error, schema_hint: &str) -> Error {
        let offset = e.span().map(|s| s.start);
        let at = Location::maybe_at(&self.path, offset.map(|o| line_at(&self.text, o)));
        let table = offset.map_or_else(|| label(""), |o| self.table_at(o));
        // serde names Rust types; users write TOML.
        let msg = e
            .message()
            .replace("expected i64", "expected an integer")
            .replace("expected u32", "expected an integer");
        let msg = msg.as_str();
        if let Some((key, expected)) = parse_unknown(msg, "unknown field") {
            return Error::UnknownKey {
                at,
                key,
                table,
                expected: expected.into(),
            };
        }
        if let Some((value, expected)) = parse_unknown(msg, "unknown variant") {
            let valid = expected
                .iter()
                .map(|v| format!("`{v}`"))
                .collect::<Vec<_>>()
                .join(", ");
            let hint = match closest(&value, expected.iter().map(String::as_str)) {
                Some(s) => format!("did you mean `{s}`? Valid values: {valid}"),
                None => format!("valid values: {valid}"),
            };
            return Error::invalid(at, format!("unknown value `{value}` in {table}"), hint);
        }
        if let Some(key) = msg
            .strip_prefix("missing field `")
            .and_then(|r| r.strip_suffix('`'))
        {
            return Error::invalid(
                at,
                format!("missing key `{key}` in {table}"),
                format!("add `{key} = ...` to {table}"),
            );
        }
        Error::invalid(at, format!("{msg} in {table}"), schema_hint)
    }

    /// The `[header]` of the table holding byte `offset`, for messages. An offset inside
    /// a header line (an unknown table name) belongs to that header's parent.
    fn table_at(&self, offset: usize) -> String {
        let offset = offset.min(self.text.len());
        let line_start = self.text[..offset].rfind('\n').map_or(0, |n| n + 1);
        let current = self.text[line_start..offset].trim_start();
        if let Some(inner) = current.strip_prefix('[') {
            let parent = inner.trim_start_matches('[').trim_end_matches(['.', ' ']);
            return label(parent);
        }
        self.text[..line_start]
            .lines()
            .rev()
            .map(str::trim)
            .find(|l| l.starts_with('['))
            .map(|l| {
                let inner = l.trim_start_matches('[');
                label(inner.split(']').next().unwrap_or(inner))
            })
            .unwrap_or_else(|| label(""))
    }
}

/// `[a.b]` for a table path, or `the top level` for the root.
fn label(path: &str) -> String {
    if path.is_empty() {
        "the top level".to_owned()
    } else {
        format!("[{path}]")
    }
}

/// 1-based line of byte `offset` in `text`.
pub(crate) fn line_at(text: &str, offset: usize) -> usize {
    text.as_bytes()[..offset.min(text.len())]
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
        + 1
}

fn table_at<'a>(root: &'a Item, keys: &[&str]) -> Option<&'a dyn TableLike> {
    let mut table = root.as_table_like()?;
    for key in keys {
        table = table.get(key)?.as_table_like()?;
    }
    Some(table)
}

fn first_child_offset(item: &Item) -> Option<usize> {
    item.as_table_like()?
        .iter()
        .filter_map(|(k, child)| {
            let key = item.as_table_like()?.key(k)?;
            key.span()
                .map(|s| s.start)
                .or_else(|| first_child_offset(child))
        })
        .min()
}

/// Split serde's `unknown field `x`, expected one of `a`, `b`` (or `unknown variant`)
/// into the offending name and the valid ones. serde's wording is stable; the
/// `serde_unknown_messages_are_recognized` test guards it.
fn parse_unknown(msg: &str, prefix: &str) -> Option<(String, Vec<String>)> {
    let rest = msg.strip_prefix(prefix)?.strip_prefix(" `")?;
    let (name, rest) = rest.split_once('`')?;
    let expected = rest
        .split('`')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect();
    Some((name.to_owned(), expected))
}

/// A TOML table deserialized as `(key, value)` pairs in document order.
#[derive(Debug)]
pub(crate) struct Entries<T>(pub Vec<(String, T)>);

impl<T> Default for Entries<T> {
    fn default() -> Self {
        Entries(Vec::new())
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Entries<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<T>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>> Visitor<'de> for V<T> {
            type Value = Entries<T>;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a table")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut out = Vec::new();
                while let Some(entry) = map.next_entry()? {
                    out.push(entry);
                }
                Ok(Entries(out))
            }
        }
        d.deserialize_map(V(PhantomData))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    #[allow(dead_code)]
    struct T {
        alpha: Option<u8>,
        beta: Option<Mode>,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "lowercase")]
    enum Mode {
        One,
        Two,
    }

    fn err(text: &str) -> Error {
        Source::parse(Path::new("f.toml"), text.to_owned())
            .unwrap()
            .deserialize::<T>("schema")
            .unwrap_err()
    }

    #[test]
    fn serde_unknown_messages_are_recognized() {
        match err("alpha = 1\n\nalpah = 2\n") {
            ref e @ Error::UnknownKey {
                ref at,
                ref key,
                ref expected,
                ..
            } => {
                assert_eq!(at.line, Some(3));
                assert_eq!(key, "alpah");
                assert_eq!(e.suggestion(), Some("alpha"));
                assert_eq!(**expected, ["alpha", "beta"]);
            }
            other => panic!("{other:?}"),
        }
        let e = err("beta = \"tow\"\n");
        assert_eq!(e.hint(), "did you mean `two`? Valid values: `one`, `two`");
    }

    #[test]
    fn line_at_counts_newlines() {
        assert_eq!(line_at("a\nb\nc", 0), 1);
        assert_eq!(line_at("a\nb\nc", 2), 2);
        assert_eq!(line_at("a\nb\nc", 99), 3);
    }

    #[test]
    fn at_resolves_keys_tables_and_dotted_keys() {
        let text = "x = 1\n\n[t]\ny = 2\n\n[t.u]\nz = 3\n\n[d]\ne.f = 4\n";
        let src = Source::parse(Path::new("f.toml"), text.to_owned()).unwrap();
        let line = |k: &[&str]| src.at(k).line;
        assert_eq!(line(&["x"]), Some(1));
        assert_eq!(line(&["t", "y"]), Some(4));
        assert_eq!(line(&["t", "u"]), Some(6));
        assert_eq!(line(&["d", "e", "f"]), Some(10));
        assert_eq!(line(&["missing"]), None);
    }

    #[test]
    fn trailing_comments_are_read_from_spans() {
        let text = "[r]\ns = \"v\"  # kipi:draft\nt = 1\n";
        let src = Source::parse(Path::new("f.toml"), text.to_owned()).unwrap();
        assert_eq!(src.trailing_comment(&["r", "s"]), Some("  # kipi:draft"));
        assert_eq!(src.trailing_comment(&["r", "t"]).map(str::trim), Some(""));
    }
}
