// `gpu_outdoor`'s Phase 16 readings of the second window's defects (Plan 0086), included by
// `gpu_outdoor.rs` (kept apart for the file-size cap).

mod defects {
    pub(super) mod reading {
        use super::super::support;

        /// Plan 0086 Phase 16 reading 1 (defect 1, the window's 21.png): the hills at 17:00
        /// streamed about 21.png's eye (`-352.5 -30.2 671.8`, chunk `-6 -1 10`; the HUD hid the x
        /// to its last digit, so the chunk's middle), looking 30 degrees down, at the fly-through's
        /// residency (16, 24) and mirror (`support::streamed::Hold`), the column wait on
        /// (`SVET_HELD_WAIT=0` off), the reach `full` (`SVET_HELD_REACH=partial`: Phase 18 bar 6),
        /// 1800 frames (`SVET_HELD_FRAMES`): every 60 frames the store's counts and the queue's;
        /// at the end every unbaked chunk within the full radius and what holds its columns from
        /// final, and the fly-through's `--dump-black` lines; the first read frame whose black
        /// count is under 1 % of the 57 600 lighting pixels. `SVET_TEST_ADAPTER` picks the
        /// adapter.
        #[test]
        #[ignore = "reading"]
        fn the_hills_at_dusk_held() {
            let wait = std::env::var("SVET_HELD_WAIT").as_deref() != Ok("0");
            let reach = match std::env::var("SVET_HELD_REACH").as_deref() {
                Ok("partial") => svet_render::ReachMode::Partial,
                _ => svet_render::ReachMode::Full,
            };
            let frames: u32 = std::env::var("SVET_HELD_FRAMES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1800);
            let reads = hills_at_dusk(wait, reach, frames);
            let under = reads.iter().find(|r| r.black < 576).map(|r| r.frame);
            println!(
                "reading: the hills at dusk, wait {wait}, reach {reach:?}: black under 1 % first at frame {under:?}"
            );
        }

        /// Plan 0086 Phase 18 bar 2: [`the_hills_at_dusk_held`]'s rig under each column wait,
        /// the reach `full`, 1800 frames: from frame 300 on the largest relative difference of
        /// the wait-on `black` against the wait-off's; per mode the first read frame at which no
        /// chunk whose reach lies within the full radius is unbaked
        /// (`StreamedRead::unbaked_inner`), and that count at the end. Printed.
        #[test]
        #[ignore = "reading"]
        fn the_hills_at_dusk_under_each_wait() {
            let on = hills_at_dusk(true, svet_render::ReachMode::Full, 1800);
            let off = hills_at_dusk(false, svet_render::ReachMode::Full, 1800);
            if on.is_empty() {
                return;
            }
            let worst = on
                .iter()
                .zip(&off)
                .filter(|(a, _)| a.frame >= 300)
                .map(|(a, b)| {
                    (f64::from(a.black) - f64::from(b.black)).abs() / f64::from(b.black.max(1))
                })
                .fold(0.0, f64::max);
            let done = |r: &[support::streamed::StreamedRead]| {
                (
                    r.iter().find(|x| x.unbaked_inner == 0).map(|x| x.frame),
                    r.last().map(|x| x.unbaked_inner),
                )
            };
            println!(
                "reading: under each wait: black from frame 300, the largest |on - off| / off {:.4}; on (first frame every inner chunk baked, unbaked inner at the end) {:?}, black at the end {}; off {:?}, black {}",
                worst,
                done(&on),
                on.last().map_or(0, |r| r.black),
                done(&off),
                off.last().map_or(0, |r| r.black),
            );
        }

        /// The hills at dusk at 21.png's pose (see [`the_hills_at_dusk_held`]), streamed
        /// `frames` frames under the column wait `wait` and the reach `reach`, read every 60.
        fn hills_at_dusk(
            wait: bool,
            reach: svet_render::ReachMode,
            frames: u32,
        ) -> Vec<support::streamed::StreamedRead> {
            let camera = flythrough::Pose {
                name: "dusk",
                position: [-352.5, -30.2, 671.8],
                yaw_deg: 0.0,
                pitch_deg: -30.0,
                exposure: 1.0,
            }
            .camera();
            let sky = flythrough::daylight::Clock {
                minutes: 17.0 * 60.0,
                ..Default::default()
            }
            .sky(flythrough::outdoor::BASE_Z as f32);
            println!("reading: the hills at dusk, column wait {wait}, reach {reach:?}");
            support::streamed::streamed(support::streamed::Streamed {
                label: "the_hills_at_dusk_held",
                source: std::sync::Arc::new(flythrough::outdoor::OutdoorSource::new(1)),
                eye: svet_world::VoxelCoord([-353, -31, 671]),
                camera,
                sky,
                styles: Vec::new(),
                r: 16,
                rays: 1,
                reads: (0..=frames).step_by(60).collect(),
                hold: Some(support::streamed::Hold {
                    r_lod: 24,
                    wait,
                    reach,
                }),
                stepped: false,
            })
        }

        /// Plan 0086 Phase 20's reading: the tunnel at noon at 19.png's pose (`--spawn inside`,
        /// the eye `221 -4.4 493.6`, facing out of the tunnel), streamed as
        /// [`the_hills_at_dusk_held`]'s rig (residency 16, 24), 600 frames read every 60: per
        /// lighting pixel the sky term's source (the pair where the ray was cast, the chain's
        /// fallback, black, the store otherwise) and the cast pixels' mean visibility, under
        /// the column wait off and on with the reach `full`, and the wait off with `partial`.
        /// `SVET_TEST_ADAPTER` picks the adapter.
        #[test]
        #[ignore = "reading"]
        fn the_tunnel_ray_region() {
            use flythrough::outdoor::{OutdoorSource, Spawn};
            let source = OutdoorSource::new(1);
            let (eye, facing) = source.spawn(Spawn::Inside);
            let p = [221.0f32, -4.4, 493.6];
            let d = facing.map(|v| v as f32);
            let camera =
                flythrough::camera_looking_at(p, [p[0] + d[0], p[1] + d[1], p[2] + d[2]]);
            let noon =
                flythrough::daylight::Clock::default().sky(flythrough::outdoor::BASE_Z as f32);
            for (wait, reach) in [
                (false, svet_render::ReachMode::Full),
                (true, svet_render::ReachMode::Full),
                (false, svet_render::ReachMode::Partial),
            ] {
                println!("reading: the tunnel ray region, column wait {wait}, reach {reach:?}, spawn eye {eye:?}");
                support::streamed::streamed(support::streamed::Streamed {
                    label: "the_tunnel_ray_region",
                    source: std::sync::Arc::new(OutdoorSource::new(1)),
                    eye: svet_world::VoxelCoord([221, -5, 493]),
                    camera,
                    sky: noon,
                    styles: Vec::new(),
                    r: 16,
                    rays: 1,
                    reads: (0..=600).step_by(60).collect(),
                    hold: Some(support::streamed::Hold {
                        r_lod: 24,
                        wait,
                        reach,
                    }),
                    stepped: false,
                });
            }
        }

        /// Plan 0086 Phase 20 bar 1a (re-stated 2026-11-19): the ray region lands within the
        /// fade. 19.png's pose streamed as [`the_tunnel_ray_region`] (residency 16, 24, the wait
        /// off, `full`), 300 frames at 320 x 180, the exposure pinned at -10.75 EV from the first
        /// frame. Per lighting pixel each frame its sky term's source: the pair (it cast the sky
        /// ray), the store (its face's own cell record holds the direction's baked visibility,
        /// read from the store's words on the CPU as `store_sky_cell`), else the chain; and the
        /// display's largest channel at its centre. A switch at `f` is a source change between
        /// `f` and `f + 1`, by kind (pair to store, pair to chain, chain to store); `old` the
        /// level at `f`, `new` at `f + 16`. Per kind: the count, the share with a frame in
        /// `f ..= f + 16` outside `[min(old, new) - 4, max(old, new) + 4]`, the leavers by side
        /// (under, over) and by whether the pixel casts again within the 16 frames, the share of
        /// leavers that do not cast again and whose largest excursion is on `f + 1` (the
        /// switch's frame: (ii)'s line), the mean and p99 of `|old - new|`, the mean
        /// `|old - new| / max(new, 8)`, and the step's sign (old under new, over). The
        /// frame-to-frame p99 over the cast pixels at frames 60 and 120. Printed.
        /// `SVET_TEST_ADAPTER` picks the adapter.
        #[test]
        #[ignore = "reading"]
        fn the_ray_region_lands() {
            use flythrough::outdoor::{OutdoorSource, Spawn};
            const FRAMES: u32 = 300;
            const FADE: usize = 16;
            let source = OutdoorSource::new(1);
            let (_, facing) = source.spawn(Spawn::Inside);
            let p = [221.0f32, -4.4, 493.6];
            let d = facing.map(|v| v as f32);
            let camera =
                flythrough::camera_looking_at(p, [p[0] + d[0], p[1] + d[1], p[2] + d[2]]);
            let noon =
                flythrough::daylight::Clock::default().sky(flythrough::outdoor::BASE_Z as f32);
            // Per frame: the source per lighting pixel and the level at it.
            let mut src: Vec<Vec<u8>> = Vec::new();
            let mut level: Vec<Vec<u8>> = Vec::new();
            support::streamed::streamed_with(
                support::streamed::Streamed {
                    label: "the_ray_region_lands",
                    source: std::sync::Arc::new(OutdoorSource::new(1)),
                    eye: svet_world::VoxelCoord([221, -5, 493]),
                    camera,
                    sky: noon,
                    styles: Vec::new(),
                    r: 16,
                    rays: 1,
                    reads: vec![0, 60, 120, 300],
                    hold: Some(support::streamed::Hold {
                        r_lod: 24,
                        wait: false,
                        reach: svet_render::ReachMode::Full,
                    }),
                    stepped: false,
                },
                |f, device, queue, frame| {
                    use svet_render::debug::{read_rgba8, read_rgba32f};
                    if f == 0 {
                        frame.set_lighting(svet_render::Lighting {
                            exposure: svet_render::Exposure::Manual((-10.75f32).exp2()),
                            ..frame.lighting()
                        });
                    }
                    if f > FRAMES {
                        return;
                    }
                    let vis = read_rgba8(device, queue, frame.visibility_samples());
                    let hits = read_rgba32f(device, queue, frame.light_hit_positions());
                    let (lw, lh) = (
                        frame.visibility_samples().width() as usize,
                        frame.visibility_samples().height() as usize,
                    );
                    let disp = read_rgba8(device, queue, frame.target());
                    let (dw, dh) = (
                        frame.target().width() as usize,
                        frame.target().height() as usize,
                    );
                    let store = frame.store();
                    let cell = store.brick_cell().max(1) as i32;
                    src.push(
                        (0..lw * lh)
                            .map(|i| {
                                if vis[4 * i + 3] > 127 {
                                    return PAIR;
                                }
                                let h = hits[i];
                                if h[3] >= 1e30 {
                                    return CHAIN;
                                }
                                let face = face_of([h[0], h[1], h[2]], p);
                                if cell_sky(store, face.0, face.1, cell).is_some() {
                                    STORE
                                } else {
                                    CHAIN
                                }
                            })
                            .collect(),
                    );
                    level.push(
                        (0..lw * lh)
                            .map(|i| {
                                let (x, y) = (i % lw, i / lw);
                                let (dx, dy) = ((2 * x + 1) * dw / (2 * lw), (2 * y + 1) * dh / (2 * lh));
                                let q = &disp[4 * (dy * dw + dx)..4 * (dy * dw + dx) + 3];
                                q[0].max(q[1]).max(q[2])
                            })
                            .collect(),
                    );
                },
            );
            if src.len() < FRAMES as usize {
                return;
            }
            let mut kinds = switch_kinds(&src, &level, FADE);
            let p99 = |v: &[u32]| v.get((v.len() * 99 / 100).min(v.len().saturating_sub(1))).copied().unwrap_or(0);
            let mean = |v: &[u32]| v.iter().map(|&x| f64::from(x)).sum::<f64>() / v.len().max(1) as f64;
            for (name, kd) in ["pair to store", "pair to chain", "chain to store"].iter().zip(kinds.iter_mut()) {
                kd.steps.sort_unstable();
                let n = kd.n.max(1) as f64;
                println!(
                    "reading: the ray region lands, {name}: {} switches, {:.2} % left the band of 4 over {FADE} frames (under {}, over {}; cast again {}; no re-cast with the excursion on the switch's frame {} = {:.2} %), |old - new| mean {:.2} p99 {} levels, mean relative step (floored at 8) {:.3}, old under new {} over {}",
                    kd.n,
                    100.0 * kd.out as f64 / n,
                    kd.side[0],
                    kd.side[1],
                    kd.recast,
                    kd.on_switch,
                    100.0 * kd.on_switch as f64 / n,
                    mean(&kd.steps),
                    p99(&kd.steps),
                    kd.rel.iter().sum::<f64>() / kd.rel.len().max(1) as f64,
                    kd.sign[0],
                    kd.sign[1]
                );
            }
            let still = |f: usize| {
                let mut c: Vec<u32> = (0..src[f].len())
                    .filter(|&i| src[f][i] == PAIR)
                    .map(|i| u32::from(level[f][i].abs_diff(level[f - 1][i])))
                    .collect();
                c.sort_unstable();
                (p99(&c), c.len())
            };
            println!(
                "reading: the ray region lands: frame-to-frame p99 over the cast pixels at 60 {:?} and at 120 {:?} (p99, pixels; the lamp-lit floor 2.11)",
                still(60),
                still(120)
            );
        }

        // A lighting pixel's sky-term source: the pair, the store, the chain (or no sky).
        const PAIR: u8 = 0;
        const STORE: u8 = 1;
        const CHAIN: u8 = 2;

        /// One kind of source switch's tally ([`switch_kinds`]).
        #[derive(Default)]
        struct SwitchKind {
            n: usize,
            out: usize,
            side: [usize; 2],
            recast: usize,
            on_switch: usize,
            steps: Vec<u32>,
            rel: Vec<f64>,
            sign: [usize; 2],
        }

        /// The switches of `src` (per frame, per pixel) by kind: pair to store, pair to chain,
        /// chain to store; `old` the level at `f`, `new` at `f + fade`, the band of 4 levels
        /// about them over `f ..= f + fade`.
        fn switch_kinds(src: &[Vec<u8>], level: &[Vec<u8>], fade: usize) -> [SwitchKind; 3] {
            let mut kinds: [SwitchKind; 3] = Default::default();
            for f in 0..src.len() - fade - 1 {
                for i in 0..src[f].len() {
                    let k = match (src[f][i], src[f + 1][i]) {
                        (PAIR, STORE) => 0,
                        (PAIR, CHAIN) => 1,
                        (CHAIN, STORE) => 2,
                        _ => continue,
                    };
                    let kd = &mut kinds[k];
                    kd.n += 1;
                    let (old, new) = (i32::from(level[f][i]), i32::from(level[f + fade][i]));
                    let (lo, hi) = (old.min(new) - 4, old.max(new) + 4);
                    let mut worst = (0, f, false);
                    for (g, row) in level.iter().enumerate().skip(f).take(fade + 1) {
                        let v = i32::from(row[i]);
                        let e = (lo - v).max(v - hi);
                        if e > worst.0 {
                            worst = (e, g, v > hi);
                        }
                    }
                    if worst.0 > 0 {
                        kd.out += 1;
                        kd.side[usize::from(worst.2)] += 1;
                        let recast = src[f + 1..=f + fade].iter().any(|r| r[i] == PAIR);
                        kd.recast += usize::from(recast);
                        kd.on_switch += usize::from(!recast && worst.1 == f + 1);
                    }
                    kd.steps.push((old - new).unsigned_abs());
                    kd.rel.push(f64::from((old - new).abs()) / f64::from(new.max(8)));
                    if old != new {
                        kd.sign[usize::from(old > new)] += 1;
                    }
                }
            }
            kinds
        }

        /// The store voxel and face index of a cube-face hit at `h` seen from `eye`: the axis whose
        /// coordinate lies nearest an integer, its normal toward the eye (`store_voxel`,
        /// `store_face`).
        fn face_of(h: [f32; 3], eye: [f32; 3]) -> ([i32; 3], u8) {
            let axis = (0..3)
                .min_by(|&a, &b| {
                    let da = (h[a] - h[a].round()).abs();
                    let db = (h[b] - h[b].round()).abs();
                    da.total_cmp(&db)
                })
                .unwrap_or(2);
            let s = if eye[axis] > h[axis] { 1.0 } else { -1.0 };
            let mut v = [0i32; 3];
            for k in 0..3 {
                let n = if k == axis { s } else { 0.0 };
                v[k] = (h[k] - 0.5 * n).floor() as i32;
            }
            (v, 2 * axis as u8 + u8::from(s > 0.0))
        }

        /// The sky visibility face `face` of voxel `v`'s own cell record holds in `store`
        /// (`store_sky_cell`), `None` where it holds none baked.
        fn cell_sky(store: &svet_render::Store, v: [i32; 3], face: u8, cell: i32) -> Option<f32> {
            let block = store.block_words(svet_world::ChunkCoord(v.map(|c| c.div_euclid(64))))?;
            let l = v.map(|c| c.rem_euclid(64));
            let b = l.map(|c| c / 8);
            let dir = block[(b[0] + 8 * (b[1] + 8 * b[2])) as usize];
            if dir == 0 {
                return None;
            }
            let n = 8 / cell;
            let c = l.map(|k| (k % 8) / cell);
            let sub = (c[0] + n * (c[1] + n * c[2])) as usize;
            let w = svet_render::internals::BRICK_WORDS;
            let end = (dir >> 11) as usize;
            let at = (end + w * sub).checked_sub(w * (n * n * n) as usize)?;
            svet_render::internals::record_visibility(block.get(at..at + w)?, usize::from(face))
        }

        /// A frame on `rig` for Phase 16's readings: 320 x 180 native, the store's counters on, the
        /// fly-through's look, voxel leaves, the lamp off, `lighting` over the defaults, `sky`,
        /// the world baked whole when `bake`.
        fn defect_frame(
            rig: &mut support::terms::Rig,
            sky: Option<svet_render::sky::Sky>,
            lighting: impl Fn(svet_render::Lighting) -> svet_render::Lighting,
            hooks: svet_render::LightHooks,
            bake: bool,
        ) -> svet_render::Frame {
            defect_frame_fsr(rig, sky, lighting, hooks, bake, false)
        }

        /// [`defect_frame`], with `fsr2` at 320 x 180 internal under FSR 2 to 480 x 270.
        fn defect_frame_fsr(
            rig: &mut support::terms::Rig,
            sky: Option<svet_render::sky::Sky>,
            lighting: impl Fn(svet_render::Lighting) -> svet_render::Lighting,
            hooks: svet_render::LightHooks,
            bake: bool,
            fsr2: bool,
        ) -> svet_render::Frame {
            let mut f = if fsr2 {
                let mut f = svet_render::Frame::with_sizes(&rig.device, [320, 180], [480, 270]);
                f.set_upscaler_kind(
                    &rig.device,
                    svet_render::UpscalerKind::Fsr2(svet_render::Fsr2Settings::default()),
                );
                f
            } else {
                svet_render::Frame::new(&rig.device, 320, 180)
            };
            f.set_count_store(true);
            f.set_light_hooks(hooks);
            f.set_look(svet_render::LookParams {
                far_normal: flythrough::LookPreset::DEFAULT.far_normal,
                ..svet_render::LookParams::default()
            });
            f.set_material_styles(
                &rig.queue,
                &flythrough::outdoor_styles(flythrough::LeafShadow::Voxel),
            );
            f.set_lighting(lighting(svet_render::Lighting {
                lamp: false,
                ..flythrough::default_lighting()
            }));
            f.set_sky(sky);
            if bake {
                f.bake_until(
                    &rig.device,
                    &rig.queue,
                    &mut rig.gpu,
                    &rig.world,
                    rig.world.extent(),
                    &svet_render::BakeSettings::default(),
                    &mut |_| {},
                )
                .expect("the store fits");
            }
            f.set_dt(1.0 / 60.0);
            f
        }

        /// Plan 0086 Phase 16 reading 2 (defect 3, the window's 20.png): the tunnel of seed 1
        /// baked whole under the noon sky, the view out (19.png's pose, `--spawn inside`), the
        /// exposure pinned at -10.75 EV, 320 x 180 native or under FSR 2 to 480 x 270: 64 settle
        /// frames, 32 frames read with no light, then the fly-through's `6` five times at once
        /// (yaws -30, -15, 0, 15, 30, 25 degrees down, so they stop on the floor near the eye as
        /// 20.png's did; `TestLights::fire` and `step`) and 200 frames read. The lit patch is the
        /// display pixels whose level at the end is over 1.5 times and 16 levels over their mean
        /// before; its p99 and mean frame-to-frame change before, in flight and at rest (16
        /// frames after the last light stops on), the share of its edge's lighting pixels whose
        /// direct sample is off its at-rest mean by over 75 % a frame, the light pass's ms, the
        /// sky rays cast and the store's black count. Arms: the sky at the defaults, `sky_rays` 0,
        /// and no sky (the lights alone: Plan 0058's mechanism). `SVET_TEST_ADAPTER` picks the
        /// adapter.
        #[test]
        #[ignore = "reading"]
        #[allow(clippy::too_many_lines, reason = "a reading's arms in one place")]
        fn the_tunnel_under_fired_lights() {
            use support::defects::{Stepper, fire_five, level, p99_mean};
            let Some((mut rig, cams)) = support::terms::tunnel_rig() else {
                return;
            };
            println!("reading: the tunnel under fired lights on {}", rig.adapter);
            let cam = cams[0].1;
            let noon = flythrough::daylight::Clock::default().sky(flythrough::outdoor::BASE_Z as f32);
            let e = 2f32.powf(-10.75);
            // `anchor`: the outdoor auto exposure under that sky anchor (Plan 0086 Phase 22 bar
            // 2) in place of the pinned exposure.
            for (arm, sky, rays, fsr2, anchor) in [
                ("sky, rays 1, native", Some(noon), 1, false, None),
                ("sky, rays 1, FSR 2", Some(noon), 1, true, None),
                ("sky, rays 0, FSR 2", Some(noon), 0, true, None),
                ("no sky, FSR 2", None, 1, true, None),
                ("sky, rays 1, FSR 2, auto anchor 4", Some(noon), 1, true, Some(4.0)),
                ("sky, rays 1, FSR 2, auto anchor 0", Some(noon), 1, true, Some(0.0)),
            ] {
                let exposure = anchor.map_or(svet_render::Exposure::Manual(e), |a| {
                    svet_render::Exposure::Auto(svet_render::AutoExposure {
                        min: flythrough::cli::OUTDOOR_MIN,
                        max: flythrough::cli::OUTDOOR_MAX,
                        sky_anchor_ev: Some(a),
                        ..svet_render::AutoExposure::default()
                    })
                });
                let f = defect_frame_fsr(
                    &mut rig,
                    sky,
                    |l| svet_render::Lighting {
                        exposure,
                        sky_rays: rays,
                        ..l
                    },
                    Default::default(),
                    true,
                    fsr2,
                );
                let (dw, dh) = (f.target().width() as usize, f.target().height() as usize);
                let mut s = Stepper::new(&rig, f);
                for _ in 0..if anchor.is_some() { 300 } else { 64 } {
                    s.run(&rig, &cam);
                }
                let mut shots = Vec::new();
                for _ in 0..32 {
                    let sh = s.step(&rig, &cam);
                    shots.push((level(&sh.display), sh, false));
                }
                let mut lights = fire_five(&rig, &cam, [-30.0, -15.0, 0.0, 15.0, 30.0], 25.0);
                let (fwd, _, up) = cam.basis();
                let mut landed = None;
                for i in 0..200 {
                    lights.step(
                        1.0 / 60.0,
                        cam.position.to_array(),
                        fwd.to_array(),
                        up.to_array(),
                    );
                    s.frame.set_lights(&lights.list());
                    let flying = lights.lights.iter().any(|l| l.flight.is_some());
                    if !flying && landed.is_none() {
                        landed = Some(i);
                    }
                    let sh = s.step(&rig, &cam);
                    shots.push((level(&sh.display), sh, flying));
                }
                let landed = landed.unwrap_or(200) + 32;
                let n = shots[0].0.len();
                let before: Vec<f64> = (0..n)
                    .map(|i| shots[..32].iter().map(|s| f64::from(s.0[i])).sum::<f64>() / 32.0)
                    .collect();
                let end = &shots.last().expect("frames").0;
                let lit: Vec<bool> = (0..n)
                    .map(|i| {
                        f64::from(end[i]) > 1.5 * before[i] && f64::from(end[i]) > before[i] + 16.0
                    })
                    .collect();
                let window = |a: usize, b: usize| {
                    let mut c = Vec::new();
                    for k in a.max(1)..b.min(shots.len()) {
                        for i in (0..n).filter(|&i| lit[i]) {
                            c.push(shots[k].0[i].abs_diff(shots[k - 1].0[i]));
                        }
                    }
                    p99_mean(c)
                };
                let rest0 = landed + 16;
                let (b, fl, r) = (
                    window(1, 32),
                    window(33, landed),
                    window(rest0, shots.len()),
                );
                // The lit patch's lighting pixels, and its edge: a lit one with an unlit 4-neighbour.
                let gl = shots[0].1.direct.len();
                let gw = 160.min(gl);
                let mut glit = vec![false; gl];
                for i in (0..n).filter(|&i| lit[i]) {
                    glit[support::defects::grid_of(i, dw, dh, gw, gl)] = true;
                }
                // The spatial dither at rest: lit-patch pixels off their 3 x 3 mean by over half.
                let dither = {
                    let e: Vec<f64> = end.iter().map(|&v| f64::from(v)).collect();
                    let mut k = 0usize;
                    for i in (0..n).filter(|&i| lit[i]) {
                        let (x, y) = ((i % dw) as i64, (i / dw) as i64);
                        let (mut s, mut c) = (0.0, 0.0);
                        for dy in -1..=1 {
                            for dx in -1..=1 {
                                let (u, v) = (x + dx, y + dy);
                                if u >= 0 && v >= 0 && (u as usize) < dw && (v as usize) < dh {
                                    s += e[v as usize * dw + u as usize];
                                    c += 1.0;
                                }
                            }
                        }
                        k += usize::from((e[i] - s / c).abs() > 0.5 * s / c);
                    }
                    100.0 * k as f64 / lit.iter().filter(|&&l| l).count().max(1) as f64
                };
                let gh = gl / gw;
                let edge: Vec<usize> = (0..gl)
                    .filter(|&g| {
                        glit[g] && {
                            let (x, y) = (g % gw, g / gw);
                            (x == 0 || !glit[g - 1])
                                || (x + 1 == gw || !glit[g + 1])
                                || (y == 0 || !glit[g - gw])
                                || (y + 1 == gh || !glit[g + gw])
                        }
                    })
                    .collect();
                let rest = &shots[rest0.min(shots.len())..];
                let mean_at = |g: usize| {
                    rest.iter().map(|s| s.1.direct[g]).sum::<f32>() / rest.len().max(1) as f32
                };
                let means: Vec<f32> = edge.iter().map(|&g| mean_at(g)).collect();
                let (mut flips, mut tot) = (0usize, 0usize);
                for s in rest {
                    for (k, &g) in edge.iter().enumerate() {
                        if means[k] > 0.0 {
                            tot += 1;
                            if (s.1.direct[g] - means[k]).abs() > 0.75 * means[k] {
                                flips += 1;
                            }
                        }
                    }
                }
                let avg = |a: usize, b: usize, f: &dyn Fn(&support::defects::Shot) -> f64| {
                    let b = b.min(shots.len());
                    (a..b).map(|k| f(&shots[k].1)).sum::<f64>() / (b - a).max(1) as f64
                };
                let ms = |s: &support::defects::Shot| s.light_ms.unwrap_or(f64::NAN);
                let cast = |s: &support::defects::Shot| s.cast.iter().filter(|&&c| c).count() as f64;
                let black = |s: &support::defects::Shot| f64::from(s.black);
                println!(
                    "reading: {arm}: landed at {} frames after the shot; lit patch {} px ({dither:.1} % off their 3 x 3 mean by half at the end), edge {} lighting px; change p99/mean before {}/{:.2}, in flight {}/{:.2}, at rest {}/{:.2}; edge flips at rest {:.2} %; light ms before {:.3}, in flight {:.3}, at rest {:.3}; sky rays cast before {:.0}, in flight {:.0}, at rest {:.0}; black before {:.0}, at rest {:.0}",
                    landed - 32,
                    lit.iter().filter(|&&l| l).count(),
                    edge.len(),
                    b.0,
                    b.1,
                    fl.0,
                    fl.1,
                    r.0,
                    r.1,
                    100.0 * flips as f64 / tot.max(1) as f64,
                    avg(0, 32, &ms),
                    avg(32, landed, &ms),
                    avg(rest0, shots.len(), &ms),
                    avg(0, 32, &cast),
                    avg(32, landed, &cast),
                    avg(rest0, shots.len(), &cast),
                    avg(0, 32, &black),
                    avg(rest0, shots.len(), &black),
                );
            }
        }

        /// Plan 0086 Phase 16 reading 3 (defect 4, the dig flash): the hills of seed 1
        /// (`hills_rig`, the view turned 30 degrees down) at noon on the live queue (the fly-through's settings: coarse 8, near 2,
        /// the default slice), auto exposure (instant, the outdoor range) pinned once the queue
        /// drains, 30 frames, then a 10-voxel box carved where the view meets the ground and 120
        /// frames. Over the display pixels whose lighting hit lies within 64 voxels of the box:
        /// per frame after the dig the excess dots (luminance over 2 times their own at the last
        /// frame and over 2 times the 5 x 5 neighbourhood's mean there), the share changing by over
        /// 16 levels, the sky rays cast and the history's mean age over the region, and the
        /// queue's dig state. Arms: `sky_rays` 0, 1, 1 with `no_clamp`, and the sun off
        /// (`key_off`). `SVET_TEST_ADAPTER` picks the adapter.
        #[test]
        #[ignore = "reading"]
        #[allow(clippy::too_many_lines, reason = "a reading's arms in one place")]
        fn the_dig_flash() {
            use support::defects::{Stepper, dig_ahead, queue_step};
            let noon = flythrough::daylight::Clock::default().sky(flythrough::outdoor::BASE_Z as f32);
            for (arm, rays, no_clamp, key_off) in [
                ("rays 0", 0, false, false),
                ("rays 1", 1, false, false),
                ("rays 1, no clamp", 1, true, false),
                ("rays 1, sun off", 1, false, true),
            ] {
                let Some((mut rig, cam)) = support::terms::hills_rig() else {
                    return;
                };
                // The spawn's view turned 30 degrees down: the dig lands about 35 voxels ahead, as
                // the window's digs did.
                let cam = {
                    let p = cam.position.to_array();
                    let f = cam.basis().0.to_array();
                    let h = (f[0] * f[0] + f[1] * f[1]).sqrt().max(1e-6);
                    let (c, s) = (30f32.to_radians().cos(), 30f32.to_radians().sin());
                    let d = [f[0] / h * c, f[1] / h * c, -s];
                    flythrough::camera_looking_at(p, [p[0] + d[0], p[1] + d[1], p[2] + d[2]])
                };
                let auto = svet_render::Exposure::Auto(svet_render::AutoExposure {
                    min: flythrough::cli::OUTDOOR_MIN,
                    max: flythrough::cli::OUTDOOR_MAX,
                    ..svet_render::AutoExposure::instant()
                });
                let mut f = defect_frame(
                    &mut rig,
                    Some(noon),
                    |l| svet_render::Lighting {
                        exposure: auto,
                        sky_rays: rays,
                        ..l
                    },
                    svet_render::LightHooks {
                        no_clamp,
                        key_off,
                        ..Default::default()
                    },
                    false,
                );
                f.start_bake_queue(
                    &rig.queue,
                    &rig.gpu,
                    svet_render::queue_settings(),
                    svet_render::DEFAULT_BAKE_SLICE_MS,
                );
                f.set_bake_coarse(8, 2);
                let mut drained = None;
                for i in 0..6000 {
                    queue_step(&mut rig, &mut f, &cam);
                    f.step(
                        &rig.device,
                        &rig.queue,
                        &rig.gpu,
                        &cam,
                        &svet_render::LightParams::headlamp(),
                    );
                    if f.bake_queue().is_some_and(|q| q.empty()) {
                        drained = Some(i);
                        break;
                    }
                }
                let st =
                    svet_render::debug::read_buffer(&rig.device, &rig.queue, f.exposure_state(), 0, 32);
                let ex = svet_render::ExposureState::from_bytes(&st).current;
                let l = f.lighting();
                f.set_lighting(svet_render::Lighting {
                    exposure: svet_render::Exposure::Manual(ex),
                    ..l
                });
                let mut s = Stepper::new(&rig, f);
                for _ in 0..30 {
                    queue_step(&mut rig, &mut s.frame, &cam);
                    s.run(&rig, &cam);
                }
                let (min, max) = dig_ahead(&mut rig, &mut s.frame, &cam);
                let mut shots = Vec::new();
                for _ in 0..120 {
                    let q = queue_step(&mut rig, &mut s.frame, &cam);
                    let sh = s.step(&rig, &cam);
                    shots.push((
                        sh,
                        q.digs,
                        s.frame.bake_queue().is_some_and(|q| q.digs_settled()),
                    ));
                }
                let lum: Vec<Vec<f32>> = shots
                    .iter()
                    .map(|(sh, _, _)| {
                        sh.display
                            .chunks(4)
                            .map(|p| {
                                0.2126 * f32::from(p[0])
                                    + 0.7152 * f32::from(p[1])
                                    + 0.0722 * f32::from(p[2])
                            })
                            .collect()
                    })
                    .collect();
                let last = shots.last().expect("frames");
                let gl = last.0.hits.len();
                let gw = 160.min(gl);
                let near = |h: &[f32; 4]| {
                    h[3] < 1.0e30
                        && (0..3).all(|k| {
                            let (lo, hi) = (min[k] as f32 - 64.0, max[k] as f32 + 64.0);
                            h[k] >= lo && h[k] <= hi
                        })
                };
                let region: Vec<usize> = (0..320 * 180)
                    .filter(|&i| near(&last.0.hits[support::defects::grid_of(i, 320, 180, gw, gl)]))
                    .collect();
                let gregion: Vec<usize> = (0..gl).filter(|&g| near(&last.0.hits[g])).collect();
                let settled = lum.last().expect("frames");
                let neigh: Vec<f32> = (0..320 * 180)
                    .map(|i| {
                        let (x, y) = (i % 320, i / 320);
                        let (mut s, mut n) = (0.0, 0.0);
                        for dy in -2..=2 {
                            for dx in -2..=2 {
                                let (u, v) = (x + dx, y + dy);
                                if (0..320).contains(&u) && (0..180).contains(&v) {
                                    s += settled[(v * 320 + u) as usize];
                                    n += 1.0;
                                }
                            }
                        }
                        s / n
                    })
                    .collect();
                println!(
                    "reading: {arm}: drained at {drained:?}, exposure {:.3} EV, box {min:?}..{max:?}, region {} px ({} lighting px), on {}",
                    ex.log2(),
                    region.len(),
                    gregion.len(),
                    rig.adapter
                );
                let mut sum = 0usize;
                for k in 0..shots.len() {
                    let excess = region
                        .iter()
                        .filter(|&&i| lum[k][i] > 2.0 * settled[i] && lum[k][i] > 2.0 * neigh[i])
                        .count();
                    if k < 60 {
                        sum += excess;
                    }
                    let moved = if k == 0 {
                        0
                    } else {
                        region
                            .iter()
                            .filter(|&&i| (lum[k][i] - lum[k - 1][i]).abs() > 16.0)
                            .count()
                    };
                    let (sh, digs, settled_q) = &shots[k];
                    let cast = gregion.iter().filter(|&&g| sh.cast[g]).count();
                    let age = gregion.iter().map(|&g| f64::from(sh.age[g])).sum::<f64>()
                        / gregion.len().max(1) as f64;
                    if k < 20 || k % 10 == 9 {
                        println!(
                            "reading: {arm}: frame {}: excess {excess} ({:.2} %), moved {:.2} %, cast {cast}, age {age:.1}, light {:.3} ms, digs {digs}, settled {settled_q}",
                            k + 1,
                            100.0 * excess as f64 / region.len().max(1) as f64,
                            100.0 * moved as f64 / region.len().max(1) as f64,
                            sh.light_ms.unwrap_or(f64::NAN)
                        );
                    }
                }
                println!("reading: {arm}: excess summed over frames 1 to 60: {sum}");
            }
        }

        /// Plan 0086 Phase 16 reading 4 (defect 6, the clipped treads): [`treads_clip`] over
        /// 128 frames under `sky_anchor_ev` none and 4, printed. `SVET_TEST_ADAPTER` picks the
        /// adapter.
        #[test]
        #[ignore = "reading"]
        fn the_forest_treads_clip() {
            for anchor in [None, Some(4.0)] {
                treads_clip(anchor, 128);
            }
        }

        /// The forest of seed 1 (`forest_rig`) at 15:00 baked whole, voxel leaves, 320 x 180
        /// native, the outdoor auto exposure (instant) under `anchor` for `frames` frames: at the
        /// last frame the share of hit pixels at 255 in the display's largest channel (percent)
        /// and the sunlit treads' (display pixels on a horizontal face whose direct part is over
        /// 4 times the stored one) mean level, printed with the exposure and the ceiling. `None`
        /// without an adapter.
        pub(in super::super) fn treads_clip(anchor: Option<f32>, frames: u32) -> Option<(f64, f64)> {
            use svet_render::debug::{read_rgba8, read_rgba16f, read_rgba32f};
            let (mut rig, cam) = support::terms::forest_rig()?;
            let sky = flythrough::daylight::Clock {
                minutes: 15.0 * 60.0,
                ..Default::default()
            }
            .sky(flythrough::outdoor::BASE_Z as f32);
            let f = defect_frame(
                &mut rig,
                Some(sky),
                |l| svet_render::Lighting {
                    exposure: svet_render::Exposure::Auto(svet_render::AutoExposure {
                        min: flythrough::cli::OUTDOOR_MIN,
                        max: flythrough::cli::OUTDOOR_MAX,
                        sky_anchor_ev: anchor,
                        ..svet_render::AutoExposure::instant()
                    }),
                    ..l
                },
                Default::default(),
                true,
            );
            let mut s = support::defects::Stepper::new(&rig, f);
            for _ in 0..frames {
                s.run(&rig, &cam);
            }
            let (d, q) = (&rig.device, &rig.queue);
            let f = &s.frame;
            let disp = support::defects::level(&read_rgba8(d, q, f.target()));
            let hits = read_rgba32f(d, q, f.light_hit_positions());
            let acc = read_rgba16f(d, q, f.accumulated());
            let dir = read_rgba16f(d, q, f.direct_accumulated());
            let lum = |p: &[f32; 4]| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
            let (gl, gw) = (hits.len(), 160.min(hits.len()));
            let (mut hit, mut clip, mut tread, mut tread_sum) = (0usize, 0usize, 0usize, 0.0f64);
            for (i, &v) in disp.iter().enumerate() {
                let g = support::defects::grid_of(i, 320, 180, gw, gl);
                if hits[g][3] >= 1.0e30 {
                    continue;
                }
                hit += 1;
                clip += usize::from(v == 255);
                let h = hits[g];
                if (h[2] - h[2].round()).abs() < 1e-3 && lum(&dir[g]) > 4.0 * lum(&acc[g]) {
                    tread += 1;
                    tread_sum += f64::from(v);
                }
            }
            let st = svet_render::debug::read_buffer(d, q, f.exposure_state(), 0, 32);
            let state = svet_render::ExposureState::from_bytes(&st);
            let share = 100.0 * clip as f64 / hit.max(1) as f64;
            let mean = tread_sum / tread.max(1) as f64;
            println!(
                "reading: the forest at 15:00, anchor {anchor:?}, {frames} frames on {}: {share:.3} % of {hit} hit px at 255; sunlit treads {tread} px, mean level {mean:.1}; exposure {:.2} EV, ceiling {:.2} EV",
                rig.adapter,
                state.current.log2(),
                state.ceiling.log2(),
            );
            Some((share, mean))
        }
    }
}
