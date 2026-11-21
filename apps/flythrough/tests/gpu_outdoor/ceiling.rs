// `gpu_outdoor`'s exposure ceiling by the sky in view (Plan 0094 Phase 7b), included by
// `gpu_outdoor.rs` (kept apart for the file-size cap).

mod ceiling {
    use super::support;
    use support::terms::Rig;
    use vxe_render::{AutoExposure, Camera, Exposure, Frame, LightParams, Lighting};

    /// What [`read`] reads at its last frame: the metered share of sky, the metered level
    /// (log2), the exposure in force (`eye`, log2), the exposure the meter settles at under the
    /// ceiling in force (`settles`, log2), the ceiling in force (log2), and the mean display
    /// luma (Rec. 709 over the encoded bytes, 0 to 1) over the view's central quarter.
    #[derive(Clone, Copy, Debug)]
    pub(super) struct Read {
        pub sky: f32,
        pub metered: f32,
        pub eye: f64,
        pub settles: f64,
        pub ceiling: f64,
        pub luma: f64,
        /// The misses' bounding box in pixels, `[x0, y0, x1, y1]`, and their count.
        pub misses: ([usize; 4], usize),
        /// The highlight's state after the last frame (Plan 0097 Phase 3a).
        pub hi: vxe_render::ExposureHighlight,
        /// Over the whole view: the share of pixels whose brightest display byte is over the
        /// white point's (`HIGHLIGHT_WHITE` encoded, 249), the share at 255, and the 99th and
        /// 99.9th percentile of the brightest byte.
        pub over_white: f64,
        pub at_255: f64,
        pub top: [u8; 2],
    }

    /// The scenes' times: noon, midnight and 17:45 (`outdoor_forest_sunset`'s clock), and the
    /// fixed sun 10 degrees up in the south.
    pub(super) fn clock(minutes: f64, fixed_sun: Option<[f32; 2]>) -> vxe_render::sky::Sky {
        flythrough::daylight::Clock {
            minutes,
            fixed_sun,
            ..Default::default()
        }
        .sky(flythrough::outdoor::BASE_Z as f32)
    }

    /// The outdoor start's auto exposure (the eye at its default rate, `OUTDOOR_MIN` and
    /// `OUTDOOR_MAX`, anchor 4) at the sky share `share`.
    pub(super) fn auto(share: f32) -> AutoExposure {
        AutoExposure {
            min: flythrough::cli::OUTDOOR_MIN,
            max: flythrough::cli::OUTDOOR_MAX,
            sky_anchor_share: share,
            ..AutoExposure::eye()
        }
    }

    /// `cam` under `sky` at native 160 x 90, the fly-through's default lighting with the lamp
    /// `lamp`, the canopy shadowing as `leaf`, under [`auto`] at `share` and `dt` 1/60: 24
    /// frames, the exposure reset to the meter (the adapted level and the ceiling jump), then
    /// 24 more. No bake: the faces with no record take the no-ray prior.
    pub(super) fn read(rig: &mut Rig, cam: &Camera, sky: vxe_render::sky::Sky, lamp: bool, leaf: flythrough::LeafShadow, share: f32) -> Read {
        read_with(rig, cam, sky, lamp, leaf, auto(share), false).0
    }

