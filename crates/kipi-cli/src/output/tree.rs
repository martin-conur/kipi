//! `kipi tree` (roadmap §7): the tree of stores → scopes → repos → skills/related, and
//! its text form. Box drawing falls back to ASCII when stdout is not a terminal; the
//! provenance markers (○ ● ✗ ✓) are plain text and print either way.

use std::path::{Path, PathBuf};

use kipi_core::git::DirStatus;
use kipi_core::sync::AlwaysOnSize;
use kipi_core::tokens::human;
use owo_colors::Style;

use super::color::{Painter, styles};
use super::display_path;
use super::status::State;
use crate::commands::explain::{EXCLUDED, EXPLICIT, INHERITED};

/// Everything `tree` shows, after filtering.
#[derive(Debug)]
pub struct Tree {
    /// The encoding the counts come from.
    pub tokenizer: &'static str,
    /// The mounted stores shown, in mount order.
    pub stores: Vec<StoreNode>,
}

/// One mounted store.
#[derive(Debug)]
pub struct StoreNode {
    /// `[store] name`.
    pub name: String,
    /// The store directory.
    pub root: PathBuf,
    /// Its git state, as `kipi stores` reports it; `None` outside a git work tree.
    pub git: Option<DirStatus>,
    /// `share_skills`.
    pub shares_skills: bool,
    /// Its top-level scopes (no `extends`), in file order.
    pub scopes: Vec<ScopeNode>,
    /// Its repos that resolve to no scope.
    pub unscoped: Vec<RepoNode>,
}

/// One scope, with the scopes extending it.
#[derive(Debug)]
pub struct ScopeNode {
    /// The scope name.
    pub name: String,
    /// `roots`, as declared.
    pub roots: Vec<PathBuf>,
    /// `machine.toml [roots]` for this scope, when it overrides `roots` here.
    pub machine_roots: Option<Vec<PathBuf>>,
    /// `extends`.
    pub extends: Option<String>,
    /// Tokens every repo in the scope pays before its own section and skills: the
    /// chain's conventions, the registry it renders, and the chain's skill descriptions.
    pub always_on: u32,
    /// The scope's own `skills`, as written.
    pub skills: Vec<String>,
    /// The registry this scope builds (`registry = true`).
    pub registry: Option<RegistrySize>,
    /// Scopes extending this one, in file order.
    pub scopes: Vec<ScopeNode>,
    /// Repos resolved to this scope, by name.
    pub repos: Vec<RepoNode>,
}

/// A registry's size.
#[derive(Debug, Clone, Copy)]
pub struct RegistrySize {
    /// Entries.
    pub repos: usize,
    /// Tokens of the rendered registry.
    pub tokens: u32,
}

/// One repo entry.
#[derive(Debug)]
pub struct RepoNode {
    /// The entry name.
    pub name: String,
    /// Its checkout on this machine.
    pub path: Option<PathBuf>,
    /// Always-on tokens vs budget.
    pub always_on: AlwaysOnSize,
    /// Sync state, as `kipi status` reports it.
    pub state: State,
    /// `repos/<name>/notes.md`, when it exists.
    pub notes: Option<PathBuf>,
    /// The summary is still `# kipi:draft`.
    pub draft_summary: bool,
    /// Effective and excluded skills, in scope-chain order.
    pub skills: Vec<SkillNode>,
    /// `related` repos.
    pub related: Vec<RelatedNode>,
    /// Show the skills line in text (the repo binds or excludes skills itself, or a
    /// filter asked for it).
    pub show_skills: bool,
}

/// One skill of a repo.
#[derive(Debug)]
pub struct SkillNode {
    /// The reference as rendered (`personal/commit-style` for a shared skill).
    pub reference: String,
    /// Inherited, explicit or excluded.
    pub mark: Mark,
}

