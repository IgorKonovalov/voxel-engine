//! The aliasing reads on the final image (`tests/gpu_strip.rs`): frame-to-frame change in 8-bit
//! levels, the fold over the jitter's cycle, the pan's crawl, and the class mask that sorts
//! display pixels by what lies under them.
//!
//! **Levels.** Every figure is on the display target's rgba8, per pixel the worst of the three
//! channels' absolute differences, in 8-bit levels.
//!
//! - **`T`**: frame `f` against `f + 1`, pooled over the run's pairs and pixels: the mean, the
//!   99.9th percentile, the max, the share over 2, and the count of **jumps**, a difference
//!   over [`JUMP`].
//! - **`L`**: the same with `f + phases` for `f + 1`. A jump `T` shows and `L` does not is locked
//!   to the jitter's cycle.
//! - **`J`**: over `c` whole cycles of `P` phases, per pixel and channel, the between-phase sum
//!   of squares `SS_b = c sum_k (m_k - m)^2` less its expectation under noise,
//!   `(P - 1) SS_w / (P (c - 1))` (`SS_w` the within-phase sum of squares), over the total sum of
//!   squares; numerator and denominator are summed over pixels and channels before the ratio.
//!   1 is all locked, 0 none (noise can read a little under 0); a constant series reads 0.
//! - **`C`**, the crawl ([`crawl`]): on a pure-yaw pan that moves the image `shift` whole pixels
//!   right over `lag` frames, frame `f` at `x + shift` against frame `f - lag` at `x`, inside a
//!   band of columns.
//!
//! **The class mask.** A display pixel takes the internal pixel under it
//! (`floor((x + 0.5) internal / display)` on each axis) and that pixel's 3 x 3 neighbours in the
//! hit buffer (`Frame::hit_positions`, `Frame::hit_records`). A hit's face is its entry axis
//! (record flags bits 3..5, less 1), the plane `round(p[axis])` and the floor of the other two
//! coordinates; a hit with no entry face keys by `floor(p)` on all three. Against each
//! neighbour: the same face is nothing; the same axis and plane is a **seam**; positions within
//! [`CREASE_REACH`] voxels a **crease**; farther (or one of the two a miss) a **silhouette**. An
//! emitting material at the centre or a neighbour makes an **emitter**; a centre and all
//! neighbours missing is a **miss**; else **interior**. The order of [`Class`] is how edge-like
//! a class is, and a still camera's pixel takes the most edge-like across a cycle
//! ([`Mask::merge`]).
//!
//! **Footprint**: a voxel's width in internal pixels at the centre hit's `t`,
//! `W / ((tan_l + tan_r) t)`, binned under 2, 2 to 6, over 6 (the far normal's band).
//!
//! **The direction map** ([`directions`]): a display pixel's store direction, the entry face
//! `2 axis + (n[axis] > 0)` of the hit under it with `n` the face normal against the ray from the
//! eye (temporal.wgsl's `store_face`), `None` on a miss or a hit with no entry face. A pixel is
//! **at a direction change** ([`changes`]) when a hit within one display pixel (its 3 x 3) holds
//! another direction. [`Mask::at`] carries it, and [`ByClass`] splits each class by it.

use std::fmt::Write as _;

/// A difference over this many levels is a jump.
pub const JUMP: u8 = 32;
/// Neighbouring hits on different faces within this many voxels are a crease, farther a
/// silhouette.
pub const CREASE_REACH: f32 = 1.5;

/// Pooled per-pixel differences in levels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Change {
    pub mean: f64,
    pub p999: u8,
    pub max: u8,
    pub over_2: f64,
    pub jumps: u64,
    pub samples: u64,
}

impl Change {
    pub fn cell(&self) -> String {
        format!(
            "{:.3}, {}, {}, {:.2} %, {}",
            self.mean,
            self.p999,
            self.max,
            self.over_2 * 100.0,
            self.jumps
        )
    }
}

/// A histogram of differences in levels, the source of a [`Change`].
#[derive(Clone)]
pub struct Hist(pub [u64; 256]);

impl Default for Hist {
    fn default() -> Self {
        Self([0; 256])
    }
}

impl Hist {
    pub fn add(&mut self, d: u8) {
        self.0[usize::from(d)] += 1;
    }

    pub fn change(&self) -> Change {
        let n: u64 = self.0.iter().sum();
        if n == 0 {
            return Change::default();
        }
        let sum: u64 = self.0.iter().enumerate().map(|(d, &c)| d as u64 * c).sum();
        let max = self.0.iter().rposition(|&c| c > 0).unwrap_or(0) as u8;
        // The smallest level with at least 99.9 % of the samples at or under it.
        let want = (n as f64 * 0.999).ceil() as u64;
        let mut acc = 0;
        let mut p999 = 0u8;
        for (d, &c) in self.0.iter().enumerate() {
            acc += c;
            if acc >= want {
                p999 = d as u8;
                break;
            }
        }
        Change {
            mean: sum as f64 / n as f64,
            p999,
            max,
            over_2: self.0[3..].iter().sum::<u64>() as f64 / n as f64,
            jumps: self.0[usize::from(JUMP) + 1..].iter().sum(),
            samples: n,
        }
    }
}

/// The worst channel's absolute difference of two rgba8 pixels.
pub fn level(a: &[u8], b: &[u8]) -> u8 {
    (0..3).map(|k| a[k].abs_diff(b[k])).max().unwrap_or(0)
}

