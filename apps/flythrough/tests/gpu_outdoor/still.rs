// `gpu_outdoor`'s stop rule on the occluded rig (Plan 0032 Phase 6's, Plan 0086 Phase 25 bar
// 1), included by `gpu_outdoor.rs` (kept apart for the file-size cap).

mod still {
    use super::support;
    use support::terms::Rig;
    use vxe_render::{
        AutoExposure, Camera, Exposure, Frame, LightParams, Lighting, UpscalerKind,
    };

    /// The display's per-pixel changes (levels of 255, the largest channel) over `frames`
    /// still frames at `cam`, after 64 settle frames under an instant auto exposure and then
    /// at its exposure pinned: with `sky` the fly-through's defaults outdoors (noon's sky, the
    /// key light and one sky ray, the lamp off, the store baked whole first, the outdoor
    /// exposure's floor and ceiling, anchor 4), else the headlamp alone with no sky. FSR 2
    /// from 320 x 180 to 480 x 270 with `fsr2`, else native 320 x 180. The display's width.
    fn changes(rig: &mut Rig, cam: &Camera, sky: bool, fsr2: bool, frames: u32) -> (Vec<Vec<u8>>, usize) {
        let c = changes_with(rig, cam, sky, fsr2, frames, Arm::default());
        (c.px, c.width)
    }

    /// A term split's arm: the sky rays (`None`: the defaults' count), the key light off, the
    /// missing-light chain off (`MissingLight::Black`).
    #[derive(Clone, Copy, Default)]
    struct Arm {
        sky_rays: Option<u32>,
        key_off: bool,
        chain_off: bool,
    }

    /// [`changes_with`]'s read: the per-pixel changes and the display's width, and at the last
    /// frame per lighting pixel the history's age and the sky term's source word, the lighting
    /// grid's width, and the history rejections per still frame.
    struct Changes {
        px: Vec<Vec<u8>>,
        width: usize,
        age: Vec<f32>,
        source: Vec<u8>,
        grid_w: usize,
        rejections: Vec<[u32; 4]>,
    }

    /// [`changes`] under `arm`, the store counted.
    fn changes_with(rig: &mut Rig, cam: &Camera, sky: bool, fsr2: bool, frames: u32, arm: Arm) -> Changes {
        let mut f = if fsr2 {
            let mut f = Frame::with_sizes(&rig.device, [320, 180], [480, 270]);
            f.set_upscaler_kind(
                &rig.device,
                UpscalerKind::Fsr2(vxe_render::Fsr2Settings::default()),
            );
            f
        } else {
            Frame::new(&rig.device, 320, 180)
        };
        f.set_look(vxe_render::LookParams {
            far_normal: flythrough::LookPreset::DEFAULT.far_normal,
            ..vxe_render::LookParams::default()
        });
        let defaults = flythrough::default_lighting();
        let base = Lighting {
            lamp: !sky,
            sky_rays: arm.sky_rays.unwrap_or(defaults.sky_rays),
            ..defaults
        };
        f.set_count_store(true);
        f.set_light_hooks(vxe_render::LightHooks {
            key_off: arm.key_off,
            ..Default::default()
        });
        let auto = if sky {
            AutoExposure {
                min: flythrough::cli::OUTDOOR_MIN,
                max: flythrough::cli::OUTDOOR_MAX,
                ..AutoExposure::instant()
            }
        } else {
            AutoExposure::instant()
        };
        f.set_lighting(Lighting {
            exposure: Exposure::Auto(auto),
            ..base
        });
        if sky {
            f.set_material_styles(
                &rig.queue,
                &flythrough::outdoor_styles(flythrough::LeafShadow::default()),
            );
            let noon =
                flythrough::daylight::Clock::default().sky(flythrough::outdoor::BASE_Z as f32);
            f.set_sky(Some(noon));
            f.bake_until(
                &rig.device,
                &rig.queue,
                &mut rig.gpu,
                &rig.world,
                rig.world.extent(),
                &vxe_render::BakeSettings::default(),
                &mut |_| {},
            )
            .expect("the store fits");
            if arm.chain_off {
                f.set_missing_light(&rig.queue, vxe_render::MissingLight::Black);
            }
        }
        f.set_dt(1.0 / 60.0);
        let light = LightParams::headlamp();
        for _ in 0..64 {
            f.step(&rig.device, &rig.queue, &rig.gpu, cam, &light);
        }
        let st = vxe_render::debug::read_buffer(&rig.device, &rig.queue, f.exposure_state(), 0, 32);
        let e = vxe_render::ExposureState::from_bytes(&st).current;
        f.set_lighting(Lighting {
            exposure: Exposure::Manual(e),
            ..base
        });
        let read = |f: &Frame| vxe_render::debug::read_rgba8(&rig.device, &rig.queue, f.target());
        let mut last = read(&f);
        let mut per_px: Vec<Vec<u8>> = vec![Vec::new(); last.len() / 4];
        let mut rejections = Vec::new();
        for _ in 0..frames {
            f.step(&rig.device, &rig.queue, &rig.gpu, cam, &light);
            rejections.push(f.read_history_rejections(&rig.device, &rig.queue));
            let now = read(&f);
            for (i, (a, b)) in now.chunks(4).zip(last.chunks(4)).enumerate() {
                per_px[i].push((0..3).map(|k| a[k].abs_diff(b[k])).max().unwrap_or(0));
            }
            last = now;
        }
        let mom = vxe_render::debug::read_rgba16f(&rig.device, &rig.queue, f.moments());
        Changes {
            px: per_px,
            width: f.target().width() as usize,
            age: mom.iter().map(|m| m[2]).collect(),
            source: mom.iter().map(|m| support::defects::source_of(m[3])).collect(),
            grid_w: f.moments().width() as usize,
            rejections,
        }
    }

