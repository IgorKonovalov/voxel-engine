//! Collapse on the seed-1 level: seeded capsule carves near the spawn, the support query after
//! each, and a drop of every region it reports.
//!
//! The digs: a SplitMix-style stream seeded 1 draws an origin within ±200 voxels of the
//! spawn on each axis and a direction within ±256 per axis. A loop iteration whose origin is
//! not air, whose direction is zero or whose ray hits nothing within 100 voxels digs nothing;
//! otherwise a capsule of radius 5 voxels carves from the origin to the hit, the grid follows
//! the carve, and the query runs over its changes with the fly-through's support parameters.

use flythrough::cave::{CaveSource, LEVEL_CHUNKS};
use flythrough::{RUBBLE, cave_materials};
use vxe_world::*;

/// Iterations of the full sweep.
const SWEEP: usize = 40_000;

fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn params() -> SupportParams {
    SupportParams {
        solid_min: 64,
        max_region: 4096,
        boundary_anchors: true,
    }
}

/// The level, its grid, the dig stream's state and the spawn.
struct Digs {
    world: World,
    grid: CoarseGrid,
    state: u64,
    spawn: [i32; 3],
}

impl Digs {
    fn new() -> Self {
        let source = CaveSource::new(1, ChunkExtent::new([0, 0, 0], LEVEL_CHUNKS));
        let world = World::generate(source.extent, &source, &cave_materials());
        let grid = CoarseGrid::build(&world);
        Self {
            world,
            grid,
            state: 1,
            spawn: source.spawn().0,
        }
    }

    fn next(&mut self) -> u64 {
        self.state = mix(self.state);
        self.state
    }

    /// One loop iteration: the regions the query reports after its dig, empty when it dug
    /// nothing or found none.
    fn step(&mut self) -> Vec<Region> {
        let o = [0, 1, 2].map(|k| i64::from(self.spawn[k]) + (self.next() % 400) as i64 - 200);
        let dir = [0, 1, 2].map(|_| (self.next() % 513) as i64 - 256);
        if dir == [0, 0, 0] {
            return Vec::new();
        }
        let origin = Fixed3(o.map(|v| v * FIXED_ONE + FIXED_ONE / 2));
        if self.world.material_at(VoxelCoord(o.map(|v| v as i32))) != Some(AIR) {
            return Vec::new();
        }
        let Ok(Some(hit)) = self.world.raycast(origin, dir, 100 * FIXED_ONE) else {
            return Vec::new();
        };
        let end = Fixed3([0, 1, 2].map(|k| origin.0[k] + dir[k] * hit.t / FIXED_ONE));
        let Ok(res) = self
            .world
            .apply(&EditOp::capsule(origin, end, 5 * FIXED_ONE, Mode::Carve))
        else {
            return Vec::new();
        };
        let changes = self.grid.update(&self.world, &res);
        find_unsupported(
            &self.world,
            &self.grid,
            &params(),
            &Anchors { explicit: &[] },
            &changes,
        )
    }
}

impl Digs {
    fn grid_ref(&self) -> &CoarseGrid {
        &self.grid
    }
}

fn voxels(r: &Region) -> u32 {
    r.counts.iter().map(|c| c.1).sum()
}

/// A region's fall as the fly-through's logic runs it: drops of up to [`FALL_STEP`] through
/// cells under [`YIELD_BELOW`] solid voxels, the cells one cell lower after each whole step,
/// until a drop moves it less; then, when it crushed anything, a pile of the whole crushed
/// volume on its cells. The grid follows every op.
#[derive(Debug, Default, PartialEq, Eq)]
struct Fall {
    /// Each drop's `dropped`.
    drops: Vec<i32>,
    /// The crushed voxels over all drops and materials.
    crushed: u32,
    /// The pile's `placed`, when there was one.
    placed: Option<u32>,
    /// The refusal that ended the fall, if any.
    refused: Option<EditError>,
}

const FALL_STEP: i32 = BRICK_EDGE;
const YIELD_BELOW: u16 = 64;
const RUBBLE_DEPTH: i32 = 64;

