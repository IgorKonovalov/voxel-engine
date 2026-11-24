//! The stream workers' no-face verdict (ADR-0080): a chunk whose voxels hold no exposed face,
//! read from its own masks and its six neighbours' border layers, is admitted to the live queue
//! already known faceless, so the main thread never lists it.
//!
//! Each worker reduces the chunk it loads ([`ChunkMasks`]) to a [`Shell`]: whether any face lies
//! between two of its own voxels, and its six border layers' opaque and solid bits. The shells of
//! every full chunk the workers have loaded live in one [`Shells`] map behind a lock; a worker
//! inserts its chunk's shell and, under the same lock, reads its own verdict and its present
//! neighbours' again, so each verdict is read from the map as it stood at a point of one total
//! order ([`Verdict::seq`]). A neighbour with no shell (not yet streamed, past the residency, or
//! edited since) counts as open: air across that border, so the verdict errs toward a job. The
//! listing reads an absent neighbour as rock (`list_chunk_faces`), so a chunk sealed here lists
//! no face there either.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use super::faces::{CLASS_AIR, CLASS_OPAQUE, ChunkMasks, MASKED};
use svet_world::{CHUNK_BRICKS, brick_of_slot};

/// Rows of a chunk's voxels: row `64 z + y` holds bit `x` of each voxel along x.
type Rows = Box<[u64; 4096]>;

/// One border layer of a chunk (64 x 64 voxels): word `j`, bit `i`, where `(i, j)` is `(y, z)`
/// across an x face, `(x, z)` across a y face and `(x, y)` across a z face, so a layer and its
/// neighbour's opposite layer index the same voxel pairs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Layer {
    /// Every voxel of the layer of one class: air, opaque, or translucent (solid, not opaque).
    Uniform(u8),
    /// The layer's opaque bits and its solid bits (opaque or translucent).
    Mixed(Box<([u64; 64], [u64; 64])>),
}

impl Layer {
    fn of(opaque: [u64; 64], solid: [u64; 64]) -> Self {
        let all = |m: &[u64; 64], v: u64| m.iter().all(|&w| w == v);
        if all(&solid, 0) {
            Self::Uniform(CLASS_AIR)
        } else if all(&opaque, u64::MAX) {
            Self::Uniform(CLASS_OPAQUE)
        } else if all(&opaque, 0) && all(&solid, u64::MAX) {
            Self::Uniform(super::faces::CLASS_TRANSLUCENT)
        } else {
            Self::Mixed(Box::new((opaque, solid)))
        }
    }

    /// Word `j` of the layer's opaque and solid bits.
    fn word(&self, j: usize) -> (u64, u64) {
        match self {
            Self::Uniform(c) if *c == CLASS_AIR => (0, 0),
            Self::Uniform(c) if *c == CLASS_OPAQUE => (u64::MAX, u64::MAX),
            Self::Uniform(_) => (0, u64::MAX),
            Self::Mixed(m) => (m.0[j], m.1[j]),
        }
    }

    /// Whether the layer holds no solid voxel.
    fn is_air(&self) -> bool {
        matches!(self, Self::Uniform(c) if *c == CLASS_AIR)
    }

    /// Bytes held beyond the enum.
    fn bytes(&self) -> usize {
        match self {
            Self::Uniform(_) => 0,
            Self::Mixed(_) => 1024,
        }
    }
}

/// What the verdict reads of a chunk: whether a face lies between two of its own voxels
/// (`inner`), and its border layers in face order (`-x, +x, -y, +y, -z, +z`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Shell {
    pub inner: bool,
    pub layers: [Layer; 6],
}

/// Whether `own` (a border layer) lists no face toward `beside` (the neighbour's layer across
/// it; `None` open, air): each opaque voxel faces an opaque one, each translucent voxel a solid
/// one.
fn sealed_toward(own: &Layer, beside: Option<&Layer>) -> bool {
    let Some(beside) = beside else {
        return own.is_air();
    };
    if own.is_air() || matches!(beside, Layer::Uniform(c) if *c == CLASS_OPAQUE) {
        return true;
    }
    (0..64).all(|j| {
        let (o, s) = own.word(j);
        let (bo, bs) = beside.word(j);
        o & !bo == 0 && (s & !o) & !bs == 0
    })
}

