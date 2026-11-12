//! What the fly-through builds before its first frame: the material table, the start
//! camera, the shell's configuration and the GPU objects of a run ([`Gfx`]).

use crate::app::{Flythrough, Pending, ms_since};
use crate::audio::{Audio, METRES_PER_VOXEL, RoomProbe};
use crate::castle;
use crate::cave::{CaveSource, LEVEL_CHUNKS, RoomSource, VAULT_EDGE};
use crate::cli::Args;
use crate::edits::Failure;
use crate::hud::{self};
use crate::now;
use crate::pad::Pad;
use crate::run::hud_feed::look_line;
use crate::showcase::{SHOWCASE_CHUNKS, Shot, ShowcaseSource};
use crate::{LogicChoice, cave_materials};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use vxe_app::{Gpu, ShellConfig};
use vxe_net::Logic;
use vxe_render::{Camera, Frame, GpuWorld, HudOverlay, MarchTimer, PassTimer, Presenter, wgpu};
use vxe_world::{
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
/// samples, and the internal and output sizes while the upscaler runs.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GfxSample {
    pub(crate) mirror_bytes: u64,
    pub(crate) pool_live: u32,
    pub(crate) pool_free: u32,
    pub(crate) pressure: f32,
    pub(crate) budget_bytes: u64,
    pub(crate) light_samples: u64,
    pub(crate) upscale: Option<[[u32; 2]; 2]>,
}

