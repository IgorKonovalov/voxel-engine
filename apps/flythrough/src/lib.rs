//! The fly-through's reusable parts: the cave world source its binary, tests and examples
//! share, its server logic (the `flythrough-logic` crate, as [`logic`]) and the choice of its
//! native or hosted build, save replay, the streaming defaults, and the small measurement
//! helpers its readings use. The engine ships no terrain; this cave is the testbed's own.

pub mod castle;
pub mod cave;
pub mod hud;
pub use flythrough_logic as logic;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use vxe_audio::Listener;
use vxe_net::{Logic, Server};
use vxe_render::Camera;
use vxe_world::hash::Fnv1a;
use vxe_world::{
    BRICK_VOXELS, Brick, Chunk, ChunkCoord, ChunkExtent, ChunkMips, LogFile, Material,
    MaterialFlags, MaterialTable, OpLog, Residency, WireError, World,
};

use cave::CaveSource;
use logic::DigLogic;

/// The fly-through's residency defaults (Plan 0002): full detail to 16 chunks (102 m), mips
/// to 24 (154 m), one chunk of hysteresis.
pub const RESIDENCY: Residency = Residency {
    r_full: 16,
    r_lod: 24,
    hysteresis: 1,
};

/// The side of the mirror window that holds a residency: `2 (r_lod + h) + 1` chunks.
pub fn window_side(r: Residency) -> i32 {
    2 * (r.r_lod.max(r.r_full) + r.hysteresis) + 1
}

/// Mirror pressure under which the fly-through's full radius returns a chunk (ADR-0013). The
/// streamed mirror's records are sized so their occupancy stays under it.
pub const PRESSURE_LOW: f32 = 0.7;

/// Records the streamed mirror reserves for residency `r`: the chunks one focus can keep,
/// `chunks_in_ball(max(r_lod, r_full) + h)`, over [`PRESSURE_LOW`] (as `* 10 / 7`, rounded
/// up), so the record term of the pressure stays under the restore line while the streamer
/// keeps its radius. 93 239 at the defaults.
pub fn stream_record_chunks(r: Residency) -> u64 {
    let ball = vxe_world::chunks_in_ball(r.r_lod.max(r.r_full) + r.hysteresis);
    (ball * 10).div_ceil(7)
}

/// The mirror budget of the streamed cave unless `--mirror-budget` sets one: the world line
/// of `nfr.md` §3, 3 GiB, of which the residency's records take their share.
pub const STREAM_MIRROR_BUDGET: u64 = vxe_render::gpu_world::DEFAULT_BUDGET_BYTES;

/// `Full` bricks of headroom beyond the bounded level's own, so digging has room: 64 Ki
/// bricks, each at the largest slot the level's rock takes (6 units, and 6 plus the most
/// attribute units with attributes).
pub const POOL_HEADROOM_BRICKS: u64 = 65_536;

/// The pool units the bounded level `world` needs: each bin's `Full` bricks in whole pages
/// of that bin, their slots with their chunk-local surface's contour attributes when `attrs`,
/// plus the headroom.
pub fn level_pool_units(world: &vxe_world::World, attrs: bool) -> u64 {
    use vxe_render::gpu_world::{
        BIN_COUNT, BINS, MAX_ATTR_UNITS, PAGE_UNITS, bin_of, chunk_surface, slot_units,
        slots_per_page,
    };
    let table = world.materials();
    let translucent = |m| {
        table
            .flags(m)
            .contains(vxe_world::MaterialFlags::TRANSLUCENT)
    };
    let mut bins = [0u64; BIN_COUNT];
    for c in world.store().coords() {
        let Some(chunk) = world.store().get(c) else {
            continue;
        };
        let chunk = chunk
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (i, b) in chunk.bricks().iter().enumerate() {
            if let vxe_world::Brick::Full(p) = b {
                let surface = attrs.then(|| {
                    let s = chunk_surface(chunk.bricks(), i);
                    s.iter().map(|w| w.count_ones()).sum()
                });
                bins[bin_of(slot_units(p, translucent, surface))] += 1;
            }
        }
    }
    let pages: u64 = bins
        .iter()
        .enumerate()
        .map(|(k, &n)| n.div_ceil(u64::from(slots_per_page(k))))
        .sum();
    let largest = 6 + if attrs { MAX_ATTR_UNITS } else { 0 };
    let headroom = POOL_HEADROOM_BRICKS * u64::from(BINS[bin_of(largest)]);
    pages * u64::from(PAGE_UNITS) + headroom
}

