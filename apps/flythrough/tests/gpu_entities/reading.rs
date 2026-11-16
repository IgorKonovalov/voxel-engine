// `gpu_entities`'s readings, included by `gpu_entities.rs` (kept apart for the file-size cap).

/// Readings, run by hand (`--run-ignored only -E 'test(=reading::<name>)'`), on the adapter
/// `VXE_TEST_ADAPTER` names (unset: lavapipe). They print and assert nothing.
mod reading {
    use super::*;
    use vxe_render::{FRAME_PASSES, GpuWorld, MirrorConfig, PassTimer, Window};

    /// [`rig_on`]'s rig on the reading adapter, and the adapter's name.
    fn reading_rig(world: World) -> Option<(Rig, String)> {
        let (device, queue, name) = support::reading_device(64 << 20)?;
        let base = MirrorConfig::new(Window::Fixed(world.extent()));
        let cfg = MirrorConfig {
            budget_bytes: 64 << 20,
            ..base
        };
        let mut gpu = GpuWorld::with_config(&device, cfg);
        for _ in 0..2 {
            support::SyncNow::sync_now(&mut gpu, &world, &device, &queue).expect("fits");
        }
        let mut frame = support::bake::new_frame(&device, [W, H]);
        frame.set_lighting(support::lights::box_lighting());
        Some((
            Rig {
                device,
                queue,
                gpu,
                world,
                frame,
                cfg,
            },
            name,
        ))
    }

    fn walker_at(rig: &Rig, feet: [f32; 3], eye: [f32; 3]) -> (Arc<Model>, Pose) {
        let (body, e) = walker::rows(rig.world.materials()).expect("the walker's rows");
        let pose = walker::pose(feet, walker::facing(feet, eye), 0.0);
        (Arc::new(walker::model(body, e)), pose)
    }

    /// Phase 3 done-when 7, the fallback: a still walker beside a world emitter (a 4 x 4 x 8
    /// block of a warm emissive row) with no list light and the lamp off; at the walker's
    /// torso front facing the camera, the temporal pass's accumulated emissive part (its
    /// luminance) at frame 64 and its frame-to-frame rms over the 64 frames.
    #[test]
    #[ignore = "reading"]
    fn the_fallback_on_a_still_walker() {
        let (mut t, _) = table();
        let glow = t.push(Material {
            emission: [4.0, 2.5, 1.0],
            ..Material::named("glow")
        });
        let world = entity_box(&t);
        support::bake::fill(&world, [96, 25, 2], [100, 29, 10], glow);
        let Some((mut rig, adapter)) = reading_rig(world) else {
            return;
        };
        let camera = level_camera(84.0, 9.0);
        let entity = walker_at(&rig, [104.0, 32.0, 2.0], camera.position.to_array());
        draw(&mut rig, &[entity]);
        let mut series = Vec::new();
        let mut px = None;
        for _ in 0..64 {
            rig.step(&camera);
            let i =
                *px.get_or_insert_with(|| nearest(&hits(&rig), [102.0, 32.0, 7.0], entity_hit(0)).0);
            let a = vxe_render::debug::read_float(&rig.device, &rig.queue, rig.frame.accumulated())[i];
            series.push(0.2126 * a[0] + 0.7152 * a[1] + 0.0722 * a[2]);
        }
        let diffs: Vec<f32> = series.windows(2).map(|w| w[1] - w[0]).collect();
        let rms = (diffs.iter().map(|d| d * d).sum::<f32>() / diffs.len() as f32).sqrt();
        println!(
            "{adapter}: the fallback at the torso's front, frame 64 {:.4}, mean of the last 32 {:.4}, frame-to-frame rms {rms:.4} ({:.2} % of the value)",
            series[63],
            series[32..].iter().sum::<f32>() / 32.0,
            100.0 * rms / series[63].max(1e-6)
        );
    }

