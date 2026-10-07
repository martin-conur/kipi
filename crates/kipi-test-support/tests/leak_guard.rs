//! Repo-wide leak guard: no denylisted pattern and no absolute home path anywhere in
//! code, docs, fixtures or examples. Add private patterns via `KIPI_LEAK_DENYLIST`
//! (newline-separated) or `KIPI_LEAK_DENYLIST_FILE`; see `.leak-denylist`.

use kipi_test_support::leak_guard::{Denylist, Hit, scan_history, scan_tree};
use kipi_test_support::workspace_root;

#[test]
fn repository_has_no_leaks() {
    let root = workspace_root();
    let denylist =
        Denylist::load(&root, |k| std::env::var(k).ok()).unwrap_or_else(|e| panic!("{e}"));
    let hits = scan_tree(&root, &denylist).expect("walk the repository");
    assert_clean(&hits);
}

/// The pre-release history audit: every blob and commit message on every branch, tag and
/// remote ref. Ignored by default (a leak already in history cannot be fixed by a commit);
/// run it before a release, with the private patterns set:
/// `KIPI_LEAK_DENYLIST=... cargo test -p kipi-test-support --test leak_guard -- --ignored`.
#[test]
#[ignore = "audits all of git history; run before a release"]
fn history_has_no_leaks() {
    let root = workspace_root();
    let denylist =
        Denylist::load(&root, |k| std::env::var(k).ok()).unwrap_or_else(|e| panic!("{e}"));
    let hits = scan_history(&root, &denylist).expect("read git history");
    assert_clean(&hits);
}

fn assert_clean(hits: &[Hit]) {
    if !hits.is_empty() {
        let report: Vec<String> = hits.iter().map(ToString::to_string).collect();
        panic!(
            "leak guard found {} hit(s):\n{}",
            hits.len(),
            report.join("\n")
        );
    }
}
