// `gpu_outdoor`'s dig at depth (Plan 0105 Phase 1), included by `gpu_outdoor.rs` (kept apart
// for the file-size cap).

mod dig_depth {
    pub(crate) mod reading {
        use super::super::dig_line::reading as dl;
        use super::super::flicker::reading as fl;
        use super::super::support;
        use std::collections::BTreeMap;
        use std::fmt::Write as _;
        use support::defects::{
            DIG_HOLE_CLASS, HoleFrame, SOURCE_NAMES, Stepper, dig_class, grid_of, hole_frame, key_slot, march,
            queue_step,
        };
        use support::terms::Rig;
        use svet_render::Camera;
        use svet_render::debug::{HeldBase, HeldPart};

        /// The read depths, voxels under the ground: 1, 2, 3, 5 and 7 m.
        const DEPTHS: [i64; 5] = [10, 20, 30, 50, 70];
        /// The frames read after the dig: D+0 to D+24.
        const AFTER: usize = 24;
        /// The settled frame the dig's frames are read against: D+60.
        const SETTLED: usize = 60;
        /// The most frames a settle before a dig waits for the queue to hold no dig and no fade.
        const SETTLE_CAP: usize = 900;
        /// Frames stepped past the queue's settle before a read dig (the temporal passes).
        const SETTLE_TAIL: usize = 24;

        /// One read dig: its label, depth (voxels), box, the digs counted whole at its edit, the
        /// frames D-1, D+0 .. D+[`AFTER`] and D+[`SETTLED`], each frame's write (the pool's
        /// state), and the held values' parts with their landed values (lockstep).
        struct DigRead {
            label: String,
            depth: i64,
            lo: [i64; 3],
            hi: [i64; 3],
            whole: u64,
            frames: Vec<HoleFrame>,
            writes: Vec<String>,
            parts: Vec<(HeldPart, [Option<[f32; 2]>; 2])>,
        }

        /// The run of one site: the rig, its stepper, the queue's mode and the reads.
        struct Session {
            rig: Rig,
            s: Stepper,
            thread: bool,
            radius: i64,
            out: [u32; 2],
            reads: Vec<DigRead>,
            log: String,
        }

        /// A capsule of `radius` voxels from `cam`'s eye to the first solid voxel along its view
        /// (the fly-through's dig, `support::defects::dig_along` at another radius), carved,
        /// mirrored and handed to the frame. Returns the changed voxels' bounds and the capsule's
        /// end (voxels).
        fn dig(rig: &mut Rig, f: &mut svet_render::Frame, cam: &Camera, radius: i64) -> Option<([i64; 3], [i64; 3], [f32; 3])> {
            use support::SyncNow;
            use svet_world::{EditOp, FIXED_ONE, Fixed3, Mode};
            let one = FIXED_ONE as f32;
            let origin = cam.position.to_array().map(|v| (v * one).floor() as i64);
            let dir = cam.basis().0.to_array().map(|v| (v * one).round() as i64);
            let reach = flythrough_logic::REACH;
            let t = rig.world.raycast(Fixed3(origin), dir, reach).ok().flatten().map_or(reach, |h| h.t);
            let end = Fixed3([0, 1, 2].map(|k| origin[k] + dir[k] * t / FIXED_ONE));
            let r = rig
                .world
                .apply(&EditOp::capsule(Fixed3(origin), end, radius * FIXED_ONE, Mode::Carve))
                .expect("the dig is resident");
            let (lo, hi) = r.aabb?;
            rig.gpu.sync_now(&rig.world, &rig.device, &rig.queue).expect("the mirror fits");
            f.invalidate(lo.0, hi.0);
            Some((lo.0.map(i64::from), hi.0.map(i64::from), end.0.map(|v| v as f32 / one)))
        }

