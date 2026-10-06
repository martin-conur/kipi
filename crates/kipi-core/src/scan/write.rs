//! Comment-preserving edits to `bindings.toml` and `machine.toml`, and writing them.

use std::fs;
use std::path::{Path, PathBuf};

use toml_edit::{Array, DocumentMut, Item, Table, Value};

use crate::error::{Error, Result};

/// The trailing comment that marks a drafted value (roadmap §5.1).
pub const DRAFT_MARK: &str = "# kipi:draft";

/// A file `scan` wants to (re)write: its current text and the new one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileWrite {
    /// The file.
    pub path: PathBuf,
    /// Its current contents; `None` when it does not exist yet.
    pub before: Option<String>,
    /// The contents to write.
    pub after: String,
}

/// Write every file atomically (temp file in the same directory, then rename),
/// creating parent directories as needed.
pub fn write_files(writes: &[FileWrite]) -> Result<()> {
    for w in writes {
        let hint = "check that the directory exists and is writable";
        if let Some(dir) = w.path.parent() {
            fs::create_dir_all(dir).map_err(|e| Error::io(dir, e, hint))?;
        }
        let tmp = tmp_path(&w.path);
        fs::write(&tmp, &w.after).map_err(|e| Error::io(&tmp, e, hint))?;
        fs::rename(&tmp, &w.path).map_err(|e| {
            let _ = fs::remove_file(&tmp);
            Error::io(&w.path, e, hint)
        })?;
    }
    Ok(())
}

fn tmp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".kipi-tmp");
    path.with_file_name(name)
}

/// A value carrying the draft marker.
pub fn drafted(value: impl Into<Value>) -> Item {
    let mut value = value.into();
    value.decor_mut().set_suffix(format!("  {DRAFT_MARK}"));
    Item::Value(value)
}

/// A string array value.
pub fn string_array(items: &[String]) -> Value {
    Value::Array(items.iter().map(String::as_str).collect::<Array>())
}

/// The `[<table>]` table of `doc`, created (as an implicit parent) when missing.
fn table_mut<'a>(doc: &'a mut DocumentMut, table: &str) -> Option<&'a mut Table> {
    if !doc.contains_key(table) {
        let mut t = Table::new();
        t.set_implicit(true);
        doc.insert(table, Item::Table(t));
    }
    doc.get_mut(table)?.as_table_mut()
}

/// Append `[repos.<name>]` with `items` (key, value) in order.
pub fn add_repo(doc: &mut DocumentMut, name: &str, items: Vec<(&str, Item)>) {
    let Some(repos) = table_mut(doc, "repos") else {
        return;
    };
    let mut table = Table::new();
    table.decor_mut().set_prefix("\n");
    for (key, item) in items {
        table.insert(key, item);
    }
    repos.insert(name, Item::Table(table));
}

/// Set `key` in the existing `[repos.<name>]` (appended when absent).
pub fn set_repo_key(doc: &mut DocumentMut, name: &str, key: &str, value: Value) {
    if let Some(repo) = doc
        .get_mut("repos")
        .and_then(|r| r.get_mut(name))
        .and_then(Item::as_table_like_mut)
    {
        repo.insert(key, Item::Value(value));
    }
}

/// Replace the drafted value at `[repos.<name>] key`, keeping its decor (and with it
/// the `# kipi:draft` marker).
pub fn replace_value(doc: &mut DocumentMut, name: &str, key: &str, mut value: Value) {
    let Some(item) = doc
        .get_mut("repos")
        .and_then(|r| r.get_mut(name))
        .and_then(|r| r.get_mut(key))
    else {
        return;
    };
    if let Some(old) = item.as_value() {
        *value.decor_mut() = old.decor().clone();
    }
    *item = Item::Value(value);
}

/// Set `"<entry>" = "<path>"` in `machine.toml [repos]`, keeping an existing line's decor.
pub fn set_machine_repo(doc: &mut DocumentMut, entry: &str, path: &str) {
    let Some(repos) = table_mut(doc, "repos") else {
        return;
    };
    repos.set_implicit(false);
    let mut value = Value::from(path);
    if let Some(old) = repos.get(entry).and_then(Item::as_value) {
        *value.decor_mut() = old.decor().clone();
    }
    repos.insert(entry, Item::Value(value));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_keep_comments_and_order() {
        let text = "# top\n[store]\nname = \"s\" # name\n\n[repos.a]   # header\n\
                    summary = \"old\"  # kipi:draft\n";
        let mut doc: DocumentMut = text.parse().unwrap();
        replace_value(&mut doc, "a", "summary", Value::from("new"));
        set_repo_key(&mut doc, "a", "remote", Value::from("github.com/o/a"));
        add_repo(
            &mut doc,
            "b",
            vec![
                ("remote", Item::Value(Value::from("github.com/o/b"))),
                ("summary", drafted("B things")),
                ("aliases", drafted(string_array(&["bee".into()]))),
            ],
        );
        assert_eq!(
            doc.to_string(),
            "# top\n[store]\nname = \"s\" # name\n\n[repos.a]   # header\n\
             summary = \"new\"  # kipi:draft\nremote = \"github.com/o/a\"\n\n\
             [repos.b]\nremote = \"github.com/o/b\"\nsummary = \"B things\"  # kipi:draft\n\
             aliases = [\"bee\"]  # kipi:draft\n"
        );
    }

    #[test]
    fn repos_table_is_created_when_missing() {
        let mut doc: DocumentMut = "[store]\nname = \"s\"\n".parse().unwrap();
        add_repo(&mut doc, "x", vec![("summary", drafted("X"))]);
        assert_eq!(
            doc.to_string(),
            "[store]\nname = \"s\"\n\n[repos.x]\nsummary = \"X\"  # kipi:draft\n"
        );
    }

    #[test]
    fn machine_repos_are_set_and_updated() {
        let mut doc: DocumentMut = "hostname = \"h\"\n".parse().unwrap();
        set_machine_repo(&mut doc, "a", "/p/a");
        set_machine_repo(&mut doc, "b.c", "/p/b");
        assert_eq!(
            doc.to_string(),
            "hostname = \"h\"\n\n[repos]\na = \"/p/a\"\n\"b.c\" = \"/p/b\"\n"
        );
        let mut doc: DocumentMut = "[repos]\na = \"/old\"  # moved\n".parse().unwrap();
        set_machine_repo(&mut doc, "a", "/new");
        assert_eq!(doc.to_string(), "[repos]\na = \"/new\"  # moved\n");
    }

    #[test]
    fn writes_atomically() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("cfg/machine.toml");
        let w = FileWrite {
            path: path.clone(),
            before: None,
            after: "x = 1\n".into(),
        };
        write_files(&[w]).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "x = 1\n");
        assert!(!tmp.path().join("cfg/machine.toml.kipi-tmp").exists());
    }
}