/// The mirror budget of the bounded level unless `--mirror-budget` sets one: the pool
/// [`level_pool_units`] needs and a record for each of its `chunks`, sized from the world it
/// loads rather than the streamed default.
pub fn level_mirror_budget(world: &vxe_world::World, chunks: u64, attrs: bool) -> u64 {
    vxe_render::gpu_world::budget_for(level_pool_units(world, attrs), chunks)
}

/// The audio listener for `camera`: its position (voxel units), its forward and its up. The
/// mixer derives the listener's right as `forward x up`, which is the camera's right
/// (`forward x Z`), so a source on the right of the screen plays on the right channel.
pub fn listener(camera: &Camera) -> Listener {
    let (forward, _, up) = camera.basis();
    Listener {
        position: camera.position.to_array(),
        forward: forward.to_array(),
        up: up.to_array(),
    }
}

/// The rubble a collapsing region leaves when `R` has the fly-through's logic remove
/// unsupported regions instead of dropping them. The cave never generates it.
pub use logic::RUBBLE;

/// The cave's table: air, rock, ore, the indestructible shell (at the ids `cave` uses), and
/// [`RUBBLE`]. It lives here rather than in `cave.rs` so that file stays free of floating point.
pub fn cave_materials() -> MaterialTable {
    let mut t = MaterialTable::new();
    let row = |name: &str, albedo: [f32; 3]| Material {
        albedo,
        ..Material::named(name)
    };
    assert_eq!(t.push(row("rock", [0.42, 0.38, 0.34])), cave::ROCK);
    assert_eq!(t.push(row("ore", [0.72, 0.45, 0.20])), cave::ORE);
    let mut shell = row("shell", [0.12, 0.12, 0.14]);
    shell.flags = MaterialFlags::INDESTRUCTIBLE;
    assert_eq!(t.push(shell), cave::SHELL);
    assert_eq!(t.push(row("rubble", [0.36, 0.33, 0.30])), RUBBLE);
    t
}

/// `table` (the cave's four rows after air) with the castle's seven rows appended at
/// `castle::MATERIAL_BASE` on, in palette order: stone, trim (darker stone for ribs, tracery
/// and merlons), slate, wood and iron, opaque; glass, translucent and tinted blue; and torch,
/// emitting a warm light. None is indestructible, so a dig into the castle is an ordinary edit.
pub fn castle_materials(table: &MaterialTable) -> MaterialTable {
    let mut t = table.clone();
    assert_eq!(
        t.len(),
        usize::from(castle::MATERIAL_BASE),
        "the castle's rows follow the cave's"
    );
    let row = |name: &str, albedo: [f32; 3]| Material {
        albedo,
        ..Material::named(name)
    };
    let rows = [
        row("castle stone", [0.55, 0.53, 0.50]),
        row("castle trim", [0.36, 0.35, 0.34]),
        row("castle slate", [0.20, 0.22, 0.27]),
        row("castle wood", [0.40, 0.27, 0.15]),
        Material {
            roughness: 0.5,
            ..row("castle iron", [0.18, 0.18, 0.20])
        },
        Material {
            flags: MaterialFlags::TRANSLUCENT,
            opacity: 0.6,
            ..row("castle glass", [0.25, 0.35, 0.85])
        },
        Material {
            emission: [3.0, 1.7, 0.6],
            ..row("castle torch", [1.0, 0.7, 0.3])
        },
    ];
    for (i, m) in (castle::STONE..=castle::TORCH).zip(rows) {
        assert_eq!(t.push(m), castle::material(i));
    }
    t
}

