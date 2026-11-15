//! A `Full` brick's exposed emissive voxels found by 512-bit masks: one word a z-layer, bit
//! `x + 8 y` (the brick index `x + 8 y + 64 z`), as the packed brick's own mask words.

use super::*;

/// Bits of a layer word whose voxel has x = 0, and those with x = 7.
const COL_LOW: u64 = 0x0101_0101_0101_0101;
const COL_HIGH: u64 = COL_LOW << 7;

/// The voxels of a brick whose material satisfies a flag, one word a z-layer.
pub(super) type BrickMask = [u64; 8];

/// The voxels of `p` whose material satisfies `f`, in the z-layers `layers` (the others left
/// clear), from the palette's flags over the planar class's index planes (`Packed::entry_matches`'
/// words, read in place); a `Wide` box, which has no palette, is read voxel by voxel. The
/// smaller side of the palette is matched: when most entries satisfy `f`, the mask is the
/// occupancy mask less the voxels of the entries that do not.
pub(super) fn mask_of(
    p: &Packed,
    f: impl Fn(MaterialId) -> bool,
    layers: std::ops::Range<usize>,
) -> BrickMask {
    let mut out = [0u64; 8];
    match p.words() {
        Some(w) => {
            let planes = p.class().planes();
            let pal = &w[8 + 8 * planes..];
            let cap = (pal.len() * 4).min(1 << planes);
            // Entries in use, and those satisfying `f`, one bit each.
            let (mut used, mut hits) = (0u32, 0u32);
            for j in 0..cap {
                let m = (pal[j / 4] >> (16 * (j % 4))) as MaterialId;
                if m == AIR {
                    break;
                }
                used |= 1 << j;
                hits |= u32::from(f(m)) << j;
            }
            let misses = used & !hits;
            let (pick, invert) = if hits.count_ones() <= misses.count_ones() {
                (hits, false)
            } else {
                (misses, true)
            };
            for z in layers {
                let mut acc = 0u64;
                let mut bits = pick;
                while bits != 0 {
                    let j = bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    let mut hit = w[z];
                    for k in 0..planes {
                        let plane = w[8 + 8 * k + z];
                        hit &= if j >> k & 1 == 1 { plane } else { !plane };
                    }
                    acc |= hit;
                }
                out[z] = if invert { w[z] & !acc } else { acc };
            }
        }
        None => {
            if let Some(ids) = p.wide_ids() {
                for z in layers {
                    for (b, &m) in ids[64 * z..64 * z + 64].iter().enumerate() {
                        if m != AIR && f(m) {
                            out[z] |= 1 << b;
                        }
                    }
                }
            }
        }
    }
    out
}

/// The opaque voxels of a neighbour brick in the z-layers `layers`: none for no brick (the
/// chunk's face).
fn opaque_of(b: Option<&Brick>, opaque: &[bool], layers: std::ops::Range<usize>) -> BrickMask {
    match b {
        None => [0; 8],
        Some(Brick::Uniform(m)) => [if opaque[usize::from(*m)] { !0 } else { 0 }; 8],
        Some(Brick::Full(p)) => mask_of(p, |m| opaque[usize::from(m)], layers),
    }
}

