//! The owned-entry JSON merge behind the read grant (roadmap §3, §8 steps 4–5): fixture
//! settings files run through a grant-mode sequence and snapshotted, and a property test
//! that entries and keys the user owns survive any sequence of syncs.

use std::fmt::Write as _;
use std::path::Path;

use kipi_core::json_merge::{Outcome, merge};
use proptest::prelude::*;
use serde_json::Value;

fn key() -> Vec<String> {
    vec!["permissions".into(), "additionalDirectories".into()]
}

fn strings(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// A settings file synced the way apply does it: the manifest's `[[grant]]` (owned
/// entries and `created`) exists only while kipi owns entries in the file.
struct File {
    text: Option<String>,
    owned: Vec<String>,
    created: bool,
}

impl File {
    fn new(text: Option<String>) -> File {
        File {
            text,
            owned: Vec::new(),
            created: false,
        }
    }

    fn sync(&mut self, want: &[String]) -> Result<Outcome, String> {
        // As apply: the recorded flag while kipi owns entries, else whether the file is new.
        let created = if self.owned.is_empty() {
            self.text.is_none()
        } else {
            self.created
        };
        let m = merge(self.text.as_deref(), &key(), want, &self.owned, created)
            .map_err(|e| format!("{e}\nhint: {}", e.hint()))?;
        match &m.outcome {
            Outcome::Unchanged => {}
            Outcome::Write(t) => self.text = Some(t.clone()),
            Outcome::Delete => self.text = None,
        }
        self.owned = m.owned;
        self.created = created;
        Ok(m.outcome)
    }
}

fn transcript(fixture: Option<&str>) -> String {
    let text = fixture.map(|name| {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/settings")
            .join(name);
        std::fs::read_to_string(path).unwrap()
    });
    let mut file = File::new(text);
    let steps: [(&str, Vec<String>); 4] = [
        (
            "grant = related",
            strings(&["/w/store/repos", "/w/store/skills", "/w/billing-api"]),
        ),
        (
            "grant = related (again)",
            strings(&["/w/store/repos", "/w/store/skills", "/w/billing-api"]),
        ),
        (
            "grant = registry",
            strings(&[
                "/w/store/repos",
                "/w/store/skills",
                "/w/billing-api",
                "/w/ml-platform",
            ]),
        ),
        ("grant = none", Vec::new()),
    ];
    let mut out = String::new();
    match &file.text {
        None => out.push_str("## before: (absent)\n\n"),
        Some(t) => write!(out, "## before\n{t}\n").unwrap(),
    }
    for (label, want) in &steps {
        write!(out, "## {label}: ").unwrap();
        match file.sync(want) {
            Err(e) => write!(out, "skipped: {e}\n\n").unwrap(),
            Ok(Outcome::Unchanged) => out.push_str("unchanged\n\n"),
            Ok(Outcome::Delete) => out.push_str("deleted\n\n"),
            Ok(Outcome::Write(t)) => write!(out, "write (owned: {:?})\n{t}\n", file.owned).unwrap(),
        }
    }
    out
}

#[test]
fn absent() {
    insta::assert_snapshot!(transcript(None));
}

#[test]
fn empty() {
    insta::assert_snapshot!(transcript(Some("empty.json")));
}

#[test]
fn user_entries_only() {
    insta::assert_snapshot!(transcript(Some("user-entries.json")));
}

#[test]
fn mixed_with_other_keys() {
    insta::assert_snapshot!(transcript(Some("mixed.json")));
}

#[test]
fn malformed() {
    insta::assert_snapshot!(transcript(Some("malformed.json")));
}

/// Directories drawn from one pool for both the user and kipi, so they collide.
fn dir() -> impl Strategy<Value = String> {
    prop::sample::select(vec!["/a", "/b", "/c", "/d", "~/e", "/f g"]).prop_map(str::to_owned)
}

/// A settings document the user wrote: other keys, maybe a `permissions` object, maybe
/// their own `additionalDirectories` entries (non-strings included).
fn user_doc() -> impl Strategy<Value = Option<Value>> {
    let entries = prop::collection::vec(
        prop_oneof![4 => dir().prop_map(Value::String), 1 => Just(Value::from(7))],
        0..4,
    );
    prop_oneof![
        1 => Just(None),
        4 => (any::<bool>(), any::<bool>(), entries).prop_map(|(other, perms, entries)| {
            let mut doc = serde_json::json!({});
            if other {
                doc["model"] = "opus".into();
                doc["env"] = serde_json::json!({"A": "1"});
            }
            if perms {
                doc["permissions"] = serde_json::json!({"allow": ["Read"]});
            }
            if !entries.is_empty() {
                doc["permissions"]["additionalDirectories"] = Value::Array(entries);
            }
            Some(doc)
        }),
    ]
}

fn entries(doc: &Value) -> Vec<Value> {
    doc.pointer("/permissions/additionalDirectories")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// `doc` without the array, for comparing everything else.
fn others(doc: &Value) -> Value {
    let mut doc = doc.clone();
    if let Some(p) = doc.get_mut("permissions").and_then(Value::as_object_mut) {
        p.shift_remove("additionalDirectories");
    }
    doc
}

proptest! {
    #[test]
    fn user_entries_and_keys_survive_any_sequence_of_syncs(
        user in user_doc(),
        syncs in prop::collection::vec(prop::collection::vec(dir(), 0..4), 1..8),
    ) {
        let text = user.as_ref().map(|d| serde_json::to_string_pretty(d).unwrap());
        let user_entries = user.as_ref().map(entries).unwrap_or_default();
        let user_others = user.as_ref().map(others);
        let mut file = File::new(text);
        for want in &syncs {
            file.sync(want).unwrap();
            let Some(text) = &file.text else {
                // Deleted only when the user had no file at all.
                prop_assert!(user.is_none());
                continue;
            };
            let doc: Value = serde_json::from_str(text).unwrap();
            let now = entries(&doc);
            // Every user entry is still there, in its order.
            let mut it = now.iter();
            for e in &user_entries {
                prop_assert!(it.any(|n| n == e), "lost {e} in {text}");
            }
            // Everything else is either the user's or wanted now.
            for n in &now {
                prop_assert!(
                    user_entries.contains(n) || want.iter().any(|w| n.as_str() == Some(w)),
                    "stray {n} in {text}"
                );
            }
            // Every wanted entry is granted.
            for w in want {
                prop_assert!(now.iter().any(|n| n.as_str() == Some(w)));
            }
            // Every other key is untouched (`permissions` may have been created by kipi).
            if let Some(o) = &user_others {
                let mut got = others(&doc);
                if o.get("permissions").is_none()
                    && got.get("permissions").is_some_and(|p| p.as_object().unwrap().is_empty())
                {
                    got.as_object_mut().unwrap().shift_remove("permissions");
                }
                prop_assert_eq!(&got, o);
            }
            // A second sync of the same state changes nothing.
            let again = merge(Some(text), &key(), want, &file.owned, file.created).unwrap();
            prop_assert_eq!(again.outcome, Outcome::Unchanged);
        }
        // Dropping the grant gives the user's document back.
        file.sync(&[]).unwrap();
        match (&user, &file.text) {
            (None, None) => {}
            (Some(u), Some(t)) => {
                let doc: Value = serde_json::from_str(t).unwrap();
                prop_assert_eq!(&entries(&doc), &user_entries);
                prop_assert_eq!(others(&doc), others(u));
            }
            (u, t) => prop_assert!(false, "user {u:?} ended as {t:?}"),
        }
    }
}

/// What happens between syncs.
#[derive(Debug, Clone)]
enum Step {
    /// kipi syncs with this grant (empty: `grant = "none"` or unbound).
    Sync(Vec<String>),
    /// The user adds a directory to the array by hand (creating the file if needed).
    UserAdds(String),
    /// The user deletes the settings file.
    UserDeletes,
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        4 => prop::collection::vec(dir(), 0..4).prop_map(Step::Sync),
        2 => Just(Step::Sync(Vec::new())),
        3 => dir().prop_map(Step::UserAdds),
        1 => Just(Step::UserDeletes),
    ]
}

proptest! {
    /// Clearing the grant (or deleting the file) and then adding, by hand, a path kipi
    /// once owned makes that path the user's: no later sync removes it or deletes the file.
    #[test]
    fn entries_the_user_adds_between_syncs_are_never_removed(
        steps in prop::collection::vec(step(), 1..16),
    ) {
        let mut file = File::new(None);
        let mut user: Vec<String> = Vec::new();
        let mut want: Vec<String> = Vec::new();
        for step in steps {
            match step {
                Step::Sync(w) => {
                    file.sync(&w).unwrap();
                    want = w;
                }
                // A path the manifest still records as kipi's is kipi's: with no sync in
                // between, nothing on disk tells a hand-added copy apart (docs/decisions.md).
                Step::UserAdds(d) if file.owned.contains(&d) => {}
                Step::UserAdds(d) => {
                    let mut doc: Value = file
                        .text
                        .as_deref()
                        .map_or_else(|| serde_json::json!({}), |t| serde_json::from_str(t).unwrap());
                    let mut now = entries(&doc);
                    if now.iter().any(|n| n.as_str() == Some(&d)) {
                        // Already there; whoever added it still owns it.
                        continue;
                    }
                    now.push(Value::String(d.clone()));
                    doc["permissions"]["additionalDirectories"] = Value::Array(now);
                    file.text = Some(serde_json::to_string_pretty(&doc).unwrap());
                    user.push(d);
                }
                Step::UserDeletes => {
                    file.text = None;
                    user.clear();
                }
            }
            let now: Vec<String> = file
                .text
                .as_deref()
                .map(|t| entries(&serde_json::from_str(t).unwrap()))
                .unwrap_or_default()
                .iter()
                .map(|v| v.as_str().unwrap().to_owned())
                .collect();
            for u in &user {
                prop_assert!(now.contains(u), "lost the user's {u}: {now:?}");
            }
            for n in &now {
                prop_assert!(user.contains(n) || want.contains(n), "stray {n}");
            }
            if !user.is_empty() {
                prop_assert!(file.text.is_some());
            }
        }
    }
}
