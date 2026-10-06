//! Repo identity (roadmap §5.1, §5.2): normalized origin URLs and matching a checkout
//! on this machine to a repo entry.

use std::fmt;
use std::str::FromStr;

use super::RepoEntry;

/// A normalized `remote.origin.url`: `host/owner/name`, with the scheme, user, port
/// and `.git` stripped and the host lowercased. Path case is kept, since self-hosted
/// forges may treat it as significant.
///
/// `https://github.com/o/n.git`, `ssh://git@github.com:22/o/n` and `git@github.com:o/n.git`
/// all normalize to `github.com/o/n`. Nested groups (`gitlab.com/group/sub/n`) keep
/// every segment.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RemoteUrl(String);

/// URL schemes a remote may use. `file://` and plain paths are local, not remotes.
const SCHEMES: &[&str] = &["https", "http", "ssh", "git", "git+ssh", "ssh+git"];

impl RemoteUrl {
    /// Normalize a raw origin URL (any form git accepts for a network remote).
    pub fn parse(raw: &str) -> Result<Self, String> {
        let s = raw.trim();
        if s.is_empty() {
            return Err("empty remote URL".to_owned());
        }
        let (host, path) = if let Some((scheme, rest)) = s.split_once("://") {
            if !SCHEMES.contains(&scheme.to_ascii_lowercase().as_str()) {
                return Err(format!(
                    "`{raw}` is not a network remote (scheme `{scheme}`)"
                ));
            }
            let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
            (strip_port(strip_user(authority)), path)
        } else {
            // scp-like `[user@]host:path`: a `:` before any `/`.
            match s.split_once(':') {
                Some((left, path)) if !left.contains('/') => (strip_user(left), path),
                _ => return Err(format!("`{raw}` is a local path, not a network remote")),
            }
        };
        let host = host.to_ascii_lowercase();
        if host.is_empty() || host.contains(char::is_whitespace) {
            return Err(format!("`{raw}` has no host"));
        }
        let mut segments: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        if let Some(last) = segments.last_mut() {
            *last = last.strip_suffix(".git").unwrap_or(last);
        }
        let bad =
            |p: &&str| p.is_empty() || *p == "." || *p == ".." || p.contains(char::is_whitespace);
        if segments.is_empty() || segments.iter().any(bad) {
            return Err(format!("`{raw}` has no repository path"));
        }
        Ok(RemoteUrl(format!("{host}/{}", segments.join("/"))))
    }

    /// The normalized form, `host/owner/name`.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The host, lowercased.
    pub fn host(&self) -> &str {
        self.0.split_once('/').map_or(&self.0, |(h, _)| h)
    }

    /// The last path segment: the repository's own name.
    pub fn basename(&self) -> &str {
        self.0.rsplit_once('/').map_or(&self.0, |(_, n)| n)
    }
}

fn strip_user(authority: &str) -> &str {
    authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host)
}

fn strip_port(host: &str) -> &str {
    match host.strip_prefix('[') {
        // `[::1]:22`: keep the bracketed address.
        Some(rest) => rest.find(']').map_or(host, |end| &host[..end + 2]),
        None => host.split_once(':').map_or(host, |(h, _)| h),
    }
}

impl FromStr for RemoteUrl {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        RemoteUrl::parse(s)
    }
}

impl fmt::Display for RemoteUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A git checkout found on this machine, as `scan` and `doctor` see it.
#[derive(Debug, Clone, Copy)]
pub struct Checkout<'a> {
    /// The checkout's directory name.
    pub dir_name: &'a str,
    /// Its normalized `remote.origin.url`, if it has a network remote.
    pub remote: Option<&'a RemoteUrl>,
}

/// Which rule matched a checkout to an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchedBy {
    /// The entry's `remote` equals the checkout's normalized origin.
    Remote,
    /// The directory name equals the entry name (entries without `remote` only).
    DirName,
    /// The origin's basename equals the entry name or its `remote`'s basename.
    RemoteBasename,
}

/// The outcome of [`match_checkout`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepoMatch<'a> {
    /// Exactly one entry matched.
    Unique {
        /// The entry.
        entry: &'a RepoEntry,
        /// The rule that found it.
        by: MatchedBy,
    },
    /// Several entries matched under the same rule; kipi does not guess.
    Ambiguous {
        /// The rule that found them.
        by: MatchedBy,
        /// Every candidate, in input order.
        candidates: Vec<&'a RepoEntry>,
    },
    /// Nothing matched.
    None,
}