    /// [`read`] under the auto exposure `a`, the store first baked by `support::bake_around`
    /// at radius 1 and 256 samples about the camera's voxel when `bake`; with the sky over the
    /// floor under the camera from the store after the frames (`vxe_render::sky_over_floor`).
    #[allow(clippy::too_many_arguments, reason = "the scene's parts, as `read` takes them")]
    pub(super) fn read_with(
        rig: &mut Rig,
        cam: &Camera,
        sky: vxe_render::sky::Sky,
        lamp: bool,
        leaf: flythrough::LeafShadow,
        a: AutoExposure,
        bake: bool,
    ) -> (Read, Option<(f32, vxe_render::AroundSource)>) {
        let mut f = Frame::new(&rig.device, 160, 90);
        f.set_look(vxe_render::LookParams {
            far_normal: flythrough::LookPreset::DEFAULT.far_normal,
            ..vxe_render::LookParams::default()
        });
        f.set_material_styles(&rig.queue, &flythrough::outdoor_styles(leaf));
        f.set_lighting(Lighting {
            lamp,
            exposure: Exposure::Auto(a),
            ..flythrough::default_lighting()
        });
        f.set_sky(Some(sky));
        f.set_dt(1.0 / 60.0);
        let eye = cam.position.to_array();
        if bake {
            let centre = eye.map(|v| v.floor() as i32);
            support::bake_around(&rig.device, &rig.queue, &mut rig.gpu, &mut f, &rig.world, centre, 1, 256);
        }
        let light = LightParams::headlamp();
        for k in 0..48 {
            if k == 24 {
                f.reset_exposure();
            }
            f.step(&rig.device, &rig.queue, &rig.gpu, cam, &light);
        }
        let st = vxe_render::debug::read_buffer(&rig.device, &rig.queue, f.exposure_state(), 0, 32);
        let s = vxe_render::ExposureState::from_bytes(&st);
        let px = vxe_render::debug::read_rgba8(&rig.device, &rig.queue, f.target());
        let (w, h) = (160usize, 90usize);
        let (mut sum, mut n) = (0.0f64, 0usize);
        for y in h / 4..3 * h / 4 {
            for x in w / 4..3 * w / 4 {
                let p = &px[(y * w + x) * 4..];
                sum += (0.2126 * f64::from(p[0]) + 0.7152 * f64::from(p[1]) + 0.0722 * f64::from(p[2])) / 255.0;
                n += 1;
            }
        }
        let hb = vxe_render::debug::read_buffer(&rig.device, &rig.queue, f.exposure_highlight(), 0, 16);
        let hi = vxe_render::ExposureHighlight::from_bytes(&hb);
        let white = (255.0 * vxe_render::HIGHLIGHT_WHITE.powf(1.0 / 2.2)).round() as u8;
        let mut tops: Vec<u8> = px.chunks_exact(4).map(|p| p[0].max(p[1]).max(p[2])).collect();
        let np = tops.len() as f64;
        let over_white = tops.iter().filter(|&&t| t > white).count() as f64 / np;
        let at_255 = tops.iter().filter(|&&t| t == 255).count() as f64 / np;
        tops.sort_unstable();
        let q = |p: f64| tops[((p * np) as usize).min(tops.len() - 1)];
        let top = [q(0.99), q(0.999)];
        let rec = vxe_render::debug::read_rgba16u(&rig.device, &rig.queue, f.hit_records());
        let mut bb = [usize::MAX, usize::MAX, 0, 0];
        let mut nm = 0;
        for (i, r) in rec.iter().enumerate() {
            if r[3] & 1 == 0 {
                let (x, y) = (i % w, i / w);
                bb = [bb[0].min(x), bb[1].min(y), bb[2].max(x), bb[3].max(y)];
                nm += 1;
            }
        }
        let read = Read {
            misses: (bb, nm),
            sky: s.sky,
            metered: s.metered_log2,
            eye: f64::from(s.current).log2(),
            settles: a.exposure_under(f64::from(s.metered_log2), s.ceiling).log2(),
            ceiling: f64::from(s.ceiling).log2(),
            luma: sum / n.max(1) as f64,
            hi,
            over_white,
            at_255,
            top,
        };
        (read, vxe_render::sky_over_floor(f.store(), eye))
    }

    /// The tunnel rig with the inside spawn's view out (the cave mouth from inside) and the
    /// deep pose: on the tunnel's axis half way from the mouth to the chamber, facing the
    /// chamber (window 94b's still `4c` was taken deep in, facing away from the mouth).
    pub(super) fn deep() -> Option<(Rig, Camera, Camera)> {
        let (rig, [(_, out), _]) = support::terms::tunnel_rig()?;
        let [a, c] = flythrough::outdoor::OutdoorSource::new(1).tunnel();
        let at = |t: f32| [0, 1, 2].map(|k| a[k] as f32 + 0.5 + (c[k] - a[k]) as f32 * t);
        let inward = flythrough::camera_looking_at(at(0.5), at(0.6));
        Some((rig, out, inward))
    }

    /// Window W2's two poses (Plan 0097, the forest at `--sun 10,180`): the deep shade and the
    /// 1 %-sky spot, eye in voxels from the HUD. The W2 log has no yaw: both face -x, pitched
    /// 10 degrees down.
    pub(super) const W2_SHADE: [f32; 3] = [367.4, 690.8, 569.4];
    pub(super) const W2_SKY: [f32; 3] = [237.4, 642.8, 596.7];

    /// A W2 pose's camera at `p`: facing -x, pitched 10 degrees down, 70 degrees vertical.
    pub(super) fn w2_camera(p: [f32; 3]) -> Camera {
        Camera {
            position: p.into(),
            yaw: std::f32::consts::PI,
            pitch: -10f32.to_radians(),
            vfov: 70f32.to_radians(),
        }
    }

    /// The forest of seed 1 generated about the midpoint of the two W2 poses (both within
    /// 2 chunks of it).
    pub(super) fn w2_rig() -> Option<Rig> {
        let mid = [0, 1, 2].map(|k| ((W2_SHADE[k] + W2_SKY[k]) / 2.0).floor() as i32);
        support::terms::forest_rig_about(mid)
    }

    /// The W2 shade pose with no sky in view: facing -x pitched 30 degrees down (Plan 0097
    /// Phase 1's yaw sweep: at pitch -10 every yaw meters some sky).
    pub(super) fn w2_shade_camera() -> Camera {
        Camera {
            pitch: -30f32.to_radians(),
            ..w2_camera(W2_SHADE)
        }
    }

