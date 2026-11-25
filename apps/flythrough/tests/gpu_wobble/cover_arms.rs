//! Plan 0100 Phase 6's reading: the cover arms (`Lighting::key_cover`) rendered at Phase 5's
//! poses and read off the frames. One fresh start under `record` (the key words' landing, a
//! census every `CENSUS_EVERY` frames with the key slot read back), then exposure pinned and
//! the queue stopped, each arm in turn rendered from the same store: its still, its key value
//! at every lighting pixel against Phase 5's reference (64 rays a face), its display against
//! `off`'s on the truly shadowed pixels, the sun-patch class's change a pair at a still camera
//! and after a turn.

use super::*;

/// The arms in the order rendered; `off` first, the base every leak is read against.
const ARMS: [svet_render::KeyCover; 4] = [
    svet_render::KeyCover::Off,
    svet_render::KeyCover::Ray,
    svet_render::KeyCover::Record,
    svet_render::KeyCover::RayRecord,
];

/// Frame pairs the still camera's change is read over, and the frames the turn holds away.
const PAIRS: usize = 32;
const AWAY: usize = 30;

/// The patch's edge class: display pixels within this many of a pixel the arm lights over
/// `off` by more than 4 levels, and of one it does not.
const EDGE: i32 = 3;

/// Per arm: the share of the reference's sun shown under cover (`off` reads under 1), the
/// light over the reference on part-lit pixels, and the leak off the display.
#[derive(Default)]
struct Row {
    shown: f64,
    over_part: f64,
    over_all: f64,
    leak_mean: f64,
    leak: [usize; 3],
}

/// The share of the reference's sun under cover the key values `k` (by internal pixel) show:
/// the sum of the smaller of value and reference over the reference's, on pixels whose `off`
/// value is under 1.
fn shown(k: &[f32], base: &[f32], refs: &[Option<f32>]) -> f64 {
    let (mut s, mut e) = (0.0f64, 0.0f64);
    for i in 0..k.len() {
        let Some(r) = refs[i] else { continue };
        if base[i] >= 1.0 - 1.0 / 512.0 {
            continue;
        }
        s += f64::from(k[i].min(r));
        e += f64::from(r);
    }
    s / e.max(1e-9)
}

/// The patch's edge class of an arm's display luma `a` against `off`'s `b`: pixels within
/// `EDGE` of both a lit-over pixel and one not.
fn edge_class(a: &[u8], b: &[u8]) -> Vec<bool> {
    let (w, h) = (OUT[0] as i32, OUT[1] as i32);
    let lit: Vec<bool> = a
        .iter()
        .zip(b)
        .map(|(x, y)| *x as i32 - *y as i32 > 4)
        .collect();
    let mut out = vec![false; lit.len()];
    for y in 0..h {
        for x in 0..w {
            let (mut on, mut off) = (false, false);
            for dy in -EDGE..=EDGE {
                for dx in -EDGE..=EDGE {
                    let (u, v) = (x + dx, y + dy);
                    if u < 0 || v < 0 || u >= w || v >= h {
                        continue;
                    }
                    if lit[(v * w + u) as usize] {
                        on = true;
                    } else {
                        off = true;
                    }
                }
            }
            out[(y * w + x) as usize] = on && off;
        }
    }
    out
}

/// The share of `class`'s pixels whose luma differs by more than 4 levels between `a` and `b`.
fn moved(a: &[u8], b: &[u8], class: &[bool]) -> (usize, usize) {
    let n = class.iter().filter(|c| **c).count();
    let m = (0..a.len())
        .filter(|&i| class[i] && (a[i] as i32 - b[i] as i32).abs() > 4)
        .count();
    (m, n)
}

mod reading {
    use super::*;

