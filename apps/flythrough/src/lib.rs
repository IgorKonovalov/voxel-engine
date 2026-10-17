//! The fly-through's reusable parts: the cave world source its binary, tests and examples
//! share, its server logic, save replay, the streaming defaults, and the small measurement
//! helpers its readings use. The engine ships no terrain; this cave is the testbed's own.

pub mod cave;
pub mod logic;

use std::sync::Arc;

use vxe_net::Server;
use vxe_world::hash::Fnv1a;
use vxe_world::{
    BRICK_VOXELS, Chunk, ChunkCoord, ChunkMips, LogFile, Material, MaterialFlags, MaterialTable,
    OpLog, Residency, WireError, World,
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

/// The cave's table: air, rock, ore, and the indestructible shell, at the ids `cave` uses.
/// It lives here rather than in `cave.rs` so that file stays free of floating point.
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
    t
}

/// Rock's procedural detail under `--look`: a 40 cm feature scale (4 voxels) at strength 0.3.
pub const ROCK_DETAIL: [f32; 2] = [4.0, 0.3];

/// [`cave_materials`] with rock's detail set to [`ROCK_DETAIL`]: the table the fly-through
/// uses under `--look`. Every other row keeps no detail.
pub fn look_materials() -> MaterialTable {
    look_materials_with(ROCK_DETAIL)
}

/// [`cave_materials`] with rock's detail set to `rock_detail` (feature scale in voxels,
/// strength): the table under `--look --rock-detail`. Every other row keeps no detail.
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
}

impl std::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Wire(e) => write!(f, "{e}"),
            Self::SourceVersion { file, ours } => {
                write!(f, "saved with cave version {file}, this build is {ours}")
            }
        }
    }
}

impl std::error::Error for ReplayError {}

/// Reads the save at `path` and rebuilds its world (see [`replay_log`]) with
/// [`cave_materials`]. Returns the world and its voxel hash.
pub fn replay_file(
    path: impl AsRef<std::path::Path>,
    mode: Mode,
) -> Result<(World, u64), ReplayError> {
    let file = read_log_file(path)?;
    let (world, _) = replay_log(&file, mode, &cave_materials())?;
    let hash = world.voxel_hash();
    Ok((world, hash))
}

/// Reads and decodes the save at `path`.
pub fn read_log_file(path: impl AsRef<std::path::Path>) -> Result<LogFile, ReplayError> {
    let mut f = std::fs::File::open(path).map_err(ReplayError::Io)?;
    LogFile::read_from(&mut f).map_err(ReplayError::Wire)
}

/// Rebuilds a save's world: the bounded cave of its seed over its extent, generated with
/// `table`, then its op log replayed ([`Mode::Ops`]) or its input log re-simulated through
/// a fresh [`DigLogic`] seeded with the save's seed ([`Mode::Inputs`]). Returns the world and
/// the op log it was rebuilt from: the save's, or the one re-simulation produced.
pub fn replay_log(
    file: &LogFile,
    mode: Mode,
    table: &MaterialTable,
) -> Result<(World, OpLog), ReplayError> {
    if file.source_version != CaveSource::VERSION {
        return Err(ReplayError::SourceVersion {
            file: file.source_version,
            ours: CaveSource::VERSION,
        });
    }
    let source = CaveSource::new(file.seed, file.extent);
    let world = World::generate(file.extent, &source, table);
    match mode {
        Mode::Ops => {
            world.replay(&file.ops);
            Ok((world, file.ops.clone()))
        }
        Mode::Inputs => {
            let world = Arc::new(world);
            let ops = Server::resimulate(
                world.clone(),
                Box::new(DigLogic),
                file.seed,
                &file.ops,
                &file.inputs,
            );
            let world = Arc::into_inner(world).expect("the re-simulation's server is dropped");
            Ok((world, ops))
        }
    }
}

/// RAM a resident full chunk holds: its slot array and the voxels of its `Full` bricks.
pub fn chunk_ram(chunk: &Chunk) -> u64 {
    let full = chunk.bricks().iter().filter(|b| b.is_full()).count();
    (size_of::<Chunk>() + full * BRICK_VOXELS * 2) as u64
}

/// RAM a LOD-only chunk holds: its 73 mip-brick slots and the cells of its `Full` ones.
pub fn mips_ram(mips: &ChunkMips) -> u64 {
    let slots: usize = vxe_world::MIP_BRICKS.iter().sum();
    (size_of::<ChunkMips>()
        + slots * size_of::<vxe_world::Brick>()
        + mips.full_bricks() * BRICK_VOXELS * 2) as u64
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn the_default_window_holds_the_default_residency() {
        assert_eq!(window_side(RESIDENCY), 51);
    }
}
