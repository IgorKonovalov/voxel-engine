//! The rig's scripted runs over the level and poses: a settle run around one event, the long
//! runs over many seeds, and the wander, each reading the parts of the rig's frames.

use super::*;

pub fn camera_at(position: [f32; 3], yaw_deg: f32, pitch_deg: f32) -> Camera {
    Camera {
        position: position.into(),
        yaw: yaw_deg.to_radians(),
        pitch: pitch_deg.to_radians(),
        vfov: 70f32.to_radians(),
    }
}

/// A settle run: settle `SETTLE` frames, the event, then frames 1 ..= `REF.1` still, its
/// random numbers from frame index `seed` (seed k is `k * SEED_STRIDE`). Only seed 0 reads the
/// dots and writes the filmstrips.
#[expect(
    clippy::too_many_lines,
    reason = "one run's frame loop: the settle, the event and the still frames read every part in one pass"
)]
pub fn settle_run(
    level: &mut VeinLevel,
    pose: &Pose,
    setting: Setting,
    event: Event,
    seed: u32,
    strip_tag: Option<&str>,
) -> SettleReading {
    let strip_tag = strip_tag.filter(|_| seed == 0);
    let mut frame = make_frame(level, pose, setting);
    frame.set_frame_index(seed);
    let light = LightParams::headlamp();
    let end = dig_camera(pose, event);
    if let Event::FlyIn { pinned: true } = event {
        let o = frame.probes().origin_for(pose.position);
        frame.probes_mut().set_origin(Some(o));
    }
    // Before the event: settle, then the motion that ends on frame 0.
    let want_none = Want {
        hits: false,
        composite: false,
        moments: false,
    };
    let mut prev: Option<Pixels> = None;
    let dots_on = event.dots() && seed == 0;
    let want_dots = Want {
        hits: true,
        composite: true,
        moments: true,
    };
    match event {
        Event::FlyIn { .. } => {
            let start = Vec3::from(fly_in_start(pose));
            let to = Vec3::from(pose.position);
            let start_cam = Camera {
                position: start,
                ..end
            };
            render(level, &mut frame, &start_cam, SETTLE);
            let steps = ((to - start).length() / (FLY_IN_SPEED / 60.0)).round() as usize;
            for i in 1..=steps {
                let c = Camera {
                    position: start.lerp(to, i as f32 / steps as f32),
                    ..end
                };
                frame.step(&level.device, &level.queue, &level.gpu, &c, &light);
            }
        }
        Event::Toggle | Event::Dig | Event::DigFar => render(level, &mut frame, &end, SETTLE),
        Event::Yaw { frames, degrees } => {
            let degrees = degrees as f32;
            let start = camera_at(pose.position, pose.yaw_deg - degrees, pose.pitch_deg);
            render(level, &mut frame, &start, SETTLE);
            for i in 1..=frames {
                let c = camera_at(
                    pose.position,
                    pose.yaw_deg - degrees + degrees * i as f32 / frames as f32,
                    pose.pitch_deg,
                );
                let last = i == frames;
                frame.step(&level.device, &level.queue, &level.gpu, &c, &light);
                if last && dots_on {
                    prev = Some(read(level, &frame, c, want_dots));
                }
            }
        }
    }
    let want0 = if dots_on || event.is_dig() {
        want_dots
    } else {
        Want {
            moments: true,
            ..want_none
        }
    };
    let f0 = read(level, &frame, end, want0);
    if prev.is_none() && dots_on {
        prev = Some(read(level, &frame, end, want_dots));
    }
    // The event.
    match event {
        Event::Toggle => frame.set_lighting(Lighting {
            lamp: !setting.lamp,
            ..lighting(pose, setting)
        }),
        Event::Dig | Event::DigFar => {
            dig(level, &mut frame, &f0, event == Event::DigFar);
        }
        _ => {}
    }
    let mut series: Vec<Vec<Vec<f32>>> = vec![Vec::new(); PARTS.len()];
    for (k, &p) in PARTS.iter().enumerate() {
        series[k].push(f0.tiles(p));
    }
    let mut exposure = vec![f0.exposure.log2()];
    let mut window = Vec::new();
    let mut dots = Dots::default();
    let (mut rej, mut age0) = (0u64, 0u64);
    let mut floor_n = 0usize;
    let mut floor_part = [0usize; 7];
    let mut floor_age = [0usize; 5];
    let mut floor_count = 0usize;
    let mut floor_counts: Vec<f64> = Vec::new();
    let mut strip = Vec::new();
    let mut strip_exposure = Vec::new();
    let mut strip_gather = Vec::new();
    let mut strip_age = Vec::new();
    let mut strip_dots: Vec<Vec<usize>> = Vec::new();
    let mut last_full = Vec::new();
    let keep_strip = |f: usize| STRIP_FRAMES.contains(&f);
    if keep_strip(0) {
        strip.push(f0.display.clone());
        strip_exposure.push(f0.exposure);
        strip_gather.push(f0.tiles(Part::Gather));
        strip_age.push(f0.moments.iter().map(|m| m[2]).collect::<Vec<f32>>());
        strip_dots.push(Vec::new());
    }
    for f in 1..=REF.1 {
        frame.step(&level.device, &level.queue, &level.gpu, &end, &light);
        if f <= 64 {
            window.push(frame.probe_light_window());
        }
        let dot_frame = dots_on && (f <= DOT_FRAMES || f >= REF.0);
        let sampled = settle_sampled(f);
        if !(sampled || dot_frame || keep_strip(f)) {
            for s in series.iter_mut() {
                let last = s.last().expect("frame 0 read").clone();
                s.push(last);
            }
            let last = *exposure.last().expect("frame 0 read");
            exposure.push(last);
            continue;
        }
        let want = if dot_frame {
            want_dots
        } else {
            Want {
                moments: keep_strip(f),
                ..want_none
            }
        };
        let px = read(level, &frame, end, want);
        for (k, &p) in PARTS.iter().enumerate() {
            let v = if sampled {
                px.tiles(p)
            } else {
                series[k].last().expect("frame 0 read").clone()
            };
            series[k].push(v);
        }
        let e = if sampled {
            px.exposure.log2()
        } else {
            *exposure.last().expect("frame 0 read")
        };
        exposure.push(e);
        let mut fd = Vec::new();
        if dot_frame {
            let (d, bp, ba) = frame_dots(&px);
            if f <= DOT_FRAMES {
                dots.counts.push(d.len());
                for (a, b) in dots.by_part.iter_mut().zip(bp) {
                    *a += b;
                }
                for (a, b) in dots.by_age.iter_mut().zip(ba) {
                    *a += b;
                }
                let s = warp_shares(&px, prev.as_ref().expect("previous frame read"));
                rej += s[2];
                age0 += s[3];
            } else {
                floor_n += 1;
                floor_count += d.len();
                floor_counts.push(d.len() as f64);
                for (a, b) in floor_part.iter_mut().zip(bp) {
                    *a += b;
                }
                for (a, b) in floor_age.iter_mut().zip(ba) {
                    *a += b;
                }
            }
            fd = d;
        }
        if keep_strip(f) {
            strip.push(px.display.clone());
            strip_exposure.push(px.exposure);
            strip_gather.push(px.tiles(Part::Gather));
            strip_age.push(px.moments.iter().map(|m| m[2]).collect());
            strip_dots.push(fd);
        }
        if f == REF.1 {
            last_full = px.meter.iter().map(|m| m.1).collect();
        }
        if dot_frame {
            prev = Some(px);
        }
    }
    let settle = PARTS
        .iter()
        .enumerate()
        .map(|(k, &p)| (p, settle_of(&series[k], &last_full)))
        .collect();
    let exposure_series: Vec<Vec<f32>> = exposure.iter().map(|&e| vec![e]).collect();
    let exposure_settle = settle_of(&exposure_series, &[true]);
    let dots = dots_on.then(|| {
        let n = floor_n.max(1) as f64;
        dots.floor = floor_count as f64 / n;
        dots.floor_sd = sd(&floor_counts);
        dots.by_part_floor = floor_part.map(|c| c as f64 / n);
        dots.by_age_floor = floor_age.map(|c| c as f64 / n);
        dots.reject_share = rej as f64 / age0.max(1) as f64;
        dots.age0 = age0 as usize;
        dots
    });
    if let Some(tag) = strip_tag {
        let name = format!("{}-{}-{}", pose.name, event.tag(), setting.tag());
        let size = f0.display_size;
        settle_sheet(
            tag,
            &format!("{name}-display"),
            size,
            &strip,
            Some(&strip_dots),
        );
        grey_sheet(tag, &format!("{name}-gather"), &strip_gather, &last_full);
        age_sheet(tag, &format!("{name}-age"), f0.grid, &strip_age);
    }
    SettleReading {
        settle,
        exposure: exposure_settle,
        window,
        dots,
        strip,
        strip_exposure,
        series,
        last_full,
        exposure_log2: exposure,
    }
}

