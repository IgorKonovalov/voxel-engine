//! The packed brick of ADR-0012: a 512-bit occupancy mask, a palette of the brick's non-air
//! materials in ascending id, and bit planes that index the palette, in one of five fixed-size
//! classes chosen by the count `n` of non-air materials.
//!
//! [`crate::Brick::Full`] holds a [`Packed`]. This module carries the encoding, `decode` and
//! `get`, and the planar edit kernel (carve, fill, replace as word operations with popcount
//! tallies).
//!
//! Word layout of a planar class with `p` planes (`Wide` is 512 raw ids instead):
//!
//! ```text
//! words 0..8          mask: word z, bit x + 8 y, set when the voxel is not air
//! words 8..8+8p       plane k at 8 + 8k .. 16 + 8k, the same bit layout; a set voxel's
//!                     palette index is the integer whose bit k is its plane-k bit
//! words 8+8p..        palette: entry j at bits 16 (j % 4) .. of word 8 + 8p + j / 4,
//!                     ascending material id, air never, unused entries zero
//! ```
//!
//! Canonical form, which `encode` produces and every kernel call leaves:
//!
//! 1. the class is [`PackedClass::of`] the count `n` of palette entries;
//! 2. the palette is strictly ascending and every entry has at least one voxel;
//! 3. a mask bit is set exactly when the voxel is not air, and plane bits are zero under a
//!    clear mask bit;
//! 4. a brick whose 512 voxels are equal is [`crate::Brick::Uniform`].
//!
//! So equal voxels have equal bytes, and the derived equality is exact voxel equality. Every
//! step is integer: shifts, masks and popcounts; nothing reads a clock or a hash map.

use crate::brick::Brick;
use crate::coords::BRICK_VOXELS;
use crate::material::{AIR, MaterialId};

/// The x span of each of a brick's 64 voxel rows (row `y + 8 z`), inclusive and brick-local;
/// an empty row has `x0 > x1`.
pub type RowSpans = [(u8, u8); 64];

/// Mask words per brick: one `u64` per z-layer.
const LAYERS: usize = 8;

/// Most palette entries a planar class holds.
const MAX_ENTRIES: usize = 16;

/// The five fixed sizes of a packed brick, by its count `n` of non-air materials.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PackedClass {
    /// `n` 1: the mask and one palette entry, 72 B.
    Mask,
    /// `n` 2: one plane, 136 B.
    Plane1,
    /// `n` 3 to 4: two planes, 200 B.
    Plane2,
    /// `n` 5 to 16: four planes, 352 B.
    Plane4,
    /// `n` above 16: 512 raw ids, 1024 B.
    Wide,
}

impl PackedClass {
    /// The class of a brick holding `n` non-air materials (`n` 0, a transient state inside
    /// an edit, is `Mask`).
    pub const fn of(n: usize) -> Self {
        match n {
            0 | 1 => Self::Mask,
            2 => Self::Plane1,
            3 | 4 => Self::Plane2,
            5..=MAX_ENTRIES => Self::Plane4,
            _ => Self::Wide,
        }
    }

    /// Bit planes of a planar class; `Wide` has none.
    pub const fn planes(self) -> usize {
        match self {
            Self::Mask | Self::Wide => 0,
            Self::Plane1 => 1,
            Self::Plane2 => 2,
            Self::Plane4 => 4,
        }
    }

    /// Palette entries the class holds; `Wide` holds any count.
    pub const fn capacity(self) -> usize {
        match self {
            Self::Mask => 1,
            Self::Plane1 => 2,
            Self::Plane2 => 4,
            Self::Plane4 => MAX_ENTRIES,
            Self::Wide => usize::MAX,
        }
    }

    /// Payload bytes of the class's box.
    pub const fn bytes(self) -> usize {
        match self {
            Self::Wide => BRICK_VOXELS * 2,
            c => 8 * words(c.planes(), c.capacity()),
        }
    }

    /// The next larger class.
    const fn grown(self) -> Self {
        match self {
            Self::Mask => Self::Plane1,
            Self::Plane1 => Self::Plane2,
            Self::Plane2 => Self::Plane4,
            Self::Plane4 | Self::Wide => Self::Wide,
        }
    }
}

/// `u64` words of a planar class with `p` planes and `cap` palette entries.
const fn words(p: usize, cap: usize) -> usize {
    LAYERS + LAYERS * p + cap.div_ceil(4)
}

