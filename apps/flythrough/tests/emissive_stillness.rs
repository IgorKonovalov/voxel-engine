//! Static emissive light holds still (ADR-0036), headless on the software adapter (lavapipe):
//! the level's ore veins with the lamp off, a still camera, and the light's tile patches,
//! sudden steps and flicker (`support::stillness`).

mod support;

use support::stillness::*;
use vxe_render::{HistoryKill, Lighting, Sampler};

/// The harness sees patches where they exist: none with the emissive candidates off at either
/// pose, at least 100 a step with a one-frame emissive history at the patch pose. The default
/// lighting's statistics at both poses are printed as `reading:` lines.
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
        let default = run(&level, &pose, Lighting::default(), SIZE);
        default.print(&format!("{} default", pose.name));
    }
    let short = run(
        &level,
        &PATCH_POSE,
        Lighting {
            emissive_history: 1,
            ..Lighting::default()
        },
        SIZE,
    );
    short.print("patch history 1");
    assert!(
        short.composite.mean_patches() >= 100.0,
        "a one-frame emissive history reads at least 100 patches a step at the patch pose: {:.2}",
        short.composite.mean_patches()
    );
}

/// One variant of the sweep: a `;`-separated list in `STILL_SWEEP`, each a `,`-separated list
/// of `key=value`: `pose` (patch, vein or both; default both), `h` (the emissive history),
/// `cap` (the reuse cap), `sampler` (list or reuse), `cand` (candidates), `guard` (on or off),
/// `kill` (edits or gradient), `size` (`WxH`). Unset keys keep `Lighting::default()`.
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
            "h" => lit.emissive_history = v.parse().expect("h"),
            "cap" => lit.reuse_m_cap = v.parse().expect("cap"),
            "cand" => lit.candidates = v.parse().expect("cand"),
            "sampler" => lit.sampler = Sampler::from_name(v).expect("sampler"),
            "kill" => lit.history = HistoryKill::from_name(v).expect("kill"),
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
            let r = run(&level, &pose, lit, size);
            r.print(&format!(
                "{} [{spec}] {}x{} h {} cap {} {} cand {}",
                pose.name,
                size[0],
                size[1],
                lit.emissive_history,
                lit.reuse_m_cap,
                lit.sampler.name(),
                lit.candidates
            ));
        }
    }
}
