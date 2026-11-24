//! The stamp cost (`nfr.md` §4 edits), each iteration one stamp on a freshly generated
//! one-chunk solid world built outside the timed region:
//!
//! - `vault16`: the 16³ vault (shell of material 2, air inside) at `[8, 8, 8]`, so the eight
//!   uniform rock bricks it covers expand.
//! - `prefab64_mixed`: a 64³ all-written prefab at `[0, 0, 0]` whose voxels are air, rock or
//!   material 2 by a hash of the voxel, so all 512 bricks of the chunk expand and stay
//!   `Full`.
//! - `prefab64_uniform`: a 64³ all-written prefab of material 2 at `[0, 0, 0]`, so all 512
//!   bricks expand and collapse back to uniform.
//! - `prefab256_castle_tile`: a 256³ prefab shaped like a castle tile (four 4-voxel walls with
//!   window gaps, a floor every 64 voxels, about 8 % written, the rest unwritten) at `[0, 0, 0]`
//!   on a 4³-chunk solid world, so most of its bricks are passed over as writing nothing.

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use svet_world::hash::hash3;
use svet_world::*;

fn solid_world() -> World {
    let mut table = MaterialTable::new();
    let rock = table.push(Material::named("rock"));
    table.push(Material::named("other"));
    World::generate(
        ChunkExtent::new([0, 0, 0], [1, 1, 1]),
        &UniformSource(rock),
        &table,
    )
}

fn mixed64() -> Prefab {
    let mut voxels = Vec::with_capacity(64 * 64 * 64);
    for z in 0..64 {
        for y in 0..64 {
            for x in 0..64 {
                voxels.push((hash3(0x5EED_57A9, [x, y, z]) % 3) as MaterialId);
            }
        }
    }
    Prefab::new([64; 3], voxels, None)
}

/// A 4³-chunk world of rock.
fn solid_world4() -> World {
    let mut table = MaterialTable::new();
    let rock = table.push(Material::named("rock"));
    table.push(Material::named("other"));
    World::generate(
        ChunkExtent::new([0, 0, 0], [4, 4, 4]),
        &UniformSource(rock),
        &table,
    )
}

/// A castle-like 256³ tile: walls 4 voxels thick on the box's four sides with a window gap
/// every 32 voxels, a floor 4 voxels thick every 64, written as material 2 (or air in the
/// gaps); every other voxel unwritten.
fn castle_tile256() -> Prefab {
    Prefab::from_fn([256; 3], |[x, y, z]| {
        let wall = !(4..252).contains(&x) || !(4..252).contains(&y);
        let floor = z % 64 < 4;
        let gap = wall && (x % 32 < 8 || y % 32 < 8) && z % 64 >= 16 && z % 64 < 40;
        if gap {
            Some(0)
        } else if wall || floor {
            Some(2)
        } else {
            None
        }
    })
}

fn run(c: &mut Criterion, name: &str, prefab: &Prefab, origin: [i32; 3]) {
    run_on(c, name, prefab, origin, solid_world);
}

fn run_on(c: &mut Criterion, name: &str, prefab: &Prefab, origin: [i32; 3], world: fn() -> World) {
    c.bench_function(name, |b| {
        b.iter_batched(
            world,
            |world| {
                std::hint::black_box(world.stamp(prefab, origin, Rot::IDENTITY).unwrap());
                world
            },
            BatchSize::LargeInput,
        )
    });
}

fn stamps(c: &mut Criterion) {
    run(c, "vault16", &Prefab::vault(16, 2), [8, 8, 8]);
    run(c, "prefab64_mixed", &mixed64(), [0, 0, 0]);
    run(
        c,
        "prefab64_uniform",
        &Prefab::new([64; 3], vec![2; 64 * 64 * 64], None),
        [0, 0, 0],
    );
}

fn tile(c: &mut Criterion) {
    let t = castle_tile256();
    println!(
        "prefab256_castle_tile: {} of {} voxels written, bricks (uniform, stored) {:?}, {} B",
        t.written_count(),
        t.len(),
        t.brick_counts(),
        t.bytes()
    );
    run_on(c, "prefab256_castle_tile", &t, [0, 0, 0], solid_world4);
}

criterion_group!(benches, stamps, tile);
criterion_main!(benches);