/// Match a checkout to a repo entry (roadmap §5.2): normalized `remote` first, then the
/// directory name (only entries without `remote`), then the origin's basename. The
/// first rule with any candidate decides; several candidates are reported as
/// [`RepoMatch::Ambiguous`] rather than resolved by order.
pub fn match_checkout<'a>(
    checkout: Checkout<'_>,
    entries: impl IntoIterator<Item = &'a RepoEntry>,
) -> RepoMatch<'a> {
    let entries: Vec<&RepoEntry> = entries.into_iter().collect();
    let matches = |by: MatchedBy, e: &RepoEntry| match (by, checkout.remote) {
        (MatchedBy::Remote, Some(r)) => e.remote.as_ref() == Some(r),
        (MatchedBy::DirName, _) => e.remote.is_none() && e.name == checkout.dir_name,
        (MatchedBy::RemoteBasename, Some(r)) => {
            let base = r.basename();
            e.name == base || e.remote.as_ref().is_some_and(|er| er.basename() == base)
        }
        (_, None) => false,
    };
    for by in [
        MatchedBy::Remote,
        MatchedBy::DirName,
        MatchedBy::RemoteBasename,
    ] {
        let mut candidates: Vec<&RepoEntry> =
            entries.iter().copied().filter(|e| matches(by, e)).collect();
        match candidates.len() {
            0 => continue,
            1 => {
                return RepoMatch::Unique {
                    entry: candidates.remove(0),
                    by,
                };
            }
            _ => return RepoMatch::Ambiguous { by, candidates },
        }
    }
    RepoMatch::None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Location;
    use crate::model::Drafted;

    #[test]
    fn normalization_table() {
        let cases = [
            // HTTPS, with and without `.git`, trailing slashes, user and port.
            (
                "https://github.com/acme-org/billing-api.git",
                "github.com/acme-org/billing-api",
            ),
            (
                "https://github.com/acme-org/billing-api",
                "github.com/acme-org/billing-api",
            ),
            (
                "https://github.com/acme-org/billing-api/",
                "github.com/acme-org/billing-api",
            ),
            (
                "https://github.com/acme-org/billing-api.git/",
                "github.com/acme-org/billing-api",
            ),
            (
                "http://github.com/acme-org/billing-api",
                "github.com/acme-org/billing-api",
            ),
            ("https://user:token@github.com/o/n.git", "github.com/o/n"),
            (
                "https://git.example.com:8443/o/n.git",
                "git.example.com/o/n",
            ),
            ("HTTPS://GitHub.COM/Owner/Name.git", "github.com/Owner/Name"),
            // ssh:// URLs.
            ("ssh://git@github.com/o/n.git", "github.com/o/n"),
            ("ssh://git@github.com:22/o/n.git", "github.com/o/n"),
            (
                "ssh://git@git.example.com:2222/team/n",
                "git.example.com/team/n",
            ),
            ("git+ssh://git@github.com/o/n.git", "github.com/o/n"),
            ("git://github.com/o/n.git", "github.com/o/n"),
            ("ssh://git@[::1]:22/o/n.git", "[::1]/o/n"),
            // scp-like.
            (
                "git@github.com:acme-org/billing-api.git",
                "github.com/acme-org/billing-api",
            ),
            (
                "git@github.com:acme-org/billing-api",
                "github.com/acme-org/billing-api",
            ),
            (
                "git@github.com:/acme-org/billing-api.git",
                "github.com/acme-org/billing-api",
            ),
            ("github.com:o/n.git", "github.com/o/n"),
            ("git@GitHub.com:o/n.git/", "github.com/o/n"),
            // GitLab nested groups and self-hosted forges.
            (
                "https://gitlab.com/group/sub/n.git",
                "gitlab.com/group/sub/n",
            ),
            ("git@gitlab.com:group/sub/n.git", "gitlab.com/group/sub/n"),
            ("git@git.example.com:n.git", "git.example.com/n"),
            ("https://git.example.com//o//n.git", "git.example.com/o/n"),
            // A `.git` inside the path is kept; only the suffix goes.
            (
                "https://github.com/o/n.github.io.git",
                "github.com/o/n.github.io",
            ),
            ("https://github.com/o/.github", "github.com/o/.github"),
        ];
        for (raw, want) in cases {
            let got = RemoteUrl::parse(raw).unwrap_or_else(|e| panic!("{raw}: {e}"));
            assert_eq!(got.as_str(), want, "{raw}");
            // The normalized form is a fixed point: as an `https://` URL it normalizes
            // to itself (which is how `bindings.toml` validates `remote`).
            let again = RemoteUrl::parse(&format!("https://{want}")).unwrap();
            assert_eq!(again, got, "{raw}");
        }
    }

    #[test]
    fn non_remotes_are_rejected() {
        for raw in [
            "",
            "   ",
            "/srv/git/n.git",
            "../n",
            "./n",
            "file:///srv/git/n.git",
            "github.com/o/n",
            "https://github.com",
            "https://github.com/",
            "git@github.com:",
            "https:///o/n",
            "https://github.com/o/..",
        ] {
            assert!(RemoteUrl::parse(raw).is_err(), "{raw:?} should be rejected");
        }
    }

    #[test]
    fn parts() {
        let r: RemoteUrl = "git@gitlab.com:group/sub/n.git".parse().unwrap();
        assert_eq!(r.host(), "gitlab.com");
        assert_eq!(r.basename(), "n");
        assert_eq!(r.to_string(), "gitlab.com/group/sub/n");
    }

    fn entry(name: &str, remote: Option<&str>) -> RepoEntry {
        RepoEntry {
            name: name.to_owned(),
            remote: remote.map(|r| RemoteUrl::parse(&format!("https://{r}")).unwrap()),
            summary: None,
            aliases: Drafted::default(),
            related: Vec::new(),
            skills: Vec::new(),
            exclude: Vec::new(),
            scope: None,
            agents: None,
            grant: None,
            budget_tokens: None,
            notes: None,
            at: Location::file("bindings.toml"),
        }
    }

    fn names(m: &RepoMatch<'_>) -> (Option<MatchedBy>, Vec<String>) {
        match m {
            RepoMatch::Unique { entry, by } => (Some(*by), vec![entry.name.clone()]),
            RepoMatch::Ambiguous { by, candidates } => (
                Some(*by),
                candidates.iter().map(|e| e.name.clone()).collect(),
            ),
            RepoMatch::None => (None, Vec::new()),
        }
    }

    fn check(
        entries: &[RepoEntry],
        dir: &str,
        origin: Option<&str>,
    ) -> (Option<MatchedBy>, Vec<String>) {
        let remote = origin.map(|o| RemoteUrl::parse(o).unwrap());
        let checkout = Checkout {
            dir_name: dir,
            remote: remote.as_ref(),
        };
        names(&match_checkout(checkout, entries))
    }

    #[test]
    fn matching_rules_in_order() {
        let entries = [
            entry("acme-api", Some("github.com/acme-org/api")),
            entry("api", Some("github.com/example-user/api")),
            entry("scratch", None),
            entry("notes", None),
            entry("tool", Some("github.com/example-user/tool")),
        ];
        let by = |b| Some(b);
        // Remote wins over directory name, in either URL form.
        assert_eq!(
            check(&entries, "api", Some("git@github.com:acme-org/api.git")),
            (by(MatchedBy::Remote), vec!["acme-api".into()])
        );
        assert_eq!(
            check(
                &entries,
                "whatever",
                Some("https://github.com/example-user/api")
            ),
            (by(MatchedBy::Remote), vec!["api".into()])
        );
        // Directory name, only for entries without `remote`; with or without an origin.
        assert_eq!(
            check(&entries, "scratch", None),
            (by(MatchedBy::DirName), vec!["scratch".into()])
        );
        assert_eq!(
            check(
                &entries,
                "notes",
                Some("https://git.example.com/me/notes-repo")
            ),
            (by(MatchedBy::DirName), vec!["notes".into()])
        );
        assert_eq!(
            check(&entries, "tool", None),
            (None, vec![]),
            "tool has a remote"
        );
        // Basename of the origin: entry name or the entry remote's basename.
        assert_eq!(
            check(
                &entries,
                "t2",
                Some("https://mirror.example.com/me/tool.git")
            ),
            (by(MatchedBy::RemoteBasename), vec!["tool".into()])
        );
        // `api` is both acme-api's remote basename and the personal entry's name.
        assert_eq!(
            check(
                &entries,
                "x",
                Some("https://mirror.example.com/someone/api")
            ),
            (
                by(MatchedBy::RemoteBasename),
                vec!["acme-api".into(), "api".into()]
            )
        );
        assert_eq!(check(&entries, "other", None), (None, vec![]));
        assert_eq!(
            check(&entries, "other", Some("https://github.com/o/unknown")),
            (None, vec![])
        );
    }

    #[test]
    fn same_remote_in_two_entries_is_ambiguous() {
        let entries = [
            entry("a", Some("github.com/o/n")),
            entry("b", Some("github.com/o/n")),
        ];
        assert_eq!(
            check(&entries, "n", Some("git@github.com:o/n.git")),
            (Some(MatchedBy::Remote), vec!["a".into(), "b".into()])
        );
        let entries = [entry("n", None), entry("n", None)];
        assert_eq!(
            check(&entries, "n", None),
            (Some(MatchedBy::DirName), vec!["n".into(), "n".into()])
        );
    }
}
