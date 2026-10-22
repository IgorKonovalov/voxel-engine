//! `--castle`: a gothic castle generated at startup as a multi-model MagicaVoxel scene, read
//! back through `vxe-assets`' parser and palette map, and stamped into the unbounded cave by a
//! [`StampedSource`] over a [`HallSource`].
//!
//! The castle is drawn in its own frame `(u, v, w)`, whose origin is the world voxel
//! [`ORIGIN`]: `u = x - 132`, `v = y + 268`, `w = z - 12`, with `w = 0` on the hall's floor.
//! Its box is `u` and `v` in `0..600` and `w` in `-100..400`, split into [`tiles`] of at most
//! 256 per axis (a `.vox` model's limit). Each tile is a model whose voxels are palette
//! indices: [`STONE`] to [`TORCH`] are the castle's materials, and [`EXPLICIT_AIR`] is air the
//! dungeon carves. Above the floor index 0 is "leave the world alone" (the hall is already
//! air); in the dungeon tiles too, so rock the dungeon does not touch stays.
//!
//! Every voxel's index is a pure integer function of its coordinate: a list of parts, each a
//! box and a rule, evaluated in order with the last part that claims a voxel winning. The file
//! lists tiles in [`tiles`] order and each tile's voxels with `x` fastest, so the bytes are
//! the same however many threads build the tiles. Nothing here uses floating point.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use vxe_assets::{Node, Slot, Unmapped, VoxFile, VoxMap, VoxModel};
use vxe_world::{MaterialId, Prefab, Rot, StampedSource, VoxelCoord};

use crate::cave::HallSource;

/// The world voxel at castle-local `(0, 0, 0)`.
pub const ORIGIN: [i32; 3] = [132, -268, 12];
/// The castle box's minimum corner, castle-local.
pub const BOX_LO: [i32; 3] = [0, 0, -100];
/// The castle box's exclusive maximum corner, castle-local.
pub const BOX_HI: [i32; 3] = [600, 600, 400];

/// Palette indices of the generated `.vox`.
pub const STONE: u8 = 1;
/// Darker stone for ribs, tracery and merlons.
pub const TRIM: u8 = 2;
pub const SLATE: u8 = 3;
pub const WOOD: u8 = 4;
pub const IRON: u8 = 5;
pub const GLASS: u8 = 6;
pub const TORCH: u8 = 7;
/// Air written explicitly: the dungeon's rooms, carved out of the plinth.
pub const EXPLICIT_AIR: u8 = 8;

/// The material ids the castle's rows take, appended to the cave's table by
/// `flythrough::castle_materials`: palette index `i` in `1..=7` maps to `MATERIAL_BASE + i - 1`.
pub const MATERIAL_BASE: MaterialId = 5;

/// The material id palette index `i` (`1..=7`) maps to.
pub const fn material(i: u8) -> MaterialId {
    MATERIAL_BASE + i as MaterialId - 1
}

/// The palette written into the `.vox`: index 1 to 8 as the castle's colours, the rest
/// MagicaVoxel's default.
const PALETTE_COLOURS: [[u8; 4]; 8] = [
    [140, 135, 128, 255], // stone
    [92, 89, 86, 255],    // trim
    [51, 56, 69, 255],    // slate
    [102, 69, 38, 255],   // wood
    [46, 46, 51, 255],    // iron
    [64, 90, 217, 255],   // glass
    [255, 179, 77, 255],  // torch
    [255, 0, 255, 255],   // explicit air, a colour no material has
];

/// One tile of the castle box: its castle-local minimum corner and its size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tile {
    pub lo: [i32; 3],
    pub size: [u32; 3],
}

impl Tile {
    /// Whether this is a dungeon tile (below the floor).
    pub fn dungeon(&self) -> bool {
        self.lo[2] < 0
    }

