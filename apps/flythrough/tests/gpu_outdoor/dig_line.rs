// `gpu_outdoor`'s dig timeline (Plan 0098 Phase 16 step 1), included by `gpu_outdoor.rs`
// (kept apart for the file-size cap).

mod dig_line {
    pub(crate) mod reading {
        use super::super::flicker::reading as fl;
        use super::super::support;
        use std::collections::{BTreeMap, BTreeSet};
        use std::fmt::Write as _;
        use support::defects::{HoleFrame, Stepper, dig_along, dig_along_timed, hole_frame, queue_step};
        use support::terms::Rig;
        use svet_render::Camera;

        /// The eye of the owner's dig on the RADV RENOIR at Portable (window 98d's run 3,
        /// `3-radeon-dig.png`'s HUD), voxels.
        const RENOIR_EYE: [f32; 3] = [31.9, 603.0, 604.3];

        /// Frames between the digs of sequence B (the owner's several in a row).
        const B_GAP: usize = 8;
        /// Frames from sequence `C`'s first dig to its second.
        const C_GAP: usize = 64;

        /// Where sequence B's five digs aim, voxels on the ground about the view's centre's hit;
        /// the sixth digs into the first.
        pub(in super::super) const B_AIMS: [[f32; 2]; 5] = [[0.0, 0.0], [9.0, 0.0], [-9.0, 2.0], [2.0, 9.0], [-2.0, -9.0]];

        /// A point of [`line_run`] a reading's tap sees: the frame made and stepped, before any
        /// step (`Start`); a dig's edit applied, before the queue's step reads its box (`Edit`,
        /// the dig from 0 and its changed voxels' bounds); and each frame after a dig, read
        /// (`Frame`, the dig, frames from its own, the frame).
        pub(in super::super) enum Tap<'a> {
            Start,
            Edit { dig: usize, lo: [i64; 3], hi: [i64; 3] },
            Frame { dig: usize, k: usize, frame: &'a HoleFrame },
        }

        /// A reading's tap on [`line_run`] ([`Tap`]).
        pub(in super::super) type TapFn<'t> = &'t mut dyn FnMut(Tap<'_>, &mut Rig, &mut Stepper);

        /// The run of [`dig_timeline`]: the frames, per dig its frame index and box, and the
        /// queue's trace after each frame's step.
        pub(in super::super) struct LineRun {
            pub frames: Vec<HoleFrame>,
            pub traces: Vec<svet_render::DigTrace>,
            pub digs: Vec<(usize, [i64; 3], [i64; 3])>,
            /// Each dig's edit wall, ms ([`dig_along_timed`]).
            pub edit_ms: Vec<f64>,
            /// Per frame: the wait for an edit's step in ms and the submits the frame issued
            /// ([`svet_render::Frame::bake_frame_issue`]).
            pub issue: Vec<(f64, u64)>,
            /// Per frame: the apply in ms and the bytes applied over the frame's cap
            /// ([`svet_render::Frame::bake_frame_apply`]).
            pub apply: Vec<(f64, usize)>,
            /// Per frame: its start and end in ms from the run's clock (the queue's step to the
            /// end of the frame's reads and render).
            pub times: Vec<(f64, f64)>,
            /// Per frame: an edit's frame's wait by its parts and its issue lap
            /// ([`svet_render::Frame::bake_frame_wait_parts`]), `None` on other frames.
            pub parts: Vec<Option<([f64; 6], f64)>>,
            /// Per frame from the first dig on, the newest dig's ring as the pool holds it: whether
            /// its ring-1 records have held the dig's block ([`support::queue::dig_held`] against
            /// the pool read after the edit, before a step), and whether ring 1's re-bake has
            /// landed over that block ([`support::queue::ring1_landed`]).
            pub pool: Vec<Option<(bool, bool)>>,
            /// Whether the queue's steps ran on the bake thread.
            pub thread: bool,
            pub out: [u32; 2],
            pub eye: [f32; 3],
            pub head: String,
        }

        /// `cam` turned to look at the ground point `at`.
        fn aimed(cam: &Camera, at: [f32; 3]) -> Camera {
            flythrough::camera_looking_at(cam.position.to_array(), at)
        }

        /// The forest spawn's facing.
        fn spawn_facing() -> [f32; 3] {
            use flythrough::outdoor::{OutdoorSource, Spawn};
            use flythrough::trees::{ForestSource, LEAF_FILL, TreeDensity};
            let source = ForestSource::new(OutdoorSource::new(1), TreeDensity::Forest, LEAF_FILL);
            source.base().spawn(Spawn::Forest).1.map(|v| v as f32)
        }

        /// The dig timeline's run: `scene` `above` (the RTX's pose of window 98d, the forest's
        /// spawn at `--sun 10,180`, the fly-through's frame sizes) or `portable` (window 98d's
        /// run 3: [`RENOIR_EYE`] at 15:00, the Portable profile's sizes and queue), both looking
        /// down at [`fl::ABOVE_PITCH`] along the spawn's heading, with the fly-through's queue
        /// flags (`--coarse` 256, `--near` 2, its tiers, drain, edit growth and ring 3).
        /// `waiting`: the first dig at the first step whose chunks in view waiting are over 0
        /// and at most 101; else the queue is stepped at most `drain` frames until none waits
        /// and settled 240. Sequence `A`: three digs down one ray, the third one frame after
        /// the second; `B`: five at [`B_AIMS`] [`B_GAP`] frames apart, then a sixth into the
        /// first; `C`: `B`'s first, then its fourth alone [`C_GAP`] frames later. After the last,
        /// frames until no dig is left plus 16 and [`fl::DIG_TAIL`]. `wait`: the stepper's
        /// device-wait mode ([`Stepper::wait`]), the fly-through's pace; `stretch`: the queue's
        /// ring-1 stretch ([`svet_render::Frame::set_bake_ring1_stretch`]); `thread`: the queue's
        /// sliced steps on the bake thread ([`svet_render::Frame::set_bake_thread`], its books
        /// carrying the dig trace), else lockstep. `tap`: a reading's look at the run's points
        /// ([`Tap`]); with one, every frame reads its stages ([`hole_frame`]'s switch).
        #[allow(
            clippy::too_many_arguments,
            reason = "the run's scene, state, sequence, arm, drain, stills, pace, stretch, mode and tap are independent"
        )]
        #[allow(clippy::too_many_lines, reason = "a reading's run, one pass over its frames")]
        pub(in super::super) fn line_run(
            scene: &str,
            waiting: bool,
            seq: &str,
            arm: &str,
            drain: u32,
            png: Option<&std::path::Path>,
            wait: bool,
            stretch: f64,
            thread: bool,
            mut tap: Option<TapFn<'_>>,
        ) -> Option<(LineRun, Rig)> {
            let stages = tap.is_some();
            let portable = scene == "portable";
            let facing = spawn_facing();
            let (mut rig, cam, clock, sizes) = if portable {
                let rig = support::terms::forest_rig_about(RENOIR_EYE.map(|v| v as i32))?;
                let p = RENOIR_EYE;
                let cam = flythrough::camera_looking_at(p, [p[0] + facing[0], p[1] + facing[1], p[2]]);
                let clock = flythrough::daylight::Clock {
                    minutes: 15.0 * 60.0,
                    ..Default::default()
                };
                (rig, cam, clock, [[853, 480], [1280, 720]])
            } else {
                let (rig, cam) = support::terms::forest_rig()?;
                let clock = flythrough::daylight::Clock {
                    fixed_sun: Some([10.0, 180.0]),
                    ..Default::default()
                };
                (rig, cam, clock, [fl::INTERNAL, fl::OUT])
            };
            let sky = clock.sky(flythrough::outdoor::BASE_Z as f32);
            let cam = fl::pitched(&cam, fl::ABOVE_PITCH);
            let mut f = fl::flicker_frame_at(&rig, sky, None, false, flythrough::LeafShadow::Voxel, sizes);
            // The fly-through's queue flags (`BakeArgs`' defaults), and Portable's profile.
            f.set_bake_budget(svet_render::Budget::default());
            f.set_bake_coarse(svet_render::DEFAULT_COARSE_SAMPLES, svet_render::DEFAULT_NEAR);
            f.set_bake_tiers(svet_render::DEFAULT_TIERS);
            f.set_bake_drain(Some(svet_render::DEFAULT_DRAIN));
            f.set_bake_digs(svet_render::DEFAULT_BAKE_EDIT_MS, true);
            f.set_bake_ring1_stretch(stretch);
            if portable {
                let p = svet_render::RenderSettings::profile(svet_render::Profile::Portable);
                f.set_bake_budget(p.bake.budget);
                f.set_bake_tiers(p.bake.tiers);
                f.set_bake_drain(p.bake.drain);
                f.set_lighting(svet_render::Lighting {
                    lamp_bounce: p.lighting.lamp_bounce,
                    ..f.lighting()
                });
            }
            let out = sizes[1];
            fl::dig_arm(&mut f, arm);
            if arm.split(',').any(|t| t == "cand0") {
                // The rig-only arm of Plan 0099 Phase 2e (5): the bake draws no emitter
                // candidate, so a sample's time is its rays' and its fixed part's.
                f.set_bake_queue(|q| q.settings.candidates = 0);
            }
            if thread {
                f.share_bake_world(rig.world.clone());
                f.set_bake_thread(&rig.queue, true);
                f.set_bake_queue(|q| q.trace_digs = true);
            }
            let mut s = Stepper::new(&rig, f);
            s.wait = wait;
            if let Some(t) = tap.as_mut() {
                t(Tap::Start, &mut rig, &mut s);
            }
            let mut n = 0;
            let head = loop {
                let q = queue_step(&mut rig, &mut s.frame, &cam);
                s.run(&rig, &cam);
                n += 1;
                let done = if waiting {
                    q.drain_waiting > 0 && q.drain_waiting <= 101 && n > 1
                } else {
                    q.drain_waiting == 0 && n > 1
                };
                if done || n >= drain {
                    break format!(
                        "line {scene} {} {seq} {arm}{}{}: adapter {}, after {n} frames in view waiting {}, coarse {}, depth {}, jobs {}",
                        if waiting { "waiting" } else { "drained" },
                        if wait { " device-wait" } else { "" },
                        if thread { " thread" } else { " lockstep" },
                        rig.adapter,
                        q.drain_waiting,
                        q.coarse,
                        q.depth,
                        q.jobs
                    ) + &format!(
                        "; baked {} coarse opened {} keys baked {} withdrawn {} retiered {} reach {} partial {} provisional {} waiting reach {} lights {} unbaked {}; samples read {} total {} in {:.3} ms, ns a sample {:.3} (price {:.3} ms + {:.3} ns), slice {:.2} ms, draining {}",
                        q.baked, q.coarse_opened, q.keys_baked, q.withdrawn, q.retiered, q.reach, q.partial,
                        q.provisional, q.waiting_reach, q.waiting_lights, q.unbaked, q.samples, q.total_samples, q.ms, q.ns_per_sample, q.fixed_ms, q.cost_ns, q.slice_ms, q.draining
                    );
                }
            };
            let rd = s.frame.bake_queue().expect("the live queue").dig_trace().read;
            let head = format!("{head}; loop read {} wasted {} submits {}", rd[0], rd[1], rd[2]);
            println!("{head}");
            if !waiting {
                fl::settle(&mut rig, &mut s, &cam, 240);
            }
            let eye = cam.position.to_array();
            let fwd = cam.basis().0.to_array();
            let t = support::defects::march(&rig.world, eye, fwd, 400.0).expect("the view hits ground");
            let centre = [0, 1, 2].map(|k| eye[k] + fwd[k] * t);
            let (mut frames, mut traces, mut digs, mut edit_ms) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
            let (mut issue, mut pool, mut apply) = (Vec::new(), Vec::new(), Vec::new());
            let (mut times, mut parts) = (Vec::new(), Vec::new());
            #[allow(clippy::disallowed_methods, reason = "a reading times the frames' wall")]
            let clock = std::time::Instant::now;
            let origin = clock();
            let ms = move || clock().duration_since(origin).as_secs_f64() * 1e3;
            let mut read = |rig: &mut Rig, s: &mut Stepper, frames: &mut Vec<HoleFrame>, traces: &mut Vec<svet_render::DigTrace>, label: String, ring: Option<&mut Ring>| {
                let t0 = ms();
                let q = queue_step(rig, &mut s.frame, &cam);
                issue.push(s.frame.bake_frame_issue());
                apply.push(s.frame.bake_frame_apply());
                parts.push(s.frame.bake_frame_wait_parts());
                pool.push(ring.map(|ring| {
                    let cur = pool_ring(&s.frame, rig, ring.lo, ring.hi);
                    if ring.held.is_none() && support::queue::dig_held(&ring.pre, &cur) {
                        ring.held = Some(cur.clone());
                    }
                    let fresh = ring.held.as_ref().is_some_and(|h| support::queue::ring1_landed(h, &cur));
                    (ring.held.is_some(), fresh)
                }));
                traces.push(s.frame.bake_queue().expect("the live queue").dig_trace());
                let path = png.map(|d| d.join(format!("{:03}-{label}.png", frames.len())));
                frames.push(hole_frame(rig, s, &cam, q, label, out, stages, path.as_deref()));
                times.push((t0, ms()));
                q
            };
            for k in [2, 1] {
                read(&mut rig, &mut s, &mut frames, &mut traces, format!("N-{k}"), None);
            }
            let aims = aims_of(seq, &cam, centre);
            for (d, (aim, gap)) in aims.iter().enumerate() {
                let Some((dug, ms)) = dig_along_timed(&mut rig, &mut s.frame, aim) else {
                    println!("{head}: dig {} changed nothing", d + 1);
                    continue;
                };
                digs.push((frames.len(), dug.0, dug.1));
                edit_ms.push(ms);
                if let Some(t) = tap.as_mut() {
                    t(Tap::Edit { dig: d, lo: dug.0, hi: dug.1 }, &mut rig, &mut s);
                }
                // The ring as the pool holds it before any step has read the box.
                let mut ring = Ring { lo: dug.0, hi: dug.1, pre: pool_ring(&s.frame, &rig, dug.0, dug.1), held: None };
                let mut tail = None;
                for k in 0..1200usize {
                    let q = read(&mut rig, &mut s, &mut frames, &mut traces, format!("D{}+{k}", d + 1), Some(&mut ring));
                    if let (Some(t), Some(frame)) = (tap.as_mut(), frames.last()) {
                        t(Tap::Frame { dig: d, k, frame }, &mut rig, &mut s);
                    }
                    if k + 1 == *gap {
                        break;
                    }
                    if q.digs == 0 && k > 0 && tail.is_none() {
                        tail = Some(k + 16 + fl::DIG_TAIL);
                    }
                    // A dig with a gap waits it out (sequence `C`'s 64 frames outlast the tail).
                    if tail == Some(k) && *gap == usize::MAX {
                        break;
                    }
                }
            }
            Some((LineRun { frames, traces, digs, edit_ms, issue, apply, times, parts, pool, thread, out, eye, head }, rig))
        }