/// The payload of a `Full` brick in one of the five classes, opaque outside this module: its
/// constructors keep the canonical form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packed(pub(crate) Repr);

impl Packed {
    /// The class of the box.
    pub fn class(&self) -> PackedClass {
        self.0.class()
    }

    /// Payload bytes of the box.
    pub fn bytes(&self) -> usize {
        self.0.bytes()
    }

    /// The palette entries, ascending; empty for `Wide`.
    pub fn palette(&self) -> Vec<MaterialId> {
        self.0.palette()
    }

    /// The occupancy mask, one `u64` per z-layer.
    pub fn mask(&self) -> [u64; LAYERS] {
        self.0.mask()
    }

    /// The material at brick index `i` (`x + 8 y + 64 z`).
    pub fn get(&self, i: usize) -> MaterialId {
        self.0.get(i)
    }

    /// Writes all 512 voxels into `out`.
    pub fn decode(&self, out: &mut [MaterialId; BRICK_VOXELS]) {
        self.0.decode(out)
    }

    /// Whether the box is in canonical form (rules 1 to 3 above).
    pub fn is_canonical(&self) -> bool {
        self.0.is_canonical()
    }

    /// Layer `z` of the occupancy mask (bit `x + 8 y`).
    pub fn mask_word(&self, z: usize) -> u64 {
        match &self.0 {
            Repr::Wide(v) => (0..64).fold(0, |w, b| w | u64::from(v[64 * z + b] != AIR) << b),
            r => r.planar().expect("planar").0[z],
        }
    }

    /// Calls `f(material, n)` with the count `n > 0` of each non-air material among the
    /// voxels whose bit is set in `sel` (layer-major words, as [`shape_words`]), once per
    /// material, ascending: one popcount per palette entry and layer; a `Wide` box tallies
    /// its ids under `sel`.
    pub fn counts_under(&self, sel: &[u64; LAYERS], f: &mut impl FnMut(MaterialId, u32)) {
        match &self.0 {
            Repr::Wide(v) => {
                let mut counts: Vec<(MaterialId, u32)> = Vec::new();
                for (z, &w) in sel.iter().enumerate() {
                    let mut bits = w;
                    while bits != 0 {
                        let b = bits.trailing_zeros() as usize;
                        bits &= bits - 1;
                        let m = v[64 * z + b];
                        if m == AIR {
                            continue;
                        }
                        match counts.iter_mut().find(|(k, _)| *k == m) {
                            Some((_, n)) => *n += 1,
                            None => counts.push((m, 1)),
                        }
                    }
                }
                counts.sort_unstable();
                for (m, n) in counts {
                    f(m, n);
                }
            }
            r => {
                let (w, p) = r.planar().expect("planar");
                for j in 0..entries(w, p) {
                    let n: u32 = (0..LAYERS)
                        .map(|z| (match_word(w, p, j, z) & sel[z]).count_ones())
                        .sum();
                    if n > 0 {
                        f(entry(w, p, j), n);
                    }
                }
            }
        }
    }

    /// The mask-only box of a `Uniform(m)` brick about to be edited (not canonical): for the
    /// kernel bench's expansion figure.
    #[doc(hidden)]
    pub fn expand_uniform(m: MaterialId) -> Self {
        Self(Repr::expand_uniform(m))
    }

    /// The box moved into a larger planar class (not canonical): for the kernel bench's class
    /// change figure.
    #[doc(hidden)]
    pub fn reclass(&self, class: PackedClass) -> Self {
        Self(self.0.reclass(class))
    }
}

