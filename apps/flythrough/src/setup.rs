//! What the fly-through builds before its first frame: the material table, the start
//! camera, the shell's configuration and the GPU objects of a run ([`Gfx`]).

use crate::LogicChoice;
use crate::app::{Flythrough, Pending, ms_since};
use crate::audio::{Audio, METRES_PER_VOXEL, RoomProbe};
use crate::castle;
use crate::cave::{CaveSource, LEVEL_CHUNKS, RoomSource, VAULT_EDGE};
use crate::cli::Args;
use crate::edits::Failure;
use crate::hud::{self};
use crate::now;
use crate::outdoor::OutdoorSource;
use crate::pad::Pad;
use crate::run::hud_lines::look_line;
use crate::showcase::{SHOWCASE_CHUNKS, Shot, ShowcaseSource};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use svet_app::{Gpu, ShellConfig};
use svet_net::Logic;
use svet_render::{Camera, Frame, GpuWorld, HudOverlay, MarchTimer, PassTimer, Presenter, wgpu};
use svet_world::{
    ChunkExtent, MaterialTable, PersistConfig, StreamConfig, Streamer, VoxelCoord, World,
};

pub(crate) struct Gfx {
    pub(crate) mirror: GpuWorld,
    pub(crate) frame: Frame,
    pub(crate) presenter: Presenter,
    /// Times the frame's passes, `FRAME_PASSES` in order.
    pub(crate) timer: Option<PassTimer>,
    /// Times the field pass, with the field's band slots and `TIMESTAMP_QUERY`.
    pub(crate) field_timer: Option<MarchTimer>,
    pub(crate) hud: HudOverlay,
    /// Times the HUD pass, with `TIMESTAMP_QUERY`.
    pub(crate) hud_timer: Option<PassTimer>,
}

/// The GPU side's values the HUD's rows read: the mirror's sizes, slots and pressure, its light
/// samples, and the frame's sizes while the upscaler runs or in a window.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GfxSample {
    pub(crate) mirror_bytes: u64,
    pub(crate) pool_live: u32,
    pub(crate) pool_free: u32,
    pub(crate) pressure: f32,
    pub(crate) budget_bytes: u64,
    pub(crate) light_samples: u64,
    pub(crate) sizes: Option<crate::hud::FrameSizes>,
}

impl GfxSample {
    /// `window` is the window's size, none headless.
    pub(crate) fn of(g: &Gfx, window: Option<[u32; 2]>) -> Self {
        let upscaling = g.frame.upscaling();
        Self {
            mirror_bytes: g.mirror.mirror_bytes(),
            pool_live: g.mirror.live_bricks(),
            pool_free: g.mirror.spare_slots().min(u64::from(u32::MAX)) as u32,
            pressure: g.mirror.pressure(),
            budget_bytes: g.mirror.allocated_bytes(),
            light_samples: g.mirror.light_samples(),
            sizes: (upscaling || window.is_some()).then(|| crate::hud::FrameSizes {
                internal: g.frame.internal_size(),
                output: g.frame.output_size(),
                window,
                upscaling,
            }),
        }
    }
}

impl Gfx {
    /// Reads that waited on the GPU so far: the three pass timers' and the store's counters'
    /// (the field pass's own timing at the initial fill and the bench is among them).
    pub(crate) fn blocking_reads(&self) -> u64 {
        [&self.timer, &self.field_timer, &self.hud_timer]
            .into_iter()
            .flatten()
            .map(PassTimer::blocking_reads)
            .sum::<u64>()
            + self.frame.store_blocking_reads()
    }

    /// Runs the field pass over the band slots placed since the last, alone, and waits for
    /// it: the band slots it filled and its GPU time when timed.
    pub(crate) fn timed_field(&mut self, gpu: &Gpu) -> (usize, Option<f64>) {
        let mut enc = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let timer = self.field_timer.as_ref();
        let n = self
            .mirror
            .update_field_timed(&mut enc, timer.map(MarchTimer::writes));
        if let Some(t) = timer.filter(|_| n > 0) {
            t.resolve(&mut enc);
        }
        gpu.queue.submit([enc.finish()]);
        let ms = timer.filter(|_| n > 0).map(|t| t.read_ms(&gpu.device));
        (n, ms)
    }
}