        /// The sky visibility and the direction value's luminance (the stored radiance) of
        /// `p`'s cell and direction in the store's CPU copy (lockstep), `None` where the chunk or
        /// brick holds no record.
        fn record_of(f: &svet_render::Frame, p: &HeldPart) -> Option<[f32; 2]> {
            use svet_render::internals::{BRICK_WORDS, record_visibility, rgb9e5_unpack};
            let w = f.store().block_words(svet_world::ChunkCoord(p.chunk))?;
            let d = w[usize::from(p.slot)];
            if d == 0 {
                return None;
            }
            let run = (d >> 11) as usize;
            let at = run - p.record as usize + BRICK_WORDS * usize::from(p.sub);
            let r = &w[at..at + BRICK_WORDS];
            let c = rgb9e5_unpack(r[usize::from(p.face)]);
            let lum = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
            Some([record_visibility(r, usize::from(p.face)).unwrap_or(f32::NAN), lum])
        }

        /// The top of the first solid voxel under `p` (voxels), within 400.
        fn top_under(rig: &Rig, p: [f32; 3]) -> Option<f32> {
            march(&rig.world, p, [0.0, 0.0, -1.0], 400.0).map(|t| (p[2] - t).ceil())
        }

        impl Session {
            /// The forest of seed 1 at 15:00 about `about` (the spawn's chunks when `None`), the
            /// fly-through's frame at the floor profile's sizes and queue (its tiers, coarse,
            /// drain, edit growth, ring-1 stretch, the lamp's bounce on), the bake thread with
            /// `thread`; the queue drained (at most 30 000 frames) and 240 frames settled at
            /// `cam`.
            fn new(about: Option<[i32; 3]>, cam: &Camera, thread: bool, radius: i64) -> Option<Self> {
                let rig = match about {
                    Some(a) => support::terms::forest_rig_about(a)?,
                    None => support::terms::forest_rig()?.0,
                };
                let clock = flythrough::daylight::Clock {
                    minutes: 15.0 * 60.0,
                    ..Default::default()
                };
                let sky = clock.sky(flythrough::outdoor::BASE_Z as f32);
                let p = svet_render::RenderSettings::profile(svet_render::Profile::Floor);
                let out = p.output_hint.expect("the floor's output");
                let internal = [out[0] * 2 / 3, out[1] * 2 / 3];
                let mut f = fl::flicker_frame_at(&rig, sky, None, false, flythrough::LeafShadow::Voxel, [internal, out]);
                f.set_bake_budget(p.bake.budget);
                f.set_bake_coarse(svet_render::DEFAULT_COARSE_SAMPLES, svet_render::DEFAULT_NEAR);
                f.set_bake_tiers(p.bake.tiers);
                f.set_bake_drain(p.bake.drain);
                f.set_bake_digs(svet_render::DEFAULT_BAKE_EDIT_MS, true);
                f.set_bake_ring1_stretch(svet_render::DEFAULT_RING1_STRETCH);
                f.set_lighting(svet_render::Lighting {
                    lamp_bounce: p.lighting.lamp_bounce,
                    ..f.lighting()
                });
                if thread {
                    f.share_bake_world(rig.world.clone());
                    f.set_bake_thread(&rig.queue, true);
                    f.set_bake_queue(|q| q.trace_digs = true);
                } else {
                    f.set_bake_queue(|q| q.trace_held = true);
                }
                let mut s = Stepper::new(&rig, f);
                s.wait = true;
                let mut me = Self { rig, s, thread, radius, out, reads: Vec::new(), log: String::new() };
                let mut n = 0;
                loop {
                    let q = queue_step(&mut me.rig, &mut me.s.frame, cam);
                    me.s.run(&me.rig, cam);
                    n += 1;
                    if (q.drain_waiting == 0 && n > 1) || n >= 30_000 {
                        break;
                    }
                }
                fl::settle(&mut me.rig, &mut me.s, cam, 240);
                let _ = writeln!(
                    me.log,
                    "dig depth: adapter {}, {}, radius {radius} voxels, drained after {n} frames, output {out:?} internal {internal:?}",
                    me.rig.adapter,
                    if thread { "bake thread" } else { "lockstep" }
                );
                Some(me)
            }