    /// Plan 0097 Phase 2 bar 3 (ADR-0081): the forest at the fixed sun 10,180 ([`w2_rig`]),
    /// the store baked at radius 1 and 256 samples about each pose, the exposure settled
    /// ([`read_with`]). With no sky in view at the W2 shade pose ([`w2_shade_camera`]) the
    /// anchor holds by the floor's share: the settled eye is within 1 EV of the 1 %-sky pose's
    /// and both ceilings in force are within 1/64 EV of `c_a` (the ceiling under a share `F`
    /// of 0, the anchor whatever is in view). With `F_floor` 0 (the before) the shade pose's
    /// ceiling releases to `OUTDOOR_MAX` within 1/64 EV. `VXE_TEST_ADAPTER` picks the adapter
    /// (unset: lavapipe).
    #[test]
    #[ignore = "soak"]
    fn the_shade_under_the_canopy_keeps_the_anchor() {
        let Some(mut rig) = w2_rig() else {
            return;
        };
        let sun = clock(720.0, Some([10.0, 180.0]));
        let leaf = flythrough::LeafShadow::default();
        let on = auto(vxe_render::DEFAULT_SKY_ANCHOR_SHARE);
        let shade = w2_shade_camera();
        let (s, fs) = read_with(&mut rig, &shade, sun, false, leaf, on, true);
        let (k, fk) = read_with(&mut rig, &w2_camera(W2_SKY), sun, false, leaf, on, true);
        let (b, _) = read_with(&mut rig, &shade, sun, false, leaf, AutoExposure { sky_anchor_floor: 0.0, ..on }, true);
        let (h, _) = read_with(&mut rig, &shade, sun, false, leaf, AutoExposure { sky_anchor_share: 0.0, ..on }, false);
        let c_a = h.ceiling;
        println!("on {}: c_a {c_a:+.3} EV, floor shade {fs:?}, floor 1 % sky {fk:?}", rig.adapter);
        row("W2 shade, floor 1/8", 1.0 / 256.0, &s);
        row("W2 1 % sky, floor 1/8", 1.0 / 256.0, &k);
        row("W2 shade, floor off (the before)", 1.0 / 256.0, &b);
        println!("reading: the eyes apart {:+.3} EV, the before's {:+.3} EV", s.eye - k.eye, b.eye - k.eye);
        assert_eq!(s.sky, 0.0, "no sky in view at the shade pose");
        assert!((s.eye - k.eye).abs() <= 1.0, "shade {} against 1 % sky {}", s.eye, k.eye);
        for (name, r) in [("shade", &s), ("1 % sky", &k)] {
            assert!((r.ceiling - c_a).abs() <= 1.0 / 64.0, "{name}: ceiling {} against c_a {c_a}", r.ceiling);
        }
        let max = f64::from(flythrough::cli::OUTDOOR_MAX).log2();
        assert!((b.ceiling - max).abs() <= 1.0 / 64.0, "the before releases: {}", b.ceiling);
    }

    /// Prints one row: the scene, the arm, and [`Read`]'s fields.
    pub(super) fn row(scene: &str, share: f32, r: &Read) {
        println!(
            "reading: {scene}, share {share}: sky {:.4}, metered {:+.2}, eye {:+.3} EV, settles {:+.3} EV, ceiling {:+.3} EV, centre luma {:.4}, misses {} in {:?}",
            r.sky, r.metered, r.eye, r.settles, r.ceiling, r.luma, r.misses.1, r.misses.0
        );
    }

    /// Plan 0094 Phase 7b bar 2: the deep tunnel (the inside spawn facing in, seed 1) with the
    /// lamp on, at 160 x 90 ([`read`]). At noon under the default share the metered sky share
    /// is 0, the ceiling in force is within 1/64 EV of `OUTDOOR_MAX` and the exposure in force
    /// within 1/64 EV of the one the meter settles at; at midnight, under the default share and
    /// under 0, the ceiling in force and the exposure in force are within 1/64 EV of
    /// `OUTDOOR_MAX` (the anchor never lifts the ceiling over it; Plan 0094 Phase 8a, the
    /// anchored night's eye of +6.24 EV printed as the before). The lamp-lit centre's mean
    /// display luma at noon against midnight is printed as a ratio in EV; within 0.5 EV it is
    /// asserted, over it printed for the log. `VXE_TEST_ADAPTER` picks the adapter (unset:
    /// lavapipe).
    #[test]
    #[ignore = "soak"]
    fn the_tunnel_exposes_for_its_walls_by_day() {
        let Some((mut rig, _, cam)) = deep() else {
            return;
        };
        let leaf = flythrough::LeafShadow::default();
        let share = vxe_render::DEFAULT_SKY_ANCHOR_SHARE;
        let noon = read(&mut rig, &cam, clock(720.0, None), true, leaf, share);
        let night = read(&mut rig, &cam, clock(0.0, None), true, leaf, share);
        let night0 = read(&mut rig, &cam, clock(0.0, None), true, leaf, 0.0);
        println!("on {}", rig.adapter);
        row("deep noon lamp", share, &noon);
        row("deep midnight lamp", share, &night);
        row("deep midnight lamp", 0.0, &night0);
        let ratio = (noon.luma / night.luma).log2();
        println!("reading: the centre's luma noon over midnight {ratio:+.3} EV");
        let max = f64::from(flythrough::cli::OUTDOOR_MAX).log2();
        assert_eq!(noon.sky, 0.0, "no sky in view");
        assert!((noon.ceiling - max).abs() <= 1.0 / 64.0, "the ceiling {}", noon.ceiling);
        assert!((noon.eye - noon.settles).abs() <= 1.0 / 64.0, "eye {} settles {}", noon.eye, noon.settles);
        println!("reading: the deep midnight eye before the clamp (Phase 7b's tip) +6.24 EV");
        for (r, arm) in [(&night, share), (&night0, 0.0)] {
            assert!((r.ceiling - max).abs() <= 1.0 / 64.0, "share {arm}: the ceiling {}", r.ceiling);
            assert!((r.eye - max).abs() <= 1.0 / 64.0, "share {arm}: the eye {}", r.eye);
        }
        if ratio.abs() <= 0.5 {
            assert!(ratio.abs() <= 0.5);
        } else {
            println!("reading: the noon-midnight ratio is over 0.5 EV, disclosed in the log");
        }
    }