/// The window's shell: `size`, the cursor locked, `pace`, and `--adapter`'s substring.
pub(crate) fn shell_config(args: &Args, size: [u32; 2], pace: svet_app::Pacing) -> ShellConfig {
    ShellConfig {
        title: "flythrough".to_owned(),
        size,
        lock_cursor: true,
        pacing: pace,
        adapter: args.adapter.clone(),
        ..ShellConfig::default()
    }
}

/// The scene's material table from the material file (`--materials`, or
/// `assets/materials.txt`; `crate::materials`): the cave's rows with the ore emissive unless
/// `--emissive-ore off`, the castle's under `--castle`, the outdoor hills' under
/// `--source outdoor`, the rigs' wood under the thin and seam rigs, the showcase's under the
/// showcase. A file that does not read or parse, or lacks a row,
/// ends the process with status 1.
pub(crate) fn materials(args: &Args) -> MaterialTable {
    let fail = |e: String| -> ! {
        eprintln!("flythrough: {e}");
        std::process::exit(1)
    };
    let (file, _) = crate::materials::load(args.materials.as_deref()).unwrap_or_else(|e| fail(e));
    crate::materials::selection(args)
        .compose(&file)
        .unwrap_or_else(|e| fail(e))
}

/// `table` with the walker's rows, then a row for each palette entry of `--model`'s and
/// `--vox`'s `.vox` (ADR-0065), then the structures library's rows
/// (`structures::add_library_rows`), then each row the entities library's rigs name in their
/// `material` lines that the table lacks, from the material file by name
/// (`entities::library::rig_material_names`; a name the file lacks stays an error on its
/// entry): the world's table, fixed once the world is generated. The rigs' rows come last, so
/// every row before them keeps its id.
pub(crate) fn world_table(args: &Args, table: &MaterialTable) -> MaterialTable {
    let mut t = table_before_entities(args, table);
    if let Ok((file, _)) = crate::materials::load(args.materials.as_deref()) {
        // A rig's name resolves as `viewer::material` reads it: as written, or each `_` a space.
        let wanted: Vec<String> = crate::entities::library::rig_material_names(args)
            .into_iter()
            .filter(|n| crate::viewer::material(&t, n).is_none())
            .filter_map(|n| {
                let spaced = n.replace('_', " ");
                [n, spaced].into_iter().find(|f| file.row(f).is_some())
            })
            .collect();
        crate::materials::append_named(&mut t, &file, wanted.iter().map(String::as_str));
    }
    t
}

/// [`world_table`] before the rows the entities library's rigs name: [`base_table`] and the
/// structures library's rows.
pub(crate) fn table_before_entities(args: &Args, table: &MaterialTable) -> MaterialTable {
    let mut t = base_table(args, table);
    let dir = crate::structures::dir_of(args);
    crate::structures::add_library_rows(&dir, args.materials.as_deref(), &mut t);
    t
}

/// [`world_table`] before the structures library's rows.
pub(crate) fn base_table(args: &Args, table: &MaterialTable) -> MaterialTable {
    let mut t = crate::walker::with_walker(table);
    if let Some(path) = &args.model {
        crate::viewer::palette_rows(path, &mut t, svet_assets::DEFAULT_EMISSION_FACTOR);
    }
    if let Some(v) = &args.vox
        && let Ok(file) = crate::structures::read(&v.path)
    {
        crate::structures::add_rows(&file, &mut t);
    }
    t
}

