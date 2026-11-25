//! Plan 0100 Phase 5's reading pass: the key light's visibility of a cube face on the CPU, for
//! the candidates of ADR-0084 under cover and their reference. Not a frame path: a slow reading
//! over the world's voxels and the GPU's column map and store read back.
//!
//! - [`Solid`]: the window's opaque voxels (every row but air and the translucent ones; the
//!   default leaf shadow stops a ray at a leaf voxel) and each voxel column's top, with a
//!   column top per 8 x 8 columns for skipping open air.
//! - [`Solid::trace`]: a voxel walk from a point along a direction until an opaque voxel
//!   (shadowed) or the window's side or top (lit: an absent chunk is sky to a lighting ray),
//!   with no cap; with `approach` the closest-approach score of walk.wgsl's `approach_probe`
//!   (probes every 2 voxels of the ray from 4 on, 2 voxels to either side across its dominant
//!   axis, the gap to the probe's 2-voxel block over `t tan r`).
//! - [`Columns`]: the column map read back and pair.wgsl's `column_key_visibility` on it.
//! - [`Records`]: store blocks read back and temporal.wgsl's key reads (`store_key_cell`,
//!   `store_key_direction`, `store_key_visibility`) at cells of 8 voxels.

use std::collections::HashMap;

use svet_world::{AIR, ChunkCoord, MaterialFlags, World};

/// The window's opaque voxels, a bit each, and each voxel column's highest opaque `z`.
pub struct Solid {
    min: [i32; 3],
    dims: [i32; 3],
    bits: Vec<u64>,
    top: Vec<i32>,
    /// Per 8 x 8 voxel columns, the highest opaque `z` among them.
    ctop: Vec<i32>,
    /// The highest opaque `z` in the window.
    pub max_top: i32,
}

/// The coarse top's columns an edge.
const CT: i32 = 8;

impl Solid {
    /// `world`'s resident chunks over its extent.
    pub fn of(world: &World) -> Self {
        let e = world.extent();
        let min = e.min.0.map(|c| c * 64);
        let dims = e.size().map(|s| s * 64);
        let n = dims.iter().map(|&d| d as usize).product::<usize>();
        let mut bits = vec![0u64; n.div_ceil(64)];
        let mut top = vec![i32::MIN; (dims[0] * dims[1]) as usize];
        let table = world.materials();
        let opaque = |m: svet_world::MaterialId| {
            m != AIR
                && table
                    .get(m)
                    .is_none_or(|r| r.flags.0 & MaterialFlags::TRANSLUCENT.0 == 0)
        };
        for coord in world.store().coords() {
            let Some(chunk) = world.store().get(coord) else {
                continue;
            };
            let chunk = chunk.read().expect("the chunk's lock");
            for slot in 0..512usize {
                let brick = chunk.brick(slot);
                let b = svet_world::brick_of_slot(slot);
                let uniform = (!brick.is_full()).then(|| brick.get(0));
                if uniform.is_some_and(|m| !opaque(m)) {
                    continue;
                }
                for i in 0..512usize {
                    if uniform.is_none() && !opaque(brick.get(i)) {
                        continue;
                    }
                    let l = [i % 8, (i / 8) % 8, i / 64].map(|v| v as i32);
                    let v = [0, 1, 2].map(|k| coord.0[k] * 64 + b[k] * 8 + l[k] - min[k]);
                    if (0..3).any(|k| v[k] < 0 || v[k] >= dims[k]) {
                        continue;
                    }
                    let idx = ((v[2] * dims[1] + v[1]) * dims[0] + v[0]) as usize;
                    bits[idx / 64] |= 1 << (idx % 64);
                    let c = (v[1] * dims[0] + v[0]) as usize;
                    top[c] = top[c].max(v[2] + min[2]);
                }
            }
        }
        let cw = [dims[0] / CT, dims[1] / CT];
        let mut ctop = vec![i32::MIN; (cw[0] * cw[1]) as usize];
        for y in 0..dims[1] {
            for x in 0..dims[0] {
                let c = ((y / CT) * cw[0] + x / CT) as usize;
                ctop[c] = ctop[c].max(top[(y * dims[0] + x) as usize]);
            }
        }
        let max_top = top.iter().copied().max().unwrap_or(i32::MIN);
        Self {
            min,
            dims,
            bits,
            top,
            ctop,
            max_top,
        }
    }

