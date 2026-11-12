//! Counts over a world's chunks: the field surface's band census, the bricks by kind, the RAM
//! a chunk holds and a hash of chunks' voxels.

use std::sync::Arc;
use vxe_world::hash::Fnv1a;
use vxe_world::{BRICK_VOXELS, Brick, Chunk, ChunkCoord, ChunkMips, World};

/// The field surface's band over a set of chunks (ADR-0040 Alternative B): a voxel is in the
/// band when its 27-count, the solid voxels of the 3 x 3 x 3 block around it with itself, is
/// neither 0 nor 27; a brick is a band brick when it holds a band voxel. Solid is not air
/// ([`Brick::solid_bits`]), a translucent voxel included, as for the field pass. The bytes
/// are a band slot's: packed, one band-mask unit plus `ceil(n / 64)` units of `u8` counts for
/// `n` band voxels, plus two header units for a uniform brick (which has no slot today);
/// dense, nine units per band brick.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BandCensus {
    /// Band voxels that are solid.
    pub solid_voxels: u64,
    /// Band voxels that are air.
    pub air_voxels: u64,
    /// Band bricks that are `Full`.
    pub full_bricks: u64,
    /// `Full` bricks with a surface voxel (a solid voxel with an air voxel among its 26
    /// neighbours).
    pub surface_full_bricks: u64,
    /// `Full` bricks with a surface voxel that are band bricks.
    pub surface_full_band_bricks: u64,
    /// Band bricks that are uniform and solid.
    pub solid_bricks: u64,
    /// Band bricks that are uniform air.
    pub air_bricks: u64,
    pub packed_bytes: u64,
    pub dense_bytes: u64,
}

impl BandCensus {
    pub fn bricks(&self) -> u64 {
        self.full_bricks + self.solid_bricks + self.air_bricks
    }
}

/// The 27-counts of the 512 voxels of brick `slot` of the chunk at the centre of `around`
/// (the 27 chunks around it, index `(dx + 1) + 3 (dy + 1) + 9 (dz + 1)`, `None` read as air),
/// voxel order `x + 8 y + 64 z`, and the brick's surface (its solid voxels with an air face
/// neighbour, bit `i % 64` of word `i / 64`); `None` when the brick and its 26 neighbours are
/// all uniform and all solid or all air (every count 27 or 0, no surface).
pub fn brick_counts(
    around: &[Option<&Chunk>; 27],
    slot: usize,
) -> Option<([u8; BRICK_VOXELS], [u64; 8])> {
    let l = vxe_world::brick_of_slot(slot);
    // The 27 bricks around, as their solid bits; `kinds` says whether all are one uniform kind.
    let mut bits = [[0u64; 8]; 27];
    let (mut any_solid, mut any_air, mut any_full) = (false, false, false);
    for (k, out) in bits.iter_mut().enumerate() {
        let d = [k as i32 % 3 - 1, (k as i32 / 3) % 3 - 1, k as i32 / 9 - 1];
        let b = [0, 1, 2].map(|a| l[a] + d[a]);
        let cd = b.map(|v| v.div_euclid(8));
        let ci = ((cd[0] + 1) + 3 * (cd[1] + 1) + 9 * (cd[2] + 1)) as usize;
        let brick = around[ci].map(|c| c.brick(vxe_world::brick_slot(b.map(|v| v.rem_euclid(8)))));
        match brick {
            None | Some(Brick::Uniform(vxe_world::AIR)) => any_air = true,
            Some(Brick::Uniform(_)) => {
                any_solid = true;
                *out = [u64::MAX; 8];
            }
            Some(b @ Brick::Full(_)) => {
                any_full = true;
                *out = b.solid_bits();
            }
        }
    }
    if !any_full && !(any_solid && any_air) {
        return None;
    }
    // The occupancy of the brick and a one-voxel shell, 10^3, cell c at voxel c - 1.
    let mut occ = [0u8; 1000];
    for z in 0..10i32 {
        for y in 0..10i32 {
            for x in 0..10i32 {
                let v = [x - 1, y - 1, z - 1];
                let d = v.map(|c| c.div_euclid(8));
                let k = ((d[0] + 1) + 3 * (d[1] + 1) + 9 * (d[2] + 1)) as usize;
                let lv = v.map(|c| c.rem_euclid(8));
                let on = (bits[k][lv[2] as usize] >> (lv[0] + 8 * lv[1])) & 1;
                occ[(x + 10 * (y + 10 * z)) as usize] = on as u8;
            }
        }
    }
    // Separable box sums: along x (8 x 10 x 10), y (8 x 8 x 10), z (8 x 8 x 8).
    let mut sx = [0u8; 800];
    for z in 0..10 {
        for y in 0..10 {
            for x in 0..8 {
                let i = x + 10 * (y + 10 * z);
                sx[x + 8 * (y + 10 * z)] = occ[i] + occ[i + 1] + occ[i + 2];
            }
        }
    }
    let mut sy = [0u8; 640];
    for z in 0..10 {
        for y in 0..8 {
            for x in 0..8 {
                let i = x + 8 * (y + 10 * z);
                sy[x + 8 * (y + 8 * z)] = sx[i] + sx[i + 8] + sx[i + 16];
            }
        }
    }
    let mut out = [0u8; BRICK_VOXELS];
    let mut surface = [0u64; 8];
    for (i, c) in out.iter_mut().enumerate() {
        *c = sy[i] + sy[i + 64] + sy[i + 128];
        let (x, y, z) = (i % 8 + 1, (i / 8) % 8 + 1, i / 64 + 1);
        let at = |x: usize, y: usize, z: usize| occ[x + 10 * (y + 10 * z)];
        if at(x, y, z) == 1
            && (at(x - 1, y, z)
                & at(x + 1, y, z)
                & at(x, y - 1, z)
                & at(x, y + 1, z)
                & at(x, y, z - 1)
                & at(x, y, z + 1))
                == 0
        {
            surface[i / 64] |= 1 << (i % 64);
        }
    }
    Some((out, surface))
}

