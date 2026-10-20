//! A listener walking past a hum with the early reflections on: each echo's tap slot glides
//! with the walk instead of stepping at every re-probe, so the output clicks no more than an
//! ideal rendering of the same walk. Also the echoes fading out when the reflections are
//! switched off.
//!
//! The output's level and phase are not bounded per block: the corridor's echoes sum to more
//! than the direct sound and each one's phase moves up to 0.258 rad per block against it, so
//! even an ideal render beats by decibels from block to block. The click index [`click_index`]
//! separates a beat from a step.

use vxe_audio::*;
use vxe_world::*;

const MPV: f32 = 0.1;
/// The cave row: absorption 0.3 per metre, transmission 0.5.
const CAVE: MaterialId = 1;
/// The hum's frequency.
const HZ: f64 = 440.0;
/// Frames before the measurements start: 100 ms.
const SETTLE: usize = 4_800;

/// A world whose voxels come from a closure of the voxel coordinate.
struct FnSource<F>(F);

impl<F: Fn([i32; 3]) -> MaterialId + Sync> WorldSource for FnSource<F> {
    fn chunk(&self, coord: ChunkCoord, out: &mut Chunk) {
        let base = coord.min_voxel().0;
        for slot in 0..CHUNK_BRICKS {
            let b = brick_of_slot(slot);
            out.set_brick(
                slot,
                Brick::from_fn(|l| (self.0)([0, 1, 2].map(|k| base[k] + b[k] * BRICK_EDGE + l[k]))),
            );
        }
    }
}

/// `CAVE` wherever `solid` holds over chunks `min..max`, air elsewhere.
fn world(min: [i32; 3], max: [i32; 3], solid: impl Fn([i32; 3]) -> bool + Sync) -> World {
    let mut t = MaterialTable::new();
    let mut m = Material::named("cave");
    m.acoustic = [0.3, 0.5, 0.0];
    assert_eq!(t.push(m), CAVE);
    World::generate(
        ChunkExtent::new(min, max),
        &FnSource(move |v| if solid(v) { CAVE } else { AIR }),
        &t,
    )
}

/// The corridor: rock at `y >= 5 m`, `y <= -5 m` and `z <= -2 m`, open above and along `x`
/// from -12.8 m to 19.2 m.
fn corridor() -> World {
    world([-2, -1, -1], [3, 1, 1], |v| {
        v[1] >= 50 || v[1] <= -50 || v[2] <= -20
    })
}

/// One second of a 440 Hz sine of amplitude 0.3: 440 whole cycles, so it loops cleanly.
fn hum() -> Sound {
    Sound::new(
        (0..RATE)
            .map(|n| {
                0.3 * (std::f64::consts::TAU * HZ * f64::from(n) / f64::from(RATE)).sin() as f32
            })
            .collect::<Vec<_>>(),
    )
}

/// One channel of interleaved stereo.
fn channel(out: &[f32], c: usize) -> Vec<f64> {
    out.iter()
        .skip(c)
        .step_by(2)
        .map(|&x| f64::from(x))
        .collect()
}

/// RMS of each whole 256-frame block of `ch`.
fn block_rms(ch: &[f64]) -> Vec<f64> {
    ch.chunks_exact(BLOCK)
        .map(|b| (b.iter().map(|x| x * x).sum::<f64>() / BLOCK as f64).sqrt())
        .collect()
}

/// The largest `|20 log10(rms[k + 1] / rms[k])|` over consecutive blocks of either channel.
fn worst_rms_step_db(out: &[f32]) -> f64 {
    (0..2)
        .flat_map(|c| {
            let rms = block_rms(&channel(out, c));
            rms.windows(2)
                .map(|w| (20.0 * (w[1] / w[0]).log10()).abs())
                .collect::<Vec<_>>()
        })
        .fold(0.0, f64::max)
}