            /// Steps at `cam` until the queue holds no dig and no fade (at most [`SETTLE_CAP`]
            /// frames), then `tail` more; the frames stepped.
            fn settle(&mut self, cam: &Camera, tail: usize) -> usize {
                let mut n = 0;
                while n < SETTLE_CAP {
                    let q = queue_step(&mut self.rig, &mut self.s.frame, cam);
                    self.s.run(&self.rig, cam);
                    n += 1;
                    if q.digs == 0 && q.fading == 0 && n > 2 {
                        break;
                    }
                }
                fl::settle(&mut self.rig, &mut self.s, cam, tail as u32);
                n + tail
            }

            /// An unread dig along `dig_cam`, then the settle at `cam`; the capsule's end.
            fn dig_unread(&mut self, cam: &Camera, dig_cam: &Camera) -> Option<[f32; 3]> {
                let d = dig(&mut self.rig, &mut self.s.frame, dig_cam, self.radius);
                self.settle(cam, 0);
                d.map(|d| d.2)
            }

            /// The pool's ring-1 records of the box ([`dl::pool_ring`]).
            fn ring(&self, lo: [i64; 3], hi: [i64; 3]) -> dl::RingRecords {
                dl::pool_ring(&self.s.frame, &self.rig, lo, hi)
            }

            /// A read dig along `dig_cam` seen from `cam` ([`DigRead`]), `depth` its floor's
            /// depth for the table; the capsule's end.
            fn dig_read(&mut self, cam: &Camera, dig_cam: &Camera, label: &str, depth: i64) -> Option<[f32; 3]> {
                self.settle(cam, SETTLE_TAIL);
                let q = queue_step(&mut self.rig, &mut self.s.frame, cam);
                let mut frames = vec![hole_frame(&self.rig, &mut self.s, cam, q, "D-1".into(), self.out, true, None)];
                if !self.thread {
                    self.s.frame.set_bake_queue(|q| q.held_trace.clear());
                }
                let Some((lo, hi, end)) = dig(&mut self.rig, &mut self.s.frame, dig_cam, self.radius) else {
                    let _ = writeln!(self.log, "{label}: the dig changed nothing");
                    return None;
                };
                let pre = self.ring(lo, hi);
                let mut held: Option<dl::RingRecords> = None;
                let mut fresh_at = None;
                let mut writes = vec!["before".to_string()];
                let whole0 = self.s.frame.bake_queue().expect("the live queue").stats().digs_whole;
                let mut whole = 0;
                let mut parts = Vec::new();
                for k in 0..=SETTLED {
                    let q = queue_step(&mut self.rig, &mut self.s.frame, cam);
                    if k == 0 {
                        whole = q.digs_whole - whole0;
                    }
                    if k == 0 && !self.thread {
                        let b = self.s.frame.bake_queue().expect("the live queue");
                        let trace: Vec<HeldPart> = b.held_trace.clone();
                        parts = trace.into_iter().map(|p| (p, [record_of(&self.s.frame, &p), None])).collect();
                    }
                    if k <= AFTER || k == SETTLED {
                        let cur = self.ring(lo, hi);
                        if held.is_none() && support::queue::dig_held(&pre, &cur) {
                            held = Some(cur.clone());
                        }
                        if fresh_at.is_none() && held.as_ref().is_some_and(|h| support::queue::ring1_landed(h, &cur)) {
                            fresh_at = Some(k);
                        }
                        writes.push(match (held.is_some(), fresh_at) {
                            (_, Some(f)) if k - f < svet_render::internals::DIG_FADE as usize => format!("fade {}", k - f),
                            (_, Some(_)) => "job".into(),
                            (true, None) => "held".into(),
                            (false, None) => "pre-held".into(),
                        });
                        frames.push(hole_frame(&self.rig, &mut self.s, cam, q, format!("D+{k}"), self.out, true, None));
                    } else {
                        self.s.run(&self.rig, cam);
                    }
                }
                if !self.thread {
                    for (p, r) in &mut parts {
                        r[1] = record_of(&self.s.frame, p);
                    }
                }
                self.reads.push(DigRead { label: label.into(), depth, lo, hi, whole, frames, writes, parts });
                Some(end)
            }
        }

