// `gpu_outdoor`'s flicker strips (Plan 0086, the owner's ask of 2026-11-18), included by
// `gpu_outdoor.rs` (kept apart for the file-size cap).

mod flicker {
    /// Plan 0086 Phase 24a bar 2 (ADR-0076 rule 4's form): a dig changes nothing outside its
    /// reach. `reading::dig_split`'s rig (the hills of seed 1 at noon, the spawn's view 30 degrees
    /// down, the fly-through's defaults, FSR 2 from 640 x 360 to 960 x 540, the live queue drained;
    /// `VXE_SPLIT_TIME` another hour), the 1 m dig about 35 voxels ahead: over every frame pair
    /// from the one ending at the dig's frame N to N + 16, the share of display pixels outside
    /// the box dilated by 64 voxels whose largest channel changes by over 4 levels is at most
    /// 0.5 % (the settled strip's largest 0.191 %; the tip before the fix printed in the log);
    /// at N the lighting pixels outside the dilated box that cast the sky ray are no more than
    /// at N - 1 and those of age 0 no more than the most of the 8 still frames before, the
    /// history's two bits set, neither change bit raised and the restarts unchanged. The share
    /// inside the dilated box is printed (Phase 21's bars hold there).
    #[test]
    #[ignore = "soak"]
    fn a_dig_changes_nothing_outside_its_reach() {
        let Some(rows) = reading::dig_split() else {
            return;
        };
        let b = reading::SPLIT_BEFORE;
        let (still, after) = rows.split_at(b);
        let n = &after[0];
        let worst = after
            .iter()
            .map(|r| r.share_out)
            .fold(0.0f64, f64::max);
        println!(
            "dig: outside the reach at most {worst:.3} % over 4; inside at N {:.3} %",
            n.share_in
        );
        assert!(worst <= 0.5, "outside the reach: {worst:.3} % over 4 levels");
        assert!(
            n.cast_out <= still[b - 1].cast_out,
            "sky rays cast outside the reach: {} at N, {} at N - 1",
            n.cast_out,
            still[b - 1].cast_out
        );
        let age0 = still.iter().map(|r| r.age0_out).max().unwrap_or(0);
        assert!(n.age0_out <= age0, "age 0 outside: {} at N, at most {age0} still", n.age0_out);
        assert_eq!(n.history, 0b11, "the history bits at N");
        assert!(!n.flags_changed, "no change bit at N");
        assert_eq!(n.restarts, still[b - 1].restarts, "no restart at N");
    }

    mod reading {
        use super::super::support;
        use support::defects::{Shot, Stepper, dig_ahead, grid_of, queue_step};
        use support::terms::Rig;
        use vxe_render::Camera;

        /// The strips' internal size; FSR 2 at 1.5 takes it to [`OUT`].
        const INTERNAL: [u32; 2] = [640, 360];
        const OUT: [u32; 2] = [960, 540];
        /// Frames a strip holds.
        const STRIP: usize = 32;
        /// The display levels a pixel's largest channel must change by between consecutive
        /// frames to count.
        const OVER: [u8; 2] = [4, 8];

        /// Plan 0086's flicker strip on the hills of seed 1 at noon: the spawn's view, the live
        /// queue drained, 240 settle frames, then [`STRIP`] frames. Each strip writes its
        /// frames, `diff.png` (the largest per-pixel change over the strip, times 8) and its
        /// counts under `live/flicker/<scene>/current/<strip>/`; the runner renames `current`.
        /// `VXE_TEST_ADAPTER` picks the adapter.
        #[test]
        #[ignore = "reading"]
        fn flicker_hills() {
            let Some((mut rig, cam)) = support::terms::hills_rig() else {
                return;
            };
            let mut s = strip_frame(&mut rig, noon(), &cam);
            settle(&mut rig, &mut s, &cam, 240);
            strip(&mut rig, &mut s, &cam, "hills", "settled");
        }

        /// The forest of seed 1 under the sun at `60,180` (Phase 24 run 5), drained and settled,
        /// a 90-degree yaw over 10 frames and back over 10, then a strip (`fresh`), 300 frames
        /// and another (`settled`).
        #[test]
        #[ignore = "reading"]
        fn flicker_forest() {
            let Some((mut rig, cam)) = support::terms::forest_rig() else {
                return;
            };
            let sky = flythrough::daylight::Clock {
                fixed_sun: Some([60.0, 180.0]),
                ..Default::default()
            }
            .sky(flythrough::outdoor::BASE_Z as f32);
            let mut s = strip_frame(&mut rig, sky, &cam);
            settle(&mut rig, &mut s, &cam, 240);
            for k in (1..=10).chain((0..10).rev()) {
                let c = yawed(&cam, 9.0 * k as f32);
                queue_step(&mut rig, &mut s.frame, &c);
                s.run(&rig, &c);
            }
            strip(&mut rig, &mut s, &cam, "forest", "fresh");
            settle(&mut rig, &mut s, &cam, 300);
            strip(&mut rig, &mut s, &cam, "forest", "settled");
        }

        /// A 1 m dig on the hills at noon (Phase 16's dig: the spawn's view 30 degrees down,
        /// the box about 35 voxels ahead), drained and settled first: the strip from the frame
        /// after the dig (`fresh`), then 600 frames and another (`settled`).
        #[test]
        #[ignore = "reading"]
        fn flicker_dig() {
            let Some((mut rig, cam)) = support::terms::hills_rig() else {
                return;
            };
            let cam = pitched(&cam, -30.0);
            let mut s = strip_frame(&mut rig, noon(), &cam);
            settle(&mut rig, &mut s, &cam, 240);
            dig_ahead(&mut rig, &mut s.frame, &cam);
            strip(&mut rig, &mut s, &cam, "dig", "fresh");
            settle(&mut rig, &mut s, &cam, 600);
            strip(&mut rig, &mut s, &cam, "dig", "settled");
        }

