//! The list of analytic lights (ADR-0044): the slot rule and the change detection against the
//! frame's API, and, as readings run by hand, the direct part each light gives a hand-built
//! closed box against its analytic term. Not the emissive light-sample mirror (`gpu_lights.rs`).

mod support;

use flythrough::cave::ROCK;
use support::bake::Rig;
use support::lights::*;
use vxe_render::{Camera, Light, Lighting};
use vxe_world::{EditOp, FIXED_ONE, Fixed3, Mode};

#[test]
fn the_slots_follow_the_lamp() {
    let Some(mut rig) = box_rig() else {
        return;
    };
    let lights: Vec<Light> = (0..9)
        .map(|i| Light::point([10.0 + 12.0 * i as f32, 32.0, 22.0], 1200.0))
        .collect();
    let lamp_on = |rig: &mut Rig, on: bool| {
        rig.frame.set_lighting(Lighting {
            lamp: on,
            ..rig.frame.lighting()
        });
    };
    rig.frame.set_lights(&lights[..8]);
    lamp_on(&mut rig, true);
    assert_eq!(
        rig.frame.lights_over_cap(),
        1,
        "eight beside the lamp: one over"
    );
    lamp_on(&mut rig, false);
    assert_eq!(
        rig.frame.lights_over_cap(),
        0,
        "eight with the lamp off: none over"
    );
    rig.frame.set_lights(&lights);
    assert_eq!(rig.frame.lights_over_cap(), 1, "a ninth with the lamp off");
    lamp_on(&mut rig, true);
    assert_eq!(rig.frame.lights_over_cap(), 2, "a ninth beside the lamp");
}

/// An unchanged list leaves the change bit clear over 32 frames; turning the lamp on sets it
/// for one frame; a list light moved sets it for that frame.
#[test]
fn the_change_bit_follows_the_list() {
    let Some(mut rig) = box_rig() else {
        return;
    };
    let camera = box_camera();
    let mut light = Light::point([64.0, 32.0, 22.0], 1200.0);
    rig.frame.set_lights(&[light]);
    step(&mut rig, &camera);
    for i in 0..32 {
        step(&mut rig, &camera);
        assert!(!rig.frame.lights_changed(), "still frame {i}");
    }
    rig.frame.set_lighting(Lighting {
        lamp: true,
        ..rig.frame.lighting()
    });
    step(&mut rig, &camera);
    assert!(rig.frame.lights_changed(), "the lamp switched on");
    step(&mut rig, &camera);
    assert!(!rig.frame.lights_changed(), "the frame after");
    light.pos[0] += 10.0;
    rig.frame.set_lights(&[light]);
    step(&mut rig, &camera);
    assert!(rig.frame.lights_changed(), "the light moved");
    step(&mut rig, &camera);
    assert!(!rig.frame.lights_changed(), "the frame after the move");
}

/// The readings: each test prints the direct part at named floor pixels against the analytic
/// term and asserts nothing a gate runs. `cargo nextest run -p flythrough --run-ignored only
/// -E 'binary(=gpu_dynamic_lights) & test(/reading::/)' --no-capture`.
mod reading {
    use super::*;

    fn alpha(rig: &Rig, tex: &vxe_render::wgpu::Texture) -> Vec<f32> {
        vxe_render::debug::read_float(&rig.device, &rig.queue, tex)
            .iter()
            .map(|p| p[3])
            .collect()
    }

    fn report(name: &str, got: f32, want: f32) {
        println!(
            "{name}: {got:.4} against {want:.4} ({:+.2} %)",
            (f64::from(got) / f64::from(want).max(1e-9) - 1.0) * 100.0
        );
    }