/// `--vox-op`'s ops for the world whose table is `table` (`crate::structures::ops`), placed
/// as `--vox` places them through the source; `None` without `--vox-op`. A file that does not
/// read or load ends the process with status 1.
pub(crate) fn vox_ops(args: &Args, table: &MaterialTable) -> Option<crate::structures::VoxOps> {
    use crate::structures as st;
    let v = args.vox.as_ref().filter(|_| args.vox_op)?;
    let scene = st::read(&v.path)
        .and_then(|f| st::scene(&f, table, v.scale))
        .unwrap_or_else(|e| {
            eprintln!("flythrough: --vox {e}");
            std::process::exit(1)
        });
    let spawn = CaveSource::unbounded(args.seed).spawn();
    let ops = st::ops(&scene, st::offset(&scene, spawn, v.at), 0, args.vox_clear);
    println!("vox op: {} ops, submitted on the first tick", ops.ops.len());
    Some(ops)
}

/// `--vox`'s scene over `base` (`crate::structures`), its rows in `table`: the source and its
/// printed line. A file that does not read or load ends the process with status 1.
fn vox_source<S: svet_world::WorldSource>(
    args: &Args,
    table: &MaterialTable,
    spawn: VoxelCoord,
    base: S,
) -> svet_world::StampedSource<crate::structures::ClearedSource<S>> {
    use crate::structures as st;
    let v = args.vox.as_ref().expect("--vox given");
    let t = now();
    let scene = st::read(&v.path)
        .and_then(|f| st::scene(&f, table, v.scale))
        .unwrap_or_else(|e| {
            eprintln!("flythrough: --vox {e}");
            std::process::exit(1)
        });
    let offset = st::offset(&scene, spawn, v.at);
    let (lo, hi) = scene.bounds;
    let size = scene.size();
    println!(
        "vox {}: {} placements of {} prefabs, {} x {} x {} voxels at {:?}..{:?}, scale {}, \
         clear {}, loaded in {:.0} ms",
        v.path.display(),
        scene.placements.len(),
        scene.prefabs.len(),
        size[0],
        size[1],
        size[2],
        [0, 1, 2].map(|k| lo[k] + offset[k]),
        [0, 1, 2].map(|k| hi[k] + offset[k]),
        v.scale,
        if args.vox_clear { "on" } else { "off" },
        ms_since(t)
    );
    st::source(base, &scene, offset, args.vox_clear)
}

/// The palette rows [`world_table`] adds for `--model`'s `.vox`: the viewer's `rows +N`.
pub(crate) fn model_rows_added(args: &Args) -> usize {
    args.model.as_ref().map_or(0, |path| {
        let mut t = crate::walker::with_walker(&materials(args));
        crate::viewer::palette_rows(path, &mut t, svet_assets::DEFAULT_EMISSION_FACTOR)
    })
}

/// `--crisp`: which rows of the fly-through's table carry `MaterialFlags::CRISP` (a testbed
/// switch for the field surface's cost lever): none added, the cave's rock row, or every row
/// that is not translucent. Rows the table marks crisp itself (the thin rig's wood) stay crisp.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CrispRows {
    #[default]
    None,
    Rock,
    All,
}

impl CrispRows {
    pub(crate) fn named(name: &str) -> Option<Self> {
        match name {
            "none" => Some(Self::None),
            "rock" => Some(Self::Rock),
            "all" => Some(Self::All),
            _ => None,
        }
    }

    /// `table` with this choice's rows marked crisp.
    pub(crate) fn apply(self, table: &MaterialTable) -> MaterialTable {
        if self == Self::None {
            return table.clone();
        }
        let mut t = MaterialTable::new();
        for (id, row) in table.rows().iter().enumerate().skip(1) {
            let mut row = row.clone();
            let pick = match self {
                Self::None => false,
                Self::Rock => id == usize::from(crate::cave::ROCK),
                Self::All => !row.is_translucent(),
            };
            if pick {
                row.flags = row.flags | svet_world::MaterialFlags::CRISP;
            }
            t.push(row);
        }
        t
    }

