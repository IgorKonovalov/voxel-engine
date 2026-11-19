//! The belt and placement mode: nine slots, each holding a library entry or nothing, one
//! selected; while the mode is on the selected slot's entry stands at the aim as the ghost (a
//! real instance in the set, `super::Walkers::preview`) and a click places it. The belt is
//! drawn along the bottom of the window only in the mode ([`append_belt`]). Where the ghost
//! stands is [`stand`]: the camera ray's first world hit within [`PLACE_REACH`], never nearer
//! than the model's own size allows, dropped to the floor under a wall or a ceiling hit (a
//! surface body's on the face it hits, [`Stand::Surface`]). A slot holds an entity or a
//! structure ([`Item`]); a structure's box stands by [`stand_box`].

use vxe_render::{HudBox, HudColour, HudTile};

use super::library::BUILTIN_NAME;
use crate::hud::Modes;

/// What a belt slot holds: an entities library entry, or a structures library entry
/// (`crate::structures::Library`), by index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Entity(usize),
    Structure(usize),
}

/// What the overlay lays over a belt's or a chooser's rows (`vxe_render::HudPaint`): the
/// spans recoloured and the icons' tiles, kept across frames.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Paint {
    pub spans: Vec<(HudBox, HudColour)>,
    pub tiles: Vec<HudTile>,
}

impl Paint {
    pub fn clear(&mut self) {
        self.spans.clear();
        self.tiles.clear();
    }

    /// The overlay's paint over these spans and tiles, with the `backdrops`' boxes.
    pub fn with<'a>(&'a self, backdrops: &'a [HudBox]) -> vxe_render::HudPaint<'a> {
        vxe_render::HudPaint {
            backdrops,
            spans: &self.spans,
            tiles: &self.tiles,
        }
    }
}

/// The belt's slots, `1`..`9`.
pub const SLOTS: usize = 9;
/// A slot's cells across, its frame's two columns included.
pub const SLOT_COLS: usize = 8;
/// The belt's rows: the name row, the frame's top, four rows of the slots, the frame's
/// bottom.
pub const BELT_ROWS: usize = 7;
/// The belt's cells across.
pub const BELT_COLS: usize = SLOTS * SLOT_COLS;
/// The ghost's turn step about the up axis, in degrees (owner, 2026-11-18: 22.5, Valheim's).
pub const TURN_STEP_DEG: f32 = 22.5;
/// How far the camera ray looks for the ghost's stand, in voxels: 30 m.
pub const PLACE_REACH: f32 = 300.0;
/// The ghost is never nearer the eye than this, in voxels (3 m), ...
pub const NEAR_MIN: f32 = 30.0;
/// ... nor nearer than this many times its model's longest rest-pose extent.
pub const NEAR_SCALE: f32 = 1.5;
/// How far below a stand point the floor is looked for, in voxels: 10 m.
pub const FLOOR_DROP: f32 = 100.0;

/// The belt: what each slot holds, the selected slot, whether placement mode is on, the
/// ghost's turn in [`TURN_STEP_DEG`] steps (a structure's in quarter turns, four steps each),
/// and when the selection last changed (the run's clock, ms; the name row shows for
/// `hud::BANNER_MS` after it).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Belt {
    pub slots: [Option<Item>; SLOTS],
    pub selected: usize,
    pub on: bool,
    pub turn: u8,
    pub changed_at: Option<f64>,
    filled: bool,
}

impl Belt {
    /// Fills the slots once from `entities` (the entities library's order,
    /// [`super::library::Library::order`])
    /// and then `structures` (the structures library's), the first nine; later calls change
    /// nothing, so a slot keeps its item for the run.
    pub fn fill(&mut self, entities: &[usize], structures: &[usize]) {
        if self.filled {
            return;
        }
        self.filled = true;
        let items = entities
            .iter()
            .map(|&i| Item::Entity(i))
            .chain(structures.iter().map(|&j| Item::Structure(j)));
        for (s, i) in self.slots.iter_mut().zip(items) {
            *s = Some(i);
        }
    }

    /// Whether [`Belt::fill`] has run.
    pub fn filled(&self) -> bool {
        self.filled
    }

    /// The selected slot's item, when it holds one.
    pub fn item(&self) -> Option<Item> {
        self.slots[self.selected]
    }

    /// The selected slot's entity, when it holds one.
    pub fn entry(&self) -> Option<usize> {
        match self.item()? {
            Item::Entity(i) => Some(i),
            Item::Structure(_) => None,
        }
    }

    /// The selected slot's structure, when it holds one.
    pub fn structure(&self) -> Option<usize> {
        match self.item()? {
            Item::Structure(j) => Some(j),
            Item::Entity(_) => None,
        }
    }

    /// The slot holding `item`, when one does.
    pub fn slot_of(&self, item: Item) -> Option<usize> {
        self.slots.iter().position(|&s| s == Some(item))
    }

