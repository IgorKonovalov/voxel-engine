// `gpu_outdoor`'s flicker strips (Plan 0086, the owner's ask of 2026-11-18), included by
// `gpu_outdoor.rs` (kept apart for the file-size cap).

mod flicker {
    /// Plan 0086 Phase 24a bar 2 (ADR-0076 rule 4's form): a dig changes nothing outside its
    /// reach. `reading::dig_split`'s rig (the hills of seed 1 at noon, the spawn's view 30 degrees
    /// down, the fly-through's defaults, FSR 2 from 640 x 360 to 960 x 540, the live queue drained;
    /// `VXE_SPLIT_TIME` another hour), the 1 m dig about 35 voxels ahead: over every frame pair
    /// from the one ending at the dig's frame N to N + 16, the share of display pixels outside
    /// the box dilated by 64 voxels whose largest channel changes by over 4 levels is at most
    /// the larger of 0.5 % and 1.5 times the still pairs' largest share over the same band
    /// (restated by Plan 0094 Phase 7a: a still image at 17:00 reads about 1 % there by
    /// itself; the still pairs printed);
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
        let still_max = still
            .iter()
            .skip(1)
            .map(|r| r.share_out)
            .fold(0.0f64, f64::max);
        println!(
            "dig: outside the reach at most {worst:.3} % over 4 (the still pairs' max {still_max:.3} %); inside at N {:.3} %",
            n.share_in
        );
        let line = f64::max(0.5, 1.5 * still_max);
        assert!(
            worst <= line,
            "outside the reach: {worst:.3} % over 4 levels, the line {line:.3} %"
        );
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

    /// Plan 0098 Phase 8: a dig under the calm picks neither darkens its surround nor flashes.
    /// `reading::dig_run`'s forest (the spawn at `--sun 10,180`, 30 degrees down, the
    /// fly-through's frame, FSR 2 from 640 x 360 to 960 x 540, the live queue stepped through
    /// [`DIG_DRAIN`] drain frames and a settle of 240), the 1 m dig ahead, read from N - 1 until its jobs
    /// have landed plus 16 and [`reading::DIG_TAIL`] frames, under the defaults with no token
    /// (`default`: the five picks, `Lighting::dig_hold` on; Plan 0098 Phase 4) against the
    /// defaults before the picks with the light history on (`before`, window W2's run 5, "no
    /// flash"). In every frame from N: the ring's share
    /// (`support::defects::DigRow`: display pixels whose hit is outside the dug box and inside it
    /// dilated by 64 voxels) darker by over 4 levels than both N - 1 and the settled frame is at
    /// most the history arm's largest plus 2 points; the hole's share (hits inside the box
    /// dilated by one voxel) brighter than the settled frame by over 4 levels is at most the
    /// history arm's largest plus 2 points, and the hole's mean luma's excess over its settled
    /// mean at most the history arm's largest plus 4 levels. `VXE_DIG`'s arm
    /// (`support::defects::dig_spec`) replaces the defaults' arm: `forest/hold=off` is the
    /// fault the bar was written for. Its claim is the ring's: the hole's is defended by
    /// [`a_fresh_hole_holds_its_value_until_its_light_lands`], whose pose shows the white hole
    /// this rig's does not and which reads no arm as its baseline.
    #[test]
    #[ignore = "soak"]
    fn a_dig_under_calm_neither_darkens_nor_flashes() {
        let (scene, arm, drain) = super::support::defects::dig_spec(("forest", "default", DIG_DRAIN));
        let Some((_, base, n1)) = reading::dig_run(&scene, "before", drain) else {
            return;
        };
        let (_, rows, _) = reading::dig_run(&scene, &arm, drain).expect("the adapter");
        let worst = |rs: &[super::support::defects::DigRow], k: fn(&super::support::defects::DigRow) -> f64| {
            rs[n1 + 1..].iter().map(k).fold(f64::MIN, f64::max)
        };
        let ring = (worst(&base, |r| r.ring_dark_both), worst(&rows, |r| r.ring_dark_both));
        let hole = (worst(&base, |r| r.hole_bright), worst(&rows, |r| r.hole_bright));
        let excess = (worst(&base, |r| r.hole_excess), worst(&rows, |r| r.hole_excess));
        println!(
            "dig {scene} {arm} against before: ring dark {:.2} % against {:.2} %, hole bright {:.2} % against {:.2} %, hole excess {:+.1} against {:+.1}",
            ring.1, ring.0, hole.1, hole.0, excess.1, excess.0
        );
        assert!(ring.1 <= ring.0 + 2.0, "the ring darkens: {:.2} % against {:.2} % + 2", ring.1, ring.0);
        assert!(hole.1 <= hole.0 + 2.0, "the hole flashes: {:.2} % against {:.2} % + 2", hole.1, hole.0);
        assert!(excess.1 <= excess.0 + 4.0, "the hole's mean: {:+.1} against {:+.1} + 4", excess.1, excess.0);
    }

    /// Plan 0098 Phase 17: a fresh hole holds its value until its light lands, and the light
    /// lands within a few frames. `dig_line::hold::claims`' runs (the owner's pose at window
    /// 98d: the forest's spawn at `--sun 10,180`, looking steeply down; the fly-through's frame
    /// and queue flags), sequence A (three digs down one ray, the third with the second in
    /// flight) and B (five digs about the view's centre eight frames apart, a sixth into the
    /// first) on the waiting queue (600 frames in: lavapipe's rig never drains, 161 chunks in
    /// view waiting), and A at the queue's start for the reference landing. No arm is the
    /// baseline: every display pixel is read against its own luma before its dig group
    /// (`before`) and in the group's last frame (`after`), the groups taken at ring 1's
    /// landing (a dig made by the one ahead's ring-1 landing joins its group), so `after` is
    /// the frame before the next group's dig (B: the frame before the next dig; the owner's
    /// pick, 2026-11-24, over the queue's landing of every job). With L the frame a dig's
    /// ring 1 lands: L is at most the dig's frame plus 8 for every dig, and no later than the
    /// reference's plus 1; from each dig's frame to L + `DIG_FADE` the hole's share over 32
    /// levels above the larger of `before` and `after`, and its share over 32 below the
    /// smaller, are each at most 2 %; after that until the next dig, the same at 4 levels; the
    /// ring's at 4 levels in every frame; dots at most the settled frame's plus 1 point; the
    /// ring and the frame within 1 level of N - 1 until the first landing; and from each
    /// group's landing no step away from its settled mean over 1 level. `VXE_DIG`'s arm
    /// (`support::defects::dig_spec`; `hold=rays` the bite check) replaces the defaults'.
    #[test]
    #[ignore = "soak"]
    fn a_fresh_hole_holds_its_value_until_its_light_lands() {
        use super::dig_line::hold::claims;
        let (_, arm, _) = super::support::defects::dig_spec(("above", "default", 0));
        let Some(reference) = claims("A", false, &arm) else {
            return;
        };
        let reference = reference.land[0].expect("the reference dig lands");
        for seq in ["A", "B"] {
            let c = claims(seq, true, &arm).expect("the adapter");
            println!(
                "{}hole {seq} {arm}: ring 1 lands D+{:?} (reference D+{reference}); before the landing white {:.2} % black {:.2} % (32 levels), after white {:.2} % black {:.2} %, ring white {:.2} % black {:.2} %, dots {:.2} % (settled {:.2} %), shift {:.2}, away {:+.2}",
                c.text, c.land, c.before[0], c.before[1], c.after[0], c.after[1], c.ring[0], c.ring[1], c.dots[0], c.dots[1], c.shift, c.away
            );
            for (k, l) in c.land.iter().enumerate() {
                let l = l.unwrap_or_else(|| panic!("{seq} dig {} never lands", k + 1));
                assert!(l <= 8 && l <= reference + 1, "{seq} dig {}: ring 1 lands D+{l} (reference D+{reference})", k + 1);
            }
            assert!(c.before[0] <= 2.0, "{seq}: the hole whitens before its light lands: {:.2} %", c.before[0]);
            assert!(c.before[1] <= 2.0, "{seq}: the hole blackens before its light lands: {:.2} %", c.before[1]);
            assert!(c.after[0] <= 2.0 && c.after[1] <= 2.0, "{seq}: not settled after the fade: {:?}", c.after);
            assert!(c.ring[0] <= 2.0 && c.ring[1] <= 2.0, "{seq}: the ring: {:?}", c.ring);
            assert!(c.dots[0] <= c.dots[1] + 1.0, "{seq}: dots {:.2} % against {:.2} % + 1", c.dots[0], c.dots[1]);
            assert!(c.shift <= 1.0, "{seq}: the ring or the frame shifts before the landing: {:.2}", c.shift);
            assert!(c.away <= 1.0, "{seq}: the landing is not one fade: {:+.2}", c.away);
        }
    }

