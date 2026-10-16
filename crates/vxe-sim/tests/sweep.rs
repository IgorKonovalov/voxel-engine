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
