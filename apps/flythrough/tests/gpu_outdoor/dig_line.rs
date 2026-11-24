// `gpu_outdoor`'s dig timeline (Plan 0098 Phase 16 step 1), included by `gpu_outdoor.rs`
// (kept apart for the file-size cap).

mod dig_line {
    pub(crate) mod reading {
        use super::super::flicker::reading as fl;
        use super::super::support;
        use std::collections::{BTreeMap, BTreeSet};
        use std::fmt::Write as _;
        use support::defects::{HoleFrame, Stepper, dig_along, hole_frame, queue_step};
        use support::terms::Rig;
        use vxe_render::Camera;

        /// The eye of the owner's dig on the RADV RENOIR at Portable (window 98d's run 3,
        /// `3-radeon-dig.png`'s HUD), voxels.
        const RENOIR_EYE: [f32; 3] = [31.9, 603.0, 604.3];

        /// Frames between the digs of sequence B (the owner's several in a row).
        const B_GAP: usize = 8;

        /// Where sequence B's five digs aim, voxels on the ground about the view's centre's hit;
        /// the sixth digs into the first.
        const B_AIMS: [[f32; 2]; 5] = [[0.0, 0.0], [9.0, 0.0], [-9.0, 2.0], [2.0, 9.0], [-2.0, -9.0]];

        /// The run of [`dig_timeline`]: the frames, per dig its frame index and box, and the
        /// queue's trace after each frame's step.
        pub(in super::super) struct LineRun {
            pub frames: Vec<HoleFrame>,
            pub traces: Vec<vxe_render::DigTrace>,
            pub digs: Vec<(usize, [i64; 3], [i64; 3])>,
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
        /// first. After the last, frames until no dig is left plus 16 and [`fl::DIG_TAIL`].
        pub(in super::super) fn line_run(
            scene: &str,
            waiting: bool,
            seq: &str,
            arm: &str,
            drain: u32,
            png: Option<&std::path::Path>,
        ) -> Option<(LineRun, Rig)> {
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
            f.set_bake_budget(vxe_render::Budget::default());
            f.set_bake_coarse(vxe_render::DEFAULT_COARSE_SAMPLES, vxe_render::DEFAULT_NEAR);
            f.set_bake_tiers(vxe_render::DEFAULT_TIERS);
            f.set_bake_drain(Some(vxe_render::DEFAULT_DRAIN));
            f.set_bake_digs(vxe_render::DEFAULT_BAKE_EDIT_MS, true);
            if portable {
                let p = vxe_render::RenderSettings::profile(vxe_render::Profile::Portable);
                f.set_bake_budget(p.bake.budget);
                f.set_bake_tiers(p.bake.tiers);
                f.set_bake_drain(p.bake.drain);
                f.set_lighting(vxe_render::Lighting {
                    lamp_bounce: p.lighting.lamp_bounce,
                    ..f.lighting()
                });
            }
            let out = sizes[1];
            fl::dig_arm(&mut f, arm);
            let mut s = Stepper::new(&rig, f);
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
                        "line {scene} {} {seq} {arm}: adapter {}, after {n} frames in view waiting {}, coarse {}, depth {}, jobs {}",
                        if waiting { "waiting" } else { "drained" },
                        rig.adapter,
                        q.drain_waiting,
                        q.coarse,
                        q.depth,
                        q.jobs
                    ) + &format!(
                        "; baked {} coarse opened {} keys baked {} withdrawn {} retiered {} reach {} partial {} provisional {} waiting reach {} lights {} unbaked {}; samples read {} total {} in {:.3} ms, ns a sample {:.3}, slice {:.2} ms, draining {}",
                        q.baked, q.coarse_opened, q.keys_baked, q.withdrawn, q.retiered, q.reach, q.partial,
                        q.provisional, q.waiting_reach, q.waiting_lights, q.unbaked, q.samples, q.total_samples, q.ms, q.ns_per_sample, q.slice_ms, q.draining
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
            let (mut frames, mut traces, mut digs) = (Vec::new(), Vec::new(), Vec::new());
            let read = |rig: &mut Rig, s: &mut Stepper, frames: &mut Vec<HoleFrame>, traces: &mut Vec<vxe_render::DigTrace>, label: String| {
                let q = queue_step(rig, &mut s.frame, &cam);
                traces.push(s.frame.bake_queue().expect("the live queue").dig_trace());
                let path = png.map(|d| d.join(format!("{:03}-{label}.png", frames.len())));
                frames.push(hole_frame(rig, s, &cam, q, label, out, false, path.as_deref()));
                q
            };
            for k in [2, 1] {
                read(&mut rig, &mut s, &mut frames, &mut traces, format!("N-{k}"));
            }
            let aims: Vec<(Camera, usize)> = match seq {
                "A" => vec![(cam, usize::MAX), (cam, 1), (cam, usize::MAX)],
                _ => B_AIMS
                    .iter()
                    .chain(std::iter::once(&B_AIMS[0]))
                    .enumerate()
                    .map(|(i, a)| {
                        let c = aimed(&cam, [centre[0] + a[0], centre[1] + a[1], centre[2]]);
                        (c, if i < B_AIMS.len() { B_GAP } else { usize::MAX })
                    })
                    .collect(),
            };
            for (d, (aim, gap)) in aims.iter().enumerate() {
                let Some(dug) = dig_along(&mut rig, &mut s.frame, aim) else {
                    println!("{head}: dig {} changed nothing", d + 1);
                    continue;
                };
                digs.push((frames.len(), dug.0, dug.1));
                let mut tail = None;
                for k in 0..1200usize {
                    let q = read(&mut rig, &mut s, &mut frames, &mut traces, format!("D{}+{k}", d + 1));
                    if k + 1 == *gap {
                        break;
                    }
                    if q.digs == 0 && k > 0 && tail.is_none() {
                        tail = Some(k + 16 + fl::DIG_TAIL);
                    }
                    if tail == Some(k) {
                        break;
                    }
                }
            }
            Some((LineRun { frames, traces, digs, out, eye, head }, rig))
        }

