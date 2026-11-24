//! Plan 0098 Phase 9: the sunlit terraces at noon, a reading. The window's noon slope
//! (`--source outdoor`, trees sparse, `--time 12:00`, the outside spawn facing +x: x -299.5 y
//! 0.5 z 549.5), the `VXE_WOBBLE` arm over the rig's frame. The class is the step shadows:
//! display pixels whose lighting pixel is on a face the key light falls on and whose 3 x 3
//! lighting neighbours (each on such a face) hold key visibilities on both sides of one half
//! (the key term's shadow edge, about 3 display pixels either way at 1.5 over a half-rate
//! lighting grid), split by the far normal's weight at the hit (0, between, 1). Per frame pair
//! the share of the class moving over 4 levels of luma, and of those which term moved at the
//! lighting pixel: the key visibility (the pair's g, by over 0.05), else the direct part (the key
//! term's cosine or its hit, by over 5 %), else the sky's shown visibility (by over 0.02), else
//! none (the upscaler's own). The protocol: 600 settle frames with the queue stepping, the
//! exposure pinned at its settled value, the queue stopped, 32 still frames (the jitter's
//! phases), then a fly of 120 frames at 0.5 m/s. Nothing is asserted.

use super::*;

/// The settle's frames, the still's and the fly's, and the fly's speed in m/s.
const SETTLE_T: usize = 600;
const STILL_T: usize = 32;
const FLY_T: usize = 120;
const FLY_M_S: f32 = 0.5;

/// The outdoor source of seed 1 with sparse trees (the fly-through's `--source outdoor`), the
/// chunks within 4 of the outside spawn's (2 down, 3 up), the spawn's camera.
fn slope_rig() -> Option<(Rig, Camera)> {
    use flythrough::outdoor::{OutdoorSource, Spawn};
    use flythrough::trees::{ForestSource, LEAF_FILL, TreeDensity};
    use svet_world::{ChunkExtent, World};
    let source = ForestSource::new(OutdoorSource::new(1), TreeDensity::Sparse, LEAF_FILL);
    let (eye, facing) = source.base().spawn(Spawn::Outside);
    let c = eye.chunk().0;
    let extent = ChunkExtent::new(
        [c[0] - 4, c[1] - 4, c[2] - 2],
        [c[0] + 5, c[1] + 5, c[2] + 3],
    );
    let world = World::generate(extent, &source, &flythrough::outdoor_materials());
    let rig = support::terms::mirrored(world, eye)?;
    let p = eye.0.map(|v| v as f32 + 0.5);
    let d = facing.map(|v| v as f32);
    let cam = flythrough::camera_looking_at(p, [p[0] + d[0], p[1] + d[1], p[2] + d[2]]);
    Some((rig, cam))
}

/// One frame's reads per lighting pixel: the key visibility (the pair's g decoded), whether the
/// key light falls on the face, the direct part's luminance, the sky's shown visibility and the
/// far normal's weight; the display's luma and the grid's width.
struct Read {
    key: Vec<f32>,
    lit: Vec<bool>,
    direct: Vec<f32>,
    sky: Vec<f32>,
    w: Vec<f32>,
    luma: Vec<u8>,
    gw: usize,
    light_ms: f64,
    temporal_ms: f64,
}