/// The settle's series reads every frame to `SETTLE_DENSE` after the event, then every
/// `SETTLE_SPARSE`-th frame and `REF.1`, holding the last read value between: past
/// `SETTLE_DENSE` a settle frame is late by at most `SETTLE_SPARSE - 1`.
pub const SETTLE_DENSE: usize = 256;
pub const SETTLE_SPARSE: usize = 4;

pub(super) fn settle_sampled(f: usize) -> bool {
    f <= SETTLE_DENSE || f.is_multiple_of(SETTLE_SPARSE) || f == REF.1
}

/// Seed k's frame index: a multiple of the jitter's 16 phases, so every seed's frames take
/// the same jitter offsets.
pub const SEED_STRIDE: u32 = 1 << 20;

/// The step settle's seeds: the smallest power of two from 4 to 32 with
/// `3 sigma_view / sqrt(N) <= |D| / 10`, i.e. `N >= (30 sigma_view / |D|)^2`; `None` (under the
/// wander) when even 32 is short.
pub fn n_step(sigma_view: f32, d: f32) -> Option<u32> {
    let need = (30.0 * sigma_view / d.abs().max(1e-9)).powi(2);
    [4u32, 8, 16, 32].into_iter().find(|&n| n as f32 >= need)
}

/// A part's step settle on the seed-averaged view-mean `M(f)`, frames 0 ..= `REF.1`, log2.
#[derive(Clone, Debug)]
pub struct Step {
    pub m_pre: f32,
    pub m_post: f32,
    /// `M_post - M_pre`.
    pub d: f32,
    /// The first frame after which `M` stays within `0.1 |D|` of `M_post` up to `UNTIL`;
    /// `None` when frame `UNTIL` is outside.
    pub s90: Option<usize>,
    /// `M` every 8 frames from 0 to `UNTIL`.
    pub curve: Vec<f32>,
}

impl Step {
    pub fn of(m: &[f32]) -> Step {
        let m_pre = m[0];
        let m_post = m[REF.0..=REF.1].iter().sum::<f32>() / (REF.1 - REF.0 + 1) as f32;
        let d = m_post - m_pre;
        let tol = 0.1 * d.abs();
        let s90 = match (0..=UNTIL).rev().find(|&f| (m[f] - m_post).abs() > tol) {
            Some(UNTIL) => None,
            Some(f) => Some(f + 1),
            None => Some(0),
        };
        Step {
            m_pre,
            m_post,
            d,
            s90,
            curve: (0..=UNTIL).step_by(8).map(|f| m[f]).collect(),
        }
    }

    pub fn cell(&self) -> String {
        format!(
            "S90 {} D {:+.4} (pre {:.4} post {:.4})",
            self.s90.map_or("none".into(), |s| s.to_string()),
            self.d,
            self.m_pre,
            self.m_post
        )
    }