/// Per-pixel worst-channel differences of two rgba8 images of the same size.
pub fn levels(a: &[u8], b: &[u8]) -> Vec<u8> {
    assert_eq!(a.len(), b.len());
    a.chunks_exact(4)
        .zip(b.chunks_exact(4))
        .map(|(p, q)| level(p, q))
        .collect()
}

/// A run's frames folded over the jitter's cycle.
#[derive(Clone, Debug)]
pub struct Fold {
    pub t: Change,
    pub l: Change,
    pub j: f64,
    pub phases: u32,
    pub cycles: u32,
    /// `T`'s jumps per pixel over the run's pairs.
    pub t_jumps: Vec<u32>,
    /// `L`'s jumps per pixel.
    pub l_jumps: Vec<u32>,
}

impl Fold {
    /// `L`'s jumps over `T`'s: the share of the jumps not locked to the cycle (1 when `T` has
    /// none).
    pub fn unlocked(&self) -> f64 {
        if self.t.jumps == 0 {
            1.0
        } else {
            self.l.jumps as f64 / self.t.jumps as f64
        }
    }
}

/// `T`, `L` and `J` over rgba8 `frames` with `phases` to a cycle. Only whole cycles count
/// toward `L` and `J`; `T` reads every consecutive pair. With fewer frames than two cycles `L`
/// and `J` read 0.
pub fn fold(frames: &[Vec<u8>], phases: u32) -> Fold {
    let n = frames.first().map_or(0, |f| f.len() / 4);
    let p = phases.max(1) as usize;
    let cycles = frames.len() / p;
    let used = cycles * p;
    let mut t_hist = Hist::default();
    let mut l_hist = Hist::default();
    let mut t_jumps = vec![0u32; n];
    let mut l_jumps = vec![0u32; n];
    for w in frames.windows(2) {
        for (i, d) in levels(&w[0], &w[1]).into_iter().enumerate() {
            t_hist.add(d);
            t_jumps[i] += u32::from(d > JUMP);
        }
    }
    if cycles >= 2 {
        for f in 0..used - p {
            for (i, d) in levels(&frames[f], &frames[f + p]).into_iter().enumerate() {
                l_hist.add(d);
                l_jumps[i] += u32::from(d > JUMP);
            }
        }
    }
    let j = if cycles >= 2 {
        let c = cycles as f64;
        let noise = (p as f64 - 1.0) / (p as f64 * (c - 1.0));
        let (mut num, mut den) = (0.0f64, 0.0f64);
        let mut phase_sum = vec![0f64; p];
        for i in 0..n {
            for k in 0..3 {
                phase_sum.iter_mut().for_each(|s| *s = 0.0);
                let mut all = 0.0;
                for (f, frame) in frames[..used].iter().enumerate() {
                    let v = f64::from(frame[4 * i + k]);
                    phase_sum[f % p] += v;
                    all += v;
                }
                let m = all / used as f64;
                let (mut ss_t, mut ss_b) = (0.0, 0.0);
                for frame in &frames[..used] {
                    let v = f64::from(frame[4 * i + k]);
                    ss_t += (v - m) * (v - m);
                }
                for s in &phase_sum {
                    let mk = s / c;
                    ss_b += c * (mk - m) * (mk - m);
                }
                let ss_w = (ss_t - ss_b).max(0.0);
                num += ss_b - noise * ss_w;
                den += ss_t;
            }
        }
        if den > 0.0 { num / den } else { 0.0 }
    } else {
        0.0
    };
    Fold {
        t: t_hist.change(),
        l: if cycles >= 2 {
            l_hist.change()
        } else {
            Change::default()
        },
        j,
        phases,
        cycles: cycles as u32,
        t_jumps,
        l_jumps,
    }
}

/// What lies under a display pixel, least edge-like first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Class {
    Miss,
    Interior,
    Seam,
    Crease,
    Silhouette,
    Emitter,
}

impl Class {
    pub const ALL: [Self; 6] = [
        Self::Interior,
        Self::Seam,
        Self::Crease,
        Self::Silhouette,
        Self::Emitter,
        Self::Miss,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Miss => "miss",
            Self::Interior => "interior",
            Self::Seam => "seam",
            Self::Crease => "crease",
            Self::Silhouette => "silhouette",
            Self::Emitter => "emitter",
        }
    }

    /// The mask image's colour.
    pub fn colour(self) -> [u8; 4] {
        match self {
            Self::Miss => [0, 0, 0, 255],
            Self::Interior => [60, 60, 60, 255],
            Self::Seam => [40, 90, 200, 255],
            Self::Crease => [40, 200, 80, 255],
            Self::Silhouette => [230, 40, 40, 255],
            Self::Emitter => [250, 220, 40, 255],
        }
    }
}

/// A voxel's width in internal pixels at the hit, binned.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Footprint {
    Under2,
    From2To6,
    Over6,
}

impl Footprint {
    pub const ALL: [Self; 3] = [Self::Under2, Self::From2To6, Self::Over6];

    pub fn of(px: f32) -> Self {
        if px < 2.0 {
            Self::Under2
        } else if px <= 6.0 {
            Self::From2To6
        } else {
            Self::Over6
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Under2 => "px < 2",
            Self::From2To6 => "px 2..6",
            Self::Over6 => "px > 6",
        }
    }
}

