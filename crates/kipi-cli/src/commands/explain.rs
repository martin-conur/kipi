//! `kipi explain <repo>`: every resolution decision for one repo, each with the rule
//! and the line it came from (roadmap §6): `code-review  ← scopes.acme.skills
//! (acme:bindings.toml:13)`. `--json` prints the same plan as `kipi resolve <repo> --json`.

use std::path::Path;

use kipi_core::model::StoreSet;
use kipi_core::plan::{AlwaysOn, GrantKind, RepoPlan, SkillOrigin, Source};

use super::Loaded;
use crate::error::Failure;
use crate::output::color::styles;
use crate::output::table::Table;
use crate::output::{Output, display_path};

/// A skill inherited from the scope chain.
pub const INHERITED: &str = "○";
/// A skill the repo binds itself.
pub const EXPLICIT: &str = "●";
/// A skill the repo excludes.
pub const EXCLUDED: &str = "✗";

/// Explain repo entry `name`.
pub fn run(loaded: &Loaded, name: &str, out: &Output) -> Result<(), Failure> {
    if out.json {
        return super::resolve::run(loaded, Some(name), out);
    }
    let repo = loaded.repo(name)?;
    out.print(&explain(loaded, repo, out))?;
    Ok(())
}

/// `← rule (store:file:line)`, or `← rule` for a decision no file states.
fn because(set: &StoreSet, source: &Source) -> String {
    match &source.at {
        Some(at) => format!("← {} ({})", source.rule, set.provenance(at)),
        None => format!("← {}", source.rule),
    }
}

/// `path` relative to `root` when it lies inside it, else as [`display_path`] shows it
/// (`~/…` under `home`), so a home directory never reaches the output.
fn within(path: &Path, root: &Path, home: Option<&Path>) -> String {
    path.strip_prefix(root)
        .map_or_else(|_| display_path(path, home), |p| p.display().to_string())
}