    /// Phase 1 done-whens 1 and 2: one light of 1200 at 20 voxels above the floor, the direct
    /// part on the first frame under it (9.40) and 20 voxels aside (3.30); then a wall across
    /// the box at x 74..76, 12 voxels high, seen from the far end: the floor it shadows (x 76
    /// to 94) reads 0, a pixel past the shadow's edge its analytic value, and with SHADOWS
    /// clear the shadowed pixel its analytic value too.
    #[test]
    #[ignore]
    fn reading_a_point_light_and_its_hard_shadow() {
        let Some(mut rig) = box_rig() else {
            return;
        };
        let camera = box_camera();
        let light = Light::point([64.0, 32.0, 22.0], 1200.0);
        let floor = rig.frame.lighting().light_floor;
        rig.frame.set_lights(&[light]);
        step(&mut rig, &camera);
        let hits = floor_hits(&rig);
        let e = alpha(&rig, rig.frame.irradiance());
        for (name, x, want) in [("under", 64.0, 9.40), ("20 aside", 84.0, 3.30)] {
            let (i, p) = nearest(&hits, x, 32.0);
            report(
                &format!("first frame, {name} at {p:?}"),
                e[i],
                light.irradiance(floor, p, UP),
            );
            report(
                &format!("first frame, {name}, the plan's figure"),
                e[i],
                want,
            );
        }
        // The wall: x 74..76, z 2..14, across the box.
        rig.world
            .apply(&EditOp::box_(
                Fixed3([74, 2, 2].map(|v: i64| v * FIXED_ONE)),
                Fixed3([76, 62, 14].map(|v: i64| v * FIXED_ONE)),
                Mode::Fill(ROCK),
            ))
            .expect("resident");
        for _ in 0..2 {
            support::SyncNow::sync_now(&mut rig.gpu, &rig.world, &rig.device, &rig.queue)
                .expect("fits");
        }
        let far = Camera {
            position: [120.0, 32.0, 45.0].into(),
            yaw: std::f32::consts::PI,
            pitch: -0.9,
            vfov: 1.2,
        };
        for flags in [Light::ON | Light::SHADOWS, Light::ON] {
            rig.frame.set_lights(&[Light { flags, ..light }]);
            rig.frame.reset_history();
            step(&mut rig, &far);
            let hits = floor_hits(&rig);
            let e = alpha(&rig, rig.frame.irradiance());
            for (name, x) in [("in the shadow", 82.0), ("past its edge", 96.0)] {
                let (i, p) = nearest(&hits, x, 32.0);
                report(
                    &format!("flags {flags:#x}, {name} at {p:?}"),
                    e[i],
                    light.irradiance(floor, p, UP),
                );
            }
        }
    }

    /// Phase 1 done-when 4: in the steps view, the shadow rays cast at a floor pixel beyond a
    /// light of 1200's reach of 109.5 (122 voxels away, the farthest floor in view) and inside
    /// it (62), at floor 0.1 and 0.025.
    #[test]
    #[ignore]
    fn reading_a_light_beyond_its_reach_casts_no_shadow_ray() {
        let Some(mut rig) = box_rig() else {
            return;
        };
        let camera = box_camera();
        let mut look = rig.frame.look();
        look.view = vxe_render::ViewMode::Steps;
        rig.frame.set_look(look);
        // The floor in view lies 62 and 122 voxels from it, inside and beyond the reach of 109.5.
        let light = Light::point([10.0, 32.0, 49.0], 1200.0);
        rig.frame.set_lights(&[light]);
        for floor in [0.1, 0.025] {
            rig.frame.set_lighting(Lighting {
                light_floor: floor,
                ..rig.frame.lighting()
            });
            step(&mut rig, &camera);
            let hits = floor_hits(&rig);
            let rays = alpha(&rig, rig.frame.irradiance());
            for target in [61.8f32, 121.7] {
                // The floor point `target` voxels from the light, along +x.
                let dz: f32 = 49.0 - FLOOR_Z;
                let dx = (target * target - dz * dz).sqrt();
                let (i, p) = nearest(&hits, 10.0 + dx, 32.0);
                let d =
                    ((p[0] - 10.0).powi(2) + (p[1] - 32.0).powi(2) + (p[2] - 49.0).powi(2)).sqrt();
                println!("floor {floor}: {d:.1} voxels away, {} shadow rays", rays[i]);
            }
        }
    }

    /// Phase 1 done-whens 6 and 11: after 32 still frames a light moved by 10 voxels gives the
    /// new analytic value on the next frame; then 16 frames of 4 voxels a frame along x, the
    /// accumulated direct part under the start and under the light each frame, and the start
    /// pixel's last value against 0.185.
    #[test]
    #[ignore]
    fn reading_a_changed_light_and_a_light_in_flight() {
        let Some(mut rig) = box_rig() else {
            return;
        };
        // Straight down over the flight's middle: the floor from x 42 to 121 in view.
        let camera = Camera {
            position: [82.0, 32.0, 49.0].into(),
            yaw: 0.0,
            pitch: -1.56,
            vfov: 1.4,
        };
        let floor = rig.frame.lighting().light_floor;
        let mut light = Light::point([40.0, 32.0, 22.0], 1200.0);
        rig.frame.set_lights(&[light]);
        for _ in 0..32 {
            step(&mut rig, &camera);
        }
        light.pos[0] += 10.0;
        rig.frame.set_lights(&[light]);
        step(&mut rig, &camera);
        let hits = floor_hits(&rig);
        let acc = alpha(&rig, rig.frame.accumulated());
        let (i, p) = nearest(&hits, 50.0, 32.0);
        report(
            "moved by 10, next frame",
            acc[i],
            light.irradiance(floor, p, UP),
        );
        for _ in 0..32 {
            step(&mut rig, &camera);
        }
        let start = light.pos;
        let mut worst = 0.0f64;
        for f in 0..16 {
            light.pos[0] += 4.0;
            rig.frame.set_lights(&[light]);
            step(&mut rig, &camera);
            let hits = floor_hits(&rig);
            let acc = alpha(&rig, rig.frame.accumulated());
            for (x, label) in [(start[0], "start"), (light.pos[0], "under")] {
                let (i, p) = nearest(&hits, x, 32.0);
                let want = light.irradiance(floor, p, UP);
                let err = (f64::from(acc[i]) / f64::from(want).max(1e-9) - 1.0).abs();
                worst = worst.max(err);
                if f == 15 {
                    report(
                        &format!("flight's last frame, {label} at {p:?}"),
                        acc[i],
                        want,
                    );
                }
            }
            assert!(rig.frame.lights_changed(), "frame {f} of the flight");
        }
        println!(
            "flight: the worst error over 16 frames {:.3} %",
            worst * 100.0
        );
        step(&mut rig, &camera);
        step(&mut rig, &camera);
        println!(
            "the second frame after: changed {}",
            rig.frame.lights_changed()
        );
    }

