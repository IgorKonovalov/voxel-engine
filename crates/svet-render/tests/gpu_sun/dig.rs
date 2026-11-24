// `gpu_sun`'s dig rig (Plan 0086 Phase 21): the sky term across the stale bit's switch and a
// dig's light, included by `gpu_sun.rs` (kept apart for the file-size cap).

/// The dig rig's lighting size; the lighting runs at full resolution, so a display pixel is a
/// lighting pixel.
const DIG_SIZE: [u32; 2] = [160, 90];
/// The dig: 1 m (10 voxels) of rock under the view's centre, carved at frame [`DIG_AT`].
const DIG_LO: [i32; 3] = [72, 91, FLOOR - 10];
const DIG_HI: [i32; 3] = [82, 101, FLOOR];
const DIG_AT: usize = 64;
/// Frames read after the dig.
const DIG_READ: usize = 60;

/// Over the floor of the 3 x 3 chunks, looking along +x 60 degrees down at the dig.
fn dig_camera() -> Camera {
    camera([60.0, 96.0])
}

/// A frame of the dig rig: [`DIG_SIZE`], lighting at full resolution, the lamp off, the
/// filter on, one sky ray, the coarse walk, manual exposure 1, `Sky::uniform(1.0)`.
fn dig_frame(device: &wgpu::Device) -> Frame {
    let mut f = frame(device, DIG_SIZE, ShadowWalk::Coarse);
    f.set_lighting(Lighting {
        filter: true,
        sky_rays: 1,
        ..f.lighting()
    });
    f.set_sky(Some(Sky {
        altitude_zero_z: FLOOR as f32,
        ..Sky::uniform(1.0)
    }));
    f
}

/// One step of `f`'s live queue over `world`'s bounded window (no streamer: the reach reads the
/// mirror alone, so a chunk outside the world is no chunk to wait for), the eye the camera's.
fn dig_queue_step(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    f: &mut Frame,
    gpu: &mut GpuWorld,
    world: &World,
    cam: &Camera,
) {
    let view = svet_render::QueueView {
        eye: cam.position.to_array(),
        forward: cam.basis().0.to_array(),
        resident: None,
        samples: None,
        interval_s: 1.0 / 60.0,
        hold: None,
        half_angle: 1.0,
        frustum: None,
        full_radius: Some(8),
        lod_radius: Some(8),
        focal_px: None,
    };
    f.bake_queue_step(device, queue, gpu, world, &view)
        .expect("the queue runs")
        .expect("the queue's step");
}

/// One read frame of the dig rig.
struct DigShot {
    /// The display's largest channel per pixel.
    level: Vec<u8>,
    /// The display's luma per pixel.
    luma: Vec<f32>,
    /// Whether the pixel cast the sky ray this frame.
    cast: Vec<bool>,
    /// The hit's position (`w` 1e30 or more at a miss).
    hit: Vec<[f32; 4]>,
}

fn dig_shot(device: &wgpu::Device, queue: &wgpu::Queue, f: &Frame) -> DigShot {
    use svet_render::debug::{read_rgba8, read_rgba32f};
    let display = read_rgba8(device, queue, f.target());
    DigShot {
        level: display
            .chunks(4)
            .map(|p| p[0].max(p[1]).max(p[2]))
            .collect(),
        luma: display
            .chunks(4)
            .map(|p| 0.2126 * f32::from(p[0]) + 0.7152 * f32::from(p[1]) + 0.0722 * f32::from(p[2]))
            .collect(),
        cast: read_rgba8(device, queue, f.visibility_samples())
            .chunks(4)
            .map(|p| p[3] > 127)
            .collect(),
        hit: read_rgba32f(device, queue, f.light_hit_positions()),
    }
}

/// The voxel distance from hit `h` to the dig's box dilated by `d` voxels (0 inside).
fn to_dig(h: [f32; 4], d: f32) -> f32 {
    let mut s = 0.0f32;
    for k in 0..3 {
        let (lo, hi) = (DIG_LO[k] as f32 - d, DIG_HI[k] as f32 + d);
        let e = (lo - h[k]).max(h[k] - hi).max(0.0);
        s += e * e;
    }
    s.sqrt()
}

/// The dig rig run: the live queue at the fly-through's settings (`queue_settings`, the
/// default slice, coarse 8 near 2) bakes the floor of 3 x 3 chunks in the first frames, the
/// dig at frame [`DIG_AT`], then [`DIG_READ`] frames: the shots of frame `DIG_AT - 1` and of
/// every frame after the dig. At frame `DIG_AT - 1` every chunk of the 3 x 3 holds a store
/// block, else the rig is at fault (a dig into baked ground is the case read). The queue's
/// whole-dig count after the dig's step is printed.
fn dig_run() -> Option<(Vec<DigShot>, World)> {
    dig_run_with(|_, _, _, _| {})
}