/// The ore's emission under `--emissive-ore on`: an orange glow, radiance per channel.
pub const ORE_EMISSION: [f32; 3] = [1.6, 0.9, 0.35];

/// `table` with the ore row emitting [`ORE_EMISSION`]: the fly-through's table unless
/// `--emissive-ore off`.
pub fn with_emissive_ore(table: &MaterialTable) -> MaterialTable {
    let mut t = MaterialTable::new();
    for (id, row) in table.rows().iter().enumerate().skip(1) {
        let mut m = row.clone();
        if id == usize::from(cave::ORE) {
            m.emission = ORE_EMISSION;
        }
        assert_eq!(usize::from(t.push(m)), id);
    }
    t
}

/// Rock's procedural detail in every look but `--look plain`: a 40 cm feature scale (4 voxels)
/// at strength 0.1.
pub const ROCK_DETAIL: [f32; 2] = [4.0, 0.1];

/// [`cave_materials`] with rock's detail set to [`ROCK_DETAIL`]: the table the fly-through
/// uses unless `--look plain`. Every other row keeps no detail.
pub fn look_materials() -> MaterialTable {
    look_materials_with(ROCK_DETAIL)
}

/// [`cave_materials`] with rock's detail set to `rock_detail` (feature scale in voxels,
/// strength): the table under `--rock-detail`. Every other row keeps no detail.
pub fn look_materials_with(rock_detail: [f32; 2]) -> MaterialTable {
    let base = cave_materials();
    let mut t = MaterialTable::new();
    for (id, row) in base.rows().iter().enumerate().skip(1) {
        let mut row = row.clone();
        if id == usize::from(cave::ROCK) {
            row.detail = rock_detail;
        }
        t.push(row);
    }
    t
}

/// FNV-1a 64 over the voxels of the resident chunks `coords`, in the given order: each as its
/// coordinate (three little-endian `i32`), then its bricks in slot order with voxels x-fastest
/// as little-endian `u16`, a `Uniform` brick expanded. `None` if one is not resident.
pub fn chunks_hash(world: &World, coords: &[ChunkCoord]) -> Option<u64> {
    let mut h = Fnv1a::default();
    for c in coords {
        for v in c.0 {
            h.write(&v.to_le_bytes());
        }
        let chunk = world.store().get(*c)?;
        let chunk = chunk
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for b in chunk.bricks() {
            for i in 0..BRICK_VOXELS {
                h.write(&b.get(i).to_le_bytes());
            }
        }
    }
    Some(h.finish())
}

impl CaveSource {
    /// The cave's generation version, written into a save's header. A save made under
    /// another version names other voxels and does not replay.
    pub const VERSION: u32 = 1;
}

/// Where `scripts/build-guests.sh` places the logic component, relative to the workspace root.
pub const DEFAULT_COMPONENT: &str = "target/guests/flythrough-logic.wasm";

/// Why `--logic wasm` is refused by a build without the `wasm` feature.
pub const WASM_NEEDS_FEATURE: &str =
    "--logic wasm needs a build with --features wasm (cargo run -p flythrough --features wasm)";

/// Which build of [`DigLogic`] the server runs: `--logic native` or `--logic wasm[=path]`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum LogicChoice {
    /// [`DigLogic`] linked into the binary.
    #[default]
    Native,
    /// The `flythrough-logic` component at this path, hosted by `vxe-script-wasm`.
    Wasm(PathBuf),
}

impl LogicChoice {
    /// Parses `native`, `wasm` (the component at [`DEFAULT_COMPONENT`]) or `wasm=PATH`.
    pub fn parse(v: &str) -> Result<Self, String> {
        match v.split_once('=') {
            None if v == "native" => Ok(Self::Native),
            None if v == "wasm" => Ok(Self::Wasm(DEFAULT_COMPONENT.into())),
            Some(("wasm", path)) if !path.is_empty() => Ok(Self::Wasm(path.into())),
            _ => Err(format!("--logic takes native, wasm or wasm=PATH, not {v}")),
        }
    }

