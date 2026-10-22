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
use vxe_render::{GpuWorld, MirrorConfig, PlaneFit, PlaneOffset, SurfaceFit, SurfaceTuning, wgpu};
use vxe_world::MaterialTable;

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
    // Beside H1 to H6: at 1 m the headlamp puts rock facing it on the ACES
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

// ---- The candidate mechanisms ----

/// One candidate of the sweep: the plane fit, the face mix, the detail after lighting, and
/// rock's detail pair and fine-octave weight.
#[derive(Clone, Debug)]
struct Candidate {
    name: String,
    fit: SurfaceFit,
    face_mix: f32,
    after: bool,
    rock: [f32; 2],
    fine: f32,
}

impl Candidate {
    /// Today: every switch off, rock at the fly-through's pair.
    fn today() -> Self {
        Self {
            name: "today".to_owned(),
            fit: SurfaceFit::default(),
            face_mix: 0.0,
            after: false,
            rock: ROCK,
            fine: 0.0,
        }
    }

    fn tuning(&self) -> SurfaceTuning {
        SurfaceTuning {
            fit: self.fit,
            face_mix: self.face_mix,
            detail_after_lighting: self.after,
        }
    }

    /// The contour look with this candidate's tuning, detail `detail`, in `view`.
    fn look(&self, detail: bool, view: ViewMode) -> vxe_render::LookParams {
        vxe_render::LookParams {
            normal_mode: NormalMode::Contour,
            detail,
            view,
            tuning: self.tuning(),
            ..vxe_render::LookParams::default()
        }
    }

    /// The contour look with detail at `tier` and `filter`.
    fn setting(&self, tier: LightingResolution, filter: bool) -> Setting {
        Setting {
            look: self.look(true, ViewMode::Shaded),
            ..Setting::new(NormalMode::Contour, true).at(tier, filter)
        }
    }
}

/// [`sharp_table`] at `rock` with rock's fine-octave weight `fine`.
fn table_with(rock: [f32; 2], fine: f32) -> MaterialTable {
    let base = sharp_table(rock);
    let mut t = MaterialTable::new();
    for (id, row) in base.rows().iter().enumerate().skip(1) {
        let mut row = row.clone();
        if id == usize::from(flythrough::cave::ROCK) {
            row.detail_fine = fine;
        }
        t.push(row);
    }
    t
}

/// `world` mirrored with contour attributes under `fit` on `device`, filled; lighting at the
/// default tier.
fn fit_rig(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    world: &vxe_world::World,
    fit: SurfaceFit,
) -> LookRig {
    let mut gpu = GpuWorld::with_surface_attrs(
        device,
        MirrorConfig::new(vxe_render::Window::Fixed(world.extent())),
    );
    gpu.set_surface_fit(fit);
    gpu.sync_now(world, device, queue)
        .expect("mirror fits the adapter");
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    let bricks = gpu.update_attrs(&mut enc);
    queue.submit([enc.finish()]);
    assert_eq!(bricks, world.stats().full_bricks);
    LookRig {
        device: device.clone(),
        queue: queue.clone(),
        gpu,
        frame: vxe_render::Frame::new(device, WIDTH, HEIGHT),
    }
}

/// A software device large enough for every scene of the sweep; `None` with a printed skip.
fn sweep_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let pool = attr_pool_bytes(&cavity_world()).max(64 << 20);
    software_device(pool)
}

/// The guardrails of the "not a Lego look" bar under `c`'s fit and face mix, undetailed: the
/// cavity's normal error (degrees) and axis fraction, and the pillar's silhouette error.
fn guardrails(device: &wgpu::Device, queue: &wgpu::Queue, c: &Candidate) -> (f64, f64, usize) {
    let cavity = cavity_world();
    let mut rig = fit_rig(device, queue, &cavity, c.fit);
    rig.frame.set_resolution(device, LightingResolution::Full);
    let cam = cavity_camera();
    let normals = rig.render(
        &cam,
        &vxe_render::LightParams::at_eye(),
        c.look(false, ViewMode::Normals),
    );
    let error = cavity_normal_error(&normals, &cam);
    let axis = axis_fraction(&normals);
    let pillar = pillar_world();
    let mut rig = fit_rig(device, queue, &pillar, c.fit);
    let cam = pillar_camera();
    let mask = rig.render(
        &cam,
        &vxe_render::LightParams::at_eye(),
        c.look(false, ViewMode::HitMask),
    );
    (error, axis, pillar_silhouette_error(&mask, &cam))
}