/// The click index of `x` over 200 ms windows (9600 frames, hop 4800) starting at `from`: per
/// window, the largest `|x[n + 1] - 2 x[n] + x[n - 1]|` over `w^2 sqrt(2) rms` with `w` the hum's
/// angular frequency per frame. A pure 440 Hz tone scores 1; a smooth sum of tones near 440 Hz
/// about its envelope's crest; a step of size `d` at least `d / (w^2 sqrt(2) rms)`. The largest
/// over the windows.
fn click_index(x: &[f64], from: usize) -> f64 {
    const WINDOW: usize = 9_600;
    const HOP: usize = 4_800;
    let w = std::f64::consts::TAU * HZ / f64::from(RATE);
    let mut worst = 0.0f64;
    let mut s = from;
    while s + WINDOW <= x.len() {
        let win = &x[s..s + WINDOW];
        let rms = (win.iter().map(|v| v * v).sum::<f64>() / WINDOW as f64).sqrt();
        let d2 = win
            .windows(3)
            .map(|t| (t[2] - 2.0 * t[1] + t[0]).abs())
            .fold(0.0, f64::max);
        worst = worst.max(d2 / (w * w * std::f64::consts::SQRT_2 * rms));
        s += HOP;
    }
    worst
}

/// The walk's numbers: the hum at `(8 m, 1 m, 0)`, the listener from `x = 0` at 3 m/s facing
/// `+x` for 2 s, moved and probed every 3 blocks.
const SOURCE: [f32; 3] = [80.0, 10.0, 0.0];
const SPEED_VOXELS: f32 = 30.0;
const BLOCKS: usize = 2 * RATE as usize / BLOCK;

/// The listener's position (voxel units) at block edge `e`.
fn walk_position(e: usize) -> [f32; 3] {
    [SPEED_VOXELS * (e * BLOCK) as f32 / RATE as f32, 0.0, 0.0]
}

/// What the walk rendered: the output, the blocks a probe arrived before, and every slot's
/// state before and after each block.
struct Walk {
    out: Vec<f32>,
    probes: Vec<usize>,
    before: Vec<[TapSlot; MAX_TAPS]>,
    after: Vec<[TapSlot; MAX_TAPS]>,
}

/// The walk through `world`, HRTF path, reflections on, as a 60 Hz game would drive it.
fn walk(world: &World) -> Walk {
    let mut mixer = Mixer::new(Hrtf::sadie2_d2(), MPV);
    let mut acoustics = Acoustics::new(mixer.clock());
    assert!(acoustics.reflections_on());
    let id = VoiceId(1);
    mixer.apply(Command::Play(
        id,
        hum(),
        PlayParams {
            looping: true,
            ..PlayParams::at(SOURCE)
        },
    ));
    let mut w = Walk {
        out: vec![0.0; 2 * BLOCKS * BLOCK],
        probes: Vec::new(),
        before: Vec::new(),
        after: Vec::new(),
    };
    for (b, chunk) in w.out.chunks_mut(2 * BLOCK).enumerate() {
        if b % 3 == 0 {
            let listener = Listener {
                position: walk_position(b),
                ..Listener::default()
            };
            mixer.apply(Command::SetListener(listener));
            let mut probed = false;
            acoustics.update(
                world,
                world.materials(),
                &listener,
                &[(id, SOURCE)],
                MPV,
                &mut |cmd| {
                    probed |= matches!(cmd, Command::SetAcoustics(..));
                    mixer.apply(cmd);
                },
            );
            if probed {
                w.probes.push(b);
            }
        }
        w.before.push(mixer.tap_slots(id).expect("playing"));
        mixer.render(chunk, BLOCK);
        w.after.push(mixer.tap_slots(id).expect("playing"));
    }
    w
}