    /// The crisp rows of `table` by name, for the start-up line.
    pub(crate) fn line(table: &MaterialTable) -> String {
        let names: Vec<&str> = table
            .rows()
            .iter()
            .enumerate()
            .filter(|&(id, _)| table.is_crisp(id as svet_world::MaterialId))
            .map(|(_, m)| m.name.as_str())
            .collect();
        format!(
            "world crisp rows: {}",
            if names.is_empty() {
                "none".to_owned()
            } else {
                names.join(", ")
            }
        )
    }
}

/// The camera at castle shot `shot`: the eye voxel's centre, its yaw and pitch.
pub(crate) fn castle_camera(shot: castle::CastleShot) -> Camera {
    let (eye, yaw, pitch) = shot.pose();
    Camera {
        position: eye.map(|v| v as f32 + 0.5).into(),
        yaw: (yaw as f32).to_radians(),
        pitch: (pitch as f32).to_radians(),
        vfov: 70f32.to_radians(),
    }
}

/// The camera at showcase shot `shot`: the eye voxel's centre, its yaw and pitch.
pub(crate) fn shot_camera(shot: &Shot) -> Camera {
    Camera {
        position: shot.eye.0.map(|v| v as f32 + 0.5).into(),
        yaw: (shot.yaw_deg as f32).to_radians(),
        pitch: (shot.pitch_deg as f32).to_radians(),
        vfov: 70f32.to_radians(),
    }
}

pub(crate) fn start_camera(args: &Args, spawn: VoxelCoord) -> Camera {
    if let Some(path) = args.path {
        let pose = path.sample(0.0);
        Camera {
            position: pose.eye.into(),
            yaw: pose.yaw_deg.to_radians(),
            pitch: pose.pitch_deg.to_radians(),
            vfov: 70f32.to_radians(),
        }
    } else if let Some(eye) = args.eye {
        eye_camera(eye)
    } else if args.castle {
        castle_camera(args.castle_shot)
    } else if args.mode.is_showcase() {
        shot_camera(&args.shot)
    } else if let Some(pose) = args.pose {
        pose.camera()
    } else if args.mode.is_thin_rig() {
        crate::thin_rig_camera()
    } else if args.mode.is_seam_rig() {
        crate::seam_rig_camera()
    } else if args.mode.is_outdoor() {
        outdoor_camera(args)
    } else {
        Camera {
            position: spawn.0.map(|v| v as f32 + 0.5).into(),
            yaw: 0.0,
            pitch: 0.0,
            vfov: 70f32.to_radians(),
        }
    }
}

/// `--eye`'s camera: the point in voxels, the yaw and pitch in degrees.
pub(crate) fn eye_camera([x, y, z, yaw, pitch]: [f32; 5]) -> Camera {
    Camera {
        position: [x, y, z].into(),
        yaw: yaw.to_radians(),
        pitch: pitch.to_radians(),
        vfov: 70f32.to_radians(),
    }
}

/// The voxel `--eye` stands in, if given: the streamer's and the mirror's centre in place of
/// the source's spawn.
fn eye_voxel(args: &Args) -> Option<VoxelCoord> {
    args.eye
        .map(|[x, y, z, ..]| VoxelCoord([x, y, z].map(|v| v.floor() as i32)))
}

/// `--spawn`'s camera on the outdoor source of `args.seed`: the eye voxel's centre, facing
/// the spawn's direction; `--eye`'s point and view in its place when given.
pub(crate) fn outdoor_camera(args: &Args) -> Camera {
    if let Some(eye) = args.eye {
        return eye_camera(eye);
    }
    let (eye, d) = OutdoorSource::new(args.seed).spawn(args.outdoor.spawn);
    let [x, y, z] = d.map(|v| v as f32);
    Camera {
        position: eye.0.map(|v| v as f32 + 0.5).into(),
        yaw: y.atan2(x),
        pitch: z.atan2((x * x + y * y).sqrt()),
        vfov: 70f32.to_radians(),
    }
}