    /// One walk of [`walk`]: per frame the metered sky share, the metered level, the ceiling
    /// in force (log2) and the exposure in force (log2).
    pub(super) type Trace = Vec<(f32, f32, f64, f64)>;

    /// The cave mouth walk at noon with the lamp on, native 320 x 180, under [`auto`] at
    /// `share` and `dt` 1/60: on the tunnel's axis (seed 1) from half way to the chamber out to
    /// 15 % of the axis' length past the mouth facing out, then back facing in, `frames` frames
    /// each way, after 24 still frames at the start.
    pub(super) fn walk(rig: &mut Rig, share: f32, frames: u32) -> Trace {
        let [a, c] = flythrough::outdoor::OutdoorSource::new(1).tunnel();
        let at = |t: f32| [0, 1, 2].map(|k| a[k] as f32 + 0.5 + (c[k] - a[k]) as f32 * t);
        let (deep, out) = (0.5f32, -0.15f32);
        let mut f = Frame::new(&rig.device, 320, 180);
        f.set_look(vxe_render::LookParams {
            far_normal: flythrough::LookPreset::DEFAULT.far_normal,
            ..vxe_render::LookParams::default()
        });
        f.set_material_styles(&rig.queue, &flythrough::outdoor_styles(flythrough::LeafShadow::default()));
        f.set_lighting(Lighting {
            exposure: Exposure::Auto(auto(share)),
            ..flythrough::default_lighting()
        });
        f.set_sky(Some(clock(720.0, None)));
        f.set_dt(1.0 / 60.0);
        let light = LightParams::headlamp();
        let mut trace = Trace::new();
        let legs = [(deep, out), (out, deep)];
        for (leg, (from, to)) in legs.into_iter().enumerate() {
            for k in 0..frames + if leg == 0 { 24 } else { 0 } {
                let s = if leg == 0 { k.saturating_sub(24) } else { k } as f32 / frames as f32;
                let t = from + (to - from) * s.min(1.0);
                let p = at(t);
                let ahead = at(t + if to < from { -0.05 } else { 0.05 });
                let cam = flythrough::camera_looking_at(p, ahead);
                f.step(&rig.device, &rig.queue, &rig.gpu, &cam, &light);
                let st = vxe_render::debug::read_buffer(&rig.device, &rig.queue, f.exposure_state(), 0, 32);
                let st = vxe_render::ExposureState::from_bytes(&st);
                trace.push((st.sky, st.metered_log2, f64::from(st.ceiling).log2(), f64::from(st.current).log2()));
            }
        }
        trace
    }

    pub(super) mod reading {
        use super::*;

        /// Plan 0094 Phase 8a bar 4: the anchored ceiling before its clamp,
        /// `c_a = 2^4 key / (0.5 E / pi)` from the pass's own `E_sky(+z)` (its luminance plus
        /// the key light's `w`), against `OUTDOOR_MAX`, at the hills' spawn over the day in
        /// 15-minute steps (one frame each at 16 x 9; `E_sky` is the sky's, not the view's),
        /// printed with the clock times `c_a` crosses `OUTDOOR_MAX`. `VXE_TEST_ADAPTER` picks
        /// the adapter (unset: lavapipe).
        #[test]
        #[ignore = "reading"]
        fn the_ceiling_by_hour() {
            let Some((rig, cam)) = support::terms::hills_rig() else {
                return;
            };
            println!("reading: the_ceiling_by_hour on {}", rig.adapter);
            let a = auto(vxe_render::DEFAULT_SKY_ANCHOR_SHARE);
            let max = a.max.log2();
            let n = a.sky_anchor_ev.expect("the outdoor anchor");
            let mut f = Frame::new(&rig.device, 16, 9);
            f.set_lighting(Lighting {
                exposure: Exposure::Auto(a),
                ..flythrough::default_lighting()
            });
            f.set_dt(1.0 / 60.0);
            let light = LightParams::headlamp();
            let mut last: Option<(f64, f32)> = None;
            for q in 0..=96 {
                let minutes = f64::from(q) * 15.0;
                f.set_sky(Some(clock(minutes, None)));
                f.step(&rig.device, &rig.queue, &rig.gpu, &cam, &light);
                let b = vxe_render::debug::read_buffer(&rig.device, &rig.queue, f.sky_irradiance(), 0, 96);
                let up: Vec<f32> = b[80..96].chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
                let e = vxe_render::ground_irradiance([up[0], up[1], up[2]], up[3]);
                let ca = (n.exp2() * a.key * a.compensation_ev.exp2() / (0.5 * e / std::f32::consts::PI)).log2();
                let (h, m) = (q * 15 / 60, q * 15 % 60);
                println!("reading: {h:02}:{m:02} E {e:.4e}, c_a {ca:+.3} EV, max {max:+.3} EV{}", if ca > max { ", clamped" } else { "" });
                if let Some((t0, c0)) = last
                    && (c0 > max) != (ca > max)
                {
                    let t = t0 + (minutes - t0) * f64::from((max - c0) / (ca - c0));
                    println!("reading: c_a crosses max at {:02}:{:02}", (t / 60.0) as u32, (t % 60.0) as u32);
                }
                last = Some((minutes, ca));
            }
        }

