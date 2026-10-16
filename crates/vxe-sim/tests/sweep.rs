//! The distance, overlap and sweep contracts on the F1 and F2 fixtures.

mod support;

use support::*;
use vxe_sim::*;
use vxe_world::*;

fn src_files() -> Vec<(String, String)> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_some_and(|x| x == "rs") {
            out.push((
                p.display().to_string(),
                std::fs::read_to_string(&p).unwrap(),
            ));
        }
    }
    assert!(!out.is_empty());
    out
}

#[test]
fn no_float_type_in_the_crate_source() {
    for (path, text) in src_files() {
        for word in text.split(|c: char| !c.is_ascii_alphanumeric() && c != '_') {
            assert!(word != "f32" && word != "f64", "{path} names {word}");
        }
    }
}

#[test]
fn depends_on_vxe_world_only() {
    let manifest = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"),
    )
    .unwrap();
    let mut section = "";
    let mut deps = Vec::new();
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            section = line;
        } else if section == "[dependencies]" && !line.is_empty() && !line.starts_with('#') {
            deps.push(line.split(['=', '.']).next().unwrap().trim().to_owned());
        }
    }
    assert_eq!(deps, ["vxe-world"]);
}

#[test]
fn walking_into_a_wall_stops_in_contact_with_an_outward_normal() {
    let w = f2();
    let s = sweep(&w, &body_at(S), Fixed3([10000, 0, 0]));
    assert_eq!(s.moved, Fixed3([1664, 0, 0]));
    assert_eq!(S[0] + s.moved.0[0], 4352);
    let c = s.contact.expect("contact");
    assert_eq!(c.voxel, [20, 10, 0]);
    assert_eq!(c.normal, [-768, 0, 0]);
    assert_eq!(c.material, ROCK);
    assert!(c.is_wall());
    assert!(s.iterations <= 2, "{} iterations", s.iterations);
    assert!(!s.started_overlapping);
}

#[test]
fn a_diagonal_approach_creeps_to_the_wall_without_overlap() {
    let w = f2();
    let s = sweep(&w, &body_at(S), Fixed3([10000, 10000, 0]));
    let [mx, my, mz] = s.moved.0;
    assert_eq!(mx, my);
    assert_eq!(mz, 0);
    assert!((1662..=1664).contains(&mx), "moved {mx}");
    assert!(s.contact.is_some());
    assert!(!overlaps(&w, &body_at([S[0] + mx, S[1] + my, 0])));
}

#[test]
fn falling_onto_the_floor_lands_on_the_voxel_under_the_axis() {
    let w = f1();
    let s = sweep(&w, &body_at([2688, 2688, 2560]), Fixed3([0, 0, -10000]));
    assert_eq!(s.moved, Fixed3([0, 0, -2560]));
    let c = s.contact.expect("contact");
    assert_eq!(c.voxel, [10, 10, -1]);
    assert_eq!(c.normal, [0, 0, 768]);
    assert!(c.is_floor());
}

#[test]
fn rising_through_open_air_spends_the_whole_delta() {
    let w = f1();
    let s = sweep(&w, &body_at(S), Fixed3([0, 0, 10000]));
    assert_eq!(s.moved, Fixed3([0, 0, 10000]));
    assert_eq!(s.contact, None);
    assert!(!s.started_overlapping);
}

#[test]
fn a_body_that_starts_overlapping_does_not_move() {
    let w = f1();
    let b = body_at([2688, 2688, -256]);
    assert!(overlaps(&w, &b));
    for delta in [
        [0, 0, 0],
        [1000, 0, 0],
        [-1000, 500, 0],
        [0, 0, 1000],
        [0, 0, -1000],
        [300, -300, 700],
    ] {
        let s = sweep(&w, &b, Fixed3(delta));
        assert_eq!(s.moved, Fixed3([0, 0, 0]), "{delta:?}");
        assert!(s.started_overlapping, "{delta:?}");
        assert_eq!(s.contact.expect("contact").voxel, [10, 10, -1], "{delta:?}");
    }
}

/// F1 with the floor bricks `z = -1` around the origin forced to `Full` bricks of equal
/// values: the same voxels, read through voxel arrays.
fn f1_expanded() -> World {
    let w = f1();
    for c in [[-1, -1, -1], [0, -1, -1], [-1, 0, -1], [0, 0, -1]] {
        let chunk = w.store().get(ChunkCoord(c)).unwrap();
        let mut chunk = chunk.write().unwrap();
        for slot in 0..CHUNK_BRICKS {
            if brick_of_slot(slot)[2] == 7 {
                chunk.set_brick(slot, Brick::Full(Box::new([ROCK; BRICK_VOXELS])));
            }
        }
    }
    w
}

#[test]
fn a_uniform_brick_is_one_box_with_the_distance_of_its_voxels() {
    let (uniform, expanded) = (f1(), f1_expanded());
    // Above the corner where four floor bricks meet, and above one brick's interior.
    for foot in [
        [2048, 2048, 128],
        [2048, 2100, 0],
        [-2048, 1000, 37],
        [2688, 2688, 200],
    ] {
        let b = body_at(foot);
        let (du, su) = distance2_with_stats(&uniform, &b);
        let (de, se) = distance2_with_stats(&expanded, &b);
        assert!(du.is_some(), "{foot:?}");
        assert_eq!(du, de, "{foot:?}");
        assert_eq!(su.voxel_arrays_read, 0, "{foot:?}");
        assert!(se.voxel_arrays_read > 0, "{foot:?}");
    }
    let (d, c) = distance2(&uniform, &body_at([2048, 2048, 128])).unwrap();
    assert_eq!(d, 128 * 128);
    assert_eq!(c.voxel, [8, 8, -1]);
}