    /// Moves the selection (the frame) to slot `s` (0-based) at `now`, the mode as it was.
    pub fn point(&mut self, s: usize, now: f64) {
        self.selected = s % SLOTS;
        self.changed_at = Some(now);
    }

    /// Selects slot `s` (0-based) at `now`, the mode on.
    pub fn select(&mut self, s: usize, now: f64) {
        self.point(s, now);
        self.on = true;
    }

    /// The selection moved `d` slots (positive the next), wrapping at nine, the mode on.
    pub fn step(&mut self, d: i32, now: f64) {
        let s = (self.selected as i32 + d).rem_euclid(SLOTS as i32) as usize;
        self.select(s, now);
    }

    /// Puts `i` in slot `s`; nothing leaves the belt but by a full one. Slot `s` empty or
    /// holding `i`: `i` moves there from any other slot. Slot `s` holding another item: when
    /// `i` is in slot `t` the two swap (the same call undoes it); when `i` is in no slot the
    /// holder moves to the first empty slot after `s` (wrapping), or with none it leaves.
    /// Gives the holder moved and where to (`None` when it left the belt).
    pub fn assign(&mut self, s: usize, i: Item) -> Option<(Item, Option<usize>)> {
        let s = s % SLOTS;
        let from = self.slot_of(i);
        let old = self.slots[s].filter(|&o| o != i);
        if let Some(t) = from {
            self.slots[t] = None;
        }
        self.slots[s] = Some(i);
        let old = old?;
        let to = from.or_else(|| {
            (1..SLOTS)
                .map(|d| (s + d) % SLOTS)
                .find(|&t| self.slots[t].is_none())
        });
        if let Some(t) = to {
            self.slots[t] = Some(old);
        }
        Some((old, to))
    }

    /// The ghost turned one more step: a structure's a quarter turn (four steps, a stamp's
    /// turns being the cube's).
    pub fn turn_step(&mut self) {
        let steps = if self.structure().is_some() { 4 } else { 1 };
        self.turn = (self.turn + steps) % (360.0 / TURN_STEP_DEG) as u8;
    }

    /// The turn in whole quarter turns, as a structure takes it.
    pub fn quarters(&self) -> u8 {
        self.turn / 4
    }

    /// The turn in radians.
    pub fn turn_rad(&self) -> f32 {
        (f32::from(self.turn) * TURN_STEP_DEG).to_radians()
    }

    /// The mode ends: the turn goes back to none.
    pub fn end(&mut self) {
        self.on = false;
        self.turn = 0;
    }
}

/// Which rule [`stand`] stands a model by: a ground body's (and a body-less entry's) on the
/// floor, or a surface body's on whatever face the aim hits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stand {
    Floor,
    Surface,
}

/// Where a model `extent` voxels across (x, y, z, standing in its rest pose) stands for an
/// eye at `eye` looking along the unit `fwd`, and the up it stands on, with `cast(origin,
/// dir, max)` the world's first hit along a unit ray within `max` voxels (its distance and the
/// entered face's outward normal; distance 0 when the origin is inside a solid voxel):
///
/// - `near = max(NEAR_MIN, NEAR_SCALE x the longest extent)`;
/// - [`Stand::Surface`]: a hit at `near` or beyond on any face: the point is the hit and the
///   up the face's normal;
/// - a hit at `near` or beyond on an upward face: the feet at the hit;
/// - a hit at `near` or beyond on a wall or a ceiling: the hit moved back along the view by
///   half the model's footprint (its longer horizontal extent), dropped to the floor;
/// - a hit nearer than `near`, or none within [`PLACE_REACH`]: the view's point at `near`,
///   dropped to the floor.
///
/// Dropped means the first floor within [`FLOOR_DROP`] below the point; when the point is
/// inside a solid voxel the drop starts at the eye's height above it instead. No floor found,
/// the point stands where it is. Every point but a surface hit's has the up `+z`.
pub fn stand(
    eye: [f32; 3],
    fwd: [f32; 3],
    extent: [f32; 3],
    kind: Stand,
    cast: impl Fn([f32; 3], [f32; 3], f32) -> Option<(f32, [f32; 3])>,
) -> ([f32; 3], [f32; 3]) {
    const UP: [f32; 3] = [0.0, 0.0, 1.0];
    let longest = extent[0].max(extent[1]).max(extent[2]);
    let near = NEAR_MIN.max(NEAR_SCALE * longest);
    let along = |t: f32| std::array::from_fn(|k| eye[k] + fwd[k] * t);
    let point = match cast(eye, fwd, PLACE_REACH) {
        Some((t, n)) if t >= near => {
            if kind == Stand::Surface && n != [0.0; 3] {
                return (along(t), n);
            }
            if n[2] > 0.5 {
                return (along(t), UP);
            }
            along(t - 0.5 * extent[0].max(extent[1]))
        }
        _ => along(near),
    };
    (drop_to_floor(eye, point, &cast), UP)
}

