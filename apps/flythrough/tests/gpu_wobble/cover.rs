//! Plan 0100 Phase 5: sun under cover, the candidates of ADR-0084 read against a reference.
//! Three poses (`SVET_COVER`): `w3`, the forest of seed 1 (`--spawn forest --trees forest`) at
//! 15:00 from window W3's eye (x -9.5 y 915.1 z 595.8), looking at the ground [`AHEAD`] voxels
//! north; `w1`, the same world from window W1's eye (x -622.4 y 607.0 z 608.6) looking at that
//! same point; `slot`, the outdoor source with sparse trees (`--source outdoor`) from the
//! inside spawn at 12:00, facing out of the tunnel. The rig's frame under the fly-through's
//! defaults and bake knobs, from a fresh store: the queue steps until the view's key words
//! land (a census every [`CENSUS_EVERY`] frames: the share of key-lit pixels whose own cell
//! holds a fresh key word), then the exposure is pinned, the queue stopped, and one frame read:
//! every internal pixel's hit face, the column map and the store's blocks. On the CPU
//! (`support::cover`), per key-lit face (its normal toward the key light): **the reference**,
//! 64 rays from the face's centre over the disc (8 x 8 strata of `disc_direction`) to the
//! window's side or top, no cap; the horizon at the face's centre and its class (1; 0 under
//! its own top; under 1 by the horizon); `disc`, the centre ray from the face's centre;
//! `approach`, its closest-approach score; the record at the face's own cell. Per pixel: the
//! horizon at the hit and the record through the cell blend by the shading normal. A cover
//! candidate is shown as the larger of it and the horizon at the face's centre (the horizon
//! alone where it holds none). Display levels per unit of key visibility (`G`) are read from
//! the image against one with the key light off, on pixels the horizon lights whole. Nothing is
//! asserted.

use super::*;
use support::cover::{Columns, Records, Solid, disc_direction, octa16_decode};

#[path = "walked.rs"]
mod walked;

/// Voxels north of W3's eye at which its camera looks at the ground.
const AHEAD: f32 = 40.0;
const W3_EYE: [f32; 3] = [-9.5, 915.1, 595.8];
const W1_EYE: [f32; 3] = [-622.4, 607.0, 608.6];

/// Frames between two censuses of the fresh start, and the most frames it may take.
const CENSUS_EVERY: usize = 120;
const START_MAX: usize = 36000;

/// The reference's rays a face: `STRATA` x `STRATA` over the disc.
const STRATA: usize = 8;

/// The CPU pass's threads.
const THREADS: usize = 6;

/// The forest of seed 1 over the chunks that hold both W1's and W3's eyes with 3 to spare (6 in
/// z from 2 under W3's eye's chunk), and the camera of `pose`.
fn forest_pose(pose: &str) -> Option<(Rig, Camera)> {
    use flythrough::outdoor::OutdoorSource;
    use flythrough::trees::{ForestSource, LEAF_FILL, TreeDensity};
    use svet_world::{ChunkExtent, VoxelCoord, World};
    let source = ForestSource::new(OutdoorSource::new(1), TreeDensity::Forest, LEAF_FILL);
    let extent = ChunkExtent::new([-13, 6, 7], [3, 18, 13]);
    let h = |x: f32, y: f32| source.base().height(x.round() as i32, y.round() as i32) as f32;
    let target = [
        W3_EYE[0],
        W3_EYE[1] + AHEAD,
        h(W3_EYE[0], W3_EYE[1] + AHEAD) + 1.0,
    ];
    let eye = if pose == "w1" { W1_EYE } else { W3_EYE };
    println!(
        "{pose}: ground under the eye z {}, the target {target:?}",
        h(eye[0], eye[1]) + 1.0
    );
    let t = std::time::Instant::now();
    let world = World::generate(extent, &source, &flythrough::outdoor_materials());
    println!("generated in {:.1} s", t.elapsed().as_secs_f64());
    let rig = support::terms::mirrored(world, VoxelCoord(eye.map(|v| v.floor() as i32)))?;
    Some((rig, flythrough::camera_looking_at(eye, target)))
}