        /// Plan 0094 Phase 7b bars 1, 3 and 6: per scene, [`read`] under the default share
        /// and under 0 (the anchor whatever is in view, the before): the tunnel's inside spawn
        /// facing out (the cave mouth from inside) and facing in (deep), at noon and at
        /// midnight with the lamp on, deep at noon with the lamp off; the hills' spawn facing
        /// the mouth (the mouth from outside) and facing away (the hills) at noon; the forest
        /// at the fixed sun 10,180 under `voxel` and at 17:45. `VXE_TEST_ADAPTER` picks the
        /// adapter (unset: lavapipe).
        #[test]
        #[ignore = "reading"]
        fn the_sky_share_in_view() {
            let shares = [vxe_render::DEFAULT_SKY_ANCHOR_SHARE, 0.0];
            let voxel = flythrough::LeafShadow::Voxel;
            let leaf = flythrough::LeafShadow::default();
            if let Some((mut rig, out, inward)) = deep() {
                println!("reading: the_sky_share_in_view on {}", rig.adapter);
                for (scene, cam, minutes, lamp) in [
                    ("mouth from inside noon lamp", &out, 720.0, true),
                    ("mouth from inside midnight lamp", &out, 0.0, true),
                    ("deep noon lamp", &inward, 720.0, true),
                    ("deep midnight lamp", &inward, 0.0, true),
                    ("deep noon lamp off", &inward, 720.0, false),
                ] {
                    for s in shares {
                        let r = read(&mut rig, cam, clock(minutes, None), lamp, leaf, s);
                        row(scene, s, &r);
                    }
                }
            }
            if let Some((mut rig, cam)) = support::terms::hills_rig() {
                let away = Camera {
                    yaw: cam.yaw + std::f32::consts::PI,
                    ..cam
                };
                for (scene, c) in [("mouth from outside noon", &cam), ("hills noon", &away)] {
                    for s in shares {
                        let r = read(&mut rig, c, clock(720.0, None), false, leaf, s);
                        row(scene, s, &r);
                    }
                }
            }
            if let Some((mut rig, cam)) = support::terms::forest_rig() {
                for (scene, sky, l) in [
                    ("forest 10,180 voxel", clock(720.0, Some([10.0, 180.0])), voxel),
                    ("forest 17:45", clock(17.0 * 60.0 + 45.0, None), leaf),
                ] {
                    for s in shares {
                        let r = read(&mut rig, &cam, sky, false, l, s);
                        row(scene, s, &r);
                    }
                }
            }
        }
        /// Plan 0097 Phase 1 bars 2 and 4: per scene, [`read_with`] under the default share
        /// and the floor's term off (Phase 7b's exposure) with the store baked at radius 1 and 256 samples about the
        /// eye: the sky over the floor (`vxe_render::sky_over_floor`) and its source beside
        /// the metered sky share, the metered level, the ceiling in force and the eye. The W2
        /// shade and 1 %-sky poses at the fixed sun 10,180 ([`w2_rig`]), the tunnel deep in
        /// at noon with the lamp on, 3 m inside the mouth facing in, the mouth from inside,
        /// the hills at noon facing away from the mouth, the forest spawn at 17:45. Then
        /// `F_floor`'s candidate (the largest power-of-two fraction under both W2 poses' share
        /// and over the deep tunnel's) and the walk's cost: 600 calls on the W2 store along the
        /// line between the poses, mean and max microseconds. `VXE_TEST_ADAPTER` picks the
        /// adapter (unset: lavapipe).
        #[test]
        #[ignore = "reading"]
        fn the_sky_over_the_floor() {
            use vxe_render::AroundSource;
            type Floor = Option<(f32, AroundSource)>;
            let a = AutoExposure {
                sky_anchor_floor: 0.0,
                ..auto(vxe_render::DEFAULT_SKY_ANCHOR_SHARE)
            };
            let leaf = flythrough::LeafShadow::default();
            let line = |scene: &str, r: &Read, fl: Floor| {
                let floor = match fl {
                    Some((v, src)) => format!("{v:.4} ({:.0}/255, {})", v * 255.0, src.name()),
                    None => "none".to_string(),
                };
                println!(
                    "reading: {scene}: floor {floor}, sky {:.4}, metered {:+.2}, ceiling {:+.3} EV, eye {:+.3} EV, settles {:+.3} EV",
                    r.sky, r.metered, r.ceiling, r.eye, r.settles
                );
            };
            let mut w2: Vec<Floor> = Vec::new();
            let mut deep_floor: Floor = None;
            if let Some(mut rig) = w2_rig() {
                println!("reading: the_sky_over_the_floor on {}", rig.adapter);
                let sun = clock(720.0, Some([10.0, 180.0]));
                for (scene, p) in [("W2 shade 10,180", W2_SHADE), ("W2 1 % sky 10,180", W2_SKY)] {
                    let (r, fl) = read_with(&mut rig, &w2_camera(p), sun, false, leaf, a, true);
                    line(scene, &r, fl);
                    w2.push(fl);
                }
                // The W2 log has no yaw: the shade spot's view by yaw (16 steps from +x) at
                // pitches -10 and -30 degrees, the metered sky share and the eye (no bake).
                for pitch in [-10.0f32, -30.0] {
                    for k in 0..16 {
                        let yaw = k as f32 * std::f32::consts::TAU / 16.0;
                        let cam = Camera {
                            yaw,
                            pitch: pitch.to_radians(),
                            ..w2_camera(W2_SHADE)
                        };
                        let (r, _) = read_with(&mut rig, &cam, sun, false, leaf, a, false);
                        println!(
                            "reading: W2 shade view yaw {:.1} deg pitch {pitch:.0}: sky {:.4}, ceiling {:+.3} EV, eye {:+.3} EV, metered {:+.2}",
                            yaw.to_degrees(),
                            r.sky,
                            r.ceiling,
                            r.eye,
                            r.metered
                        );
                    }
                }
                // The walk's cost on the shade's store: 600 eyes on the line between the poses.
                let mut f = Frame::new(&rig.device, 16, 9);
                f.set_sky(Some(sun));
                let centre = W2_SHADE.map(|v| v.floor() as i32);
                support::bake_around(&rig.device, &rig.queue, &mut rig.gpu, &mut f, &rig.world, centre, 1, 256);
                let (mut total, mut worst, mut answered) = (0.0f64, 0.0f64, 0);
                for k in 0..600 {
                    let t = k as f32 / 599.0;
                    let eye = [0, 1, 2].map(|i| W2_SHADE[i] + (W2_SKY[i] - W2_SHADE[i]) * t);
                    let t0 = std::time::Instant::now();
                    let fl = std::hint::black_box(vxe_render::sky_over_floor(f.store(), std::hint::black_box(eye)));
                    let us = t0.elapsed().as_secs_f64() * 1e6;
                    total += us;
                    worst = worst.max(us);
                    answered += usize::from(fl.is_some());
                }
                println!(
                    "reading: the walk over 600 eyes between the W2 poses: mean {:.3} us, max {:.3} us, {answered} answered ({})",
                    total / 600.0,
                    worst,
                    if cfg!(debug_assertions) { "debug" } else { "release" }
                );
            }
            if let Some((mut rig, out, inward)) = deep() {
                let [ma, mc] = flythrough::outdoor::OutdoorSource::new(1).tunnel();
                let at = |t: f32| [0, 1, 2].map(|k| ma[k] as f32 + 0.5 + (mc[k] - ma[k]) as f32 * t);
                let len = (0..3).map(|k| ((mc[k] - ma[k]) as f32).powi(2)).sum::<f32>().sqrt();
                let t3 = 30.0 / len;
                let in3 = flythrough::camera_looking_at(at(t3), at(t3 + 0.1));
                let noon = clock(720.0, None);
                for (scene, cam) in [
                    ("tunnel deep noon lamp", &inward),
                    ("tunnel 3 m in, facing in, noon lamp", &in3),
                    ("mouth from inside noon lamp", &out),
                ] {
                    let (r, fl) = read_with(&mut rig, cam, noon, true, leaf, a, true);
                    line(scene, &r, fl);
                    if scene.starts_with("tunnel deep") {
                        deep_floor = fl;
                    }
                }
            }
            if let Some((mut rig, cam)) = support::terms::hills_rig() {
                let away = Camera {
                    yaw: cam.yaw + std::f32::consts::PI,
                    ..cam
                };
                let (r, fl) = read_with(&mut rig, &away, clock(720.0, None), false, leaf, a, true);
                line("hills noon facing away", &r, fl);
            }
            if let Some((mut rig, cam)) = support::terms::forest_rig() {
                let (r, fl) = read_with(&mut rig, &cam, clock(17.0 * 60.0 + 45.0, None), false, leaf, a, true);
                line("forest spawn 17:45", &r, fl);
            }
            let low = w2.iter().map(|f| f.map_or(0.0, |v| v.0)).fold(f32::INFINITY, f32::min);
            let deep_v = deep_floor.map_or(0.0, |v| v.0);
            let pick = (1..=16).map(|k| 2f32.powi(-k)).find(|&p| p < low && p > deep_v);
            println!("reading: F_floor candidate {pick:?} (W2 lowest {low:.4}, deep tunnel {deep_v:.4})");
        }

