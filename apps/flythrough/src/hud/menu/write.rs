//! The menu's text on the HUD's grid: a title row, the page's rows (scrolled to keep the
//! selection in view) and a footer naming the keys that act on the page, written into the
//! HUD's reused buffers. A row is at most [`MENU_COLS`] bytes and the text at most
//! [`MENU_ROWS`] rows: the overlay's 80 x 45 cells at 1280 x 720. [`place`] puts that text
//! in the window's panel ([`Panel`]: centred, at the menu's glyph scale) with the frame-rate
//! readout on the window's top row, and gives the backdrop's box.

use super::RunView;
use super::page::{Menu, PageId, Row};
use crate::hud::Modes;
use std::fmt::Write as _;
use vxe_render::{HudBox, HudColour};

/// The widest row, in cells.
pub const MENU_COLS: usize = 80;
/// The most rows the menu writes.
pub const MENU_ROWS: usize = 45;
/// The page's rows shown at once: the rows between the title and the blank row and footer.
pub const LIST_ROWS: usize = MENU_ROWS - 3;
/// The column the binds start at.
pub const KEY_COL: usize = 60;

/// The menu's glyph scale for a `width` x `height` window: the largest integer scale at which
/// [`MENU_COLS`] x [`MENU_ROWS`] cells of 8 x 8 glyph pixels fit, at least 2.
pub fn panel_scale(width: u32, height: u32) -> u32 {
    let w = width / (8 * MENU_COLS as u32);
    let h = height / (8 * MENU_ROWS as u32);
    w.min(h).max(2)
}

/// Where the menu's text sits in the window, in cells at [`panel_scale`]: the panel's
/// top-left cell (`col`, `row`) and the cells wholly inside the window (`cols` x `rows`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Panel {
    pub scale: u32,
    pub col: usize,
    pub row: usize,
    pub cols: usize,
    pub rows: usize,
}

impl Panel {
    /// The panel of a `width` x `height` window: [`MENU_COLS`] x [`MENU_ROWS`] cells centred
    /// on the window's whole cells (rounding left and up), at the top-left when they do not
    /// fit. The page grows down from its top row.
    pub fn of(width: u32, height: u32) -> Self {
        let scale = panel_scale(width, height);
        let cols = (width / (8 * scale)) as usize;
        let rows = (height / (8 * scale)) as usize;
        Self {
            scale,
            col: cols.saturating_sub(MENU_COLS) / 2,
            row: rows.saturating_sub(MENU_ROWS) / 2,
            cols,
            rows,
        }
    }
}