/// The streamer's default generation workers: [`svet_render::ThreadPlan`]'s on this machine,
/// half the logical cores and at least 1.
pub(crate) fn default_workers() -> usize {
    svet_render::ThreadPlan::here(None).workers
}

/// The castle's banner row for `shot`: `castle gate`.
pub(crate) fn castle_banner(shot: castle::CastleShot) -> String {
    format!("castle {}", shot.name())
}

/// `--castle`'s source over the cave of `seed`: the castle generated, encoded (and written to
/// `--castle-out`), parsed, mapped and placed, each stage's time printed, then the prefabs'
/// bytes.
pub(crate) fn castle_source(
    seed: u64,
    out: Option<&std::path::Path>,
) -> svet_world::StampedSource<crate::cave::HallSource> {
    let t = now();
    let file = castle::castle_vox();
    let listed: usize = file.models.iter().map(|m| m.voxels.len()).sum();
    println!(
        "castle: generated {} tiles, {listed} voxels listed, in {:.0} ms",
        file.models.len(),
        ms_since(t)
    );
    let t = now();
    let bytes = svet_assets::encode(&file);
    drop(file);
    println!(
        "castle: encoded {} bytes of .vox in {:.0} ms",
        bytes.len(),
        ms_since(t)
    );
    if let Some(path) = out {
        match std::fs::write(path, &bytes) {
            Ok(()) => println!("castle: wrote {}", path.display()),
            Err(e) => eprintln!("flythrough: --castle-out {}: {e}", path.display()),
        }
    }
    let t = now();
    let file = svet_assets::VoxFile::parse(&bytes).unwrap_or_else(|e| {
        eprintln!("flythrough: the generated castle does not parse: {e}");
        std::process::exit(1);
    });
    drop(bytes);
    println!("castle: parsed in {:.0} ms", ms_since(t));
    let t = now();
    let placements = castle::castle_prefabs(&file).unwrap_or_else(|e| {
        eprintln!("flythrough: the castle's palette map: {e}");
        std::process::exit(1);
    });
    drop(file);
    let bytes = castle::prefab_bytes(placements.iter().map(|(p, _, _)| &**p));
    let dense = castle::dense_prefab_bytes(placements.iter().map(|(p, _, _)| &**p));
    println!(
        "castle: mapped {} prefabs, {bytes} bytes ({dense} dense), in {:.0} ms",
        placements.len(),
        ms_since(t)
    );
    let t = now();
    let source = castle::castle_source(seed, placements);
    println!("castle: placed in {:.0} ms", ms_since(t));
    source
}

/// `choice`'s logic, or, when it cannot load, the reason printed and the process ended with
/// status 1.
pub(crate) fn load_logic(choice: &LogicChoice) -> Box<dyn Logic> {
    match choice.load() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("flythrough: --logic {choice}: {e}");
            std::process::exit(1);
        }
    }
}

/// The world `args` name, generating on its own threads or streaming: the spawn and what the
/// app builds its run from. A replay continues under the save's seed (`args.seed` takes it); one
/// whose file cannot be read sets `failure` and builds nothing, so the session tears down as
/// usual and the failure prints at the end.
pub(crate) fn build_world(args: &mut Args, failure: &Failure) -> (VoxelCoord, Pending) {
    let (spawn, pending) = source_world(args, failure);
    (eye_voxel(args).unwrap_or(spawn), pending)
}

