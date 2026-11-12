//! The planar edit kernel: carve, fill and replace as word operations with popcount tallies,
//! and the class moves they make.

use crate::brick::Brick;
use crate::coords::BRICK_VOXELS;
use crate::material::{AIR, MaterialId};

use super::format::{
    LAYERS, Packed, PackedClass, Repr, bounds_of, entries, entry, match_word, set_entry,
};

/// What the planar kernel did to the boxes, summed over calls. Not part of any result: a
/// count of work, for tests and readings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PackedStats {
    /// A `Full` brick's box moved to another class (grown for an insert, shrunk after a
    /// removal, widened to or narrowed from raw ids). Expanding a `Uniform` brick and
    /// collapsing to one are not counted.
    pub class_changes: u64,
    /// Palette entries removed because their last voxel changed.
    pub entries_removed: u64,
    /// Replaces done by renaming one palette entry, with no plane written.
    pub renames: u64,
}

/// What a kernel call does to each old material.
#[derive(Clone, Copy, Debug)]
pub enum KernelMode<'a> {
    /// Bit `m` of `carves` (word `m / 64`) set: material `m` becomes air. An id at or beyond
    /// `rows` (a voxel the table does not describe) becomes air as well.
    Carve { carves: &'a [u64], rows: usize },
    /// Air becomes this material.
    Fill(MaterialId),
    /// `from` becomes `to`.
    Replace { from: MaterialId, to: MaterialId },
}

impl KernelMode<'_> {
    /// The material a voxel of `old` becomes, or `None` when the op leaves it unchanged.
    #[inline]
    pub(crate) fn target(self, old: MaterialId) -> Option<MaterialId> {
        match self {
            Self::Carve { carves, rows } => {
                let m = usize::from(old);
                (m >= rows || carves[m / 64] >> (m % 64) & 1 != 0).then_some(AIR)
            }
            Self::Fill(m) => (old == AIR && m != AIR).then_some(m),
            Self::Replace { from, to } => (old == from && from != to).then_some(to),
        }
    }
}

impl Brick {
    /// Applies `mode` to the voxels of `shape` (from [`shape_words`](super::shape_words)) in
    /// place, counting each changed voxel under its old material through `count`, and leaves
    /// the brick canonical. Returns the brick-local bounds of the changed voxels, `None` when
    /// nothing changed.
    pub(crate) fn edit_planar(
        &mut self,
        mode: KernelMode<'_>,
        shape: &[u64; LAYERS],
        count: &mut impl FnMut(MaterialId, u32),
        st: &mut PackedStats,
    ) -> Option<([u8; 3], [u8; 3])> {
        let bounds = self.edit_planar_uncanonical(mode, shape, count, st);
        self.canonicalize(st);
        bounds
    }

    /// [`Brick::edit_planar`] without the closing [`Brick::canonicalize`]: the brick may be
    /// left non-canonical, and the caller canonicalizes it before anyone else reads it. The op
    /// path's stage timer reads the two halves apart.
    pub(crate) fn edit_planar_uncanonical(
        &mut self,
        mode: KernelMode<'_>,
        shape: &[u64; LAYERS],
        count: &mut impl FnMut(MaterialId, u32),
        st: &mut PackedStats,
    ) -> Option<([u8; 3], [u8; 3])> {
        if let Self::Uniform(m) = *self {
            mode.target(m)?;
            if shape.iter().all(|&w| w == 0) {
                return None;
            }
            *self = Self::Full(Packed(Repr::expand_uniform(m)));
        }
        let Self::Full(Packed(packed)) = self else {
            unreachable!("expanded above")
        };
        let changed = match packed {
            Repr::Wide(v) => wide_edit(v, mode, shape, count),
            _ => planar_edit(packed, mode, shape, count, st),
        };
        bounds_of(&changed)
    }

