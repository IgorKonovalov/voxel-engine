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
//! is closed, as the bake's are), at least [`HELD_FLOOR`].

use super::faces::{class_masks, mask_bit};
use super::*;

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

/// A brick's opaque mask (bit `x + 8 y` of word `z`) by global brick.
type BrickMask<'a> = Box<dyn FnMut([i32; 3]) -> [u64; 8] + 'a>;

/// The opaque voxels about a dig's touched bricks, a mask a brick over a box of bricks, each
/// read at its first use; a voxel outside the box reads empty.
pub(crate) struct Opaque<'a> {
    lo: [i32; 3],
    size: [i32; 3],
    bricks: Vec<Option<[u64; 8]>>,
    mask: BrickMask<'a>,
}

impl<'a> Opaque<'a> {
    /// The grid over global bricks `lo..=hi`, each brick's opaque mask `mask(brick)`.
    pub(crate) fn from_bricks(
        lo: [i32; 3],
        hi: [i32; 3],
        mask: impl FnMut([i32; 3]) -> [u64; 8] + 'a,
    ) -> Self {
        let size = [0, 1, 2].map(|k| (hi[k] - lo[k] + 1).max(0));
        Self {
            lo,
            size,
            bricks: vec![None; (size[0] * size[1] * size[2]) as usize],
            mask: Box::new(mask),
        }
    }

    /// The grid about chunk `c`'s bricks `slots` in `world` under `classes`
    /// ([`face_classes`]), [`GRID_MARGIN`] bricks wide; a chunk not resident reads empty.
    pub(crate) fn about(world: &'a World, classes: &'a [u8], c: [i32; 3], slots: &[u16]) -> Self {
        let mut lo = [i32::MAX; 3];
        let mut hi = [i32::MIN; 3];
        for &s in slots {
            let b = brick_of_slot(usize::from(s));
            for k in 0..3 {
                lo[k] = lo[k].min(c[k] * 8 + b[k] - GRID_MARGIN);
                hi[k] = hi[k].max(c[k] * 8 + b[k] + GRID_MARGIN);
            }
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

    /// Whether voxel `v` (global) is opaque.
    pub(crate) fn at(&mut self, v: [i32; 3]) -> bool {
        let b = [0, 1, 2].map(|k| v[k].div_euclid(8) - self.lo[k]);
        if (0..3).any(|k| b[k] < 0 || b[k] >= self.size[k]) {
            return false;
        }
        let i = (b[0] + self.size[0] * (b[1] + self.size[1] * b[2])) as usize;
        let l = v.map(|x| x.rem_euclid(8) as usize);
        let mask = match self.bricks[i] {
            Some(m) => m,
            None => {
                let m = (self.mask)([0, 1, 2].map(|k| self.lo[k] + b[k]));
                self.bricks[i] = Some(m);
                m
            }
        };
        mask_bit(&mask, l)
    }
}

/// The unit vector of face direction `f` (`2 axis + 1` toward `+axis`).
fn face_normal(f: usize) -> [f32; 3] {
    let mut n = [0.0; 3];
    n[f / 2] = if f % 2 == 1 { 1.0 } else { -1.0 };
    n
}

/// The near openness along face direction `f` from the empty voxel `o` (global): the share of
/// the fixed rays about `f` that leave [`HELD_REACH`] voxels with no opaque voxel; a direction
/// that looks down, 0.
pub(crate) fn openness(solid: &mut Opaque<'_>, o: [i32; 3], f: usize) -> f32 {
    if f == 4 {
        return 0.0;
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
    for r in 0..HELD_RINGS {
        let s2 = (r as f32 + 0.5) / HELD_RINGS as f32;
        let (st, ct) = (s2.sqrt(), (1.0 - s2).sqrt());
        for k in 0..HELD_AZ {
            let a = (k as f32 + 0.5 * (r & 1) as f32) * std::f32::consts::TAU / HELD_AZ as f32;
            let d = [0, 1, 2].map(|i| n[i] * ct + (t[i] * a.cos() + b[i] * a.sin()) * st);
            let mut hit = d[2] <= 0.0;
            let mut i = 1;
            while !hit && i <= HELD_REACH {
                hit = solid.at([0, 1, 2].map(|q| (c[q] + d[q] * i as f32).floor() as i32));
                i += 1;
            }
            open += u32::from(!hit);
        }
    }
    open as f32 / (HELD_RINGS * HELD_AZ) as f32
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

/// The held visibilities of chunk `c`'s bricks `touched` that `faces` lists, a `[f32; 6]` a
/// cell in cell order, from the store's blocks as the edit found them (`words`), the edit
/// boxes and the opacity about them: a direction's value is the mean over the cell's faces of
/// that direction, a face outside every box dilated by one voxel at the cell's old record's,
/// a face inside one at the held estimate; `None` for a cell with no removed surface's record
/// above it and no chunk summary under a sky.
pub(crate) fn held_visibilities(
    solid: &mut Opaque<'_>,
    words: &BTreeMap<[i32; 3], Vec<u32>>,
    boxes: &[[[i32; 3]; 2]],
    c: [i32; 3],
    faces: &ChunkFaces,
    touched: &[u16],
    cell: u32,
) -> Vec<(u16, Vec<Option<[f32; 6]>>)> {
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
                })
                .or(summary_up);
            let Some(up) = up else {
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
                if !new.is_empty() {
                    let picks = new.len().min(HELD_FACES);
                    let open: f32 = (0..picks)
                        .map(|i| {
                            let lv = new[i * new.len() / picks];
                            let o = [0, 1, 2].map(|k| gb[k] * 8 + lv[k] + n[k]);
                            openness(solid, o, f)
                        })
                        .sum::<f32>()
                        / picks as f32;
                    let base = own_f.map_or(up, |o| o.min(up));
                    sum += new.len() as f32 * (base * open.sqrt()).max(HELD_FLOOR);
                }
                vis[f] = sum / list.len() as f32;
            }
            // A direction with no face of the cell takes the mean of those with one (the light
            // pass's smoothed normal reads beside directions); a cell with none, the base.
            let held: Vec<f32> = vis.iter().copied().filter(|v| !v.is_nan()).collect();
            let fill = if held.is_empty() {
                up.max(HELD_FLOOR)
            } else {
                held.iter().sum::<f32>() / held.len() as f32
            };
            cells.push(Some(vis.map(|v| if v.is_nan() { fill } else { v })));
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
        let roofed =
            held_visibilities(&mut dug(true), &words, &BOX, [0, 0, 0], &faces, &touched, 8);
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