/// [`build_world`]'s world before `--eye` moves the spawn: the source's own spawn.
fn source_world(args: &mut Args, failure: &Failure) -> (VoxelCoord, Pending) {
    let seed = args.seed;
    if let Some(path) = args.replay.clone() {
        replay_world(args, &path, failure)
    } else if args.mode.is_thin_rig() {
        let source = crate::thin_rig::ThinRig {
            wood: crate::THIN_RIG_WOOD,
        };
        let table = world_table(args, &materials(args));
        println!(
            "thin rig: a 5-voxel trunk on a floor with free one-, two- and three-voxel rods, a \
             diagonal branch and two sheets of wood, {:?} chunks",
            source.extent().max.0
        );
        let generation =
            std::thread::spawn(move || Ok(World::generate(source.extent(), &source, &table)));
        (source.spawn(), Pending::Level(generation))
    } else if args.mode.is_seam_rig() {
        let source = crate::thin_rig::SeamRig {
            crisp: crate::THIN_RIG_WOOD,
        };
        let table = world_table(args, &materials(args));
        println!(
            "seam rig: a rock wall with an 8 x 8 wood inlay flush in its face and a wood L on the \
             rock floor, {:?} chunks",
            source.extent().max.0
        );
        let generation =
            std::thread::spawn(move || Ok(World::generate(source.extent(), &source, &table)));
        (source.spawn(), Pending::Level(generation))
    } else if let Some(edge) = args.mode.room() {
        let source = RoomSource::new(edge);
        let table = world_table(args, &materials(args));
        println!(
            "room: a closed {:.1} m cube of air in 2 m of rock, {:?} chunks",
            edge as f32 * METRES_PER_VOXEL,
            source.extent.max.0
        );
        let generation =
            std::thread::spawn(move || Ok(World::generate(source.extent, &source, &table)));
        (source.spawn(), Pending::Level(generation))
    } else if args.mode.is_showcase() {
        let source = ShowcaseSource::new(seed);
        let table = world_table(args, &materials(args));
        let generation = std::thread::spawn(move || {
            let t = now();
            let extent = source.extent();
            let world = World::generate(extent, &source, &table);
            println!(
                "generated the showcase, seed {seed}, over {SHOWCASE_CHUNKS:?} chunks in {:.0} ms: {} full bricks",
                ms_since(t),
                world.stats().full_bricks
            );
            Ok(world)
        });
        (args.shot.eye, Pending::Level(generation))
    } else if args.mode.is_level() {
        level_world(args)
    } else if args.mode.is_outdoor() {
        outdoor_world(args)
    } else {
        streamed_world(args)
    }
}

/// `--replay`'s world: the save read, then its inputs re-simulated through `args.logic` on a
/// thread of its own.
fn replay_world(args: &mut Args, path: &Path, failure: &Failure) -> (VoxelCoord, Pending) {
    let replay_logic = load_logic(&args.logic);
    match crate::read_log_file(path) {
        Err(e) => {
            // The session tears down as usual and `main` prints this at the end.
            failure.set(format!("--replay {}: {e}", path.display()));
            (VoxelCoord([0; 3]), Pending::Failed)
        }
        Ok(file) => {
            // The session continues under the save's seed.
            args.seed = file.seed;
            let table = world_table(args, &materials(args));
            let source = CaveSource::new(file.seed, file.extent);
            let with_vault = args.vault;
            if with_vault {
                println!(
                    "vault: {VAULT_EDGE}^3 shell room at {:?}",
                    source.vault_origin().0
                );
            }
            let path = path.to_path_buf();
            let generation = std::thread::spawn(move || {
                let (world, ops) = crate::replay_log_on(
                    &file,
                    crate::Mode::Inputs,
                    ChunkExtent::new([0, 0, 0], LEVEL_CHUNKS),
                    &table,
                    replay_logic,
                    with_vault,
                )
                .map_err(|e| format!("--replay {}: {e}", path.display()))?;
                println!(
                    "replayed {} inputs, {} ops, hash 0x{:016x}",
                    file.inputs.len(),
                    ops.len(),
                    world.voxel_hash()
                );
                if ops != file.ops {
                    println!("replay: the re-simulated op log differs from the saved one");
                }
                Ok(world)
            });
            (source.spawn(), Pending::Level(generation))
        }
    }
}

