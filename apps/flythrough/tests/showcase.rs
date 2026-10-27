//! The showcase cavern generated whole: its open volume and connectivity, and the light
//! samples its four ores give, counted on the CPU by the mirror's rule.

use std::collections::{BTreeMap, VecDeque};

use flythrough::showcase::{
    AMBER, AMETHYST, CHASM_HALF, CHASM_X, CRYSTAL, CRYSTAL_FLOOR_X, EMBER, EMBER_TOP_Z, ORE_TOP_Z,
    SHOTS, SHOWCASE_CHUNKS, ShowcaseSource,
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

/// The light samples of a generated world.
struct Census {
    /// Each exposed emissive voxel's material and voxel.
    samples: Vec<(MaterialId, [i32; 3])>,
    /// The fullest chunk's sample count and coordinate.
    worst: (u64, ChunkCoord),
    /// Which material ids are opaque.
    opaque: Vec<bool>,
}

/// The exposed emissive voxels of each chunk by the mirror's rule (an opaque emissive voxel
/// with a non-opaque six-neighbour, across brick faces inside the chunk, a chunk face
/// counting as exposed), counted on the CPU.
fn census(source: &ShowcaseSource, world: &World) -> Census {
    let rows = world.materials().rows();
    let emissive: Vec<bool> = (0..1 << 16)
        .map(|id| {
            rows.get(id)
                .is_some_and(|m| m.emission.iter().any(|&e| e > 0.0))
        })
        .collect();
    let opaque: Vec<bool> = (0..1 << 16)
        .map(|id| id != usize::from(AIR) && rows.get(id).is_some_and(|m| !m.is_translucent()))
        .collect();
    let mut samples = Vec::new();
    let mut worst = (0u64, ChunkCoord([0; 3]));
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
                samples.push((s.material(), s.voxel));
                in_chunk += 1;
            }
        }
        if in_chunk > worst.0 {
            worst = (in_chunk, coord);
        }
    }
    Census {
        samples,
        worst,
        opaque,
    }
}