    /// Phase 3 done-when 7, the cost: the light pass's time over 64 frames on the lit box (a
    /// light of 1200 at (64, 32, 22)) with 0 and 4 walkers standing around under the light, the
    /// median and the p90 in ms.
    #[test]
    #[ignore = "reading"]
    fn the_light_pass_cost_with_walkers() {
        let (t, _) = table();
        let Some((mut rig, adapter)) = reading_rig(entity_box(&t)) else {
            return;
        };
        let Some(timer) = PassTimer::with_passes(&rig.device, &rig.queue, FRAME_PASSES.len() as u32)
        else {
            println!("{adapter}: no timestamp queries");
            return;
        };
        let slot = FRAME_PASSES
            .iter()
            .position(|&n| n == "light")
            .expect("light");
        rig.frame
            .set_lights(&[Light::point([64.0, 32.0, 22.0], 1200.0)]);
        let camera = Camera {
            position: [40.0, 32.0, 30.0].into(),
            yaw: 0.0,
            pitch: -0.6,
            vfov: 1.2,
        };
        for n in [0usize, 4] {
            let entities: Vec<_> = (0..n)
                .map(|k| {
                    let a = std::f32::consts::TAU * k as f32 / 4.0;
                    let feet = [64.0 + 8.0 * a.cos(), 32.0 + 8.0 * a.sin(), 2.0];
                    walker_at(&rig, feet, camera.position.to_array())
                })
                .collect();
            draw(&mut rig, &entities);
            let mut ms = Vec::new();
            for _ in 0..64 {
                rig.frame.advance();
                let params = rig
                    .frame
                    .params(&rig.gpu, &camera, &vxe_render::LightParams::headlamp());
                let mut enc = rig.device.create_command_encoder(&Default::default());
                rig.frame.encode(
                    &rig.device,
                    &rig.queue,
                    &mut enc,
                    &rig.gpu,
                    &params,
                    Some(&timer),
                );
                timer.resolve(&mut enc);
                rig.queue.submit([enc.finish()]);
                ms.push(timer.read_all_ms(&rig.device)[slot]);
            }
            ms.sort_by(f64::total_cmp);
            println!(
                "{adapter}: {n} walkers, light pass median {:.3} ms, p90 {:.3} ms (64 frames, {W} x {H})",
                ms[32], ms[57]
            );
        }
    }

