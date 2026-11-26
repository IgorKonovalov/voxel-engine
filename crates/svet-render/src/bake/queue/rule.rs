//! The rule that answers a listed chunk without a GPU job (ADR-0080): a chunk whose reach box
//! lies under the column map's lowest top over its footprint, with no exposed emitter within
//! the light radius, takes a zero record written on the CPU. Under ADR-0070's column clause
//! every ray that ends under the column top is not sky, and no emitter reaches it, so the
//! GPU's bake of it draws nothing; the record is the one such a bake writes.

use super::list::neighbour;
use super::*;
use crate::gpu_world::COLUMN_NONE;

/// Whether `coord`'s reach box (the chunk dilated by `lr` chunks) lies under the column map's
/// lowest top over the box's footprint (the chunk columns within `lr` of `coord`'s, Chebyshev),
/// each column's lowest top read by `lowest(cx, cy)` ([`GpuWorld::column_lowest`]): the box's
/// highest voxel is under every column's top, and no column of the footprint holds a cell with
/// no opaque voxel ([`COLUMN_NONE`]).
pub(crate) fn buried(coord: ChunkCoord, lr: i32, mut lowest: impl FnMut(i32, i32) -> i32) -> bool {
    let box_top = (coord.0[2] + lr + 1) * 64 - 1;
    for y in coord.0[1] - lr..=coord.0[1] + lr {
        for x in coord.0[0] - lr..=coord.0[0] + lr {
            let l = lowest(x, y);
            if l == COLUMN_NONE || l <= box_top {
                return false;
            }
        }
    }
    true
}

/// The block a GPU bake of `coord`'s `faces` writes where no sample finds light, no sky ray reaches the
/// sky and no key ray the key light's disc: each listed face drawn and dark, through the same
/// record as a job's close ([`brick_record`]), with the sky's and the key light's flags as the
/// bake sets them (`sky`, `key`: the bake's key direction) and the key direction in
/// [`KEY_SUN_AT`].
pub(crate) fn zero_block(
    coord: ChunkCoord,
    faces: &ChunkFaces,
    cell: u32,
    clamp: bool,
    sky: bool,
    key: Option<[f32; 3]>,
    tier: u8,
) -> Vec<u32> {
    let (dir, words) = layout(faces, record_words(cell));
    let mut block = dir;
    block.reserve(words as usize);
    for (slot, keys) in &faces.bricks {
        let b = brick_of_slot(usize::from(*slot));
        let states: Vec<FaceState> = keys
            .iter()
            .map(|&k| {
                let i = i32::from(k >> 3);
                let l = [i % 8, i / 8 % 8, i / 64];
                FaceState {
                    voxel: [0, 1, 2].map(|a| coord.0[a] * 64 + b[a] * 8 + l[a]),
                    face: (k & 7) as u8,
                    n_min: 1,
                    n_max: 1,
                    n: 1,
                    queued: 0,
                    sum: [0.0; 3],
                    clamped: [0.0; 3],
                    lum2: 0.0,
                    best: 0.0,
                    material: 0,
                    snapshots: Vec::new(),
                    done: true,
                    sky_n: u32::from(sky),
                    sky_open: 0,
                    key_n: u32::from(key.is_some()),
                    key_open: 0,
                    key_only: false,
                }
            })
            .collect();
        block.extend(brick_record(&states, clamp, cell, sky, key.is_some(), tier));
    }
    debug_assert_eq!(block.len() as u64, words);
    block[KEY_SUN_AT as usize] = key.map_or(0, pack_direction);
    block
}

/// Of the chunks `zeroed` holds (zero records by the rule), those within `lr` chunks
/// (Chebyshev, x and y) of a column of `changed` (chunk columns whose mirror changed) that
/// `still` does not find buried now: due to bake again. `None` reads every zeroed chunk.
pub(crate) fn relisted(
    zeroed: &ChunkSet,
    changed: Option<&[[i32; 2]]>,
    lr: i32,
    mut still: impl FnMut([i32; 3]) -> bool,
) -> Vec<[i32; 3]> {
    let near = |z: &[i32; 3]| {
        changed.is_none_or(|ch| {
            ch.iter()
                .any(|c| (c[0] - z[0]).abs() <= lr && (c[1] - z[1]).abs() <= lr)
        })
    };
    let mut out: Vec<[i32; 3]> = zeroed.iter().filter(|z| near(z)).copied().collect();
    out.sort_unstable();
    out.retain(|z| !still(*z));
    out
}

impl BakeQueue {
    /// Whether the rule answers `coord` ([`buried`] under the step's lowest column tops, and no
    /// exposed emitter within the light radius `lr`, [`GpuWorld::emitters_near`]).
    pub(super) fn rule_holds(&mut self, gpu: &Shadow, coord: ChunkCoord, lr: i32) -> bool {
        let memo = &mut self.columns_lowest;
        buried(coord, lr, |x, y| {
            *memo
                .entry([x, y])
                .or_insert_with(|| gpu.column_lowest(x, y))
        }) && !gpu.emitters_near(coord, lr)
    }