#[test]
fn the_sweep_reads_no_voxel_array_of_a_uniform_brick() {
    let w = f1();
    let (s, stats) = sweep_with_stats(&w, &body_at([2048, 2048, 2560]), Fixed3([0, 0, -10000]));
    assert_eq!(s.moved, Fixed3([0, 0, -2560]));
    assert_eq!(stats.voxel_arrays_read, 0);
    let (s, stats) = sweep_with_stats(
        &f1_expanded(),
        &body_at([2048, 2048, 2560]),
        Fixed3([0, 0, -10000]),
    );
    assert_eq!(s.moved, Fixed3([0, 0, -2560]));
    assert!(stats.voxel_arrays_read > 0);
}

/// A pillar one voxel square, `x, y in [20, 21)`, all `z >= 0`, on the F1 floor.
fn pillar() -> World {
    world(|[x, y, z]| z < 0 || (x == 20 && y == 20))
}

/// The axis `(763, 95)` units from the pillar's `(-x, -y)` vertical edge: 768.89 units in
/// xy, which floors to the radius, so the pillar is at `d² == 0` without overlapping. On
/// the seed-1 level a falling body reached such a pose beside a voxel edge and no sweep moved
/// it again, in any direction.
const BESIDE_EDGE: [i64; 2] = [5120 - 763, 5120 - 95];

#[test]
fn a_box_under_one_unit_away_stops_only_a_move_that_closes_on_it() {
    let w = pillar();
    let b = body_at([BESIDE_EDGE[0], BESIDE_EDGE[1], 1000]);
    assert!(!overlaps(&w, &b));
    assert_eq!(
        distance2(&w, &b).map(|(d, c)| (d, c.voxel)),
        Some((0, VoxelCoord([20, 20, 3])))
    );

    // Down past it onto the floor.
    let s = sweep(&w, &b, Fixed3([0, 0, -2000]));
    assert_eq!(s.moved, Fixed3([0, 0, -1000]));
    assert!(s.contact.expect("floor").is_floor());

    // Away from it, along its tangent, and up: the whole delta, no contact.
    for delta in [
        [-128, 0, 0],
        [0, -128, 0],
        [-90, -90, 0],
        [-95, 763, 0],
        [95, -763, 0],
        [0, 0, 500],
        [-100, 20, 300],
    ] {
        let s = sweep(&w, &b, Fixed3(delta));
        assert_eq!(s.moved, Fixed3(delta), "{delta:?}");
        assert_eq!(s.contact, None, "{delta:?}");
    }

    // Into it: no motion, the pillar is the contact.
    for delta in [[128, 16, 0], [128, 128, 0], [16, 128, -40]] {
        let s = sweep(&w, &b, Fixed3(delta));
        assert_eq!(s.moved, Fixed3([0, 0, 0]), "{delta:?}");
        let c = s.contact.expect("contact");
        assert_eq!([c.voxel.0[0], c.voxel.0[1]], [20, 20], "{delta:?}");
        assert!(c.is_wall(), "{delta:?}");
    }
}

/// One unit further out (`(763, 100)`: 769.52 units, `g == 1`), every oblique advance rounds
/// to zero on every component; a move away from the pillar must not stop on that.
#[test]
fn an_oblique_move_away_from_a_box_one_unit_off_spends_its_delta() {
    let w = pillar();
    let b = body_at([5120 - 763, 5120 - 100, 1000]);
    assert_eq!(distance2(&w, &b).map(|(d, _)| d), Some(1));
    for delta in [[-90, -90, 0], [-120, 40, 0], [-30, 120, 0]] {
        let s = sweep(&w, &b, Fixed3(delta));
        assert_eq!(s.moved, Fixed3(delta), "{delta:?}");
        assert_eq!(s.contact, None, "{delta:?}");
    }
    // Toward it, the advance rounds to zero and the pillar is the contact (ADR-0005's creep).
    let s = sweep(&w, &b, Fixed3([90, 90, 0]));
    assert_eq!(s.moved, Fixed3([0, 0, 0]));
    assert_eq!(
        s.contact.map(|c| [c.voxel.0[0], c.voxel.0[1]]),
        Some([20, 20])
    );
}

/// Short moves at every angle toward the pillar, from 1 to 24 units off its face and off its
/// edge: the sweep never ends inside it. With `|remaining|` rounded down, a remainder longer
/// than the gap by under one unit passed for the gap and was taken whole: on the seed-1 level
/// a remainder of `(-4, -10)` (10.77 units) against a gap of 10 ended inside a voxel.
#[test]
fn a_short_move_never_ends_inside_a_box() {
    let w = pillar();
    let mut ends = 0;
    for gap in 1..=24i64 {
        let k = isqrt(((RADIUS + gap) * (RADIUS + gap) / 2) as u128) as i64 + 1;
        for axis in [[5120 - RADIUS - gap, 5248], [5120 - k, 5120 - k]] {
            let b = body_at([axis[0], axis[1], 0]);
            assert!(!overlaps(&w, &b));
            for dx in -24..=24 {
                for dy in -24..=24 {
                    let s = sweep(&w, &b, Fixed3([dx, dy, 0]));
                    let f = [axis[0] + s.moved.0[0], axis[1] + s.moved.0[1], 0];
                    assert!(
                        !overlaps(&w, &body_at(f)),
                        "gap {gap} from {axis:?} by {dx}, {dy}"
                    );
                    ends += 1;
                }
            }
        }
    }
    assert_eq!(ends, 24 * 2 * 49 * 49);
}
