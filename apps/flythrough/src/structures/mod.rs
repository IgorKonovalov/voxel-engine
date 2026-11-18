//! `.vox` structures placed in the cave (Plan 0083): `--vox PATH[@x,y,z][*k]` loads a
//! MagicaVoxel scene, its palette resolved to rows before the world is generated (ADR-0065),
//! and places it through a `StampedSource` over the cave, its box carved clear first unless
//! `--vox-clear off`. The library is `assets/structures/<name>/<name>.vox` ([`library`]),
//! placed from the belt as logged stamps ([`ops`]); [`words`] are its rows in the chooser
//! and the belt.

pub mod library;
pub mod words;

pub use library::*;

use std::path::{Path, PathBuf};

use vxe_assets::{
    PaletteRows, SceneOptions, Slot, VoxFile, VoxMap, VoxMaterial, VoxMaterialKind, VoxScene,
    VoxText, decode_frame, resolve_existing,
};
use vxe_world::{
    AIR, Brick, Chunk, ChunkCoord, MaterialTable, StampedSource, VoxelCoord, WorldSource,
};

/// The structures library.
pub const LIBRARY: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/structures");

/// Voxels of clear air around a structure's box on its sides and above it: 1 m.
pub const CLEAR_MARGIN: i32 = 10;

/// How far ahead of the spawn, along +x (the start camera's heading), a structure's box
/// starts, and how far below the spawn its base lies, in voxels.
pub const AHEAD: i32 = 30;
pub const BELOW: i32 = 16;

/// `--vox`'s value: the file, where its box's minimum corner goes (`None`: ahead of the
/// spawn) and its scale.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VoxArg {
    pub path: PathBuf,
    pub at: Option<[i32; 3]>,
    pub scale: u32,
}

impl VoxArg {
    /// Parses `PATH[@x,y,z][*k]`.
    pub fn parse(text: &str) -> Result<Self, String> {
        let (rest, scale) = match text.rsplit_once('*') {
            Some((r, k)) => {
                let k: u32 = k
                    .parse()
                    .map_err(|_| format!("`*{k}`: a scale is a whole number from 1 to 4"))?;
                if !(1..=vxe_assets::MAX_SCALE).contains(&k) {
                    return Err(format!(
                        "scale {k} is outside 1..={}",
                        vxe_assets::MAX_SCALE
                    ));
                }
                (r, k)
            }
            None => (text, 1),
        };
        let (path, at) = match rest.rsplit_once('@') {
            Some((p, at)) => {
                let v: Vec<i32> = at
                    .split(',')
                    .map(|c| {
                        c.trim()
                            .parse()
                            .map_err(|_| format!("`@{at}`: three integers"))
                    })
                    .collect::<Result<_, _>>()?;
                let [x, y, z] = v[..] else {
                    return Err(format!("`@{at}`: three integers x,y,z"));
                };
                (p, Some([x, y, z]))
            }
            None => (rest, None),
        };
        if path.is_empty() {
            return Err("a path".into());
        }
        Ok(Self {
            path: path.into(),
            at,
            scale,
        })
    }
}

/// The `.vox` at `path`, parsed.
pub fn read(path: &Path) -> Result<VoxFile, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    VoxFile::parse(&bytes).map_err(|e| format!("{}: {e}", path.display()))
}

/// A structure's own decisions before its palette: index 0 (the empty cells) leaves the world
/// alone.
fn explicit() -> VoxMap {
    let mut m = VoxMap::new();
    m.set(0, Slot::Skip);
    m
}

/// Adds a row to `table` for each palette entry of `file` (ADR-0065): the rows added.
pub fn add_rows(file: &VoxFile, table: &mut MaterialTable) -> usize {
    let mut rows = PaletteRows::new(table, vxe_assets::DEFAULT_EMISSION_FACTOR);
    rows.resolve(file, &explicit());
    rows.added()
}

/// `file`'s scene at `scale`, each palette entry mapped to its row in `table` (added by
/// [`add_rows`]).
pub fn scene(file: &VoxFile, table: &MaterialTable, scale: u32) -> Result<VoxScene, String> {
    let map = resolve_existing(table, file, &explicit());
    VoxScene::load(
        file,
        &map,
        SceneOptions {
            scale,
            ..SceneOptions::default()
        },
    )
    .map_err(|e| e.to_string())
}