        /// Whether the brick whose lowest voxel is `b` lies within `pad` voxels of the box.
        fn near_box(b: [i32; 3], lo: [i64; 3], hi: [i64; 3], pad: i64) -> bool {
            (0..3).all(|k| i64::from(b[k]) + 7 >= lo[k] - pad && i64::from(b[k]) <= hi[k] + pad)
        }

        /// The timeline's text: a row a frame (the queue's cap and the submit's samples, the
        /// samples in flight on ring 1's bricks, ring 2's and other jobs', the free slots,
        /// pending, open and unread, the step's and the passes' ms) and under it a row per
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
                    "{fi:4} {:8} cap {:7} sent {:7} | flight r1 {:6} r2 {:6} other {:7} | free {:2} pend {:3} open {:2} unread {} withdrawn {} pool+{pool} | step {:6.3} ms light {:6.3} temporal {:6.3} gpu {:6.2} | waiting {} depth {} digs {}",
                    f.label, f.q.cap, f.q.submitted, fl[0], fl[1], fl[2], t.free_slots, t.pending, t.open,
                    t.in_flight, t.withdrawn, f.q.stages.total_ms, f.light_ms, f.temporal_ms, f.gpu_ms,
                    f.q.drain_waiting, f.q.depth, f.q.digs
                );
                let jobs: BTreeMap<[i32; 3], &vxe_render::DigTraceJob> = t.jobs.iter().map(|j| (j.coord, j)).collect();
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
                // The dig frame's step by stage, the stages over 0.5 ms.
                let st = &run.frames[d0].q.stages;
                let laps: Vec<String> = vxe_render::debug::STEP_STAGES
                    .iter()
                    .zip(st.ms)
                    .filter(|(_, ms)| *ms > 0.5)
                    .map(|(n, ms)| format!("{n} {ms:.2}"))
                    .collect();
                let _ = writeln!(out, "  D+0 stages over 0.5 ms (opened {}): {}", st.opened, laps.join(", "));
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

