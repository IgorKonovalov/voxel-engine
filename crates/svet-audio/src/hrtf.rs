//! Head-related impulse responses: the engine's pack format, loading, and nearest-direction
//! lookup (ADR-0006).
//!
//! Directions are unit vectors in the listener's frame: x front, y left, z up. A pack stores
//! azimuth (anticlockwise from the front, toward the left) and elevation (up) in degrees, as
//! SOFA does; [`Hrtf::load`] turns them into vectors.
//!
//! [`Hrtf::nearest`] answers through a 32 x 64 elevation-azimuth bin table. Each bin lists
//! every direction that can be the nearest to some point of the bin: with `c` the bin's centre,
//! `r` an upper bound on the angle from `c` to any point of the bin and `b` the direction
//! nearest to `c`, the nearest direction `n` to a point `p` of the bin satisfies
//! `angle(c, n) <= angle(c, p) + angle(p, n) <= r + angle(p, b) <= 2r + angle(c, b)`. The
//! candidates are scanned in ascending index with the same dot product and tie rule as a
//! brute-force search, so both return the same index.

use std::sync::{Arc, OnceLock};

use crate::{AudioError, RATE, dot};

/// Elevation bands and azimuth sectors of the lookup table.
const BIN_ELEVATIONS: usize = 32;
const BIN_AZIMUTHS: usize = 64;
/// Added to each bin's radius against rounding in the bound above, in radians.
const BIN_MARGIN: f32 = 0.01;
/// Header bytes: magic and four `u32`.
const HEADER: usize = 20;
/// Largest tap count a pack may carry.
const MAX_TAPS: usize = 1024;

/// The SADIE II subject D2 pack shipped with the crate (see `assets/README.md`).
static SADIE2_D2: &[u8] = include_bytes!("../assets/sadie2_d2_48k.hrir");

/// A loaded HRIR set.
pub struct Hrtf {
    taps: usize,
    /// Unit direction per measurement, listener frame.
    dirs: Vec<[f32; 3]>,
    /// Per direction, left then right, each `taps` long and stored time-reversed so a
    /// convolution is a forward dot product.
    fir: Vec<f32>,
    /// `BIN_ELEVATIONS * BIN_AZIMUTHS + 1` offsets into `candidates`.
    bin_start: Vec<u32>,
    candidates: Vec<u16>,
}

impl std::fmt::Debug for Hrtf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hrtf")
            .field("taps", &self.taps)
            .field("directions", &self.dirs.len())
            .finish_non_exhaustive()
    }
}

fn read_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn read_f32(bytes: &[u8], at: usize) -> f32 {
    f32::from_bits(read_u32(bytes, at))
}

/// Unit vector of `azimuth`, `elevation` in degrees.
fn direction(azimuth: f32, elevation: f32) -> [f32; 3] {
    let (sa, ca) = azimuth.to_radians().sin_cos();
    let (se, ce) = elevation.to_radians().sin_cos();
    [ce * ca, ce * sa, se]
}

/// The bin holding unit direction `d`.
fn bin_of(d: [f32; 3]) -> usize {
    let el = d[2].clamp(-1.0, 1.0).asin().to_degrees();
    let az = d[1].atan2(d[0]).to_degrees();
    let e = (((el + 90.0) / 180.0 * BIN_ELEVATIONS as f32) as usize).min(BIN_ELEVATIONS - 1);
    let a = (((az + 180.0) / 360.0 * BIN_AZIMUTHS as f32) as usize).min(BIN_AZIMUTHS - 1);
    e * BIN_AZIMUTHS + a
}

/// Index of the direction with the greatest dot product with `d` among `indices`, in any
/// order; the lowest index wins a tie.
fn best_of(dirs: &[[f32; 3]], d: [f32; 3], indices: impl Iterator<Item = usize>) -> usize {
    let mut best = (f32::NEG_INFINITY, usize::MAX);
    for i in indices {
        let p = dot(dirs[i], d);
        if p > best.0 || (p == best.0 && i < best.1) {
            best = (p, i);
        }
    }
    best.1
}

