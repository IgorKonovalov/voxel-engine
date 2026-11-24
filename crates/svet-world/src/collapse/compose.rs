//! The bricks an op writes, composed in memory over the packed brick or decoded ids, and
//! where a dropped brick's layers land.

use std::collections::BTreeMap;

use super::BE;
use super::snapshot::Snapshot;
use crate::brick::Brick;
#[cfg(feature = "bench-internals")]
use crate::brick::{RawBrick, SpareArrays};
use crate::coarse::CellCoord;
use crate::coords::ChunkCoord;
use crate::material::{AIR, MaterialId};
use crate::packed::{KernelMode, PackedStats};
use crate::store::{ChunkMap, ChunkReader, World, write};
use crate::support::{CellKey, CellMap};

/// A brick as [`Compose`] holds it while an op writes it: the packed [`Brick`] itself (the
/// native path: palette merges and layer words) or decoded `RawBrick` ids (the reference
/// path, written voxel by voxel).
pub(super) trait Composed: Sized {
    /// The world's brick.
    fn of_world(b: &Brick) -> Self;
    /// A region cell's brick holding only the voxels `spare` keeps.
    fn kept(b: &Brick, spare: &impl Fn(MaterialId) -> bool) -> Self;
    fn is_air(&self) -> bool;
    fn set_uniform(&mut self, m: MaterialId);
    /// Writes the non-air voxels of `src`'s z-layers `layers` into layers `first..`.
    fn place(&mut self, src: &Brick, layers: std::ops::Range<usize>, first: usize);
    /// Sets the air voxels under `shape` (layer words) to `m`.
    fn fill(&mut self, shape: &[u64; 8], m: MaterialId);
    /// Sets the voxels under `shape` to air.
    fn clear(&mut self, shape: &[u64; 8]);
    fn into_brick(self) -> Brick;
}

impl Composed for Brick {
    fn of_world(b: &Brick) -> Self {
        b.clone()
    }

    fn kept(b: &Brick, spare: &impl Fn(MaterialId) -> bool) -> Self {
        let mut b = b.clone();
        b.keep_only(spare);
        b
    }

    fn is_air(&self) -> bool {
        *self == Brick::Uniform(AIR)
    }

    fn set_uniform(&mut self, m: MaterialId) {
        *self = Brick::Uniform(m);
    }

    fn place(&mut self, src: &Brick, layers: std::ops::Range<usize>, first: usize) {
        self.overlay_layers(src, layers, first);
    }

    fn fill(&mut self, shape: &[u64; 8], m: MaterialId) {
        self.edit_planar(
            KernelMode::Fill(m),
            shape,
            &mut |_, _| {},
            &mut PackedStats::default(),
        );
    }

    fn clear(&mut self, shape: &[u64; 8]) {
        // Every id is at or past `rows == 0`, so every voxel under the shape becomes air.
        self.edit_planar(
            KernelMode::Carve {
                carves: &[],
                rows: 0,
            },
            shape,
            &mut |_, _| {},
            &mut PackedStats::default(),
        );
    }

    fn into_brick(self) -> Brick {
        self
    }
}

#[cfg(feature = "bench-internals")]
impl Composed for RawBrick {
    fn of_world(b: &Brick) -> Self {
        RawBrick::of(b)
    }

    fn kept(b: &Brick, spare: &impl Fn(MaterialId) -> bool) -> Self {
        let kept = |m: MaterialId| if spare(m) { m } else { AIR };
        match RawBrick::of(b) {
            RawBrick::Uniform(m) => RawBrick::Uniform(kept(m)),
            RawBrick::Full(v) => {
                let mut b = RawBrick::Full(Box::new(v.map(kept)));
                b.collapse(&mut SpareArrays::new());
                b
            }
        }
    }

    fn is_air(&self) -> bool {
        *self == RawBrick::Uniform(AIR)
    }

    fn set_uniform(&mut self, m: MaterialId) {
        *self = RawBrick::Uniform(m);
    }

    fn place(&mut self, src: &Brick, layers: std::ops::Range<usize>, first: usize) {
        const LAYER: usize = (BE * BE) as usize;
        let src = RawBrick::of(src);
        let span = layers.start * LAYER..layers.end * LAYER;
        let at = first * LAYER;
        match (&src, &*self) {
            (RawBrick::Uniform(m), RawBrick::Uniform(d)) if m == d => {}
            (RawBrick::Uniform(m), _) => {
                self.expand(&mut SpareArrays::new())[at..at + span.len()].fill(*m)
            }
            (RawBrick::Full(v), _) => {
                let from = &v[span];
                if let RawBrick::Uniform(d) = *self
                    && from.iter().all(|&m| m == AIR || m == d)
                {
                    return;
                }
                let dst = &mut self.expand(&mut SpareArrays::new())[at..at + from.len()];
                for (d, &m) in dst.iter_mut().zip(from) {
                    if m != AIR {
                        *d = m;
                    }
                }
            }
        }
    }

    fn fill(&mut self, shape: &[u64; 8], m: MaterialId) {
        for_each_bit(shape, |i| {
            if self.get(i) == AIR && m != AIR {
                self.expand(&mut SpareArrays::new())[i] = m;
            }
        });
    }

