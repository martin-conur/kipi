//! `docs/config-reference.md` documents exactly the keys the loaders accept: one table
//! row per key under each file's `## <file>` heading, checked against the serde model.

use std::collections::BTreeSet;
use std::path::Path;

use kipi_core::config::{BINDINGS_FILE, CONFIG_FILE, MACHINE_FILE, schema};

fn reference() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/config-reference.md");
    std::fs::read_to_string(path).expect("docs/config-reference.md exists")
}

/// The key column of every table row in the `## <file>` section.
fn documented(doc: &str, file: &str) -> BTreeSet<String> {
    let heading = format!("## {file}");
    let section = doc
        .split("\n## ")
        .map(|s| format!("## {s}"))
        .find(|s| s.lines().next() == Some(heading.as_str()))
        .unwrap_or_else(|| panic!("no `{heading}` section in docs/config-reference.md"));
    section
        .lines()
        .filter_map(|line| line.strip_prefix("| `"))
        .filter_map(|rest| rest.split_once('`'))
        .map(|(key, _)| key.to_owned())
        .collect()
}

#[test]
fn every_key_is_documented_and_every_row_is_a_key() {
    let doc = reference();
    for file in [BINDINGS_FILE, MACHINE_FILE, CONFIG_FILE] {
        let accepted: BTreeSet<String> = schema::keys(file).unwrap().into_iter().collect();
        let rows = documented(&doc, file);
        let missing: Vec<_> = accepted.difference(&rows).collect();
        let unknown: Vec<_> = rows.difference(&accepted).collect();
        assert!(
            missing.is_empty(),
            "{file}: keys the loader accepts but docs/config-reference.md does not document: \
             {missing:?} (add a row to the `## {file}` table)"
        );
        assert!(
            unknown.is_empty(),
            "{file}: rows in docs/config-reference.md for keys the loader rejects: {unknown:?}"
        );
    }
}
