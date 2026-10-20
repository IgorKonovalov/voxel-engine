//! The direct path under motion: a listener walking past a hum in open air hears it at 440 Hz
//! with the direct delay off, and with it on at the Doppler-shifted `440 (1 + v_r / c)`. In an
//! unobstructed corridor the probes change nothing on the direct path but distance and
//! direction.
//!
//! **The arithmetic.** A listener moving at `v` toward a fixed source hears `f (1 + v_r / c)`,
//! `v_r` the approach speed: `3 x 6 / sqrt(37) = 2.96` m/s at the walk's ends, so 443.80 Hz
//! coming and 436.20 Hz going. The head's interaural delay changes by at most
//! `(0.0875 m / 343 m/s) x 2 x (v / 1 m)`, which moves an ear's frequency by at most 0.67 Hz.

use vxe_audio::*;
use vxe_world::*;

const MPV: f32 = 0.1;
const FS: f64 = RATE as f64;
const HZ: f64 = 440.0;
/// The cave row: absorption 0.3 per metre, transmission 0.5.
const CAVE: MaterialId = 1;
/// The hum at `(0, 1 m, 0)`.
const SOURCE: [f32; 3] = [0.0, 10.0, 0.0];
/// From `x = -6 m` to `6 m` at 3 m/s: 4 s.
const START: f64 = -60.0;
const SPEED: f64 = 30.0;
const BLOCKS: usize = 4 * RATE as usize / BLOCK;
/// The demodulation window: 20 periods of 440 Hz under a Hann window, centred on each block.
const WINDOW: usize = 2_182;
/// Readings start at 100 ms.
const FROM: usize = 4_800;

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

/// One second of a 440 Hz sine of amplitude 0.3.
fn hum() -> Sound {
    Sound::new(
        (0..RATE)
            .map(|n| 0.3 * (std::f64::consts::TAU * HZ * f64::from(n) / FS).sin() as f32)
            .collect::<Vec<_>>(),
    )
}

/// The listener's `x` (voxel units) at frame `n`.
fn walk_x(n: f64) -> f64 {
    START + SPEED * n / FS
}

/// The walk through `world`, HRTF path, facing `+x`, the position sent and probed every 3
/// blocks, reflections and tail off, the direct delay `delayed`: the output and every
/// `SetAcoustics` sent.
fn walk(world: &World, delayed: bool) -> (Vec<f32>, Vec<AcousticTargets>) {
    let mut mixer = Mixer::new(Hrtf::sadie2_d2(), MPV);
    mixer.set_mix(MixSettings {
        direct_delay: delayed,
        ..MixSettings::default()
    });
    let mut acoustics = Acoustics::new(mixer.clock());
    acoustics.set_reflections(false);
    acoustics.set_reverb(false);
    let id = VoiceId(1);
    mixer.apply(Command::Play(
        id,
        hum(),
        PlayParams {
            looping: true,
            ..PlayParams::at(SOURCE)
        },
    ));
    let mut out = vec![0.0; 2 * BLOCKS * BLOCK];
    let mut sent = Vec::new();
    for (b, chunk) in out.chunks_mut(2 * BLOCK).enumerate() {
        if b % 3 == 0 {
            let listener = Listener {
                position: [walk_x((b * BLOCK) as f64) as f32, 0.0, 0.0],
                ..Listener::default()
            };
            mixer.apply(Command::SetListener(listener));
            acoustics.update(
                world,
                world.materials(),
                &listener,
                &[(id, SOURCE)],
                MPV,
                &mut |cmd| {
                    if let Command::SetAcoustics(_, t) = &cmd {
                        sent.push(*t);
                    }
                    mixer.apply(cmd);
                },
            );
        }
        mixer.render(chunk, BLOCK);
    }
    (out, sent)
}

/// Per ear, the frequency between consecutive block centres from `FROM` on: each centre's
/// phase from demodulating at 440 Hz over a Hann window of [`WINDOW`] frames (20 periods, so
/// the demodulation's image at 880 Hz falls in the window's far sidelobes), unwrapped and
/// differenced. Each reading with the frame it is taken at (between the two centres).
fn frequencies(out: &[f32], ear: usize) -> Vec<(usize, f64)> {
    let x: Vec<f64> = out
        .iter()
        .skip(ear)
        .step_by(2)
        .map(|&v| f64::from(v))
        .collect();
    let w = std::f64::consts::TAU * HZ / FS;
    let half = WINDOW / 2;
    let mut phases = Vec::new();
    for b in 0..BLOCKS {
        let c = b * BLOCK + BLOCK / 2;
        if c < half.max(FROM) || c + half > x.len() {
            continue;
        }
        let (mut re, mut im) = (0.0, 0.0);
        for i in 0..WINDOW {
            let n = c - half + i;
            let h = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / WINDOW as f64).cos();
            let (s, co) = (w * n as f64).sin_cos();
            re += h * x[n] * co;
            im -= h * x[n] * s;
        }
        phases.push((c, im.atan2(re)));
    }
    let dt = BLOCK as f64 / FS;
    phases
        .windows(2)
        .map(|p| {
            let mut d = p[1].1 - p[0].1;
            while d > std::f64::consts::PI {
                d -= std::f64::consts::TAU;
            }
            while d < -std::f64::consts::PI {
                d += std::f64::consts::TAU;
            }
            ((p[0].0 + p[1].0) / 2, HZ + d / (std::f64::consts::TAU * dt))
        })
        .collect()
}

