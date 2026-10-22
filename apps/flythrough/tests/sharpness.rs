//! The sharpness harness on the software adapter (lavapipe): a rough rock wall and a convex
//! rock edge at about 1 m under the headlamp, read with a band-passed contrast metric and an
//! edge-spread width (`support/sharpness.rs`), the wall's face and contour goldens, and the
//! source-isolation matrix. `VXE_BLESS=1` rewrites the wall goldens. The side-by-side images
//! land in `target/look-sharpness/`.
//!
//! `cargo nextest run -p flythrough --test sharpness --no-capture` prints the tables.

mod support;

use support::sharpness::*;
use support::*;
use vxe_render::{Camera, LightingResolution, NormalMode, ViewMode};

/// Rock's detail in the harness: the fly-through's default pair.
const ROCK: [f32; 2] = flythrough::ROCK_DETAIL;

/// The wall mirrored with contour attributes on the software adapter, or `None` with a
/// printed skip.
fn wall_rig() -> Option<LookRig> {
    let spheres = wall_spheres();
    println!("wall spheres (centre voxel, radius, carve): {spheres:?}");
    attr_rig(&wall_world_with(sharp_table(ROCK)))
}

/// The fly-through's `face` and `contour` looks: detail on, the default tier, filter on.
fn face() -> Setting {
    Setting::new(NormalMode::Face, true)
}

fn contour() -> Setting {
    Setting::new(NormalMode::Contour, true)
}

/// Face mode against today's contour on the wall: both contrasts finite and above 0, and face
/// mode's mid band above contour's (the voxel-edge contrast contour removes by design).
#[test]
fn face_against_contour_on_the_wall() {
    let Some(mut rig) = wall_rig() else {
        return;
    };
    let cam = wall_camera();
    let f = rig.converge(&cam, face());
    let c = rig.converge(&cam, contour());
    let (f_fine, f_mid) = (s_fine(&f), s_mid(&f));
    let (c_fine, c_mid) = (s_fine(&c), s_mid(&c));
    println!("| look | S_fine | S_mid |");
    println!("|---|---|---|");
    println!("| {} | {f_fine:.5} | {f_mid:.5} |", face().name());
    println!("| {} | {c_fine:.5} | {c_mid:.5} |", contour().name());
    for v in [f_fine, f_mid, c_fine, c_mid] {
        assert!(v.is_finite() && v > 0.0, "a contrast reads {v}");
    }
    assert!(
        f_mid > c_mid,
        "face S_mid {f_mid:.5} against contour {c_mid:.5}"
    );
    let path = out_dir().join(format!(
        "wall_face_fine{f_fine:.4}_mid{f_mid:.4}_contour_fine{c_fine:.4}_mid{c_mid:.4}.png"
    ));
    write_wide_png(&path, &side_by_side(&[&f, &c]), 2);
    println!("wrote {path:?}");
    assert_golden("look_wall_face.png", &f);
    assert_golden("look_wall_contour.png", &c);
}

/// The edge scene's `view = Normals` render in `mode`, detail `detail`.
fn edge_normals(rig: &mut LookRig, mode: NormalMode, detail: bool) -> Vec<u8> {
    let look = vxe_render::LookParams {
        normal_mode: mode,
        detail,
        view: ViewMode::Normals,
        ..vxe_render::LookParams::default()
    };
    rig.render(&edge_camera(), &vxe_render::LightParams::headlamp(), look)
}

/// Edge spread on the convex edge, read on the normals view ([`spread_of`]): face mode's step
/// without detail is at most 1 px; the rest are printed in pixels and voxels.
#[test]
fn edge_spread() {
    let Some(mut rig) = attr_rig(&edge_world_with(sharp_table(ROCK))) else {
        return;
    };
    let mut rows = Vec::new();
    let mut images = Vec::new();
    for detail in [false, true] {
        for mode in [NormalMode::Face, NormalMode::Contour] {
            let img = edge_normals(&mut rig, mode, detail);
            let (e, n) = spread_of(&img);
            println!(
                "{}{}: E {e:.2} px = {:.3} voxel over {n} rows",
                mode.name(),
                if detail { " + detail" } else { "" },
                e / EDGE_PX_PER_VOXEL
            );
            rows.push((mode, detail, e));
            images.push(img);
        }
    }
    let refs: Vec<&[u8]> = images.iter().map(Vec::as_slice).collect();
    let path = out_dir().join(format!(
        "edge_normals_face_contour_then_with_detail_E{}.png",
        rows.iter()
            .map(|(.., e)| format!("{e:.2}"))
            .collect::<Vec<_>>()
            .join("_")
    ));
    write_wide_png(&path, &side_by_side(&refs), refs.len() as u32);
    println!("wrote {path:?}");
    let (_, _, face) = rows[0];
    assert!(face <= 1.0, "face: E {face:.2} px");
    for (mode, detail, e) in &rows {
        assert!(
            e.is_finite(),
            "{}, detail {detail}: no row has a step",
            mode.name()
        );
    }
}

