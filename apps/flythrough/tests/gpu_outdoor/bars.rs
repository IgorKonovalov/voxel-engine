// `gpu_outdoor`'s Plan 0086 bars written from their readings (Phase 25), included by
// `gpu_outdoor.rs` (kept apart for the file-size cap).

/// Plan 0086 Phase 5 bar 4: an open rock floor (4 x 4 chunks, rock below z 16) under
/// `Sky::uniform(1.0)` through the live queue (`support::queue`'s rig at its counts), a camera
/// 40 voxels up looking down at 60 degrees. Settled, then a 10-voxel box dug into the floor
/// ahead: per frame, the lighting pixels where the sky rays were cast, until three frames in a
/// row read none (or 240 frames). At rest no pixel casts. With `sky_rays` 1 the dug rings'
/// pixels cast on the dig's first frame and none once their re-bake lands (the frame printed).
/// With `sky_rays` 0 no pixel casts at any frame. Plan 0094 Phase 5 bar 9 (ADR-0076 rule 3, the
/// dig's light changes once), in both arms: the dug floor's centre pixel's stored part, from
/// the 8 frames before the dig (`old`, their mean) to the landing (the first frame after the
/// dig whose source word is `record`) plus `F + 16` (`new`, the value there), leaves no frame
/// outside `[min(old, new) - T_fade, max(old, new) + T_fade]` and is within `T_fade` of `new`
/// from the landing plus `F` on; frame 1's source word is not `own summary`. The trace is
/// printed with the step clause's result beside it (no one-frame change over
/// `|old - new| / F + T_fade`), not asserted: the frame-1 value is the column's seed.
#[test]
#[ignore = "soak"]
fn the_sky_ray_runs_only_where_stale() {
    use support::queue::{Setup, box_op, fill, rig, world_of};
    use support::rules::{F, MEAN_FRAMES, T_FADE};
    use vxe_world::{AIR, Mode};
    let floor = |rays: u32| {
        let world = world_of([4, 4, 1], AIR);
        fill(&world, [0, 0, 0], [255, 255, 15], flythrough::cave::ROCK);
        let eye = [80.0, 128.0, 56.0];
        let mut q = rig(world, &Setup::at(eye))?;
        q.camera.pitch = -60f32.to_radians();
        q.frame.set_count_store(true);
        q.frame.set_lighting(vxe_render::Lighting {
            sky_rays: rays,
            ..q.frame.lighting()
        });
        q.frame.set_sky(Some(vxe_render::sky::Sky {
            altitude_zero_z: 16.0,
            ..vxe_render::sky::Sky::uniform(1.0)
        }));
        Some(q)
    };
    for rays in [1u32, 0] {
        let Some(mut q) = floor(rays) else {
            return;
        };
        let settled = q.settle();
        let cast = |q: &mut support::queue::Q| {
            q.step();
            q.render();
            let v =
                vxe_render::debug::read_rgba8(&q.device, &q.queue, q.frame.visibility_samples());
            let n = v.chunks(4).filter(|p| p[3] > 0).count();
            let a = vxe_render::debug::read_rgba16f(&q.device, &q.queue, q.frame.accumulated());
            let (w, h) = (
                q.frame.accumulated().width() as usize,
                q.frame.accumulated().height() as usize,
            );
            let c = (h / 2) * w + w / 2;
            let m = vxe_render::debug::read_rgba16f(&q.device, &q.queue, q.frame.moments());
            let p = vxe_render::debug::read_rgba16f(&q.device, &q.queue, q.frame.visibility_pair());
            let word = support::defects::source_of(m[c][3]);
            (n, a[c][1], word, p[c])
        };
        let rest: Vec<_> = (0..MEAN_FRAMES).map(|_| cast(&mut q)).collect();
        let at_rest: Vec<usize> = rest.iter().map(|r| r.0).collect();
        let (_, rest_g, rest_word, _) = rest[MEAN_FRAMES - 1];
        let old = rest.iter().map(|r| f64::from(r.1)).sum::<f64>() / MEAN_FRAMES as f64;
        println!(
            "sky rays {rays}: at rest the centre stored g {rest_g:.3}, source {}",
            support::defects::SOURCE_NAMES[usize::from(rest_word).min(6)]
        );
        println!("sky rays {rays}: settled in {settled} steps; at rest {at_rest:?} pixels cast");
        assert!(at_rest.iter().all(|&n| n == 0), "sky rays {rays}: {at_rest:?} at rest");
        // The camera's centre ray meets the floor about 23 voxels ahead: dig 10 voxels there.
        q.edit(&box_op(
            [98, 123, 6],
            [107, 132, 15],
            Mode::Replace {
                from: flythrough::cave::ROCK,
                to: AIR,
            },
        ));
        let mut row = Vec::new();
        let mut words = Vec::new();
        let mut word_at = Vec::new();
        let mut quiet = 0;
        for f in 1..=240 {
            let (n, centre, word, pair) = cast(&mut q);
            row.push((n, centre));
            word_at.push(word);
            words.push(format!(
                "{f}:{}:pair {:.3}/{:.3}/{}",
                support::defects::SOURCE_NAMES[usize::from(word).min(6)],
                pair[0],
                pair[2],
                pair[3]
            ));
            quiet = if n == 0 { quiet + 1 } else { 0 };
            let landed = word_at.iter().position(|&w| w == 1);
            if quiet >= 3 && f > 8 && landed.is_some_and(|l| f > l + 1 + F + 16) {
                break;
            }
        }
        let text: Vec<String> = row
            .iter()
            .enumerate()
            .map(|(f, (n, c))| format!("{}:{n}:{c:.3}", f + 1))
            .collect();
        println!(
            "sky rays {rays}: after the dig, frame:cast pixels:centre stored g {}",
            text.join(" ")
        );
        println!(
            "sky rays {rays}: the centre's source word and pair (mean/short/age) per frame {}",
            words.join(" ")
        );
        assert!(quiet >= 3, "sky rays {rays}: still casting after 240 frames");
        // Rule 3 over the centre's stored part: `row[k]` is frame `k + 1` after the dig.
        let landing = word_at
            .iter()
            .position(|&w| w == 1)
            .expect("the centre's record lands");
        let end = (landing + F + 16).min(row.len() - 1);
        let new = f64::from(row[end].1);
        let t = f64::from(T_FADE);
        let (lo, hi) = (old.min(new) - t, old.max(new) + t);
        let trace: Vec<f64> = rest
            .iter()
            .map(|r| f64::from(r.1))
            .chain(row[..=end].iter().map(|r| f64::from(r.1)))
            .collect();
        let out: Vec<(usize, f64)> = row[..=end]
            .iter()
            .enumerate()
            .map(|(k, r)| (k + 1, f64::from(r.1)))
            .filter(|(_, v)| *v < lo || *v > hi)
            .collect();
        let late: Vec<(usize, f64)> = row[landing + F..=end]
            .iter()
            .enumerate()
            .map(|(k, r)| (landing + F + k + 1, f64::from(r.1)))
            .filter(|(_, v)| (v - new).abs() > t)
            .collect();
        let step_line = (old - new).abs() / F as f64 + t;
        let steps: Vec<(usize, f64)> = trace
            .windows(2)
            .enumerate()
            .map(|(k, w)| (k + 1, (w[1] - w[0]).abs()))
            .filter(|(_, d)| *d > step_line)
            .map(|(k, d)| (k as i64 - MEAN_FRAMES as i64 + 1, d))
            .map(|(k, d)| (k.max(0) as usize, d))
            .collect();
        println!(
            "sky rays {rays}: rule 3 over the centre: old {old:.3}, new {new:.3} (frame {}), landing at frame {}; out of [{lo:.3}, {hi:.3}] {out:?}; off new by over {t} from frame {} {late:?}; the step clause (no one-frame change over {step_line:.3}): {} (frame after the dig, step) {steps:?}",
            end + 1,
            landing + 1,
            landing + F + 1,
            if steps.is_empty() { "holds" } else { "fails" }
        );
        assert_ne!(
            word_at[0], 3,
            "sky rays {rays}: frame 1 reads the own summary"
        );
        assert!(out.is_empty(), "sky rays {rays}: the trace leaves the band: {out:?}");
        assert!(late.is_empty(), "sky rays {rays}: off new from the landing plus F: {late:?}");
        if rays == 1 {
            assert!(row[0].0 > 0, "the dig's first frame casts over the rings");
        } else {
            assert!(row.iter().all(|&(n, _)| n == 0), "sky rays 0 never casts");
        }
    }
}