/// [`dig_run`], with `each` called after every frame's step (its index, the device, the
/// queue and the frame).
fn dig_run_with(
    mut each: impl FnMut(usize, &wgpu::Device, &wgpu::Queue, &Frame),
) -> Option<(Vec<DigShot>, World)> {
    let (device, queue) = device()?;
    let world = floor_world([3, 3, 1], |_| false);
    let mut gpu = mirror(&device, &queue, &world, None);
    let mut f = dig_frame(&device);
    f.start_bake_queue(
        &queue,
        &gpu,
        svet_render::queue_settings(),
        svet_render::DEFAULT_BAKE_SLICE_MS,
    );
    f.set_bake_coarse(8, 2);
    let cam = dig_camera();
    let mut shots = Vec::new();
    for i in 0..DIG_AT + DIG_READ {
        if i == DIG_AT {
            let lo = DIG_LO.map(|v| i64::from(v) * FIXED_ONE);
            let hi = DIG_HI.map(|v| i64::from(v) * FIXED_ONE);
            let r = world
                .apply(&EditOp::box_(Fixed3(lo), Fixed3(hi), Mode::Carve))
                .expect("the dig is resident");
            let (a, b) = r.aabb.expect("the dig changed voxels");
            let mut enc = device.create_command_encoder(&Default::default());
            gpu.sync(&world, &queue, &mut enc).expect("the mirror fits");
            queue.submit([enc.finish()]);
            f.invalidate(a.0, b.0);
        }
        dig_queue_step(&device, &queue, &mut f, &mut gpu, &world, &cam);
        if i == DIG_AT {
            let st = f.bake_queue().expect("the queue runs").stats();
            println!(
                "the dig's step: whole digs {}, digs {}",
                st.digs_whole, st.digs
            );
        }
        f.step(&device, &queue, &gpu, &cam, &LightParams::headlamp());
        each(i, &device, &queue, &f);
        if i + 1 == DIG_AT {
            let bare: Vec<[i32; 2]> = (0..9)
                .map(|k| [k % 3, k / 3])
                .filter(|c| f.store().block_words(ChunkCoord([c[0], c[1], 0])).is_none())
                .collect();
            assert!(
                bare.is_empty(),
                "rig fault: chunks {bare:?} hold no store block at frame {}",
                DIG_AT - 1
            );
        }
        if i + 1 >= DIG_AT {
            shots.push(dig_shot(&device, &queue, &f));
        }
    }
    Some((shots, world))
}

/// Pixel `i`'s largest excursion out of `[lo, hi]` over `shots` (levels), the index of its
/// frame in `shots` and whether it lies over `hi`.
fn band_excursion(shots: &[DigShot], i: usize, lo: i32, hi: i32) -> (i32, usize, bool) {
    let mut best = (0, 0, false);
    for (k, s) in shots.iter().enumerate() {
        let v = i32::from(s.level[i]);
        let out = (lo - v).max(v - hi).max(0);
        if out > best.0 {
            best = (out, k, v > hi);
        }
    }
    best
}

/// Plan 0086 Phase 21 bar 1: the direct part holds no sky term. The open floor of 3 x 3 chunks
/// under `Sky::uniform(1.0)` with no store block, so every floor face casts the sky ray; no
/// key light (the uniform sky has none), the lamp off, the filter on; 16 frames at 160 x 90.
/// At the centre pixel the accumulated direct part reads 0 in rgb at every frame, and at frame
/// 16 the filter's output reads the pair's term, `pair.x * E_sky(+z) * exposure` (`E_sky(+z)`
/// pi under a radiance of 1), within 5 %.
#[test]
fn the_direct_part_holds_no_sky_term() {
    let Some((device, queue)) = device() else {
        return;
    };
    let world = floor_world([3, 3, 1], |_| false);
    let gpu = mirror(&device, &queue, &world, None);
    let mut f = dig_frame(&device);
    let cam = camera([96.0, 96.0]);
    let w = DIG_SIZE[0] as usize;
    let c = (DIG_SIZE[1] as usize / 2) * w + w / 2;
    let mut worst = 0.0f32;
    for _ in 0..16 {
        f.step(&device, &queue, &gpu, &cam, &LightParams::headlamp());
        let d = svet_render::debug::read_rgba16f(&device, &queue, f.direct_accumulated())[c];
        worst = worst.max(d[0].max(d[1]).max(d[2]));
    }
    let pair = svet_render::debug::read_rgba16f(&device, &queue, f.visibility_pair())[c];
    let filt = svet_render::debug::read_rgba16f(&device, &queue, f.filtered())[c];
    let want = pair[0] * std::f32::consts::PI;
    println!(
        "the direct part's largest rgb over 16 frames {worst:.4}; the pair {:.4}, the filter's output {:?}, the pair's term {want:.4}",
        pair[0],
        &filt[..3]
    );
    assert_eq!(worst, 0.0, "the direct part holds a sky term");
    assert!(pair[0] > 0.9, "the open floor's pair reads {}", pair[0]);
    for k in 0..3 {
        assert!(
            (filt[k] / want - 1.0).abs() <= 0.05,
            "the filter's output {:?} against the pair's term {want}",
            &filt[..3]
        );
    }
}