    /// Phase 1 done-when 13: the lamp off, one list light, 32 still frames at the half tier;
    /// the accumulated and filtered direct part under the light against 9.40.
    #[test]
    #[ignore]
    fn reading_a_list_light_survives_the_lamp_off_skips() {
        let Some(mut rig) = box_rig() else {
            return;
        };
        rig.frame
            .set_resolution(&rig.device, vxe_render::LightingResolution::Half);
        let camera = box_camera();
        let floor = rig.frame.lighting().light_floor;
        let light = Light::point([64.0, 32.0, 22.0], 1200.0);
        rig.frame.set_lights(&[light]);
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        let (mut flo, mut fhi) = (f32::MAX, f32::MIN);
        for _ in 0..32 {
            step(&mut rig, &camera);
            // The lighting grid is half the frame's: its hit buffer is the light pass's.
            let pos = vxe_render::debug::read_rgba32f(
                &rig.device,
                &rig.queue,
                rig.frame.light_hit_positions(),
            );
            let acc = alpha(&rig, rig.frame.accumulated());
            let filt: Vec<f32> =
                vxe_render::debug::read_float(&rig.device, &rig.queue, rig.frame.filtered())
                    .iter()
                    .map(|p| p[0])
                    .collect();
            let i = (0..pos.len())
                .filter(|&i| pos[i][3] > 0.0 && (pos[i][2] - FLOOR_Z).abs() < 1e-3)
                .min_by(|&a, &b| {
                    let d = |p: [f32; 4]| (p[0] - 64.0).powi(2) + (p[1] - 32.0).powi(2);
                    d(pos[a]).total_cmp(&d(pos[b]))
                })
                .expect("the floor is in view");
            let want = light.irradiance(floor, [pos[i][0], pos[i][1], pos[i][2]], UP);
            lo = lo.min(acc[i] / want);
            hi = hi.max(acc[i] / want);
            flo = flo.min(filt[i] / want);
            fhi = fhi.max(filt[i] / want);
        }
        println!(
            "half tier, 32 frames: accumulated / analytic in [{lo:.4}, {hi:.4}], filtered in [{flo:.4}, {fhi:.4}]"
        );
    }

    /// Phase 1 done-when 8: the gradient mode with two still list lights and the lamp off; the
    /// strata's lambda over 16 still frames after the first (which has no history): the largest
    /// value, the strata above 0 and above 1e-3, and the strata that re-shaded a lit pixel.
    #[test]
    #[ignore]
    fn reading_the_gradient_mode_holds_still_under_a_still_list() {
        let Some(mut rig) = box_rig() else {
            return;
        };
        rig.frame.set_lighting(Lighting {
            history: vxe_render::HistoryKill::Gradient,
            ..rig.frame.lighting()
        });
        let camera = box_camera();
        rig.frame.set_lights(&[
            Light::point([54.0, 32.0, 22.0], 1200.0),
            Light::point([84.0, 20.0, 22.0], 1200.0),
        ]);
        step(&mut rig, &camera);
        let (mut worst, mut over0, mut over3, mut lit, mut n) = (0.0f32, 0usize, 0usize, 0usize, 0);
        for _ in 0..16 {
            step(&mut rig, &camera);
            assert!(!rig.frame.lights_changed());
            let strata = vxe_render::debug::read_rgba16f(
                &rig.device,
                &rig.queue,
                rig.frame.gradient_strata(),
            );
            for s in &strata {
                n += 1;
                worst = worst.max(s[0]);
                over0 += usize::from(s[0] > 0.0);
                over3 += usize::from(s[0] > 1e-3);
                lit += usize::from(s[1] > 0.0);
            }
        }
        println!(
            "gradient, two still lights, 16 frames: {n} strata-frames, {lit} re-shaded a lit pixel; lambda max {worst:.6}, above 0 {over0}, above 1e-3 {over3}"
        );
    }