/// The provenance marker of a skill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// ○: bound by a scope of the chain.
    Inherited,
    /// ●: bound by the repo (or, on a scope's skills line, by that scope).
    Explicit,
    /// ✗: excluded by the repo.
    Excluded,
}

impl Mark {
    /// The name used in JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            Mark::Inherited => "inherited",
            Mark::Explicit => "explicit",
            Mark::Excluded => "excluded",
        }
    }

    fn glyph(self) -> (&'static str, Style) {
        match self {
            Mark::Inherited => (INHERITED, styles::source()),
            Mark::Explicit => (EXPLICIT, styles::explicit()),
            Mark::Excluded => (EXCLUDED, styles::removed()),
        }
    }
}

/// One `related` repo.
#[derive(Debug)]
pub struct RelatedNode {
    /// The entry name.
    pub name: String,
    /// Whether the agent may read it.
    pub access: Access,
}

/// Read access to a related repo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Its checkout is in the grant.
    Granted,
    /// Cloned here but not granted (e.g. `grant = "none"`).
    NotGranted,
    /// Not cloned on this machine.
    NotCloned,
}

impl Access {
    /// The name used in JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            Access::Granted => "granted",
            Access::NotGranted => "not_granted",
            Access::NotCloned => "not_cloned",
        }
    }
}

/// What the text shows beyond the default.
#[derive(Debug, Clone, Copy, Default)]
pub struct Display {
    /// ASCII box drawing (stdout is not a terminal).
    pub ascii: bool,
    /// `--budget`: each repo's budget next to its tokens, and an `over budget` tag.
    pub budget: bool,
    /// `--machine`: this machine's roots and checkouts.
    pub machine: bool,
}

/// The widest scope header (box prefix, name, roots, `extends`) that still sets the right
/// column; a wider one puts its token count two spaces after itself instead, so the repo
/// rows stay readable at 100 columns.
const SCOPE_LABEL_CAP: usize = 48;

/// A styled piece of a line.
type Seg = (String, Style);

/// What follows a line's left part.
enum Right {
    None,
    /// Starts at the tree's right column.
    Aligned(Vec<Seg>),
    /// At the right column when the left part is short enough, else two spaces after it.
    Soft(Vec<Seg>),
}

struct Line {
    left: Vec<Seg>,
    right: Right,
}

fn width(segs: &[Seg]) -> usize {
    segs.iter().map(|(t, _)| t.chars().count()).sum()
}

fn plain(text: impl Into<String>) -> Seg {
    (text.into(), styles::none())
}

/// The tree as text, one line per node.
pub fn render(tree: &Tree, display: Display, paint: &Painter, home: Option<&Path>) -> String {
    let mut r = Renderer {
        display,
        home,
        lines: Vec::new(),
        tok_width: 0,
    };
    r.tok_width = tree
        .stores
        .iter()
        .flat_map(|s| s.scopes.iter().flat_map(all_repos).chain(&s.unscoped))
        .map(|repo| r.tokens(&repo.always_on).chars().count())
        .max()
        .unwrap_or(0);
    for store in &tree.stores {
        r.store(store);
    }
    let column = r
        .lines
        .iter()
        .filter(|l| matches!(l.right, Right::Aligned(_)))
        .map(|l| width(&l.left))
        .max()
        .map_or(0, |w| w + 2);
    let mut out = String::new();
    for line in &r.lines {
        let mut text: String = line.left.iter().map(|(t, s)| paint.paint(t, *s)).collect();
        let w = width(&line.left);
        let right = match &line.right {
            Right::None => None,
            Right::Aligned(segs) => Some((column.saturating_sub(w), segs)),
            Right::Soft(segs) => Some((if w + 2 <= column { column - w } else { 2 }, segs)),
        };
        if let Some((pad, segs)) = right {
            text.push_str(&" ".repeat(pad));
            text.extend(segs.iter().map(|(t, s)| paint.paint(t, *s)));
        }
        out.push_str(text.trim_end());
        out.push('\n');
    }
    out
}