/// Plan 0086 Phase 21 bar 2: the switch neither dips nor flashes. [`dig_run`]'s rig (the sky
/// alone lights the floor): over the ring's pixels (those that cast the sky ray at any frame
/// after the dig, the sign of a stale record; outside the dig's box dilated by one voxel and
/// within 64 voxels of it), with `before` the display level at frame 63 and `after` at frame
/// 124, every frame's level from 63 to 124 lies in
/// `[min(before, after) - 2, max(before, after) + 2]`; no ring pixel leaves the band. The ring is non-empty: at least one frame casts the
/// ray over at least 1 % of the pixels within 64 voxels of the box, else the rig is at fault.
#[test]
fn the_sky_term_switches_without_a_dip() {
    let Some((shots, _)) = dig_run() else {
        return;
    };
    let (first, last) = (&shots[0], &shots[shots.len() - 1]);
    let n = first.level.len();
    let near = |s: &DigShot, i: usize| s.hit[i][3] < 1e30 && to_dig(s.hit[i], 0.0) <= 64.0;
    let mut most_cast = 0.0f64;
    for s in &shots[1..] {
        let near_n = (0..n).filter(|&i| near(s, i)).count().max(1);
        let cast = (0..n).filter(|&i| near(s, i) && s.cast[i]).count();
        most_cast = most_cast.max(cast as f64 / near_n as f64);
    }
    let ring: Vec<usize> = (0..n)
        .filter(|&i| {
            shots[1..].iter().any(|s| s.cast[i])
                && near(last, i)
                && to_dig(last.hit[i], 1.0) > 0.0
                && to_dig(first.hit[i], 1.0) > 0.0
        })
        .collect();
    let mut left = 0usize;
    let mut worst = 0i32;
    // The leavers by the frame of their largest excursion and its side (under, over).
    let mut by_frame = std::collections::BTreeMap::<usize, [usize; 2]>::new();
    for &i in &ring {
        let (b, a) = (i32::from(first.level[i]), i32::from(last.level[i]));
        let (lo, hi) = (b.min(a) - 2, b.max(a) + 2);
        let (out, k, over) = band_excursion(&shots, i, lo, hi);
        worst = worst.max(out);
        left += usize::from(out > 0);
        if out > 0 {
            by_frame.entry(DIG_AT - 1 + k).or_default()[usize::from(over)] += 1;
        }
    }
    println!("the ring's leavers by frame (under, over): {by_frame:?}");
    println!(
        "the switch: the most a frame cast near the box {:.2} %, ring {} pixels, {} left the band (largest excursion {worst} levels)",
        100.0 * most_cast,
        ring.len(),
        left
    );
    assert!(most_cast >= 0.01, "the rig cast no ring: {most_cast}");
    assert!(!ring.is_empty(), "an empty ring");
    assert_eq!(
        left,
        0,
        "{left} of {} ring pixels left the band",
        ring.len()
    );
}

/// The mean of `luma` over the pixels `px`.
fn floor_mean(luma: &[f32], px: &[usize]) -> f64 {
    px.iter().map(|&i| f64::from(luma[i])).sum::<f64>() / px.len().max(1) as f64
}

/// [`landing_rule3`]'s reading of the hole's floor.
struct Rule3 {
    /// The floor's mean over the 8 frames before the landing, and over the last 8 frames.
    pre: f64,
    settled: f64,
    /// `[min(pre, settled) - T_fade, max(pre, settled) + T_fade]`.
    band: [f64; 2],
    /// The frames from the landing plus `F` on whose mean is off the baked whole's by over
    /// `T_fade`, with the difference; the frames after the dig out of the band, with the mean.
    off: Vec<(usize, f64)>,
    out: Vec<(usize, f64)>,
}

/// ADR-0076 rule 3 over the hole's floor `px` (its `T_fade` 4 and `F` 16): `shots` from frame
/// `DIG_AT - 1`, `lumas` the display's luma from frame `DIG_AT - 16`, `want` the baked whole's
/// mean, `at` the landing's index in `shots`.
fn landing_rule3(shots: &[DigShot], lumas: &[Vec<f32>], px: &[usize], want: f64, at: usize) -> Rule3 {
    const T_FADE: f64 = 4.0;
    const F: usize = 16;
    let mean = |s: &DigShot| floor_mean(&s.luma, px);
    let off = shots
        .iter()
        .enumerate()
        .skip(at + F)
        .map(|(k, s)| (DIG_AT + k - 1, mean(s) - want))
        .filter(|(_, d)| d.abs() > T_FADE)
        .collect();
    // `lumas[at + 15]` is the landing's frame.
    let pre = lumas[at + 7..at + 15].iter().map(|l| floor_mean(l, px)).sum::<f64>() / 8.0;
    let tail = &shots[shots.len() - 8..];
    let settled = tail.iter().map(mean).sum::<f64>() / 8.0;
    let band = [pre.min(settled) - T_FADE, pre.max(settled) + T_FADE];
    let out = shots
        .iter()
        .enumerate()
        .skip(1)
        .map(|(k, s)| (DIG_AT + k - 1, mean(s)))
        .filter(|(_, m)| *m < band[0] || *m > band[1])
        .collect();
    Rule3 {
        pre,
        settled,
        band,
        off,
        out,
    }
}

