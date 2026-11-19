//! The outdoor term matrix's rigs (Plan 0086's readings): a world mirrored whole, the tunnel
//! and the forest of seed 1, the sky ray's hit rate per pixel, a cell of the matrix run over 64
//! still frames and its printed row, the key ray's steps, a region's mean display level.

use super::stillness::change_stats;
use vxe_render::{AutoExposure, Exposure, Frame, LightParams, Lighting};

/// A world mirrored whole for the term matrix: the device, the mirror and the adapter's
/// name.
pub struct Rig {
    pub device: vxe_render::wgpu::Device,
    pub queue: vxe_render::wgpu::Queue,
    pub gpu: vxe_render::GpuWorld,
    pub adapter: String,
    pub world: vxe_world::World,
}

/// `world` mirrored whole (an absent chunk is sky to a lighting ray) with the focus at `eye`.
pub fn mirrored(world: vxe_world::World, eye: vxe_world::VoxelCoord) -> Option<Rig> {
    use super::SyncNow;
    use vxe_render::{GpuWorld, MirrorConfig, Window};
    let pool = flythrough::level_pool_units(&world, false) * vxe_render::internals::UNIT_BYTES;
    let (device, queue, adapter) = super::reading_device(pool)?;
    let cfg = MirrorConfig {
        budget_bytes: 512 << 20,
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

/// The tunnel of seed 1 (`--source outdoor --spawn inside`): 3 chunks around the spawn's
/// chunk and on to 10 along +x and 4 down, the eye and its view out of the tunnel and into
/// it.
pub fn tunnel_rig() -> Option<(Rig, [(&'static str, vxe_render::Camera); 2])> {
    use flythrough::outdoor::{OutdoorSource, Spawn};
    use vxe_world::{ChunkExtent, World};
    let source = OutdoorSource::new(1);
    let (eye, facing) = source.spawn(Spawn::Inside);
    let c = eye.chunk().0;
    let extent = ChunkExtent::new(
        [c[0] - 3, c[1] - 3, c[2] - 4],
        [c[0] + 11, c[1] + 4, c[2] + 2],
    );
    let world = World::generate(extent, &source, &flythrough::outdoor_materials());
    let rig = mirrored(world, eye)?;
    let p = eye.0.map(|v| v as f32 + 0.5);
    let d = facing.map(|v| v as f32);
    let out = flythrough::camera_looking_at(p, [p[0] + d[0], p[1] + d[1], p[2] + d[2]]);
    let inward = flythrough::camera_looking_at(p, [p[0] - d[0], p[1] - d[1], p[2] - d[2]]);
    Some((rig, [("tunnel out", out), ("tunnel in", inward)]))
}

/// The hills of seed 1 (`--source outdoor --spawn outside`, no trees): the chunks within 4 of
/// the spawn's (2 in z), the spawn's camera facing the cave mouth.
pub fn hills_rig() -> Option<(Rig, vxe_render::Camera)> {
    use flythrough::outdoor::{OutdoorSource, Spawn};
    use vxe_world::{ChunkExtent, World};
    let source = OutdoorSource::new(1);
    let (eye, facing) = source.spawn(Spawn::Outside);
    let c = eye.chunk().0;
    let extent = ChunkExtent::new(
        [c[0] - 4, c[1] - 4, c[2] - 2],
        [c[0] + 5, c[1] + 5, c[2] + 3],
    );
    let world = World::generate(extent, &source, &flythrough::outdoor_materials());
    let rig = mirrored(world, eye)?;
    let p = eye.0.map(|v| v as f32 + 0.5);
    let d = facing.map(|v| v as f32);
    let cam = flythrough::camera_looking_at(p, [p[0] + d[0], p[1] + d[1], p[2] + d[2]]);
    Some((rig, cam))
}

/// The forest of seed 1 at `LEAF_FILL` (`--spawn forest --trees forest`): the chunks within
/// 4 of the spawn's (2 in z), the spawn's camera.
pub fn forest_rig() -> Option<(Rig, vxe_render::Camera)> {
    use flythrough::outdoor::{OutdoorSource, Spawn};
    use flythrough::trees::{ForestSource, LEAF_FILL, TreeDensity};
    use vxe_world::{ChunkExtent, World};
    let source = ForestSource::new(OutdoorSource::new(1), TreeDensity::Forest, LEAF_FILL);
    let (eye, facing) = source.base().spawn(Spawn::Forest);
    let c = eye.chunk().0;
    let extent = ChunkExtent::new(
        [c[0] - 4, c[1] - 4, c[2] - 2],
        [c[0] + 5, c[1] + 5, c[2] + 3],
    );
    let world = World::generate(extent, &source, &flythrough::outdoor_materials());
    let rig = mirrored(world, eye)?;
    let p = eye.0.map(|v| v as f32 + 0.5);
    let d = facing.map(|v| v as f32);
    let cam = flythrough::camera_looking_at(p, [p[0] + d[0], p[1] + d[1], p[2] + d[2]]);
    Some((rig, cam))
}

/// The sky ray's hit rate per lighting pixel of a native 320 x 180 frame of `cam` (64 frames
/// under `Sky::uniform(1.0)`, the filter and the key light off, exposure 1), `None` at a
/// miss, and the grid's width.
pub fn hit_rates(rig: &Rig, cam: &vxe_render::Camera) -> (Vec<Option<f64>>, usize) {
    use vxe_render::sky::Sky;
    let mut f = Frame::new(&rig.device, 320, 180);
    f.set_lighting(Lighting {
        lamp: false,
        filter: false,
        exposure: Exposure::Manual(1.0),
        sky_rays: 1,
        ..flythrough::default_lighting()
    });
    f.set_sky(Some(Sky {
        altitude_zero_z: flythrough::outdoor::BASE_Z as f32,
        ..Sky::uniform(1.0)
    }));
    let mut sum: Vec<f64> = Vec::new();
    for _ in 0..64 {
        f.step(
            &rig.device,
            &rig.queue,
            &rig.gpu,
            cam,
            &LightParams::headlamp(),
        );
        // The sky rays' share that reached the sky (ADR-0070), in r of 255.
        let px = vxe_render::debug::read_rgba8(&rig.device, &rig.queue, f.visibility_samples());
        sum.resize(px.len() / 4, 0.0);
        for (s, p) in sum.iter_mut().zip(px.chunks(4)) {
            *s += f64::from(p[0]) / 255.0 / 64.0;
        }
    }
    let hits = vxe_render::debug::read_rgba32f(&rig.device, &rig.queue, f.light_hit_positions());
    let rate = sum
        .iter()
        .zip(&hits)
        .map(|(s, h)| (h[3] < 1.0e30).then_some(*s))
        .collect();
    (rate, f.direct().width() as usize)
}

/// One cell of the term matrix.
#[derive(Clone, Copy, Debug)]
pub struct Cell {
    /// "key": `sky_rays` 0; "sky": the key light hooked off; "both".
    pub term: &'static str,
    pub filter: bool,
    /// The history's material and normal tests hooked off.
    pub any_history: bool,
    pub fsr2: bool,
    /// How the testbed's `leaf` shadows (`--leaf-shadow`).
    pub leaf: flythrough::LeafShadow,
    /// The far normal's stair rules hooked off.
    pub no_stair: bool,
    /// The store baked whole (ADR-0046's one-off bake at the defaults, the sky's channel
    /// with it) before the first frame.
    pub bake: bool,
    /// The key light's penumbra is the closest approach's (`--penumbra approach`).
    pub approach: bool,
    /// The auto exposure's ceiling anchored this many EV over the sunlit ground's exposure
    /// (`--sky-anchor N`); `None` keeps `OUTDOOR_MAX`.
    pub anchor: Option<f32>,
    /// The baked sky term's normal (`--sky-normal`) and bounce floor (`--sky-bounce`).
    pub sky_normal: vxe_render::SkyNormal,
    pub sky_bounce: f32,
}

/// What a cell read over its 64 still frames.
pub struct CellReading {
    /// Per display pixel, its frame-to-frame changes (levels of 255, largest channel).
    pub changes: Vec<Vec<u8>>,
    /// The display's width.
    pub width: usize,
    /// Per frame, the history counters: tested, material, normal, other.
    pub rejections: Vec<[u32; 4]>,
    /// The light pass's direct samples' luminance per frame and lighting pixel.
    pub samples: Vec<Vec<f32>>,
    pub grid_width: usize,
    pub exposure: f32,
    /// The ceiling the auto exposure was clamped under at its 64th frame.
    pub ceiling: f32,
    /// The stripe metric at the last frame: over the lighting pixels whose hit is within 48
    /// voxels, the mean luminance of the stored and direct accumulations at hits on a
    /// horizontal face (a tread) and on a vertical one (a riser), and their counts.
    pub stripe: [(f64, u32); 2],
    /// The last frame's mean luminance over every hit of the stored accumulation (the
    /// emissive and the baked sky part) and of the direct one (the key light and the sky
    /// rays' pair), pre-exposed.
    pub parts: [f64; 2],
    /// The last frame's display (rgba8).
    pub last: Vec<u8>,
}

/// `cell` at `cam` under `sky` (the key light and the sky rays at the defaults otherwise, the
/// lamp off): 64 frames under the outdoor auto exposure (instant), then 64 at that exposure
/// pinned, reading the display's changes, the history counters and the direct samples.
#[allow(
    clippy::disallowed_methods,
    reason = "the reading prints the bake's wall time"
)]
pub fn run_cell(
    rig: &mut Rig,
    cam: &vxe_render::Camera,
    sky: vxe_render::sky::Sky,
    cell: Cell,
) -> CellReading {
    use vxe_render::{LightHooks, UpscalerKind};
    let mut f = if cell.fsr2 {
        let mut f = Frame::with_sizes(&rig.device, [320, 180], [480, 270]);
        f.set_upscaler_kind(
            &rig.device,
            UpscalerKind::Fsr2(vxe_render::Fsr2Settings::default()),
        );
        f
    } else {
        Frame::new(&rig.device, 320, 180)
    };
    let base = Lighting {
        lamp: false,
        filter: cell.filter,
        sky_rays: if cell.term == "key" { 0 } else { 1 },
        penumbra: if cell.approach {
            vxe_render::Penumbra::Approach
        } else {
            vxe_render::Penumbra::Disc
        },
        sky_normal: cell.sky_normal,
        sky_bounce: cell.sky_bounce,
        ..flythrough::default_lighting()
    };
    f.set_light_hooks(LightHooks {
        key_off: cell.term == "sky",
        any_history: cell.any_history,
        no_stair: cell.no_stair,
        ..LightHooks::default()
    });
    f.set_count_store(true);
    // The fly-through's default look: cubes with the far normal at [2, 6].
    f.set_look(vxe_render::LookParams {
        far_normal: flythrough::LookPreset::DEFAULT.far_normal,
        ..vxe_render::LookParams::default()
    });
    f.set_material_styles(&rig.queue, &flythrough::outdoor_styles(cell.leaf));
    f.set_lighting(Lighting {
        exposure: Exposure::Auto(AutoExposure {
            min: flythrough::cli::OUTDOOR_MIN,
            max: flythrough::cli::OUTDOOR_MAX,
            sky_anchor_ev: cell.anchor,
            ..AutoExposure::instant()
        }),
        ..base
    });
    f.set_sky(Some(sky));
    if cell.bake {
        let t = std::time::Instant::now();
        let report = f
            .bake_until(
                &rig.device,
                &rig.queue,
                &mut rig.gpu,
                &rig.world,
                rig.world.extent(),
                &vxe_render::BakeSettings::default(),
                &mut |_| {},
            )
            .expect("the store fits");
        println!(
            "reading: bake {} chunks, {} faces, {} samples, {} sky rays in {:.1} s",
            report.chunks,
            report.faces,
            report.samples,
            report.sky_rays,
            t.elapsed().as_secs_f64()
        );
    }
    f.set_dt(1.0 / 60.0);
    let light = LightParams::headlamp();
    for _ in 0..64 {
        f.step(&rig.device, &rig.queue, &rig.gpu, cam, &light);
    }
    // The state buffer read back blocking: the exposure the next frame uses (the readout's
    // ring returns an older frame's state).
    let st = vxe_render::debug::read_buffer(&rig.device, &rig.queue, f.exposure_state(), 0, 32);
    let state = vxe_render::ExposureState::from_bytes(&st);
    let (exposure, ceiling) = (state.current, state.ceiling);
    f.set_lighting(Lighting {
        exposure: Exposure::Manual(exposure),
        ..base
    });
    let read = |f: &Frame| vxe_render::debug::read_rgba8(&rig.device, &rig.queue, f.target());
    let mut last = read(&f);
    let mut changes: Vec<Vec<u8>> = vec![Vec::new(); last.len() / 4];
    let mut rejections = Vec::new();
    let mut samples = Vec::new();
    for _ in 0..64 {
        f.step(&rig.device, &rig.queue, &rig.gpu, cam, &light);
        let now = read(&f);
        for (i, (a, b)) in now.chunks(4).zip(last.chunks(4)).enumerate() {
            changes[i].push((0..3).map(|k| a[k].abs_diff(b[k])).max().unwrap_or(0));
        }
        last = now;
        rejections.push(f.read_history_rejections(&rig.device, &rig.queue));
        let d = vxe_render::debug::read_rgba16f(&rig.device, &rig.queue, f.direct());
        samples.push(d.iter().map(|p| p[3]).collect());
    }
    let parts: [f64; 2];
    let stripe = {
        let hits =
            vxe_render::debug::read_rgba32f(&rig.device, &rig.queue, f.light_hit_positions());
        let a = vxe_render::debug::read_rgba16f(&rig.device, &rig.queue, f.accumulated());
        let d = vxe_render::debug::read_rgba16f(&rig.device, &rig.queue, f.direct_accumulated());
        let on = |x: f32| (x - x.round()).abs() < 1e-3;
        let mut s = [(0.0, 0u32); 2];
        let lum = |p: &[f32; 4]| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
        let (mut pa, mut pd, mut n) = (0.0, 0.0, 0u32);
        for ((h, a), d) in hits.iter().zip(&a).zip(&d) {
            if h[3] < 1.0e30 {
                pa += f64::from(lum(a));
                pd += f64::from(lum(d));
                n += 1;
            }
        }
        parts = [pa / f64::from(n.max(1)), pd / f64::from(n.max(1))];
        for ((h, a), d) in hits.iter().zip(&a).zip(&d) {
            if h[3] > 48.0 {
                continue;
            }
            let k = if on(h[2]) {
                0
            } else if on(h[0]) || on(h[1]) {
                1
            } else {
                continue;
            };
            s[k].0 += f64::from(lum(a) + lum(d));
            s[k].1 += 1;
        }
        s
    };
    CellReading {
        last,
        width: f.target().width() as usize,
        changes,
        rejections,
        samples,
        grid_width: f.direct().width() as usize,
        exposure,
        ceiling,
        stripe,
        parts,
    }
}

/// Every cell of the matrix: the term (key, sky, both) by the filter (off, on) by the history
/// (as built, any) by the upscaler (native, FSR 2 at 1.5).
pub fn cells() -> Vec<Cell> {
    let mut v = Vec::new();
    for fsr2 in [false, true] {
        for term in ["key", "sky", "both"] {
            for filter in [false, true] {
                for any_history in [false, true] {
                    v.push(Cell {
                        term,
                        filter,
                        any_history,
                        fsr2,
                        leaf: flythrough::LeafShadow::Voxel,
                        no_stair: false,
                        bake: false,
                        approach: false,
                        anchor: None,
                        sky_normal: vxe_render::SkyNormal::Face,
                        sky_bounce: 0.0,
                    });
                }
            }
        }
    }
    v
}

/// A named region of a 320 x 180 frame's pixels.
pub type Region<'a> = (&'a str, &'a dyn Fn(usize, usize) -> bool);

/// One table row: the cell, then per region its p99 and mean change, then the region's
/// firefly rate (direct samples over four times the region's mean sample), then the
/// history's reset share by cause over the frame's tested hits.
pub fn print_row(pose: &str, cell: Cell, r: &CellReading, regions: &[Region<'_>]) {
    let mut line = format!(
        "{pose:10} | {:4} | filter {:3} | history {:5} | {:6}",
        cell.term,
        if cell.filter { "on" } else { "off" },
        if cell.any_history { "any" } else { "built" },
        if cell.fsr2 { "fsr2" } else { "native" },
    );
    let height = r.changes.len() / r.width;
    let gh = r.samples[0].len() / r.grid_width;
    for (name, inside) in regions {
        let (p99, mean, n) = change_stats(&r.changes, |i| {
            inside(i % r.width * 320 / r.width, i / r.width * 180 / height)
        });
        // The region's direct samples on the lighting grid.
        let grid: Vec<usize> = (0..r.grid_width * gh)
            .filter(|&i| {
                inside(
                    i % r.grid_width * 320 / r.grid_width,
                    i / r.grid_width * 180 / gh,
                )
            })
            .collect();
        let all: Vec<f32> = r
            .samples
            .iter()
            .flat_map(|s| grid.iter().map(|&i| s[i]))
            .collect();
        let m = all.iter().map(|&v| f64::from(v)).sum::<f64>() / all.len().max(1) as f64;
        let ff = all
            .iter()
            .filter(|&&v| f64::from(v) > 4.0 * m && m > 0.0)
            .count() as f64
            / all.len().max(1) as f64;
        line += &format!(
            " | {name} p99 {} mean {mean:.2} ({n} px) ff {ff:.4}",
            p99.map_or("-".into(), |v| v.to_string())
        );
    }
    let t: [u64; 4] = std::array::from_fn(|k| r.rejections.iter().map(|w| u64::from(w[k])).sum());
    let share = |k: usize| t[k] as f64 / t[0].max(1) as f64;
    line += &format!(
        " | reset {:.4} (material {:.4} normal {:.4} other {:.4}) | exposure {:.3e}",
        share(1) + share(2) + share(3),
        share(1),
        share(2),
        share(3),
        r.exposure
    );
    println!("reading: {line}");
}

/// The defaults' cell: both terms, the filter on, the history as built, FSR 2 at 1.5.
pub fn default_cell(leaf: flythrough::LeafShadow) -> Cell {
    Cell {
        term: "both",
        filter: true,
        any_history: false,
        fsr2: true,
        leaf,
        no_stair: false,
        bake: false,
        approach: false,
        anchor: None,
        sky_normal: vxe_render::SkyNormal::Face,
        sky_bounce: 0.0,
    }
}

/// The key light's shadow ray's steps in the steps view at `cam` under `sky` (native 320 x
/// 180, the defaults with `penumbra`, the lamp off), over the lighting pixels that cast one:
/// the mean and the 99th percentile, and the mean storage loads, over 4 frames.
pub fn key_steps(
    rig: &Rig,
    cam: &vxe_render::Camera,
    sky: vxe_render::sky::Sky,
    leaf: flythrough::LeafShadow,
    penumbra: vxe_render::Penumbra,
) -> (f64, f32, f64) {
    use vxe_render::{LookParams, ViewMode};
    let mut f = Frame::new(&rig.device, 320, 180);
    f.set_lighting(Lighting {
        lamp: false,
        penumbra,
        ..flythrough::default_lighting()
    });
    f.set_look(LookParams {
        view: ViewMode::Steps,
        ..LookParams::default()
    });
    f.set_material_styles(&rig.queue, &flythrough::outdoor_styles(leaf));
    f.set_sky(Some(sky));
    let mut steps: Vec<f32> = Vec::new();
    let mut loads = 0.0f64;
    for _ in 0..4 {
        f.step(
            &rig.device,
            &rig.queue,
            &rig.gpu,
            cam,
            &LightParams::headlamp(),
        );
        let d = vxe_render::debug::read_rgba16f(&rig.device, &rig.queue, f.direct());
        steps.extend(d.iter().filter(|p| p[3] > 0.5).map(|p| p[0]));
        loads += d
            .iter()
            .filter(|p| p[3] > 0.5)
            .map(|p| f64::from(p[1]))
            .sum::<f64>();
    }
    steps.sort_by(f32::total_cmp);
    let n = steps.len().max(1) as f64;
    let mean = steps.iter().map(|&v| f64::from(v)).sum::<f64>() / n;
    (
        mean,
        steps.get(steps.len() * 99 / 100).copied().unwrap_or(0.0),
        loads / n,
    )
}

/// The mean display level (largest channel, levels of 255) of `r`'s last frame over the
/// pixels of `inside` (at 320 x 180).
pub fn region_level(r: &CellReading, inside: &dyn Fn(usize, usize) -> bool) -> f64 {
    let height = r.last.len() / 4 / r.width;
    let (mut sum, mut n) = (0.0, 0usize);
    for (i, p) in r.last.chunks(4).enumerate() {
        if inside(i % r.width * 320 / r.width, i / r.width * 180 / height) {
            sum += f64::from(p[0].max(p[1]).max(p[2]));
            n += 1;
        }
    }
    sum / n.max(1) as f64
}