/// `point` dropped to the first floor within [`FLOOR_DROP`] below it; when it is inside a
/// solid voxel the drop starts at the eye's height above it instead. No floor found, the point
/// stands where it is.
fn drop_to_floor(
    eye: [f32; 3],
    point: [f32; 3],
    cast: &impl Fn([f32; 3], [f32; 3], f32) -> Option<(f32, [f32; 3])>,
) -> [f32; 3] {
    let down = [0.0, 0.0, -1.0];
    match cast(point, down, FLOOR_DROP) {
        Some((t, _)) if t > 0.0 => [point[0], point[1], point[2] - t],
        Some(_) if eye[2] > point[2] => {
            let from = [point[0], point[1], eye[2]];
            match cast(from, down, eye[2] - point[2] + FLOOR_DROP) {
                Some((t, _)) if t > 0.0 => [from[0], from[1], from[2] - t],
                _ => point,
            }
        }
        _ => point,
    }
}

/// How a structure's box finds its base ([`stand_box`]): on the terrain under its footprint,
/// or where the aim points, in the air too (the `structures` group's `snap`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Snap {
    #[default]
    Ground,
    Free,
}

impl Snap {
    pub fn name(self) -> &'static str {
        match self {
            Self::Ground => "ground",
            Self::Free => "free",
        }
    }

    pub fn other(self) -> Self {
        match self {
            Self::Ground => Self::Free,
            Self::Free => Self::Ground,
        }
    }
}

/// The most columns [`stand_box`]'s ground snap casts along an axis of the footprint.
pub const SNAP_COLUMNS: usize = 8;
/// How far over the stand point the ground snap's columns start, in voxels: inside the air
/// voxel over a floor hit.
pub const SNAP_START: f32 = 0.5;

/// Where a box `size` voxels across (x, y, z) stands for an eye at `eye` looking along the
/// unit `fwd` by the rule `snap`, as its minimum corner (`cast` as [`stand`]'s).
///
/// `p`, by [`Snap::Ground`]: the camera ray's first world hit within [`PLACE_REACH`] (on an
/// upward face, as it is; on a wall or a ceiling, half a voxel back along the view and
/// dropped to the floor), else the view's point at [`NEAR_MIN`] dropped to the floor. By
/// [`Snap::Free`]: the first hit on any face as it is, else the view's point at
/// [`PLACE_REACH`].
///
/// `h` is `fwd` level and of unit length (`+x` when `fwd` is vertical); the box's support
/// along `h` is `s = (|h.x| size.x + |h.y| size.y) / 2` and its footprint is centred on
/// `p + h s`. So its near side or corner is at `p` and it grows away from the eye, which is
/// never inside it.
///
/// The base: free, `p.z`. Ground, the lowest column's top over a lattice of
/// `n = min(SNAP_COLUMNS, ceil(size / 8) + 1)` columns an axis spanning the footprint corner
/// to corner, each cast straight down from `p.z + SNAP_START` to `p.z - FLOOR_DROP`: a hit
/// is its height, a cast that starts inside solid counts as its start, a miss as
/// `p.z - FLOOR_DROP`. The casts start just over `p`, not high above it, so a roof over the
/// footprint (a cave's) is never read as the ground; since the lowest column wins, a column
/// that starts in the terrain uphill changes nothing.
pub fn stand_box(
    eye: [f32; 3],
    fwd: [f32; 3],
    size: [f32; 3],
    snap: Snap,
    cast: impl Fn([f32; 3], [f32; 3], f32) -> Option<(f32, [f32; 3])>,
) -> [f32; 3] {
    let along = |t: f32| std::array::from_fn(|k| eye[k] + fwd[k] * t);
    let p = match (snap, cast(eye, fwd, PLACE_REACH)) {
        (Snap::Free, Some((t, _))) if t > 0.0 => along(t),
        (Snap::Free, _) => along(PLACE_REACH),
        (Snap::Ground, Some((t, n))) if t > 0.0 && n[2] > 0.5 => along(t),
        (Snap::Ground, Some((t, _))) if t > 0.0 => {
            drop_to_floor(eye, along((t - 0.5).max(0.0)), &cast)
        }
        (Snap::Ground, _) => drop_to_floor(eye, along(NEAR_MIN), &cast),
    };
    let len = fwd[0].hypot(fwd[1]);
    let h = if len < 1e-6 {
        [1.0, 0.0]
    } else {
        [fwd[0] / len, fwd[1] / len]
    };
    let s = (h[0].abs() * size[0] + h[1].abs() * size[1]) / 2.0;
    let c = [p[0] + h[0] * s, p[1] + h[1] * s];
    let lo = [c[0] - size[0] / 2.0, c[1] - size[1] / 2.0];
    let base = match snap {
        Snap::Free => p[2],
        Snap::Ground => lowest_column(lo, [size[0], size[1]], p[2], &cast),
    };
    [lo[0], lo[1], base]
}

