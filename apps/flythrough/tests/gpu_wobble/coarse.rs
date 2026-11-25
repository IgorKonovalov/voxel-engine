//! Plan 0100 Phase 3: the coarse tier's wobble, a reading. The forest of seed 1 (the wobble
//! rig's, `--spawn forest --trees forest`), the `SVET_WOBBLE` arm over the rig's frame with the
//! fly-through's bake knobs (the coarse pass's count, the full-first radius, the tiers by size on
//! screen; `coarse=N` and `ff=N|none` override the first two). From a fresh store at the spawn's
//! camera: 120 frames with the queue stepping, a 60-degree turn over 40, then the queue steps at
//! the turned pose until every chunk a lighting pixel's hit lies in holds a block (**held
//! coarse**): the exposure is pinned, the queue stopped, 32 still pairs over the jitter's phases
//! read. Then the queue steps until no chunk in view waits for its tier and no job runs
//! (**landed**), is stopped again and the same 32 pairs are read. The classes are the held
//! state's, by display pixel: near (under 6 m), the far normal's weight 0, and the pixels on a
//! coarse block at weight 0, between and 1. For the moved pixels of the weight-0 and the
//! between-and-coarse classes a source table: the sky source word in either frame, whether the
//! two frames read different words, whether the hit's brick differs, and which term moved.
//! Nothing is asserted.

use super::*;
use support::defects::{SOURCE_NAMES, key_slot};

/// The fresh start's frames, the turn's, the most frames the hold and the drain may take, and
/// the still's pairs.
const START: usize = 120;
const TURN: usize = 40;
const HOLD_MAX: usize = 4000;
const DRAIN_MAX: usize = 60000;
const STILL: usize = 32;

/// The drain's frames between two reads of the census: the store counts as landed once no
/// lighting pixel at weight 0 lies on a coarse block (the bounded world's edge chunks keep the
/// queue from draining).
const CENSUS_EVERY: usize = 300;

/// The queue-stepping frames after the turn, pair by pair: the weight-0 class's share over 4
/// (mean, largest and its frame), the near class's mean, and the pairs over 0.5 % at weight 0
/// with and without a block landed in their step.
fn live_line(live: &[(f64, f64, u64)]) -> String {
    let n = live.len().max(1) as f64;
    let (mut max, mut at) = (0.0f64, 0);
    for (i, l) in live.iter().enumerate() {
        if l.0 > max {
            (max, at) = (l.0, i + 1);
        }
    }
    let over: Vec<_> = live.iter().filter(|l| l.0 > 0.5).collect();
    format!(
        "live after the turn, the queue stepping ({} pairs): w 0 mean {:.2} % max {:.2} % at +{at}; near mean {:.2} %; pairs over 0.5 % at w 0: {}, {} of them with a block landed in their step; blocks landed {}\n",
        live.len(),
        live.iter().map(|l| l.0).sum::<f64>() / n,
        max,
        live.iter().map(|l| l.1).sum::<f64>() / n,
        over.len(),
        over.iter().filter(|l| l.2 > 0).count(),
        live.iter().map(|l| l.2).sum::<u64>(),
    )
}

/// The class names, by bin of [`classes_of`].
const BINS: [&str; 5] = [
    "near",
    "w 0",
    "w 0 coarse",
    "w between coarse",
    "w 1 coarse",
];

/// The live queue stepped as the fly-through's: the view's half angles and focal length, so the
/// tiers by size on screen apply.
pub(super) fn step_queue(rig: &mut Rig, f: &mut Frame, cam: &Camera) -> svet_render::QueueStats {
    let size = f.output_size();
    let aspect = size[0] as f32 / size[1] as f32;
    let view = svet_render::QueueView {
        eye: cam.position.to_array(),
        forward: cam.basis().0.to_array(),
        resident: None,
        samples: None,
        interval_s: 1.0 / 60.0,
        hold: None,
        half_angle: svet_render::internals::view_half_angle(cam.vfov, aspect),
        frustum: Some(svet_render::internals::view_half_angles(cam.vfov, aspect)),
        full_radius: None,
        lod_radius: None,
        focal_px: Some(svet_render::Bands::focal_px(size[1], cam.vfov)),
    };
    f.bake_queue_step(&rig.device, &rig.queue, &mut rig.gpu, &rig.world, &view)
        .expect("the queue runs")
        .expect("the queue's step")
}

