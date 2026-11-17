//! The store's record words and their codecs: tone levels, the RGB9E5 pack, the brick and
//! block words and flags. Pure CPU.

use super::*;

/// The stop's standard error, in display levels.
pub(crate) const BAKE_SE: f32 = 0.5;

/// The exposure the stop reads the display at (the owner's windows' fixed exposure).
pub(crate) const BAKE_EXPOSURE: f32 = 8.0;

/// The word of the pool's chunk table.
pub(crate) const STORE_TABLE: u64 = 16;

/// A chunk table word's flag: the chunk's block is the live queue's coarse pass, refined later.
/// The block's first word is the rest (a pool is under 2^31 words).
pub(crate) const TABLE_COARSE: u32 = 1 << 31;

/// The frame's read counters' word that counts the lighting pixels that read a coarse block.
pub const STORE_COARSE_READS: u64 = 2;

/// The words of a block's directory.
pub(crate) const DIRECTORY_WORDS: u64 = CHUNK_BRICKS as u64;

/// The bytes of one face in the working buffer (`BakeFace` in `bake.wgsl`).
pub(crate) const FACE_BYTES: u64 = 80;

/// The display's level, 0 to 255, of luminance `l` at unit exposure: `tone.wgsl`'s ACES and 2.2
/// encode at [`BAKE_EXPOSURE`], unrounded.
pub fn tone_level(l: f64) -> f64 {
    let x = (l * f64::from(BAKE_EXPOSURE)).max(0.0);
    let m = ((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14)).clamp(0.0, 1.0);
    m.powf(1.0 / 2.2) * 255.0
}

/// The standard error, in display levels, of the mean of `n` samples whose luminances sum to `sum`
/// and whose squares sum to `sum2`.
pub(crate) fn se_levels(n: u64, sum: f64, sum2: f64) -> f64 {
    if n == 0 {
        return f64::INFINITY;
    }
    let n = n as f64;
    let mean = sum / n;
    let se = ((sum2 / n - mean * mean).max(0.0) / n).sqrt();
    0.5 * (tone_level(mean + se) - tone_level((mean - se).max(0.0)))
}

/// `c` as an `rgb9e5` word: 9-bit mantissas over a shared exponent, red in the low bits, the
/// exponent (bias 15) in the top five.
pub(crate) fn rgb9e5_pack(c: [f32; 3]) -> u32 {
    const MAX: f32 = 65408.0;
    let c = c.map(|v| if v.is_nan() { 0.0 } else { v.clamp(0.0, MAX) });
    let top = c[0].max(c[1]).max(c[2]);
    if top <= 0.0 {
        return 0;
    }
    let e2 = (top.to_bits() >> 23) as i32 - 127;
    let mut exp = e2.max(-16) + 1 + 15;
    let mut scale = 2f64.powi(exp - 24);
    if (f64::from(top) / scale + 0.5).floor() >= 512.0 {
        exp += 1;
        scale *= 2.0;
    }
    let m = c.map(|v| (f64::from(v) / scale + 0.5).floor() as u32);
    m[0] | m[1] << 9 | m[2] << 18 | (exp as u32) << 27
}

/// The value of an `rgb9e5` word.
pub fn rgb9e5_unpack(w: u32) -> [f32; 3] {
    let scale = 2f64.powi((w >> 27) as i32 - 24);
    [w & 511, (w >> 9) & 511, (w >> 18) & 511].map(|m| (f64::from(m) * scale) as f32)
}

/// Words of a brick-level cell's record: six direction values in [`rgb9e5_pack`], face order, then
/// `material | direction mask << 16`, the mask's bit `face` set where the cell has a face
/// of that direction, then two words of sky visibility (ADR-0070): direction `face`'s `u8` at
/// byte `face % 4` of word `7 + face / 4`, and [`VIS_BAKED`] in the second. A brick's record
/// is its cells' records in cell order ([`record_words`]): one cell of 8 voxels, or eight of 4
/// (`x + 2 y + 4 z`).
pub const BRICK_WORDS: usize = 9;

/// A record's first sky visibility word.
pub(crate) const VIS_WORD: usize = 7;

/// The second sky visibility word's flag: the record's visibilities were baked under a sky.
pub(crate) const VIS_BAKED: u32 = 1 << 16;

/// The second sky visibility word's flag: a dig's ring holds the brick (ADR-0070), so its
/// visibilities predate the edit until its re-bake lands, whose record clears it. The light
/// pass casts the sky rays at a face of a record with it.
pub(crate) const SKY_STALE: u32 = 1 << 17;

/// Sky rays a brick direction draws at most in the bake (ADR-0070's `SKY_BAKE_RAYS`, a guess:
/// the binomial standard deviation at a half open sky is 1/32).
pub const SKY_BAKE_RAYS: u32 = 256;

/// Direction `face`'s sky visibility in [0, 1] of record `r`, `None` where it was baked with no
/// sky or lacks the direction.
pub fn record_visibility(r: &[u32], face: usize) -> Option<f32> {
    if r[VIS_WORD + 1] & VIS_BAKED == 0 || (r[6] >> (16 + face)) & 1 == 0 {
        return None;
    }
    Some(((r[VIS_WORD + face / 4] >> (8 * (face % 4))) & 255) as f32 / 255.0)
}

/// The two visibility words of six fractions in [0, 1] (rounded to `u8`), with [`VIS_BAKED`].
pub(crate) fn visibility_words(v: [f32; 6]) -> [u32; 2] {
    let b = v.map(|x| (x.clamp(0.0, 1.0) * 255.0).round() as u32);
    [
        b[0] | b[1] << 8 | b[2] << 16 | b[3] << 24,
        b[4] | b[5] << 8 | VIS_BAKED,
    ]
}