/// [`stand_box`]'s ground base: the lowest column top over the footprint from `lo`, `size`
/// across, for a stand point at height `z`.
fn lowest_column(
    lo: [f32; 2],
    size: [f32; 2],
    z: f32,
    cast: &impl Fn([f32; 3], [f32; 3], f32) -> Option<(f32, [f32; 3])>,
) -> f32 {
    let n = size.map(|v| SNAP_COLUMNS.min((v / 8.0).ceil() as usize + 1).max(2));
    let top = z + SNAP_START;
    let mut base = f32::INFINITY;
    for i in 0..n[0] {
        let x = lo[0] + size[0] * i as f32 / (n[0] - 1) as f32;
        for j in 0..n[1] {
            let y = lo[1] + size[1] * j as f32 / (n[1] - 1) as f32;
            let column = match cast([x, y, top], [0.0, 0.0, -1.0], SNAP_START + FLOOR_DROP) {
                Some((t, _)) => top - t,
                None => z - FLOOR_DROP,
            };
            base = base.min(column);
        }
    }
    base
}

/// The entities in the belt's order: [`super::library::Library::order`] with the walker's
/// entry ([`BUILTIN_NAME`]) moved to the front, after `--model`'s when there is one.
pub fn entity_order(lib: &super::library::Library) -> Vec<usize> {
    let mut order = lib.order();
    let flagged = order
        .iter()
        .take_while(|&&i| lib.entries[i].flagged)
        .count();
    if let Some(at) = order
        .iter()
        .position(|&i| lib.entries[i].name == BUILTIN_NAME)
        .filter(|&at| at >= flagged)
    {
        let w = order.remove(at);
        order.insert(flagged, w);
    }
    order
}

/// The name row's words for the selected slot: the entry's name (grey with its error after
/// it for an entry in error, and where the error starts in the words), a structure's name and
/// size (`crate::structures::words::write_name`), or `slot <n> empty (choose with `)`.
fn name_words(m: &Modes, out: &mut String) -> (HudColour, Option<usize>) {
    use std::fmt::Write as _;
    let (belt, lib) = (&m.walkers.belt, &m.walkers.library);
    match belt.item() {
        None => {
            let _ = write!(out, "slot {} empty  (choose with `)", belt.selected + 1);
            (HudColour::Yellow, None)
        }
        Some(Item::Structure(j)) => {
            let (grey, reason) = crate::structures::words::write_name(m, j, out);
            let c = if grey {
                HudColour::Grey
            } else {
                HudColour::Yellow
            };
            (c, reason)
        }
        Some(Item::Entity(i)) => match lib.entries.get(i) {
            None => {
                out.push_str(BUILTIN_NAME);
                (HudColour::Yellow, None)
            }
            Some(e) => match e.error() {
                Some(err) => {
                    let _ = write!(out, "{}: {err}", e.name);
                    (HudColour::Grey, Some(e.name.len() + 2))
                }
                None => {
                    out.push_str(&e.name);
                    (HudColour::Yellow, None)
                }
            },
        },
    }
}

/// A slot's words on its second row: its item's name, cut to the slot's six inner cells.
fn slot_words(m: &Modes, i: Item) -> &str {
    let name = match i {
        Item::Entity(i) => m
            .walkers
            .library
            .entries
            .get(i)
            .map_or(BUILTIN_NAME, |e| e.name.as_str()),
        Item::Structure(j) => m.structures.entries.get(j).map_or("", |e| e.name.as_str()),
    };
    let mut end = name.len().min(SLOT_COLS - 2);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    &name[..end]
}

/// The overlay tile an item's icon takes: an entity's its index, a structure's after the
/// entities' (`entities::icon::Icons` holds both).
pub fn tile_of(m: &Modes, i: Item) -> usize {
    match i {
        Item::Entity(i) => i,
        Item::Structure(j) => m.walkers.library.entries.len().max(1) + j,
    }
}