/// One frame's reads per lighting pixel.
struct Read {
    luma: Vec<u8>,
    source: Vec<u8>,
    sky: Vec<f32>,
    key: Vec<f32>,
    /// The hit's chunk's block: 0 none, 1 coarse, 2 not coarse; 255 a miss.
    block: Vec<u8>,
    /// The hit's voxel's brick (the hit nudged along the view ray into the solid).
    brick: Vec<[i32; 3]>,
    /// The far normal's weight at the hit, -1 at a miss; the hit's distance in m.
    w: Vec<f32>,
    dist: Vec<f32>,
    gw: usize,
}

fn read_frame(rig: &Rig, s: &mut Stepper, cam: &Camera, arm: &Arm) -> Read {
    let shot = s.step(rig, cam);
    let f = &s.frame;
    let pair = svet_render::debug::read_rgba16f(&rig.device, &rig.queue, f.visibility_pair());
    let internal = if arm.upscaler == "off" { OUT } else { INTERNAL };
    let eye = cam.position.to_array();
    let tan_h = (cam.vfov * 0.5).tan() * internal[0] as f32 / internal[1] as f32;
    let [lo, hi] = arm.far_normal;
    let cell = svet_render::internals::BRICK_CELL as f32;
    let mut held = std::collections::HashMap::<[i32; 3], u8>::new();
    let (mut block, mut brick, mut w, mut dist) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for h in &shot.hits {
        if h[3] >= 1e30 || !h[3].is_finite() {
            block.push(255);
            brick.push([i32::MIN; 3]);
            w.push(-1.0);
            dist.push(f32::INFINITY);
            continue;
        }
        let d = [h[0] - eye[0], h[1] - eye[1], h[2] - eye[2]];
        let t = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-3);
        let v = [0, 1, 2].map(|k| (h[k] + 0.01 * d[k] / t).floor());
        let c = v.map(|x| (x / 64.0).floor() as i32);
        block.push(*held.entry(c).or_insert_with(|| {
            let coord = svet_world::ChunkCoord(c);
            match (f.store().block(coord).is_some(), f.store().is_coarse(coord)) {
                (false, _) => 0,
                (true, true) => 1,
                (true, false) => 2,
            }
        }));
        brick.push(v.map(|x| (x / cell).floor() as i32));
        dist.push(t * VOXEL_M);
        w.push(if hi <= lo {
            0.0
        } else {
            let px = internal[0] as f32 / (2.0 * tan_h * t);
            let x = ((px - lo) / (hi - lo)).clamp(0.0, 1.0);
            1.0 - x * x * (3.0 - 2.0 * x)
        });
    }
    Read {
        luma: lumas(&shot.display),
        source: shot.source,
        sky: pair.iter().map(|p| p[0]).collect(),
        key: pair.iter().map(|p| key_slot(p[1]).0).collect(),
        block,
        brick,
        w,
        dist,
        gw: f.light_hit_positions().width() as usize,
    }
}

/// The in-view chunks of `r` (the distinct hit chunks, as blocks counted by lighting pixel):
/// lighting pixels on no block, a coarse one and a full one; and the same at weight 0.
fn census(r: &Read) -> ([usize; 3], [usize; 3]) {
    let (mut all, mut w0) = ([0usize; 3], [0usize; 3]);
    for (g, &b) in r.block.iter().enumerate() {
        if b == 255 {
            continue;
        }
        all[usize::from(b)] += 1;
        if r.w[g] <= 0.01 {
            w0[usize::from(b)] += 1;
        }
    }
    (all, w0)
}