    /// A fresh logic of this choice. For [`LogicChoice::Wasm`], loads and compiles the
    /// component under the default `WasmConfig`; fails with the load error, or with
    /// [`WASM_NEEDS_FEATURE`] in a build without the `wasm` feature.
    pub fn load(&self) -> Result<Box<dyn Logic>, String> {
        match self {
            Self::Native => Ok(Box::new(DigLogic::default())),
            Self::Wasm(path) => load_component(path),
        }
    }
}

impl std::fmt::Display for LogicChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Native => write!(f, "native"),
            Self::Wasm(path) => write!(f, "wasm ({})", path.display()),
        }
    }
}

#[cfg(feature = "wasm")]
fn load_component(path: &Path) -> Result<Box<dyn Logic>, String> {
    let config = vxe_script_wasm::WasmConfig::default();
    match vxe_script_wasm::WasmLogic::load(path, &config) {
        Ok(logic) => Ok(Box::new(logic)),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(not(feature = "wasm"))]
fn load_component(_path: &Path) -> Result<Box<dyn Logic>, String> {
    Err(WASM_NEEDS_FEATURE.to_owned())
}

/// How [`replay_file`] rebuilds a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Applies the op log.
    Ops,
    /// Re-simulates the input log through a fresh [`DigLogic`], with the op log for the
    /// boundaries of directly submitted ops.
    Inputs,
}

/// Why a save did not replay.
#[derive(Debug)]
pub enum ReplayError {
    Io(std::io::Error),
    Wire(WireError),
    /// The save was written by another [`CaveSource::VERSION`].
    SourceVersion {
        file: u32,
        ours: u32,
    },
    /// The save's extent is not the one the caller replays (the bounded level's, for the
    /// binary); checked before anything is generated.
    Extent {
        file: ChunkExtent,
    },
}

impl std::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Wire(e) => write!(f, "{e}"),
            Self::SourceVersion { file, ours } => {
                write!(f, "saved with cave version {file}, this build is {ours}")
            }
            Self::Extent { file } => write!(
                f,
                "saved over chunks {:?}..{:?}, not the level's extent",
                file.min.0, file.max.0
            ),
        }
    }
}

impl std::error::Error for ReplayError {}

/// Reads the save at `path` and rebuilds its world (see [`replay_log`]) with
/// [`cave_materials`], refusing a save whose extent is not `extent`. Returns the world and
/// its voxel hash.
pub fn replay_file(
    path: impl AsRef<std::path::Path>,
    mode: Mode,
    extent: ChunkExtent,
) -> Result<(World, u64), ReplayError> {
    replay_file_with(path, mode, extent, Box::new(DigLogic::default()))
}

/// [`replay_file`] with `logic` re-simulating the inputs under [`Mode::Inputs`] (unused under
/// [`Mode::Ops`]): a fresh [`DigLogic`], native or hosted.
pub fn replay_file_with(
    path: impl AsRef<std::path::Path>,
    mode: Mode,
    extent: ChunkExtent,
    logic: Box<dyn Logic>,
) -> Result<(World, u64), ReplayError> {
    let file = read_log_file(path)?;
    let (world, _) = replay_log_with(&file, mode, extent, &cave_materials(), logic)?;
    let hash = world.voxel_hash();
    Ok((world, hash))
}

/// Reads and decodes the save at `path`.
pub fn read_log_file(path: impl AsRef<std::path::Path>) -> Result<LogFile, ReplayError> {
    let mut f = std::fs::File::open(path).map_err(ReplayError::Io)?;
    LogFile::read_from(&mut f).map_err(ReplayError::Wire)
}