/// Appends the belt to the overlay's `text` and `colours` (one colour per line) for a grid of
/// `grid` = (columns, rows) cells: the text is cut to leave the grid's last [`BELT_ROWS`]
/// rows and padded down to them, then the belt's rows, centred: the selected slot's name
/// (shown for `banner_ms` after the belt's `changed_at`, blank after), the selected slot's
/// frame top (`+------+`), the four slot rows (each slot's digit, then its name on the next
/// row, or its entry's icon from the slot's third column once made; `|` at the selected
/// slot's sides; the digits' row ends with `free` in yellow while the modes' snap is
/// [`Snap::Free`]), the frame's bottom. `paint` gains the frame's cells as yellow spans (the
/// rows are white) and the icons' tiles, after what it holds (a chooser's tiles above the
/// belt). Gives the backdrop's box: the slots' rows, the belt's columns plus one each side,
/// cut to the grid.
pub fn append_belt(
    text: &mut String,
    colours: &mut Vec<HudColour>,
    grid: (usize, usize),
    m: &Modes,
    now_ms: f64,
    banner_ms: f64,
    paint: &mut Paint,
) -> HudBox {
    let (walkers, belt) = (&m.walkers, &m.walkers.belt);
    let (cols, rows) = grid;
    let first = rows.saturating_sub(BELT_ROWS);
    // Keep at most `first` of the rows there (rows are joined by `\n`).
    if first == 0 {
        text.clear();
    } else if let Some((at, _)) = text.match_indices('\n').nth(first - 1) {
        text.truncate(at);
    }
    let n = if text.is_empty() {
        0
    } else {
        text.split('\n').count()
    };
    colours.resize(n, HudColour::White);
    let push_row = |text: &mut String, colours: &mut Vec<HudColour>, row: &str, c| {
        if !colours.is_empty() {
            text.push('\n');
        }
        text.push_str(row.trim_end());
        colours.push(c);
    };
    while colours.len() < first {
        push_row(text, colours, "", HudColour::White);
    }
    let x0 = cols.saturating_sub(BELT_COLS) / 2;
    let mut row = String::with_capacity(cols);
    // The name row.
    let shown = belt.changed_at.is_some_and(|at| now_ms - at < banner_ms);
    let mut colour = HudColour::White;
    let mut reason = None;
    if shown {
        let mut words = String::new();
        (colour, reason) = name_words(m, &mut words);
        let at = x0 + BELT_COLS.saturating_sub(words.len()) / 2;
        row.extend(std::iter::repeat_n(' ', at));
        row.push_str(&words);
        reason = reason.map(|r| at + r);
    }
    push_row(text, colours, &cut_to(&row, cols), colour);
    // An entry's error after its grey name is red.
    if let Some(col) = reason.filter(|&c| c < row.len().min(cols)) {
        let cell = HudBox {
            col: col as u32,
            row: first as u32,
            cols: (row.len().min(cols) - col) as u32,
            rows: 1,
        };
        paint.spans.push((cell, HudColour::Red));
    }
    let sel = x0 + belt.selected * SLOT_COLS;
    let frame = |row: &mut String| {
        row.clear();
        row.extend(std::iter::repeat_n(' ', sel));
        row.push('+');
        row.extend(std::iter::repeat_n('-', SLOT_COLS - 2));
        row.push('+');
    };
    frame(&mut row);
    push_row(text, colours, &cut_to(&row, cols), HudColour::White);
    let icon = |s: usize| {
        belt.slots[s]
            .map(|i| tile_of(m, i))
            .filter(|&t| walkers.icons.ready(t))
    };
    let mut free = None;
    for r in 0..4 {
        row.clear();
        row.extend(std::iter::repeat_n(' ', x0));
        for s in 0..SLOTS {
            let side = if s == belt.selected { '|' } else { ' ' };
            let start = row.len();
            row.push(side);
            match r {
                0 => row.push(char::from(b'1' + s as u8)),
                1 if icon(s).is_none() => {
                    if let Some(i) = belt.slots[s] {
                        row.push_str(slot_words(m, i));
                    }
                }
                _ => {}
            }
            while row.len() < start + SLOT_COLS - 1 {
                row.push(' ');
            }
            row.push(side);
        }
        if r == 0 && m.snap == Snap::Free {
            row.push(' ');
            free = Some(row.len());
            row.push_str(Snap::Free.name());
        }
        push_row(text, colours, &cut_to(&row, cols), HudColour::White);
    }
    frame(&mut row);
    push_row(text, colours, &cut_to(&row, cols), HudColour::White);
    let top = first + 1;
    let cell = |col: usize, row: usize, cols: usize, rows: usize| HudBox {
        col: col as u32,
        row: row as u32,
        cols: cols as u32,
        rows: rows as u32,
    };
    for (b, c) in [
        cell(sel, top, SLOT_COLS, 1),
        cell(sel, top + 5, SLOT_COLS, 1),
        cell(sel, top + 1, 1, 4),
        cell(sel + SLOT_COLS - 1, top + 1, 1, 4),
    ]
    .into_iter()
    .zip(std::iter::repeat(HudColour::Yellow))
    {
        paint.spans.push((b, c));
    }
    if let Some(col) = free.filter(|&c| c < cols) {
        let n = Snap::Free.name().len().min(cols - col);
        paint
            .spans
            .push((cell(col, top + 1, n, 1), HudColour::Yellow));
    }
    for s in 0..SLOTS {
        if let Some(i) = icon(s) {
            paint.tiles.push(HudTile {
                col: (x0 + s * SLOT_COLS + 2) as u32,
                row: (top + 1) as u32,
                index: i as u32,
            });
        }
    }
    let col = x0.saturating_sub(1);
    let end = (x0 + BELT_COLS + 1).min(cols);
    HudBox {
        col: col as u32,
        row: top as u32,
        cols: end.saturating_sub(col) as u32,
        rows: rows.saturating_sub(top).min(BELT_ROWS - 1) as u32,
    }
}