/// `--level`'s world: the bounded cave of `args.seed`, with the vault when `--vault` asks.
fn level_world(args: &Args) -> (VoxelCoord, Pending) {
    let seed = args.seed;
    let source = CaveSource::new(seed, ChunkExtent::new([0, 0, 0], LEVEL_CHUNKS));
    if args.vault {
        println!(
            "vault: {VAULT_EDGE}^3 shell room at {:?}",
            source.vault_origin().0
        );
    }
    let table = world_table(args, &materials(args));
    let with_vault = args.vault;
    let generation = std::thread::spawn(move || {
        let t = now();
        let world = if with_vault {
            World::generate(source.extent, &source.with_vault(), &table)
        } else {
            World::generate(source.extent, &source, &table)
        };
        println!(
            "generated seed {seed} over {LEVEL_CHUNKS:?} chunks in {:.0} ms: {} full bricks",
            ms_since(t),
            world.stats().full_bricks
        );
        Ok(world)
    });
    (source.spawn(), Pending::Level(generation))
}

/// The unbounded cave, streamed around the camera: an empty world and a streamer whose workers
/// start on the spawn's chunks while the shell opens the window.
fn streamed_world(args: &Args) -> (VoxelCoord, Pending) {
    let seed = args.seed;
    let source = CaveSource::unbounded(seed);
    // `--vox` places at the source's spawn; the workers start around `--eye` when given.
    let placed_at = source.spawn();
    let spawn = eye_voxel(args).unwrap_or(placed_at);
    let workers = args.workers.unwrap_or_else(default_workers);
    let pack = args.mirror_pack;
    let table = world_table(args, &materials(args));
    let world = World::new(&table);
    let cfg = StreamConfig {
        residency: args.residency,
        workers,
        persist: PersistConfig::default(),
    };
    println!(
        "streaming seed {seed}: r_full {}, r_lod {}, h {}, {workers} workers, window {}",
        args.residency.r_full,
        args.residency.r_lod,
        args.residency.hysteresis,
        crate::window_side(args.residency)
    );
    println!("{}", crate::hud::mirror_sync_row(args.sync_time));
    if args.castle {
        // The castle's prefabs build on a thread of their own, so the window, the device and
        // the first loading frame do not wait for them; the run joins it after the device.
        let out = args.castle_out.clone();
        let build = std::thread::spawn(move || {
            let streamer = Streamer::new(Arc::new(castle_source(seed, out.as_deref())), cfg);
            Box::new(start_streaming(world, streamer, workers, spawn, pack))
        });
        return (spawn, Pending::StreamBuilding(build));
    }
    let streamer = if args.vox.is_some() && !args.vox_op {
        Streamer::new(Arc::new(vox_source(args, &table, placed_at, source)), cfg)
    } else if args.vault {
        println!(
            "vault: {VAULT_EDGE}^3 shell room at {:?}",
            source.vault_origin().0
        );
        Streamer::new(Arc::new(source.with_vault()), cfg)
    } else {
        Streamer::new(Arc::new(source), cfg)
    };
    (
        spawn,
        Pending::Stream(Box::new(start_streaming(
            world, streamer, workers, spawn, pack,
        ))),
    )
}