/// Rebuilds a save's world: the bounded cave of its seed over its extent, which must equal
/// `extent` (so a corrupt header cannot ask for an unbounded world), generated with `table`,
/// then its op log replayed ([`Mode::Ops`]) or its input log re-simulated through
/// a fresh [`DigLogic`] seeded with the save's seed ([`Mode::Inputs`]). Returns the world and
/// the op log it was rebuilt from: the save's, or the one re-simulation produced.
pub fn replay_log(
    file: &LogFile,
    mode: Mode,
    extent: ChunkExtent,
    table: &MaterialTable,
) -> Result<(World, OpLog), ReplayError> {
    replay_log_with(file, mode, extent, table, Box::new(DigLogic::default()))
}

/// [`replay_log`] with `logic` re-simulating the inputs under [`Mode::Inputs`] (unused under
/// [`Mode::Ops`]): a fresh [`DigLogic`], native or hosted, initialised with the save's seed.
pub fn replay_log_with(
    file: &LogFile,
    mode: Mode,
    extent: ChunkExtent,
    table: &MaterialTable,
    logic: Box<dyn Logic>,
) -> Result<(World, OpLog), ReplayError> {
    replay_log_on(file, mode, extent, table, logic, false)
}

/// [`replay_log_with`] on the cave generated with the `--vault` room when `vault` is set. The
/// save's header does not record the room (its placement at generation is not an op), so a
/// session saved with `--vault` replays to its world only with `vault` set.
pub fn replay_log_on(
    file: &LogFile,
    mode: Mode,
    extent: ChunkExtent,
    table: &MaterialTable,
    logic: Box<dyn Logic>,
    vault: bool,
) -> Result<(World, OpLog), ReplayError> {
    if file.extent != extent {
        return Err(ReplayError::Extent { file: file.extent });
    }
    if file.source_version != CaveSource::VERSION {
        return Err(ReplayError::SourceVersion {
            file: file.source_version,
            ours: CaveSource::VERSION,
        });
    }
    let source = CaveSource::new(file.seed, file.extent);
    let world = if vault {
        World::generate(file.extent, &source.with_vault(), table)
    } else {
        World::generate(file.extent, &source, table)
    };
    match mode {
        Mode::Ops => {
            world.replay(&file.ops, &file.prefabs);
            Ok((world, file.ops.clone()))
        }
        Mode::Inputs => {
            let world = Arc::new(world);
            let ops = Server::resimulate(
                world.clone(),
                logic,
                file.seed,
                &file.prefabs,
                &file.ops,
                &file.inputs,
            );
            let world = Arc::into_inner(world).expect("the re-simulation's server is dropped");
            Ok((world, ops))
        }
    }
}

/// RAM a resident full chunk holds: its slot array and the class boxes of its `Full` bricks.
pub fn chunk_ram(chunk: &Chunk) -> u64 {
    let payload: usize = chunk.bricks().iter().map(Brick::payload_bytes).sum();
    (size_of::<Chunk>() + payload) as u64
}

/// RAM a LOD-only chunk holds: its 73 mip-brick slots and the class boxes of its `Full` ones.
pub fn mips_ram(mips: &ChunkMips) -> u64 {
    let slots: usize = vxe_world::MIP_BRICKS.iter().sum();
    let payload: usize = (1..=vxe_world::MIP_LEVELS)
        .flat_map(|n| mips.level(n))
        .map(Brick::payload_bytes)
        .sum();
    (size_of::<ChunkMips>() + slots * size_of::<Brick>() + payload) as u64
}

/// The `p`-th percentile (0 to 100) of `v` by nearest rank; `NaN` when empty. Sorts `v`.
pub fn percentile(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(f64::total_cmp);
    let rank = ((p / 100.0) * v.len() as f64).ceil() as usize;
    v[rank.clamp(1, v.len()) - 1]
}

/// Scroll lines banked into whole steps: smooth or touchpad scrolling delivers a fraction of
/// a line per frame, which rounding per frame would turn into no step at all.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrollSteps {
    carry: f32,
}

impl ScrollSteps {
    /// Adds a frame's scroll in lines and returns the whole steps banked, rounded toward
    /// zero; the fraction stays for the next frame.
    pub fn add(&mut self, delta: f32) -> i64 {
        self.carry += delta;
        let steps = self.carry.trunc();
        self.carry -= steps;
        steps as i64
    }

