//! The edit budget (`nfr.md` §4), three cases, each iteration on a freshly generated world
//! built outside the timed region:
//!
//! - `worst`: 100 carves of radius 10 voxels (1 m at 10 cm) at distinct seeded positions on a
//!   solid world, so every carve expands uniform bricks.
//! - `mixed`: the same 100 carves on a world of 16-voxel cubes, two thirds rock and one third
//!   air, so the carves meet uniform-air, uniform-rock and (after earlier carves) `Full` bricks.
//! - `tool`: 100 capsule carves of radius 5 voxels and length 30 voxels at seeded centres and
//!   seeded directions on the solid world.
//! - `worst_warm` and `tool_warm`: `worst` and `tool` with the world's brick-array pool
//!   reserved in the untimed setup, as a caller that reserves off the edit path sees them.
//!
//! The `expand` group splits what `Brick::expand` costs per 1000 arrays: the allocation on
//! fresh heap, the allocator's hot path, the 1 KiB fill, and the allocation on heap that was
//! touched before.

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use vxe_world::hash::mix64;
use vxe_world::*;

fn extent() -> ChunkExtent {
    ChunkExtent::new([-2, -2, -2], [2, 2, 2])
}

fn table() -> (MaterialTable, MaterialId) {
    let mut table = MaterialTable::new();
    let rock = table.push(Material::named("rock"));
    (table, rock)
}

fn solid_world() -> World {
    let (table, rock) = table();
    World::generate(extent(), &UniformSource(rock), &table)
}

/// `material = if ((x >> 4) + (y >> 4) + (z >> 4)).rem_euclid(3) != 0 { 1 } else { 0 }`. The
/// value is constant over each 16³ cube, so it is evaluated once per brick (bricks are 8³ and
/// aligned to the cubes).
struct CubesSource;

impl WorldSource for CubesSource {
    fn chunk(&self, coord: ChunkCoord, out: &mut Chunk) {
        let base = coord.min_voxel().0;
        for slot in 0..CHUNK_BRICKS {
            let b = brick_of_slot(slot);
            let v = [0, 1, 2].map(|k| base[k] + b[k] * BRICK_EDGE);
            let m = if ((v[0] >> 4) + (v[1] >> 4) + (v[2] >> 4)).rem_euclid(3) != 0 {
                1
            } else {
                AIR
            };
            out.set_brick(slot, Brick::Uniform(m));
        }
    }
}

fn mixed_world() -> World {
    let (table, _) = table();
    World::generate(extent(), &CubesSource, &table)
}

/// 100 distinct voxels within ±100 voxels of the origin, so every op stays resident.
fn seeded_voxels(seed: u64) -> Vec<[i32; 3]> {
    let mut out: Vec<[i32; 3]> = Vec::new();
    let mut state = seed;
    while out.len() < 100 {
        let v = [0; 3].map(|_| {
            state = mix64(state);
            (state % 201) as i32 - 100
        });
        if !out.contains(&v) {
            out.push(v);
        }
    }
    out
}

fn sphere_ops() -> Vec<EditOp> {
    seeded_voxels(0x5EED_0004)
        .into_iter()
        .map(|v| EditOp::sphere(centre_of_voxel(v), 10 * FIXED_ONE, Mode::Carve))
        .collect()
}

/// Capsules of radius 5 voxels whose segment is 30 voxels long (to within one fixed unit),
/// centred on seeded voxels, along seeded non-zero integer directions.
fn tool_ops() -> Vec<EditOp> {
    let mut state = 0x5EED_7001u64;
    seeded_voxels(0x5EED_7000)
        .into_iter()
        .map(|v| {
            let d = loop {
                let d = [0; 3].map(|_| {
                    state = mix64(state);
                    (state % 513) as i64 - 256
                });
                if d != [0; 3] {
                    break d;
                }
            };
            let len = (d.iter().map(|c| c * c).sum::<i64>()).isqrt().max(1);
            let half = d.map(|c| c * 15 * FIXED_ONE / len);
            let c = centre_of_voxel(v).0;
            EditOp::capsule(
                Fixed3([0, 1, 2].map(|k| c[k] - half[k])),
                Fixed3([0, 1, 2].map(|k| c[k] + half[k])),
                5 * FIXED_ONE,
                Mode::Carve,
            )
        })
        .collect()
}