        /// Phase 24a's split of the dig's view-wide flash: [`flicker_dig`]'s rig, the frame
        /// before the dig (N - 1) read, then the dig's frame N and [`SPLIT_AFTER`] more. Per
        /// frame the frame-wide counters (the history bits and the lighting flags the encode
        /// wrote, the restarts, the sky and lights change bits, the store's chunks and window,
        /// the queue's digs) and the lighting pixels' counts inside and outside the dig's box
        /// dilated by 64 voxels: the sky rays cast, the history's age 0 and the chained count;
        /// per frame pair the share of display pixels over 4 levels inside and outside. The
        /// frames N - 1 to N + 2 land under `live/flicker/dig/current/split/`.
        #[test]
        #[ignore = "reading"]
        fn flicker_dig_split() {
            dig_split();
        }

        /// [`flicker_dig_split`]'s rig and split: its rows, `None` without an adapter.
        pub(in super::super) fn dig_split() -> Option<Vec<SplitRow>> {
            let (mut rig, cam) = support::terms::hills_rig()?;
            let cam = pitched(&cam, -30.0);
            let mut s = strip_frame(&mut rig, split_sky(), &cam);
            // Attribution switches: `VXE_SPLIT_FILTER=0` the a-trous off, `VXE_SPLIT_FACE=1` the
            // sky term's normal the face's, `VXE_SPLIT_BOUNCE=0` no bounce floor.
            let mut l = s.frame.lighting();
            if std::env::var("VXE_SPLIT_FILTER").as_deref() == Ok("0") {
                l.filter = false;
            }
            if std::env::var("VXE_SPLIT_FACE").as_deref() == Ok("1") {
                l.sky_normal = vxe_render::SkyNormal::Face;
            }
            if std::env::var("VXE_SPLIT_BOUNCE").as_deref() == Ok("0") {
                l.sky_bounce = 0.0;
            }
            s.frame.set_lighting(l);
            // Drained before the dig, so the split reads one state on any binary.
            let mut n = 0;
            while n < 30_000 && !s.frame.bake_queue().is_some_and(|q| q.empty()) {
                queue_step(&mut rig, &mut s.frame, &cam);
                s.run(&rig, &cam);
                n += 1;
            }
            println!(
                "split: drained {} after {n} more frames",
                s.frame.bake_queue().is_some_and(|q| q.empty())
            );
            settle(&mut rig, &mut s, &cam, 240);
            Some(split_frames(&mut rig, &mut s, &cam, "dig", &mut queue_step))
        }