    /// Voxels in the tile.
    pub fn len(&self) -> usize {
        self.size.iter().map(|&s| s as usize).product()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The 27 tiles: `u` and `v` split at 256, 256, 88; `w` at the dungeon's 100 below the floor,
/// then 256 and 144 above it. Ordered by layer (dungeon, lower, upper), then `v`, then `u`.
pub fn tiles() -> Vec<Tile> {
    const UV: [(i32, u32); 3] = [(0, 256), (256, 256), (512, 88)];
    const W: [(i32, u32); 3] = [(-100, 100), (0, 256), (256, 144)];
    let mut out = Vec::with_capacity(27);
    for (w, sw) in W {
        for (v, sv) in UV {
            for (u, su) in UV {
                out.push(Tile {
                    lo: [u, v, w],
                    size: [su, sv, sw],
                });
            }
        }
    }
    out
}

/// A part of the castle: a castle-local box (minimum inclusive, maximum exclusive) and the
/// index its rule gives each voxel inside it, or `None` to leave the voxel to earlier parts.
struct Part {
    lo: [i32; 3],
    hi: [i32; 3],
    rule: Box<dyn Fn(i32, i32, i32) -> Option<u8> + Send + Sync>,
}

impl Part {
    fn new(
        lo: [i32; 3],
        hi: [i32; 3],
        rule: impl Fn(i32, i32, i32) -> Option<u8> + Send + Sync + 'static,
    ) -> Self {
        Self {
            lo,
            hi,
            rule: Box::new(rule),
        }
    }

    /// A box filled with one index.
    fn fill(lo: [i32; 3], hi: [i32; 3], index: u8) -> Self {
        Self::new(lo, hi, move |_, _, _| Some(index))
    }

    fn meets(&self, lo: [i32; 3], hi: [i32; 3]) -> bool {
        (0..3).all(|k| self.lo[k] < hi[k] && lo[k] < self.hi[k])
    }
}

/// `a * a + b * b` in `i64`.
fn d2(a: i32, b: i32) -> i64 {
    let (a, b) = (i64::from(a), i64::from(b));
    a * a + b * b
}

/// A two-centred pointed arch standing on `h = 0`: true inside the opening at `d` across the
/// span's centre and height `h`. The span is `2 * half`; each side is an arc of radius `2 *
/// half` centred on the springing line at the opposite jamb, so the apex rises `half * sqrt(3)`
/// above the springing line, which sits that far below `height`.
fn pointed_arch(d: i32, h: i32, half: i32, height: i32) -> bool {
    let spring = height - half * 1732 / 1000;
    if h < 0 || d <= -half || d >= half {
        return false;
    }
    if h < spring {
        return true;
    }
    let r2 = 4 * i64::from(half) * i64::from(half);
    d2(d + half, h - spring) < r2 && d2(d - half, h - spring) < r2
}

/// A merlon's pattern along a parapet: solid for the first 6 of every 12 voxels of `s`.
fn merlon(s: i32) -> bool {
    s.rem_euclid(12) < 6
}

/// A round tower's merlons: a checker of 6-voxel cells around the ring.
fn ring_merlon(du: i32, dv: i32) -> bool {
    (du.div_euclid(6) + dv.div_euclid(6)).rem_euclid(2) == 0
}

/// The gate-side curtain wall: outer face at `u = 60`, 20 thick, 12 m tall, merlons on its
/// outer 4 voxels with a wall walk behind them.
fn gate_wall() -> Vec<Part> {
    vec![
        Part::fill([60, 60, 0], [80, 540, 120], STONE),
        Part::new([60, 60, 120], [64, 540, 128], |_, v, _| {
            merlon(v - 60).then_some(TRIM)
        }),
    ]
}

/// A corner tower centred at `(cu, cv)`: radius 50, walls 15 thick, 20 m tall, four arrow
/// slits at two heights, a doorway toward the courtyard (`toward` is the courtyard's
/// direction on each axis), a trim cornice and a conical slate roof to `w = 280`.
fn corner_tower(cu: i32, cv: i32, toward: [i32; 2]) -> Vec<Part> {
    const R: i32 = 50;
    const INNER: i32 = 35;
    const TOP: i32 = 200;
    const ROOF_R: i32 = 56;
    const ROOF_TOP: i32 = 280;
    let lo = [cu - ROOF_R, cv - ROOF_R, 0];
    let hi = [cu + ROOF_R + 1, cv + ROOF_R + 1, ROOF_TOP];
    vec![
        Part::new(
            [cu - R, cv - R, 0],
            [cu + R, cv + R, TOP],
            move |u, v, w| {
                let (du, dv) = (u - cu, v - cv);
                let r2 = d2(du, dv);
                if r2 >= i64::from(R * R) {
                    return None;
                }
                if r2 < i64::from(INNER * INNER) {
                    return Some(0);
                }
                let slit = (60..80).contains(&w) || (130..150).contains(&w);
                if slit && (du.abs() <= 1 || dv.abs() <= 1) {
                    return Some(0);
                }
                // The doorway: on the courtyard's diagonal, 12 wide and 3 m tall.
                let (a, b) = (du * toward[0], dv * toward[1]);
                if a > 0 && b > 0 && (a - b).abs() < 9 && w < 30 {
                    return Some(0);
                }
                Some(STONE)
            },
        ),
        Part::new(lo, [hi[0], hi[1], TOP], move |u, v, w| {
            let r2 = d2(u - cu, v - cv);
            (w >= TOP - 4 && r2 >= i64::from(R * R) && r2 < 53 * 53).then_some(TRIM)
        }),
        Part::new([lo[0], lo[1], TOP], hi, move |u, v, w| {
            let r = ROOF_R * (ROOF_TOP - w) / (ROOF_TOP - TOP);
            let r2 = d2(u - cu, v - cv);
            let inner = (r - 4).max(0);
            (r2 <= i64::from(r * r) && (r < 4 || r2 >= i64::from(inner * inner))).then_some(SLATE)
        }),
    ]
}

/// The gatehouse on the `u = 60` face at `v = 300`: a block over the passage, two flanking
/// towers of radius 35 and 16 m, merlons on both, a pointed arch 40 wide and 60 tall through
/// the block and the wall, an iron portcullis half raised, and a torch on each side of the
/// arch.
fn gatehouse() -> Vec<Part> {
    const CV: i32 = 300;
    let mut parts = vec![
        Part::fill([45, 260, 0], [95, 340, 150], STONE),
        Part::new([45, 260, 150], [95, 340, 158], |u, v, _| {
            let rim = !(49..91).contains(&u) || !(264..336).contains(&v);
            (rim && merlon(u + v)).then_some(TRIM)
        }),
    ];
    for cv in [CV - 60, CV + 60] {
        let cu = 60;
        parts.push(Part::new(
            [cu - 35, cv - 35, 0],
            [cu + 35, cv + 35, 168],
            move |u, v, w| {
                let (du, dv) = (u - cu, v - cv);
                let r2 = d2(du, dv);
                if r2 >= 35 * 35 {
                    return None;
                }
                if w >= 160 {
                    return (r2 >= 30 * 30 && ring_merlon(du, dv)).then_some(TRIM);
                }
                if (80..100).contains(&w) && dv.abs() <= 1 && du < 0 {
                    return Some(0);
                }
                Some(STONE)
            },
        ));
    }
    // The passage through the block and the wall.
    parts.push(Part::new([40, 280, 0], [100, 320, 60], |_, v, w| {
        pointed_arch(v - CV, w, 20, 60).then_some(0)
    }));
    // The portcullis: 2-voxel iron bars on a 6-voxel grid, raised to half the arch.
    parts.push(Part::new([47, 280, 30], [49, 320, 60], |_, v, w| {
        let bar = (v - 280).rem_euclid(6) < 2 || w.rem_euclid(8) < 2;
        (pointed_arch(v - CV, w, 20, 60) && bar).then_some(IRON)
    }));
    for v in [275, 323] {
        parts.push(torch([43, v, 40]));
    }
    parts
}

/// A torch: a 2 x 2 x 3 block of torch material with its minimum corner at `lo`.
fn torch(lo: [i32; 3]) -> Part {
    Part::fill(lo, [lo[0] + 2, lo[1] + 2, lo[2] + 3], TORCH)
}

/// Every part of the castle, in evaluation order.
fn parts() -> Vec<Part> {
    let mut p = gate_wall();
    p.extend(corner_tower(60, 60, [1, 1]));
    p.extend(gatehouse());
    p
}

/// The index at castle-local `(u, v, w)` from `parts`, the last claim winning.
fn index_at(parts: &[&Part], u: i32, v: i32, w: i32) -> u8 {
    let mut idx = 0;
    for p in parts {
        if (p.lo[2]..p.hi[2]).contains(&w)
            && let Some(i) = (p.rule)(u, v, w)
        {
            idx = i;
        }
    }
    idx
}

/// The tile's model: its voxels with a non-zero index, `x` fastest, then `y`, then `z`.
fn tile_model(all: &[Part], tile: Tile) -> VoxModel {
    let hi = [0, 1, 2].map(|k| tile.lo[k] + tile.size[k] as i32);
    let parts: Vec<&Part> = all.iter().filter(|p| p.meets(tile.lo, hi)).collect();
    let [sx, sy, _] = tile.size.map(|s| s as usize);
    // The parts whose box covers each column, in evaluation order.
    let mut columns: Vec<Vec<&Part>> = Vec::with_capacity(sx * sy);
    for y in 0..sy as i32 {
        for x in 0..sx as i32 {
            let (u, v) = (tile.lo[0] + x, tile.lo[1] + y);
            columns.push(
                parts
                    .iter()
                    .copied()
                    .filter(|p| (p.lo[0]..p.hi[0]).contains(&u) && (p.lo[1]..p.hi[1]).contains(&v))
                    .collect(),
            );
        }
    }
    let mut voxels = Vec::new();
    for z in 0..tile.size[2] {
        let w = tile.lo[2] + z as i32;
        for y in 0..tile.size[1] {
            for x in 0..tile.size[0] {
                let col = &columns[x as usize + sx * y as usize];
                if col.is_empty() {
                    continue;
                }
                let i = index_at(col, tile.lo[0] + x as i32, tile.lo[1] + y as i32, w);
                if i != 0 {
                    voxels.push([x as u8, y as u8, z as u8, i]);
                }
            }
        }
    }
    VoxModel {
        size: tile.size,
        voxels,
    }
}

/// The castle as a `.vox` scene: one model per tile that lists any voxel, in [`tiles`] order,
/// under a root transform and a group, each model behind its own transform whose `_t` puts
/// the model's centre (`lo + size / 2`) at its castle-local place. Tiles are built on up to
/// `threads` threads; the result does not depend on the count.
pub fn castle_vox_on(threads: usize) -> VoxFile {
    let parts = parts();
    let tiles = tiles();
    let next = AtomicUsize::new(0);
    let mut built: Vec<Option<VoxModel>> = vec![None; tiles.len()];
    std::thread::scope(|s| {
        let workers: Vec<_> = (0..threads.clamp(1, tiles.len()))
            .map(|_| {
                s.spawn(|| {
                    let mut out = Vec::new();
                    loop {
                        let k = next.fetch_add(1, Ordering::Relaxed);
                        let Some(&t) = tiles.get(k) else { break };
                        out.push((k, tile_model(&parts, t)));
                    }
                    out
                })
            })
            .collect();
        for h in workers {
            for (k, m) in h.join().expect("a castle tile worker panicked") {
                built[k] = Some(m);
            }
        }
    });
    let mut models = Vec::new();
    let mut placed = Vec::new();
    for (t, m) in tiles.iter().zip(built) {
        let m = m.expect("every tile is built");
        if !m.voxels.is_empty() {
            placed.push(*t);
            models.push(m);
        }
    }
    let mut nodes = std::collections::BTreeMap::new();
    nodes.insert(
        0,
        Node::Transform {
            child: 1,
            translation: [0; 3],
            rot: Rot::IDENTITY,
        },
    );
    nodes.insert(
        1,
        Node::Group {
            children: (0..models.len() as u32).map(|k| 2 + 2 * k).collect(),
        },
    );
    for (k, t) in placed.iter().enumerate() {
        let id = 2 + 2 * k as u32;
        nodes.insert(
            id,
            Node::Transform {
                child: id + 1,
                translation: [0, 1, 2].map(|a| t.lo[a] + (t.size[a] / 2) as i32),
                rot: Rot::IDENTITY,
            },
        );
        nodes.insert(
            id + 1,
            Node::Shape {
                models: vec![k as u32],
            },
        );
    }
    let mut palette = vxe_assets::DEFAULT_PALETTE;
    palette[1..=8].copy_from_slice(&PALETTE_COLOURS);
    VoxFile {
        version: 200,
        models,
        palette,
        nodes,
        imap_present: false,
    }
}

/// [`castle_vox_on`] on the machine's available threads.
pub fn castle_vox() -> VoxFile {
    castle_vox_on(std::thread::available_parallelism().map_or(1, usize::from))
}

/// How many voxels of each palette index the file's models list.
pub fn index_counts(file: &VoxFile) -> [u64; 256] {
    let mut n = [0u64; 256];
    for m in &file.models {
        for v in &m.voxels {
            n[usize::from(v[3])] += 1;
        }
    }
    n
}

/// The map for the tiles above the floor: index 0 leaves the hall's air alone, 1 to 7 are the
/// castle's materials. Index 8 is left unset, so an explicit air voxel above the floor is an
/// error.
pub fn above_map() -> VoxMap {
    let mut m = VoxMap::new();
    m.set(0, Slot::Skip);
    for i in STONE..=TORCH {
        m.set(i, Slot::Material(material(i)));
    }
    m
}

/// The map for the dungeon's tiles: [`above_map`] with index 8 as air.
pub fn dungeon_map() -> VoxMap {
    let mut m = above_map();
    m.set(EXPLICIT_AIR, Slot::Air);
    m
}

/// The castle's placements in the world: each placed model as a prefab through
/// [`dungeon_map`] when its box lies below the floor and [`above_map`] otherwise, at the
/// world corner [`ORIGIN`] plus the placement's minimum corner, in placement order.
pub fn castle_prefabs(file: &VoxFile) -> Result<Vec<(Arc<Prefab>, VoxelCoord, Rot)>, Unmapped> {
    let (above, dungeon) = (above_map(), dungeon_map());
    file.placements()
        .into_iter()
        .map(|p| {
            let m = &file.models[p.model];
            let corner = p.min_corner(p.rot.apply_size(m.size)).0;
            let map = if corner[2] < 0 { &dungeon } else { &above };
            let prefab = map.to_prefab(m)?;
            let at = VoxelCoord([0, 1, 2].map(|k| ORIGIN[k] + corner[k]));
            Ok((Arc::new(prefab), at, p.rot))
        })
        .collect()
}

/// The hall in the cave of `seed` with the castle's placements written over it. Each `Arc` is
/// moved in, so a prefab's unrotated copy is freed as its rotated one is made.
pub fn castle_source(
    seed: u64,
    placements: Vec<(Arc<Prefab>, VoxelCoord, Rot)>,
) -> StampedSource<HallSource> {
    StampedSource::new(HallSource::new(seed), placements)
}

/// The bytes the kept prefabs hold: per prefab of `n` voxels, `2 n` of material and
/// `8 * ceil(n / 64)` of mask.
pub fn prefab_bytes<'a>(prefabs: impl IntoIterator<Item = &'a Prefab>) -> u64 {
    prefabs
        .into_iter()
        .map(|p| {
            let n = p.len() as u64;
            2 * n + 8 * n.div_ceil(64)
        })
        .sum()
}