        /// Plan 0098 Phase 16 step 1's timeline: `VXE_LINE` is `SCENE/STATE/SEQ/ARM/DRAIN`
        /// (`above` or `portable`, `drained` or `waiting`, `A` or `B`, `support::defects`'
        /// dig arm tokens, the most frames the queue drains); the text under
        /// `live/p16/<adapter>-<scene>-<state>-<seq>-<arm>.txt`, the frames' PNGs beside it with
        /// `VXE_LINE_PNG` set.
        #[test]
        #[ignore = "reading"]
        fn dig_timeline() {
            let v = std::env::var("VXE_LINE").unwrap_or_default();
            let mut it = v.split('/');
            let mut part = |d: &str| it.next().filter(|s| !s.is_empty()).unwrap_or(d).to_string();
            let (scene, state, seq, arm) = (part("above"), part("drained"), part("A"), part("default"));
            let drain: u32 = part("30000").parse().expect("VXE_LINE's drain");
            let adapter = std::env::var("VXE_TEST_ADAPTER").unwrap_or_else(|_| "lavapipe".into());
            let name = format!("{adapter}-{scene}-{state}-{seq}-{}", arm.replace([',', '='], "-"));
            let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../live/p16");
            let png = std::env::var("VXE_LINE_PNG").is_ok().then(|| dir.join(&name));
            if let Some(p) = &png {
                std::fs::create_dir_all(p).expect("the stills' directory");
            }
            std::fs::create_dir_all(&dir).expect("the reading's directory");
            let Some((run, _rig)) = line_run(&scene, state == "waiting", &seq, &arm, drain, png.as_deref()) else {
                return;
            };
            let text = line_text(&run);
            let summary: String = text
                .lines()
                .filter(|l| l.starts_with("line ") || l.starts_with("dig ") || l.starts_with("  cost") || l.starts_with("before"))
                .collect::<Vec<_>>()
                .join("\n");
            println!("{summary}\neye {:?} out {:?}", run.eye, run.out);
            std::fs::write(dir.join(format!("{name}.txt")), text).expect("the timeline");
        }

        /// A block's brick records by slot ([`vxe_render::internals::BRICK_WORDS`] words each,
        /// the directory's run end in each slot's word).
        fn records(block: &[u32]) -> BTreeMap<usize, Vec<u32>> {
            use vxe_render::internals::{BRICK_WORDS, SUMMARY_AT};
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
            use vxe_render::internals::{record_key_visibility, record_visibility, rgb9e5_unpack};
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
        fn ring_blocks(f: &vxe_render::Frame, lo: [i64; 3], hi: [i64; 3], pad: i64) -> BTreeMap<[i32; 3], Vec<u32>> {
            let mut out = BTreeMap::new();
            let a = lo.map(|v| (v - pad).div_euclid(64) as i32);
            let b = hi.map(|v| (v + pad).div_euclid(64) as i32);
            for z in a[2]..=b[2] {
                for y in a[1]..=b[1] {
                    for x in a[0]..=b[0] {
                        if let Some(w) = f.store().block_words(vxe_world::ChunkCoord([x, y, z])) {
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
                let head: Vec<usize> = (vxe_render::internals::SUMMARY_AT as usize..=vxe_render::internals::LANDED_AT as usize)
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
                    if s.frame.store().is_coarse(vxe_world::ChunkCoord(c)) && !out.contains_key(&c) {
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
            f.set_bake_budget(vxe_render::Budget::default());
            f.set_bake_coarse(vxe_render::DEFAULT_COARSE_SAMPLES, vxe_render::DEFAULT_NEAR);
            f.set_bake_tiers(vxe_render::DEFAULT_TIERS);
            f.set_bake_drain(Some(vxe_render::DEFAULT_DRAIN));
            f.set_bake_digs(vxe_render::DEFAULT_BAKE_EDIT_MS, true);
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
        const DIG_FADE: usize = vxe_render::internals::DIG_FADE as usize;

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

        /// The run of `seq` (`A` or `B`) under `arm` at the image test's start (`waiting`
        /// false: one frame and the settle) or after 600 frames (`waiting`), on the adapter
        /// the test support opens, with its claims' figures.
        pub(crate) fn claims(seq: &str, waiting: bool, arm: &str) -> Option<Claims> {
            let drain = if waiting { 600 } else { 1 };
            let (line, _rig) = line_run("above", waiting, seq, arm, drain, None)?;
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
        use super::claims;

        /// Plan 0098 Phase 17 step 1's histogram: [`claims`] of A and B on the waiting queue
        /// under `VXE_DIG`'s arm (`support::defects::dig_spec`), printed, nothing asserted.
        #[test]
        #[ignore = "reading"]
        fn hold_histogram() {
            let (_, arm, _) = super::super::super::support::defects::dig_spec(("above", "default", 0));
            for seq in ["A", "B"] {
                let Some(c) = claims(seq, true, &arm) else {
                    return;
                };
                println!(
                    "{}hole {seq} {arm}: ring 1 lands D+{:?}; before the landing white {:.2} % black {:.2} % (32 levels), after white {:.2} % black {:.2} %, ring white {:.2} % black {:.2} %, dots {:.2} % (settled {:.2} %), shift {:.2}, away {:+.2}",
                    c.text, c.land, c.before[0], c.before[1], c.after[0], c.after[1], c.ring[0], c.ring[1], c.dots[0], c.dots[1], c.shift, c.away
                );
            }
        }
        }
    }
}