impl Shell {
    /// The shell of a chunk of `masks`.
    pub(crate) fn of(masks: &ChunkMasks) -> Self {
        if let Some(c) = uniform_class(masks) {
            return Self {
                inner: false,
                layers: std::array::from_fn(|_| Layer::Uniform(c)),
            };
        }
        let (opaque, solid) = rows(masks);
        let inner = inner_face(&opaque, &solid);
        let layers = std::array::from_fn(|face| {
            let o = layer_of(&opaque, face);
            let s = layer_of(&solid, face);
            Layer::of(o, s)
        });
        Self { inner, layers }
    }

    /// Whether the chunk lists no face with `beside(face)` its neighbour's shell across each face
    /// (`None` open).
    pub(crate) fn sealed<'a>(&self, beside: impl Fn(usize) -> Option<&'a Shell>) -> bool {
        !self.inner
            && (0..6).all(|f| sealed_toward(&self.layers[f], beside(f).map(|s| &s.layers[f ^ 1])))
    }

    fn bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.layers.iter().map(Layer::bytes).sum::<usize>()
    }
}

/// The class every voxel of `masks` holds, if one does.
fn uniform_class(masks: &ChunkMasks) -> Option<u8> {
    let first = masks.classes[0];
    (first != MASKED && masks.classes.iter().all(|&c| c == first)).then_some(first)
}

/// `masks`' opaque and solid voxels as rows ([`Rows`]).
fn rows(masks: &ChunkMasks) -> (Rows, Rows) {
    let mut opaque: Rows = Box::new([0; 4096]);
    let mut solid: Rows = Box::new([0; 4096]);
    for slot in 0..CHUNK_BRICKS {
        let (o, s) = masks.get(slot);
        if s == [0; 8] {
            continue;
        }
        let b = brick_of_slot(slot).map(|v| v as usize);
        for z in 0..8 {
            for y in 0..8 {
                let row = 64 * (b[2] * 8 + z) + b[1] * 8 + y;
                let shift = 8 * b[0];
                opaque[row] |= (o[z] >> (8 * y) & 0xFF) << shift;
                solid[row] |= (s[z] >> (8 * y) & 0xFF) << shift;
            }
        }
    }
    (opaque, solid)
}

/// Whether a face lies between two voxels of the chunk: an opaque voxel beside one that is not
/// opaque, or a translucent voxel beside air, along any axis.
fn inner_face(opaque: &Rows, solid: &Rows) -> bool {
    // Faces between `a` (a voxel's row) and `b` (its neighbour's, the same bits): an opaque bit
    // of either beside a non-opaque bit of the other, a translucent bit beside a non-solid one.
    let pair = |ao: u64, as_: u64, bo: u64, bs: u64| -> bool {
        (ao & !bo) | (bo & !ao) | ((as_ & !ao) & !bs) | ((bs & !bo) & !as_) != 0
    };
    const INNER: u64 = !(1u64 << 63);
    for r in 0..4096 {
        let (o, s) = (opaque[r], solid[r]);
        // Along x: bit x against bit x + 1, for x < 63.
        if pair(o & INNER, s & INNER, (o >> 1) & INNER, (s >> 1) & INNER) {
            return true;
        }
        let (z, y) = (r / 64, r % 64);
        if y < 63 && pair(o, s, opaque[r + 1], solid[r + 1]) {
            return true;
        }
        if z < 63 && pair(o, s, opaque[r + 64], solid[r + 64]) {
            return true;
        }
    }
    false
}

/// `rows`' border layer across `face` ([`Layer`]'s indexing).
fn layer_of(rows: &Rows, face: usize) -> [u64; 64] {
    let mut out = [0u64; 64];
    match face {
        0 | 1 => {
            let bit = if face == 0 { 0 } else { 63 };
            for (z, w) in out.iter_mut().enumerate() {
                for y in 0..64 {
                    *w |= (rows[64 * z + y] >> bit & 1) << y;
                }
            }
        }
        2 | 3 => {
            let y = if face == 2 { 0 } else { 63 };
            for (z, w) in out.iter_mut().enumerate() {
                *w = rows[64 * z + y];
            }
        }
        _ => {
            let z = if face == 4 { 0 } else { 63 };
            for (y, w) in out.iter_mut().enumerate() {
                *w = rows[64 * z + y];
            }
        }
    }
    out
}

