//! The support query: which solid regions of the coarse grid lost their connection to an
//! anchor (ADR-0008).
//!
//! A cell is load-bearing when it holds at least `solid_min` solid voxels. Candidates are the
//! load-bearing cells among an edit's changed cells and their six neighbours, visited in
//! ascending `(x, y, z)` order. Each candidate not yet settled starts a breadth-first flood over
//! load-bearing 6-neighbours in the order -x, +x, -y, +y, -z, +z. The flood ends as supported
//! when it reaches an anchor, a cell an earlier flood settled as supported, or a cell whose
//! chunk is unknown, or when it holds more than `max_region` cells; it ends as a region when it
//! runs out of cells. A chunk is unknown when the grid or the world lacks it and it lies inside
//! the world's extent, or anywhere in a world with an empty (unbounded) extent; unknown counts
//! as anchored. A chunk outside a non-empty extent is known air: it holds nothing up, so with
//! `boundary_anchors` off a piece touching the extent's faces can come loose. Every step is a
//! fixed order over integer data, so the output is canonical. Nothing is kept between calls.

use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{BuildHasherDefault, Hasher};

use crate::brick::Brick;
use crate::coarse::{CellChange, CellCoord, CoarseCell, CoarseGrid};
use crate::coords::{BRICK_EDGE, CHUNK_BRICKS, CHUNK_EDGE_BRICKS, ChunkCoord, VoxelCoord};
use crate::material::{AIR, MaterialId};
use crate::store::{World, read};

/// What holds a cell up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SupportParams {
    /// The fewest solid voxels a load-bearing cell holds. A cell below it is neither part of a
    /// region nor a path to an anchor.
    pub solid_min: u16,
    /// The most cells a flood visits before it gives up and calls its cells supported.
    pub max_region: u32,
    /// Whether the cells on the faces of the world's extent are anchors. Off, the chunks
    /// outside a non-empty extent read as air, so the faces hold nothing up; a world with an
    /// empty extent has no faces, and every chunk it lacks stays unknown (anchored).
    pub boundary_anchors: bool,
}

/// Cells the caller declares anchored, on top of the anchors every query knows (an
/// `INDESTRUCTIBLE` dominant material, the extent's faces when asked, a chunk inside the extent
/// (or anywhere, in an unbounded world) that the grid or the world lacks).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Anchors<'a> {
    pub explicit: &'a [CellCoord],
}

/// A connected set of load-bearing cells that reaches no anchor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Region {
    /// The region's cells, ascending.
    pub cells: Vec<CellCoord>,
    /// Solid voxels of the region's bricks per material, ascending by material.
    pub counts: Vec<(MaterialId, u32)>,
    /// Inclusive bounds of the region's solid voxels. A region whose bricks hold no solid voxel
    /// (possible only with `solid_min == 0`) reports the bounds of its cells.
    pub aabb: (VoxelCoord, VoxelCoord),
}

/// A multiplicative hash for sets and maps of cell coordinates. They are only probed, never
/// iterated into an output, so the hash affects speed and not results.
#[derive(Default)]
pub(crate) struct CellHasher(u64);

impl CellHasher {
    fn mix(&mut self, v: u64) {
        self.0 = (self.0.rotate_left(5) ^ v).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
}

impl Hasher for CellHasher {
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.mix(u64::from(b));
        }
    }

    fn write_i32(&mut self, i: i32) {
        self.mix(u64::from(i as u32));
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

impl std::hash::Hash for CellKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        for v in self.0.0 {
            state.write_i32(v);
        }
    }
}

/// A cell coordinate hashed one `i32` per axis, with no length prefix.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct CellKey(pub(crate) CellCoord);

pub(crate) type CellSet = HashSet<CellKey, BuildHasherDefault<CellHasher>>;
pub(crate) type CellMap<V> = HashMap<CellKey, V, BuildHasherDefault<CellHasher>>;

/// The six neighbours of `c` in flood order: -x, +x, -y, +y, -z, +z.
fn neighbours(c: CellCoord) -> [CellCoord; 6] {
    let [x, y, z] = c.0;
    [
        [x - 1, y, z],
        [x + 1, y, z],
        [x, y - 1, z],
        [x, y + 1, z],
        [x, y, z - 1],
        [x, y, z + 1],
    ]
    .map(CellCoord)
}

/// A cell as the flood sees it.
#[derive(Clone, Copy)]
enum Seen {
    /// In a chunk the grid or the world lacks, inside the extent or in an unbounded world: an
    /// anchor.
    Unknown,
    Cell(CoarseCell),
}