/// The exposed emissive voxels of `p` (the palette holding an emitter checked by the caller),
/// as [`exposed_emitters`] gives them. For each axis and direction, the voxel beside each
/// voxel is opaque in `side`: the brick's opaque mask shifted by one along the axis (x by 1
/// within a row of 8, y by 8 within a layer, z by a word), the brick's face layer filled from
/// the neighbour's facing layer. A sample is a set bit of `emissive & open`, visited in
/// ascending index (x fastest), with its six face bits.
pub(super) fn scan_masks<'a>(
    coord: ChunkCoord,
    slot: usize,
    p: &Packed,
    emissive: &[bool],
    opaque: &[bool],
    neighbour: impl Fn(usize, i32) -> Option<&'a Brick>,
) -> Vec<LightSample> {
    let em = mask_of(
        p,
        |m| emissive[usize::from(m)] && opaque[usize::from(m)],
        0..8,
    );
    if em == [0; 8] {
        return Vec::new();
    }
    let op = mask_of(p, |m| opaque[usize::from(m)], 0..8);
    // A neighbour on x or y faces the brick in every layer; one on z in its far layer only.
    let nb = |k: usize, dir: i32| {
        let layers = match (k, dir) {
            (2, -1) => 7..8,
            (2, _) => 0..1,
            _ => 0..8,
        };
        opaque_of(neighbour(k, dir), opaque, layers)
    };
    // side[2 k]: the voxel toward -k is opaque; side[2 k + 1]: toward +k.
    let mut side = [[0u64; 8]; 6];
    let (xl, xh) = (nb(0, -1), nb(0, 1));
    let (yl, yh) = (nb(1, -1), nb(1, 1));
    for z in 0..8 {
        side[0][z] = (op[z] << 1 & !COL_LOW) | (xl[z] & COL_HIGH) >> 7;
        side[1][z] = (op[z] >> 1 & !COL_HIGH) | (xh[z] & COL_LOW) << 7;
        side[2][z] = op[z] << 8 | yl[z] >> 56;
        side[3][z] = op[z] >> 8 | yh[z] << 56;
    }
    let (zl, zh) = (nb(2, -1), nb(2, 1));
    side[4][0] = zl[7];
    side[5][7] = zh[0];
    side[4][1..].copy_from_slice(&op[..7]);
    side[5][..7].copy_from_slice(&op[1..]);
    let b = brick_of_slot(slot);
    let base = [0, 1, 2].map(|k| coord.0[k] * CHUNK_EDGE_BRICKS * BRICK_EDGE + b[k] * BRICK_EDGE);
    let e = BRICK_EDGE as usize;
    let mut out = Vec::new();
    for z in 0..8 {
        let mut bits = em[z] & !(side.iter().fold(!0, |acc, s| acc & s[z]));
        while bits != 0 {
            let bit = bits.trailing_zeros() as usize;
            bits &= bits - 1;
            let faces = (0..6).fold(0u32, |f, d| f | u32::from(side[d][z] >> bit & 1 == 0) << d);
            let i = 64 * z + bit;
            let l = [i % e, i / e % e, i / (e * e)];
            out.push(LightSample {
                voxel: [0, 1, 2].map(|k| base[k] + l[k] as i32),
                material_faces: u32::from(p.get(i)) | faces << 16,
            });
        }
    }
    out
}