/// The offset that puts `scene`'s box with its minimum corner at `at`, or, with none, ahead of
/// `spawn` along +x by [`AHEAD`], centred on it in y, its base [`BELOW`] under it.
pub fn offset(scene: &VoxScene, spawn: VoxelCoord, at: Option<[i32; 3]>) -> [i32; 3] {
    let (lo, _) = scene.bounds;
    let size = scene.size().map(|v| v as i32);
    let s = spawn.0;
    let want = at.unwrap_or([s[0] + AHEAD, s[1] - size[1] / 2, s[2] - BELOW]);
    [0, 1, 2].map(|k| want[k] - lo[k])
}

/// A source whose voxels in an inclusive box are air, `base`'s elsewhere.
#[derive(Clone, Debug)]
pub struct ClearedSource<S> {
    pub base: S,
    pub lo: [i32; 3],
    pub hi: [i32; 3],
}

impl<S: WorldSource> WorldSource for ClearedSource<S> {
    fn chunk(&self, coord: ChunkCoord, out: &mut Chunk) {
        self.base.chunk(coord, out);
        let c = coord.min_voxel().0;
        let lo = [0, 1, 2].map(|k| self.lo[k].max(c[k]));
        let hi = [0, 1, 2].map(|k| self.hi[k].min(c[k] + 63));
        if (0..3).any(|k| lo[k] > hi[k]) {
            return;
        }
        for slot in 0..vxe_world::CHUNK_BRICKS {
            let b = vxe_world::brick_of_slot(slot);
            let bmin = [0, 1, 2].map(|k| c[k] + b[k] * 8);
            let inside = |p: [i32; 3]| (0..3).all(|k| lo[k] <= p[k] && p[k] <= hi[k]);
            if (0..3).any(|k| bmin[k] + 7 < lo[k] || bmin[k] > hi[k]) {
                continue;
            }
            let old = out.brick(slot).clone();
            let brick = Brick::from_fn(|l| {
                let p = [0, 1, 2].map(|k| bmin[k] + l[k]);
                if inside(p) {
                    AIR
                } else {
                    old.get(vxe_world::index_in_brick(l))
                }
            });
            out.set_brick(slot, brick);
        }
    }
}

/// The inclusive box [`source`] carves for `scene` turned `quarters` quarter turns and moved
/// by `offset` ([`VoxScene::placed_at`]): the turned box with [`CLEAR_MARGIN`] on its sides
/// and above. `None` for a scene with no placement.
pub fn clear_box(scene: &VoxScene, offset: [i32; 3], quarters: u8) -> Option<([i32; 3], [i32; 3])> {
    if scene.placements.is_empty() {
        return None;
    }
    let (lo, hi) = scene.turned_bounds(quarters);
    let m = CLEAR_MARGIN;
    let lo = [lo[0] - m, lo[1] - m, lo[2]];
    let hi = [hi[0] + m, hi[1] + m, hi[2] + m];
    Some((
        [0, 1, 2].map(|k| lo[k] + offset[k]),
        [0, 1, 2].map(|k| hi[k] + offset[k]),
    ))
}

/// A scene as ops ([`ops`]): `--vox-op`'s, submitted on the run's first tick, and a belt
/// placement's, submitted at the click.
#[derive(Clone, Debug)]
pub struct VoxOps {
    /// The prefabs the stamps name, the air box's first when there is one, registered on the
    /// server and the client.
    pub prefabs: Vec<std::sync::Arc<vxe_world::Prefab>>,
    /// The air box's stamp when clear is on, then one stamp per placement.
    pub ops: Vec<vxe_world::Op>,
}

