//! The logic a session runs (native or a component) and the replay of a saved session's log
//! file through it.

use crate::cave::CaveSource;
use crate::logic::DigLogic;
use crate::scene::cave_materials;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use vxe_net::{Logic, Server};
use vxe_world::{ChunkExtent, LogFile, MaterialTable, OpLog, WireError, World};

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