        /// [`flicker_dig_split`] on a streamed world as the window streams it: the hills of
        /// seed 1 about the spawn at the fly-through's residency (`RESIDENCY`, full 16, LOD 24)
        /// and mirror (the default budget, `stream_record_chunks`, a following window), the
        /// streamer polled and the mirror synced every frame, the live queue stepped with the
        /// streamer's residency; [`STREAM_SETTLE`] frames (`VXE_SPLIT_SETTLE`) before the split.
        #[test]
        #[ignore = "reading"]
        fn flicker_dig_streamed() {
            use std::cell::RefCell;
            use vxe_render::{GpuWorld, MirrorConfig, QueueView, Window};
            use vxe_world::{ChunkCoord, PersistConfig, StreamConfig, Streamer, Tier, World};
            let residency = flythrough::RESIDENCY;
            let budget = vxe_render::DEFAULT_BUDGET_BYTES;
            let Some((device, queue, adapter)) = support::reading_device(budget) else {
                return;
            };
            let source = flythrough::outdoor::OutdoorSource::new(1);
            let (eye, facing) = source.spawn(flythrough::outdoor::Spawn::Outside);
            let world = World::new(&flythrough::outdoor_materials());
            let streamer = RefCell::new(Streamer::new(
                std::sync::Arc::new(source),
                StreamConfig {
                    residency,
                    workers: 8,
                    persist: PersistConfig::default(),
                },
            ));
            let side = 2 * (residency.r_lod + residency.hysteresis) + 1;
            let mut gpu = GpuWorld::with_config(
                &device,
                MirrorConfig {
                    budget_bytes: budget,
                    record_chunks: Some(flythrough::stream_record_chunks(residency)),
                    ..MirrorConfig::new(Window::Follow { side })
                },
            );
            {
                use support::SyncNow;
                let mut st = streamer.borrow_mut();
                st.set_radii(1, 1);
                st.update(&world, &[eye]);
                gpu.apply_events(&st.wait_idle(&world));
                gpu.set_focus(eye);
                gpu.sync_now(&world, &device, &queue).expect("the mirror fits");
                st.set_radii(residency.r_full, residency.r_lod);
            }
            let mut rig = Rig {
                device,
                queue,
                gpu,
                adapter,
                world,
            };
            let p = eye.0.map(|v| v as f32 + 0.5);
            let d = facing.map(|v| v as f32);
            let cam = flythrough::camera_looking_at(p, [p[0] + d[0], p[1] + d[1], p[2] + d[2]]);
            let cam = pitched(&cam, -30.0);
            let f = flicker_frame(&rig, split_sky(), None, false);
            let mut s = Stepper::new(&rig, f);
            let mut tick = |rig: &mut Rig, f: &mut vxe_render::Frame, cam: &Camera| {
                let mut st = streamer.borrow_mut();
                st.update(&rig.world, &[eye]);
                rig.gpu.apply_events(&st.poll(&rig.world));
                rig.gpu.set_focus(eye);
                let mut enc = rig.device.create_command_encoder(&Default::default());
                rig.gpu
                    .sync(&rig.world, &rig.queue, &mut enc)
                    .expect("the mirror fits");
                rig.queue.submit([enc.finish()]);
                let resident_fn = |c: ChunkCoord| st.tier(c) == Some(Tier::Full);
                let view = QueueView {
                    eye: cam.position.to_array(),
                    forward: cam.basis().0.to_array(),
                    resident: Some(&resident_fn),
                    samples: None,
                    interval_s: 1.0 / 60.0,
                    hold: Some(residency.r_full + rig.gpu.light_radius() + 1),
                    half_angle: {
                        let [w, h] = f.output_size();
                        vxe_render::internals::view_half_angle(cam.vfov, w as f32 / h as f32)
                    },
                    frustum: None,
                    full_radius: Some(residency.r_full),
                    lod_radius: Some(residency.r_lod.max(residency.r_full)),
                };
                f.bake_queue_step(&rig.device, &rig.queue, &mut rig.gpu, &rig.world, &view)
                    .expect("the queue runs")
                    .expect("the queue's step")
            };
            let settle_n: u32 = std::env::var("VXE_SPLIT_SETTLE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(STREAM_SETTLE);
            for n in 0..settle_n {
                let q = tick(&mut rig, &mut s.frame, &cam);
                s.run(&rig, &cam);
                if n % 300 == 0 {
                    println!(
                        "streamed settle {n}: mirrored {}, pending {}, queue depth {}",
                        rig.gpu.mirrored_chunks(),
                        rig.gpu.pending_chunks(),
                        q.depth
                    );
                }
            }
            split_frames(&mut rig, &mut s, &cam, "dig-streamed", &mut tick);
        }

        /// The split's sky: noon, or the clock at `VXE_SPLIT_TIME` (`HH:MM`).
        fn split_sky() -> vxe_render::sky::Sky {
            let minutes = std::env::var("VXE_SPLIT_TIME").ok().and_then(|t| {
                let (h, m) = t.split_once(':')?;
                Some(h.parse::<f64>().ok()? * 60.0 + m.parse::<f64>().ok()?)
            });
            match minutes {
                Some(minutes) => flythrough::daylight::Clock {
                    minutes,
                    ..Default::default()
                }
                .sky(flythrough::outdoor::BASE_Z as f32),
                None => noon(),
            }
        }

        /// The frames [`flicker_dig_streamed`] runs before its split.
        const STREAM_SETTLE: u32 = 1800;

        /// Frames [`flicker_dig_split`] reads after the dig's.
        const SPLIT_AFTER: usize = 16;
        /// Frames it reads before the dig, the last N - 1.
        pub(in super::super) const SPLIT_BEFORE: usize = 8;

        /// One frame of the split: its read-backs and counters.
        struct SplitFrame {
            shot: Shot,
            history: u32,
            lighting: u32,
            restarts: u64,
            sky_changed: bool,
            lights_changed: bool,
            store_chunks: u64,
            store_coarse: u64,
            window: Option<vxe_world::ChunkExtent>,
            q: vxe_render::QueueStats,
            /// Per lighting pixel: the stored part's, the direct part's and the filter's output
            /// luminance, the pair's sky mean and the stored flag (the moments' w).
            parts: Vec<[f32; 5]>,
        }

        /// A frame's queue step (and, on a streamed rig, the stream's poll and the mirror's
        /// sync before it).
        type Tick<'a> = dyn FnMut(&mut Rig, &mut vxe_render::Frame, &Camera) -> vxe_render::QueueStats + 'a;

        fn split_frame(rig: &mut Rig, s: &mut Stepper, cam: &Camera, tick: &mut Tick<'_>) -> SplitFrame {
            let q = tick(rig, &mut s.frame, cam);
            let shot = s.step(rig, cam);
            let f = &s.frame;
            let (history, lighting) = f.last_encoded_flags();
            let st = f.store().stats(0, 0, 0);
            let parts = {
                use vxe_render::debug::read_rgba16f;
                let (d, q) = (&rig.device, &rig.queue);
                let lum = |p: &[f32; 4]| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
                let acc = read_rgba16f(d, q, f.accumulated());
                let dir = read_rgba16f(d, q, f.direct_accumulated());
                let fil = read_rgba16f(d, q, f.filtered());
                let pair = read_rgba16f(d, q, f.visibility_pair());
                let mom = read_rgba16f(d, q, f.moments());
                (0..acc.len())
                    .map(|i| [lum(&acc[i]), lum(&dir[i]), lum(&fil[i]), pair[i][0], mom[i][3]])
                    .collect()
            };
            SplitFrame {
                parts,
                shot,
                history,
                lighting,
                restarts: f.history_restarts(),
                sky_changed: f.sky_changed(),
                lights_changed: f.lights_changed(),
                store_chunks: st.chunks,
                store_coarse: st.coarse_chunks,
                window: f.store().piece(),
                q,
            }
        }

        /// The split's frames on `rig` from a settled `s`: N - 1, the dig, N .. N + SPLIT_AFTER;
        /// the table printed and written, its rows returned.
        #[allow(
            clippy::too_many_lines,
            reason = "a reading's table: the counters, the shares and the parts per frame"
        )]
        fn split_frames(
            rig: &mut Rig,
            s: &mut Stepper,
            cam: &Camera,
            scene: &str,
            tick: &mut Tick<'_>,
        ) -> Vec<SplitRow> {
            let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../live/flicker")
                .join(scene)
                .join("current/split");
            std::fs::create_dir_all(&dir).expect("the split's directory");
            let mut frames: Vec<SplitFrame> = (0..SPLIT_BEFORE).map(|_| split_frame(rig, s, cam, tick)).collect();
            let (min, max) = dig_ahead(rig, &mut s.frame, cam);
            let (lo, hi) = (min.map(|v| v - 64), max.map(|v| v + 64));
            for _ in 0..=SPLIT_AFTER {
                frames.push(split_frame(rig, s, cam, tick));
            }
            let inside = |h: &[f32; 4]| {
                h[3] < 1e30
                    && (0..3).all(|k| {
                        let v = h[k].floor() as i64;
                        (lo[k]..=hi[k]).contains(&v)
                    })
            };
            let n = (OUT[0] * OUT[1]) as usize;
            let gw_of = |sh: &Shot| {
                ((sh.hits.len() as f64 * f64::from(INTERNAL[0]) / f64::from(INTERNAL[1]))
                    .sqrt()
                    .round() as usize)
                    .max(1)
            };
            let mut out = String::new();
            let mut worst = 0.0f64;
            let mut rows = Vec::with_capacity(frames.len());
            for (k, fr) in frames.iter().enumerate() {
                let label = if k < SPLIT_BEFORE {
                    format!("N-{}", SPLIT_BEFORE - k)
                } else {
                    format!("N+{}", k - SPLIT_BEFORE)
                };
                let sh = &fr.shot;
                let (mut cast_in, mut cast_out, mut a0_in, mut a0_out, mut g_in) =
                    (0u32, 0u32, 0u32, 0u32, 0u32);
                for (g, h) in sh.hits.iter().enumerate() {
                    let i = inside(h);
                    g_in += u32::from(i);
                    let c = u32::from(sh.cast[g]);
                    let a = u32::from(h[3] < 1e30 && sh.age[g] < 0.5);
                    if i {
                        cast_in += c;
                        a0_in += a;
                    } else {
                        cast_out += c;
                        a0_out += a;
                    }
                }
                let mut pair = String::new();
                let (mut share_in, mut share_out) = (0.0, 0.0);
                if k > 0 {
                    let a = &frames[k - 1].shot;
                    let gw = gw_of(sh);
                    let (mut over_in, mut over_out, mut px_in) = (0u64, 0u64, 0u64);
                    for i in 0..n {
                        let g = grid_of(i, OUT[0] as usize, OUT[1] as usize, gw, sh.hits.len());
                        let is_in = inside(&sh.hits[g]) || inside(&a.hits[g]);
                        px_in += u64::from(is_in);
                        let d = (0..3)
                            .map(|j| a.display[i * 4 + j].abs_diff(sh.display[i * 4 + j]))
                            .max()
                            .unwrap_or(0);
                        if d > OVER[0] {
                            if is_in {
                                over_in += 1;
                            } else {
                                over_out += 1;
                            }
                        }
                    }
                    let px_out = n as u64 - px_in;
                    share_out = 100.0 * over_out as f64 / px_out.max(1) as f64;
                    share_in = 100.0 * over_in as f64 / px_in.max(1) as f64;
                    if k >= SPLIT_BEFORE {
                        worst = worst.max(share_out);
                    }
                    pair = format!(
                        "; over 4 inside {share_in:.3} % of {px_in}, outside {share_out:.3} % of {px_out}"
                    );
                }
                rows.push(SplitRow {
                    share_in,
                    share_out,
                    cast_out,
                    age0_out: a0_out,
                    history: fr.history,
                    flags_changed: fr.sky_changed || fr.lights_changed,
                    restarts: fr.restarts,
                });
                let ms = |name: &str| {
                    vxe_render::FRAME_PASSES
                        .iter()
                        .position(|&p| p == name)
                        .and_then(|i| sh.passes.get(i).copied())
                        .unwrap_or(f64::NAN)
                };
                pair.push_str(&format!(
                    "; ms light {:.3} temporal {:.3} atrous {:.3} step {:.3}",
                    ms("light"),
                    ms("temporal"),
                    ms("atrous"),
                    fr.q.stages.total_ms
                ));
                let line = format!(
                    "split {scene} {label}: history bits {:#04b}, lighting {:#010x} (sky changed {}, lights changed {}), restarts {}; \
                     store chunks {} coarse {} window {:?}; queue digs {} digs baked {} whole {} depth {}; \
                     lighting px inside {g_in}: cast in {cast_in} out {cast_out}, age 0 in {a0_in} out {a0_out}; \
                     missing {} chained {} black {}{pair}",
                    fr.history,
                    fr.lighting,
                    fr.sky_changed,
                    fr.lights_changed,
                    fr.restarts,
                    fr.store_chunks,
                    fr.store_coarse,
                    fr.window.map(|w| (w.min.0, w.size())),
                    fr.q.digs,
                    fr.q.digs_baked,
                    fr.q.digs_whole,
                    fr.q.depth,
                    sh.missing,
                    sh.chained,
                    sh.black,
                );
                println!("{line}");
                out.push_str(&line);
                out.push('\n');
                if (SPLIT_BEFORE - 1..SPLIT_BEFORE + 3).contains(&k) {
                    // The parts' means over the lighting pixels inside that cast at the dig's
                    // frame N, at this frame.
                    let n_sh = &frames[SPLIT_BEFORE].shot;
                    let mut sum = [0.0f64; 5];
                    let mut cnt = 0u64;
                    for (g, p) in fr.parts.iter().enumerate() {
                        if g < n_sh.cast.len() && n_sh.cast[g] && inside(&n_sh.hits[g]) {
                            cnt += 1;
                            for j in 0..5 {
                                sum[j] += f64::from(p[j]);
                            }
                        }
                    }
                    let m = sum.map(|v| v / cnt.max(1) as f64);
                    let line = format!(
                        "split {scene} {label} parts over N's cast pixels ({cnt}): stored {:.4} direct {:.4} filtered {:.4} pair x {:.4} stored flag {:.3}",
                        m[0], m[1], m[2], m[3], m[4]
                    );
                    println!("{line}");
                    out.push_str(&line);
                    out.push('\n');
                }
                if (SPLIT_BEFORE - 1..SPLIT_BEFORE + 3).contains(&k) {
                    support::write_png(
                        &dir.join(format!("frame-{label}.png")),
                        &sh.display,
                        OUT[0],
                        OUT[1],
                    );
                }
            }
            let line = format!(
                "split {scene}: the largest outside share over 4 over the pairs from N - 1 to N on {worst:.3} %; adapter {}",
                rig.adapter
            );
            println!("{line}");
            out.push_str(&line);
            std::fs::write(dir.join("split.txt"), out).expect("the split's table");
            rows
        }