    /// The frames [`a_dig_under_calm_neither_darkens_nor_flashes`] drains its queue at most
    /// before the settle: none (on lavapipe the view's records are in after the settle's 240).
    const DIG_DRAIN: u32 = 0;

    pub(super) mod reading {
        use super::super::support;
        use support::defects::{Shot, Stepper, dig_ahead, grid_of, queue_step};
        use support::terms::Rig;
        use svet_render::Camera;

        /// The strips' internal size; FSR 2 at 1.5 takes it to [`OUT`].
        pub(in super::super) const INTERNAL: [u32; 2] = [640, 360];
        pub(in super::super) const OUT: [u32; 2] = [960, 540];
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
            let arm = std::env::var("VXE_HOLE_ARM").unwrap_or_else(|_| "no-pool".into());
            s.frame.set_light_hooks(svet_render::LightHooks {
                pool: !arm.ends_with("no-pool"),
                seed_stale_f: arm.starts_with("seed-f"),
                ..Default::default()
            });
            println!("split: arm {arm}");
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

        /// Plan 0094 Phase 1 bar 2: the hills of seed 1 at 17:00 at rest, [`dig_split`]'s pose
        /// and frame (the spawn's view 30 degrees down, the defaults, FSR 2 at 1.5, the queue
        /// drained, 240 settle frames), no dig: over 32 still pairs the share of display
        /// pixels whose largest channel changes by over 4 levels (mean and largest pair). One
        /// arm a test, each in its own process (a second frame on one rig did not drain its
        /// queue in 6000 frames): the defaults under the auto exposure, the exposure pinned at
        /// its settled value, then pinned with `key_no_jitter`, and pinned with `key_off`.
        /// `VXE_TEST_ADAPTER` picks the adapter.
        fn hills_at_rest(arm: &str, pin: bool, hooks: svet_render::LightHooks) {
            let Some((mut rig, cam)) = support::terms::hills_rig() else {
                return;
            };
            let cam = pitched(&cam, -30.0);
            let sky = flythrough::daylight::Clock {
                minutes: 17.0 * 60.0,
                ..Default::default()
            }
            .sky(flythrough::outdoor::BASE_Z as f32);
            let mut s = strip_frame(&mut rig, sky, &cam);
            settle(&mut rig, &mut s, &cam, 240);
            if pin {
                let e = s.frame.exposure_readout(&rig.device).map(|r| r.exposure).expect("a read-back");
                s.frame.set_lighting(svet_render::Lighting {
                    exposure: svet_render::Exposure::Manual(e),
                    ..s.frame.lighting()
                });
            }
            s.frame.set_light_hooks(hooks);
            settle(&mut rig, &mut s, &cam, 64);
            let mut last = s.step(&rig, &cam).display;
            let n = last.len() / 4;
            let mut shares = Vec::new();
            for _ in 0..32 {
                queue_step(&mut rig, &mut s.frame, &cam);
                let now = s.step(&rig, &cam).display;
                let c = support::rules::still_count(&last, &now, support::rules::T_STILL, None);
                shares.push(100.0 * c as f64 / n as f64);
                last = now;
            }
            let mean = shares.iter().sum::<f64>() / shares.len() as f64;
            let max = shares.iter().copied().fold(0.0, f64::max);
            println!(
                "reading: hills 17:00 at rest, {arm}: over 4 mean {mean:.3} % max {max:.3} % of {n} px over 32 pairs ({})",
                rig.adapter
            );
        }

        #[test]
        #[ignore = "reading"]
        fn hills_at_rest_auto() {
            hills_at_rest("auto exposure", false, svet_render::LightHooks::default());
        }

        #[test]
        #[ignore = "reading"]
        fn hills_at_rest_pinned() {
            hills_at_rest("exposure pinned", true, svet_render::LightHooks::default());
        }

        #[test]
        #[ignore = "reading"]
        fn hills_at_rest_no_jitter() {
            let hooks = svet_render::LightHooks {
                key_no_jitter: true,
                ..Default::default()
            };
            hills_at_rest("exposure pinned, key jitter off", true, hooks);
        }

        #[test]
        #[ignore = "reading"]
        fn hills_at_rest_key_off() {
            let hooks = svet_render::LightHooks {
                key_off: true,
                ..Default::default()
            };
            hills_at_rest("exposure pinned, key light off", true, hooks);
        }

        /// Plan 0094 Phase 1 bar 4 (ADR-0076 rule 2's form): the forest of seed 1 under the sun
        /// at `60,180`, the strips' frame with the leaves under `absorb` and under `voxel`, the
        /// queue drained, 240 settle frames, a settled strip of 16 pairs (its largest share of
        /// pixels over `T_STILL`), then a 90-degree yaw over 10 frames, 64 held, back over 10:
        /// the share of display pixels over `T_RETURN` at the return frame against the settled
        /// image, and the frames after the return until a pair's share over `T_STILL` is at or
        /// under the settled strip's largest (at most 600). The queue drains for at most 30 000
        /// frames first; each leaf style runs in its own process. `VXE_TEST_ADAPTER` picks the
        /// adapter.
        #[test]
        #[ignore = "reading"]
        fn turn_and_return_absorb() {
            turn_and_return(flythrough::LeafShadow::Absorb);
        }

        #[test]
        #[ignore = "reading"]
        fn turn_and_return_voxel() {
            turn_and_return(flythrough::LeafShadow::Voxel);
        }

        std::thread_local! {
            /// Whether [`castle_arm`]'s frame reads the prior's arm B.
            static COLUMN_PRIOR: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
        }