        /// One frame's figures over the dig's box: the display pixels hole in it and in the
        /// settled frame (their count, mean and p95 luma, the mean and p95 of the excess over
        /// the settled), and over its lighting pixels hole in it the sky source shares, the cast
        /// share, the table's shares, the sky term (the pair's x), the key slot's value and
        /// source shares, the stored and direct parts.
        #[derive(Default, Clone)]
        struct Figs {
            n: usize,
            mean: f64,
            p95: f64,
            excess: f64,
            excess95: f64,
            ln: u64,
            sources: [u64; 7],
            cast: u64,
            table: [u64; 3],
            sky: f64,
            key: f64,
            key_src: [u64; 5],
            stored: f64,
            direct: f64,
        }

        fn figs(f: &HoleFrame, settled: &HoleFrame, lo: [i64; 3], hi: [i64; 3], out: [u32; 2]) -> Figs {
            let gl = f.hits.len();
            let (dw, dh) = (out[0] as usize, out[1] as usize);
            let gw = ((gl as f64 * dw as f64 / dh as f64).sqrt().round() as usize).max(1);
            let hole = |fr: &HoleFrame, g: usize| dig_class(&fr.hits[g], lo, hi) == DIG_HOLE_CLASS;
            let mut r = Figs::default();
            let (mut l, mut ex) = (Vec::new(), Vec::new());
            for i in 0..dw * dh {
                let g = grid_of(i, dw, dh, gw, gl);
                if hole(f, g) && hole(settled, g) {
                    l.push(f64::from(f.luma[i]));
                    ex.push(f64::from(f.luma[i]) - f64::from(settled.luma[i]));
                }
            }
            let p95 = |v: &mut Vec<f64>| {
                v.sort_by(f64::total_cmp);
                v.get((v.len() * 95 / 100).min(v.len().saturating_sub(1))).copied().unwrap_or(f64::NAN)
            };
            r.n = l.len();
            let n = l.len().max(1) as f64;
            r.mean = l.iter().sum::<f64>() / n;
            r.excess = ex.iter().sum::<f64>() / n;
            r.p95 = p95(&mut l);
            r.excess95 = p95(&mut ex);
            for g in 0..gl {
                if !hole(f, g) {
                    continue;
                }
                r.ln += 1;
                r.sources[usize::from(f.source[g]).min(6)] += 1;
                r.cast += u64::from(f.cast[g]);
                r.table[usize::from(f.table[g]).min(2)] += 1;
                r.sky += f64::from(f.pair[g][0]);
                let (k, ks) = key_slot(f.pair[g][1]);
                r.key += f64::from(k);
                r.key_src[ks] += 1;
                r.stored += f64::from(f.stored[g]);
                r.direct += f64::from(f.direct[g]);
            }
            let m = r.ln.max(1) as f64;
            r.sky /= m;
            r.key /= m;
            r.stored /= m;
            r.direct /= m;
            r
        }

        fn figs_line(label: &str, write: &str, x: &Figs, exposure: f32) -> String {
            let m = x.ln.max(1) as f64;
            let pc = |c: u64| 100.0 * c as f64 / m;
            let src: Vec<String> = x
                .sources
                .iter()
                .enumerate()
                .filter(|(_, c)| **c > 0)
                .map(|(k, c)| format!("{} {:.0}", SOURCE_NAMES[k], pc(*c)))
                .collect();
            let ks: Vec<String> = ["mean", "rec", "hor", "fade", "none"]
                .iter()
                .zip(x.key_src)
                .filter(|(_, c)| *c > 0)
                .map(|(k, c)| format!("{k} {:.0}", pc(c)))
                .collect();
            format!(
                "  {label:>5} {write:<8} px {:>6} luma mean {:6.1} p95 {:5.0} | over settled mean {:+6.1} p95 {:+5.0} | lpx {:>5} sky {:.3} [{}] cast {:.0} % table none {:.0} blk {:.0} coarse {:.0} | key {:.3} [{}] | stored {:.3} direct {:.3} | exposure {:.3e}",
                x.n, x.mean, x.p95, x.excess, x.excess95, x.ln, x.sky, src.join(", "), pc(x.cast), pc(x.table[0]), pc(x.table[1]), pc(x.table[2]), x.key, ks.join(", "), x.stored, x.direct, exposure
            )
        }

