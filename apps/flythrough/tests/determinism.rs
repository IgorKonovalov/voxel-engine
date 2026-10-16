//! The cross-platform determinism vector (`nfr.md` §6): a fixed cave, a fixed op list, one
//! hash over the voxels and every result, and a second over the mips of the edited chunks.
//! CI runs it on Linux and Windows; the constants were blessed on Linux and a mismatch on
//! either arm is a defect in an integer path.

use flythrough::cave::{CaveSource, SHELL};
use flythrough::cave_materials;
use vxe_world::hash::Fnv1a;
use vxe_world::*;

fn extent() -> ChunkExtent {
    ChunkExtent::new([0, 0, 0], [4, 4, 4])
}

fn generate(seed: u64) -> World {
    World::generate(
        extent(),
        &CaveSource::new(seed, extent()),
        &cave_materials(),
    )
}

/// 24 ops covering every shape and mode, the shell, tool-along-ray, and one op that fails on
/// a non-resident chunk.
fn ops(world: &World) -> Vec<EditOp> {
    let c = centre_of_voxel;
    let s = [128, 128, 128];
    let at = |d: [i32; 3]| c([0, 1, 2].map(|k| s[k] + d[k]));
    let tool = |d: [i64; 3], max_t: i64, r: i64, mode| {
        EditOp::tool_along_ray(world, c(s), d, max_t, r, mode)
    };
    vec![
        EditOp::sphere(at([0, 0, 35]), 10 * FIXED_ONE, Mode::Carve),
        EditOp::sphere(at([-40, 5, 0]), 7 * FIXED_ONE + 100, Mode::Carve),
        EditOp::box_(at([20, -10, -10]), at([50, 10, 10]), Mode::Carve),
        EditOp::capsule(at([0, 0, 0]), at([60, 60, 0]), 4 * FIXED_ONE, Mode::Carve),
        tool([256, 0, 0], 100 * 256, 5 * FIXED_ONE, Mode::Carve),
        tool([-200, 90, 31], 120 * 256, 3 * FIXED_ONE, Mode::Carve),
        tool([0, -256, 0], 10 * 256, 2 * FIXED_ONE, Mode::Carve),
        EditOp::sphere(at([0, 0, 0]), 6 * FIXED_ONE, Mode::Fill(1)),
        EditOp::box_(at([-35, -35, -35]), at([-20, -25, -28]), Mode::Fill(2)),
        EditOp::capsule(
            at([-10, 30, 0]),
            at([10, 30, 20]),
            3 * FIXED_ONE,
            Mode::Fill(2),
        ),
        EditOp::sphere(
            at([0, 0, 0]),
            40 * FIXED_ONE,
            Mode::Replace { from: 1, to: 2 },
        ),
        EditOp::box_(
            at([-60, -60, -60]),
            at([60, 60, 60]),
            Mode::Replace { from: 2, to: 1 },
        ),
        EditOp::capsule(
            at([-50, 0, 0]),
            at([50, 0, 0]),
            9 * FIXED_ONE,
            Mode::Replace { from: 0, to: 2 },
        ),
        // Into the shell at the low x face: shell voxels survive a carve.
        EditOp::sphere(c([6, 64, 64]), 6 * FIXED_ONE, Mode::Carve),
        EditOp::box_(c([0, 100, 100]), c([8, 110, 110]), Mode::Carve),
        // Replace ignores the shell's flag.
        EditOp::sphere(
            c([251, 200, 200]),
            3 * FIXED_ONE,
            Mode::Replace { from: SHELL, to: 1 },
        ),
        // Reaches chunk x = 4, which is not resident.
        EditOp::sphere(c([250, 128, 128]), 10 * FIXED_ONE, Mode::Carve),
        EditOp::sphere(c([3, 3, 3]), 2 * FIXED_ONE, Mode::Carve),
        EditOp::capsule(
            c([10, 10, 10]),
            c([245, 245, 245]),
            FIXED_ONE + 77,
            Mode::Carve,
        ),
        EditOp::box_(
            Fixed3([0; 3]),
            Fixed3([256 * 256; 3]),
            Mode::Replace { from: 2, to: 5 },
        ),
        EditOp::sphere(at([1, -2, 3]), 25 * FIXED_ONE, Mode::Fill(3)),
        EditOp::capsule(at([5, 5, 5]), at([5, 5, 5]), 12 * FIXED_ONE, Mode::Carve),
        EditOp::box_(at([-3, -3, -3]), at([4, 4, 4]), Mode::Fill(0)),
        EditOp::sphere(at([0, 0, 0]), -1, Mode::Carve),
    ]
}