    fn local(&self, v: [i32; 3]) -> Option<[i32; 3]> {
        let l = [0, 1, 2].map(|k| v[k] - self.min[k]);
        (0..3)
            .all(|k| l[k] >= 0 && l[k] < self.dims[k])
            .then_some(l)
    }

    /// Whether voxel `v` is opaque (outside the window: not).
    pub fn solid(&self, v: [i32; 3]) -> bool {
        self.local(v).is_some_and(|l| {
            let idx = ((l[2] * self.dims[1] + l[1]) * self.dims[0] + l[0]) as usize;
            self.bits[idx / 64] >> (idx % 64) & 1 != 0
        })
    }

    /// Voxel column `(x, y)`'s highest opaque `z` (`i32::MIN` for none or outside).
    pub fn top(&self, x: i32, y: i32) -> i32 {
        let (lx, ly) = (x - self.min[0], y - self.min[1]);
        if lx < 0 || ly < 0 || lx >= self.dims[0] || ly >= self.dims[1] {
            return i32::MIN;
        }
        self.top[(ly * self.dims[0] + lx) as usize]
    }

    fn ctop_at(&self, l: [i32; 3]) -> i32 {
        self.ctop[((l[1] / CT) * (self.dims[0] / CT) + l[0] / CT) as usize]
    }

    /// The walk from `o` along unit `d` (`d.z` over 0): `None` where it meets an opaque voxel,
    /// else `Some` of the closest-approach score (1 without `approach`; `tan_r` the disc's
    /// half angle's tangent). Without `approach`, a stretch whose 8 x 8 columns' top is under
    /// the ray is skipped whole.
    pub fn trace(&self, o: [f32; 3], d: [f32; 3], approach: bool, tan_r: f32) -> Option<f32> {
        self.walk(o, d, approach, tan_r).ok()
    }