/// Plan 0086 Phase 19 bar 4: `reading::forest_floor_run` with the turn (64 settle frames, a
/// 90-degree yaw over 10 frames, 64 still frames) at `60,180` and `10,180` and the headlamp
/// alone: the floor's p99 change per frame from the turn's first frame, its value at rest (the
/// mean of the last 16 frames), and the frames after the turn ends until it stays within 2
/// display levels of that. Under each sun the floor settles within the headlamp's frames plus
/// 16 (the pair's mean at age 16 steps by 1/17). `VXE_TEST_ADAPTER` picks the adapter (unset:
/// lavapipe).
#[test]
#[ignore = "soak"]
fn the_forest_after_a_turn() {
    let Some((mut rig, cam)) = support::terms::forest_rig() else {
        return;
    };
    println!("the_forest_after_a_turn on {}", rig.adapter);
    let mut settles = Vec::new();
    for (name, sky) in [
        ("sun 60,180", Some(reading::fixed_sun_sky(60.0))),
        ("sun 10,180", Some(reading::fixed_sun_sky(10.0))),
        ("the headlamp", None),
    ] {
        let r = reading::forest_floor_run(&mut rig, &cam, sky, true);
        let rest = reading::floor_means(&r.frames[r.frames.len() - 16..])[0];
        let after = &r.frames[10..];
        let settle = (0..after.len())
            .find(|&k| after[k..].iter().all(|v| v[0] <= rest + 2.0))
            .unwrap_or(after.len());
        let series: Vec<String> = r.frames.iter().map(|v| format!("{:.0}", v[0])).collect();
        println!(
            "forest after a turn, {name}: floor p99 at rest {rest:.2}, settled {settle} frames after the turn; per frame [{}]",
            series.join(" ")
        );
        settles.push(settle);
    }
    let lamp = settles[2];
    for (name, s) in ["60,180", "10,180"].iter().zip(&settles) {
        assert!(
            *s <= lamp + 16,
            "the floor under the sun at {name} settles in {s} frames, the headlamp's in {lamp}"
        );
    }
}