        /// Plan 0097 Phase 2's table: per scene, [`read_with`] with the store baked about the
        /// eye under the floor's term off (the before) and at the default `F_floor`: the floor
        /// share the frame fed the exposure, the metered sky share, the ceiling in force and
        /// the eye. The W2 shade pose facing -x at pitch -30 (no sky in view) at the fixed sun
        /// 10,180 and at 15:00, the W2 1 %-sky pose; the tunnel deep in at noon and at midnight
        /// and 3 m inside the mouth with the lamp on, the mouth from inside at noon; the hills
        /// facing away from the mouth at noon and at midnight; the forest spawn at 17:45.
        /// `VXE_TEST_ADAPTER` picks the adapter (unset: lavapipe).
        #[test]
        #[ignore = "reading"]
        fn the_anchor_by_the_floor() {
            let on = auto(vxe_render::DEFAULT_SKY_ANCHOR_SHARE);
            let arms = [("floor off", AutoExposure { sky_anchor_floor: 0.0, ..on }), ("floor 1/8", on)];
            let leaf = flythrough::LeafShadow::default();
            let go = |rig: &mut Rig, scene: &str, cam: &Camera, sky: vxe_render::sky::Sky, lamp: bool| {
                for (arm, a) in arms {
                    let (r, fl) = read_with(rig, cam, sky, lamp, leaf, a, true);
                    println!(
                        "reading: {scene}, {arm}: floor {}, sky {:.4}, metered {:+.2}, ceiling {:+.3} EV, eye {:+.3} EV",
                        fl.map_or("none".to_string(), |(v, s)| format!("{v:.4} ({})", s.name())),
                        r.sky,
                        r.metered,
                        r.ceiling,
                        r.eye
                    );
                }
            };
            if let Some(mut rig) = w2_rig() {
                println!("reading: the_anchor_by_the_floor on {}", rig.adapter);
                let shade = w2_shade_camera();
                go(&mut rig, "W2 shade 10,180", &shade, clock(720.0, Some([10.0, 180.0])), false);
                go(&mut rig, "W2 shade 15:00", &shade, clock(900.0, None), false);
                go(&mut rig, "W2 1 % sky 10,180", &w2_camera(W2_SKY), clock(720.0, Some([10.0, 180.0])), false);
            }
            if let Some((mut rig, out, inward)) = deep() {
                let [ma, mc] = flythrough::outdoor::OutdoorSource::new(1).tunnel();
                let at = |t: f32| [0, 1, 2].map(|k| ma[k] as f32 + 0.5 + (mc[k] - ma[k]) as f32 * t);
                let len = (0..3).map(|k| ((mc[k] - ma[k]) as f32).powi(2)).sum::<f32>().sqrt();
                let in3 = flythrough::camera_looking_at(at(30.0 / len), at(30.0 / len + 0.1));
                go(&mut rig, "tunnel deep noon lamp", &inward, clock(720.0, None), true);
                go(&mut rig, "tunnel deep midnight lamp", &inward, clock(0.0, None), true);
                go(&mut rig, "tunnel 3 m in noon lamp", &in3, clock(720.0, None), true);
                go(&mut rig, "mouth from inside noon lamp", &out, clock(720.0, None), true);
            }
            if let Some((mut rig, cam)) = support::terms::hills_rig() {
                let away = Camera {
                    yaw: cam.yaw + std::f32::consts::PI,
                    ..cam
                };
                go(&mut rig, "hills noon away", &away, clock(720.0, None), false);
                go(&mut rig, "hills midnight away", &away, clock(0.0, None), false);
            }
            if let Some((mut rig, cam)) = support::terms::forest_rig() {
                go(&mut rig, "forest spawn 17:45", &cam, clock(17.0 * 60.0 + 45.0, None), false);
            }
        }

