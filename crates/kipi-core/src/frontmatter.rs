//! YAML frontmatter parse/serialize for `SKILL.md` (standard fields plus `x-kipi`).
//!
//! A [`Frontmatter`] keeps the file's exact bytes alongside the parsed mapping:
//! [`Frontmatter::render`] returns them unchanged until a field is [`set`](Frontmatter::set),
//! so a load/save round trip of an untouched file is byte-stable. Fields kipi does not
//! know (`license`, `allowed-tools`, ...) are kept as-is; only `x-kipi` is validated
//! strictly ([`SkillMeta`]).

use std::path::Path;

use yaml_rust2::yaml::Hash;
use yaml_rust2::{Yaml, YamlEmitter, YamlLoader};

use crate::error::{Error, Location, Result};

/// The frontmatter delimiter line.
const FENCE: &str = "---";

/// Keys allowed under `x-kipi`.
pub const X_KIPI_KEYS: &[&str] = &["scope", "tags"];

/// Longest skill name the Agent Skills format allows.
pub const MAX_NAME_LEN: usize = 64;

/// A Markdown file with a YAML frontmatter block.
#[derive(Debug, Clone)]
pub struct Frontmatter {
    /// The opening fence line, line ending included.
    open: String,
    /// The YAML between the fences, exactly as read.
    yaml_text: String,
    /// The closing fence line, line ending (if any) included.
    close: String,
    /// Everything after the closing fence.
    body: String,
    /// The parsed mapping.
    fields: Hash,
    /// A field changed since parsing, so `render` must re-emit the YAML.
    modified: bool,
}

impl Frontmatter {
    /// Parse `text`, the contents of `path` (used in error locations).
    pub fn parse(path: &Path, text: &str) -> Result<Self> {
        let hint_missing = "start the file with a `---` line, the YAML fields (`name`, \
                            `description`), and a closing `---` line";
        let mut lines = text.split_inclusive('\n');
        let open = lines.next().unwrap_or_default();
        if open.trim_end_matches(['\r', '\n']) != FENCE {
            return Err(Error::invalid(
                Location::at(path, 1),
                "missing YAML frontmatter",
                hint_missing,
            ));
        }
        let mut offset = open.len();
        let mut close = None;
        for line in lines {
            if line.trim_end_matches(['\r', '\n']) == FENCE {
                close = Some((offset, line));
                break;
            }
            offset += line.len();
        }
        let Some((close_at, close)) = close else {
            return Err(Error::invalid(
                Location::at(path, 1),
                "frontmatter is not closed",
                "add a `---` line after the last YAML field",
            ));
        };
        let yaml_text = &text[open.len()..close_at];
        let body = &text[close_at + close.len()..];

        let docs = YamlLoader::load_from_str(yaml_text).map_err(|e| {
            Error::invalid(
                // Marker lines are 1-based within the YAML, which starts on file line 2.
                Location::at(path, e.marker().line() + 1),
                format!("invalid YAML frontmatter: {}", e.info()),
                "fix the YAML syntax between the `---` lines",
            )
        })?;
        let fields = match docs.into_iter().next() {
            Some(Yaml::Hash(h)) => h,
            None | Some(Yaml::Null) => Hash::new(),
            Some(_) => {
                return Err(Error::invalid(
                    Location::at(path, 2),
                    "frontmatter is not a mapping",
                    "write the frontmatter as `key: value` lines",
                ));
            }
        };
        Ok(Frontmatter {
            open: open.to_owned(),
            yaml_text: yaml_text.to_owned(),
            close: close.to_owned(),
            body: body.to_owned(),
            fields,
            modified: false,
        })
    }

    /// A top-level field.
    pub fn get(&self, key: &str) -> Option<&Yaml> {
        self.fields.get(&Yaml::String(key.to_owned()))
    }

    /// Set a top-level field (appended if new, replaced in place otherwise).
    pub fn set(&mut self, key: &str, value: Yaml) {
        let key = Yaml::String(key.to_owned());
        // `insert` would move an existing key to the end; keep its position instead.
        match self.fields.get_mut(&key) {
            Some(slot) => *slot = value,
            None => {
                self.fields.insert(key, value);
            }
        }
        self.modified = true;
    }