        /// One frame of [`split_frames`]: frames N - [`SPLIT_BEFORE`] to N - 1, then N to N +
        /// [`SPLIT_AFTER`]; the shares (percent of the display pixels over 4 levels against the
        /// frame before; 0 at the first) inside and outside the box dilated by 64 voxels, and the
        /// frame's counters.
        pub(in super::super) struct SplitRow {
            pub share_in: f64,
            pub share_out: f64,
            /// Lighting pixels outside the dilated box that cast the sky ray, and whose
            /// history's age is 0.
            pub cast_out: u32,
            pub age0_out: u32,
            pub history: u32,
            /// The sky or the lights' change bit.
            pub flags_changed: bool,
            pub restarts: u64,
        }

        /// The generated castle (the belt's `castle`, its tiles above the floor) placed on the
        /// hills at noon after the hills have drained (at most 30 000 more frames) and settled:
        /// its box and 1 m around carved to air above the ground at its centre, then a stamp a
        /// placement, each op's box invalidated as the belt's run does; the camera at the
        /// castle's `nave` pose inside the keep the whole run. The strip from the frame after
        /// the stamps (`fresh`), then frames paced to 60 Hz until the queue drains (at most
        /// 3000, 50 s) with the lighting grid's missing count read every 100, then 120 more and
        /// another strip (`settled`). The pacing: the queue's slice is GPU time, so an unpaced
        /// test frame (about 2.3 ms in a release build) leaves the bake a few submits per
        /// hundred frames behind the frames' own GPU work.
        ///
        /// The split (Plan 0086's amendment after the castle's stop, call 3): the sequence runs
        /// three times in one process, (a) as above; (b) the exposure pinned from the first frame
        /// (`Exposure::Manual`) at the exposure arm (a) reads after its settled strip; (c) arm
        /// (b) with `LightHooks::key_off`. Each strip prints its per-frame series (the over-8
        /// count per pair, the store's missing and chained counts, the sky rays cast, the
        /// queue's `digs_baked`); the strips land under `<arm>-fresh` and `<arm>-settled`.
        #[test]
        #[ignore = "reading"]
        fn flicker_castle() {
            let Some(e) = castle_arm("a", None, false) else {
                return;
            };
            println!("castle split: arm (a)'s settled exposure {e}, pinned in (b) and (c)");
            castle_arm("b", Some(e), false);
            castle_arm("c", Some(e), true);
        }

