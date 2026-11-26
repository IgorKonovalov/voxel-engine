//! A dig's held record (ADR-0082; ADR-0076 rule 3: the history is the store's provisional
//! record): at the edit, each cell of a brick the dig touched takes six sky visibilities made on
//! the CPU against the world, which the light pass reads as it reads any record (no ray cast, no
//! pair kept) until the dig's job lands and the store fades to it. Pure CPU.
//!
//! The value of direction `f` is the mean over the cell's faces of that direction: a face the
//! dig did not make (outside every edit box dilated by one voxel) keeps the cell's old record's
//! value, and a dig's face takes the removed surface's up visibility (the nearest record at or
//! above the cell within [`HELD_CELLS`], stale or not, else the chunk summary's), capped by the
//! cell's own old record's for `f` where it held one, times the square root of a near openness
//! along `f` (the share of [`HELD_RINGS`] x [`HELD_AZ`] fixed cosine-weighted rays that leave
//! [`HELD_REACH`] voxels without meeting an opaque voxel, from the empty voxel before each of up
//! to [`HELD_FACES`] of the cell's dig faces of direction `f`, their mean; a ray that does not rise
//! is closed, as the bake's are), at least [`HELD_FLOOR`] under [`DigEstimate::Surface`].
//! Under [`DigEstimate::Depth`] no value is raised to a floor, 0 included, and a ray that
//! clears the near reach counts open only where it goes on, with no opaque voxel, until it
//! stands at the height the base was read at (the top of the record's cell above, or the
//! chunk's top for the summary; a base from the cell's own record keeps the near rays alone);
//! the value is the base times [`depth_form`] of that share, and each of the cell's six values
//! under one byte of the record is written as 0 ([`written`]).

use std::collections::HashMap;

use super::faces::{class_masks, mask_bit};
use super::*;
use crate::DigEstimate;

/// Cells above a cell walked for the removed surface's record.
pub(crate) const HELD_CELLS: i32 = 4;
/// The held value's floor: never black.
pub(crate) const HELD_FLOOR: f32 = 0.02;
/// The openness rays' rings about a direction, at equal cosine-weighted shares.
const HELD_RINGS: u32 = 3;
/// The openness rays a ring.
const HELD_AZ: u32 = 4;
/// Voxels an openness ray marches.
const HELD_REACH: i32 = 16;
/// Faces of one direction a cell's openness is read from, at most, spread through its faces
/// (the bake's record is the mean over all of them).
const HELD_FACES: usize = 8;
/// Bricks of margin about the touched bricks the opacity grid holds: a ray's reach and one.
const GRID_MARGIN: i32 = (HELD_REACH + 8) / 8;

/// The held value's share of its base under [`DigEstimate::Depth`] from the share of the rays
/// that reach the base's height: the share itself (Plan 0105 Phase 4's rule on its CPU table).
fn depth_form(reach: f32) -> f32 {
    reach
}

/// The value the record takes for a held value `v`: under [`DigEstimate::Depth`] a value under
/// one byte of the record (1/255) is 0, so a hole the bake will land at 0 never shows a byte of
/// sky that rounding up gave it; from one byte up, and under [`DigEstimate::Surface`], `v`
/// (rounded to the byte as every record is).
fn written(v: f32, estimate: DigEstimate) -> f32 {
    if estimate == DigEstimate::Depth && v < 1.0 / 255.0 {
        0.0
    } else {
        v
    }
}

/// A brick's opaque mask (bit `x + 8 y` of word `z`) by global brick.
type BrickMask<'a> = Box<dyn FnMut([i32; 3]) -> [u64; 8] + 'a>;

/// The opaque voxels about a dig's touched bricks, a mask a brick over a box of bricks, each
/// read at its first use and kept by brick (the last one read cached); a voxel outside the box
/// reads empty.
pub(crate) struct Opaque<'a> {
    lo: [i32; 3],
    hi: [i32; 3],
    bricks: HashMap<[i32; 3], [u64; 8]>,
    last: Option<([i32; 3], [u64; 8])>,
    mask: BrickMask<'a>,
}

impl<'a> Opaque<'a> {
    /// The grid over global bricks `lo..=hi`, each brick's opaque mask `mask(brick)`.
    pub(crate) fn from_bricks(
        lo: [i32; 3],
        hi: [i32; 3],
        mask: impl FnMut([i32; 3]) -> [u64; 8] + 'a,
    ) -> Self {
        Self {
            lo,
            hi,
            bricks: HashMap::new(),
            last: None,
            mask: Box::new(mask),
        }
    }