/// `S_fine` and `S_mid` of one wall render.
#[derive(Clone, Copy)]
struct Reading {
    fine: f64,
    mid: f64,
}

/// The wall in `setting`.
fn read(rig: &mut LookRig, cam: &Camera, setting: Setting) -> Reading {
    let img = rig.converge(cam, setting);
    Reading {
        fine: s_fine(&img),
        mid: s_mid(&img),
    }
}

/// The source-isolation matrix: {face, contour} x {half, full} x {filter on, off} x {detail
/// on, off} on the wall, contour's detail survival `R`, the share of the face-to-contour
/// `S_fine` gap each of H1 to H4 recovers, and H6's normals-view contrast.
#[test]
fn source_matrix() {
    use LightingResolution::{Full, Half};
    let Some(mut rig) = wall_rig() else {
        return;
    };
    let cam = wall_camera();
    let mut table = Vec::new();
    for mode in [NormalMode::Face, NormalMode::Contour] {
        for tier in [Half, Full] {
            for filter in [true, false] {
                for detail in [true, false] {
                    let s = Setting::new(mode, detail).at(tier, filter);
                    table.push((s, read(&mut rig, &cam, s)));
                }
            }
        }
    }
    let get = |mode: NormalMode, tier: LightingResolution, filter: bool, detail: bool| {
        table
            .iter()
            .find(|(s, _)| {
                s.look.normal_mode == mode
                    && s.tier == tier
                    && s.filter == filter
                    && s.look.detail == detail
            })
            .map(|(_, r)| *r)
            .expect("the matrix holds every setting")
    };
    println!("| mode | detail | tier | filter | S_fine | S_mid |");
    println!("|---|---|---|---|---|---|");
    for (s, r) in &table {
        println!(
            "| {} | {} | {} | {} | {:.5} | {:.5} |",
            s.look.normal_mode.name(),
            if s.look.detail { "on" } else { "off" },
            s.tier.name(),
            if s.filter { "on" } else { "off" },
            r.fine,
            r.mid
        );
    }
    let (c, f) = (NormalMode::Contour, NormalMode::Face);
    let r = get(c, Half, true, true).fine / get(c, Full, false, true).fine;
    let r_face = get(f, Half, true, true).fine / get(f, Full, false, true).fine;
    println!("detail survival R: contour {r:.3}, face {r_face:.3}");
    let gap = get(f, Half, true, true).fine - get(c, Half, true, true).fine;
    let share = |v: f64| v / gap * 100.0;
    println!("face-to-contour S_fine gap at the default (half, filter on, detail on): {gap:.5}");
    println!(
        "H1 half -> full tier (contour, filter off, detail on): {:+.1} % of the gap",
        share(get(c, Full, false, true).fine - get(c, Half, false, true).fine)
    );
    println!(
        "H2 filter on -> off (contour, full tier, detail on): {:+.1} % of the gap",
        share(get(c, Full, false, true).fine - get(c, Full, true, true).fine)
    );
    println!(
        "H3 detail off -> on (contour, half, filter on): {:+.1} % of the gap",
        share(get(c, Half, true, true).fine - get(c, Half, true, false).fine)
    );
    println!(
        "H4 contour -> face (detail off, full tier, filter off): {:+.1} % of the gap",
        share(get(f, Full, false, false).fine - get(c, Full, false, false).fine)
    );
    // Not a source the plan names: at 1 m the headlamp puts rock facing it on the ACES
    // curve's shoulder (radiance about 5.6 displays at about 0.99), which flattens shading
    // contrast. The default pair at a quarter of the exposure, for the record.
    let quarter = [face(), contour()].map(|s| {
        read(
            &mut rig,
            &cam,
            Setting {
                exposure: 0.25,
                ..s
            },
        )
    });
    println!(
        "exposure 0.25 at the default: face S_fine {:.5}, S_mid {:.5}; contour S_fine {:.5}, S_mid {:.5}",
        quarter[0].fine, quarter[0].mid, quarter[1].fine, quarter[1].mid
    );
    for detail in [false, true] {
        let mut s = [0.0; 2];
        for (k, mode) in [f, c].into_iter().enumerate() {
            let look = vxe_render::LookParams {
                normal_mode: mode,
                detail,
                view: ViewMode::Normals,
                ..vxe_render::LookParams::default()
            };
            let img = rig.render(&cam, &vxe_render::LightParams::headlamp(), look);
            s[k] = s_fine(&img);
        }
        println!(
            "H6 normals view S_fine, detail {}: face {:.5}, contour {:.5}",
            if detail { "on" } else { "off" },
            s[0],
            s[1]
        );
    }
    for (s, r) in &table {
        assert!(
            r.fine.is_finite() && r.fine > 0.0 && r.mid.is_finite() && r.mid > 0.0,
            "{}: S_fine {}, S_mid {}",
            s.name(),
            r.fine,
            r.mid
        );
    }
}
