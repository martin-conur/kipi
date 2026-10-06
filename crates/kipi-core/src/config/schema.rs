//! Every key each config file accepts, read off the serde model itself, so the config
//! reference (`docs/config-reference.md`) can be checked for completeness by a test.
//!
//! [`fields`] asks a struct's `Deserialize` impl for its field list: derived impls hand
//! it to `deserialize_struct`, which records it and stops. Each loader composes its own
//! key paths from those lists ([`keys`]), so a field added to any `Raw*` struct shows up
//! here without anyone remembering to list it.

use std::fmt;

use serde::Deserialize;
use serde::de::{self, Visitor};

use super::{BINDINGS_FILE, CONFIG_FILE, MACHINE_FILE};

/// The dotted key paths `file` (`bindings.toml`, `machine.toml` or `config.toml`)
/// accepts, with `<scope>`, `<repo>` standing for table names the user picks; `None`
/// for any other file name.
pub fn keys(file: &str) -> Option<Vec<String>> {
    match file {
        BINDINGS_FILE => Some(super::bindings::schema_keys()),
        MACHINE_FILE => Some(super::machine::schema_keys()),
        CONFIG_FILE => Some(super::settings::schema_keys()),
        _ => None,
    }
}

/// The field names of struct `T`, as its derived `Deserialize` impl declares them.
pub(super) fn fields<'de, T: Deserialize<'de>>() -> &'static [&'static str] {
    let mut found = None;
    let _ = T::deserialize(Introspect(&mut found));
    found.expect("a struct with derived Deserialize")
}

/// `prefix.<field>` for every field of `T`.
pub(super) fn under<'de, T: Deserialize<'de>>(prefix: &str) -> Vec<String> {
    fields::<T>()
        .iter()
        .map(|f| format!("{prefix}.{f}"))
        .collect()
}

struct Introspect<'a>(&'a mut Option<&'static [&'static str]>);

#[derive(Debug)]
struct Stop;

impl fmt::Display for Stop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("introspection stops at the first struct")
    }
}

impl std::error::Error for Stop {}

impl de::Error for Stop {
    fn custom<T: fmt::Display>(_: T) -> Self {
        Stop
    }
}

impl<'de> de::Deserializer<'de> for Introspect<'_> {
    type Error = Stop;

    fn deserialize_any<V: Visitor<'de>>(self, _: V) -> Result<V::Value, Stop> {
        Err(Stop)
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        fields: &'static [&'static str],
        _: V,
    ) -> Result<V::Value, Stop> {
        *self.0 = Some(fields);
        Err(Stop)
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes
        byte_buf option unit unit_struct newtype_struct seq tuple tuple_struct map enum
        identifier ignored_any
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_file_has_its_keys() {
        let bindings = keys(BINDINGS_FILE).unwrap();
        for key in ["store.name", "defaults.grant", "scopes.<scope>.extends"] {
            assert!(bindings.iter().any(|k| k == key), "{key} in {bindings:?}");
        }
        assert!(bindings.iter().any(|k| k == "repos.<repo>.related"));
        let machine = keys(MACHINE_FILE).unwrap();
        assert_eq!(machine, ["hostname", "roots.<scope>", "repos.<repo>"]);
        assert_eq!(
            keys(CONFIG_FILE).unwrap(),
            ["stores", "editor", "tokenizer"]
        );
        assert!(keys("notes.md").is_none());
    }
}
