// `gpu_sun`'s readings, included by `gpu_sun.rs` (kept apart for the file-size cap).

mod reading {
    use super::*;

    /// Plan 0086 Phase 4 bar 3: the baked sky term (ADR-0070) against the ray. Under
    /// `Sky::uniform(1.0)`, filter off, exposure 1, one sky ray, the coarse walk: `world`'s
    /// bricks within two of `brick` baked alone at 256 samples a brick direction (the bake's
    /// face filter), then four frames: the centre pixel's emissive part (`accumulated`, the
    /// store's term with no history) and direct part (the ray's sample where the store holds
    /// no visibility). Returns `(baked, direct)`.
    fn baked_term(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        world: &World,
        cam: &Camera,
        brick: [i32; 3],
    ) -> ([f32; 4], [f32; 4]) {
        let mut gpu = mirror(device, queue, world, None);
        let mut f = frame(device, SIZE, ShadowWalk::Coarse);
        f.set_lighting(Lighting {
            sky_rays: 1,
            ..f.lighting()
        });
        f.set_sky(Some(Sky {
            altitude_zero_z: FLOOR as f32,
            ..Sky::uniform(1.0)
        }));
        let settings = vxe_render::BakeSettings {
            round: 64,
            brick_samples: 256,
            filter: Some(std::sync::Arc::new(move |v: [i32; 3], _| {
                (0..3).all(|k| (v[k].div_euclid(8) - brick[k]).abs() <= 2)
            })),
            ..vxe_render::BakeSettings::default()
        };
        let report = f
            .bake_until(
                device,
                queue,
                &mut gpu,
                world,
                world.extent(),
                &settings,
                &mut |_| {},
            )
            .expect("the store fits");
        println!(
            "  bake: {} faces, {} sky rays",
            report.faces, report.sky_rays
        );
        for _ in 0..4 {
            f.step(device, queue, &gpu, cam, &LightParams::headlamp());
        }
        let at = |t: &wgpu::Texture| {
            let d = vxe_render::debug::read_rgba16f(device, queue, t);
            let (w, h) = (t.width() as usize, t.height() as usize);
            d[(h / 2) * w + w / 2]
        };
        (at(f.accumulated()), at(f.direct()))
    }

    /// Plan 0086 Phase 4 bar 3: the open floor's baked sky term against `pi` (Plan 0032 Phase 6
    /// bar 2's ray read 3.1406), a wall's outward face in the open against `pi / 2`, the closed
    /// box's floor against 0; each beside the ray's 64-frame mean on the same pixel with no
    /// bake. Printed.
    #[test]
    #[ignore = "reading"]
    fn the_baked_sky_term_matches_the_ray() {
        let Some((device, queue)) = device() else {
            return;
        };
        let walls = |p: [i32; 3]| {
            let inside = |k: usize, lo: i32, hi: i32| (lo..hi).contains(&p[k]);
            let outer = inside(0, 100, 140) && inside(1, 100, 140) && inside(2, FLOOR, FLOOR + 30);
            let inner = inside(0, 105, 135) && inside(1, 105, 135) && inside(2, FLOOR, FLOOR + 25);
            let wall = inside(0, 192, 200) && inside(1, 40, 216) && inside(2, FLOOR, FLOOR + 80);
            (outer && !inner) || wall
        };
        let world = floor_world([4, 4, 1], walls);
        let pi = std::f32::consts::PI;
        let rigs = [
            // About 40 voxels ahead of the camera on the open floor.
            (
                "open floor",
                north_at([40.0, 20.0, FLOOR as f32 + 40.0], -45.0),
                [5, 7, 1],
                pi,
            ),
            // The wall's +x face at x 200, looking -x at z FLOOR + 30.
            (
                "wall +x",
                Camera {
                    position: Vec3::new(230.0, 128.0, FLOOR as f32 + 30.0),
                    yaw: std::f32::consts::PI,
                    pitch: 0.0,
                    vfov: 60f32.to_radians(),
                },
                [24, 16, 5],
                pi / 2.0,
            ),
            (
                "closed box",
                north_at([120.0, 110.0, FLOOR as f32 + 12.0], -45.0),
                [15, 14, 1],
                0.0,
            ),
        ];
        for (name, cam, brick, want) in rigs {
            let (baked, direct) = baked_term(&device, &queue, &world, &cam, brick);
            let ray = under_uniform_sky(&device, &queue, &world, &cam);
            println!(
                "{name}: baked {:.4} ({:.4} of the expected {want:.4}), direct with the bake \
                 {:.4}, the ray's 64-frame mean {:.4}",
                baked[1],
                baked[1] / want.max(1e-6),
                direct[1],
                ray[1]
            );
        }
    }

    /// Bar 3 under the coarse walk, printed (the bar is on the full walk).
    #[test]
    #[ignore = "reading"]
    fn a_roof_slab_casts_a_soft_shadow_under_the_coarse_walk() {
        if let Some(r) = super::roof_slab_row(ShadowWalk::Coarse) {
            println!("coarse: {r:?}");
        }
    }

    /// Bars 1, 2 and 4 under the default coarse walk, printed.
    #[test]
    #[ignore = "reading"]
    fn the_floor_values_under_the_coarse_walk() {
        let Some((device, queue)) = device() else {
            return;
        };
        let world = floor_world([2, 2, 1], |_| false);
        let gpu = mirror(&device, &queue, &world, None);
        let cam = camera([64.0, 40.0]);
        for (name, s) in [
            (
                "noon",
                sky(dir(90.0, 0.0), dir(-45.0, 0.0), [[1.0; 3], [0.0; 3]]),
            ),
            (
                "slant",
                sky(dir(30.0, 90.0), dir(-45.0, 0.0), [[1.0; 3], [0.0; 3]]),
            ),
            (
                "moon",
                sky(dir(-30.0, 0.0), dir(90.0, 0.0), [[1.0; 3], [1e-3; 3]]),
            ),
        ] {
            let mut f = frame(&device, SIZE, ShadowWalk::Coarse);
            f.set_sky(Some(s));
            let got = centre_direct(&device, &queue, &mut f, &gpu, &cam, 16);
            println!("coarse, {name}: {got:?}");
        }
    }