/// Plan 0086 Phase 21 bars 3 and 4 (re-stated 2026-11-19): a dig leaves no dots on its ring,
/// and the hole takes its light. [`dig_run`]'s rig (every chunk of the floor holds a block at
/// frame 63). Bar 3: in every frame after the dig, the share of the lighting pixels within 64
/// voxels of the box and outside the box dilated by one voxel whose display luma is over 2
/// times their value at frame 124 and over 2 times the 3 x 3 neighbourhood's mean there is
/// under 1 %, and 0 from 16 frames after the dig on; the same count on the hole's own faces
/// (inside the dilated box) is printed, not barred. Bar 4: over the pixels inside the dilated
/// box at frame 124, with `old` the display level at frame 63 and `new` at frame 124, the
/// pixels with a frame from 64 to 124 outside `[min(old, new) - 4, max(old, new) + 4]`
/// (ADR-0076 rule 3's form) are printed by the frame of their largest excursion, its side and
/// their sky term's source there ([`sky_sources`]: the pair, the store or the chain), not
/// barred (the chain's prior on the no-ray path is a later plan's). The landing, in ADR-0076
/// rule 3's form (restated by Plan 0094 Phase 7a, the no-ray value's fade): with the dig job's
/// landing frame (the queue's `digs_baked` rises; printed), the hole's floor (hits within half
/// a voxel of the box's bottom, inside its footprint) reads the same pixels' mean on the dug
/// world baked whole and run 64 frames within `T_fade` 4 levels from the landing plus `F` 16
/// frames on, and no frame after the dig leaves `[min - T_fade, max + T_fade]` of the floor's
/// pre-landing mean (the 8 frames before the landing, the dig's own among them, read at the
/// same pixels) and its settled mean (the last 8 frames); no frame has over 5 % of the pixels changing by over 16 levels from the frame
/// before.
/// Soak: 15.1 s on lavapipe in Plan 0086 Phase 24i's quiet gate.
#[test]
#[ignore = "soak"]
fn a_dig_leaves_no_dots() {
    let mut baked_count = Vec::new();
    let mut src: Vec<Vec<u8>> = Vec::new();
    // The display's luma per frame from 16 frames before the dig (frame `DIG_AT - 16 + k`).
    let mut lumas: Vec<Vec<f32>> = Vec::new();
    let eye = dig_camera().position.to_array();
    let Some((shots, world)) = dig_run_with(|i, device, queue, f| {
        baked_count.push(f.bake_queue().expect("the queue runs").stats().digs_baked);
        if i + 16 >= DIG_AT {
            lumas.push(dig_shot(device, queue, f).luma);
        }
        if i + 1 >= DIG_AT {
            src.push(sky_sources(device, queue, f, eye));
        }
    }) else {
        return;
    };
    let last = &shots[shots.len() - 1];
    let n = last.level.len();
    let w = DIG_SIZE[0] as usize;
    let h = n / w;
    let near = |s: &DigShot, i: usize| s.hit[i][3] < 1e30 && to_dig(s.hit[i], 0.0) <= 64.0;
    let settled_mean = |i: usize| {
        let (x, y) = ((i % w) as i64, (i / w) as i64);
        let (mut s, mut k) = (0.0f32, 0.0f32);
        for dy in -1..=1 {
            for dx in -1..=1 {
                let (qx, qy) = (x + dx, y + dy);
                if qx >= 0 && qy >= 0 && (qx as usize) < w && (qy as usize) < h {
                    s += last.luma[qy as usize * w + qx as usize];
                    k += 1.0;
                }
            }
        }
        s / k
    };
    let mut worst_change = 0.0f64;
    let mut fails = Vec::new();
    for (k, s) in shots.iter().enumerate().skip(1) {
        let near_n = (0..n).filter(|&i| near(s, i)).count().max(1);
        let dot = |i: usize| s.luma[i] > 2.0 * last.luma[i] && s.luma[i] > 2.0 * settled_mean(i);
        let in_box = |i: usize| to_dig(s.hit[i], 1.0) == 0.0;
        let ring = (0..n)
            .filter(|&i| near(s, i) && dot(i) && !in_box(i))
            .count();
        let hole = (0..n).filter(|&i| near(s, i) && dot(i) && in_box(i)).count();
        let prev = &shots[k - 1];
        let changed = (0..n)
            .filter(|&i| s.level[i].abs_diff(prev.level[i]) > 16)
            .count();
        let share = ring as f64 / near_n as f64;
        worst_change = worst_change.max(changed as f64 / n as f64);
        println!(
            "frame {}: dots on the ring {ring} ({:.3} %), on the hole's faces {hole} ({:.3} %), changed over 16 levels {:.3} %",
            DIG_AT + k - 1,
            100.0 * share,
            100.0 * hole as f64 / near_n as f64,
            100.0 * changed as f64 / n as f64
        );
        if share >= 0.01 || (k > 16 && ring > 0) {
            fails.push(DIG_AT + k - 1);
        }
    }
    // Bar 4's band over the hole's own faces.
    let first = &shots[0];
    let hole_px: Vec<usize> = (0..n)
        .filter(|&i| last.hit[i][3] < 1e30 && to_dig(last.hit[i], 1.0) == 0.0)
        .collect();
    let mut left = 0usize;
    let mut worst_out = 0i32;
    // (frame, side, source) -> leavers.
    let mut by_frame = std::collections::BTreeMap::<(usize, &str, &str), usize>::new();
    for &i in &hole_px {
        let (o, nw) = (i32::from(first.level[i]), i32::from(last.level[i]));
        let (lo, hi) = (o.min(nw) - 4, o.max(nw) + 4);
        let (out, k, over) = band_excursion(&shots, i, lo, hi);
        worst_out = worst_out.max(out);
        left += usize::from(out > 0);
        if out > 0 {
            let side = if over { "over" } else { "under" };
            let source = ["pair", "store", "chain"][usize::from(src[k][i])];
            *by_frame.entry((DIG_AT - 1 + k, side, source)).or_default() += 1;
        }
    }
    println!("the hole's leavers by (frame, side, source): {by_frame:?}");
    // The frame the dig job landed: the first frame after the dig whose `digs_baked` rose.
    let at_dig = baked_count[DIG_AT - 1];
    let landed = (DIG_AT..baked_count.len()).find(|&i| baked_count[i] > at_dig);
    // The hole's floor against the dug world baked whole.
    let floor_px: Vec<usize> = (0..n)
        .filter(|&i| {
            let p = last.hit[i];
            p[3] < 1e30
                && (p[2] - DIG_LO[2] as f32).abs() <= 0.5
                && (DIG_LO[0] as f32..DIG_HI[0] as f32).contains(&p[0])
                && (DIG_LO[1] as f32..DIG_HI[1] as f32).contains(&p[1])
        })
        .collect();
    let (device, queue) = device().expect("the device opened once");
    let mut gpu = mirror(&device, &queue, &world, None);
    let mut f = dig_frame(&device);
    f.bake_until(
        &device,
        &queue,
        &mut gpu,
        &world,
        world.extent(),
        &svet_render::BakeSettings::default(),
        &mut |_| {},
    )
    .expect("the store fits");
    let cam = dig_camera();
    for _ in 0..64 {
        f.step(&device, &queue, &gpu, &cam, &LightParams::headlamp());
    }
    let baked = dig_shot(&device, &queue, &f);
    let mean = |l: &[f32]| floor_mean(l, &floor_px);
    let want = mean(&baked.luma);
    let at = landed.map_or(shots.len() - 1, |l| l + 1 - DIG_AT).min(shots.len() - 1);
    let rule3 = landing_rule3(&shots, &lumas, &floor_px, want, at);
    let (pre, settled, lo, hi) = (rule3.pre, rule3.settled, rule3.band[0], rule3.band[1]);
    let (off, out) = (&rule3.off, &rule3.out);
    let floor_by_frame: Vec<(usize, f64)> = [1usize, 2, 3, 4, 8, 16, 60]
        .iter()
        .map(|&k| (DIG_AT + k - 1, mean(&shots[k].luma)))
        .collect();
    println!(
        "the hole's faces: {} pixels, {left} left the band of 4 (largest excursion {worst_out}; printed, not barred); the dig job landed at frame {landed:?}",
        hole_px.len()
    );
    println!(
        "the hole's floor: {} pixels, baked whole {want:.2}, by frame {floor_by_frame:?}; pre-landing mean {pre:.2}, settled {settled:.2}; frames off by over 4 levels from the landing plus 16 on {off:?}; frames out of [{lo:.2}, {hi:.2}] {out:?}; the most pixels changing over 16 levels in a frame {:.3} %",
        floor_px.len(),
        100.0 * worst_change
    );
    assert!(fails.is_empty(), "dots on the ring over the bar at frames {fails:?}");
    assert!(!hole_px.is_empty(), "the hole is not in view");
    assert!(!floor_px.is_empty(), "the hole's floor is not in view");
    assert!(landed.is_some(), "the dig job did not land in the rig's frames");
    assert!(off.is_empty(), "the hole's floor against {want}: {off:?}");
    assert!(out.is_empty(), "the hole's floor out of [{lo}, {hi}]: {out:?}");
    assert!(worst_change <= 0.05, "{worst_change}");
}