/// `row` cut to `cols` bytes (its cells are ASCII).
fn cut_to(row: &str, cols: usize) -> String {
    row[..row.len().min(cols)].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_belt_steps_wraps_and_assigns() {
        let mut b = Belt::default();
        let e = Item::Entity;
        b.fill(&[1, 0], &[0]);
        b.fill(&[5], &[]);
        assert_eq!(
            b.slots[..4],
            [Some(e(1)), Some(e(0)), Some(Item::Structure(0)), None]
        );
        b.step(-1, 0.0);
        assert_eq!((b.selected, b.on), (8, true));
        b.step(1, 0.0);
        assert_eq!(b.selected, 0);
        b.assign(4, e(1));
        let s = Some(Item::Structure(0));
        assert_eq!(b.slots[..5], [None, Some(e(0)), s, None, Some(e(1))]);
        assert_eq!(b.slot_of(e(1)), Some(4));
        for _ in 0..17 {
            b.turn_step();
        }
        assert_eq!(b.turn, 1);
        b.end();
        assert_eq!((b.on, b.turn), (false, 0));
    }

    /// The stand rules against a floor at z = 0 and a wall at x = 100, the eye at z = 17.
    #[test]
    fn the_stand_rule_over_a_floor_and_a_wall() {
        let cast = |o: [f32; 3], d: [f32; 3], max: f32| -> Option<(f32, [f32; 3])> {
            if o[2] < 0.0 || o[0] > 100.0 {
                return Some((0.0, [0.0; 3]));
            }
            let mut best: Option<(f32, [f32; 3])> = None;
            if d[2] < 0.0 {
                let t = o[2] / -d[2];
                best = Some((t, [0.0, 0.0, 1.0]));
            }
            if d[0] > 0.0 {
                let t = (100.0 - o[0]) / d[0];
                if best.is_none_or(|(b, _)| t < b) {
                    best = Some((t, [-1.0, 0.0, 0.0]));
                }
            }
            best.filter(|(t, _)| *t <= max)
        };
        let eye = [0.0, 0.0, 17.0];
        let a = 40f32.to_radians();
        let fwd = [a.cos(), 0.0, -a.sin()];
        let walker = [6.0, 3.0, 15.0];
        let (feet, up) = stand(eye, fwd, walker, Stand::Floor, cast);
        assert_eq!(up, [0.0, 0.0, 1.0]);
        assert!((feet[0] - 30.0 * a.cos()).abs() < 1e-3, "{feet:?}");
        assert!(feet[2].abs() < 1e-3, "{feet:?}");
        let level = [1.0, 0.0, 0.0];
        let (feet, up) = stand(eye, level, walker, Stand::Floor, cast);
        assert!(
            (feet[0] - 97.0).abs() < 1e-3 && feet[2].abs() < 1e-3,
            "{feet:?}"
        );
        assert_eq!(up, [0.0, 0.0, 1.0]);
        let z = [0.0, 0.0, 1.0];
        assert_eq!(stand(eye, z, walker, Stand::Floor, cast), ([0.0; 3], z));
        // A surface body stands on the wall's face itself, its up the face's normal; on the
        // floor's face as the floor rule does; with no hit it falls back to the floor rule.
        assert_eq!(
            stand(eye, level, walker, Stand::Surface, cast),
            ([100.0, 0.0, 17.0], [-1.0, 0.0, 0.0])
        );
        let (feet, up) = stand(eye, fwd, walker, Stand::Surface, cast);
        assert!(
            (feet[0] - 30.0 * a.cos()).abs() < 1e-3 && up == z,
            "{feet:?}"
        );
        assert_eq!(stand(eye, z, walker, Stand::Surface, cast), ([0.0; 3], z));
    }

    /// A cast over a step terrain, the ground's top at z = 100 for x < `step` and at `far`
    /// from it on (any y), counting the downward casts in `downs`: a downward ray's hit is
    /// its column's top (distance 0 inside solid); any other ray hits the near top's plane
    /// before `step`.
    fn step_cast(step: f32, far: f32, downs: &std::cell::Cell<usize>) -> Cast<'_> {
        Box::new(move |o, d, max| {
            let top = |x: f32| if x < step { 100.0 } else { far };
            if d == [0.0, 0.0, -1.0] {
                downs.set(downs.get() + 1);
                let t = (o[2] - top(o[0])).max(0.0);
                return (t <= max).then_some((t, [0.0, 0.0, 1.0]));
            }
            if d[2] >= 0.0 {
                return None;
            }
            let t = (o[2] - 100.0) / -d[2];
            (t <= max && o[0] + d[0] * t < step).then_some((t, [0.0, 0.0, 1.0]))
        })
    }

    /// A boxed world cast, as [`stand_box`] takes one.
    type Cast<'a> = Box<dyn Fn([f32; 3], [f32; 3], f32) -> Option<(f32, [f32; 3])> + 'a>;

    fn unit(v: [f32; 3]) -> [f32; 3] {
        let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        v.map(|c| c / l)
    }

    fn near(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|k| (a[k] - b[k]).abs() < 1e-3)
    }

    /// Done-when 1 of Plan 0083 Phase 7d: the eye at (0, 0, 110) looking at (50, 0, 100) on a
    /// step terrain (z 100 below x 300, z 70 from it on): the castle's box (600 x 600 x 500)
    /// takes its base from the lowest of its 8 x 8 columns; the step past the footprint
    /// leaves it at the hit; a valley past `FLOOR_DROP` caps it; the chapel's box (48 x 32)
    /// casts 7 x 5 columns; a wall inside the footprint (up to z 250) counts as the casts'
    /// start, so the lowest column, at 100, wins.
    #[test]
    fn a_snapped_box_sinks_to_its_lowest_column() {
        let eye = [0.0, 0.0, 110.0];
        let fwd = unit([50.0, 0.0, -10.0]);
        let castle = [600.0, 600.0, 500.0];
        let chapel = [48.0, 32.0, 40.0];
        let downs = std::cell::Cell::new(0);
        let corner = |step: f32, far: f32, size: [f32; 3]| {
            downs.set(0);
            let c = stand_box(eye, fwd, size, Snap::Ground, step_cast(step, far, &downs));
            (c, downs.get())
        };
        let (c, n) = corner(300.0, 70.0, castle);
        assert!(near(c, [50.0, -300.0, 70.0]), "{c:?}");
        assert_eq!(n, 64);
        let (c, _) = corner(700.0, 70.0, castle);
        assert!(near(c, [50.0, -300.0, 100.0]), "{c:?}");
        let (c, _) = corner(300.0, -200.0, castle);
        assert!(near(c, [50.0, -300.0, 0.0]), "{c:?}");
        let (c, n) = corner(80.0, 70.0, chapel);
        assert!(near(c, [50.0, -16.0, 70.0]), "{c:?}");
        assert_eq!(n, 35);
        let (c, _) = corner(80.0, 250.0, chapel);
        assert!(near(c, [50.0, -16.0, 100.0]), "{c:?}");
        // A box under 8 a side casts its four corners.
        let (_, n) = corner(300.0, 70.0, [6.0, 6.0, 6.0]);
        assert_eq!(n, 4);
    }

    /// Done-when 2 of Plan 0083 Phase 7d: with the snap free the box stands at the aim: no hit
    /// along +x puts its near side at `PLACE_REACH` at the eye's height in one cast; a hit on
    /// a ceiling 30 degrees up at t 100 is taken as it is (base 160); on the step terrain of
    /// [`a_snapped_box_sinks_to_its_lowest_column`] the base is the hit's 100, not 70.
    #[test]
    fn a_free_box_stands_where_the_aim_points() {
        let eye = [0.0, 0.0, 110.0];
        let chapel = [48.0, 32.0, 40.0];
        let calls = std::cell::Cell::new(0);
        let none = |_: [f32; 3], _: [f32; 3], _: f32| {
            calls.set(calls.get() + 1);
            None
        };
        let c = stand_box(eye, [1.0, 0.0, 0.0], chapel, Snap::Free, none);
        assert!(near(c, [PLACE_REACH, -16.0, 110.0]), "{c:?}");
        assert_eq!(PLACE_REACH, 300.0);
        assert_eq!(calls.get(), 1);
        let a = 30f32.to_radians();
        let up = [a.cos(), 0.0, a.sin()];
        let ceiling =
            |_: [f32; 3], d: [f32; 3], _: f32| (d[2] > 0.0).then_some((100.0, [0.0, 0.0, -1.0]));
        let c = stand_box(eye, up, chapel, Snap::Free, ceiling);
        assert!(near(c, [100.0 * a.cos(), -16.0, 160.0]), "{c:?}");
        let downs = std::cell::Cell::new(0);
        let fwd = unit([50.0, 0.0, -10.0]);
        let step = step_cast(300.0, 70.0, &downs);
        let c = stand_box(eye, fwd, [600.0, 600.0, 500.0], Snap::Free, step);
        assert!(near(c, [50.0, -300.0, 100.0]), "{c:?}");
        assert_eq!(downs.get(), 0);
    }

    /// Done-when 4 of Plan 0083 Phase 7d: a digit on an occupied slot swaps, and the same
    /// press undoes it; an item from no slot moves the holder to the next empty slot, or off
    /// a full belt; the item's own slot changes nothing.
    #[test]
    fn a_digit_on_an_occupied_slot_swaps() {
        let it = |c: char| Item::Entity(c as usize - 'A' as usize);
        let mut b = Belt::default();
        b.fill(&[0, 1, 2, 3], &[]);
        let row = |b: &Belt| b.slots[..5].to_vec();
        let abcd = |rest: Option<Item>| {
            let mut v: Vec<_> = "ABCD".chars().map(|c| Some(it(c))).collect();
            v.push(rest);
            v
        };
        assert_eq!(b.assign(3, it('A')), Some((it('D'), Some(0))));
        let mut want = abcd(None);
        want.swap(0, 3);
        assert_eq!(row(&b), want);
        assert_eq!(b.assign(0, it('A')), Some((it('D'), Some(3))));
        assert_eq!(row(&b), abcd(None));
        assert_eq!(b.assign(0, it('E')), Some((it('A'), Some(4))));
        let mut want = abcd(Some(it('A')));
        want[0] = Some(it('E'));
        assert_eq!(row(&b), want);
        let mut full = Belt::default();
        full.fill(&(0..9).collect::<Vec<_>>(), &[]);
        assert_eq!(full.assign(0, it('J')), Some((it('A'), None)));
        assert_eq!(full.slots[0], Some(it('J')));
        assert_eq!(full.slot_of(it('A')), None);
        let before = full.clone();
        assert_eq!(full.assign(1, it('B')), None);
        assert_eq!(full, before);
    }
}