impl Digs {
    fn fall(&mut self, region: &Region) -> Fall {
        let mut cells = region.cells.clone();
        let mut out = Fall::default();
        loop {
            match self
                .world
                .drop_region_through(&cells, FALL_STEP, YIELD_BELOW)
            {
                Ok(d) => {
                    self.grid.update(&self.world, &d.edit_result());
                    out.drops.push(d.dropped);
                    out.crushed += d.crushed.iter().map(|c| c.1).sum::<u32>();
                    if d.dropped < FALL_STEP {
                        break;
                    }
                    for c in &mut cells {
                        c.0[2] -= 1;
                    }
                }
                Err(e) => {
                    out.refused = Some(e);
                    break;
                }
            }
        }
        if out.crushed > 0 {
            let p = self
                .world
                .pile_rubble(&cells, RUBBLE, out.crushed, RUBBLE_DEPTH)
                .expect("the pile scans the cells the fall just crossed");
            self.grid.update(&self.world, &p.edit_result());
            assert!(p.placed <= out.crushed, "{p:?}");
            if p.placed < out.crushed {
                println!(
                    "a pile on {:?} placed {} of {}",
                    cells[0], p.placed, out.crushed
                );
            }
            out.placed = Some(p.placed);
        }
        out
    }
}

/// The first region the digs detach: 4 cells, 501 voxels, at loop iteration 3 243. It rests
/// on cells the query calls too weak to hold it: `drop_region` stops at their voxels, and
/// `drop_region_through` with the query's threshold falls through them, crushing what it
/// passes, until it lands on a cell that bears load.
#[test]
fn the_level_repro_finds_a_region_that_falls_through_slivers() {
    let mut digs = Digs::new();
    let (at, region) = (0..SWEEP)
        .find_map(|i| digs.step().into_iter().next().map(|r| (i, r)))
        .expect("a region within the sweep");
    assert_eq!(at, 3243);
    assert_eq!(region.cells.len(), 4);
    assert_eq!(voxels(&region), 501);
    assert_eq!(
        region.aabb,
        (VoxelCoord([553, 794, 80]), VoxelCoord([566, 803, 87]))
    );
    let before = digs.world.voxel_hash();
    let d = digs.world.drop_region(&region.cells, BRICK_EDGE).unwrap();
    assert_eq!(d, DropResult::default());
    assert_eq!(digs.world.voxel_hash(), before);

    // Nine whole steps, 72 voxels, then a cell that bears load; the whole crushed volume
    // fits on top of the landed piece.
    let fall = digs.fall(&region);
    assert_eq!(
        fall,
        Fall {
            drops: vec![8, 8, 8, 8, 8, 8, 8, 8, 8, 0],
            crushed: 141,
            placed: Some(141),
            refused: None,
        }
    );
    assert_eq!(*digs.grid_ref(), CoarseGrid::build(&digs.world));
}

/// The whole sweep, every reported region falling as [`Digs::fall`] runs it: no region is
/// wedged on its first drop and none is refused. The first region falls away, so the later
/// digs that detached it again while it stayed wedged find nothing: one region in all.
#[test]
fn the_level_sweep_drops_every_region() {
    let mut digs = Digs::new();
    let (mut regions, mut crushed, mut placed) = (0, 0, 0);
    let mut falls = Vec::new();
    for _ in 0..SWEEP {
        for region in digs.step() {
            regions += 1;
            let fall = digs.fall(&region);
            println!(
                "{} cells, {} voxels: {fall:?}",
                region.cells.len(),
                voxels(&region)
            );
            crushed += fall.crushed;
            placed += fall.placed.unwrap_or(0);
            falls.push(fall);
        }
    }
    println!("{regions} regions, {crushed} crushed, {placed} placed");
    assert!(falls.iter().all(|f| f.refused.is_none()), "{falls:?}");
    assert!(falls.iter().all(|f| f.drops[0] > 0), "{falls:?}");
    assert_eq!((regions, crushed, placed), (1, 141, 141));
}