/// A lighting pixel's sky-term source ([`sky_sources`]).
const SRC_PAIR: u8 = 0;
const SRC_STORE: u8 = 1;
const SRC_CHAIN: u8 = 2;

/// Each lighting pixel's sky-term source in `f`'s frame seen from `eye`: the pair where it
/// cast the sky ray, the store where its face's own cell record holds the direction's baked
/// visibility ([`face_record`]), else the chain (a miss included).
fn sky_sources(device: &wgpu::Device, queue: &wgpu::Queue, f: &Frame, eye: [f32; 3]) -> Vec<u8> {
    use svet_render::debug::{read_rgba8, read_rgba32f};
    let vis = read_rgba8(device, queue, f.visibility_samples());
    let hits = read_rgba32f(device, queue, f.light_hit_positions());
    hits.iter()
        .enumerate()
        .map(|(i, h)| {
            if vis[4 * i + 3] > 127 {
                SRC_PAIR
            } else if h[3] < 1e30 && face_record(f, *h, eye).is_some() {
                SRC_STORE
            } else {
                SRC_CHAIN
            }
        })
        .collect()
}

/// The sky visibility the cell record of hit `h`'s face (the face seen from `eye`, on the axis
/// where `h` lies nearest a voxel boundary) holds in `f`'s store, `None` where it holds none.
fn face_record(f: &Frame, h: [f32; 4], eye: [f32; 3]) -> Option<f32> {
    let axis = (0..3)
        .min_by(|&a, &b| (h[a] - h[a].round()).abs().total_cmp(&(h[b] - h[b].round()).abs()))
        .unwrap_or(2);
    let s = if eye[axis] > h[axis] { 1.0 } else { -1.0 };
    let mut v = [0i32; 3];
    for k in 0..3 {
        let n = if k == axis { s } else { 0.0 };
        v[k] = (h[k] - 0.5 * n).floor() as i32;
    }
    let face = 2 * axis + usize::from(s > 0.0);
    let block = f.store().block_words(ChunkCoord(v.map(|c| c.div_euclid(64))))?;
    let l = v.map(|c| c.rem_euclid(64));
    let b = l.map(|c| c / 8);
    let dir = block[(b[0] + 8 * (b[1] + 8 * b[2])) as usize];
    if dir == 0 {
        return None;
    }
    let cell = svet_render::BakeSettings::default().cell as i32;
    let n = 8 / cell;
    let c = l.map(|k| (k % 8) / cell);
    let sub = (c[0] + n * (c[1] + n * c[2])) as usize;
    let w = svet_render::internals::BRICK_WORDS;
    let end = (dir >> 11) as usize;
    let at = (end + w * sub).checked_sub(w * (n * n * n) as usize)?;
    svet_render::internals::record_visibility(block.get(at..at + w)?, face)
}