    pub fn curve_line(&self) -> String {
        let c: Vec<String> = self.curve.iter().map(|v| format!("{v:.3}")).collect();
        c.join(" ")
    }
}

/// The seeds' runs of one event and setting, summed: each part's tile series, and the
/// exposure's log2.
pub struct SeedSum {
    pub n: u32,
    sums: Vec<Vec<Vec<f64>>>,
    exposure: Vec<f64>,
    /// The counted tiles: full in the meter at frame `REF.1` of every seed.
    counted: Vec<bool>,
}

impl SeedSum {
    pub fn new() -> SeedSum {
        SeedSum {
            n: 0,
            sums: Vec::new(),
            exposure: Vec::new(),
            counted: Vec::new(),
        }
    }

    pub fn add(&mut self, r: &SettleReading) {
        if self.n == 0 {
            self.sums = r
                .series
                .iter()
                .map(|s| s.iter().map(|t| vec![0.0; t.len()]).collect())
                .collect();
            self.exposure = vec![0.0; r.exposure_log2.len()];
            self.counted = r.last_full.clone();
        }
        for (a, s) in self.sums.iter_mut().zip(&r.series) {
            for (a, t) in a.iter_mut().zip(s) {
                for (a, &v) in a.iter_mut().zip(t) {
                    *a += f64::from(v);
                }
            }
        }
        for (a, &e) in self.exposure.iter_mut().zip(&r.exposure_log2) {
            *a += f64::from(e);
        }
        for (c, &f) in self.counted.iter_mut().zip(&r.last_full) {
            *c &= f;
        }
        self.n += 1;
    }

    fn mean_series(&self, k: usize) -> Vec<Vec<f32>> {
        let n = f64::from(self.n.max(1));
        self.sums[k]
            .iter()
            .map(|t| t.iter().map(|&v| (v / n) as f32).collect())
            .collect()
    }

    /// Each part's step settle on the seeds' mean view-mean (the mean of the counted tiles
    /// whose seed-mean reference is above `FULL_LOG2`), and its per-tile settle on the seeds'
    /// mean tile series; then the exposure's step settle (log2).
    pub fn settles(&self) -> (Vec<(Part, Step, Settle)>, Step) {
        let parts = PARTS
            .iter()
            .enumerate()
            .map(|(k, &p)| {
                let series = self.mean_series(k);
                let tiles = self.counted.len();
                let reference: Vec<f32> = (0..tiles)
                    .map(|t| {
                        (REF.0..=REF.1).map(|f| series[f][t]).sum::<f32>()
                            / (REF.1 - REF.0 + 1) as f32
                    })
                    .collect();
                let idx: Vec<usize> = (0..tiles)
                    .filter(|&t| self.counted[t] && reference[t] > FULL_LOG2)
                    .collect();
                let m: Vec<f32> = series
                    .iter()
                    .map(|f| idx.iter().map(|&t| f[t]).sum::<f32>() / idx.len().max(1) as f32)
                    .collect();
                (p, Step::of(&m), settle_of(&series, &self.counted))
            })
            .collect();
        let n = f64::from(self.n.max(1));
        let e: Vec<f32> = self.exposure.iter().map(|&v| (v / n) as f32).collect();
        (parts, Step::of(&e))
    }
}

impl Default for SeedSum {
    fn default() -> Self {
        Self::new()
    }
}

/// A long series: settle `SETTLE` frames, the event (toggle or dig), then `frames` still
/// frames; every 128 the gather's, the composite's and the emissive part's mean tile log2
/// over the composite's full tiles.
pub fn long_run(
    level: &mut VeinLevel,
    pose: &Pose,
    setting: Setting,
    event: Event,
    frames: usize,
) -> Vec<(usize, f32, f32, f32)> {
    let mut frame = make_frame(level, pose, setting);
    let cam = pose.camera();
    render(level, &mut frame, &cam, SETTLE);
    match event {
        Event::Toggle => frame.set_lighting(Lighting {
            lamp: !setting.lamp,
            ..lighting(pose, setting)
        }),
        other => panic!("the long series takes the toggle, not {other:?}"),
    }
    let want = Want {
        hits: false,
        composite: false,
        moments: false,
    };
    let mut rows = Vec::new();
    for f in 1..=frames {
        render(level, &mut frame, &cam, 1);
        if f % 128 == 0 {
            let px = read(level, &frame, cam, want);
            let mean = |p: Part| {
                let t = px.tiles(p);
                let full: Vec<f32> = t
                    .iter()
                    .zip(&px.meter)
                    .filter(|(v, m)| m.1 && **v > FULL_LOG2)
                    .map(|(v, _)| *v)
                    .collect();
                full.iter().sum::<f32>() / full.len().max(1) as f32
            };
            rows.push((
                f,
                mean(Part::Gather),
                mean(Part::Composite),
                mean(Part::Emissive),
            ));
        }
    }
    rows
}

/// The wander's series after the toggle: `WANDER_FRAMES` still frames, read from
/// `WANDER_FROM + 1` in blocks of `WANDER_BLOCK` (one jitter cycle, `JITTER_PHASES`).
pub const WANDER_FRAMES: usize = 8192;
pub const WANDER_FROM: usize = 1024;
pub const WANDER_BLOCK: usize = 16;

/// One part's still-camera wander (run (vi)), log2: over the blocks, the standard deviation
/// and range of the view-mean (the mean of the counted tiles' block values), each tile's
/// standard deviation and range (their mean, and the standard deviation's 90th percentile),
/// and the tiles' standard deviation over single frames.
#[derive(Clone, Copy, Debug)]
pub struct Wander {
    pub tiles: usize,
    pub sigma_view: f32,
    pub range_view: f32,
    pub sigma_tile: f32,
    pub sigma_tile_p90: f32,
    pub range_tile: f32,
    pub sigma_tile_frames: f32,
}