    /// The fraction of a step banked.
    pub fn carry(&self) -> f32 {
        self.carry
    }

    /// Drops the banked fraction.
    pub fn clear(&mut self) {
        self.carry = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_steps_bank_fractions_and_step_toward_zero() {
        let mut s = ScrollSteps::default();
        let quarters: Vec<i64> = [0.25; 4].iter().map(|&d| s.add(d)).collect();
        assert_eq!(quarters, [0, 0, 0, 1]);
        assert_eq!(s.carry(), 0.0);
        assert_eq!([s.add(0.3), s.add(-0.3)], [0, 0]);
        assert_eq!(s.add(2.0), 2);
        let mut s = ScrollSteps::default();
        assert_eq!([s.add(-0.6), s.add(-0.6)], [0, -1]);
        assert!((s.carry() + 0.2).abs() < 1e-6, "carry {}", s.carry());
    }

    #[test]
    fn look_materials_puts_the_override_on_rock_only() {
        let t = look_materials_with([8.0, 0.75]);
        let base = cave_materials();
        assert_eq!(t.rows().len(), base.rows().len());
        for (id, row) in t.rows().iter().enumerate() {
            let want = if id == usize::from(cave::ROCK) {
                [8.0, 0.75]
            } else {
                [0.0, 0.0]
            };
            assert_eq!(row.detail, want, "row {id} ({})", row.name);
            assert_eq!(row.albedo, base.rows()[id].albedo, "row {id}");
        }
        assert_eq!(
            look_materials().get(cave::ROCK).map(|r| r.detail),
            Some(ROCK_DETAIL)
        );
    }

    #[test]
    fn percentiles_by_nearest_rank() {
        let mut v: Vec<f64> = (1..=100).rev().map(f64::from).collect();
        assert_eq!(percentile(&mut v, 50.0), 50.0);
        assert_eq!(percentile(&mut v, 99.0), 99.0);
        assert_eq!(percentile(&mut v, 100.0), 100.0);
        assert_eq!(percentile(&mut [3.0], 95.0), 3.0);
        assert!(percentile(&mut [], 50.0).is_nan());
    }

    #[test]
    fn logic_choice_parses_native_wasm_and_a_path() {
        assert_eq!(LogicChoice::parse("native"), Ok(LogicChoice::Native));
        assert_eq!(
            LogicChoice::parse("wasm"),
            Ok(LogicChoice::Wasm(DEFAULT_COMPONENT.into()))
        );
        assert_eq!(
            LogicChoice::parse("wasm=a/b.wasm"),
            Ok(LogicChoice::Wasm("a/b.wasm".into()))
        );
        for bad in ["", "wasm=", "lua", "native=x"] {
            assert!(LogicChoice::parse(bad).is_err(), "{bad:?}");
        }
    }

    /// Without the `wasm` feature a hosted logic is refused with a message naming it.
    #[cfg(not(feature = "wasm"))]
    #[test]
    fn a_build_without_the_wasm_feature_refuses_the_component() {
        let e = LogicChoice::Wasm(DEFAULT_COMPONENT.into())
            .load()
            .err()
            .expect("refused");
        assert!(e.contains("--features wasm"), "{e}");
        assert!(LogicChoice::Native.load().is_ok());
    }

    #[test]
    fn the_streamed_mirror_reserves_records_for_the_residency_ball_over_the_restore_line() {
        assert_eq!(vxe_world::chunks_in_ball(25), 65_267);
        let n = stream_record_chunks(RESIDENCY);
        assert_eq!(n, 93_239);
        assert_eq!(n * 2624, 244_659_136);
        // The smallest count whose occupancy at the full ball stays at or under 0.7.
        assert!(65_267 * 10 <= n * 7 && 65_267 * 10 > (n - 1) * 7);
        assert!((PRESSURE_LOW - 0.7).abs() < f32::EPSILON);
    }

    #[test]
    fn the_default_window_holds_the_default_residency() {
        assert_eq!(window_side(RESIDENCY), 51);
    }
}