/// A reset pair's seed age where the seed is a record's value (temporal.wgsl's `SEED_AGE`),
/// and where it is the column's answer (`SEED_COLUMN`).
const SEED_AGE: f32 = 8.0;
const SEED_COLUMN: f32 = 2.0;

/// The `+z` record of the voxel under floor hit `h` (a face at integral z) in `f`'s store:
/// its sky visibility for `+z` and whether it is `SKY_STALE`; `None` where no record holds the
/// direction under a sky.
fn up_record(f: &Frame, h: [f32; 4]) -> Option<(f32, bool)> {
    let v = [h[0].floor() as i32, h[1].floor() as i32, h[2].round() as i32 - 1];
    let block = f.store().block_words(ChunkCoord(v.map(|c| c.div_euclid(64))))?;
    let l = v.map(|c| c.rem_euclid(64));
    let b = l.map(|c| c / 8);
    let dir = block[(b[0] + 8 * (b[1] + 8 * b[2])) as usize];
    if dir == 0 {
        return None;
    }
    let cell = svet_render::BakeSettings::default().cell as i32;
    let n = 8 / cell;
    let c = l.map(|k| (k % 8) / cell);
    let sub = (c[0] + n * (c[1] + n * c[2])) as usize;
    let w = svet_render::internals::BRICK_WORDS;
    let end = (dir >> 11) as usize;
    let at = end - w * (n * n * n) as usize + w * sub;
    let rec = &block[at..at + w];
    svet_render::internals::record_visibility(rec, 5)
        .map(|vis| (vis, svet_render::internals::record_sky_stale(rec)))
}

/// The column map's cell edge in voxels (temporal.wgsl's `COLUMN_CELL`) and the seed's lift
/// along the face normal (`SEED_LIFT`, `COLUMN_CELL + 0.5`).
const COLUMN_CELL: i32 = 2;
const SEED_LIFT: f32 = COLUMN_CELL as f32 + 0.5;

/// Whether point `p` lies above the top of solid of its column-map cell in `world`
/// (walk.wgsl's `above_column`; the scan covers the rig's one chunk layer).
fn above_column(world: &World, p: [f32; 3]) -> bool {
    let x = [p[0].floor() as i32, p[1].floor() as i32];
    let cell = x.map(|k| k.div_euclid(COLUMN_CELL) * COLUMN_CELL);
    let mut top = i32::MIN;
    for dx in 0..COLUMN_CELL {
        for dy in 0..COLUMN_CELL {
            for z in (0..64).rev() {
                if world
                    .material_at([cell[0] + dx, cell[1] + dy, z])
                    .is_some_and(|m| m != AIR)
                {
                    top = top.max(z);
                    break;
                }
            }
        }
    }
    top == i32::MIN || p[2] >= (top + 1) as f32
}

/// The `+z` seed temporal.wgsl's order takes at floor hit `h`: the record's visibility
/// ([`up_record`]), else the column test at the hit lifted `SEED_LIFT` voxels up (1 above
/// the column's top of solid, else 0). The chain is not in the order.
fn up_seed(f: &Frame, world: &World, h: [f32; 4]) -> f32 {
    if let Some((v, _)) = up_record(f, h) {
        return v;
    }
    if above_column(world, [h[0], h[1], h[2] + SEED_LIFT]) {
        1.0
    } else {
        0.0
    }
}