    /// The grid about chunk `c`'s bricks `slots` in `world` under `classes`
    /// ([`face_classes`]), [`GRID_MARGIN`] bricks wide; a chunk not resident reads empty.
    /// Under [`DigEstimate::Depth`] it grows to hold a far ray's march ([`far_extent`]):
    /// up to the highest height a base can be read at (chunk `c`'s top, or [`HELD_CELLS`]
    /// cells of `cell` voxels over the highest touched brick), and sideways by the outer
    /// ring's run over that rise.
    pub(crate) fn about(
        world: &'a World,
        classes: &'a [u8],
        c: [i32; 3],
        slots: &[u16],
        cell: u32,
        estimate: DigEstimate,
    ) -> Self {
        let mut lo = [i32::MAX; 3];
        let mut hi = [i32::MIN; 3];
        for &s in slots {
            let b = brick_of_slot(usize::from(s));
            for k in 0..3 {
                lo[k] = lo[k].min(c[k] * 8 + b[k] - GRID_MARGIN);
                hi[k] = hi[k].max(c[k] * 8 + b[k] + GRID_MARGIN);
            }
        }
        if estimate == DigEstimate::Depth && !slots.is_empty() {
            (lo, hi) = far_extent(lo, hi, c, cell);
        }
        let store = world.store();
        let mut chunks: BTreeMap<[i32; 3], Option<Arc<std::sync::RwLock<Chunk>>>> = BTreeMap::new();
        Self::from_bricks(lo, hi, move |g| {
            let cc = g.map(|v| v.div_euclid(8));
            let Some(h) = chunks
                .entry(cc)
                .or_insert_with(|| store.get(ChunkCoord(cc)))
            else {
                return [0; 8];
            };
            let chunk = h.read().unwrap_or_else(std::sync::PoisonError::into_inner);
            let brick = &chunk.bricks()[brick_slot(g.map(|v| v.rem_euclid(8)))];
            class_masks(brick, classes).0
        })
    }

    /// Whether voxel `v` (global) lies in the grid's box.
    pub(crate) fn holds(&self, v: [i32; 3]) -> bool {
        (0..3).all(|k| (self.lo[k]..=self.hi[k]).contains(&v[k].div_euclid(8)))
    }

    /// Whether voxel `v` (global) is opaque.
    pub(crate) fn at(&mut self, v: [i32; 3]) -> bool {
        if !self.holds(v) {
            return false;
        }
        let g = v.map(|x| x.div_euclid(8));
        let l = v.map(|x| x.rem_euclid(8) as usize);
        let mask = match self.last {
            Some((b, m)) if b == g => m,
            _ => {
                let m = match self.bricks.get(&g) {
                    Some(m) => *m,
                    None => {
                        let m = (self.mask)(g);
                        self.bricks.insert(g, m);
                        m
                    }
                };
                self.last = Some((g, m));
                m
            }
        };
        mask_bit(&mask, l)
    }
}

/// The outer ring's run sideways a voxel of rise about a vertical normal (its share of the
/// cosine-weighted hemisphere is `(HELD_RINGS - 0.5) / HELD_RINGS` of `sin^2`, so the tangent
/// is the root of `2 HELD_RINGS - 1`).
fn outer_run() -> f32 {
    ((2 * HELD_RINGS - 1) as f32).sqrt()
}

/// The box of bricks `lo..=hi` about chunk `c`'s touched bricks grown to hold a far ray's
/// march under [`DigEstimate::Depth`]: up to the highest base height (the chunk's top, or
/// [`HELD_CELLS`] cells of `cell` voxels over the box's top) and one brick, and sideways by
/// [`outer_run`] times the rise from the box's bottom to that height.
fn far_extent(lo: [i32; 3], hi: [i32; 3], c: [i32; 3], cell: u32) -> ([i32; 3], [i32; 3]) {
    let top = ((c[2] + 1) * 64).max((hi[2] + 1) * 8 + HELD_CELLS * cell as i32);
    let rise = top - lo[2] * 8;
    let side = (rise as f32 * outer_run() / 8.0).ceil() as i32;
    (
        [lo[0] - side, lo[1] - side, lo[2]],
        [hi[0] + side, hi[1] + side, hi[2].max(top.div_euclid(8) + 1)],
    )
}

/// The unit vector of face direction `f` (`2 axis + 1` toward `+axis`).
fn face_normal(f: usize) -> [f32; 3] {
    let mut n = [0.0; 3];
    n[f / 2] = if f % 2 == 1 { 1.0 } else { -1.0 };
    n
}