impl Hrtf {
    /// Parses and validates a pack.
    pub fn load(bytes: &[u8]) -> Result<Hrtf, AudioError> {
        if bytes.len() < HEADER {
            return Err(AudioError::Pack("shorter than the header"));
        }
        if &bytes[..4] != b"VXEH" {
            return Err(AudioError::Pack("wrong magic"));
        }
        if read_u32(bytes, 4) != 1 {
            return Err(AudioError::Pack("unknown version"));
        }
        if read_u32(bytes, 8) != RATE {
            return Err(AudioError::Pack("sample rate is not 48000"));
        }
        let taps = read_u32(bytes, 12) as usize;
        let n = read_u32(bytes, 16) as usize;
        if taps == 0 || taps > MAX_TAPS {
            return Err(AudioError::Pack("tap count out of range"));
        }
        if n == 0 || n > usize::from(u16::MAX) {
            return Err(AudioError::Pack("direction count out of range"));
        }
        if bytes.len() != HEADER + n * 8 + n * 2 * taps * 4 {
            return Err(AudioError::Pack("size does not match the header"));
        }
        let dirs: Vec<[f32; 3]> = (0..n)
            .map(|i| {
                let at = HEADER + 8 * i;
                direction(read_f32(bytes, at), read_f32(bytes, at + 4))
            })
            .collect();
        let taps_at = HEADER + n * 8;
        let mut fir = Vec::with_capacity(n * 2 * taps);
        for ear in 0..n * 2 {
            let at = taps_at + ear * taps * 4;
            fir.extend((0..taps).rev().map(|k| read_f32(bytes, at + 4 * k)));
        }
        if fir.iter().any(|t| !t.is_finite()) {
            return Err(AudioError::Pack("a tap is not finite"));
        }
        let (bin_start, candidates) = build_bins(&dirs);
        Ok(Hrtf {
            taps,
            dirs,
            fir,
            bin_start,
            candidates,
        })
    }

    /// The SADIE II subject D2 pack shipped with the crate, loaded once.
    pub fn sadie2_d2() -> Arc<Hrtf> {
        static LOADED: OnceLock<Arc<Hrtf>> = OnceLock::new();
        Arc::clone(LOADED.get_or_init(|| {
            Arc::new(Hrtf::load(SADIE2_D2).expect("the shipped HRIR pack is valid"))
        }))
    }

    pub fn taps(&self) -> usize {
        self.taps
    }

    pub fn len(&self) -> usize {
        self.dirs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.dirs.is_empty()
    }

    /// Unit direction of measurement `i`, listener frame (x front, y left, z up).
    pub fn direction(&self, i: usize) -> [f32; 3] {
        self.dirs[i]
    }

    /// Bytes the loaded set holds on the heap.
    pub fn heap_bytes(&self) -> usize {
        self.dirs.len() * 12
            + self.fir.len() * 4
            + self.bin_start.len() * 4
            + self.candidates.len() * 2
    }

    /// Left and right impulse responses of measurement `i`, in time order.
    pub fn impulse(&self, i: usize) -> [Vec<f32>; 2] {
        [0, 1].map(|ear| {
            let mut h = self.reversed(i, ear).to_vec();
            h.reverse();
            h
        })
    }

    /// The time-reversed FIR of measurement `i`, ear 0 (left) or 1 (right).
    pub(crate) fn reversed(&self, i: usize, ear: usize) -> &[f32] {
        let at = (2 * i + ear) * self.taps;
        &self.fir[at..at + self.taps]
    }

    /// Index of the measured direction with the greatest dot product with unit vector `d`
    /// (listener frame), through the bin table.
    pub fn nearest(&self, d: [f32; 3]) -> usize {
        let b = bin_of(d);
        let (s, e) = (self.bin_start[b] as usize, self.bin_start[b + 1] as usize);
        best_of(
            &self.dirs,
            d,
            self.candidates[s..e].iter().map(|&i| usize::from(i)),
        )
    }