        /// Plan 0097 Phase 3a (highlight protection): window 97's tunnel pose (run 4, the HUD's
        /// x 304.4 y 8.8 z 465.7 voxels, noon, the lamp on, the store baked about it) facing
        /// +x, +y, -x and -y at pitch 0 and on the axis toward the chamber ([`deep`]'s), under
        /// the cap off and at the percentiles 95, 97, 98, 99 and 99.5 (at pitch -35 and +25 and
        /// with no bake, the cap off and the default): the eye, the ceiling, the
        /// tail level this frame and smoothed, the cap, and the view's share over the white
        /// point and at 255 with its 99th and 99.9th percentile byte. Then bar 2's scenes, the
        /// cap off against the default: the W2 shade pose at 10,180 and at 15:00, the forest
        /// spawn at 15:00 and the open hills at midnight (the default spawn's view).
        /// `VXE_TEST_ADAPTER` picks the adapter (the RTX for the reading).
        #[test]
        #[ignore = "reading"]
        fn the_highlight_cap() {
            const W97_TUNNEL: [f32; 3] = [304.4, 8.8, 465.7];
            let on = auto(vxe_render::DEFAULT_SKY_ANCHOR_SHARE);
            let leaf = flythrough::LeafShadow::default();
            let go_with = |rig: &mut Rig, scene: &str, cam: &Camera, sky: vxe_render::sky::Sky, lamp: bool, ps: &[f32], bake: bool| {
                for &p in ps {
                    let a = AutoExposure { highlight_percentile: p, ..on };
                    let (r, fl) = read_with(rig, cam, sky, lamp, leaf, a, bake);
                    let arm = if p > 0.0 { format!("P{p}") } else { "off".to_string() };
                    println!(
                        "reading: {scene}, {arm}: floor {}, sky {:.4}, metered {:+.2}, ceiling {:+.3} EV, eye {:+.3} EV, tail {:+.2} (smoothed {:+.2}), cap {}, over white {:.4}, at 255 {:.4}, top p99 {} p99.9 {}",
                        fl.map_or("none".to_string(), |(v, _)| format!("{v:.4}")),
                        r.sky,
                        r.metered,
                        r.ceiling,
                        r.eye,
                        r.hi.now_log2,
                        r.hi.log2,
                        r.hi.cap().map_or("-".to_string(), |c| format!("{:+.3} EV", c.log2())),
                        r.over_white,
                        r.at_255,
                        r.top[0],
                        r.top[1]
                    );
                }
            };
            let go = |rig: &mut Rig, scene: &str, cam: &Camera, sky: vxe_render::sky::Sky, lamp: bool, ps: &[f32]| {
                go_with(rig, scene, cam, sky, lamp, ps, true)
            };
            let pair = [0.0, vxe_render::DEFAULT_EXPOSURE_HIGHLIGHT];
            let noon = clock(720.0, None);
            let sweep = [0.0, 95.0, 97.0, 98.0, 99.0, 99.5];
            if let Some((mut rig, _, inward)) = deep() {
                println!("reading: the_highlight_cap on {}", rig.adapter);
                for (name, yaw) in [("+x", 0.0f32), ("+y", 90.0), ("-x", 180.0), ("-y", 270.0)] {
                    let cam = Camera {
                        position: W97_TUNNEL.into(),
                        yaw: yaw.to_radians(),
                        pitch: 0.0,
                        vfov: 70f32.to_radians(),
                    };
                    go(&mut rig, &format!("W97 tunnel facing {name}"), &cam, noon, true, &sweep);
                    for pitch in [-35.0f32, 25.0] {
                        let c = Camera {
                            pitch: pitch.to_radians(),
                            ..cam
                        };
                        go(&mut rig, &format!("W97 tunnel facing {name} pitch {pitch}"), &c, noon, true, &pair);
                    }
                    go_with(&mut rig, &format!("W97 tunnel facing {name} no bake"), &cam, noon, true, &pair, false);
                }
                let axis = Camera {
                    position: W97_TUNNEL.into(),
                    ..inward
                };
                go(&mut rig, "W97 tunnel along the axis", &axis, noon, true, &sweep);
            }
            if let Some(mut rig) = w2_rig() {
                let shade = w2_shade_camera();
                go(&mut rig, "W2 shade 10,180", &shade, clock(720.0, Some([10.0, 180.0])), false, &pair);
                go(&mut rig, "W2 shade 15:00", &shade, clock(900.0, None), false, &pair);
            }
            if let Some((mut rig, cam)) = support::terms::forest_rig() {
                go(&mut rig, "forest spawn 15:00", &cam, clock(900.0, None), false, &pair);
            }
            if let Some((mut rig, cam)) = support::terms::hills_rig() {
                go(&mut rig, "hills midnight", &cam, clock(0.0, None), false, &pair);
            }
        }