/// The near openness along face direction `f` from the empty voxel `o` (global): the share of
/// the fixed rays about `f` that leave [`HELD_REACH`] voxels with no opaque voxel; a direction
/// that looks down, 0. Beside it the far share: with `top`, of those rays the ones that go on
/// with no opaque voxel until their voxel's z reaches `top` (a ray that leaves the grid's box
/// first is closed); without, the near share.
pub(crate) fn openness(
    solid: &mut Opaque<'_>,
    o: [i32; 3],
    f: usize,
    top: Option<i32>,
) -> [f32; 2] {
    if f == 4 {
        return [0.0; 2];
    }
    let n = face_normal(f);
    let a0 = if n[2].abs() > 0.5 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 0.0, 1.0]
    };
    let d0 = n[0] * a0[0] + n[1] * a0[1] + n[2] * a0[2];
    let t = normalize([a0[0] - n[0] * d0, a0[1] - n[1] * d0, a0[2] - n[2] * d0]);
    let b = [
        n[1] * t[2] - n[2] * t[1],
        n[2] * t[0] - n[0] * t[2],
        n[0] * t[1] - n[1] * t[0],
    ];
    let c = o.map(|v| v as f32 + 0.5);
    let mut open = 0u32;
    let mut far = 0u32;
    for r in 0..HELD_RINGS {
        let s2 = (r as f32 + 0.5) / HELD_RINGS as f32;
        let (st, ct) = (s2.sqrt(), (1.0 - s2).sqrt());
        for k in 0..HELD_AZ {
            let a = (k as f32 + 0.5 * (r & 1) as f32) * std::f32::consts::TAU / HELD_AZ as f32;
            let d = [0, 1, 2].map(|i| n[i] * ct + (t[i] * a.cos() + b[i] * a.sin()) * st);
            let at = |i: i32| [0, 1, 2].map(|q| (c[q] + d[q] * i as f32).floor() as i32);
            let mut hit = d[2] <= 0.0;
            let mut i = 1;
            while !hit && i <= HELD_REACH {
                hit = solid.at(at(i));
                i += 1;
            }
            open += u32::from(!hit);
            if hit {
                continue;
            }
            let Some(top) = top else {
                far += 1;
                continue;
            };
            // On from the near reach until the ray's voxel stands at `top`: an opaque voxel or
            // the grid's edge on the way closes it.
            let mut v = at(i - 1);
            while !hit && v[2] < top {
                v = at(i);
                hit = !solid.holds(v) || solid.at(v);
                i += 1;
            }
            far += u32::from(!hit);
        }
    }
    let rays = (HELD_RINGS * HELD_AZ) as f32;
    [open as f32 / rays, far as f32 / rays]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-12);
    v.map(|x| x / l)
}

/// Global cell `g`'s record (cells of `cell` voxels) in `words` (the store's blocks by chunk),
/// stale or not.
fn cell_record(
    words: &BTreeMap<[i32; 3], Vec<u32>>,
    g: [i32; 3],
    cell: u32,
    record: usize,
) -> Option<&[u32]> {
    let per = 8 / cell as i32;
    let brick = g.map(|v| v.div_euclid(per));
    let block = words.get(&brick.map(|v| v.div_euclid(8)))?;
    let dir = block[brick_slot(brick.map(|v| v.rem_euclid(8)))];
    if dir == 0 {
        return None;
    }
    let run = (dir >> 11) as usize;
    let l = g.map(|v| v.rem_euclid(per));
    let sub = (l[0] + per * (l[1] + per * l[2])) as usize;
    let at = run - record + BRICK_WORDS * sub;
    Some(&block[at..at + BRICK_WORDS])
}

/// A record's visibility for `face` with [`VIS_BAKED`] and the direction, stale or not.
fn any_visibility(r: &[u32], face: usize) -> Option<f32> {
    record_visibility(r, face)
}

/// Where a held value's base came from (a reading's trace, [`HeldPart`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeldBase {
    /// The record of the cell this many cells above (0: the cell's own).
    Above(i32),
    /// The chunk summary's up visibility.
    Summary,
}

/// Where a held cell's direction value (its stored radiance) comes from in the dig's
/// provisional block (`provisional_block`, one cell a brick), for a reading ([`HeldPart`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RadianceFrom {
    /// The kept brick's old record held the direction.
    OldRecord,
    /// The kept brick's old record held other directions: their mean (`fill_directions`).
    Filled,
    /// The brick held no record: the mean of the records around it (or none).
    Neighbours,
}

/// One cell's held value of one direction with its parts, for a reading
/// (`BakeQueue::trace_held`): the dug cell, the value the record holds (the mean over the
/// cell's faces of the direction), the estimate its dig faces took, the removed surface's up
/// visibility and where it came from, the cell's own old record's value (the cap), the
/// openness (the mean over the picked faces), the faces the dig made and the faces it kept,
/// the record's words a brick ([`record_words`]), and where its direction value comes from
/// with the old record's luminance for it (the held direction's, or the mean of the held
/// ones' for a filled one; `None` for a brick with no record).
#[derive(Clone, Copy, Debug)]
pub struct HeldPart {
    pub chunk: [i32; 3],
    pub slot: u16,
    pub sub: u16,
    pub face: u8,
    pub value: f32,
    pub estimate: f32,
    pub up: f32,
    pub from: HeldBase,
    pub own: Option<f32>,
    pub open: f32,
    /// The share of the rays that also reach the base's height (under
    /// [`DigEstimate::Depth`]; the near share where the base is the cell's own record, and
    /// under [`DigEstimate::Surface`]).
    pub reach: f32,
    pub new_faces: u16,
    pub old_faces: u16,
    pub record: u32,
    pub radiance_from: RadianceFrom,
    pub old_radiance: Option<f32>,
}