        /// The read's table: a row a frame, then the peak over settled (D+0 to D+[`AFTER`]),
        /// its frame, write, and each term at the peak against the settled frame's; and, in
        /// lockstep, the held parts' summary by face kind and base source.
        fn read_text(r: &DigRead, out: [u32; 2], site: &str, mode: &str) -> (String, String) {
            let mut t = String::new();
            let settled = r.frames.last().expect("the settled frame");
            let f: Vec<Figs> = r.frames.iter().map(|fr| figs(fr, settled, r.lo, r.hi, out)).collect();
            let _ = writeln!(t, "{site} {mode} {} depth {} voxels box {:?}..{:?} digs whole at the edit {}", r.label, r.depth, r.lo, r.hi, r.whole);
            for (i, x) in f.iter().enumerate() {
                let _ = writeln!(t, "{}", figs_line(&r.frames[i].label, &r.writes[i], x, r.frames[i].exposure));
            }
            let s = f.last().expect("settled");
            let (pk, px) = f[1..f.len() - 1]
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.excess.total_cmp(&b.1.excess))
                .map(|(i, x)| (i + 1, x.clone()))
                .expect("a frame");
            let p95_peak = f[1..f.len() - 1].iter().map(|x| x.excess95).fold(f64::MIN, f64::max);
            let over4 = f[1..f.len() - 1].iter().filter(|x| x.excess > 4.0).count();
            let line = format!(
                "{site} {mode} {} depth {:.1} m: peak mean over settled {:+.1} levels (ratio {:.2}) at {} ({}), p95 peak {:+.0}, frames over 4 levels {over4}; settled luma {:.1}; at the peak sky {:.3} (settled {:.3}) key {:.3} ({:.3}) stored {:.3} ({:.3}) direct {:.3} ({:.3}); digs whole {}",
                r.label,
                r.depth as f64 / 10.0,
                px.excess,
                px.mean / s.mean.max(1e-9),
                r.frames[pk].label,
                r.writes[pk],
                p95_peak,
                s.mean,
                px.sky,
                s.sky,
                px.key,
                s.key,
                px.stored,
                s.stored,
                px.direct,
                s.direct,
                r.whole
            );
            let _ = writeln!(t, "{line}");
            let mut summary = line;
            if !r.parts.is_empty() {
                // By face kind (floor up, wall sides, ceiling down) and base source.
                let mut g: BTreeMap<(String, String), [f64; 10]> = BTreeMap::new();
                for (p, rec) in &r.parts {
                    let kind = match p.face {
                        5 => "floor",
                        4 => "ceiling",
                        _ => "wall",
                    };
                    let from = match p.from {
                        HeldBase::Above(0) => "own cell".to_string(),
                        HeldBase::Above(k) => format!("above {k}"),
                        HeldBase::Summary => "summary".into(),
                    };
                    let capped = p.own.is_some_and(|o| o < p.up);
                    let from = if capped { format!("{from}, own cap") } else { from };
                    let e = g.entry((kind.into(), from)).or_default();
                    e[0] += 1.0;
                    e[1] += f64::from(p.estimate);
                    e[2] += f64::from(p.value);
                    e[3] += f64::from(p.up);
                    e[4] += f64::from(p.open);
                    if let [Some(h), Some(l)] = rec {
                        e[5] += f64::from(l[0]);
                        e[6] += 1.0;
                        e[7] += f64::from(h[1]);
                        e[8] += f64::from(l[1]);
                        e[9] += 1.0;
                    }
                }
                for ((kind, from), e) in &g {
                    let n = e[0];
                    let landed = e[5] / e[6].max(1.0);
                    let (hl, ll) = (e[7] / e[9].max(1.0), e[8] / e[9].max(1.0));
                    let row = format!(
                        "    held {kind:<7} base {from:<18} n {:>3} estimate {:.3} value {:.3} (up {:.3} open {:.3}) landed {:.3} (n {}), value over landed {:.2}; stored radiance at D+0 {:.4} landed {:.4} ratio {:.2}",
                        n,
                        e[1] / n,
                        e[2] / n,
                        e[3] / n,
                        e[4] / n,
                        landed,
                        e[6],
                        (e[2] / n) / landed.max(1e-6),
                        hl,
                        ll,
                        hl / ll.max(1e-9)
                    );
                    let _ = writeln!(t, "{row}");
                    summary += &format!("\n{row}");
                }
                for (p, rec) in &r.parts {
                    let _ = writeln!(
                        t,
                        "      part {:?} slot {} sub {} face {} value {:.3} estimate {:.3} up {:.3} from {:?} own {:?} open {:.3} faces new {} old {} record at D+0 (vis, radiance) {:?} landed {:?}",
                        p.chunk, p.slot, p.sub, p.face, p.value, p.estimate, p.up, p.from, p.own, p.open, p.new_faces, p.old_faces, rec[0], rec[1]
                    );
                }
            }
            (t, summary)
        }

