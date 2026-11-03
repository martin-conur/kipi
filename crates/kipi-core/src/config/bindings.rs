//! `bindings.toml` (roadmap §5.1): one store's name, defaults, scopes and repo entries.

use std::path::{Component, Path, PathBuf};

use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};

use super::paths::{ABSOLUTE_PATH_HINT, Env, absolute_path};
use super::skills::load_skills;
use super::source::{Entries, Source};
use super::{BINDINGS_FILE, NAME_HINT};
use crate::error::{Error, Location, Result};
use crate::model::{
    AgentId, Attribute, Defaults, Drafted, GrantMode, Related, RemoteUrl, RepoEntry, STORE_VERSION,
    Scope, SkillRef, Store, SyncCommand, check_attribute_key, check_attribute_value, check_name,
    check_via,
};
use crate::on_sync;

/// Every key `bindings.toml` accepts (see [`super::schema`]).
pub(super) fn schema_keys() -> Vec<String> {
    use super::schema::{fields, under};
    fields::<RawBindings>()
        .iter()
        .flat_map(|&table| match table {
            "store" => under::<RawStore>(table),
            "defaults" => under::<RawDefaults>(table),
            "scopes" => {
                let mut keys = under::<RawScope>("scopes.<scope>");
                keys.extend(under::<RawOnSync>("scopes.<scope>.on_sync[]"));
                keys
            }
            "repos" => {
                let mut keys = under::<RawRepo>("repos.<repo>");
                keys.extend(under::<RawRelatedTable>("repos.<repo>.related[]"));
                keys.extend(under::<RawOnSync>("repos.<repo>.on_sync[]"));
                keys
            }
            other => vec![other.to_owned()],
        })
        .collect()
}

const TYPE_HINT: &str = "change the value to the expected type";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBindings {
    store: RawStore,
    #[serde(default)]
    defaults: RawDefaults,
    #[serde(default)]
    scopes: Entries<RawScope>,
    #[serde(default)]
    repos: Entries<RawRepo>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStore {
    name: String,
    version: i64,
    #[serde(default)]
    share_skills: bool,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawDefaults {
    budget_tokens: Option<i64>,
    agents: Option<Vec<AgentId>>,
    grant: Option<GrantMode>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawScope {
    extends: Option<String>,
    #[serde(default)]
    roots: Vec<String>,
    #[serde(default)]
    skills: Vec<String>,
    #[serde(default)]
    always: Vec<String>,
    #[serde(default)]
    registry: bool,
    #[serde(default)]
    remote_patterns: Vec<String>,
    agents: Option<Vec<AgentId>>,
    grant: Option<GrantMode>,
    budget_tokens: Option<i64>,
    /// Read off the spanned document instead ([`Validator::attributes`]).
    #[allow(dead_code)]
    attributes: Option<de::IgnoredAny>,
    #[serde(default)]
    on_sync: Vec<RawOnSync>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRepo {
    remote: Option<String>,
    summary: Option<String>,
    aliases: Option<Vec<String>>,
    #[serde(default)]
    related: Vec<RawRelated>,
    #[serde(default)]
    skills: Vec<String>,
    #[serde(default)]
    exclude: Vec<String>,
    scope: Option<String>,
    agents: Option<Vec<AgentId>>,
    grant: Option<GrantMode>,
    budget_tokens: Option<i64>,
    /// Read off the spanned document instead ([`Validator::attributes`]).
    #[allow(dead_code)]
    attributes: Option<de::IgnoredAny>,
    #[serde(default)]
    on_sync: Vec<RawOnSync>,
}

/// One `[[scopes.<scope>.on_sync]]` / `[[repos.<repo>.on_sync]]` table.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOnSync {
    name: String,
    run: StrOrList,
    when: Option<StrOrList>,
    worktrees: Option<bool>,
}

/// A string or an array of strings (`run`, `when`).
struct StrOrList {
    items: Vec<String>,
    list: bool,
}

impl<'de> Deserialize<'de> for StrOrList {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = StrOrList;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a string or an array of strings")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(StrOrList {
                    items: vec![v.to_owned()],
                    list: false,
                })
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut items = Vec::new();
                while let Some(item) = seq.next_element::<String>()? {
                    items.push(item);
                }
                Ok(StrOrList { items, list: true })
            }
        }
        d.deserialize_any(V)
    }
}