/// Writes `page` (the menu's rows from [`Menu::write`], at the top-left) into `text` and
/// `colours`, which it clears, moved to `panel`'s cell, with `readout` on the window's top
/// row ending at its last whole column (white; on the title row when the panel starts at row
/// 0, the title cut to leave two cells before it). Gives the backdrop's box: the panel's
/// [`MENU_COLS`] columns plus one on each side, from the title row through one row below the
/// page's last, cut to the window's whole cells.
pub fn place(
    page: &str,
    page_colours: &[HudColour],
    panel: Panel,
    readout: &str,
    text: &mut String,
    colours: &mut Vec<HudColour>,
) -> HudBox {
    text.clear();
    colours.clear();
    let readout_at = panel.cols.saturating_sub(readout.len());
    let mut n = 0;
    if panel.row > 0 {
        if !readout.is_empty() {
            pad_to(text, 0, readout_at);
            text.push_str(readout);
        }
        colours.push(HudColour::White);
        n = 1;
    }
    for _ in n..panel.row {
        text.push('\n');
        colours.push(HudColour::White);
    }
    for (i, line) in page.lines().enumerate() {
        if panel.row + i > 0 {
            text.push('\n');
        }
        let start = text.len();
        pad_to(text, start, panel.col);
        text.push_str(line);
        if panel.row + i == 0 && !readout.is_empty() {
            let room = readout_at.saturating_sub(2);
            if text.len() - start > room {
                let row = text[start..].to_owned();
                text.truncate(start);
                push_cut(text, &row, room);
            }
            pad_to(text, start, readout_at);
            text.push_str(readout);
        }
        colours.push(page_colours.get(i).copied().unwrap_or_default());
    }
    let used = page.lines().count();
    let col = panel.col.saturating_sub(1);
    let end_col = (panel.col + MENU_COLS + 1).min(panel.cols);
    let end_row = (panel.row + used + 1).min(panel.rows);
    HudBox {
        col: col as u32,
        row: panel.row as u32,
        cols: end_col.saturating_sub(col) as u32,
        rows: end_row.saturating_sub(panel.row) as u32,
    }
}

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
        self.write_with(m, v, None, text, colours);
    }

    /// [`Menu::write`] with `notice` (the last action's line, yellow) in the row above the
    /// footer. An unavailable entry's row, and a group's on the root whose entries are all
    /// unavailable for one reason, is grey (selected or not), the group's with its reason in
    /// the key column; while one is selected the footer names its reason. A number being
    /// edited reads `label  < value >  range` and the footer `Up Down change  Enter Esc
    /// done`.
    pub fn write_with(
        &self,
        m: &Modes,
        v: &RunView,
        notice: Option<&str>,
        text: &mut String,
        colours: &mut Vec<HudColour>,
    ) {
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
        let mut why = None;
        for (i, row) in rows.iter().enumerate() {
            let selected = row.selectable() && pick == sel;
            if row.selectable() {
                pick += 1;
            }
            let unavailable = unavailable(row, m, v);
            if selected {
                why = unavailable;
            }
            if i < top || i >= top + LIST_ROWS {
                continue;
            }
            text.push('\n');
            let start = text.len();
            text.push_str(if selected { "> " } else { "  " });
            match row {
                Row::Entry(e) if selected && self.editing() && e.is_number() => {
                    let _ = write!(
                        text,
                        "{}  < {} >  {}",
                        e.label,
                        e.number_words(m, v),
                        e.range(m, v).unwrap_or_default()
                    );
                }
                _ => write_row(text, start, row, page, searching, m, v),
            }
            if let (Row::Group(_), Some(why)) = (row, unavailable) {
                pad_to(text, start, KEY_COL);
                text.push_str(why);
            }
            cut_row(text, start);
            colours.push(if unavailable.is_some() {
                HudColour::Grey
            } else if selected {
                HudColour::Yellow
            } else {
                HudColour::White
            });
        }
        text.push('\n');
        if let Some(line) = notice {
            let start = text.len();
            text.push_str(line);
            cut_row(text, start);
            colours.push(HudColour::Yellow);
        } else {
            colours.push(HudColour::White);
        }
        text.push('\n');
        let start = text.len();
        if self.editing() {
            text.push_str("Up Down change  Enter Esc done");
        } else if let Some(why) = why {
            let _ = write!(text, "{why}  Esc back  Tab closes  type to search");
        } else {
            text.push_str(footer(page, searching));
        }
        cut_row(text, start);
        colours.push(HudColour::White);
    }
}

/// Why `row` does nothing at `m` and `v`: an unavailable entry's reason, or a group's
/// whose entries are all unavailable for the same reason; `None` for any other row.
fn unavailable(row: &Row, m: &Modes, v: &RunView) -> Option<&'static str> {
    match row {
        Row::Entry(e) => e.available(m, v).err(),
        Row::Group(g) => {
            let mut why = None;
            for e in super::group_entries(*g) {
                match (e.available(m, v), why) {
                    (Ok(()), _) => return None,
                    (Err(w), None) => why = Some(w),
                    (Err(w), Some(x)) if w != x => return None,
                    (Err(_), Some(_)) => {}
                }
            }
            why
        }
        Row::Fixed(..) | Row::Header(_) => None,
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