/// Plan 0086 Phase 21 bar 4a, restated by Plan 0094 Phase 7a (the seed's weight by its
/// source, Phase 5): a reset pixel starts from its record, else from the column. On
/// [`dig_run`]'s rig (the floor baked before the dig, the reset), at the first frame after the
/// dig, every lighting pixel inside the dig's box dilated by one voxel that cast the sky ray
/// (its history reset by the edit) reads the pair's age `SEED_AGE` or `SEED_COLUMN`; on a `+z`
/// face it is `(record, SEED_AGE)` where the face's record holds a value ([`up_record`]) and
/// `(column, SEED_COLUMN)` where it holds none, the mean within `1 / (age + 1)` of the seed
/// [`up_seed`] takes, and both kinds are in the box. On the open floor under
/// `Sky::uniform(1.0)` with the record withheld (no store block, bar 1's rig) the centre
/// pixel's first frame reads `(column, SEED_COLUMN)`: the column's 1. The mean of the pair over
/// the box's cast pixels at frames 1, 8, 16 and 60 after the dig is printed, with the count of
/// `+z` pixels whose face holds a record (stale) at frame 1. Plan 0094 Phase 5 bar 1's second
/// clause: at frame 1 the centre chunk's summary record is `SKY_STALE` (the dig job queued and
/// not landed) and every `+z` pixel inside the box whose face holds no record cast the sky ray
/// (arm A's answer, so no source word `own summary`).
/// Soak: 14.2 s on lavapipe in Plan 0086 Phase 24i's quiet gate.
#[test]
#[ignore = "soak"]
fn a_reset_pixel_starts_from_its_record() {
    let Some((device, queue)) = device() else {
        return;
    };
    {
        let world = floor_world([3, 3, 1], |_| false);
        let gpu = mirror(&device, &queue, &world, None);
        let mut f = dig_frame(&device);
        let w = DIG_SIZE[0] as usize;
        let c = (DIG_SIZE[1] as usize / 2) * w + w / 2;
        f.step(&device, &queue, &gpu, &camera([96.0, 96.0]), &LightParams::headlamp());
        let pair = svet_render::debug::read_rgba16f(&device, &queue, f.visibility_pair())[c];
        println!("the open floor's first frame, the record withheld: pair {:.4} at age {}", pair[0], pair[3]);
        assert_eq!((pair[0], pair[3]), (1.0, SEED_COLUMN));
    }
    let (mut plus_z, mut with_record, mut stale_n) = (0usize, 0usize, 0usize);
    let (mut worst_record, mut worst_column) = (0.0f32, 0.0f32);
    let mut means = Vec::new();
    // Frame 1: the centre's summary's stale bit, and the dug +z pixels with no record that
    // did not cast the sky ray (a summary answered them).
    let (mut summary_stale, mut unanswered) = (None, 0usize);
    // The rig's world after its dig, for the column test.
    let dug = floor_world([3, 3, 1], |_| false);
    dug.apply(&EditOp::box_(
        Fixed3(DIG_LO.map(|v| i64::from(v) * FIXED_ONE)),
        Fixed3(DIG_HI.map(|v| i64::from(v) * FIXED_ONE)),
        Mode::Carve,
    ))
    .expect("the dig is resident");
    let r = dig_run_with(|i, device, queue, f| {
        let k = i as i64 - DIG_AT as i64 + 1;
        if ![1, 8, 16, 60].contains(&k) {
            return;
        }
        let pair = svet_render::debug::read_rgba16f(device, queue, f.visibility_pair());
        let vis = svet_render::debug::read_rgba8(device, queue, f.visibility_samples());
        let hits = svet_render::debug::read_rgba32f(device, queue, f.light_hit_positions());
        let (mut sum, mut n) = (0.0f64, 0usize);
        if k == 1 {
            let block = f.store().block_words(ChunkCoord([1, 1, 0])).expect("the centre's block");
            let at = svet_render::internals::SUMMARY_AT as usize;
            let rec = &block[at..at + svet_render::internals::BRICK_WORDS];
            summary_stale = Some(svet_render::internals::record_sky_stale(rec));
            for (j, h) in hits.iter().enumerate() {
                let floor = h[3] < 1e30 && (h[2] - h[2].round()).abs() < 1e-3;
                if floor && to_dig(*h, 0.0) == 0.0 && up_record(f, *h).is_none() && vis[4 * j + 3] <= 127 {
                    unanswered += 1;
                }
            }
        }
        for (j, h) in hits.iter().enumerate() {
            if h[3] >= 1e30 || to_dig(*h, 1.0) > 0.0 || vis[4 * j + 3] <= 127 {
                continue;
            }
            sum += f64::from(pair[j][0]);
            n += 1;
            if k != 1 {
                continue;
            }
            assert!(
                pair[j][3] == SEED_AGE || pair[j][3] == SEED_COLUMN,
                "pixel {j}: age {}",
                pair[j][3]
            );
            if (h[2] - h[2].round()).abs() < 1e-3 {
                plus_z += 1;
                let d = (pair[j][0] - up_seed(f, &dug, *h)).abs();
                if let Some((_, st)) = up_record(f, *h) {
                    with_record += 1;
                    stale_n += usize::from(st);
                    assert_eq!(pair[j][3], SEED_AGE, "pixel {j} holds a record");
                    worst_record = worst_record.max(d);
                } else {
                    assert_eq!(pair[j][3], SEED_COLUMN, "pixel {j} holds no record");
                    worst_column = worst_column.max(d);
                }
            }
        }
        means.push((k, sum / n.max(1) as f64, n));
    });
    if r.is_none() {
        return;
    }
    println!(
        "frame 1: {plus_z} cast +z pixels in the box, {with_record} with a record, {stale_n} stale"
    );
    println!(
        "the box's cast pixels' mean pair at frames after the dig (frame, mean, pixels): {means:?}; largest |pair - seed| over the +z pixels at frame 1: with a record {worst_record:.4}, without {worst_column:.4}"
    );
    println!(
        "frame 1: the centre's summary SKY_STALE {summary_stale:?}; dug +z pixels with no record and no ray {unanswered}"
    );
    assert_eq!(summary_stale, Some(true), "the dig job's chunk summary at frame 1");
    assert_eq!(unanswered, 0, "a stale summary answered a no-record face");
    assert!(with_record > 0, "no +z pixel with a record inside the box at frame 1");
    assert!(plus_z > with_record, "no +z pixel without a record inside the box at frame 1");
    assert!(worst_record <= 1.0 / (SEED_AGE + 1.0), "{worst_record}");
    assert!(worst_column <= 1.0 / (SEED_COLUMN + 1.0), "{worst_column}");
}