    /// Moves a `Full` brick to the class of its palette, or to `Uniform` when it is all air
    /// or all one material; a `Wide` box is re-encoded when it may have fallen to 16 or fewer
    /// materials.
    pub(crate) fn canonicalize(&mut self, st: &mut PackedStats) {
        let Self::Full(Packed(packed)) = self else {
            return;
        };
        if let Repr::Wide(v) = packed {
            *self = Self::encode(v);
            if self.class().is_some_and(|c| c != PackedClass::Wide) {
                st.class_changes += 1;
            }
            return;
        }
        let (w, p) = packed.planar().expect("planar");
        let n = entries(w, p);
        if n == 0 {
            *self = Self::Uniform(AIR);
            return;
        }
        if n == 1 && w[..LAYERS].iter().all(|&m| m == u64::MAX) {
            *self = Self::Uniform(entry(w, p, 0));
            return;
        }
        let class = PackedClass::of(n);
        if class != packed.class() {
            *packed = packed.reclass(class);
            st.class_changes += 1;
        }
    }
}

/// The per-voxel write of a `Wide` box over the set bits of `shape`; returns the changed
/// voxels' words. The caller re-encodes the box.
fn wide_edit(
    v: &mut [MaterialId; BRICK_VOXELS],
    mode: KernelMode<'_>,
    shape: &[u64; LAYERS],
    count: &mut impl FnMut(MaterialId, u32),
) -> [u64; LAYERS] {
    let mut changed = [0u64; LAYERS];
    for z in 0..LAYERS {
        let mut bits = shape[z];
        while bits != 0 {
            let b = bits.trailing_zeros() as usize;
            bits &= bits - 1;
            let cell = &mut v[64 * z + b];
            if let Some(new) = mode.target(*cell) {
                count(*cell, 1);
                *cell = new;
                changed[z] |= 1 << b;
            }
        }
    }
    changed
}

/// The planar kernel on a planar box; may move the box to a larger class (or to `Wide`) when
/// a material is inserted. Returns the changed voxels' words.
fn planar_edit(
    packed: &mut Repr,
    mode: KernelMode<'_>,
    shape: &[u64; LAYERS],
    count: &mut impl FnMut(MaterialId, u32),
    st: &mut PackedStats,
) -> [u64; LAYERS] {
    match mode {
        KernelMode::Carve { .. } => carve(packed, mode, shape, count, st),
        KernelMode::Fill(m) => fill(packed, m, shape, count, st),
        KernelMode::Replace { from, to } => {
            if from == to {
                [0; LAYERS]
            } else if from == AIR {
                fill(packed, to, shape, count, st)
            } else {
                replace(packed, from, to, shape, count, st)
            }
        }
    }
}

/// Carve: `hit = mask & shape & OR(match_j : j carvable)`, a popcount per carvable entry,
/// `mask &= !hit`, planes masked, emptied entries removed.
fn carve(
    packed: &mut Repr,
    mode: KernelMode<'_>,
    shape: &[u64; LAYERS],
    count: &mut impl FnMut(MaterialId, u32),
    st: &mut PackedStats,
) -> [u64; LAYERS] {
    let (w, p) = packed.planar_mut().expect("planar");
    let n = entries(w, p);
    let mut carvable = 0u32;
    for j in 0..n {
        if mode.target(entry(w, p, j)).is_some() {
            carvable |= 1 << j;
        }
    }
    if carvable == 0 {
        return [0; LAYERS];
    }
    let all = carvable == (1u32 << n) - 1;
    let mut hit = [0u64; LAYERS];
    for z in 0..LAYERS {
        hit[z] = if all {
            w[z] & shape[z]
        } else {
            let mut c = 0;
            for j in 0..n {
                if carvable >> j & 1 == 1 {
                    c |= match_word(w, p, j, z);
                }
            }
            c & shape[z]
        };
    }
    if hit.iter().all(|&h| h == 0) {
        return hit;
    }
    // Tallies, and which entries the carve empties.
    let mut emptied = 0u32;
    if n == 1 {
        let h: u32 = hit.iter().map(|w| w.count_ones()).sum();
        let total: u32 = w[..LAYERS].iter().map(|w| w.count_ones()).sum();
        count(entry(w, p, 0), h);
        if h == total {
            emptied = 1;
        }
    } else {
        for j in 0..n {
            if carvable >> j & 1 == 0 {
                continue;
            }
            let (mut h, mut total) = (0u32, 0u32);
            for (z, &hz) in hit.iter().enumerate() {
                let m = match_word(w, p, j, z);
                h += (m & hz).count_ones();
                total += m.count_ones();
            }
            if h > 0 {
                count(entry(w, p, j), h);
                if h == total {
                    emptied |= 1 << j;
                }
            }
        }
    }
    for z in 0..LAYERS {
        w[z] &= !hit[z];
        for k in 0..p {
            w[LAYERS + LAYERS * k + z] &= w[z];
        }
    }
    remove_entries(w, p, emptied, st);
    hit
}