        /// Plan 0094 Phase 7b bar 5: [`walk`] at the default share and at 0 (the anchored
        /// ceiling, the share-0 arm's ceiling in force), 600 frames each way: the trace printed
        /// every 8th frame and in full to stdout's `trace` lines; the largest one-frame step of
        /// log2 exposure at or under `R_f + 1/256`; from the frame the share first reaches `F`
        /// on the way out, the exposure in force at or under the anchored ceiling (plus 1/256)
        /// within `(log2 max - log2 anchored) / R_f + 2` frames and while the share stays at
        /// or over `F`. `VXE_TEST_ADAPTER` picks the adapter (the RTX for the reading).
        #[test]
        #[ignore = "reading"]
        fn the_cave_mouth_walk_exposure() {
            let Some((mut rig, _, _)) = deep() else {
                return;
            };
            let f_full = vxe_render::DEFAULT_SKY_ANCHOR_SHARE;
            let trace = walk(&mut rig, f_full, 600);
            let before = walk(&mut rig, 0.0, 600);
            let anchored = before[24].2;
            println!("reading: the_cave_mouth_walk_exposure on {}, anchored ceiling {anchored:+.3} EV", rig.adapter);
            for (k, (r, b)) in trace.iter().zip(&before).enumerate() {
                println!(
                    "trace {k}: sky {:.4} metered {:+.3} ceiling {:+.3} exposure {:+.3} | share 0: sky {:.4} ceiling {:+.3} exposure {:+.3}",
                    r.0, r.1, r.2, r.3, b.0, b.2, b.3
                );
            }
            let r_f = f64::from(vxe_render::DEFAULT_EXPOSURE_RATE_EV_S) / 60.0;
            let worst = trace.windows(2).map(|w| (w[1].3 - w[0].3).abs()).fold(0.0, f64::max);
            println!("reading: the largest one-frame step {worst:.4} EV against R_f {r_f:.4}");
            let max = f64::from(flythrough::cli::OUTDOOR_MAX).log2();
            let within = ((max - anchored) / r_f).ceil() as usize + 2;
            let half = trace.len() / 2;
            let first = (24..half).find(|&k| trace[k].0 >= f_full);
            println!("reading: the share first reaches F at frame {first:?}; bound {within} frames");
            assert!(worst <= r_f + 1.0 / 256.0, "{worst}");
            if let Some(k0) = first {
                let mut late = Vec::new();
                for k in k0 + within..trace.len() {
                    if (k0..=k).any(|j| trace[j].0 < f_full) {
                        break;
                    }
                    if trace[k].3 > anchored + 1.0 / 256.0 {
                        late.push(k);
                    }
                }
                assert!(late.is_empty(), "over the anchored ceiling at frames {late:?}");
            }
        }
    }
}