impl Wander {
    pub fn line(&self) -> String {
        format!(
            "sigma_view {:.4} R_view {:.4} sigma_tile {:.4} p90 {:.4} R_tile {:.4} sigma_tile(frames) {:.4} ({} tiles)",
            self.sigma_view,
            self.range_view,
            self.sigma_tile,
            self.sigma_tile_p90,
            self.range_tile,
            self.sigma_tile_frames,
            self.tiles
        )
    }
}

/// Each part's wander, `None` where it has no counted tile.
pub type Wanders = Vec<(Part, Option<Wander>)>;

pub(super) fn sd(v: &[f64]) -> f64 {
    let n = v.len().max(1) as f64;
    let m = v.iter().sum::<f64>() / n;
    (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / n).sqrt()
}

pub(super) fn range(v: &[f64]) -> f64 {
    let (lo, hi) = v
        .iter()
        .fold((f64::MAX, f64::MIN), |(lo, hi), &x| (lo.min(x), hi.max(x)));
    hi - lo
}

/// Run (vi): settle `SETTLE` frames with the lamp in the other state, switch it into the
/// setting's state (the window opens), then `WANDER_FRAMES` still frames; each part's
/// [`Wander`] over frames `WANDER_FROM + 1 ..= WANDER_FRAMES`, and the frames a second of the
/// read frames (rendering and reading back). A tile counts where the meter's tile is full at
/// the first and the last read frame and the part's mean over the read frames is above
/// `FULL_LOG2`; a part with no counted tile is `None`.
pub fn wander_run(level: &VeinLevel, pose: &Pose, setting: Setting, seed: u32) -> (Wanders, f64) {
    let off = Setting {
        lamp: !setting.lamp,
        ..setting
    };
    let mut frame = make_frame(level, pose, off);
    frame.set_frame_index(seed);
    let cam = pose.camera();
    render(level, &mut frame, &cam, SETTLE);
    frame.set_lighting(lighting(pose, setting));
    render(level, &mut frame, &cam, WANDER_FROM);
    let want = Want {
        hits: false,
        composite: false,
        moments: false,
    };
    let [tw, th] = SIZE.map(|n| n.div_ceil(TILE));
    let nt = (tw * th) as usize;
    let blocks = (WANDER_FRAMES - WANDER_FROM) / WANDER_BLOCK;
    // Per part: block sums per tile, and per-tile sum and sum of squares over single frames.
    let mut block = vec![vec![vec![0.0f64; nt]; blocks]; PARTS.len()];
    let mut s1 = vec![vec![0.0f64; nt]; PARTS.len()];
    let mut s2 = vec![vec![0.0f64; nt]; PARTS.len()];
    let mut full_first = Vec::new();
    let mut full_last = Vec::new();
    #[allow(
        clippy::disallowed_methods,
        reason = "a reading test's frames a second, not simulation"
    )]
    let t0 = std::time::Instant::now();
    for i in 0..blocks * WANDER_BLOCK {
        render(level, &mut frame, &cam, 1);
        let px = read(level, &frame, cam, want);
        let full: Vec<bool> = px.meter.iter().map(|m| m.1).collect();
        if i == 0 {
            full_first = full.clone();
        }
        full_last = full;
        for (k, &p) in PARTS.iter().enumerate() {
            for (t, v) in px.tiles(p).into_iter().enumerate() {
                let v = f64::from(v);
                block[k][i / WANDER_BLOCK][t] += v / WANDER_BLOCK as f64;
                s1[k][t] += v;
                s2[k][t] += v * v;
            }
        }
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a reading test's frames a second, not simulation"
    )]
    let fps = (blocks * WANDER_BLOCK) as f64 / t0.elapsed().as_secs_f64();
    let n = (blocks * WANDER_BLOCK) as f64;
    let out = PARTS
        .iter()
        .enumerate()
        .map(|(k, &p)| {
            let idx: Vec<usize> = (0..nt)
                .filter(|&t| full_first[t] && full_last[t] && s1[k][t] / n > f64::from(FULL_LOG2))
                .collect();
            if idx.is_empty() {
                return (p, None);
            }
            let view: Vec<f64> = (0..blocks)
                .map(|b| idx.iter().map(|&t| block[k][b][t]).sum::<f64>() / idx.len() as f64)
                .collect();
            let per: Vec<(f64, f64)> = idx
                .iter()
                .map(|&t| {
                    let x: Vec<f64> = (0..blocks).map(|b| block[k][b][t]).collect();
                    (sd(&x), range(&x))
                })
                .collect();
            let mut sds: Vec<f32> = per.iter().map(|&(s, _)| s as f32).collect();
            let k_t = idx.len() as f64;
            let frames_sd = idx
                .iter()
                .map(|&t| {
                    let m = s1[k][t] / n;
                    (s2[k][t] / n - m * m).max(0.0).sqrt()
                })
                .sum::<f64>()
                / k_t;
            let w = Wander {
                tiles: idx.len(),
                sigma_view: sd(&view) as f32,
                range_view: range(&view) as f32,
                sigma_tile: (per.iter().map(|p| p.0).sum::<f64>() / k_t) as f32,
                sigma_tile_p90: {
                    sds.sort_by(f32::total_cmp);
                    sds[((sds.len() - 1) as f32 * 0.9).round() as usize]
                },
                range_tile: (per.iter().map(|p| p.1).sum::<f64>() / k_t) as f32,
                sigma_tile_frames: frames_sd as f32,
            };
            (p, Some(w))
        })
        .collect();
    (out, fps)
}

/// The worktree's `live/filmstrip/<tag>/` directory.
pub fn strip_dir(tag: &str) -> std::path::PathBuf {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = root.canonicalize().unwrap_or(root);
    let dir = root.join("live/filmstrip").join(tag);
    std::fs::create_dir_all(&dir).expect("create the filmstrip directory");
    dir
}

