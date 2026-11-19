//! The menu's state: a stack of pages with a selection each, the palette's filter, and the
//! edit mode of a number (`Enter` on it; the arrows step it, `Enter` or `Esc` leave). A page
//! is a list of rows built from the key tables when it is shown, so its values are live.

use super::filter::palette;
use super::{Entry, EntryKind, FIXED_ROWS, Group, RunView, entries, group_entries};
use crate::entities::belt::Item;
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
    /// The chooser, a dynamic page, its rows built each frame from the run: the entities
    /// library's entries in [`crate::entities::library::Library::order`]
    /// (`crate::entities::picker`). `` ` `` and the `entities` group's `choose an entity`
    /// open it.
    Entities,
    /// The structures' chooser, a dynamic page as [`PageId::Entities`] is: the structures
    /// library's entries in its order (`crate::structures::words`). The `structures` group's
    /// `choose a structure` opens it.
    Structures,
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
    /// A chooser's entry, an entity or a structure, drawn on [`ITEM_LINES`] lines: `Enter`
    /// hands it to placement mode, a digit puts it in that belt slot.
    Item(Item),
}

/// The lines an entry of the chooser takes: its words and its clips beside a 4 x 4 cell tile.
pub const ITEM_LINES: usize = 4;

impl Row {
    /// Whether the selection can rest on the row.
    pub fn selectable(&self) -> bool {
        matches!(self, Self::Group(_) | Self::Entry(_) | Self::Item(_))
    }
}

/// What the menu hands the run to do: a toggle's flip, an action (a number's step of an
/// action is its [`Act::signed`]), a toggle's number stepped up (`1`) or down (`-1`), or the
/// reason an unavailable entry did nothing.
#[derive(Clone, Copy)]
pub enum Activated {
    Toggle(&'static Toggle),
    Action(Act),
    Step(&'static Toggle, i32),
    Refused(&'static str),
    /// A chooser's `Enter` on `item`: into the belt's selected slot (or the slot that holds
    /// it selected), the menu closed, placement mode on.
    Choose {
        item: Item,
    },
    /// A digit on a chooser's `item`: into belt slot `slot` (0-based), the menu staying open.
    Assign {
        item: Item,
        slot: usize,
    },
}

impl std::fmt::Debug for Activated {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Toggle(t) => write!(f, "Toggle({})", t.name),
            Self::Action(a) => write!(f, "Action({a:?})"),
            Self::Step(t, d) => write!(f, "Step({}, {d})", t.name),
            Self::Refused(why) => write!(f, "Refused({why})"),
            Self::Choose { item } => write!(f, "Choose({item:?})"),
            Self::Assign { item, slot } => write!(f, "Assign({item:?}, slot {slot})"),
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
    /// `` ` `` while open: on the chooser hands the selected entry over as `Enter` does,
    /// elsewhere shows the chooser.
    Entities,
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
    /// Whether the selected number is being edited.
    edit: bool,
    /// The arrow held in the edit mode: its direction, when it was pressed (the run's clock,
    /// ms) and the repeats it has given (`route::step_at`).
    pub(crate) hold: Option<(i32, f64, u32)>,
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

    /// Whether a number is being edited: the arrows step it.
    pub fn editing(&self) -> bool {
        self.edit
    }

    /// Opens the chooser alone (so `Esc` closes it), its selection on its `selected`th row.
    pub fn open_entities(&mut self, selected: usize) {
        self.close();
        self.stack.push((PageId::Entities, selected));
    }

    /// Opens the structures' chooser alone, its selection on its `selected`th row.
    pub fn open_structures(&mut self, selected: usize) {
        self.close();
        self.stack.push((PageId::Structures, selected));
    }

    /// Whether a chooser (the entities' or the structures') is the page shown, no filter typed.
    pub fn choosing(&self) -> bool {
        matches!(self.page(), Some(PageId::Entities | PageId::Structures)) && self.filter.is_empty()
    }

    /// The selected row shown at `m` and `v`, when one is.
    pub fn selected_row(&self, m: &Modes, v: &RunView) -> Option<Row> {
        let picks: Vec<Row> = self
            .rows(m, v)
            .into_iter()
            .filter(Row::selectable)
            .collect();
        let s = self.selected().min(picks.len().saturating_sub(1));
        picks.get(s).copied()
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
        self.edit = false;
        self.hold = None;
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
            Some(PageId::Entities) => {
                let order = m.walkers.library.order();
                if order.is_empty() {
                    return vec![Row::Item(Item::Entity(0))];
                }
                order
                    .into_iter()
                    .map(|i| Row::Item(Item::Entity(i)))
                    .collect()
            }
            Some(PageId::Structures) => m
                .structures
                .order()
                .into_iter()
                .map(|j| Row::Item(Item::Structure(j)))
                .collect(),
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
            MenuInput::Entities => {
                if self.choosing() {
                    return match self.selected_row(m, v) {
                        Some(Row::Item(item)) => Some(Activated::Choose { item }),
                        _ if self.page() == Some(PageId::Entities) => Some(Activated::Choose {
                            item: Item::Entity(m.walkers.library.selected),
                        }),
                        _ => None,
                    };
                }
                self.open_entities(0);
                return None;
            }
            MenuInput::Type(c) if self.choosing() && ('1'..='9').contains(&c) => {
                let slot = usize::from(c as u8 - b'1');
                return match self.selected_row(m, v) {
                    Some(Row::Item(item)) => Some(Activated::Assign { item, slot }),
                    _ => None,
                };
            }
            _ => {}
        }
        let rows = self.rows(m, v);
        let picks: Vec<Row> = rows.into_iter().filter(Row::selectable).collect();
        let n = picks.len();
        let s = self.selected().min(n.saturating_sub(1));
        if self.edit {
            let number = match picks.get(s) {
                Some(Row::Entry(e)) if e.is_number() => Some(*e),
                _ => None,
            };
            match (i, number) {
                (MenuInput::Up | MenuInput::Down, Some(e)) => {
                    let d = if i == MenuInput::Up { 1 } else { -1 };
                    return Some(match e.kind {
                        EntryKind::Toggle(t) => Activated::Step(t, d),
                        EntryKind::Action(a) => Activated::Action(a.act.signed(d)),
                    });
                }
                (MenuInput::Activate | MenuInput::Back, _) => {
                    self.edit = false;
                    self.hold = None;
                    return None;
                }
                _ => {
                    // Anything else leaves the mode and then does what it does on the page.
                    self.edit = false;
                    self.hold = None;
                }
            }
        }
        match i {
            MenuInput::Up => self.select(s.saturating_sub(1)),
            MenuInput::Down => self.select((s + 1).min(n.saturating_sub(1))),
            MenuInput::Scroll(lines) => {
                let to = s as i64 - i64::from(lines);
                self.select(to.clamp(0, n.saturating_sub(1) as i64) as usize);
            }
            MenuInput::Activate | MenuInput::Right => match picks.get(s) {
                Some(Row::Group(g)) => self.stack.push((PageId::Group(*g), 0)),
                Some(Row::Item(item)) if i == MenuInput::Activate => {
                    return Some(Activated::Choose { item: *item });
                }
                Some(Row::Entry(e)) if i == MenuInput::Activate => {
                    if let Err(why) = e.available(m, v) {
                        return Some(Activated::Refused(why));
                    }
                    if e.is_number() {
                        self.edit = true;
                        return None;
                    }
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
            MenuInput::Menu | MenuInput::Keys | MenuInput::Entities => {}
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