/// One frame's internal hit buffer, as read back.
pub struct Hits<'a> {
    pub size: [u32; 2],
    /// World position in voxels, then `t` (`3e38` on a miss).
    pub pos: &'a [[f32; 4]],
    /// Normal u and v, material, flags (bit 0 hit, bits 3..5 the entry axis plus one).
    pub info: &'a [[u16; 4]],
}

/// A hit's face: entry axis (3 for none), the plane or `floor(p[axis])`, and the floor of the
/// other two coordinates.
fn face(pos: [f32; 4], info: [u16; 4]) -> (u8, [i32; 3]) {
    let axis = i32::from((info[3] >> 3) & 3) - 1;
    if axis < 0 {
        return (3, [0, 1, 2].map(|k| pos[k].floor() as i32));
    }
    let a = axis as usize;
    let mut key = [0, 1, 2].map(|k| pos[k].floor() as i32);
    key[a] = pos[a].round() as i32;
    (a as u8, key)
}

fn is_hit(info: [u16; 4]) -> bool {
    info[3] & 1 != 0
}

impl Hits<'_> {
    fn at(&self, x: i32, y: i32) -> Option<([f32; 4], [u16; 4])> {
        let [w, h] = self.size;
        if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
            return None;
        }
        let i = (y as u32 * w + x as u32) as usize;
        Some((self.pos[i], self.info[i]))
    }

    /// The class of internal pixel (`x`, `y`) from it and its 3 x 3 neighbours; `emits` names
    /// the emitting materials.
    pub fn class(&self, x: i32, y: i32, emits: &dyn Fn(u16) -> bool) -> Class {
        let Some((cp, ci)) = self.at(x, y) else {
            return Class::Miss;
        };
        let centre_hit = is_hit(ci);
        let mut class = if centre_hit {
            Class::Interior
        } else {
            Class::Miss
        };
        if centre_hit && emits(ci[2]) {
            return Class::Emitter;
        }
        let cf = face(cp, ci);
        for dy in -1..=1 {
            for dx in -1..=1 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                let Some((np, ni)) = self.at(x + dx, y + dy) else {
                    continue;
                };
                let n_hit = is_hit(ni);
                if n_hit && emits(ni[2]) {
                    return Class::Emitter;
                }
                let c = match (centre_hit, n_hit) {
                    (false, false) => Class::Miss,
                    (true, false) | (false, true) => Class::Silhouette,
                    (true, true) => {
                        let nf = face(np, ni);
                        let d = ((np[0] - cp[0]).powi(2)
                            + (np[1] - cp[1]).powi(2)
                            + (np[2] - cp[2]).powi(2))
                        .sqrt();
                        if nf == cf {
                            Class::Interior
                        } else if nf.0 == cf.0
                            && cf.0 < 3
                            && nf.1[nf.0 as usize] == cf.1[cf.0 as usize]
                        {
                            Class::Seam
                        } else if d <= CREASE_REACH {
                            Class::Crease
                        } else {
                            Class::Silhouette
                        }
                    }
                };
                class = class.max(c);
            }
        }
        class
    }

    /// The centre hit's footprint at internal pixel (`x`, `y`), `None` on a miss. `tan_sum` is
    /// `tan_l + tan_r` of the view.
    pub fn footprint(&self, x: i32, y: i32, tan_sum: f32) -> Option<Footprint> {
        let (p, i) = self.at(x, y)?;
        if !is_hit(i) || p[3] <= 0.0 || p[3] > 1e30 {
            return None;
        }
        Some(Footprint::of(self.size[0] as f32 / (tan_sum * p[3])))
    }
}

/// Display pixels' classes and footprints.
#[derive(Clone, Debug)]
pub struct Mask {
    pub size: [u32; 2],
    pub class: Vec<Class>,
    pub footprint: Vec<Option<Footprint>>,
    /// Per pixel, whether it is at a direction change ([`changes`]); empty until
    /// [`Mask::set_directions`] fills it.
    pub at: Vec<bool>,
}

/// The internal pixel under display pixel `x` of `display` pixels over `internal`.
pub fn under(x: u32, display: u32, internal: u32) -> i32 {
    ((u64::from(x) * 2 + 1) * u64::from(internal) / (2 * u64::from(display))) as i32
}

impl Mask {
    /// The mask of one frame's hits at the `display` size.
    pub fn of(
        hits: &Hits<'_>,
        display: [u32; 2],
        tan_sum: f32,
        emits: &dyn Fn(u16) -> bool,
    ) -> Mask {
        let [w, h] = display;
        let mut class = Vec::with_capacity((w * h) as usize);
        let mut footprint = Vec::with_capacity((w * h) as usize);
        // Classify each internal pixel once; a display pixel takes the one under it.
        let [iw, ih] = hits.size;
        let mut cache: Vec<Option<(Class, Option<Footprint>)>> = vec![None; (iw * ih) as usize];
        for y in 0..h {
            let iy = under(y, h, ih);
            for x in 0..w {
                let ix = under(x, w, iw);
                let k = (iy as u32 * iw + ix as u32) as usize;
                let v = *cache[k].get_or_insert_with(|| {
                    (hits.class(ix, iy, emits), hits.footprint(ix, iy, tan_sum))
                });
                class.push(v.0);
                footprint.push(v.1);
            }
        }
        Mask {
            size: display,
            class,
            footprint,
            at: Vec::new(),
        }
    }