    /// Whether `c`'s block is the rule's zero record.
    pub fn zeroed(&self, c: ChunkCoord) -> bool {
        self.zeroed.contains(&c.0)
    }

    /// Bakes again each zeroed chunk near a mirror change (`changes`; `None` untracked, every
    /// one) that the rule does not answer now: a dig that lowered a column top over its reach,
    /// an emitter arriving within it. Its block re-bakes whole ([`BakeQueue::reach`]) and fades.
    /// Lists again, the same way, each zeroed chunk one of whose neighbours not held full at its
    /// record's write ([`BakeQueue::zeroed_open`]) is held full now: the next opening rules it
    /// again on its faces as they stand. A zeroed chunk whose block is gone is forgotten.
    pub(super) fn relist_zeroed(
        &mut self,
        gpu: &Shadow,
        store: &Store,
        changes: Option<&[ChunkCoord]>,
    ) {
        if self.zeroed.is_empty() {
            return;
        }
        let open = &mut self.zeroed_open;
        self.zeroed.retain(|c| {
            let kept = store.blocks.contains_key(c);
            if !kept {
                open.remove(c);
            }
            kept
        });
        let cols: Option<Vec<[i32; 2]>> = changes.map(|ch| {
            let mut v: Vec<[i32; 2]> = ch.iter().map(|c| [c.0[0], c.0[1]]).collect();
            v.sort_unstable();
            v.dedup();
            v
        });
        if cols.as_ref().is_some_and(Vec::is_empty) {
            return;
        }
        let lr = gpu.light_radius();
        let zeroed = std::mem::take(&mut self.zeroed);
        let due = relisted(&zeroed, cols.as_deref(), lr, |z| {
            self.rule_holds(gpu, ChunkCoord(z), lr)
        });
        self.zeroed = zeroed;
        for z in due {
            self.stats.zero_relisted += 1;
            self.list_zeroed_again(z);
        }
        // The border: a zeroed chunk whose record was listed with a neighbour not held full
        // (read as rock) is listed again once one of them is held full, and ruled again on its
        // faces as they stand (ADR-0085). Only a changed chunk can be one newly held.
        let full = |n: [i32; 3]| gpu.is_mirrored(ChunkCoord(n)) && !gpu.is_lod(ChunkCoord(n));
        let mut border: Vec<[i32; 3]> = match changes {
            Some(ch) => ch
                .iter()
                .filter(|n| full(n.0))
                .flat_map(|n| (0..6).map(move |f| (neighbour(*n, f), f ^ 1)))
                .filter(|(z, f)| self.zeroed_open.get(z).is_some_and(|m| m & 1 << f != 0))
                .map(|(z, _)| z)
                .collect(),
            None => self
                .zeroed_open
                .iter()
                .filter(|(z, m)| {
                    (0..6).any(|f| *m & 1 << f != 0 && full(neighbour(ChunkCoord(**z), f)))
                })
                .map(|(z, _)| *z)
                .collect(),
        };
        border.sort_unstable();
        border.dedup();
        for z in border {
            if self.zeroed.contains(&z) {
                self.stats.zero_border_relisted += 1;
                self.list_zeroed_again(z);
            }
        }
    }

    /// Takes zeroed `z` out of the zeroed set and lists it again, whole ([`BakeQueue::reach`]):
    /// the next opening reads the rule on it again.
    fn list_zeroed_again(&mut self, z: [i32; 3]) {
        self.zeroed.remove(&z);
        self.zeroed_open.remove(&z);
        self.reach.insert(z);
        self.touched.insert(z);
        self.counted_idle = false;
        self.quiet_until = self.steps;
    }

    /// The rule reading's check of a candidate the opening found ready and the rule did not
    /// answer: when the rule holds on the tops as they stand, notes the first exclusion that
    /// applies (`provisional`, `partial`, a held block's class 1) and whether `c` lies in the
    /// view cone or the near shell, for its job's count ([`QueueStats::rule_missed`]).
    #[allow(
        clippy::too_many_arguments,
        reason = "the opening's check, read in place"
    )]
    pub(super) fn note_rule_missed(
        &mut self,
        gpu: &Shadow,
        view: &QueueView,
        eye: [i32; 3],
        c: [i32; 3],
        class: u8,
        partial: bool,
        provisional: bool,
    ) {
        let why = if provisional {
            0
        } else if partial {
            1
        } else if class != 0 {
            2
        } else {
            return;
        };
        if !self.rule_holds(gpu, ChunkCoord(c), gpu.light_radius()) {
            return;
        }
        let o = [0, 1, 2].map(|k| c[k] - eye[k]);
        let cone = angle_term(o, view.forward, view.half_angle, self.near) <= 0.0;
        self.rule_missed_checked.insert(c, (why, cone));
    }
}