/// row (2 x 1 x 2 chunks), the column of chunk (0, 0, 0) baked under `Sky::uniform(1.0)`, so the
/// floor's neighbour (1, 0, 0) holds no block and waits in the pool; the live
/// queue at the fly-through's settings. A 10-voxel rock box stamped into the air chunk
/// (0, 0, 1) before the queue's first step: after that step the chunk is in the queue's digs
/// (`digs` or `dig_open`) and, where both are in the jobs, opens ahead of (1, 0, 0); stepped
/// until it holds no track, its block holds a record on every listed brick, each baked under
/// the sky (`VIS_BAKED`) and none `SKY_STALE`. Then a box invalidated in the air chunk
/// (1, 0, 1) with no voxel changed (an empty prefab's stamp) lands no block and leaves the
/// chunk in `faceless`.
#[test]
fn a_stamp_into_air_is_a_dig() {
    let Some((device, queue)) = device() else {
        return;
    };
    let world = floor_world([2, 1, 2], |_| false);
    let mut gpu = mirror(&device, &queue, &world, None);
    let mut f = dig_frame(&device);
    let (air, neighbour, empty) = (
        ChunkCoord([0, 0, 1]),
        ChunkCoord([1, 0, 0]),
        ChunkCoord([1, 0, 1]),
    );
    // Only the first column baked: the neighbour holds no block and waits in the pool.
    f.bake_until(
        &device,
        &queue,
        &mut gpu,
        &world,
        ChunkExtent::new([0, 0, 0], [1, 1, 2]),
        &svet_render::BakeSettings::default(),
        &mut |_| {},
    )
    .expect("the store fits");
    assert!(f.store().block_words(neighbour).is_none(), "withheld");
    f.start_bake_queue(
        &queue,
        &gpu,
        svet_render::queue_settings(),
        svet_render::DEFAULT_BAKE_SLICE_MS,
    );
    let cam = camera([64.0, 32.0]);
    let stamp = |lo: [i32; 3], hi: [i32; 3]| {
        let r = world
            .apply(&EditOp::box_(
                Fixed3(lo.map(|v| i64::from(v) * FIXED_ONE)),
                Fixed3(hi.map(|v| i64::from(v) * FIXED_ONE)),
                Mode::Fill(ROCK),
            ))
            .expect("the stamp is resident");
        r.aabb.expect("the stamp changed voxels")
    };
    let (a, b) = stamp([20, 20, 80], [30, 30, 90]);
    let mut enc = device.create_command_encoder(&Default::default());
    gpu.sync(&world, &queue, &mut enc).expect("the mirror fits");
    queue.submit([enc.finish()]);
    f.invalidate(a.0, b.0);
    dig_queue_step(&device, &queue, &mut f, &mut gpu, &world, &cam);
    let q = f.bake_queue().expect("the queue runs");
    let tracks = q.tracks(air);
    let jobs = q.jobs();
    let at = |c: ChunkCoord| jobs.iter().position(|j| *j == c);
    println!(
        "after the stamp's step: the air chunk's tracks {tracks:?}, at {:?} in the jobs, the neighbour at {:?}",
        at(air),
        at(neighbour)
    );
    assert!(
        tracks.contains(&"digs") || tracks.contains(&"dig_open"),
        "{tracks:?}"
    );
    if let (Some(x), Some(y)) = (at(air), at(neighbour)) {
        assert!(x < y, "the dig at {x}, the neighbour at {y}");
    }
    let mut steps = 1;
    while steps < 600
        && (f.store().block_words(air).is_none()
            || !f.bake_queue().is_some_and(|q| q.tracks(air).is_empty()))
    {
        dig_queue_step(&device, &queue, &mut f, &mut gpu, &world, &cam);
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("device poll");
        steps += 1;
    }
    let block = f.store().block_words(air).expect("the stamped chunk's block landed");
    let w = svet_render::internals::BRICK_WORDS;
    let (mut bricks, mut baked, mut stale) = (0, 0, 0);
    for slot in 0..512usize {
        let dir = block[slot];
        if dir == 0 {
            continue;
        }
        let rec = &block[(dir >> 11) as usize - w..(dir >> 11) as usize];
        bricks += 1;
        baked += usize::from((0..6).any(|k| svet_render::internals::record_visibility(rec, k).is_some()));
        stale += usize::from(svet_render::internals::record_sky_stale(rec));
    }
    println!(
        "landed after {steps} steps: {bricks} bricks, {baked} baked under the sky, {stale} stale"
    );
    assert!(bricks > 0, "no brick listed");
    assert_eq!(baked, bricks);
    assert_eq!(stale, 0);
    // An empty prefab's stamp: a box with no voxel changed.
    f.invalidate([70, 10, 70], [75, 15, 75]);
    for _ in 0..60 {
        dig_queue_step(&device, &queue, &mut f, &mut gpu, &world, &cam);
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("device poll");
    }
    let tracks = f.bake_queue().expect("the queue runs").tracks(empty);
    println!("the empty stamp's chunk: tracks {tracks:?}");
    assert!(f.store().block_words(empty).is_none());
    assert!(tracks.contains(&"faceless"), "{tracks:?}");
}