pub(super) const GAP: u32 = 2;
pub(super) const DOT_MARK: [u8; 4] = [255, 0, 255, 255];

/// A sheet of `rows` of images, each row a list of (`w`, `h`, rgba) panels, laid out left to
/// right with `GAP` between.
pub(super) fn sheet(rows: &[Vec<(u32, u32, Vec<u8>)>]) -> (Vec<u8>, u32, u32) {
    let sw = rows
        .iter()
        .map(|r| r.iter().map(|p| p.0 + GAP).sum::<u32>())
        .max()
        .unwrap_or(1);
    let sh = rows
        .iter()
        .map(|r| r.iter().map(|p| p.1).max().unwrap_or(0) + GAP)
        .sum::<u32>()
        .max(1);
    let mut out = vec![0u8; (sw * sh * 4) as usize];
    let mut oy = 0;
    for r in rows {
        let mut ox = 0;
        for (w, h, px) in r {
            for y in 0..*h {
                let s = ((y * w) * 4) as usize;
                let d = (((oy + y) * sw + ox) * 4) as usize;
                out[d..d + (*w * 4) as usize].copy_from_slice(&px[s..s + (*w * 4) as usize]);
            }
            ox += w + GAP;
        }
        oy += r.iter().map(|p| p.1).max().unwrap_or(0) + GAP;
    }
    (out, sw, sh)
}

pub(super) fn write_sheet(tag: &str, name: &str, rows: &[Vec<(u32, u32, Vec<u8>)>], what: &str) {
    let (px, w, h) = sheet(rows);
    let path = strip_dir(tag).join(format!("{name}.png"));
    write_png(&path, &px, w, h);
    println!("filmstrip: {} ({what})", path.display());
}

/// The display images at `STRIP_FRAMES`, 7 to a row, dots marked.
pub(super) fn settle_sheet(
    tag: &str,
    name: &str,
    size: [u32; 2],
    images: &[Vec<u8>],
    dots: Option<&[Vec<usize>]>,
) {
    let panels: Vec<(u32, u32, Vec<u8>)> = images
        .iter()
        .enumerate()
        .map(|(k, im)| {
            let mut im = im.clone();
            if let Some(d) = dots {
                for &i in &d[k] {
                    im[i * 4..i * 4 + 4].copy_from_slice(&DOT_MARK);
                }
            }
            (size[0], size[1], im)
        })
        .collect();
    let rows: Vec<_> = panels.chunks(7).map(<[_]>::to_vec).collect();
    write_sheet(
        tag,
        name,
        &rows,
        &format!("display after the event at frames {STRIP_FRAMES:?}, row by row, dots magenta"),
    );
}

/// Tiles as grey from the sheet's lowest counted mean to its highest.
pub(super) fn grey_sheet(tag: &str, name: &str, tiles: &[Vec<f32>], counted: &[bool]) {
    let [tw, _] = SIZE.map(|n| n.div_ceil(TILE));
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for t in tiles {
        for (i, &m) in t.iter().enumerate() {
            if counted[i] && m > FULL_LOG2 {
                lo = lo.min(m);
                hi = hi.max(m);
            }
        }
    }
    let panels: Vec<(u32, u32, Vec<u8>)> = tiles
        .iter()
        .map(|t| {
            let mut im = vec![0u8; (SIZE[0] * SIZE[1] * 4) as usize];
            for y in 0..SIZE[1] {
                for x in 0..SIZE[0] {
                    let k = ((y / TILE) * tw + x / TILE) as usize;
                    let g = if counted[k] {
                        ((t[k] - lo) / (hi - lo).max(1e-6) * 255.0).clamp(0.0, 255.0) as u8
                    } else {
                        0
                    };
                    let i = ((y * SIZE[0] + x) * 4) as usize;
                    im[i..i + 4].copy_from_slice(&[g, g, g, 255]);
                }
            }
            (SIZE[0], SIZE[1], im)
        })
        .collect();
    let rows: Vec<_> = panels.chunks(7).map(<[_]>::to_vec).collect();
    write_sheet(
        tag,
        name,
        &rows,
        &format!("gather tile means grey from {lo:.3} to {hi:.3} log2, frames {STRIP_FRAMES:?}"),
    );
}

/// Pixel ages as grey, black at 0 to white at 64 and over.
pub(super) fn age_sheet(tag: &str, name: &str, grid: [u32; 2], ages: &[Vec<f32>]) {
    let panels: Vec<(u32, u32, Vec<u8>)> = ages
        .iter()
        .map(|a| {
            let im = a
                .iter()
                .flat_map(|&v| {
                    let g = (v.min(64.0) / 64.0 * 255.0) as u8;
                    [g, g, g, 255]
                })
                .collect();
            (grid[0], grid[1], im)
        })
        .collect();
    let rows: Vec<_> = panels.chunks(7).map(<[_]>::to_vec).collect();
    write_sheet(
        tag,
        name,
        &rows,
        &format!("lighting-grid age, black 0 to white 64+, frames {STRIP_FRAMES:?}"),
    );
}

/// The Auto run's display row beneath the pinned run's.
pub fn auto_sheet(tag: &str, name: &str, size: [u32; 2], pinned: &[Vec<u8>], auto: &[Vec<u8>]) {
    let row = |im: &[Vec<u8>]| -> Vec<(u32, u32, Vec<u8>)> {
        im.iter().map(|i| (size[0], size[1], i.clone())).collect()
    };
    write_sheet(
        tag,
        name,
        &[row(pinned), row(auto)],
        &format!("pinned exposure row over the Auto row, frames {STRIP_FRAMES:?}"),
    );
}

/// The pose list.
pub const POSES: [Pose; 2] = [PATCH_POSE, VEIN_POSE];

/// A hit's cube face: its voxel `floor(p - 0.5 nf)` and entry face (2 axis + 1 toward +axis).
pub type FaceKey = ([i32; 3], u8);

