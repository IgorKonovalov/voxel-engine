//! A listener walking past a hum with the early reflections on: each echo's tap slot glides
//! with the walk, at the walk's own rate, instead of stepping at every re-probe, so the output
//! clicks no more than an ideal rendering of the same walk. Also the echoes fading out when the
//! reflections are switched off.
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

/// One second of a sine of `hz` (whole hertz, so it loops cleanly) and amplitude 0.3.
fn hum(hz: f64) -> Sound {
    Sound::new(
        (0..RATE)
            .map(|n| {
                0.3 * (std::f64::consts::TAU * hz * f64::from(n) / f64::from(RATE)).sin() as f32
            })
            .collect::<Vec<_>>(),
    )
}

/// The fade test's hum: exactly two cycles per 256-frame block, so a steady tone's block RMS
/// does not ripple from one block to the next (a 440 Hz tone's ripples by 0.4 dB).
const FADE_HZ: f64 = 375.0;

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

/// The walk's numbers: the hum at `(8 m, 1 m, 0)`, the listener facing `+x` for 2 s, moved and
/// probed every 3 blocks.
const SOURCE: [f32; 3] = [80.0, 10.0, 0.0];
const BLOCKS: usize = 2 * RATE as usize / BLOCK;
/// Frames before the slot checks start: 200 ms.
const SETTLE_SLOTS: usize = 9_600;

/// A walk along `x`: where it starts and how fast, voxel units and voxels per second.
#[derive(Clone, Copy, Debug)]
struct Pace {
    start: f32,
    speed: f32,
}

/// 3 m/s from `x = 0`.
const WALK: Pace = Pace {
    start: 0.0,
    speed: 30.0,
};
/// The boost, 12 m/s from `x = -8 m`, so the 2 s end at 16 m inside the corridor.
const BOOST: Pace = Pace {
    start: -80.0,
    speed: 120.0,
};

impl Pace {
    /// The listener's position (voxel units) at block edge `e`.
    fn at(self, e: usize) -> [f32; 3] {
        [
            self.start + self.speed * (e * BLOCK) as f32 / RATE as f32,
            0.0,
            0.0,
        ]
    }
}

/// What the walk rendered: the output, the blocks a probe arrived before, and every slot's
/// state before and after each block.
struct Walk {
    out: Vec<f32>,
    probes: Vec<usize>,
    before: Vec<[TapSlot; MAX_TAPS]>,
    after: Vec<[TapSlot; MAX_TAPS]>,
}

