//! Every GitHub Actions step in `.github/workflows/*.yml` is pinned to a full commit SHA
//! with a `# vX.Y.Z` comment (docs/decisions.md). `dist generate` writes mutable tags
//! into `release.yml`, so this catches a regeneration that forgot to re-pin.

use std::fs;

use kipi_test_support::workspace_root;

/// The problem with one `uses:` value (everything after `uses:`), if any.
fn check(value: &str) -> Option<String> {
    let (action, comment) = match value.split_once('#') {
        Some((a, c)) => (a.trim(), Some(c.trim())),
        None => (value.trim(), None),
    };
    let action = action.trim_matches(|c| c == '"' || c == '\'');
    if action.starts_with("./") || action.starts_with("docker://") {
        return None;
    }
    let Some((_, rev)) = action.rsplit_once('@') else {
        return Some(format!("`{action}` names no revision"));
    };
    if rev.len() != 40 || !rev.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Some(format!("`{action}` is not pinned to a 40-hex commit SHA"));
    }
    match comment {
        Some(c) if c.starts_with('v') => None,
        _ => Some(format!("`{action}` lacks a `# vX.Y.Z` comment")),
    }
}

#[test]
fn every_action_is_pinned_to_a_commit_sha() {
    let dir = workspace_root().join(".github/workflows");
    let mut problems = Vec::new();
    let mut seen = 0;
    for entry in fs::read_dir(&dir).expect(".github/workflows exists") {
        let path = entry.unwrap().path();
        if !matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("yml" | "yaml")
        ) {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        for (i, line) in fs::read_to_string(&path).unwrap().lines().enumerate() {
            let line = line.trim_start().trim_start_matches("- ");
            let Some(value) = line.strip_prefix("uses:") else {
                continue;
            };
            seen += 1;
            if let Some(problem) = check(value) {
                problems.push(format!("{name}:{}: {problem}", i + 1));
            }
        }
    }
    assert!(seen > 0, "no `uses:` lines found under {}", dir.display());
    assert!(
        problems.is_empty(),
        "unpinned actions (pin each to a full SHA with a `# vX.Y.Z` comment; after \
         `dist generate`, see docs/release.md):\n{}",
        problems.join("\n")
    );
}

#[test]
fn the_check_accepts_pins_and_rejects_tags() {
    let sha = "11d5960a326750d5838078e36cf38b85af677262";
    assert_eq!(check(&format!(" actions/checkout@{sha} # v4.4.0")), None);
    assert_eq!(check(" ./.github/actions/local"), None);
    assert!(check(" actions/checkout@v6").is_some());
    assert!(
        check(&format!(" actions/checkout@{sha}")).is_some(),
        "no version comment"
    );
    assert!(check(" actions/checkout@main # v6").is_some());
    assert!(check(" actions/checkout").is_some());
}