/// Per display pixel, the bins of [`BINS`] it falls in (a bit each), from the held state's read.
fn classes_of(r: &Read) -> Vec<u8> {
    let (dw, dh) = (OUT[0] as usize, OUT[1] as usize);
    let gl = r.block.len();
    (0..dw * dh)
        .map(|i| {
            let g = grid_of(i, dw, dh, r.gw, gl);
            if r.block[g] == 255 {
                return 0;
            }
            let (w, coarse) = (r.w[g], r.block[g] == 1);
            let mut m = u8::from(r.dist[g] < 6.0);
            if w <= 0.01 {
                m |= 2;
                if coarse {
                    m |= 4;
                }
            } else if w < 0.99 {
                m |= if coarse { 8 } else { 0 };
            } else if coarse {
                m |= 16;
            }
            m
        })
        .collect()
}

/// A pair's figures: per bin the pixels and those over 4 levels; per source-table bin (weight 0,
/// between and coarse) the moved pixels' rows.
#[derive(Clone, Copy, Default)]
struct PairRow {
    n: [u64; 5],
    over: [u64; 5],
    src: [SrcRow; 2],
}

/// The moved pixels of a source-table bin: how many; on a coarse block; reading word `k` in
/// either frame; a different word in the two frames; the hit's brick differs; the key moved
/// (over 0.05), else the sky (over 0.02), else neither; and the summed |d sky| where the brick
/// differs and where it does not.
#[derive(Clone, Copy, Default)]
struct SrcRow {
    moved: u64,
    coarse: u64,
    word: [u64; 7],
    switched: u64,
    crossed: u64,
    by: [u64; 3],
    dsky_crossed: f64,
    dsky_same: f64,
}

fn pair_row(cls: &[u8], a: &Read, b: &Read) -> PairRow {
    let (dw, dh) = (OUT[0] as usize, OUT[1] as usize);
    let gl = a.block.len();
    let mut r = PairRow::default();
    for (i, &m) in cls.iter().enumerate() {
        if m == 0 {
            continue;
        }
        let moved = a.luma[i].abs_diff(b.luma[i]) > 4;
        for k in 0..5 {
            if m & (1 << k) != 0 {
                r.n[k] += 1;
                r.over[k] += u64::from(moved);
            }
        }
        if !moved {
            continue;
        }
        let g = grid_of(i, dw, dh, a.gw, gl);
        for (t, bit) in [(0usize, 2u8), (1, 8)] {
            if m & bit == 0 {
                continue;
            }
            let s = &mut r.src[t];
            s.moved += 1;
            s.coarse += u64::from(a.block[g] == 1 || b.block[g] == 1);
            let (wa, wb) = (usize::from(a.source[g]), usize::from(b.source[g]));
            for (k, n) in s.word.iter_mut().enumerate() {
                *n += u64::from(wa == k || wb == k);
            }
            s.switched += u64::from(wa != wb);
            let ds = f64::from((a.sky[g] - b.sky[g]).abs());
            if a.brick[g] != b.brick[g] {
                s.crossed += 1;
                s.dsky_crossed += ds;
            } else {
                s.dsky_same += ds;
            }
            let k = if (a.key[g] - b.key[g]).abs() > 0.05 {
                0
            } else if ds > 0.02 {
                1
            } else {
                2
            };
            s.by[k] += 1;
        }
    }
    r
}

