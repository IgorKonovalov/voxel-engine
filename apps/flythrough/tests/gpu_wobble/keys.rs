//! Plan 0104 Phase 4: a clip's track rendered again with each frame's world units written
//! beside it, for the measures (`measure.rs`) to score on the decoded frames the owner judged.
//! Nothing is asserted.
//!
//! One record a frame in `<stem>.keys`, the display sampled on a stride of [`STRIDE`] pixels
//! (sample `(sx, sy)` is display pixel `(STRIDE * sx, STRIDE * sy)`), three planes in sample
//! order, rows top to bottom:
//! - `u32` the sample's unit id (0 for a miss), the display pixel's internal pixel's hit;
//! - `u8` flags: the key slot's source (`support::defects::key_slot`, bits 0..2), the sun
//!   state (bits 3..4: 0 sunlit, 1 shadowed, 2 turned away, 3 a miss) and a leaf bit (5), all
//!   of the lighting pixel the display pixel lies in;
//! - `u8` the key slot's value, 0..255.
//!
//! The unit: a face where it covers 16 display pixels, else the visible faces of one normal
//! inside a world cube of `2^k` voxels, the smallest `k` at which they cover 16. Coverage is
//! read from the hit: a voxel face at ray distance `t` covers `(ppv * 2^k)^2 * |cos|` display
//! pixels, `ppv` the display's pixels a voxel at `t` (its height over the view's vertical
//! extent) and `cos` the view ray against the face's normal. `k` is read a pixel a frame, so a
//! surface whose distance or angle crosses a step changes unit there. The face is the hit
//! record's entry axis (`Frame::hit_records`' bits 3..5) with the sign toward the eye; the
//! voxel is the hit nudged 0.01 along the ray into the solid. Ids are given in the order units
//! are first seen and hold for the whole clip: an id is one unit fixed in the world.

use std::collections::HashMap;
use std::io::Write;

use super::*;

/// The display's sampling stride, in pixels.
pub(super) const STRIDE: u32 = 2;

/// The display pixels a unit covers at least.
const COVER: f32 = 16.0;

/// The writer of one clip's `.keys`.
pub(super) struct Keys {
    out: std::io::BufWriter<std::fs::File>,
    ids: HashMap<u64, u32>,
    window: [u32; 2],
    /// Samples written a frame, and the time spent.
    pub(super) samples: usize,
    pub(super) secs: f64,
}