/// The outdoor source with sparse trees over the tunnel rig's chunks, the inside spawn's camera
/// facing out.
fn slot_pose() -> Option<(Rig, Camera)> {
    use flythrough::outdoor::{OutdoorSource, Spawn};
    use flythrough::trees::{ForestSource, LEAF_FILL, TreeDensity};
    use svet_world::{ChunkExtent, World};
    let source = ForestSource::new(OutdoorSource::new(1), TreeDensity::Sparse, LEAF_FILL);
    let (eye, facing) = source.base().spawn(Spawn::Inside);
    let c = eye.chunk().0;
    let extent = ChunkExtent::new(
        [c[0] - 3, c[1] - 3, c[2] - 4],
        [c[0] + 11, c[1] + 4, c[2] + 2],
    );
    let world = World::generate(extent, &source, &flythrough::outdoor_materials());
    let rig = support::terms::mirrored(world, eye)?;
    let p = eye.0.map(|v| v as f32 + 0.5);
    let d = facing.map(|v| v as f32);
    Some((
        rig,
        flythrough::camera_looking_at(p, [p[0] + d[0], p[1] + d[1], p[2] + d[2]]),
    ))
}

/// Every internal pixel's hit cube face (voxel, face) whose normal faces the key light `l`, with
/// its hit and shading normal; `None` elsewhere.
pub(super) type PixelFace = Option<([i32; 3], u8, [f32; 3], [f32; 3])>;

fn pixel_faces(rig: &Rig, f: &Frame, eye: [f32; 3], l: [f32; 3]) -> Vec<PixelFace> {
    let pos = svet_render::debug::read_rgba32f(&rig.device, &rig.queue, f.hit_positions());
    let info = svet_render::debug::read_rgba16u(&rig.device, &rig.queue, f.hit_records());
    pos.iter()
        .zip(&info)
        .map(|(p, i)| {
            let (v, face) = support::bake::hit_face(eye, *p, *i)?;
            let nf = face_normal(face);
            if nf[0] * l[0] + nf[1] * l[1] + nf[2] * l[2] <= 0.0 {
                return None;
            }
            Some((v, face, [p[0], p[1], p[2]], octa16_decode([i[0], i[1]])))
        })
        .collect()
}

pub(super) fn face_normal(face: u8) -> [f32; 3] {
    let mut n = [0.0; 3];
    n[usize::from(face / 2)] = if face % 2 == 1 { 1.0 } else { -1.0 };
    n
}

/// The store's blocks of every chunk within 8 voxels of a face of `px`.
fn records(rig: &Rig, f: &Frame, px: &[PixelFace]) -> Records {
    let mut want = std::collections::BTreeSet::new();
    for (v, ..) in px.iter().flatten() {
        for dz in [-8, 0, 8] {
            for dy in [-8, 0, 8] {
                for dx in [-8, 0, 8] {
                    want.insert([v[0] + dx, v[1] + dy, v[2] + dz].map(|x| x >> 6));
                }
            }
        }
    }
    let mut r = Records::default();
    for c in want {
        if let Some(b) = f.read_store_block(&rig.device, &rig.queue, svet_world::ChunkCoord(c)) {
            r.blocks.insert(c, b);
        }
    }
    r
}

/// The fresh start's census: key-lit pixels, those whose own cell holds a fresh key word, those
/// on a coarse block.
fn census(rig: &Rig, f: &Frame, eye: [f32; 3], l: [f32; 3]) -> [usize; 3] {
    let px = pixel_faces(rig, f, eye, l);
    let mut chunks = std::collections::BTreeSet::new();
    for (v, ..) in px.iter().flatten() {
        chunks.insert(v.map(|x| x >> 6));
    }
    let mut r = Records::default();
    for &c in &chunks {
        if let Some(b) = f.read_store_block(&rig.device, &rig.queue, svet_world::ChunkCoord(c)) {
            r.blocks.insert(c, b);
        }
    }
    let mut out = [0usize; 3];
    for (v, face, ..) in px.iter().flatten() {
        out[0] += 1;
        out[1] += usize::from(r.key_cell(v.map(|x| x >> 3), *face).is_some());
        out[2] += usize::from(
            f.store()
                .is_coarse(svet_world::ChunkCoord(v.map(|x| x >> 6))),
        );
    }
    out
}

