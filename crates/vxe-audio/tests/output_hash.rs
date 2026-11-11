//! The mixer's output for a fixed scene, hashed: a change that should leave normal play
//! sample-identical (an allocation, a free or a clear moved, a store resized) keeps the hash.
//!
//! The scene: the HRTF path at the default balance; a looping tone that moves and takes
//! changing taps and cutoffs, one-shot bursts that start, end and are stopped, the listener
//! turning, and three rooms the tail hands over between, each handover further apart than the
//! spare lines take to clean, so every clear swaps at once. The file uses only API that
//! exists at 5274772, where the hash was first read, so it can be read there again.
//!
//! The samples go through `sin`, `cos` and `exp` (here and in the mixer), which Rust leaves to
//! the platform's libm, and those differ in the last bit between platforms. So the bit-exact
//! hash is Linux's pin and runs only there. Every platform runs the same scene against values
//! read on Linux within a tolerance far above that last-bit noise and far below what a moved
//! allocation, free or clear does to the output: the energy of each room's stretch of blocks
//! and a few single samples, one in each stretch.

use std::f32::consts::PI;

use vxe_audio::*;

const BLOCKS: u32 = 900;

/// A deterministic burst: `len` samples of a linear congruential sequence under a decay.
fn burst(seed: u32, len: usize) -> Sound {
    let mut x = seed;
    Sound::new(
        (0..len)
            .map(|n| {
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let white = (x >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0;
                white * 0.3 * (-(n as f32) / 4_000.0).exp()
            })
            .collect::<Vec<_>>(),
    )
}

fn tone(freq: f32, len: usize) -> Sound {
    Sound::new(
        (0..len)
            .map(|n| 0.2 * (2.0 * PI * freq * n as f32 / RATE as f32).sin())
            .collect::<Vec<_>>(),
    )
}

fn room(t60_s: f32, mean_free_path_m: f32) -> Command {
    Command::SetRoom(RoomTargets {
        t60_s,
        mean_free_path_m,
        energy: 1.0,
    })
}

/// Taps that move with `step`: delays across the 16 slots, gains falling with delay.
fn targets(step: u32) -> AcousticTargets {
    let mut t = AcousticTargets {
        cutoff_hz: 4_000.0 + 1_000.0 * (step % 7) as f32,
        gain: 0.8,
        ..AcousticTargets::default()
    };
    for (i, tap) in t.taps.iter_mut().enumerate() {
        let i = i as u32;
        *tap = Tap {
            delay_frames: 200 + 97 * i + 13 * (step % 11),
            gain: 0.3 / (1.0 + i as f32),
        };
    }
    t
}

/// FNV-1a over the samples' bits, little-endian.
fn fnv(out: &[f32], mut h: u64) -> u64 {
    for s in out {
        for b in s.to_bits().to_le_bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    h
}

/// Renders the fixed scene, handing each block's interleaved stereo output to `each`.
fn render_scene(mut each: impl FnMut(u32, &[f32])) {
    let mut m = Mixer::new(Hrtf::sadie2_d2(), 0.1);
    let mut out = vec![0.0; 2 * BLOCK];
    m.apply(room(0.8, 2.667));
    let looping = PlayParams {
        looping: true,
        ..PlayParams::at([30.0, 10.0, 0.0])
    };
    m.apply(Command::Play(VoiceId(1), tone(220.0, 4 * BLOCK), looping));
    m.apply(Command::SetAcoustics(VoiceId(1), targets(0)));
    for b in 0..BLOCKS {
        let a = b as f32 * 0.01;
        m.apply(Command::SetListener(Listener {
            forward: [a.cos(), a.sin(), 0.0],
            ..Listener::default()
        }));
        m.apply(Command::SetPosition(
            VoiceId(1),
            [30.0 * (a * 3.0).cos(), 10.0 + 20.0 * (a * 2.0).sin(), 2.0],
        ));
        if b % 4 == 0 {
            m.apply(Command::SetAcoustics(VoiceId(1), targets(b / 4)));
        }
        if b % 60 == 10 {
            let id = VoiceId(100 + b);
            m.apply(Command::Play(
                id,
                burst(b, 6_000),
                PlayParams::at([-20.0, 5.0, 3.0]),
            ));
            m.apply(Command::SetAcoustics(id, targets(b)));
        }
        if b % 60 == 25 {
            m.apply(Command::Stop(VoiceId(100 + b - 15)));
        }
        match b {
            250 => m.apply(room(1.6, 8.0)),
            500 => m.apply(room(0.8, 2.667)),
            750 => m.apply(room(2.5, 24.0)),
            _ => {}
        }
        m.render(&mut out, BLOCK);
        each(b, &out);
    }
}

#[test]
#[cfg_attr(
    not(target_os = "linux"),
    ignore = "libm's sin, cos and exp differ in the last bit across platforms; the pin is Linux's"
)]
fn a_fixed_scene_renders_to_a_pinned_hash() {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    render_scene(|_, out| h = fnv(out, h));
    println!("output hash {h:016x}");
    assert_eq!(h, 0x0bb1_bae2_e686_a53f, "the output of the fixed scene");
}

/// The blocks each room holds: `render_scene` hands over at blocks 250, 500 and 750.
const STRETCHES: [(u32, u32); 4] = [(0, 250), (250, 500), (500, 750), (750, BLOCKS)];

/// Single samples `(block, index into the interleaved block)`, one in each stretch.
const PROBES: [(u32, usize); 4] = [(120, 301), (380, 100), (610, 511), (820, 250)];

/// The scene's energy per stretch (sum of squared samples) within 1e-4 of the Linux reading,
/// relative, and each probe within 1e-5 of it, absolute. Nudging every `sin`, `cos` and `exp`
/// result in this file up by one ulp moves the energies by under 2e-6 and the probes by under
/// 2e-7; moving one handover by a single block moves a stretch's energy by about 2e-3 and a
/// probe by about 1e-4.
#[test]
fn a_fixed_scene_renders_to_pinned_energies_and_samples_on_every_platform() {
    let mut energy = [0.0f64; STRETCHES.len()];
    let mut probes = [0.0f32; PROBES.len()];
    render_scene(|b, out| {
        let k = STRETCHES.iter().position(|&(lo, hi)| (lo..hi).contains(&b));
        energy[k.unwrap()] += out
            .iter()
            .map(|&s| f64::from(s) * f64::from(s))
            .sum::<f64>();
        for (p, &(pb, i)) in probes.iter_mut().zip(&PROBES) {
            if pb == b {
                *p = out[i];
            }
        }
    });
    println!("energies {energy:?}, probes {probes:?}");
    let pinned_energy = [
        83_669.712_179,
        38_150.994_213,
        86_400.905_277,
        6_446.990_545,
    ];
    let pinned_probes = [-0.114_623_28, -0.685_756_15, -0.160_597_82, 0.235_070_66];
    for (k, (&e, &want)) in energy.iter().zip(&pinned_energy).enumerate() {
        assert!(
            (e - want).abs() <= 1e-4 * want,
            "stretch {k}: energy {e}, pinned {want}"
        );
    }
    for (k, (&s, &want)) in probes.iter().zip(&pinned_probes).enumerate() {
        assert!(
            (s - want).abs() <= 1e-5,
            "probe {k} {:?}: sample {s}, pinned {want}",
            PROBES[k]
        );
    }
}