/// The face key of a hit (position with t, record) seen from `eye`; `None` for a miss, a ray
/// that started in rock (no entry face), a field hit or a hit behind translucent layers. The entry face's axis is the record's
/// (flag bits 3..5, less 1), its outward side the one holding the eye: the shading normal can
/// lean away from the face under the far normal.
pub fn face_key(eye: Vec3, pos: [f32; 4], info: [u16; 4]) -> Option<FaceKey> {
    let flags = info[3];
    // A miss, a field-surface hit or a hit behind translucent layers has no cell.
    if flags & 1 == 0 || flags & 6 != 0 {
        return None;
    }
    let axis = i32::from((flags >> 3) & 3) - 1;
    if axis < 0 {
        return None;
    }
    let axis = axis as usize;
    let positive = eye.to_array()[axis] > pos[axis];
    let mut p = [pos[0], pos[1], pos[2]];
    p[axis] -= if positive { 0.5 } else { -0.5 };
    Some((
        p.map(|v| v.floor() as i32),
        (2 * axis + usize::from(positive)) as u8,
    ))
}

/// The world-keyed flicker over a run: frames go in as (key, mean luma), each key's mean luma
/// that frame. Reads of a key absent from the frame before count nothing.
#[derive(Default)]
pub struct KeyFlicker {
    prev: HashMap<FaceKey, f32>,
    samples: Vec<f32>,
    frame_keys: Vec<usize>,
}

impl KeyFlicker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a frame; `counted` false only updates the previous frame's means (a frame before
    /// the run's reading window).
    pub fn add(&mut self, keys: HashMap<FaceKey, f32>, counted: bool) {
        if counted {
            let mut n = 0;
            for (k, v) in &keys {
                if let Some(q) = self.prev.get(k) {
                    self.samples.push((meter_log2(*v) - meter_log2(*q)).abs());
                    n += 1;
                }
            }
            self.frame_keys.push(n);
        }
        self.prev = keys;
    }

    /// `K`: the mean over frames and keys.
    pub fn k(&self) -> f32 {
        (self.samples.iter().map(|&s| f64::from(s)).sum::<f64>() / self.samples.len().max(1) as f64)
            as f32
    }

    pub fn p95(&self) -> f32 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let mut v = self.samples.clone();
        let k = ((v.len() as f64 * 0.95) as usize).min(v.len() - 1);
        *v.select_nth_unstable_by(k, f32::total_cmp).1
    }

    pub fn cell(&self) -> String {
        format!(
            "K {:.4} p95 {:.4} ({} key-frames, {:.0} keys a frame)",
            self.k(),
            self.p95(),
            self.samples.len(),
            self.samples.len() as f64 / self.frame_keys.len().max(1) as f64,
        )
    }
}

/// The frame's face keys with their pixels' mean luma as seen.
pub(super) fn frame_keys(px: &Pixels) -> HashMap<FaceKey, f32> {
    let mut sums: HashMap<FaceKey, (f64, u32)> = HashMap::new();
    for (i, c) in px.composite.iter().enumerate() {
        if let Some(k) = face_key(px.camera.position, px.hit_pos[i], px.hit_info[i]) {
            let e = sums.entry(k).or_default();
            e.0 += f64::from(seen_lum(c));
            e.1 += 1;
        }
    }
    sums.into_iter()
        .map(|(k, (s, n))| (k, (s / f64::from(n)) as f32))
        .collect()
}

/// A run the flicker reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum KRun {
    /// The rig's fast yaw: 90 degrees onto the pose over 10 frames, then still.
    Yaw,
    /// `Flight::Path`: the steady flight, facing the travel.
    Path,
}

impl KRun {
    pub fn name(self) -> &'static str {
        match self {
            KRun::Yaw => "yaw 10f",
            KRun::Path => "flight",
        }
    }

    /// The camera at run frame `f` (0 the start, settled).
    pub fn camera(self, pose: &Pose, f: usize) -> Camera {
        match self {
            KRun::Yaw => camera_at(
                pose.position,
                pose.yaw_deg - 90.0 + 90.0 * (f.min(10) as f32) / 10.0,
                pose.pitch_deg,
            ),
            KRun::Path => {
                let (start, end) = flight_path(pose);
                let (start, end) = (Vec3::from(start), Vec3::from(end));
                let dir = (end - start).normalize();
                let len = (end - start).length();
                let s = (FLY_SPEED / 60.0 * f as f32).min(len);
                Camera {
                    position: start + dir * s,
                    yaw: dir.y.atan2(dir.x),
                    pitch: dir.z.asin(),
                    vfov: 70f32.to_radians(),
                }
            }
        }
    }
}

/// The flicker of a run: settle `SETTLE` frames at its start, then frames 1 .. `FLIGHT_FRAMES`;
/// `K` over frames `SHIMMER_FROM ..`, and over the frames before them (the yaw's own motion).
pub fn flicker_run(
    level: &VeinLevel,
    pose: &Pose,
    setting: Setting,
    run: KRun,
) -> (KeyFlicker, KeyFlicker) {
    let mut frame = make_frame(level, pose, setting);
    let light = LightParams::headlamp();
    render(level, &mut frame, &run.camera(pose, 0), SETTLE);
    let want = Want {
        hits: true,
        composite: true,
        moments: false,
    };
    let (mut late, mut early) = (KeyFlicker::new(), KeyFlicker::new());
    for f in 1..FLIGHT_FRAMES {
        let cam = run.camera(pose, f);
        frame.step(&level.device, &level.queue, &level.gpu, &cam, &light);
        let keys = frame_keys(&read(level, &frame, cam, want));
        early.add(keys.clone(), f < SHIMMER_FROM);
        late.add(keys, f >= SHIMMER_FROM);
    }
    (late, early)
}