/// Face mode's pillar silhouette error, the silhouette guardrail's reference.
fn face_silhouette(device: &wgpu::Device, queue: &wgpu::Queue) -> usize {
    let pillar = pillar_world();
    let mut rig = LookRig::on(device.clone(), queue.clone(), &pillar, None);
    let cam = pillar_camera();
    let look = vxe_render::LookParams {
        view: ViewMode::HitMask,
        ..vxe_render::LookParams::default()
    };
    let mask = rig.render(&cam, &vxe_render::LightParams::at_eye(), look);
    pillar_silhouette_error(&mask, &cam)
}

/// One row of the sweep.
struct SweepRow {
    fine: f64,
    mid: f64,
    e: f64,
    r: f64,
    error: f64,
    silhouette: usize,
    axis: f64,
}

/// `c` read on every metric of the sweep.
fn sweep_row(device: &wgpu::Device, queue: &wgpu::Queue, c: &Candidate) -> SweepRow {
    use LightingResolution::{Full, Half};
    let wall = wall_world_with(table_with(c.rock, c.fine));
    let mut rig = fit_rig(device, queue, &wall, c.fit);
    let cam = wall_camera();
    let default = read(&mut rig, &cam, c.setting(Half, true));
    let full = read(&mut rig, &cam, c.setting(Full, false));
    let edge = edge_world_with(table_with(c.rock, c.fine));
    let mut rig = fit_rig(device, queue, &edge, c.fit);
    let normals = rig.render(
        &edge_camera(),
        &vxe_render::LightParams::headlamp(),
        c.look(false, ViewMode::Normals),
    );
    let (error, axis, silhouette) = guardrails(device, queue, c);
    SweepRow {
        fine: default.fine,
        mid: default.mid,
        e: spread_of(&normals).0,
        r: default.fine / full.fine,
        error,
        silhouette,
        axis,
    }
}

/// C5 is exact for the lamp: on the wall at the full tier with the filter off and no fine
/// octave, detail after lighting matches today's render within 2/255 per channel at 98.4 % of
/// the window's pixels; at the default tier its detail survival `R` is printed beside today's.
/// The misses are printed in two classes read from the images: C5 unlit where today is lit
/// (the undetailed normal faces away from the lamp, so the light pass stores 0 and no ratio
/// recovers it), and C5 lit but darker than today (the ratio's clamp at 2, or its floor on
/// `n.l`, binds).
#[test]
fn detail_after_lighting_is_exact_for_the_lamp() {
    use LightingResolution::{Full, Half};
    let Some((device, queue)) = sweep_device() else {
        return;
    };
    let wall = wall_world_with(sharp_table(ROCK));
    let mut rig = fit_rig(&device, &queue, &wall, SurfaceFit::default());
    let cam = wall_camera();
    let today = Candidate::today();
    let c5 = Candidate {
        name: "C5".to_owned(),
        after: true,
        ..Candidate::today()
    };
    let a = rig.converge(&cam, today.setting(Full, false));
    let b = rig.converge(&cam, c5.setting(Full, false));
    let w = WIDTH as usize;
    let [x0, x1, y0, y1] = WINDOW;
    let (mut within, mut n, mut worst) = (0usize, 0usize, 0u8);
    let (mut unlit, mut darker) = (0usize, 0usize);
    for y in y0..y1 {
        for x in x0..x1 {
            let i = (y * w + x) * 4;
            let d = (0..3).map(|k| a[i + k].abs_diff(b[i + k])).max().unwrap();
            worst = worst.max(d);
            within += usize::from(d <= 2);
            n += 1;
            if d > 2 {
                let lum = |p: &[u8]| -> u32 { p[i..i + 3].iter().map(|&c| u32::from(c)).sum() };
                if b[i..i + 3].iter().all(|&c| c <= 2) {
                    unlit += 1;
                } else if lum(&b) < lum(&a) {
                    darker += 1;
                }
            }
        }
    }
    let pct = |k: usize| k as f64 / n as f64 * 100.0;
    let share = within as f64 / n as f64;
    println!(
        "C5 against today, full tier, filter off: {:.3} % of the window within 2/255 (worst {worst}/255); misses: {:.3} % unlit in C5 and lit today (undetailed normal faces away), {:.3} % lit and darker in C5 (the ratio's clamp binds), {:.3} % other",
        share * 100.0,
        pct(unlit),
        pct(darker),
        pct(n - within - unlit - darker),
    );
    assert!(share >= 0.984, "{:.3} %", share * 100.0);
    for c in [&today, &c5] {
        let half = read(&mut rig, &cam, c.setting(Half, true));
        let full = read(&mut rig, &cam, c.setting(Full, false));
        println!(
            "{}: S_fine default {:.5}, full + filter off {:.5}, R {:.3}",
            c.name,
            half.fine,
            full.fine,
            half.fine / full.fine
        );
    }
}