    /// Fills [`Mask::at`] from `hits` seen from `eye`, or with another frame's merges it (a
    /// pixel at a change in either is at one).
    pub fn set_directions(&mut self, hits: &Hits<'_>, eye: [f32; 3]) {
        let at = changes(&directions(hits, self.size, eye), self.size);
        if self.at.is_empty() {
            self.at = at;
        } else {
            self.at.iter_mut().zip(at).for_each(|(a, b)| *a |= b);
        }
    }

    /// Each pixel takes the more edge-like class of the two; the footprint of the first that
    /// has one.
    pub fn merge(&mut self, other: &Mask) {
        assert_eq!(self.size, other.size);
        for (a, b) in self.class.iter_mut().zip(&other.class) {
            *a = (*a).max(*b);
        }
        for (a, b) in self.footprint.iter_mut().zip(&other.footprint) {
            if a.is_none() {
                *a = *b;
            }
        }
        if self.at.is_empty() {
            self.at = other.at.clone();
        } else if !other.at.is_empty() {
            self.at
                .iter_mut()
                .zip(&other.at)
                .for_each(|(a, b)| *a |= *b);
        }
    }

    pub fn image(&self) -> Vec<u8> {
        self.class.iter().flat_map(|c| c.colour()).collect()
    }
}

/// Counts per class and footprint (`None` the fourth column): pixels and a per-pixel figure's
/// sum; `at_pixels` and `at_sum` the same over the pixels at a direction change ([`Mask::at`]).
#[derive(Clone, Debug, Default)]
pub struct ByClass {
    pub pixels: [[u64; 4]; 6],
    pub sum: [[u64; 4]; 6],
    pub at_pixels: [[u64; 4]; 6],
    pub at_sum: [[u64; 4]; 6],
    /// Whether any pixel came with a direction map, so the table prints the split.
    pub split: bool,
}

fn fp_col(f: Option<Footprint>) -> usize {
    match f {
        Some(Footprint::Under2) => 0,
        Some(Footprint::From2To6) => 1,
        Some(Footprint::Over6) => 2,
        None => 3,
    }
}

impl ByClass {
    pub fn add(&mut self, class: Class, fp: Option<Footprint>, value: u64) {
        let (r, c) = (class as usize, fp_col(fp));
        self.pixels[r][c] += 1;
        self.sum[r][c] += value;
    }

    /// Adds display pixel `i` of `mask` with `value`, split by [`Mask::at`] when it is filled.
    pub fn add_pixel(&mut self, mask: &Mask, i: usize, value: u64) {
        let (c, f) = (mask.class[i], mask.footprint[i]);
        self.add(c, f, value);
        if let Some(&at) = mask.at.get(i) {
            self.split = true;
            if at {
                let (r, col) = (c as usize, fp_col(f));
                self.at_pixels[r][col] += 1;
                self.at_sum[r][col] += value;
            }
        }
    }

    pub fn of(mask: &Mask, per_pixel: &[u32]) -> ByClass {
        Self::of_where(mask, per_pixel, &|_| true)
    }

    /// [`ByClass::of`] over the pixels `keep` names.
    pub fn of_where(mask: &Mask, per_pixel: &[u32], keep: &dyn Fn(usize) -> bool) -> ByClass {
        let mut b = ByClass::default();
        for (i, v) in per_pixel.iter().enumerate() {
            if keep(i) {
                b.add_pixel(mask, i, u64::from(*v));
            }
        }
        b
    }

    /// The sum over the pixels at a direction change, all classes.
    pub fn at_total(&self) -> u64 {
        self.at_sum.iter().flatten().sum()
    }

    /// The largest row of the split table by sum: each class's pixels inside one direction, and
    /// all pixels at a direction change as one row. Its name and share of the total.
    pub fn largest_split_row(&self) -> (String, f64) {
        let total = self.total().max(1) as f64;
        let mut rows: Vec<(String, u64)> = Class::ALL
            .iter()
            .map(|&c| {
                let r = c as usize;
                let inside = self.sum[r].iter().sum::<u64>() - self.at_sum[r].iter().sum::<u64>();
                (format!("{} inside one direction", c.name()), inside)
            })
            .collect();
        rows.push((
            "at a direction change (any class)".to_owned(),
            self.at_total(),
        ));
        let (name, v) = rows.into_iter().max_by_key(|r| r.1).expect("rows");
        (name, v as f64 / total)
    }

    /// The largest class by sum, with interior split into at a direction change and inside one
    /// direction, its name and share.
    pub fn largest_interior_split(&self) -> (String, f64) {
        let total = self.total().max(1) as f64;
        let mut rows: Vec<(String, u64)> = Vec::new();
        for c in Class::ALL {
            let r = c as usize;
            let all: u64 = self.sum[r].iter().sum();
            if c == Class::Interior {
                let at: u64 = self.at_sum[r].iter().sum();
                rows.push(("interior at a direction change".to_owned(), at));
                rows.push(("interior inside one direction".to_owned(), all - at));
            } else {
                rows.push((c.name().to_owned(), all));
            }
        }
        let (name, v) = rows.into_iter().max_by_key(|r| r.1).expect("rows");
        (name, v as f64 / total)
    }

    pub fn total(&self) -> u64 {
        self.sum.iter().flatten().sum()
    }

