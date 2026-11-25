//! The levelled walk over the column map (Plan 0100 Phase 5b, a CPU port): coarse maps of the
//! largest top of each `B x B` map cells, the walk over every cell entering a coarse cell only
//! where its bound can raise the horizon, and ending where the visibility is 0 or no top of the
//! window can reach the disc. Its visibility equals [`Columns::walk_at`]'s; what it changes is
//! the texels read.

use super::cover::{COLUMN_CELL, COLUMN_CELLS, Columns};

/// One coarse level: the largest top (as `cell_height` reads it) of each `block x block` cells
/// over the window, from its least cell.
pub struct Level {
    pub block: i32,
    min: [i32; 2],
    size: [i32; 2],
    tops: Vec<f32>,
}

/// The coarse levels, coarsest first, and the window's highest top.
pub struct Levels {
    pub levels: Vec<Level>,
    pub top_max: f32,
}

impl Levels {
    /// Levels of `blocks` cells (each a multiple of the next, coarsest first) over `cols`.
    pub fn of(cols: &Columns, blocks: &[i32]) -> Self {
        let min = cols.window_min.map(|v| v * COLUMN_CELLS);
        let size = cols.window_size.map(|v| v * COLUMN_CELLS);
        let mut fine = vec![-1.0e9f32; (size[0] * size[1]) as usize];
        let mut top_max = -1.0e9f32;
        for y in 0..size[1] {
            for x in 0..size[0] {
                let h = cols.cell_height([min[0] + x, min[1] + y]);
                fine[(y * size[0] + x) as usize] = h;
                top_max = top_max.max(h);
            }
        }
        let levels = blocks
            .iter()
            .map(|&b| {
                let n = [size[0] / b, size[1] / b];
                let mut tops = vec![-1.0e9f32; (n[0] * n[1]) as usize];
                for y in 0..size[1] {
                    for x in 0..size[0] {
                        let i = ((y / b) * n[0] + x / b) as usize;
                        tops[i] = tops[i].max(fine[(y * size[0] + x) as usize]);
                    }
                }
                Level {
                    block: b,
                    min: [min[0] / b, min[1] / b],
                    size: n,
                    tops,
                }
            })
            .collect();
        Self { levels, top_max }
    }

    /// The texels the coarse maps hold, all levels.
    pub fn texels(&self) -> usize {
        self.levels.iter().map(|l| l.tops.len()).sum()
    }
}

/// The parameters at which the line from `q` along `u` enters and leaves the box `lo..hi`.
fn span(q: [f32; 2], u: [f32; 2], lo: [f32; 2], hi: [f32; 2]) -> (f32, f32) {
    let (mut t0, mut t1) = (-3.0e38f32, 3.0e38f32);
    for k in 0..2 {
        if u[k].abs() > 1.0e-6 {
            let a = (lo[k] - q[k]) / u[k];
            let b = (hi[k] - q[k]) / u[k];
            t0 = t0.max(a.min(b));
            t1 = t1.min(a.max(b));
        }
    }
    (t0, t1)
}

/// The largest `tan` a cell of top at most `top` can give a point at height `qz` between
/// parameters `t0` and `t1` (cells at 0.5 or nearer are not read by the walk).
fn bound(top: f32, qz: f32, t0: f32, t1: f32) -> f32 {
    let num = top - qz;
    if num >= 0.0 {
        num / t0.max(0.5)
    } else {
        num / t1
    }
}

impl Columns {
    /// [`Columns::walk_at`] over `lv`'s levels: the same visibility, and the texels read (each
    /// coarse cell entered is one, each fine cell not skipped is one).
    pub fn walk_levelled(
        &self,
        lv: &Levels,
        p: [f32; 3],
        n: [f32; 3],
        l: [f32; 3],
        radius: f32,
        lift: f32,
    ) -> (f32, u32) {
        if l[2] <= 0.0 {
            return (0.0, 0);
        }
        let q = [0, 1, 2].map(|k| p[k] + n[k] * lift);
        let flat = (l[0] * l[0] + l[1] * l[1]).sqrt();
        let tan_e = l[2] / flat.max(1.0e-6);
        let u = if flat > 1.0e-6 {
            [l[0] / flat, l[1] / flat]
        } else {
            [1.0, 0.0]
        };
        let r = radius.tan().max(1.0e-4);
        let cs = COLUMN_CELL as f32;
        let mut c = [(q[0] / cs).floor() as i32, (q[1] / cs).floor() as i32];
        let own = self.cell_height(c);
        if own < 1.0e38 && own > q[2] {
            return (0.0, 1);
        }
        let s = u.map(|x| if x > 0.0 { 1 } else { -1 });
        let mut tan_h = -3.0e38f32;
        let mut reads = 1u32;
        let mut cur: Vec<Option<[i32; 2]>> = vec![None; lv.levels.len()];
        let mut skip = vec![false; lv.levels.len()];
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
            if self.cell_height(c) > 1.0e38 {
                break;
            }
            let te = Self::cell_entry([q[0], q[1]], u, c);
            let floor = tan_h.max(tan_e - r);
            // Visibility 0 already, or no top of the window can raise the horizon from here on.
            if tan_h >= tan_e + r || bound(lv.top_max, q[2], te, 3.0e38) < floor {
                break;
            }
            let mut skipped = false;
            for (k, level) in lv.levels.iter().enumerate() {
                if skipped {
                    cur[k] = None;
                    continue;
                }
                let b = [c[0].div_euclid(level.block), c[1].div_euclid(level.block)];
                if cur[k] != Some(b) {
                    cur[k] = Some(b);
                    reads += 1;
                    let i = ((b[1] - level.min[1]) * level.size[0] + b[0] - level.min[0]) as usize;
                    let lo = b.map(|v| (v * level.block) as f32 * cs);
                    let hi = lo.map(|v| v + (level.block as f32) * cs);
                    let (t0, t1) = span([q[0], q[1]], u, lo, hi);
                    skip[k] = bound(level.tops[i], q[2], t0, t1) < floor;
                }
                skipped = skip[k];
            }
            if skipped {
                continue;
            }
            reads += 1;
            let h = self.cell_height(c);
            if te > 0.5 {
                tan_h = tan_h.max((h - q[2]) / te);
            }
        }
        (((tan_e - tan_h) / (2.0 * r) + 0.5).clamp(0.0, 1.0), reads)
    }
}
