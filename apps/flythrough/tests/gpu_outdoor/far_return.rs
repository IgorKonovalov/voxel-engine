// `gpu_outdoor`'s ADR-0076 rule 2 at the far under ADR-0079's key light (Plan 0095 Phase 10),
// included by `gpu_outdoor.rs` (kept apart for the file-size cap).

/// The far class's share over 4 levels at the return frame of a turn away and back against the
/// settled image, for `arm` on [`support::wobble`]'s scene: 120 settle frames, the settled
/// frame, a 90-degree yaw over 10 frames, 64 held, back over 10, the queue stepping throughout
/// (no drain). Returns the share and the far class's share of the display at the return frame.
fn far_return(rig: &mut support::terms::Rig, cam: &svet_render::Camera, arm: support::wobble::Arm) -> (f64, f64) {
    use support::wobble::{FAR, frame, over, share, step, yawed};
    let mut f = frame(rig, arm);
    for _ in 0..120 {
        step(rig, &mut f, cam, true);
    }
    let settled = step(rig, &mut f, cam, true);
    for k in (1..=10).chain(std::iter::repeat_n(10, 64)).chain((1..10).rev()) {
        step(rig, &mut f, &yawed(cam, 9.0 * k as f32), true);
    }
    let ret = step(rig, &mut f, cam, true);
    let at = over(&settled.luma, &ret.luma, &ret.class, FAR);
    println!(
        "far return, {} on {}: {at:.2} % of the far pixels over {} ({:.1} % of the display far); {}",
        arm.name(),
        rig.adapter,
        support::wobble::T,
        share(&ret.class, FAR),
        support::wobble::depth(&f)
    );
    (at, share(&ret.class, FAR))
}

/// Plan 0095 Phase 10's rule 2 reading at the far (ADR-0076 rule 2, ADR-0079; a reading, not a
/// bar, as amended 2026-11-21): on [`support::wobble`]'s scene (a 3 x 3 x 2 chunk forest of seed
/// 1 at 320 x 180 through FSR 2 at 1.5, no drain), [`far_return`]'s share of far pixels over 4
/// levels at the return frame under the picks and under the before (`key far ray`, `penumbra
/// disc`), both printed. Asserted only that the scene has far pixels to read. Plan 0096's far
/// tiers and Plan 0090's counter take the scene.
#[test]
#[ignore = "soak"]
fn the_far_return_prints_against_the_before() {
    use support::wobble::Arm;
    let Some((mut rig, cam)) = support::wobble::forest() else {
        return;
    };
    let (picks, far_px) = far_return(&mut rig, &cam, Arm::Picks);
    let (before, _) = far_return(&mut rig, &cam, Arm::Before);
    println!("far return: picks {picks:.2} %, before {before:.2} %");
    assert!(far_px > 5.0, "far {far_px:.1} % of the display");
}