        /// The forest spawn's eye and heading (the shaft's axis), unit xy.
        fn spawn_eye() -> ([f32; 3], [f32; 2]) {
            use flythrough::outdoor::{OutdoorSource, Spawn};
            use flythrough::trees::{ForestSource, LEAF_FILL, TreeDensity};
            let source = ForestSource::new(OutdoorSource::new(1), TreeDensity::Forest, LEAF_FILL);
            let (eye, facing) = source.base().spawn(Spawn::Forest);
            let h = (facing[0] as f32).hypot(facing[1] as f32).max(1e-6);
            (eye.0.map(|v| v as f32 + 0.5), [facing[0] as f32 / h, facing[1] as f32 / h])
        }

        /// A camera at `eye` looking along heading `h` (unit xy) at pitch `deg` (up positive).
        fn look(eye: [f32; 3], h: [f32; 2], deg: f32) -> Camera {
            let (c, s) = (deg.to_radians().cos(), deg.to_radians().sin());
            flythrough::camera_looking_at(eye, [eye[0] + h[0] * c, eye[1] + h[1] * c, eye[2] + s])
        }

        /// A camera at `eye` looking straight down.
        fn down(eye: [f32; 3]) -> Camera {
            flythrough::camera_looking_at(eye, [eye[0], eye[1], eye[2] - 1.0])
        }

        /// The next read depth at or under `depth` not yet read.
        fn due(depth: i64, read: &mut usize) -> Option<i64> {
            let d = *DEPTHS.get(*read)?;
            (depth >= d).then(|| {
                *read += 1;
                d
            })
        }

        /// Site `down`: a shaft one dig wide dug straight down from the forest spawn's eye,
        /// read at the dig whose floor reaches each of [`DEPTHS`] (the floor's depth predicted
        /// as the floor before it plus the radius).
        fn site_down(s: &mut Session, eye: [f32; 3]) {
            let cam = down(eye);
            let g0 = top_under(&s.rig, eye).expect("ground under the spawn");
            let mut read = 0;
            for _ in 0..40 {
                let floor = top_under(&s.rig, eye).expect("a floor");
                let next = (g0 - floor) as i64 + s.radius;
                if read >= DEPTHS.len() {
                    break;
                }
                match due(next, &mut read) {
                    Some(d) => {
                        let _ = s.dig_read(&cam, &cam, &format!("down {d}"), next);
                    }
                    None => {
                        let _ = s.dig_unread(&cam, &cam);
                    }
                }
            }
            let floor = top_under(&s.rig, eye).expect("a floor");
            let _ = writeln!(s.log, "down: ground top {g0}, floor at the end {floor} ({} voxels down)", g0 - floor);
        }

