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

/// Plan 0032 Phase 8 bar 2: for 16 seeded shapes at the default leaf fill (`LEAF_FILL`, 64),
/// on the single-tree builder: the trunk's axis column from the base to `base + height - 1`
/// is bark, the voxel `rc + 1` over the canopy's centre is left unwritten (air over air), and
/// the canopy's leaf count is within 3 % of `LEAF_FILL / 256` (a quarter) of its sphere's
/// lattice points outside the trunk: at the smallest canopy, about 24 400 points, the
/// binomial standard deviation of a quarter keep is about 68, 1.1 % of the expected 6100.
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
        let want = points as f64 * f64::from(LEAF_FILL) / 256.0;
        let off = (leaves as f64 - want).abs() / want;
        println!(
            "{shape:?}: {leaves} leaves of {points} points, {:.2} % off {want:.0}",
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

mod reading {
    use super::*;

    /// Plan 0086 Phase 8 diagnosis: what lies at the owner's run-12 eye (743, 131, 15), seed 1.
    /// The surface over it, the distance to rock along each axis, and a 6-connected flood of
    /// air from the eye bounded to a box of +-`R` in xy and from z -256 to the surface: the
    /// pocket's voxel count, its bounds, and whether it reaches a column's surface (an opening
    /// to the sky) or the box's faces. Printed.
    #[test]
    #[ignore = "reading"]
    fn what_is_at_the_underground_eye() {
        const R: i32 = 256;
        let s = OutdoorSource::new(1);
        let eye = [743, 131, 15];
        let h = s.height(eye[0], eye[1]);
        println!(
            "reading: eye {eye:?} material {} surface z {h}",
            s.material(eye)
        );
        for (k, d) in [(0, 1), (0, -1), (1, 1), (1, -1), (2, 1), (2, -1)] {
            let mut p = eye;
            let mut n = 0;
            while s.material(p) == AIR && n < 2000 {
                p[k] += d;
                n += 1;
            }
            println!("reading: axis {k} dir {d}: rock after {n} voxels at {p:?}");
        }
        let mut heights: HashMap<(i32, i32), i32> = HashMap::new();
        let mut mat = |p: [i32; 3]| {
            let h = *heights
                .entry((p[0], p[1]))
                .or_insert_with(|| s.height(p[0], p[1]));
            (s.material_at(p, h), h)
        };
        let lo = [eye[0] - R, eye[1] - R, -256];
        let hi = [eye[0] + R, eye[1] + R, 900];
        let mut seen = std::collections::HashSet::new();
        let mut queue = VecDeque::from([eye]);
        seen.insert(eye);
        let (mut bmin, mut bmax) = (eye, eye);
        let mut open = None;
        let mut faces = 0usize;
        while let Some(p) = queue.pop_front() {
            for k in 0..3 {
                bmin[k] = bmin[k].min(p[k]);
                bmax[k] = bmax[k].max(p[k]);
            }
            if (0..3).any(|k| p[k] == lo[k] || p[k] == hi[k]) {
                faces += 1;
            }
            for k in 0..3 {
                for d in [-1, 1] {
                    let mut q = p;
                    q[k] += d;
                    if (0..3).all(|j| (lo[j]..=hi[j]).contains(&q[j])) && !seen.contains(&q) {
                        let (m, hq) = mat(q);
                        if m == AIR {
                            if q[2] > hq && open.is_none() {
                                open = Some(q);
                            }
                            if q[2] <= hq {
                                seen.insert(q);
                                queue.push_back(q);
                            }
                        }
                    }
                }
            }
            if seen.len() > 20_000_000 {
                break;
            }
        }
        println!(
            "reading: pocket {} voxels, bounds {bmin:?}..{bmax:?}, open to the sky at {open:?}, box-face voxels {faces}",
            seen.len()
        );
    }

    /// The first rock voxel's distance along `d` from `o` (a voxel-centre DDA), or `None` past
    /// `cap` voxels.
    fn free_length(
        mat: &mut dyn FnMut([i32; 3]) -> MaterialIdAlias,
        o: [f64; 3],
        d: [f64; 3],
        cap: f64,
    ) -> Option<f64> {
        let mut v = o.map(|x| x.floor() as i32);
        let step = d.map(|x| if x > 0.0 { 1 } else { -1 });
        let mut tmax = [0.0f64; 3];
        let mut tdelta = [0.0f64; 3];
        for k in 0..3 {
            tdelta[k] = if d[k] == 0.0 {
                f64::INFINITY
            } else {
                1.0 / d[k].abs()
            };
            let edge = if d[k] > 0.0 {
                f64::from(v[k]) + 1.0
            } else {
                f64::from(v[k])
            };
            tmax[k] = if d[k] == 0.0 {
                f64::INFINITY
            } else {
                (edge - o[k]) / d[k]
            };
        }
        loop {
            let k = if tmax[0] < tmax[1] && tmax[0] < tmax[2] {
                0
            } else if tmax[1] < tmax[2] {
                1
            } else {
                2
            };
            let t = tmax[k];
            if t > cap {
                return None;
            }
            v[k] += step[k];
            tmax[k] += tdelta[k];
            if mat(v) != AIR {
                return Some(t);
            }
        }
    }

    type MaterialIdAlias = vxe_world::MaterialId;

    /// Plan 0086 Phase 8 diagnosis: the free path of the sky rays from the cavern's faces
    /// within 48 voxels of the run-12 eye (743, 131, 15), seed 1: 64 cosine-weighted rays a face
    /// about its normal, those with d.z > 0 walked on the voxel function to 4096 voxels. The
    /// share of all rays whose free path passes 256 (the bake's cap, `light_radius` 4 x 64) and
    /// 512 (the light pass's `SKY_RAY_VOXELS`); none of them can reach the sky (the caverns
    /// are closed, `what_is_at_the_underground_eye`). Printed.
    #[test]
    #[ignore = "reading"]
    fn sky_ray_free_paths_at_the_underground_eye() {
        let s = OutdoorSource::new(1);
        let mut heights: HashMap<(i32, i32), i32> = HashMap::new();
        let mut mat = |p: [i32; 3]| {
            let h = *heights
                .entry((p[0], p[1]))
                .or_insert_with(|| s.height(p[0], p[1]));
            s.material_at(p, h)
        };
        let eye = [743, 131, 15];
        let mut rng = SplitMix64(86);
        let mut u = || (rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64;
        let (mut faces, mut rays, mut up, mut over256, mut over512, mut over4096) =
            (0, 0, 0, 0, 0, 0);
        let mut per_axis = [[0usize; 3]; 6];
        for z in eye[2] - 48..=eye[2] + 48 {
            for y in (eye[1] - 48..=eye[1] + 48).step_by(3) {
                for x in (eye[0] - 48..=eye[0] + 48).step_by(3) {
                    let p = [x, y, z];
                    if mat(p) != AIR {
                        continue;
                    }
                    for (f, (k, sgn)) in [(0, -1), (0, 1), (1, -1), (1, 1), (2, -1), (2, 1)]
                        .into_iter()
                        .enumerate()
                    {
                        // A face of the air voxel `p` toward a rock neighbour: its normal points
                        // into the air, away from the rock.
                        let mut q = p;
                        q[k] += sgn;
                        if mat(q) == AIR {
                            continue;
                        }
                        faces += 1;
                        let mut n = [0.0f64; 3];
                        n[k] = -f64::from(sgn);
                        let a = if n[0].abs() > 0.9 {
                            [0.0, 1.0, 0.0]
                        } else {
                            [1.0, 0.0, 0.0]
                        };
                        let cross = |a: [f64; 3], b: [f64; 3]| {
                            [
                                a[1] * b[2] - a[2] * b[1],
                                a[2] * b[0] - a[0] * b[2],
                                a[0] * b[1] - a[1] * b[0],
                            ]
                        };
                        let t = cross(a, n);
                        let b = cross(n, t);
                        let o = [0, 1, 2].map(|j| f64::from(p[j]) + 0.5 + n[j] * 0.49);
                        for _ in 0..64 {
                            let (u1, u2) = (u(), u());
                            let r = u1.sqrt();
                            let phi = 2.0 * std::f64::consts::PI * u2;
                            let c = (1.0 - u1).max(0.0).sqrt();
                            let d = [0, 1, 2]
                                .map(|j| t[j] * r * phi.cos() + b[j] * r * phi.sin() + n[j] * c);
                            rays += 1;
                            per_axis[f][0] += 1;
                            if d[2] <= 0.0 {
                                continue;
                            }
                            up += 1;
                            match free_length(&mut mat, o, d, 4096.0) {
                                Some(l) if l <= 256.0 => {}
                                Some(l) if l <= 512.0 => {
                                    over256 += 1;
                                    per_axis[f][1] += 1;
                                }
                                Some(_) => {
                                    over256 += 1;
                                    over512 += 1;
                                    per_axis[f][1] += 1;
                                    per_axis[f][2] += 1;
                                }
                                None => {
                                    over256 += 1;
                                    over512 += 1;
                                    over4096 += 1;
                                    per_axis[f][1] += 1;
                                    per_axis[f][2] += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
        let pct = |a: usize| 100.0 * a as f64 / rays as f64;
        println!(
            "reading: {faces} faces, {rays} rays, {up} up ({:.1} %); free path > 256: {:.2} % of all rays, > 512: {:.2} %, > 4096: {:.3} %",
            pct(up),
            pct(over256),
            pct(over512),
            pct(over4096)
        );
        for (f, name) in ["-x", "+x", "-y", "+y", "-z (ceiling)", "+z (floor)"]
            .iter()
            .enumerate()
        {
            // `per_axis[f]` is indexed by the rock side; the face normal is the opposite.
            let [n, a, b] = per_axis[f];
            println!(
                "reading: faces with rock on side {name}: {n} rays, > 256: {:.2} %, > 512: {:.2} %",
                100.0 * a as f64 / n.max(1) as f64,
                100.0 * b as f64 / n.max(1) as f64
            );
        }
    }

    /// Plan 0086 Phase 8 diagnosis: the sky rays' fate in the tunnel of seed 1 at eleven
    /// stations along its axis (fraction 0 at the mouth's surface, 1 at the chamber's centre).
    /// At each, the air voxels with a rock neighbour within 24 voxels of the station (every
    /// third in x and y), 64 cosine-weighted rays a face about its normal, those with d.z > 0
    /// walked on the voxel function: the share of all rays that truly reach the sky (an air
    /// voxel over its column's surface before any rock), and the share that the caps count as
    /// sky without reaching it (no rock within 256, the bake's cap; within 512, the light
    /// pass's). Printed.
    #[test]
    #[ignore = "reading"]
    fn sky_ray_fates_along_the_tunnel() {
        let s = OutdoorSource::new(1);
        let mut heights: HashMap<(i32, i32), i32> = HashMap::new();
        let [a, c] = s.tunnel();
        let mut rng = SplitMix64(87);
        for station in 0..=10i64 {
            let centre = [0, 1, 2].map(|k| (a[k] + (c[k] - a[k]) * station / 10) as i32);
            let (mut rays, mut sky, mut fake256, mut fake512) = (0usize, 0usize, 0usize, 0usize);
            let mut faces = 0usize;
            for z in centre[2] - 24..=centre[2] + 24 {
                for y in (centre[1] - 24..=centre[1] + 24).step_by(3) {
                    for x in (centre[0] - 24..=centre[0] + 24).step_by(3) {
                        let mut mat = |p: [i32; 3]| {
                            let h = *heights
                                .entry((p[0], p[1]))
                                .or_insert_with(|| s.height(p[0], p[1]));
                            (s.material_at(p, h), h)
                        };
                        let p = [x, y, z];
                        if mat(p).0 != AIR || z > mat(p).1 {
                            continue;
                        }
                        for (k, sgn) in [(0, -1), (0, 1), (1, -1), (1, 1), (2, -1), (2, 1)] {
                            let mut q = p;
                            q[k] += sgn;
                            if mat(q).0 == AIR {
                                continue;
                            }
                            faces += 1;
                            let mut n = [0.0f64; 3];
                            n[k] = -f64::from(sgn);
                            let aa = if n[0].abs() > 0.9 {
                                [0.0, 1.0, 0.0]
                            } else {
                                [1.0, 0.0, 0.0]
                            };
                            let cross = |a: [f64; 3], b: [f64; 3]| {
                                [
                                    a[1] * b[2] - a[2] * b[1],
                                    a[2] * b[0] - a[0] * b[2],
                                    a[0] * b[1] - a[1] * b[0],
                                ]
                            };
                            let t = cross(aa, n);
                            let b = cross(n, t);
                            let o = [0, 1, 2].map(|j| f64::from(p[j]) + 0.5 + n[j] * 0.49);
                            for _ in 0..64 {
                                let u1 = (rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64;
                                let u2 = (rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64;
                                let r = u1.sqrt();
                                let phi = 2.0 * std::f64::consts::PI * u2;
                                let cz = (1.0 - u1).max(0.0).sqrt();
                                let d = [0, 1, 2].map(|j| {
                                    t[j] * r * phi.cos() + b[j] * r * phi.sin() + n[j] * cz
                                });
                                rays += 1;
                                if d[2] <= 0.0 {
                                    continue;
                                }
                                // Walk in steps of 0.25 voxel (fine enough for a fate).
                                let mut len = 0.0;
                                let fate = loop {
                                    len += 0.25;
                                    let v = [0, 1, 2].map(|j| (o[j] + d[j] * len).floor() as i32);
                                    let (m, h) = mat(v);
                                    if m != AIR {
                                        break Some(len);
                                    }
                                    if v[2] > h + 1 {
                                        break None;
                                    }
                                    if len > 4096.0 {
                                        break Some(len);
                                    }
                                };
                                match fate {
                                    None => sky += 1,
                                    Some(l) => {
                                        if l > 256.0 {
                                            fake256 += 1;
                                        }
                                        if l > 512.0 {
                                            fake512 += 1;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            let pct = |v: usize| 100.0 * v as f64 / rays.max(1) as f64;
            println!(
                "reading: station {station} at {centre:?} (surface {}): {faces} faces, {rays} rays; true sky {:.3} %, counted sky by the 256 cap without reaching it {:.3} %, by the 512 cap {:.3} %",
                s.height(centre[0], centre[1]),
                pct(sky),
                pct(fake256),
                pct(fake512)
            );
        }
    }
}