    /// The parsed mapping, in file order.
    pub fn fields(&self) -> &Hash {
        &self.fields
    }

    /// The Markdown after the closing fence.
    pub fn body(&self) -> &str {
        &self.body
    }

    /// Replace the body.
    pub fn set_body(&mut self, body: impl Into<String>) {
        self.body = body.into();
    }

    /// The file contents: byte-identical to the input unless a field was set.
    pub fn render(&self) -> String {
        let yaml = if self.modified {
            let mut out = String::new();
            YamlEmitter::new(&mut out)
                .dump(&Yaml::Hash(self.fields.clone()))
                .expect("emitting a mapping of plain values cannot fail");
            // The emitter starts with its own `---` line; we keep the original fences.
            let out = out
                .strip_prefix("---")
                .unwrap_or(&out)
                .trim_start_matches('\n');
            let eol = if self.open.ends_with("\r\n") {
                "\r\n"
            } else {
                "\n"
            };
            out.lines().map(|l| format!("{l}{eol}")).collect()
        } else {
            self.yaml_text.clone()
        };
        let close = if self.close.ends_with('\n') || self.body.is_empty() {
            self.close.clone()
        } else {
            format!("{}\n", self.close)
        };
        format!("{}{yaml}{close}{}", self.open, self.body)
    }

    /// The 1-based file line declaring `path` (`["x-kipi", "scope"]`), found by
    /// indentation; falls back to the nearest declared ancestor.
    ///
    /// Best effort, for error locations only: it scans block-style `key:` lines rather
    /// than walking the parsed tree (yaml-rust2's `Yaml` carries no positions), so flow
    /// mappings, multi-line keys or anchors resolve to the nearest ancestor's line.
    pub fn line_of(&self, path: &[&str]) -> Option<usize> {
        let mut found = None;
        let mut depth = 0;
        let mut parent_indent: Option<usize> = None;
        for (i, line) in self.yaml_text.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let indent = line.len() - trimmed.len();
            if parent_indent.is_some_and(|p| indent <= p) {
                break;
            }
            let Some(key) = path.get(depth) else { break };
            let key_matches = trimmed
                .strip_prefix(key)
                .is_some_and(|rest| rest.trim_start().starts_with(':'));
            if key_matches && (depth == 0) == (indent == 0) {
                found = Some(i + 2);
                depth += 1;
                parent_indent = Some(indent);
                if depth == path.len() {
                    break;
                }
            }
        }
        found
    }
}

/// Whether `name` is a valid Agent Skills name: lowercase letters, digits and `-`, at
/// most [`MAX_NAME_LEN`] characters.
pub fn is_skill_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME_LEN
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// The fields kipi reads from a `SKILL.md` frontmatter (roadmap §5.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillMeta {
    /// `name`.
    pub name: String,
    /// `description`.
    pub description: Option<String>,
    /// `x-kipi.scope`.
    pub scope: Option<String>,
    /// `x-kipi.tags`.
    pub tags: Vec<String>,
}

