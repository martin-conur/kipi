//! Left-aligned columns separated by two spaces; widths count chars, so colors and
//! markers do not break alignment. The last cell of a row is never padded, and a
//! column that is empty in every row takes no space.

use owo_colors::Style;

use super::color::Painter;

/// Rows of cells, rendered with every column as wide as its widest cell.
#[derive(Debug, Default)]
pub struct Table {
    indent: usize,
    rows: Vec<Vec<(String, Style)>>,
}

impl Table {
    /// An empty table whose lines start with `indent` spaces.
    pub fn new(indent: usize) -> Self {
        Table {
            indent,
            rows: Vec::new(),
        }
    }

    /// Append a row of `(text, style)` cells.
    pub fn row(&mut self, cells: impl IntoIterator<Item = (String, Style)>) -> &mut Self {
        self.rows.push(cells.into_iter().collect());
        self
    }

    /// Whether no row was added.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The lines, each ending in `\n`.
    pub fn render(&self, paint: &Painter) -> String {
        let cols = self.rows.iter().map(Vec::len).max().unwrap_or(0);
        let mut widths = vec![0; cols];
        for row in &self.rows {
            for (i, (text, _)) in row.iter().enumerate() {
                // Chars, not display columns: right for the narrow Unicode kipi prints
                // (○ ● ✗ ← →); wide characters such as CJK or emoji would misalign.
                widths[i] = widths[i].max(text.chars().count());
            }
        }
        let mut out = String::new();
        for row in &self.rows {
            // Columns empty in every row are dropped; trailing empty cells add nothing.
            let cells: Vec<(usize, &(String, Style))> = row
                .iter()
                .enumerate()
                .filter(|(i, _)| widths[*i] > 0)
                .collect();
            let used = cells
                .iter()
                .rposition(|(_, (t, _))| !t.is_empty())
                .map_or(0, |i| i + 1);
            let mut line = " ".repeat(self.indent);
            for (n, (i, (text, style))) in cells[..used].iter().enumerate() {
                if n > 0 {
                    line.push_str("  ");
                }
                line.push_str(&paint.paint(text, *style));
                if n + 1 < used {
                    line.push_str(&" ".repeat(widths[*i] - text.chars().count()));
                }
            }
            out.push_str(&line);
            out.push('\n');
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(s: &str) -> (String, Style) {
        (s.to_owned(), Style::new())
    }

    #[test]
    fn aligns_columns_by_char_count() {
        let mut t = Table::new(2);
        t.row([cell("○ a"), cell("x"), cell("end")]);
        t.row([cell("● longer"), cell(""), cell("e")]);
        t.row([cell("✗ b"), cell("yy"), cell("")]);
        assert_eq!(
            t.render(&Painter::plain()),
            "  ○ a       x   end\n  ● longer      e\n  ✗ b       yy\n"
        );
    }

    #[test]
    fn empty_columns_take_no_space() {
        let mut t = Table::new(0);
        t.row([cell("a"), cell(""), cell("z")]);
        t.row([cell("bb"), cell(""), cell("z")]);
        assert_eq!(t.render(&Painter::plain()), "a   z\nbb  z\n");
    }

    #[test]
    fn empty_table_renders_nothing() {
        let t = Table::new(2);
        assert!(t.is_empty());
        assert_eq!(t.render(&Painter::plain()), "");
    }
}
