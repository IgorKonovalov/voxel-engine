//! Composing bricks layer by layer: a brick keeping only some materials, and a source's
//! z-layers written over a brick, as the collapse ops do.

use crate::brick::Brick;
use crate::coords::BRICK_VOXELS;
use crate::material::{AIR, MaterialId};

use super::format::{LAYERS, MAX_ENTRIES, Packed, Repr, entries, entry, match_word};
use super::kernel::{PackedStats, index_or_insert, remove_entries};

/// Palette entry `j`'s match word in layer `z` of a `Uniform` or planar brick: every bit for
/// a `Uniform` brick's single entry. `Wide` bricks never reach it.
fn src_match(src: &Brick, j: usize, z: usize) -> u64 {
    match src {
        Brick::Uniform(_) => u64::MAX,
        Brick::Full(Packed(r)) => {
            let (w, p) = r.planar().expect("a planar source");
            match_word(w, p, j, z)
        }
    }
}

impl Brick {
    /// Turns to air every voxel whose material `keep` refuses: for a planar box, the refused
    /// entries' voxels leave the mask, the planes are masked and the entries are removed; the
    /// brick is left canonical. A `Wide` box is mapped id by id and re-encoded.
    pub(crate) fn keep_only(&mut self, keep: impl Fn(MaterialId) -> bool) {
        let st = &mut PackedStats::default();
        let Self::Full(Packed(packed)) = self else {
            if let Self::Uniform(m) = *self
                && !keep(m)
            {
                *self = Self::Uniform(AIR);
            }
            return;
        };
        if let Repr::Wide(v) = packed {
            for m in v.iter_mut() {
                if *m != AIR && !keep(*m) {
                    *m = AIR;
                }
            }
            self.canonicalize(st);
            return;
        }
        let (w, p) = packed.planar_mut().expect("planar");
        let n = entries(w, p);
        let drop: u32 = (0..n)
            .filter(|&j| !keep(entry(w, p, j)))
            .fold(0, |a, j| a | 1 << j);
        if drop == 0 {
            return;
        }
        for z in 0..LAYERS {
            let hit = (0..n)
                .filter(|&j| drop >> j & 1 == 1)
                .fold(0, |a, j| a | match_word(w, p, j, z));
            w[z] &= !hit;
            for k in 0..p {
                w[LAYERS + LAYERS * k + z] &= w[z];
            }
        }
        remove_entries(w, p, drop, st);
        self.canonicalize(st);
    }

    /// Writes the non-air voxels of `src`'s z-layers `layers` into this brick's layers from
    /// `first` on, keeping this brick's voxels where `src`'s mask is clear: this brick's
    /// palette takes `src`'s entries, then per layer the source mask word joins the mask, the
    /// planes are cleared under it and each source entry's match word sets its new index; an
    /// entry left with no voxel is removed and the brick is left canonical. When the merged
    /// palette would pass 16 materials, or either brick is `Wide`, the layers are written id
    /// by id over decoded voxels instead.
    pub(crate) fn overlay_layers(
        &mut self,
        src: &Brick,
        layers: std::ops::Range<usize>,
        first: usize,
    ) {
        let st = &mut PackedStats::default();
        // The source's entries; `src_match(src, j, z)` is entry `j`'s match word in layer `z`.
        let src_pal = match src {
            Self::Uniform(AIR) => return,
            Self::Uniform(m) => vec![*m],
            Self::Full(p) => p.palette(),
        };
        let src_wide = matches!(src, Self::Full(Packed(Repr::Wide(_))));
        // The entries the moved layers hold.
        let moved: Vec<usize> = (0..src_pal.len())
            .filter(|&j| layers.clone().any(|z| src_match(src, j, z) != 0))
            .collect();
        let dst_pal = match self {
            Self::Uniform(AIR) => Vec::new(),
            Self::Uniform(d) => vec![*d],
            Self::Full(p) => p.palette(),
        };
        let mut merged = dst_pal.clone();
        for &j in &moved {
            if !merged.contains(&src_pal[j]) {
                merged.push(src_pal[j]);
            }
        }
        let dst_wide = matches!(self, Self::Full(Packed(Repr::Wide(_))));
        if src_wide || dst_wide || merged.len() > MAX_ENTRIES {
            let mut from = [AIR; BRICK_VOXELS];
            src.decode(&mut from);
            self.modify(|to| {
                for (k, z) in layers.clone().enumerate() {
                    for b in 0..64 {
                        let m = from[64 * z + b];
                        if m != AIR {
                            to[64 * (first + k) + b] = m;
                        }
                    }
                }
            });
            return;
        }
        if moved.is_empty() {
            return;
        }
        if let Self::Uniform(d) = *self {
            *self = Self::Full(Packed(Repr::expand_uniform(d)));
        }
        let Self::Full(Packed(packed)) = self else {
            unreachable!("expanded above")
        };
        for &j in &moved {
            index_or_insert(packed, src_pal[j], st).expect("at most 16 materials");
        }
        let (w, p) = packed.planar_mut().expect("planar");
        let to: Vec<usize> = moved
            .iter()
            .map(|&j| {
                (0..entries(w, p))
                    .position(|e| entry(w, p, e) == src_pal[j])
                    .expect("inserted")
            })
            .collect();
        for (k, z) in layers.enumerate() {
            let dz = first + k;
            let sm = moved.iter().fold(0, |a, &j| a | src_match(src, j, z));
            w[dz] |= sm;
            for q in 0..p {
                w[LAYERS + LAYERS * q + dz] &= !sm;
            }
            for (&j, &jd) in moved.iter().zip(&to) {
                let mw = src_match(src, j, z);
                for q in 0..p {
                    if jd >> q & 1 == 1 {
                        w[LAYERS + LAYERS * q + dz] |= mw;
                    }
                }
            }
        }
        let n = entries(w, p);
        let empty = (0..n)
            .filter(|&j| (0..LAYERS).all(|z| match_word(w, p, j, z) == 0))
            .fold(0u32, |a, j| a | 1 << j);
        remove_entries(w, p, empty, st);
        self.canonicalize(st);
    }
}