/// Every cell of a chunk outside a non-empty extent: air.
static OUTSIDE: [CoarseCell; CHUNK_BRICKS] = [CoarseCell {
    solid: 0,
    dominant: AIR,
}; CHUNK_BRICKS];

/// Cell lookups with the chunk resolved once: per chunk, the grid's cells when both the grid
/// and the world hold it, `OUTSIDE` for a chunk outside a non-empty extent, `None` (unknown)
/// otherwise.
struct Cells<'g> {
    grid: &'g CoarseGrid,
    world: &'g World,
    chunks: HashMap<ChunkCoord, Option<&'g [CoarseCell; CHUNK_BRICKS]>>,
    last: Option<(ChunkCoord, Option<&'g [CoarseCell; CHUNK_BRICKS]>)>,
}

impl<'g> Cells<'g> {
    fn new(world: &'g World, grid: &'g CoarseGrid) -> Self {
        Self {
            grid,
            world,
            chunks: HashMap::new(),
            last: None,
        }
    }

    fn get(&mut self, c: CellCoord) -> Seen {
        let (chunk, slot) = c.split();
        let cells = match self.last {
            Some((k, cells)) if k == chunk => cells,
            _ => {
                let (grid, world) = (self.grid, self.world);
                let cells = *self.chunks.entry(chunk).or_insert_with(|| {
                    let e = world.extent();
                    grid.chunk(chunk)
                        .filter(|_| world.store.contains(chunk))
                        .or_else(|| (!e.is_empty() && !e.contains(chunk)).then_some(&OUTSIDE))
                });
                self.last = Some((chunk, cells));
                cells
            }
        };
        cells.map_or(Seen::Unknown, |cells| Seen::Cell(cells[slot]))
    }
}

/// The anchor rules that need the cell itself: an indestructible dominant material, the
/// extent's faces, the caller's list (sorted, for a binary search).
struct AnchorRules<'a> {
    world: &'a World,
    /// Inclusive cell bounds of the extent when its faces anchor.
    faces: Option<([i32; 3], [i32; 3])>,
    explicit: Vec<CellCoord>,
}

impl<'a> AnchorRules<'a> {
    fn new(world: &'a World, params: &SupportParams, anchors: &Anchors<'_>) -> Self {
        let e = world.extent();
        let faces = (params.boundary_anchors && !e.is_empty()).then(|| {
            (
                e.min.0.map(|v| v * CHUNK_EDGE_BRICKS),
                e.max.0.map(|v| v * CHUNK_EDGE_BRICKS - 1),
            )
        });
        let mut explicit = anchors.explicit.to_vec();
        explicit.sort_unstable();
        explicit.dedup();
        Self {
            world,
            faces,
            explicit,
        }
    }

    fn holds(&self, c: CellCoord, cell: CoarseCell) -> bool {
        self.world.materials.is_indestructible(cell.dominant)
            || self
                .faces
                .is_some_and(|(lo, hi)| (0..3).any(|k| c.0[k] == lo[k] || c.0[k] == hi[k]))
            || self.explicit.binary_search(&c).is_ok()
    }
}

/// The unsupported regions after an edit whose grid update reported `changed`. See the module
/// documentation for the flood; regions come back in the order their first cell was visited.
pub fn find_unsupported(
    world: &World,
    grid: &CoarseGrid,
    params: &SupportParams,
    anchors: &Anchors<'_>,
    changed: &[CellChange],
) -> Vec<Region> {
    find_unsupported_with_stats(world, grid, params, anchors, changed).0
}