/// One `related` item: a repo name, or `{ repo = "…", via = "…" }`.
struct RawRelated(RawRelatedTable);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRelatedTable {
    repo: String,
    via: Option<String>,
}

impl<'de> Deserialize<'de> for RawRelated {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = RawRelated;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a repo name or a `{ repo = \"…\", via = \"…\" }` table")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(RawRelated(RawRelatedTable {
                    repo: v.to_owned(),
                    via: None,
                }))
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
                RawRelatedTable::deserialize(de::value::MapAccessDeserializer::new(map))
                    .map(RawRelated)
            }
        }
        d.deserialize_any(V)
    }
}

/// Load the store at `root`: its `bindings.toml`, `skills/*/SKILL.md` and
/// `repos/*/notes.md` presence.
pub fn load_store(root: &Path, env: &Env) -> Result<Store> {
    let path = root.join(BINDINGS_FILE);
    let src = Source::read_optional(&path)?.ok_or_else(|| {
        Error::invalid(
            Location::file(&path),
            "no bindings.toml: not a kipi store",
            "create a store with `kipi init --store <path>`, or point --store / KIPI_STORE \
             at an existing one",
        )
    })?;
    let mut store = parse_store(root, src, &env.home)?;
    store.skills = load_skills(root)?;
    store.tag_locations();
    Ok(store)
}

/// Load the store at `root` as if its `bindings.toml` held `text` (skills and notes are
/// read from disk). The authoring commands validate an edit with it before writing.
pub fn load_store_text(root: &Path, text: String, env: &Env) -> Result<Store> {
    let src = Source::parse(&root.join(BINDINGS_FILE), text)?;
    let mut store = parse_store(root, src, &env.home)?;
    store.skills = load_skills(root)?;
    store.tag_locations();
    Ok(store)
}

/// Validate a parsed `bindings.toml` into a [`Store`] (skills not loaded).
fn parse_store(root: &Path, src: Source, home: &Path) -> Result<Store> {
    check_version(&src)?;
    let raw: RawBindings = src.deserialize(TYPE_HINT)?;
    let v = Validator { src: &src };

    v.name(&["store", "name"], &raw.store.name, "store")?;
    let defaults = Defaults {
        budget_tokens: v.budget(&["defaults", "budget_tokens"], raw.defaults.budget_tokens)?,
        agents: raw.defaults.agents,
        grant: raw.defaults.grant,
        at: src
            .doc
            .contains_key("defaults")
            .then(|| src.at(&["defaults"])),
    };

    let mut scopes = Vec::with_capacity(raw.scopes.0.len());
    for (name, s) in raw.scopes.0 {
        let k = |key: &'static str| ["scopes", name.as_str(), key];
        v.name(&["scopes", &name], &name, "scope")?;
        if let Some(parent) = &s.extends {
            v.name(&k("extends"), parent, "scope")?;
        }
        let mut roots = Vec::with_capacity(s.roots.len());
        for r in &s.roots {
            roots.push(
                absolute_path(r, home)
                    .map_err(|m| Error::invalid(src.at(&k("roots")), m, ABSOLUTE_PATH_HINT))?,
            );
        }
        let mut always = Vec::with_capacity(s.always.len());
        for a in &s.always {
            always.push(v.store_relative(&k("always"), a)?);
        }
        for p in &s.remote_patterns {
            v.non_empty(&k("remote_patterns"), p, "remote pattern")?;
        }
        scopes.push(Scope {
            skills: v.skill_refs(&k("skills"), &s.skills)?,
            budget_tokens: v.budget(&k("budget_tokens"), s.budget_tokens)?,
            attributes: v.attributes(&["scopes", &name])?,
            on_sync: v.on_sync(&k("on_sync"), s.on_sync)?,
            at: src.at(&["scopes", &name]),
            name,
            extends: s.extends,
            roots,
            always,
            registry: s.registry,
            remote_patterns: s.remote_patterns,
            agents: s.agents,
            grant: s.grant,
        });
    }

    let mut repos = Vec::with_capacity(raw.repos.0.len());
    for (name, r) in raw.repos.0 {
        let k = |key: &'static str| ["repos", name.as_str(), key];
        v.name(&["repos", &name], &name, "repo")?;
        let remote = r
            .remote
            .as_deref()
            .map(|remote| v.remote(&k("remote"), remote))
            .transpose()?;
        if let Some(summary) = &r.summary {
            v.summary(&k("summary"), summary)?;
        }
        for alias in r.aliases.iter().flatten() {
            v.non_empty(&k("aliases"), alias, "alias")?;
        }
        let mut related = Vec::with_capacity(r.related.len());
        for RawRelated(rel) in r.related {
            v.name(&k("related"), &rel.repo, "repo")?;
            if let Some(via) = &rel.via {
                v.via(&k("related"), &rel.repo, via)?;
            }
            related.push(Related {
                name: rel.repo,
                via: rel.via,
            });
        }
        if let Some(scope) = &r.scope {
            v.name(&k("scope"), scope, "scope")?;
        }
        let notes = root.join("repos").join(&name).join("notes.md");
        repos.push(RepoEntry {
            summary: r.summary.map(|value| Drafted {
                value,
                is_draft: v.is_draft(&k("summary")),
            }),
            aliases: Drafted {
                is_draft: r.aliases.is_some() && v.is_draft(&k("aliases")),
                value: r.aliases.unwrap_or_default(),
            },
            skills: v.skill_refs(&k("skills"), &r.skills)?,
            exclude: v.skill_refs(&k("exclude"), &r.exclude)?,
            budget_tokens: v.budget(&k("budget_tokens"), r.budget_tokens)?,
            attributes: v.attributes(&["repos", &name])?,
            on_sync: v.on_sync(&k("on_sync"), r.on_sync)?,
            notes: notes.is_file().then_some(notes),
            at: src.at(&["repos", &name]),
            name,
            remote,
            related,
            scope: r.scope,
            agents: r.agents,
            grant: r.grant,
        });
    }

    Ok(Store {
        name_at: src.at(&["store", "name"]),
        name: raw.store.name,
        version: raw.store.version,
        share_skills: raw.store.share_skills,
        root: root.to_path_buf(),
        defaults,
        scopes,
        repos,
        skills: Vec::new(),
        bindings: src.into_doc(),
    })
}