    /// Phase 5 done-when 2, the CPU: `Frame::set_entities` at 64 walkers on a ring (each frame a
    /// new pose, so every part moves), and `EntitySet::raycast` at those walkers over 4096 seeded
    /// rays from the ring's centre, the mean of each.
    #[test]
    #[ignore = "reading"]
    #[allow(
        clippy::disallowed_methods,
        reason = "a reading of the CPU's time, by wall clock"
    )]
    fn the_cpu_cost_at_64_walkers() {
        let (t, _) = table();
        let Some((mut rig, adapter)) = reading_rig(entity_box(&t)) else {
            return;
        };
        let (body, eye) = walker::rows(rig.world.materials()).expect("the walker's rows");
        let model = Arc::new(walker::model(body, eye));
        let h = rig
            .frame
            .entity_mirror_mut()
            .upload_model(&rig.queue, &model, rig.world.materials())
            .expect("room");
        let centre = [64.0f32, 32.0, 2.0];
        let feet: Vec<[f32; 3]> = (0..64)
            .map(|k| {
                let a = std::f32::consts::TAU * k as f32 / 64.0;
                [centre[0] + 24.0 * a.cos(), centre[1] + 24.0 * a.sin(), 2.0]
            })
            .collect();
        let mut set = EntitySet::new();
        let ids: Vec<EntityId> = feet
            .iter()
            .map(|&f| {
                set.insert(model.clone(), walker::pose(f, 0.0, 0.0))
                    .expect("fits")
            })
            .collect();
        let list: Vec<(EntityId, ModelHandle)> = ids.iter().map(|&id| (id, h)).collect();
        let rounds = 256;
        let mut set_ms = Vec::new();
        for r in 0..rounds {
            for (k, &id) in ids.iter().enumerate() {
                let swing = 0.5 * ((r + k) as f32 * 0.1).sin();
                set.set_pose(id, walker::pose(feet[k], 0.01 * r as f32, swing))
                    .expect("fits");
            }
            let t0 = std::time::Instant::now();
            rig.frame.set_entities(&rig.queue, &set, &list);
            set_ms.push(t0.elapsed().as_secs_f64() * 1e3);
        }
        assert_eq!(rig.frame.entity_mirror().parts_drawn(), 256);
        set_ms.sort_by(f64::total_cmp);
        let mean = set_ms.iter().sum::<f64>() / rounds as f64;
        // Seeded rays from above the ring's centre, spread over the lower hemisphere.
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        let origin = Fixed3([64 * 256, 32 * 256, 8 * 256]);
        let rays: Vec<[i64; 3]> = (0..4096)
            .map(|_| {
                let a = std::f64::consts::TAU * next();
                let z = -0.6 * next();
                let r = (1.0 - z * z).sqrt();
                [a.cos() * r, a.sin() * r, z].map(|v| (v * 256.0).round() as i64)
            })
            .collect();
        let t0 = std::time::Instant::now();
        let mut hits = 0;
        for d in &rays {
            hits += usize::from(set.raycast(origin, *d, 200 * 256).is_some());
        }
        let ray_us = t0.elapsed().as_secs_f64() * 1e6 / rays.len() as f64;
        println!(
            "{adapter}: set_entities at 64 walkers (256 parts), {rounds} rounds: mean {mean:.4} ms, median {:.4} ms, p90 {:.4} ms; EntitySet::raycast over 4096 rays: mean {ray_us:.2} us, {hits} hits",
            set_ms[rounds / 2],
            set_ms[rounds * 9 / 10]
        );
    }

    /// The grain reading's walker `k` of eight at `frame`: walking, a 6-voxel circle at 1 m/s
    /// about its centre in a 2 x 4 grid, legs swinging; still, its pose at frame 0.
    fn grain_pose(k: usize, frame: usize, walking: bool) -> Pose {
        let c = [104.0 + 12.0 * (k / 4) as f32, 14.0 + 12.0 * (k % 4) as f32];
        let time = if walking { frame as f32 / 60.0 } else { 0.0 };
        let a = 0.7 * k as f32 + time * walker::WALK_SPEED / 6.0;
        let feet = [c[0] + 6.0 * a.cos(), c[1] + 6.0 * a.sin(), 2.0];
        let swing = walker::WALK_SWING * (std::f32::consts::TAU * (time + 0.13 * k as f32)).sin();
        walker::pose(feet, a, if walking { swing } else { 0.0 })
    }

    /// One measured frame of the grain reading. At the internal size: the walkers' pixels, the
    /// stages' luma (the light pass's sample, the accumulation, the filter's output), the age
    /// and the fresh flag; at the output size the walkers' and the world's pixels (those whose
    /// internal pixel is) and the display's luma.
    struct Shot {
        ent: Vec<bool>,
        age: Vec<f32>,
        fresh: Vec<bool>,
        stages: [Vec<f32>; 3],
        out_ent: Vec<bool>,
        out_world: Vec<bool>,
        display: Vec<f32>,
    }

    fn grain_shot(rig: &Rig, internal: [u32; 2]) -> Shot {
        let lum = |p: &[f32; 4]| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
        let rec = vxe_render::debug::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
        let ent: Vec<bool> = rec
            .iter()
            .map(|r| r[3] & 1 != 0 && r[3] & 32 != 0)
            .collect();
        let wor: Vec<bool> = rec
            .iter()
            .map(|r| r[3] & 1 != 0 && r[3] & 32 == 0)
            .collect();
        let rd =
            |t: &vxe_render::wgpu::Texture| vxe_render::debug::read_float(&rig.device, &rig.queue, t);
        let irr = rd(rig.frame.irradiance());
        let acc = rd(rig.frame.accumulated());
        let fil = rd(rig.frame.filtered());
        let mom = rd(rig.frame.moments());
        let img = rig.display();
        let display: Vec<f32> = img
            .chunks_exact(4)
            .map(|p| 0.2126 * f32::from(p[0]) + 0.7152 * f32::from(p[1]) + 0.0722 * f32::from(p[2]))
            .collect();
        let inner = |i: usize| {
            let (x, y) = (i % W as usize, i / W as usize);
            (y * internal[1] as usize / H as usize) * internal[0] as usize
                + x * internal[0] as usize / W as usize
        };
        Shot {
            age: mom.iter().map(|m| m[2]).collect(),
            fresh: irr.iter().map(|p| p[3] > 0.5).collect(),
            stages: [
                irr.iter().map(lum).collect(),
                acc.iter().map(lum).collect(),
                fil.iter().map(lum).collect(),
            ],
            out_ent: (0..display.len()).map(|i| ent[inner(i)]).collect(),
            out_world: (0..display.len()).map(|i| wor[inner(i)]).collect(),
            ent,
            display,
        }
    }

    /// The grain reading's run: a fresh frame (at 128 x 72 under an upscaler, jittered) with
    /// `light` as its entities' emissive light and the box baked at `test_counts(256)`, its
    /// frame counter `offset` on, 128 frames of the eight walkers, frames 64 to 127 measured.
    fn grain_run(
        rig: &mut Rig,
        model: &Arc<Model>,
        (offset, walking): (u32, bool),
        kind: Option<vxe_render::UpscalerKind>,
        light: vxe_render::EntityLight,
    ) -> Vec<Shot> {
        let camera = Camera {
            position: [78.0, 32.0, 16.0].into(),
            yaw: 0.0,
            pitch: -0.3,
            vfov: 1.2,
        };
        let internal = if kind.is_some() { [128, 72] } else { [W, H] };
        let mut frame = support::lights::box_frame(&rig.device, internal, [W, H], kind);
        frame.set_jitter(&rig.device, kind.is_some());
        frame.set_lighting(vxe_render::Lighting {
            entity_light: light,
            ..support::lights::box_lighting()
        });
        for _ in 0..offset {
            frame.advance();
        }
        rig.frame = frame;
        rig.bake(&support::bake::test_counts(256));
        let h = rig
            .frame
            .entity_mirror_mut()
            .upload_model(&rig.queue, model, rig.world.materials())
            .expect("room");
        let mut set = EntitySet::new();
        let ids: Vec<EntityId> = (0..8)
            .map(|k| {
                set.insert(model.clone(), grain_pose(k, 0, walking))
                    .expect("fits")
            })
            .collect();
        let list: Vec<(EntityId, ModelHandle)> = ids.iter().map(|&id| (id, h)).collect();
        let mut shots = Vec::new();
        for f in 0..128 {
            for (k, &id) in ids.iter().enumerate() {
                set.set_pose(id, grain_pose(k, f, walking)).expect("fits");
            }
            rig.frame.set_entities(&rig.queue, &set, &list);
            rig.step(&camera);
            if f >= 64 {
                shots.push(grain_shot(rig, internal));
            }
        }
        shots
    }

    /// Sums of the squared difference, the mean and the count, for a relative rms.
    #[derive(Default, Clone, Copy)]
    struct Diff([f64; 3]);

    impl Diff {
        fn add(&mut self, x: f32, y: f32) {
            let (x, y) = (f64::from(x), f64::from(y));
            self.0[0] += (x - y).powi(2);
            self.0[1] += 0.5 * (x + y);
            self.0[2] += 1.0;
        }

        /// The rms of the difference over `sqrt 2` (one run's noise), of the mean.
        fn rel(self) -> String {
            let rms = (self.0[0] / self.0[2] / 2.0).sqrt();
            let mean = self.0[1] / self.0[2];
            format!(
                "{rms:.4} of {mean:.4} ({:.1} %)",
                100.0 * rms / mean.max(1e-9)
            )
        }
    }

    /// Plan 0062 Phase 6 (e), the grain on walking walkers: eight walkers in a crowd beside a
    /// world emitter (the fallback rig), lamp off and no list light, still or walking (each a
    /// 6-voxel circle at 1 m/s, legs swinging), at the output size with no jitter, and under
    /// FSR 2 and our upscaler at 1.5 with it. Each case runs twice from a fresh frame, the
    /// second with its frame counter 1008 on (the jitter repeats every 16 frames, ours, and
    /// 18, FSR 2's at 1.5), so the poses and the jitter match and only the random stream
    /// differs; the noise of a stage is the rms of the two runs' difference over `sqrt 2`, over
    /// the walkers' pixels of frames 64 to 127, relative to their mean. The stages: the light
    /// pass's sample, the temporal pass's accumulation, the filter's output (split at age 8)
    /// and the display's luma; with the walkers' mean age, the share younger than 8 frames and
    /// the share fresh, and the world pixels' display noise. Each case under both entity lights
    /// (`EntityLight`), the box baked; at the output size, still, the shading `world` loses: the
    /// rms over the walkers' pixels of the difference of the two lights' filtered means over
    /// frames 64 to 127, relative to `own`'s mean, and the ratio of the means.
    #[test]
    #[ignore = "reading"]
    fn the_grain_on_walking_walkers() {
        let (mut t, _) = table();
        let glow = t.push(Material {
            emission: [4.0, 2.5, 1.0],
            ..Material::named("glow")
        });
        let world = entity_box(&t);
        support::bake::fill(&world, [96, 25, 2], [100, 29, 10], glow);
        let Some((mut rig, adapter)) = reading_rig(world) else {
            return;
        };
        let (body, eye) = walker::rows(rig.world.materials()).expect("the walker's rows");
        let model = Arc::new(walker::model(body, eye));
        let kinds = [
            None,
            Some(support::rigs::fsr2_kind()),
            Some(vxe_render::UpscalerKind::Own(
                vxe_render::Upscaler::default(),
            )),
        ];
        let lights = [vxe_render::EntityLight::Own, vxe_render::EntityLight::World];
        // The still output-size runs' filtered means per walker pixel, `own` then `world`.
        let mut still: Vec<Vec<Option<f64>>> = Vec::new();
        for (kind, filter) in kinds.into_iter().flat_map(|k| lights.map(|y| (k, y))) {
            let name = kind.map_or("output size", |k| k.name());
            for walking in [false, true] {
                let a = grain_run(&mut rig, &model, (0, walking), kind, filter);
                let b = grain_run(&mut rig, &model, (1008, walking), kind, filter);
                if kind.is_none() && !walking {
                    still.push(
                        (0..a[0].ent.len())
                            .map(|i| {
                                a.iter().all(|s| s.ent[i]).then(|| {
                                    a.iter().map(|s| f64::from(s.stages[2][i])).sum::<f64>()
                                        / a.len() as f64
                                })
                            })
                            .collect(),
                    );
                }
                let mut stage = [Diff::default(); 3];
                // The filtered stage split by age: younger than 8, then 8 and older.
                let mut by_age = [Diff::default(); 2];
                let (mut display, mut world) = (Diff::default(), Diff::default());
                let (mut ages, mut young, mut fresh, mut n) = (0.0f64, 0usize, 0usize, 0usize);
                for (sa, sb) in a.iter().zip(&b) {
                    for i in (0..sa.ent.len()).filter(|&i| sa.ent[i] && sb.ent[i]) {
                        for (d, (x, y)) in stage.iter_mut().zip(sa.stages.iter().zip(&sb.stages)) {
                            d.add(x[i], y[i]);
                        }
                        by_age[usize::from(sa.age[i] >= 8.0)].add(sa.stages[2][i], sb.stages[2][i]);
                        ages += f64::from(sa.age[i]);
                        young += usize::from(sa.age[i] < 8.0);
                        fresh += usize::from(sa.fresh[i]);
                        n += 1;
                    }
                    for i in 0..sa.display.len() {
                        if sa.out_ent[i] && sb.out_ent[i] {
                            display.add(sa.display[i], sb.display[i]);
                        } else if sa.out_world[i] && sb.out_world[i] {
                            world.add(sa.display[i], sb.display[i]);
                        }
                    }
                }
                println!(
                    "{adapter}, {name}, entity light {}, {} walkers: {n} pixel-frames; noise: sample {}, accum {}, filtered {} (age < 8: {:.0} px, {}; older: {}), display {} /255; age mean {:.1}, younger than 8 {:.1} %, fresh {:.1} %; world display {} /255",
                    filter.name(),
                    if walking { "walking" } else { "still" },
                    stage[0].rel(),
                    stage[1].rel(),
                    stage[2].rel(),
                    by_age[0].0[2],
                    by_age[0].rel(),
                    by_age[1].rel(),
                    display.rel(),
                    ages / n as f64,
                    100.0 * young as f64 / n as f64,
                    100.0 * fresh as f64 / n as f64,
                    world.rel(),
                );
            }
        }
        let (own, world) = (&still[0], &still[1]);
        let (mut sq, mut so, mut sw, mut n) = (0.0f64, 0.0f64, 0.0f64, 0usize);
        for (o, w) in own.iter().zip(world) {
            if let (Some(o), Some(w)) = (o, w) {
                sq += (w - o).powi(2);
                so += o;
                sw += w;
                n += 1;
            }
        }
        println!(
            "{adapter}, output size, still walkers: world against own over {n} pixels: rms of the difference {:.1} % of own's mean {:.4}; world's mean {:.4} ({:.1} % of own's)",
            100.0 * (sq / n as f64).sqrt() / (so / n as f64),
            so / n as f64,
            sw / n as f64,
            100.0 * sw / so,
        );
    }

    /// Phase 6d: the time `Frame::new` takes at 192 x 108 on a fresh device, three frames in a
    /// row (the first pays the driver's cold compiles; a driver cache may serve the rest), and the
    /// compute pipelines each creates.
    #[test]
    #[ignore = "reading"]
    #[allow(
        clippy::disallowed_methods,
        reason = "a reading of the frame's creation time, by wall clock"
    )]
    fn the_frame_creation_time() {
        let Some((device, _queue, adapter)) = support::reading_device(64 << 20) else {
            return;
        };
        for k in 0..3 {
            let made = vxe_render::debug::compute_pipelines_created();
            let t0 = std::time::Instant::now();
            let frame = vxe_render::Frame::new(&device, W, H);
            let ms = t0.elapsed().as_secs_f64() * 1e3;
            let n = vxe_render::debug::compute_pipelines_created() - made;
            println!("{adapter}: Frame::new {k}: {ms:.1} ms, {n} compute pipelines");
            drop(frame);
        }
    }
}
