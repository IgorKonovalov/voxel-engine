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
        let base = Lighting {
            lamp: !sky,
            ..flythrough::default_lighting()
        };
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
        for _ in 0..frames {
            f.step(&rig.device, &rig.queue, &rig.gpu, cam, &light);
            let now = read(&f);
            for (i, (a, b)) in now.chunks(4).zip(last.chunks(4)).enumerate() {
                per_px[i].push((0..3).map(|k| a[k].abs_diff(b[k])).max().unwrap_or(0));
            }
            last = now;
        }
        (per_px, f.target().width() as usize)
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
    }
}