/// The chunks around `c` that `world` holds, for [`brick_counts`]; the guards are read locks.
pub(crate) fn around_guards(
    world: &World,
    c: ChunkCoord,
) -> Vec<Option<Arc<std::sync::RwLock<Chunk>>>> {
    (0..27)
        .map(|k| {
            let d = [k % 3 - 1, (k / 3) % 3 - 1, k / 9 - 1];
            world
                .store()
                .get(ChunkCoord([0, 1, 2].map(|a| c.0[a] + d[a])))
        })
        .collect()
}

/// [`BandCensus`] over the bricks of `chunks` in `world`; a voxel of a chunk `world` does not
/// hold reads as air.
pub fn band_census(world: &World, chunks: &[ChunkCoord]) -> BandCensus {
    let mut cs = BandCensus::default();
    for &c in chunks {
        let arcs = around_guards(world, c);
        let guards: Vec<_> = arcs
            .iter()
            .map(|a| {
                a.as_ref()
                    .map(|a| a.read().unwrap_or_else(std::sync::PoisonError::into_inner))
            })
            .collect();
        let around: [Option<&Chunk>; 27] = std::array::from_fn(|k| guards[k].as_deref());
        let Some(centre) = around[13] else {
            continue;
        };
        for slot in 0..vxe_world::CHUNK_BRICKS {
            let Some((counts, surface)) = brick_counts(&around, slot) else {
                continue;
            };
            let brick = centre.brick(slot);
            let solid = brick.solid_bits();
            let surface = surface.iter().any(|&w| w != 0);
            let mut n = 0u64;
            for (i, &k) in counts.iter().enumerate() {
                let on = (solid[i / 64] >> (i % 64)) & 1 != 0;
                if k > 0 && k < 27 {
                    n += 1;
                    if on {
                        cs.solid_voxels += 1;
                    } else {
                        cs.air_voxels += 1;
                    }
                }
            }
            let full = brick.is_full();
            if full && surface {
                cs.surface_full_bricks += 1;
                if n > 0 {
                    cs.surface_full_band_bricks += 1;
                }
            }
            if n == 0 {
                continue;
            }
            match brick {
                Brick::Full(_) => cs.full_bricks += 1,
                Brick::Uniform(vxe_world::AIR) => cs.air_bricks += 1,
                Brick::Uniform(_) => cs.solid_bricks += 1,
            }
            let header = if full { 0 } else { 2 };
            cs.packed_bytes += 64 * (1 + n.div_ceil(64) + header);
            cs.dense_bytes += 64 * 9;
        }
    }
    cs
}

/// FNV-1a 64 over the voxels of the resident chunks `coords`, in the given order: each as its
/// coordinate (three little-endian `i32`), then its bricks in slot order with voxels x-fastest
/// as little-endian `u16`, a `Uniform` brick expanded. `None` if one is not resident.
pub fn chunks_hash(world: &World, coords: &[ChunkCoord]) -> Option<u64> {
    let mut h = Fnv1a::default();
    for c in coords {
        for v in c.0 {
            h.write(&v.to_le_bytes());
        }
        let chunk = world.store().get(*c)?;
        let chunk = chunk
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for b in chunk.bricks() {
            for i in 0..BRICK_VOXELS {
                h.write(&b.get(i).to_le_bytes());
            }
        }
    }
    Some(h.finish())
}

/// RAM a resident full chunk holds: its slot array and the class boxes of its `Full` bricks.
pub fn chunk_ram(chunk: &Chunk) -> u64 {
    let payload: usize = chunk.bricks().iter().map(Brick::payload_bytes).sum();
    (size_of::<Chunk>() + payload) as u64
}

/// RAM a LOD-only chunk holds: its 73 mip-brick slots and the class boxes of its `Full` ones.
pub fn mips_ram(mips: &ChunkMips) -> u64 {
    let slots: usize = vxe_world::MIP_BRICKS.iter().sum();
    let payload: usize = (1..=vxe_world::MIP_LEVELS)
        .flat_map(|n| mips.level(n))
        .map(Brick::payload_bytes)
        .sum();
    (size_of::<ChunkMips>() + slots * size_of::<Brick>() + payload) as u64
}