/// The five class boxes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Repr {
    Mask(Box<[u64; words(0, 1)]>),
    Plane1(Box<[u64; words(1, 2)]>),
    Plane2(Box<[u64; words(2, 4)]>),
    Plane4(Box<[u64; words(4, MAX_ENTRIES)]>),
    Wide(Box<[MaterialId; BRICK_VOXELS]>),
}

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
    fn target(self, old: MaterialId) -> Option<MaterialId> {
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

/// The eight shape words of a brick-edit: bit `x + 8 y` of word `z` set when voxel
/// `(x, y, z)` lies in its row's span.
pub fn shape_words(spans: &RowSpans) -> [u64; LAYERS] {
    let mut w = [0u64; LAYERS];
    for (r, &(x0, x1)) in spans.iter().enumerate() {
        if x0 <= x1 {
            let row = (0xFFu64 >> (7 - (x1 - x0))) << x0;
            w[r / 8] |= row << (8 * (r % 8));
        }
    }
    w
}

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

/// The words of the brick-local box `lo..=hi` (components in `0..8`, inclusive).
pub fn box_words(lo: [u8; 3], hi: [u8; 3]) -> [u64; LAYERS] {
    let row = (0xFFu64 >> (7 - (hi[0] - lo[0]))) << lo[0];
    let mut layer = 0u64;
    for y in lo[1]..=hi[1] {
        layer |= row << (8 * y);
    }
    std::array::from_fn(|z| {
        if (lo[2]..=hi[2]).contains(&(z as u8)) {
            layer
        } else {
            0
        }
    })
}

/// The brick-local bounds `(lo, hi)` of the set bits of `changed`, or `None` when none is
/// set: z from the non-zero words, y from the non-zero bytes of their OR, x from the OR of
/// that word's bytes.
pub fn bounds_of(changed: &[u64; LAYERS]) -> Option<([u8; 3], [u8; 3])> {
    let zlo = changed.iter().position(|&w| w != 0)?;
    let zhi = changed.iter().rposition(|&w| w != 0)?;
    let fold = changed.iter().fold(0, |a, &w| a | w);
    let ylo = fold.trailing_zeros() / 8;
    let yhi = 7 - fold.leading_zeros() / 8;
    let mut xb = 0u8;
    for y in 0..8 {
        xb |= (fold >> (8 * y)) as u8;
    }
    let xlo = xb.trailing_zeros();
    let xhi = 7 - xb.leading_zeros();
    Some((
        [xlo as u8, ylo as u8, zlo as u8],
        [xhi as u8, yhi as u8, zhi as u8],
    ))
}

impl Repr {
    /// The class of this box.
    pub fn class(&self) -> PackedClass {
        match self {
            Self::Mask(_) => PackedClass::Mask,
            Self::Plane1(_) => PackedClass::Plane1,
            Self::Plane2(_) => PackedClass::Plane2,
            Self::Plane4(_) => PackedClass::Plane4,
            Self::Wide(_) => PackedClass::Wide,
        }
    }

    /// The words of a planar class and its plane count; `None` for `Wide`.
    fn planar(&self) -> Option<(&[u64], usize)> {
        match self {
            Self::Mask(w) => Some((&w[..], 0)),
            Self::Plane1(w) => Some((&w[..], 1)),
            Self::Plane2(w) => Some((&w[..], 2)),
            Self::Plane4(w) => Some((&w[..], 4)),
            Self::Wide(_) => None,
        }
    }

    fn planar_mut(&mut self) -> Option<(&mut [u64], usize)> {
        match self {
            Self::Mask(w) => Some((&mut w[..], 0)),
            Self::Plane1(w) => Some((&mut w[..], 1)),
            Self::Plane2(w) => Some((&mut w[..], 2)),
            Self::Plane4(w) => Some((&mut w[..], 4)),
            Self::Wide(_) => None,
        }
    }

    /// A zeroed box of a planar class.
    fn zeroed(class: PackedClass) -> Self {
        match class {
            PackedClass::Mask => Self::Mask(Box::new([0; words(0, 1)])),
            PackedClass::Plane1 => Self::Plane1(Box::new([0; words(1, 2)])),
            PackedClass::Plane2 => Self::Plane2(Box::new([0; words(2, 4)])),
            PackedClass::Plane4 => Self::Plane4(Box::new([0; words(4, MAX_ENTRIES)])),
            PackedClass::Wide => Self::Wide(Box::new([AIR; BRICK_VOXELS])),
        }
    }

    /// The mask-only box of a `Uniform(m)` brick about to be edited: every mask bit set and
    /// the palette `[m]`, or for air, no bit and no entry. The box is not canonical until the
    /// edit finishes.
    pub fn expand_uniform(m: MaterialId) -> Self {
        let mut w = Box::new([0u64; words(0, 1)]);
        if m != AIR {
            w[..LAYERS].fill(u64::MAX);
            w[LAYERS] = u64::from(m);
        }
        Self::Mask(w)
    }

    /// The box's words moved into a box of `class` (a planar class at least as large as the
    /// box's palette): the mask, the planes the new class has, and the palette. Planes beyond
    /// the new class's must be zero, which holds when every index is below its capacity.
    pub fn reclass(&self, class: PackedClass) -> Self {
        let (src, p) = self.planar().expect("reclass of a planar class");
        let mut out = Self::zeroed(class);
        let (dst, q) = out.planar_mut().expect("a planar class");
        dst[..LAYERS * (1 + p.min(q))].copy_from_slice(&src[..LAYERS * (1 + p.min(q))]);
        let n = entries(src, p);
        for j in 0..n {
            set_entry(dst, q, j, entry(src, p, j));
        }
        out
    }

    /// Payload bytes of the box.
    pub fn bytes(&self) -> usize {
        self.class().bytes()
    }

    /// The palette entries, ascending; empty for `Wide`.
    pub fn palette(&self) -> Vec<MaterialId> {
        match self.planar() {
            Some((w, p)) => (0..entries(w, p)).map(|j| entry(w, p, j)).collect(),
            None => Vec::new(),
        }
    }

    /// The occupancy mask, one `u64` per z-layer.
    pub fn mask(&self) -> [u64; LAYERS] {
        match self.planar() {
            Some((w, _)) => std::array::from_fn(|z| w[z]),
            None => {
                let Self::Wide(v) = self else { unreachable!() };
                let mut bits = [0u64; LAYERS];
                for (i, &m) in v.iter().enumerate() {
                    bits[i / 64] |= u64::from(m != AIR) << (i % 64);
                }
                bits
            }
        }
    }

    /// The material at brick index `i` (`x + 8 y + 64 z`).
    pub fn get(&self, i: usize) -> MaterialId {
        let Some((w, p)) = self.planar() else {
            let Self::Wide(v) = self else { unreachable!() };
            return v[i];
        };
        let (z, b) = (i / 64, i % 64);
        if w[z] >> b & 1 == 0 {
            return AIR;
        }
        let mut idx = 0;
        for k in 0..p {
            idx |= ((w[LAYERS + LAYERS * k + z] >> b & 1) as usize) << k;
        }
        entry(w, p, idx)
    }

    /// Writes all 512 voxels into `out`.
    pub fn decode(&self, out: &mut [MaterialId; BRICK_VOXELS]) {
        match self {
            Self::Mask(w) => decode_planar::<0>(&w[..], out),
            Self::Plane1(w) => decode_planar::<1>(&w[..], out),
            Self::Plane2(w) => decode_planar::<2>(&w[..], out),
            Self::Plane4(w) => decode_planar::<4>(&w[..], out),
            Self::Wide(v) => out.copy_from_slice(&v[..]),
        }
    }

    /// Whether the box is in canonical form (rules 1 to 3 above; rule 4 is a property of the
    /// brick that holds it).
    pub fn is_canonical(&self) -> bool {
        match self.planar() {
            Some((w, p)) => {
                let slots = (w.len() - LAYERS - LAYERS * p) * 4;
                let n = entries(w, p);
                if PackedClass::of(n) != self.class() || n == 0 {
                    return false;
                }
                // Ascending entries, then zero in every slot after them.
                if (1..n).any(|j| entry(w, p, j - 1) >= entry(w, p, j))
                    || (n..slots).any(|j| entry(w, p, j) != AIR)
                {
                    return false;
                }
                // No plane bit under a clear mask bit, and no voxel indexing past the entries.
                for z in 0..LAYERS {
                    if (0..p).any(|k| w[LAYERS + LAYERS * k + z] & !w[z] != 0) {
                        return false;
                    }
                    if (0..n).fold(0, |a, j| a | match_word(w, p, j, z)) != w[z] {
                        return false;
                    }
                }
                (0..n).all(|j| (0..LAYERS).any(|z| match_word(w, p, j, z) != 0))
            }
            None => {
                let Self::Wide(v) = self else { unreachable!() };
                distinct_non_air(v) > MAX_ENTRIES
            }
        }
    }
}

/// Palette entry `j` of a planar box with `p` planes.
#[inline]
fn entry(w: &[u64], p: usize, j: usize) -> MaterialId {
    (w[LAYERS + LAYERS * p + j / 4] >> (16 * (j % 4))) as MaterialId
}

#[inline]
fn set_entry(w: &mut [u64], p: usize, j: usize, m: MaterialId) {
    let word = &mut w[LAYERS + LAYERS * p + j / 4];
    let s = 16 * (j % 4);
    *word = *word & !(0xFFFF << s) | u64::from(m) << s;
}

/// Palette entries in use: the leading non-zero entries.
fn entries(w: &[u64], p: usize) -> usize {
    let cap = (w.len() - LAYERS - LAYERS * p) * 4;
    let cap = cap.min(1 << p);
    (0..cap).take_while(|&j| entry(w, p, j) != AIR).count()
}

/// The voxels of layer `z` whose palette index is `j`.
#[inline]
fn match_word(w: &[u64], p: usize, j: usize, z: usize) -> u64 {
    let mut m = w[z];
    for k in 0..p {
        let plane = w[LAYERS + LAYERS * k + z];
        m &= if j >> k & 1 == 1 { plane } else { !plane };
    }
    m
}

fn decode_planar<const P: usize>(w: &[u64], out: &mut [MaterialId; BRICK_VOXELS]) {
    // lut[0] is air; lut[1 + j] is entry j.
    let mut lut = [AIR; MAX_ENTRIES + 1];
    for (j, slot) in lut[1..=1 << P].iter_mut().enumerate() {
        *slot = entry(w, P, j);
    }
    for z in 0..LAYERS {
        let mask = w[z];
        let planes: [u64; P] = std::array::from_fn(|k| w[LAYERS + LAYERS * k + z]);
        for b in 0..64 {
            let mut idx = 1;
            for (k, plane) in planes.iter().enumerate() {
                idx += ((plane >> b & 1) as usize) << k;
            }
            let set = (mask >> b & 1) as usize;
            out[64 * z + b] = lut[idx * set];
        }
    }
}

/// Distinct non-air ids among `v`, by a sort of a copy.
fn distinct_non_air(v: &[MaterialId; BRICK_VOXELS]) -> usize {
    let mut s = *v;
    s.sort_unstable();
    let mut n = 0;
    let mut last = AIR;
    for m in s {
        if m != AIR && m != last {
            n += 1;
        }
        last = m;
    }
    n
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

    /// Calls `f(i, m)` for every non-air voxel of the brick-local box `lo..=hi` (inclusive,
    /// components in `0..8`), `i` its brick index and `m` its material, in index order (z,
    /// then y, then x): a `Full` brick walks the set bits of its mask under the box's words.
    pub fn for_each_solid_in(
        &self,
        lo: [u8; 3],
        hi: [u8; 3],
        mut f: impl FnMut(usize, MaterialId),
    ) {
        let sel = box_words(lo, hi);
        match self {
            Self::Uniform(AIR) => {}
            Self::Uniform(m) => {
                for (z, &w) in sel.iter().enumerate() {
                    let mut bits = w;
                    while bits != 0 {
                        let b = bits.trailing_zeros() as usize;
                        bits &= bits - 1;
                        f(64 * z + b, *m);
                    }
                }
            }
            Self::Full(p) => {
                for (z, &w) in sel.iter().enumerate() {
                    let mut bits = w & p.mask_word(z);
                    while bits != 0 {
                        let b = bits.trailing_zeros() as usize;
                        bits &= bits - 1;
                        f(64 * z + b, p.get(64 * z + b));
                    }
                }
            }
        }
    }

    /// Whether the 2^3 block holding brick-local voxel `(x, y, z)` of a `Full` brick has no
    /// non-air voxel: two mask words under `0x0303 << (2 (x / 2) + 16 (y / 2))`. `false` for a
    /// `Uniform` brick.
    pub(crate) fn block_clear(&self, [x, y, z]: [usize; 3]) -> bool {
        let Self::Full(p) = self else {
            return false;
        };
        let sel = 0x0303u64 << (2 * (x / 2) + 16 * (y / 2));
        let z0 = z & !1;
        (p.mask_word(z0) | p.mask_word(z0 + 1)) & sel == 0
    }

    /// The canonical form of 512 voxels: `Uniform` when they are all equal, else the class of
    /// their count of non-air materials.
    pub(crate) fn encode(v: &[MaterialId; BRICK_VOXELS]) -> Self {
        let first = v[0];
        if v.iter().all(|&m| m == first) {
            return Self::Uniform(first);
        }
        // Up to 16 distinct non-air ids, the last one found cached.
        let mut pal = [AIR; MAX_ENTRIES];
        let mut n = 0;
        let mut last = AIR;
        for &m in v {
            if m == AIR || m == last {
                continue;
            }
            last = m;
            if pal[..n].contains(&m) {
                continue;
            }
            if n == MAX_ENTRIES {
                return Self::Full(Packed(Repr::Wide(Box::new(*v))));
            }
            pal[n] = m;
            n += 1;
        }
        pal[..n].sort_unstable();
        let mut out = Repr::zeroed(PackedClass::of(n));
        let (w, p) = out.planar_mut().expect("n <= 16 is planar");
        for (j, &m) in pal[..n].iter().enumerate() {
            set_entry(w, p, j, m);
        }
        let (mut last, mut last_j) = (AIR, 0);
        for (i, &m) in v.iter().enumerate() {
            if m == AIR {
                continue;
            }
            if m != last {
                last = m;
                last_j = pal[..n].iter().position(|&e| e == m).unwrap_or(0);
            }
            let (z, b) = (i / 64, i % 64);
            w[z] |= 1 << b;
            for k in 0..p {
                w[LAYERS + LAYERS * k + z] |= ((last_j >> k & 1) as u64) << b;
            }
        }
        Self::Full(Packed(out))
    }

    /// Applies `mode` to the voxels of `shape` (from [`shape_words`]) in place, counting each
    /// changed voxel under its old material through `count`, and leaves the brick canonical.
    /// Returns the brick-local bounds of the changed voxels, `None` when nothing changed.
    #[doc(hidden)]
    pub fn edit_planar(
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
        let bounds = bounds_of(&changed);
        self.canonicalize(st);
        bounds
    }

    /// Moves a `Full` brick to the class of its palette, or to `Uniform` when it is all air
    /// or all one material; a `Wide` box is re-encoded when it may have fallen to 16 or fewer
    /// materials.
    fn canonicalize(&mut self, st: &mut PackedStats) {
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
fn index_or_insert(packed: &mut Repr, m: MaterialId, st: &mut PackedStats) -> Option<usize> {
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
fn remove_entries(w: &mut [u64], p: usize, which: u32, st: &mut PackedStats) {
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
    use crate::hash::mix64;

    /// A seeded brick: each voxel solid with probability `solid / 256`, a solid voxel one of
    /// `materials` ids starting at 1.
    fn seeded_brick(seed: u64, solid: u64, materials: u64) -> [MaterialId; BRICK_VOXELS] {
        let mut s = seed;
        std::array::from_fn(|_| {
            s = mix64(s);
            if s % 256 < solid {
                (1 + (s >> 8) % materials) as MaterialId
            } else {
                AIR
            }
        })
    }

    fn round_trip(v: &[MaterialId; BRICK_VOXELS]) {
        let e = Brick::encode(v);
        let mut d = [0; BRICK_VOXELS];
        e.decode(&mut d);
        assert_eq!(&d, v);
        assert_eq!(Brick::encode(&d), e);
        for (i, &m) in v.iter().enumerate() {
            assert_eq!(e.get(i), m);
        }
        if let Brick::Full(p) = &e {
            assert!(p.is_canonical(), "{:?}", p.class());
            assert_eq!(p.mask(), {
                let mut bits = [0u64; 8];
                for (i, &m) in v.iter().enumerate() {
                    bits[i / 64] |= u64::from(m != AIR) << (i % 64);
                }
                bits
            });
        }
    }

    #[test]
    fn classes_have_the_ladder_sizes() {
        let sizes = [
            PackedClass::Mask,
            PackedClass::Plane1,
            PackedClass::Plane2,
            PackedClass::Plane4,
            PackedClass::Wide,
        ]
        .map(PackedClass::bytes);
        assert_eq!(sizes, [72, 136, 200, 352, 1024]);
        assert_eq!(size_of::<Packed>(), 16);
        assert_eq!(size_of::<Brick>(), 16);
    }

    /// `decode(encode(v)) == v` and `encode(decode(encode(v))) == encode(v)` over 256 seeded
    /// bricks at solid fractions 1/256 to 1 and material counts 1 to 300.
    #[test]
    fn encode_round_trips_over_seeded_bricks() {
        let mut classes = std::collections::BTreeSet::new();
        for i in 0..256u64 {
            let solid = 1 + i;
            let materials = 1 + (i * 299) / 255;
            let v = seeded_brick(0x5EED_0021 ^ i, solid, materials);
            round_trip(&v);
            classes.insert(Brick::encode(&v).class());
        }
        // Every class is exercised; Uniform bricks are added by hand.
        assert_eq!(classes.len(), 5, "{classes:?}");
        for m in [AIR, 5] {
            round_trip(&[m; BRICK_VOXELS]);
            assert_eq!(Brick::encode(&[m; BRICK_VOXELS]), Brick::Uniform(m));
        }
    }

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