/// The state's line: per bin the share over 4 a pair and the pixels a pair; then the source
/// tables.
fn state_line(name: &str, rows: &[PairRow]) -> String {
    let p = rows.len().max(1) as u64;
    let mut out = format!("{name} ({} pairs):", rows.len());
    for (k, bin) in BINS.iter().enumerate() {
        let n: u64 = rows.iter().map(|r| r.n[k]).sum();
        let over: u64 = rows.iter().map(|r| r.over[k]).sum();
        out.push_str(&format!(
            " {bin} {:.2} % of {} px;",
            100.0 * over as f64 / n.max(1) as f64,
            n / p
        ));
    }
    for (t, tname) in ["w 0", "w between coarse"].iter().enumerate() {
        let mut s = SrcRow::default();
        for r in rows {
            let x = &r.src[t];
            s.moved += x.moved;
            s.coarse += x.coarse;
            s.switched += x.switched;
            s.crossed += x.crossed;
            s.dsky_crossed += x.dsky_crossed;
            s.dsky_same += x.dsky_same;
            for k in 0..7 {
                s.word[k] += x.word[k];
            }
            for k in 0..3 {
                s.by[k] += x.by[k];
            }
        }
        let m = s.moved.max(1) as f64;
        let pct = |v: u64| 100.0 * v as f64 / m;
        let words: Vec<String> = (0..7)
            .filter(|&k| s.word[k] > 0)
            .map(|k| format!("{} {:.0} %", SOURCE_NAMES[k], pct(s.word[k])))
            .collect();
        out.push_str(&format!(
            "\n  sources, {tname}: moved {} a pair; on a coarse block {:.0} %; read in either frame: {}; \
             a different word in the two {:.0} %; the brick differs {:.0} % (mean |d sky| {:.3} crossed, \
             {:.3} not); key {:.0} %, sky {:.0} %, neither {:.0} %",
            s.moved / p,
            pct(s.coarse),
            words.join(", "),
            pct(s.switched),
            pct(s.crossed),
            s.dsky_crossed / s.crossed.max(1) as f64,
            s.dsky_same / (s.moved - s.crossed).max(1) as f64,
            pct(s.by[0]),
            pct(s.by[1]),
            pct(s.by[2]),
        ));
    }
    out
}

mod reading {
    use super::*;