/// The outdoor hills of `args.seed`, with `--trees` stood on them, streamed around the camera
/// as the cave is, over the same [`world_table`] as every other source.
fn outdoor_world(args: &Args) -> (VoxelCoord, Pending) {
    let hills = OutdoorSource::new(args.seed);
    let spawn = eye_voxel(args).unwrap_or_else(|| hills.spawn(args.outdoor.spawn).0);
    let trees = args.outdoor.trees();
    let workers = args.workers.unwrap_or_else(default_workers);
    let pack = args.mirror_pack;
    let world = World::new(&world_table(args, &materials(args)));
    let cfg = StreamConfig {
        residency: args.residency,
        workers,
        persist: PersistConfig::default(),
    };
    println!(
        "streaming the outdoor hills, seed {}, trees {} (leaf fill {}): spawn {} at {:?}, r_full {}, r_lod {}, {workers} workers",
        args.seed,
        trees.name(),
        args.outdoor.leaf_fill(),
        args.outdoor.spawn.name(),
        spawn.0,
        args.residency.r_full,
        args.residency.r_lod,
    );
    println!("{}", crate::hud::mirror_sync_row(args.sync_time));
    let streamer = if trees == crate::trees::TreeDensity::None {
        Streamer::new(Arc::new(hills), cfg)
    } else {
        let forest = crate::trees::ForestSource::new(hills, trees, args.outdoor.leaf_fill());
        Streamer::new(Arc::new(forest), cfg)
    };
    (
        spawn,
        Pending::Stream(Box::new(start_streaming(
            world, streamer, workers, spawn, pack,
        ))),
    )
}

/// A streamed world as the app takes it: the world, its streamer, the workers and the
/// workers' slot images ([`svet_render::ImageHook`]).
pub(crate) type Streamed = (World, Streamer, usize, svet_render::ImageHook);

/// `streamer` with the live queue's listing and the mirror's slot images (`pack`, ADR-0056,
/// under the default cap) on its workers (ADR-0017), from the first chunk on, and the
/// workers started on the `spawn`'s chunks. Prints the packing's row.
fn start_streaming(
    world: World,
    mut streamer: Streamer,
    workers: usize,
    spawn: VoxelCoord,
    pack: svet_render::Packing,
) -> Streamed {
    let images = svet_render::ImageHook::new(
        world.materials(),
        pack,
        svet_render::DEFAULT_IMAGE_CAP_BYTES,
    );
    streamer.set_prep(Some(Arc::new(svet_render::ChunkWork::new(
        world.materials(),
        Some(images.clone()),
    ))));
    println!("{}", crate::hud::mirror_pack_row(pack));
    streamer.update(&world, &[spawn]);
    (world, streamer, workers, images)
}

/// The app `args` ask for, from the session's `start`: the look line printed, the logic loaded,
/// the world started ([`build_world`]), then the start camera at the spawn (or `--pose`'s,
/// printed), the audio and the room probe as `args` ask, and the pad.
pub fn flythrough(start: Instant, mut args: Args) -> Flythrough {
    println!("{}", look_line(&args.start_look(), &args.look.preset));
    let logic = load_logic(&args.logic);
    println!("logic: {}", args.logic);
    let failure = Failure::default();
    let (spawn, pending) = build_world(&mut args, &failure);
    if let Some(pose) = args.pose {
        println!(
            "{}: position {:?}, yaw {} deg, pitch {} deg, {}",
            hud::pose_banner(pose.name),
            pose.position,
            pose.yaw_deg,
            pose.pitch_deg,
            args.light
                .exposure
                .map_or("exposure auto".to_owned(), |e| format!(
                    "exposure manual {e}"
                ))
        );
    }
    let camera = start_camera(&args, spawn);
    let audio = if args.audio.on {
        Audio::open(
            args.audio.debug,
            args.audio.absorption,
            args.audio.mic,
            args.audio.room_line,
            args.audio.mix,
        )
    } else {
        None
    };
    let room_probe = (audio.is_none() && args.audio.room_line)
        .then(|| RoomProbe::new(args.audio.absorption, args.audio.mix));
    if args.audio.on || args.audio.room_line {
        println!("{}", args.audio.mix.line());
    }
    let pad = if args.wants_pad() { Pad::open() } else { None };
    if !args.headless && !args.autopilot {
        println!(
            "{}",
            pad.as_ref()
                .map_or_else(|| "pad: none".to_owned(), Pad::startup_line)
        );
    }
    Flythrough {
        start,
        args,
        camera,
        logic: Some(logic),
        pending: Some(pending),
        run: None,
        audio,
        room_probe,
        pad,
        failure,
        splash: None,
        switching: None,
    }
}