impl GfxSample {
    pub(crate) fn of(g: &Gfx) -> Self {
        Self {
            mirror_bytes: g.mirror.mirror_bytes(),
            pool_live: g.mirror.live_bricks(),
            pool_free: g.mirror.spare_slots().min(u64::from(u32::MAX)) as u32,
            pressure: g.mirror.pressure(),
            budget_bytes: g.mirror.allocated_bytes(),
            light_samples: g.mirror.light_samples(),
            upscale: g
                .frame
                .upscaling()
                .then(|| [g.frame.internal_size(), g.frame.output_size()]),
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
pub(crate) fn shell_config(args: &Args, size: [u32; 2], pace: vxe_app::Pacing) -> ShellConfig {
    ShellConfig {
        title: "flythrough".to_owned(),
        size,
        lock_cursor: true,
        pacing: pace,
        adapter: args.adapter.clone(),
        ..ShellConfig::default()
    }
}

/// The cave's material table, the ore emissive unless `--emissive-ore off`; the castle's rows
/// under `--castle`.
pub(crate) fn materials(args: &Args) -> MaterialTable {
    let table = cave_materials();
    let table = if args.emissive_ore {
        crate::with_emissive_ore(&table)
    } else {
        table
    };
    let table = if args.castle {
        crate::castle_materials(&table)
    } else if args.thin_rig || args.seam_rig {
        crate::with_wood(&table)
    } else {
        table
    };
    args.crisp.apply(&table)
}

/// `--crisp`: which rows of the fly-through's table carry `MaterialFlags::CRISP` (a testbed
/// switch for the field surface's cost lever): none added, the cave's rock row, or every row
/// that is not translucent. Rows the table marks crisp itself (the thin rig's wood) stay crisp.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum CrispRows {
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
                row.flags = row.flags | vxe_world::MaterialFlags::CRISP;
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
            .filter(|&(id, _)| table.is_crisp(id as vxe_world::MaterialId))
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
    } else if args.castle {
        castle_camera(args.castle_shot)
    } else if args.showcase {
        shot_camera(&args.shot)
    } else if let Some(pose) = args.pose {
        pose.camera()
    } else if args.thin_rig {
        crate::thin_rig_camera()
    } else if args.seam_rig {
        crate::seam_rig_camera()
    } else {
        Camera {
            position: spawn.0.map(|v| v as f32 + 0.5).into(),
            yaw: 0.0,
            pitch: 0.0,
            vfov: 70f32.to_radians(),
        }
    }
}

/// The streamer's default generation workers: one fewer than the logical cores, at least 1,
/// leaving a core to the render thread.
pub(crate) fn default_workers() -> usize {
    std::thread::available_parallelism()
        .map_or(1, usize::from)
        .saturating_sub(1)
        .max(1)
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
) -> vxe_world::StampedSource<crate::cave::HallSource> {
    let t = now();
    let file = castle::castle_vox();
    let listed: usize = file.models.iter().map(|m| m.voxels.len()).sum();
    println!(
        "castle: generated {} tiles, {listed} voxels listed, in {:.0} ms",
        file.models.len(),
        ms_since(t)
    );
    let t = now();
    let bytes = vxe_assets::encode(&file);
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
    let file = vxe_assets::VoxFile::parse(&bytes).unwrap_or_else(|e| {
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
    println!(
        "castle: mapped {} prefabs, {bytes} bytes, in {:.0} ms",
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
    let seed = args.seed;
    if let Some(path) = args.replay.clone() {
        replay_world(args, &path, failure)
    } else if args.thin_rig {
        let source = crate::thin_rig::ThinRig {
            wood: crate::THIN_RIG_WOOD,
        };
        let table = materials(args);
        println!(
            "thin rig: a 5-voxel trunk on a floor with free one-, two- and three-voxel rods, a \
             diagonal branch and two sheets of wood, {:?} chunks",
            source.extent().max.0
        );
        let generation =
            std::thread::spawn(move || Ok(World::generate(source.extent(), &source, &table)));
        (source.spawn(), Pending::Level(generation))
    } else if args.seam_rig {
        let source = crate::thin_rig::SeamRig {
            crisp: crate::THIN_RIG_WOOD,
        };
        let table = materials(args);
        println!(
            "seam rig: a rock wall with an 8 x 8 wood inlay flush in its face and a wood L on the \
             rock floor, {:?} chunks",
            source.extent().max.0
        );
        let generation =
            std::thread::spawn(move || Ok(World::generate(source.extent(), &source, &table)));
        (source.spawn(), Pending::Level(generation))
    } else if let Some(edge) = args.room {
        let source = RoomSource::new(edge);
        let table = materials(args);
        println!(
            "room: a closed {:.1} m cube of air in 2 m of rock, {:?} chunks",
            edge as f32 * METRES_PER_VOXEL,
            source.extent.max.0
        );
        let generation =
            std::thread::spawn(move || Ok(World::generate(source.extent, &source, &table)));
        (source.spawn(), Pending::Level(generation))
    } else if args.showcase {
        let source = ShowcaseSource::new(seed);
        let table = crate::with_showcase_rows(&materials(args));
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
    } else if args.level {
        level_world(args)
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
            let table = materials(args);
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
    let table = materials(args);
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
    let workers = args.workers.unwrap_or_else(default_workers);
    let world = World::new(&materials(args));
    let cfg = StreamConfig {
        residency: args.residency,
        workers,
        persist: PersistConfig::default(),
    };
    let mut streamer = if args.castle {
        Streamer::new(
            Arc::new(castle_source(seed, args.castle_out.as_deref())),
            cfg,
        )
    } else if args.vault {
        println!(
            "vault: {VAULT_EDGE}^3 shell room at {:?}",
            source.vault_origin().0
        );
        Streamer::new(Arc::new(source.with_vault()), cfg)
    } else {
        Streamer::new(Arc::new(source), cfg)
    };
    // The live queue's listing runs on the workers (ADR-0017), from the first chunk on.
    streamer.set_prep(Some(Arc::new(vxe_render::internals::FacePrep {
        classes: Arc::new(vxe_render::internals::face_classes(world.materials())),
    })));
    // Workers start on the spawn's chunks while the shell opens the window.
    streamer.update(&world, &[source.spawn()]);
    println!(
        "streaming seed {seed}: r_full {}, r_lod {}, h {}, {workers} workers, window {}",
        args.residency.r_full,
        args.residency.r_lod,
        args.residency.hysteresis,
        crate::window_side(args.residency)
    );
    (
        source.spawn(),
        Pending::Stream(Box::new((world, streamer, workers))),
    )
}

/// The app `args` ask for, from the session's `start`: the look line printed, the logic loaded,
/// the world started ([`build_world`]), then the start camera at the spawn (or `--pose`'s,
/// printed), the audio and the room probe as `args` ask, and the pad.
pub fn flythrough(start: Instant, mut args: Args) -> Flythrough {
    println!("{}", look_line(&args.start_look(), &args.look));
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
            args.exposure
                .map_or("exposure auto".to_owned(), |e| format!(
                    "exposure manual {e}"
                ))
        );
    }
    let camera = start_camera(&args, spawn);
    let audio = if args.audio {
        Audio::open(
            args.audio_debug,
            args.absorption,
            args.mic,
            args.room_line,
            args.mix,
        )
    } else {
        None
    };
    let room_probe =
        (audio.is_none() && args.room_line).then(|| RoomProbe::new(args.absorption, args.mix));
    if args.audio || args.room_line {
        println!("{}", args.mix.line());
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
    }
}