    /// [`Solid::trace`] with the opaque voxel it met and the ray's parameter there.
    pub fn walk(
        &self,
        o: [f32; 3],
        d: [f32; 3],
        approach: bool,
        tan_r: f32,
    ) -> Result<f32, (f32, [i32; 3])> {
        let mut amin = 1.0f32;
        let mut next_probe = 4.0f32;
        let mut t = 0.0f32;
        'restart: loop {
            let p = [0, 1, 2].map(|k| o[k] + d[k] * t);
            let mut v = p.map(|x| x.floor() as i32);
            let step = d.map(|x| if x > 0.0 { 1 } else { -1 });
            let delta = d.map(|x| {
                if x.abs() > 1e-9 {
                    1.0 / x.abs()
                } else {
                    f32::INFINITY
                }
            });
            let mut tmax = [0, 1, 2].map(|k| {
                if d[k].abs() <= 1e-9 {
                    f32::INFINITY
                } else {
                    let edge = if d[k] > 0.0 { v[k] + 1 } else { v[k] } as f32;
                    t + (edge - p[k]) / d[k]
                }
            });
            loop {
                let Some(l) = self.local(v) else {
                    return Ok(amin);
                };
                if v[2] > self.max_top {
                    return Ok(amin);
                }
                if !approach && v[2] > self.ctop_at(l) {
                    // Every voxel of these 8 x 8 columns the ray still crosses lies over
                    // their top: leave them across the nearer side wall.
                    let c0 = [l[0] / CT * CT + self.min[0], l[1] / CT * CT + self.min[1]];
                    let mut t_out = f32::INFINITY;
                    for k in 0..2 {
                        if d[k].abs() > 1e-9 {
                            let wall = if d[k] > 0.0 { c0[k] + CT } else { c0[k] } as f32;
                            t_out = t_out.min((wall - o[k]) / d[k]);
                        }
                    }
                    if t_out.is_finite() && t_out > t {
                        t = t_out + 1e-3;
                        continue 'restart;
                    }
                }
                let bits = {
                    let idx = ((l[2] * self.dims[1] + l[1]) * self.dims[0] + l[0]) as usize;
                    self.bits[idx / 64] >> (idx % 64) & 1
                };
                if bits != 0 {
                    return Err((t, v));
                }
                let k = if tmax[0] <= tmax[1] && tmax[0] <= tmax[2] {
                    0
                } else if tmax[1] <= tmax[2] {
                    1
                } else {
                    2
                };
                let t1 = tmax[k];
                if approach {
                    while next_probe <= t1 {
                        if next_probe >= t {
                            amin = amin.min(self.probe(o, d, next_probe, tan_r));
                        }
                        next_probe += 2.0;
                    }
                }
                t = t1;
                v[k] += step[k];
                tmax[k] += delta[k];
            }
        }
    }

    /// walk.wgsl's `approach_probe` at the ray's point at `t`.
    fn probe(&self, o: [f32; 3], d: [f32; 3], t: f32, tan_r: f32) -> f32 {
        let q = [0, 1, 2].map(|k| o[k] + d[k] * t);
        let v = q.map(|x| x.floor() as i32);
        let a = d.map(f32::abs);
        let k = if a[0] >= a[1] && a[0] >= a[2] {
            0
        } else if a[1] >= a[2] {
            1
        } else {
            2
        };
        let mut best = 1.0f32;
        for axis in [(k + 1) % 3, (k + 2) % 3] {
            for s in [2, -2] {
                let mut p = v;
                p[axis] += s;
                if !self.solid(p) {
                    continue;
                }
                let lo = p.map(|x| (x & -2) as f32);
                let gap: f32 = (0..3)
                    .map(|j| {
                        let g = (lo[j] - q[j]).max(0.0) + (q[j] - (lo[j] + 2.0)).max(0.0);
                        g * g
                    })
                    .sum::<f32>()
                    .sqrt();
                best = best.min(gap / (t * tan_r));
            }
        }
        best
    }
}