        /// One arm of [`flicker_castle`]: the exposure pinned at `pin` from the first frame
        /// (else the auto exposure), the key light off with `key_off`; the exposure read after
        /// the settled strip. `None` without an adapter.
        fn castle_arm(arm: &str, pin: Option<f32>, key_off: bool) -> Option<f32> {
            use flythrough::castle;
            use flythrough::outdoor::{OutdoorSource, Spawn};
            use vxe_world::{ChunkExtent, VoxelCoord, World};
            let file = castle::castle_vox_where(8, |t| !t.dungeon());
            let mut table = flythrough::outdoor_materials();
            let base = table.len() as vxe_world::MaterialId;
            let castle_rows = flythrough::castle_materials(&flythrough::cave_materials());
            for i in castle::STONE..=castle::TORCH {
                let row = castle_rows.rows()[usize::from(castle::material(i))].clone();
                assert_eq!(table.push(row), base + vxe_world::MaterialId::from(i) - 1);
            }
            let mut map = vxe_assets::VoxMap::new();
            map.set(0, vxe_assets::Slot::Skip);
            for i in castle::STONE..=castle::TORCH {
                map.set(
                    i,
                    vxe_assets::Slot::Material(base + vxe_world::MaterialId::from(i) - 1),
                );
            }
            let source = OutdoorSource::new(1);
            let (eye, _) = source.spawn(Spawn::Outside);
            let (ox, oy) = (eye.0[0] + 40, eye.0[1] - 300);
            let o = [ox, oy, source.height(ox + 300, oy + 300)];
            let placements: Vec<_> = file
                .placements()
                .into_iter()
                .map(|p| {
                    let m = &file.models[p.model];
                    let corner = p.min_corner(p.rot.apply_size(m.size)).0;
                    let prefab = map.to_prefab(m).expect("the castle's indices map");
                    (
                        prefab,
                        VoxelCoord([0, 1, 2].map(|k| o[k] + corner[k])),
                        p.rot,
                    )
                })
                .collect();
            let edge = vxe_world::CHUNK_EDGE;
            let lo = [o[0] - 64, o[1] - 64, o[2] - 96].map(|v| v.div_euclid(edge));
            let hi = [o[0] + 664, o[1] + 664, o[2] + 440].map(|v| v.div_euclid(edge) + 1);
            let extent = ChunkExtent::new(lo, hi);
            let carve = (
                [o[0] - 10, o[1] - 10, o[2] + 1].map(i64::from),
                [o[0] + 610, o[1] + 610, o[2] + 410].map(i64::from),
            );
            let place = |w: &World| {
                w.apply(&support::queue::box_op(
                    carve.0,
                    carve.1,
                    vxe_world::Mode::Carve,
                ))
                .expect("the box is resident");
                let mut boxes = vec![(
                    carve.0.map(|v| v as i32),
                    carve.1.map(|v| v as i32),
                )];
                for (p, at, rot) in &placements {
                    let r = w.stamp(p, *at, *rot).expect("the castle is resident");
                    boxes.extend(r.aabb.map(|(a, b)| (a.0, b.0)));
                }
                boxes
            };
            // The pool the mirror takes is the placed world's.
            let pool = {
                let w = World::generate(extent, &source, &table);
                place(&w);
                flythrough::level_pool_units(&w, false) * vxe_render::internals::UNIT_BYTES
            };
            let world = World::generate(extent, &source, &table);
            let mut rig = mirrored_with(world, eye, pool * 5 / 4)?;
            println!("castle arm ({arm}): exposure pinned {pin:?}, key light off {key_off}");
            let (at, yaw, pitch) = castle::CastleShot::Nave.pose();
            let p = [0, 1, 2].map(|k| (at[k] - castle::ORIGIN[k] + o[k]) as f32 + 0.5);
            let (y, pt) = ((yaw as f32).to_radians(), (pitch as f32).to_radians());
            let d = [y.cos() * pt.cos(), y.sin() * pt.cos(), pt.sin()];
            let cam = flythrough::camera_looking_at(p, [p[0] + d[0], p[1] + d[1], p[2] + d[2]]);
            let mut s = strip_frame_with(&mut rig, noon(), &cam, pin, key_off);
            let mut pre = 0;
            while pre < 30_000 && !s.frame.bake_queue().is_some_and(|q| q.empty()) {
                queue_step(&mut rig, &mut s.frame, &cam);
                s.run(&rig, &cam);
                pre += 1;
            }
            println!(
                "castle: before the stamps the queue drained {} after {pre} more frames",
                s.frame.bake_queue().is_some_and(|q| q.empty())
            );
            settle(&mut rig, &mut s, &cam, 240);
            let t = std::time::Instant::now();
            let boxes = place(&rig.world);
            use support::SyncNow;
            rig.gpu
                .sync_now(&rig.world, &rig.device, &rig.queue)
                .expect("the mirror fits");
            for (a, b) in &boxes {
                s.frame.invalidate(*a, *b);
            }
            println!(
                "castle placed at {o:?} in {:.1} s, {} boxes, the carve {:?}..{:?}, eye {p:?}",
                t.elapsed().as_secs_f64(),
                boxes.len(),
                boxes[0].0,
                boxes[0].1
            );
            strip(&mut rig, &mut s, &cam, "castle", &format!("{arm}-fresh"));
            let mut n = 0;
            let mut missing = Vec::new();
            let t = std::time::Instant::now();
            while n < 3000 && !s.frame.bake_queue().is_some_and(|q| q.empty()) {
                let tf = std::time::Instant::now();
                queue_step(&mut rig, &mut s.frame, &cam);
                if n % 100 == 0 {
                    missing.push(s.step(&rig, &cam).missing);
                } else {
                    s.run(&rig, &cam);
                }
                n += 1;
                let frame = std::time::Duration::from_micros(16_667);
                std::thread::sleep(frame.saturating_sub(tf.elapsed()));
            }
            let st = s.frame.bake_queue().map(vxe_render::BakeQueue::stats);
            println!(
                "castle: the queue drained {} after {n} frames, {:.1} s; missing every 100 frames {missing:?}; digs {:?}, digs baked {:?}",
                s.frame.bake_queue().is_some_and(|q| q.empty()),
                t.elapsed().as_secs_f64(),
                st.as_ref().map(|st| st.digs),
                st.as_ref().map(|st| st.digs_baked),
            );
            settle(&mut rig, &mut s, &cam, 120);
            strip(&mut rig, &mut s, &cam, "castle", &format!("{arm}-settled"));
            let e = s.frame.exposure_readout(&rig.device).map(|r| r.exposure);
            println!("castle arm ({arm}): the exposure after the settled strip {e:?}");
            e
        }