/// The walk through `world` at `pace`, HRTF path, reflections on, the direct delay `delayed`,
/// as a 60 Hz game would drive it.
fn walk(world: &World, pace: Pace, delayed: bool) -> Walk {
    let mut mixer = Mixer::new(Hrtf::sadie2_d2(), MPV);
    mixer.set_direct_delay(delayed);
    let mut acoustics = Acoustics::new(mixer.clock());
    assert!(acoustics.reflections_on());
    let id = VoiceId(1);
    mixer.apply(Command::Play(
        id,
        hum(HZ),
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
                position: pace.at(b),
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

/// The direct path's distance gain `1 / max(d, 1 m)` at every block edge of the 3 m/s walk.
fn direct_gains() -> Vec<f64> {
    (0..=BLOCKS)
        .map(|e| {
            let p = WALK.at(e);
            let d = [0, 1, 2]
                .map(|k| f64::from(SOURCE[k] - p[k]))
                .iter()
                .map(|c| c * c)
                .sum::<f64>()
                .sqrt()
                * f64::from(MPV);
            1.0 / d.max(1.0)
        })
        .collect()
}

/// A block's slots at its start and its end.
type Ends = (
    [Option<(f64, f64)>; MAX_TAPS],
    [Option<(f64, f64)>; MAX_TAPS],
);

/// Per sounding slot after the first 200 ms of a walk at `pace`: the largest per-block delay
/// change, the largest difference from the ideal delay's change over the same block, and the
/// largest distance from the ideal delay; whether every sounding slot stayed above
/// [`TAP_SILENT`]; and how many sounding slots had no ideal hit at a block's ends (skipped).
fn slot_figures(
    w: &Walk,
    ideal: &[[Option<(f64, f64)>; MAX_TAPS]],
) -> (f64, f64, f64, bool, usize) {
    let (mut change, mut off_change, mut off, mut loud) = (0.0f64, 0.0f64, 0.0f64, true);
    let mut unmatched = 0;
    for b in SETTLE_SLOTS.div_ceil(BLOCK)..BLOCKS {
        for (i, (s0, s1)) in w.before[b].iter().zip(&w.after[b]).enumerate() {
            if s1.target.gain == 0.0 {
                continue;
            }
            loud &= s1.gain >= TAP_SILENT;
            let moved = f64::from(s1.delay) - f64::from(s0.delay);
            change = change.max(moved.abs());
            let (Some((d0, _)), Some((d1, _))) = (ideal[b][i], ideal[b + 1][i]) else {
                unmatched += 1;
                continue;
            };
            off_change = off_change.max((moved - (d1 - d0)).abs());
            off = off.max((f64::from(s1.delay) - d1).abs());
        }
    }
    (change, off_change, off, loud, unmatched)
}

/// **The arithmetic** (3 m/s): the physical rate is at most `2v / c x 256 = 4.48` frames per
/// block, and a probe every 9 blocks steps a target by at most 40.3 frames, which spread over
/// those 9 blocks is the same 4.48. So after the first 200 ms every sounding slot's per-block
/// delay change is at most `1.25 x 4.48 = 5.6` frames, within 1.0 frame of the ideal delay's
/// change over that block, and within `40.3 + 4.48 + 1.5 = 46` frames of the ideal delay (a
/// probe's step behind, a block of walk, rounding).
#[test]
fn walking_past_a_hum_glides_every_echo_and_clicks_no_more_than_an_ideal_render() {
    let corridor = corridor();
    let w = walk(&corridor, WALK, false);
    let ideal: Vec<_> = (0..=BLOCKS)
        .map(|e| ideal_slots(&corridor, WALK.at(e)))
        .collect();
    let (worst_change, worst_off_change, worst_off, loud, unmatched) = slot_figures(&w, &ideal);
    let settle_block = SETTLE.div_ceil(BLOCK);

    // At 12 m/s (2v / c = 7 percent, past the 5 percent cap) the slots fall behind: printed.
    let boost = walk(&corridor, BOOST, false);
    let boost_ideal: Vec<_> = (0..=BLOCKS)
        .map(|e| ideal_slots(&corridor, BOOST.at(e)))
        .collect();
    let (_, _, boost_lag, _, _) = slot_figures(&boost, &boost_ideal);
    println!(
        "boost at 12 m/s: {} probes, largest distance behind the ideal delay {boost_lag:.2} frames",
        boost.probes.len()
    );

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
    let direct = direct_gains();
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
        "walking at 3 m/s past a 440 Hz hum: {} probes; largest change {worst_change:.3} frames \
         per block, largest difference from the ideal change {worst_off_change:.3} frames, \
         largest distance from the ideal delay {worst_off:.2} frames; click index: mixer \
         {k_mixer:.3}, ideal reference {k_ref:.3}, stepped reference {k_stepped:.3}",
        w.probes.len()
    );
    assert!(loud, "a sounding slot fell below TAP_SILENT");
    assert_eq!(unmatched, 0, "sounding slots without an ideal hit");
    assert!(
        worst_change <= 1.25 * 4.48,
        "a slot moved {worst_change} frames in a block"
    );
    assert!(
        worst_off_change <= 1.0,
        "a slot's change differs from the ideal change by {worst_off_change} frames"
    );
    assert!(
        worst_off <= 46.0,
        "a slot is {worst_off} frames from its ideal delay"
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

/// The same walk with the direct path's propagation delay on: the direct path and every echo
/// carry the walk's Doppler, and the output still clicks no more than twice the ideal
/// reference (a 440 Hz render with no delay on the direct path; the Doppler moves the tone by
/// under 1 percent, its click index by under 2 percent).
#[test]
fn walking_with_the_direct_delay_clicks_no_more_than_an_ideal_render() {
    let corridor = corridor();
    let w = walk(&corridor, WALK, true);
    let ideal: Vec<_> = (0..=BLOCKS)
        .map(|e| ideal_slots(&corridor, WALK.at(e)))
        .collect();
    let smooth = reference(&direct_gains(), |b| (ideal[b], ideal[b + 1]));
    let k_ref = click_index(&smooth, SETTLE);
    let k = [0, 1].map(|c| click_index(&channel(&w.out, c), SETTLE));
    println!(
        "walking at 3 m/s with the direct delay on: click index left {:.3}, right {:.3}, ideal \
         reference {k_ref:.3}",
        k[0], k[1]
    );
    for (c, k) in k.iter().enumerate() {
        assert!(*k <= 2.0 * k_ref, "ear {c}: {k} against twice {k_ref}");
    }
}

/// A wall of `CAVE` over voxel `x in [50, 60)` and the listener 4.95 m in front of it at voxel
/// `(0.5, 128.5, 128.5)`, a 375 Hz hum 2 m to its left: 0.5 s with the echoes, then the
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
        hum(FADE_HZ),
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
    // Whole blocks, so the measured blocks are the mixer's.
    let half = 94 * BLOCK;
    let mut on = vec![0.0; 2 * half];
    mixer.render(&mut on, half);
    acoustics.set_reflections(false);
    update(&mut acoustics, &mut mixer);
    let mut off = vec![0.0; 2 * half];
    mixer.render(&mut off, half);
    let settle = SETTLE.div_ceil(BLOCK) * BLOCK;
    let both: Vec<f32> = on[2 * settle..].iter().chain(&off).copied().collect();
    let step = worst_rms_step_db(&both);
    let change = (0..2)
        .map(|c| {
            let (before, after) = (block_rms(&channel(&on, c)), block_rms(&channel(&off, c)));
            let (b, a) = (before[before.len() - 1], after[after.len() - 1]);
            println!("ear {c}: RMS {b:.5} with the echoes, {a:.5} 0.5 s after switching them off");
            (20.0 * (b / a).log10()).abs()
        })
        .fold(0.0, f64::max);
    println!("reflections off mid-hum: largest block-to-block RMS step {step:.4} dB");
    assert!(change > 0.5, "the echoes change the level by {change} dB");
    assert!(step <= 0.5, "RMS step {step} dB");
}