fn read_frame(rig: &Rig, s: &mut Stepper, cam: &Camera, arm: &Arm, internal: [u32; 2]) -> Read {
    use svet_render::debug::{read_rgba8, read_rgba16f};
    let shot = s.step(rig, cam);
    let (d, q) = (&rig.device, &rig.queue);
    let f = &s.frame;
    let pair = read_rgba16f(d, q, f.visibility_pair());
    let vis = read_rgba8(d, q, f.visibility_samples());
    let dir = read_rgba16f(d, q, f.direct_accumulated());
    let gw = f.light_hit_positions().width() as usize;
    let eye = cam.position.to_array();
    let tan_h = (cam.vfov * 0.5).tan() * internal[0] as f32 / internal[1] as f32;
    let [lo, hi] = arm.far_normal;
    let weight = |h: &[f32; 4]| {
        if h[3] >= 1e30 || !h[3].is_finite() {
            return -1.0;
        }
        let t = ((h[0] - eye[0]).powi(2) + (h[1] - eye[1]).powi(2) + (h[2] - eye[2]).powi(2))
            .sqrt()
            .max(1e-3);
        if hi <= lo {
            return 0.0;
        }
        let px = internal[0] as f32 / (2.0 * tan_h * t);
        let x = ((px - lo) / (hi - lo)).clamp(0.0, 1.0);
        1.0 - x * x * (3.0 - 2.0 * x)
    };
    let ms = |name: &str| {
        svet_render::FRAME_PASSES
            .iter()
            .position(|&p| p == name)
            .and_then(|i| shot.passes.get(i).copied())
            .unwrap_or(f64::NAN)
    };
    Read {
        key: pair
            .iter()
            .map(|p| support::defects::key_slot(p[1]).0)
            .collect(),
        lit: vis.chunks(4).map(|p| p[2] > 0).collect(),
        direct: dir
            .iter()
            .map(|p| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2])
            .collect(),
        sky: pair.iter().map(|p| p[0]).collect(),
        w: shot.hits.iter().map(weight).collect(),
        luma: lumas(&shot.display),
        gw,
        light_ms: ms("light"),
        temporal_ms: ms("temporal"),
    }
}

/// Per lighting pixel of `r`: 0, 1 or 2 by the far weight (0, between, 1) where it lies on a
/// shadow edge of the key term, else 255.
fn edge_class(r: &Read) -> Vec<u8> {
    let gw = r.gw;
    let gh = r.key.len() / gw;
    (0..r.key.len())
        .map(|g| {
            if !r.lit[g] || r.w[g] < 0.0 {
                return 255;
            }
            let (x, y) = ((g % gw) as i64, (g / gw) as i64);
            let (mut lo, mut hi) = (f32::MAX, f32::MIN);
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx < 0 || ny < 0 || nx >= gw as i64 || ny >= gh as i64 {
                        continue;
                    }
                    let n = (ny as usize) * gw + nx as usize;
                    if r.lit[n] {
                        lo = lo.min(r.key[n]);
                        hi = hi.max(r.key[n]);
                    }
                }
            }
            if !(lo < 0.5 && hi >= 0.5) {
                return 255;
            }
            if r.w[g] <= 0.01 {
                0
            } else if r.w[g] >= 0.99 {
                2
            } else {
                1
            }
        })
        .collect()
}

/// A pair's figures per weight bin: the class's display pixels, those over 4 levels, and of
/// those the term that moved (key, direct, sky, none).
#[derive(Clone, Copy, Default)]
struct PairRow {
    n: [u64; 3],
    over: [u64; 3],
    by: [[u64; 4]; 3],
}

fn pair_row(a: &Read, b: &Read) -> PairRow {
    let cls = edge_class(a);
    let (dw, dh) = (OUT[0] as usize, OUT[1] as usize);
    let gl = a.key.len();
    let mut r = PairRow::default();
    for i in 0..dw * dh {
        let g = grid_of(i, dw, dh, a.gw, gl);
        let c = cls[g];
        if c == 255 {
            continue;
        }
        let c = usize::from(c);
        r.n[c] += 1;
        if a.luma[i].abs_diff(b.luma[i]) <= 4 {
            continue;
        }
        r.over[c] += 1;
        let dd =
            (a.direct[g] - b.direct[g]).abs() / a.direct[g].abs().max(b.direct[g].abs()).max(1e-6);
        let k = if (a.key[g] - b.key[g]).abs() > 0.05 {
            0
        } else if dd > 0.05 {
            1
        } else if (a.sky[g] - b.sky[g]).abs() > 0.02 {
            2
        } else {
            3
        };
        r.by[c][k] += 1;
    }
    r
}