#[cfg(test)]
mod reading {
    use super::*;

    /// The ground snap's cost on the outdoor hills (seed 1): the castle's box (600 x 600 x 500)
    /// stood from the outdoor spawn's eye at 16 yaws, 15 degrees down, over a generated block
    /// of 37 x 37 x 7 chunks around it (the castle's footprint inside it at every yaw); per re-stand the downward casts and the wall time
    /// (median and max of 20 a yaw), the columns that find no ground within 10 m, and the base
    /// against the free stand's height (the aim's hit). Printed, not asserted.
    /// Run by hand under the perf lock:
    /// `--run-ignored only -E 'test(=entities::belt::reading::the_snap_costs)'`.
    #[test]
    #[ignore = "a reading"]
    fn the_snap_costs() {
        use crate::outdoor::{OutdoorSource, Spawn};
        use vxe_world::{CHUNK_EDGE, ChunkExtent, World};
        let source = OutdoorSource::new(1);
        let (eye, _) = source.spawn(Spawn::Outside);
        let c = eye.0.map(|v| v.div_euclid(CHUNK_EDGE));
        // The aim's reach and the castle's diagonal past it, every way: 1150 voxels.
        let lo = [c[0] - 18, c[1] - 18, c[2] - 5];
        let hi = [c[0] + 19, c[1] + 19, c[2] + 2];
        let t = crate::now();
        let world = World::generate(
            ChunkExtent::new(lo, hi),
            &source,
            &crate::outdoor_materials(),
        );
        println!(
            "snap reading: {} chunks generated in {:.1} s",
            (0..3).map(|k| hi[k] - lo[k]).product::<i32>(),
            (crate::now() - t).as_secs_f64()
        );
        let run = crate::test_support::bare_run(world);
        let eye = eye.0.map(|v| v as f32 + 0.5);
        let castle = [600.0, 600.0, 500.0];
        let pitch = (-15f32).to_radians();
        for i in 0..16 {
            let yaw = (i as f32 * 22.5).to_radians();
            let fwd = [
                pitch.cos() * yaw.cos(),
                pitch.cos() * yaw.sin(),
                pitch.sin(),
            ];
            let (downs, misses) = (std::cell::Cell::new(0), std::cell::Cell::new(0));
            let cast = |o: [f32; 3], d: [f32; 3], m: f32| {
                let hit = run.cast_world(o, d, m);
                if d == [0.0, 0.0, -1.0] {
                    downs.set(downs.get() + 1);
                    misses.set(misses.get() + usize::from(hit.is_none()));
                }
                hit
            };
            let free = stand_box(eye, fwd, castle, Snap::Free, cast);
            let mut ms = Vec::with_capacity(20);
            let mut corner = [0.0; 3];
            for _ in 0..20 {
                downs.set(0);
                misses.set(0);
                let t = crate::now();
                corner = stand_box(eye, fwd, castle, Snap::Ground, cast);
                ms.push((crate::now() - t).as_secs_f64() * 1e3);
            }
            ms.sort_by(f64::total_cmp);
            println!(
                "snap reading: yaw {:5.1}: {} downward casts ({} miss), {:.3} ms median, {:.3} ms max; \
                 base {:.1}, the free stand's z {:.1}, the ground under the eye {}",
                i as f32 * 22.5,
                downs.get(),
                misses.get(),
                ms[10],
                ms[19],
                corner[2],
                free[2],
                source.height(eye[0] as i32, eye[1] as i32)
            );
        }
    }
}