impl SkillMeta {
    /// Validate and extract the skill fields of `fm`, read from `path`.
    pub fn from_frontmatter(path: &Path, fm: &Frontmatter) -> Result<Self> {
        let at = |keys: &[&str]| Location::maybe_at(path, fm.line_of(keys).or(Some(2)));
        let string = |keys: &[&str], value: &Yaml| match value {
            Yaml::String(s) => Ok(s.clone()),
            _ => Err(Error::invalid(
                at(keys),
                format!("`{}` must be a string", keys.join(".")),
                "write a single text value, quoted if it contains `:` or `#`",
            )),
        };

        let name = match fm.get("name") {
            Some(v) => string(&["name"], v)?,
            None => {
                return Err(Error::invalid(
                    Location::at(path, 2),
                    "missing `name`",
                    "add `name: <skill directory name>` to the frontmatter",
                ));
            }
        };
        if !is_skill_name(&name) {
            return Err(Error::invalid(
                at(&["name"]),
                format!("invalid skill name `{name}`"),
                format!("use lowercase letters, digits and `-`, at most {MAX_NAME_LEN} characters"),
            ));
        }
        let description = fm
            .get("description")
            .map(|v| string(&["description"], v))
            .transpose()?;

        let (mut scope, mut tags) = (None, Vec::new());
        match fm.get("x-kipi") {
            None | Some(Yaml::Null) => {}
            Some(Yaml::Hash(x)) => {
                for (key, value) in x {
                    let key = key.as_str().unwrap_or_default();
                    match key {
                        "scope" => scope = Some(string(&["x-kipi", "scope"], value)?),
                        "tags" => {
                            tags = string_list(value).ok_or_else(|| {
                                Error::invalid(
                                    at(&["x-kipi", "tags"]),
                                    "`x-kipi.tags` must be a list of strings",
                                    "write it as `tags: [data, sql]`",
                                )
                            })?
                        }
                        other => {
                            return Err(Error::UnknownKey {
                                at: at(&["x-kipi", other]),
                                key: other.to_owned(),
                                table: "x-kipi".to_owned(),
                                expected: X_KIPI_KEYS.iter().map(|k| (*k).to_owned()).collect(),
                            });
                        }
                    }
                }
            }
            Some(_) => {
                return Err(Error::invalid(
                    at(&["x-kipi"]),
                    "`x-kipi` must be a mapping",
                    "nest its fields: `x-kipi:` then `  scope: <scope>` on the next line",
                ));
            }
        }
        Ok(SkillMeta {
            name,
            description,
            scope,
            tags,
        })
    }
}

fn string_list(value: &Yaml) -> Option<Vec<String>> {
    value
        .as_vec()?
        .iter()
        .map(|v| v.as_str().map(str::to_owned))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SKILL: &str = "---\nname: dbt-conventions\ndescription: Use when writing dbt.\n\
                         license: MIT\nx-kipi:\n  scope: acme\n  tags: [data, sql]\n---\n\n# body\n";

    fn parse(text: &str) -> Result<Frontmatter> {
        Frontmatter::parse(Path::new("SKILL.md"), text)
    }

    #[test]
    fn untouched_round_trip_is_byte_stable() {
        for text in [
            SKILL,
            "---\nname: x\n---\n",
            "---\nname: x\n---",
            "---\r\nname: x  # comment\r\n\r\n---\r\nbody\r\n",
            "---\n---\nempty frontmatter\n",
        ] {
            assert_eq!(parse(text).unwrap().render(), text);
        }
    }

    #[test]
    fn modified_fields_are_reemitted_in_order() {
        let mut fm = parse(SKILL).unwrap();
        fm.set("description", Yaml::String("Updated.".into()));
        let out = fm.render();
        assert!(
            out.starts_with("---\nname: dbt-conventions\ndescription: Updated.\nlicense: MIT\n"),
            "{out}"
        );
        assert!(out.ends_with("---\n\n# body\n"), "{out}");
        let again = parse(&out).unwrap();
        assert_eq!(again.fields(), fm.fields());
        assert_eq!(again.body(), "\n# body\n");
    }

    #[test]
    fn skill_meta_reads_standard_and_x_kipi_fields() {
        let meta = SkillMeta::from_frontmatter(Path::new("SKILL.md"), &parse(SKILL).unwrap());
        assert_eq!(
            meta.unwrap(),
            SkillMeta {
                name: "dbt-conventions".into(),
                description: Some("Use when writing dbt.".into()),
                scope: Some("acme".into()),
                tags: vec!["data".into(), "sql".into()],
            }
        );
    }

    #[test]
    fn line_of_finds_nested_keys() {
        let fm = parse(SKILL).unwrap();
        assert_eq!(fm.line_of(&["name"]), Some(2));
        assert_eq!(fm.line_of(&["x-kipi"]), Some(5));
        assert_eq!(fm.line_of(&["x-kipi", "tags"]), Some(7));
        assert_eq!(fm.line_of(&["x-kipi", "nope"]), Some(5));
        assert_eq!(fm.line_of(&["nope"]), None);
    }

    #[test]
    fn missing_or_unclosed_frontmatter_is_an_error() {
        assert!(parse("# just markdown\n").is_err());
        assert!(parse("---\nname: x\n").is_err());
        assert!(parse("").is_err());
    }
}