fn all_repos(scope: &ScopeNode) -> Box<dyn Iterator<Item = &RepoNode> + '_> {
    Box::new(
        scope
            .repos
            .iter()
            .chain(scope.scopes.iter().flat_map(all_repos)),
    )
}

/// One child of a node, drawn after the node's box prefix.
enum Item<'a> {
    ScopeSkills(&'a ScopeNode),
    Registry(RegistrySize),
    Scope(&'a ScopeNode),
    Repo(&'a RepoNode),
    RepoSkills(&'a RepoNode),
    Related(&'a RepoNode),
}

struct Renderer<'a> {
    display: Display,
    home: Option<&'a Path>,
    lines: Vec<Line>,
    tok_width: usize,
}

impl Renderer<'_> {
    fn glyphs(&self) -> [&'static str; 4] {
        // branch, last branch, continuation, blank
        if self.display.ascii {
            ["|- ", "`- ", "|  ", "   "]
        } else {
            ["├─ ", "└─ ", "│  ", "   "]
        }
    }

    fn path(&self, p: &Path) -> String {
        display_path(p, self.home)
    }

    fn tokens(&self, size: &AlwaysOnSize) -> String {
        if self.display.budget {
            format!("{}/{}", human(size.tokens), human(size.budget))
        } else {
            human(size.tokens)
        }
    }

    fn store(&mut self, store: &StoreNode) {
        let mut state = match store.git {
            None => "no git",
            Some(DirStatus { clean: true, .. }) => "git: clean",
            Some(DirStatus { clean: false, .. }) => "git: uncommitted changes",
        }
        .to_owned();
        if store.git.is_some_and(|g| !g.has_remote) {
            state.push_str(", no remote");
        }
        if store.shares_skills {
            state.push_str(", shares skills");
        }
        let dim = styles::source();
        self.lines.push(Line {
            left: vec![
                ("[store ".into(), dim),
                (store.name.clone(), styles::name()),
                (format!("  {}  {state}]", self.path(&store.root)), dim),
            ],
            right: Right::None,
        });
        for scope in &store.scopes {
            self.scope_line(scope, "");
            self.children(&scope_items(scope), "");
        }
        if !store.unscoped.is_empty() {
            self.lines.push(Line {
                left: vec![("(unscoped)".into(), styles::name())],
                right: Right::None,
            });
            let items: Vec<Item> = store.unscoped.iter().map(Item::Repo).collect();
            self.children(&items, "");
        }
    }

    fn children(&mut self, items: &[Item], prefix: &str) {
        let [branch, last, cont, blank] = self.glyphs();
        for (i, item) in items.iter().enumerate() {
            let is_last = i + 1 == items.len();
            let head = format!("{prefix}{}", if is_last { last } else { branch });
            let next = format!("{prefix}{}", if is_last { blank } else { cont });
            match item {
                Item::ScopeSkills(scope) => {
                    let mut left = vec![plain(head)];
                    for (j, s) in scope.skills.iter().enumerate() {
                        if j > 0 {
                            left.push(plain(" "));
                        }
                        left.extend(skill(s, Mark::Explicit));
                    }
                    self.lines.push(Line {
                        left,
                        right: Right::Soft(vec![("[scope]".into(), styles::source())]),
                    });
                }
                Item::Registry(r) => self.lines.push(Line {
                    left: vec![
                        plain(head),
                        plain(format!(
                            "registry ({} {}, {} tok)",
                            r.repos,
                            if r.repos == 1 { "repo" } else { "repos" },
                            human(r.tokens)
                        )),
                    ],
                    right: Right::None,
                }),
                Item::Scope(scope) => {
                    self.scope_line(scope, &head);
                    self.children(&scope_items(scope), &next);
                }
                Item::Repo(repo) => {
                    self.repo_line(repo, &head);
                    let mut sub = Vec::new();
                    if repo.show_skills && !repo.skills.is_empty() {
                        sub.push(Item::RepoSkills(repo));
                    }
                    if !repo.related.is_empty() {
                        sub.push(Item::Related(repo));
                    }
                    self.children(&sub, &next);
                }
                Item::RepoSkills(repo) => {
                    let mut left = vec![plain(head), plain("skills:")];
                    for s in &repo.skills {
                        left.push(plain(" "));
                        left.extend(skill(&s.reference, s.mark));
                    }
                    self.lines.push(Line {
                        left,
                        right: Right::None,
                    });
                }
                Item::Related(repo) => {
                    let mut left = vec![plain(head), plain("related:")];
                    for (j, r) in repo.related.iter().enumerate() {
                        left.push(plain(if j == 0 { " " } else { "   " }));
                        left.push(plain(format!("{} ", r.name)));
                        left.push(match r.access {
                            Access::Granted => ("✓ granted".into(), styles::explicit()),
                            Access::NotGranted => ("✗ not granted".into(), styles::removed()),
                            Access::NotCloned => ("· not cloned".into(), styles::source()),
                        });
                    }
                    self.lines.push(Line {
                        left,
                        right: Right::None,
                    });
                }
            }
        }
    }

    fn scope_line(&mut self, scope: &ScopeNode, head: &str) {
        let roots = match (&scope.machine_roots, self.display.machine) {
            (Some(machine), true) => machine,
            _ => &scope.roots,
        };
        let mut left = vec![plain(head), (scope.name.clone(), styles::name())];
        if let [first, rest @ ..] = roots.as_slice() {
            // Several roots show as the first and a count, so a scope with per-repo roots
            // keeps a short line; `--machine` lists them all, and `--json` always does.
            let label = if rest.is_empty() || self.display.machine {
                let roots: Vec<String> = roots.iter().map(|r| self.path(r)).collect();
                roots.join(", ")
            } else {
                format!("{} +{}", self.path(first), rest.len())
            };
            left.push(plain(format!("  ({label})")));
        }
        if let Some(parent) = &scope.extends {
            left.push((format!("  extends {parent}"), styles::source()));
        }
        let right = vec![plain(format!("always-on: {} tok", human(scope.always_on)))];
        // A header wider than the cap does not set the column the repo rows align to.
        let right = if width(&left) <= SCOPE_LABEL_CAP {
            Right::Aligned(right)
        } else {
            Right::Soft(right)
        };
        self.lines.push(Line { left, right });
    }

    fn repo_line(&mut self, repo: &RepoNode, head: &str) {
        let mut left = vec![plain(head), (repo.name.clone(), styles::name())];
        if self.display.machine {
            let at = match &repo.path {
                Some(p) => self.path(p),
                None => "(not cloned here)".into(),
            };
            left.push((format!("  {at}"), styles::source()));
        }
        let tokens = format!(
            "{:>w$} tok  ",
            self.tokens(&repo.always_on),
            w = self.tok_width
        );
        let tok_style = if self.display.budget && repo.always_on.over() {
            styles::removed()
        } else {
            styles::none()
        };
        let mut right = vec![(tokens, tok_style), state(repo.state)];
        let mut tags = Vec::new();
        if self.display.budget && repo.always_on.over() {
            tags.push(("over budget", styles::removed()));
        }
        if repo.notes.is_some() {
            tags.push(("notes", styles::source()));
        }
        if repo.draft_summary {
            tags.push(("draft summary", Style::new().yellow()));
        }
        for (tag, style) in tags {
            right.push(plain("  "));
            right.push((tag.into(), style));
        }
        self.lines.push(Line {
            left,
            right: Right::Aligned(right),
        });
    }
}

fn scope_items(scope: &ScopeNode) -> Vec<Item<'_>> {
    let mut items = Vec::new();
    if !scope.skills.is_empty() {
        items.push(Item::ScopeSkills(scope));
    }
    if let Some(r) = scope.registry {
        items.push(Item::Registry(r));
    }
    items.extend(scope.scopes.iter().map(Item::Scope));
    items.extend(scope.repos.iter().map(Item::Repo));
    items
}

fn skill(reference: &str, mark: Mark) -> [Seg; 2] {
    let (glyph, style) = mark.glyph();
    [plain(format!("{reference} ")), (glyph.into(), style)]
}

fn state(state: State) -> Seg {
    match state {
        State::Synced => ("✓ synced".into(), styles::explicit()),
        State::Stale => ("✗ stale".into(), styles::removed()),
        State::Missing => ("✗ missing".into(), styles::removed()),
        State::NotCloned => ("· not cloned".into(), styles::source()),
        State::CheckoutMissing => ("✗ checkout gone".into(), styles::removed()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(name: &str, tokens: u32, state: State) -> RepoNode {
        RepoNode {
            name: name.into(),
            path: Some(PathBuf::from(format!("/h/work/{name}"))),
            always_on: AlwaysOnSize {
                tokens,
                budget: 1000,
            },
            state,
            notes: None,
            draft_summary: false,
            skills: Vec::new(),
            related: Vec::new(),
            show_skills: false,
        }
    }

    fn sample() -> Tree {
        let mut events = repo("events", 1340, State::Synced);
        events.skills = vec![
            SkillNode {
                reference: "review".into(),
                mark: Mark::Inherited,
            },
            SkillNode {
                reference: "pr".into(),
                mark: Mark::Excluded,
            },
            SkillNode {
                reference: "recipes".into(),
                mark: Mark::Explicit,
            },
        ];
        events.show_skills = true;
        events.related = vec![
            RelatedNode {
                name: "billing".into(),
                access: Access::Granted,
            },
            RelatedNode {
                name: "ml".into(),
                access: Access::NotCloned,
            },
            RelatedNode {
                name: "web".into(),
                access: Access::NotGranted,
            },
        ];
        events.notes = Some(PathBuf::from("/h/store/repos/events/notes.md"));
        let mut billing = repo("billing", 620, State::Stale);
        billing.draft_summary = true;
        let child = ScopeNode {
            name: "team-ml".into(),
            roots: vec![PathBuf::from("/h/work/ml")],
            machine_roots: Some(vec![PathBuf::from("/Volumes/ml")]),
            extends: Some("team".into()),
            always_on: 700,
            skills: vec!["dbt".into()],
            registry: None,
            scopes: Vec::new(),
            repos: vec![events],
        };
        let team = ScopeNode {
            name: "team".into(),
            roots: vec![PathBuf::from("/h/work")],
            machine_roots: None,
            extends: None,
            always_on: 620,
            skills: vec!["review".into(), "pr".into(), "me/commit".into()],
            registry: Some(RegistrySize {
                repos: 1,
                tokens: 350,
            }),
            scopes: vec![child],
            repos: vec![billing],
        };
        Tree {
            tokenizer: "cl100k_base",
            stores: vec![
                StoreNode {
                    name: "work".into(),
                    root: PathBuf::from("/h/work/.kipi"),
                    git: Some(DirStatus {
                        clean: true,
                        has_remote: false,
                    }),
                    shares_skills: false,
                    scopes: vec![team],
                    unscoped: vec![repo("stray", 90, State::NotCloned)],
                },
                StoreNode {
                    name: "me".into(),
                    root: PathBuf::from("/h/.agents/store"),
                    git: Some(DirStatus {
                        clean: false,
                        has_remote: true,
                    }),
                    shares_skills: true,
                    scopes: Vec::new(),
                    unscoped: vec![repo("gone", 12, State::CheckoutMissing)],
                },
            ],
        }
    }

    #[test]
    fn unicode_box_drawing_follows_the_sketch() {
        let text = render(
            &sample(),
            Display::default(),
            &Painter::plain(),
            Some(Path::new("/h")),
        );
        insta::assert_snapshot!(text, @r"
        [store work  ~/work/.kipi  git: clean, no remote]
        team  (~/work)                         always-on: 620 tok
        ├─ review ● pr ● me/commit ●           [scope]
        ├─ registry (1 repo, 350 tok)
        ├─ team-ml  (~/work/ml)  extends team  always-on: 700 tok
        │  ├─ dbt ●                            [scope]
        │  └─ events                           1.3k tok  ✓ synced  notes
        │     ├─ skills: review ○ pr ✗ recipes ●
        │     └─ related: billing ✓ granted   ml · not cloned   web ✗ not granted
        └─ billing                              620 tok  ✗ stale  draft summary
        (unscoped)
        └─ stray                                 90 tok  · not cloned
        [store me  ~/.agents/store  git: uncommitted changes, shares skills]
        (unscoped)
        └─ gone                                  12 tok  ✗ checkout gone
        ");
    }

    #[test]
    fn ascii_budget_and_machine() {
        let display = Display {
            ascii: true,
            budget: true,
            machine: true,
        };
        let text = render(&sample(), display, &Painter::plain(), Some(Path::new("/h")));
        assert!(!text.contains('├') && !text.contains('│'), "{text}");
        assert!(text.contains("|  `- events  ~/work/events"), "{text}");
        assert!(
            text.contains("1.3k/1.0k tok  ✓ synced  over budget  notes"),
            "{text}"
        );
        assert!(text.contains("team-ml  (/Volumes/ml)"), "{text}");
        assert!(text.contains("~/work/stray"), "{text}");
    }

    /// One scope with per-repo roots under long paths (#52).
    fn many_roots() -> Tree {
        let roots: Vec<PathBuf> = ["geo-tools", "chat-service", "ranking-lab", "hex-map"]
            .iter()
            .map(|r| PathBuf::from(format!("/Volumes/Disk/code/{r}")))
            .collect();
        let mut chat = repo("chat-service", 231, State::Missing);
        chat.draft_summary = true;
        let personal = ScopeNode {
            name: "personal".into(),
            roots: roots.clone(),
            machine_roots: None,
            extends: None,
            always_on: 184,
            skills: Vec::new(),
            registry: None,
            scopes: Vec::new(),
            repos: vec![chat, repo("geo-tools", 1210, State::Synced)],
        };
        Tree {
            tokenizer: "cl100k_base",
            stores: vec![StoreNode {
                name: "me".into(),
                root: PathBuf::from("/h/.agents/store"),
                git: None,
                shares_skills: false,
                scopes: vec![personal],
                unscoped: Vec::new(),
            }],
        }
    }

    #[test]
    fn many_long_roots_keep_the_scope_line_short() {
        let text = render(
            &many_roots(),
            Display::default(),
            &Painter::plain(),
            Some(Path::new("/h")),
        );
        insta::assert_snapshot!(text, @r"
        [store me  ~/.agents/store  no git]
        personal  (/Volumes/Disk/code/geo-tools +3)  always-on: 184 tok
        ├─ chat-service                               231 tok  ✗ missing  draft summary
        └─ geo-tools                                 1.2k tok  ✓ synced
        ");
    }

    #[test]
    fn machine_lists_every_root_without_moving_the_columns() {
        let display = Display {
            machine: true,
            ..Display::default()
        };
        let text = render(
            &many_roots(),
            display,
            &Painter::plain(),
            Some(Path::new("/h")),
        );
        insta::assert_snapshot!(text, @r"
        [store me  ~/.agents/store  no git]
        personal  (/Volumes/Disk/code/geo-tools, /Volumes/Disk/code/chat-service, /Volumes/Disk/code/ranking-lab, /Volumes/Disk/code/hex-map)  always-on: 184 tok
        ├─ chat-service  ~/work/chat-service   231 tok  ✗ missing  draft summary
        └─ geo-tools  ~/work/geo-tools        1.2k tok  ✓ synced
        ");
    }
}
