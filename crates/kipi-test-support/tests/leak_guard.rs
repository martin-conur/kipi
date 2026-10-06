//! Repo-wide leak guard: no denylisted pattern and no absolute home path anywhere in
//! code, docs, fixtures or examples. Add private patterns via `KIPI_LEAK_DENYLIST`
//! (newline-separated) or `KIPI_LEAK_DENYLIST_FILE`; see `.leak-denylist`.

use kipi_test_support::leak_guard::{Denylist, scan_tree};
use kipi_test_support::workspace_root;

#[test]
fn repository_has_no_leaks() {
    let root = workspace_root();
    let denylist =
        Denylist::load(&root, |k| std::env::var(k).ok()).unwrap_or_else(|e| panic!("{e}"));
    let hits = scan_tree(&root, &denylist).expect("walk the repository");
    if !hits.is_empty() {
        let report: Vec<String> = hits.iter().map(ToString::to_string).collect();
        panic!(
            "leak guard found {} hit(s):\n{}",
            hits.len(),
            report.join("\n")
        );
    }
}
