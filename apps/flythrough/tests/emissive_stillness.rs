//! Static emissive light holds still (ADR-0036), headless on the software adapter (lavapipe):
//! the level's ore veins with the lamp off, a still camera, and the light's tile patches,
//! sudden steps and flicker (`support::stillness`).

mod support;

use support::stillness::*;
use vxe_render::{HistoryKill, Lighting, Sampler};

/// The harness sees patches where they exist: none with the emissive candidates off at either
/// pose, at least 100 a step with a one-frame emissive history at the patch pose.
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

/// The probe gather's bars at one pose: its mean and largest patches a step, its sudden steps
/// per second and its tile flicker.
struct GatherBar {
    mean: f64,
    max: usize,
    sudden_per_s: f64,
    flicker: f64,
}

/// Static ore light holds still with the lamp off at the defaults, at both poses. The light the
/// gather does not explain (`Direct`) averages at most 3 patches a step (1 % of the patch pose's
/// 312 full tiles), none over 8, and at most 1 sudden step a second. The composite's tile
/// flicker is at most `sqrt(0.002^2 + G^2)` and its display luma's at most
/// `0.06 sqrt(1 + (G / 0.002)^2)` /255, `G` the gather's tile flicker in the same run: 0.002
/// and 0.06/255 for the direct light, the gather's own noise added in quadrature. The gather
/// itself stays within its bars: at the patch pose its readings with the probe slices spread
/// by a hash (8.15 a step, 44 at most, 21.0 sudden steps a second) plus 10 %, at `VEIN_POSE`
/// 1, 3 and 1; at both its flicker no higher than with the slices by `slot mod slices`
/// (0.00430 and 0.00341). The patch pose's composite stays under its 77.10 patches a step at
/// a 32-frame history.
#[test]
fn the_vein_wall_holds_still_with_the_lamp_off() {
    let Some(level) = vein_level() else {
        return;
    };
    for (pose, bar) in [
        (
            PATCH_POSE,
            GatherBar {
                mean: 9.0,
                max: 48,
                sudden_per_s: 23.0,
                flicker: 0.00430,
            },
        ),
        (
            VEIN_POSE,
            GatherBar {
                mean: 1.0,
                max: 3,
                sudden_per_s: 1.0,
                flicker: 0.00341,
            },
        ),
    ] {
        let r = run(&level, &pose, Lighting::default(), SIZE);
        r.print(&format!("{} default", pose.name));
        let name = pose.name;
        let d = &r.direct;
        assert!(
            d.mean_patches() <= 3.0 && d.max_patches() <= 8,
            "{name}: direct-attributed patches {}",
            d.line()
        );
        assert!(
            d.sudden_per_s <= 1.0,
            "{name}: direct-attributed sudden steps {}",
            d.line()
        );
        let g = r.gather.flicker;
        let flicker = (0.002f64.powi(2) + g * g).sqrt();
        let luma = 0.06 * (1.0 + (g / 0.002).powi(2)).sqrt();
        let c = &r.composite;
        assert!(
            c.flicker <= flicker,
            "{name}: composite flicker {:.5} over {flicker:.5}",
            c.flicker
        );
        let l = c.luma_flicker.expect("the composite reads its luma");
        assert!(l <= luma, "{name}: luma flicker {l:.3} over {luma:.3}");
        let ga = &r.gather;
        assert!(
            ga.mean_patches() <= bar.mean
                && ga.max_patches() <= bar.max
                && ga.sudden_per_s <= bar.sudden_per_s
                && ga.flicker <= bar.flicker,
            "{name}: gather {}",
            ga.line()
        );
        if name == PATCH_POSE.name {
            assert!(c.mean_patches() <= 77.10, "{name}: composite {}", c.line());
        }
    }
}

/// One variant of the sweep: a `;`-separated list in `STILL_SWEEP`, each a `,`-separated list
/// of `key=value`: `pose` (patch, vein or both; default both), `h` (the emissive history),
/// `cap` (the reuse cap), `sampler` (list or reuse), `cand` (candidates), `guard` (on or off),
/// `kill` (edits or gradient), `size` (`WxH`), `probe_rays` and `probe_slices` (the probe
/// volume's rays per update and slices), `trace` (on: the gather's per-frame trace, printed
/// around each gather step over 8 patches). Unset keys keep `Lighting::default()`.
fn variant(spec: &str) -> (Vec<Pose>, Lighting, [u32; 2], bool) {
    let mut lit = Lighting::default();
    let mut poses = vec![PATCH_POSE, VEIN_POSE];
    let mut size = SIZE;
    let mut trace = false;
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
            "guard" => lit.emissive_guard = v == "on",
            "probe_rays" => lit.probe_rays = v.parse().expect("probe_rays"),
            "probe_slices" => lit.probe_slices = v.parse().expect("probe_slices"),
            "trace" => trace = v == "on",
            "size" => {
                let (w, h) = v.split_once('x').expect("WxH");
                size = [w.parse().expect("W"), h.parse().expect("H")];
            }
            _ => panic!("unknown sweep key {k}"),
        }
    }
    (poses, lit, size, trace)
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
        let (poses, lit, size, traced) = variant(spec);
        for pose in poses {
            let name = format!(
                "{} [{spec}] {}x{} h {} cap {} {} cand {} guard {} rays {} slices {}",
                pose.name,
                size[0],
                size[1],
                lit.emissive_history,
                lit.reuse_m_cap,
                lit.sampler.name(),
                lit.candidates,
                lit.emissive_guard,
                lit.rays(),
                lit.slices()
            );
            if traced {
                let (r, trace) = run_traced(&level, &pose, lit, size);
                r.print(&name);
                print_trace(&name, &r.gather, &trace, 8);
            } else {
                run(&level, &pose, lit, size).print(&name);
            }
        }
    }
}