/// light.wgsl's `disc_direction`: a unit direction uniform over the solid angle of the disc of
/// angular radius `r` about unit `l`, from `u` and `v` in [0, 1).
pub fn disc_direction(l: [f32; 3], r: f32, u: f32, v: f32) -> [f32; 3] {
    let h = (0.5 * r).sin();
    let omc = 2.0 * h * h * u;
    let sin_t = (omc * (2.0 - omc)).max(0.0).sqrt();
    let phi = 2.0 * std::f32::consts::PI * v;
    let a = if l[0].abs() > 0.9 {
        [0.0, 1.0, 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let t = normalize(cross(a, l));
    let b = cross(l, t);
    normalize([0, 1, 2].map(|k| l[k] * (1.0 - omc) + (t[k] * phi.cos() + b[k] * phi.sin()) * sin_t))
}

pub fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub fn normalize(a: [f32; 3]) -> [f32; 3] {
    let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt().max(1e-20);
    a.map(|x| x / l)
}

/// The column map read back (`read_r32i` of `GpuWorld::column_map`) over a fixed window whose
/// least chunk is `window_min` and size `window_size`, in chunks.
pub struct Columns {
    pub map: Vec<i32>,
    pub window_min: [i32; 2],
    pub window_size: [i32; 2],
}

/// pair.wgsl's constants.
const COLUMN_CELL: i32 = 2;
const COLUMN_CELLS: i32 = 32;
const COLUMN_KEY_STEPS: u32 = 12;
const COLUMN_KEY_D0: f32 = 4.0;
const COLUMN_KEY_D1: f32 = 512.0;
const COLUMN_KEY_REFINE: u32 = 8;
pub const SEED_LIFT: f32 = COLUMN_CELL as f32 + 0.5;

impl Columns {
    /// pair.wgsl's `column_cell_height`.
    pub fn cell_height(&self, c: [i32; 2]) -> f32 {
        let x = c.map(|v| v * COLUMN_CELL);
        let lo = self.window_min.map(|v| v * 64);
        let hi = [0, 1].map(|k| (self.window_min[k] + self.window_size[k]) * 64);
        if (0..2).any(|k| x[k] < lo[k] || x[k] >= hi[k]) {
            return 3.0e38;
        }
        let k = x.map(|v| v >> 6);
        let w = self.window_size;
        let tile = [0, 1].map(|j| ((k[j] % w[j]) + w[j]) % w[j]);
        let tx = tile[0] * COLUMN_CELLS + (x[0] - k[0] * 64) / COLUMN_CELL;
        let ty = tile[1] * COLUMN_CELLS + (x[1] - k[1] * 64) / COLUMN_CELL;
        let top = self.map[(ty * w[0] * COLUMN_CELLS + tx) as usize];
        if top == i32::MIN {
            -1.0e9
        } else {
            top as f32 + 1.0
        }
    }

    /// Whether `p` lifted [`SEED_LIFT`] along `n` lies under its own map cell's top.
    pub fn under_own_top(&self, p: [f32; 3], n: [f32; 3]) -> bool {
        let q = [0, 1, 2].map(|k| p[k] + n[k] * SEED_LIFT);
        let own = self.cell_height([
            (q[0] / COLUMN_CELL as f32).floor() as i32,
            (q[1] / COLUMN_CELL as f32).floor() as i32,
        ]);
        own < 1.0e38 && own > q[2]
    }

    fn cell_entry(q: [f32; 2], u: [f32; 2], c: [i32; 2]) -> f32 {
        let t = [0, 1].map(|k| {
            let lo = (c[k] * COLUMN_CELL) as f32;
            let near = if u[k] > 0.0 {
                lo
            } else {
                lo + COLUMN_CELL as f32
            };
            if u[k].abs() > 1.0e-6 {
                (near - q[k]) / u[k]
            } else {
                -3.0e38
            }
        });
        t[0].max(t[1])
    }

    /// pair.wgsl's `column_key_visibility` at `p` with face normal `n`, the key light toward
    /// unit `l` of angular radius `radius`.
    pub fn horizon(&self, p: [f32; 3], n: [f32; 3], l: [f32; 3], radius: f32) -> f32 {
        if l[2] <= 0.0 {
            return 0.0;
        }
        let q = [0, 1, 2].map(|k| p[k] + n[k] * SEED_LIFT);
        let flat = (l[0] * l[0] + l[1] * l[1]).sqrt();
        let tan_e = l[2] / flat.max(1.0e-6);
        let u = if flat > 1.0e-6 {
            [l[0] / flat, l[1] / flat]
        } else {
            [1.0, 0.0]
        };
        let cell = |x: f32, y: f32| {
            [
                (x / COLUMN_CELL as f32).floor() as i32,
                (y / COLUMN_CELL as f32).floor() as i32,
            ]
        };
        let own = self.cell_height(cell(q[0], q[1]));
        if own < 1.0e38 && own > q[2] {
            return 0.0;
        }
        let mut tan_h = -3.0e38f32;
        let ratio = (COLUMN_KEY_D1 / COLUMN_KEY_D0).powf(1.0 / (COLUMN_KEY_STEPS - 1) as f32);
        let mut d = COLUMN_KEY_D0;
        let mut d_prev = COLUMN_CELL as f32;
        let mut best = -3.0e38f32;
        let mut seg = [0.0f32, -1.0];
        for _ in 0..COLUMN_KEY_STEPS {
            let c = cell(q[0] + u[0] * d, q[1] + u[1] * d);
            let h = self.cell_height(c);
            if h > 1.0e38 {
                break;
            }
            let t = Self::cell_entry([q[0], q[1]], u, c).max(0.5);
            tan_h = tan_h.max((h - q[2]) / t);
            let score = (h - q[2]) / d_prev;
            if score > best {
                best = score;
                seg = [d_prev, d];
            }
            d_prev = d;
            d *= ratio;
        }
        if seg[1] > seg[0] {
            let mut c = cell(q[0] + u[0] * seg[0], q[1] + u[1] * seg[0]);
            let s = u.map(|x| {
                if x > 0.0 {
                    1
                } else if x < 0.0 {
                    -1
                } else {
                    0
                }
            });
            for _ in 0..COLUMN_KEY_REFINE {
                let t = Self::cell_entry([q[0], q[1]], u, c);
                if t >= seg[1] {
                    break;
                }
                let h = self.cell_height(c);
                if h > 1.0e38 {
                    break;
                }
                if t > 0.5 {
                    tan_h = tan_h.max((h - q[2]) / t);
                }
                let far = [0, 1].map(|k| {
                    (c[k] * COLUMN_CELL) as f32 + if u[k] > 0.0 { COLUMN_CELL as f32 } else { 0.0 }
                });
                let tx = [0, 1].map(|k| {
                    if u[k].abs() > 1.0e-6 {
                        (far[k] - q[k]) / u[k]
                    } else {
                        3.0e38
                    }
                });
                if tx[0] <= tx[1] {
                    c[0] += s[0];
                } else {
                    c[1] += s[1];
                }
            }
        }
        let r = radius.tan().max(1.0e-4);
        ((tan_e - tan_h) / (2.0 * r) + 0.5).clamp(0.0, 1.0)
    }

    /// [`Columns::horizon`] with every map cell along the sun's horizontal direction walked
    /// to the window's side (no coarse march, no refinement cap): what the heightfield itself
    /// answers at the same lift.
    pub fn horizon_exact(&self, p: [f32; 3], n: [f32; 3], l: [f32; 3], radius: f32) -> f32 {
        if l[2] <= 0.0 {
            return 0.0;
        }
        let q = [0, 1, 2].map(|k| p[k] + n[k] * SEED_LIFT);
        let flat = (l[0] * l[0] + l[1] * l[1]).sqrt();
        let tan_e = l[2] / flat.max(1.0e-6);
        let u = if flat > 1.0e-6 {
            [l[0] / flat, l[1] / flat]
        } else {
            [1.0, 0.0]
        };
        let cs = COLUMN_CELL as f32;
        let mut c = [(q[0] / cs).floor() as i32, (q[1] / cs).floor() as i32];
        let own = self.cell_height(c);
        if own < 1.0e38 && own > q[2] {
            return 0.0;
        }
        let s = u.map(|x| if x > 0.0 { 1 } else { -1 });
        let mut tan_h = -3.0e38f32;
        loop {
            let far = [0, 1].map(|k| (c[k] as f32 + if u[k] > 0.0 { 1.0 } else { 0.0 }) * cs);
            let tx = [0, 1].map(|k| {
                if u[k].abs() > 1.0e-6 {
                    (far[k] - q[k]) / u[k]
                } else {
                    3.0e38
                }
            });
            if tx[0] <= tx[1] {
                c[0] += s[0];
            } else {
                c[1] += s[1];
            }
            let h = self.cell_height(c);
            if h > 1.0e38 {
                break;
            }
            let t = Self::cell_entry([q[0], q[1]], u, c);
            if t > 0.5 {
                tan_h = tan_h.max((h - q[2]) / t);
            }
        }
        let r = radius.tan().max(1.0e-4);
        ((tan_e - tan_h) / (2.0 * r) + 0.5).clamp(0.0, 1.0)
    }
}

/// Store blocks read back by chunk, read at cells of 8 voxels (`BRICK_CELL`).
#[derive(Default)]
pub struct Records {
    pub blocks: HashMap<[i32; 3], Vec<u32>>,
}

impl Records {
    /// Cell `c`'s record.
    fn cell(&self, c: [i32; 3]) -> Option<&[u32]> {
        let block = self.blocks.get(&c.map(|v| v >> 3))?;
        let s = c.map(|v| v & 7);
        let dir = block[(s[0] + 8 * (s[1] + 8 * s[2])) as usize];
        if dir == 0 {
            return None;
        }
        let end = (dir >> 11) as usize;
        Some(&block[end - svet_render::internals::BRICK_WORDS..end])
    }

    /// temporal.wgsl's `store_key_cell`: cell `c`'s key visibility for direction `face`;
    /// `None` where it lacks the direction, was baked with no key light, or is SKY_STALE.
    pub fn key_cell(&self, c: [i32; 3], face: u8) -> Option<f32> {
        let r = self.cell(c)?;
        if svet_render::internals::record_sky_stale(r) {
            return None;
        }
        svet_render::internals::record_key_visibility(r, usize::from(face))
    }

    /// temporal.wgsl's `store_key_direction`: the 2 x 2 cells' bilinear over those that hold
    /// the direction, the voxel's own cell's where none has weight; `None` where its own cell
    /// holds none.
    pub fn key_direction(&self, v: [i32; 3], face: u8) -> Option<f32> {
        let own = self.key_cell(v.map(|x| x >> 3), face)?;
        let axis = usize::from(face / 2);
        let (ta, tb) = ((axis + 1) % 3, (axis + 2) % 3);
        let g = v.map(|x| (x as f32 + 0.5) / 8.0 - 0.5);
        let lo = g.map(|x| x.floor() as i32);
        let f = [0, 1, 2].map(|k| g[k] - g[k].floor());
        let (mut sum, mut weight) = (0.0f32, 0.0f32);
        for i in 0..4 {
            let (oa, ob) = (i & 1, i >> 1);
            let mut c = v.map(|x| x >> 3);
            c[ta] = lo[ta] + oa;
            c[tb] = lo[tb] + ob;
            let wa = if oa == 1 { f[ta] } else { 1.0 - f[ta] };
            let wb = if ob == 1 { f[tb] } else { 1.0 - f[tb] };
            if let Some(x) = self.key_cell(c, face) {
                sum += x * wa * wb;
                weight += wa * wb;
            }
        }
        Some(if weight > 0.0 { sum / weight } else { own })
    }

    /// temporal.wgsl's `store_key_visibility`: the mix over the directions shading normal `n`
    /// leans toward of each direction's [`Records::key_direction`].
    pub fn key_visibility(&self, v: [i32; 3], face: u8, n: [f32; 3]) -> Option<f32> {
        let own = self.key_direction(v, face)?;
        let (mut sum, mut weight) = (0.0f32, 0.0f32);
        for (a, &na) in n.iter().enumerate() {
            let w = na * na;
            if w < 1.0e-4 {
                continue;
            }
            let dir = (2 * a + usize::from(na > 0.0)) as u8;
            let r = if dir == face {
                Some(own)
            } else {
                self.key_direction(v, dir)
            };
            if let Some(r) = r {
                sum += r * w;
                weight += w;
            }
        }
        Some(if weight > 0.0 { sum / weight } else { own })
    }

    /// Whether chunk `c`'s block was read.
    pub fn has(&self, c: ChunkCoord) -> bool {
        self.blocks.contains_key(&c.0)
    }
}

/// hitrec.wgsl's `octa16_decode`: the shading normal of a hit record's first two words.
pub fn octa16_decode(e: [u16; 2]) -> [f32; 3] {
    let p = e.map(|x| f32::from(x as i16) / 32767.0);
    let mut d = [p[0], p[1], 1.0 - p[0].abs() - p[1].abs()];
    if d[2] < 0.0 {
        let s = [d[0], d[1]].map(|x| if x >= 0.0 { 1.0 } else { -1.0 });
        let (x, y) = ((1.0 - d[1].abs()) * s[0], (1.0 - d[0].abs()) * s[1]);
        d[0] = x;
        d[1] = y;
    }
    normalize(d)
}
