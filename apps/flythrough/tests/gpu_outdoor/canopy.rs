// `gpu_outdoor`'s canopy readings (Plan 0086 Phase 24b, the 86c window's defect B), included by
// `gpu_outdoor.rs` (kept apart for the file-size cap).

mod canopy {
    pub(super) mod reading {
        use super::super::support;
        use support::terms::Rig;
        use vxe_render::{Camera, Exposure, Frame, LightHooks, LightParams, Lighting};

        const INTERNAL: [u32; 2] = [320, 180];
        const OUT: [u32; 2] = [480, 270];
        /// The frames a run holds before it is read, after a history reset.
        const FRAMES: u32 = 128;
        /// The frames the long reference holds (at least), under [`REFERENCE_HISTORY`].
        const REFERENCE: u32 = 1200;
        /// The long reference's cap on the visibility pair's means (`Lighting::sky_history` and
        /// `key_history`).
        const REFERENCE_HISTORY: u32 = 1024;
        /// The pairs of frames the pop share reads, the last of a run's.
        const POP_PAIRS: u32 = 32;

        /// One arm of the split.
        #[derive(Clone, Copy)]
        struct Arm {
            name: &'static str,
            /// The store baked whole (else none, and `sky_rays` 0: the key light alone).
            baked: bool,
            hooks: LightHooks,
        }

        const ARMS: [Arm; 4] = [
            Arm {
                name: "both",
                baked: true,
                hooks: LightHooks {
                    key_off: false,
                    any_history: false,
                    no_stair: false,
                    sky_ray_always: false,
                    no_clamp: false,
                    sky_end_is_sky: false,
                    key_no_jitter: false,
                    column_prior: false,
                    no_fade: false,
                    pool: false,
                    seed_stale_f: false,
                    no_history: false,
                },
            },
            Arm {
                name: "key alone",
                baked: false,
                hooks: LightHooks {
                    key_off: false,
                    any_history: false,
                    no_stair: false,
                    sky_ray_always: false,
                    no_clamp: false,
                    sky_end_is_sky: false,
                    key_no_jitter: false,
                    column_prior: false,
                    no_fade: false,
                    pool: false,
                    seed_stale_f: false,
                    no_history: false,
                },
            },
            Arm {
                name: "sky alone",
                baked: true,
                hooks: LightHooks {
                    key_off: true,
                    any_history: false,
                    no_stair: false,
                    sky_ray_always: false,
                    no_clamp: false,
                    sky_end_is_sky: false,
                    key_no_jitter: false,
                    column_prior: false,
                    no_fade: false,
                    pool: false,
                    seed_stale_f: false,
                    no_history: false,
                },
            },
            Arm {
                name: "key alone, no jitter",
                baked: false,
                hooks: LightHooks {
                    key_off: false,
                    any_history: false,
                    no_stair: false,
                    sky_ray_always: false,
                    no_clamp: false,
                    sky_end_is_sky: false,
                    key_no_jitter: true,
                    column_prior: false,
                    no_fade: false,
                    pool: false,
                    seed_stale_f: false,
                    no_history: false,
                },
            },
        ];

        /// A pixel's class: the floor (no leaf or bark among the 8 voxels about the hit), a
        /// trunk (bark), or neither (a leaf, the sky).
        #[derive(Clone, Copy, PartialEq, Eq)]
        pub(in super::super) enum Class {
            Floor,
            Trunk,
            Other,
        }

        /// A frame stepped by hand with its step count (the jitter's phase follows it) and its
        /// display size.
        pub(in super::super) struct Run {
            f: Frame,
            steps: u32,
            out: [u32; 2],
        }

        impl Run {
            pub(in super::super) fn step(&mut self, rig: &Rig, cam: &Camera, light: &LightParams) {
                self.f.step(&rig.device, &rig.queue, &rig.gpu, cam, light);
                self.steps += 1;
            }
        }