/// The per-voxel scan [`scan_masks`] replaces: the brick decoded, each emissive opaque voxel's
/// six neighbours read, across a brick face through `Brick::get` on the neighbour.
#[cfg(test)]
pub(super) fn scan_voxels<'a>(
    coord: ChunkCoord,
    slot: usize,
    p: &Packed,
    emissive: &[bool],
    opaque: &[bool],
    neighbour: impl Fn(usize, i32) -> Option<&'a Brick>,
) -> Vec<LightSample> {
    let mut v = [AIR; vxe_world::BRICK_VOXELS];
    p.decode(&mut v);
    let b = brick_of_slot(slot);
    let base = [0, 1, 2].map(|k| coord.0[k] * CHUNK_EDGE_BRICKS * BRICK_EDGE + b[k] * BRICK_EDGE);
    let e = BRICK_EDGE as usize;
    let beside = [0, 1, 2].map(|k| [neighbour(k, -1), neighbour(k, 1)]);
    let across = |l: [usize; 3], k: usize, dir: usize| match beside[k][dir] {
        None => true,
        Some(b) => {
            let mut q = l;
            q[k] = if dir == 0 { e - 1 } else { 0 };
            !opaque[usize::from(b.get(q[0] + e * (q[1] + e * q[2])))]
        }
    };
    let mut out = Vec::new();
    for (i, &m) in v.iter().enumerate() {
        if !emissive[usize::from(m)] || !opaque[usize::from(m)] {
            continue;
        }
        let l = [i % e, i / e % e, i / (e * e)];
        let mut faces = 0u32;
        for k in 0..3 {
            let stride = [1, e, e * e][k];
            let low = if l[k] == 0 {
                across(l, k, 0)
            } else {
                !opaque[usize::from(v[i - stride])]
            };
            let high = if l[k] == e - 1 {
                across(l, k, 1)
            } else {
                !opaque[usize::from(v[i + stride])]
            };
            faces |= u32::from(low) << (2 * k) | u32::from(high) << (2 * k + 1);
        }
        if faces != 0 {
            out.push(LightSample {
                voxel: [0, 1, 2].map(|k| base[k] + l[k] as i32),
                material_faces: u32::from(m) | faces << 16,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Materials 1 to 20 by `(id - 1) mod 4`: emissive opaque, emissive translucent, opaque,
    /// translucent; 0 is air.
    fn flags() -> (Vec<bool>, Vec<bool>) {
        let kind = |m: usize| (m + 3) % 4;
        let emissive = (0..=20).map(|m| m != 0 && kind(m) < 2).collect();
        let opaque = (0..=20).map(|m| m != 0 && kind(m) % 2 == 0).collect();
        (emissive, opaque)
    }

    /// A seeded brick of `n` materials drawn from 1 to 20 and air: noise, or a slab of one
    /// material through noise (runs of opaque voxels), as `Brick::from_fn` encodes it.
    fn brick(next: &mut impl FnMut(u64) -> u64, n: usize) -> Brick {
        let mats: Vec<MaterialId> = (0..n).map(|_| 1 + next(20) as MaterialId).collect();
        let air = next(4);
        let slab = next(3) == 0;
        let (axis, cut) = (next(3) as usize, next(9) as i32);
        Brick::from_fn(|l| {
            if slab && l[axis] < cut {
                return mats[0];
            }
            if next(4) < air {
                AIR
            } else {
                mats[next(n as u64) as usize]
            }
        })
    }

    /// 10 000 seeded `Full` bricks of 2 to 16 materials (and, one in eight, 17 to 20: the `Wide`
    /// class) drawn from emissive opaque, emissive translucent, opaque, translucent and air,
    /// each of the six neighbours a seeded `Full` brick, a `Uniform` brick or none, at seeded
    /// chunks and slots: the mask scan's samples equal the per-voxel scan's, element for
    /// element.
    #[test]
    fn the_mask_scan_finds_what_the_voxel_scan_finds() {
        let (emissive, opaque) = flags();
        let mut z = 0x0074_0003_u64;
        let mut next = |n: u64| {
            z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
            vxe_world::hash::mix64(z) % n
        };
        let mut checked = 0;
        let mut samples = 0;
        while checked < 10_000 {
            let n = if next(8) == 0 {
                17 + next(4) as usize
            } else {
                2 + next(15) as usize
            };
            let Brick::Full(p) = brick(&mut next, n) else {
                continue;
            };
            let beside: Vec<Option<Brick>> = (0..6)
                .map(|_| match next(3) {
                    0 => None,
                    1 => Some(Brick::Uniform(next(21) as MaterialId)),
                    _ => {
                        let n = 1 + next(16) as usize;
                        Some(brick(&mut next, n))
                    }
                })
                .collect();
            let neighbour = |k: usize, dir: i32| beside[2 * k + usize::from(dir > 0)].as_ref();
            let coord = ChunkCoord([0, 1, 2].map(|_| next(64) as i32 - 32));
            let slot = next(CHUNK_BRICKS as u64) as usize;
            let want = scan_voxels(coord, slot, &p, &emissive, &opaque, neighbour);
            let got = scan_masks(coord, slot, &p, &emissive, &opaque, neighbour);
            assert_eq!(got, want, "brick {checked}: {n} materials, {:?}", p.class());
            samples += want.len();
            checked += 1;
        }
        assert!(samples > 10_000, "{samples} samples");
    }
}