/// A trace's source of `own`'s (the cell's old record's) direction `f` value and its
/// luminance, as `provisional_block` takes it for a cell of a brick.
fn radiance_source(own: Option<&[u32]>, f: usize) -> (RadianceFrom, Option<f32>) {
    let lum = |w: u32| {
        let c = rgb9e5_unpack(w);
        0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
    };
    let Some(r) = own else {
        return (RadianceFrom::Neighbours, None);
    };
    let mask = r[6] >> 16;
    if mask & 1 << f != 0 {
        return (RadianceFrom::OldRecord, Some(lum(r[f])));
    }
    let held: Vec<f32> = (0..6)
        .filter(|d| mask & 1 << d != 0)
        .map(|d| lum(r[d]))
        .collect();
    let mean = (!held.is_empty()).then(|| held.iter().sum::<f32>() / held.len() as f32);
    (RadianceFrom::Filled, mean)
}

/// The held visibilities of chunk `c`'s bricks `touched` that `faces` lists, a `[f32; 6]` a
/// cell in cell order, from the store's blocks as the edit found them (`words`), the edit
/// boxes and the opacity about them: a direction's value is the mean over the cell's faces of
/// that direction, a face outside every box dilated by one voxel at the cell's old record's,
/// a face inside one at the held estimate; `None` for a cell with no removed surface's record
/// above it and no chunk summary under a sky. `estimate` sets the floor: [`HELD_FLOOR`] under
/// [`DigEstimate::Surface`] for a dig's face and for a cell with no face, none under
/// [`DigEstimate::Depth`], where each of the six values under one byte is 0 ([`written`]). Each
/// dig direction's parts are pushed to `trace` where given (a
/// reading).
#[allow(
    clippy::too_many_arguments,
    reason = "the edit's inputs and a reading's trace"
)]
pub(crate) fn held_visibilities(
    solid: &mut Opaque<'_>,
    words: &BTreeMap<[i32; 3], Vec<u32>>,
    boxes: &[[[i32; 3]; 2]],
    c: [i32; 3],
    faces: &ChunkFaces,
    touched: &[u16],
    cell: u32,
    estimate: DigEstimate,
    mut trace: Option<&mut Vec<HeldPart>>,
) -> Vec<(u16, Vec<Option<[f32; 6]>>)> {
    let floor = match estimate {
        DigEstimate::Surface => HELD_FLOOR,
        DigEstimate::Depth => 0.0,
    };
    let record = record_words(cell);
    let per = 8 / cell as i32;
    let summary_up = words.get(&c).and_then(|b| {
        let at = SUMMARY_AT as usize;
        any_visibility(&b[at..at + BRICK_WORDS], 5)
    });
    let mut out = Vec::new();
    for (slot, keys) in &faces.bricks {
        if !touched.contains(slot) {
            continue;
        }
        let b = brick_of_slot(usize::from(*slot));
        let gb = [0, 1, 2].map(|k| c[k] * 8 + b[k]);
        let mut cells = Vec::with_capacity((per * per * per) as usize);
        for sub in 0..per * per * per {
            let l = [sub % per, (sub / per) % per, sub / (per * per)];
            let g = [0, 1, 2].map(|k| gb[k] * per + l[k]);
            let up = (0..HELD_CELLS)
                .find_map(|k| {
                    cell_record(words, [g[0], g[1], g[2] + k], cell, record)
                        .and_then(|r| any_visibility(r, 5))
                        .map(|v| (v, HeldBase::Above(k)))
                })
                .or(summary_up.map(|v| (v, HeldBase::Summary)));
            let Some((up, from)) = up else {
                cells.push(None);
                continue;
            };
            let own = cell_record(words, g, cell, record);
            // The cell's faces by direction, voxel coordinates in the brick.
            let mut by_dir: [Vec<[i32; 3]>; 6] = Default::default();
            for &key in keys {
                let v = i32::from(key >> 3);
                let lv = [v % 8, (v / 8) % 8, v / 64];
                if (0..3).all(|k| lv[k] / cell as i32 == l[k]) {
                    by_dir[usize::from(key & 7)].push(lv);
                }
            }
            let mut vis = [f32::NAN; 6];
            for f in 0..6 {
                let list = &by_dir[f];
                if list.is_empty() {
                    continue;
                }
                let n = face_normal(f).map(|x| x as i32);
                let own_f = own.and_then(|r| any_visibility(r, f));
                // A face outside every box dilated by one voxel held its record's value and
                // only gained sky by the dig; the rest are the dig's faces.
                let (old, new): (Vec<[i32; 3]>, Vec<[i32; 3]>) = list.iter().partition(|lv| {
                    own_f.is_some()
                        && !boxes.iter().any(|[lo, hi]| {
                            (0..3).all(|k| (lo[k] - 1..=hi[k] + 1).contains(&(gb[k] * 8 + lv[k])))
                        })
                });
                let mut sum = old.len() as f32 * own_f.unwrap_or(0.0);
                let mut parts = None;
                if !new.is_empty() {
                    let picks = new.len().min(HELD_FACES);
                    let capped = own_f.is_some_and(|o| o < up);
                    let base = own_f.map_or(up, |o| o.min(up));
                    // Under `Depth` a far ray rises to the height the base was read at: the top
                    // of the record's cell above, the chunk's top for the summary; a base from
                    // the cell's own record keeps the near rays alone.
                    let top = match (estimate, from) {
                        (DigEstimate::Surface, _) | (_, HeldBase::Above(0)) => None,
                        _ if capped => None,
                        (_, HeldBase::Above(k)) => Some((g[2] + k + 1) * cell as i32),
                        (_, HeldBase::Summary) => Some((c[2] + 1) * 64),
                    };
                    let [open, reach] = (0..picks)
                        .map(|i| {
                            let lv = new[i * new.len() / picks];
                            let o = [0, 1, 2].map(|k| gb[k] * 8 + lv[k] + n[k]);
                            openness(solid, o, f, top)
                        })
                        .fold([0.0; 2], |s, x| [s[0] + x[0], s[1] + x[1]])
                        .map(|s| s / picks as f32);
                    let value = match estimate {
                        DigEstimate::Surface => (base * open.sqrt()).max(floor),
                        DigEstimate::Depth => base * depth_form(reach),
                    };
                    sum += new.len() as f32 * value;
                    parts = Some((value, open, reach, new.len(), old.len()));
                }
                vis[f] = sum / list.len() as f32;
                if let (Some(t), Some((part, open, reach, n_new, n_old))) =
                    (trace.as_deref_mut(), parts)
                {
                    let (radiance_from, old_radiance) = radiance_source(own, f);
                    t.push(HeldPart {
                        chunk: c,
                        slot: *slot,
                        sub: sub as u16,
                        face: f as u8,
                        value: written(vis[f], estimate),
                        estimate: part,
                        up,
                        from,
                        own: own_f,
                        open,
                        reach,
                        new_faces: n_new as u16,
                        old_faces: n_old as u16,
                        record: record as u32,
                        radiance_from,
                        old_radiance,
                    });
                }
            }
            // A direction with no face of the cell takes the mean of those with one (the light
            // pass's smoothed normal reads beside directions); a cell with none, the base.
            let held: Vec<f32> = vis.iter().copied().filter(|v| !v.is_nan()).collect();
            let fill = if held.is_empty() {
                up.max(floor)
            } else {
                held.iter().sum::<f32>() / held.len() as f32
            };
            cells.push(Some(
                vis.map(|v| written(if v.is_nan() { fill } else { v }, estimate)),
            ));
        }
        out.push((*slot, cells));
    }
    out
}

