//! Owned-entry merge into a JSON settings file: kipi's read grant in
//! `.claude/settings.local.json` → `permissions.additionalDirectories` (roadmap §3, §8 step 5).
//!
//! [`merge`] adds the entries kipi wants and removes only the entries it added before (the
//! manifest's record of them). Entries anyone else put in the array, and every other key,
//! stay as they are; an entry the user already had is never claimed as kipi's. Key order is
//! kept (`serde_json` with `preserve_order`) and the file's indentation is reused, so a
//! change shows as the lines it touches. Malformed JSON is an error, never overwritten.

use std::fmt;

use serde::Serialize;
use serde_json::ser::{PrettyFormatter, Serializer};
use serde_json::{Map, Value};

/// What [`merge`] decided for the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Leave the file as it is (or absent).
    Unchanged,
    /// Write this text.
    Write(String),
    /// Delete the file: kipi created it and removing its entries left nothing else.
    Delete,
}

/// The result of one [`merge`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Merge {
    /// What to do with the file.
    pub outcome: Outcome,
    /// The entries kipi owns afterwards, in the order it wants them.
    pub owned: Vec<String>,
    /// Entries kipi added to the array.
    pub added: Vec<String>,
    /// Entries kipi removed from the array.
    pub removed: Vec<String>,
}

/// Why [`merge`] left a file alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeError {
    /// The text is not valid JSON.
    Malformed(String),
    /// The value at this key path (`""` for the top level) is not an object.
    NotObject(String),
    /// The value at this key path is not an array.
    NotArray(String),
}

impl MergeError {
    /// What the user can do about it.
    pub fn hint(&self) -> String {
        match self {
            MergeError::Malformed(_) => {
                "fix the JSON syntax (or delete the file), then re-run; kipi never overwrites a \
                 file it cannot parse"
                    .into()
            }
            MergeError::NotObject(key) | MergeError::NotArray(key) => {
                format!("fix the value of `{key}` by hand, then re-run")
            }
        }
    }
}

impl fmt::Display for MergeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MergeError::Malformed(e) => write!(f, "malformed JSON: {e}"),
            MergeError::NotObject(key) if key.is_empty() => {
                write!(f, "the top level is not a JSON object")
            }
            MergeError::NotObject(key) => write!(f, "`{key}` is not a JSON object"),
            MergeError::NotArray(key) => write!(f, "`{key}` is not a JSON array"),
        }
    }
}

impl std::error::Error for MergeError {}

/// Merge `want` into the array at `key` (object keys from the top level down) of the JSON
/// document `text` (`None`: the file does not exist). `owned` lists the entries kipi added
/// earlier; those no longer wanted are removed. With `created` (kipi created the file),
/// a document left empty is deleted. Missing objects and the array are created as needed;
/// an array, and objects, that removal leaves empty are dropped again.
pub fn merge(
    text: Option<&str>,
    key: &[String],
    want: &[String],
    owned: &[String],
    created: bool,
) -> Result<Merge, MergeError> {
    let before = match text {
        None => Value::Object(Map::new()),
        Some(t) if t.trim().is_empty() => Value::Object(Map::new()),
        Some(t) => serde_json::from_str(t).map_err(|e| MergeError::Malformed(e.to_string()))?,
    };
    let mut doc = before.clone();
    let Value::Object(root) = &mut doc else {
        return Err(MergeError::NotObject(String::new()));
    };
    let Some((last, parents)) = key.split_last() else {
        return Err(MergeError::NotArray(String::new()));
    };
    let mut obj = root;
    for (i, k) in parents.iter().enumerate() {
        if !obj.contains_key(k) {
            if want.is_empty() {
                return Ok(UNCHANGED);
            }
            obj.insert(k.clone(), Value::Object(Map::new()));
        }
        obj = match obj.get_mut(k) {
            Some(Value::Object(o)) => o,
            _ => return Err(MergeError::NotObject(key[..=i].join("."))),
        };
    }
    if !obj.contains_key(last) {
        if want.is_empty() {
            return Ok(UNCHANGED);
        }
        obj.insert(last.clone(), Value::Array(Vec::new()));
    }
    let Some(Value::Array(array)) = obj.get_mut(last) else {
        return Err(MergeError::NotArray(key.join(".")));
    };

    let mut removed = Vec::new();
    array.retain(|v| match v.as_str() {
        Some(s) if owned.iter().any(|o| o == s) && !want.iter().any(|w| w == s) => {
            removed.push(s.to_owned());
            false
        }
        _ => true,
    });
    let mut added = Vec::new();
    let mut now_owned = Vec::new();
    for w in want {
        if now_owned.contains(w) {
            continue;
        }
        if array.iter().any(|v| v.as_str() == Some(w)) {
            // Present already: kipi's only if kipi put it there.
            if owned.contains(w) {
                now_owned.push(w.clone());
            }
        } else {
            array.push(Value::String(w.clone()));
            added.push(w.clone());
            now_owned.push(w.clone());
        }
    }
    if array.is_empty() && !removed.is_empty() {
        let Value::Object(root) = &mut doc else {
            unreachable!("checked above")
        };
        prune(root, key);
    }

    let outcome = if doc == before {
        Outcome::Unchanged
    } else if created && doc.as_object().is_some_and(Map::is_empty) {
        Outcome::Delete
    } else {
        Outcome::Write(render(&doc, text))
    };
    Ok(Merge {
        outcome,
        owned: now_owned,
        added,
        removed,
    })
}