    fn clear(&mut self, shape: &[u64; 8]) {
        for_each_bit(shape, |i| {
            if self.get(i) != AIR {
                self.expand(&mut SpareArrays::new())[i] = AIR;
            }
        });
    }

    fn into_brick(self) -> Brick {
        RawBrick::into_brick(self)
    }
}

/// Calls `f(i)` for the brick index of every set bit of `shape`, ascending.
#[cfg(feature = "bench-internals")]
fn for_each_bit(shape: &[u64; 8], mut f: impl FnMut(usize)) {
    for (z, &w) in shape.iter().enumerate() {
        let mut bits = w;
        while bits != 0 {
            f(64 * z + bits.trailing_zeros() as usize);
            bits &= bits - 1;
        }
    }
}

/// The cells the solid voxels of `src`, the brick of region cell `c`, land in when dropped by
/// `dropped`, as `(destination cell, source layers, destination layer of the first source
/// layer)`. Layers `r..8` of `src` (with `r = dropped mod 8`) land in the cell `dropped / 8`
/// below `c`, layers `0..r` in the cell under that. A destination is named only when its
/// source layers hold a solid voxel: a piece whose bottom layers are air lands in one cell, and
/// the cell under it is neither read nor required to be resident.
fn landings(
    c: CellCoord,
    src: &Brick,
    dropped: i32,
) -> impl Iterator<Item = (CellCoord, std::ops::Range<usize>, usize)> {
    let (q, r) = (dropped.div_euclid(BE), dropped.rem_euclid(BE));
    let [cx, cy, cz] = c.0;
    let (r, be) = (r as usize, BE as usize);
    let bits = src.solid_bits();
    let upper = (CellCoord([cx, cy, cz - q]), r..be, 0);
    let lower = (CellCoord([cx, cy, cz - q - 1]), 0..r, be - r);
    [upper, lower]
        .into_iter()
        .filter(move |(_, layers, _)| bits[layers.clone()].iter().any(|&w| w != 0))
}

/// Fails with the chunk of the first landing cell of `snap`'s region, dropped by `dropped`,
/// that is not resident, taking the region's cells ascending by `(x, y)`, then `z`.
pub(super) fn check_landings(
    snap: &Snapshot,
    reader: &mut ChunkReader<'_>,
    dropped: i32,
) -> Result<(), ChunkCoord> {
    for (&[cx, cy], zs) in &snap.stacks {
        for &cz in zs {
            let c = CellCoord([cx, cy, cz]);
            for (to, _, _) in landings(c, &snap.packed[&CellKey(c)], dropped) {
                reader.brick(to)?;
            }
        }
    }
    Ok(())
}

/// The bricks an op will write, composed in memory: each starts as the world's brick (or, for
/// a region cell, as its spared voxels in air, since every other solid voxel leaves) and takes
/// writes; [`Compose::commit`] stores each in canonical form.
pub(super) struct Compose<B> {
    bricks: CellMap<B>,
}

impl<B: Composed> Compose<B> {
    /// Every region cell, holding only the voxels `spare` keeps.
    pub(super) fn new(snap: &Snapshot, spare: &impl Fn(MaterialId) -> bool) -> Self {
        let bricks = snap
            .packed
            .iter()
            .map(|(&k, b)| (k, B::kept(b, spare)))
            .collect();
        Self { bricks }
    }

    /// The composed brick of `c`, read from the world on first use. Every cell an op composes
    /// is resident by construction: a region cell is in the snapshot, a crushed cell was read
    /// by the gap scan, and every landing cell is checked by [`check_landings`] before
    /// anything is composed.
    pub(super) fn brick(&mut self, reader: &mut ChunkReader<'_>, c: CellCoord) -> &mut B {
        self.bricks.entry(CellKey(c)).or_insert_with(|| {
            B::of_world(
                reader
                    .brick(c)
                    .expect("composed cells are checked resident before composing"),
            )
        })
    }

    /// Writes the solid voxels of `src`, the brick of region cell `c`, `dropped` voxels lower,
    /// into the cells [`landings`] names. Each destination brick is looked up once and takes
    /// whole z-layers.
    pub(super) fn place_dropped(
        &mut self,
        reader: &mut ChunkReader<'_>,
        c: CellCoord,
        src: &Brick,
        dropped: i32,
    ) {
        for (to, layers, first) in landings(c, src, dropped) {
            self.brick(reader, to).place(src, layers, first);
        }
    }

    /// Writes every composed brick that differs from the world's, in canonical form, ascending
    /// by chunk.
    pub(super) fn commit(self, world: &World, map: &ChunkMap) {
        let mut by_chunk: BTreeMap<ChunkCoord, Vec<(usize, Brick)>> = BTreeMap::new();
        for (CellKey(c), b) in self.bricks {
            let (chunk, slot) = c.split();
            by_chunk
                .entry(chunk)
                .or_default()
                .push((slot, b.into_brick()));
        }
        for (coord, bricks) in by_chunk {
            let mut chunk = write(&map[&coord]);
            let mut changed = false;
            for (slot, b) in bricks {
                if *chunk.brick(slot) != b {
                    chunk.set_brick(slot, b);
                    changed = true;
                }
            }
            if changed {
                chunk.mark_edited();
                world.store.mark_dirty(coord);
            }
        }
    }
}