    fn dir_of() -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../live/wobble/cover");
        std::fs::create_dir_all(&dir).expect("the reading's directory");
        dir
    }

    /// `SVET_COVER_ARMS=<pose>[+<arm spec>]` (`w3`, `w1` or `slot`; the spec as the wobble
    /// rig's, `keyhorizon=walk` the one Phase 6 reads beside the default): the arms' table,
    /// printed and written to `live/wobble/cover/arms-<tag>.txt`, the stills beside it.
    #[test]
    #[ignore = "reading"]
    fn cover_arms() {
        let var = std::env::var("SVET_COVER_ARMS").unwrap_or_else(|_| "w3".into());
        let (pose, spec) = var.split_once('+').unwrap_or((&var, "default"));
        let tag = if spec == "default" {
            format!("{pose}-march")
        } else {
            format!("{pose}-{}", spec.replace(['=', ','], "-"))
        };
        let (minutes, built) = match pose {
            "w3" | "w1" => (15.0 * 60.0, forest_pose(pose)),
            "slot" => (12.0 * 60.0, slot_pose()),
            other => panic!("SVET_COVER_ARMS={other}: w3, w1 or slot"),
        };
        let Some((mut rig, cam)) = built else {
            return;
        };
        let rig = &mut rig;
        let arm = Arm::from_spec(spec);
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
        let set_cover = |f: &mut Frame, key_cover| {
            f.set_lighting(svet_render::Lighting {
                key_cover,
                ..f.lighting()
            })
        };
        set_cover(&mut f, svet_render::KeyCover::Record);
        let mut s = Stepper::new(rig, f);
        let eye = cam.position.to_array();
        let mut out = format!(
            "cover arms {pose} ({spec}) on {}: eye {eye:?} yaw {:.1} pitch {:.1} deg; sun {l:?}\n",
            rig.adapter,
            cam.yaw.to_degrees(),
            cam.pitch.to_degrees(),
        );
        // The fresh start under `record`, the key slot kept at each census.
        let t0 = std::time::Instant::now();
        let mut series: Vec<(usize, f64, Vec<[f32; 4]>)> = Vec::new();
        let mut k = 0usize;
        loop {
            let q = coarse::step_queue(rig, &mut s.frame, &cam);
            s.run(rig, &cam);
            k += 1;
            if !k.is_multiple_of(CENSUS_EVERY) && k < START_MAX {
                continue;
            }
            let pair = svet_render::debug::read_rgba16f(
                &rig.device,
                &rig.queue,
                s.frame.visibility_pair(),
            );
            series.push((k, t0.elapsed().as_secs_f64(), pair));
            let drained = q.drain_waiting == 0 && q.jobs == 0 && !q.draining;
            if drained || k >= START_MAX {
                break;
            }
        }
        out.push_str(&format!(
            "fresh start under record: {k} frames, {:.1} s (wall, RTX-step rig, not the fly-through's drain)\n",
            t0.elapsed().as_secs_f64()
        ));
        // Exposure pinned, the queue stopped.
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
        let settle = svet_render::internals::FADE as usize + 8;
        let mut shots = Vec::new();
        for a in ARMS {
            set_cover(&mut s.frame, a);
            for _ in 0..settle {
                s.run(rig, &cam);
            }
            let shot = s.step(rig, &cam);
            let pair = svet_render::debug::read_rgba16f(
                &rig.device,
                &rig.queue,
                s.frame.visibility_pair(),
            );
            support::write_png(
                &dir_of().join(format!("arms-{tag}-{}.png", a.name())),
                &shot.display,
                OUT[0],
                OUT[1],
            );
            shots.push((lumas(&shot.display), pair));
        }
        // The faces and the reference.
        let px = pixel_faces(rig, &s.frame, eye, l);
        let rec = records(rig, &s.frame, &px);
        let ext = rig.world.extent();
        let cols = support::cover::Columns {
            map: svet_render::debug::read_r32i(&rig.device, &rig.queue, rig.gpu.column_map()),
            window_min: [ext.min.0[0], ext.min.0[1]],
            window_size: [ext.size()[0], ext.size()[1]],
        };
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
        let solid = support::cover::Solid::of(&rig.world);
        let rows = face_rows(&faces, &solid, &cols, &rec, l, radius);
        let refs: Vec<Option<f32>> = px_face
            .iter()
            .map(|f| f.map(|f| rows[f].reference))
            .collect();
        // Internal pixel to lighting pixel and to display pixel.
        let (iw, ih) = (INTERNAL[0] as usize, INTERNAL[1] as usize);
        let lw = s.frame.light_hit_positions().width() as usize;
        let lh = s.frame.light_hit_positions().height() as usize;
        let lpx = |i: usize| ((i / iw) * lh / ih) * lw + (i % iw) * lw / iw;
        let disp = |i: usize| {
            ((i / iw) * OUT[1] as usize / ih) * OUT[0] as usize + (i % iw) * OUT[0] as usize / iw
        };
        let key_of = |pair: &[[f32; 4]]| -> Vec<f32> {
            (0..px.len())
                .map(|i| support::defects::key_slot(pair[lpx(i)][1]).0)
                .collect()
        };
        let keys: Vec<Vec<f32>> = shots.iter().map(|(_, p)| key_of(p)).collect();
        let base_l = &shots[0].0;
        let (n_dark, n_part) = refs.iter().fold((0, 0), |(d, p), r| match r {
            Some(r) if *r <= 0.0 => (d + 1, p),
            Some(r) if *r < 1.0 => (d, p + 1),
            _ => (d, p),
        });
        out.push_str(&format!(
            "key-lit px {}, truly shadowed {n_dark}, part lit {n_part}; under cover (off reads under 1) {}\n",
            refs.iter().flatten().count(),
            (0..px.len())
                .filter(|&i| refs[i].is_some() && keys[0][i] < 1.0 - 1.0 / 512.0)
                .count()
        ));
        out.push_str("arm | shown under cover | over the reference, part-lit px / all px (px-equiv.) | leak off the frames on truly shadowed px: mean levels, over 4 / 16 / 64\n");
        for (a, arm_name) in ARMS.iter().enumerate() {
            let mut r = Row {
                shown: shown(&keys[a], &keys[0], &refs),
                ..Default::default()
            };
            for i in 0..px.len() {
                let Some(rf) = refs[i] else { continue };
                let over = f64::from((keys[a][i] - rf).max(0.0));
                r.over_all += over;
                if rf > 0.0 && rf < 1.0 {
                    r.over_part += over;
                }
                if rf <= 0.0 {
                    let d = disp(i);
                    let dl = f64::from(shots[a].0[d]) - f64::from(base_l[d]);
                    r.leak_mean += dl;
                    for (j, bar) in [4.0, 16.0, 64.0].iter().enumerate() {
                        r.leak[j] += usize::from(dl > *bar);
                    }
                }
            }
            let nd = n_dark.max(1) as f64;
            out.push_str(&format!(
                "  {:<10} {:6.2} % | {:9.1} / {:9.1} | {:6.2}, {:5.2} / {:5.2} / {:5.2} %\n",
                arm_name.name(),
                100.0 * r.shown,
                r.over_part,
                r.over_all,
                r.leak_mean / nd,
                100.0 * r.leak[0] as f64 / nd,
                100.0 * r.leak[1] as f64 / nd,
                100.0 * r.leak[2] as f64 / nd
            ));
        }
        // Before the record lands: `record`'s shown share at each census (the fresh start ran
        // under it; `ray-record` shows the same far and the ray near from the first frame).
        let fin = shown(&keys[2], &keys[0], &refs);
        let mut at90 = None;
        out.push_str("record during the fresh start (frame, s, shown under cover):");
        for (k, t, pair) in &series {
            let v = shown(&key_of(pair), &keys[0], &refs);
            if at90.is_none() && v >= 0.9 * fin {
                at90 = Some((*k, *t));
            }
            out.push_str(&format!(" {k} {t:.1} {:.2} %;", 100.0 * v));
        }
        out.push_str(&format!(
            "\n  90 % of the settled share at {at90:?} (frame, s)\n"
        ));
        // Steady at a still camera, and present after a turn: the patch's edge class.
        let mut turned = cam;
        turned.yaw += std::f32::consts::FRAC_PI_2;
        let hist = svet_render::Lighting {
            light_history: true,
            key_cover: svet_render::KeyCover::Off,
            ..s.frame.lighting()
        };
        out.push_str("the patch's edge class (display px within 3 of the arm's light over off): moved over 4 levels a pair at a still camera (mean of 32 pairs), and the first frame back after a 90 degree turn against the settled frame\n");
        for (a, arm_name) in ARMS.iter().enumerate().skip(1) {
            let class = edge_class(&shots[a].0, base_l);
            for (label, lighting) in [
                (arm_name.name(), None),
                ("light history on", Some(hist)),
                (
                    "off",
                    Some(svet_render::Lighting {
                        key_cover: svet_render::KeyCover::Off,
                        ..s.frame.lighting()
                    }),
                ),
            ] {
                let keep = s.frame.lighting();
                let lit = lighting.unwrap_or(svet_render::Lighting {
                    key_cover: *arm_name,
                    ..keep
                });
                s.frame.set_lighting(lit);
                for _ in 0..settle {
                    s.run(rig, &cam);
                }
                let mut prev = lumas(&s.step(rig, &cam).display);
                let settled = prev.clone();
                let (mut m, mut n) = (0usize, 0usize);
                for _ in 0..PAIRS {
                    let now = lumas(&s.step(rig, &cam).display);
                    let (a, b) = moved(&now, &prev, &class);
                    m += a;
                    n = b;
                    prev = now;
                }
                for _ in 0..AWAY {
                    s.run(rig, &turned);
                }
                let back = lumas(&s.step(rig, &cam).display);
                let (tm, _) = moved(&back, &settled, &class);
                out.push_str(&format!(
                    "  {:<10} class of {}: {n} px; still {:.3} % a pair; after the turn {:.2} %\n",
                    arm_name.name(),
                    label,
                    100.0 * m as f64 / (PAIRS * n.max(1)) as f64,
                    100.0 * tm as f64 / n.max(1) as f64
                ));
                s.frame.set_lighting(keep);
            }
        }
        print!("{out}");
        std::fs::write(dir_of().join(format!("arms-{tag}.txt")), out).expect("the table");
    }
}
