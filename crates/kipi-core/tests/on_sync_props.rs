//! Sync-command templates (`on_sync`): whatever an attribute value holds (spaces, quotes,
//! `;`, `$(...)`, braces), filling it never adds, drops or splits an argument, and the
//! value lands verbatim.

use kipi_core::on_sync::{fill, split, template};
use proptest::prelude::*;

/// Any one-line attribute value, weighted toward shell-significant characters.
fn value() -> impl Strategy<Value = String> {
    prop_oneof![
        "[ -~]{1,40}",
        "[ ;|&$()`'\"{}<>*?~\\\\a-z]{1,40}",
        Just("$(rm -rf ~); echo 'x' \"y\" {attr.z}".to_owned()),
    ]
}

/// A `run` string with `{attr.v}` in some argument positions (never the program).
fn run() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop_oneof![
            Just("{attr.v}".to_owned()),
            Just("--key={attr.v}".to_owned()),
            Just("'quoted {attr.v} arg'".to_owned()),
            Just("\"dq {attr.v}\"".to_owned()),
            "[a-z-]{1,8}",
        ],
        0..6,
    )
    .prop_map(|args| format!("tool {}", args.join(" ")))
}

proptest! {
    #[test]
    fn filling_keeps_argv_shape_and_values_verbatim(run in run(), v in value()) {
        let words = split(&run).unwrap();
        let argv = template(&words).unwrap();
        let filled = fill(&argv, |k| (k == "v").then_some(v.as_str())).unwrap();
        prop_assert_eq!(filled.len(), argv.len());
        // Each hole holds exactly the value: fill with a marker, then put the value there.
        let marked = fill(&argv, |_| Some("\u{1}")).unwrap();
        for (out, mark) in filled.iter().zip(&marked) {
            prop_assert_eq!(out, &mark.replace('\u{1}', &v));
        }
        prop_assert_eq!(&filled[0], "tool");
    }

    #[test]
    fn array_words_are_taken_as_is(args in prop::collection::vec("[ -~]{0,20}", 0..6)) {
        let escaped: Vec<String> = std::iter::once("tool".to_owned())
            .chain(args.iter().map(|a| a.replace('{', "{{").replace('}', "}}")))
            .collect();
        let argv = template(&escaped).unwrap();
        let filled = fill(&argv, |_| None).unwrap();
        prop_assert_eq!(&filled[1..], args.as_slice());
    }
}