        /// Each dig's aim and the frames to the next ([`line_run`]'s sequences).
        fn aims_of(seq: &str, cam: &Camera, centre: [f32; 3]) -> Vec<(Camera, usize)> {
            match seq {
                "A" => vec![(*cam, usize::MAX), (*cam, 1), (*cam, usize::MAX)],
                // B's dig 1, then B's dig 4 alone 64 frames later (Plan 0099 Phase 2e).
                "C" => [(B_AIMS[0], C_GAP), (B_AIMS[3], usize::MAX)]
                    .iter()
                    .map(|(a, gap)| (aimed(cam, [centre[0] + a[0], centre[1] + a[1], centre[2]]), *gap))
                    .collect(),
                _ => B_AIMS
                    .iter()
                    .chain(std::iter::once(&B_AIMS[0]))
                    .enumerate()
                    .map(|(i, a)| {
                        let c = aimed(cam, [centre[0] + a[0], centre[1] + a[1], centre[2]]);
                        (c, if i < B_AIMS.len() { B_GAP } else { usize::MAX })
                    })
                    .collect(),
            }
        }

        /// A dig's ring-1 records as the pool held them after the edit, before a step read its box,
        /// and as it held them in the first frame that held the dig's block.
        struct Ring {
            lo: [i64; 3],
            hi: [i64; 3],
            pre: RingRecords,
            held: Option<RingRecords>,
        }

        /// Brick records by chunk and slot.
        pub(in super::super) type RingRecords = BTreeMap<([i32; 3], usize), Vec<u32>>;

        /// The ring-1 records of the box `lo ..= hi` (every brick within a voxel of it) as the
        /// pool holds them now, by chunk and slot, from the blocks the frame reads
        /// (`Frame::read_store_block`: each at the place the pool's chunk table names, read back).
        pub(in super::super) fn pool_ring(f: &svet_render::Frame, rig: &Rig, lo: [i64; 3], hi: [i64; 3]) -> RingRecords {
            let mut out = BTreeMap::new();
            let a = lo.map(|v| (v - 1).div_euclid(64) as i32);
            let b = hi.map(|v| (v + 1).div_euclid(64) as i32);
            for z in a[2]..=b[2] {
                for y in a[1]..=b[1] {
                    for x in a[0]..=b[0] {
                        let Some(w) = f.read_store_block(&rig.device, &rig.queue, svet_world::ChunkCoord([x, y, z])) else {
                            continue;
                        };
                        for (slot, r) in records(&w) {
                            let b = [slot % 8, slot / 8 % 8, slot / 64].map(|v| v as i32 * 8);
                            let b = [x * 64 + b[0], y * 64 + b[1], z * 64 + b[2]];
                            if near_box(b, lo, hi, 1) {
                                out.insert(([x, y, z], slot), r);
                            }
                        }
                    }
                }
            }
            out
        }

        /// Whether the brick whose lowest voxel is `b` lies within `pad` voxels of the box.
        fn near_box(b: [i32; 3], lo: [i64; 3], hi: [i64; 3], pad: i64) -> bool {
            (0..3).all(|k| i64::from(b[k]) + 7 >= lo[k] - pad && i64::from(b[k]) <= hi[k] + pad)
        }