/// [`find_unsupported`] plus the cells the query looked at: the candidates after
/// deduplication (each changed cell and its six neighbours) plus every cell a flood took on,
/// its start included. Neighbours a flood looked up and passed over (not load-bearing) are
/// not counted.
pub fn find_unsupported_with_stats(
    world: &World,
    grid: &CoarseGrid,
    params: &SupportParams,
    anchors: &Anchors<'_>,
    changed: &[CellChange],
) -> (Vec<Region>, u64) {
    let mut cells = Cells::new(world, grid);
    let rules = AnchorRules::new(world, params, anchors);
    let cap = usize::try_from(params.max_region).unwrap_or(usize::MAX);

    let mut candidates: Vec<CellCoord> = Vec::with_capacity(changed.len() * 7);
    for ch in changed {
        let c = ch.cell();
        candidates.push(c);
        candidates.extend(neighbours(c));
    }
    candidates.sort_unstable();
    candidates.dedup();
    let mut steps = candidates.len() as u64;

    // Settled cells: `true` supported, `false` part of a returned region.
    let mut settled: CellMap<bool> = CellMap::default();
    let mut out = Vec::new();
    let mut flood: CellSet = CellSet::default();
    let mut order: Vec<CellCoord> = Vec::new();
    let mut queue: VecDeque<(CellCoord, CoarseCell)> = VecDeque::new();
    for start in candidates {
        if settled.contains_key(&CellKey(start)) {
            continue;
        }
        let Seen::Cell(cell) = cells.get(start) else {
            continue;
        };
        if cell.solid < params.solid_min {
            continue;
        }
        flood.clear();
        order.clear();
        queue.clear();
        flood.insert(CellKey(start));
        order.push(start);
        queue.push_back((start, cell));
        let mut supported = false;
        'flood: while let Some((c, cell)) = queue.pop_front() {
            if rules.holds(c, cell) {
                supported = true;
                break;
            }
            for n in neighbours(c) {
                let key = CellKey(n);
                if flood.contains(&key) {
                    continue;
                }
                if settled.get(&key) == Some(&true) {
                    supported = true;
                    break 'flood;
                }
                let Seen::Cell(ncell) = cells.get(n) else {
                    supported = true;
                    break 'flood;
                };
                if ncell.solid < params.solid_min {
                    continue;
                }
                flood.insert(key);
                order.push(n);
                if order.len() > cap {
                    supported = true;
                    break 'flood;
                }
                queue.push_back((n, ncell));
            }
        }
        steps += order.len() as u64;
        for &c in &order {
            settled.insert(CellKey(c), supported);
        }
        if !supported {
            let mut region = order.clone();
            region.sort_unstable();
            let (counts, aabb) = measure(world, &region);
            out.push(Region {
                cells: region,
                counts,
                aabb,
            });
        }
    }
    (out, steps)
}

/// Per-material solid voxel counts (ascending by material) and the inclusive bounds of the
/// solid voxels of the bricks of `cells` (ascending); the cells' own bounds when no voxel is
/// solid. Bricks in chunks that are not resident count as air.
fn measure(
    world: &World,
    cells: &[CellCoord],
) -> (Vec<(MaterialId, u32)>, (VoxelCoord, VoxelCoord)) {
    let mut counts: Vec<(MaterialId, u32)> = Vec::new();
    let mut add = |m: MaterialId, n: u32| match counts.iter_mut().find(|(k, _)| *k == m) {
        Some((_, c)) => *c += n,
        None => counts.push((m, n)),
    };
    let mut bounds: Option<([i32; 3], [i32; 3])> = None;
    let mut widen = |lo: [i32; 3], hi: [i32; 3]| {
        bounds = Some(match bounds {
            None => (lo, hi),
            Some((a, b)) => (
                [0, 1, 2].map(|k| a[k].min(lo[k])),
                [0, 1, 2].map(|k| b[k].max(hi[k])),
            ),
        });
    };
    let be = BRICK_EDGE;
    let mut i = 0;
    while i < cells.len() {
        let chunk_coord = cells[i].split().0;
        let run = cells[i..]
            .iter()
            .take_while(|c| c.split().0 == chunk_coord)
            .count();
        if let Some(chunk) = world.store.get(chunk_coord) {
            let chunk = read(&chunk);
            for &c in &cells[i..i + run] {
                let min = c.min_voxel().0;
                match chunk.brick(c.split().1) {
                    Brick::Uniform(AIR) => {}
                    Brick::Uniform(m) => {
                        add(*m, 512);
                        widen(min, min.map(|v| v + be - 1));
                    }
                    Brick::Full(v) => {
                        let (mut lo, mut hi) = ([be; 3], [-1; 3]);
                        for (idx, &m) in v.iter().enumerate() {
                            if m == AIR {
                                continue;
                            }
                            add(m, 1);
                            let l = [idx % 8, idx / 8 % 8, idx / 64].map(|x| x as i32);
                            lo = [0, 1, 2].map(|k| lo[k].min(l[k]));
                            hi = [0, 1, 2].map(|k| hi[k].max(l[k]));
                        }
                        if hi[0] >= 0 {
                            widen(
                                [0, 1, 2].map(|k| min[k] + lo[k]),
                                [0, 1, 2].map(|k| min[k] + hi[k]),
                            );
                        }
                    }
                }
            }
        }
        i += run;
    }
    counts.sort_unstable();
    let (lo, hi) = bounds.unwrap_or_else(|| {
        let lo = cells.iter().fold([i32::MAX; 3], |a, c| {
            [0, 1, 2].map(|k| a[k].min(c.0[k] * be))
        });
        let hi = cells.iter().fold([i32::MIN; 3], |a, c| {
            [0, 1, 2].map(|k| a[k].max(c.0[k] * be + be - 1))
        });
        (lo, hi)
    });
    (counts, (VoxelCoord(lo), VoxelCoord(hi)))
}