    pub fn class_sum(&self, c: Class) -> u64 {
        self.sum[c as usize].iter().sum()
    }

    /// A markdown table: per class, per footprint, `sum (share of all) / pixels`.
    pub fn table(&self, what: &str) -> String {
        let total = self.total().max(1) as f64;
        let mut s = String::new();
        let _ = writeln!(
            s,
            "| class | {what}: px < 2 | px 2..6 | px > 6 | no hit | all |"
        );
        s.push_str("|---|---|---|---|---|---|\n");
        for c in Class::ALL {
            let r = c as usize;
            let _ = write!(s, "| {} |", c.name());
            for col in 0..4 {
                let _ = write!(
                    s,
                    " {} ({:.1} %) / {} px |",
                    self.sum[r][col],
                    self.sum[r][col] as f64 / total * 100.0,
                    self.pixels[r][col]
                );
            }
            let (sum, px): (u64, u64) = (self.sum[r].iter().sum(), self.pixels[r].iter().sum());
            let _ = writeln!(
                s,
                " {sum} ({:.1} %) / {px} px |",
                sum as f64 / total * 100.0
            );
            if self.split {
                for (label, at) in [
                    ("at a direction change", true),
                    ("inside one direction", false),
                ] {
                    let _ = write!(s, "| {} {label} |", c.name());
                    let cell = |col: usize| {
                        if at {
                            (self.at_sum[r][col], self.at_pixels[r][col])
                        } else {
                            (
                                self.sum[r][col] - self.at_sum[r][col],
                                self.pixels[r][col] - self.at_pixels[r][col],
                            )
                        }
                    };
                    let (mut ts, mut tp) = (0, 0);
                    for col in 0..4 {
                        let (v, p) = cell(col);
                        ts += v;
                        tp += p;
                        let _ = write!(s, " {v} ({:.1} %) / {p} px |", v as f64 / total * 100.0);
                    }
                    let _ = writeln!(s, " {ts} ({:.1} %) / {tp} px |", ts as f64 / total * 100.0);
                }
            }
        }
        if self.split {
            let (ts, tp): (u64, u64) = (self.at_total(), self.at_pixels.iter().flatten().sum());
            let _ = writeln!(
                s,
                "| at a direction change, all classes | | | | | {ts} ({:.1} %) / {tp} px |",
                ts as f64 / total * 100.0
            );
        }
        s
    }
}

/// The `n` tiles of `tile` pixels with the most of `per_pixel`, most first: (x, y, sum) of each
/// tile's corner.
pub fn worst_tiles(per_pixel: &[u32], size: [u32; 2], tile: u32, n: usize) -> Vec<(u32, u32, u64)> {
    let [w, h] = size;
    let (tw, th) = (w.div_ceil(tile), h.div_ceil(tile));
    let mut sums = vec![0u64; (tw * th) as usize];
    for y in 0..h {
        for x in 0..w {
            sums[((y / tile) * tw + x / tile) as usize] +=
                u64::from(per_pixel[(y * w + x) as usize]);
        }
    }
    let mut order: Vec<usize> = (0..sums.len()).filter(|&i| sums[i] > 0).collect();
    order.sort_by(|&a, &b| sums[b].cmp(&sums[a]).then(a.cmp(&b)));
    order
        .into_iter()
        .take(n)
        .map(|i| ((i as u32 % tw) * tile, (i as u32 / tw) * tile, sums[i]))
        .collect()
}

/// A grey image of a per-pixel count out of `of`: white at `of`.
pub fn count_image(per_pixel: &[u32], of: u32) -> Vec<u8> {
    let of = of.max(1);
    per_pixel
        .iter()
        .flat_map(|&c| {
            let g = (u64::from(c) * 255 / u64::from(of)).min(255) as u8;
            [g, g, g, 255]
        })
        .collect()
}

/// The crawl between two rgba8 frames of width `w`: `cur` at `x + shift` against `prev` at
/// `x`, for `x` in `cols` (pixels whose shifted column leaves the image are skipped). Adds each
/// pixel's level to `hist`, and with a mask (of `cur`) to `by` with 1 per jump.
pub fn crawl(
    cur: &[u8],
    prev: &[u8],
    w: u32,
    shift: i32,
    cols: std::ops::Range<u32>,
    hist: &mut Hist,
    mut by: Option<(&Mask, &mut ByClass, &mut Hist)>,
) {
    let h = (cur.len() / 4) as u32 / w;
    for y in 0..h {
        for x in cols.clone() {
            let xs = x as i32 + shift;
            if xs < 0 || xs >= w as i32 {
                continue;
            }
            let a = ((y * w + xs as u32) * 4) as usize;
            let b = ((y * w + x) * 4) as usize;
            let d = level(&cur[a..a + 4], &prev[b..b + 4]);
            hist.add(d);
            if let Some((mask, by, edge)) = by.as_mut() {
                let k = (y * w + xs as u32) as usize;
                let c = mask.class[k];
                by.add_pixel(mask, k, u64::from(d > JUMP));
                if matches!(c, Class::Crease | Class::Silhouette) {
                    edge.add(d);
                }
            }
        }
    }
}

/// A splitmix64 stream for the seeded synthetic series.
pub struct Mix(pub u64);

impl Mix {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn byte(&mut self) -> u8 {
        (self.next() >> 56) as u8
    }
}