/// The segment's mean share over 4 per bin and the terms' shares of the moved pixels, summed
/// over its pairs.
fn segment_line(name: &str, rows: &[PairRow]) -> String {
    let bins = ["w 0", "w between", "w 1"];
    let mut out = format!("{name} ({} pairs):", rows.len());
    for (c, bin) in bins.iter().enumerate() {
        let n: u64 = rows.iter().map(|r| r.n[c]).sum();
        let over: u64 = rows.iter().map(|r| r.over[c]).sum();
        let by: [u64; 4] = std::array::from_fn(|k| rows.iter().map(|r| r.by[c][k]).sum());
        let m = over.max(1) as f64;
        out.push_str(&format!(
            " {}: {:.2} % of {} px a pair (key {:.0} %, direct {:.0} %, sky {:.0} %, none {:.0} %);",
            bin,
            100.0 * over as f64 / n.max(1) as f64,
            n / rows.len().max(1) as u64,
            100.0 * by[0] as f64 / m,
            100.0 * by[1] as f64 / m,
            100.0 * by[2] as f64 / m,
            100.0 * by[3] as f64 / m,
        ));
    }
    out
}

mod reading {
    use super::*;

    /// The reading's directory, made.
    fn dir_of() -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../live/wobble/terraces");
        std::fs::create_dir_all(&dir).expect("the reading's directory");
        dir
    }

    /// The last frame's display.
    fn reads_png(rig: &Rig, s: &Stepper) -> Vec<u8> {
        svet_render::debug::read_rgba8(&rig.device, &rig.queue, s.frame.target())
    }

    /// The reading: the arm `VXE_WOBBLE` names on the noon slope, its still and fly lines and
    /// pass times printed and written to `live/wobble/terraces/<arm>.txt`.
    #[test]
    #[ignore = "reading"]
    fn terraces() {
        let arm = Arm::parse();
        let Some((mut rig, cam0)) = slope_rig() else {
            return;
        };
        let rig = &mut rig;
        let internal = if arm.upscaler == "off" { OUT } else { INTERNAL };
        let mut f = frame_for(rig, &arm);
        f.set_sky(Some(
            flythrough::daylight::Clock {
                minutes: 12.0 * 60.0,
                ..Default::default()
            }
            .sky(flythrough::outdoor::BASE_Z as f32),
        ));
        let mut s = Stepper::new(rig, f);
        QUEUE_ON.with(|q| q.set(true));
        for _ in 0..SETTLE_T {
            queue_step(rig, &mut s.frame, &cam0);
            s.run(rig, &cam0);
        }
        let st = svet_render::debug::read_buffer(
            &rig.device,
            &rig.queue,
            s.frame.exposure_state(),
            0,
            32,
        );
        let e = svet_render::ExposureState::from_bytes(&st).current;
        s.frame.set_lighting(svet_render::Lighting {
            exposure: svet_render::Exposure::Manual(e),
            ..s.frame.lighting()
        });
        QUEUE_ON.with(|q| q.set(false));
        for _ in 0..8 {
            s.run(rig, &cam0);
        }
        let mut out = format!(
            "terraces {} on {}: eye {:?}, exposure pinned {e:.3e}, the queue stopped\n",
            arm.name,
            rig.adapter,
            cam0.position.to_array()
        );
        support::write_png(
            &dir_of().join(format!("{}-still.png", arm.name)),
            &reads_png(rig, &s),
            OUT[0],
            OUT[1],
        );
        let mut reads = Vec::new();
        for _ in 0..=STILL_T {
            reads.push(read_frame(rig, &mut s, &cam0, &arm, internal));
        }
        let still: Vec<PairRow> = reads.windows(2).map(|w| pair_row(&w[0], &w[1])).collect();
        let still_ms = reads
            .iter()
            .fold((0.0, 0.0), |a, r| (a.0 + r.light_ms, a.1 + r.temporal_ms));
        let n = reads.len() as f64;
        out.push_str(&segment_line("still", &still));
        out.push_str(&format!(
            " ms light {:.3} temporal {:.3}\n",
            still_ms.0 / n,
            still_ms.1 / n
        ));
        let step = FLY_M_S / 60.0;
        let mut prev = reads.pop().expect("a still frame");
        let mut fly = Vec::new();
        for k in 1..=FLY_T {
            let cam = moved(&cam0, step * k as f32);
            let r = read_frame(rig, &mut s, &cam, &arm, internal);
            fly.push(pair_row(&prev, &r));
            prev = r;
        }
        out.push_str(&segment_line("fly", &fly));
        out.push('\n');
        print!("{out}");
        let dir = dir_of();
        std::fs::write(dir.join(format!("{}.txt", arm.name)), out).expect("the table");
    }
}