fn explain(loaded: &Loaded, repo: &RepoPlan, out: &Output) -> String {
    let set = &loaded.stores;
    let plan = &loaded.plan;
    let p = &out.paint;
    let src = |s: &Source| (because(set, s), styles::source());
    let plain = |s: String| (s, styles::none());
    let name = |s: String| (s, styles::name());
    let store_root = |store: &str| set.store(store).map(|s| s.root.as_path());
    let mut text = String::new();
    // A heading, its table (or `(none)`), then free lines that should not widen it.
    let mut section = |title: &str, table: &Table, notes: &[String]| {
        text.push('\n');
        text.push_str(&p.heading(title));
        text.push('\n');
        if table.is_empty() {
            text.push_str(&p.dim("  (none)\n"));
        } else {
            text.push_str(&table.render(p));
        }
        for note in notes {
            text.push_str(&format!("  {note}\n"));
        }
    };

    // Header.
    let mut t = Table::new(2);
    t.row([plain("entry".into()), plain(set.provenance(&repo.at))]);
    t.row([plain("store".into()), plain(repo.store.clone())]);
    t.row([
        plain("checkout".into()),
        plain(match &repo.path {
            Some(path) => out.path(path),
            None => "(not cloned here)".into(),
        }),
    ]);
    let mut head = format!("{}\n", p.heading(&repo.name));
    head.push_str(&t.render(p));

    // Scope.
    let mut t = Table::new(2);
    t.row([
        name(
            repo.scope
                .value
                .clone()
                .unwrap_or_else(|| "(unscoped)".into()),
        ),
        src(&repo.scope.source),
    ]);
    let mut notes = Vec::new();
    if !repo.chain.is_empty() {
        notes.push(format!("chain: {}", repo.chain.join(" → ")));
    }
    section("Scope", &t, &notes);

    // Skills.
    let mut t = Table::new(2);
    for s in &repo.skills {
        let (marker, style) = match s.origin {
            SkillOrigin::Scope(_) => (INHERITED, styles::none()),
            SkillOrigin::Repo => (EXPLICIT, styles::explicit()),
        };
        t.row([(format!("{marker} {}", s.reference), style), src(&s.source)]);
    }
    for s in &repo.excluded {
        t.row([
            (format!("{EXCLUDED} {}", s.reference), styles::removed()),
            src(&s.source),
        ]);
    }
    section(
        &format!("Skills  {INHERITED} inherited  {EXPLICIT} explicit  {EXCLUDED} excluded"),
        &t,
        &[],
    );

    // Always-on content.
    let mut t = Table::new(2);
    for a in &repo.always {
        match a {
            AlwaysOn::File { path, source, .. } => {
                let shown = store_root(&repo.store)
                    .map_or_else(|| out.path(path), |r| within(path, r, out.home.as_deref()));
                t.row([plain(shown), src(source)]);
            }
            AlwaysOn::Registry { scope, source } => {
                t.row([plain(format!("registry of {scope}")), src(source)]);
            }
            AlwaysOn::ThisRepo => {
                t.row([
                    plain("this-repo section".into()),
                    plain("← built in".into()),
                ]);
            }
        }
    }
    section("Always-on", &t, &[]);

    // Registry.
    let mut t = Table::new(2);
    let mut notes = Vec::new();
    if let (Some(decided), Some(reg)) = (&repo.registry, plan.registry_of(repo)) {
        t.row([name(decided.value.clone()), src(&decided.source)]);
        let n = reg.entries.len();
        notes.push(format!(
            "{n} repo{} in scope{} {}",
            if n == 1 { "" } else { "s" },
            if reg.scopes.len() == 1 { "" } else { "s" },
            reg.scopes.join(", ")
        ));
        for c in &reg.collisions {
            notes.push(format!(
                "alias `{}` collides: {}",
                c.key,
                c.repos.join(", ")
            ));
        }
    }
    section("Registry", &t, &notes);

    // Related repos.
    let mut t = Table::new(2);
    for r in &repo.related {
        t.row([
            name(r.name.clone()),
            plain(r.scope.clone().unwrap_or_else(|| "(unscoped)".into())),
            plain(match &r.path {
                Some(path) => out.path(path),
                None => "(not cloned here)".into(),
            }),
            plain(if r.notes.is_some() {
                "notes".into()
            } else {
                String::new()
            }),
            plain(match &r.via {
                Some(via) => format!("via: {via}"),
                None => String::new(),
            }),
            src(&r.source),
        ]);
    }
    section("Related", &t, &[]);

    // Read grant.
    let mut t = Table::new(2);
    for d in &repo.grant.dirs {
        let what = match &d.kind {
            GrantKind::StoreRepos => format!("store {} repos/", repo.store),
            GrantKind::StoreSkills { store } => format!("store {store} skills/"),
            GrantKind::Repo { name } => format!("repo {name}"),
        };
        t.row([plain(out.path(&d.path)), plain(what), src(&d.source)]);
    }
    section(
        &format!(
            "Grant: {}  {}",
            repo.grant.mode.value,
            because(set, &repo.grant.mode.source)
        ),
        &t,
        &[],
    );

    // Precedence: repo > scope chain > defaults > built-in.
    let mut t = Table::new(2);
    let agents: Vec<&str> = repo.agents.value.iter().map(|a| a.as_str()).collect();
    t.row([
        plain("agents".into()),
        plain(agents.join(", ")),
        src(&repo.agents.source),
    ]);
    t.row([
        plain("budget_tokens".into()),
        plain(repo.budget_tokens.value.to_string()),
        src(&repo.budget_tokens.source),
    ]);
    t.row([
        plain("grant".into()),
        plain(repo.grant.mode.value.to_string()),
        src(&repo.grant.mode.source),
    ]);
    section(
        "Settings  (repo > scope chain > defaults > built-in)",
        &t,
        &[],
    );

    head + &text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn within_is_store_relative_and_never_shows_the_home_outside_it() {
        let home = Some(Path::new("/h/me"));
        let root = Path::new("/h/me/stores/acme");
        let inside = Path::new("/h/me/stores/acme/conventions/acme.md");
        assert_eq!(within(inside, root, home), "conventions/acme.md");
        let outside = Path::new("/h/me/elsewhere/acme.md");
        assert_eq!(within(outside, root, home), "~/elsewhere/acme.md");
        assert_eq!(within(Path::new("/srv/x.md"), root, home), "/srv/x.md");
    }
}
