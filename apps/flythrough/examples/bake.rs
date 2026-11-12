//! The loaded light's readings (ADR-0046, Plan 0059; lane rule 9c): measurements on a named
//! adapter that print numbers and append them to a log under `live/bake-count/`, asserting
//! nothing. The bake's asserted tests are `tests/gpu_bake.rs`, over the same rigs
//! (`tests/support/bake.rs`).
//!
//! `cargo run --release -p flythrough --example bake -- --read NAME [--adapter SUBSTRING]
//! [--views LIST] [--size WxH] [--far-normal LO,HI] [--ref-hall whole|r2]`; `--help` lists the
//! flags and the readings. Run it under the perf lock: it opens a GPU device.

#![allow(
    clippy::disallowed_methods,
    reason = "the readings time their bakes on the wall clock"
)]

#[path = "../tests/support/mod.rs"]
mod support;

use std::collections::BTreeMap;

use flythrough::cave::ROCK;
use flythrough::cli::table::{Flag, Section, Table};
use support::adapter_device;
use support::bake::{
    Rig, StairArm, StairFrame, brick_value, by_normal, cells_of, frame_blocks, hit_face, luma, rgb,
    rig_on, stair_cameras, stair_frame, staircase, test_counts, within,
};
use support::rigs::tone_bytes;
use support::{SyncNow, cave_table};
use vxe_render::{
    BakeSettings, Camera, Exposure, Frame, GpuWorld, LightParams, Lighting, LightingResolution,
    MirrorConfig, Window, internals::UNIT_BYTES, wgpu,
};
use vxe_world::{ChunkExtent, EditOp, FIXED_ONE, Fixed3, Mode, World};

/// What the flags set.
struct Args {
    /// The reading to run, a name of [`READINGS`].
    read: Option<&'static str>,
    /// The adapter's name, or a part of it; empty is lavapipe.
    adapter: String,
    /// The views the resolution, rest and pass readings read, by name, comma-separated.
    views: String,
    /// The glass and rest readings' frame size.
    size: [u32; 2],
    /// The pass reading's far normal.
    far_normal: [f32; 2],
    /// The hall's bake piece is the chunks within 2 of its eye, not the whole world.
    hall_r2: bool,
    help: bool,
}

/// A reading `--read` names: its name, one line of help, and its run.
struct Reading {
    name: &'static str,
    help: &'static str,
    run: fn(&Args),
}

/// The readings, in `--help`'s order.
const READINGS: [Reading; 7] = [
    Reading {
        name: "adapter_records",
        help: "the vein piece's records on --adapter, compared across the RTX's and the Radeon's files",
        run: adapter_records,
    },
    Reading {
        name: "glass_readings",
        help: "the hall's glass pane with the lamp off: its reflected term and share",
        run: glass_readings,
    },
    Reading {
        name: "queue_dig_readings",
        help: "the live queue's dig against a fresh bake (on the RTX)",
        run: queue_dig_readings,
    },
    Reading {
        name: "resolution_readings",
        help: "the resolution cut under the brick store (on the RTX)",
        run: resolution_readings,
    },
    Reading {
        name: "rest_readings",
        help: "the light's budget at rest: passes, one frame's change and the drift",
        run: rest_readings,
    },
    Reading {
        name: "pass_readings",
        help: "the per-pass table: tiers, lamp, at rest and walking",
        run: pass_readings,
    },
    Reading {
        name: "staircase_read",
        help: "Plan 0061 Phase 2's staircase claims, on lavapipe",
        run: staircase_read,
    },
];

/// A `A<sep>B` pair of numbers, or `None`.
fn pair<V: std::str::FromStr>(text: &str, sep: char) -> Option<[V; 2]> {
    let (a, b) = text.split_once(sep)?;
    Some([a.parse().ok()?, b.parse().ok()?])
}

/// The example's flag table (`flythrough::cli::table`).
fn table() -> Table<Args> {
    Table {
        usage: "bake --read NAME [FLAGS]: one loaded-light reading (see the example's doc comment)",
        sections: vec![Section {
            title: "Reading",
            flags: vec![
                Flag::switch(
                    &["--help", "-h"],
                    "print this help and exit",
                    |a: &mut Args| {
                        a.help = true;
                    },
                ),
                Flag::value(
                    &["--read"],
                    "NAME",
                    "the reading to run (the readings are listed below)",
                    |a: &mut Args, v| {
                        let names: Vec<&str> = READINGS.iter().map(|r| r.name).collect();
                        a.read = Some(
                            READINGS
                                .iter()
                                .find(|r| r.name == v.text)
                                .map(|r| r.name)
                                .ok_or_else(|| v.takes(&names.join(", ")))?,
                        );
                        Ok(())
                    },
                ),
                Flag::value(
                    &["--adapter"],
                    "SUBSTRING",
                    "the first Vulkan adapter whose name contains it, ignoring case (lavapipe)",
                    |a: &mut Args, v| {
                        a.adapter = v.string();
                        Ok(())
                    },
                ),
                Flag::value(
                    &["--views"],
                    "LIST",
                    "the resolution, rest and pass readings' views of vein, back, patch, hall (vein,hall)",
                    |a: &mut Args, v| {
                        a.views = v.string();
                        Ok(())
                    },
                ),
                Flag::value(
                    &["--size"],
                    "WxH",
                    "the glass and rest readings' frame size (the window's native 2530x1378)",
                    |a: &mut Args, v| {
                        a.size = pair(v.text, 'x').ok_or_else(|| v.takes("WxH"))?;
                        Ok(())
                    },
                ),
                Flag::value(
                    &["--far-normal"],
                    "LO,HI",
                    "the pass reading's far normal (2,6, the fly-through's)",
                    |a: &mut Args, v| {
                        a.far_normal = pair(v.text, ',').ok_or_else(|| v.takes("LO,HI"))?;
                        Ok(())
                    },
                ),
                Flag::value(
                    &["--ref-hall"],
                    "whole|r2",
                    "the hall's bake piece: the whole world, or the chunks within 2 of its eye (whole)",
                    |a: &mut Args, v| {
                        a.hall_r2 = v.word(&[("whole", false), ("r2", true)], "whole or r2")?;
                        Ok(())
                    },
                ),
            ],
        }],
    }
}

fn parse_args_from(argv: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut args = Args {
        read: None,
        adapter: String::new(),
        views: "vein,hall".to_owned(),
        size: NATIVE,
        far_normal: [2.0, 6.0],
        hall_r2: false,
        help: false,
    };
    table().parse_into(argv, &mut args)?;
    Ok(args)
}

/// `--help`: the table's rows, then the readings `--read` takes.
fn help() -> String {
    let mut out = table().help();
    out.push_str("\nReadings:\n");
    let width = READINGS.iter().map(|r| r.name.len()).max().unwrap_or(0);
    for r in &READINGS {
        out.push_str(&format!("  {:width$}  {}\n", r.name, r.help));
    }
    out
}

fn main() -> Result<(), String> {
    let args = parse_args_from(std::env::args().skip(1))?;
    if args.help {
        print!("{}", help());
        return Ok(());
    }
    let name = args
        .read
        .ok_or("--read names the reading to run; --help lists them")?;
    let reading = READINGS
        .iter()
        .find(|r| r.name == name)
        .expect("--read takes a listed name");
    (reading.run)(&args);
    Ok(())
}

// ---- The readings' views ----

/// A view of the reading: the name, the world and the eye.
#[derive(Clone, Copy, PartialEq)]
enum Scene {
    Level,
    Showcase,
}

struct ReadView {
    name: &'static str,
    scene: Scene,
    eye: [f32; 3],
    /// The hall's piece is the chunks within 2 of its eye, not the whole world (`--ref-hall r2`).
    hall_r2: bool,
}

