//! A structure's words: its rows in the Structures chooser (`hud::menu::PageId::Structures`)
//! and its name in the belt's name row, at the modes' scale.

use std::fmt::Write as _;

use crate::entities::library::cut;
use crate::hud::Modes;

/// The words of an entry before its scene arrives.
pub const BUILDING: &str = "building";

/// Why structure `j` cannot be placed, at the modes `m`.
pub fn item_error(m: &Modes, j: usize) -> Option<&str> {
    m.structures.entries.get(j)?.error()
}

/// Why structure `j` cannot be placed at the modes' scale in the run's edit limit
/// ([`crate::structures::Entry::refusal`]): the belt's name row shows it while the slot is
/// selected, and the click's notice says it.
pub fn refusal(m: &Modes, j: usize) -> Option<String> {
    m.structures
        .entries
        .get(j)?
        .refusal(m.structure_scale, m.edit_axis_max)
}

/// Whether structure `j` is drawn grey: in error, or the castle before its build.
pub fn grey(m: &Modes, j: usize) -> bool {
    m.structures
        .entries
        .get(j)
        .is_some_and(|e| e.error().is_some() || e.building())
}

/// Where the reason of structure `j` in error starts in its [`write_item`] words, in bytes.
pub fn reason_at(m: &Modes, j: usize) -> Option<usize> {
    let e = m.structures.entries.get(j)?;
    e.error().map(|_| e.name.len().max(10) + 1)
}

/// `<x> x <y> x <z> m` for a box `size` voxels across (a voxel is 10 cm).
pub fn metres(size: [u32; 3], out: &mut String) {
    let m = size.map(|v| v as f32 / 10.0);
    let _ = write!(out, "{:.1} x {:.1} x {:.1} m", m[0], m[1], m[2]);
}

/// Structure `j`'s row after its selection mark, at most `width` bytes: `<name> <n>
/// placements  <x> x <y> x <z> m` at scale 1, `castle: building` before the castle's build,
/// or its name and its error.
pub fn write_item(m: &Modes, j: usize, text: &mut String, width: usize) {
    let Some(e) = m.structures.entries.get(j) else {
        return;
    };
    let mut row = String::new();
    match (e.error(), e.scene()) {
        (Some(err), _) => {
            let _ = write!(row, "{:<10} {err}", e.name);
        }
        (None, Some(s)) => {
            let _ = write!(row, "{:<10} {:>2} placements  ", e.name, s.placements.len());
            metres(s.size(), &mut row);
        }
        (None, None) => {
            let _ = write!(row, "{}: {BUILDING}", e.name);
        }
    }
    text.push_str(&cut(row, width));
}

/// Structure `j`'s second row: the palette rows its file added and the belt's slot that holds
/// it (`rows +255   slot 3`, or `no slot`).
pub fn write_item_rows(m: &Modes, j: usize, text: &mut String) {
    let rows = m.structures.entries.get(j).map_or(0, |e| e.rows);
    let _ = write!(text, "rows +{rows}");
    let item = crate::entities::belt::Item::Structure(j);
    match m.walkers.belt.slot_of(item) {
        Some(s) => {
            let _ = write!(text, "   slot {}", s + 1);
        }
        None => text.push_str("   no slot"),
    }
}

/// The belt's name row for structure `j`: `chapel  4.8 x 3.2 x 4.0 m` at the modes' scale and
/// the belt's turn (`  x2` after it at scale 2), `castle: building`, its name and error, or
/// its [`refusal`] at the scale (grey, the reason red). Gives whether it is grey and where its
/// red reason starts.
pub fn write_name(m: &Modes, j: usize, out: &mut String) -> (bool, Option<usize>) {
    let Some(e) = m.structures.entries.get(j) else {
        return (false, None);
    };
    if let Some(err) = e.error() {
        let _ = write!(out, "{}: {err}", e.name);
        return (true, Some(e.name.len() + 2));
    }
    let scale = m.structure_scale;
    if let Some(words) = refusal(m, j) {
        out.push_str(&words);
        return (true, words.find(": ").map(|at| at + 2));
    }
    match e.size(scale, m.walkers.belt.quarters()) {
        Some(size) => {
            let _ = write!(out, "{}  ", e.name);
            metres(size, out);
            if scale > 1 {
                let _ = write!(out, "  x{scale}");
            }
            (false, None)
        }
        None => {
            let _ = write!(out, "{}: {BUILDING}", e.name);
            (true, None)
        }
    }
}