    /// As [`Hrtf::nearest`], by scanning every direction.
    pub fn nearest_brute_force(&self, d: [f32; 3]) -> usize {
        best_of(&self.dirs, d, 0..self.dirs.len())
    }
}

/// The candidate lists of every bin (see the module comment).
///
/// Every scan is narrowed to the directions whose elevation lies within the angle being
/// searched: a direction `delta` away in elevation is at least `delta` away in angle.
fn build_bins(dirs: &[[f32; 3]]) -> (Vec<u32>, Vec<u16>) {
    let mut start = Vec::with_capacity(BIN_ELEVATIONS * BIN_AZIMUTHS + 1);
    let mut candidates = Vec::new();
    let el_step = 180.0 / BIN_ELEVATIONS as f32;
    let az_step = 360.0 / BIN_AZIMUTHS as f32;
    let mut by_z: Vec<usize> = (0..dirs.len()).collect();
    by_z.sort_by(|&a, &b| dirs[a][2].total_cmp(&dirs[b][2]));
    let zs: Vec<f32> = by_z.iter().map(|&i| dirs[i][2]).collect();
    // The directions within `w` radians of elevation `el` (radians), with a little slack for
    // rounding in `sin`, in elevation order.
    let window = |el: f32, w: f32| -> &[usize] {
        let half = std::f32::consts::FRAC_PI_2;
        let lo = (el - w - 1e-3).max(-half).sin();
        let hi = (el + w + 1e-3).min(half).sin();
        let (s, e) = (
            zs.partition_point(|&z| z < lo),
            zs.partition_point(|&z| z <= hi),
        );
        &by_z[s..e]
    };
    for e in 0..BIN_ELEVATIONS {
        let e0 = -90.0 + e as f32 * el_step;
        let el_c = (e0 + el_step / 2.0).to_radians();
        // The band's radius (the same for every azimuth sector), bounded by sampling the
        // bin finely; sampling misses at most one sample step's angle.
        const S: usize = 16;
        let c0 = direction(az_step / 2.0, e0 + el_step / 2.0);
        let mut r = 0.0f32;
        for i in 0..=S {
            for j in 0..=S {
                let p = direction(
                    az_step * j as f32 / S as f32,
                    e0 + el_step * i as f32 / S as f32,
                );
                r = r.max(dot(c0, p).clamp(-1.0, 1.0).acos());
            }
        }
        r += el_step.max(az_step).to_radians() / S as f32 + BIN_MARGIN;
        for a in 0..BIN_AZIMUTHS {
            start.push(candidates.len() as u32);
            let a0 = -180.0 + a as f32 * az_step;
            let c = direction(a0 + az_step / 2.0, e0 + el_step / 2.0);
            // The direction nearest to the centre: found within 0.3 rad when one lies there,
            // since everything outside the window is farther than that.
            const W: f32 = 0.3;
            let near = window(el_c, W);
            let mut b = best_of(dirs, c, near.iter().copied());
            if near.is_empty() || dot(c, dirs[b]).clamp(-1.0, 1.0).acos() > W {
                b = best_of(dirs, c, 0..dirs.len());
            }
            let limit = dot(c, dirs[b]).clamp(-1.0, 1.0).acos() + 2.0 * r;
            if limit >= std::f32::consts::PI {
                candidates.extend((0..dirs.len()).map(|i| i as u16));
                continue;
            }
            let cos_limit = limit.cos();
            // `nearest` scans candidates in ascending index, as the brute force does.
            let first = candidates.len();
            candidates.extend(
                window(el_c, limit)
                    .iter()
                    .filter(|&&i| dot(c, dirs[i]) >= cos_limit)
                    .map(|&i| i as u16),
            );
            candidates[first..].sort_unstable();
        }
    }
    start.push(candidates.len() as u32);
    (start, candidates)
}