        /// The timeline's text: a row a frame (the queue's cap and the submit's samples, the
        /// samples in flight on ring 1's bricks, ring 2's and other jobs', the free slots,
        /// pending, open and unread, the read-back's samples kept and drawn, its ms and mean
        /// ray length, the prices: the general one and the dig's by class, the step's and the
        /// passes' ms) and under it a row per
        /// watched chunk (where it is and what holds it); then per dig the frame its ring 1 and
        /// its ring 2 land, ring 1's samples and the samples thrown away with stale jobs.
        #[allow(clippy::too_many_lines, reason = "a reading's report, one pass over the frames")]
        pub(in super::super) fn line_text(run: &LineRun) -> String {
            let mut out = String::new();
            let _ = writeln!(out, "{}", run.head);
            let ring1 = |b: [i32; 3]| run.digs.iter().any(|&(_, lo, hi)| near_box(b, lo, hi, 1));
            // Per chunk: the frame it last held a dig job that was not stale.
            let mut in_dig: BTreeMap<[i32; 3], bool> = BTreeMap::new();
            let mut landed: BTreeMap<[i32; 3], Vec<usize>> = BTreeMap::new();
            // A chunk's ring-1 landings: a live dig job holding ring-1 bricks placed (the chunk
            // then waits with no ring-1 brick, or not at all).
            let mut landed1: BTreeMap<[i32; 3], Vec<usize>> = BTreeMap::new();
            let mut stale_seen: BTreeMap<[i32; 3], u64> = BTreeMap::new();
            let mut wasted = vec![0u64; run.digs.len()];
            let mut r1_samples: BTreeMap<([i32; 3], usize), u64> = BTreeMap::new();
            let mut pool_full = None;
            for (fi, (f, t)) in run.frames.iter().zip(&run.traces).enumerate() {
                let dig_now = run.digs.iter().rposition(|d| d.0 <= fi);
                let mut fl = [0u64; 3];
                for j in &t.jobs {
                    for b in &j.bricks {
                        let k = if !j.dig || j.stale {
                            2
                        } else if ring1(b.0) {
                            0
                        } else {
                            1
                        };
                        fl[k] += b.2;
                    }
                }
                fl[2] += t.others[1];
                let pf = f.q.pool_full;
                let pool = pool_full.map_or(0, |p| pf - p);
                pool_full = Some(pf);
                let _ = writeln!(
                    out,
                    "{fi:4} {:8} cap {:7} sent {:7} | flight r1 {:6} r2 {:6} other {:7} | free {:2} pend {:3} open {:2} unread {} withdrawn {} pool+{pool} | read {:7} drawn {:7} in {} at {:.4} ms, ray {:.2}, ns {:.1} price {:.3} ms + {:.2} ns r1 {:.2} dig {:.2} | step {:6.3} ms light {:6.3} temporal {:6.3} gpu {:6.2} | waiting {} depth {} digs {} stretched {} | wait {:.3} ms submits {}",
                    f.label, f.q.cap, f.q.submitted, fl[0], fl[1], fl[2], t.free_slots, t.pending, t.open,
                    t.in_flight, t.withdrawn, f.q.samples, f.q.drawn, f.q.read_submits, f.q.ms, f.q.ray_len, f.q.ns_per_sample, f.q.fixed_ms, f.q.cost_ns, f.q.ring1_ns, f.q.dig_ns, f.q.stages.total_ms, f.light_ms, f.temporal_ms, f.gpu_ms,
                    f.q.drain_waiting, f.q.depth, f.q.digs, u8::from(f.q.stretched),
                    run.issue.get(fi).map_or(0.0, |i| i.0),
                    run.issue.get(fi).map_or(0, |i| i.1)
                );
                let jobs: BTreeMap<[i32; 3], &svet_render::DigTraceJob> = t.jobs.iter().map(|j| (j.coord, j)).collect();
                let mut seen = BTreeSet::new();
                for &(c, near, slots, in_job) in &t.waiting {
                    seen.insert(c);
                    let hold = match jobs.get(&c) {
                        Some(j) if in_job => {
                            let [l, q, d] = j.totals();
                            format!(
                                "held by a stale {} job ({} at {}) left {l} flight {q} drawn {d}",
                                if j.dig { "dig" } else { "other" },
                                if j.open { "open" } else { "pending" },
                                j.at
                            )
                        }
                        _ if in_job => "held by a job".to_string(),
                        _ => "not opened (two slices or the pool)".to_string(),
                    };
                    let _ = writeln!(out, "       {c:?} digs ring1 {near} bricks {slots}: {hold}");
                }
                for j in &t.jobs {
                    let [l, q, d] = j.totals();
                    if j.stale {
                        stale_seen.insert(j.coord, d + q);
                    }
                    let mut r1 = 0;
                    if j.dig
                        && !j.stale
                        && let Some(di) = dig_now
                    {
                        r1 = j.bricks.iter().filter(|b| ring1(b.0)).map(|b| b.1 + b.2 + b.3).sum();
                        let e = r1_samples.entry((j.coord, di)).or_default();
                        *e = (*e).max(r1);
                    }
                    if seen.contains(&j.coord) && j.stale {
                        continue;
                    }
                    let _ = writeln!(
                        out,
                        "       {:?} {} job {} at {}{}: left {l} flight {q} drawn {d}, ring-1 bricks' samples {r1}",
                        j.coord,
                        if j.dig { "dig" } else { "other" },
                        if j.open { "open" } else if t.free_slots == 0 { "pending, no free slot" } else { "pending" },
                        j.at,
                        if j.stale { " STALE" } else { "" },
                    );
                }
                // Landings and thrown-away jobs: a chunk that held a live dig job and holds
                // none now, nor waits (ring 1's: nor waits with a ring-1 brick).
                let now: BTreeMap<[i32; 3], bool> = t
                    .jobs
                    .iter()
                    .filter(|j| j.dig && !j.stale)
                    .map(|j| (j.coord, j.bricks.iter().any(|b| ring1(b.0))))
                    .collect();
                let waits_near: BTreeMap<[i32; 3], bool> = t.waiting.iter().map(|w| (w.0, w.1)).collect();
                for (c, had1) in &in_dig {
                    if now.contains_key(c) {
                        continue;
                    }
                    if !seen.contains(c) {
                        landed.entry(*c).or_default().push(fi);
                        landed1.entry(*c).or_default().push(fi);
                        let _ = writeln!(out, "       {c:?} LANDED");
                    } else if *had1 && waits_near.get(c) == Some(&false) {
                        landed1.entry(*c).or_default().push(fi);
                        let _ = writeln!(out, "       {c:?} RING 1 LANDED");
                    }
                }
                in_dig = now;
                let alive: BTreeSet<[i32; 3]> = t.jobs.iter().filter(|j| j.stale).map(|j| j.coord).collect();
                let gone: Vec<[i32; 3]> = stale_seen.keys().filter(|c| !alive.contains(*c)).copied().collect();
                for c in gone {
                    let d = stale_seen.remove(&c).unwrap_or(0);
                    if let Some(di) = dig_now {
                        wasted[di] += d;
                    }
                    let _ = writeln!(out, "       {c:?} stale job ended, {d} samples thrown away");
                }
            }
            // Per dig: its ring chunks as the frame after it lists them.
            let _ = writeln!(out, "\nper dig (frames from the dig's frame D+0):");
            for (di, &(d0, lo, hi)) in run.digs.iter().enumerate() {
                let t = &run.traces[d0];
                let mut chunks: BTreeMap<[i32; 3], bool> = BTreeMap::new();
                for (c, near) in t.waiting.iter().map(|w| (w.0, w.1)).chain(
                    t.jobs
                        .iter()
                        .filter(|j| j.dig)
                        .map(|j| (j.coord, j.bricks.iter().any(|b| near_box(b.0, lo, hi, 1)))),
                ) {
                    *chunks.entry(c).or_default() |= near;
                }
                let first = |m: &BTreeMap<[i32; 3], Vec<usize>>, c: &[i32; 3]| {
                    m.get(c).and_then(|v| v.iter().find(|&&f| f >= d0).map(|f| f - d0))
                };
                let mut l = [None::<usize>; 2];
                let mut never = Vec::new();
                for (c, near) in &chunks {
                    if *near && let Some(x) = first(&landed1, c) {
                        l[0] = Some(l[0].map_or(x, |y: usize| y.max(x)));
                    }
                    match first(&landed, c) {
                        Some(x) => l[1] = Some(l[1].map_or(x, |y: usize| y.max(x))),
                        None => never.push(*c),
                    }
                }
                let r1: u64 = r1_samples.iter().filter(|((_, d), _)| *d == di).map(|(_, v)| v).sum();
                let _ = writeln!(
                    out,
                    "dig {} at frame {d0} box {lo:?}..{hi:?}: ring 1 lands D+{} every ring chunk D+{}, ring-1 chunks {} ring-2 {}, ring-1 samples {r1}, thrown away {}, not landed in the run {never:?}",
                    di + 1,
                    l[0].map_or("-".into(), |x| x.to_string()),
                    l[1].map_or("-".into(), |x| x.to_string()),
                    chunks.values().filter(|n| **n).count(),
                    chunks.values().filter(|n| !**n).count(),
                    wasted[di]
                );
                let cost: Vec<String> = (d0..(d0 + 9).min(run.frames.len()))
                    .map(|i| {
                        let f = &run.frames[i];
                        format!("{:.2}/{:.2}/{:.2}/{:.1}", f.q.stages.total_ms, f.light_ms, f.temporal_ms, f.gpu_ms)
                    })
                    .collect();
                let _ = writeln!(out, "  cost D+0..8 (step / light / temporal / gpu ms): {}", cost.join(" "));
                // The pool's reads of this dig's ring, its frames to the next dig.
                let end = run.digs.get(di + 1).map_or(run.frames.len(), |d| d.0);
                let first_pool = |want: fn((bool, bool)) -> bool| {
                    (d0..end).find(|&i| run.pool.get(i).copied().flatten().is_some_and(want)).map(|i| i - d0)
                };
                let held = first_pool(|p| p.0);
                let fresh = first_pool(|p| p.1);
                let step_at = |i: usize| run.frames[i].q.steps;
                let took = |i: usize| i > 0 && i < run.frames.len() && step_at(i) > step_at(i - 1);
                // A fade of 4 frames from ring 1's records applied: frames that took a step.
                let fade = fresh.map(|r| (d0 + r..d0 + r + 4).filter(|&i| took(i)).count());
                let unread = d0 > 0 && run.frames[d0].q.in_flight_full > run.frames[d0 - 1].q.in_flight_full;
                let (wait, submits) = run.issue.get(d0).copied().unwrap_or((f64::NAN, 0));
                let (apply_ms, over) = run.apply.get(d0).copied().unwrap_or((f64::NAN, 0));
                let _ = writeln!(
                    out,
                    "  pool {}: held record D+{} ring-1 records fresh D+{}; D+0 wait {wait:.3} ms submits {submits}, every slot unread {}, fade steps {}/4; D+0 apply {apply_ms:.3} ms over the cap {over} bytes",
                    if run.thread { "thread" } else { "lockstep" },
                    held.map_or("-".into(), |x| x.to_string()),
                    fresh.map_or("-".into(), |x| x.to_string()),
                    u8::from(unread),
                    fade.map_or("-".into(), |n| n.to_string()),
                );
                // The dig in ms: from the edit's frame's start to the end of the frame that held
                // the block and of the one whose ring-1 records were fresh; the mean frame over
                // the edit's frame to ring 1's; the wait's parts and the frame's laps.
                let at = |r: Option<usize>| {
                    r.and_then(|r| run.times.get(d0 + r)).map(|t| t.1 - run.times[d0].0)
                };
                let shown = |v: Option<f64>| v.map_or("-".into(), |v| format!("{v:.2}"));
                let mean = fresh.and_then(|r| at(Some(r)).map(|v| v / (r + 1) as f64));
                let (p, issue_lap) = run.parts.get(d0).copied().flatten().map_or(([f64::NAN; 6], f64::NAN), |p| p);
                let _ = writeln!(
                    out,
                    "  ms {}: held at {} ring 1 at {} mean frame {}; parts rest {:.3} pickup {:.3} boxes {:.3} step_rest {:.3} snapshot {:.3} handback {:.3} sum {:.3}; issue {issue_lap:.3} apply {apply_ms:.3} submits {submits}; call {:.3}",
                    if run.thread { "thread" } else { "lockstep" },
                    shown(at(held)),
                    shown(at(fresh)),
                    shown(mean),
                    p[0], p[1], p[2], p[3], p[4], p[5],
                    p.iter().sum::<f64>(),
                    run.frames[d0].q.stages.total_ms,
                );
                // The dig frame's step by stage, the stages over 0.5 ms.
                let st = &run.frames[d0].q.stages;
                let laps: Vec<String> = svet_render::debug::STEP_STAGES
                    .iter()
                    .zip(st.ms)
                    .filter(|(_, ms)| *ms > 0.5)
                    .map(|(n, ms)| format!("{n} {ms:.2}"))
                    .collect();
                let _ = writeln!(out, "  D+0 stages over 0.5 ms (opened {}): {}", st.opened, laps.join(", "));
                let lap = |name: &str| {
                    let k = svet_render::debug::STEP_STAGES.iter().position(|n| *n == name).expect("a stage");
                    st.ms[k]
                };
                let _ = writeln!(
                    out,
                    "  D+0 step {:.3} ms render {:.3} movable {:.3} boxes {:.3} digs {:.3} bytes {} {:?}; edit wall {:.3} ms",
                    st.total_ms,
                    st.render(),
                    st.movable(),
                    lap("boxes"),
                    lap("digs"),
                    st.bytes_total(),
                    st.bytes,
                    run.edit_ms.get(di).copied().unwrap_or(f64::NAN)
                );
            }
            let rest = &run.frames[..run.digs.first().map_or(run.frames.len(), |d| d.0)];
            let mean = |k: fn(&HoleFrame) -> f64| rest.iter().map(k).sum::<f64>() / rest.len().max(1) as f64;
            let _ = writeln!(
                out,
                "before the first dig (mean of {}): step {:.3} light {:.3} temporal {:.3} gpu {:.2} ms",
                rest.len(),
                mean(|f| f.q.stages.total_ms),
                mean(|f| f.light_ms),
                mean(|f| f.temporal_ms),
                mean(|f| f.gpu_ms)
            );
            out
        }