    /// Plan 0094 Phase 1 bar 3: [`tunnel_beside_the_lamp`]'s rig under FSR 2 at 1.5 over
    /// `frames` still frames, the far end's (sky-ray hit rate under 0.05) and the rim band's
    /// p99 change per arm: the defaults, `sky_rays` 0, the key light off, the chain off
    /// (`MissingLight::Black`), and the headlamp; over the far end's lighting pixels at the
    /// defaults the age histogram (under 16, under 64, at the run's length), the source words'
    /// shares, and the mean rejections per frame.
    fn tunnel_far_end_by_term(frames: u32) {
        let Some((mut rig, views)) = support::terms::tunnel_rig() else {
            return;
        };
        let (_, cam) = &views[0];
        let (rate, gw) = support::terms::hit_rates(&rig, cam);
        let gh = rate.len() / gw;
        let arms = [
            ("defaults", true, Arm::default()),
            ("sky_rays 0", true, Arm { sky_rays: Some(0), ..Arm::default() }),
            ("key off", true, Arm { key_off: true, ..Arm::default() }),
            ("chain off", true, Arm { chain_off: true, ..Arm::default() }),
            ("headlamp", false, Arm::default()),
        ];
        for (name, sky, arm) in arms {
            let c = changes_with(&mut rig, cam, sky, true, frames, arm);
            let w = c.width;
            let h = c.px.len() / w;
            let at = |x: usize, y: usize| rate[(y * gh / h) * gw + x * gw / w];
            let rim = |x: usize, y: usize| at(x, y).is_some_and(|r| (0.3..0.6).contains(&r));
            let far = |x: usize, y: usize| at(x, y).is_some_and(|r| r < 0.05);
            let (pr, nr) = p99(&c.px, w, &rim);
            let (pf, nf) = p99(&c.px, w, &far);
            println!(
                "reading: tunnel far end by term on {}, {name}: far end p99 {pf} ({nf} px), rim band p99 {pr} ({nr} px), {frames} frames under FSR 2",
                rig.adapter
            );
            if name != "defaults" {
                continue;
            }
            // The far end's lighting pixels: the lighting grid's pixels whose rate is under 0.05.
            let lw = c.grid_w;
            let lh = c.age.len() / lw;
            let far_l: Vec<usize> = (0..c.age.len())
                .filter(|&i| {
                    let (x, y) = (i % lw, i / lw);
                    rate[(y * gh / lh) * gw + x * gw / lw].is_some_and(|r| r < 0.05)
                })
                .collect();
            let n = far_l.len().max(1) as f64;
            let share = |f: &dyn Fn(usize) -> bool| 100.0 * far_l.iter().filter(|&&i| f(i)).count() as f64 / n;
            let full = (frames + 64 - 1) as f32;
            println!(
                "reading: tunnel far end ages: {} lighting px, under 16 {:.2} %, under 64 {:.2} %, at {} or over {:.2} %",
                far_l.len(),
                share(&|i| c.age[i] < 16.0),
                share(&|i| c.age[i] < 64.0),
                full,
                share(&|i| c.age[i] >= full),
            );
            let words: Vec<String> = support::defects::SOURCE_NAMES
                .iter()
                .enumerate()
                .map(|(k, s)| format!("{s} {:.2} %", share(&|i| usize::from(c.source[i]) == k)))
                .collect();
            println!("reading: tunnel far end sources: {}", words.join(", "));
            let m = c.rejections.len().max(1) as f64;
            let r: [f64; 4] = std::array::from_fn(|k| c.rejections.iter().map(|v| f64::from(v[k])).sum::<f64>() / m);
            println!(
                "reading: tunnel rejections per frame (the whole grid): tested {:.0}, material {:.1}, normal {:.1}, other {:.1}",
                r[0], r[1], r[2], r[3]
            );
        }
    }