/// `E`'s lamp: at the eye, no offset and no shadow ray, its cone opened past the view (the
/// half-diagonal at 16:9 and 70 degrees is 0.96 rad, so `inner` 1.2 and `outer` 1.5), at
/// `intensity`.
pub fn eye_lamp(intensity: f32) -> vxe_render::LightParams {
    vxe_render::LightParams {
        intensity,
        inner: 1.2,
        outer: 1.5,
        ..vxe_render::LightParams::at_eye()
    }
}

/// `E`'s lighting over `base`: the lamp on, no indirect gather and no emissive candidates, so
/// the light pass draws no emissive sample (the hall's eye chunk holds emitters, so the mirror's
/// `light_radius` 0 alone leaves it samples). Emitters still glow, from the composite.
pub fn eye_lighting(base: vxe_render::Lighting) -> vxe_render::Lighting {
    vxe_render::Lighting {
        lamp: true,
        lamp_bounce: false,
        candidates: 0,
        ..base
    }
}

// ---- The pan ----

/// The yaw in radians that moves the image centre one display pixel, for a view `height`
/// pixels high with vertical field of view `vfov` at `pitch`: `atan(1 / focal) / cos(pitch)`,
/// `focal = (height / 2) / tan(vfov / 2)`.
pub fn yaw_per_pixel(height: u32, vfov: f32, pitch: f32) -> f32 {
    let focal = height as f32 * 0.5 / (vfov * 0.5).tan();
    (1.0 / focal).atan() / pitch.cos()
}

/// `C`'s lag in frames and shift in pixels at `px_per_frame`: a whole pixel over `1 / rate`
/// frames under one pixel a frame, else the rate's whole pixels over one frame.
pub fn lag_shift(px_per_frame: f32) -> (usize, i32) {
    if px_per_frame.abs() >= 1.0 {
        (1, px_per_frame.round() as i32)
    } else {
        (
            (1.0 / px_per_frame.abs()).round() as usize,
            px_per_frame.signum() as i32,
        )
    }
}

/// `C`'s centre band for a display `w` wide: columns 810 to 1109 at 1920, scaled.
pub fn band(w: u32) -> std::ops::Range<u32> {
    w * 810 / 1920..w * 1110 / 1920
}

/// The display-encoded luma of an rgba8 pixel in levels, 0 to 255.
pub fn luma8(p: &[u8]) -> f32 {
    0.2126 * f32::from(p[0]) + 0.7152 * f32::from(p[1]) + 0.0722 * f32::from(p[2])
}

/// Dots in an rgba8 image: a pixel whose luma (levels) is over its 5 x 5 median by more than
/// 24 and more than twice the median (the settle rig's rule).
pub fn dots(rgba: &[u8], w: u32, h: u32) -> Vec<bool> {
    let y: Vec<f32> = rgba.chunks_exact(4).map(luma8).collect();
    let mut out = vec![false; y.len()];
    let mut win = Vec::with_capacity(25);
    for py in 2..h.saturating_sub(2) {
        for px in 2..w.saturating_sub(2) {
            let v = y[(py * w + px) as usize];
            if v <= 24.0 {
                continue;
            }
            win.clear();
            for dy in 0..5 {
                for dx in 0..5 {
                    win.push(y[((py + dy - 2) * w + px + dx - 2) as usize]);
                }
            }
            win.sort_by(f32::total_cmp);
            let m = win[12];
            out[(py * w + px) as usize] = v - m > 24.0 && v > 2.0 * m;
        }
    }
    out
}

/// A yellow pixel: red and green both over twice blue, and green over half of red.
pub fn yellow(p: &[u8]) -> bool {
    let (r, g, b) = (u32::from(p[0]), u32::from(p[1]), u32::from(p[2]));
    r > 2 * b && g > 2 * b && 2 * g > r
}

/// Whether `set` holds a pixel within one pixel of (`x`, `y`).
pub fn near(set: &[bool], w: u32, h: u32, x: i32, y: i32) -> bool {
    (-1..=1).any(|dy| {
        (-1..=1).any(|dx| {
            let (nx, ny) = (x + dx, y + dy);
            nx >= 0
                && ny >= 0
                && nx < w as i32
                && ny < h as i32
                && set[(ny as u32 * w + nx as u32) as usize]
        })
    })
}

// ---- The rings ----

/// Per display pixel, the hit under it (as [`Mask::of`] maps it): `floor(p.z)` and `t`, `None`
/// on a miss.
pub fn z_levels(hits: &Hits<'_>, display: [u32; 2]) -> Vec<Option<(i32, f32)>> {
    let [w, h] = display;
    let [iw, ih] = hits.size;
    let mut out = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        let iy = under(y, h, ih) as u32;
        for x in 0..w {
            let ix = under(x, w, iw) as u32;
            let i = (iy * iw + ix) as usize;
            out.push(if is_hit(hits.info[i]) && hits.pos[i][3] < 1e30 {
                Some((hits.pos[i][2].floor() as i32, hits.pos[i][3]))
            } else {
                None
            });
        }
    }
    out
}

/// The z-level map's image: `floor(p.z)` parity black and white, misses red.
pub fn z_image(z: &[Option<(i32, f32)>]) -> Vec<u8> {
    z.iter()
        .flat_map(|v| match v {
            Some((l, _)) if l.rem_euclid(2) == 0 => [230, 230, 230, 255],
            Some(_) => [30, 30, 30, 255],
            None => [200, 0, 0, 255],
        })
        .collect()
}

