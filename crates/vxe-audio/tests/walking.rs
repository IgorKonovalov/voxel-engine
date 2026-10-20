//! A listener walking past a hum with the early reflections on: the echoes glide with the walk
//! instead of stepping at every re-probe, so neither the output's level nor its 440 Hz phase
//! jumps from one 256-frame block to the next. Also the echoes fading out when the reflections
//! are switched off.

use vxe_audio::*;
use vxe_world::*;

const MPV: f32 = 0.1;
/// The cave row: absorption 0.3 per metre, transmission 0.5.
const CAVE: MaterialId = 1;
/// 440 Hz, the frequency the phase is read at.
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

/// The phase difference of two complex values, wrapped into `[-pi, pi]`.
fn phase_step(a: (f64, f64), b: (f64, f64)) -> f64 {
    let d = b.1.atan2(b.0) - a.1.atan2(a.0);
    (d + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
}

/// `ch` demodulated at 440 Hz over a sliding one-cycle window (109 frames): the unwrapped
/// phase at every frame from the 109th on, so entry `k` is the window ending at frame
/// `k + 108`.
fn phase_440(ch: &[f64]) -> Vec<f64> {
    let w = std::f64::consts::TAU * HZ / f64::from(RATE);
    let cycle = 109usize;
    let (mut re, mut im) = (0.0f64, 0.0f64);
    let mut z = Vec::with_capacity(ch.len());
    for n in 0..ch.len() {
        let (s, c) = (w * n as f64).sin_cos();
        re += ch[n] * c;
        im -= ch[n] * s;
        if n >= cycle {
            let (s, c) = (w * (n - cycle) as f64).sin_cos();
            re -= ch[n - cycle] * c;
            im += ch[n - cycle] * s;
            z.push((re, im));
        }
    }
    let mut acc = 0.0;
    let mut phase = Vec::with_capacity(z.len());
    for k in 0..z.len() {
        if k > 0 {
            acc += phase_step(z[k - 1], z[k]);
        }
        phase.push(acc);
    }
    phase
}

/// The largest change of either channel's 440 Hz phase from one block end to the next.
fn worst_phase_step(out: &[f32]) -> f64 {
    (0..2)
        .flat_map(|c| {
            let phase = phase_440(&channel(out, c));
            // The window ending at the last frame of each whole block.
            let ends: Vec<f64> = (1..)
                .map(|k| k * BLOCK - 110)
                .take_while(|&i| i < phase.len())
                .map(|i| phase[i])
                .collect();
            ends.windows(2)
                .map(|w| (w[1] - w[0]).abs())
                .collect::<Vec<_>>()
        })
        .fold(0.0, f64::max)
}

/// The hum at `(8 m, 1 m, 0)` heard by a listener facing `+x` who walks from `x = 0` to
/// `x = 6 m` at 3 m/s through the corridor, HRTF path, reflections on. Every 3 blocks (16 ms
/// of audio) the listener is moved and `Acoustics::update` runs, as a 60 Hz game would.
fn walk(world: &World) -> Vec<f32> {
    const SECONDS: f32 = 2.0;
    const SPEED_VOXELS: f32 = 30.0;
    let source = [80.0, 10.0, 0.0];
    let mut mixer = Mixer::new(Hrtf::sadie2_d2(), MPV);
    let mut acoustics = Acoustics::new(mixer.clock());
    assert!(acoustics.reflections_on());
    let id = VoiceId(1);
    mixer.apply(Command::Play(
        id,
        hum(),
        PlayParams {
            looping: true,
            ..PlayParams::at(source)
        },
    ));
    let blocks = (SECONDS * RATE as f32) as usize / BLOCK;
    let mut out = vec![0.0; 2 * blocks * BLOCK];
    for (b, chunk) in out.chunks_mut(2 * BLOCK).enumerate() {
        if b % 3 == 0 {
            let t = (b * BLOCK) as f32 / RATE as f32;
            let listener = Listener {
                position: [SPEED_VOXELS * t, 0.0, 0.0],
                ..Listener::default()
            };
            mixer.apply(Command::SetListener(listener));
            acoustics.update(
                world,
                world.materials(),
                &listener,
                &[(id, source)],
                MPV,
                &mut |cmd| mixer.apply(cmd),
            );
        }
        mixer.render(chunk, BLOCK);
    }
    out
}

#[test]
fn walking_past_a_hum_with_echoes_steps_neither_level_nor_phase() {
    let corridor = corridor();
    let out = walk(&corridor);
    let measured = &out[2 * SETTLE..];
    let rms = worst_rms_step_db(measured);
    let phase = worst_phase_step(measured);
    println!(
        "walking at 3 m/s past a 440 Hz hum, echoes on: largest block-to-block RMS step \
         {rms:.4} dB, largest per-block 440 Hz phase step {phase:.4} rad"
    );
    assert!(rms <= 0.5, "RMS step {rms} dB");
    assert!(phase <= 0.1, "phase step {phase} rad");
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