/// Writes `held` ([`held_visibilities`]) into `block`'s records: each held cell's two sky
/// visibility words with [`VIS_BAKED`] (no [`SKY_STALE`]: the light pass reads them as a
/// record), its key words cleared (the old surface's key light is not the hole's), every
/// direction in its mask.
pub(crate) fn apply_held(block: &mut [u32], held: &[(u16, Vec<Option<[f32; 6]>>)], record: usize) {
    for (slot, cells) in held {
        let dir = block[usize::from(*slot)];
        if dir == 0 {
            continue;
        }
        let run = (dir >> 11) as usize;
        for (i, v) in cells.iter().enumerate() {
            let Some(v) = v else {
                continue;
            };
            let at = run - record + BRICK_WORDS * i;
            let r = &mut block[at..at + BRICK_WORDS];
            r[VIS_WORD..VIS_WORD + 2].copy_from_slice(&visibility_words(*v));
            r[KEY_WORD] = 0;
            r[KEY_WORD + 1] = 0;
            r[6] = (r[6] & 0xFFFF) | 0x3F << 16;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The dig's box.
    const BOX: [[[i32; 3]; 2]; 1] = [[[8, 8, 24], [23, 23, 31]]];

    /// Ground opaque under z = 32 in chunk [0, 0, 0], dug over x and y in 8..24, z in 24..32
    /// (bricks [1..=2, 1..=2, 3]); with `roof`, an opaque layer at z = 33 over everything.
    fn dug(roof: bool) -> Opaque<'static> {
        Opaque::from_bricks([-3, -3, -3], [10, 10, 10], move |g| {
            let mut m = [0u64; 8];
            for z in 0..8 {
                for y in 0..8 {
                    for x in 0..8 {
                        let v = [g[0] * 8 + x, g[1] * 8 + y, g[2] * 8 + z];
                        let hole = (8..24).contains(&v[0])
                            && (8..24).contains(&v[1])
                            && (24..32).contains(&v[2]);
                        if (v[2] < 32 && !hole) || (roof && v[2] == 33) {
                            m[z as usize] |= 1 << (x + 8 * y);
                        }
                    }
                }
            }
            m
        })
    }

    /// Chunk [0, 0, 0]'s block before the dig: the surface bricks `slots` each one record of
    /// up visibility 0.9, sides 0.8 and a baked key, and a summary of up 0.9.
    fn surface(slots: &[[i32; 3]]) -> BTreeMap<[i32; 3], Vec<u32>> {
        let faces = ChunkFaces {
            bricks: slots
                .iter()
                .map(|b| (brick_slot(*b) as u16, vec![]))
                .collect(),
            glass: vec![],
        };
        let (mut block, words) = super::super::summary::layout(&faces, BRICK_WORDS);
        block.resize(words as usize, 0);
        let rec = |r: &mut [u32]| {
            r[6] = 1 | 0x3F << 16;
            r[VIS_WORD..VIS_WORD + 2]
                .copy_from_slice(&visibility_words([0.8, 0.8, 0.8, 0.8, 0.0, 0.9]));
            r[KEY_WORD] = u32::MAX;
            r[KEY_WORD + 1] = KEY_BAKED | 0xFFFF;
        };
        for b in slots {
            let run = (block[brick_slot(*b)] >> 11) as usize;
            rec(&mut block[run - BRICK_WORDS..run]);
        }
        let at = SUMMARY_AT as usize;
        rec(&mut block[at..at + BRICK_WORDS]);
        BTreeMap::from([([0, 0, 0], block)])
    }

    /// The floor's up face at voxel [16, 16, 23] (brick [2, 2, 2], voxel [0, 0, 7]), a wall's
    /// +x face at voxel [7, 16, 28] (brick [0, 2, 3], voxel [7, 0, 4]), and the ground's up face
    /// at voxel [3, 16, 31] beside the hole in the wall's brick, which the dig did not make.
    fn faces() -> (ChunkFaces, [u16; 2]) {
        let (floor, wall) = (brick_slot([2, 2, 2]) as u16, brick_slot([0, 2, 3]) as u16);
        let mut bricks = vec![
            (floor, vec![(64 * 7) << 3 | 5]),
            (wall, vec![(7 + 64 * 4) << 3 | 1, (3 + 64 * 7) << 3 | 5]),
        ];
        bricks.sort_unstable();
        (
            ChunkFaces {
                bricks,
                glass: vec![],
            },
            [floor, wall],
        )
    }

    fn held_of(held: &[(u16, Vec<Option<[f32; 6]>>)], slot: u16) -> [f32; 6] {
        held.iter().find(|(s, _)| *s == slot).expect("the brick").1[0].expect("a value")
    }

    #[test]
    fn a_dug_brick_holds_its_removed_surface_times_its_openness() {
        let words = surface(&[[2, 2, 3], [0, 2, 3]]);
        let (faces, [floor, wall]) = faces();
        let touched = [floor, wall];
        let held = held_visibilities(
            &mut dug(false),
            &words,
            &BOX,
            [0, 0, 0],
            &faces,
            &touched,
            8,
            DigEstimate::Surface,
            None,
        );
        let up = held_of(&held, floor)[5];
        let side = held_of(&held, wall)[1];
        // The floor's up: above the floor and at most the removed surface's 0.9.
        assert!(up > HELD_FLOOR && up <= 0.9, "floor up {up}");
        // A wall's sideways direction: less than the floor's up.
        assert!(side < up, "wall +x {side} against the floor's up {up}");
        // The ground beside the hole in a dug brick, a face the dig did not make: its record's.
        let ground = held_of(&held, wall)[5];
        assert!(
            (ground - (0.9f32 * 255.0).round() / 255.0).abs() < 1e-6,
            "the ground's up {ground}"
        );
        // Under a roof one voxel above the ground, the floor holds the floor value.
        let roofed = held_visibilities(
            &mut dug(true),
            &words,
            &BOX,
            [0, 0, 0],
            &faces,
            &touched,
            8,
            DigEstimate::Surface,
            None,
        );
        assert_eq!(held_of(&roofed, floor)[5], HELD_FLOOR);
        // A kept brick the box touches holds the estimate, not its old record, and its key
        // words go; the held record is not stale.
        let mut block = words[&[0, 0, 0]].clone();
        apply_held(&mut block, &held, BRICK_WORDS);
        let run = (block[usize::from(wall)] >> 11) as usize;
        let r = &block[run - BRICK_WORDS..run];
        let got = record_visibility(r, 1).expect("a visibility");
        assert!(
            (got - side).abs() <= 0.5 / 255.0 + 1e-6,
            "kept brick +x {got} against {side}"
        );
        assert!(
            (got - 0.8).abs() > 1.0 / 255.0,
            "the old record's 0.8 is replaced"
        );
        assert_eq!(r[KEY_WORD + 1] & KEY_BAKED, 0);
        assert!(!record_sky_stale(r));
    }

    /// Under `Depth` no floor: a floor face at the centre of a shaft 10 voxels wide (x and y in
    /// 11..21) and 40 deep (z in 24..64, the ground's top at 64), base 0.9 (its own record and
    /// the summary), sees every ray close inside the near reach (the inner ring runs 6.5
    /// voxels sideways in 16, the wall is at most 5.5 away): it holds `HELD_FLOOR` under
    /// `Surface` and exactly 0 under `Depth`, and so does every direction it fills.
    #[test]
    fn a_shaft_floor_holds_the_floor_under_surface_and_zero_under_depth() {
        const SHAFT: [[[i32; 3]; 2]; 1] = [[[11, 11, 24], [20, 20, 63]]];
        let shaft = || {
            Opaque::from_bricks([-3, -3, -3], [10, 10, 10], |g| {
                let mut m = [0u64; 8];
                for z in 0..8 {
                    for y in 0..8 {
                        for x in 0..8 {
                            let v = [g[0] * 8 + x, g[1] * 8 + y, g[2] * 8 + z];
                            let hole = (11..21).contains(&v[0])
                                && (11..21).contains(&v[1])
                                && (24..64).contains(&v[2]);
                            if v[2] < 64 && !hole {
                                m[z as usize] |= 1 << (x + 8 * y);
                            }
                        }
                    }
                }
                m
            })
        };
        let words = surface(&[[2, 2, 2]]);
        let (faces, [floor, _]) = faces();
        let held = |estimate| {
            held_visibilities(
                &mut shaft(),
                &words,
                &SHAFT,
                [0, 0, 0],
                &faces,
                &[floor],
                8,
                estimate,
                None,
            )
        };
        let surface = held_of(&held(DigEstimate::Surface), floor);
        let depth = held_of(&held(DigEstimate::Depth), floor);
        assert_eq!(surface, [HELD_FLOOR; 6], "under surface");
        assert_eq!(depth, [0.0; 6], "under depth");
    }

    /// Under `Depth` a ray is open only where it rises to its base's height: a floor face at
    /// voxel [16, 16, z] of a pit 20 voxels wide (x and y in 6..26) under an open sky, the
    /// ground's top at 64 (chunk [0, 0, 0]'s top), its base the summary's 0.9 (no record in
    /// the pit's column). 40 voxels down (z 23) it holds more than 0.4 under `Surface` (the
    /// near rays clear 16 voxels) and exactly 0 under `Depth` (a ray 24 degrees off the
    /// vertical runs 17.9 voxels sideways in 40, the wall is at most 10.5 away); 8 voxels down
    /// (z 55) it holds more than 0 under `Depth`.
    #[test]
    fn a_pit_floor_is_open_under_depth_only_where_its_rays_reach_the_summarys_height() {
        let pit = |deep: i32| {
            Opaque::from_bricks([-3, -3, -3], [10, 10, 10], move |g| {
                let mut m = [0u64; 8];
                for z in 0..8 {
                    for y in 0..8 {
                        for x in 0..8 {
                            let v = [g[0] * 8 + x, g[1] * 8 + y, g[2] * 8 + z];
                            let hole = (6..26).contains(&v[0])
                                && (6..26).contains(&v[1])
                                && (64 - deep..64).contains(&v[2]);
                            if v[2] < 64 && !hole {
                                m[z as usize] |= 1 << (x + 8 * y);
                            }
                        }
                    }
                }
                m
            })
        };
        let words = surface(&[[0, 0, 7]]);
        let held = |deep: i32, estimate| {
            let floor = brick_slot([2, 2, (63 - deep) / 8]) as u16;
            let faces = ChunkFaces {
                bricks: vec![(floor, vec![((64 * ((63 - deep) % 8)) << 3 | 5) as _])],
                glass: vec![],
            };
            let boxes = [[[6, 6, 64 - deep], [25, 25, 63]]];
            held_of(
                &held_visibilities(
                    &mut pit(deep),
                    &words,
                    &boxes,
                    [0, 0, 0],
                    &faces,
                    &[floor],
                    8,
                    estimate,
                    None,
                ),
                floor,
            )[5]
        };
        let surface40 = held(40, DigEstimate::Surface);
        assert!(surface40 > 0.4, "40 down under surface {surface40}");
        assert_eq!(held(40, DigEstimate::Depth), 0.0, "40 down under depth");
        let depth8 = held(8, DigEstimate::Depth);
        assert!(depth8 > 0.0, "8 down under depth {depth8}");
    }

    /// The byte a value takes in the record (`visibility_words`' rounding).
    fn byte(v: f32) -> u32 {
        visibility_words([v; 6])[0] & 0xFF
    }

    /// Under `Depth` a held value under one byte holds 0 and one of 1.10 bytes holds one byte;
    /// under `Surface` 0.54 and 0.84 of a byte round as every record does.
    #[test]
    fn a_held_value_under_one_byte_holds_zero_under_depth() {
        for b in [0.54f32, 0.69, 0.84] {
            let v = b / 255.0;
            assert_eq!(written(v, DigEstimate::Depth), 0.0, "{b} of a byte");
            assert_eq!(byte(written(v, DigEstimate::Depth)), 0, "{b} of a byte");
        }
        assert_eq!(byte(written(1.10 / 255.0, DigEstimate::Depth)), 1);
        assert_eq!(byte(written(0.54 / 255.0, DigEstimate::Surface)), 1);
        assert_eq!(byte(written(0.84 / 255.0, DigEstimate::Surface)), 1);
    }

    /// A cell's up direction of 24 faces the dig did not make (x in 21..24 at z 23, outside the
    /// box dilated by one), its old record at 2 bytes, beside 38 dig faces (x in 16..21) under
    /// a roof one voxel over them, so every ray closes: under `Depth` the dig faces hold 0 and
    /// the mean is 48 / 62 of a byte, which holds 0 in all six values; under `Surface` the dig
    /// faces hold `HELD_FLOOR` and the cell more than 0.
    #[test]
    fn kept_faces_under_one_byte_beside_closed_dig_faces_hold_zero_under_depth() {
        // Opaque but the layer z = 24 (the faces' empty voxels).
        let pocket = || {
            Opaque::from_bricks([-3, -3, -3], [10, 10, 10], |g| {
                let mut m = [0u64; 8];
                for (z, w) in m.iter_mut().enumerate() {
                    if g[2] * 8 + z as i32 != 24 {
                        *w = u64::MAX;
                    }
                }
                m
            })
        };
        let slot = brick_slot([2, 2, 2]);
        let mut words = surface(&[[2, 2, 2]]);
        let block = words.get_mut(&[0, 0, 0]).expect("the block");
        let run = (block[slot] >> 11) as usize;
        block[run - BRICK_WORDS + VIS_WORD..run - BRICK_WORDS + VIS_WORD + 2]
            .copy_from_slice(&visibility_words([2.0 / 255.0; 6]));
        // Up faces at z 23 (brick voxel z 7): 38 dig faces at x 16..21, then 24 kept at 21..24.
        let up = |x: i32, y: i32| ((((x - 16) + 8 * (y - 16) + 64 * 7) << 3) | 5) as u16;
        let mut keys: Vec<u16> = (16..21)
            .flat_map(|x| (16..24).map(move |y| up(x, y)))
            .take(38)
            .collect();
        keys.extend((21..24).flat_map(|x| (16..24).map(move |y| up(x, y))));
        assert_eq!(keys.len(), 62);
        let faces = ChunkFaces {
            bricks: vec![(slot as u16, keys)],
            glass: vec![],
        };
        let boxes = [[[16, 16, 24], [19, 23, 24]]];
        let held = |estimate| {
            held_of(
                &held_visibilities(
                    &mut pocket(),
                    &words,
                    &boxes,
                    [0, 0, 0],
                    &faces,
                    &[slot as u16],
                    8,
                    estimate,
                    None,
                ),
                slot as u16,
            )
        };
        assert_eq!(held(DigEstimate::Depth), [0.0; 6], "under depth");
        let surface = held(DigEstimate::Surface)[5];
        assert!(surface > 0.0, "under surface {surface}");
    }

    #[test]
    fn the_held_record_is_replaced_by_the_jobs_at_the_landing() {
        let words = surface(&[[2, 2, 3], [0, 2, 3]]);
        let (faces, [floor, wall]) = faces();
        let held = held_visibilities(
            &mut dug(false),
            &words,
            &BOX,
            [0, 0, 0],
            &faces,
            &[floor, wall],
            8,
            DigEstimate::Surface,
            None,
        );
        let mut from = words[&[0, 0, 0]].clone();
        apply_held(&mut from, &held, BRICK_WORDS);
        let mut job = words[&[0, 0, 0]].clone();
        let run = (job[usize::from(wall)] >> 11) as usize;
        job[run - BRICK_WORDS + VIS_WORD..run - BRICK_WORDS + VIS_WORD + 2]
            .copy_from_slice(&visibility_words([0.3; 6]));
        let mut shown = Vec::new();
        super::super::summary::fade_mix(&mut shown, &from, &job, 1.0, BRICK_WORDS, true);
        assert_eq!(shown, job);
    }
}