    fn dir_of() -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../live/wobble/coarse");
        std::fs::create_dir_all(&dir).expect("the reading's directory");
        dir
    }

    /// 32 still pairs at `cam` with the queue stopped, after `FADE` frames and 8 more; the
    /// first frame's display written to `<name>-still.png`.
    fn still(rig: &Rig, s: &mut Stepper, cam: &Camera, arm: &Arm, name: &str) -> Vec<Read> {
        for _ in 0..svet_render::internals::FADE as usize + 8 {
            s.run(rig, cam);
        }
        let reads: Vec<Read> = (0..=STILL).map(|_| read_frame(rig, s, cam, arm)).collect();
        support::write_png(
            &dir_of().join(format!("{name}-still.png")),
            &svet_render::debug::read_rgba8(&rig.device, &rig.queue, s.frame.target()),
            OUT[0],
            OUT[1],
        );
        reads
    }

    /// The reading for the arm `SVET_WOBBLE` names, its lines printed and written to
    /// `live/wobble/coarse/<arm>.txt`.
    #[test]
    #[ignore = "reading"]
    fn coarse_hold() {
        let arm = Arm::parse();
        let Some((mut rig, cam0)) = forest_rig(8) else {
            return;
        };
        let rig = &mut rig;
        let mut f = frame_for(rig, &arm);
        f.set_bake_coarse(arm.coarse, svet_render::DEFAULT_NEAR);
        f.set_bake_full_first(arm.full_first);
        f.set_bake_tiers(svet_render::DEFAULT_TIERS);
        f.set_bake_drain(Some(svet_render::DEFAULT_DRAIN));
        let mut s = Stepper::new(rig, f);
        let mut out = format!(
            "coarse {} on {}: coarse {} full-first {:?}, eye {:?}\n",
            arm.name,
            rig.adapter,
            arm.coarse,
            arm.full_first,
            cam0.position.to_array()
        );
        for _ in 0..START {
            step_queue(rig, &mut s.frame, &cam0);
            s.run(rig, &cam0);
        }
        let cam1 = yawed(&cam0, arm.turn_deg);
        for k in 1..=TURN {
            let cam = yawed(&cam0, arm.turn_deg * k as f32 / TURN as f32);
            step_queue(rig, &mut s.frame, &cam);
            s.run(rig, &cam);
        }
        let mut k = 0;
        let baked = |s: &Stepper| s.frame.bake_queue().map_or(0, |q| q.stats().baked);
        let (mut prev, mut live, mut b0): (Option<Read>, Vec<(f64, f64, u64)>, u64) =
            (None, Vec::new(), baked(&s));
        let census_at_hold = loop {
            let r = read_frame(rig, &mut s, &cam1, &arm);
            if let Some(p) = &prev {
                let row = pair_row(&classes_of(p), p, &r);
                let share = |b: usize| 100.0 * row.over[b] as f64 / row.n[b].max(1) as f64;
                live.push((share(1), share(0), baked(&s) - b0));
                b0 = baked(&s);
            }
            let c = census(&r);
            if c.0[0] == 0 || k >= HOLD_MAX {
                break c;
            }
            prev = Some(r);
            step_queue(rig, &mut s.frame, &cam1);
            k += 1;
        };
        out.push_str(&live_line(&live));
        out.push_str(&format!(
            "held at turn +{k} frames: lighting px on no block / coarse / full {:?}, at w 0 {:?}\n",
            census_at_hold.0, census_at_hold.1
        ));
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
        let held = still(rig, &mut s, &cam1, &arm, &format!("{}-held", arm.name));
        let cls = classes_of(&held[0]);
        let rows: Vec<PairRow> = held
            .windows(2)
            .map(|w| pair_row(&cls, &w[0], &w[1]))
            .collect();
        out.push_str(&state_line("held coarse", &rows));
        out.push('\n');
        let mut k = 0usize;
        loop {
            let q = step_queue(rig, &mut s.frame, &cam1);
            k += 1;
            let w0_full = if k.is_multiple_of(CENSUS_EVERY) {
                let c = census(&read_frame(rig, &mut s, &cam1, &arm)).1;
                c[0] == 0 && c[1] == 0
            } else {
                s.run(rig, &cam1);
                false
            };
            if (q.drain_waiting == 0 && q.jobs == 0 && !q.draining) || w0_full || k >= DRAIN_MAX {
                out.push_str(&format!(
                    "landed after {k} more frames: depth {} jobs {} drain waiting {}\n",
                    q.depth, q.jobs, q.drain_waiting
                ));
                break;
            }
        }
        let landed = still(rig, &mut s, &cam1, &arm, &format!("{}-landed", arm.name));
        let c = census(&landed[0]);
        out.push_str(&format!(
            "landed: lighting px on no block / coarse / full {:?}, at w 0 {:?}\n",
            c.0, c.1
        ));
        let rows: Vec<PairRow> = landed
            .windows(2)
            .map(|w| pair_row(&cls, &w[0], &w[1]))
            .collect();
        out.push_str(&state_line("landed", &rows));
        let mut d = [0u64; 5];
        let mut n = [0u64; 5];
        for (i, &m) in cls.iter().enumerate() {
            for b in 0..5 {
                if m & (1 << b) != 0 {
                    n[b] += 1;
                    d[b] += u64::from(held[0].luma[i].abs_diff(landed[0].luma[i]));
                }
            }
        }
        out.push_str("\nheld to landed, mean |dY|:");
        for b in 0..5 {
            out.push_str(&format!(
                " {} {:.2};",
                BINS[b],
                d[b] as f64 / n[b].max(1) as f64
            ));
        }
        out.push('\n');
        print!("{out}");
        std::fs::write(dir_of().join(format!("{}.txt", arm.name)), out).expect("the table");
    }
}