    /// Phase 1's hall reading: the showcase's `hall` shot at the fly-through's defaults (FSR 2
    /// at 1.5 with sharpening 0.5, or native: no upscaler, no jitter), the lamp off, exposure 8,
    /// 320 x 180, with one white light of 1200 two voxels over the floor where the centre ray
    /// meets it, and with none: the frame after 64 frames against the one after 512 (pixels
    /// over 2/255 in any channel, the largest step). Runs on `VXE_TEST_ADAPTER`.
    #[test]
    #[ignore]
    fn reading_the_hall_at_64_against_512_frames() {
        use flythrough::showcase::{SHOTS, ShowcaseSource};
        use vxe_render::{GpuWorld, MirrorConfig, Window};
        let table = flythrough::showcase_materials();
        let source = ShowcaseSource::new(1);
        let world = vxe_world::World::generate(source.extent(), &source, &table);
        let pool = flythrough::level_pool_units(&world, false) * vxe_render::internals::UNIT_BYTES;
        let Some((device, queue, adapter)) = support::reading_device(pool) else {
            return;
        };
        let chunks = world.extent().len() as u64;
        let cfg = MirrorConfig {
            budget_bytes: flythrough::level_mirror_budget(&world, chunks, false),
            light_radius: 19,
            ..MirrorConfig::new(Window::Fixed(world.extent()))
        };
        let mut gpu = GpuWorld::with_config(&device, cfg);
        let shot = SHOTS[0];
        gpu.set_focus(shot.eye);
        for _ in 0..64 {
            support::SyncNow::sync_now(&mut gpu, &world, &device, &queue).expect("fits");
        }
        let dir = shot.ray_milli.map(|c| i64::from(c) * FIXED_ONE / 1000);
        let eye = Fixed3(shot.eye.0.map(|v| i64::from(v) * FIXED_ONE + FIXED_ONE / 2));
        let hit = world
            .raycast(eye, dir, 600 * FIXED_ONE)
            .expect("in range")
            .expect("the centre ray meets the floor");
        let v = hit.voxel.0;
        let light = Light::point(
            [v[0] as f32 + 0.5, v[1] as f32 + 0.5, v[2] as f32 + 3.0],
            1200.0,
        );
        let camera = Camera {
            position: shot.eye.0.map(|v| v as f32 + 0.5).into(),
            yaw: (shot.yaw_deg as f32).to_radians(),
            pitch: (shot.pitch_deg as f32).to_radians(),
            vfov: 70f32.to_radians(),
        };
        let output = [320u32, 180];
        for fsr in [false, true] {
            for lights in [&[light][..], &[]] {
                let mut frame = if fsr {
                    let mut f = vxe_render::Frame::with_sizes(
                        &device,
                        vxe_render::RenderScale::OneHalf.internal(output),
                        output,
                    );
                    f.set_upscaler_kind(
                        &device,
                        vxe_render::UpscalerKind::Fsr2(vxe_render::Fsr2Settings {
                            sharpness: 0.5,
                            ..vxe_render::Fsr2Settings::default()
                        }),
                    );
                    f
                } else {
                    let mut f = vxe_render::Frame::with_sizes(&device, output, output);
                    f.set_jitter(&device, false);
                    f
                };
                frame.set_lighting(Lighting {
                    lamp: false,
                    exposure: vxe_render::Exposure::Manual(8.0),
                    store_read: flythrough::DEFAULT_STORE_READ,
                    light_weights: flythrough::DEFAULT_LIGHT_WEIGHTS,
                    ..Lighting::default()
                });
                frame.set_lights(lights);
                let mut at64 = Vec::new();
                for f in 1..=512 {
                    frame.step(
                        &device,
                        &queue,
                        &gpu,
                        &camera,
                        &vxe_render::LightParams::headlamp(),
                    );
                    if f == 64 {
                        at64 = vxe_render::debug::read_rgba8(&device, &queue, frame.target());
                    }
                }
                let at512 = vxe_render::debug::read_rgba8(&device, &queue, frame.target());
                let (mut over, mut worst) = (0usize, 0u8);
                for (a, b) in at64.chunks(4).zip(at512.chunks(4)) {
                    let d = (0..3).map(|k| a[k].abs_diff(b[k])).max().unwrap_or(0);
                    over += usize::from(d > 2);
                    worst = worst.max(d);
                }
                println!(
                    "hall on {adapter}, {}, {} light(s) (at {:?}): 64 against 512 frames, {:.3} % of pixels over 2/255, largest {worst}/255",
                    if fsr { "fsr2 at 1.5" } else { "native" },
                    lights.len(),
                    light.pos,
                    over as f64 * 100.0 / (output[0] * output[1]) as f64
                );
            }
        }
    }
}