/// The strip flight's dots: the display's count per flown frame, the floor's mean and spread
/// over `STRIP_FLOOR_FRAMES` still frames at the start, and the counts by part over the flight
/// and per floor frame.
pub struct StripDots {
    pub counts: Vec<usize>,
    pub floor: f64,
    pub floor_sd: f64,
    pub by_part: [usize; 7],
    pub by_part_floor: [f64; 7],
}

impl StripDots {
    pub fn line(&self) -> String {
        let n = self.counts.len() as f64;
        let part: Vec<String> = BY_PART
            .iter()
            .enumerate()
            .map(|(i, name)| {
                format!(
                    "{name} {:.0}",
                    self.by_part[i] as f64 - n * self.by_part_floor[i]
                )
            })
            .collect();
        let every: Vec<String> = self
            .counts
            .iter()
            .step_by(12)
            .map(|c| c.to_string())
            .collect();
        format!(
            "excess sum {:.0} (floor {:.1}/frame, sd {:.1}); by part [{}]; dots every 12 frames [{}]",
            self.counts
                .iter()
                .map(|&c| c as f64 - self.floor)
                .sum::<f64>(),
            self.floor,
            self.floor_sd,
            part.join(", "),
            every.join(" ")
        )
    }
}

/// The strip tool's flight (`tests/gpu_strip.rs`): along the view at `STRIP_SPEED` with the yaw at
/// `STRIP_YAW`, from a start settled `SETTLE` frames. The camera may enter rock, as the tool's
/// does.
pub fn strip_dots(level: &VeinLevel, pose: &Pose, setting: Setting) -> StripDots {
    let mut camera = pose.camera();
    const DT: f32 = 1.0 / 60.0;
    flown_dots(level, pose, setting, camera, STRIP_FRAMES_FLOWN, |_| {
        let (fwd, _, _) = camera.basis();
        camera.position += fwd * STRIP_SPEED * DT;
        camera.yaw += STRIP_YAW.to_radians() * DT;
        camera
    })
}

/// The same read along the rig's steady flight, whose path has air around every step.
pub fn path_dots(level: &VeinLevel, pose: &Pose, setting: Setting) -> StripDots {
    let start = KRun::Path.camera(pose, 0);
    flown_dots(level, pose, setting, start, FLIGHT_FRAMES - 1, |f| {
        KRun::Path.camera(pose, f + 1)
    })
}

pub(super) fn flown_dots(
    level: &VeinLevel,
    pose: &Pose,
    setting: Setting,
    start: Camera,
    frames: usize,
    mut at: impl FnMut(usize) -> Camera,
) -> StripDots {
    let mut frame = make_frame(level, pose, setting);
    let camera = start;
    render(level, &mut frame, &camera, SETTLE);
    let want = Want {
        hits: false,
        composite: true,
        moments: true,
    };
    let mut floor = Vec::new();
    let mut part_floor = [0usize; 7];
    for _ in 0..STRIP_FLOOR_FRAMES {
        render(level, &mut frame, &camera, 1);
        let (d, bp, _) = frame_dots(&read(level, &frame, camera, want));
        floor.push(d.len() as f64);
        for (a, b) in part_floor.iter_mut().zip(bp) {
            *a += b;
        }
    }
    let mut out = StripDots {
        counts: Vec::new(),
        floor: floor.iter().sum::<f64>() / floor.len() as f64,
        floor_sd: sd(&floor),
        by_part: [0; 7],
        by_part_floor: part_floor.map(|c| c as f64 / STRIP_FLOOR_FRAMES as f64),
    };
    for f in 0..frames {
        let camera = at(f);
        render(level, &mut frame, &camera, 1);
        let (d, bp, _) = frame_dots(&read(level, &frame, camera, want));
        out.counts.push(d.len());
        for (a, b) in out.by_part.iter_mut().zip(bp) {
            *a += b;
        }
    }
    out
}

/// The level's spawn as a pose facing +x, as the strip tool has it, at `exposure`.
pub fn spawn_pose(exposure: f32) -> Pose {
    let source = flythrough::cave::CaveSource::new(
        1,
        ChunkExtent::new([0, 0, 0], flythrough::cave::LEVEL_CHUNKS),
    );
    Pose {
        name: "spawn",
        position: source.spawn().0.map(|v| v as f32 + 0.5),
        yaw_deg: 0.0,
        pitch_deg: 0.0,
        exposure,
    }
}

/// The eye's exposure at `pose` after `frames` still frames at `setting`, `Auto` whatever the
/// setting says.
pub fn eye_exposure(level: &VeinLevel, pose: &Pose, setting: Setting, frames: usize) -> f32 {
    let auto = Setting {
        auto: true,
        ..setting
    };
    let mut frame = make_frame(level, pose, auto);
    render(level, &mut frame, &pose.camera(), frames);
    let st =
        vxe_render::debug::read_buffer(&level.device, &level.queue, frame.exposure_state(), 0, 32);
    ExposureState::from_bytes(&st).previous
}

/// The eye's exposure at `pose` after 600 frames lamp on and 900 off, under `Auto`, at the
/// default look with the lamp channel's moving-lamp blend `probe_move` (`None`: the default).
pub fn eye_after_lamp_off(level: &VeinLevel, pose: &Pose, probe_move: Option<f32>) -> f32 {
    let on = Setting {
        lamp: true,
        indirect: true,
        auto: true,
        upscaler: true,
        bridge: false,
        probe_freeze: None,
        probe_move,
    };
    let mut frame = make_frame(level, pose, on);
    render(level, &mut frame, &pose.camera(), 600);
    frame.set_lighting(lighting(pose, Setting { lamp: false, ..on }));
    render(level, &mut frame, &pose.camera(), 900);
    let st =
        vxe_render::debug::read_buffer(&level.device, &level.queue, frame.exposure_state(), 0, 32);
    ExposureState::from_bytes(&st).previous
}