/// Seed 1's light samples: every ore lights some of the hall and the crystals outnumber each
/// other ore; no chunk holds more than the per-chunk cap and the total is at most a quarter
/// of the buffer. Each ore lies only in its own zone, all under the dome's upper half.
#[test]
fn the_four_ores_light_the_hall_from_their_own_zones() {
    let (source, world) = generate(1);
    let c = census(&source, &world);
    let mut totals: BTreeMap<MaterialId, u64> = BTreeMap::new();
    let mut misplaced = Vec::new();
    for &(m, v) in &c.samples {
        *totals.entry(m).or_default() += 1;
        let [x, y, z] = v.map(i64::from);
        let band = (x - CHASM_X - source.chasm_offset(v[1])).abs() <= CHASM_HALF + 3;
        let home = match m {
            AMBER => x < 448 && z < ORE_TOP_Z,
            AMETHYST => x >= 448 && z < ORE_TOP_Z,
            EMBER => band && z < EMBER_TOP_Z,
            CRYSTAL => x >= CRYSTAL_FLOOR_X && z < ORE_TOP_Z,
            _ => false,
        };
        if !home {
            misplaced.push((m, [x, y, z]));
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
        c.worst.1.0,
        c.worst.0
    );
    assert!(
        misplaced.is_empty(),
        "{} out of their zones: {:?}",
        misplaced.len(),
        &misplaced[..misplaced.len().min(8)]
    );
    assert_eq!(totals.len(), 4, "{totals:?}");
    for ore in [AMBER, AMETHYST, EMBER] {
        assert!(get(ore) > 0, "ore {ore}");
        assert!(get(CRYSTAL) > get(ore), "crystal against ore {ore}");
    }
    assert!(
        c.worst.0 <= u64::from(DEFAULT_LIGHT_CHUNK_CAP),
        "{:?}",
        c.worst
    );
    assert!(total <= u64::from(DEFAULT_LIGHT_SAMPLES) / 4, "{total}");
}

/// No light sample of seed 1 is a ceiling face (its only non-opaque six-neighbour the one
/// below it) or lies in the dome's upper half over the floor plane.
#[test]
fn no_ore_lights_the_roof() {
    let (source, world) = generate(1);
    let c = census(&source, &world);
    let mut ceiling = 0u64;
    let mut high = 0u64;
    let mut first = Vec::new();
    for &(m, v) in &c.samples {
        let open: Vec<usize> = (0..6)
            .filter(|&i| {
                let mut n = v;
                n[i / 2] += if i % 2 == 0 { -1 } else { 1 };
                world
                    .material_at(VoxelCoord(n))
                    .is_some_and(|m| !c.opaque[usize::from(m)])
            })
            .collect();
        let bad = open == [4] || i64::from(v[2]) >= ORE_TOP_Z;
        ceiling += u64::from(open == [4]);
        high += u64::from(i64::from(v[2]) >= ORE_TOP_Z);
        if bad && first.len() < 8 {
            first.push((m, v));
        }
    }
    println!(
        "{} light samples: {ceiling} ceiling faces, {high} at or over z {ORE_TOP_Z}",
        c.samples.len()
    );
    assert_eq!((ceiling, high), (0, 0), "{first:?}");
}

/// On a 2 m grid over the hall's floor (each cell's centre column, where the voxel over the
/// floor is air), at least half the cells have no light sample of seed 1 within 6 m of that
/// voxel: about half the hall is dark.
#[test]
fn about_half_the_hall_floor_is_dark() {
    let (source, world) = generate(1);
    let c = census(&source, &world);
    const REACH: i64 = 60;
    let ores = [AMBER, CRYSTAL, AMETHYST, EMBER];
    let mut grid: BTreeMap<[i64; 3], Vec<(usize, [i64; 3])>> = BTreeMap::new();
    for &(m, v) in &c.samples {
        let v = v.map(i64::from);
        let ore = ores
            .iter()
            .position(|&o| o == m)
            .expect("one of the four ores");
        grid.entry(v.map(|k| k.div_euclid(REACH)))
            .or_default()
            .push((ore, v));
    }
    // The ores with a sample within REACH of `p`, one bit each in `ores`' order.
    let lit_by = |p: [i64; 3]| {
        let cell = p.map(|k| k.div_euclid(REACH));
        let mut bits = 0u8;
        for d in 0..27 {
            let n = [d % 3 - 1, d / 3 % 3 - 1, d / 9 - 1];
            for &(ore, v) in grid
                .get(&[0, 1, 2].map(|k| cell[k] + n[k]))
                .into_iter()
                .flatten()
            {
                if (0..3).map(|k| (v[k] - p[k]).pow(2)).sum::<i64>() <= REACH * REACH {
                    bits |= 1 << ore;
                }
            }
        }
        bits
    };
    let (mut cells, mut dark, mut by) = (0u64, 0u64, [0u64; 4]);
    let size = [0, 1].map(|k| SHOWCASE_CHUNKS[k] * CHUNK_EDGE);
    for y in (10..size[1]).step_by(20) {
        for x in (10..size[0]).step_by(20) {
            let z = source.floor_at(x, y) as i32 + 1;
            if world.material_at(VoxelCoord([x, y, z])) != Some(AIR) {
                continue;
            }
            cells += 1;
            let bits = lit_by([x, y, z].map(i64::from));
            dark += u64::from(bits == 0);
            for (k, n) in by.iter_mut().enumerate() {
                *n += u64::from(bits & 1 << k != 0);
            }
        }
    }
    println!(
        "floor cells: {dark} of {cells} dark ({:.1} %); lit by amber {}, crystal {}, amethyst {}, ember {}",
        100.0 * dark as f64 / cells as f64,
        by[0],
        by[1],
        by[2],
        by[3]
    );
    assert!(cells > 0);
    assert!(2 * dark >= cells, "{dark} of {cells}");
}

/// For every shot, the radius-10 ball around the eye is all air, and the view's centre ray
/// (the yaw and pitch's, which `Shot::ray_milli` holds to a thousandth) meets rock no nearer
/// than 10 m (100 voxels): no shot looks into a wall.
#[test]
fn every_shot_has_air_around_its_eye_and_a_long_centre_ray() {
    let (_, world) = generate(1);
    for s in SHOTS {
        let e = s.eye.0;
        for z in -10..=10 {
            for y in -10..=10 {
                for x in -10..=10 {
                    if x * x + y * y + z * z > 100 {
                        continue;
                    }
                    let p = VoxelCoord([e[0] + x, e[1] + y, e[2] + z]);
                    assert_eq!(
                        world.material_at(p),
                        Some(AIR),
                        "{} eye ball at {:?}",
                        s.name,
                        p.0
                    );
                }
            }
        }
        let (yaw, pitch) = (
            f64::from(s.yaw_deg).to_radians(),
            f64::from(s.pitch_deg).to_radians(),
        );
        let dir = [
            pitch.cos() * yaw.cos(),
            pitch.cos() * yaw.sin(),
            pitch.sin(),
        ];
        for (c, want) in dir.iter().zip(s.ray_milli) {
            let milli = (c * 1000.0).round() as i32;
            assert!(
                (milli - want).abs() <= 1,
                "{} ray {:?}",
                s.name,
                s.ray_milli
            );
        }
        let d = dir.map(|c| (c * FIXED_ONE as f64).round() as i64);
        let hit = world
            .raycast(centre_of_voxel(s.eye), d, 1000 * FIXED_ONE)
            .expect("in range")
            .expect("the ray meets rock inside the shell");
        println!(
            "{}: the centre ray meets material {} at {:.1} m",
            s.name,
            hit.material,
            hit.t as f64 / FIXED_ONE as f64 / 10.0
        );
        assert!(
            hit.t >= 100 * FIXED_ONE,
            "{}: {} voxels",
            s.name,
            hit.t / FIXED_ONE
        );
    }
}
