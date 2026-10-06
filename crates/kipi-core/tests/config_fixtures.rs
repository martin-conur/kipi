//! Every schema key of `bindings.toml`, `machine.toml`, `config.toml` and `SKILL.md`
//! has a positive and a negative fixture under `tests/fixtures/config/`; the loaded
//! value or the error report of each is snapshotted.
//!
//! Fixture stems are `<key>` or `<key>-<what>` (`scopes-roots`, `scopes-roots-relative`),
//! which is what [`every_key_has_a_positive_and_a_negative_case`] checks.

use std::fs;
use std::path::{Path, PathBuf};

use kipi_core::config::{self, Env};
use kipi_core::error::Error;
use tempfile::TempDir;

/// What `~` expands to in these tests: never the real home directory.
const HOME: &str = "/HOME";

const BINDINGS_KEYS: &[&str] = &[
    "store-name",
    "store-version",
    "store-share_skills",
    "defaults-budget_tokens",
    "defaults-agents",
    "defaults-grant",
    "scopes-extends",
    "scopes-roots",
    "scopes-skills",
    "scopes-always",
    "scopes-registry",
    "scopes-remote_patterns",
    "scopes-agents",
    "scopes-grant",
    "scopes-budget_tokens",
    "repos-remote",
    "repos-summary",
    "repos-aliases",
    "repos-related",
    "repos-skills",
    "repos-exclude",
    "repos-scope",
    "repos-agents",
    "repos-grant",
    "repos-budget_tokens",
];
const MACHINE_KEYS: &[&str] = &["hostname", "roots", "repos"];
const CONFIG_KEYS: &[&str] = &["stores", "editor", "tokenizer"];
const SKILL_KEYS: &[&str] = &[
    "name",
    "description",
    "x-kipi",
    "x-kipi-scope",
    "x-kipi-tags",
];

/// The minimal valid bindings the skill cases are loaded with.
const MINIMAL_BINDINGS: &str = "[store]\nname = \"fx\"\nversion = 1\n";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/config")
}

/// `(stem, path)` of every fixture in `<kind>/<valid|invalid>/`, sorted.
fn cases(kind: &str, valid: bool) -> Vec<(String, PathBuf)> {
    let dir = fixtures()
        .join(kind)
        .join(if valid { "valid" } else { "invalid" });
    let mut out: Vec<_> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| {
            let path = e.unwrap().path();
            let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
            (stem, path)
        })
        .collect();
    out.sort();
    assert!(!out.is_empty(), "no fixtures in {}", dir.display());
    out
}

fn env(config_dir: &Path) -> Env {
    let mut env = Env::from_lookup(|_| None, Some(PathBuf::from(HOME))).unwrap();
    env.config_dir = config_dir.to_path_buf();
    env
}

/// A copy of `base-store` with the given `bindings.toml` and, optionally, a
/// `skills/case/SKILL.md`.
fn store_with(bindings: &str, skill: Option<&str>) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    copy_dir(&fixtures().join("base-store"), dir.path());
    fs::write(dir.path().join("bindings.toml"), bindings).unwrap();
    if let Some(skill) = skill {
        let case = dir.path().join("skills/case");
        fs::create_dir_all(&case).unwrap();
        fs::write(case.join("SKILL.md"), skill).unwrap();
    }
    dir
}

fn copy_dir(src: &Path, dest: &Path) {
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let target = dest.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            fs::create_dir_all(&target).unwrap();
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

/// Replace the tempdir prefix so snapshots are stable.
fn redact(text: &str, dir: &Path) -> String {
    text.replace(&dir.display().to_string(), "<dir>")
}

/// The error report, after checking it carries a location and a hint.
fn report(err: &Error, dir: &Path) -> String {
    assert!(err.location().is_some(), "error has a location: {err}");
    assert!(!err.hint().trim().is_empty(), "error has a hint: {err}");
    redact(&err.report(), dir)
}

#[test]
fn bindings_valid() {
    for (stem, path) in cases("bindings", true) {
        let dir = store_with(&fs::read_to_string(&path).unwrap(), None);
        let store = config::load_store(dir.path(), &env(dir.path()))
            .unwrap_or_else(|e| panic!("{stem}: {}", e.report()));
        let out = format!(
            "name: {}\nversion: {}\nshare_skills: {}\ndefaults: {:#?}\nscopes: {:#?}\nrepos: {:#?}",
            store.name,
            store.version,
            store.share_skills,
            store.defaults,
            store.scopes,
            store.repos
        );
        insta::assert_snapshot!(format!("bindings-valid-{stem}"), redact(&out, dir.path()));
    }
}

#[test]
fn bindings_invalid() {
    for (stem, path) in cases("bindings", false) {
        let dir = store_with(&fs::read_to_string(&path).unwrap(), None);
        let err = config::load_store(dir.path(), &env(dir.path()))
            .err()
            .unwrap_or_else(|| panic!("{stem} should fail to load"));
        insta::assert_snapshot!(format!("bindings-invalid-{stem}"), report(&err, dir.path()));
    }
}

#[test]
fn skill_valid() {
    for (stem, path) in cases("skill", true) {
        let dir = store_with(MINIMAL_BINDINGS, Some(&fs::read_to_string(&path).unwrap()));
        let store = config::load_store(dir.path(), &env(dir.path()))
            .unwrap_or_else(|e| panic!("{stem}: {}", e.report()));
        let skill = store.skill("case").expect("case skill loaded");
        let out = format!("{skill:#?}");
        insta::assert_snapshot!(format!("skill-valid-{stem}"), redact(&out, dir.path()));
    }
}

#[test]
fn skill_invalid() {
    for (stem, path) in cases("skill", false) {
        let dir = store_with(MINIMAL_BINDINGS, Some(&fs::read_to_string(&path).unwrap()));
        let err = config::load_store(dir.path(), &env(dir.path()))
            .err()
            .unwrap_or_else(|| panic!("{stem} should fail to load"));
        insta::assert_snapshot!(format!("skill-invalid-{stem}"), report(&err, dir.path()));
    }
}

/// Write `path`'s contents as `<tempdir>/<file>` and return the tempdir.
fn config_dir_with(file: &str, path: &Path) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::copy(path, dir.path().join(file)).unwrap();
    dir
}

