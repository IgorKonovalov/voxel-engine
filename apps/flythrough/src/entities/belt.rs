//! The belt and placement mode: nine slots, each holding a library entry or nothing, one
//! selected; while the mode is on the selected slot's entry stands at the aim as the ghost (a
//! real instance in the set, `super::Walkers::preview`) and a click places it. The belt is
//! drawn along the bottom of the window only in the mode ([`append_belt`]). Where the ghost
//! stands is [`stand`]: the camera ray's first world hit within [`PLACE_REACH`], never nearer
//! than the model's own size allows, dropped to the floor under a wall or a ceiling hit. A
//! slot holds an entity or a structure ([`Item`]); a structure's box stands by [`stand_box`].

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

    /// Puts `i` in slot `s`, taking it out of any other slot that held it.
    pub fn assign(&mut self, s: usize, i: Item) {
        for slot in &mut self.slots {
            if *slot == Some(i) {
                *slot = None;
            }
        }
        self.slots[s % SLOTS] = Some(i);
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

/// Where a model `extent` voxels across (x, y, z, standing in its rest pose) stands for an
/// eye at `eye` looking along the unit `fwd`, with `cast(origin, dir, max)` the world's first
/// hit along a unit ray within `max` voxels (its distance and the entered face's outward
/// normal; distance 0 when the origin is inside a solid voxel):
///
/// - `near = max(NEAR_MIN, NEAR_SCALE x the longest extent)`;
/// - a hit at `near` or beyond on an upward face: the feet at the hit;
/// - a hit at `near` or beyond on a wall or a ceiling: the hit moved back along the view by
///   half the model's footprint (its longer horizontal extent), dropped to the floor;
/// - a hit nearer than `near`, or none within [`PLACE_REACH`]: the view's point at `near`,
///   dropped to the floor.
///
/// Dropped means the first floor within [`FLOOR_DROP`] below the point; when the point is
/// inside a solid voxel the drop starts at the eye's height above it instead. No floor found,
/// the point stands where it is.
pub fn stand(
    eye: [f32; 3],
    fwd: [f32; 3],
    extent: [f32; 3],
    cast: impl Fn([f32; 3], [f32; 3], f32) -> Option<(f32, [f32; 3])>,
) -> [f32; 3] {
    let longest = extent[0].max(extent[1]).max(extent[2]);
    let near = NEAR_MIN.max(NEAR_SCALE * longest);
    let along = |t: f32| std::array::from_fn(|k| eye[k] + fwd[k] * t);
    let point = match cast(eye, fwd, PLACE_REACH) {
        Some((t, n)) if t >= near => {
            if n[2] > 0.5 {
                return along(t);
            }
            along(t - 0.5 * extent[0].max(extent[1]))
        }
        _ => along(near),
    };
    drop_to_floor(eye, point, &cast)
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

/// Where a box `size` voxels across (x, y, z) stands for an eye at `eye` looking along the
/// unit `fwd`, as its minimum corner (`cast` as [`stand`]'s): `p` is the camera ray's first
/// world hit within [`PLACE_REACH`] (on an upward face, as it is; on a wall or a ceiling,
/// half a voxel back along the view and dropped to the floor), else the view's point at
/// [`NEAR_MIN`] dropped to the floor; `h` is `fwd` level and of unit length (`+x` when `fwd`
/// is vertical); the box's support along `h` is `s = (|h.x| size.x + |h.y| size.y) / 2`, its
/// footprint centred on `p + h s` and its base at `p.z`. So its near side or corner is at `p`
/// and it grows away from the eye, which is never inside it.
pub fn stand_box(
    eye: [f32; 3],
    fwd: [f32; 3],
    size: [f32; 3],
    cast: impl Fn([f32; 3], [f32; 3], f32) -> Option<(f32, [f32; 3])>,
) -> [f32; 3] {
    let along = |t: f32| std::array::from_fn(|k| eye[k] + fwd[k] * t);
    let p = match cast(eye, fwd, PLACE_REACH) {
        Some((t, n)) if t > 0.0 && n[2] > 0.5 => along(t),
        Some((t, _)) if t > 0.0 => drop_to_floor(eye, along((t - 0.5).max(0.0)), &cast),
        _ => drop_to_floor(eye, along(NEAR_MIN), &cast),
    };
    let len = fwd[0].hypot(fwd[1]);
    let h = if len < 1e-6 {
        [1.0, 0.0]
    } else {
        [fwd[0] / len, fwd[1] / len]
    };
    let s = (h[0].abs() * size[0] + h[1].abs() * size[1]) / 2.0;
    let c = [p[0] + h[0] * s, p[1] + h[1] * s];
    [c[0] - size[0] / 2.0, c[1] - size[1] / 2.0, p[2]]
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
/// slot's sides), the frame's bottom. `paint` gains the frame's cells as yellow spans (the
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

    /// The stand rule against a floor at z = 0 and a wall at x = 100, the eye at z = 17.
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
        let feet = stand(eye, fwd, walker, cast);
        assert!((feet[0] - 30.0 * a.cos()).abs() < 1e-3, "{feet:?}");
        assert!(feet[2].abs() < 1e-3, "{feet:?}");
        let level = [1.0, 0.0, 0.0];
        let feet = stand(eye, level, walker, cast);
        assert!(
            (feet[0] - 97.0).abs() < 1e-3 && feet[2].abs() < 1e-3,
            "{feet:?}"
        );
        let up = [0.0, 0.0, 1.0];
        assert_eq!(stand(eye, up, walker, cast), [0.0, 0.0, 0.0]);
    }
}