/// Fill: `f = !mask & shape`, the fill entry inserted if absent, `mask |= f`, the entry's
/// index written into the planes under `f`.
fn fill(
    packed: &mut Repr,
    m: MaterialId,
    shape: &[u64; LAYERS],
    count: &mut impl FnMut(MaterialId, u32),
    st: &mut PackedStats,
) -> [u64; LAYERS] {
    if m == AIR {
        return [0; LAYERS];
    }
    let f: [u64; LAYERS] = {
        let (w, _) = packed.planar().expect("planar");
        std::array::from_fn(|z| !w[z] & shape[z])
    };
    let air: u32 = f.iter().map(|w| w.count_ones()).sum();
    if air == 0 {
        return f;
    }
    count(AIR, air);
    let Some(j) = index_or_insert(packed, m, st) else {
        // Above 16 materials: the raw path.
        let Repr::Wide(v) = packed else {
            unreachable!("index_or_insert widens")
        };
        for z in 0..LAYERS {
            let mut bits = f[z];
            while bits != 0 {
                let b = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                v[64 * z + b] = m;
            }
        }
        return f;
    };
    let (w, p) = packed.planar_mut().expect("planar");
    write_index(w, p, j, &f, true);
    f
}

/// Replace of a non-air `from` by `to`: `h = match_from & shape`; a rename of the entry when
/// every `from` voxel is in `h`, `to` is absent and the rename keeps the palette ascending;
/// otherwise `to`'s index written over `h` (or the voxels carved, for air) and `from`'s entry
/// removed if emptied.
fn replace(
    packed: &mut Repr,
    from: MaterialId,
    to: MaterialId,
    shape: &[u64; LAYERS],
    count: &mut impl FnMut(MaterialId, u32),
    st: &mut PackedStats,
) -> [u64; LAYERS] {
    let (w, p) = packed.planar().expect("planar");
    let n = entries(w, p);
    let Some(jf) = (0..n).find(|&j| entry(w, p, j) == from) else {
        return [0; LAYERS];
    };
    let mut h = [0u64; LAYERS];
    let (mut hn, mut total) = (0u32, 0u32);
    for z in 0..LAYERS {
        let m = match_word(w, p, jf, z);
        h[z] = m & shape[z];
        hn += h[z].count_ones();
        total += m.count_ones();
    }
    if hn == 0 {
        return h;
    }
    count(from, hn);
    let every = hn == total;
    if to == AIR {
        let (w, p) = packed.planar_mut().expect("planar");
        for z in 0..LAYERS {
            w[z] &= !h[z];
            for k in 0..p {
                w[LAYERS + LAYERS * k + z] &= w[z];
            }
        }
        if every {
            remove_entries(w, p, 1 << jf, st);
        }
        return h;
    }
    let present = (0..n).any(|j| entry(w, p, j) == to);
    if every
        && !present
        && (jf == 0 || entry(w, p, jf - 1) < to)
        && (jf + 1 == n || to < entry(w, p, jf + 1))
    {
        let (w, p) = packed.planar_mut().expect("planar");
        set_entry(w, p, jf, to);
        st.renames += 1;
        return h;
    }
    let Some(jt) = index_or_insert(packed, to, st) else {
        let Repr::Wide(v) = packed else {
            unreachable!("index_or_insert widens")
        };
        for z in 0..LAYERS {
            let mut bits = h[z];
            while bits != 0 {
                let b = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                v[64 * z + b] = to;
            }
        }
        return h;
    };
    let (w, p) = packed.planar_mut().expect("planar");
    write_index(w, p, jt, &h, false);
    if every {
        let jf = (0..entries(w, p))
            .find(|&j| entry(w, p, j) == from)
            .expect("from is still an entry");
        remove_entries(w, p, 1 << jf, st);
    }
    h
}

