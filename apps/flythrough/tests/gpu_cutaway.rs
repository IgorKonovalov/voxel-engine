//! The cutaway walk on the software adapter (lavapipe): a camera buried in rock or outside
//! the world sees through the rock to the far wall of the first cavern, and rock with no
//! cavern behind it is a miss.

mod support;

use support::*;
use vxe_render::{LightParams, LookParams, ViewMode};

/// A face-normal look in `view`, cutaway as given.
fn look(view: ViewMode, cutaway: bool) -> LookParams {
    LookParams {
        view,
        cutaway,
        ..LookParams::default()
    }
}

/// The cavity's far wall at x = 41 faces -x: `n * 0.5 + 0.5` of `(-1, 0, 0)`.
const FAR_WALL: [u8; 3] = [0, 128, 128];
const WHITE: [u8; 3] = [255, 255, 255];
const BLACK: [u8; 3] = [0, 0, 0];

fn assert_near(got: [u8; 3], want: [u8; 3], tolerance: u8, what: &str) {
    for k in 0..3 {
        assert!(
            got[k].abs_diff(want[k]) <= tolerance,
            "{what}: channel {k}: {got:?} against {want:?}"
        );
    }
}

/// The centre pixel of the `normals` and `hitmask` views of `camera` over `rig`.
fn centre(rig: &mut LookRig, position: [f32; 3], cutaway: bool) -> ([u8; 3], [u8; 3]) {
    let cam = camera_looking_x(position);
    let n = centre_pixel(&rig.render(
        &cam,
        &LightParams::at_eye(),
        look(ViewMode::Normals, cutaway),
    ));
    let m = centre_pixel(&rig.render(
        &cam,
        &LightParams::at_eye(),
        look(ViewMode::HitMask, cutaway),
    ));
    println!("camera {position:?}, cutaway {cutaway}: normals {n:?}, hitmask {m:?}");
    (n, m)
}

/// Buried in rock at x = -100 and outside the extent at x = -200 (absent chunks), the
/// cutaway ray reaches the cavity's far wall facing the camera; a ray at y = 60, which
/// crosses rock only, misses.
#[test]
fn a_camera_in_rock_or_outside_sees_the_cavity_far_wall() {
    let world = cavity_world();
    let Some(mut rig) = LookRig::new(&world, None) else {
        return;
    };
    for position in [[-100.0, 0.5, 0.5], [-200.0, 0.5, 0.5]] {
        let (n, m) = centre(&mut rig, position, true);
        assert_near(n, FAR_WALL, 2, "normals");
        assert_eq!(m, WHITE, "hitmask");
    }
    let (_, m) = centre(&mut rig, [-200.0, 60.5, 0.5], true);
    assert_eq!(m, BLACK, "a ray through rock only misses");
}

/// With cutaway off the outside camera's centre ray hits the hull's -x face (or starts
/// inside it): its normal reads `(0, 128, 128)` or black. That centre pixel can equal the
/// cutaway reading (the hull's face and the far wall both face -x), so the difference is
/// asserted over the whole `normals` view, where the cutaway shows the cavity's curved far
/// wall inside a ring of misses and the hull is one flat face.
#[test]
fn without_cutaway_the_outside_camera_meets_the_hull() {
    let world = cavity_world();
    let Some(mut rig) = LookRig::new(&world, None) else {
        return;
    };
    let position = [-200.0, 0.5, 0.5];
    let (on_n, on_m) = centre(&mut rig, position, true);
    let (off_n, off_m) = centre(&mut rig, position, false);
    assert!(
        off_n.iter().zip(FAR_WALL).all(|(a, b)| a.abs_diff(b) <= 2) || off_n == BLACK,
        "normals {off_n:?}"
    );
    println!(
        "centre pixel differs from cutaway: normals {}, hitmask {}",
        on_n != off_n,
        on_m != off_m
    );
    let cam = camera_looking_x(position);
    let on = rig.render(&cam, &LightParams::at_eye(), look(ViewMode::Normals, true));
    let off = rig.render(&cam, &LightParams::at_eye(), look(ViewMode::Normals, false));
    let f = fraction_differing(&on, &off, 2);
    println!(
        "normals view: {:.2} % of pixels differ between cutaway on and off",
        f * 100.0
    );
    assert!(
        f > 0.0,
        "the cutaway changes the outside camera's normals view"
    );
}