        fn turn_and_return(leaf: flythrough::LeafShadow) {
            use support::rules::{T_RETURN, T_STILL, return_count, still_count};
            let Some((mut rig, cam)) = support::terms::forest_rig() else {
                return;
            };
            let sky = flythrough::daylight::Clock {
                fixed_sun: Some([60.0, 180.0]),
                ..Default::default()
            }
            .sky(flythrough::outdoor::BASE_Z as f32);
            {
                let f = flicker_frame_with(&rig, sky, None, false, leaf);
                let mut s = Stepper::new(&rig, f);
                let mut n = 0;
                while n < 30_000 && !s.frame.bake_queue().is_some_and(|q| q.empty()) {
                    queue_step(&mut rig, &mut s.frame, &cam);
                    s.run(&rig, &cam);
                    n += 1;
                }
                println!(
                    "turn and return, leaves {leaf:?}: the queue drained {} after {n} frames",
                    s.frame.bake_queue().is_some_and(|q| q.empty())
                );
                settle(&mut rig, &mut s, &cam, 240);
                let settled = s.step(&rig, &cam).display;
                let px = settled.len() / 4;
                let pct = |c: usize| 100.0 * c as f64 / px as f64;
                let mut last = settled.clone();
                let mut still_max = 0.0f64;
                for _ in 0..16 {
                    queue_step(&mut rig, &mut s.frame, &cam);
                    let now = s.step(&rig, &cam).display;
                    still_max = still_max.max(pct(still_count(&last, &now, T_STILL, None)));
                    last = now;
                }
                let settled_return = pct(return_count(&settled, &last, T_RETURN));
                for k in (1..=10).chain(std::iter::repeat_n(10, 64)).chain((1..10).rev()) {
                    let c = yawed(&cam, 9.0 * k as f32);
                    queue_step(&mut rig, &mut s.frame, &c);
                    s.run(&rig, &c);
                }
                queue_step(&mut rig, &mut s.frame, &cam);
                let ret = s.step(&rig, &cam).display;
                let at_return = pct(return_count(&settled, &ret, T_RETURN));
                let mut last = ret;
                let mut back = None;
                let mut series = Vec::new();
                for i in 1..=600 {
                    queue_step(&mut rig, &mut s.frame, &cam);
                    let now = s.step(&rig, &cam).display;
                    let share = pct(still_count(&last, &now, T_STILL, None));
                    if i <= 64 && i % 8 == 0 {
                        series.push(format!("{i}: {share:.2} % / ret {:.2} %", pct(return_count(&settled, &now, T_RETURN))));
                    }
                    last = now;
                    if share <= still_max {
                        back = Some(i);
                        break;
                    }
                }
                println!(
                    "reading: turn and return, leaves {leaf:?} on {}: return frame {at_return:.3} % over {T_RETURN} (a settled frame 16 later {settled_return:.3} %); \
                     settled pairs over {T_STILL} at most {still_max:.3} %; back at it after {back:?} frames; series {series:?}",
                    rig.adapter
                );
            }
        }