/// A verdict on chunk `c`: sealed (no face) or not, at point `seq` of the shells' order. Of two
/// verdicts on one chunk the later `seq` holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Verdict {
    pub c: [i32; 3],
    pub sealed: bool,
    pub seq: u64,
}

/// The shells of the full chunks the workers loaded, each with its last verdict, and the order's
/// counter. Shared by the workers ([`super::FacePrep`]) and the live queue, which forgets an
/// edited chunk's shell ([`Shells::forget`]) and prunes the unheld ([`Shells::prune`]).
#[derive(Debug, Default)]
pub(crate) struct Shells {
    seq: u64,
    chunks: HashMap<[i32; 3], (Shell, bool)>,
    bytes: usize,
}

fn beside(c: [i32; 3], face: usize) -> [i32; 3] {
    let mut n = c;
    n[face / 2] += if face % 2 == 1 { 1 } else { -1 };
    n
}

impl Shells {
    fn verdict_of(&self, c: [i32; 3]) -> Option<bool> {
        let (shell, _) = self.chunks.get(&c)?;
        Some(shell.sealed(|f| self.chunks.get(&beside(c, f)).map(|(s, _)| s)))
    }

    /// Takes `c`'s shell (a worker's load): `c`'s verdict, and those of its present neighbours
    /// whose verdict it changed, all at one new point of the order.
    pub(crate) fn admit(&mut self, c: [i32; 3], shell: Shell) -> Vec<Verdict> {
        self.seq += 1;
        let seq = self.seq;
        self.bytes += shell.bytes();
        if let Some((old, _)) = self.chunks.insert(c, (shell, false)) {
            self.bytes -= old.bytes();
        }
        let mut out = Vec::with_capacity(7);
        let own = self.verdict_of(c).expect("inserted above");
        self.chunks.get_mut(&c).expect("inserted above").1 = own;
        out.push(Verdict {
            c,
            sealed: own,
            seq,
        });
        for f in 0..6 {
            let n = beside(c, f);
            let Some(v) = self.verdict_of(n) else {
                continue;
            };
            let entry = self.chunks.get_mut(&n).expect("read above");
            if entry.1 != v {
                entry.1 = v;
                out.push(Verdict {
                    c: n,
                    sealed: v,
                    seq,
                });
            }
        }
        out
    }

    /// After an edit: `edited`'s shells go (their content changed), and `near`'s verdicts read
    /// not sealed until a neighbour's arrival reads them again. The new point of the order: a
    /// verdict on any of them from before it does not hold.
    pub(crate) fn forget(&mut self, edited: &[[i32; 3]], near: &[[i32; 3]]) -> u64 {
        self.seq += 1;
        for c in edited {
            if let Some((s, _)) = self.chunks.remove(c) {
                self.bytes -= s.bytes();
            }
        }
        for c in near {
            if let Some(e) = self.chunks.get_mut(c) {
                e.1 = false;
            }
        }
        self.seq
    }

    /// Keeps the shells of the chunks `keep` holds.
    pub(crate) fn prune(&mut self, keep: impl Fn([i32; 3]) -> bool) {
        let mut bytes = 0;
        self.chunks.retain(|c, (s, _)| {
            let k = keep(*c);
            if k {
                bytes += s.bytes();
            }
            k
        });
        self.bytes = bytes;
    }

    /// Shells held.
    pub(crate) fn len(&self) -> usize {
        self.chunks.len()
    }

    /// Bytes the shells hold.
    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }
}

/// The handle the workers and the queue share.
pub(crate) type SharedShells = Arc<Mutex<Shells>>;

/// Locks `s`, through a poisoned lock.
pub(crate) fn lock(s: &SharedShells) -> std::sync::MutexGuard<'_, Shells> {
    s.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests;