/// Every view, the hall's piece by `hall_r2`.
fn read_views(hall_r2: bool) -> Vec<ReadView> {
    use flythrough::{PATCH_POSE, VEIN_POSE};
    let hall = flythrough::showcase::SHOTS[0].eye.0;
    vec![
        ReadView {
            name: "vein",
            scene: Scene::Level,
            eye: VEIN_POSE.position,
            hall_r2,
        },
        ReadView {
            name: "back",
            scene: Scene::Level,
            eye: support::settle::VEIN_FLY_IN_START,
            hall_r2,
        },
        ReadView {
            name: "patch",
            scene: Scene::Level,
            eye: PATCH_POSE.position,
            hall_r2,
        },
        ReadView {
            name: "hall",
            scene: Scene::Showcase,
            eye: hall.map(|v| v as f32),
            hall_r2,
        },
    ]
}

fn scene_world(scene: Scene) -> World {
    match scene {
        Scene::Level => {
            let source = flythrough::cave::CaveSource::new(
                1,
                ChunkExtent::new([0, 0, 0], flythrough::cave::LEVEL_CHUNKS),
            );
            World::generate(source.extent, &source, &cave_table())
        }
        Scene::Showcase => {
            let source = flythrough::showcase::ShowcaseSource::new(1);
            World::generate(source.extent(), &source, &flythrough::showcase_materials())
        }
    }
}

struct ReadRig {
    device: wgpu::Device,
    queue: wgpu::Queue,
    gpu: GpuWorld,
    world: World,
    frame: Frame,
    adapter: String,
}

fn read_rig(scene: Scene, eye: [f32; 3], adapter: &str) -> ReadRig {
    let world = scene_world(scene);
    let pool = flythrough::level_pool_units(&world, false) * UNIT_BYTES;
    let (device, queue, adapter) = adapter_device(adapter, "--adapter ", pool).expect("a device");
    let cfg = MirrorConfig {
        budget_bytes: 512 << 20,
        light_radius: if scene == Scene::Showcase { 19 } else { 4 },
        ..MirrorConfig::new(Window::Fixed(world.extent()))
    };
    let mut gpu = GpuWorld::with_config(&device, cfg);
    gpu.set_focus(vxe_world::VoxelCoord(eye.map(|v| v as i32)));
    for _ in 0..8 {
        gpu.sync_now(&world, &device, &queue)
            .expect("the mirror fits the adapter");
    }
    let mut frame = Frame::new(&device, 16, 16);
    frame.set_lighting(Lighting {
        lamp: false,
        lamp_bounce: false,
        exposure: Exposure::Manual(8.0),
        ..Lighting::default()
    });
    ReadRig {
        device,
        queue,
        gpu,
        world,
        frame,
        adapter,
    }
}

/// A log under `live/`, written line by line so it outlives a lost device.
struct Log(std::fs::File);

impl Log {
    fn open(name: &str) -> Self {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../live/bake-count");
        std::fs::create_dir_all(dir).expect("live/bake-count");
        Self(
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(format!("{dir}/{name}"))
                .expect("the log"),
        )
    }

    fn line(&mut self, s: &str) {
        use std::io::Write as _;
        println!("{s}");
        writeln!(self.0, "{s}").expect("the log");
        self.0.flush().expect("the log");
    }
}

// ---- The readings on a named adapter ----

/// The window's native size (the owner's 2530 x 1378).
const NATIVE: [u32; 2] = [2530, 1378];

/// A view's piece: its name, the piece's centre eye and its radius (`None`: the whole world).
fn file_of(view: &ReadView) -> (&'static str, [f32; 3], Option<i32>) {
    let vein = flythrough::VEIN_POSE.position;
    match view.name {
        "vein" | "back" => ("vein", vein, Some(2)),
        "patch" => ("patch", view.eye, Some(2)),
        _ => (
            if view.hall_r2 { "hall-r2" } else { "hall" },
            view.eye,
            view.hall_r2.then_some(2),
        ),
    }
}

/// The fly-through's `bake_extent`: the chunks within `radius` of the eye's chunk, in `world`.
fn piece_of(world: ChunkExtent, eye: [f32; 3], radius: Option<i32>) -> ChunkExtent {
    let Some(r) = radius else {
        return world;
    };
    let size = world.size();
    let c = eye.map(|v| (v / 64.0).floor() as i32);
    let lo = [0, 1, 2].map(|k| (c[k] - r).max(world.min.0[k]));
    let hi = [0, 1, 2].map(|k| (c[k] + r + 1).min(world.min.0[k] + size[k]));
    ChunkExtent::new(lo, hi)
}

/// The reading's world: the scene's, with the reference's glass row appended as the fly-through
/// does under `--reference`.
fn reference_world(scene: Scene) -> World {
    let glass = flythrough::castle_materials(&flythrough::cave_materials())
        .get(flythrough::castle::material(flythrough::castle::GLASS))
        .expect("the castle's glass")
        .clone();
    let mut table = match scene {
        Scene::Level => cave_table(),
        Scene::Showcase => flythrough::showcase_materials(),
    };
    table.push(glass);
    match scene {
        Scene::Level => {
            let source = flythrough::cave::CaveSource::new(
                1,
                ChunkExtent::new([0, 0, 0], flythrough::cave::LEVEL_CHUNKS),
            );
            World::generate(source.extent, &source, &table)
        }
        Scene::Showcase => {
            let source = flythrough::showcase::ShowcaseSource::new(1);
            World::generate(source.extent(), &source, &table)
        }
    }
}

/// A rig on the RTX over `view`'s world at `size`, native and full-resolution lighting, no jitter,
/// lamp off, exposure 8, an empty store: the rig and the piece.
fn view_rig(view: &ReadView, size: [u32; 2], adapter: &str) -> (ReadRig, ChunkExtent) {
    let world = reference_world(view.scene);
    let pool = flythrough::level_pool_units(&world, false) * UNIT_BYTES;
    let (device, queue, adapter) = adapter_device(adapter, "--adapter ", pool).expect("a device");
    let cfg = MirrorConfig {
        budget_bytes: 512 << 20,
        light_radius: if view.scene == Scene::Showcase { 19 } else { 4 },
        ..MirrorConfig::new(Window::Fixed(world.extent()))
    };
    let mut gpu = GpuWorld::with_config(&device, cfg);
    let (_, centre_eye, radius) = file_of(view);
    gpu.set_focus(vxe_world::VoxelCoord(view.eye.map(|v| v as i32)));
    for _ in 0..8 {
        gpu.sync_now(&world, &device, &queue)
            .expect("the mirror fits the adapter");
    }
    let mut frame = Frame::new(&device, size[0], size[1]);
    frame.set_jitter(&device, false);
    frame.set_resolution(&device, LightingResolution::Full);
    frame.set_lighting(reference_lighting(false));
    frame.set_dt(1.0 / 60.0);
    let piece = piece_of(world.extent(), centre_eye, radius);
    (
        ReadRig {
            device,
            queue,
            gpu,
            world,
            frame,
            adapter,
        },
        piece,
    )
}

fn reference_lighting(lamp: bool) -> Lighting {
    Lighting {
        lamp,
        lamp_bounce: false,
        exposure: Exposure::Manual(8.0),
        ..Lighting::default()
    }
}

fn inside(piece: ChunkExtent, v: [i32; 3]) -> bool {
    let s = piece.size();
    (0..3).all(|k| {
        let c = v[k].div_euclid(64);
        c >= piece.min.0[k] && c < piece.min.0[k] + s[k]
    })
}

fn view_camera(view: &ReadView) -> Camera {
    match view.name {
        "vein" => flythrough::VEIN_POSE.camera(),
        "patch" => flythrough::PATCH_POSE.camera(),
        "back" => support::settle::KRun::Path.camera(&flythrough::VEIN_POSE, 0),
        _ => {
            let shot = &flythrough::showcase::SHOTS[0];
            Camera {
                position: shot.eye.0.map(|v| v as f32 + 0.5).into(),
                yaw: (shot.yaw_deg as f32).to_radians(),
                pitch: (shot.pitch_deg as f32).to_radians(),
                vfov: 70f32.to_radians(),
            }
        }
    }
}