fn hash_result(h: &mut Fnv1a, r: &Result<EditResult, EditError>) {
    match r {
        Ok(r) => {
            h.write(&[1]);
            h.write(&(r.changed_from.len() as u32).to_le_bytes());
            for &(m, n) in &r.changed_from {
                h.write(&m.to_le_bytes());
                h.write(&n.to_le_bytes());
            }
            h.write(&(r.touched.len() as u32).to_le_bytes());
            for c in &r.touched {
                for v in c.0 {
                    h.write(&v.to_le_bytes());
                }
            }
            match r.aabb {
                None => h.write(&[0]),
                Some((lo, hi)) => {
                    h.write(&[1]);
                    for v in lo.0.iter().chain(&hi.0) {
                        h.write(&v.to_le_bytes());
                    }
                }
            }
        }
        Err(EditError::NotResident(c)) => {
            h.write(&[2]);
            for v in c.0 {
                h.write(&v.to_le_bytes());
            }
        }
    }
}

#[test]
fn determinism_vector() {
    let world = generate(0x5EED_0001);
    let mut h = Fnv1a::default();
    h.write(&world.voxel_hash().to_le_bytes());
    let ops = ops(&world);
    assert_eq!(ops.len(), 24);
    let mut results = Vec::new();
    for op in &ops {
        let r = world.apply(op);
        hash_result(&mut h, &r);
        results.push(r);
    }
    h.write(&world.voxel_hash().to_le_bytes());

    // The vector exercises what it claims to.
    assert_eq!(
        results[16],
        Err(EditError::NotResident(ChunkCoord([4, 1, 1])))
    );
    assert_eq!(results.iter().filter(|r| r.is_err()).count(), 1);
    assert_eq!(world.material_at([1, 64, 64]), Some(SHELL));
    assert_eq!(world.material_at([2, 64, 64]), Some(AIR));
    assert_eq!(world.material_at([254, 200, 200]), Some(1));
    assert_eq!(world.material_at([255, 200, 200]), Some(SHELL));
    let changed = results
        .iter()
        .filter(|r| matches!(r, Ok(r) if !r.changed_from.is_empty()))
        .count();
    assert!(changed >= 18, "{changed} ops changed something");

    // Blessed on Linux x86_64. Never loosened to make another arm pass.
    assert_eq!(h.finish(), 0x2b6b_6a0d_1f9b_8d75);

    // The mips of every chunk of the edited world (`World::mips_hash`), the LOD-only tier's
    // contents. Blessed on Linux x86_64, asserted on every arm.
    let mips = world.mips_hash();
    println!("mips hash {mips:#018x}");
    assert_eq!(mips, 0x6cc1_4fda_4b7f_5188);
}

#[test]
fn same_seed_same_bytes_and_seeds_differ() {
    let chunk = |seed, c: [i32; 3]| {
        let mut out = Chunk::default();
        CaveSource::new(seed, extent()).chunk(ChunkCoord(c), &mut out);
        out
    };
    let coords = [[0, 0, 0], [1, 2, 3], [2, 2, 2], [3, 0, 1]];
    for c in coords {
        assert_eq!(chunk(1, c), chunk(1, c));
    }
    assert!(coords.iter().any(|&c| chunk(1, c) != chunk(2, c)));
}

/// Generation code paths stay integer: no floating-point type, transcendental call or float
/// literal in the cave or in the engine's hash and noise helpers.
#[test]
fn generation_sources_contain_no_floating_point() {
    let root = env!("CARGO_MANIFEST_DIR");
    for path in ["src/cave.rs", "../../crates/vxe-world/src/hash.rs"] {
        let text = std::fs::read_to_string(format!("{root}/{path}")).unwrap();
        for word in text.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_')) {
            assert!(
                !["f32", "f64", "sin", "exp", "powf"].contains(&word),
                "{path} contains {word:?}"
            );
        }
        let b = text.as_bytes();
        for w in b.windows(3) {
            assert!(
                !(w[0].is_ascii_digit() && w[1] == b'.' && w[2].is_ascii_digit()),
                "{path} contains a float literal near {:?}",
                String::from_utf8_lossy(w)
            );
        }
    }
}
