//! Line diffs for `--dry-run`: unified-style hunks over a longest-common-subsequence
//! alignment (config files are small, so the quadratic table is fine).

use super::color::Painter;
use owo_colors::Style;

/// Lines of context around each change.
const CONTEXT: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Keep,
    Del,
    Add,
}

/// A unified diff from `before` to `after` under `--- name` / `+++ name` headers;
/// empty when they are equal.
pub fn unified(name: &str, before: &str, after: &str, paint: &Painter) -> String {
    let (a, b): (Vec<&str>, Vec<&str>) = (before.lines().collect(), after.lines().collect());
    let ops = align(&a, &b);
    if ops.iter().all(|(op, _)| *op == Op::Keep) {
        return String::new();
    }
    let mut out = paint.heading(format!("--- {name}\n+++ {name}\n"));
    let changed: Vec<usize> = (0..ops.len()).filter(|&i| ops[i].0 != Op::Keep).collect();
    let mut i = 0;
    while i < changed.len() {
        // Group changes whose context windows touch into one hunk.
        let start = changed[i].saturating_sub(CONTEXT);
        let mut end = changed[i];
        while i < changed.len() && changed[i] <= end + 2 * CONTEXT + 1 {
            end = changed[i];
            i += 1;
        }
        let end = (end + CONTEXT + 1).min(ops.len());
        let (old_start, new_start) = position(&ops[..start]);
        let (old_len, new_len) = position(&ops[start..end]);
        out.push_str(&paint.dim(format!(
            "@@ -{},{old_len} +{},{new_len} @@\n",
            old_start + 1,
            new_start + 1
        )));
        for (op, line) in &ops[start..end] {
            let (mark, style) = match op {
                Op::Keep => (' ', Style::new()),
                Op::Del => ('-', Style::new().red()),
                Op::Add => ('+', Style::new().green()),
            };
            out.push_str(&paint.paint(format!("{mark}{line}"), style));
            out.push('\n');
        }
    }
    out
}

/// How many old and new lines `ops` spans.
fn position(ops: &[(Op, &str)]) -> (usize, usize) {
    let old = ops.iter().filter(|(op, _)| *op != Op::Add).count();
    let new = ops.iter().filter(|(op, _)| *op != Op::Del).count();
    (old, new)
}

fn align<'a>(a: &[&'a str], b: &[&'a str]) -> Vec<(Op, &'a str)> {
    let mut lcs = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let (mut i, mut j, mut ops) = (0, 0, Vec::new());
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            ops.push((Op::Keep, a[i]));
            (i, j) = (i + 1, j + 1);
        } else if i < a.len() && (j == b.len() || lcs[i + 1][j] >= lcs[i][j + 1]) {
            ops.push((Op::Del, a[i]));
            i += 1;
        } else {
            ops.push((Op::Add, b[j]));
            j += 1;
        }
    }
    ops
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hunks_with_context() {
        let before = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\n";
        let after = "a\nB\nc\nd\ne\nf\ng\nh\ni\nj\nk\n";
        assert_eq!(
            unified("f.toml", before, after, &Painter::plain()),
            "--- f.toml\n+++ f.toml\n@@ -1,4 +1,4 @@\n a\n-b\n+B\n c\n d\n\
             @@ -9,2 +9,3 @@\n i\n j\n+k\n"
        );
    }

    #[test]
    fn new_file_and_no_change() {
        assert_eq!(
            unified("m.toml", "", "x\ny\n", &Painter::plain()),
            "--- m.toml\n+++ m.toml\n@@ -1,0 +1,2 @@\n+x\n+y\n"
        );
        assert_eq!(unified("m.toml", "x\n", "x\n", &Painter::plain()), "");
    }
}
