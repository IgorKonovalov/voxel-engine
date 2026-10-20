//! Collapse on the seed-1 level: seeded capsule carves near the spawn, the support query after
//! each, and a drop of every region it reports.
//!
//! The digs: a SplitMix-style stream seeded 1 draws an origin within ±200 voxels of the
//! spawn on each axis and a direction within ±256 per axis. A loop iteration whose origin is
//! not air, whose direction is zero or whose ray hits nothing within 100 voxels digs nothing;
//! otherwise a capsule of radius 5 voxels carves from the origin to the hit, the grid follows
//! the carve, and the query runs over its changes with the fly-through's support parameters.

use flythrough::cave::{CaveSource, LEVEL_CHUNKS};
use flythrough::cave_materials;
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

fn voxels(r: &Region) -> u32 {
    r.counts.iter().map(|c| c.1).sum()
}

/// The first region the digs detach: 4 cells, 501 voxels, at loop iteration 3 243. It rests
/// on cells the query calls too weak to hold it, and `drop_region` stops at their voxels, so
/// it drops nothing. Plan 0022 Phase 2 adds the yielding drop that moves it.
#[test]
fn the_level_repro_finds_a_region_that_drops_nothing() {
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
    assert_eq!(d.dropped, 0);
    assert_eq!(d, DropResult::default());
    assert_eq!(digs.world.voxel_hash(), before);
}

/// The whole sweep, every reported region dropped once by up to a cell: six regions, none
/// moved, none refused. Plan 0022 Phase 2 adds the yielding drop that moves them.
#[test]
fn the_level_sweep_drops_no_region() {
    let mut digs = Digs::new();
    let (mut regions, mut moved, mut refused) = (0, 0, 0);
    for _ in 0..SWEEP {
        for region in digs.step() {
            regions += 1;
            match digs.world.drop_region(&region.cells, BRICK_EDGE) {
                Ok(d) if d.dropped > 0 => {
                    moved += 1;
                    digs.grid.update_aabb(&digs.world, d.aabb);
                }
                Ok(_) => {}
                Err(e) => {
                    refused += 1;
                    println!("refused: {e}");
                }
            }
        }
    }
    assert_eq!((regions, moved, refused), (6, 0, 0));
}