/// `440 (1 + v_r / c)` at frame `n`, `v_r` the listener's approach speed to the source.
fn doppler(n: usize) -> f64 {
    let x = walk_x(n as f64);
    let (dx, dy) = (f64::from(SOURCE[0]) - x, f64::from(SOURCE[1]));
    let v_r = SPEED * dx / (dx * dx + dy * dy).sqrt() * f64::from(MPV);
    HZ * (1.0 + v_r / f64::from(SPEED_OF_SOUND))
}

#[test]
fn walking_past_a_hum_is_heard_at_its_pitch_or_with_the_doppler_of_the_walk() {
    let air = world([-2, -1, -1], [2, 1, 1], |_| false);
    let (off, _) = walk(&air, false);
    let (on, _) = walk(&air, true);
    let (mut off_440, mut on_doppler, mut off_doppler_ends) = (0.0f64, 0.0f64, f64::INFINITY);
    for ear in 0..2 {
        let f_off = frequencies(&off, ear);
        let f_on = frequencies(&on, ear);
        for &(_, f) in &f_off {
            off_440 = off_440.max((f - HZ).abs());
        }
        for &(n, f) in &f_on {
            on_doppler = on_doppler.max((f - doppler(n)).abs());
        }
        let ends = [f_on[0], f_on[f_on.len() - 1]];
        let off_ends = [f_off[0], f_off[f_off.len() - 1]];
        for ((n, f), (_, g)) in ends.iter().zip(&off_ends) {
            off_doppler_ends = off_doppler_ends.min((g - doppler(*n)).abs());
            println!(
                "ear {ear}, {:.3} s: on {f:.2} Hz, off {g:.2} Hz, Doppler {:.2} Hz",
                *n as f64 / FS,
                doppler(*n)
            );
        }
    }
    println!(
        "walk from -6 m to 6 m at 3 m/s past a hum 1 m aside: off, largest distance from 440 Hz \
         {off_440:.3} Hz; on, largest distance from the Doppler {on_doppler:.3} Hz; off at the \
         ends, smallest distance from the Doppler {off_doppler_ends:.3} Hz"
    );
    assert!(off_440 <= 2.2, "off: {off_440} Hz from 440 Hz");
    assert!(on_doppler <= 1.0, "on: {on_doppler} Hz from the Doppler");
    // The bite: without the delay the ends are at least 3 Hz from the Doppler (3.80 Hz at
    // the walk's ends), three times the bound.
    assert!(
        off_doppler_ends >= 3.0,
        "off is only {off_doppler_ends} Hz from the Doppler at the ends"
    );
}

/// The corridor of the walking test (rock at `y >= 5 m`, `y <= -5 m`, `z <= -2 m`): nothing
/// crosses the direct or side rays, so every probe of the walk carries gain 1 and the open
/// cutoff, and the direct path changes only in distance, direction and delay.
#[test]
fn an_unobstructed_walk_leaves_the_direct_path_open() {
    let corridor = world([-2, -1, -1], [3, 1, 1], |v| {
        v[1] >= 50 || v[1] <= -50 || v[2] <= -20
    });
    for delayed in [false, true] {
        let (_, sent) = walk(&corridor, delayed);
        println!(
            "corridor walk, direct delay {delayed}: {} probes, gains {:?}, cutoffs {:?}",
            sent.len(),
            sent.iter()
                .map(|t| t.gain)
                .fold((f32::INFINITY, f32::NEG_INFINITY), |m, g| (
                    m.0.min(g),
                    m.1.max(g)
                )),
            sent.iter()
                .map(|t| t.cutoff_hz)
                .fold((f32::INFINITY, f32::NEG_INFINITY), |m, c| (
                    m.0.min(c),
                    m.1.max(c)
                )),
        );
        assert!(sent.len() > 40);
        for t in &sent {
            assert_eq!(t.gain, 1.0);
            assert_eq!(t.cutoff_hz, OPEN_CUTOFF_HZ);
        }
    }
}