        /// Site `side`: the same shaft, dug unread to 12 voxels under each of [`DEPTHS`], then a
        /// dig into its wall from an eye on the axis at that depth, looking level along the
        /// spawn's heading turned 72 degrees more at each depth.
        fn site_side(s: &mut Session, eye: [f32; 3], h: [f32; 2]) {
            let cam = down(eye);
            let g0 = top_under(&s.rig, eye).expect("ground under the spawn");
            for (k, &d) in DEPTHS.iter().enumerate() {
                for _ in 0..40 {
                    let floor = top_under(&s.rig, eye).expect("a floor");
                    if (g0 - floor) as i64 >= d + 12 {
                        break;
                    }
                    let _ = s.dig_unread(&cam, &cam);
                }
                let a = (k as f32 * 72.0).to_radians();
                let hk = [h[0] * a.cos() - h[1] * a.sin(), h[0] * a.sin() + h[1] * a.cos()];
                let side = look([eye[0], eye[1], g0 - d as f32], hk, 0.0);
                let _ = s.dig_read(&side, &side, &format!("side {d}"), d);
            }
        }

        /// Site `wide`: a pit dug from the spawn's eye looking down by rounds of five digs at
        /// [`dl::B_AIMS`] about the axis on the floor below each aim; a round's first dig (the
        /// axis) is read when it reaches each of [`DEPTHS`].
        fn site_wide(s: &mut Session, eye: [f32; 3]) {
            let cam = down(eye);
            let g0 = top_under(&s.rig, eye).expect("ground under the spawn");
            let mut read = 0;
            for _ in 0..30 {
                if read >= DEPTHS.len() {
                    break;
                }
                for (i, a) in dl::B_AIMS.iter().enumerate() {
                    let p = [eye[0] + a[0], eye[1] + a[1], eye[2]];
                    let floor = top_under(&s.rig, p).expect("a floor");
                    let aim = flythrough::camera_looking_at(eye, [p[0], p[1], floor - 0.5]);
                    let when = if i == 0 { due((g0 - floor) as i64 + s.radius, &mut read) } else { None };
                    match when {
                        Some(d) => {
                            let _ = s.dig_read(&cam, &aim, &format!("wide {d}"), (g0 - floor) as i64 + s.radius);
                        }
                        None => {
                            let _ = s.dig_unread(&cam, &aim);
                        }
                    }
                }
            }
        }

        /// Site `slope`: a tunnel 30 degrees down along the spawn's heading from its eye, the
        /// eye moved on by the radius along it after each dig; a dig is read when its end
        /// first lies at each of [`DEPTHS`] under the ground as it stood before the first dig.
        fn site_slope(s: &mut Session, eye0: [f32; 3], h: [f32; 2]) {
            let tops: Vec<f32> = (0..400)
                .map(|k| top_under(&s.rig, [eye0[0] + h[0] * k as f32, eye0[1] + h[1] * k as f32, eye0[2] + 40.0]).unwrap_or(f32::NAN))
                .collect();
            let ground = |p: [f32; 3]| {
                let k = ((p[0] - eye0[0]) * h[0] + (p[1] - eye0[1]) * h[1]).round().clamp(0.0, 399.0) as usize;
                tops[k]
            };
            let (c, sn) = (30f32.to_radians().cos(), 30f32.to_radians().sin());
            let d = [h[0] * c, h[1] * c, -sn];
            let mut eye = eye0;
            let mut read = 0;
            let mut end_depth = 0i64;
            for _ in 0..120 {
                if read >= DEPTHS.len() {
                    break;
                }
                let cam = look(eye, h, -30.0);
                // The next dig's end: its first solid voxel plus the radius along the view.
                let t = march(&s.rig.world, eye, d, 200.0).unwrap_or(0.0);
                let e = [0, 1, 2].map(|k| eye[k] + d[k] * t);
                let next = (ground(e) - e[2]) as i64 + (s.radius as f32 * sn) as i64;
                end_depth = end_depth.max(next);
                match due(next, &mut read) {
                    Some(dd) => {
                        let _ = s.dig_read(&cam, &cam, &format!("slope {dd}"), next);
                    }
                    None => {
                        let _ = s.dig_unread(&cam, &cam);
                    }
                }
                eye = [0, 1, 2].map(|k| eye[k] + d[k] * s.radius as f32);
            }
            let _ = writeln!(s.log, "slope: deepest end {end_depth} voxels, eye at the end {eye:?}");
        }