impl Keys {
    pub(super) fn create(path: &Path, window: [u32; 2]) -> Self {
        let f = std::fs::File::create(path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
        let samples = (window[0].div_ceil(STRIDE) * window[1].div_ceil(STRIDE)) as usize;
        Self {
            out: std::io::BufWriter::with_capacity(1 << 22, f),
            ids: HashMap::new(),
            window,
            samples,
            secs: 0.0,
        }
    }

    /// Units seen so far.
    pub(super) fn units(&self) -> usize {
        self.ids.len()
    }

    /// The last encode's frame from `cam`.
    pub(super) fn frame(&mut self, rig: &Rig, f: &Frame, cam: &Camera) {
        use flythrough::outdoor::{LEAF, LEAF_2, LEAF_3};
        use svet_render::debug::{read_rgba8, read_rgba16f, read_rgba16u, read_rgba32f};
        let t0 = std::time::Instant::now();
        let (d, q) = (&rig.device, &rig.queue);
        let hits = read_rgba32f(d, q, f.hit_positions());
        let recs = read_rgba16u(d, q, f.hit_records());
        let pair = read_rgba16f(d, q, f.visibility_pair());
        let vis = read_rgba8(d, q, f.visibility_samples());
        let [iw, ih] = [f.hit_positions().width(), f.hit_positions().height()];
        let [lw, lh] = [f.visibility_pair().width(), f.visibility_pair().height()];
        let lhits = read_rgba32f(d, q, f.light_hit_positions());
        let [dw, dh] = self.window;
        let eye = cam.position.to_array();
        let ppv_k = dh as f32 / (2.0 * (cam.vfov * 0.5).tan());
        let (sw, sh) = (dw.div_ceil(STRIDE), dh.div_ceil(STRIDE));
        let mut ids = Vec::with_capacity(self.samples);
        let mut flags = Vec::with_capacity(self.samples);
        let mut vals = Vec::with_capacity(self.samples);
        let leaf = |h: &[f32; 4]| -> bool {
            let dv = [h[0] - eye[0], h[1] - eye[1], h[2] - eye[2]];
            let t = (dv[0] * dv[0] + dv[1] * dv[1] + dv[2] * dv[2])
                .sqrt()
                .max(1e-3);
            let v =
                svet_world::VoxelCoord([0, 1, 2].map(|k| (h[k] + dv[k] / t * 0.01).floor() as i32));
            rig.world
                .material_at(v)
                .is_some_and(|m| [LEAF, LEAF_2, LEAF_3].contains(&m))
        };
        for sy in 0..sh {
            let y = sy * STRIDE;
            let iy = (((y as f32 + 0.5) * ih as f32 / dh as f32) as u32).min(ih - 1);
            let ly = (((y as f32 + 0.5) * lh as f32 / dh as f32) as u32).min(lh - 1);
            for sx in 0..sw {
                let x = sx * STRIDE;
                let ix = (((x as f32 + 0.5) * iw as f32 / dw as f32) as u32).min(iw - 1);
                let lx = (((x as f32 + 0.5) * lw as f32 / dw as f32) as u32).min(lw - 1);
                let g = (ly * lw + lx) as usize;
                let lh_ = lhits[g];
                let (val, src) = support::defects::key_slot(pair[g][1]);
                let state: u8 = if lh_[3] >= 1e30 || !lh_[3].is_finite() {
                    3
                } else if vis[4 * g + 2] == 0 {
                    2
                } else if val >= 0.5 {
                    0
                } else {
                    1
                };
                let lf = state != 3 && leaf(&lh_);
                flags.push(src as u8 | (state << 3) | (u8::from(lf) << 5));
                vals.push((val * 255.0).round() as u8);
                let p = (iy * iw + ix) as usize;
                ids.push(self.unit(&hits[p], recs[p][3], eye, ppv_k));
            }
        }
        for v in &ids {
            self.out.write_all(&v.to_le_bytes()).expect("keys");
        }
        self.out.write_all(&flags).expect("keys");
        self.out.write_all(&vals).expect("keys");
        self.secs += t0.elapsed().as_secs_f64();
    }

    /// The unit id of hit `h` with record flags `fl`.
    fn unit(&mut self, h: &[f32; 4], fl: u16, eye: [f32; 3], ppv_k: f32) -> u32 {
        if h[3] >= 1e30 || !h[3].is_finite() || fl & 1 == 0 {
            return 0;
        }
        let dv = [h[0] - eye[0], h[1] - eye[1], h[2] - eye[2]];
        let t = (dv[0] * dv[0] + dv[1] * dv[1] + dv[2] * dv[2])
            .sqrt()
            .max(1e-3);
        let dir = dv.map(|c| c / t);
        let mut axis = usize::from((fl >> 3) & 7);
        if axis == 0 || axis > 3 {
            // No entry face: the ray's dominant axis.
            axis = 1
                + (0..3)
                    .max_by(|&a, &b| dir[a].abs().total_cmp(&dir[b].abs()))
                    .expect("an axis");
        }
        let a = axis - 1;
        let cos = dir[a].abs().max(0.05);
        let ppv = ppv_k / t;
        let one = ppv * ppv * cos;
        let mut k = 0u32;
        while k < 12 && one * 4f32.powi(k as i32) < COVER {
            k += 1;
        }
        let v = [0, 1, 2].map(|c| (h[c] + dir[c] * 0.01).floor() as i32);
        let cube = v.map(|c| c >> k);
        let sign = u64::from(dir[a] < 0.0);
        let enc = |c: i32| u64::from((c + (1 << 18)) as u32 & 0x7_ffff);
        let key = u64::from(k)
            | (a as u64) << 4
            | sign << 6
            | enc(cube[0]) << 7
            | enc(cube[1]) << 26
            | enc(cube[2]) << 45;
        let n = self.ids.len() as u32 + 1;
        *self.ids.entry(key).or_insert(n)
    }

    pub(super) fn finish(mut self) {
        self.out.flush().expect("keys");
    }
}
