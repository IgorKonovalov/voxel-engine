//! The brick census: bins of materials per `Full` brick, translucent counts and the three
//! payload totals.

use vxe_world::hash::{ValueNoise, hash3};
use vxe_world::*;

/// Rows: air, rock 1, ore 2, glass 3 (`TRANSLUCENT`), then 4..=20.
fn table() -> MaterialTable {
    let mut t = MaterialTable::new();
    assert_eq!(t.push(Material::named("rock")), 1);
    assert_eq!(t.push(Material::named("ore")), 2);
    let mut glass = Material::named("glass");
    glass.flags = MaterialFlags::TRANSLUCENT;
    assert_eq!(t.push(glass), 3);
    for i in 4..=20u16 {
        assert_eq!(t.push(Material::named(format!("m{i}"))), i);
    }
    t
}

/// Slot 0 `Uniform(rock)`, slot 1 `{air, rock}`, slot 2 `{rock, ore}`, slot 3 seventeen
/// materials (4 to 20, no air), slot 4 `{rock, glass}`, slot 5 `{air, glass}`; the rest air.
struct SixSlots;

impl WorldSource for SixSlots {
    fn chunk(&self, _: ChunkCoord, out: &mut Chunk) {
        let half = |a: MaterialId, b: MaterialId| Brick::from_fn(|l| if l[2] < 4 { a } else { b });
        out.set_brick(0, Brick::Uniform(1));
        out.set_brick(1, half(AIR, 1));
        out.set_brick(2, half(1, 2));
        out.set_brick(
            3,
            Brick::from_fn(|l| 4 + (index_in_brick(l) % 17) as MaterialId),
        );
        out.set_brick(4, half(1, 3));
        out.set_brick(5, half(AIR, 3));
    }
}

#[test]
fn the_census_bins_a_chunk_of_six_kinds_of_brick() {
    let world = World::generate(ChunkExtent::new([0, 0, 0], [1, 1, 1]), &SixSlots, &table());
    let c = world.brick_census();
    let b = c.bricks;
    assert_eq!(b.uniform, 507);
    assert_eq!(b.full_by_materials, [2, 2, 0, 0, 1, 0]);
    assert_eq!((b.translucent_any, b.translucent_only), (2, 1));
    assert_eq!(
        (b.payload_today, b.payload_ladder, b.payload_exact_planes),
        (5120, 1440, 854)
    );
    assert_eq!((b.slots, b.slot_bytes()), (512, 8192));
    assert_eq!(c.mip_bricks, BrickCensus::default());
}

#[test]
fn mip_bricks_are_counted_apart_from_bricks() {
    let world = World::new(&table());
    let mut chunk = Chunk::default();
    SixSlots.chunk(ChunkCoord([0, 0, 0]), &mut chunk);
    world.insert_lod(ChunkCoord([0, 0, 0]), ChunkMips::build(&chunk).into());
    let c = world.brick_census();
    assert_eq!(c.bricks, BrickCensus::default());
    let m = c.mip_bricks;
    assert_eq!(m.slots, 73);
    assert_eq!(m.uniform + m.full(), 73);
    assert!(m.full() > 0);
}

/// The source behind `world_budget --veins N`: air where a seeded value noise of 16-voxel
/// cells is below -4600 (about a third of the voxels), otherwise material
/// `1 + hash3(seed, cell) % (n - 1)` per 2^3 cell.
struct Veins {
    seed: u64,
    n: u64,
    noise: ValueNoise,
}

impl Veins {
    fn new(seed: u64, n: u64) -> Self {
        Self {
            seed,
            n,
            noise: ValueNoise::new(seed ^ 0xA1A1, 4),
        }
    }
}

impl WorldSource for Veins {
    fn chunk(&self, coord: ChunkCoord, out: &mut Chunk) {
        let base = coord.min_voxel().0;
        for slot in 0..CHUNK_BRICKS {
            let b = brick_of_slot(slot);
            out.set_brick(
                slot,
                Brick::from_fn(|l| {
                    let v = [0, 1, 2].map(|k| base[k] + b[k] * BRICK_EDGE + l[k]);
                    if self.noise.sample(v) < -4600 {
                        AIR
                    } else {
                        (1 + hash3(self.seed, v.map(|c| c >> 1)) % (self.n - 1)) as MaterialId
                    }
                }),
            );
        }
    }
}

/// 64 cells drawing from `N - 1` ids hold about 7, 15 and 40 distinct materials at `N` 8, 16
/// and 64, so most `Full` bricks fall in the 5 to 16 bin at 8 and 16 and in the 17 to 256 bin
/// at 64.
#[test]
fn the_veins_source_fills_the_bins_the_arithmetic_says() {
    for (n, bin) in [(8, 3), (16, 3), (64, 4)] {
        let mut table = MaterialTable::new();
        for i in 1..n {
            table.push(Material::named(format!("m{i}")));
        }
        let world = World::generate(
            ChunkExtent::new([0, 0, 0], [2, 2, 1]),
            &Veins::new(1, n),
            &table,
        );
        let c = world.brick_census().bricks;
        assert_eq!(c.majority_bin(), Some(bin), "N {n}: {c}");
    }
}
