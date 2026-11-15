//! The list of analytic lights (ADR-0044): the slot rule and the change detection against the
//! frame's API, and, as readings run by hand, the direct part each light gives a hand-built
//! closed box against its analytic term. Not the emissive light-sample mirror (`gpu_lights.rs`).

mod support;

use flythrough::cave::ROCK;
use support::bake::Rig;
use support::lights::*;
use vxe_render::{Camera, Light, Lighting};
use vxe_world::{EditOp, FIXED_ONE, Fixed3, Mode};

/// The slot rule at sixteen (owner, 2026-11-14): with the lamp on, sixteen given keep fifteen
/// and one is over; with it off all sixteen are kept; a seventeenth is over in both.
#[test]
fn the_slots_follow_the_lamp() {
    let Some(mut rig) = box_rig() else {
        return;
    };
    let lights: Vec<Light> = (0..17)
        .map(|i| Light::point([6.0 + 7.0 * i as f32, 32.0, 22.0], 600.0))
        .collect();
    let lamp_on = |rig: &mut Rig, on: bool| {
        rig.frame.set_lighting(Lighting {
            lamp: on,
            ..rig.frame.lighting()
        });
    };
    rig.frame.set_lights(&lights[..16]);
    lamp_on(&mut rig, true);
    assert_eq!(
        rig.frame.lights_over_cap(),
        1,
        "sixteen beside the lamp: one over"
    );
    lamp_on(&mut rig, false);
    assert_eq!(
        rig.frame.lights_over_cap(),
        0,
        "sixteen with the lamp off: none over"
    );
    rig.frame.set_lights(&lights);
    assert_eq!(
        rig.frame.lights_over_cap(),
        1,
        "a seventeenth with the lamp off"
    );
    lamp_on(&mut rig, true);
    assert_eq!(
        rig.frame.lights_over_cap(),
        2,
        "a seventeenth beside the lamp"
    );
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

/// The a-trous iterations follow `Lighting::atrous_iterations`: a frame at 1, 2 and 4 records
/// that many a-trous dispatches (0 with the filter off), and the composite on the box with one
/// light at 1 and at 2 differs from the frame at 4 (the composite reads the last iteration's
/// target).
#[test]
fn the_atrous_count_follows_the_setting() {
    let Some(mut rig) = box_rig() else {
        return;
    };
    let camera = box_camera();
    rig.frame
        .set_lights(&[Light::point([64.0, 32.0, 22.0], 1200.0)]);
    let mut images = Vec::new();
    for n in [4u32, 2, 1] {
        rig.frame.set_lighting(Lighting {
            atrous_iterations: n,
            ..rig.frame.lighting()
        });
        rig.frame.reset_history();
        for _ in 0..4 {
            step(&mut rig, &camera);
        }
        assert_eq!(rig.frame.atrous_dispatches(), n, "at {n}");
        images.push(vxe_render::debug::read_rgba8(
            &rig.device,
            &rig.queue,
            rig.frame.target(),
        ));
    }
    assert_ne!(images[1], images[0], "2 against 4");
    assert_ne!(images[2], images[0], "1 against 4");
    rig.frame.set_lighting(Lighting {
        filter: false,
        ..rig.frame.lighting()
    });
    step(&mut rig, &camera);
    assert_eq!(rig.frame.atrous_dispatches(), 0, "the filter off");
}

/// Whether the CPU's exact walk (`World::raycast`) finds rock on the segment from `o` to `end`,
/// and whether that rock holds `o` itself.
fn cpu_blocked(world: &vxe_world::World, o: [f32; 3], end: [f32; 3]) -> (bool, bool) {
    let origin = Fixed3(o.map(|v| (f64::from(v) * FIXED_ONE as f64).round() as i64));
    let dir: [i64; 3] =
        std::array::from_fn(|k| (f64::from(end[k] - o[k]) * FIXED_ONE as f64).round() as i64);
    let hit = world.raycast(origin, dir, 256).expect("in range");
    (hit.is_some(), hit.is_some_and(|h| h.face.is_none()))
}

/// A staircase on the light box's far wall (step k over x 110 + 2k..126, y 40..62, z 2..6 +
/// 4k), synced to the rig's mirror.
fn add_steps(rig: &mut Rig) {
    for k in 0..8i64 {
        rig.world
            .apply(&EditOp::box_(
                Fixed3([110 + 2 * k, 40, 2].map(|v| v * FIXED_ONE)),
                Fixed3([126, 62, 6 + 4 * k].map(|v| v * FIXED_ONE)),
                Mode::Fill(ROCK),
            ))
            .expect("resident");
    }
    for _ in 0..2 {
        support::SyncNow::sync_now(&mut rig.gpu, &rig.world, &rig.device, &rig.queue)
            .expect("fits");
    }
}

/// A light 2 voxels off a surface (the fly-through's `SURFACE_GAP`) shadows as the exact ray
/// does (Plan 0058 Phase 5c): a white light of 600 two voxels off a riser of a staircase, the
/// default (coarse) shadow walk; on the first frame every hit within 24 voxels facing the
/// light is lit in the light pass exactly when the CPU's exact walk from its shadow origin
/// (half a voxel off the face) to the light finds no rock. Left out: a hit whose CPU walk
/// starts inside rock, and a grazing one (its CPU verdict flips when the light moves 0.02
/// voxel along an axis).
#[test]
fn a_light_beside_steps_shadows_as_the_exact_ray_does() {
    let Some(mut rig) = box_rig() else {
        return;
    };
    add_steps(&mut rig);
    let camera = Camera {
        position: [96.0, 46.0, 30.0].into(),
        yaw: 0.6,
        pitch: -0.5,
        vfov: 1.2,
    };
    let at = [116.0, 51.0, 20.0];
    rig.frame.set_lights(&[Light::point(at, 600.0)]);
    step(&mut rig, &camera);
    let direct = vxe_render::debug::read_float(&rig.device, &rig.queue, rig.frame.direct());
    let info = vxe_render::debug::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
    let eye = camera.position.to_array();
    let (mut checked, mut wrong) = (0, Vec::new());
    for (i, p, _) in hits_of(&rig, &rig.frame) {
        // The entry face's normal: the hit record's axis (flags bits 3..4, minus one) against
        // the view ray, as `shadow_normal_of`.
        let axis = i32::from((info[i][3] >> 3) & 3) - 1;
        if axis < 0 || dist(p, at) > 24.0 {
            continue;
        }
        let mut n = [0.0f32; 3];
        n[axis as usize] = -(p[axis as usize] - eye[axis as usize]).signum();
        if (0..3).map(|k| n[k] * (at[k] - p[k])).sum::<f32>() <= 0.0 {
            continue;
        }
        let o: [f32; 3] = std::array::from_fn(|k| p[k] + n[k] * 0.5);
        let (dark, inside) = cpu_blocked(&rig.world, o, at);
        let grazing = (0..6).any(|j| {
            let mut a = at;
            a[j / 2] += if j % 2 == 0 { 0.02 } else { -0.02 };
            cpu_blocked(&rig.world, o, a).0 != dark
        });
        if inside || grazing {
            continue;
        }
        checked += 1;
        if (direct[i][3] == 0.0) != dark {
            wrong.push((p, dark));
        }
    }
    println!(
        "{checked} facing hits checked, {} against the CPU",
        wrong.len()
    );
    assert!(checked > 10_000, "{checked} hits");
    assert!(
        wrong.is_empty(),
        "{} hits disagree with the exact ray, first {:?} (CPU dark: the second)",
        wrong.len(),
        wrong.first()
    );
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
        let e = alpha(&rig, rig.frame.direct());
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
            let e = alpha(&rig, rig.frame.direct());
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
        let acc = alpha(&rig, rig.frame.direct_accumulated());
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
            let acc = alpha(&rig, rig.frame.direct_accumulated());
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
            let acc = alpha(&rig, rig.frame.direct_accumulated());
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

    /// The direct part's rgb at the floor pixel nearest `(x, y)` on the last frame of `tex`,
    /// and that pixel's hit.
    fn rgb_at(rig: &Rig, tex: &vxe_render::wgpu::Texture, x: f32, y: f32) -> ([f32; 3], [f32; 3]) {
        let hits = floor_hits(rig);
        let (i, p) = nearest(&hits, x, y);
        let v = vxe_render::debug::read_float(&rig.device, &rig.queue, tex)[i];
        ([v[0], v[1], v[2]], p)
    }

    /// Phase 2 done-when 1: a red and a blue light of 1200 at 20 voxels above the floor and 20
    /// apart; the direct part per channel on the first frame under each, against each light's
    /// analytic term in its channel ((9.40, 0, 3.30) and (3.30, 0, 9.40) at 0 and 20 voxels aside).
    #[test]
    #[ignore]
    fn reading_two_coloured_lights_sum_per_channel() {
        let Some(mut rig) = box_rig() else {
            return;
        };
        let camera = box_camera();
        let floor = rig.frame.lighting().light_floor;
        let red = Light {
            colour: [1.0, 0.0, 0.0],
            ..Light::point([54.0, 32.0, 22.0], 1200.0)
        };
        let blue = Light {
            colour: [0.0, 0.0, 1.0],
            ..Light::point([74.0, 32.0, 22.0], 1200.0)
        };
        rig.frame.set_lights(&[red, blue]);
        step(&mut rig, &camera);
        for (name, x) in [("under red", 54.0), ("under blue", 74.0)] {
            let (got, p) = rgb_at(&rig, rig.frame.direct(), x, 32.0);
            let want = [
                red.irradiance(floor, p, UP),
                0.0,
                blue.irradiance(floor, p, UP),
            ];
            println!("{name} at {p:?}: direct {got:.4?} against {want:.4?}");
        }
    }

    /// Phase 2 done-when 3: one still light, the camera stepping sideways 0.5 voxel a frame
    /// for 32 frames; the accumulated direct luminance at the floor pixel nearest (64, 32)
    /// against the analytic term at its hit, the worst error over the frames.
    #[test]
    #[ignore]
    fn reading_a_dropped_light_holds_still_while_the_camera_moves() {
        let Some(mut rig) = box_rig() else {
            return;
        };
        let mut camera = box_camera();
        let floor = rig.frame.lighting().light_floor;
        let light = Light::point([64.0, 32.0, 22.0], 1200.0);
        rig.frame.set_lights(&[light]);
        for _ in 0..8 {
            step(&mut rig, &camera);
        }
        let mut worst = 0.0f64;
        for _ in 0..32 {
            camera.position.y += 0.5;
            step(&mut rig, &camera);
            let hits = floor_hits(&rig);
            let acc = alpha(&rig, rig.frame.direct_accumulated());
            for x in [56.0, 64.0, 72.0] {
                let (i, p) = nearest(&hits, x, 32.0);
                let want = light.irradiance(floor, p, UP);
                worst = worst.max((f64::from(acc[i]) / f64::from(want) - 1.0).abs());
            }
        }
        println!(
            "camera stepping 0.5 voxel a frame, 32 frames: worst error {:.3} % at three floor pixels",
            worst * 100.0
        );
    }

    /// Phase 2 done-when 8: done-when 11's flight with the light red; per frame the
    /// accumulated direct rgb under the light against `(E, 0, 0)`, the worst red error and the
    /// largest green or blue; the start pixel on the last frame against (0.185, 0, 0).
    #[test]
    #[ignore]
    fn reading_a_coloured_light_in_flight_lights_its_channel_only() {
        let Some(mut rig) = box_rig() else {
            return;
        };
        let camera = Camera {
            position: [82.0, 32.0, 49.0].into(),
            yaw: 0.0,
            pitch: -1.56,
            vfov: 1.4,
        };
        let floor = rig.frame.lighting().light_floor;
        let mut light = Light {
            colour: [1.0, 0.0, 0.0],
            ..Light::point([40.0, 32.0, 22.0], 1200.0)
        };
        rig.frame.set_lights(&[light]);
        for _ in 0..32 {
            step(&mut rig, &camera);
        }
        let start = light.pos;
        let (mut worst, mut other) = (0.0f64, 0.0f32);
        let mut last = ([0.0; 3], [0.0; 3]);
        for _ in 0..16 {
            light.pos[0] += 4.0;
            rig.frame.set_lights(&[light]);
            step(&mut rig, &camera);
            let (got, p) = rgb_at(&rig, rig.frame.direct_accumulated(), light.pos[0], 32.0);
            let want = light.irradiance(floor, p, UP);
            worst = worst.max((f64::from(got[0]) / f64::from(want) - 1.0).abs());
            other = other.max(got[1].abs()).max(got[2].abs());
            last = rgb_at(&rig, rig.frame.direct_accumulated(), start[0], 32.0);
        }
        println!(
            "red flight: worst red error under the light {:.3} %, largest green or blue {other}; start pixel last frame {:.4?} at {:?} against ({:.4}, 0, 0)",
            worst * 100.0,
            last.0,
            last.1,
            light.irradiance(floor, last.1, UP)
        );
    }

    /// Whether the CPU's exact walk finds rock between `o` and `light` (the world's raycast),
    /// and whether that rock holds `o` itself (the walk starts inside it).
    fn cpu_shadowed(world: &vxe_world::World, o: [f32; 3], light: [f32; 3]) -> (bool, bool) {
        let origin = Fixed3(o.map(|v| (f64::from(v) * FIXED_ONE as f64).round() as i64));
        let dir: [i64; 3] =
            std::array::from_fn(|k| (f64::from(light[k] - o[k]) * FIXED_ONE as f64).round() as i64);
        let hit = world.raycast(origin, dir, 256).expect("in range");
        (hit.is_some(), hit.is_some_and(|h| h.face.is_none()))
    }

    /// Phase 5c: near a white light of 600 at 2 and 4 voxels off a pillar's face and off a
    /// staircase's riser in the light box, auto exposure, native 192 x 108 at full lighting
    /// resolution, the coarse and the full shadow walk: the pre-exposed direct peak against
    /// HDR_MAX (60000) and f16's 65504 and the share of texels at either; the display's
    /// per-pixel spread over 64 still frames within 8 voxels of the light; and each facing hit
    /// within 24 voxels of the light, lit or not on the first frame's direct part, against the
    /// CPU's exact ray from the same shadow origin.
    #[test]
    #[ignore]
    #[allow(clippy::too_many_lines, reason = "a reading's arms in one place")]
    fn reading_near_a_light_clamps_spread_and_shadows() {
        use vxe_render::{AutoExposure, Exposure, ShadowWalk, debug::read_rgba8};
        let Some(mut rig) = box_rig() else {
            return;
        };
        // The pillar: x 90..96, y 29..35, floor to ceiling; the staircase on the far wall:
        // step k over x 110 + 2k..126, z 2..6 + 4k.
        rig.world
            .apply(&EditOp::box_(
                Fixed3([90, 29, 2].map(|v: i64| v * FIXED_ONE)),
                Fixed3([96, 35, 50].map(|v: i64| v * FIXED_ONE)),
                Mode::Fill(ROCK),
            ))
            .expect("resident");
        for k in 0..8i64 {
            rig.world
                .apply(&EditOp::box_(
                    Fixed3([110 + 2 * k, 40, 2].map(|v| v * FIXED_ONE)),
                    Fixed3([126, 62, 6 + 4 * k].map(|v| v * FIXED_ONE)),
                    Mode::Fill(ROCK),
                ))
                .expect("resident");
        }
        for _ in 0..2 {
            support::SyncNow::sync_now(&mut rig.gpu, &rig.world, &rig.device, &rig.queue)
                .expect("fits");
        }
        // (scene, the surface point the light faces, its outward normal, the camera)
        let pillar_cam = Camera {
            position: [74.0, 32.0, 26.0].into(),
            yaw: 0.0,
            pitch: 0.0,
            vfov: 1.2,
        };
        let steps_cam = Camera {
            position: [96.0, 46.0, 30.0].into(),
            yaw: 0.6,
            pitch: -0.5,
            vfov: 1.2,
        };
        let scenes = [
            ("pillar", [90.0, 32.0, 26.0], [-1.0, 0.0, 0.0], pillar_cam),
            ("steps", [118.0, 51.0, 20.0], [-1.0, 0.0, 0.0], steps_cam),
        ];
        for (scene, surface, n, camera) in scenes {
            for gap in [2.0f32, 4.0] {
                let at: [f32; 3] = std::array::from_fn(|k| surface[k] + n[k] * gap);
                for walk in [ShadowWalk::Coarse, ShadowWalk::Full] {
                    rig.frame.set_lighting(Lighting {
                        exposure: Exposure::Auto(AutoExposure::default()),
                        shadow_walk: walk,
                        ..box_lighting()
                    });
                    rig.frame.reset_history();
                    rig.frame.reset_exposure();
                    rig.frame.set_lights(&[Light::point(at, 600.0)]);
                    step(&mut rig, &camera);
                    // The first frame's direct part: lit or not per pixel.
                    let direct =
                        vxe_render::debug::read_float(&rig.device, &rig.queue, rig.frame.direct());
                    let hits = hits_of(&rig, &rig.frame);
                    let info = vxe_render::debug::read_rgba16u(
                        &rig.device,
                        &rig.queue,
                        rig.frame.hit_records(),
                    );
                    let (mut facing, mut agree, mut gpu_dark_cpu_lit, mut gpu_lit_cpu_dark) =
                        (0, 0, 0, 0);
                    let (mut inside, mut grazing, mut shifted) = (0, 0, 0);
                    for &(i, p, _) in &hits {
                        if dist(p, at) > 24.0 {
                            continue;
                        }
                        // The entry face's normal: the hit record's axis (flags bits 3..4, minus
                        // one), against the view ray (`shadow_normal_of`).
                        let axis = i32::from((info[i][3] >> 3) & 3) - 1;
                        if axis < 0 {
                            continue;
                        }
                        let axis = axis as usize;
                        let mut nn = [0.0f32; 3];
                        nn[axis] = -(p[axis] - camera.position.to_array()[axis]).signum();
                        let l: [f32; 3] = std::array::from_fn(|k| at[k] - p[k]);
                        if (0..3).map(|k| nn[k] * l[k]).sum::<f32>() <= 0.0 {
                            continue;
                        }
                        facing += 1;
                        let o: [f32; 3] = std::array::from_fn(|k| p[k] + nn[k] * 0.5);
                        let (cpu_dark, starts_inside) = cpu_shadowed(&rig.world, o, at);
                        let gpu_dark = direct[i][3] == 0.0;
                        match (gpu_dark, cpu_dark) {
                            (a, b) if a == b => agree += 1,
                            (true, false) => gpu_dark_cpu_lit += 1,
                            _ => {
                                gpu_lit_cpu_dark += 1;
                                inside += usize::from(starts_inside);
                                // The light pass's own geometry: from the shadow origin along
                                // the direction from the hit, for the hit's distance, so it
                                // ends half a voxel off the light along the normal.
                                let end: [f32; 3] = std::array::from_fn(|k| at[k] + nn[k] * 0.5);
                                shifted += usize::from(!cpu_shadowed(&rig.world, o, end).0);
                                // Grazing: the CPU's ray clears the rock when its light end
                                // moves 0.02 voxel along any axis.
                                grazing += usize::from((0..6).any(|j| {
                                    let mut a = at;
                                    a[j / 2] += if j % 2 == 0 { 0.02 } else { -0.02 };
                                    !cpu_shadowed(&rig.world, o, a).0
                                }));
                            }
                        }
                    }
                    // Settle the history and the exposure before the still frames.
                    for _ in 0..128 {
                        step(&mut rig, &camera);
                    }
                    let mut frames = Vec::new();
                    let mut dark_before: Vec<bool> = direct.iter().map(|p| p[3] == 0.0).collect();
                    let (mut flips, mut flipping) = (0usize, vec![false; dark_before.len()]);
                    for _ in 0..64 {
                        step(&mut rig, &camera);
                        frames.push(read_rgba8(&rig.device, &rig.queue, rig.frame.target()));
                        let d = vxe_render::debug::read_float(
                            &rig.device,
                            &rig.queue,
                            rig.frame.direct(),
                        );
                        for (i, p) in d.iter().enumerate() {
                            let dark = p[3] == 0.0;
                            if dark != dark_before[i] {
                                flips += 1;
                                flipping[i] = true;
                            }
                            dark_before[i] = dark;
                        }
                    }
                    println!(
                        "  {scene} gap {gap} walk {walk:?}: the light pass's lit/dark flips over 64 still frames {flips}, at {} texels; GPU-lit CPU-dark hits whose CPU walk starts inside rock {inside}, that clear 0.02 voxel off {grazing}, clear on the light pass's shifted ray {shifted}",
                        flipping.iter().filter(|&&f| f).count()
                    );
                    let direct =
                        vxe_render::debug::read_float(&rig.device, &rig.queue, rig.frame.direct());
                    let peak = direct
                        .iter()
                        .map(|p| p[0].max(p[1]).max(p[2]))
                        .fold(0.0f32, f32::max);
                    let at_max = direct
                        .iter()
                        .filter(|p| p[0] >= 60000.0 || p[1] >= 60000.0 || p[2] >= 60000.0)
                        .count();
                    let at_inf = direct
                        .iter()
                        .filter(|p| !p[0].is_finite() || p[0] >= 65504.0)
                        .count();
                    let ex = rig
                        .frame
                        .exposure_readout(&rig.device)
                        .map_or(f32::NAN, |r| r.exposure);
                    let hits = hits_of(&rig, &rig.frame);
                    let (mut near, mut spread_sum, mut spread_max, mut grainy) =
                        (0usize, 0u32, 0u8, 0usize);
                    for &(i, p, _) in &hits {
                        if dist(p, at) > 8.0 {
                            continue;
                        }
                        near += 1;
                        let mut s = 0u8;
                        for k in 0..3 {
                            let (lo, hi) = frames.iter().fold((255u8, 0u8), |(lo, hi), f| {
                                (lo.min(f[4 * i + k]), hi.max(f[4 * i + k]))
                            });
                            s = s.max(hi - lo);
                        }
                        spread_sum += u32::from(s);
                        spread_max = spread_max.max(s);
                        grainy += usize::from(s > 2);
                    }
                    let saturated = frames[63]
                        .chunks(4)
                        .filter(|p| p[0] == 255 && p[1] == 255 && p[2] == 255)
                        .count();
                    println!(
                        "{scene} gap {gap} walk {walk:?}: exposure {ex:.3}; direct peak {peak:.1}, {at_max} texels at HDR_MAX, {at_inf} at f16's max; {saturated} display pixels white; within 8 voxels {near} pixels, spread over 64 frames mean {:.2} max {spread_max}, {grainy} past 2; shadows within 24 voxels: {facing} facing, {agree} agree with the CPU, {gpu_dark_cpu_lit} dark on the GPU and lit on the CPU, {gpu_lit_cpu_dark} lit on the GPU and dark on the CPU",
                        spread_sum as f64 / near.max(1) as f64
                    );
                }
            }
        }
    }

    /// Phases 5d and 5e on the level at the vein pose (seed 1, the ore emissive, baked around
    /// the camera), lamp off, the pose's manual exposure, the lamp's bounce on, 192 x 108 at
    /// the default (half) lighting resolution, no upscaler; one list light of 600 dropped 2
    /// voxels short of the surface the camera looks at. 5d: 240 frames with `BOUNCE` clear and
    /// set; per lighting texel, the gathered indirect (times the exposure) against the filtered
    /// direct part, its share where the indirect is lit, and at texels the light does not see
    /// (direct 0) within its bounce reach; the composite's bytes there. 5e: 64 frames with the
    /// ore's scale pulsing against steady, the texels of each stage that differ on the last
    /// frame, and of those how many the ore's baked light reaches.
    #[test]
    #[ignore]
    #[allow(clippy::too_many_lines, reason = "a reading's arms in one place")]
    fn reading_the_vein_bounce_and_pulse() {
        use support::stillness::{VEIN_POSE, vein_level};
        use vxe_render::debug::{read_float, read_rgba8};
        use vxe_render::{Exposure, Frame};
        let Some(mut level) = vein_level() else {
            return;
        };
        let camera = VEIN_POSE.camera();
        let (fwd, _, _) = camera.basis();
        let eye = camera.position.to_array();
        let f = fwd.to_array();
        let dir: [i64; 3] = f.map(|v| (v * FIXED_ONE as f32).round() as i64);
        let origin = Fixed3(eye.map(|v| (f64::from(v) * FIXED_ONE as f64).round() as i64));
        let hit = level
            .world
            .raycast(origin, dir, 256 * 200)
            .expect("in range")
            .expect("the camera looks at rock");
        let t = hit.t as f32 / 256.0;
        let at: [f32; 3] = std::array::from_fn(|k| eye[k] + f[k] * (t - 2.0));
        println!("the light at {at:?}, {t:.1} voxels ahead");
        let make = |level: &mut support::stillness::VeinLevel| {
            let mut frame = Frame::new(&level.device, 192, 108);
            frame.set_lighting(Lighting {
                lamp: false,
                lamp_bounce: true,
                exposure: Exposure::Manual(VEIN_POSE.exposure),
                ..Lighting::default()
            });
            frame.set_dt(1.0 / 60.0);
            support::bake_around(
                &level.device,
                &level.queue,
                &mut level.gpu,
                &mut frame,
                &level.world,
                eye.map(|c| c.floor() as i32),
                1,
                64,
            );
            frame
        };
        let light = |bounce: bool| {
            let l = Light::point(at, 600.0);
            Light {
                flags: l.flags | if bounce { Light::BOUNCE } else { 0 },
                ..l
            }
        };
        let rd = |level: &support::stillness::VeinLevel, t: &vxe_render::wgpu::Texture| {
            read_float(&level.device, &level.queue, t)
        };
        // 5d
        let mut bytes = Vec::new();
        for bounce in [false, true] {
            let mut frame = make(&mut level);
            frame.set_lights(&[light(bounce)]);
            for _ in 0..240 {
                frame.step(
                    &level.device,
                    &level.queue,
                    &level.gpu,
                    &camera,
                    &vxe_render::LightParams::headlamp(),
                );
            }
            let ex = VEIN_POSE.exposure;
            let direct = rd(&level, frame.filtered());
            let pos = rd(&level, frame.light_hit_positions());
            let ind = if bounce {
                rd(&level, frame.indirect_irradiance())
            } else {
                vec![[0.0; 4]; direct.len()]
            };
            let (mut lit, mut share_sum, mut unseen, mut unseen_ind) =
                (0usize, 0.0f64, 0usize, 0.0f64);
            for i in 0..direct.len() {
                if !(pos[i][3] > 0.0 && pos[i][3] < 1e30) {
                    continue;
                }
                let p = [pos[i][0], pos[i][1], pos[i][2]];
                let d = direct[i][0] + direct[i][1] + direct[i][2];
                let g = (ind[i][0] + ind[i][1] + ind[i][2]) * ex;
                if g > 0.0 {
                    lit += 1;
                    share_sum += f64::from(g / (g + d));
                }
                if d == 0.0 && dist(p, at) <= 64.0 {
                    unseen += 1;
                    unseen_ind += f64::from(g);
                }
            }
            println!(
                "5d BOUNCE {bounce}: probe dispatches {}; {lit} texels with indirect, its mean share of direct + indirect {:.3}; {unseen} texels within 64 voxels the light does not see, their mean indirect (pre-exposed, rgb sum) {:.4}",
                frame.probe_dispatches(),
                share_sum / lit.max(1) as f64,
                unseen_ind / unseen.max(1) as f64
            );
            bytes.push(read_rgba8(&level.device, &level.queue, frame.target()));
        }
        let diff = bytes[0]
            .chunks(4)
            .zip(bytes[1].chunks(4))
            .filter(|(a, b)| a[..3] != b[..3])
            .count();
        let mean_up: f64 = bytes[0]
            .chunks(4)
            .zip(bytes[1].chunks(4))
            .map(|(a, b)| {
                (0..3)
                    .map(|k| f64::from(b[k]) - f64::from(a[k]))
                    .sum::<f64>()
            })
            .sum::<f64>()
            / (bytes[0].len() / 4) as f64;
        println!(
            "5d composite: {diff} pixels differ flagged against unflagged, mean rgb-sum change {mean_up:+.2} /255"
        );
        // 5e
        let ores: Vec<u16> = level
            .world
            .materials()
            .rows()
            .iter()
            .enumerate()
            .filter(|(_, r)| r.emission != [0.0; 3])
            .map(|(m, _)| m as u16)
            .collect();
        let mut stages = Vec::new();
        for pulse in [false, true] {
            let mut frame = make(&mut level);
            frame.set_lights(&[light(false)]);
            for i in 0..64 {
                let s = if pulse {
                    0.5 + 0.5 * (i as f32).sin()
                } else {
                    1.0
                };
                for &m in &ores {
                    frame.set_emission_scale(&level.queue, m, s);
                }
                frame.step(
                    &level.device,
                    &level.queue,
                    &level.gpu,
                    &camera,
                    &vxe_render::LightParams::headlamp(),
                );
            }
            stages.push([
                rd(&level, frame.direct_accumulated()),
                rd(&level, frame.moments()),
                rd(&level, frame.filtered()),
                rd(&level, frame.accumulated()),
            ]);
        }
        let lit: Vec<bool> = stages[0][3]
            .iter()
            .map(|p| p[0] + p[1] + p[2] > 0.0)
            .collect();
        println!(
            "5e: {} of {} texels in the ores' baked light",
            lit.iter().filter(|&&l| l).count(),
            lit.len()
        );
        for (k, name) in [
            "direct accumulation",
            "moments",
            "filter",
            "emissive accumulation",
        ]
        .iter()
        .enumerate()
        {
            let (mut n, mut n_lit) = (0, 0);
            for i in 0..lit.len() {
                if stages[0][k][i] != stages[1][k][i] {
                    n += 1;
                    n_lit += usize::from(lit[i]);
                }
            }
            println!("  {name}: {n} texels differ, {n_lit} in the ores' light");
        }
    }

    /// Phase 5e: what the ore pulse changes per frame. The light box with the emissive ore's
    /// table and an ore slab on the floor at x 108..116 (baked), one still list light of 600 at
    /// (40, 32, 22), camera still down the box's length, manual exposure 1; 64 frames with the
    /// ore's scale pulsing (`0.5 + 0.5 sin`, a new scale each frame) against 64 at scale 1.
    /// Per stage, the texels that differ on the last frame, and of those how many lie where
    /// the ore's baked light reaches (the accumulated emissive part non-zero at scale 1). FSR 2
    /// at 1.5 adds the reactive mask's marked texels.
    #[test]
    #[ignore]
    fn reading_what_the_ore_pulse_changes() {
        use flythrough::cave::ORE;
        use support::bake::{fill, rig_on, test_counts};
        use vxe_render::debug::{read_float, read_rgba8};
        for native in [true, false] {
            let world = vxe_world::World::generate(
                vxe_world::ChunkExtent::new([0, 0, 0], [2, 1, 1]),
                &vxe_world::UniformSource(ROCK),
                &support::cave_table(),
            );
            world
                .apply(&EditOp::box_(
                    Fixed3([2, 2, 2].map(|v: i64| v * FIXED_ONE)),
                    Fixed3([126, 62, 50].map(|v: i64| v * FIXED_ONE)),
                    Mode::Carve,
                ))
                .expect("resident");
            fill(&world, [108, 28, 2], [116, 36, 4], ORE);
            let Some(mut rig) = rig_on(world, [192, 108], None) else {
                return;
            };
            rig.frame = if native {
                box_frame(&rig.device, [192, 108], [192, 108], None)
            } else {
                box_frame(
                    &rig.device,
                    [128, 72],
                    [192, 108],
                    Some(vxe_render::UpscalerKind::Fsr2(Default::default())),
                )
            };
            rig.bake(&test_counts(256));
            let camera = box_long_camera();
            rig.frame
                .set_lights(&[Light::point([40.0, 32.0, 22.0], 600.0)]);
            let mut run = |pulse: bool| {
                rig.frame.reset_history();
                let mut changed = 0;
                for i in 0..64 {
                    let s = if pulse {
                        0.5 + 0.5 * (i as f32).sin()
                    } else {
                        1.0
                    };
                    rig.frame.set_emission_scale(&rig.queue, ORE, s);
                    step(&mut rig, &camera);
                    changed += usize::from(rig.frame.lights_changed());
                }
                let rd = |t: &vxe_render::wgpu::Texture| read_float(&rig.device, &rig.queue, t);
                let reactive = rig
                    .frame
                    .fsr2()
                    .map(|f| read_rgba8(&rig.device, &rig.queue, f.reactive()));
                (
                    changed,
                    [
                        rd(rig.frame.direct_accumulated()),
                        rd(rig.frame.moments()),
                        rd(rig.frame.filtered()),
                        rd(rig.frame.accumulated()),
                    ],
                    reactive,
                )
            };
            let (c_off, off, r_off) = run(false);
            let (c_on, on, r_on) = run(true);
            let lit: Vec<bool> = off[3].iter().map(|p| p[0] + p[1] + p[2] > 0.0).collect();
            println!(
                "== {}: change bit set on {c_off} frames steady, {c_on} pulsing; {} of {} texels in the ore's baked light",
                if native { "native" } else { "fsr2 1.5" },
                lit.iter().filter(|&&l| l).count(),
                lit.len()
            );
            for (k, name) in [
                "direct accumulation",
                "moments (l, l^2, age)",
                "filter (rgb, variance)",
                "emissive accumulation",
            ]
            .iter()
            .enumerate()
            {
                let (mut n, mut n_lit, mut age_n) = (0, 0, 0);
                for i in 0..off[k].len() {
                    if off[k][i] != on[k][i] {
                        n += 1;
                        n_lit += usize::from(lit[i]);
                    }
                    if k == 1 && off[k][i][2] != on[k][i][2] {
                        age_n += 1;
                    }
                }
                println!(
                    "  {name}: {n} texels differ, {n_lit} of them in the ore's light{}",
                    if k == 1 {
                        format!("; ages differ at {age_n}")
                    } else {
                        String::new()
                    }
                );
            }
            if let (Some(a), Some(b)) = (r_off, r_on) {
                let marked = |r: &[u8]| r.chunks(4).filter(|p| p[0] > 0).count();
                println!(
                    "  reactive mask: {} texels marked steady, {} pulsing, of {}",
                    marked(&a),
                    marked(&b),
                    a.len() / 4
                );
            }
        }
    }

    /// One stage's texels of a coloured run against the white run's: in the light's channel
    /// `k`, the largest relative difference and the share past 1e-3; in the other two, the
    /// largest value and the share above 1e-3 of the white channel's peak.
    fn stage_line(name: &str, c: &[[f32; 4]], w: &[[f32; 4]], k: usize) {
        let peak = w.iter().map(|p| p[k]).fold(0.0f32, f32::max).max(1e-6);
        let (mut rel, mut rel_n, mut other, mut other_n) = (0.0f32, 0usize, 0.0f32, 0usize);
        for (a, b) in c.iter().zip(w) {
            let r = (a[k] - b[k]).abs() / b[k].abs().max(peak * 1e-4);
            rel = rel.max(r);
            rel_n += usize::from(r > 1e-3);
            for j in (0..3).filter(|&j| j != k) {
                other = other.max(a[j].abs());
                other_n += usize::from(a[j].abs() > peak * 1e-3);
            }
        }
        println!(
            "  {name}: own channel worst {:.3e}, {:.3} % past 1e-3; others max {other:.4} ({other_n} texels past 1e-3 of the peak {peak:.1})",
            rel,
            rel_n as f64 * 100.0 / c.len() as f64
        );
    }

    /// Phase 5b: a pure red and a pure blue light of 600 at 2 voxels (the fly-through's
    /// SURFACE_GAP) off the box's far wall, lamp off, identity tone map, native and FSR 2 at
    /// 1.5, lighting full and half, at manual exposure 1 and at the exposure auto settles to
    /// with the white light; each stage after 32 still frames against the white light's.
    #[test]
    #[ignore]
    fn reading_a_saturated_light_near_a_wall_per_stage() {
        use vxe_render::{
            AutoExposure, Exposure, LightingResolution, ToneMap, UpscalerKind,
            debug::{read_float, read_rgba8},
        };
        let Some(rig) = box_rig() else {
            return;
        };
        let camera = Camera {
            position: [110.0, 32.0, 26.0].into(),
            yaw: 0.0,
            pitch: 0.0,
            vfov: 1.2,
        };
        let at = [124.0, 32.0, 26.0];
        let make = |colour: [f32; 3]| Light {
            colour,
            ..Light::point(at, 600.0)
        };
        let fsr = || UpscalerKind::Fsr2(vxe_render::Fsr2Settings::default());
        // `VXE_WIDE=1` widens the rig toward the window: manual exposure 16 (auto's +4 EV
        // limit, where the level settled), no light floor, and a dim white light of 30 at
        // (60, 32, 26) in every frame, so the wall's other channels are small but not zero
        // (the level's baked light in the window).
        let wide = std::env::var("VXE_WIDE").as_deref() == Ok("1");
        let ambient = Light::point([60.0, 32.0, 26.0], 30.0);
        let run = |native: bool, res: LightingResolution, exposure: Exposure, lights: &[Light]| {
            let mut lights = lights.to_vec();
            if wide {
                lights.push(ambient);
            }
            let lights = &lights[..];
            let mut f = if native {
                box_frame(&rig.device, [192, 108], [192, 108], None)
            } else {
                box_frame(&rig.device, [128, 72], [192, 108], Some(fsr()))
            };
            f.set_resolution(&rig.device, res);
            f.set_lighting(Lighting {
                tone_map: ToneMap::Identity,
                exposure,
                light_floor: if wide { 0.0 } else { f.lighting().light_floor },
                ..f.lighting()
            });
            f.set_lights(lights);
            for _ in 0..32 {
                step_frame(&rig, &mut f, &camera);
            }
            f
        };
        let resolutions: &[LightingResolution] = if wide {
            &[LightingResolution::Half]
        } else {
            &[LightingResolution::Full, LightingResolution::Half]
        };
        for native in [true, false] {
            for &res in resolutions {
                if wide {
                    println!("(wide: exposure 16, floor 0, the dim white light in every frame)");
                }
                let auto = run(
                    native,
                    res,
                    Exposure::Auto(AutoExposure::instant()),
                    &[make([1.0; 3])],
                );
                let settled = auto
                    .exposure_readout(&rig.device)
                    .map_or(1.0, |r| r.exposure);
                let exposures = if wide { vec![16.0] } else { vec![1.0, settled] };
                for ex in exposures {
                    println!(
                        "== {} lighting {res:?} exposure {ex}",
                        if native { "native" } else { "fsr2 1.5" }
                    );
                    let white = run(native, res, Exposure::Manual(ex), &[make([1.0; 3])]);
                    let none = run(native, res, Exposure::Manual(ex), &[]);
                    let w_target = read_rgba8(&rig.device, &rig.queue, white.target());
                    let n_target = read_rgba8(&rig.device, &rig.queue, none.target());
                    for (cname, colour, k) in
                        [("red", [1.0, 0.0, 0.0], 0), ("blue", [0.0, 0.0, 1.0], 2)]
                    {
                        let c = run(native, res, Exposure::Manual(ex), &[make(colour)]);
                        println!(" {cname}:");
                        let rd = |f: &vxe_render::Frame, t: &str| -> Vec<[f32; 4]> {
                            let tex = match t {
                                "direct" => f.direct(),
                                "direct_acc" => f.direct_accumulated(),
                                "filtered" => f.filtered(),
                                "colour_in" => f.fsr2().unwrap().colour_in(),
                                "prepared" => f.fsr2().unwrap().prepared(),
                                "upscaled0" => f.fsr2().unwrap().upscaled(0),
                                _ => f.fsr2().unwrap().upscaled(1),
                            };
                            read_float(&rig.device, &rig.queue, tex)
                        };
                        let mut names = vec!["direct", "direct_acc", "filtered"];
                        if !native {
                            names.extend(["colour_in", "upscaled0", "upscaled1"]);
                        }
                        for t in names {
                            stage_line(t, &rd(&c, t), &rd(&white, t), k);
                        }
                        if !native {
                            // The prepared colour is YCoCg: G = Y + Cg, B = Y - Co - Cg.
                            let back = |v: Vec<[f32; 4]>| -> Vec<[f32; 4]> {
                                v.iter()
                                    .map(|p| {
                                        [p[0] + p[1] - p[2], p[0] + p[2], p[0] - p[1] - p[2], 0.0]
                                    })
                                    .collect()
                            };
                            stage_line(
                                "prepared (rgb)",
                                &back(rd(&c, "prepared")),
                                &back(rd(&white, "prepared")),
                                k,
                            );
                        }
                        // The final image, bytes against the white frame's (own) and the
                        // no-light frame's (others).
                        let t = read_rgba8(&rig.device, &rig.queue, c.target());
                        let px = t.len() / 4;
                        let (mut own_n, mut own_max, mut oth_n, mut oth_max) = (0, 0u8, 0, 0u8);
                        for i in 0..px {
                            let d = t[4 * i + k].abs_diff(w_target[4 * i + k]);
                            own_max = own_max.max(d);
                            own_n += usize::from(d > 2);
                            for j in (0..3).filter(|&j| j != k) {
                                let d = t[4 * i + j].abs_diff(n_target[4 * i + j]);
                                oth_max = oth_max.max(d);
                                oth_n += usize::from(d > 2);
                            }
                        }
                        println!(
                            "  target: own channel against white max {own_max}, {:.3} % past 2; others against none max {oth_max}, {oth_n} past 2",
                            own_n as f64 * 100.0 / px as f64
                        );
                    }
                }
            }
        }
    }
}
