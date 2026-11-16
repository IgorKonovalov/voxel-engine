//! The menu's text on the HUD's grid: a title row, the page's rows (scrolled to keep the
//! selection in view) and a footer naming the keys that act on the page, written into the
//! HUD's reused buffers. A row is at most [`MENU_COLS`] bytes and the text at most
//! [`MENU_ROWS`] rows: the overlay's 80 x 45 cells at 1280 x 720.

use super::RunView;
use super::page::{Menu, PageId, Row};
use crate::hud::Modes;
use std::fmt::Write as _;
use vxe_render::HudColour;

/// The widest row, in cells.
pub const MENU_COLS: usize = 80;
/// The most rows the menu writes.
pub const MENU_ROWS: usize = 45;
/// The page's rows shown at once: the rows between the title and the blank row and footer.
pub const LIST_ROWS: usize = MENU_ROWS - 3;
/// The column the binds start at.
pub const KEY_COL: usize = 60;

/// Appends `s` to `out` cut to `width` bytes, a `~` in the last cell when cut.
fn push_cut(out: &mut String, s: &str, width: usize) {
    if s.len() <= width {
        out.push_str(s);
    } else {
        let mut end = width.saturating_sub(1);
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        out.push_str(&s[..end]);
        out.push('~');
    }
}

/// Pads the row being written (from byte `start` of `out`) to `col` cells.
fn pad_to(out: &mut String, start: usize, col: usize) {
    let len = out.len() - start;
    for _ in len..col {
        out.push(' ');
    }
}

impl Menu {
    /// Writes the open menu at `m` and `v` into `text` and `colours`, which it clears: the
    /// title (`menu`, `menu  >  lighting`, the filter as `search: lam`, the selection as
    /// `n/m` when the rows do not fit), the rows with the selected one yellow, a blank row and
    /// the footer. Writes nothing while closed.
    pub fn write(&self, m: &Modes, v: &RunView, text: &mut String, colours: &mut Vec<HudColour>) {
        text.clear();
        colours.clear();
        let Some(page) = self.page() else { return };
        let rows = self.rows(m, v);
        let n = rows.iter().filter(|r| r.selectable()).count();
        let sel = self.selected().min(n.saturating_sub(1));
        // The selected row's place among all rows; headers scroll with the rest.
        let at = rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.selectable())
            .nth(sel)
            .map_or(0, |(i, _)| i);
        let top = (at + 1).saturating_sub(LIST_ROWS);
        let searching = !self.filter().is_empty();
        let start = text.len();
        text.push_str("menu");
        match page {
            PageId::Root => {}
            PageId::Group(g) => {
                let _ = write!(text, "  >  {}", g.name());
            }
            PageId::Keys => text.push_str("  >  keys"),
        }
        if searching {
            let _ = write!(text, "  search: {}", self.filter());
        }
        if rows.len() > LIST_ROWS {
            let _ = write!(text, "  {}/{n}", sel + 1);
        }
        cut_row(text, start);
        colours.push(HudColour::White);
        if searching && rows.is_empty() {
            text.push_str("\n  no match");
            colours.push(HudColour::White);
        }
        let mut pick = 0;
        for (i, row) in rows.iter().enumerate() {
            let selected = row.selectable() && pick == sel;
            if row.selectable() {
                pick += 1;
            }
            if i < top || i >= top + LIST_ROWS {
                continue;
            }
            text.push('\n');
            let start = text.len();
            text.push_str(if selected { "> " } else { "  " });
            write_row(text, start, row, page, searching, m, v);
            cut_row(text, start);
            colours.push(if selected {
                HudColour::Yellow
            } else {
                HudColour::White
            });
        }
        text.push_str("\n\n");
        colours.push(HudColour::White);
        let start = text.len();
        text.push_str(footer(page, searching));
        cut_row(text, start);
        colours.push(HudColour::White);
    }
}

/// Cuts the row being written (from byte `start`) to [`MENU_COLS`] cells and drops its
/// trailing spaces.
fn cut_row(text: &mut String, start: usize) {
    if text.len() - start > MENU_COLS {
        let row = text[start..].to_owned();
        text.truncate(start);
        push_cut(text, &row, MENU_COLS);
    }
    let kept = text.trim_end_matches(' ').len().max(start);
    text.truncate(kept);
}

/// One row's text after its selection mark: a group's name, an entry's words and binds
/// (the palette's prefixed with the group), or a Keys page row (binds first).
fn write_row(
    text: &mut String,
    start: usize,
    row: &Row,
    page: PageId,
    searching: bool,
    m: &Modes,
    v: &RunView,
) {
    match *row {
        Row::Group(g) => text.push_str(g.name()),
        Row::Header(g) => text.push_str(g.name()),
        Row::Fixed(keys, does) => {
            let _ = write!(text, "  {keys:<18}{does}");
        }
        Row::Entry(e) if page == PageId::Keys && !searching => {
            let keys = e.keys();
            let keys = if keys.is_empty() { "menu" } else { &keys };
            let _ = write!(text, "  {keys:<18}");
            push_cut(text, &e.words(m, v), MENU_COLS - 22);
        }
        Row::Entry(e) => {
            if searching {
                let _ = write!(text, "{}  ", e.group.name());
            }
            let room = (KEY_COL - 2).saturating_sub(text.len() - start);
            push_cut(text, &e.words(m, v), room);
            pad_to(text, start, KEY_COL);
            text.push_str(&e.keys());
        }
    }
}

/// The keys that act on `page`.
fn footer(page: PageId, searching: bool) -> &'static str {
    if searching {
        return "Enter acts  Backspace erases  Esc clears  Up Down select";
    }
    match page {
        PageId::Root => "Enter opens  Esc closes  Tab closes  type to search",
        PageId::Group(_) => "Enter acts  Esc back  Tab closes  type to search",
        PageId::Keys => "F1 closes  Tab the menu  Enter acts  type to search",
    }
}
