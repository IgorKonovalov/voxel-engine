//! The edit vectors the CPU tests share (`determinism.rs`, `replay.rs`, `component.rs`): the
//! determinism vector's ops, the hash of an op's result and the replay's dig inputs. Taken by
//! `#[path]`, apart from the GPU fixtures of `mod.rs`.
#![allow(dead_code, reason = "shared across test binaries; each uses a subset")]

use flythrough::cave::SHELL;
use flythrough::logic::DigInput;
use vxe_world::hash::Fnv1a;
use vxe_world::*;

/// 24 ops covering every shape and mode, the shell, tool-along-ray, and one op that fails on
/// a non-resident chunk.
pub fn vector_ops(world: &World) -> Vec<EditOp> {
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

/// Feeds one op's result to `h`: the changed counts, the touched chunks and the box of an
/// applied op, or the error's tag and fields.
pub fn hash_result(h: &mut Fnv1a, r: &Result<EditResult, EditError>) {
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
        Err(EditError::UnknownPrefab(id)) => {
            h.write(&[3]);
            h.write(&id.to_le_bytes());
        }
        Err(e @ EditError::OutOfRange { .. }) => {
            let mut bytes = Vec::new();
            vxe_world::wire::encode_error(&mut bytes, e);
            h.write(&[4]);
            h.write(&bytes);
        }
    }
}

/// The replay fixture of `tests/replay.rs`: twenty digs from the spawn pocket's centre along
/// ten fixed rays, each ray twice, so the second dig's raycast depends on the first's carve.
pub fn dig_inputs() -> Vec<DigInput> {
    let dirs: [[i64; 3]; 10] = [
        [256, 0, 0],
        [-256, 0, 0],
        [0, 256, 0],
        [0, -256, 0],
        [0, 0, 256],
        [0, 0, -256],
        [181, 181, 0],
        [-148, 148, 148],
        [100, -200, 120],
        [-30, -60, 247],
    ];
    (0..20)
        .map(|i| DigInput::Dig {
            origin: centre_of_voxel([128, 128, 128]),
            dir: dirs[i % 10],
            radius: (3 + (i as i64 % 3)) * FIXED_ONE,
        })
        .collect()
}
