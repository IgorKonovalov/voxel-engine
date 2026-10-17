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

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use vxe_world::hash::hash3;
use vxe_world::*;

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

fn run(c: &mut Criterion, name: &str, prefab: &Prefab, origin: [i32; 3]) {
    c.bench_function(name, |b| {
        b.iter_batched(
            solid_world,
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

criterion_group!(benches, stamps);
criterion_main!(benches);