/// Ring reads on a crop `[x, y, w, h]` of an rgba8 image `w` wide and its z-level map.
#[derive(Clone, Debug, Default)]
pub struct Rings {
    /// Image-edge pixels (a horizontal or vertical neighbour's luma over [`RING_EDGE`] levels
    /// apart) and the share of them within one pixel of a z-level change.
    pub edges: u64,
    pub edges_at_z: f64,
    /// The share of all crop pixels within one pixel of a z-level change.
    pub base_at_z: f64,
    /// Per `t` bin of [`RING_BIN`] voxels: (edges, edges at a z change, pixels, pixels at a z
    /// change).
    pub by_t: Vec<[u64; 4]>,
    /// Horizontal neighbours' `|dY|` in whole levels, 0 to 8 and over.
    pub hist: [u64; 10],
    /// Mean run length along rows of equal 8-bit luma, and the runs' count.
    pub plateau: f64,
    pub runs: u64,
}

/// An image edge for the ring read: neighbours' luma this many levels apart.
pub const RING_EDGE: f32 = 2.0;
/// The ring read's `t` bins, voxels.
pub const RING_BIN: f32 = 100.0;

impl Rings {
    pub fn read(rgba: &[u8], z: &[Option<(i32, f32)>], w: u32, c: [u32; 4]) -> Rings {
        let y8 = |x: u32, y: u32| luma8(&rgba[((y * w + x) * 4) as usize..]);
        let zl = |x: u32, y: u32| z[(y * w + x) as usize];
        let mut r = Rings::default();
        let (mut at_z, mut base, mut n) = (0u64, 0u64, 0u64);
        for py in c[1] + 1..c[1] + c[3] - 1 {
            let mut run = 1u64;
            for px in c[0] + 1..c[0] + c[2] - 1 {
                let v = y8(px, py);
                let dh = (y8(px + 1, py) - v).abs();
                let dv = (y8(px, py + 1) - v).abs();
                r.hist[(dh.round() as usize).min(9)] += 1;
                if y8(px + 1, py).round() == v.round() {
                    run += 1;
                } else {
                    r.plateau += run as f64;
                    r.runs += 1;
                    run = 1;
                }
                let Some((l, t)) = zl(px, py) else {
                    continue;
                };
                let z_change = (-1i32..=1).any(|dy| {
                    (-1i32..=1).any(|dx| {
                        zl((px as i32 + dx) as u32, (py as i32 + dy) as u32)
                            .is_some_and(|(m, _)| m != l)
                    })
                });
                let edge = dh > RING_EDGE || dv > RING_EDGE;
                let bin = (t / RING_BIN) as usize;
                if r.by_t.len() <= bin {
                    r.by_t.resize(bin + 1, [0; 4]);
                }
                let b = &mut r.by_t[bin];
                b[2] += 1;
                n += 1;
                if z_change {
                    b[3] += 1;
                    base += 1;
                }
                if edge {
                    b[0] += 1;
                    r.edges += 1;
                    if z_change {
                        b[1] += 1;
                        at_z += 1;
                    }
                }
            }
        }
        r.edges_at_z = at_z as f64 / r.edges.max(1) as f64;
        r.base_at_z = base as f64 / n.max(1) as f64;
        r.plateau /= r.runs.max(1) as f64;
        r
    }

    pub fn lines(&self) -> String {
        let mut s = format!(
            "rings: {} edge pixels, {:.3} of them within 1 px of a z-level change, against {:.3} of all hit pixels; plateau mean run {:.2} px over {} runs; |dY| histogram 0..8,9+: {:?}\n",
            self.edges, self.edges_at_z, self.base_at_z, self.plateau, self.runs, self.hist
        );
        for (k, b) in self.by_t.iter().enumerate() {
            if b[2] == 0 {
                continue;
            }
            let p_edge_z = b[1] as f64 / b[3].max(1) as f64;
            let p_edge_not = (b[0] - b[1]) as f64 / (b[2] - b[3]).max(1) as f64;
            let _ = writeln!(
                s,
                "rings t {}..{}: {} px, {} at z change; P(edge | z change) {:.3}, P(edge | none) {:.3}, ratio {:.2}",
                k as f32 * RING_BIN,
                (k + 1) as f32 * RING_BIN,
                b[2],
                b[3],
                p_edge_z,
                p_edge_not,
                p_edge_z / p_edge_not.max(1e-9)
            );
        }
        s
    }
}

// ---- The direction map ----

/// Per display pixel, the store direction of the hit under it (as [`Mask::of`] maps it) seen
/// from `eye`: `2 axis + (n[axis] > 0)`, `n` the entry face's normal, `-sign(p - eye)` on the
/// axis (temporal.wgsl's `store_face`). `None` on a miss or a hit with no entry face.
pub fn directions(hits: &Hits<'_>, display: [u32; 2], eye: [f32; 3]) -> Vec<Option<u8>> {
    let [w, h] = display;
    let [iw, ih] = hits.size;
    let mut out = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        let iy = under(y, h, ih) as u32;
        for x in 0..w {
            let ix = under(x, w, iw) as u32;
            let i = (iy * iw + ix) as usize;
            let (p, info) = (hits.pos[i], hits.info[i]);
            let axis = i32::from((info[3] >> 3) & 3) - 1;
            out.push(if is_hit(info) && p[3] < 1e30 && axis >= 0 {
                let a = axis as usize;
                // The ray runs along `p - eye`; the face's normal points against it.
                let n_positive = p[a] < eye[a];
                Some(2 * a as u8 + u8::from(n_positive))
            } else {
                None
            });
        }
    }
    out
}

