//! The showcase cavern generated whole: its open volume and connectivity, and the light
//! samples its four ores give, counted on the CPU by the mirror's rule.

use std::collections::{BTreeMap, VecDeque};

use flythrough::showcase::{
    AMBER, AMETHYST, CHASM_HALF, CHASM_X, CRYSTAL, CRYSTAL_FLOOR_X, DOME_THIRD_Z, EMBER, SHOTS,
    ShowcaseSource,
};
use vxe_render::gpu_world::{DEFAULT_LIGHT_CHUNK_CAP, DEFAULT_LIGHT_SAMPLES, exposed_emitters};
use vxe_world::*;

fn generate(seed: u64) -> (ShowcaseSource, World) {
    let source = ShowcaseSource::new(seed);
    let world = World::generate(source.extent(), &source, &flythrough::showcase_materials());
    (source, world)
}

/// Whether the brick of `world` at brick coordinate `b` is `Uniform(AIR)`.
fn air_brick(world: &World, b: [i32; 3]) -> bool {
    let edge = CHUNK_EDGE_BRICKS;
    let coord = ChunkCoord(b.map(|v| v.div_euclid(edge)));
    let Some(chunk) = world.store().get(coord) else {
        return false;
    };
    let chunk = chunk
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    matches!(
        chunk.brick(brick_slot(b.map(|v| v.rem_euclid(edge)))),
        Brick::Uniform(AIR)
    )
}

/// A 6-connected flood over bricks from the brick of `hall`'s eye through `Uniform(AIR)`
/// bricks reaches at least 48 828 of them (25 000 m^3 at 0.512 m^3 a brick), the other two
/// shots' eye bricks and a brick at the chasm's bottom (brick z 4).
#[test]
fn the_hall_is_one_open_volume_reaching_every_shot_and_the_chasm_floor() {
    let (_, world) = generate(1);
    let size = [14, 12, 7].map(|c: i32| c * CHUNK_EDGE_BRICKS);
    let index = |b: [i32; 3]| (b[0] + size[0] * (b[1] + size[1] * b[2])) as usize;
    let mut seen = vec![false; (size[0] * size[1] * size[2]) as usize];
    let brick_of = |v: VoxelCoord| v.0.map(|c| c.div_euclid(BRICK_EDGE));
    let start = brick_of(SHOTS[0].eye);
    let mut queue = VecDeque::from([start]);
    seen[index(start)] = true;
    let mut reached = 0u64;
    let mut chasm_floor = 0u64;
    while let Some(b) = queue.pop_front() {
        reached += 1;
        chasm_floor += u64::from(b[2] == 4);
        for i in 0..6 {
            let mut n = b;
            n[i / 2] += if i % 2 == 0 { -1 } else { 1 };
            if (0..3).any(|k| n[k] < 0 || n[k] >= size[k]) || seen[index(n)] {
                continue;
            }
            if air_brick(&world, n) {
                seen[index(n)] = true;
                queue.push_back(n);
            }
        }
    }
    println!(
        "flood: {reached} air bricks ({:.0} m^3), {chasm_floor} at brick z 4",
        reached as f64 * 0.512
    );
    assert!(reached >= 48_828, "{reached} bricks");
    for s in &SHOTS[1..] {
        assert!(seen[index(brick_of(s.eye))], "the {} shot's brick", s.name);
    }
    assert!(chasm_floor > 0, "the flood reaches the chasm's bottom");
}

/// The exposed emissive voxels of each chunk by the mirror's rule (an opaque emissive voxel
/// with a non-opaque six-neighbour, across brick faces inside the chunk, a chunk face
/// counting as exposed): seed 1 gives amber >= 3000, crystal >= 5000, amethyst >= 3000 and
/// ember >= 7000; no chunk holds more than the per-chunk cap and the total is at most a
/// quarter of the buffer. Each ore lies only in its own zone.
#[test]
fn the_four_ores_light_the_hall_from_their_own_zones() {
    let (source, world) = generate(1);
    let table = world.materials();
    let rows = table.rows();
    let emissive: Vec<bool> = (0..1 << 16)
        .map(|id| {
            rows.get(id)
                .is_some_and(|m| m.emission.iter().any(|&e| e > 0.0))
        })
        .collect();
    let opaque: Vec<bool> = (0..1 << 16)
        .map(|id| id != usize::from(AIR) && rows.get(id).is_some_and(|m| !m.is_translucent()))
        .collect();
    let mut totals: BTreeMap<MaterialId, u64> = BTreeMap::new();
    let mut worst = (0u64, ChunkCoord([0; 3]));
    let mut misplaced = Vec::new();
    for coord in source.extent().coords() {
        let chunk = world.store().get(coord).expect("resident");
        let chunk = chunk
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut in_chunk = 0u64;
        for slot in 0..CHUNK_BRICKS {
            let b = brick_of_slot(slot);
            let neighbour = |k: usize, dir: i32| {
                let mut n = b;
                n[k] += dir;
                (0..CHUNK_EDGE_BRICKS)
                    .contains(&n[k])
                    .then(|| chunk.brick(brick_slot(n)))
            };
            for s in exposed_emitters(
                coord,
                slot,
                chunk.brick(slot),
                &emissive,
                &opaque,
                neighbour,
            ) {
                let m = s.material();
                *totals.entry(m).or_default() += 1;
                in_chunk += 1;
                let [x, y, z] = s.voxel.map(i64::from);
                let band = (x - CHASM_X - source.chasm_offset(s.voxel[1])).abs() <= CHASM_HALF + 3;
                let home = match m {
                    AMBER => x < 448 && z < DOME_THIRD_Z,
                    AMETHYST => z >= DOME_THIRD_Z,
                    EMBER => band && z < 128,
                    CRYSTAL => x >= CRYSTAL_FLOOR_X && z < DOME_THIRD_Z,
                    _ => false,
                };
                if !home {
                    misplaced.push((m, [x, y, z]));
                }
            }
        }
        if in_chunk > worst.0 {
            worst = (in_chunk, coord);
        }
    }
    let total: u64 = totals.values().sum();
    let get = |m| totals.get(&m).copied().unwrap_or(0);
    println!(
        "light samples: amber {}, crystal {}, amethyst {}, ember {}; total {total}; the fullest chunk {:?} holds {}",
        get(AMBER),
        get(CRYSTAL),
        get(AMETHYST),
        get(EMBER),
        worst.1.0,
        worst.0
    );
    assert!(
        misplaced.is_empty(),
        "{} out of their zones: {:?}",
        misplaced.len(),
        &misplaced[..misplaced.len().min(8)]
    );
    assert_eq!(totals.len(), 4, "{totals:?}");
    assert!(get(AMBER) >= 3000);
    assert!(get(CRYSTAL) >= 5000);
    assert!(get(AMETHYST) >= 3000);
    assert!(get(EMBER) >= 7000);
    assert!(worst.0 <= u64::from(DEFAULT_LIGHT_CHUNK_CAP), "{worst:?}");
    assert!(total <= u64::from(DEFAULT_LIGHT_SAMPLES) / 4, "{total}");
}
