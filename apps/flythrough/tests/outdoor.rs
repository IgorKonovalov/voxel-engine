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

/// Plan 0032 Phase 8 bar 2: for 16 seeded shapes at leaf fill 128, on the single-tree
/// builder: the trunk's axis column from the base to `base + height - 1` is bark, the voxel
/// `rc + 1` over the canopy's centre is left unwritten (air over air), and the canopy's leaf
/// count is within 3 % of half its sphere's lattice points outside the trunk.
#[test]
fn a_tree_is_what_the_formula_says() {
    use flythrough::trees::{LEAF_FILL, TRUNK_SINK, TreeShape, build_tree};
    let mut rng = SplitMix64(0x7EE5);
    for draw in 0..16 {
        let shape = TreeShape::from_hash(rng.next_u64());
        let tree = build_tree(shape, draw, LEAF_FILL);
        let [ax, ay] = shape.axis();
        for z in TRUNK_SINK..TRUNK_SINK + shape.height {
            assert_eq!(tree.get([ax, ay, z]), Some(BARK), "{shape:?} z {z}");
        }
        let c = shape.canopy_centre();
        assert_eq!(tree.get([c[0], c[1], c[2] + shape.canopy_r + 1]), None);
        let [sx, sy, sz] = tree.size().map(|v| v as i32);
        let (mut points, mut leaves) = (0u64, 0u64);
        for z in 0..sz {
            for y in 0..sy {
                for x in 0..sx {
                    let l = [x, y, z];
                    if shape.in_canopy(l) && !shape.is_trunk(l) {
                        points += 1;
                        leaves += u64::from(tree.get(l) == Some(LEAF));
                    }
                }
            }
        }
        let half = points as f64 / 2.0;
        let off = (leaves as f64 - half).abs() / half;
        println!(
            "{shape:?}: {leaves} leaves of {points} points, {:.2} % off half",
            100.0 * off
        );
        assert!(off <= 0.03, "{shape:?}: {leaves} of {points}");
    }
}

/// Plan 0032 Phase 8 bar 3: with `--trees forest` no bark or leaf voxel lies within 120
/// voxels of the mouth in xy, over the chunks that square covers from 16 under its lowest
/// surface to 130 over its highest; and at 16 seeded trees the axis column from `h + 1` to
/// `base + height - 1` is bark or, where a later tree's canopy covers it, leaf (the leaf share
/// is printed, with no bar).
#[test]
fn no_tree_stands_at_the_mouth() {
    use flythrough::trees::{ForestSource, LEAF_FILL, TreeDensity};
    use vxe_world::{CHUNK_EDGE, ChunkExtent, World};
    let source = ForestSource::new(OutdoorSource::new(1), TreeDensity::Forest, LEAF_FILL);
    let r = 120;
    let [mx, my] = MOUTH;
    let (mut lo, mut hi) = (i32::MAX, i32::MIN);
    for y in my - r..=my + r {
        for x in mx - r..=mx + r {
            let h = source.base().height(x, y);
            (lo, hi) = (lo.min(h), hi.max(h));
        }
    }
    let c = |v: i32| v.div_euclid(CHUNK_EDGE);
    let extent = ChunkExtent::new(
        [c(mx - r), c(my - r), c(lo - 16)],
        [c(mx + r) + 1, c(my + r) + 1, c(hi + 130) + 1],
    );
    let world = World::generate(extent, &source, &flythrough::outdoor_materials());
    let mut near = 0u64;
    for y in my - r..=my + r {
        for x in mx - r..=mx + r {
            if i64::from(x - mx).pow(2) + i64::from(y - my).pow(2) > i64::from(r).pow(2) {
                continue;
            }
            for z in lo - 16..=hi + 130 {
                let m = world.material_at(vxe_world::VoxelCoord([x, y, z]));
                near += u64::from(matches!(m, Some(BARK | LEAF)));
            }
        }
    }
    assert_eq!(near, 0, "bark or leaf within {r} voxels of the mouth");
    let trees = source.placements();
    assert!(trees.len() > 3000, "{} trees", trees.len());
    let mut rng = SplitMix64(0xF0E5);
    let (mut column, mut leaf) = (0u64, 0u64);
    for _ in 0..16 {
        let t = &trees[(rng.next_u64() % trees.len() as u64) as usize];
        let h = source.base().height(t.axis[0], t.axis[1]);
        for z in h + 1..t.base + t.shape.height {
            let m = source.material([t.axis[0], t.axis[1], z]);
            assert!(matches!(m, BARK | LEAF), "{:?} z {z}: {m}", t.axis);
            column += 1;
            leaf += u64::from(m == LEAF);
        }
    }
    println!(
        "16 trees' axis columns: {column} voxels, {leaf} leaf ({:.2} %)",
        100.0 * leaf as f64 / column as f64
    );
}
