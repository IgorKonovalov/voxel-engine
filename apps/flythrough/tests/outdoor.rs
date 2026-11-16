//! The outdoor source's geometry (Plan 0032 Phase 3), on its voxel function alone: no world,
//! no adapter.

use std::collections::{HashMap, VecDeque};

use flythrough::outdoor::*;
use vxe_world::AIR;
use vxe_world::hash::SplitMix64;

/// The mouth leads in and the chamber is roofed: the tunnel's axis from the surface at the
/// mouth to the chamber's centre is air; every column within 40 voxels of the chamber's centre
/// has at least 60 non-air voxels between the chamber's top and the surface; and a 6-connected
/// flood of air from the chamber's centre, bounded to the chamber's box grown by 32 voxels,
/// meets the box's faces only inside the tunnel's capsule.
#[test]
fn the_mouth_leads_in_and_the_chamber_is_roofed() {
    for seed in [1, 2, 3] {
        let s = OutdoorSource::new(seed);
        let mut heights: HashMap<(i32, i32), i32> = HashMap::new();
        let mut mat = |p: [i32; 3]| {
            let h = *heights
                .entry((p[0], p[1]))
                .or_insert_with(|| s.height(p[0], p[1]));
            s.material_at(p, h)
        };
        let [a, c] = s.tunnel();
        let surface = s.height(MOUTH[0], MOUTH[1]);
        let n = (0..3).map(|k| (c[k] - a[k]).abs()).max().unwrap();
        for i in 0..=n {
            let p = [0, 1, 2].map(|k| (a[k] + (c[k] - a[k]) * i / n) as i32);
            if p[2] <= surface {
                assert_eq!(mat(p), AIR, "seed {seed}: the axis at {p:?}");
            }
        }
        let centre = s.chamber().0;
        let r = CHAMBER_RADIUS as i32;
        for dy in -40..=40 {
            for dx in -40..=40 {
                if dx * dx + dy * dy > 40 * 40 {
                    continue;
                }
                let (x, y) = (centre[0] + dx, centre[1] + dy);
                let roof = (centre[2] + r + 1..=s.height(x, y))
                    .filter(|&z| mat([x, y, z]) != AIR)
                    .count();
                assert!(
                    roof >= 60,
                    "seed {seed}: column ({x}, {y}) has {roof} voxels of roof"
                );
            }
        }
        let grow = r + 32;
        let lo = centre.map(|v| v - grow);
        let hi = centre.map(|v| v + grow);
        let mut seen = std::collections::HashSet::new();
        let mut queue = VecDeque::from([centre]);
        seen.insert(centre);
        let mut exits = 0;
        while let Some(p) = queue.pop_front() {
            if (0..3).any(|k| p[k] == lo[k] || p[k] == hi[k]) {
                assert!(
                    s.carved(p),
                    "seed {seed}: air leaves the box at {p:?} outside the tunnel"
                );
                exits += 1;
            }
            for k in 0..3 {
                for d in [-1, 1] {
                    let mut q = p;
                    q[k] += d;
                    if (0..3).all(|j| (lo[j]..=hi[j]).contains(&q[j]))
                        && !seen.contains(&q)
                        && mat(q) == AIR
                    {
                        seen.insert(q);
                        queue.push_back(q);
                    }
                }
            }
        }
        assert!(
            exits > 0,
            "seed {seed}: the tunnel leaves the chamber's box"
        );
    }
}

/// At 16 seeded columns the topmost non-air voxel is grass at the formula's height and the 8
/// below it soil, except inside the tunnel's capsule or the chamber.
#[test]
fn the_surface_is_grass_over_soil_at_the_formula_height() {
    let s = OutdoorSource::new(1);
    let mut rng = SplitMix64(0x0D00_D00D);
    let mut checked = 0;
    while checked < 16 {
        let (x, y) = (rng.range(-3000, 3000) as i32, rng.range(-3000, 3000) as i32);
        let h = s.height(x, y);
        if (h - SOIL_DEPTH..=h + 8).any(|z| s.carved([x, y, z])) {
            continue;
        }
        for z in h + 1..=h + 8 {
            assert_eq!(
                s.material([x, y, z]),
                AIR,
                "({x}, {y}, {z}) over the surface"
            );
        }
        assert_eq!(s.material([x, y, h]), GRASS, "({x}, {y}) at {h}");
        for z in h - SOIL_DEPTH..h {
            assert_eq!(
                s.material([x, y, z]),
                SOIL,
                "({x}, {y}, {z}) under the grass"
            );
        }
        checked += 1;
    }
}

/// A chunk's bricks, with their uniform short cuts, read voxel for voxel what
/// [`OutdoorSource::material`] reads: chunks at the mouth, along the tunnel, at the chamber and
/// deep under it.
#[test]
fn the_chunk_short_cuts_match_the_voxel_function() {
    use vxe_world::{
        BRICK_EDGE, CHUNK_BRICKS, CHUNK_EDGE, Chunk, ChunkCoord, VoxelCoord, WorldSource,
        brick_of_slot,
    };
    let s = OutdoorSource::new(1);
    let [a, c] = s.tunnel();
    let mid = [0, 1, 2].map(|k| ((a[k] + c[k]) / 2) as i32);
    let deep = {
        let p = s.chamber().0;
        [p[0] + 300, p[1], p[2] - 200]
    };
    for v in [a.map(|x| x as i32), mid, s.chamber().0, deep] {
        let coord = ChunkCoord(VoxelCoord(v).0.map(|x| x.div_euclid(CHUNK_EDGE)));
        let mut chunk = Chunk::default();
        s.chunk(coord, &mut chunk);
        let base = coord.min_voxel().0;
        for slot in 0..CHUNK_BRICKS {
            let b = brick_of_slot(slot).map(|v| v * BRICK_EDGE);
            for i in 0..512usize {
                let l = [i % 8, i / 8 % 8, i / 64].map(|v| v as i32);
                let p = [0, 1, 2].map(|k| base[k] + b[k] + l[k]);
                assert_eq!(chunk.brick(slot).get(i), s.material(p), "{p:?}");
            }
        }
    }
}