        /// The defaults' frame on the forest rig under `sky` (none: the headlamp alone), FSR 2
        /// from [`INTERNAL`] to [`OUT`], voxel leaves, the exposure pinned at `exposure`, the
        /// store baked whole with `baked`, `sky_rays` 0 without.
        fn frame(
            rig: &mut Rig,
            sky: Option<vxe_render::sky::Sky>,
            baked: bool,
            exposure: Exposure,
        ) -> Run {
            frame_sized(rig, sky, baked, exposure, INTERNAL, OUT)
        }

        /// [`frame`] from `internal` to `out`.
        pub(in super::super) fn frame_sized(
            rig: &mut Rig,
            sky: Option<vxe_render::sky::Sky>,
            baked: bool,
            exposure: Exposure,
            internal: [u32; 2],
            out: [u32; 2],
        ) -> Run {
            let mut f = Frame::with_sizes(&rig.device, internal, out);
            f.set_upscaler_kind(
                &rig.device,
                vxe_render::UpscalerKind::Fsr2(vxe_render::Fsr2Settings::default()),
            );
            f.set_look(vxe_render::LookParams {
                far_normal: flythrough::LookPreset::DEFAULT.far_normal,
                ..vxe_render::LookParams::default()
            });
            f.set_material_styles(
                &rig.queue,
                &flythrough::outdoor_styles(flythrough::LeafShadow::Voxel),
            );
            let base = flythrough::default_lighting();
            f.set_lighting(Lighting {
                lamp: sky.is_none(),
                exposure,
                sky_rays: if baked || sky.is_none() {
                    base.sky_rays
                } else {
                    0
                },
                ..base
            });
            f.set_sky(sky);
            if baked && sky.is_some() {
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
            Run { f, steps: 0, out }
        }

        /// The settled exposure of the defaults under `sky` (or the headlamp alone): 64 frames
        /// under the outdoor auto exposure (instant, the default sky anchor).
        fn settled_exposure(rig: &mut Rig, cam: &Camera, sky: Option<vxe_render::sky::Sky>) -> f32 {
            let auto = Exposure::Auto(vxe_render::AutoExposure {
                min: flythrough::cli::OUTDOOR_MIN,
                max: flythrough::cli::OUTDOOR_MAX,
                sky_anchor_ev: Some(vxe_render::DEFAULT_SKY_ANCHOR_EV),
                ..vxe_render::AutoExposure::instant()
            });
            let mut r = frame(rig, sky, sky.is_some(), auto);
            for _ in 0..64 {
                r.step(rig, cam, &LightParams::headlamp());
            }
            let st = vxe_render::debug::read_buffer(&rig.device, &rig.queue, r.f.exposure_state(), 0, 32);
            vxe_render::ExposureState::from_bytes(&st).current
        }

        /// What a run's last frame reads: the display's largest channel, the class and the key
        /// mean `g` per display pixel, and over the last [`POP_PAIRS`] pairs the over-4 counts
        /// per class (floor, trunk) with the class's pixel count.
        pub(in super::super) struct Read {
            level: Vec<u8>,
            class: Vec<Class>,
            g: Vec<f32>,
            pops: [(u64, u64); 2],
            /// Per lighting pixel at the last frame: the class, the history's age (the moments'
            /// `z`) and the key mean `g`.
            pub(in super::super) light_class: Vec<Class>,
            pub(in super::super) light_age: Vec<f32>,
            light_g: Vec<f32>,
            /// The history rejections per frame over the last [`POP_PAIRS`] frames (tested,
            /// material, normal, other: [`Frame::read_history_rejections`]).
            pub(in super::super) rejections: Vec<[u32; 4]>,
            /// Per class (floor, trunk) the frame-to-frame display changes over the last
            /// [`POP_PAIRS`] pairs.
            diffs: [Vec<u8>; 2],
        }

        /// Resets `r`'s history, sets `hooks` and `history`, steps `frames` frames (then more
        /// until `align`'s `(phase, phases)` holds the step count at `phase` modulo `phases`)
        /// and reads the last [`POP_PAIRS`] + 1.
        #[allow(clippy::too_many_arguments, reason = "a reading's knobs")]
        fn hold(
            rig: &Rig,
            cam: &Camera,
            r: &mut Run,
            hooks: LightHooks,
            history: u32,
            frames: u32,
            align: Option<(u32, u32)>,
        ) -> Read {
            // The long reference holds both of the pair's means at `history`; a run keeps the
            // defaults' caps.
            let key = if history == REFERENCE_HISTORY {
                history
            } else {
                flythrough::default_lighting().key_history
            };
            hold_capped(rig, cam, r, hooks, history, key, frames, align)
        }

        /// [`hold`] with the key mean's cap `key` named.
        #[allow(clippy::too_many_arguments, reason = "a reading's knobs")]
        pub(in super::super) fn hold_capped(
            rig: &Rig,
            cam: &Camera,
            r: &mut Run,
            hooks: LightHooks,
            history: u32,
            key: u32,
            frames: u32,
            align: Option<(u32, u32)>,
        ) -> Read {
            use vxe_render::debug::{read_rgba8, read_rgba16f, read_rgba32f};
            let out = r.out;
            r.f.set_light_hooks(hooks);
            r.f.set_count_store(true);
            r.f.set_lighting(Lighting {
                sky_history: history,
                key_history: key,
                ..r.f.lighting()
            });
            r.f.reset_history();
            let mut total = frames;
            if let Some((phase, phases)) = align {
                while (r.steps + total) % phases != phase {
                    total += 1;
                }
            }
            let light = LightParams::headlamp();
            for _ in 0..total - POP_PAIRS - 1 {
                r.step(rig, cam, &light);
            }
            let (d, q) = (&rig.device, &rig.queue);
            let level = |f: &Frame| support::defects::level(&read_rgba8(d, q, f.target()));
            let classify = |f: &Frame| {
                let hits = read_rgba32f(d, q, f.light_hit_positions());
                let gw = f.light_hit_positions().width() as usize;
                let cls: Vec<Class> = hits.iter().map(|h| class_of(&rig.world, h)).collect();
                let n = (out[0] * out[1]) as usize;
                (0..n)
                    .map(|i| {
                        cls[support::defects::grid_of(
                            i,
                            out[0] as usize,
                            out[1] as usize,
                            gw,
                            hits.len(),
                        )]
                    })
                    .collect::<Vec<_>>()
            };
            r.step(rig, cam, &light);
            let mut last = level(&r.f);
            let class = classify(&r.f);
            let mut pops = [(0u64, 0u64); 2];
            let mut rejections = Vec::with_capacity(POP_PAIRS as usize);
            let mut diffs: [Vec<u8>; 2] = Default::default();
            for _ in 0..POP_PAIRS {
                r.step(rig, cam, &light);
                rejections.push(r.f.read_history_rejections(d, q));
                let now = level(&r.f);
                for (i, (&a, &b)) in last.iter().zip(&now).enumerate() {
                    let k = match class[i] {
                        Class::Floor => 0,
                        Class::Trunk => 1,
                        Class::Other => continue,
                    };
                    pops[k].1 += 1;
                    pops[k].0 += u64::from(a.abs_diff(b) > 4);
                    diffs[k].push(a.abs_diff(b));
                }
                last = now;
            }
            let pair = read_rgba16f(d, q, r.f.visibility_pair());
            let gw = r.f.visibility_pair().width() as usize;
            let n = (out[0] * out[1]) as usize;
            let g = (0..n)
                .map(|i| {
                    pair[support::defects::grid_of(i, out[0] as usize, out[1] as usize, gw, pair.len())][1]
                })
                .collect();
            let hits = read_rgba32f(d, q, r.f.light_hit_positions());
            Read {
                level: last,
                class,
                g,
                pops,
                light_class: hits.iter().map(|h| class_of(&rig.world, h)).collect(),
                light_age: read_rgba16f(d, q, r.f.moments()).iter().map(|m| m[2]).collect(),
                light_g: pair.iter().map(|p| p[1]).collect(),
                rejections,
                diffs,
            }
        }

        /// The class of a lighting pixel's hit `h`.
        fn class_of(world: &vxe_world::World, h: &[f32; 4]) -> Class {
            use flythrough::outdoor::{BARK, is_leaf};
            if h[3] <= 0.0 || h[3] >= 1.0e30 || h[..3].iter().any(|v| v.is_nan()) {
                return Class::Other;
            }
            let (mut leaf, mut bark) = (false, false);
            for k in 0..8 {
                let v: [i32; 3] = std::array::from_fn(|a| {
                    let o = if k >> a & 1 == 1 { 0.01 } else { -0.01 };
                    (h[a] + o).floor() as i32
                });
                match world.material_at(vxe_world::VoxelCoord(v)) {
                    Some(m) if is_leaf(m) => leaf = true,
                    Some(m) if m == BARK => bark = true,
                    _ => {}
                }
            }
            if leaf {
                Class::Other
            } else if bark {
                Class::Trunk
            } else {
                Class::Floor
            }
        }

        /// The residual of `a` against the reference `r` per class (floor, trunk) and key
        /// mean (lit: `g` over 0.5 in `a`; shadowed): (p99, mean, pixels).
        fn residual(a: &Read, r: &Read) -> [[(u8, f64, usize); 2]; 2] {
            let mut v: [[Vec<u8>; 2]; 2] = Default::default();
            for i in 0..a.level.len() {
                let k = match a.class[i] {
                    Class::Floor => 0,
                    Class::Trunk => 1,
                    Class::Other => continue,
                };
                let lit = usize::from(a.g[i] <= 0.5);
                v[k][lit].push(a.level[i].abs_diff(r.level[i]));
            }
            v.map(|row| {
                row.map(|c| {
                    let n = c.len();
                    let (p, m) = support::defects::p99_mean(c);
                    (p, m, n)
                })
            })
        }

        /// The effective sample count of a running mean over `age + 1` samples capped at
        /// `cap` (past the cap an exponential mean of weight `1 / cap`, whose variance is a
        /// sample's over `2 cap - 1`).
        fn n_eff(age: f32, cap: u32) -> f64 {
            let n = f64::from(age.max(0.0)) + 1.0;
            if n < f64::from(cap) {
                n
            } else {
                2.0 * f64::from(cap) - 1.0
            }
        }

        /// One row of Plan 0094 Phase 1's age table for run `a` (key cap `key`, `frames`
        /// frames) against the reference `refr`: per class (floor, trunks) over the lighting
        /// pixels, the shares of history age under 16, under 64 and at the run's full length
        /// (`frames - 1`), the key mean's observed RMS residual against the reference over
        /// its binomial prediction (`g (1 - g)` at the reference's `g`, over the run's and the
        /// reference's effective counts, pixels with the reference's `g` in 0.02..0.98), the
        /// mean history rejections per frame (tested, material, normal, other), the display
        /// residual and the pop share.
        fn age_row(scene: &str, label: &str, a: &Read, refr: &Read, key: u32, frames: u32) {
            let mut rej = [0f64; 4];
            for r in &a.rejections {
                for k in 0..4 {
                    rej[k] += f64::from(r[k]) / a.rejections.len().max(1) as f64;
                }
            }
            let res = residual(a, refr);
            for (k, name) in [(Class::Floor, "floor"), (Class::Trunk, "trunks")]
                .into_iter()
                .enumerate()
            {
                let (cls, name) = name;
                let (mut n, mut u16, mut u64_, mut full) = (0usize, 0usize, 0usize, 0usize);
                let (mut obs, mut pred, mut m) = (0f64, 0f64, 0usize);
                for i in 0..a.light_class.len() {
                    if a.light_class[i] != cls {
                        continue;
                    }
                    let age = a.light_age[i];
                    n += 1;
                    u16 += usize::from(age < 16.0);
                    u64_ += usize::from(age < 64.0);
                    full += usize::from(age >= (frames - 1) as f32);
                    let gr = f64::from(refr.light_g[i]);
                    if (0.02..=0.98).contains(&gr) {
                        let d = f64::from(a.light_g[i]) - gr;
                        obs += d * d;
                        pred += gr * (1.0 - gr)
                            * (1.0 / n_eff(age, key) + 1.0 / n_eff(refr.light_age[i], REFERENCE_HISTORY));
                        m += 1;
                    }
                }
                let pct = |v: usize| 100.0 * v as f64 / n.max(1) as f64;
                let ratio = if pred > 0.0 { (obs / pred).sqrt() } else { f64::NAN };
                let lit = res[k][0];
                let shd = res[k][1];
                let (f2f_p99, f2f_mean) = support::defects::p99_mean(a.diffs[k].clone());
                println!(
                    "reading: ages {scene}, {label}, {name}: {n} lighting px; age under 16 {:.2} %, under 64 {:.2} %, at {} {:.2} %; \
                     key mean RMS residual over predicted {ratio:.2} ({m} px with g in 0.02..0.98, observed RMS {:.4}); \
                     display residual lit p99 {} mean {:.2}, shadowed p99 {} mean {:.2}; pop share {:.3} %; \
                     frame-to-frame p99 {f2f_p99} mean {f2f_mean:.3}",
                    pct(u16),
                    pct(u64_),
                    frames - 1,
                    pct(full),
                    (obs / m.max(1) as f64).sqrt(),
                    lit.0,
                    lit.1,
                    shd.0,
                    shd.1,
                    100.0 * a.pops[k].0 as f64 / a.pops[k].1.max(1) as f64,
                );
            }
            println!(
                "reading: ages {scene}, {label}: rejections per frame at rest: tested {:.0}, material {:.1}, normal {:.1}, other {:.1}",
                rej[0], rej[1], rej[2], rej[3]
            );
        }

        /// Plan 0094 Phase 1 bars 1 and 2: on both forest scenes (the sun at `10,180`, 15:00),
        /// the defaults (both terms, the store baked), the exposure pinned, a still camera: the
        /// long reference ([`REFERENCE`] frames, both caps [`REFERENCE_HISTORY`]), then runs of
        /// 128 frames at the default key cap, 512 frames at key cap 256 and 512 frames at key cap
        /// 64, each aligned to the reference's jitter phase, each read by [`age_row`].
        /// `VXE_TEST_ADAPTER` picks the adapter; `VXE_CANOPY_ARM` (0 to 2) runs one run alone
        /// after its own reference, so each run reads in a process of its own.
        #[test]
        #[ignore = "reading"]
        fn the_canopy_ages() {
            let Some((mut rig, cam)) = support::terms::forest_rig() else {
                return;
            };
            let phases = vxe_render::Fsr2Settings::default().phases(INTERNAL[0], OUT[0]);
            println!("reading: the_canopy_ages on {}, {phases} jitter phases", rig.adapter);
            let t0 = std::time::Instant::now();
            let sky_cap = flythrough::default_lighting().sky_history;
            let key_cap = flythrough::default_lighting().key_history;
            for (scene, clock) in [
                (
                    "sun 10,180",
                    flythrough::daylight::Clock {
                        fixed_sun: Some([10.0, 180.0]),
                        ..Default::default()
                    },
                ),
                (
                    "15:00",
                    flythrough::daylight::Clock {
                        minutes: 15.0 * 60.0,
                        ..Default::default()
                    },
                ),
            ] {
                let sky = clock.sky(flythrough::outdoor::BASE_Z as f32);
                let e = settled_exposure(&mut rig, &cam, Some(sky));
                let mut r = frame(&mut rig, Some(sky), true, Exposure::Manual(e));
                let off = LightHooks::default();
                let refr = hold_capped(&rig, &cam, &mut r, off, REFERENCE_HISTORY, REFERENCE_HISTORY, REFERENCE, None);
                let phase = r.steps % phases;
                let only: Option<usize> = std::env::var("VXE_CANOPY_ARM").ok().and_then(|v| v.parse().ok());
                for (i, (frames, key)) in [(128, key_cap), (512, 256), (512, 64)].into_iter().enumerate() {
                    if only.is_some_and(|o| o != i) {
                        continue;
                    }
                    let a = hold_capped(&rig, &cam, &mut r, off, sky_cap, key, frames, Some((phase, phases)));
                    age_row(scene, &format!("{frames} frames, key cap {key}"), &a, &refr, key, frames);
                }
            }
            println!("reading: the_canopy_ages wall {:.1} s", t0.elapsed().as_secs_f64());
        }

        /// Plan 0086 Phase 24b's table: the forest of seed 1 at its spawn's view, the sun at
        /// `10,180` and at 15:00, voxel leaves, FSR 2 from 320 x 180 to 480 x 270, the exposure
        /// pinned at the defaults' settled value: per arm of the split (both terms; the key light
        /// alone, `sky_rays` 0 and no store; the sky alone, `key_off`; the key light alone with
        /// `key_no_jitter`) the run of [`FRAMES`] frames after a history reset against the same
        /// arm's long reference ([`REFERENCE`] frames under `sky_history` [`REFERENCE_HISTORY`],
        /// read at the same jitter phase): the residual's p99 and mean in display levels on the
        /// floor and the trunks, lit (the key mean over 0.5) and shadowed; and the pop share
        /// over 4 levels per frame pair over the run's last [`POP_PAIRS`] pairs, beside the
        /// lamp-lit floor's. `VXE_TEST_ADAPTER` picks the adapter.
        #[test]
        #[ignore = "reading"]
        fn the_canopy_residual() {
            let Some((mut rig, cam)) = support::terms::forest_rig() else {
                return;
            };
            let phases = vxe_render::Fsr2Settings::default().phases(INTERNAL[0], OUT[0]);
            println!("reading: the_canopy_residual on {}, {phases} jitter phases", rig.adapter);
            // The lamp-lit floor: the headlamp alone, at its settled exposure.
            let e = settled_exposure(&mut rig, &cam, None);
            let mut lamp = frame(&mut rig, None, false, Exposure::Manual(e));
            let r = hold(&rig, &cam, &mut lamp, LightHooks::default(), 64, FRAMES, None);
            let lamp_pop = 100.0 * r.pops[0].0 as f64 / (r.pops[0].1.max(1) as f64);
            println!("reading: canopy, the headlamp alone: floor pop share {lamp_pop:.3} % of {} px-pairs", r.pops[0].1);
            let scenes = [
                (
                    "sun 10,180",
                    flythrough::daylight::Clock {
                        fixed_sun: Some([10.0, 180.0]),
                        ..Default::default()
                    },
                ),
                (
                    "15:00",
                    flythrough::daylight::Clock {
                        minutes: 15.0 * 60.0,
                        ..Default::default()
                    },
                ),
            ];
            let default_history = flythrough::default_lighting().sky_history;
            println!(
                "reading: canopy caps: sky {default_history}, key {}",
                flythrough::default_lighting().key_history
            );
            for (scene, clock) in scenes {
                let sky = clock.sky(flythrough::outdoor::BASE_Z as f32);
                let e = settled_exposure(&mut rig, &cam, Some(sky));
                println!("reading: canopy {scene}: exposure pinned at {e:.4e}");
                let mut baked = frame(&mut rig, Some(sky), true, Exposure::Manual(e));
                let mut bare = frame(&mut rig, Some(sky), false, Exposure::Manual(e));
                for arm in ARMS {
                    let r = if arm.baked { &mut baked } else { &mut bare };
                    let a = hold(&rig, &cam, r, arm.hooks, default_history, FRAMES, None);
                    let phase = r.steps % phases;
                    let refr = hold(
                        &rig,
                        &cam,
                        r,
                        arm.hooks,
                        REFERENCE_HISTORY,
                        REFERENCE,
                        Some((phase, phases)),
                    );
                    let res = residual(&a, &refr);
                    let pop = |k: usize| 100.0 * a.pops[k].0 as f64 / a.pops[k].1.max(1) as f64;
                    let cell = |c: (u8, f64, usize)| format!("p99 {} mean {:.2} ({} px)", c.0, c.1, c.2);
                    println!(
                        "reading: canopy {scene}, {}: floor lit {}, shadowed {}; trunks lit {}, shadowed {}; pop share floor {:.3} % trunks {:.3} % (lamp-lit floor {lamp_pop:.3} %)",
                        arm.name,
                        cell(res[0][0]),
                        cell(res[0][1]),
                        cell(res[1][0]),
                        cell(res[1][1]),
                        pop(0),
                        pop(1),
                    );
                }
            }
        }
    }
}