/// `scene` turned `quarters` quarter turns and moved by `offset` as ops: when `clear`, a stamp
/// of written air the size of [`clear_box`] at its corner (a stamp, not a carve: a stamp past
/// residency finishes as the chunks arrive, ADR-0066), then a stamp per placement in order,
/// as [`source`] writes them.
pub fn ops(scene: &VoxScene, offset: [i32; 3], quarters: u8, clear: bool) -> VoxOps {
    use vxe_world::{Op, Prefab, Rot, VoxelCoord};
    let mut ops = Vec::new();
    let mut prefabs = Vec::new();
    if clear && let Some((lo, hi)) = clear_box(scene, offset, quarters) {
        let size = [0, 1, 2].map(|k| (hi[k] - lo[k] + 1) as u32);
        let air = std::sync::Arc::new(Prefab::uniform(size, AIR));
        ops.push(Op::Stamp {
            prefab: air.id(),
            origin: VoxelCoord(lo),
            rot: Rot::IDENTITY,
        });
        prefabs.push(air);
    }
    for (p, o, r) in scene.placed_at(offset, quarters) {
        ops.push(Op::Stamp {
            prefab: p.id(),
            origin: o,
            rot: r,
        });
    }
    prefabs.extend(scene.prefabs.iter().cloned());
    VoxOps { prefabs, ops }
}

/// `scene` moved by `offset` over `base`, its box (with [`CLEAR_MARGIN`] on its sides and
/// above) carved to air first when `clear`.
pub fn source<S: WorldSource>(
    base: S,
    scene: &VoxScene,
    offset: [i32; 3],
    clear: bool,
) -> StampedSource<ClearedSource<S>> {
    // An empty box when nothing is carved.
    let (lo, hi) = clear_box(scene, offset, 0)
        .filter(|_| clear)
        .unwrap_or(([1, 1, 1], [0, 0, 0]));
    StampedSource::new(ClearedSource { base, lo, hi }, scene.placed_at(offset, 0))
}

/// The chapel's glass and lamp palette indices in `chapel.voxtext`.
pub const CHAPEL_GLASS: u8 = 3;
pub const CHAPEL_LAMP: u8 = 4;

/// The `_r` byte of the chapel's mirrored transept: y negated, a reflection.
pub const CHAPEL_MIRROR: u8 = 4 | 32;

/// `chapel.vox` from `chapel.voxtext`: the text's models (nave, transept, tower, scaffold),
/// the glass entry `_glass` and the lamp entry `_emit`, the transept placed again through a
/// reflected frame on the nave's other side, and the scaffold on a hidden layer `scaffold`.
pub fn chapel_vox(voxtext: &str) -> Result<VoxFile, String> {
    let vt = VoxText::parse(voxtext).map_err(|e| e.to_string())?;
    let mut file = vt.to_vox_file();
    file.materials[usize::from(CHAPEL_GLASS)] = VoxMaterial {
        kind: VoxMaterialKind::Glass {
            trans: 0.6,
            rough: 0.1,
        },
    };
    file.materials[usize::from(CHAPEL_LAMP)] = VoxMaterial {
        kind: VoxMaterialKind::Emit { emit: 1.0, flux: 1 },
    };
    // `to_vox_file` writes model k under transform 2 + 2k over shape 3 + 2k, under group 1.
    let (transept, scaffold) = (2 + 2, 2 + 2 * 3);
    let Some(vxe_assets::Node::Transform { translation, .. }) = file.nodes.get(&transept).cloned()
    else {
        return Err("the chapel has no transept".into());
    };
    let mirror = 2 + 2 * file.models.len() as u32;
    let (rot, _) = decode_frame(CHAPEL_MIRROR).map_err(|e| e.to_string())?;
    file.nodes.insert(
        mirror,
        vxe_assets::Node::Transform {
            child: transept + 1,
            translation: [translation[0], -translation[1], translation[2]],
            rot,
        },
    );
    file.reflected.insert(mirror, CHAPEL_MIRROR);
    file.names.insert(mirror, "transept mirrored".into());
    if let Some(vxe_assets::Node::Group { children }) = file.nodes.get_mut(&1) {
        children.push(mirror);
    }
    file.node_layers.insert(scaffold, 1);
    file.layers.insert(0, vxe_assets::VoxLayer::default());
    file.layers.insert(
        1,
        vxe_assets::VoxLayer {
            name: "scaffold".into(),
            hidden: true,
        },
    );
    Ok(file)
}