/// Every candidate alone, and rock's detail pairs with and without C5, against the metrics and
/// the guardrails, in one table: `cargo nextest run -p flythrough --test sharpness
/// --no-capture candidate_sweep`.
#[test]
fn candidate_sweep() {
    let Some((device, queue)) = sweep_device() else {
        return;
    };
    let today = Candidate::today();
    let fit = |name: &str, fit: PlaneFit, offset: PlaneOffset| Candidate {
        name: name.to_owned(),
        fit: SurfaceFit { fit, offset },
        ..today.clone()
    };
    let mut rows = vec![
        today.clone(),
        fit("C1 weighted27", PlaneFit::Weighted27, PlaneOffset::Fraction),
        fit("C2 volume", PlaneFit::Uniform27, PlaneOffset::VolumeMatched),
        fit("C1 + C2", PlaneFit::Weighted27, PlaneOffset::VolumeMatched),
    ];
    for k in [0.25, 0.5] {
        rows.push(Candidate {
            name: format!("C3 face mix {k}"),
            face_mix: k,
            ..today.clone()
        });
    }
    for w in [0.25, 0.5] {
        rows.push(Candidate {
            name: format!("C4 fine {w}"),
            fine: w,
            ..today.clone()
        });
    }
    rows.push(Candidate {
        name: "C5".to_owned(),
        after: true,
        ..today.clone()
    });
    for w in [0.25, 0.5] {
        rows.push(Candidate {
            name: format!("C4 fine {w} + C5"),
            fine: w,
            after: true,
            ..today.clone()
        });
    }
    for strength in [0.1, 0.2, 0.3] {
        for scale in [2.0, 4.0] {
            for after in [false, true] {
                rows.push(Candidate {
                    name: format!(
                        "rock {scale}, {strength}{}",
                        if after { " + C5" } else { "" }
                    ),
                    rock: [scale, strength],
                    after,
                    ..today.clone()
                });
            }
        }
    }
    let face_sil = face_silhouette(&device, &queue);
    println!("face mode's pillar silhouette error: {face_sil} px");
    let mut table = Vec::new();
    for c in &rows {
        let r = sweep_row(&device, &queue, c);
        let meets = r.error <= 12.0 && 2 * r.silhouette <= face_sil && r.axis <= 0.10;
        println!(
            "{}: S_fine {:.5}, S_mid {:.5}, E {:.2}, R {:.3}, error {:.2}, silhouette {}, A {:.2} %, guardrails {}",
            c.name,
            r.fine,
            r.mid,
            r.e,
            r.r,
            r.error,
            r.silhouette,
            r.axis * 100.0,
            if meets { "met" } else { "missed" }
        );
        table.push((c.name.clone(), r, meets));
    }
    println!(
        "| candidate | S_fine | S_mid | E (px) | R | cavity normal error (deg) | silhouette error (px) | A (%) | guardrails |"
    );
    println!("|---|---|---|---|---|---|---|---|---|");
    for (name, r, meets) in &table {
        println!(
            "| {name} | {:.5} | {:.5} | {:.2} | {:.3} | {:.2} | {} | {:.2} | {} |",
            r.fine,
            r.mid,
            r.e,
            r.r,
            r.error,
            r.silhouette,
            r.axis * 100.0,
            if *meets { "met" } else { "missed" }
        );
    }
    for (name, r, _) in &table {
        assert!(
            r.fine.is_finite() && r.mid.is_finite() && r.r.is_finite(),
            "{name}: a metric is not finite"
        );
    }
    // Today's contour meets the guardrails.
    let today_row = &table[0].1;
    assert_eq!(table[0].0, "today");
    assert!(today_row.error <= 12.0 && 2 * today_row.silhouette <= face_sil);
}