/// Nothing wanted, and no array to remove anything from.
const UNCHANGED: Merge = Merge {
    outcome: Outcome::Unchanged,
    owned: Vec::new(),
    added: Vec::new(),
    removed: Vec::new(),
};

/// Remove the (empty) array at `key`, then every object on the way to it that this leaves
/// empty.
fn prune(obj: &mut Map<String, Value>, key: &[String]) {
    let Some((first, rest)) = key.split_first() else {
        return;
    };
    if !rest.is_empty() {
        let Some(Value::Object(child)) = obj.get_mut(first) else {
            return;
        };
        prune(child, rest);
        if !child.is_empty() {
            return;
        }
    }
    obj.shift_remove(first);
}

/// `doc` pretty-printed with the indentation of `original` (two spaces when it has none),
/// ending in a newline unless `original` did not.
fn render(doc: &Value, original: Option<&str>) -> String {
    let indent = original
        .and_then(|t| {
            t.lines().find_map(|line| {
                let ws: &str = &line[..line.len() - line.trim_start().len()];
                (!ws.is_empty() && !line.trim().is_empty()).then_some(ws)
            })
        })
        .unwrap_or("  ");
    let mut out = Vec::new();
    let mut ser =
        Serializer::with_formatter(&mut out, PrettyFormatter::with_indent(indent.as_bytes()));
    doc.serialize(&mut ser).expect("a JSON value serializes");
    let mut text = String::from_utf8(out).expect("serde_json writes UTF-8");
    if original.is_none_or(|t| t.trim().is_empty() || t.ends_with('\n')) {
        text.push('\n');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> Vec<String> {
        vec!["permissions".into(), "additionalDirectories".into()]
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn indentation_and_trailing_newline_are_kept() {
        let text = "{\n\t\"a\": 1\n}";
        let m = merge(Some(text), &key(), &s(&["/x"]), &[], false).unwrap();
        assert_eq!(
            m.outcome,
            Outcome::Write(
                "{\n\t\"a\": 1,\n\t\"permissions\": {\n\t\t\"additionalDirectories\": [\n\t\t\t\"/x\"\n\t\t]\n\t}\n}"
                    .into()
            )
        );
    }

    #[test]
    fn a_user_entry_equal_to_a_wanted_one_is_not_claimed() {
        let text = r#"{"permissions":{"additionalDirectories":["/x"]}}"#;
        let m = merge(Some(text), &key(), &s(&["/x"]), &[], false).unwrap();
        assert_eq!(m.outcome, Outcome::Unchanged);
        assert!(m.owned.is_empty());
        let m = merge(Some(text), &key(), &[], &[], false).unwrap();
        assert_eq!(m.outcome, Outcome::Unchanged);
    }

    #[test]
    fn nothing_wanted_and_nothing_there_leaves_the_file_absent() {
        let m = merge(None, &key(), &[], &[], false).unwrap();
        assert_eq!(m.outcome, Outcome::Unchanged);
    }

    #[test]
    fn a_created_file_is_deleted_once_empty_but_a_user_file_is_kept() {
        let first = merge(None, &key(), &s(&["/x"]), &[], true).unwrap();
        let Outcome::Write(text) = first.outcome else {
            panic!()
        };
        let gone = merge(Some(&text), &key(), &[], &first.owned, true).unwrap();
        assert_eq!(gone.outcome, Outcome::Delete);
        assert_eq!(gone.removed, s(&["/x"]));
        let kept = merge(Some(&text), &key(), &[], &first.owned, false).unwrap();
        assert_eq!(kept.outcome, Outcome::Write("{}\n".into()));
    }

    #[test]
    fn wrong_shapes_are_errors_with_hints() {
        let e = merge(Some("[1]"), &key(), &s(&["/x"]), &[], false).unwrap_err();
        assert_eq!(e.to_string(), "the top level is not a JSON object");
        let e = merge(
            Some(r#"{"permissions": 3}"#),
            &key(),
            &s(&["/x"]),
            &[],
            false,
        )
        .unwrap_err();
        assert_eq!(e.to_string(), "`permissions` is not a JSON object");
        assert!(e.hint().contains("`permissions`"));
        let e = merge(
            Some(r#"{"permissions": {"additionalDirectories": "/x"}}"#),
            &key(),
            &[],
            &[],
            false,
        )
        .unwrap_err();
        assert_eq!(
            e.to_string(),
            "`permissions.additionalDirectories` is not a JSON array"
        );
    }
}