/// One face's CPU values.
#[derive(Clone, Copy, Default)]
pub(super) struct FaceRow {
    pub(super) reference: f32,
    pub(super) disc: f32,
    pub(super) approach: f32,
    pub(super) horizon: f32,
    /// The horizon with every map cell walked (`Columns::horizon_exact`).
    pub(super) exact: f32,
    pub(super) under_top: bool,
    pub(super) own: Option<f32>,
}

fn face_rows(
    faces: &[([i32; 3], u8)],
    solid: &Solid,
    cols: &Columns,
    rec: &Records,
    l: [f32; 3],
    radius: f32,
) -> Vec<FaceRow> {
    let tan_r = radius.tan().max(1.0e-4);
    let dirs: Vec<[f32; 3]> = (0..STRATA * STRATA)
        .map(|i| {
            let (a, b) = (i % STRATA, i / STRATA);
            disc_direction(
                l,
                radius,
                (a as f32 + 0.5) / STRATA as f32,
                (b as f32 + 0.5) / STRATA as f32,
            )
        })
        .collect();
    let row = |&(v, face): &([i32; 3], u8)| {
        let nf = face_normal(face);
        let centre = [0, 1, 2].map(|k| v[k] as f32 + 0.5 + 0.5 * nf[k]);
        let o = [0, 1, 2].map(|k| centre[k] + 0.01 * nf[k]);
        let lit = dirs
            .iter()
            .filter(|d| solid.trace(o, **d, false, tan_r).is_some())
            .count();
        FaceRow {
            reference: lit as f32 / dirs.len() as f32,
            disc: if solid.trace(o, l, false, tan_r).is_some() {
                1.0
            } else {
                0.0
            },
            approach: solid
                .trace(o, l, true, tan_r)
                .unwrap_or(0.0)
                .clamp(0.0, 1.0),
            horizon: cols.horizon(centre, nf, l, radius),
            exact: cols.horizon_exact(centre, nf, l, radius),
            under_top: cols.under_own_top(centre, nf),
            own: rec.key_cell(v.map(|x| x >> 3), face),
        }
    };
    let per = faces.len().div_ceil(THREADS).max(1);
    std::thread::scope(|s| {
        let hs: Vec<_> = faces
            .chunks(per)
            .map(|part| s.spawn(move || part.iter().map(row).collect::<Vec<_>>()))
            .collect();
        hs.into_iter()
            .flat_map(|h| h.join().expect("a face thread"))
            .collect()
    })
}

/// The `q` quantile of `v` (sorted in place); 0 for none.
pub(super) fn quantile(v: &mut [f32], q: f64) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f32::total_cmp);
    v[((v.len() - 1) as f64 * q).round() as usize]
}

/// The candidates' names, in the order of [`candidates`].
pub(super) const CANDIDATES: [&str; 6] = [
    "horizon at the hit",
    "horizon at the face's centre",
    "disc (centre ray)",
    "approach",
    "record, own cell",
    "record, cell blend",
];

/// A pixel's candidate values: the two horizons bare, the cover sources as the larger of each
/// and the horizon at the face's centre.
pub(super) fn candidates(r: &FaceRow, h_hit: f32, blend: Option<f32>) -> [f32; 6] {
    let h = r.horizon;
    [
        h_hit,
        h,
        h.max(r.disc),
        h.max(r.approach),
        r.own.map_or(h, |x| h.max(x)),
        blend.map_or(h, |x| h.max(x)),
    ]
}

mod reading {
    use super::*;