/// Per slot at a listener position: the unrounded excess-path delay in frames and the gain of
/// its reflection, `None` for a ray that hits nothing or lies past the delay line.
fn ideal_slots(world: &World, listener: [f32; 3]) -> [Option<(f64, f64)>; MAX_TAPS] {
    let hits = reflections(
        world,
        world.materials(),
        listener,
        &reflection_directions(),
        MPV,
    );
    let occ = f64::from(occlusion(world, world.materials(), listener, SOURCE, MPV).gain);
    let dist = |a: [f32; 3], b: [f32; 3]| {
        let d = [0, 1, 2].map(|k| f64::from(a[k]) - f64::from(b[k]));
        (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() * f64::from(MPV)
    };
    let direct = dist(listener, SOURCE);
    hits.map(|h| {
        let h = h?;
        let path = dist(listener, h.point) + dist(h.point, SOURCE);
        let delay = (path - direct) / f64::from(SPEED_OF_SOUND) * f64::from(RATE);
        (delay <= MAX_DELAY_FRAMES as f64).then(|| (delay, f64::from(h.keep) / path.max(1.0) * occ))
    })
}

/// The walk rendered mono, an analytic 440 Hz sine of amplitude 0.3 through the direct path
/// and every slot: block `b` runs from `ends(b).0` to `ends(b).1`, each slot's delay and gain
/// and the direct gain (`direct[b]` to `direct[b + 1]`) interpolated linearly per sample.
fn reference(direct: &[f64], ends: impl Fn(usize) -> Ends) -> Vec<f64> {
    let w = std::f64::consts::TAU * HZ / f64::from(RATE);
    let mut y = Vec::with_capacity(BLOCKS * BLOCK);
    for b in 0..BLOCKS {
        let (start, end) = ends(b);
        for j in 0..BLOCK {
            let f = j as f64 / BLOCK as f64;
            let n = (b * BLOCK + j) as f64;
            let gd = direct[b] + (direct[b + 1] - direct[b]) * f;
            let mut v = gd * (w * n).sin();
            for (s0, s1) in start.iter().zip(&end) {
                // A slot that starts or stops sounding fades from or to its other end's delay.
                let (d0, g0) = s0.unwrap_or((s1.map_or(0.0, |s| s.0), 0.0));
                let (d1, g1) = s1.unwrap_or((d0, 0.0));
                let d = d0 + (d1 - d0) * f;
                v += (g0 + (g1 - g0) * f) * (w * (n - d)).sin();
            }
            y.push(0.3 * v);
        }
    }
    y
}

/// A block's slots at its start and its end.
type Ends = (
    [Option<(f64, f64)>; MAX_TAPS],
    [Option<(f64, f64)>; MAX_TAPS],
);

#[test]
fn walking_past_a_hum_glides_every_echo_and_clicks_no_more_than_an_ideal_render() {
    let corridor = corridor();
    let w = walk(&corridor);
    let settle_block = SETTLE.div_ceil(BLOCK);
    let ideal: Vec<_> = (0..=BLOCKS)
        .map(|e| ideal_slots(&corridor, walk_position(e)))
        .collect();

    // Per-slot continuity, read at every block edge after the first 100 ms.
    let max_glide = TAP_SLEW_FRAMES_PER_FRAME * BLOCK as f32;
    let (mut worst_glide, mut worst_off) = (0.0f32, 0.0f64);
    for b in settle_block..BLOCKS {
        for (i, (s0, s1)) in w.before[b].iter().zip(&w.after[b]).enumerate() {
            let glide = (s1.delay - s0.delay).abs();
            worst_glide = worst_glide.max(glide);
            assert!(
                glide <= max_glide + 1e-3,
                "block {b} slot {i}: delay {} -> {}",
                s0.delay,
                s1.delay
            );
            if s1.target.gain != 0.0 {
                assert!(
                    s1.gain >= TAP_SILENT,
                    "block {b} slot {i}: a sounding slot fell silent ({})",
                    s1.gain
                );
                let (d, _) = ideal[b + 1][i].expect("a sounding slot has an ideal hit");
                let off = (f64::from(s1.delay) - d).abs();
                worst_off = worst_off.max(off);
                assert!(
                    off <= 42.0,
                    "block {b} slot {i}: delay {} against the ideal {d}",
                    s1.delay
                );
            }
        }
    }

    // Ramped gains: toward the target by at most the one-pole's share of the distance.
    let share = 1.0 - (-(BLOCK as f32) / (RATE as f32 * SMOOTHING_SECONDS)).exp();
    for b in settle_block..BLOCKS {
        for (i, (s0, s1)) in w.before[b].iter().zip(&w.after[b]).enumerate() {
            let (g0, g1, t) = (s0.gain, s1.gain, s0.target.gain);
            assert!(
                (g1 - g0).abs() <= share * (t - g0).abs() + 1e-6,
                "block {b} slot {i}: gain {g0} -> {g1}, target {t}"
            );
            assert!(
                (t - g1) * (t - g0) >= -1e-12,
                "block {b} slot {i}: gain {g0} -> {g1} passed its target {t}"
            );
        }
    }

    // The click index against an ideal render and a stepped one.
    let direct: Vec<f64> = (0..=BLOCKS)
        .map(|e| {
            let p = walk_position(e);
            let d = [0, 1, 2]
                .map(|k| f64::from(SOURCE[k] - p[k]))
                .iter()
                .map(|c| c * c)
                .sum::<f64>()
                .sqrt()
                * f64::from(MPV);
            1.0 / d.max(1.0)
        })
        .collect();
    // Nothing that can click: every slot at the exact walk position at each block edge.
    let smooth = reference(&direct, |b| (ideal[b], ideal[b + 1]));
    // A swap: each slot's delay held at its value at the last probe block, stepped at the next.
    let mut held = ideal.clone();
    let mut last = 0;
    for (e, slots) in held.iter_mut().enumerate() {
        if w.probes.contains(&e) {
            last = e;
        }
        for (s, h) in slots.iter_mut().zip(&ideal[last]) {
            if let (Some(s), Some(h)) = (s.as_mut(), h) {
                s.0 = h.0;
            }
        }
    }
    let stepped = reference(&direct, |b| {
        let mut end = ideal[b + 1];
        for (s, h) in end.iter_mut().zip(&held[b]) {
            if let (Some(s), Some(h)) = (s.as_mut(), h) {
                s.0 = h.0;
            }
        }
        (held[b], end)
    });
    let k_ref = click_index(&smooth, SETTLE);
    let k_stepped = click_index(&stepped, SETTLE);
    let k_mixer = (0..2)
        .map(|c| click_index(&channel(&w.out, c), SETTLE))
        .fold(0.0, f64::max);
    println!(
        "walking at 3 m/s past a 440 Hz hum: {} probes; largest glide {worst_glide:.3} frames per \
         block, largest distance from the ideal delay {worst_off:.2} frames; click index: mixer \
         {k_mixer:.3}, ideal reference {k_ref:.3}, stepped reference {k_stepped:.3}",
        w.probes.len()
    );
    assert!(
        k_stepped >= 4.0 * 2.0 * k_ref,
        "the stepped reference {k_stepped} must score at least four times the bound {}",
        2.0 * k_ref
    );
    assert!(
        k_mixer <= 2.0 * k_ref,
        "the mixer's click index {k_mixer} against twice the reference's {k_ref}"
    );
}

/// A wall of `CAVE` over voxel `x in [50, 60)` and the listener 4.95 m in front of it at voxel
/// `(0.5, 128.5, 128.5)`, a 440 Hz hum 2 m to its left: 0.5 s with the echoes, then the
/// reflections switched off. The echo sum fades out over the smoother's 50 ms instead of
/// vanishing at a block edge.
#[test]
fn switching_the_reflections_off_fades_the_echoes_out() {
    let walled = world([0, 0, 0], [4, 4, 4], |v| (50..60).contains(&v[0]));
    let listener = Listener {
        position: [0.5, 128.5, 128.5],
        ..Listener::default()
    };
    let source = [0.5, 148.5, 128.5];
    let mut mixer = Mixer::new(Hrtf::sadie2_d2(), MPV);
    mixer.apply(Command::SetListener(listener));
    let mut acoustics = Acoustics::new(mixer.clock());
    let id = VoiceId(2);
    mixer.apply(Command::Play(
        id,
        hum(),
        PlayParams {
            looping: true,
            ..PlayParams::at(source)
        },
    ));
    let update = |acoustics: &mut Acoustics, mixer: &mut Mixer| {
        acoustics.update(
            &walled,
            walled.materials(),
            &listener,
            &[(id, source)],
            MPV,
            &mut |cmd| mixer.apply(cmd),
        );
    };
    update(&mut acoustics, &mut mixer);
    let half = RATE as usize / 2;
    let mut on = vec![0.0; 2 * half];
    mixer.render(&mut on, half);
    acoustics.set_reflections(false);
    update(&mut acoustics, &mut mixer);
    let mut off = vec![0.0; 2 * half];
    mixer.render(&mut off, half);
    let both: Vec<f32> = on[2 * SETTLE..].iter().chain(&off).copied().collect();
    let step = worst_rms_step_db(&both);
    let before = block_rms(&channel(&on, 0));
    let after = block_rms(&channel(&off, 0));
    let (b, a) = (before[before.len() - 1], after[after.len() - 1]);
    println!(
        "reflections off mid-hum: largest block-to-block RMS step {step:.4} dB; left RMS \
         {b:.5} with the echoes, {a:.5} 0.5 s after switching them off"
    );
    assert!(
        (20.0 * (b / a).log10()).abs() > 0.5,
        "the echoes change the level: {b} against {a}"
    );
    assert!(step <= 0.5, "RMS step {step} dB");
}