        /// Plan 0094 Phase 2 bar 2: the dig's hole band. [`dig_split`]'s rig at
        /// `VXE_SPLIT_TIME` (16:00 and 17:00 on the shadowed side), drained and
        /// settled, 8 frames read, the dig at N, then frames until the dig's job lands (the
        /// queue's `digs_baked` rises; at most 400) and `F + 16` more. The bands by the hit's
        /// distance from the dig's box at N: the hole band (within 8 voxels) and the ring's edge
        /// (8 to 16). Per band: on the pairs ending at N to N + 4, of the pixels over 4 levels,
        /// the shares whose lighting pixel cast the sky ray, holds a pair at the seed's age
        /// (`SEED_AGE` 8), a history of age 0 (the key mean's restart), and each source word;
        /// the mean |seed - settled| of the pair's mean over the pixels seeded at N (the
        /// contact shadow's seed against what the pixel settles to, frame `landing + F + 16`);
        /// and rule 3's class shares over the trace from N - 8 to the landing plus `F + 16`, the
        /// fade allowed `landing + F` frames; the a-trous pass's ms still and over the band's
        /// frames. `VXE_HOLE_ARM`: `no-pool` (the default), `pool` (`LightHooks::pool`),
        /// `key-off` (the pool with `LightHooks::key_off`), `seed-f` (the pool) and
        /// `seed-f-no-pool` (those with `LightHooks::seed_stale_f`); [`dig_split`] reads it too. `VXE_TEST_ADAPTER` picks the adapter.
        #[test]
        #[ignore = "reading"]
        fn hole_band() {
            use support::rules::{F, MEAN_FRAMES, T_FADE, TraceClass, trace_classes};
            let Some((mut rig, cam)) = support::terms::hills_rig() else {
                return;
            };
            let cam = pitched(&cam, -30.0);
            let mut s = strip_frame(&mut rig, split_sky(), &cam);
            let arm = std::env::var("VXE_HOLE_ARM").unwrap_or_else(|_| "no-pool".into());
            s.frame.set_light_hooks(svet_render::LightHooks {
                pool: !arm.ends_with("no-pool"),
                key_off: arm == "key-off",
                seed_stale_f: arm.starts_with("seed-f"),
                ..Default::default()
            });
            let mut n = 0;
            while n < 30_000 && !s.frame.bake_queue().is_some_and(|q| q.empty()) {
                queue_step(&mut rig, &mut s.frame, &cam);
                s.run(&rig, &cam);
                n += 1;
            }
            settle(&mut rig, &mut s, &cam, 240);
            let pair_of = |rig: &Rig, s: &Stepper| {
                svet_render::debug::read_rgba16f(&rig.device, &rig.queue, s.frame.visibility_pair())
            };
            let mut shots: Vec<(Shot, Vec<[f32; 4]>)> = Vec::new();
            for _ in 0..MEAN_FRAMES {
                queue_step(&mut rig, &mut s.frame, &cam);
                let sh = s.step(&rig, &cam);
                shots.push((sh, pair_of(&rig, &s)));
            }
            let (min, max) = dig_ahead(&mut rig, &mut s.frame, &cam);
            let mut landing = None;
            let mut baked0 = None;
            let mut k = 0usize;
            loop {
                let q = queue_step(&mut rig, &mut s.frame, &cam);
                let b0 = *baked0.get_or_insert(q.digs_baked);
                let sh = s.step(&rig, &cam);
                shots.push((sh, pair_of(&rig, &s)));
                if landing.is_none() && q.digs_baked > b0 {
                    landing = Some(k);
                }
                k += 1;
                if landing.is_some_and(|l| k > l + F + 16) || k > 400 {
                    break;
                }
            }
            let Some(landing) = landing else {
                println!("hole band: the dig's job did not land within 400 frames");
                return;
            };
            let at_n = &shots[MEAN_FRAMES].0;
            let gw = ((at_n.hits.len() as f64 * f64::from(INTERNAL[0]) / f64::from(INTERNAL[1]))
                .sqrt()
                .round() as usize)
                .max(1);
            let dist = |h: &[f32; 4]| -> Option<f32> {
                if h[3] >= 1e30 {
                    return None;
                }
                let d2: f32 = (0..3)
                    .map(|k| {
                        let v = h[k];
                        let d = (min[k] as f32 - v).max(v - (max[k] + 1) as f32).max(0.0);
                        d * d
                    })
                    .sum();
                Some(d2.sqrt())
            };
            let n_px = (OUT[0] * OUT[1]) as usize;
            let grid = |i: usize| grid_of(i, OUT[0] as usize, OUT[1] as usize, gw, at_n.hits.len());
            let band_of = |i: usize| match dist(&at_n.hits[grid(i)]) {
                Some(d) if d <= 8.0 => Some(0),
                Some(d) if d <= 16.0 => Some(1),
                _ => None,
            };
            let settled = &shots[(MEAN_FRAMES + landing + F + 16).min(shots.len() - 1)].1;
            // The a-trous passes' ms (the variance pass and its four iterations share the
            // slot): the 8 still frames, and the frames from N to the landing plus F.
            let atrous = svet_render::FRAME_PASSES.iter().position(|&n| n == "atrous");
            let ms = |r: std::ops::Range<usize>| {
                let v: Vec<f64> = shots[r]
                    .iter()
                    .filter_map(|(sh, _)| atrous.and_then(|k| sh.passes.get(k).copied()))
                    .collect();
                v.iter().sum::<f64>() / v.len().max(1) as f64
            };
            println!(
                "reading: hole band arm {arm} at {}: the a-trous pass {:.3} ms still, {:.3} ms from N to the landing + F ({})",
                std::env::var("VXE_SPLIT_TIME").unwrap_or_else(|_| "12:00".into()),
                ms(0..MEAN_FRAMES),
                ms(MEAN_FRAMES..(MEAN_FRAMES + landing + F).min(shots.len())),
                rig.adapter
            );
            for (b, name) in ["hole band", "ring edge 8..16"].iter().enumerate() {
                let region: Vec<bool> = (0..n_px).map(|i| band_of(i) == Some(b)).collect();
                let px = region.iter().filter(|&&r| r).count();
                // The pairs ending at N .. N + 4.
                let (mut over, mut cast, mut seed, mut age0) = (0u64, 0u64, 0u64, 0u64);
                let mut words = [0u64; 7];
                for e in MEAN_FRAMES..MEAN_FRAMES + 5 {
                    let (a, (sh, pair)) = (&shots[e - 1].0, &shots[e]);
                    for i in (0..n_px).filter(|&i| region[i]) {
                        if support::rules::change(&a.display, &sh.display, i) <= 4 {
                            continue;
                        }
                        let g = grid(i);
                        over += 1;
                        cast += u64::from(sh.cast[g]);
                        seed += u64::from((pair[g][3] - 8.0).abs() < 0.5);
                        age0 += u64::from(sh.age[g] < 0.5);
                        words[usize::from(sh.source[g]).min(6)] += 1;
                    }
                }
                let pc = |v: u64| 100.0 * v as f64 / over.max(1) as f64;
                let w: Vec<String> = support::defects::SOURCE_NAMES
                    .iter()
                    .zip(words)
                    .filter(|(_, c)| *c > 0)
                    .map(|(s, c)| format!("{s} {:.1} %", pc(c)))
                    .collect();
                // The seed against the settled mean at the lighting pixels seeded at N.
                let pn = &shots[MEAN_FRAMES].1;
                let (mut dev, mut seeded) = (0f64, 0u64);
                for g in 0..pn.len() {
                    let h = &at_n.hits[g];
                    let in_band = match dist(h) {
                        Some(d) => if b == 0 { d <= 8.0 } else { d > 8.0 && d <= 16.0 },
                        None => false,
                    };
                    if in_band && at_n.cast[g] && (pn[g][3] - 8.0).abs() < 0.5 {
                        dev += f64::from((pn[g][0] - settled[g][0]).abs());
                        seeded += 1;
                    }
                }
                let refs: Vec<&[u8]> = shots[..].iter().map(|(sh, _)| sh.display.as_slice()).collect();
                let c = trace_classes(&refs, &region, T_FADE, landing + F);
                let cls: Vec<String> = TraceClass::ALL
                    .iter()
                    .map(|t| format!("{t:?} {:.2} %", 100.0 * c[t.index()] as f64 / px.max(1) as f64))
                    .collect();
                println!(
                    "reading: hole band at {}, {name}: {px} px; landing at N + {landing}; pairs N..N+4 over 4: {over} px-pairs, \
                     cast {:.1} %, pair at the seed's age {:.1} %, history age 0 {:.1} %, sources [{}]; \
                     seeded at N {seeded} lighting px, mean |seed - settled| {:.4}; rule 3 classes [{}] ({})",
                    std::env::var("VXE_SPLIT_TIME").unwrap_or_else(|_| "12:00".into()),
                    pc(cast),
                    pc(seed),
                    pc(age0),
                    w.join(", "),
                    dev / seeded.max(1) as f64,
                    cls.join(", "),
                    rig.adapter
                );
            }
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
            use svet_render::{GpuWorld, MirrorConfig, QueueView, Window};
            use svet_world::{ChunkCoord, PersistConfig, StreamConfig, Streamer, Tier, World};
            let residency = flythrough::RESIDENCY;
            let budget = svet_render::DEFAULT_BUDGET_BYTES;
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
            let mut tick = |rig: &mut Rig, f: &mut svet_render::Frame, cam: &Camera| {
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
                        svet_render::internals::view_half_angle(cam.vfov, w as f32 / h as f32)
                    },
                    frustum: None,
                    full_radius: Some(residency.r_full),
                    lod_radius: Some(residency.r_lod.max(residency.r_full)),
                    focal_px: None,
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

        /// Plan 0098 Phase 8 step 1: a dig's ring and hole under an arm, from N - 1 until every
        /// dig job has landed (the queue's `digs` 0) plus its longest fade (16 frames) plus
        /// [`DIG_TAIL`] frames. The scene, the arm and the drain from
        /// `support::defects::dig_spec`: `hills` (seed 1 at noon, [`flicker_dig`]'s pose) or
        /// `forest` (the forest's spawn at `--sun 10,180`, 30 degrees down), the arm's tokens
        /// ([`dig_arm`]), and the most frames the queue drains before the settle. Per frame the ring's and the hole's display figures
        /// (`support::defects::DigRow`) and their lighting pixels' sources, rays, shown
        /// visibilities, key slots and parts; the table lands in
        /// `live/flicker/dig-terms/<scene>-<arm>.txt`.
        #[test]
        #[ignore = "reading"]
        fn dig_terms() {
            let (scene, arm, drain) = support::defects::dig_spec(("hills", "default", 30_000));
            let Some((frames, rows, n1)) = dig_run(&scene, &arm, drain) else {
                return;
            };
            let mut out = String::new();
            for (f, r) in frames.iter().zip(&rows) {
                let line = support::defects::dig_line(f, r);
                println!("{line}");
                out.push_str(&line);
                out.push('\n');
            }
            let after = &rows[n1 + 1..];
            let worst = |k: fn(&support::defects::DigRow) -> f64| {
                after.iter().map(k).fold(f64::MIN, f64::max)
            };
            let line = format!(
                "dig {scene} {arm}: largest ring dark {:.2} % (both {:.2} %), hole bright {:.2} %, hole excess {:+.1}; frames {}",
                worst(|r| r.ring_dark),
                worst(|r| r.ring_dark_both),
                worst(|r| r.hole_bright),
                worst(|r| r.hole_excess),
                frames.len()
            );
            println!("{line}");
            out.push_str(&line);
            let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../live/flicker/dig-terms");
            std::fs::create_dir_all(&dir).expect("the reading's directory");
            std::fs::write(dir.join(format!("{scene}-{}.txt", arm.replace([',', '='], "-"))), out)
                .expect("the table");
        }

        /// Frames read after the dig's jobs have landed and their fades ended.
        pub(in super::super) const DIG_TAIL: usize = 24;

        /// The arm's tokens over the fly-through's defaults (the five picks of ADR-0082):
        /// `calm` (the five picks named: light history off, key far column, key far
        /// mix step, the band 4..8, the disc), then one put back each: `hist` (light history
        /// on), `baked` (key far baked), `blend` (key far mix blend), `band` (the band 2..6),
        /// `approach` (the approach penumbra); `before` (the fly-through's defaults before the
        /// picks: `hist`, `baked`, `blend`, `band` and `approach`, window W2's run 5 "no
        /// flash"); `pin` the
        /// exposure manual at its settled value from the settle's end; `hold=on|land|rays|off`
        /// (`Lighting::dig_hold`).
        pub(in super::super) fn dig_arm(f: &mut svet_render::Frame, arm: &str) -> bool {
            let mut l = f.lighting();
            let mut look = f.look();
            let mut pin = false;
            for t in arm.split(',') {
                match t {
                    "default" => {}
                    "calm" => {
                        l.light_history = false;
                        l.key_far = svet_render::KeyFar::Column;
                        l.key_far_mix = svet_render::KeyFarMix::Step;
                        l.penumbra = svet_render::Penumbra::Disc;
                        look.far_normal = [4.0, 8.0];
                    }
                    "before" => {
                        l.light_history = true;
                        l.key_far = svet_render::KeyFar::Baked;
                        l.key_far_mix = svet_render::KeyFarMix::Blend;
                        look.far_normal = [2.0, 6.0];
                        l.penumbra = svet_render::Penumbra::Approach;
                    }
                    "hist" => l.light_history = true,
                    "baked" => l.key_far = svet_render::KeyFar::Baked,
                    "blend" => l.key_far_mix = svet_render::KeyFarMix::Blend,
                    "band" => look.far_normal = [2.0, 6.0],
                    "approach" => l.penumbra = svet_render::Penumbra::Approach,
                    "pin" => pin = true,
                    t if t.starts_with("hold=") => {
                        l.dig_hold = svet_render::DigHold::from_name(&t[5..]).expect("a hold");
                    }
                    other => panic!("unknown dig arm token {other}"),
                }
            }
            f.set_lighting(l);
            f.set_look(look);
            pin
        }

        /// [`dig_terms`]' run, the queue stepped at most `drain` frames before the settle
        /// until it drains: the frames from N - 1 (index 0 is N - 2), their rows, and N - 1's
        /// index; `None` without an adapter.
        pub(in super::super) fn dig_run(
            scene: &str,
            arm: &str,
            drain: u32,
        ) -> Option<(Vec<support::defects::DigFrame>, Vec<support::defects::DigRow>, usize)> {
            let (mut rig, cam, sky) = match scene {
                "forest" => {
                    let (rig, cam) = support::terms::forest_rig()?;
                    let sky = flythrough::daylight::Clock {
                        fixed_sun: Some([10.0, 180.0]),
                        ..Default::default()
                    }
                    .sky(flythrough::outdoor::BASE_Z as f32);
                    (rig, cam, sky)
                }
                _ => {
                    let (rig, cam) = support::terms::hills_rig()?;
                    (rig, cam, noon())
                }
            };
            let cam = pitched(&cam, -30.0);
            let mut f = flicker_frame(&rig, sky, None, false);
            let pin = dig_arm(&mut f, arm);
            let mut s = Stepper::new(&rig, f);
            let mut n = 0;
            while n < drain && !s.frame.bake_queue().is_some_and(|q| q.empty()) {
                queue_step(&mut rig, &mut s.frame, &cam);
                s.run(&rig, &cam);
                n += 1;
            }
            println!(
                "dig {scene} {arm}: adapter {}, drained {} after {n} frames",
                rig.adapter,
                s.frame.bake_queue().is_some_and(|q| q.empty())
            );
            settle(&mut rig, &mut s, &cam, 240);
            if pin {
                let st = svet_render::debug::read_buffer(&rig.device, &rig.queue, s.frame.exposure_state(), 0, 32);
                let e = svet_render::ExposureState::from_bytes(&st).current;
                s.frame.set_lighting(svet_render::Lighting {
                    exposure: svet_render::Exposure::Manual(e),
                    ..s.frame.lighting()
                });
                settle(&mut rig, &mut s, &cam, 8);
            }
            // The box is known only after the dig: the frames before it are read against the
            // box `dig_ahead` will cut, found by the same march.
            let fwd = cam.basis().0.to_array();
            let eye = cam.position.to_array();
            let t = support::defects::march(&rig.world, eye, fwd, 400.0).expect("the view hits ground");
            let c = [0, 1, 2].map(|k| (eye[k] + fwd[k] * (t + 0.5)).floor() as i64);
            let bx = (c.map(|v| v - 5), c.map(|v| v + 5));
            let mut frames = Vec::new();
            for k in [2, 1] {
                let q = queue_step(&mut rig, &mut s.frame, &cam);
                frames.push(support::defects::dig_frame(&rig, &mut s, &cam, q, bx, format!("N-{k}"), OUT));
            }
            let dug = dig_ahead(&mut rig, &mut s.frame, &cam);
            assert_eq!(dug, bx, "the dig's box");
            let mut tail = None;
            for k in 0..1200usize {
                let q = queue_step(&mut rig, &mut s.frame, &cam);
                // Landed: no dig waits or runs; its fade is FADE frames at most.
                let done = q.digs == 0 && k > 0;
                frames.push(support::defects::dig_frame(&rig, &mut s, &cam, q, bx, format!("N+{k}"), OUT));
                if done && tail.is_none() {
                    tail = Some(k + 16 + DIG_TAIL);
                }
                if tail == Some(k) {
                    break;
                }
            }
            println!("dig {scene} {arm}: landed {}", tail.is_some());
            let rows = support::defects::dig_rows(&frames, 1);
            Some((frames, rows, 1))
        }

        /// The pitch of the dig from above (degrees, up positive): the owner's pose at W3
        /// (`1b-dig2-during.png`: the forest's spawn, looking steeply down at open ground).
        pub(in super::super) const ABOVE_PITCH: f32 = -65.0;

        /// The eye of the owner's dig on the RADV RENOIR at Portable (W3's run 5,
        /// `5-radeon-dig-dots.png`'s HUD), voxels.
        const PORTABLE_EYE: [f32; 3] = [1501.0, 1245.5, 564.3];

        /// Plan 0098 Phase 14 step 1: the fly-through's dig (`support::defects::dig_along`)
        /// three times down one ray from above, in the owner's pose (the forest's spawn at
        /// `--sun 10,180`, [`ABOVE_PITCH`]). The scene `above` steps the queue at most `drain`
        /// frames until no chunk in view waits, then settles 240 frames; `coarse` digs at the
        /// first step whose queue holds a coarse block with chunks in view waiting; `portable`
        /// and `portable-coarse` likewise at [`PORTABLE_EYE`] facing +x, the forest at 15:00,
        /// the Portable profile's march (853 x 480 to 1280 x 720) and queue. Frames N - 2
        /// and N - 1, the first dig, frames until it lands plus 16 and [`DIG_TAIL`], the second
        /// dig, one frame, the third (the second still in flight where the queue says so), and
        /// frames until both land plus 16 and [`DIG_TAIL`]. With the rig after the digs; `None`
        /// without an adapter.
        pub(in super::super) fn hole_run(
            scene: &str,
            arm: &str,
            drain: u32,
            stages: bool,
            png: Option<&std::path::Path>,
        ) -> Option<(support::defects::HoleRun, Rig)> {
            use support::defects::{HoleFrame, dig_along, hole_frame};
            let portable = scene.starts_with("portable");
            let (mut rig, cam, clock, sizes) = if portable {
                let rig = support::terms::forest_rig_about(PORTABLE_EYE.map(|v| v as i32))?;
                let cam = flythrough::camera_looking_at(PORTABLE_EYE, [PORTABLE_EYE[0] + 1.0, PORTABLE_EYE[1], PORTABLE_EYE[2]]);
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
                (rig, cam, clock, [INTERNAL, OUT])
            };
            let sky = clock.sky(flythrough::outdoor::BASE_Z as f32);
            let cam = pitched(&cam, ABOVE_PITCH);
            let mut f = flicker_frame_at(&rig, sky, None, false, flythrough::LeafShadow::Voxel, sizes);
            if portable {
                // The Portable profile's queue and lamp (`RenderSettings::profile`).
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
            dig_arm(&mut f, arm);
            let hold = f.lighting().dig_hold != svet_render::DigHold::Off;
            let mut s = Stepper::new(&rig, f);
            let mut n = 0;
            loop {
                let q = queue_step(&mut rig, &mut s.frame, &cam);
                s.run(&rig, &cam);
                n += 1;
                let done = match scene {
                    "coarse" | "portable-coarse" => q.coarse > 0 && q.drain_waiting > 0,
                    _ => q.drain_waiting == 0 && n > 1,
                };
                if done || n >= drain {
                    println!(
                        "hole {scene} {arm}: adapter {}, after {n} frames in view waiting {}, coarse {}, depth {}",
                        rig.adapter, q.drain_waiting, q.coarse, q.depth
                    );
                    break;
                }
            }
            if !scene.ends_with("coarse") {
                settle(&mut rig, &mut s, &cam, 240);
            }
            let eye = cam.position.to_array();
            let ground = support::defects::march(&rig.world, eye, cam.basis().0.to_array(), 400.0);
            println!("hole {scene} {arm}: eye {eye:?}, ground {ground:?} voxels along the view");
            let (mut frames, mut digs): (Vec<HoleFrame>, _) = (Vec::new(), Vec::new());
            let read = |rig: &mut Rig, s: &mut Stepper, frames: &mut Vec<HoleFrame>, label: String| {
                let q = queue_step(rig, &mut s.frame, &cam);
                let path = png.map(|d| d.join(format!("{:03}-{label}.png", frames.len())));
                frames.push(hole_frame(rig, s, &cam, q, label, out, stages, path.as_deref()));
                q
            };
            for k in [2, 1] {
                read(&mut rig, &mut s, &mut frames, format!("N-{k}"));
            }
            for d in 1..=3 {
                let dug = dig_along(&mut rig, &mut s.frame, &cam).expect("the dig changed voxels");
                digs.push((frames.len(), dug.0, dug.1));
                let mut tail = None;
                for k in 0..1200usize {
                    let q = read(&mut rig, &mut s, &mut frames, format!("D{d}+{k}"));
                    if d == 2 {
                        println!("hole {scene} {arm}: the third dig with digs {} in flight", q.digs);
                        break;
                    }
                    if q.digs == 0 && k > 0 && tail.is_none() {
                        tail = Some(k + 16 + DIG_TAIL);
                    }
                    if tail == Some(k) {
                        break;
                    }
                }
            }
            println!("hole {scene} {arm}: {} frames, digs {digs:?}", frames.len());
            Some((support::defects::HoleRun { frames, n1: 1, digs, out, hold, eye }, rig))
        }

        /// Plan 0098 Phase 14 step 1's reading ([`hole_run`]; `VXE_DIG`'s scene `above` or
        /// `coarse`, its arm and drain): the frames under `live/flicker/dig-above/<scene>-<arm>/`
        /// and the tables (`support::defects::hole_tables`) in `tables.txt` beside them.
        #[test]
        #[ignore = "reading"]
        fn dig_from_above() {
            let (scene, arm, drain) = support::defects::dig_spec(("above", "default", 30_000));
            let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../live/flicker/dig-above")
                .join(format!("{scene}-{}", arm.replace([',', '='], "-")));
            std::fs::create_dir_all(&dir).expect("the reading's directory");
            let Some((run, rig)) = hole_run(&scene, &arm, drain, true, Some(&dir)) else {
                return;
            };
            let text = support::defects::hole_tables(&run, &rig.world);
            println!("{text}");
            std::fs::write(dir.join("tables.txt"), text).expect("the tables");
        }

        /// The split's sky: noon, or the clock at `VXE_SPLIT_TIME` (`HH:MM`).
        fn split_sky() -> svet_render::sky::Sky {
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
            window: Option<svet_world::ChunkExtent>,
            q: svet_render::QueueStats,
            /// Per lighting pixel: the stored part's, the direct part's and the filter's output
            /// luminance, the pair's sky mean and the stored flag (the moments' w).
            parts: Vec<[f32; 5]>,
        }

        /// A frame's queue step (and, on a streamed rig, the stream's poll and the mirror's
        /// sync before it).
        type Tick<'a> = dyn FnMut(&mut Rig, &mut svet_render::Frame, &Camera) -> svet_render::QueueStats + 'a;

        fn split_frame(rig: &mut Rig, s: &mut Stepper, cam: &Camera, tick: &mut Tick<'_>) -> SplitFrame {
            let q = tick(rig, &mut s.frame, cam);
            let shot = s.step(rig, cam);
            let f = &s.frame;
            let (history, lighting) = f.last_encoded_flags();
            let st = f.store().stats(0, 0, 0);
            let parts = {
                use svet_render::debug::read_rgba16f;
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
                    svet_render::FRAME_PASSES
                        .iter()
                        .position(|&p| p == name)
                        .and_then(|i| sh.passes.get(i).copied())
                        .unwrap_or(f64::NAN)
                };
                pair.push_str(&format!(
                    "; ms light {:.3} temporal {:.3} atrous {:.3} step {:.3}; step samples {} dig bricks {}",
                    ms("light"),
                    ms("temporal"),
                    ms("atrous"),
                    fr.q.stages.total_ms,
                    fr.q.samples,
                    fr.q.dig_bricks
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
        /// `VXE_CASTLE_ARM` runs one arm alone in its process: `a`, or `b:E` / `c:E` pinned at
        /// exposure `E` (arm (a)'s printed one); a name ending in `B` (`aB`) reads the
        /// no-record prior's arm B (`LightHooks::column_prior`).
        #[test]
        #[ignore = "reading"]
        fn flicker_castle() {
            if let Ok(v) = std::env::var("VXE_CASTLE_ARM") {
                let (arm, pin) = v.split_once(':').map_or((v.as_str(), None), |(a, e)| {
                    (a, Some(e.parse::<f32>().expect("an exposure")))
                });
                COLUMN_PRIOR.with(|c| c.set(arm.ends_with('B')));
                castle_arm(arm, pin, arm.starts_with('c'));
                return;
            }
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
            use svet_world::{ChunkExtent, VoxelCoord, World};
            let file = castle::castle_vox_where(8, |t| !t.dungeon());
            let mut table = flythrough::outdoor_materials();
            let base = table.len() as svet_world::MaterialId;
            let castle_rows = flythrough::castle_materials(&flythrough::cave_materials());
            for i in castle::STONE..=castle::TORCH {
                let row = castle_rows.rows()[usize::from(castle::material(i))].clone();
                assert_eq!(table.push(row), base + svet_world::MaterialId::from(i) - 1);
            }
            let mut map = svet_assets::VoxMap::new();
            map.set(0, svet_assets::Slot::Skip);
            for i in castle::STONE..=castle::TORCH {
                map.set(
                    i,
                    svet_assets::Slot::Material(base + svet_world::MaterialId::from(i) - 1),
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
            let edge = svet_world::CHUNK_EDGE;
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
                    svet_world::Mode::Carve,
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
                flythrough::level_pool_units(&w, false) * svet_render::internals::UNIT_BYTES
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
            s.frame.set_light_hooks(svet_render::LightHooks {
                key_off,
                column_prior: COLUMN_PRIOR.with(std::cell::Cell::get),
                ..Default::default()
            });
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
            let mut light = Vec::new();
            let t = std::time::Instant::now();
            while n < 3000 && !s.frame.bake_queue().is_some_and(|q| q.empty()) {
                let tf = std::time::Instant::now();
                queue_step(&mut rig, &mut s.frame, &cam);
                if n % 100 == 0 {
                    let shot = s.step(&rig, &cam);
                    missing.push(shot.missing);
                    light.extend(shot.light_ms);
                } else {
                    light.extend(s.timed(&rig, &cam));
                }
                n += 1;
                let frame = std::time::Duration::from_micros(16_667);
                std::thread::sleep(frame.saturating_sub(tf.elapsed()));
            }
            let st = s.frame.bake_queue().map(svet_render::BakeQueue::stats);
            println!(
                "castle: the queue drained {} after {n} frames, {:.1} s; missing every 100 frames {missing:?}; digs {:?}, digs baked {:?}",
                s.frame.bake_queue().is_some_and(|q| q.empty()),
                t.elapsed().as_secs_f64(),
                st.as_ref().map(|st| st.digs),
                st.as_ref().map(|st| st.digs_baked),
            );
            print_light_series(&light);
            settle(&mut rig, &mut s, &cam, 120);
            strip(&mut rig, &mut s, &cam, "castle", &format!("{arm}-settled"));
            let e = s.frame.exposure_readout(&rig.device).map(|r| r.exposure);
            println!("castle arm ({arm}): the exposure after the settled strip {e:?}");
            e
        }

        /// The light pass's ms over the castle's drain (one value a timed frame): the count,
        /// the mean, the p50, p95 and max, the frames over the lighting line (4.0 ms) and the
        /// longest run of them, and the mean of each 100 frames.
        fn print_light_series(ms: &[f64]) {
            if ms.is_empty() {
                println!("castle: light pass untimed (no timestamps)");
                return;
            }
            let mut v = ms.to_vec();
            v.sort_by(f64::total_cmp);
            let q = |p: f64| v[((v.len() - 1) as f64 * p).round() as usize];
            let (mut over, mut run, mut longest) = (0, 0, 0);
            for &x in ms {
                if x > 4.0 {
                    over += 1;
                    run += 1;
                    longest = longest.max(run);
                } else {
                    run = 0;
                }
            }
            let hundreds: Vec<String> = ms
                .chunks(100)
                .map(|c| format!("{:.3}", c.iter().sum::<f64>() / c.len() as f64))
                .collect();
            println!(
                "castle: light pass over the drain: {} frames, mean {:.3} ms, p50 {:.3}, p95 {:.3}, max {:.3}; over 4.0 ms {over} frames, longest run {longest}; mean a 100 frames [{}]",
                ms.len(),
                ms.iter().sum::<f64>() / ms.len() as f64,
                q(0.5),
                q(0.95),
                v[v.len() - 1],
                hundreds.join(", ")
            );
        }

        /// [`support::terms::mirrored`] with the pool given.
        fn mirrored_with(
            world: svet_world::World,
            eye: svet_world::VoxelCoord,
            pool: u64,
        ) -> Option<Rig> {
            use support::SyncNow;
            use svet_render::{GpuWorld, MirrorConfig, Window};
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

        fn noon() -> svet_render::sky::Sky {
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
        pub(in super::super) fn pitched(cam: &Camera, deg: f32) -> Camera {
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
        /// 30 000 frames).
        fn strip_frame(rig: &mut Rig, sky: svet_render::sky::Sky, cam: &Camera) -> Stepper {
            strip_frame_with(rig, sky, cam, None, false)
        }

        /// [`strip_frame`], the exposure pinned at `pin` (`Exposure::Manual`) from the first
        /// frame where given, the key light off with `key_off`.
        fn strip_frame_with(
            rig: &mut Rig,
            sky: svet_render::sky::Sky,
            cam: &Camera,
            pin: Option<f32>,
            key_off: bool,
        ) -> Stepper {
            let f = flicker_frame(rig, sky, pin, key_off);
            let mut s = Stepper::new(rig, f);
            let mut n = 0;
            while n < 30_000 && !s.frame.bake_queue().is_some_and(|q| q.empty()) {
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
            sky: svet_render::sky::Sky,
            pin: Option<f32>,
            key_off: bool,
        ) -> svet_render::Frame {
            flicker_frame_with(rig, sky, pin, key_off, flythrough::LeafShadow::Voxel)
        }

        /// [`flicker_frame`] with the leaves' shadow `leaf`.
        fn flicker_frame_with(
            rig: &Rig,
            sky: svet_render::sky::Sky,
            pin: Option<f32>,
            key_off: bool,
            leaf: flythrough::LeafShadow,
        ) -> svet_render::Frame {
            flicker_frame_at(rig, sky, pin, key_off, leaf, [INTERNAL, OUT])
        }

        /// [`flicker_frame_with`] at the internal and output sizes `sizes`.
        pub(in super::super) fn flicker_frame_at(
            rig: &Rig,
            sky: svet_render::sky::Sky,
            pin: Option<f32>,
            key_off: bool,
            leaf: flythrough::LeafShadow,
            sizes: [[u32; 2]; 2],
        ) -> svet_render::Frame {
            let mut f = svet_render::Frame::with_sizes(&rig.device, sizes[0], sizes[1]);
            f.set_upscaler_kind(
                &rig.device,
                svet_render::UpscalerKind::Fsr2(svet_render::Fsr2Settings::default()),
            );
            f.set_count_store(true);
            f.set_look(svet_render::LookParams {
                far_normal: flythrough::LookPreset::DEFAULT.far_normal,
                ..svet_render::LookParams::default()
            });
            f.set_material_styles(
                &rig.queue,
                &flythrough::outdoor_styles(leaf),
            );
            f.set_lighting(svet_render::Lighting {
                lamp: false,
                exposure: svet_render::Exposure::Auto(svet_render::AutoExposure {
                    min: flythrough::cli::OUTDOOR_MIN,
                    max: flythrough::cli::OUTDOOR_MAX,
                    ..svet_render::AutoExposure::default()
                }),
                ..flythrough::default_lighting()
            });
            if let Some(e) = pin {
                f.set_lighting(svet_render::Lighting {
                    exposure: svet_render::Exposure::Manual(e),
                    ..f.lighting()
                });
            }
            if key_off {
                f.set_light_hooks(svet_render::LightHooks {
                    key_off: true,
                    ..Default::default()
                });
            }
            f.set_sky(Some(sky));
            f.set_dt(1.0 / 60.0);
            f.start_bake_queue(
                &rig.queue,
                &rig.gpu,
                svet_render::queue_settings(),
                svet_render::DEFAULT_BAKE_SLICE_MS,
            );
            f.set_bake_coarse(8, 2);
            f
        }

        pub(in super::super) fn settle(rig: &mut Rig, s: &mut Stepper, cam: &Camera, frames: u32) {
            for _ in 0..frames {
                queue_step(rig, &mut s.frame, cam);
                s.run(rig, cam);
            }
        }

        /// A castle strip frame's parts: the exposure this frame used, and per lighting pixel
        /// the stored part's and the accumulated direct part's luminance (pre-exposed).
        struct CastleParts {
            exposure: f32,
            stored: Vec<f32>,
            direct: Vec<f32>,
        }

        fn castle_parts(rig: &Rig, s: &Stepper) -> CastleParts {
            use svet_render::debug::{read_buffer, read_rgba16f};
            let (d, q) = (&rig.device, &rig.queue);
            let lum = |p: &[f32; 4]| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
            let st = read_buffer(d, q, s.frame.exposure_state(), 0, 32);
            CastleParts {
                exposure: svet_render::ExposureState::from_bytes(&st).previous,
                stored: read_rgba16f(d, q, s.frame.accumulated()).iter().map(lum).collect(),
                direct: read_rgba16f(d, q, s.frame.direct_accumulated()).iter().map(lum).collect(),
            }
        }

        /// Plan 0094 Phase 2 bar 1: the castle strip's pixels over 4 and over 8 by source and
        /// transition. Per pair and pixel over the bar, its lighting pixel `g`, in order: the
        /// exposure's (its step alone, `level |e_b / e_a - 1|`, is over the bar); a switch (the
        /// sky term's source word differs between the two frames: `a>b`); the key light's (the
        /// accumulated direct part's change, rescaled by the exposure step, is larger than the
        /// stored part's) at a no-ray pixel; else the source word at `b`: the chain (own
        /// summary, neighbours), the store (a fresh record), the pair (a stale record or no
        /// value: the ray, its key light term included), none. The transition: `reset` (the
        /// history's age fell), `young` (under 16), `steady`. The keep's walls (the hit within
        /// a voxel of a castle material) are a class of their own; the over-8 counts per pair
        /// are printed beside `digs_baked`.
        fn castle_sources(rig: &Rig, shots: &[Shot], parts: &[CastleParts], baked: &[u64], name: &str) {
            use std::collections::BTreeMap;
            use support::defects::{
                SOURCE_NAMES, SOURCE_COLUMN, SOURCE_NONE, SOURCE_OWN_SUMMARY, SOURCE_RECORD,
            };
            let base = flythrough::outdoor_materials().len() as svet_world::MaterialId;
            let n = (OUT[0] * OUT[1]) as usize;
            let gl = shots[0].hits.len();
            let gw = ((gl as f64 * f64::from(INTERNAL[0]) / f64::from(INTERNAL[1])).sqrt().round() as usize).max(1);
            let wall_of = |h: &[f32; 4]| {
                if h[3] >= 1e30 {
                    return false;
                }
                (0..8).any(|k| {
                    let v: [i32; 3] = std::array::from_fn(|a| (h[a] + if k >> a & 1 == 1 { 0.5 } else { -0.5 }).floor() as i32);
                    rig.world.material_at(svet_world::VoxelCoord(v)).is_some_and(|m| m >= base)
                })
            };
            for bar in [4u8, 8] {
                // (class, walls) -> count; transition -> count.
                let mut by: BTreeMap<(String, bool), u64> = BTreeMap::new();
                let mut trans: BTreeMap<(String, bool), u64> = BTreeMap::new();
                let mut per_pair = Vec::new();
                let mut total = [0u64; 2];
                for k in 1..shots.len() {
                    let (a, b) = (&shots[k - 1], &shots[k]);
                    let (pa, pb) = (&parts[k - 1], &parts[k]);
                    let ratio = pb.exposure / pa.exposure.max(1e-12);
                    let mut c = 0u64;
                    for i in 0..n {
                        let d = support::rules::change(&a.display, &b.display, i);
                        if d <= bar {
                            continue;
                        }
                        c += 1;
                        let g = grid_of(i, OUT[0] as usize, OUT[1] as usize, gw, gl);
                        let wall = wall_of(&b.hits[g]);
                        total[usize::from(wall)] += 1;
                        let lv = f32::from(support::rules::level(&a.display, i));
                        let (sa, sb) = (a.source[g], b.source[g]);
                        let class = if lv * (ratio - 1.0).abs() > f32::from(bar) {
                            "exposure".to_string()
                        } else if sa != sb {
                            format!("switch {}>{}", SOURCE_NAMES[usize::from(sa).min(6)], SOURCE_NAMES[usize::from(sb).min(6)])
                        } else {
                            let ds = (pb.stored[g] - pa.stored[g] * ratio).abs();
                            let dd = (pb.direct[g] - pa.direct[g] * ratio).abs();
                            match sb {
                                SOURCE_OWN_SUMMARY | SOURCE_COLUMN | SOURCE_RECORD if dd > ds => "key".into(),
                                SOURCE_OWN_SUMMARY | SOURCE_COLUMN => "chain".into(),
                                SOURCE_RECORD => "store".into(),
                                SOURCE_NONE => "none".into(),
                                _ => "pair".into(),
                            }
                        };
                        *by.entry((class, wall)).or_default() += 1;
                        let t = if b.age[g] + 0.5 < a.age[g] {
                            "reset"
                        } else if b.age[g] < 16.0 {
                            "young"
                        } else {
                            "steady"
                        };
                        let tl = format!("{} {t}", SOURCE_NAMES[usize::from(sb).min(6)]);
                        *trans.entry((tl, wall)).or_default() += 1;
                    }
                    per_pair.push(c);
                }
                let fmt = |m: &BTreeMap<(String, bool), u64>, wall: bool| {
                    let t = total[usize::from(wall)].max(1) as f64;
                    let mut v: Vec<(&String, &u64)> = m.iter().filter(|((_, w), _)| *w == wall).map(|((s, _), c)| (s, c)).collect();
                    v.sort_by(|x, y| y.1.cmp(x.1));
                    v.iter().map(|(s, c)| format!("{s} {:.1} %", 100.0 * **c as f64 / t)).collect::<Vec<_>>().join(", ")
                };
                println!(
                    "castle {name} sources over {bar}: walls {} px-pairs [{}]; elsewhere {} px-pairs [{}]",
                    total[1],
                    fmt(&by, true),
                    total[0],
                    fmt(&by, false)
                );
                println!(
                    "castle {name} transitions over {bar}: walls [{}]; elsewhere [{}]",
                    fmt(&trans, true),
                    fmt(&trans, false)
                );
                println!("castle {name} over {bar} per pair {per_pair:?}; digs baked {:?}", &baked[1..]);
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
            let mut parts: Vec<CastleParts> = Vec::new();
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
                if scene == "castle" {
                    parts.push(castle_parts(rig, s));
                }
                shots.push(sh);
            }
            if scene == "castle" {
                castle_sources(rig, &shots, &parts, &baked, name);
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