        /// Plan 0098 Phase 16 step 1's timeline: `SVET_LINE` is
        /// `SCENE/STATE/SEQ/ARM/DRAIN/PACE/STRETCH/MODE` (`above` or `portable`, `drained` or
        /// `waiting`, `A`, `B` or `C`, `support::defects`' dig arm tokens, the most frames the
        /// queue drains, `wait` for the stepper's device-wait mode or `free`, the ring-1 stretch,
        /// [`svet_render::DEFAULT_RING1_STRETCH`] by default, named `-sX` in the file's name
        /// otherwise, and `thread` for the bake thread or `lock`, lockstep, by default, named
        /// `-thread`); the text under
        /// `live/p16/<adapter>-<scene>-<state>-<seq>-<arm>[-wait][-sX][-thread].txt`, the frames'
        /// PNGs beside it with `SVET_LINE_PNG` set.
        #[test]
        #[ignore = "reading"]
        fn dig_timeline() {
            let v = std::env::var("SVET_LINE").unwrap_or_default();
            let mut it = v.split('/');
            let mut part = |d: &str| it.next().filter(|s| !s.is_empty()).unwrap_or(d).to_string();
            let (scene, state, seq, arm) = (part("above"), part("drained"), part("A"), part("default"));
            let drain: u32 = part("30000").parse().expect("SVET_LINE's drain");
            let wait = match part("wait").as_str() {
                "wait" => true,
                "free" => false,
                p => panic!("SVET_LINE's pace {p}: wait or free"),
            };
            let stretch: f64 = part("1.5").parse().expect("SVET_LINE's stretch");
            let thread = match part("lock").as_str() {
                "thread" => true,
                "lock" => false,
                m => panic!("SVET_LINE's mode {m}: thread or lock"),
            };
            let adapter = std::env::var("SVET_TEST_ADAPTER").unwrap_or_else(|_| "lavapipe".into());
            let name = format!(
                "{adapter}-{scene}-{state}-{seq}-{}{}{}",
                arm.replace([',', '='], "-"),
                if wait { "-wait" } else { "" },
                if stretch == svet_render::DEFAULT_RING1_STRETCH { String::new() } else { format!("-s{stretch}") }
            ) + if thread { "-thread" } else { "" };
            let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../live/p16");
            let png = std::env::var("SVET_LINE_PNG").is_ok().then(|| dir.join(&name));
            if let Some(p) = &png {
                std::fs::create_dir_all(p).expect("the stills' directory");
            }
            std::fs::create_dir_all(&dir).expect("the reading's directory");
            let Some((run, _rig)) = line_run(&scene, state == "waiting", &seq, &arm, drain, png.as_deref(), wait, stretch, thread, None) else {
                return;
            };
            let text = line_text(&run);
            let summary: String = text
                .lines()
                .filter(|l| {
                    l.starts_with("line ") || l.starts_with("dig ") || l.starts_with("  cost") || l.starts_with("  pool") || l.starts_with("  ms ") || l.starts_with("  D+0 step") || l.starts_with("before")
                })
                .collect::<Vec<_>>()
                .join("\n");
            println!("{summary}\neye {:?} out {:?}", run.eye, run.out);
            std::fs::write(dir.join(format!("{name}.txt")), text).expect("the timeline");
        }

        /// A block's brick records by slot ([`svet_render::internals::BRICK_WORDS`] words each,
        /// the directory's run end in each slot's word).
        pub(in super::super) fn records(block: &[u32]) -> BTreeMap<usize, Vec<u32>> {
            use svet_render::internals::{BRICK_WORDS, SUMMARY_AT};
            let mut out = BTreeMap::new();
            for (slot, &d) in block[..SUMMARY_AT as usize].iter().enumerate() {
                if d != 0 {
                    let end = (d >> 11) as usize;
                    out.insert(slot, block[end - BRICK_WORDS..end].to_vec());
                }
            }
            out
        }

        /// A record's means over its faces: the direction values' luminance, the sky and key
        /// visibilities, and its second sky word's flag bits (16 and up).
        fn record_means(r: &[u32]) -> [f64; 4] {
            use svet_render::internals::{record_key_visibility, record_visibility, rgb9e5_unpack};
            let (mut n, mut l, mut v, mut k) = (0.0, 0.0, 0.0, 0.0);
            for f in 0..6 {
                if (r[6] >> (16 + f)) & 1 == 0 {
                    continue;
                }
                n += 1.0;
                let c = rgb9e5_unpack(r[f]);
                l += 0.2126 * f64::from(c[0]) + 0.7152 * f64::from(c[1]) + 0.0722 * f64::from(c[2]);
                v += f64::from(record_visibility(r, f).unwrap_or(-1.0));
                k += f64::from(record_key_visibility(r, f).unwrap_or(-1.0));
            }
            let n = f64::max(n, 1.0);
            [l / n, v / n, k / n, f64::from(r[8] >> 16)]
        }

        /// The blocks of the chunks with one within `pad` voxels of the box.
        fn ring_blocks(f: &svet_render::Frame, lo: [i64; 3], hi: [i64; 3], pad: i64) -> BTreeMap<[i32; 3], Vec<u32>> {
            let mut out = BTreeMap::new();
            let a = lo.map(|v| (v - pad).div_euclid(64) as i32);
            let b = hi.map(|v| (v + pad).div_euclid(64) as i32);
            for z in a[2]..=b[2] {
                for y in a[1]..=b[1] {
                    for x in a[0]..=b[0] {
                        if let Some(w) = f.store().block_words(svet_world::ChunkCoord([x, y, z])) {
                            out.insert([x, y, z], w.to_vec());
                        }
                    }
                }
            }
            out
        }

        /// Plan 0098 Phase 16 step 1 (b): the land arm's ring chunks, word by word. Three rigs
        /// of [`line_run`]'s scene `above` (the queue stepped 3000 frames, settled): the first
        /// dig of sequence A under `hold=land` and under the default, each until it lands plus
        /// 16 and [`fl::DIG_TAIL`], the ring's blocks before and after; and a third with no dig
        /// whose chunks take the coarse pass first (`--coarse 256`, full-first none), each
        /// ring chunk's block read the step it is first coarse. Printed per ring chunk: the
        /// means of the records the dig changed under each; for the chunk whose land block is
        /// darkest against the default's, which records and which words differ.
        #[test]
        #[ignore = "reading"]
        fn land_words() {
            let mut arms = Vec::new();
            for arm in ["hold=land", "default"] {
                let Some((run, rig, before, after)) = land_run(arm) else {
                    return;
                };
                arms.push((arm, run, before, after));
                drop(rig);
            }
            let (lo, hi) = (arms[0].1 .0, arms[0].1 .1);
            let coarse = coarse_blocks(lo, hi).unwrap_or_default();
            let mut out = String::new();
            let mut worst: Option<(f64, [i32; 3])> = None;
            for (c, before) in &arms[1].2 {
                let b = records(before);
                let mut row = format!("{c:?}:");
                let mut lum = [0.0f64; 3];
                for (k, blocks) in [&arms[0].3, &arms[1].3, &coarse].into_iter().enumerate() {
                    let Some(w) = blocks.get(c) else {
                        row += " -";
                        continue;
                    };
                    let r = records(w);
                    let changed: Vec<[f64; 4]> = r
                        .iter()
                        .filter(|(s, v)| k == 2 || b.get(s) != Some(v))
                        .map(|(_, v)| record_means(v))
                        .collect();
                    let n = changed.len().max(1) as f64;
                    let m = changed.iter().fold([0.0; 4], |a, x| [a[0] + x[0], a[1] + x[1], a[2] + x[2], a[3] + x[3]]);
                    lum[k] = m[0] / n;
                    row += &format!(
                        " {}: {} records, lum {:.4} sky {:.3} key {:.3} flags {:.1}",
                        ["land", "default", "coarse first"][k],
                        changed.len(),
                        m[0] / n,
                        m[1] / n,
                        m[2] / n,
                        m[3] / n
                    );
                }
                let ratio = lum[0] / lum[1].max(1e-9);
                if worst.is_none_or(|w| ratio < w.0) && arms[0].3.contains_key(c) {
                    worst = Some((ratio, *c));
                }
                let _ = writeln!(out, "{row}; land / default {ratio:.3}");
            }
            if let Some((ratio, c)) = worst {
                let _ = writeln!(out, "\nthe darkest: {c:?} (land / default {ratio:.3}); per brick slot the differing words (index: land default coarse-first)");
                let l = records(&arms[0].3[&c]);
                let d = records(arms[1].3.get(&c).expect("the default's block"));
                let cf = coarse.get(&c).map(|w| records(w)).unwrap_or_default();
                let bf = records(&arms[1].2[&c]);
                let (mut kept, mut new_rec, mut diff) = (0, 0, [0usize; 11]);
                for (slot, lw) in &l {
                    let Some(dw) = d.get(slot) else {
                        new_rec += 1;
                        continue;
                    };
                    if bf.get(slot) == Some(lw) && lw == dw {
                        kept += 1;
                        continue;
                    }
                    let mut line = format!("  slot {slot}:");
                    for i in 0..11 {
                        if lw[i] != dw[i] {
                            diff[i] += 1;
                            let cw = cf.get(slot).map_or(0, |v| v[i]);
                            line += &format!(" {i}: {:08x} {:08x} {cw:08x}", lw[i], dw[i]);
                        }
                    }
                    let (ml, md) = (record_means(lw), record_means(dw));
                    let _ = writeln!(out, "{line} | lum {:.4} {:.4} sky {:.3} {:.3}", ml[0], md[0], ml[1], md[1]);
                }
                let head: Vec<usize> = (svet_render::internals::SUMMARY_AT as usize..=svet_render::internals::LANDED_AT as usize)
                    .filter(|&i| arms[0].3[&c].get(i) != arms[1].3[&c].get(i))
                    .collect();
                let _ = writeln!(out, "records kept alike {kept}, only in land {new_rec}, differing per word {diff:?}; head words differing {head:?}");
            }
            println!("{out}");
            let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../live/p16");
            std::fs::write(dir.join("land-words.txt"), out).expect("the table");
        }

