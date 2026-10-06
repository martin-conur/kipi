//! Property tests for splicing the generated block (roadmap §8): idempotent, and never
//! a byte changed outside the markers.

use std::path::Path;

use kipi_core::model::Machine;
use kipi_core::render::block::{self, RenderedBlock};
use kipi_core::render::splice::splice;
use kipi_core::resolve::resolve;
use kipi_test_support::builders::{MachineBuilder, StoreBuilder, store_set};
use proptest::prelude::*;

/// Two different blocks: an unscoped repo's, and a scoped one with a registry.
fn blocks() -> [RenderedBlock; 2] {
    let set = store_set(vec![
        StoreBuilder::new("s", "/st")
            .scope("a", |s| s.registry = true)
            .repo("x", |_| {})
            .repo("y", |r| {
                r.scope = Some("a".into());
                r.related = vec!["z".into()];
            })
            .repo("z", |r| r.scope = Some("a".into()))
            .build(),
    ]);
    let machine: Machine = MachineBuilder::new().repo("z", "/c/z").build();
    let plan = resolve(&set, &machine).unwrap();
    let render = |name| block::render(&plan, plan.repo(name).unwrap(), &[], None);
    [render("x"), render("y")]
}

fn is_marker(line: &str) -> bool {
    line.starts_with("<!-- kipi:begin") || line.trim_end() == "<!-- kipi:end -->"
}

/// User text built from fragments that include the markers, with every line that
/// would be a marker defused, so the text itself holds no block.
fn user_text() -> impl Strategy<Value = String> {
    let fragment = prop_oneof![
        Just("<!-- kipi:begin".to_owned()),
        Just("<!-- kipi:end -->".to_owned()),
        Just("\n".to_owned()),
        Just("\r\n".to_owned()),
        Just(" ".to_owned()),
        "[a-z#`*-]{1,8}",
        "\\PC{0,4}",
    ];
    prop::collection::vec(fragment, 0..24).prop_map(|parts| {
        parts
            .concat()
            .split_inclusive('\n')
            .map(|line| {
                if is_marker(line.strip_suffix('\n').unwrap_or(line)) {
                    format!("> {line}")
                } else {
                    line.to_owned()
                }
            })
            .collect()
    })
}

fn run(existing: Option<&str>, block: &RenderedBlock) -> String {
    splice(Path::new("CLAUDE.md"), existing, block).unwrap()
}

proptest! {
    #[test]
    fn splicing_twice_equals_splicing_once(text in user_text(), which in 0usize..2, absent: bool) {
        let block = &blocks()[which];
        let existing = (!absent).then_some(text.as_str());
        let once = run(existing, block);
        prop_assert_eq!(run(Some(&once), block), once);
    }

    #[test]
    fn appending_keeps_the_existing_text_as_a_prefix(text in user_text()) {
        let [block, _] = blocks();
        let out = run(Some(&text), &block);
        prop_assert!(out.starts_with(&text));
        let tail = format!("{}\n", block.text());
        prop_assert!(out.ends_with(&tail));
    }

    #[test]
    fn replacing_never_changes_bytes_outside_the_markers(
        before in user_text(),
        after in user_text(),
        crlf: bool,
    ) {
        let [old, new] = blocks();
        // The begin marker must start a line and the end marker must end one.
        let before = if before.is_empty() || before.ends_with('\n') { before } else { format!("{before}\n") };
        let after = format!("{}{after}", if crlf { "\r\n" } else { "\n" });
        let file = format!("{before}{}{after}", old.text());
        let out = run(Some(&file), &new);
        prop_assert_eq!(&out, &format!("{before}{}{after}", new.text()));
        // ...and switching back restores the original byte for byte.
        prop_assert_eq!(run(Some(&out), &old), file);
    }
}