        /// Site `tunnel`: from the tunnel's eye (`--spawn inside`) in the forest, a dig into the
        /// wall (the heading turned 90 degrees, level) and one into the floor (straight down),
        /// each read; the cover over the eye printed.
        fn site_tunnel(s: &mut Session, eye: [f32; 3], h: [f32; 2]) {
            let cover = march(&s.rig.world, eye, [0.0, 0.0, 1.0], 400.0);
            let _ = writeln!(s.log, "tunnel: eye {eye:?}, first solid above at {cover:?} voxels");
            let wall = look(eye, [-h[1], h[0]], 0.0);
            let floor = top_under(&s.rig, eye).unwrap_or(eye[2]);
            let _ = s.dig_read(&wall, &wall, "tunnel wall", -1);
            let dn = down(eye);
            let _ = s.dig_read(&dn, &dn, "tunnel floor", (eye[2] - floor) as i64);
        }

        /// Plan 0105 Phase 1: the dig's blink at depth on the forest of seed 1 at 15:00 at
        /// the floor profile's sizes and queue. `SVET_DIG` is `SITE/MODE/RADIUS`: `down` (a
        /// shaft one dig wide, dug looking down), `side` (the shaft's wall dug from its axis),
        /// `wide` (a pit of five digs a round), `slope` (30 degrees down) or `tunnel` (`--spawn
        /// inside`); `lock` or `thread`; the dig's radius in voxels (5, the fly-through's). Per
        /// read dig the frames D-1 to D+24 and D+60 over the pixels whose hit lies in the dig's
        /// box: luma and its excess over D+60, the sky source shares, the sky and key terms, the
        /// stored and direct parts, the write the pool showed; in lockstep the held values'
        /// parts beside the landed records. The text under
        /// `live/p0105/<adapter>-<site>-<mode>-r<radius>.txt`.
        #[test]
        #[ignore = "reading"]
        fn dig_depth() {
            let v = std::env::var("SVET_DIG").unwrap_or_default();
            let mut it = v.split('/');
            let mut part = |d: &str| it.next().filter(|s| !s.is_empty()).unwrap_or(d).to_string();
            let (site, mode) = (part("down"), part("lock"));
            let radius: i64 = part("5").parse().expect("SVET_DIG's radius");
            let thread = match mode.as_str() {
                "thread" => true,
                "lock" => false,
                m => panic!("SVET_DIG's mode {m}: thread or lock"),
            };
            let (eye, h) = spawn_eye();
            let (about, eye, h) = if site == "tunnel" {
                use flythrough::outdoor::{OutdoorSource, Spawn};
                let (e, facing) = OutdoorSource::new(1).spawn(Spawn::Inside);
                let n = (facing[0] as f32).hypot(facing[1] as f32).max(1e-6);
                (Some(e.0), e.0.map(|v| v as f32 + 0.5), [facing[0] as f32 / n, facing[1] as f32 / n])
            } else {
                (None, eye, h)
            };
            let start = if site == "tunnel" { look(eye, h, 0.0) } else { down(eye) };
            let Some(mut s) = Session::new(about, &start, thread, radius) else {
                return;
            };
            match site.as_str() {
                "down" => site_down(&mut s, eye),
                "side" => site_side(&mut s, eye, h),
                "wide" => site_wide(&mut s, eye),
                "slope" => site_slope(&mut s, eye, h),
                "tunnel" => site_tunnel(&mut s, eye, h),
                o => panic!("SVET_DIG's site {o}: down, side, wide, slope or tunnel"),
            }
            let mut text = s.log.clone();
            let mut summary = s.log.clone();
            for r in &s.reads {
                let (t, sm) = read_text(r, s.out, &site, &mode);
                text += &t;
                summary += &sm;
                summary.push('\n');
            }
            println!("{summary}");
            let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../live/p0105");
            std::fs::create_dir_all(&dir).expect("the reading's directory");
            std::fs::write(dir.join(format!("{}-{site}-{mode}-r{radius}.txt", s.rig.adapter.replace(' ', "_"))), text).expect("the table");
        }
    }
}