        /// One arm of [`land_words`]: the dig's box, the rig, and the ring's blocks before the
        /// dig and after its landing.
        #[allow(clippy::type_complexity, reason = "a reading's tuple")]
        fn land_run(arm: &str) -> Option<(([i64; 3], [i64; 3]), Rig, BTreeMap<[i32; 3], Vec<u32>>, BTreeMap<[i32; 3], Vec<u32>>)> {
            let (mut rig, mut s, cam) = above_rig(arm, true)?;
            for _ in 0..3000 {
                queue_step(&mut rig, &mut s.frame, &cam);
                s.run(&rig, &cam);
            }
            fl::settle(&mut rig, &mut s, &cam, 240);
            let eye = cam.position.to_array();
            let fwd = cam.basis().0.to_array();
            let t = support::defects::march(&rig.world, eye, fwd, 400.0).expect("the view hits ground");
            let c = [0, 1, 2].map(|k| (eye[k] + fwd[k] * t) as i64);
            let before = ring_blocks(&s.frame, [c[0] - 6, c[1] - 6, c[2] - 6], [c[0] + 6, c[1] + 6, c[2] + 6], 40);
            let dug = dig_along(&mut rig, &mut s.frame, &cam)?;
            let mut tail = None;
            for k in 0..1200usize {
                let q = queue_step(&mut rig, &mut s.frame, &cam);
                s.run(&rig, &cam);
                if q.digs == 0 && k > 0 && tail.is_none() {
                    tail = Some(k + 16 + fl::DIG_TAIL);
                    println!("land words {arm}: landed at D+{k}");
                }
                if tail == Some(k) {
                    break;
                }
            }
            let after = ring_blocks(&s.frame, dug.0, dug.1, 32);
            Some((dug, rig, before, after))
        }

        /// The coarse-first rig of [`land_words`]: each chunk within 32 voxels of the box,
        /// its block the step it is first coarse (3000 steps at most).
        fn coarse_blocks(lo: [i64; 3], hi: [i64; 3]) -> Option<BTreeMap<[i32; 3], Vec<u32>>> {
            let (mut rig, mut s, cam) = above_rig("default", false)?;
            let mut out = BTreeMap::new();
            for k in 0..3000 {
                queue_step(&mut rig, &mut s.frame, &cam);
                s.run(&rig, &cam);
                for (c, w) in ring_blocks(&s.frame, lo, hi, 32) {
                    if s.frame.store().is_coarse(svet_world::ChunkCoord(c)) && !out.contains_key(&c) {
                        println!("coarse first: {c:?} at step {k}");
                        out.insert(c, w);
                    }
                }
            }
            Some(out)
        }