    /// Plan 0032 Phase 6 Step A: a still camera 40 voxels over a 1024 x 1024 rock floor,
    /// looking down at 45 degrees, filter on, the default lighting tier, native 320 x 180 and
    /// FSR 2 at 1.5 to 480 x 270. Under `Sky::uniform(1.0)` at 1, 2 and 4 sky rays, and with
    /// the headlamp and no sky: after 64 settle frames, the mean and the 99th percentile of
    /// the per-pixel change of the displayed image (the largest channel, in levels of 255)
    /// between consecutive frames over 64 frames. The exposure is pinned so the floor's mean
    /// display level is near 118 (mid-grey, 0.18 linear).
    #[test]
    #[ignore = "reading"]
    fn sky_ray_stillness() {
        let Some((device, queue)) = device() else {
            return;
        };
        let world = floor_world([16, 16, 1], |_| false);
        let gpu = mirror(&device, &queue, &world, None);
        let cam = Camera {
            position: Vec3::new(512.0, 400.0, FLOOR as f32 + 40.0),
            yaw: FRAC_PI_2,
            pitch: -45f32.to_radians(),
            vfov: 60f32.to_radians(),
        };
        let run = |rays, fsr, exposure| {
            let mut f = still_frame(&device, fsr);
            still_change(&device, &queue, &gpu, &cam, &mut f, rays, exposure, 64)
        };
        for fsr in [false, true] {
            let name = if fsr { "FSR 2 at 1.5" } else { "native" };
            for rays in [None, Some(1), Some(2), Some(4)] {
                // Two passes: the second's exposure scales the first's toward level 118 by the
                // display's gamma of 2.2.
                let mut ex = if rays.is_some() { 0.5 } else { 1.0 };
                let (_, _, level) = run(rays, fsr, ex);
                ex *= ((118.0 / level.max(1.0)) as f32).powf(2.2);
                let (mean, p99, level) = run(rays, fsr, ex);
                let what = rays.map_or("the headlamp, no sky".into(), |n| format!("{n} sky rays"));
                println!(
                    "{name}, {what}: exposure {ex:.4}, mean level {level:.1}, change mean {mean:.3}, p99 {p99}"
                );
            }
        }
    }

    /// The canopy rig's leaf material.
    const LEAF: MaterialId = 2;
    /// The canopy's centre column and height above the floor, and its radius.
    const CANOPY: [i32; 3] = [64, 64, 80];
    const CANOPY_R: i32 = 12;
    /// The canopy's kept voxels, of 256 (the testbed's `LEAF_FILL`).
    const FILL: u64 = 64;