#[test]
fn machine_valid() {
    for (stem, path) in cases("machine", true) {
        let dir = config_dir_with(config::MACHINE_FILE, &path);
        let machine = config::load_machine(&env(dir.path()))
            .unwrap_or_else(|e| panic!("{stem}: {}", e.report()));
        let out = format!(
            "hostname: {:?}\nroots: {:#?}\nrepos: {:#?}",
            machine.hostname, machine.roots, machine.repos
        );
        insta::assert_snapshot!(format!("machine-valid-{stem}"), redact(&out, dir.path()));
    }
}

#[test]
fn machine_invalid() {
    for (stem, path) in cases("machine", false) {
        let dir = config_dir_with(config::MACHINE_FILE, &path);
        let err = config::load_machine(&env(dir.path()))
            .err()
            .unwrap_or_else(|| panic!("{stem} should fail to load"));
        insta::assert_snapshot!(format!("machine-invalid-{stem}"), report(&err, dir.path()));
    }
}

#[test]
fn config_valid() {
    for (stem, path) in cases("config", true) {
        let dir = config_dir_with(config::CONFIG_FILE, &path);
        let cfg = config::load_config(&env(dir.path()))
            .unwrap_or_else(|e| panic!("{stem}: {}", e.report()));
        let out = format!(
            "stores: {:#?}\neditor: {:?}\ntokenizer: {:?}",
            cfg.stores, cfg.editor, cfg.tokenizer
        );
        insta::assert_snapshot!(format!("config-valid-{stem}"), redact(&out, dir.path()));
    }
}

#[test]
fn config_invalid() {
    for (stem, path) in cases("config", false) {
        let dir = config_dir_with(config::CONFIG_FILE, &path);
        let err = config::load_config(&env(dir.path()))
            .err()
            .unwrap_or_else(|| panic!("{stem} should fail to load"));
        insta::assert_snapshot!(format!("config-invalid-{stem}"), report(&err, dir.path()));
    }
}

#[test]
fn every_key_has_a_positive_and_a_negative_case() {
    for (kind, keys) in [
        ("bindings", BINDINGS_KEYS),
        ("machine", MACHINE_KEYS),
        ("config", CONFIG_KEYS),
        ("skill", SKILL_KEYS),
    ] {
        for valid in [true, false] {
            let stems: Vec<String> = cases(kind, valid).into_iter().map(|(s, _)| s).collect();
            for key in keys {
                let covered = stems
                    .iter()
                    .any(|s| s == key || s.starts_with(&format!("{key}-")));
                let which = if valid { "valid" } else { "invalid" };
                assert!(covered, "{kind}: no {which} fixture for `{key}`");
            }
        }
    }
}

#[test]
fn missing_machine_and_config_files_load_empty() {
    let dir = tempfile::tempdir().unwrap();
    let env = env(dir.path());
    let machine = config::load_machine(&env).unwrap();
    assert!(machine.repos.is_empty() && machine.roots.is_empty() && machine.source.is_none());
    let cfg = config::load_config(&env).unwrap();
    assert!(cfg.stores.is_empty() && cfg.source.is_none());
}

#[test]
fn a_directory_without_bindings_is_not_a_store() {
    let dir = tempfile::tempdir().unwrap();
    let err = config::load_store(dir.path(), &env(dir.path())).unwrap_err();
    insta::assert_snapshot!(report(&err, dir.path()), @r"
    error: <dir>/bindings.toml: no bindings.toml: not a kipi store
      hint: create a store with `kipi init --store <path>`, or point --store / KIPI_STORE at an existing one
    ");
}

#[test]
fn a_skill_directory_without_skill_md_is_an_error() {
    let dir = store_with(MINIMAL_BINDINGS, None);
    fs::create_dir_all(dir.path().join("skills/empty")).unwrap();
    // Hidden entries and plain files under skills/ are ignored.
    fs::create_dir_all(dir.path().join("skills/.cache")).unwrap();
    fs::write(dir.path().join("skills/README.md"), "not a skill").unwrap();
    let err = config::load_store(dir.path(), &env(dir.path())).unwrap_err();
    insta::assert_snapshot!(report(&err, dir.path()), @r"
    error: <dir>/skills/empty: skill directory has no SKILL.md
      hint: add one (`kipi add skill <name>` writes a template) or remove the directory
    ");
}

#[test]
fn bindings_documents_are_retained_for_write_back() {
    let text = fs::read_to_string(fixtures().join("bindings/valid/repos-summary.toml")).unwrap();
    let dir = store_with(&text, None);
    let store = config::load_store(dir.path(), &env(dir.path())).unwrap();
    assert_eq!(store.bindings.path, dir.path().join("bindings.toml"));
    assert_eq!(store.bindings.doc.to_string(), text, "comments survive");
}