        /// [`line_run`]'s scene `above` frame and camera under `arm`; `full_first` false gives
        /// every chunk with no block the coarse pass first.
        fn above_rig(arm: &str, full_first: bool) -> Option<(Rig, Stepper, Camera)> {
            let (rig, cam) = support::terms::forest_rig()?;
            let clock = flythrough::daylight::Clock {
                fixed_sun: Some([10.0, 180.0]),
                ..Default::default()
            };
            let sky = clock.sky(flythrough::outdoor::BASE_Z as f32);
            let cam = fl::pitched(&cam, fl::ABOVE_PITCH);
            let mut f = fl::flicker_frame_at(&rig, sky, None, false, flythrough::LeafShadow::Voxel, [fl::INTERNAL, fl::OUT]);
            f.set_bake_budget(svet_render::Budget::default());
            f.set_bake_coarse(svet_render::DEFAULT_COARSE_SAMPLES, svet_render::DEFAULT_NEAR);
            f.set_bake_tiers(svet_render::DEFAULT_TIERS);
            f.set_bake_drain(Some(svet_render::DEFAULT_DRAIN));
            f.set_bake_digs(svet_render::DEFAULT_BAKE_EDIT_MS, true);
            if !full_first {
                f.set_bake_full_first(Some(-1));
            }
            fl::dig_arm(&mut f, arm);
            let s = Stepper::new(&rig, f);
            Some((rig, s, cam))
        }
    }

    /// Plan 0098 Phase 17's image test of a fresh hole ([`reading::line_run`]'s scene `above`,
    /// the owner's pose): its claims' figures over one run.
    pub(crate) mod hold {
        use super::super::support::defects::{self, HoleFigures, HoleRun};
        use super::reading::{LineRun, line_run};

        /// The store's fade from a dig's held record to its job's, frames.
        const DIG_FADE: usize = svet_render::internals::DIG_FADE as usize;

        /// The levels the histogram reads the hole's excess at.
        const LEVELS: [i32; 4] = [8, 16, 32, 64];

        /// One run's figures: each dig's ring-1 landing in frames from its own frame; the
        /// worst shares (%) of the hole's pixels over 32 levels whiter or blacker than both
        /// `before` and `after` from each dig's frame to its landing plus [`DIG_FADE`], and over
        /// 4 levels after that until its group ends (groups at ring 1's landing, `after` the
        /// frame before the next group's dig); the ring's at 4 levels in every frame from
        /// the first dig; the dots against the settled frame's; the ring's and the frame's
        /// shift before the first landing; the largest step away from a group's settled mean
        /// after its landing; and the histogram's text.
        pub(crate) struct Claims {
            pub land: Vec<Option<usize>>,
            pub before: [f64; 2],
            pub after: [f64; 2],
            pub ring: [f64; 2],
            pub dots: [f64; 2],
            pub shift: f64,
            pub away: f64,
            pub text: String,
        }

        /// Each dig's ring-1 landing: the first frame from its own whose queue trace holds no
        /// live dig job with a brick within one voxel of its box and no waiting ring-1 chunk the
        /// box dilated by one voxel reaches.
        fn ring1_landings(run: &LineRun) -> Vec<Option<usize>> {
            let near = |b: [i32; 3], lo: [i64; 3], hi: [i64; 3]| {
                (0..3).all(|k| i64::from(b[k]) + 7 >= lo[k] - 1 && i64::from(b[k]) <= hi[k] + 1)
            };
            let reaches = |c: [i32; 3], lo: [i64; 3], hi: [i64; 3]| {
                (0..3).all(|k| i64::from(c[k]) * 64 + 63 >= lo[k] - 1 && i64::from(c[k]) * 64 <= hi[k] + 1)
            };
            run.digs
                .iter()
                .map(|&(d0, lo, hi)| {
                    (d0..run.traces.len()).find(|&f| {
                        let t = &run.traces[f];
                        !t.jobs.iter().any(|j| j.dig && !j.stale && j.bricks.iter().any(|b| near(b.0, lo, hi)))
                            && !t.waiting.iter().any(|w| w.1 && reaches(w.0, lo, hi))
                    })
                })
                .collect()
        }

        /// Plan 0105 Phase 7a2's print (a reading, nothing asserted). Per dig: the change of
        /// `QueueStats::digs_whole` from the frame before the dig's first frame (D-1) to it (D+0),
        /// and from D+0 to D+1. Per frame whose ring breaks 2 % at 4 levels: its ring pixels
        /// darker than both `before` and `after` by over 4 levels ([`defects::before_after`],
        /// the pixels [`defects::hole_figures_in`] counts) by the store's table word of their
        /// hit's chunk in the `before` frame (none, a block, coarse, or unseen: the chunk under
        /// no pixel of that frame), each split by whether their hit's brick (the hit point's
        /// voxel's) is one a dig's box made by that frame, dilated by one voxel, touches; and the
        /// ring's dark shares over 8, 16 and 32 levels.
        fn whole_text(run: &HoleRun, groups: &[(usize, usize)], f4: &[HoleFigures]) -> String {
            use std::collections::HashMap;
            let mut text = String::new();
            for (k, &(d0, _, _)) in run.digs.iter().enumerate() {
                let whole = |i: usize| run.frames.get(i).map(|f| f.q.digs_whole);
                let step = |a: usize, b: usize| match (whole(a), whole(b)) {
                    (Some(x), Some(y)) => format!("{:+}", y as i64 - x as i64),
                    _ => "-".into(),
                };
                text += &format!(
                    "  dig {} ({}): digs whole {:?} at D+0, {} from D-1, {} to D+1\n",
                    k + 1,
                    run.frames[d0].label,
                    whole(d0),
                    step(d0 - 1, d0),
                    step(d0, d0 + 1)
                );
            }
            let (dw, dh) = (run.out[0] as usize, run.out[1] as usize);
            let chunk_of = |h: &[f32; 4]| [0, 1, 2].map(|k| (h[k].floor() as i32).div_euclid(64));
            let first = run.digs[0].0;
            for (fi, fig) in f4.iter().enumerate().skip(first) {
                if fig.white_ring.max(fig.black_ring) <= 2.0 {
                    continue;
                }
                let f = &run.frames[fi];
                let (bi, ai) = defects::before_after(run, groups, fi);
                let (c, cb, ca) = (
                    defects::hole_classes(run, f).1,
                    defects::hole_classes(run, &run.frames[bi]).1,
                    defects::hole_classes(run, &run.frames[ai]).1,
                );
                let (before, after) = (&run.frames[bi].luma, &run.frames[ai].luma);
                // The before frame's table word by chunk (its pixels' hits; the largest word a
                // chunk shows).
                let mut words: HashMap<[i32; 3], u8> = HashMap::new();
                for (h, &w) in run.frames[bi].hits.iter().zip(&run.frames[bi].table) {
                    if h[3] < 1e30 && h[3].is_finite() {
                        let e = words.entry(chunk_of(h)).or_insert(w);
                        *e = (*e).max(w);
                    }
                }
                let gl = f.hits.len();
                let gw = ((gl as f64 * dw as f64 / dh as f64).sqrt().round() as usize).max(1);
                let boxes: Vec<([i64; 3], [i64; 3])> =
                    run.digs.iter().filter(|d| d.0 <= fi).map(|d| (d.1, d.2)).collect();
                let touched = |h: &[f32; 4]| {
                    let v = [0, 1, 2].map(|k| i64::from(h[k].floor() as i32));
                    boxes.iter().any(|(lo, hi)| {
                        (0..3).all(|k| {
                            let b = v[k].div_euclid(8) * 8;
                            b + 7 >= lo[k] - 1 && b <= hi[k] + 1
                        })
                    })
                };
                // [word: none, block, coarse, unseen][touched: no, yes]
                let mut by = [[0u64; 2]; 4];
                let (mut nr, mut dark) = (0u64, [0u64; 4]);
                for i in 0..f.luma.len() {
                    if !(c[i] == defects::DIG_RING_CLASS && cb[i] == defects::DIG_RING_CLASS && ca[i] == defects::DIG_RING_CLASS) {
                        continue;
                    }
                    nr += 1;
                    let (l, b, a) = (i32::from(f.luma[i]), i32::from(before[i]), i32::from(after[i]));
                    for (n, lv) in [4, 8, 16, 32].into_iter().enumerate() {
                        dark[n] += u64::from(l < b.min(a) - lv);
                    }
                    if l >= b.min(a) - 4 {
                        continue;
                    }
                    let h = &f.hits[defects::grid_of(i, dw, dh, gw, gl)];
                    let w = words.get(&chunk_of(h)).map_or(3, |&w| usize::from(w.min(2)));
                    by[w][usize::from(touched(h))] += 1;
                }
                let pct = |a: u64| 100.0 * a as f64 / nr.max(1) as f64;
                let n4 = dark[0].max(1) as f64;
                let cell = |w: usize| {
                    format!(
                        "{} touched / {} not ({:.0} / {:.0} %)",
                        by[w][1],
                        by[w][0],
                        100.0 * by[w][1] as f64 / n4,
                        100.0 * by[w][0] as f64 / n4
                    )
                };
                text += &format!(
                    "  {} (before {}): ring {} px, dark over 4 {} ({:.2} %), over 8 {:.2} %, 16 {:.2} %, 32 {:.2} %; by the before frame's chunk word: none {}; block {}; coarse {}; unseen {}\n",
                    f.label,
                    run.frames[bi].label,
                    nr,
                    dark[0],
                    pct(dark[0]),
                    pct(dark[1]),
                    pct(dark[2]),
                    pct(dark[3]),
                    cell(0),
                    cell(1),
                    cell(2),
                    cell(3)
                );
            }
            text
        }

        /// The run of `seq` (`A` or `B`) under `arm` at the image test's start (`waiting`
        /// false: one frame and the settle) or after 600 frames (`waiting`), on the adapter
        /// the test support opens, with its claims' figures; `wait` the stepper's device-wait
        /// mode ([`line_run`]'s).
        pub(crate) fn claims(seq: &str, waiting: bool, arm: &str, wait: bool) -> Option<Claims> {
            let drain = if waiting { 600 } else { 1 };
            let (line, _rig) = line_run("above", waiting, seq, arm, drain, None, wait, svet_render::DEFAULT_RING1_STRETCH, false, None)?;
            let land = ring1_landings(&line);
            let hold = !arm.split(',').any(|t| t == "hold=off");
            let LineRun { frames, digs, out, eye, head, .. } = line;
            let run = HoleRun { frames, n1: 1, digs, out, hold, eye };
            // The groups at ring 1's landing (the owner's pick, 2026-11-24): a dig made at or
            // before the ring-1 landing of the one ahead joins its group, so a dig's `after` is
            // the frame before the next group's dig (B: the frame before the next dig), not the
            // queue's landing of every dig job (ring 2's, D+10 to 50 on lavapipe, which put all
            // of B in one group read against the run's last frame).
            let groups = defects::hole_groups_at(&run, &land);
            let figs: Vec<Vec<HoleFigures>> = std::iter::once(4)
                .chain(LEVELS)
                .map(|l| defects::hole_figures_in(&run, l, &groups).0)
                .collect();
            let (f4, at) = (&figs[0], |l: i32| &figs[1 + LEVELS.iter().position(|&x| x == l).expect("a level")]);
            let last = run.frames.len() - 1;
            // Each frame's span: before the landing and its fade (true) or after (false), by the
            // latest dig at or before it.
            let mut early = vec![None::<bool>; run.frames.len()];
            for (k, &(d0, _, _)) in run.digs.iter().enumerate() {
                let end = groups.iter().find(|g| d0 <= g.1).map_or(last, |g| g.1);
                let fade_end = land[k].map_or(end, |l| (l + DIG_FADE).min(end));
                for (fi, e) in early.iter_mut().enumerate().take(end + 1).skip(d0) {
                    *e = Some(fi <= fade_end);
                }
            }
            let worst = |fs: &[HoleFigures], pick: &dyn Fn(usize) -> bool, k: fn(&HoleFigures) -> f64| {
                (0..fs.len()).filter(|&i| pick(i)).map(|i| k(&fs[i])).fold(0.0f64, f64::max)
            };
            let is_early = |i: usize| early[i] == Some(true);
            let is_late = |i: usize| early[i] == Some(false);
            let first = run.digs[0].0;
            let from_first = |i: usize| i >= first;
            let mut text = format!("{head}\n");
            for &l in &LEVELS {
                let fs = at(l);
                text += &format!(
                    "  over {l:>2} levels: before the landing + fade white {:.2} % black {:.2} %; settled (last two frames) white {:.2} % black {:.2} %\n",
                    worst(fs, &is_early, |f| f.white_hole),
                    worst(fs, &is_early, |f| f.black_hole),
                    worst(fs, &|i| i + 1 >= last, |f| f.white_hole),
                    worst(fs, &|i| i + 1 >= last, |f| f.black_hole)
                );
            }
            // The frames whose ring or hole breaks a 4-level share of 2 %, by label.
            let over: Vec<String> = (first..=last)
                .filter(|&i| f4[i].white_ring.max(f4[i].black_ring) > 2.0 || (is_late(i) && f4[i].white_hole.max(f4[i].black_hole) > 2.0))
                .map(|i| {
                    let f = &f4[i];
                    format!("{} ring {:.1}/{:.1} hole {:.1}/{:.1}", run.frames[i].label, f.white_ring, f.black_ring, f.white_hole, f.black_hole)
                })
                .collect();
            text += &format!("  frames over 2 % at 4 levels: {}\n", over.join("; "));
            text += &whole_text(&run, &groups, f4);
            let first_landing = land.iter().flatten().min().copied().unwrap_or(last);
            let shift = f4[first..first_landing.max(first)]
                .iter()
                .map(|f| f.ring_shift.abs().max(f.out_shift.abs()))
                .fold(0.0f64, f64::max);
            // The one fade: the largest step away from a group's settled mean after its landing.
            let mut away = 0.0f64;
            for &(start, gend) in &groups {
                let Some(l) = (0..run.digs.len())
                    .filter(|&k| run.digs[k].0 >= start && run.digs[k].0 <= gend)
                    .map(|k| land[k])
                    .collect::<Option<Vec<_>>>()
                    .and_then(|v| v.into_iter().max())
                else {
                    continue;
                };
                let settled = f4[gend].hole_mean;
                for fi in l + 1..=gend {
                    let step = (f4[fi].hole_mean - settled).abs() - (f4[fi - 1].hole_mean - settled).abs();
                    away = away.max(step);
                }
            }
            let land: Vec<Option<usize>> = land.iter().zip(&run.digs).map(|(l, d)| l.map(|l| l - d.0)).collect();
            Some(Claims {
                before: [worst(at(32), &is_early, |f| f.white_hole), worst(at(32), &is_early, |f| f.black_hole)],
                after: [worst(f4, &is_late, |f| f.white_hole), worst(f4, &is_late, |f| f.black_hole)],
                ring: [worst(f4, &from_first, |f| f.white_ring), worst(f4, &from_first, |f| f.black_ring)],
                dots: [worst(f4, &from_first, |f| f.dots), f4[last].dots],
                shift,
                away,
                land,
                text,
            })
        }

        mod reading {
        use super::super::super::support::defects::{self, HoleRun, Stepper};
        use super::super::super::support::terms::Rig;
        use super::super::reading::{LineRun, Tap, line_run, records};
        use super::claims;
        use std::collections::{BTreeMap, BTreeSet};
        use std::fmt::Write as _;
        use svet_render::debug::HeldPart;
        use svet_render::internals::{BRICK_WORDS, SUMMARY_AT, face_classes, list_world_chunk, record_sky_stale, record_visibility, rgb9e5_unpack};
        use svet_world::ChunkCoord;

        /// Plan 0098 Phase 17 step 1's histogram: [`claims`] of A and B on the waiting queue
        /// under `SVET_DIG`'s arm (`support::defects::dig_spec`), printed, nothing asserted.
        #[test]
        #[ignore = "reading"]
        fn hold_histogram() {
            let (_, arm, _) = super::super::super::support::defects::dig_spec(("above", "default", 0));
            for seq in ["A", "B"] {
                let Some(c) = claims(seq, true, &arm, false) else {
                    return;
                };
                println!(
                    "{}hole {seq} {arm}: ring 1 lands D+{:?}; before the landing white {:.2} % black {:.2} % (32 levels), after white {:.2} % black {:.2} %, ring white {:.2} % black {:.2} %, dots {:.2} % (settled {:.2} %), shift {:.2}, away {:+.2}",
                    c.text, c.land, c.before[0], c.before[1], c.after[0], c.after[1], c.ring[0], c.ring[1], c.dots[0], c.dots[1], c.shift, c.away
                );
            }
        }

        /// A dig's edit as the CPU store read it (Plan 0105 Phase 7a3's T1 and T2): the box, the
        /// chunks its box dilated by one voxel touches, each one's block before the edit and
        /// after the step that read the box (the store's copy) and the pool's after that frame
        /// (`Frame::read_store_block`), the listing after the edit (bricks, faces, the touched
        /// bricks that list faces), the chain of the chunks and their face neighbours before and
        /// after, and the held trace's parts the step pushed.
        #[derive(Default)]
        struct DigCpu {
            lo: [i64; 3],
            hi: [i64; 3],
            chunks: Vec<[i32; 3]>,
            before: BTreeMap<[i32; 3], Option<Vec<u32>>>,
            after: BTreeMap<[i32; 3], Option<Vec<u32>>>,
            pool: BTreeMap<[i32; 3], Option<Vec<u32>>>,
            listed: BTreeMap<[i32; 3], [usize; 3]>,
            chain_before: BTreeMap<[i32; 3], Option<[u32; BRICK_WORDS]>>,
            chain_after: BTreeMap<[i32; 3], Option<[u32; BRICK_WORDS]>>,
            trace0: usize,
            trace: Vec<HeldPart>,
        }

        /// The chunks box `lo ..= hi` dilated by one voxel touches.
        fn box_chunks(lo: [i64; 3], hi: [i64; 3]) -> Vec<[i32; 3]> {
            let a = lo.map(|v| (v - 1).div_euclid(64) as i32);
            let b = hi.map(|v| (v + 1).div_euclid(64) as i32);
            let mut out = Vec::new();
            for z in a[2]..=b[2] {
                for y in a[1]..=b[1] {
                    for x in a[0]..=b[0] {
                        out.push([x, y, z]);
                    }
                }
            }
            out
        }

        /// Whether brick `slot` of chunk `c` lies within one voxel of box `lo ..= hi`.
        fn brick_touched(c: [i32; 3], slot: usize, lo: [i64; 3], hi: [i64; 3]) -> bool {
            let b = [slot % 8, slot / 8 % 8, slot / 64];
            (0..3).all(|k| {
                let l = i64::from(c[k]) * 64 + b[k] as i64 * 8;
                l + 7 >= lo[k] - 1 && l <= hi[k] + 1
            })
        }

        /// The read chain (`LightSummaries::chain`) of `chunks` and their six face neighbours.
        fn chains(store: &svet_render::Store, chunks: &[[i32; 3]]) -> BTreeMap<[i32; 3], Option<[u32; BRICK_WORDS]>> {
            let mut out = BTreeMap::new();
            for c in chunks {
                for f in 0..7 {
                    let mut n = *c;
                    if f < 6 {
                        n[f / 2] += if f % 2 == 0 { -1 } else { 1 };
                    }
                    out.entry(n).or_insert_with(|| store.light().chain(n, store.missing_light()));
                }
            }
            out
        }

        fn lum(w: u32) -> f64 {
            let c = rgb9e5_unpack(w);
            0.2126 * f64::from(c[0]) + 0.7152 * f64::from(c[1]) + 0.0722 * f64::from(c[2])
        }

        /// A record's six sky bytes read with every direction's bit set (`None`: no `VIS_BAKED`).
        fn sky_bytes(r: &[u32]) -> Option<[u8; 6]> {
            let mut f = r.to_vec();
            f[6] |= 0x3F << 16;
            record_visibility(&f, 0)?;
            Some(std::array::from_fn(|d| (record_visibility(&f, d).unwrap_or(0.0) * 255.0).round() as u8))
        }

        /// The solid voxel under a hit's face: of the two voxels across the face plane nearest
        /// the hit point, the one the world holds solid while the other is not (the axes tried
        /// nearest first); `None` at a miss or where no axis splits so.
        fn face_voxel(world: &svet_world::World, h: &[f32; 4]) -> Option<[i32; 3]> {
            if h[3] >= 1e30 || !h[3].is_finite() {
                return None;
            }
            let solid = |v: [i32; 3]| world.material_at(svet_world::VoxelCoord(v)).is_some_and(|m| m != svet_world::AIR);
            let mut axes = [0usize, 1, 2];
            axes.sort_by(|&a, &b| (h[a] - h[a].round()).abs().total_cmp(&(h[b] - h[b].round()).abs()));
            for k in axes {
                let p = h[k].round() as i32;
                let mut a = [0, 1, 2].map(|j| h[j].floor() as i32);
                let mut b = a;
                a[k] = p - 1;
                b[k] = p;
                match (solid(a), solid(b)) {
                    (true, false) => return Some(a),
                    (false, true) => return Some(b),
                    _ => {}
                }
            }
            None
        }

        /// One group of T3's dark ring pixels: the count, over 32 levels, the sky source words
        /// (before, frame), and in the before frame, the frame and the after frame the means of
        /// the shown sky visibility, the stored stage and the direct stage; the rays cast and the
        /// pair's age in the frame.
        #[derive(Default)]
        struct Group {
            n: u64,
            over32: u64,
            src: BTreeMap<(u8, u8), u64>,
            x: [f64; 3],
            stored: [f64; 3],
            direct: [f64; 3],
            cast: u64,
            age: f64,
        }

        /// T1's rows of one dig.
        fn t1(out: &mut String, n: usize, d: &DigCpu) {
            let _ = writeln!(out, "T1 dig {n}: box {:?}..{:?}", d.lo, d.hi);
            let mut whole = Vec::new();
            for c in &d.chunks {
                let (b, a) = (d.before[c].is_some(), d.after[c].is_some());
                let put = d.before[c] != d.after[c];
                if !b && a {
                    whole.push(*c);
                }
                let l = d.listed.get(c).copied().unwrap_or_default();
                let _ = writeln!(
                    out,
                    "  {c:?}: block before {} after {}{}{}; listed bricks {} faces {}; touched bricks listing faces {}",
                    u8::from(b),
                    u8::from(a),
                    if !b && a { " WHOLE" } else { "" },
                    if put { " put" } else { "" },
                    l[0],
                    l[1],
                    l[2]
                );
                if !put {
                    continue;
                }
                let block = d.pool[c].as_ref().or(d.after[c].as_ref());
                let Some(block) = block else {
                    continue;
                };
                // [held baked, of those all 0, kept stale, kept fresh, no VIS_BAKED with a mask, zero mask]
                let mut kinds = [0u64; 6];
                for (slot, r) in records(block) {
                    let mask = (r[6] >> 16) & 0x3F;
                    let sky = sky_bytes(&r);
                    if mask == 0 {
                        kinds[5] += 1;
                    } else if sky.is_none() {
                        kinds[4] += 1;
                    } else if brick_touched(*c, slot, d.lo, d.hi) {
                        kinds[0] += 1;
                        kinds[1] += u64::from(sky.is_some_and(|s| s == [0; 6]));
                    } else if record_sky_stale(&r) {
                        kinds[2] += 1;
                    } else {
                        kinds[3] += 1;
                    }
                }
                let at = SUMMARY_AT as usize;
                let sm = &block[at..at + BRICK_WORDS];
                let _ = writeln!(
                    out,
                    "    records ({}): held baked {} (all 0: {}), kept stale {}, kept fresh {}, no VIS_BAKED with a mask {}, zero mask {}; summary mask {:06b} VIS_BAKED {} SKY_STALE {} sky {:?}",
                    if d.pool[c].is_some() { "pool" } else { "store copy" },
                    kinds[0], kinds[1], kinds[2], kinds[3], kinds[4], kinds[5],
                    (sm[6] >> 16) & 0x3F,
                    u8::from(sky_bytes(sm).is_some()),
                    u8::from(record_sky_stale(sm)),
                    sky_bytes(sm)
                );
            }
            let row = |ch: &Option<[u32; BRICK_WORDS]>| -> String {
                ch.map_or("none".into(), |r| {
                    (0..6)
                        .map(|f| if (r[6] >> (16 + f)) & 1 != 0 { format!("{:.4}", lum(r[f])) } else { "-".into() })
                        .collect::<Vec<_>>()
                        .join(" ")
                })
            };
            for w in &whole {
                for f in 0..7 {
                    let mut n = *w;
                    if f < 6 {
                        n[f / 2] += if f % 2 == 0 { -1 } else { 1 };
                    }
                    let _ = writeln!(
                        out,
                        "  chain {}{n:?} by direction (-x +x -y +y -z +z): before {} | after {}",
                        if f == 6 { "WHOLE " } else { "neighbour " },
                        row(&d.chain_before[&n]),
                        row(&d.chain_after[&n])
                    );
                }
                let block = d.pool[w].as_ref().or(d.after[w].as_ref());
                let (mut s, mut m) = (0.0, 0u64);
                for (slot, r) in block.map(|b| records(b)).unwrap_or_default() {
                    if brick_touched(*w, slot, d.lo, d.hi) {
                        continue;
                    }
                    for f in 0..6 {
                        if (r[6] >> (16 + f)) & 1 != 0 {
                            s += lum(r[f]);
                            m += 1;
                        }
                    }
                }
                let _ = writeln!(out, "  whole {w:?}: guess records' mean luminance {:.4} over {m} directions", s / m.max(1) as f64);
            }
        }

        /// T2's rows of one dig.
        fn t2(out: &mut String, n: usize, d: &DigCpu) {
            let _ = writeln!(out, "T2 dig {n}: {} parts traced", d.trace.len());
            let mut by: BTreeMap<[i32; 3], Vec<&HeldPart>> = BTreeMap::new();
            for p in &d.trace {
                by.entry(p.chunk).or_default().push(p);
            }
            let byte = |v: f32| (v * 255.0).round() as i32;
            for (c, parts) in by {
                let kept: Vec<&&HeldPart> = parts.iter().filter(|p| p.old_faces > 0).collect();
                let kn = kept.len().max(1) as f64;
                let old = kept.iter().map(|p| f64::from(p.own.unwrap_or(f32::NAN))).sum::<f64>() / kn;
                let new = kept.iter().map(|p| f64::from(p.value)).sum::<f64>() / kn;
                let under = kept.iter().filter(|p| p.own.is_some_and(|o| byte(o) - byte(p.value) > 4)).count();
                let outs: Vec<&&HeldPart> = parts.iter().filter(|p| p.outside > 0).collect();
                let on = outs.len().max(1) as f64;
                let _ = writeln!(
                    out,
                    "  {c:?}: parts {}; with kept faces {} old {:.4} written {:.4} written over 4 bytes under {}; with faces outside counted as the dig's {} ({} faces) written {:.4}",
                    parts.len(),
                    kept.len(),
                    old,
                    new,
                    under,
                    outs.len(),
                    outs.iter().map(|p| u64::from(p.outside)).sum::<u64>(),
                    outs.iter().map(|p| f64::from(p.value)).sum::<f64>() / on
                );
            }
        }

        /// Plan 0105 Phase 7a3: what darkens B's ring at dig 5, by the light's own words (a
        /// reading; nothing asserted). Sequence B alone on the waiting queue, the default arm,
        /// device-wait lockstep, every frame's stages read and the held trace on. Prints T1 (the
        /// edit, digs 5 and 4), T2 (the held trace, digs 4 and 5) and T3 (the ring's dark
        /// pixels at D5+0 and D5+1, the set the bar counts, by their face's voxel's chunk kind
        /// and whether dig 5's box dilated by one voxel touches its brick); written to
        /// `live/p7a3/whole.txt`.
        #[test]
        #[ignore = "reading"]
        #[allow(clippy::too_many_lines, reason = "a reading's three tables over one run")]
        fn whole_dig_ring() {
            let mut cpu: BTreeMap<usize, DigCpu> = BTreeMap::new();
            let mut voxels: BTreeMap<usize, Vec<Option<[i32; 3]>>> = BTreeMap::new();
            let mut tap = |t: Tap<'_>, rig: &mut Rig, s: &mut Stepper| match t {
                Tap::Start => s.frame.set_bake_queue(|q| q.trace_held = true),
                Tap::Edit { dig, lo, hi } => {
                    if dig != 3 && dig != 4 {
                        return;
                    }
                    let store = s.frame.store();
                    let chunks = box_chunks(lo, hi);
                    let mut d = DigCpu { lo, hi, ..Default::default() };
                    d.trace0 = s.frame.bake_queue().expect("the live queue").held_trace.len();
                    for c in &chunks {
                        d.before.insert(*c, store.block_words(ChunkCoord(*c)).map(<[u32]>::to_vec));
                    }
                    d.chain_before = chains(store, &chunks);
                    d.chunks = chunks;
                    cpu.insert(dig, d);
                }
                Tap::Frame { dig, k, frame } => {
                    if k == 0
                        && let Some(d) = cpu.get_mut(&dig)
                    {
                        let store = s.frame.store();
                        let classes = face_classes(rig.world.materials());
                        for c in d.chunks.clone() {
                            let coord = ChunkCoord(c);
                            d.after.insert(c, store.block_words(coord).map(<[u32]>::to_vec));
                            d.pool.insert(c, s.frame.read_store_block(&rig.device, &rig.queue, coord));
                            let l = list_world_chunk(&rig.world, coord, &classes).unwrap_or_default();
                            let faces = l.bricks.iter().map(|b| b.1.len()).sum();
                            let touched = l.bricks.iter().filter(|b| !b.1.is_empty() && brick_touched(c, usize::from(b.0), d.lo, d.hi)).count();
                            d.listed.insert(c, [l.bricks.len(), faces, touched]);
                        }
                        d.chain_after = chains(store, &d.chunks);
                        d.trace = s.frame.bake_queue().expect("the live queue").held_trace[d.trace0..].to_vec();
                    }
                    if dig == 4 && k < 2 {
                        voxels.insert(k, frame.hits.iter().map(|h| face_voxel(&rig.world, h)).collect());
                    }
                }
            };
            let Some((line, _rig)) = line_run("above", true, "B", "default", 600, None, true, svet_render::DEFAULT_RING1_STRETCH, false, Some(&mut tap)) else {
                return;
            };
            let land = super::ring1_landings(&line);
            let LineRun { frames, digs, out, eye, head, .. } = line;
            let run = HoleRun { frames, n1: 1, digs, out, hold: true, eye };
            let groups = defects::hole_groups_at(&run, &land);
            let f4 = defects::hole_figures_in(&run, 4, &groups).0;
            let mut text = format!("{head}\n");
            for (k, (&(d0, lo, hi), l)) in run.digs.iter().zip(&land).enumerate() {
                let _ = writeln!(text, "dig {} at frame {d0} box {lo:?}..{hi:?} ring 1 lands D+{:?}; digs whole {}", k + 1, l.map(|l| l - d0), run.frames[d0].q.digs_whole);
            }
            for (n, k) in [(5, 4), (4, 3)] {
                if let Some(d) = cpu.get(&k) {
                    t1(&mut text, n, d);
                }
            }
            for (n, k) in [(4, 3), (5, 4)] {
                if let Some(d) = cpu.get(&k) {
                    t2(&mut text, n, d);
                }
            }
            // T3: dig 5's chunks by kind.
            let Some(d5) = cpu.get(&4) else {
                println!("{text}no dig 5");
                return;
            };
            let whole: BTreeSet<[i32; 3]> = d5.chunks.iter().filter(|c| d5.before[*c].is_none() && d5.after[*c].is_some()).copied().collect();
            let held: BTreeSet<[i32; 3]> = d5.chunks.iter().filter(|c| d5.before[*c].is_some()).copied().collect();
            const KINDS: [&str; 4] = ["whole chunk", "dug, held a block", "other", "no face voxel"];
            let (dw, dh) = (run.out[0] as usize, run.out[1] as usize);
            for k in 0..2 {
                let fi = run.digs[4].0 + k;
                let Some(vox) = voxels.get(&k) else {
                    continue;
                };
                let (bi, ai) = defects::before_after(&run, &groups, fi);
                let (f, fb, fa) = (&run.frames[fi], &run.frames[bi], &run.frames[ai]);
                let (c, cb, ca) = (defects::hole_classes(&run, f).1, defects::hole_classes(&run, fb).1, defects::hole_classes(&run, fa).1);
                let gl = f.hits.len();
                let gw = ((gl as f64 * dw as f64 / dh as f64).sqrt().round() as usize).max(1);
                let mut by: BTreeMap<(usize, bool), Group> = BTreeMap::new();
                let (mut nr, mut dark) = (0u64, 0u64);
                for i in 0..f.luma.len() {
                    let ring = defects::DIG_RING_CLASS;
                    if !(c[i] == ring && cb[i] == ring && ca[i] == ring) {
                        continue;
                    }
                    nr += 1;
                    let (l, b, a) = (i32::from(f.luma[i]), i32::from(fb.luma[i]), i32::from(fa.luma[i]));
                    if l >= b.min(a) - 4 {
                        continue;
                    }
                    dark += 1;
                    let gi = defects::grid_of(i, dw, dh, gw, gl);
                    let (kind, touched) = match vox[gi] {
                        None => (3, false),
                        Some(v) => {
                            let ch = v.map(|x| x.div_euclid(64));
                            let kind = if whole.contains(&ch) {
                                0
                            } else if held.contains(&ch) {
                                1
                            } else {
                                2
                            };
                            let lv = [0, 1, 2].map(|j| v[j].rem_euclid(64) as usize / 8);
                            (kind, brick_touched(ch, lv[0] + 8 * lv[1] + 64 * lv[2], d5.lo, d5.hi))
                        }
                    };
                    let g = by.entry((kind, touched)).or_default();
                    g.n += 1;
                    g.over32 += u64::from(l < b.min(a) - 32);
                    *g.src.entry((fb.source[gi], f.source[gi])).or_default() += 1;
                    for (j, fr) in [fb, f, fa].into_iter().enumerate() {
                        g.x[j] += f64::from(fr.pair[gi][0]);
                        g.stored[j] += fr.stored.get(gi).copied().map_or(f64::NAN, f64::from);
                        g.direct[j] += fr.direct.get(gi).copied().map_or(f64::NAN, f64::from);
                    }
                    g.cast += u64::from(f.cast[gi]);
                    g.age += f64::from(f.pair[gi][3]);
                }
                let _ = writeln!(
                    text,
                    "T3 {} (before {}, after {}): ring {nr} px, dark over 4 {dark} ({:.2} %), ring bar {:.2} % black; exposure before {:.6} frame {:.6} after {:.6} (frame / before {:+.2} %)",
                    f.label,
                    fb.label,
                    fa.label,
                    100.0 * dark as f64 / nr.max(1) as f64,
                    f4[fi].black_ring,
                    fb.exposure,
                    f.exposure,
                    fa.exposure,
                    100.0 * (f.exposure / fb.exposure.max(1e-12) - 1.0)
                );
                let mut big = 0u64;
                for ((kind, touched), g) in &by {
                    let n = g.n.max(1) as f64;
                    if g.n >= 1000 {
                        big += g.n;
                    }
                    let src: Vec<String> = g
                        .src
                        .iter()
                        .map(|(&(a, b), &m)| format!("{} -> {} {m}", defects::SOURCE_NAMES[usize::from(a.min(6))], defects::SOURCE_NAMES[usize::from(b.min(6))]))
                        .collect();
                    let tri = |v: &[f64; 3]| format!("{:.4} / {:.4} / {:.4}", v[0] / n, v[1] / n, v[2] / n);
                    let _ = writeln!(
                        text,
                        "  {} / {}: {} ({:.0} %), over 32 {}; sources (before -> frame) {}; sky x (before / frame / after) {}; stored {}; direct {}; cast {:.1} %; age {:.2}",
                        KINDS[*kind],
                        if *touched { "brick dig 5 touches" } else { "brick dig 5 does not touch" },
                        g.n,
                        100.0 * g.n as f64 / dark.max(1) as f64,
                        g.over32,
                        src.join(", "),
                        tri(&g.x),
                        tri(&g.stored),
                        tri(&g.direct),
                        100.0 * g.cast as f64 / n,
                        g.age / n
                    );
                }
                let _ = writeln!(text, "  groups of 1000 or more: {big} of {dark} ({:.0} %)", 100.0 * big as f64 / dark.max(1) as f64);
            }
            println!("{text}");
            let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../live/p7a3");
            std::fs::create_dir_all(&dir).expect("the reading's directory");
            std::fs::write(dir.join("whole.txt"), text).expect("the tables");
        }
        }
    }
}