/// Reject a missing `[store]` or an unknown `version` before schema validation, so a
/// store written by a newer kipi reports its version rather than its new keys.
fn check_version(src: &Source) -> Result<()> {
    let Some(store) = src.doc.get("store") else {
        return Err(Error::invalid(
            Location::file(&src.path),
            "missing [store] table",
            "add `[store]` with `name = \"<store name>\"` and `version = 1`",
        ));
    };
    let Some(version) = store.get("version") else {
        return Err(Error::invalid(
            src.at(&["store"]),
            "missing key `version` in [store]",
            format!("add `version = {STORE_VERSION}` to [store]"),
        ));
    };
    // A non-integer falls through to serde, whose type error is already precise.
    match version.as_integer() {
        Some(v) if v != STORE_VERSION => Err(Error::invalid(
            src.at(&["store", "version"]),
            format!("unsupported store version {v}"),
            format!(
                "this kipi reads version {STORE_VERSION}; upgrade kipi if the store was \
                 written by a newer one, otherwise set `version = {STORE_VERSION}`"
            ),
        )),
        _ => Ok(()),
    }
}

/// Value checks that point at the offending key.
struct Validator<'a> {
    src: &'a Source,
}

impl Validator<'_> {
    fn name(&self, keys: &[&str], value: &str, what: &str) -> Result<()> {
        check_name(value).map_err(|why| {
            Error::invalid(
                self.src.at(keys),
                format!("invalid {what} name `{value}`: {why}"),
                NAME_HINT,
            )
        })
    }

    fn non_empty(&self, keys: &[&str], value: &str, what: &str) -> Result<()> {
        if value.trim().is_empty() {
            return Err(Error::invalid(
                self.src.at(keys),
                format!("empty {what}"),
                format!("remove the empty string or write the {what}"),
            ));
        }
        Ok(())
    }

    fn budget(&self, keys: &[&str], value: Option<i64>) -> Result<Option<u32>> {
        value
            .map(|v| {
                u32::try_from(v).ok().filter(|&b| b > 0).ok_or_else(|| {
                    Error::invalid(
                        self.src.at(keys),
                        format!("budget_tokens must be a positive integer, got {v}"),
                        "set a token count such as `budget_tokens = 4000`",
                    )
                })
            })
            .transpose()
    }

    fn skill_refs(&self, keys: &[&str], values: &[String]) -> Result<Vec<SkillRef>> {
        values
            .iter()
            .map(|s| {
                s.parse().map_err(|why| {
                    Error::invalid(
                        self.src.at(keys),
                        format!("invalid skill reference: {why}"),
                        "name a skill in this store (`code-review`) or a shared skill of \
                         another store (`personal/commit-style`)",
                    )
                })
            })
            .collect()
    }

    /// A path inside the store: relative, no `..`, stored canonically (`./a//b` → `a/b`).
    fn store_relative(&self, keys: &[&str], raw: &str) -> Result<PathBuf> {
        let mut path = PathBuf::new();
        let mut inside = true;
        for c in Path::new(raw).components() {
            match c {
                Component::Normal(part) => path.push(part),
                Component::CurDir => {}
                _ => inside = false,
            }
        }
        if !inside || path.as_os_str().is_empty() {
            return Err(Error::invalid(
                self.src.at(keys),
                format!("`{raw}` is not a path inside the store"),
                "write it relative to the store root, e.g. `conventions/<scope>.md`",
            ));
        }
        Ok(path)
    }

    /// `remote` must already be normalized: `host/owner/name`. A raw origin URL is
    /// rejected with its normalized form as the hint.
    fn remote(&self, keys: &[&str], raw: &str) -> Result<RemoteUrl> {
        // The normalized form is exactly what `https://<form>` normalizes back to.
        let as_https = RemoteUrl::parse(&format!("https://{raw}"));
        match as_https {
            Ok(url) if url.as_str() == raw => return Ok(url),
            _ => {}
        }
        let hint = match RemoteUrl::parse(raw).or(as_https) {
            Ok(url) => format!("write it normalized: `remote = \"{url}\"`"),
            Err(_) => "write it as `host/owner/name` (no scheme, user, port or `.git`), e.g. \
                       `github.com/acme-org/billing-api`; `kipi scan` fills it in"
                .to_owned(),
        };
        Err(Error::invalid(
            self.src.at(keys),
            format!("remote `{raw}` is not a normalized origin URL"),
            hint,
        ))
    }

    fn summary(&self, keys: &[&str], raw: &str) -> Result<()> {
        self.non_empty(keys, raw, "summary")?;
        if raw.contains('\n') {
            return Err(Error::invalid(
                self.src.at(keys),
                "summary spans several lines",
                "keep it to one line; longer knowledge belongs in repos/<name>/notes.md",
            ));
        }
        Ok(())
    }

    /// The `via` of the `related` item naming `repo`.
    fn via(&self, keys: &[&str], repo: &str, via: &str) -> Result<()> {
        check_via(via).map_err(|why| {
            Error::invalid(
                self.src.at(keys),
                format!("the `via` of related repo `{repo}` {why}"),
                "keep `via` to one short line (at most 120 characters); longer knowledge \
                 belongs in repos/<name>/notes.md",
            )
        })
    }

    /// The `attributes` of the table at `table` (`["repos", "billing-api"]`), flattened to
    /// dotted keys: `"tracker.project" = "…"` and a nested `tracker.project = "…"` (or
    /// `[repos.<repo>.attributes.tracker]`) are the same key, and writing both is an error.
    /// Values must be strings.
    fn attributes(&self, table: &[&str]) -> Result<Vec<Attribute>> {
        let mut path: Vec<String> = table.iter().map(|k| (*k).to_owned()).collect();
        path.push("attributes".to_owned());
        let mut item = self.src.doc.as_item();
        for key in &path {
            match item.get(key) {
                Some(next) => item = next,
                None => return Ok(Vec::new()),
            }
        }
        if !item.is_table_like() {
            let keys: Vec<&str> = path.iter().map(String::as_str).collect();
            return Err(Error::invalid(
                self.src.at(&keys),
                "`attributes` must be a table",
                "write `attributes = { \"tracker.project\" = \"PROJ\" }`, or set it with \
                 `kipi bind <repo> --attr key=value` / `kipi scope <scope> --attr key=value`",
            ));
        }
        let mut out = Vec::new();
        self.flatten(item, &mut path, &mut Vec::new(), &mut out)?;
        Ok(out)
    }

    fn flatten(
        &self,
        item: &toml_edit::Item,
        path: &mut Vec<String>,
        key: &mut Vec<String>,
        out: &mut Vec<Attribute>,
    ) -> Result<()> {
        fn keys(path: &[String]) -> Vec<&str> {
            path.iter().map(String::as_str).collect()
        }
        let hint = "write `attributes = { \"tracker.project\" = \"PROJ\" }`: dotted keys, \
                    string values; `kipi bind <repo> --attr key=value` writes it for you";
        if let Some(value) = item.as_str() {
            let at = self.src.at(&keys(path));
            let dotted = key.join(".");
            check_attribute_key(&dotted).map_err(|why| {
                Error::invalid(at.clone(), format!("attribute key `{dotted}` {why}"), hint)
            })?;
            check_attribute_value(value).map_err(|why| {
                Error::invalid(
                    at.clone(),
                    format!("the value of attribute `{dotted}` {why}"),
                    "give it a one-line value, or remove the key (`kipi unbind <repo> --attr \
                     <key>`)",
                )
            })?;
            if out.iter().any(|a: &Attribute| a.key == dotted) {
                return Err(Error::invalid(
                    at,
                    format!("attribute `{dotted}` is set twice (quoted and as nested keys)"),
                    "keep one of the two spellings",
                ));
            }
            out.push(Attribute {
                key: dotted,
                value: value.to_owned(),
                at,
            });
            return Ok(());
        }
        let Some(table) = item.as_table_like() else {
            return Err(Error::invalid(
                self.src.at(&keys(path)),
                format!("attribute `{}` must be a string", key.join(".")),
                hint,
            ));
        };
        for (k, child) in table.iter() {
            path.push(k.to_owned());
            key.push(k.to_owned());
            self.flatten(child, path, key, out)?;
            key.pop();
            path.pop();
        }
        Ok(())
    }

    /// The `on_sync` commands of one table, at `keys` (its `on_sync` key: a location
    /// cannot point into an array of tables, so errors name the command instead).
    fn on_sync(&self, keys: &[&str], raw: Vec<RawOnSync>) -> Result<Vec<SyncCommand>> {
        let at = self.src.at(keys);
        let mut out: Vec<SyncCommand> = Vec::with_capacity(raw.len());
        for (i, cmd) in raw.into_iter().enumerate() {
            check_name(&cmd.name).map_err(|why| {
                Error::invalid(
                    at.clone(),
                    format!(
                        "sync command #{} has an invalid name `{}`: {why}",
                        i + 1,
                        cmd.name
                    ),
                    NAME_HINT,
                )
            })?;
            let name = &cmd.name;
            if out.iter().any(|c| c.name == *name) {
                return Err(Error::invalid(
                    at.clone(),
                    format!("two sync commands in this table are named `{name}`"),
                    "rename one: a name is unique within one table's `on_sync` (a repo's \
                     command of the same name as its scope's replaces it)",
                ));
            }
            let words = if cmd.run.list {
                cmd.run.items
            } else {
                on_sync::split(&cmd.run.items[0]).map_err(|why| {
                    Error::invalid(
                        at.clone(),
                        format!("the `run` of sync command `{name}` cannot be split: {why}"),
                        "close the quote or escape the character with `\\`; or write `run` \
                         as an array of arguments",
                    )
                })?
            };
            let argv = on_sync::template(&words).map_err(|e| {
                Error::invalid(
                    at.clone(),
                    format!("sync command `{name}`: {}", e.message),
                    e.hint,
                )
            })?;
            let mut when = Vec::new();
            for key in cmd.when.map(|w| w.items).unwrap_or_default() {
                let key = key.strip_prefix("attr.").unwrap_or(&key).to_owned();
                check_attribute_key(&key).map_err(|why| {
                    Error::invalid(
                        at.clone(),
                        format!("the `when` of sync command `{name}` names attribute key `{key}`, which {why}"),
                        "name an attribute key, e.g. `when = \"tracker.project\"`",
                    )
                })?;
                when.push(key);
            }
            out.push(SyncCommand {
                name: cmd.name,
                argv,
                when,
                worktrees: cmd.worktrees.unwrap_or(false),
                at: at.clone(),
            });
        }
        Ok(out)
    }

    /// The value at `keys` still carries a trailing `# kipi:draft` comment.
    fn is_draft(&self, keys: &[&str]) -> bool {
        self.src.trailing_comment(keys).is_some_and(|c| {
            c.trim()
                .strip_prefix('#')
                .and_then(|c| c.split_whitespace().next())
                == Some("kipi:draft")
        })
    }
}