        /// [`support::terms::mirrored`] with the pool given.
        fn mirrored_with(
            world: vxe_world::World,
            eye: vxe_world::VoxelCoord,
            pool: u64,
        ) -> Option<Rig> {
            use support::SyncNow;
            use vxe_render::{GpuWorld, MirrorConfig, Window};
            let (device, queue, adapter) = support::reading_device(pool)?;
            let cfg = MirrorConfig {
                budget_bytes: pool.max(512 << 20),
                ..MirrorConfig::new(Window::Fixed(world.extent()))
            };
            let mut gpu = GpuWorld::with_config(&device, cfg);
            gpu.set_focus(eye);
            while gpu
                .sync_now(&world, &device, &queue)
                .expect("the mirror fits")
                .pending_chunks
                > 0
            {}
            Some(Rig {
                device,
                queue,
                gpu,
                adapter,
                world,
            })
        }

        fn noon() -> vxe_render::sky::Sky {
            flythrough::daylight::Clock::default().sky(flythrough::outdoor::BASE_Z as f32)
        }

        /// `cam` turned `deg` degrees to the left about the vertical.
        fn yawed(cam: &Camera, deg: f32) -> Camera {
            let p = cam.position.to_array();
            let f = cam.basis().0.to_array();
            let (c, s) = (deg.to_radians().cos(), deg.to_radians().sin());
            let d = [f[0] * c - f[1] * s, f[0] * s + f[1] * c, f[2]];
            flythrough::camera_looking_at(p, [p[0] + d[0], p[1] + d[1], p[2] + d[2]])
        }