    fn dir_of() -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../live/wobble/cover");
        std::fs::create_dir_all(&dir).expect("the reading's directory");
        dir
    }

    /// The reading at the pose `SVET_COVER` names (`w3`, `w1` or `slot`), its lines printed and
    /// written to `live/wobble/cover/<pose>.txt`, its stills beside them.
    #[test]
    #[ignore = "reading"]
    fn sun_under_cover() {
        // `SVET_COVER=<pose>[+<arm spec>]`: the arm (`Arm::from_spec`, `default` unless given)
        // the frames are rendered under; under `keyhorizon=walk` the port's check reads the CPU
        // walk (`Columns::horizon_exact`), not the march.
        let var = std::env::var("SVET_COVER").unwrap_or_else(|_| "w3".into());
        let (pose, spec) = var.split_once('+').unwrap_or((&var, "default"));
        let pose = pose.to_owned();
        let tag = if spec == "default" {
            pose.clone()
        } else {
            format!("{pose}-{}", spec.replace(['=', ','], "-"))
        };
        let (minutes, built) = match pose.as_str() {
            "w3" | "w1" => (15.0 * 60.0, forest_pose(&pose)),
            "slot" => (12.0 * 60.0, slot_pose()),
            "terr" => (12.0 * 60.0, terraces::w3_rig()),
            other => panic!("SVET_COVER={other}: w3, w1, slot or terr"),
        };
        let Some((mut rig, cam)) = built else {
            return;
        };
        let rig = &mut rig;
        let arm = Arm::from_spec(spec);
        let walk = arm.key_horizon == svet_render::KeyHorizon::Walk;
        let mut f = frame_for(rig, &arm);
        let sky = flythrough::daylight::Clock {
            minutes,
            ..Default::default()
        }
        .sky(flythrough::outdoor::BASE_Z as f32);
        let (_, body) = sky.key_light().expect("the sun is up");
        let (l, radius) = (body.direction, body.angular_radius);
        f.set_sky(Some(sky));
        f.set_bake_coarse(
            svet_render::DEFAULT_COARSE_SAMPLES,
            svet_render::DEFAULT_NEAR,
        );
        f.set_bake_full_first(Some(svet_render::DEFAULT_NEAR));
        f.set_bake_tiers(svet_render::DEFAULT_TIERS);
        f.set_bake_drain(Some(svet_render::DEFAULT_DRAIN));
        let mut s = Stepper::new(rig, f);
        let eye = cam.position.to_array();
        let mut out = format!(
            "cover {pose} on {}: eye {eye:?} yaw {:.1} pitch {:.1} deg; sun {l:?} (elevation {:.1} deg, radius {:.2} deg)\n",
            rig.adapter,
            cam.yaw.to_degrees(),
            cam.pitch.to_degrees(),
            l[2].asin().to_degrees(),
            radius.to_degrees()
        );
        // The fresh start: the queue steps at the fly-through's knobs; a census every
        // CENSUS_EVERY frames until the queue is drained in view, or the key words cover the
        // view with no coarse block and hold.
        let t0 = std::time::Instant::now();
        let mut firsts: [Option<(usize, f64)>; 3] = [None; 3];
        let mut last = [usize::MAX; 3];
        let mut k = 0usize;
        loop {
            let q = coarse::step_queue(rig, &mut s.frame, &cam);
            s.run(rig, &cam);
            k += 1;
            if !k.is_multiple_of(CENSUS_EVERY) && k < START_MAX {
                continue;
            }
            let c = census(rig, &s.frame, eye, l);
            let share = c[1] as f64 / c[0].max(1) as f64;
            for (i, bar) in [0.5, 0.9, 0.99].iter().enumerate() {
                if firsts[i].is_none() && share >= *bar {
                    firsts[i] = Some((k, t0.elapsed().as_secs_f64()));
                }
            }
            out.push_str(&format!(
                "  frame {k} ({:.1} s): key-lit px {}, own cell keyed {:.2} %, on a coarse block {}; drain waiting {} jobs {} depth {}\n",
                t0.elapsed().as_secs_f64(),
                c[0],
                100.0 * share,
                c[2],
                q.drain_waiting,
                q.jobs,
                q.depth
            ));
            let drained = q.drain_waiting == 0 && q.jobs == 0 && !q.draining;
            let held = c == last && c[2] == 0 && c[1] == c[0];
            if drained || held || k >= START_MAX {
                break;
            }
            last = c;
        }
        out.push_str(&format!(
            "the key words in view: half at {:?}, 90 % at {:?}, 99 % at {:?} (frame, s since the first frame)\n",
            firsts[0], firsts[1], firsts[2]
        ));
        // The read: exposure pinned, the queue stopped.
        let st = svet_render::debug::read_buffer(
            &rig.device,
            &rig.queue,
            s.frame.exposure_state(),
            0,
            32,
        );
        let e = svet_render::ExposureState::from_bytes(&st).current;
        s.frame.set_lighting(svet_render::Lighting {
            exposure: svet_render::Exposure::Manual(e),
            ..s.frame.lighting()
        });
        for _ in 0..svet_render::internals::FADE as usize + 8 {
            s.run(rig, &cam);
        }
        let shot = s.step(rig, &cam);
        let pair =
            svet_render::debug::read_rgba16f(&rig.device, &rig.queue, s.frame.visibility_pair());
        let px = pixel_faces(rig, &s.frame, eye, l);
        let rec = records(rig, &s.frame, &px);
        let ext = rig.world.extent();
        let cols = Columns {
            map: svet_render::debug::read_r32i(&rig.device, &rig.queue, rig.gpu.column_map()),
            window_min: [ext.min.0[0], ext.min.0[1]],
            window_size: [ext.size()[0], ext.size()[1]],
        };
        let calm = lumas(&shot.display);
        support::write_png(
            &dir_of().join(format!("{tag}-still.png")),
            &shot.display,
            OUT[0],
            OUT[1],
        );
        s.frame.set_light_hooks(svet_render::LightHooks {
            key_off: true,
            ..Default::default()
        });
        for _ in 0..svet_render::internals::FADE as usize + 8 {
            s.run(rig, &cam);
        }
        let off_shot = s.step(rig, &cam);
        let off = lumas(&off_shot.display);
        support::write_png(
            &dir_of().join(format!("{tag}-keyoff.png")),
            &off_shot.display,
            OUT[0],
            OUT[1],
        );
        // The port's check: the GPU's key value at each lighting pixel under the defaults (the
        // horizon at its hit) against the CPU's horizon at that hit.
        let gw = s.frame.light_hit_positions().width() as usize;
        let (mut checked, mut agree) = (0usize, 0usize);
        let mut mismatches = Vec::new();
        for (g, h) in shot.hits.iter().enumerate() {
            if h[3] >= 1e30 || !h[3].is_finite() {
                continue;
            }
            let p = [h[0], h[1], h[2]];
            let on: Vec<usize> = (0..3)
                .filter(|&k| (p[k] - p[k].round()).abs() < 1e-4)
                .collect();
            if on.len() != 1 {
                continue;
            }
            let a = on[0];
            let mut nf = [0.0; 3];
            nf[a] = if eye[a] > p[a] { 1.0 } else { -1.0 };
            if nf[0] * l[0] + nf[1] * l[1] + nf[2] * l[2] <= 0.0 {
                continue;
            }
            let gpu = support::defects::key_slot(pair[g][1]).0;
            let cpu = if walk {
                cols.horizon_exact(p, nf, l, radius)
            } else {
                cols.horizon(p, nf, l, radius)
            };
            checked += 1;
            agree += usize::from((gpu - cpu).abs() <= 2.0 / 255.0);
            if (gpu - cpu).abs() > 2.0 / 255.0 {
                mismatches.push(walked::Mismatch { p, nf, gpu, cpu });
            }
        }
        out.push_str(&format!(
            "the CPU horizon ({}) at the lighting pixel's hit against the GPU's key value: {agree} of {checked} within 2/255 (grid {gw} wide)\n",
            arm.key_horizon.name()
        ));
        // The faces, and the CPU pass.
        let mut index = std::collections::HashMap::new();
        let mut faces = Vec::new();
        let px_face: Vec<Option<usize>> = px
            .iter()
            .map(|p| {
                p.map(|(v, face, ..)| {
                    *index.entry((v, face)).or_insert_with(|| {
                        faces.push((v, face));
                        faces.len() - 1
                    })
                })
            })
            .collect();
        let t = std::time::Instant::now();
        let solid = Solid::of(&rig.world);
        let rows = face_rows(&faces, &solid, &cols, &rec, l, radius);
        out.push_str(&format!(
            "key-lit faces {} over {} px; the CPU pass {:.1} s\n",
            faces.len(),
            px_face.iter().flatten().count(),
            t.elapsed().as_secs_f64()
        ));
        // Levels a unit of key visibility, by face direction: the display's change with the key
        // off, on pixels the horizon at the hit lights whole.
        let (iw, ih) = (INTERNAL[0] as usize, INTERNAL[1] as usize);
        let disp = |i: usize| {
            let (x, y) = (i % iw, i / iw);
            (y * OUT[1] as usize / ih) * OUT[0] as usize + x * OUT[0] as usize / iw
        };
        let mut gs: [Vec<f32>; 6] = Default::default();
        let mut h_hit = vec![0.0f32; px.len()];
        let mut blend = vec![None; px.len()];
        for (i, p) in px.iter().enumerate() {
            let Some((v, face, hit, n)) = p else { continue };
            let nf = face_normal(*face);
            h_hit[i] = cols.horizon(*hit, nf, l, radius);
            blend[i] = rec.key_visibility(*v, *face, *n);
            if h_hit[i] >= 0.999 {
                let d = disp(i);
                gs[usize::from(*face)].push(f32::from(calm[d]) - f32::from(off[d]));
            }
        }
        let mut all_g: Vec<f32> = gs.iter().flatten().copied().collect();
        let g_all = quantile(&mut all_g, 0.5);
        let g: Vec<f32> = gs
            .iter_mut()
            .map(|v| {
                if v.len() >= 200 {
                    quantile(v, 0.5)
                } else {
                    g_all
                }
            })
            .collect();
        out.push_str(&format!(
            "levels a unit of key visibility (median, -x +x -y +y -z +z): {:?}, all {g_all:.1}\n",
            g.iter()
                .map(|x| (x * 10.0).round() / 10.0)
                .collect::<Vec<_>>()
        ));
        // The classes by the horizon at the face's centre.
        let class_of = |r: &FaceRow| {
            if r.horizon >= 1.0 {
                0
            } else if r.horizon <= 0.0 && r.under_top {
                1
            } else {
                2
            }
        };
        let names = [
            "reads 1",
            "reads 0 under its own top",
            "under 1 by the horizon",
        ];
        let mut cpx = [0usize; 3];
        let mut cref = [0.0f64; 3];
        let mut clit = [0usize; 3];
        let mut cfaces = [0usize; 3];
        for r in &rows {
            cfaces[class_of(r)] += 1;
        }
        for f in px_face.iter().flatten() {
            let r = &rows[*f];
            let c = class_of(r);
            cpx[c] += 1;
            cref[c] += f64::from(r.reference);
            clit[c] += usize::from(r.reference > 0.0);
        }
        for c in 0..3 {
            out.push_str(&format!(
                "class {}: {} px, {} faces; reference mean {:.3}, px with any sun {:.2} %\n",
                names[c],
                cpx[c],
                cfaces[c],
                cref[c] / cpx[c].max(1) as f64,
                100.0 * clit[c] as f64 / cpx[c].max(1) as f64
            ));
        }
        // The premise: faces the horizon reads 1 where the reference is short of 1.
        let lit_faces: Vec<&FaceRow> = rows.iter().filter(|r| class_of(r) == 0).collect();
        let short = |b: f32| lit_faces.iter().filter(|r| r.reference < b).count();
        let ramp = rows
            .iter()
            .filter(|r| r.horizon > 0.0 && r.horizon < 1.0)
            .count();
        out.push_str(&format!(
            "premise: of {} faces the horizon reads 1, the reference under 1 on {}, under 0.5 on {}, 0 on {}; the ramp's faces (horizon strictly between 0 and 1) {}\n",
            lit_faces.len(),
            short(1.0),
            short(0.5),
            lit_faces.iter().filter(|r| r.reference <= 0.0).count(),
            ramp
        ));
        // The faces the horizon reads 1 and the reference under 0.5, by why: the face's own
        // voxel not opaque at level 0 (a coarse cube's face), the centre ray's origin in an
        // opaque voxel, the centre ray lit (the disc's edge), or the centre ray's blocker by its
        // distance along the ray (under the horizon's lift of SEED_LIFT, or past it) and its
        // material; and their pixels.
        let mut why = std::collections::BTreeMap::<String, (usize, usize)>::new();
        let mut px_of = vec![0usize; rows.len()];
        for f in px_face.iter().flatten() {
            px_of[*f] += 1;
        }
        for (i, (r, &(v, face))) in rows.iter().zip(&faces).enumerate() {
            if class_of(r) != 0 || r.reference >= 0.5 {
                continue;
            }
            let nf = face_normal(face);
            let o = [0, 1, 2].map(|k| v[k] as f32 + 0.5 + 0.51 * nf[k]);
            let key = if !solid.solid(v) {
                "own voxel not opaque".to_owned()
            } else if solid.solid(o.map(|x| x.floor() as i32)) {
                "origin in an opaque voxel".to_owned()
            } else {
                match solid.walk(o, l, false, 1.0) {
                    Ok(_) => "centre ray lit".to_owned(),
                    Err((t, hv)) => format!(
                        "blocked at t {} by material {}",
                        if t < support::cover::SEED_LIFT {
                            "under the lift"
                        } else if t < 16.0 {
                            "2.5..16"
                        } else if t < 64.0 {
                            "16..64"
                        } else {
                            "64 on"
                        },
                        rig.world
                            .material_at(svet_world::VoxelCoord(hv))
                            .unwrap_or(0)
                    ),
                }
            };
            let e = why.entry(key).or_default();
            e.0 += 1;
            e.1 += px_of[i];
        }
        for (k, (n, p)) in &why {
            out.push_str(&format!("  premise breaks, {k}: {n} faces, {p} px\n"));
        }
        let breaks: Vec<&FaceRow> = lit_faces
            .iter()
            .copied()
            .filter(|r| r.reference < 0.5)
            .collect();
        let exact_lit: Vec<&FaceRow> = rows.iter().filter(|r| r.exact >= 1.0).collect();
        out.push_str(&format!(
            "  every map cell walked: of the {} breaks it reads under 1 on {}, 0 on {}; of {} faces it reads 1, the reference under 1 on {}, under 0.5 on {}, 0 on {}; its ramp's faces {}\n",
            breaks.len(),
            breaks.iter().filter(|r| r.exact < 1.0).count(),
            breaks.iter().filter(|r| r.exact <= 0.0).count(),
            exact_lit.len(),
            exact_lit.iter().filter(|r| r.reference < 1.0).count(),
            exact_lit.iter().filter(|r| r.reference < 0.5).count(),
            exact_lit.iter().filter(|r| r.reference <= 0.0).count(),
            rows.iter().filter(|r| r.exact > 0.0 && r.exact < 1.0).count(),
        ));
        // The candidates against the reference.
        let mut shown = [0.0f64; 6];
        let mut mean_u = [0.0f64; 6];
        let mut leak = [0usize; 6];
        let (mut e_ref, mut n_u, mut n_dark) = (0.0f64, 0usize, 0usize);
        let mut keyed_u = [0usize; 2];
        // By class: the reference's sun and each candidate's share of it; the truly shadowed
        // pixels and each candidate's leaks.
        let mut e_c = [0.0f64; 3];
        let mut shown_c = [[0.0f64; 6]; 3];
        let mut dark_c = [0usize; 3];
        let mut leak_c = [[0usize; 6]; 3];
        for (i, f) in px_face.iter().enumerate() {
            let Some(f) = f else { continue };
            let r = &rows[*f];
            let face = px[i].map_or(0, |p| p.1);
            let c = candidates(r, h_hit[i], blend[i]);
            let cl = class_of(r);
            e_c[cl] += f64::from(r.reference);
            for k in 0..6 {
                shown_c[cl][k] += f64::from(c[k].min(r.reference));
            }
            if r.reference <= 0.0 {
                n_dark += 1;
                dark_c[cl] += 1;
                for k in 0..6 {
                    let l = c[k] * g[usize::from(face)] > 4.0;
                    leak[k] += usize::from(l);
                    leak_c[cl][k] += usize::from(l);
                }
            }
            if r.horizon < 1.0 {
                n_u += 1;
                e_ref += f64::from(r.reference);
                keyed_u[0] += usize::from(r.own.is_some());
                keyed_u[1] += usize::from(blend[i].is_some());
                for k in 0..6 {
                    shown[k] += f64::from(c[k].min(r.reference));
                    mean_u[k] += f64::from(c[k]);
                }
            }
        }
        out.push_str(&format!(
            "under cover (the horizon at the face's centre under 1): {n_u} px, the reference's sun {:.1} px-equivalents ({:.2} % of the area); own cell keyed {:.2} %, blend keyed {:.2} %; truly shadowed (reference 0) {n_dark} px\n",
            e_ref,
            100.0 * e_ref / n_u.max(1) as f64,
            100.0 * keyed_u[0] as f64 / n_u.max(1) as f64,
            100.0 * keyed_u[1] as f64 / n_u.max(1) as f64,
        ));
        for k in 0..6 {
            out.push_str(&format!(
                "  {:<30} shows {:6.2} % of the reference's sun under cover; leaks over 4 levels on {:6.2} % of the truly shadowed px; mean under cover {:.3}\n",
                CANDIDATES[k],
                100.0 * shown[k] / e_ref.max(1e-9),
                100.0 * leak[k] as f64 / n_dark.max(1) as f64,
                mean_u[k] / n_u.max(1) as f64
            ));
            out.push_str("    by class (reads 1 / under its own top / by the horizon): shows");
            for cl in 0..3 {
                out.push_str(&format!(
                    " {:.2} %",
                    100.0 * shown_c[cl][k] / e_c[cl].max(1e-9)
                ));
            }
            out.push_str("; leaks");
            for cl in 0..3 {
                out.push_str(&format!(
                    " {:.2} % of {}",
                    100.0 * leak_c[cl][k] as f64 / dark_c[cl].max(1) as f64,
                    dark_c[cl]
                ));
            }
            out.push('\n');
        }
        // The ray against the record, one face (own cell) or one pixel (blend).
        let mut d_own: Vec<f32> = rows
            .iter()
            .filter_map(|r| r.own.map(|o| (r.disc - o).abs()))
            .collect();
        let mut d_blend: Vec<f32> = px_face
            .iter()
            .enumerate()
            .filter_map(|(i, f)| Some((rows[(*f)?].disc - blend[i]?).abs()))
            .collect();
        for (name, d) in [
            ("own cell, by face", &mut d_own),
            ("cell blend, by px", &mut d_blend),
        ] {
            let n = d.len();
            let over = d.iter().filter(|x| **x > 0.5).count();
            out.push_str(&format!(
                "disc against the record ({name}): {n}; |difference| max {:.3}, p99 {:.3}, median {:.3}; over 0.5 on {:.2} %\n",
                quantile(d, 1.0),
                quantile(d, 0.99),
                quantile(d, 0.5),
                100.0 * over as f64 / n.max(1) as f64
            ));
        }
        let coarse: std::collections::HashSet<[i32; 3]> = faces
            .iter()
            .map(|(v, _)| v.map(|x| x >> 6))
            .filter(|c| s.frame.store().is_coarse(svet_world::ChunkCoord(*c)))
            .collect();
        let (text, unequal) = walked::read(&walked::Input {
            faces: &faces,
            rows: &rows,
            px: &px,
            px_face: &px_face,
            h_hit: &h_hit,
            blend: &blend,
            cols: &cols,
            solid: &solid,
            l,
            radius,
            eye,
            vfov: cam.vfov,
            coarse: &coarse,
            mismatches: &mismatches,
        });
        out.push_str(&text);
        print!("{out}");
        std::fs::write(dir_of().join(format!("{tag}.txt")), out).expect("the table");
        assert_eq!(unequal, 0, "the levelled walk differs from the walk");
    }
}