/// One timed frame: the passes' milliseconds by name.
fn timed_step(rig: &mut ReadRig, timer: &vxe_render::PassTimer, c: &Camera) -> Vec<f64> {
    rig.frame.advance();
    let params = rig.frame.params(&rig.gpu, c, &LightParams::default());
    let mut enc = rig
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    rig.frame.encode(
        &rig.device,
        &rig.queue,
        &mut enc,
        &rig.gpu,
        &params,
        Some(timer),
    );
    timer.resolve(&mut enc);
    rig.queue.submit([enc.finish()]);
    timer.read_all_ms(&rig.device)
}

fn pass_line(sum: &[f64], frames: usize) -> String {
    let total: f64 = sum.iter().sum::<f64>() / frames as f64;
    let mut parts: Vec<String> = vxe_render::FRAME_PASSES
        .iter()
        .zip(sum)
        .filter(|(_, ms)| **ms / frames as f64 >= 0.05)
        .map(|(n, ms)| format!("{n} {:.2}", ms / frames as f64))
        .collect();
    parts.insert(0, format!("total {total:.2} ms"));
    parts.join(", ")
}

// Plan 0059 Phase 4: the samples cut, the shared seeds and the filter, read against the
// reference's files.

/// Bakes the brick store within 12 chunks of a view's centre (`file_of`'s eye), the default
/// brick level (8/4096), 512 a face's round: the milliseconds it took.
fn brick_bake(rig: &mut ReadRig, view: &ReadView) -> f64 {
    let (_, centre_eye, _) = file_of(view);
    let centre = centre_eye.map(|v| (v / 64.0).floor() as i32);
    let settings = BakeSettings {
        round: 512,
        ..BakeSettings::default()
    };
    let world = rig.world.extent();
    let size = world.size();
    let lo = [0, 1, 2].map(|k| (centre[k] - 12).max(world.min.0[k]));
    let hi = [0, 1, 2].map(|k| (centre[k] + 13).min(world.min.0[k] + size[k]));
    let extent = ChunkExtent::new(lo, [0, 1, 2].map(|k| hi[k].max(lo[k])));
    let t = std::time::Instant::now();
    rig.frame
        .bake_until(
            &rig.device,
            &rig.queue,
            &mut rig.gpu,
            &rig.world,
            extent,
            &settings,
            &mut |_| {},
        )
        .expect("the extent fits the pool");
    t.elapsed().as_secs_f64() * 1e3
}

/// Frames of a yaw's read, as the settle rig's fast yaw.
const YAW_DOT_FRAMES: usize = 60;

