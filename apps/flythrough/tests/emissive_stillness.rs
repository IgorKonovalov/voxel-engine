//! Static emissive light holds still (ADR-0036), headless on the software adapter (lavapipe):
//! the level's ore veins with the lamp off, a still camera, and the light's tile patches,
//! sudden steps and flicker (`support::stillness`).

mod support;

use support::stillness::*;
use vxe_render::{HistoryKill, Lighting};

/// The off frame at which `the_vein_wall_patches_are_counted` halves the emission, inside
/// patch step 5 (off frames 270 to 300).
const HALVED_AT: usize = 285;

/// The harness sees light that really moves, on the step where it moves, and nothing where
/// nothing moves. With the emissive candidates off no step at either pose reads a patch. At the
/// patch pose, halving every emissive material's emission at off frame `HALVED_AT` moves an
/// emissive-lit tile by 1 log2 against the patch threshold of `log2 1.1`: steps 0 to 4, before
/// the change, read at most 8 patches each and step 5, which holds it, at least 250. The steps
/// after it are the cleared table refilling and carry no bar.
#[test]
fn the_vein_wall_patches_are_counted() {
    let Some(level) = vein_level() else {
        return;
    };
    for pose in [PATCH_POSE, VEIN_POSE] {
        let off = run(
            &level,
            &pose,
            Lighting {
                candidates: 0,
                ..Lighting::default()
            },
            SIZE,
        );
        off.print(&format!("{} candidates 0", pose.name));
        assert!(
            off.composite.patches.iter().all(|&p| p == 0),
            "{}: with no emissive candidates no step may read a patch: {:?}",
            pose.name,
            off.composite.patches
        );
    }
    let halved = run_halved_at(&level, &PATCH_POSE, Lighting::default(), SIZE, HALVED_AT);
    halved.print("patch halved");
    let patches = &halved.composite.patches;
    assert!(
        patches[..5].iter().all(|&p| p <= 8),
        "before the emission halves, no step may read over 8 patches: {patches:?}"
    );
    assert!(
        patches[5] >= 250,
        "the step that halves the emission reads at least 250 patches: {patches:?}"
    );
}

/// Static ore light holds still with the lamp off at the defaults, at both poses. The composite
/// averages at most 3 patches a step (1 % of the patch pose's 309 full tiles), none over 8, and
/// at most 1 sudden step a second, with a tile flicker of at most 0.002 and a display luma
/// flicker of at most 0.06/255.
#[test]
fn the_vein_wall_holds_still_with_the_lamp_off() {
    let Some(level) = vein_level() else {
        return;
    };
    for pose in [PATCH_POSE, VEIN_POSE] {
        let r = run(&level, &pose, Lighting::default(), SIZE);
        r.print(&format!("{} default", pose.name));
        let name = pose.name;
        let c = &r.composite;
        assert!(
            c.mean_patches() <= 3.0 && c.max_patches() <= 8,
            "{name}: composite patches {}",
            c.line()
        );
        assert!(
            c.sudden_per_s <= 1.0,
            "{name}: composite sudden steps {}",
            c.line()
        );
        assert!(
            c.flicker <= 0.002,
            "{name}: composite flicker {:.5} over 0.002",
            c.flicker
        );
        let l = c.luma_flicker.expect("the composite reads its luma");
        assert!(l <= 0.06, "{name}: luma flicker {l:.3} over 0.06");
    }
}

/// One variant of the sweep: a `;`-separated list in `STILL_SWEEP`, each a `,`-separated list
/// of `key=value`: `pose` (patch, vein or both; default both), `cand` (candidates), `read`
/// (the table's read, on or off), `kill` (edits or gradient), `size` (`WxH`), `probe_rays` and `probe_slices` (the probe
/// volume's rays per update and slices). Unset keys keep `Lighting::default()`.
fn variant(spec: &str) -> (Vec<Pose>, Lighting, [u32; 2]) {
    let mut lit = Lighting::default();
    let mut poses = vec![PATCH_POSE, VEIN_POSE];
    let mut size = SIZE;
    for kv in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let (k, v) = kv.split_once('=').expect("key=value");
        match k {
            "pose" => {
                poses = match v {
                    "patch" => vec![PATCH_POSE],
                    "vein" => vec![VEIN_POSE],
                    _ => vec![PATCH_POSE, VEIN_POSE],
                }
            }
            "cand" => lit.candidates = v.parse().expect("cand"),
            "kill" => lit.history = HistoryKill::from_name(v).expect("kill"),
            "probe_rays" => lit.probe_rays = v.parse().expect("probe_rays"),
            "probe_slices" => lit.probe_slices = v.parse().expect("probe_slices"),
            "size" => {
                let (w, h) = v.split_once('x').expect("WxH");
                size = [w.parse().expect("W"), h.parse().expect("H")];
            }
            _ => panic!("unknown sweep key {k}"),
        }
    }
    (poses, lit, size)
}

/// The statistics of each variant in `STILL_SWEEP` (see `variant`), printed as `reading:`
/// lines; with it unset, the default at both poses and at 384 x 216 at the patch pose.
#[test]
#[ignore = "a sweep: run by hand with STILL_SWEEP"]
fn emissive_patch_sweep() {
    let Some(level) = vein_level() else {
        return;
    };
    let specs = std::env::var("STILL_SWEEP").unwrap_or_else(|_| ";pose=patch,size=384x216".into());
    for spec in specs.split(';') {
        let (poses, lit, size) = variant(spec);
        for pose in poses {
            let name = format!(
                "{} [{spec}] {}x{} cand {} rays {} slices {}",
                pose.name,
                size[0],
                size[1],
                lit.candidates,
                lit.rays(),
                lit.slices()
            );
            run(&level, &pose, lit, size).print(&name);
        }
    }
}