fn run(c: &mut Criterion, name: &str, setup: fn() -> World, ops: &[EditOp]) {
    c.bench_function(name, |b| {
        b.iter_batched(
            setup,
            |world| {
                for op in ops {
                    std::hint::black_box(world.apply(op).unwrap());
                }
                world
            },
            BatchSize::LargeInput,
        )
    });
}

/// Arrays per `expand` iteration.
const ARRAYS: usize = 1000;

type Voxels = [MaterialId; BRICK_VOXELS];

/// Per 1000 arrays of 512 `u16`:
///
/// - `fresh_kept`: `Box::new([3; 512])`, each box kept alive to the end of the iteration and
///   freed outside the timed region, as the edit bench's expansions are. The previous
///   iteration's arrays were freed at the top of the heap, which glibc returns to the kernel
///   past its trim threshold, so these allocations meet fresh pages.
/// - `alloc_drop`: the same allocation with each box dropped before the next: the allocator's
///   hot path, one chunk reused.
/// - `fill`: `[3; 512]` written into each of 1000 existing, touched arrays.
/// - `pretouched`: as `fresh_kept`, after the untimed setup allocated and freed 1000 arrays
///   below a live fence allocation, so the freed megabyte is not at the top of the heap, is
///   not trimmed, and its pages are touched.
fn expand(c: &mut Criterion) {
    let mut g = c.benchmark_group("expand");
    g.bench_function("fresh_kept", |b| {
        b.iter_batched(
            || Vec::<Box<Voxels>>::with_capacity(ARRAYS),
            |mut v| {
                for _ in 0..ARRAYS {
                    v.push(Box::new([std::hint::black_box(3); BRICK_VOXELS]));
                }
                v
            },
            BatchSize::LargeInput,
        )
    });
    g.bench_function("alloc_drop", |b| {
        b.iter(|| {
            for _ in 0..ARRAYS {
                std::hint::black_box(Box::new([std::hint::black_box(3); BRICK_VOXELS]));
            }
        })
    });
    let mut arrays: Vec<Box<Voxels>> = (0..ARRAYS).map(|_| Box::new([0; BRICK_VOXELS])).collect();
    g.bench_function("fill", |b| {
        b.iter(|| {
            for a in &mut arrays {
                a.fill(std::hint::black_box(3));
                std::hint::black_box(&mut **a);
            }
        })
    });
    drop(arrays);
    g.bench_function("pretouched", |b| {
        b.iter_batched(
            || {
                let freed: Vec<Box<Voxels>> =
                    (0..ARRAYS).map(|_| Box::new([0; BRICK_VOXELS])).collect();
                let fence = Box::new([0u64; 4]);
                drop(freed);
                (fence, Vec::<Box<Voxels>>::with_capacity(ARRAYS))
            },
            |(fence, mut v)| {
                for _ in 0..ARRAYS {
                    v.push(Box::new([std::hint::black_box(3); BRICK_VOXELS]));
                }
                (fence, v)
            },
            BatchSize::LargeInput,
        )
    });
    g.finish();
}

/// Arrays reserved for the warm cases: about 25 bricks expanded per carve (24.9 for the
/// spheres and 24.1 for the capsules, counted over the 100 carves), times 100 carves.
const WARM_ARRAYS: usize = 2500;

fn solid_world_warm() -> World {
    let world = solid_world();
    world.reserve_brick_arrays(WARM_ARRAYS);
    world
}

fn edits(c: &mut Criterion) {
    let spheres = sphere_ops();
    let tools = tool_ops();
    run(c, "worst", solid_world, &spheres);
    run(c, "worst_warm", solid_world_warm, &spheres);
    run(c, "mixed", mixed_world, &spheres);
    run(c, "tool", solid_world, &tools);
    run(c, "tool_warm", solid_world_warm, &tools);
}

criterion_group!(benches, edits, expand);
criterion_main!(benches);