        /// `cam` with its heading kept and its pitch set to `deg` (up positive).
        fn pitched(cam: &Camera, deg: f32) -> Camera {
            let p = cam.position.to_array();
            let f = cam.basis().0.to_array();
            let h = (f[0] * f[0] + f[1] * f[1]).sqrt().max(1e-6);
            let (c, s) = (deg.to_radians().cos(), deg.to_radians().sin());
            let d = [f[0] / h * c, f[1] / h * c, s];
            flythrough::camera_looking_at(p, [p[0] + d[0], p[1] + d[1], p[2] + d[2]])
        }

        /// The fly-through's frame at its defaults on `rig` (the look, voxel leaves, the lamp
        /// off, the outdoor auto exposure with the sky anchor, FSR 2 at 1.5, the live queue at
        /// the fly-through's settings), `sky` set, the queue stepped until it drains (at most
        /// 6000 frames).
        fn strip_frame(rig: &mut Rig, sky: vxe_render::sky::Sky, cam: &Camera) -> Stepper {
            strip_frame_with(rig, sky, cam, None, false)
        }

        /// [`strip_frame`], the exposure pinned at `pin` (`Exposure::Manual`) from the first
        /// frame where given, the key light off with `key_off`.
        fn strip_frame_with(
            rig: &mut Rig,
            sky: vxe_render::sky::Sky,
            cam: &Camera,
            pin: Option<f32>,
            key_off: bool,
        ) -> Stepper {
            let f = flicker_frame(rig, sky, pin, key_off);
            let mut s = Stepper::new(rig, f);
            let mut n = 0;
            while n < 6000 && !s.frame.bake_queue().is_some_and(|q| q.empty()) {
                queue_step(rig, &mut s.frame, cam);
                s.run(rig, cam);
                n += 1;
            }
            println!(
                "adapter {}, the queue drained {} after {n} frames",
                rig.adapter,
                s.frame.bake_queue().is_some_and(|q| q.empty())
            );
            s
        }

        /// The strips' frame: [`strip_frame_with`]'s settings, the live queue started, no
        /// frame stepped.
        fn flicker_frame(
            rig: &Rig,
            sky: vxe_render::sky::Sky,
            pin: Option<f32>,
            key_off: bool,
        ) -> vxe_render::Frame {
            let mut f = vxe_render::Frame::with_sizes(&rig.device, INTERNAL, OUT);
            f.set_upscaler_kind(
                &rig.device,
                vxe_render::UpscalerKind::Fsr2(vxe_render::Fsr2Settings::default()),
            );
            f.set_count_store(true);
            f.set_look(vxe_render::LookParams {
                far_normal: flythrough::LookPreset::DEFAULT.far_normal,
                ..vxe_render::LookParams::default()
            });
            f.set_material_styles(
                &rig.queue,
                &flythrough::outdoor_styles(flythrough::LeafShadow::Voxel),
            );
            f.set_lighting(vxe_render::Lighting {
                lamp: false,
                exposure: vxe_render::Exposure::Auto(vxe_render::AutoExposure {
                    min: flythrough::cli::OUTDOOR_MIN,
                    max: flythrough::cli::OUTDOOR_MAX,
                    ..vxe_render::AutoExposure::default()
                }),
                ..flythrough::default_lighting()
            });
            if let Some(e) = pin {
                f.set_lighting(vxe_render::Lighting {
                    exposure: vxe_render::Exposure::Manual(e),
                    ..f.lighting()
                });
            }
            if key_off {
                f.set_light_hooks(vxe_render::LightHooks {
                    key_off: true,
                    ..Default::default()
                });
            }
            f.set_sky(Some(sky));
            f.set_dt(1.0 / 60.0);
            f.start_bake_queue(
                &rig.queue,
                &rig.gpu,
                vxe_render::queue_settings(),
                vxe_render::DEFAULT_BAKE_SLICE_MS,
            );
            f.set_bake_coarse(8, 2);
            f
        }

        fn settle(rig: &mut Rig, s: &mut Stepper, cam: &Camera, frames: u32) {
            for _ in 0..frames {
                queue_step(rig, &mut s.frame, cam);
                s.run(rig, cam);
            }
        }