/// Per display pixel, whether a hit within one pixel (its 3 x 3) holds another direction than
/// its own. A miss is at no change, and a missing neighbour makes none.
pub fn changes(dirs: &[Option<u8>], display: [u32; 2]) -> Vec<bool> {
    let [w, h] = display;
    let at = |x: i32, y: i32| -> Option<u8> {
        if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
            return None;
        }
        dirs[(y as u32 * w + x as u32) as usize]
    };
    let mut out = Vec::with_capacity(dirs.len());
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let Some(d) = at(x, y) else {
                out.push(false);
                continue;
            };
            out.push(
                (-1..=1).any(|dy| (-1..=1).any(|dx| at(x + dx, y + dy).is_some_and(|n| n != d))),
            );
        }
    }
    out
}

/// The direction map's image: one colour a direction, misses black.
pub fn direction_image(dirs: &[Option<u8>]) -> Vec<u8> {
    const COLOURS: [[u8; 4]; 6] = [
        [200, 60, 60, 255],
        [255, 150, 150, 255],
        [60, 160, 60, 255],
        [150, 255, 150, 255],
        [60, 60, 200, 255],
        [150, 150, 255, 255],
    ];
    dirs.iter()
        .flat_map(|d| d.map_or([0, 0, 0, 255], |d| COLOURS[usize::from(d)]))
        .collect()
}

/// The direction map's edge read on a crop `[x, y, w, h]` of an rgba8 image `w` wide: image-edge
/// pixels (a horizontal or vertical neighbour's luma over [`RING_EDGE`] levels apart, as
/// [`Rings`]) at a direction change against elsewhere, per `t` bin of [`RING_BIN`] voxels
/// (10 m), and beyond `t_from`.
#[derive(Clone, Debug, Default)]
pub struct DirEdges {
    /// Per bin: (edges, edges at a change, hit pixels, hit pixels at a change).
    pub by_t: Vec<[u64; 4]>,
    /// The same over every pixel with `t >= t_from`.
    pub beyond: [u64; 4],
    pub t_from: f32,
}

/// `P(edge | at a change) / P(edge | elsewhere)` of a bin's counts.
pub fn edge_ratio(b: [u64; 4]) -> f64 {
    let at = b[1] as f64 / b[3].max(1) as f64;
    let not = (b[0] - b[1]) as f64 / (b[2] - b[3]).max(1) as f64;
    at / not.max(1e-9)
}

impl DirEdges {
    pub fn read(
        rgba: &[u8],
        dirs: &[Option<u8>],
        at: &[bool],
        t: &[Option<(i32, f32)>],
        w: u32,
        c: [u32; 4],
        t_from: f32,
    ) -> DirEdges {
        let y8 = |x: u32, y: u32| luma8(&rgba[((y * w + x) * 4) as usize..]);
        let mut r = DirEdges {
            t_from,
            ..DirEdges::default()
        };
        for py in c[1]..(c[1] + c[3]).saturating_sub(1) {
            for px in c[0]..(c[0] + c[2]).saturating_sub(1) {
                let i = (py * w + px) as usize;
                let (Some(_), Some((_, ti))) = (dirs[i], t[i]) else {
                    continue;
                };
                let v = y8(px, py);
                let edge = (y8(px + 1, py) - v).abs() > RING_EDGE
                    || (y8(px, py + 1) - v).abs() > RING_EDGE;
                let bin = (ti / RING_BIN) as usize;
                if r.by_t.len() <= bin {
                    r.by_t.resize(bin + 1, [0; 4]);
                }
                let add = |b: &mut [u64; 4]| {
                    b[2] += 1;
                    b[3] += u64::from(at[i]);
                    b[0] += u64::from(edge);
                    b[1] += u64::from(edge && at[i]);
                };
                add(&mut r.by_t[bin]);
                if ti >= t_from {
                    add(&mut r.beyond);
                }
            }
        }
        r
    }

    pub fn lines(&self, what: &str) -> String {
        let mut s = String::new();
        let line = |s: &mut String, label: String, b: [u64; 4]| {
            let _ = writeln!(
                s,
                "dir edges {what} {label}: {} px, {} at a change; edges {} ({} at a change); P(edge | change) {:.3}, P(edge | none) {:.3}, ratio {:.2}",
                b[2],
                b[3],
                b[0],
                b[1],
                b[1] as f64 / b[3].max(1) as f64,
                (b[0] - b[1]) as f64 / (b[2] - b[3]).max(1) as f64,
                edge_ratio(b)
            );
        };
        for (k, b) in self.by_t.iter().enumerate() {
            if b[2] > 0 {
                line(
                    &mut s,
                    format!("t {}..{}", k as f32 * RING_BIN, (k + 1) as f32 * RING_BIN),
                    *b,
                );
            }
        }
        line(
            &mut s,
            format!("t >= {:.1} (the band's start)", self.t_from),
            self.beyond,
        );
        s
    }
}