/// The probe volume's state at rest: of the slots holding their probe (traced), how many are
/// inside rock.
pub fn probe_census(level: &VeinLevel, frame: &Frame) -> (u32, u32) {
    use vxe_render::internals::MAX_AGE;
    let v = frame.probes();
    let b = vxe_render::debug::read_buffer(
        &level.device,
        &level.queue,
        v.data(),
        u64::from(v.state_base()) * 4,
        u64::from(v.count()) * 4,
    );
    let (mut traced, mut inside) = (0, 0);
    for c in b.chunks_exact(4) {
        let s = u32::from_le_bytes(c.try_into().expect("a word"));
        if (s >> 1) & MAX_AGE > 0 {
            traced += 1;
            inside += s & 1;
        }
    }
    (traced, inside)
}

/// The volume's state against a freeze count: traced, inside rock, frozen (at the count or
/// over, not inside), and of the probes in `pose`'s view within `VIEW_REACH` voxels the same
/// three.
pub struct FreezeCensus {
    pub all: [u32; 3],
    pub in_view: [u32; 3],
}

/// How far a probe counts as in view: the gather's reach at the volume's spacing.
pub const VIEW_REACH: f32 = 128.0;

pub fn freeze_census(
    level: &VeinLevel,
    frame: &Frame,
    camera: &Camera,
    freeze: u32,
) -> FreezeCensus {
    use vxe_render::internals::MAX_AGE;
    let v = frame.probes();
    let b = vxe_render::debug::read_buffer(
        &level.device,
        &level.queue,
        v.data(),
        u64::from(v.state_base()) * 4,
        u64::from(v.count()) * 4,
    );
    let word = |s: u32| {
        u32::from_le_bytes(
            b[s as usize * 4..s as usize * 4 + 4]
                .try_into()
                .expect("a word"),
        )
    };
    let origin = frame.probes().origin_for(camera.position.to_array());
    let n = v.axis() as i32;
    let mut c = FreezeCensus {
        all: [0; 3],
        in_view: [0; 3],
    };
    for z in 0..n {
        for y in 0..n {
            for x in 0..n {
                let w = [origin[0] + x, origin[1] + y, origin[2] + z];
                let s = word(v.slot(w));
                let age = (s >> 1) & MAX_AGE;
                if age == 0 {
                    continue;
                }
                let inside = s & 1 != 0;
                let flags = [true, inside, !inside && age >= freeze];
                let p = Vec3::from(frame.probes().probe_position(w));
                let seen = (p - camera.position).length() < VIEW_REACH
                    && project(camera, SIZE, p).is_some();
                for (i, &f) in flags.iter().enumerate() {
                    c.all[i] += u32::from(f);
                    c.in_view[i] += u32::from(f && seen);
                }
            }
        }
    }
    c
}

/// One frame at `camera` under `timer`; the milliseconds of each of `FRAME_PASSES`.
pub fn timed_step(
    level: &VeinLevel,
    frame: &mut Frame,
    timer: &vxe_render::PassTimer,
    camera: &Camera,
) -> Vec<f64> {
    let light = LightParams::headlamp();
    frame.advance();
    let params = frame.params(&level.gpu, camera, &light);
    let mut enc = level
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    frame.encode(
        &level.device,
        &level.queue,
        &mut enc,
        &level.gpu,
        &params,
        Some(timer),
    );
    timer.resolve(&mut enc);
    level.queue.submit([enc.finish()]);
    timer.read_all_ms(&level.device)
}

/// The per-pass cost of `run` at 1080p from 720p (the frame `docs/nfr/02-frame-budget.md`
/// ledgers): each pass's median over the run's frames, from a start settled `settle` frames.
pub fn flight_cost(
    level: &VeinLevel,
    pose: &Pose,
    lighting: Lighting,
    run: KRun,
    settle: usize,
) -> Option<Vec<(&'static str, f64)>> {
    use vxe_render::FRAME_PASSES;
    let (device, queue) = (&level.device, &level.queue);
    let timer = vxe_render::PassTimer::with_passes(device, queue, FRAME_PASSES.len() as u32)?;
    let output = [1920, 1080];
    let mut frame = Frame::with_sizes(
        device,
        vxe_render::RenderScale::OneHalf.internal(output),
        output,
    );
    frame.set_upscaler_kind(device, start_upscaler());
    frame.set_look(look_of(LookPreset::DEFAULT));
    frame.set_lighting(lighting);
    frame.set_dt(1.0 / 60.0);
    render(level, &mut frame, &run.camera(pose, 0), settle);
    let mut ms = vec![Vec::new(); FRAME_PASSES.len()];
    for f in 1..FLIGHT_FRAMES {
        let t = timed_step(level, &mut frame, &timer, &run.camera(pose, f));
        for (a, v) in ms.iter_mut().zip(t) {
            a.push(v);
        }
    }
    Some(
        FRAME_PASSES
            .iter()
            .zip(ms)
            .map(|(&n, mut v)| {
                v.sort_by(f64::total_cmp);
                (n, v[v.len() / 2])
            })
            .collect(),
    )
}

/// The camera `degrees` of yaw from `pose`.
pub fn yawed(pose: &Pose, degrees: f32) -> Camera {
    camera_at(pose.position, pose.yaw_deg + degrees, pose.pitch_deg)
}

// ---- The creases ----

/// A yellow display pixel (Plan 0056's rule): red and green both over twice blue, and green
/// over half of red.
pub fn yellow(p: &[u8]) -> bool {
    let (r, g, b) = (u32::from(p[0]), u32::from(p[1]), u32::from(p[2]));
    r > 2 * b && g > 2 * b && 2 * g > r
}

/// The yaw in radians that moves the image centre one display pixel, for a view `height`
/// pixels high (Plan 0056's pan: `atan(1 / focal) / cos(pitch)`).
pub fn yaw_per_pixel(height: u32, vfov: f32, pitch: f32) -> f32 {
    let focal = height as f32 * 0.5 / (vfov * 0.5).tan();
    (1.0 / focal).atan() / pitch.cos()
}