/// Writes index `j` into the planes over the voxels of `over`; `fresh` when those voxels'
/// plane bits are known to be zero (they were air), so only the set bits are written.
fn write_index(w: &mut [u64], p: usize, j: usize, over: &[u64; LAYERS], fresh: bool) {
    for z in 0..LAYERS {
        w[z] |= over[z];
        for k in 0..p {
            let plane = &mut w[LAYERS + LAYERS * k + z];
            if j >> k & 1 == 1 {
                *plane |= over[z];
            } else if !fresh {
                *plane &= !over[z];
            }
        }
    }
}

/// The index of `m` in the palette, inserting it in ascending order when absent: the box
/// grows a class first if full, and every voxel at or above the insert position takes its
/// index plus one by a bit-sliced add. `None` when the insert widens the box to raw ids
/// (above 16 materials); the box is then `Wide`.
pub(super) fn index_or_insert(
    packed: &mut Repr,
    m: MaterialId,
    st: &mut PackedStats,
) -> Option<usize> {
    let (w, p) = packed.planar().expect("planar");
    let n = entries(w, p);
    let mut q = n;
    for j in 0..n {
        let e = entry(w, p, j);
        if e == m {
            return Some(j);
        }
        if e > m {
            q = j;
            break;
        }
    }
    if n == packed.class().capacity() {
        let class = packed.class().grown();
        st.class_changes += 1;
        if class == PackedClass::Wide {
            let mut v = Box::new([AIR; BRICK_VOXELS]);
            packed.decode(&mut v);
            *packed = Repr::Wide(v);
            return None;
        }
        *packed = packed.reclass(class);
    }
    let (w, p) = packed.planar_mut().expect("planar");
    // Voxels with index >= q: add one, carrying up the planes.
    for z in 0..LAYERS {
        let mut c = 0;
        for j in q..n {
            c |= match_word(w, p, j, z);
        }
        for k in 0..p {
            let plane = &mut w[LAYERS + LAYERS * k + z];
            let t = *plane & c;
            *plane ^= c;
            c = t;
        }
    }
    for j in (q..n).rev() {
        let e = entry(w, p, j);
        set_entry(w, p, j + 1, e);
    }
    set_entry(w, p, q, m);
    Some(q)
}