/// The vein piece (chunks within 2 of the vein pose's) baked at the queue's settings on the
/// adapter `--adapter` names (`fallback` when unset); its blocks saved as `live/bake-count/adapter-<name>.bin`.
/// When the RTX's and the Cezanne's files are both there, every direction value either holds is
/// compared: the equal words' share, and the luminance's tone-mapped level difference, mean and
/// p99.
fn adapter_records(args: &Args) {
    let name = if args.adapter.is_empty() {
        "fallback"
    } else {
        args.adapter.as_str()
    };
    let mut log = Log::open("adapter.txt");
    let vein = flythrough::VEIN_POSE.position;
    let mut rig = read_rig(Scene::Level, vein, &args.adapter);
    let piece = piece_of(rig.world.extent(), vein, Some(2));
    let t = std::time::Instant::now();
    let r = rig
        .frame
        .bake_until(
            &rig.device,
            &rig.queue,
            &mut rig.gpu,
            &rig.world,
            piece,
            &vxe_render::queue_settings(),
            &mut |_| {},
        )
        .expect("the piece fits the pool");
    log.line(&format!(
    "adapter records, {}: the vein piece {:?}..{:?} at 8/4096, {} chunks, {} faces, {:.3e} samples in {:.0} ms",
    rig.adapter,
    piece.min.0,
    [0, 1, 2].map(|k| piece.min.0[k] + piece.size()[k]),
    r.chunks,
    r.faces,
    r.samples as f64,
    t.elapsed().as_secs_f64() * 1e3
));
    let blocks = frame_blocks(&rig.device, &rig.queue, &rig.frame, piece);
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../live/bake-count");
    let file = |n: &str| format!("{dir}/adapter-{}.bin", n.to_lowercase());
    let mut bytes = Vec::new();
    for (c, b) in &blocks {
        for v in c
            .iter()
            .map(|&x| x as u32)
            .chain([b.len() as u32])
            .chain(b.iter().copied())
        {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
    }
    std::fs::write(file(name), &bytes).expect("the records file");
    let read = |n: &str| -> Option<BTreeMap<[i32; 3], Vec<u32>>> {
        let raw = std::fs::read(file(n)).ok()?;
        let w: Vec<u32> = raw
            .chunks(4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        let mut out = BTreeMap::new();
        let mut i = 0;
        while i < w.len() {
            let c = [w[i] as i32, w[i + 1] as i32, w[i + 2] as i32];
            let n = w[i + 3] as usize;
            out.insert(c, w[i + 4..i + 4 + n].to_vec());
            i += 4 + n;
        }
        Some(out)
    };
    let (Some(a), Some(b)) = (read("rtx"), read("radeon")) else {
        log.line("adapter records: the other adapter's file is not there yet");
        return;
    };
    let (ca, cb) = (cells_of(&a), cells_of(&b));
    let level = |w: u32| {
        let v = vxe_render::internals::rgb9e5_unpack(w);
        vxe_render::internals::tone_level(
            0.2126 * f64::from(v[0]) + 0.7152 * f64::from(v[1]) + 0.0722 * f64::from(v[2]),
        )
    };
    let mut diffs = Vec::new();
    let (mut equal, mut only) = (0usize, 0usize);
    for (k, x) in &ca {
        let Some(y) = cb.get(k) else {
            only += 1;
            continue;
        };
        equal += usize::from(x[0] == y[0]);
        diffs.push((level(x[0]) - level(y[0])).abs());
    }
    only += cb.keys().filter(|k| !ca.contains_key(k)).count();
    diffs.sort_by(f64::total_cmp);
    let n = diffs.len().max(1);
    log.line(&format!(
    "adapter records, RTX against Cezanne: {} directions, {only} held by one alone; equal words {:.4}; |levels| mean {:.3}, p50 {:.3}, p99 {:.3}, max {:.3}",
    diffs.len(),
    equal as f64 / n as f64,
    diffs.iter().sum::<f64>() / n as f64,
    diffs[(n as f64 * 0.5) as usize % n],
    diffs[((n as f64 * 0.99) as usize).min(n - 1)],
    diffs.last().copied().unwrap_or(0.0)
));
}

/// The hall's pane with the lamp off: at the hall view (native, `--size`), a 40 x 40 x 2
/// box of the reference's glass row 6 voxels ahead along the horizontal axis the view faces most,
/// placed before the brick bake (`brick_bake`), then 64 still frames. Over the pixels with the
/// pane in front: the reflected term before exposure (the layer target's rgb), its luminance p50
/// and p90 and the pixels where it is 0; its display level alone at exposure 8 against the
/// pixel's shown luma, p50 and p90 of each and of their ratio (the reflected share). Logs to
/// `live/bake-count/glass.txt`.
fn glass_readings(args: &Args) {
    let mut log = Log::open("glass.txt");
    let size = args.size;
    let view = read_views(args.hall_r2)
        .into_iter()
        .find(|v| v.name == "hall")
        .expect("the hall view");
    let (mut rig, _) = view_rig(&view, size, &args.adapter);
    let glass = flythrough::showcase_materials().len() as vxe_world::MaterialId;
    let camera = view_camera(&view);
    let (fwd, _, _) = camera.basis();
    let a = if fwd.x.abs() >= fwd.y.abs() { 0 } else { 1 };
    let eye = camera.position.to_array().map(|v| v.floor() as i64);
    let ahead = if fwd.to_array()[a] > 0.0 { 6 } else { -7 };
    let mut min = eye.map(|v| v - 20);
    let mut max = eye.map(|v| v + 20);
    min[a] = eye[a] + ahead;
    max[a] = min[a] + 2;
    let r = rig
        .world
        .apply(&EditOp::box_(
            Fixed3(min.map(|v| v * FIXED_ONE)),
            Fixed3(max.map(|v| v * FIXED_ONE)),
            Mode::Fill(glass),
        ))
        .expect("the pane is resident");
    rig.gpu
        .sync_now(&rig.world, &rig.device, &rig.queue)
        .expect("the mirror fits");
    let placed: u32 = r.changed_from.iter().map(|(_, n)| n).sum();
    let bake_ms = brick_bake(&mut rig, &view);
    for _ in 0..64 {
        rig.frame.step(
            &rig.device,
            &rig.queue,
            &rig.gpu,
            &camera,
            &LightParams::default(),
        );
    }
    let lighting = rig.frame.lighting();
    let level = |c: [f32; 3]| {
        let b = tone_bytes([c[0], c[1], c[2], 1.0], &lighting);
        0.2126 * f32::from(b[0]) + 0.7152 * f32::from(b[1]) + 0.0722 * f32::from(b[2])
    };
    let info = vxe_render::debug::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
    let layer = vxe_render::debug::read_float(&rig.device, &rig.queue, rig.frame.layers());
    let shown = luma(&vxe_render::debug::read_rgba8(
        &rig.device,
        &rig.queue,
        rig.frame.target(),
    ));
    let lum = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let (mut refl, mut levels, mut shares, mut lumas) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut zero = 0usize;
    for i in 0..info.len() {
        if info[i][3] & 4 == 0 {
            continue;
        }
        let c = [layer[i][0], layer[i][1], layer[i][2]];
        if c == [0.0; 3] {
            zero += 1;
        }
        let l = level(c.map(|x| x * 8.0));
        refl.push(lum(c));
        levels.push(l);
        lumas.push(shown[i]);
        shares.push(if shown[i] > 0.0 { l / shown[i] } else { 0.0 });
    }
    let q = |v: &mut Vec<f32>, p: f32| {
        v.sort_by(f32::total_cmp);
        v.get(((v.len() as f32 - 1.0) * p) as usize)
            .copied()
            .unwrap_or(0.0)
    };
    log.line(&format!(
    "glass hall adapter {} {}x{} lamp off exposure 8: pane {min:?}..{max:?}, {placed} voxels; bake {bake_ms:.0} ms; {} pixels behind the pane, {zero} reflecting 0; reflected luminance before exposure p50 {:.5} p90 {:.5}; its level alone p50 {:.1} p90 {:.1}; shown luma p50 {:.1} p90 {:.1}; share (level / shown) p50 {:.3} p90 {:.3}",
    rig.adapter,
    size[0],
    size[1],
    refl.len(),
    q(&mut refl, 0.5),
    q(&mut refl, 0.9),
    q(&mut levels, 0.5),
    q(&mut levels, 0.9),
    q(&mut lumas, 0.5),
    q(&mut lumas, 0.9),
    q(&mut shares, 0.5),
    q(&mut shares, 0.9),
));
}

/// The live queue's dig against a fresh bake (Plan 0059 Phase 8e): at the far pose, a radius-1
/// piece baked at the queue's settings ([`vxe_render::queue_settings`], 8/4096), one 5-voxel carve at
/// the view's centre, its ring re-baked to each arm's settings in turn (the same box each time,
/// so each arm re-bakes the same bricks from the same edited world), then a fresh bake of the
/// edited world at the queue's settings. For each arm, every ring brick's six direction values
/// against the fresh bake's: equal words, within one `rgb9e5` step a channel, and display
/// levels; split into the bricks the box dilated by one voxel reaches ("touched") and the rest
/// ("untouched"). The pre-dig store against the fresh bake rides beside them, which is the edit's
/// own change. Logs to `live/bake-count/queue-dig.txt`.
fn queue_dig_readings(args: &Args) {
    let mut log = Log::open("queue-dig.txt");
    let pose = flythrough::FAR_POSE;
    let eye = pose.position;
    let size = [192, 108];
    let mut rig = read_rig(Scene::Level, eye, &args.adapter);
    rig.frame = Frame::new(&rig.device, size[0], size[1]);
    rig.frame.set_jitter(&rig.device, false);
    rig.frame
        .set_resolution(&rig.device, LightingResolution::Full);
    rig.frame.set_lighting(Lighting {
        lamp: false,
        lamp_bounce: false,
        exposure: Exposure::Manual(8.0),
        ..Lighting::default()
    });
    let world_extent = rig.world.extent();
    let centre = eye.map(|v| (v / 64.0).floor() as i32);
    let piece = ChunkExtent::new(centre.map(|v| v - 1), centre.map(|v| v + 2));
    let piece = {
        let s = world_extent.size();
        let lo = [0, 1, 2].map(|k| piece.min.0[k].max(world_extent.min.0[k]));
        let hi = [0, 1, 2].map(|k| (piece.min.0[k] + 3).min(world_extent.min.0[k] + s[k]));
        ChunkExtent::new(lo, hi)
    };
    let queue_settings = vxe_render::queue_settings();
    let t = std::time::Instant::now();
    let r = rig
        .frame
        .bake_until(
            &rig.device,
            &rig.queue,
            &mut rig.gpu,
            &rig.world,
            piece,
            &queue_settings,
            &mut |_| {},
        )
        .expect("the piece fits the pool");
    log.line(&format!(
    "queue dig, adapter {}, far pose chunk {centre:?}, piece {:?}..{:?}: the piece at 8/4096 in {:.0} ms, {} chunks, {} faces, {:.3e} samples",
    rig.adapter,
    piece.min.0,
    [0, 1, 2].map(|k| piece.min.0[k] + piece.size()[k]),
    t.elapsed().as_secs_f64() * 1e3,
    r.chunks,
    r.faces,
    r.samples as f64
));
    let before = frame_blocks(&rig.device, &rig.queue, &rig.frame, piece);
    let camera = pose.camera();
    rig.frame.step(
        &rig.device,
        &rig.queue,
        &rig.gpu,
        &camera,
        &LightParams::default(),
    );
    // The hit in the view's central half nearest 30 voxels whose carve (radius 5, plus the
    // ring's voxel) lies inside the piece: the view's centre can hit past the level's edge.
    let pos = vxe_render::debug::read_float(&rig.device, &rig.queue, rig.frame.hit_positions());
    let info = vxe_render::debug::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
    let dist = |p: &[f32; 3]| (0..3).map(|k| (p[k] - eye[k]).powi(2)).sum::<f32>().sqrt();
    let hit = (size[1] / 4..3 * size[1] / 4)
        .flat_map(|y| (size[0] / 4..3 * size[0] / 4).map(move |x| (y * size[0] + x) as usize))
        .filter(|&i| info[i][3] & 1 != 0)
        .map(|i| [pos[i][0], pos[i][1], pos[i][2]])
        .filter(|p| {
            let v = p.map(|v| v.floor() as i32);
            inside(piece, v.map(|c| c - 6)) && inside(piece, v.map(|c| c + 6))
        })
        .min_by(|a, b| (dist(a) - 30.0).abs().total_cmp(&(dist(b) - 30.0).abs()))
        .expect("a hit inside the piece in the view's central half");
    let c = hit.map(|v| i64::from(v.floor() as i32) * FIXED_ONE + FIXED_ONE / 2);
    let op = EditOp::sphere(Fixed3(c), 5 * FIXED_ONE, Mode::Carve);
    let applied = rig.world.apply(&op).expect("the carve applies");
    let (min, max) = applied.aabb.expect("the carve changed voxels");
    let dug = [min.0, max.0];
    rig.gpu
        .sync_now(&rig.world, &rig.device, &rig.queue)
        .expect("the mirror fits");
    let ring = vxe_render::internals::dig_ring(&[dug], piece);
    log.line(&format!(
    "queue dig: carve at {:?} ({:.1} voxels from the eye), box {:?}..{:?}, ring {} bricks in {} chunks",
    hit.map(|v| v.floor() as i32),
    dist(&hit),
    dug[0],
    dug[1],
    ring.values().map(Vec::len).sum::<usize>(),
    ring.len()
));
    let arms: Vec<(&str, BakeSettings)> = vec![
        ("the queue's settings (round 64)", queue_settings.clone()),
        (
            "the queue's settings, round BAKE_ROUND",
            BakeSettings {
                round: vxe_render::internals::BAKE_ROUND,
                ..queue_settings.clone()
            },
        ),
    ];
    let mut after = Vec::new();
    for (name, settings) in &arms {
        rig.frame.invalidate(dug[0], dug[1]);
        let t = std::time::Instant::now();
        let r = rig
            .frame
            .rebake_digs(
                &rig.device,
                &rig.queue,
                &mut rig.gpu,
                &rig.world,
                settings,
                &mut |_| {},
            )
            .expect("a box waits")
            .expect("the ring fits");
        let ms = t.elapsed().as_secs_f64() * 1e3;
        log.line(&format!(
            "queue dig, {name}: {} bricks, {} chunks, {} faces, {:.3e} samples, {ms:.0} ms",
            r.bricks, r.bake.chunks, r.bake.faces, r.bake.samples as f64
        ));
        after.push(frame_blocks(&rig.device, &rig.queue, &rig.frame, piece));
    }
    let fresh = fresh_piece(&rig, eye, piece, &queue_settings);
    compare_ring(&mut log, "before the dig", &before, &fresh, &ring, dug);
    for ((name, _), got) in arms.iter().zip(&after) {
        compare_ring(&mut log, name, got, &fresh, &ring, dug);
    }
}

/// A fresh mirror and frame of `rig`'s world as it is now, focused at `eye`, `piece` baked at
/// `settings`: its blocks.
fn fresh_piece(
    rig: &ReadRig,
    eye: [f32; 3],
    piece: ChunkExtent,
    settings: &BakeSettings,
) -> BTreeMap<[i32; 3], Vec<u32>> {
    let cfg = MirrorConfig {
        budget_bytes: 512 << 20,
        light_radius: 4,
        ..MirrorConfig::new(Window::Fixed(rig.world.extent()))
    };
    let mut gpu = GpuWorld::with_config(&rig.device, cfg);
    gpu.set_focus(vxe_world::VoxelCoord(eye.map(|v| v as i32)));
    for _ in 0..8 {
        gpu.sync_now(&rig.world, &rig.device, &rig.queue)
            .expect("the mirror fits");
    }
    let mut fresh_frame = Frame::new(&rig.device, 16, 16);
    fresh_frame.set_lighting(Lighting {
        lamp: false,
        lamp_bounce: false,
        exposure: Exposure::Manual(8.0),
        ..Lighting::default()
    });
    fresh_frame
        .bake_until(
            &rig.device,
            &rig.queue,
            &mut gpu,
            &rig.world,
            piece,
            settings,
            &mut |_| {},
        )
        .expect("the piece fits the pool");
    frame_blocks(&rig.device, &rig.queue, &fresh_frame, piece)
}

/// The ring's direction values in `got` against `fresh`, split into the bricks the dug box
/// `dug` dilated by one voxel reaches ("touched") and the rest ("untouched"): equal words,
/// within one `rgb9e5` step a channel, and display levels, one log line a part.
fn compare_ring(
    log: &mut Log,
    name: &str,
    got: &BTreeMap<[i32; 3], Vec<u32>>,
    fresh: &BTreeMap<[i32; 3], Vec<u32>>,
    ring: &BTreeMap<[i32; 3], Vec<u16>>,
    dug: [[i32; 3]; 2],
) {
    let touched = |c: [i32; 3], slot: u16| {
        let b = [
            i32::from(slot) % 8,
            i32::from(slot) / 8 % 8,
            i32::from(slot) / 64,
        ];
        (0..3).all(|k| {
            let lo = c[k] * 64 + b[k] * 8;
            lo + 7 >= dug[0][k] - 1 && lo <= dug[1][k] + 1
        })
    };
    for part in ["touched", "untouched"] {
        let mut diffs = Vec::new();
        let (mut equal, mut one_step, mut dir_differ) = (0usize, 0usize, 0usize);
        for (c, slots) in ring {
            let (Some(a), Some(f)) = (got.get(c), fresh.get(c)) else {
                continue;
            };
            for &slot in slots {
                if touched(*c, slot) != (part == "touched") {
                    continue;
                }
                let (da, df) = (a[usize::from(slot)], f[usize::from(slot)]);
                if da == 0 || df == 0 {
                    continue;
                }
                if (da & 0x7ff) != (df & 0x7ff) {
                    dir_differ += 1;
                    continue;
                }
                let (ra, rf) = ((da >> 11) as usize, (df >> 11) as usize);
                for d in 0..6 {
                    let (wa, wf) = (
                        a[ra - vxe_render::internals::BRICK_WORDS + d],
                        f[rf - vxe_render::internals::BRICK_WORDS + d],
                    );
                    if wa == wf {
                        equal += 1;
                    }
                    let (va, vf) = (
                        vxe_render::internals::rgb9e5_unpack(wa),
                        vxe_render::internals::rgb9e5_unpack(wf),
                    );
                    let step = |w: u32| 2f32.powi((w >> 27) as i32 - 24);
                    let ulp = step(wa).max(step(wf));
                    if (0..3).all(|k| (va[k] - vf[k]).abs() <= ulp * 1.0001) {
                        one_step += 1;
                    }
                    let l = |v: [f32; 3]| {
                        vxe_render::internals::tone_level(
                            0.2126 * f64::from(v[0])
                                + 0.7152 * f64::from(v[1])
                                + 0.0722 * f64::from(v[2]),
                        )
                    };
                    diffs.push((l(va) - l(vf)).abs());
                }
            }
        }
        diffs.sort_by(f64::total_cmp);
        let n = diffs.len();
        if n == 0 {
            log.line(&format!("queue dig, {name}, {part}: nothing compared"));
            continue;
        }
        let at = |q: f64| diffs[((n as f64 * q) as usize).min(n - 1)];
        log.line(&format!(
        "queue dig, {name}, {part} bricks: {n} direction values ({dir_differ} bricks whose cell counts differ, skipped); equal words {:.4}, within one rgb9e5 step {:.4}, within 1 level {:.4}; |levels| p50 {:.3} p90 {:.3} p99 {:.3} max {:.3}",
        equal as f64 / n as f64,
        one_step as f64 / n as f64,
        diffs.iter().filter(|d| **d <= 1.0).count() as f64 / n as f64,
        at(0.5),
        at(0.9),
        at(0.99),
        diffs[n - 1]
    ));
    }
}

/// The resolution cut under the brick store (Plan 0059 Phase 10 as re-read at Phase 7b): at each
/// view of `--views` (default vein and hall), the view's brick store baked (`brick_bake`), and
/// three settings at `NATIVE`: native with no upscaler (the reference), FSR 2 at 1.5 with far
/// normal 1..2, and FSR 2 at 1.5 with far normal 2..6 (the fly-through's default image). Per
/// setting, after 128 still frames: the share of the pixels within 1, 2 and 4 display levels of
/// the reference's, the passes' milliseconds over 120 timed frames at rest, and the fast yaw's
/// excess dots (64 frames at -90 degrees, ten 9-degree steps back, 60 frames
/// counted against the still floor). Logs to `live/bake-count/resolution.txt`.
fn resolution_readings(args: &Args) {
    let mut log = Log::open("resolution.txt");
    let names = &args.views;
    for view in read_views(args.hall_r2) {
        if !names.split(',').any(|n| n == view.name) {
            continue;
        }
        let (mut rig, _) = view_rig(&view, NATIVE, &args.adapter);
        let load_ms = brick_bake(&mut rig, &view);
        let timer = vxe_render::PassTimer::with_passes(
            &rig.device,
            &rig.queue,
            vxe_render::FRAME_PASSES.len() as u32,
        )
        .expect("timestamps on the RTX");
        let still = view_camera(&view);
        let yawed = |deg: f32| Camera {
            yaw: still.yaw + deg.to_radians(),
            ..still
        };
        let light = LightParams::default();
        let fsr = || {
            vxe_render::UpscalerKind::Fsr2(vxe_render::Fsr2Settings {
                sharpness: 0.5,
                ..vxe_render::Fsr2Settings::default()
            })
        };
        let settings: [(&str, Option<vxe_render::UpscalerKind>, [f32; 2]); 3] = [
            ("native, no upscaler, far normal 1..2", None, [1.0, 2.0]),
            ("fsr2 at 1.5, far normal 1..2", Some(fsr()), [1.0, 2.0]),
            ("fsr2 at 1.5, far normal 2..6", Some(fsr()), [2.0, 6.0]),
        ];
        let mut reference: Option<Vec<f32>> = None;
        for (setting, kind, far) in settings {
            match kind {
                None => {
                    rig.frame
                        .set_upscaler_kind(&rig.device, vxe_render::UpscalerKind::Off);
                    rig.frame.set_jitter(&rig.device, false);
                    rig.frame.set_sizes(&rig.device, NATIVE, NATIVE);
                }
                Some(k) => {
                    rig.frame.set_upscaler_kind(&rig.device, k);
                    rig.frame.set_sizes(
                        &rig.device,
                        vxe_render::RenderScale::OneHalf.internal(NATIVE),
                        NATIVE,
                    );
                }
            }
            rig.frame.set_look(vxe_render::LookParams {
                far_normal: far,
                ..rig.frame.look()
            });
            rig.frame.reset_history();
            for _ in 0..128 {
                rig.frame
                    .step(&rig.device, &rig.queue, &rig.gpu, &still, &light);
            }
            let img = luma(&vxe_render::debug::read_rgba8(
                &rig.device,
                &rig.queue,
                rig.frame.target(),
            ));
            let shares = match &reference {
                None => {
                    reference = Some(img.clone());
                    "the reference".to_owned()
                }
                Some(r) => {
                    let n = r.len() as f64;
                    let within = |lv: f32| {
                        r.iter()
                            .zip(&img)
                            .filter(|(a, b)| (*a - *b).abs() <= lv)
                            .count() as f64
                            / n
                    };
                    format!(
                        "within 1/2/4 levels of the reference {:.4} / {:.4} / {:.4}",
                        within(1.0),
                        within(2.0),
                        within(4.0)
                    )
                }
            };
            let mut sum = vec![0.0; vxe_render::FRAME_PASSES.len()];
            for _ in 0..120 {
                for (s, ms) in sum.iter_mut().zip(timed_step(&mut rig, &timer, &still)) {
                    *s += ms;
                }
            }
            let passes = pass_line(&sum, 120);
            let dots = |rig: &ReadRig| -> f64 {
                let px = vxe_render::debug::read_rgba8(&rig.device, &rig.queue, rig.frame.target());
                support::aliasing::dots(&px, NATIVE[0], NATIVE[1])
                    .iter()
                    .filter(|d| **d)
                    .count() as f64
            };
            let mut floor = Vec::new();
            for _ in 0..64 {
                rig.frame
                    .step(&rig.device, &rig.queue, &rig.gpu, &still, &light);
                floor.push(dots(&rig));
            }
            for _ in 0..64 {
                rig.frame
                    .step(&rig.device, &rig.queue, &rig.gpu, &yawed(-90.0), &light);
            }
            for i in 1..=10 {
                rig.frame.step(
                    &rig.device,
                    &rig.queue,
                    &rig.gpu,
                    &yawed(-90.0 + 9.0 * i as f32),
                    &light,
                );
            }
            let mean = floor.iter().sum::<f64>() / floor.len() as f64;
            let sd =
                (floor.iter().map(|f| (f - mean).powi(2)).sum::<f64>() / floor.len() as f64).sqrt();
            let mut excess = 0.0;
            for _ in 0..YAW_DOT_FRAMES {
                rig.frame
                    .step(&rig.device, &rig.queue, &rig.gpu, &still, &light);
                excess += dots(&rig) - mean;
            }
            log.line(&format!(
            "resolution {} adapter {} {}x{} (bricks 8/4096 loaded in {load_ms:.0} ms), {setting}: {shares}; at rest {passes}; fast yaw excess dots 1-60 {excess:+.1} (floor {mean:.1}, sd {sd:.1}, line {:.1})",
            view.name,
            rig.adapter,
            NATIVE[0],
            NATIVE[1],
            YAW_DOT_FRAMES as f64 * sd
        ));
        }
    }
}

/// The light's budget at rest (Plan 0059 Phase 12): at each view of `--views` (default
/// vein and hall) at `--size` (default `NATIVE`), the brick store (`brick_bake`), native
/// with no upscaler and FSR 2 at 1.5, far normal 1..2, lamp off. Per arm, after 1024 still frames
/// from no history: the passes over 120 timed frames, the lighting line (gradient, light,
/// temporal, probes, gather), and the display's change from frame 1024 to 1025 (the one frame)
/// and from 512 to 1024 (the drift): the mean of the channels' level differences, the largest,
/// and whether the two frames are bit-identical. Logs to `live/bake-count/rest.txt`.
fn rest_readings(args: &Args) {
    let mut log = Log::open("rest.txt");
    let names = &args.views;
    let size = args.size;
    const LINE: [&str; 5] = ["gradient", "light", "temporal", "probes", "gather"];
    for view in read_views(args.hall_r2) {
        if !names.split(',').any(|n| n == view.name) {
            continue;
        }
        let (mut rig, _) = view_rig(&view, size, &args.adapter);
        brick_bake(&mut rig, &view);
        let timer = vxe_render::PassTimer::with_passes(
            &rig.device,
            &rig.queue,
            vxe_render::FRAME_PASSES.len() as u32,
        )
        .expect("timestamps on the adapter");
        let still = view_camera(&view);
        let light = LightParams::default();
        let light_name = "baked";
        {
            for fsr in [false, true] {
                rig.frame.set_lighting(reference_lighting(false));
                if fsr {
                    rig.frame.set_upscaler_kind(
                        &rig.device,
                        vxe_render::UpscalerKind::Fsr2(vxe_render::Fsr2Settings {
                            sharpness: 0.5,
                            ..vxe_render::Fsr2Settings::default()
                        }),
                    );
                    rig.frame.set_sizes(
                        &rig.device,
                        vxe_render::RenderScale::OneHalf.internal(size),
                        size,
                    );
                } else {
                    rig.frame
                        .set_upscaler_kind(&rig.device, vxe_render::UpscalerKind::Off);
                    rig.frame.set_jitter(&rig.device, false);
                    rig.frame.set_sizes(&rig.device, size, size);
                }
                rig.frame.set_look(vxe_render::LookParams {
                    far_normal: [1.0, 2.0],
                    ..rig.frame.look()
                });
                rig.frame.reset_history();
                let read = |rig: &ReadRig| {
                    vxe_render::debug::read_rgba8(&rig.device, &rig.queue, rig.frame.target())
                };
                let mut at512 = Vec::new();
                for f in 1..=1024 {
                    rig.frame
                        .step(&rig.device, &rig.queue, &rig.gpu, &still, &light);
                    if f == 512 {
                        at512 = read(&rig);
                    }
                }
                let at1024 = read(&rig);
                rig.frame
                    .step(&rig.device, &rig.queue, &rig.gpu, &still, &light);
                let at1025 = read(&rig);
                let change = |a: &[u8], b: &[u8]| {
                    let mut sum = 0u64;
                    let mut max = 0u8;
                    for (p, q) in a.chunks(4).zip(b.chunks(4)) {
                        for k in 0..3 {
                            let d = p[k].abs_diff(q[k]);
                            sum += u64::from(d);
                            max = max.max(d);
                        }
                    }
                    format!(
                        "mean {:.4} max {max}{}",
                        sum as f64 / (a.len() / 4 * 3) as f64,
                        if a == b { ", bit-identical" } else { "" }
                    )
                };
                let one = change(&at1024, &at1025);
                let drift = change(&at512, &at1024);
                let mut sum = vec![0.0; vxe_render::FRAME_PASSES.len()];
                for _ in 0..120 {
                    for (s, ms) in sum.iter_mut().zip(timed_step(&mut rig, &timer, &still)) {
                        *s += ms;
                    }
                }
                let of = |n: &str| {
                    vxe_render::FRAME_PASSES
                        .iter()
                        .position(|p| *p == n)
                        .map_or(0.0, |i| sum[i] / 120.0)
                };
                let line: f64 = LINE.iter().map(|n| of(n)).sum();
                log.line(&format!(
                "rest {} adapter {} {}x{} {light_name} {}: lighting line {line:.2} ms (temporal {:.2}); one frame 1024/1025 {one}; drift 512/1024 {drift}; {}",
                view.name,
                rig.adapter,
                size[0],
                size[1],
                if fsr { "fsr2 at 1.5" } else { "native" },
                of("temporal"),
                pass_line(&sum, 120)
            ));
            }
        }
    }
}

/// The per-pass table (Plan 0065 Phase 1): at each view of `--views` (default vein and
/// hall), the brick store (`brick_bake`), FSR 2 at 1.5 to `NATIVE` (internal 1687 x 919),
/// sharpness 0.5, the far normal at `--far-normal` (`lo,hi`, default the fly-through's
/// `2,6`). Per arm (lighting tier half and full, lamp off and on, at rest and walking), 128
/// still frames from no history, then 120 timed frames: still, or walking (the eye moves at 14
/// voxels a second, 60 frames out and 60 back, its view held: at the vein back along the
/// steady flight that ends there, at the hall along the level heading). Each pass's
/// mean milliseconds, every pass printed to 3 decimals. Logs to `live/bake-count/passes.txt`.
fn pass_readings(args: &Args) {
    let mut log = Log::open("passes.txt");
    let names = &args.views;
    let far = args.far_normal;
    const FRAMES: usize = 120;
    for view in read_views(args.hall_r2) {
        if !names.split(',').any(|n| n == view.name) {
            continue;
        }
        let (mut rig, _) = view_rig(&view, NATIVE, &args.adapter);
        brick_bake(&mut rig, &view);
        let timer = vxe_render::PassTimer::with_passes(
            &rig.device,
            &rig.queue,
            vxe_render::FRAME_PASSES.len() as u32,
        )
        .expect("timestamps on the adapter");
        let internal = vxe_render::RenderScale::OneHalf.internal(NATIVE);
        rig.frame.set_upscaler_kind(
            &rig.device,
            vxe_render::UpscalerKind::Fsr2(vxe_render::Fsr2Settings {
                sharpness: 0.5,
                ..vxe_render::Fsr2Settings::default()
            }),
        );
        rig.frame.set_sizes(&rig.device, internal, NATIVE);
        rig.frame.set_look(vxe_render::LookParams {
            far_normal: far,
            ..rig.frame.look()
        });
        let still = view_camera(&view);
        // The walk's direction: at the vein back along the steady flight that ends at its
        // pose (a cube of 5 voxels of air around every step), elsewhere the view's level
        // heading.
        let ahead = if view.name == "vein" {
            let (start, end) = support::settle::flight_path(&flythrough::VEIN_POSE);
            (vxe_render::glam::Vec3::from(start) - vxe_render::glam::Vec3::from(end)).normalize()
        } else {
            vxe_render::glam::Vec3::new(still.yaw.cos(), still.yaw.sin(), 0.0)
        };
        let walk = |f: usize| {
            let t = (f % FRAMES) as f32;
            let out = if t < 60.0 { t } else { FRAMES as f32 - t };
            Camera {
                position: still.position + ahead * (14.0 / 60.0 * out),
                ..still
            }
        };
        let light = LightParams::default();
        for tier in [LightingResolution::Half, LightingResolution::Full] {
            rig.frame.set_resolution(&rig.device, tier);
            for lamp in [false, true] {
                rig.frame.set_lighting(reference_lighting(lamp));
                for walking in [false, true] {
                    rig.frame.reset_history();
                    for _ in 0..128 {
                        rig.frame
                            .step(&rig.device, &rig.queue, &rig.gpu, &still, &light);
                    }
                    let mut sum = vec![0.0; vxe_render::FRAME_PASSES.len()];
                    for f in 0..FRAMES {
                        let c = if walking { walk(f) } else { still };
                        for (s, ms) in sum.iter_mut().zip(timed_step(&mut rig, &timer, &c)) {
                            *s += ms;
                        }
                    }
                    let total: f64 = sum.iter().sum::<f64>() / FRAMES as f64;
                    let passes: Vec<String> = vxe_render::FRAME_PASSES
                        .iter()
                        .zip(&sum)
                        .filter(|(_, ms)| **ms > 0.0)
                        .map(|(n, ms)| format!("{n} {:.3}", ms / FRAMES as f64))
                        .collect();
                    log.line(&format!(
                        "passes {} adapter {} {}x{} to {}x{} fsr2 at 1.5, far normal {}..{}, tier {}, lamp {}, {}: total {total:.3} ms, {}",
                        view.name,
                        rig.adapter,
                        internal[0],
                        internal[1],
                        NATIVE[0],
                        NATIVE[1],
                        far[0],
                        far[1],
                        tier.name(),
                        if lamp { "on" } else { "off" },
                        if walking { "walking" } else { "at rest" },
                        passes.join(", ")
                    ));
                }
            }
        }
    }
}

/// Plan 0061 Phase 2's claims on the staircase, as a reading (the phase is exploration):
/// done-when 1 at 192 x 108 (`t >= 140`) and 1b at 96 x 54 (`t >= 72`), far normal `[1, 2]`:
/// with the read on every rock riser or tread pixel past the line reads ADR-0049's mix within
/// 1 % (the CPU's `by_normal` over the bake's records), with it off its face's value within
/// 1 % and both directions occur; done-when 2: `t <= 64` reads the same with the read on and
/// off under `face`, the frame is bit-identical with the far normal off, and under
/// `gradient27` and `trilinear8` every stored part is finite; done-when 2b: with the far
/// normal off the face weights leave the frame bit-identical at half and full resolution.
/// Counts and failures go to `live/bake-count/staircase.txt`.
fn staircase_read(args: &Args) {
    let mut log = Log::open("staircase.txt");
    log.line(&format!(
        "staircase reading, {}",
        if args.adapter.is_empty() {
            "lavapipe"
        } else {
            args.adapter.as_str()
        }
    ));
    for (size, line) in [([192u32, 108u32], 140f32), ([96, 54], 72.0)] {
        let world = staircase();
        let [camera, near_camera] = stair_cameras(size);
        let Some(mut rig) = rig_on(world, size, None) else {
            return;
        };
        rig.frame.set_lighting(Lighting {
            lamp: false,
            lamp_bounce: false,
            exposure: Exposure::Manual(8.0),
            bake_pool_bytes: 64 << 20,
            ..Lighting::default()
        });
        let t0 = std::time::Instant::now();
        let report = rig.bake(&test_counts(256));
        log.line(&format!(
            "{}x{}: baked {} faces in {:.1} s",
            size[0],
            size[1],
            report.faces,
            t0.elapsed().as_secs_f64()
        ));
        stair_mix(&mut log, &mut rig, &camera, size, line);
        stair_near(&mut log, &mut rig, [&camera, &near_camera], size);
    }
}

/// `arm` of the staircase reading: (store read, far normal, look, weights, resolution).
fn arm(
    read: vxe_render::StoreRead,
    far_normal: [f32; 2],
    mode: vxe_render::NormalMode,
    weights: vxe_render::LightWeights,
    resolution: LightingResolution,
) -> StairArm {
    StairArm {
        read,
        weights,
        far_normal,
        mode,
        resolution,
    }
}

/// Done-when 1 / 1b of the staircase reading at `size`: the far `camera`'s rock risers and
/// treads past `line`, read on against ADR-0049's mix and read off against their face's value.
fn stair_mix(log: &mut Log, rig: &mut Rig, camera: &Camera, size: [u32; 2], line: f32) {
    use vxe_render::{LightWeights, NormalMode, StoreRead};
    let cells = rig.cells();
    let eye = camera.position.to_array();
    let full = LightingResolution::Full;
    let off = rig_frame(
        rig,
        camera,
        arm(
            StoreRead::Face,
            [1.0, 2.0],
            NormalMode::Face,
            LightWeights::Normal,
            full,
        ),
    );
    let on = rig_frame(
        rig,
        camera,
        arm(
            StoreRead::Normal,
            [1.0, 2.0],
            NormalMode::Face,
            LightWeights::Normal,
            full,
        ),
    );
    let (mut checked, mut bad_on, mut bad_off) = (0, 0, 0);
    let (mut risers, mut treads) = (0, 0);
    let (mut vx, mut vz) = (0f64, 0f64);
    let mut worst = 0f32;
    let mut faces = [0u32; 6];
    let mut moved = 0;
    let mut n_sum = [0f64; 3];
    for i in 0..on.pos.len() {
        if let Some((_, f)) = hit_face(eye, on.pos[i], on.info[i])
            && on.pos[i][3] >= line
        {
            faces[usize::from(f)] += 1;
        }
    }
    log.line(&format!(
        "{}x{}: hit faces past the line by direction {faces:?}",
        size[0], size[1]
    ));
    for i in 0..on.pos.len() {
        let Some((v, face)) = hit_face(eye, on.pos[i], on.info[i]) else {
            continue;
        };
        if on.info[i][2] != ROCK || on.pos[i][3] < line || !(face == 0 || face == 5) {
            continue;
        }
        let n = support::rigs::octa16_normal(on.info[i][0], on.info[i][1]);
        let Some(want) = by_normal(&cells, v, face, n) else {
            continue;
        };
        checked += 1;
        let got = rgb(on.accum[i]);
        let e = (0..3)
            .map(|k| (got[k] - want[k]).abs() / want[k].abs().max(1e-3))
            .fold(0f32, f32::max);
        worst = worst.max(e);
        if !within(got, want, 0.01) {
            bad_on += 1;
        }
        let own = brick_value(&cells, v, face).unwrap_or([0.0; 3]);
        if !within(rgb(off.accum[i]), own, 0.01) {
            bad_off += 1;
        }
        if !within(got, own, 0.01) {
            moved += 1;
        }
        n_sum = [0, 1, 2].map(|k| n_sum[k] + f64::from(n[k]));
        let l = f64::from(own[0] + own[1] + own[2]) / 3.0;
        if face == 0 {
            risers += 1;
            vx += l;
        } else {
            treads += 1;
            vz += l;
        }
    }
    log.line(&format!(
        "{}x{}: done-when 1: {checked} rock pixels at t >= {line}: read on, {bad_on} off the mix by over 1 % (worst {:.4}); read off, {bad_off} off their face's value; {risers} risers mean {:.4}, {treads} treads mean {:.4} (ratio {:.2})",
        size[0],
        size[1],
        worst,
        vx / f64::from(risers.max(1)),
        vz / f64::from(treads.max(1)),
        (vz / f64::from(treads.max(1))) / (vx / f64::from(risers.max(1))).max(1e-9),
    ));
    log.line(&format!(
        "{}x{}: the read moves {moved} of {checked} past the line by over 1 % from their face's value; mean hit normal [{:.3}, {:.3}, {:.3}]",
        size[0],
        size[1],
        n_sum[0] / f64::from(checked.max(1)),
        n_sum[1] / f64::from(checked.max(1)),
        n_sum[2] / f64::from(checked.max(1)),
    ));
}

/// Done-when 2 and 2b of the staircase reading at `size`: the near camera's pixels at
/// `t <= 64` with the read on and off, the far camera with the far normal off, the smooth
/// looks' stored parts, and the face weights against the normal's.
fn stair_near(log: &mut Log, rig: &mut Rig, [camera, near_camera]: [&Camera; 2], size: [u32; 2]) {
    use vxe_render::{LightWeights, NormalMode, StoreRead};
    let full = LightingResolution::Full;
    let off = rig_frame(
        rig,
        near_camera,
        arm(
            StoreRead::Face,
            [1.0, 2.0],
            NormalMode::Face,
            LightWeights::Normal,
            full,
        ),
    );
    let on = rig_frame(
        rig,
        near_camera,
        arm(
            StoreRead::Normal,
            [1.0, 2.0],
            NormalMode::Face,
            LightWeights::Normal,
            full,
        ),
    );
    let near_diff = (0..on.pos.len())
        .filter(|&i| on.info[i][3] & 1 != 0 && on.pos[i][3] <= 64.0 && on.accum[i] != off.accum[i])
        .count();
    let near = (0..on.pos.len())
        .filter(|&i| on.info[i][3] & 1 != 0 && on.pos[i][3] <= 64.0)
        .count();
    let none_off = rig_frame(
        rig,
        camera,
        arm(
            StoreRead::Face,
            [0.0, 0.0],
            NormalMode::Face,
            LightWeights::Normal,
            full,
        ),
    );
    let none_on = rig_frame(
        rig,
        camera,
        arm(
            StoreRead::Normal,
            [0.0, 0.0],
            NormalMode::Face,
            LightWeights::Normal,
            full,
        ),
    );
    log.line(&format!(
        "{}x{}: done-when 2: {near_diff} of {near} hit pixels at t <= 64 differ read on against off; far normal off, display differs in {} bytes, accumulation in {} pixels",
        size[0],
        size[1],
        none_on.display.iter().zip(&none_off.display).filter(|(a, b)| a != b).count(),
        none_on.accum.iter().zip(&none_off.accum).filter(|(a, b)| a != b).count(),
    ));
    for mode in [NormalMode::Gradient27, NormalMode::Trilinear8] {
        let f = rig_frame(
            rig,
            camera,
            arm(
                StoreRead::Normal,
                [1.0, 2.0],
                mode,
                LightWeights::Normal,
                full,
            ),
        );
        let nonfinite = f
            .accum
            .iter()
            .filter(|p| p.iter().any(|x| !x.is_finite()))
            .count();
        let face_off = rig_frame(
            rig,
            camera,
            arm(
                StoreRead::Face,
                [1.0, 2.0],
                mode,
                LightWeights::Normal,
                full,
            ),
        );
        let moved = f
            .accum
            .iter()
            .zip(&face_off.accum)
            .filter(|(a, b)| a != b)
            .count();
        log.line(&format!(
            "{}x{}: done-when 2, {mode:?}: {nonfinite} non-finite stored parts, read on moves {moved} pixels",
            size[0], size[1]
        ));
    }
    // Done-when 2b.
    for resolution in [LightingResolution::Half, LightingResolution::Full] {
        let a = rig_frame(
            rig,
            camera,
            arm(
                StoreRead::Face,
                [0.0, 0.0],
                NormalMode::Face,
                LightWeights::Normal,
                resolution,
            ),
        );
        let b = rig_frame(
            rig,
            camera,
            arm(
                StoreRead::Face,
                [0.0, 0.0],
                NormalMode::Face,
                LightWeights::Face,
                resolution,
            ),
        );
        log.line(&format!(
            "{}x{}: done-when 2b, {resolution:?}: face weights against normal, far normal off: display differs in {} bytes",
            size[0],
            size[1],
            a.display.iter().zip(&b.display).filter(|(x, y)| x != y).count()
        ));
    }
}

fn rig_frame(rig: &mut Rig, camera: &Camera, arm: StairArm) -> StairFrame {
    stair_frame(rig, camera, &arm)
}