        /// [`STRIP`] frames read back and written, and their counts printed.
        fn strip(rig: &mut Rig, s: &mut Stepper, cam: &Camera, scene: &str, name: &str) {
            let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../live/flicker")
                .join(scene)
                .join("current")
                .join(name);
            std::fs::create_dir_all(&dir).expect("the strip's directory");
            let mut shots: Vec<Shot> = Vec::with_capacity(STRIP);
            let mut depth = Vec::with_capacity(STRIP);
            let mut baked = Vec::with_capacity(STRIP);
            for i in 0..STRIP {
                let q = queue_step(rig, &mut s.frame, cam);
                if i == 0 {
                    // The edit's boxes are taken at the strip's first step (stage 6, `boxes`).
                    println!(
                        "{scene} {name}: the first step {:.2} ms, its boxes stage {:.2} ms, digs {}",
                        q.stages.total_ms, q.stages.ms[6], q.digs
                    );
                }
                let sh = s.step(rig, cam);
                support::write_png(
                    &dir.join(format!("frame-{i:03}.png")),
                    &sh.display,
                    OUT[0],
                    OUT[1],
                );
                depth.push(q.depth);
                baked.push(q.digs_baked);
                shots.push(sh);
            }
            let n = (OUT[0] * OUT[1]) as usize;
            let level = |sh: &Shot, i: usize| {
                let p = &sh.display[i * 4..i * 4 + 3];
                [p[0], p[1], p[2]]
            };
            let mut max_d = vec![0u8; n];
            let mut over = [Vec::new(), Vec::new()];
            // Of the pixels over the larger bar: their lighting pixel cast the sky ray, its
            // history is under 16 frames old, it is a miss (the sky).
            let (mut src_cast, mut src_young, mut src_miss, mut src_n) = (0u64, 0u64, 0u64, 0u64);
            for k in 1..STRIP {
                let (a, b) = (&shots[k - 1], &shots[k]);
                let gw = ((a.hits.len() as f64 * f64::from(INTERNAL[0]) / f64::from(INTERNAL[1]))
                    .sqrt()
                    .round() as usize)
                    .max(1);
                let mut c = [0u64; 2];
                for (i, md) in max_d.iter_mut().enumerate() {
                    let (x, y) = (level(a, i), level(b, i));
                    let d = (0..3).map(|j| x[j].abs_diff(y[j])).max().unwrap_or(0);
                    *md = (*md).max(d);
                    for (t, &bar) in OVER.iter().enumerate() {
                        if d > bar {
                            c[t] += 1;
                        }
                    }
                    if d > OVER[1] {
                        let g = grid_of(i, OUT[0] as usize, OUT[1] as usize, gw, b.hits.len());
                        src_n += 1;
                        src_cast += u64::from(b.cast[g]);
                        src_young += u64::from(b.age[g] < 16.0);
                        src_miss += u64::from(b.hits[g][3] >= 1e30);
                    }
                }
                over[0].push(c[0]);
                over[1].push(c[1]);
            }
            let diff: Vec<u8> = max_d
                .iter()
                .flat_map(|&d| {
                    let v = d.saturating_mul(8);
                    [v, v, v, 255]
                })
                .collect();
            support::write_png(&dir.join("diff.png"), &diff, OUT[0], OUT[1]);
            let pct = |v: u64| 100.0 * v as f64 / n as f64;
            let stat = |v: &[u64]| {
                let mean = v.iter().sum::<u64>() as f64 / v.len() as f64;
                (100.0 * mean / n as f64, pct(*v.iter().max().unwrap_or(&0)))
            };
            let (m4, x4) = stat(&over[0]);
            let (m8, x8) = stat(&over[1]);
            let share = |v: u64| {
                if src_n == 0 {
                    0.0
                } else {
                    100.0 * v as f64 / src_n as f64
                }
            };
            let black = shots.iter().map(|s| u64::from(s.black)).max().unwrap_or(0);
            let missing = shots
                .iter()
                .map(|s| u64::from(s.missing))
                .max()
                .unwrap_or(0);
            let cast_px = shots
                .last()
                .map_or(0, |s| s.cast.iter().filter(|&&c| c).count());
            let line = format!(
                "flicker {scene} {name}: over 4 mean {m4:.3} % max {x4:.3} %, over 8 mean {m8:.3} % max {x8:.3} %; \
                 of the over-8 pixels: sky ray cast {:.1} %, age under 16 {:.1} %, sky {:.1} %; \
                 black max {black}, missing max {missing}, sky rays cast (last frame) {cast_px} of {}, queue depth {}..{}; \
                 adapter {}",
                share(src_cast),
                share(src_young),
                share(src_miss),
                shots.last().map_or(0, |s| s.cast.len()),
                depth.first().unwrap_or(&0),
                depth.last().unwrap_or(&0),
                rig.adapter,
            );
            // Per pair k (frames k - 1 to k), the values at frame k.
            let series = format!(
                "flicker {scene} {name} series (frames 1..{STRIP}): over 8 {:?}; missing {:?}; chained {:?}; sky rays cast {:?}; digs baked {:?}",
                over[1],
                shots[1..].iter().map(|s| s.missing).collect::<Vec<_>>(),
                shots[1..].iter().map(|s| s.chained).collect::<Vec<_>>(),
                shots[1..]
                    .iter()
                    .map(|s| s.cast.iter().filter(|&&c| c).count())
                    .collect::<Vec<_>>(),
                &baked[1..],
            );
            println!("{line}");
            println!("{series}");
            std::fs::write(dir.join("counts.txt"), format!("{line}\n{series}\n")).expect("the counts");
        }
    }
}
