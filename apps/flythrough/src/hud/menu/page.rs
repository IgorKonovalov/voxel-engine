//! The menu's state: a stack of pages with a selection each, and the palette's filter. A page
//! is a list of rows built from the key tables when it is shown, so its values are live.

use super::filter::palette;
use super::{Entry, EntryKind, FIXED_ROWS, Group, RunView, entries, group_entries};
use crate::hud::Modes;
use crate::toggles::{Act, Toggle};

/// A page of the menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageId {
    /// The groups, one row each.
    Root,
    /// One group's entries.
    Group(Group),
    /// Every entry of every group with its binds, under the fixed keys (`F1`).
    Keys,
}

/// One row of a page.
#[derive(Clone, Copy, Debug)]
pub enum Row {
    /// A group on the root page: `Enter` opens it.
    Group(Group),
    /// An entry: `Enter` flips or runs it.
    Entry(Entry),
    /// A row that only shows: the Keys page's fixed keys (binds, what they do) and its group
    /// headers (a group, no binds).
    Fixed(&'static str, &'static str),
    Header(Group),
}

impl Row {
    /// Whether the selection can rest on the row.
    pub fn selectable(&self) -> bool {
        matches!(self, Self::Group(_) | Self::Entry(_))
    }
}

/// What the menu hands the run to do: a toggle's flip or an action.
#[derive(Clone, Copy)]
pub enum Activated {
    Toggle(&'static Toggle),
    Action(Act),
}

impl std::fmt::Debug for Activated {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Toggle(t) => write!(f, "Toggle({})", t.name),
            Self::Action(a) => write!(f, "Action({a:?})"),
        }
    }
}

/// One step of the menu's input, as `route` reads it from the keys, the wheel, the clicks and
/// the pad.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuInput {
    Up,
    Down,
    Left,
    Right,
    /// `Enter`, the left click, the pad's `South`.
    Activate,
    /// `Esc`, the right click, the pad's `East`: the filter, then the page, then the menu.
    Back,
    /// `Tab` (and the pad's `Start`): opens the root, or from the Keys page shows the root, or
    /// closes.
    Menu,
    /// `F1`: opens the Keys page, or closes it.
    Keys,
    /// A typed letter, digit, space or `-`.
    Type(char),
    /// `Backspace`: the filter's last letter.
    Erase,
    /// Wheel lines, positive away from the user (up the list).
    Scroll(i32),
}

/// The menu: closed, or a stack of open pages with a selection each, and the filter typed on
/// top of them (with its own selection).
#[derive(Clone, Debug, Default)]
pub struct Menu {
    stack: Vec<(PageId, usize)>,
    filter: String,
    found: usize,
}

impl Menu {
    pub fn is_open(&self) -> bool {
        !self.stack.is_empty()
    }

    /// The page shown, under the filter when one is typed.
    pub fn page(&self) -> Option<PageId> {
        self.stack.last().map(|&(p, _)| p)
    }

    pub fn filter(&self) -> &str {
        &self.filter
    }

    /// Opens the menu on `page`, with no filter: a group's page over the root, so `Esc` goes
    /// back to it; the root or the Keys page alone.
    pub fn open_at(&mut self, page: PageId) {
        self.close();
        if let PageId::Group(g) = page {
            let at = Group::ALL.iter().position(|&x| x == g).unwrap_or(0);
            self.stack.push((PageId::Root, at));
        }
        self.stack.push((page, 0));
    }

    pub fn close(&mut self) {
        self.stack.clear();
        self.filter.clear();
        self.found = 0;
    }

    /// The rows shown at `m` and `v`: the palette's matches while a filter is typed, else the
    /// page's rows; none while closed.
    pub fn rows(&self, m: &Modes, v: &RunView) -> Vec<Row> {
        if !self.filter.is_empty() {
            return palette(&self.filter, m, v)
                .into_iter()
                .map(Row::Entry)
                .collect();
        }
        match self.page() {
            None => Vec::new(),
            Some(PageId::Root) => Group::ALL.into_iter().map(Row::Group).collect(),
            Some(PageId::Group(g)) => group_entries(g).map(Row::Entry).collect(),
            Some(PageId::Keys) => {
                let mut rows: Vec<Row> = FIXED_ROWS
                    .iter()
                    .map(|&(keys, does)| Row::Fixed(keys, does))
                    .collect();
                for g in Group::ALL {
                    rows.push(Row::Header(g));
                    rows.extend(entries().filter(|e| e.group == g).map(Row::Entry));
                }
                rows
            }
        }
    }

    /// The selection among the selectable rows shown.
    pub fn selected(&self) -> usize {
        if self.filter.is_empty() {
            self.stack.last().map_or(0, |&(_, s)| s)
        } else {
            self.found
        }
    }

    fn select(&mut self, s: usize) {
        if !self.filter.is_empty() {
            self.found = s;
        } else if let Some(top) = self.stack.last_mut() {
            top.1 = s;
        }
    }

    /// Takes one input at `m` and `v`; gives what the run does, if anything.
    pub fn input(&mut self, i: MenuInput, m: &Modes, v: &RunView) -> Option<Activated> {
        match i {
            MenuInput::Menu => {
                match self.page() {
                    None | Some(PageId::Keys) => self.open_at(PageId::Root),
                    Some(_) => self.close(),
                }
                return None;
            }
            MenuInput::Keys => {
                match self.page() {
                    Some(PageId::Keys) => self.close(),
                    _ => self.open_at(PageId::Keys),
                }
                return None;
            }
            _ if !self.is_open() => return None,
            _ => {}
        }
        let rows = self.rows(m, v);
        let picks: Vec<Row> = rows.into_iter().filter(Row::selectable).collect();
        let n = picks.len();
        let s = self.selected().min(n.saturating_sub(1));
        match i {
            MenuInput::Up => self.select(s.saturating_sub(1)),
            MenuInput::Down => self.select((s + 1).min(n.saturating_sub(1))),
            MenuInput::Scroll(lines) => {
                let to = s as i64 - i64::from(lines);
                self.select(to.clamp(0, n.saturating_sub(1) as i64) as usize);
            }
            MenuInput::Activate | MenuInput::Right => match picks.get(s) {
                Some(Row::Group(g)) => self.stack.push((PageId::Group(*g), 0)),
                Some(Row::Entry(e)) if i == MenuInput::Activate => {
                    return Some(match e.kind {
                        EntryKind::Toggle(t) => Activated::Toggle(t),
                        EntryKind::Action(a) => Activated::Action(a.act),
                    });
                }
                _ => {}
            },
            MenuInput::Back => self.back(),
            MenuInput::Left => {
                if self.filter.is_empty() && self.stack.len() > 1 {
                    self.stack.pop();
                }
            }
            MenuInput::Type(c) => {
                self.filter.push(c);
                self.found = 0;
            }
            MenuInput::Erase => {
                self.filter.pop();
                self.found = 0;
            }
            MenuInput::Menu | MenuInput::Keys => {}
        }
        None
    }

    /// `Esc`: clears the filter, else goes back a page, else closes.
    fn back(&mut self) {
        if !self.filter.is_empty() {
            self.filter.clear();
            self.found = 0;
        } else {
            self.stack.pop();
        }
    }
}
