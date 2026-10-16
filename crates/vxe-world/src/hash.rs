//! Deterministic integer hashing and value noise, for sources and for pinning results.
//!
//! Everything here is integer arithmetic with wrapping where stated, so a seed gives the same
//! values on every platform and compiler.

use crate::coords::BRICK_VOXELS;
use crate::mips::{ChunkMips, MIP_LEVELS};
use crate::store::{World, read};

/// FNV-1a, 64-bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fnv1a(pub u64);

impl Default for Fnv1a {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv1a {
    pub fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    pub fn finish(&self) -> u64 {
        self.0
    }
}

impl World {
    /// FNV-1a 64 over the resident voxels: chunks ascending (coordinate as three little-endian
    /// `i32`), bricks in slot order, voxels x-fastest as little-endian `u16`. A `Uniform` brick
    /// hashes as its 512 expanded values, so representation never moves the hash.
    pub fn voxel_hash(&self) -> u64 {
        let mut h = Fnv1a::default();
        for c in self.store.coords() {
            for v in c.0 {
                h.write(&v.to_le_bytes());
            }
            let Some(chunk) = self.store.get(c) else {
                continue;
            };
            let chunk = read(&chunk);
            for b in chunk.bricks() {
                for i in 0..BRICK_VOXELS {
                    h.write(&b.get(i).to_le_bytes());
                }
            }
        }
        h.finish()
    }
}

impl ChunkMips {
    /// Feeds the mips to `h`: levels 1 to 3, mip-bricks in order, cells x-fastest as
    /// little-endian `u16`, a `Uniform` mip-brick as its 512 expanded cells.
    pub fn write_hash(&self, h: &mut Fnv1a) {
        for n in 1..=MIP_LEVELS {
            for b in self.level(n) {
                for i in 0..BRICK_VOXELS {
                    h.write(&b.get(i).to_le_bytes());
                }
            }
        }
    }
}

impl World {
    /// FNV-1a 64 over the mips of every resident chunk, built from its current voxels:
    /// chunks ascending, each as its coordinate (three little-endian `i32`) then
    /// [`ChunkMips::write_hash`].
    pub fn mips_hash(&self) -> u64 {
        let mut h = Fnv1a::default();
        for c in self.store.coords() {
            let Some(chunk) = self.store.get(c) else {
                continue;
            };
            for v in c.0 {
                h.write(&v.to_le_bytes());
            }
            ChunkMips::build(&read(&chunk)).write_hash(&mut h);
        }
        h.finish()
    }
}

/// The SplitMix64 finalizer: a bijective 64-bit mix with full avalanche.
pub const fn mix64(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// A hash of a seed and an integer lattice point.
pub const fn hash3(seed: u64, p: [i32; 3]) -> u64 {
    let mut h = mix64(seed ^ 0x9e37_79b9_7f4a_7c15);
    h = mix64(h ^ (p[0] as u32 as u64));
    h = mix64(h ^ (p[1] as u32 as u64).rotate_left(21));
    mix64(h ^ (p[2] as u32 as u64).rotate_left(42))
}

/// Value noise: a pseudo-random value in `-32768..=32767` at every multiple of `cell` along
/// each axis, interpolated trilinearly between them.
///
/// Interpolation is exact up to one final floor: the eight corner values are weighted by
/// integer offsets inside the cell and the sum is shifted down once. Within one cell the
/// result therefore differs from a true trilinear function by less than one unit, and over
/// any box inside one cell it lies within one unit of the range spanned by the box's corners.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ValueNoise {
    pub seed: u64,
    /// `cell == 1 << cell_log2` voxels between lattice points.
    pub cell_log2: u32,
}

impl ValueNoise {
    pub const fn new(seed: u64, cell_log2: u32) -> Self {
        Self { seed, cell_log2 }
    }

    /// The lattice value at lattice point `c`.
    pub const fn lattice(&self, c: [i32; 3]) -> i64 {
        (hash3(self.seed, c) >> 48) as i64 - 32768
    }

    /// The interpolated value at integer point `p`.
    pub fn sample(&self, p: [i32; 3]) -> i64 {
        self.interpolate(&self.corners(self.cell_of(p)), p)
    }

    /// The lattice cell holding `p`.
    pub fn cell_of(&self, p: [i32; 3]) -> [i32; 3] {
        p.map(|v| v >> self.cell_log2)
    }

    /// The eight lattice values around `cell`, corner `i` at offset
    /// `(i & 1, (i >> 1) & 1, (i >> 2) & 1)`.
    pub fn corners(&self, cell: [i32; 3]) -> [i64; 8] {
        std::array::from_fn(|i| {
            let o = [i & 1, (i >> 1) & 1, (i >> 2) & 1].map(|b| b as i32);
            self.lattice([0, 1, 2].map(|k| cell[k].wrapping_add(o[k])))
        })
    }

    /// The value at `p` from the corners of the cell holding it; equal to `sample(p)`. Lets a
    /// caller evaluating many points of one cell hash its corners once.
    pub fn interpolate(&self, corners: &[i64; 8], p: [i32; 3]) -> i64 {
        let s = self.cell_log2;
        let cell = 1i64 << s;
        let f = p.map(|v| i64::from(v) & (cell - 1));
        let mut sum = 0i64;
        for (i, &c) in corners.iter().enumerate() {
            let mut w = 1i64;
            for (k, &fk) in f.iter().enumerate() {
                w *= if i >> k & 1 == 1 { fk } else { cell - fk };
            }
            sum += c * w;
        }
        sum >> (3 * s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a_matches_the_reference_vectors() {
        let h = |s: &[u8]| {
            let mut f = Fnv1a::default();
            f.write(s);
            f.finish()
        };
        assert_eq!(h(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(h(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(h(b"foobar"), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn noise_hits_the_lattice_values_on_lattice_points() {
        let n = ValueNoise::new(7, 5);
        for c in [[0, 0, 0], [-1, 2, -3], [5, -7, 11]] {
            assert_eq!(n.sample(c.map(|v| v * 32)), n.lattice(c));
        }
    }

    #[test]
    fn noise_is_pinned() {
        let n = ValueNoise::new(0x5EED, 3);
        let mut f = Fnv1a::default();
        for x in -20..20 {
            for y in -3..3 {
                for z in -3..3 {
                    f.write(&n.sample([x, y, z]).to_le_bytes());
                }
            }
        }
        assert_eq!(f.finish(), 0xe235_ade2_75c9_202f);
    }
}