    /// Whether `p` is a kept canopy voxel: inside the sphere and its hash under FILL of 256.
    fn canopy_at(p: [i32; 3]) -> bool {
        let d = [0, 1, 2].map(|k| i64::from(p[k] - [CANOPY[0], CANOPY[1], FLOOR + CANOPY[2]][k]));
        if d.iter().map(|v| v * v).sum::<i64>() > i64::from(CANOPY_R * CANOPY_R) {
            return false;
        }
        let mut h = (p[0] as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ (p[1] as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
            ^ (p[2] as u64).wrapping_mul(0x1656_67B1_9E37_79F9);
        h ^= h >> 31;
        h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h ^= h >> 29;
        h & 255 < FILL
    }

    /// Per floor pixel of the canopy rig (a floating canopy of radius 12 at `leaf_fill` 64, 80
    /// voxels over a 128 x 128 rock floor; the camera 40 voxels over the floor under it looking
    /// straight down, 256 x 256 at 4 pixels a voxel; a zenith sun of `E = (1, 1, 1)` at the real
    /// radius, the key light alone, the coarse walk, lighting at full resolution): the direct
    /// target's luminance, a 16-frame mean, and the hit's floor voxel (x, y). `canopy` false
    /// drops the canopy; a nonzero `style` is its material's style word.
    fn canopy_floor(canopy: bool, style: u32) -> Option<Vec<(f32, [i32; 2])>> {
        let (device, queue) = device()?;
        let table: MaterialTable = [Material::named("rock"), Material::named("leaf")]
            .into_iter()
            .collect();
        let f = move |p: [i32; 3]| {
            if p[2] < FLOOR {
                ROCK
            } else if canopy && canopy_at(p) {
                LEAF
            } else {
                AIR
            }
        };
        let world = World::generate(ChunkExtent::new([0, 0, 0], [2, 2, 2]), &FnSource(f), &table);
        let gpu = mirror(&device, &queue, &world, None);
        let mut frame = super::frame(&device, [256, 256], ShadowWalk::Coarse);
        frame.set_sky(Some(sky(
            dir(90.0, 0.0),
            dir(-90.0, 0.0),
            [[1.0; 3], [0.0; 3]],
        )));
        if style != 0 {
            let mut styles = vec![0; usize::from(LEAF) + 1];
            styles[usize::from(LEAF)] = style;
            frame.set_material_styles(&queue, &styles);
        }
        // tan(hfov / 2) = 32 / 40: 64 voxels across 256 pixels at the floor.
        let cam = Camera {
            position: Vec3::new(64.0, 64.0, FLOOR as f32 + 40.0),
            yaw: 0.0,
            pitch: -FRAC_PI_2 + 1e-4,
            vfov: 2.0 * 0.8f32.atan(),
        };
        let mut sum: Vec<f64> = Vec::new();
        for _ in 0..16 {
            frame.step(&device, &queue, &gpu, &cam, &LightParams::headlamp());
            let d = vxe_render::debug::read_rgba16f(&device, &queue, frame.direct());
            sum.resize(d.len(), 0.0);
            for (s, p) in sum.iter_mut().zip(&d) {
                *s += f64::from(p[3]) / 16.0;
            }
        }
        let hits = vxe_render::debug::read_rgba32f(&device, &queue, frame.light_hit_positions());
        Some(
            sum.iter()
                .zip(&hits)
                .filter(|(_, h)| h[3] < 1.0e30 && (h[2] - FLOOR as f32).abs() < 1e-2)
                .map(|(s, h)| (*s as f32, [h[0].floor() as i32, h[1].floor() as i32]))
                .collect(),
        )
    }

    /// The canopy's occupied bricks, projected down: (x >> 3, y >> 3).
    fn canopy_bricks() -> std::collections::HashSet<[i32; 2]> {
        let mut bricks = std::collections::HashSet::new();
        for z in CANOPY[2] - CANOPY_R..=CANOPY[2] + CANOPY_R {
            for y in CANOPY[1] - CANOPY_R..=CANOPY[1] + CANOPY_R {
                for x in CANOPY[0] - CANOPY_R..=CANOPY[0] + CANOPY_R {
                    if canopy_at([x, y, FLOOR + z]) {
                        bricks.insert([x >> 3, y >> 3]);
                    }
                }
            }
        }
        bricks
    }

    /// The canopy's leaf voxels in column (x, y): the path in voxels a zenith ray crosses.
    fn canopy_depth(v: [i32; 2]) -> i32 {
        (CANOPY[2] - CANOPY_R..=CANOPY[2] + CANOPY_R)
            .filter(|&z| canopy_at([v[0], v[1], FLOOR + z]))
            .count() as i32
    }

    /// Plan 0086 Phase 14 bar 3, as a reading: on the canopy rig with the leaf's style
    /// SHADOW_ABSORB at sigma 0.15 a voxel, against the open value (no canopy): over the floor
    /// voxels within 2 of the canopy's centre column, the mean ratio to open beside
    /// `exp(-sigma x)` for their mean leaf depth `x` and the mean of `exp(-sigma x)` per column;
    /// inside the projection of the canopy's occupied bricks, the share of pixels over 50 % and
    /// over 1 % of open, and the same over the columns of leaf depth 0; and of the pixels more
    /// than 8 voxels outside that projection, the share off open by more than 1 % (the
    /// silhouette). Prints them.
    #[test]
    #[ignore = "reading"]
    fn a_canopy_absorbs_by_path_length() {
        const SIGMA: f32 = 0.15;
        let Some(open) = canopy_floor(false, 0) else {
            return;
        };
        let open_v = open.iter().map(|p| f64::from(p.0)).sum::<f64>() / open.len() as f64;
        let px = canopy_floor(true, vxe_render::shadow_absorb(SIGMA)).expect("the adapter");
        let sigma = f64::from(SIGMA);
        let near = |v: [i32; 2]| (v[0] - CANOPY[0]).abs() <= 2 && (v[1] - CANOPY[1]).abs() <= 2;
        let centre: Vec<(f64, i32)> = px
            .iter()
            .filter(|p| near(p.1))
            .map(|p| (f64::from(p.0) / open_v, canopy_depth(p.1)))
            .collect();
        let n = centre.len().max(1) as f64;
        let ratio = centre.iter().map(|c| c.0).sum::<f64>() / n;
        let x = centre.iter().map(|c| f64::from(c.1)).sum::<f64>() / n;
        let per_column = centre.iter().map(|c| (-sigma * f64::from(c.1)).exp()).sum::<f64>() / n;
        println!(
            "reading: absorb, sigma {SIGMA}: centre {} px, mean ratio {ratio:.4}, mean depth {x:.3}, exp(-sigma x) {:.4}, mean of exp(-sigma x) per column {per_column:.4}",
            centre.len(),
            (-sigma * x).exp()
        );
        let bricks = canopy_bricks();
        let inside = |v: [i32; 2]| bricks.contains(&[v[0] >> 3, v[1] >> 3]);
        let ins: Vec<(f64, i32)> = px
            .iter()
            .filter(|p| inside(p.1))
            .map(|p| (f64::from(p.0) / open_v, canopy_depth(p.1)))
            .collect();
        let share = |set: &[(f64, i32)], r: f64| {
            set.iter().filter(|c| c.0 > r).count() as f64 / set.len().max(1) as f64
        };
        let empty: Vec<(f64, i32)> = ins.iter().copied().filter(|c| c.1 == 0).collect();
        println!(
            "reading: absorb inside the brick projection: {} px, over 50 % {:.4}, over 1 % {:.4}; of them {} px over leaf depth 0, over 50 % {:.4}",
            ins.len(),
            share(&ins, 0.5),
            share(&ins, 0.01),
            empty.len(),
            share(&empty, 0.5),
        );
        let far_out = |v: [i32; 2]| {
            bricks.iter().all(|b| {
                let dx = (b[0] * 8 - v[0]).max(v[0] - (b[0] * 8 + 7)).max(0);
                let dy = (b[1] * 8 - v[1]).max(v[1] - (b[1] * 8 + 7)).max(0);
                dx.max(dy) > 8
            })
        };
        let outs: Vec<f64> = px
            .iter()
            .filter(|p| far_out(p.1))
            .map(|p| f64::from(p.0) / open_v)
            .collect();
        let off = outs.iter().filter(|&&v| (v - 1.0).abs() > 0.01).count();
        println!(
            "reading: absorb outside by 8: {} px, off open by 1 % {off}",
            outs.len()
        );
    }

    /// Plan 0086 Phase 2 bars 1 and 2, as readings: on the canopy rig, the floor pixels inside
    /// the projection of the canopy's occupied bricks, with SHADOW_AS_BRICK set and clear: the
    /// share over 1 % and over 50 % of the open value (the same pose with no canopy); and with
    /// it set, of the floor pixels more than 8 voxels outside that projection, the share off the
    /// open value by more than 1 %. Prints them.
    #[test]
    #[ignore = "reading"]
    fn a_flagged_canopy_shadows_as_its_bricks() {
        let Some(open) = canopy_floor(false, 0) else {
            return;
        };
        let open_v = open.iter().map(|p| f64::from(p.0)).sum::<f64>() / open.len() as f64;
        let bricks = canopy_bricks();
        let inside = |v: [i32; 2]| bricks.contains(&[v[0] >> 3, v[1] >> 3]);
        // Over 8 voxels from every projected brick's square.
        let far_out = |v: [i32; 2]| {
            bricks.iter().all(|b| {
                let dx = (b[0] * 8 - v[0]).max(v[0] - (b[0] * 8 + 7)).max(0);
                let dy = (b[1] * 8 - v[1]).max(v[1] - (b[1] * 8 + 7)).max(0);
                dx.max(dy) > 8
            })
        };
        println!(
            "reading: canopy rig, {} occupied brick columns, open value {open_v:.4} over {} px",
            bricks.len(),
            open.len()
        );
        for styled in [true, false] {
            let style = if styled { vxe_render::SHADOW_AS_BRICK } else { 0 };
            let px = canopy_floor(true, style).expect("the adapter");
            let ins: Vec<f64> = px
                .iter()
                .filter(|p| inside(p.1))
                .map(|p| f64::from(p.0) / open_v)
                .collect();
            let over =
                |r: f64| ins.iter().filter(|&&v| v > r).count() as f64 / ins.len().max(1) as f64;
            let outs: Vec<f64> = px
                .iter()
                .filter(|p| far_out(p.1))
                .map(|p| f64::from(p.0) / open_v)
                .collect();
            let off = outs.iter().filter(|&&v| (v - 1.0).abs() > 0.01).count();
            println!(
                "reading: SHADOW_AS_BRICK {}: inside {} px, over 1 % {:.4}, over 50 % {:.4}; outside by 8 {} px, off open by 1 % {off}",
                if styled { "set" } else { "clear" },
                ins.len(),
                over(0.01),
                over(0.5),
                outs.len(),
            );
        }
    }

    /// Per frame the history counters, then the direct accumulation's luminance and the lighting
    /// grid's hit positions.
    type FarRun = (Vec<[u32; 4]>, Vec<f32>, Vec<[f32; 4]>);

    /// A far frame of `world` at `cam`: 320 x 180 under FSR 2 at 1.5 (480 x 270), half lighting,
    /// the store counters on, the key light alone (lamp off, no sky ray, filter on), manual
    /// exposure 1, `sky`, the stair rules hooked off with `no_stair`; `frames` frames, then per
    /// frame of 64 more the history counters and, at the last, the direct accumulation's
    /// luminance and the lighting grid's hit positions.
    fn far_run(world: &World, cam: &Camera, sky_: Sky, no_stair: bool) -> Option<FarRun> {
        let (device, queue) = device()?;
        let gpu = mirror(&device, &queue, world, None);
        let mut frame = Frame::with_sizes(&device, [320, 180], [480, 270]);
        frame.set_upscaler_kind(
            &device,
            vxe_render::UpscalerKind::Fsr2(vxe_render::Fsr2Settings::default()),
        );
        frame.set_lighting(Lighting {
            lamp: false,
            exposure: Exposure::Manual(1.0),
            sky_rays: 0,
            ..Lighting::default()
        });
        frame.set_light_hooks(vxe_render::LightHooks {
            no_stair,
            ..Default::default()
        });
        frame.set_count_store(true);
        frame.set_resolution(&device, LightingResolution::Full);
        // The fly-through's default far normal.
        frame.set_look(vxe_render::LookParams {
            far_normal: [2.0, 6.0],
            ..vxe_render::LookParams::default()
        });
        frame.set_sky(Some(sky_));
        let light = LightParams::headlamp();
        for _ in 0..16 {
            frame.step(&device, &queue, &gpu, cam, &light);
        }
        let mut counts = Vec::new();
        for _ in 0..64 {
            frame.step(&device, &queue, &gpu, cam, &light);
            counts.push(frame.read_history_rejections(&device, &queue));
        }
        let lum = vxe_render::debug::read_rgba16f(&device, &queue, frame.direct_accumulated())
            .iter()
            .map(|p| p[3])
            .collect();
        let hits = vxe_render::debug::read_rgba32f(&device, &queue, frame.light_hit_positions());
        Some((counts, lum, hits))
    }

    /// A camera at `eye` looking at `at`.
    fn looking(eye: [f32; 3], at: [f32; 3]) -> Camera {
        let d = Vec3::from(at) - Vec3::from(eye);
        Camera {
            position: Vec3::from(eye),
            yaw: d.y.atan2(d.x),
            pitch: (d.z / d.length()).asin(),
            vfov: 60f32.to_radians(),
        }
    }

    /// Plan 0086 Phase 3 bar 1, as a reading: a staircase of 64 one-voxel steps up along +x
    /// (rock under `z = FLOOR + clamp(x - 160, 0, 64)` over a 384 x 192 floor) seen from about
    /// 220 voxels (0.7 pixels a voxel at 320 wide, the far normal's full weight), FSR 2 at 1.5,
    /// a still camera, a zenith sun: the share of the history test's hits whose history reset per
    /// frame over 64 frames, by cause, with the stair rules hooked off and on. Prints them.
    #[test]
    #[ignore = "reading"]
    fn a_far_staircase_keeps_its_history() {
        let f = |p: [i32; 3]| {
            if p[2] < FLOOR + (p[0] - 160).clamp(0, 64) {
                ROCK
            } else {
                AIR
            }
        };
        let world = World::generate(
            ChunkExtent::new([0, 0, 0], [6, 3, 2]),
            &FnSource(f),
            &table(),
        );
        let cam = looking(
            [40.0, 96.0, FLOOR as f32 + 120.0],
            [192.0, 96.0, FLOOR as f32 + 32.0],
        );
        let s = sky(dir(90.0, 0.0), dir(-90.0, 0.0), [[1.0; 3], [0.0; 3]]);
        for no_stair in [true, false] {
            let Some((counts, _, _)) = far_run(&world, &cam, s, no_stair) else {
                return;
            };
            let t: [u64; 4] = std::array::from_fn(|k| counts.iter().map(|w| u64::from(w[k])).sum());
            let max = counts
                .iter()
                .map(|w| f64::from(w[1] + w[2] + w[3]) / f64::from(w[0].max(1)))
                .fold(0.0, f64::max);
            println!(
                "reading: far staircase, stair rules {}: tested {} a frame, reset {:.4} (material {:.4} normal {:.4} other {:.4}), worst frame {max:.4}",
                if no_stair { "off" } else { "on" },
                t[0] / 64,
                (t[1] + t[2] + t[3]) as f64 / t[0].max(1) as f64,
                t[1] as f64 / t[0].max(1) as f64,
                t[2] as f64 / t[0].max(1) as f64,
                t[3] as f64 / t[0].max(1) as f64,
            );
        }
    }

    /// Plan 0086 Phase 3 bar 2, as a reading: a one-voxel wall 40 voxels tall (x = 256, y 32 to
    /// 160) on a 384 x 192 floor, seen from about 220 voxels on its -x side (0.7 pixels a voxel),
    /// the sun 20 degrees up on its +x side (its shadow falls toward the camera), lighting at
    /// full resolution: of the floor pixels 1 to 3 rows under the wall's lowest pixel in each
    /// column, the share whose key light reads over 10 % of the open floor's (the floor behind
    /// the wall, x over 262), with the stair rules (the lift) hooked off and on. Prints them.
    #[test]
    #[ignore = "reading"]
    fn a_thin_far_wall_does_not_leak() {
        let f = |p: [i32; 3]| {
            let wall = p[0] == 256 && (32..160).contains(&p[1]) && p[2] < FLOOR + 40;
            if p[2] < FLOOR || wall { ROCK } else { AIR }
        };
        let world = World::generate(
            ChunkExtent::new([0, 0, 0], [6, 3, 2]),
            &FnSource(f),
            &table(),
        );
        let cam = looking(
            [56.0, 96.0, FLOOR as f32 + 90.0],
            [256.0, 96.0, FLOOR as f32],
        );
        let s = sky(dir(20.0, 90.0), dir(-20.0, 270.0), [[1.0; 3], [0.0; 3]]);
        for no_stair in [true, false] {
            let Some((_, lum, hits)) = far_run(&world, &cam, s, no_stair) else {
                return;
            };
            let floor = |h: &[f32; 4]| h[3] < 1.0e30 && (h[2] - FLOOR as f32).abs() < 1e-2;
            let wall = |h: &[f32; 4]| h[3] < 1.0e30 && h[2] > FLOOR as f32 + 0.5 && h[0] < 257.5;
            // Open: the lit floor behind the wall.
            let open: Vec<f32> = lum
                .iter()
                .zip(&hits)
                .filter(|(_, h)| floor(h) && h[0] > 262.0)
                .map(|(l, _)| *l)
                .collect();
            let open_v = open.iter().sum::<f32>() / open.len().max(1) as f32;
            // Per column, the floor pixels 1 to 3 rows under the wall's lowest pixel.
            let (w, h) = (320usize, 180usize);
            let mut band: Vec<f32> = Vec::new();
            for x in 0..w {
                let Some(base) = (0..h).rev().find(|&y| wall(&hits[y * w + x])) else {
                    continue;
                };
                for y in base + 1..=(base + 3).min(h - 1) {
                    let i = y * w + x;
                    if floor(&hits[i]) && hits[i][0] < 256.0 {
                        band.push(lum[i] / open_v);
                    }
                }
            }
            let leak = band.iter().filter(|&&v| v > 0.1).count();
            println!(
                "reading: thin far wall, stair rules {}: open {open_v:.4} over {} px; band {} px, over 10 % {leak} ({:.4})",
                if no_stair { "off" } else { "on" },
                open.len(),
                band.len(),
                leak as f64 / band.len().max(1) as f64
            );
        }
    }

    /// One frame of [`pair_run`]: the pair and the visibility sample per lighting pixel.
    type PairFrame = (Vec<[f32; 4]>, Vec<[u8; 4]>);

    /// [`pair_run`]'s edit: per frame index, the box it changed in the world, if any.
    type PairEdit<'a> = dyn FnMut(u32, &World) -> Option<[[i32; 3]; 2]> + 'a;

    /// The visibility pair's rig (ADR-0070): `world` mirrored, a frame of SIZE at full lighting
    /// resolution with no jitter, filter off, exposure 1, one sky ray, the coarse walk,
    /// `hooks`, under `Sky::uniform(1.0)` (no key light). `frames` frames of `cam`, `edit`
    /// called before each with the frame's index, the world and the mirror (it returns an edit
    /// box to invalidate); per frame, the pair and the visibility sample at every lighting pixel.
    fn pair_run(
        world: &World,
        cam: &Camera,
        frames: u32,
        hooks: vxe_render::LightHooks,
        edit: &mut PairEdit<'_>,
    ) -> Option<Vec<PairFrame>> {
        let (device, queue) = device()?;
        let mut gpu = mirror(&device, &queue, world, None);
        let mut f = frame(&device, SIZE, ShadowWalk::Coarse);
        f.set_jitter(&device, false);
        f.set_lighting(Lighting {
            sky_rays: 1,
            ..f.lighting()
        });
        f.set_light_hooks(hooks);
        f.set_sky(Some(Sky {
            altitude_zero_z: FLOOR as f32,
            ..Sky::uniform(1.0)
        }));
        let mut out = Vec::new();
        for i in 0..frames {
            if let Some([min, max]) = edit(i, world) {
                let mut enc =
                    device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
                gpu.sync(world, &queue, &mut enc).expect("the mirror fits");
                queue.submit([enc.finish()]);
                f.invalidate(min, max);
            }
            f.step(&device, &queue, &gpu, cam, &LightParams::headlamp());
            let pair = vxe_render::debug::read_rgba16f(&device, &queue, f.visibility_pair());
            let vis = vxe_render::debug::read_rgba8(&device, &queue, f.visibility_samples());
            let vis = vis.chunks(4).map(|c| [c[0], c[1], c[2], c[3]]).collect();
            out.push((pair, vis));
        }
        Some(out)
    }

    /// The centre lighting pixel's index of a SIZE frame.
    fn centre() -> usize {
        let [w, h] = SIZE.map(|v| v as usize);
        (h / 2) * w + w / 2
    }

    /// Plan 0086 Phase 5 bar 1: a floor point at the foot of a half-plane roof's edge 20 voxels
    /// up (the roof over y >= 512 across the 1024 x 1024 floor, a hit rate of 0.5), under
    /// `Sky::uniform(1.0)`, the ray forced on: the pair's mean at the centre pixel after frame
    /// 64, and its largest frame-to-frame change over frames 65 to 128 against the running
    /// mean's step at the cap, 1 / 64. Printed.
    #[test]
    #[ignore = "reading"]
    fn the_visibility_pairs_mean_holds_still() {
        let roof = |p: [i32; 3]| p[2] == FLOOR + 20 && p[1] >= 512;
        let world = floor_world([16, 16, 1], roof);
        let cam = looking(
            [512.0, 470.0, FLOOR as f32 + 10.0],
            [512.0, 512.0, FLOOR as f32],
        );
        let hooks = vxe_render::LightHooks {
            sky_ray_always: true,
            ..Default::default()
        };
        let Some(run) = pair_run(&world, &cam, 128, hooks, &mut |_, _| None) else {
            return;
        };
        let c = centre();
        let r: Vec<f32> = run.iter().map(|(p, _)| p[c][0]).collect();
        let hits = run.iter().filter(|(_, v)| v[c][0] > 127).count();
        let late = &r[64..];
        let mean = late.iter().sum::<f32>() / late.len() as f32;
        let step = late
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0f32, f32::max);
        println!(
            "reading: half-plane edge: hit rate over 128 frames {:.3}; pair mean at frame 64 \
             {:.4}, over 65..128 {mean:.4}; largest change a frame {step:.4} (1/64 = {:.4}); age \
             {}",
            hits as f32 / 128.0,
            r[63],
            1.0 / 64.0,
            run[127].0[c][3]
        );
    }

    /// Plan 0086 Phase 5 bar 2: a floor point under a roof 20 voxels up with a 6 x 6 hole
    /// straight above it (the form factor `36 / (400 pi) = 0.029`), the ray forced on: the
    /// pair's largest mean over frames 9 to 128 and its mean over them (the clamp's dark bias
    /// against 0.029), with the clamp and without it. Printed.
    #[test]
    #[ignore = "reading"]
    fn a_rare_hit_is_clamped() {
        let (cx, cy) = (512, 512);
        let roof = move |p: [i32; 3]| {
            p[2] == FLOOR + 20
                && !((cx - 3..cx + 3).contains(&p[0]) && (cy - 3..cy + 3).contains(&p[1]))
        };
        let world = floor_world([16, 16, 1], roof);
        let cam = looking(
            [512.0, 480.0, FLOOR as f32 + 10.0],
            [512.0, 512.0, FLOOR as f32],
        );
        for no_clamp in [false, true] {
            let hooks = vxe_render::LightHooks {
                sky_ray_always: true,
                no_clamp,
                ..Default::default()
            };
            let Some(run) = pair_run(&world, &cam, 128, hooks, &mut |_, _| None) else {
                return;
            };
            let c = centre();
            let r: Vec<f32> = run.iter().map(|(p, _)| p[c][0]).collect();
            let hits = run.iter().filter(|(_, v)| v[c][0] > 127).count();
            let tail = &r[8..];
            let max = tail.iter().copied().fold(0f32, f32::max);
            let mean = tail.iter().sum::<f32>() / tail.len() as f32;
            println!(
                "reading: 6 x 6 hole, clamp {}: {hits} hits in 128 frames; pair over frames 9..128: \
                 largest {max:.4}, mean {mean:.4} ({:.3} of 0.029)",
                if no_clamp { "off" } else { "on" },
                mean / 0.029
            );
        }
    }

    /// Plan 0086 Phase 5 bar 3: Plan 0032 Phase 5's 4 x 4 pillar, 30 voxels tall, moved one
    /// voxel a frame along +x across the open floor under `Sky::uniform(1.0)` (an edit a frame,
    /// its box invalidated), the ray forced on: for every lighting pixel the pillar's sky
    /// shadow leaves (its pair mean under 0.9 at frame 32 and its sample open from frame 33 on
    /// within the open band), the frames until the mean reads at least 0.9 of the open floor's.
    /// The distribution, printed.
    #[test]
    #[ignore = "reading"]
    fn a_moving_shadow_edge_resets_the_history() {
        let world = floor_world([4, 4, 1], |_| false);
        let x0 = 100i64;
        let pillar = |x: i64, m: MaterialId| {
            EditOp::box_(
                Fixed3([x, 126, i64::from(FLOOR)].map(|v| v * FIXED_ONE)),
                Fixed3([x + 3, 129, i64::from(FLOOR) + 29].map(|v| v * FIXED_ONE)),
                Mode::Fill(m),
            )
        };
        world.apply(&pillar(x0, ROCK)).expect("resident");
        let cam = looking(
            [120.0, 80.0, FLOOR as f32 + 60.0],
            [120.0, 128.0, FLOOR as f32],
        );
        let hooks = vxe_render::LightHooks {
            sky_ray_always: true,
            ..Default::default()
        };
        let mut edit = |i: u32, w: &World| {
            if !(32..64).contains(&i) {
                return None;
            }
            let x = x0 + i64::from(i - 32);
            w.apply(&pillar(x, AIR)).expect("resident");
            let r = w.apply(&pillar(x + 1, ROCK)).expect("resident");
            let (min, max) = r.aabb.expect("a change");
            Some([[min.0[0] - 1, min.0[1], min.0[2]], max.0])
        };
        let Some(run) = pair_run(&world, &cam, 128, hooks, &mut edit) else {
            return;
        };
        let n = run[0].0.len();
        let mut frames = Vec::new();
        let mut never = 0;
        for i in 0..n {
            // Shadowed at frame 31 (the pillar still) and open at the end.
            let before = run[31].0[i][0];
            let after = run[127].0[i][0];
            if !(before < 0.9 && after > 0.97) {
                continue;
            }
            // The first frame from which the sample stays open.
            let Some(open) = (32..128)
                .rev()
                .find(|&f| run[f].1[i][0] < 255)
                .map(|f| f + 1)
            else {
                continue;
            };
            if open >= 120 {
                continue;
            }
            match (open..128).find(|&f| run[f].0[i][0] >= 0.9) {
                Some(f) => frames.push(f - open + 1),
                None => never += 1,
            }
        }
        frames.sort_unstable();
        let q = |p: usize| frames.get(frames.len() * p / 100).copied().unwrap_or(0);
        println!(
            "reading: moving pillar: {} pixels the shadow left; frames to 0.9 of open: median {}, \
             p90 {}, max {}, within 12 {}; never {never}",
            frames.len(),
            q(50),
            q(90),
            frames.last().copied().unwrap_or(0),
            frames.iter().filter(|&&f| f <= 12).count()
        );
    }

    /// The slab rig's frame-to-frame change in the penumbra band (Plan 0086 Phase 6's
    /// reading): Plan 0032 Phase 5 bar 3's 16 x 16 slab 20 voxels over the floor, the camera 8
    /// voxels up looking straight down, 128 x 128 internal under FSR 2 at 1.5 to 192 x 192 (6
    /// display pixels a voxel), filter on, the defaults otherwise. Under a zenith sun of
    /// `radius_deg` (`Some`), or the headlamp and no sky (`None`), at manual `exposure`: after 64
    /// settle frames, over 64 frames, the mean and 99th percentile of the display's per-pixel
    /// change (the largest channel, levels of 255) over the band within 1.5 voxels of the
    /// slab's +x edge (x 40) along its middle 12 voxels, and the band's mean green level.
    fn penumbra_band(radius_deg: Option<f32>, exposure: f32) -> Option<(f64, f64, f64)> {
        let (device, queue) = device()?;
        let slab = |p: [i32; 3]| {
            p[2] == FLOOR + 20 && (24..40).contains(&p[0]) && (24..40).contains(&p[1])
        };
        let world = floor_world([1, 1, 1], slab);
        let gpu = mirror(&device, &queue, &world, None);
        let cam = Camera {
            position: Vec3::new(40.0, 32.0, FLOOR as f32 + 8.0),
            yaw: FRAC_PI_2,
            pitch: -FRAC_PI_2 + 1e-4,
            vfov: 2.0 * 2f32.atan(),
        };
        let mut f = Frame::with_sizes(&device, [128, 128], [192, 192]);
        f.set_upscaler_kind(
            &device,
            vxe_render::UpscalerKind::Fsr2(vxe_render::Fsr2Settings::default()),
        );
        f.set_lighting(Lighting {
            lamp: radius_deg.is_none(),
            exposure: Exposure::Manual(exposure),
            sky_rays: 0,
            ..Lighting::default()
        });
        f.set_sky(radius_deg.map(|r| {
            let mut s = sky(dir(90.0, 0.0), dir(-90.0, 0.0), [[1.0; 3], [0.0; 3]]);
            s.sun.angular_radius = r.to_radians();
            s
        }));
        let light = LightParams::headlamp();
        for _ in 0..64 {
            f.step(&device, &queue, &gpu, &cam, &light);
        }
        // The band's display pixels: 6 a voxel about the centre (x 40, y 32).
        let band: Vec<usize> = (0..192usize * 192)
            .filter(|&i| {
                let (x, y) = ((i % 192) as f32, (i / 192) as f32);
                let (vx, vy) = (40.0 + (x + 0.5 - 96.0) / 6.0, 32.0 + (y + 0.5 - 96.0) / 6.0);
                (vx - 40.0).abs() <= 1.5 && (26.0..38.0).contains(&vy)
            })
            .collect();
        let read = |f: &Frame| vxe_render::debug::read_rgba8(&device, &queue, f.target());
        let mut last = read(&f);
        let level = band
            .iter()
            .map(|&i| f64::from(last[4 * i + 1]))
            .sum::<f64>()
            / band.len() as f64;
        let mut changes = Vec::new();
        for _ in 0..64 {
            f.step(&device, &queue, &gpu, &cam, &light);
            let now = read(&f);
            for &i in &band {
                let d = (0..3)
                    .map(|k| now[4 * i + k].abs_diff(last[4 * i + k]))
                    .max()
                    .unwrap_or(0);
                changes.push(f64::from(d));
            }
            last = now;
        }
        changes.sort_by(f64::total_cmp);
        let mean = changes.iter().sum::<f64>() / changes.len() as f64;
        Some((mean, changes[changes.len() * 99 / 100], level))
    }

    /// Plan 0086 Phase 6's reading: the penumbra band's change at 3 degrees (and 5) against
    /// the lamp's on the same floor, each exposure pinned so the band's mean level is near 118
    /// (two passes, the second scaled by the display's gamma of 2.2). Printed.
    #[test]
    #[ignore = "reading"]
    fn the_penumbra_against_the_lamp() {
        for r in [Some(3.0), Some(5.0), None] {
            let mut ex = 1.0f32;
            let Some((_, _, level)) = penumbra_band(r, ex) else {
                return;
            };
            ex *= ((118.0 / level.max(1.0)) as f32).powf(2.2);
            let Some((mean, p99, level)) = penumbra_band(r, ex) else {
                return;
            };
            let what = r.map_or("the headlamp, no sky".to_string(), |d| {
                format!("sun {d} degrees")
            });
            println!(
                "reading: penumbra band, {what}: exposure {ex:.4}, mean level {level:.1}, change mean {mean:.3}, p99 {p99}"
            );
        }
    }

    /// The key light's visibility across the slab rig's +x edge (`penumbra_band`'s slab and
    /// camera, native 128 x 128, the lighting at full resolution, the filter off, exposure 1,
    /// no sky rays) under a zenith sun of `radius_deg`: per bin of a quarter voxel in x over
    /// 36..46 (the lighting hits with y in 30..34), the light pass's direct sample over the open
    /// floor's (x 46..50), with `penumbra`; the disc's as a mean over 64 frames, the approach's
    /// at frame 1, and the approach's largest change of a bin from frame 1 to frame 2.
    fn edge_profile(radius_deg: f32, penumbra: vxe_render::Penumbra) -> Option<(Vec<f64>, f64)> {
        let (device, queue) = device()?;
        let slab = |p: [i32; 3]| {
            p[2] == FLOOR + 20 && (24..40).contains(&p[0]) && (24..40).contains(&p[1])
        };
        let world = floor_world([1, 1, 1], slab);
        let gpu = mirror(&device, &queue, &world, None);
        let cam = Camera {
            position: Vec3::new(40.0, 32.0, FLOOR as f32 + 8.0),
            yaw: FRAC_PI_2,
            pitch: -FRAC_PI_2 + 1e-4,
            vfov: 2.0 * 2f32.atan(),
        };
        let mut f = frame(&device, [128, 128], ShadowWalk::Coarse);
        f.set_lighting(Lighting {
            penumbra,
            ..f.lighting()
        });
        let mut s = sky(dir(90.0, 0.0), dir(-90.0, 0.0), [[1.0; 3], [0.0; 3]]);
        s.sun.angular_radius = radius_deg.to_radians();
        f.set_sky(Some(s));
        const BINS: usize = 40;
        let frames = if penumbra == vxe_render::Penumbra::Disc {
            64
        } else {
            2
        };
        let mut runs = Vec::new();
        let light = LightParams::headlamp();
        for _ in 0..frames {
            f.step(&device, &queue, &gpu, &cam, &light);
            let d = vxe_render::debug::read_rgba16f(&device, &queue, f.direct());
            let h = vxe_render::debug::read_rgba32f(&device, &queue, f.light_hit_positions());
            let (mut sum, mut n) = ([0.0f64; BINS], [0u32; BINS]);
            let (mut open, mut on) = (0.0f64, 0u32);
            for (p, q) in h.iter().zip(&d) {
                if p[3] > 1.0e30 || !(30.0..34.0).contains(&p[1]) {
                    continue;
                }
                if (46.0..50.0).contains(&p[0]) {
                    open += f64::from(q[3]);
                    on += 1;
                }
                let b = ((p[0] - 36.0) * 4.0).floor();
                if (0.0..BINS as f32).contains(&b) {
                    sum[b as usize] += f64::from(q[3]);
                    n[b as usize] += 1;
                }
            }
            let open = open / f64::from(on.max(1));
            runs.push(
                (0..BINS)
                    .map(|i| sum[i] / f64::from(n[i].max(1)) / open.max(1e-9))
                    .collect::<Vec<f64>>(),
            );
        }
        let mean: Vec<f64> = (0..BINS)
            .map(|i| runs.iter().map(|r| r[i]).sum::<f64>() / runs.len() as f64)
            .collect();
        if penumbra == vxe_render::Penumbra::Disc {
            return Some((mean, 0.0));
        }
        let change = (0..BINS)
            .map(|i| (runs[1][i] - runs[0][i]).abs())
            .fold(0.0, f64::max);
        Some((runs[0].clone(), change))
    }

    /// The first x (voxels) at which `profile` (bins of a quarter voxel from x 36) reaches
    /// `level`, linear between bins' centres.
    fn crossing(profile: &[f64], level: f64) -> Option<f64> {
        let x = |i: usize| 36.0 + (i as f64 + 0.5) / 4.0;
        (1..profile.len()).find_map(|i| {
            let (a, b) = (profile[i - 1], profile[i]);
            (a < level && b >= level).then(|| x(i - 1) + (level - a) / (b - a) * 0.25)
        })
    }

    /// Plan 0086 Phase 7 bar 1: the approach penumbra's profile across the slab's edge at one
    /// frame against the disc's 64-frame mean, at 5 and 3 degrees: the 5 %, 50 % and 95 %
    /// crossings of each, the rise's width (5 % to 95 %), the largest gap between the profiles
    /// and the approach's change from frame 1 to 2. Printed; lavapipe.
    #[test]
    #[ignore = "reading"]
    fn approach_penumbra_against_the_disc() {
        use vxe_render::Penumbra;
        for r in [5.0, 3.0] {
            let Some((disc, _)) = edge_profile(r, Penumbra::Disc) else {
                return;
            };
            let Some((approach, change)) = edge_profile(r, Penumbra::Approach) else {
                return;
            };
            for (name, p) in [("disc", &disc), ("approach", &approach)] {
                let c: Vec<String> = [0.05, 0.5, 0.95]
                    .iter()
                    .map(|&l| crossing(p, l).map_or("-".into(), |x| format!("{x:.2}")))
                    .collect();
                let width = crossing(p, 0.05)
                    .zip(crossing(p, 0.95))
                    .map_or("-".into(), |(a, b)| format!("{:.2}", b - a));
                println!(
                    "reading: edge at {r} degrees, {name}: crossings 5/50/95 % at x {}, width {width}",
                    c.join(" / ")
                );
            }
            let gap = disc
                .iter()
                .zip(&approach)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0, f64::max);
            let row = |p: &[f64]| {
                p.iter()
                    .step_by(2)
                    .map(|v| format!("{v:.2}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            println!(
                "reading: edge at {r} degrees: largest gap {gap:.3}, approach change frame 1 to 2 {change:.4}\n  disc     {}\n  approach {}",
                row(&disc),
                row(&approach)
            );
        }
    }

    /// The stair of Plan 0086 Phase 13: 64 one-voxel steps up along +x (rock under `z = FLOOR +
    /// clamp(x - 96, 0, 64)`) on a 256 x 256 floor, under a roof slab 4 thick at `FLOOR + 80`
    /// over `y >= 96` (a half-plane, so `+z` and the horizontal differ by design).
    fn stair_world() -> World {
        let f = |p: [i32; 3]| {
            let stair = p[2] < FLOOR + (p[0] - 96).clamp(0, 64);
            let roof = p[1] >= 96 && (FLOOR + 80..FLOOR + 84).contains(&p[2]);
            if p[2] < FLOOR || stair || roof {
                ROCK
            } else {
                AIR
            }
        };
        World::generate(
            ChunkExtent::new([0, 0, 0], [4, 4, 2]),
            &FnSource(f),
            &table(),
        )
    }

    /// Plan 0086 Phase 13 bar 1: a stair reads one slope. `stair_world` baked whole under
    /// `Sky::uniform(1.0)` (the default bake settings), seen from `(60, 64, FLOOR + 70)` toward
    /// `(128, 64, FLOOR + 32)` at 320 x 180, lighting at full resolution, filter off, exposure 1,
    /// one sky ray, after 4 frames: the stored part's mean luminance (`accumulated`, where the
    /// baked sky term lands) over the treads (hits on a `+z` face at a stair's height) and the
    /// risers (on a `-x` face over the floor), their ratio, and the open floor's (`z = FLOOR`,
    /// `x < 90`) under `sky_normal` face and smoothed, each at `sky_bounce` 0 and 0.3. On the
    /// fastest Vulkan adapter (`VK_DRIVER_FILES` picks another).
    #[test]
    #[ignore = "reading"]
    fn a_one_voxel_stair_reads_one_slope() {
        let Some((device, queue, adapter)) =
            vxe_render::test_support::high_performance_device(64 << 20, wgpu::Features::empty())
        else {
            return;
        };
        println!("reading: a_one_voxel_stair_reads_one_slope on {adapter}");
        let world = stair_world();
        let cam = looking(
            [60.0, 64.0, FLOOR as f32 + 70.0],
            [128.0, 64.0, FLOOR as f32 + 32.0],
        );
        for (normal, bounce) in [
            (vxe_render::SkyNormal::Face, 0.0),
            (vxe_render::SkyNormal::Smoothed, 0.0),
            (vxe_render::SkyNormal::Face, 0.3),
            (vxe_render::SkyNormal::Smoothed, 0.3),
        ] {
            let mut gpu = mirror(&device, &queue, &world, None);
            let mut f = frame(&device, [320, 180], ShadowWalk::Full);
            f.set_lighting(Lighting {
                sky_rays: 1,
                sky_normal: normal,
                sky_bounce: bounce,
                ..f.lighting()
            });
            f.set_sky(Some(Sky {
                altitude_zero_z: FLOOR as f32,
                ..Sky::uniform(1.0)
            }));
            f.bake_until(
                &device,
                &queue,
                &mut gpu,
                &world,
                world.extent(),
                &vxe_render::BakeSettings::default(),
                &mut |_| {},
            )
            .expect("the store fits");
            for _ in 0..4 {
                f.step(&device, &queue, &gpu, &cam, &LightParams::headlamp());
            }
            let acc = vxe_render::debug::read_rgba16f(&device, &queue, f.accumulated());
            let hits = vxe_render::debug::read_rgba32f(&device, &queue, f.light_hit_positions());
            let (mut tread, mut riser, mut floor) = ((0.0, 0), (0.0, 0), (0.0, 0));
            let on = |x: f32| (x - x.round()).abs() < 1e-3;
            for (a, h) in acc.iter().zip(&hits) {
                if h[3] >= 1.0e30 {
                    continue;
                }
                let l = f64::from(a[1]);
                let add = |s: &mut (f64, u32)| {
                    s.0 += l;
                    s.1 += 1;
                };
                let z = h[2] - FLOOR as f32;
                if on(h[2]) && z > 0.5 && (96.0..161.0).contains(&h[0]) {
                    add(&mut tread);
                } else if on(h[0]) && z > 0.5 && (96.0..161.0).contains(&h[0]) && !on(h[2]) {
                    add(&mut riser);
                } else if on(h[2]) && z.abs() < 1e-3 && h[0] < 90.0 {
                    add(&mut floor);
                }
            }
            let mean = |s: (f64, u32)| s.0 / f64::from(s.1.max(1));
            println!(
                "reading: stair, sky normal {}, bounce {bounce}: tread {:.4} ({} px), riser {:.4} ({} px), ratio {:.3}; open floor {:.4} ({} px)",
                normal.name(),
                mean(tread),
                tread.1,
                mean(riser),
                riser.1,
                mean(tread) / mean(riser).max(1e-9),
                mean(floor),
                floor.1
            );
        }
    }
}