/// Removes the entries whose bits are set in `which` (each with no voxel left), highest
/// first: every voxel above a removed entry takes its index minus one by a bit-sliced
/// subtract, and the palette closes the gap.
pub(super) fn remove_entries(w: &mut [u64], p: usize, which: u32, st: &mut PackedStats) {
    let mut which = which;
    while which != 0 {
        let q = 31 - which.leading_zeros() as usize;
        which &= !(1 << q);
        let n = entries(w, p);
        for z in 0..LAYERS {
            let mut c = 0;
            for j in q + 1..n {
                c |= match_word(w, p, j, z);
            }
            for k in 0..p {
                let plane = &mut w[LAYERS + LAYERS * k + z];
                let t = !*plane & c;
                *plane ^= c;
                c = t;
            }
        }
        for j in q..n - 1 {
            let e = entry(w, p, j + 1);
            set_entry(w, p, j, e);
        }
        set_entry(w, p, n - 1, AIR);
        st.entries_removed += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coords::BRICK_VOXELS;
    use crate::hash::mix64;
    use crate::packed::format::tests::seeded_brick;
    use crate::packed::shape_words;

    /// The kernel against a per-voxel reference: 4000 seeded brick-edits mixing carve (with
    /// an indestructible id), fill and replace over bricks of 1 to 20 materials, each
    /// comparing voxels, per-material counts, bounds and the canonical form.
    #[test]
    fn the_kernel_matches_a_per_voxel_reference() {
        // Ids 1..=20 carvable except 4; rows 21.
        let mut carves = vec![0u64; 1];
        for m in 1..=20u16 {
            if m != 4 {
                carves[0] |= 1 << m;
            }
        }
        let mut s = 0x0021_0001u64;
        let mut next = |r: u64| {
            s = mix64(s);
            s % r
        };
        for case in 0..4000u64 {
            let materials = 1 + next(20);
            let solid = next(257);
            let v = if next(8) == 0 {
                [next(materials + 1) as MaterialId; BRICK_VOXELS]
            } else {
                seeded_brick(case, solid, materials)
            };
            let mut spans = [(1u8, 0u8); 64];
            for span in &mut spans {
                if next(3) != 0 {
                    let a = next(8) as u8;
                    let b = next(8) as u8;
                    *span = (a.min(b), a.max(b));
                }
            }
            let mode = match next(4) {
                0 | 1 => KernelMode::Carve {
                    carves: &carves,
                    rows: 21,
                },
                2 => KernelMode::Fill(next(materials + 2) as MaterialId),
                _ => KernelMode::Replace {
                    from: next(materials + 1) as MaterialId,
                    to: next(materials + 2) as MaterialId,
                },
            };
            let shape = shape_words(&spans);

            // Reference: per voxel.
            let mut want = v;
            let mut want_counts = std::collections::BTreeMap::new();
            let mut changed = [0u64; 8];
            for i in 0..BRICK_VOXELS {
                if shape[i / 64] >> (i % 64) & 1 == 0 {
                    continue;
                }
                if let Some(n) = mode.target(want[i]) {
                    *want_counts.entry(want[i]).or_insert(0u32) += 1;
                    want[i] = n;
                    changed[i / 64] |= 1 << (i % 64);
                }
            }

            let mut b = Brick::encode(&v);
            let mut counts = std::collections::BTreeMap::new();
            let bounds = b.edit_planar(
                mode,
                &shape,
                &mut |m, n| {
                    *counts.entry(m).or_insert(0u32) += n;
                },
                &mut PackedStats::default(),
            );
            let mut got = [0; BRICK_VOXELS];
            b.decode(&mut got);
            assert_eq!(got, want, "case {case} {mode:?}");
            assert_eq!(counts, want_counts, "case {case} {mode:?}");
            assert_eq!(bounds, bounds_of(&changed), "case {case}");
            assert_eq!(b, Brick::encode(&want), "case {case} canonical");
            if let Brick::Full(p) = &b {
                assert!(p.is_canonical(), "case {case}");
            }
        }
    }

    #[test]
    fn a_fill_inserts_and_a_carve_removes_an_entry() {
        // {air, 3}: fill 7 into the top layer, then carve every 7.
        let mut v = [AIR; BRICK_VOXELS];
        v[..64].fill(3);
        let mut b = Brick::encode(&v);
        assert_eq!(b.class(), Some(PackedClass::Mask));
        let mut spans = [(1u8, 0u8); 64];
        for span in &mut spans[56..] {
            *span = (0, 7);
        }
        let mut counts = Vec::new();
        let mut st = PackedStats::default();
        b.edit_planar(
            KernelMode::Fill(7),
            &shape_words(&spans),
            &mut |m, n| counts.push((m, n)),
            &mut st,
        );
        assert_eq!(counts, [(AIR, 64)]);
        assert_eq!(b.class(), Some(PackedClass::Plane1));
        let Brick::Full(p) = &b else { panic!() };
        assert_eq!(p.palette(), [3, 7]);
        let carves = [1u64 << 7];
        counts.clear();
        b.edit_planar(
            KernelMode::Carve {
                carves: &carves,
                rows: 8,
            },
            &shape_words(&[(0, 7); 64]),
            &mut |m, n| counts.push((m, n)),
            &mut st,
        );
        assert_eq!(
            st,
            PackedStats {
                class_changes: 2,
                entries_removed: 1,
                renames: 0
            }
        );
        assert_eq!(counts, [(7, 64)]);
        assert_eq!(b, Brick::encode(&v));
    }
}