/// The store header's flags word: bit 1 ([`FLAG_CELL4`]) where the records' cells are 4 voxels.
pub(crate) const STORE_FLAGS: u64 = 12;

/// The brick records hold eight cells of 4 voxels.
pub(crate) const FLAG_CELL4: u32 = 2;

/// The brick level's cell by default, in voxels: the brick.
pub const BRICK_CELL: u32 = 8;

/// Words of a brick's record of cells of `cell` voxels: [`BRICK_WORDS`] a cell.
pub(crate) fn record_words(cell: u32) -> usize {
    BRICK_WORDS * (8 / cell).pow(3) as usize
}

/// A block's word holding its light summary (ADR-0053): one record in the cell record's format,
/// just past the directory, so the chunk table's word reaches it with no second read.
pub const SUMMARY_AT: u64 = DIRECTORY_WORDS;

/// A block's words before its first brick record: the directory, then the summary.
pub(crate) const BLOCK_HEAD_WORDS: u64 = DIRECTORY_WORDS + BRICK_WORDS as u64;

/// The flags word's bits 2 and 3: the missing-light mode ([`MissingLight::bits`]).
pub(crate) const FLAG_MISSING_SHIFT: u32 = 2;

/// A brick's record from its faces' states, one [`BRICK_WORDS`] record a cell of `cell` voxels in
/// cell order: each direction the mean of the cell's baked faces' values, the mask of the
/// directions it has, and the material of the sample that read the largest luminance over the
/// cell; under `sky` each direction's visibility the mean of its faces' open fractions.
pub(super) fn brick_record(states: &[FaceState], clamp: bool, cell: u32, sky: bool) -> Vec<u32> {
    let side = (8 / cell) as usize;
    let cells = side * side * side;
    let mut sums = vec![[[0f64; 3]; 6]; cells];
    let mut counts = vec![[0u32; 6]; cells];
    let mut best = vec![(-1f32, 0u16); cells];
    let mut vis = vec![[(0f64, 0u32); 6]; cells];
    for s in states {
        let l = s.voxel.map(|x| (x.rem_euclid(8) as u32 / cell) as usize);
        let c = l[0] + side * (l[1] + side * l[2]);
        let f = usize::from(s.face);
        if s.n == 0 {
            continue;
        }
        if s.best > best[c].0 {
            best[c] = (s.best, s.material);
        }
        let v = s.mean(s.kept(clamp));
        for k in 0..3 {
            sums[c][f][k] += f64::from(v[k]);
        }
        counts[c][f] += 1;
        if let Some(open) = s.sky_fraction() {
            vis[c][f].0 += open;
            vis[c][f].1 += 1;
        }
    }
    let mut out = vec![0u32; BRICK_WORDS * cells];
    for c in 0..cells {
        let r = &mut out[BRICK_WORDS * c..BRICK_WORDS * (c + 1)];
        let mut mask = 0u32;
        for f in 0..6 {
            if counts[c][f] > 0 {
                let n = f64::from(counts[c][f]);
                r[f] = rgb9e5_pack(sums[c][f].map(|x| (x / n) as f32));
                mask |= 1 << f;
            }
        }
        r[6] = u32::from(best[c].1) | mask << 16;
        if sky {
            let v = vis[c].map(|(sum, n)| {
                if n > 0 {
                    (sum / f64::from(n)) as f32
                } else {
                    0.0
                }
            });
            r[VIS_WORD..VIS_WORD + 2].copy_from_slice(&visibility_words(v));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb9e5_holds_a_value_within_its_step() {
        for c in [
            [1.0f32, 0.5, 0.25],
            [0.003, 0.002, 0.001],
            [1234.5, 3.0, 800.0],
            [1e-6, 0.0, 0.0],
        ] {
            let d = rgb9e5_unpack(rgb9e5_pack(c));
            let top = c[0].max(c[1]).max(c[2]);
            for k in 0..3 {
                // Half a step: a 9-bit mantissa, over the format's floor of 2^-24.
                let half_step = (top / 256.0).max(2f32.powi(-25));
                assert!((d[k] - c[k]).abs() <= half_step, "{c:?} read {d:?}");
            }
        }
        assert_eq!(rgb9e5_pack([0.0; 3]), 0);
        assert_eq!(rgb9e5_unpack(0), [0.0; 3]);
    }

    #[test]
    fn the_visibility_words_round_trip_by_direction() {
        let mut r = [0u32; BRICK_WORDS];
        r[6] = 0b11_1111 << 16;
        let v = [0.0, 0.5, 1.0, 0.25, 0.75, 0.2];
        r[VIS_WORD..VIS_WORD + 2].copy_from_slice(&visibility_words(v));
        for (f, x) in v.iter().enumerate() {
            let got = record_visibility(&r, f).expect("baked with a sky");
            assert!((got - x).abs() <= 0.5 / 255.0 + 1e-6, "{f}: {got} {x}");
        }
        r[6] = 1 << 16;
        assert_eq!(
            record_visibility(&r, 1),
            None,
            "the mask lacks the direction"
        );
        assert_eq!(
            record_visibility(&[0; BRICK_WORDS], 0),
            None,
            "no sky baked"
        );
    }

    #[test]
    fn the_standard_error_reads_zero_for_a_constant_and_falls_with_the_count() {
        assert_eq!(se_levels(1024, 0.0, 0.0), 0.0);
        let constant = se_levels(1024, 1024.0 * 0.01, 1024.0 * 0.0001);
        assert!(constant.abs() < 1e-3, "{constant}");
        let at = |n: u64| se_levels(n, n as f64 * 0.01, n as f64 * 0.0002);
        assert!(at(4096) < at(1024) * 0.51, "{} {}", at(4096), at(1024));
    }
}