    /// The 99th percentile of the changes of the display pixels in `region`, and their count.
    fn p99(px: &[Vec<u8>], width: usize, region: &dyn Fn(usize, usize) -> bool) -> (u8, usize) {
        let inside: Vec<usize> = (0..px.len())
            .filter(|&i| region(i % width, i / width))
            .collect();
        let mut v: Vec<u8> = inside.iter().flat_map(|&i| px[i].iter().copied()).collect();
        v.sort_unstable();
        (v.get(v.len() * 99 / 100).copied().unwrap_or(0), inside.len())
    }

    /// Plan 0032 Phase 6's stop rule on the tunnel (`--source outdoor --spawn inside`, seed 1,
    /// noon, facing out), read: over `frames` still frames, the 99th-percentile frame-to-frame
    /// change of the display over the mouth's rim band (the sky ray's hit rate 0.3 to 0.6) and
    /// over the far end (under 0.05) at the defaults, beside the headlamp's at the same pose
    /// with the sky off. Printed; Plan 0086 Phase 25 bar 1's claim is the first at or under the
    /// second in both regions.
    fn tunnel_beside_the_lamp(frames: u32, fsr2: bool) {
        let Some((mut rig, views)) = support::terms::tunnel_rig() else {
            return;
        };
        let (_, cam) = &views[0];
        let (rate, gw) = support::terms::hit_rates(&rig, cam);
        let gh = rate.len() / gw;
        let (sky_px, w) = changes(&mut rig, cam, true, fsr2, frames);
        let (lamp_px, _) = changes(&mut rig, cam, false, fsr2, frames);
        let h = sky_px.len() / w;
        let at = |x: usize, y: usize| rate[(y * gh / h) * gw + x * gw / w];
        let rim = |x: usize, y: usize| at(x, y).is_some_and(|r| (0.3..0.6).contains(&r));
        let far = |x: usize, y: usize| at(x, y).is_some_and(|r| r < 0.05);
        for (name, region) in [
            ("rim band", &rim as &dyn Fn(usize, usize) -> bool),
            ("far end", &far),
        ] {
            let (s, n) = p99(&sky_px, w, region);
            let (l, _) = p99(&lamp_px, w, region);
            println!(
                "reading: tunnel on {}, {name}: {n} display pixels, p99 change sky {s}, headlamp {l} ({frames} frames, fsr2 {fsr2})",
                rig.adapter
            );
        }
    }

    pub(super) mod reading {
        /// Plan 0086 Phase 25 bar 1 as a reading (the bar fails as written: the far end's p99
        /// over the headlamp's under FSR 2, the rim's over a still headlamp's 0 natively):
        /// 128 frames under FSR 2 at 1.5, and 32 at native 320 x 180. `VXE_TEST_ADAPTER`
        /// picks the adapter (unset: lavapipe).
        #[test]
        #[ignore = "reading"]
        fn the_tunnel_beside_the_lamp_lit_floor() {
            super::tunnel_beside_the_lamp(128, true);
            super::tunnel_beside_the_lamp(32, false);
        }

        /// Plan 0094 Phase 1 bar 3: the far end's p99 split by term (`tunnel_far_end_by_term`),
        /// 128 frames under FSR 2 at 1.5. `VXE_TEST_ADAPTER` picks the adapter.
        #[test]
        #[ignore = "reading"]
        fn the_tunnel_far_end_by_term() {
            super::tunnel_far_end_by_term(128);
        }
    }
}
