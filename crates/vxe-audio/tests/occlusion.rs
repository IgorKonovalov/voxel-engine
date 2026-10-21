//! Occlusion through the voxel world: the seven-ray gain multiplier, the direct ray's cutoff,
//! what the mixer renders with them, and the probe cadence.

use std::f32::consts::PI;

use vxe_audio::*;
use vxe_world::*;

const MPV: f32 = 0.1;
const FRAMES: usize = 24_000;
const TAIL: usize = 4_800;
/// The wall material's row: absorption 0.3 per metre, transmission 0.5.
const W: MaterialId = 1;

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

fn table() -> MaterialTable {
    let mut t = MaterialTable::new();
    let mut w = Material::named("wall");
    w.acoustic = [0.3, 0.5, 0.0];
    assert_eq!(t.push(w), W);
    t
}

/// Air over chunks `[0, 4)³` (voxels `[0, 256)³`), `W` wherever `solid` holds.
fn world(solid: impl Fn([i32; 3]) -> bool + Sync) -> World {
    World::generate(
        ChunkExtent::new([0, 0, 0], [4, 4, 4]),
        &FnSource(move |v| if solid(v) { W } else { AIR }),
        &table(),
    )
}

fn centre(v: [i32; 3]) -> [f32; 3] {
    v.map(|c| c as f32 + 0.5)
}

const LISTENER: [i32; 3] = [50, 128, 128];
const SOURCE: [i32; 3] = [200, 128, 128];

fn wall(x0: i32, x1: i32) -> impl Fn([i32; 3]) -> bool + Sync {
    move |v| (x0..x1).contains(&v[0])
}

fn occ(world: &World) -> Occlusion {
    occlusion(
        world,
        world.materials(),
        centre(LISTENER),
        centre(SOURCE),
        MPV,
    )
}

fn tone(freq: f32, amplitude: f32, len: usize) -> Sound {
    Sound::new(
        (0..len)
            .map(|n| amplitude * (2.0 * PI * freq * n as f32 / RATE as f32).sin())
            .collect::<Vec<_>>(),
    )
}

/// RMS over both channels of the last `TAIL` frames.
fn tail_rms(out: &[f32]) -> f32 {
    let tail = &out[out.len() - 2 * TAIL..];
    let sum: f64 = tail.iter().map(|&s| f64::from(s) * f64::from(s)).sum();
    (sum / tail.len() as f64).sqrt() as f32
}

/// 0.5 s of a tone of `freq` and amplitude 0.5 at `SOURCE`, heard at `LISTENER` facing +x,
/// with the acoustics probed once through `world` before the first block and the early
/// reflections off, so the render is the direct path alone.
fn render(world: &World, freq: f32) -> f32 {
    let mut mixer = Mixer::new(Hrtf::sadie2_d2(), MPV);
    mixer.set_mix(MixSettings::flat());
    let listener = Listener {
        position: centre(LISTENER),
        ..Listener::default()
    };
    mixer.apply(Command::SetListener(listener));
    let id = VoiceId(7);
    mixer.apply(Command::Play(
        id,
        tone(freq, 0.5, FRAMES),
        PlayParams::at(centre(SOURCE)),
    ));
    let mut acoustics = Acoustics::new(mixer.clock());
    acoustics.set_reflections(false);
    acoustics.update(
        world,
        world.materials(),
        &listener,
        &[(id, centre(SOURCE))],
        MPV,
        &mut |cmd| mixer.apply(cmd),
    );
    assert_eq!(acoustics.last_probed(), 1);
    assert_eq!(acoustics.active_taps(), 0);
    let mut out = vec![0.0; 2 * FRAMES];
    mixer.render(&mut out, FRAMES);
    tail_rms(&out)
}

fn assert_within(actual: f32, expected: f32, rel: f32, what: &str) {
    println!("{what}: {actual} (expected {expected} within {rel})");
    assert!(
        (actual - expected).abs() <= rel * expected,
        "{what}: {actual} vs {expected} (tolerance {rel})"
    );
}

#[test]
fn without_a_wall_nothing_is_occluded() {
    let o = occ(&world(|_| false));
    assert_eq!(o.gain, 1.0);
    assert_eq!(o.thickness_m, 0.0);
    assert_eq!(o.cutoff_hz, 20_000.0);
}

#[test]
fn five_metres_of_wall_render_at_0_0814_of_the_open_level() {
    let walled = world(wall(100, 150));
    let o = occ(&walled);
    // Every one of the seven rays crosses 5 m: 0.5 * 0.7^5. The four side rays off the x axis
    // cross 5.003 m; the 1 % covers it.
    assert_within(o.gain, 0.5 * 0.7f32.powi(5), 0.01, "gain multiplier");
    assert_eq!(o.thickness_m, 5.0);
    assert_within(o.cutoff_hz, 20_000.0 / 51.0, 1e-4, "cutoff");
    assert_within(o.cutoff_hz, 392.16, 1e-4, "cutoff, the plan's figure");
    let open = render(&world(|_| false), 100.0);
    let behind = render(&walled, 100.0);
    assert_within(behind / open, 0.0814, 0.02, "100 Hz behind 5 m over open");
}

#[test]
fn half_a_metre_of_wall_renders_a_2_khz_tone_at_0_358() {
    let walled = world(wall(100, 105));
    let o = occ(&walled);
    assert_within(o.gain, 0.4183, 0.01, "gain multiplier");
    assert_within(o.thickness_m, 0.5, 1e-6, "thickness");
    assert_within(o.cutoff_hz, 3333.33, 1e-4, "cutoff");
    let open = render(&world(|_| false), 2000.0);
    let behind = render(&walled, 2000.0);
    assert_within(behind / open, 0.358, 0.02, "2 kHz behind 0.5 m over open");
}

#[test]
fn a_pillar_covering_three_rays_gives_0_6074() {
    // A 3 x 3 pillar x in [100, 150), y and z in [127, 130) holds the axis (y = z = 128.5), so
    // it covers the direct ray and the two collinear +-x side rays. The +-y and +-z side rays
    // aim 5 voxels off the axis at the source, so at x = 100 they sit 1.65 voxels off it, at y
    // or z 130.15 or 126.85, outside the pillar.
    let pillar = world(|v| {
        (100..150).contains(&v[0]) && (127..130).contains(&v[1]) && (127..130).contains(&v[2])
    });
    let o = occ(&pillar);
    assert_within(o.gain, (3.0 * 0.08404 + 4.0) / 7.0, 0.01, "gain multiplier");
}

#[test]
fn probes_follow_movement_and_the_500_ms_cadence() {
    let world = world(wall(100, 150));
    let mut mixer = Mixer::new(Hrtf::sadie2_d2(), MPV);
    mixer.set_mix(MixSettings::flat());
    let mut acoustics = Acoustics::new(mixer.clock());
    let mut listener = Listener {
        position: centre(LISTENER),
        ..Listener::default()
    };
    let id = VoiceId(1);
    let mut source = centre(SOURCE);
    let mut sent = Vec::new();
    let mut update = |acoustics: &mut Acoustics, listener: &Listener, source: [f32; 3]| {
        sent.clear();
        acoustics.update(
            &world,
            world.materials(),
            listener,
            &[(id, source)],
            MPV,
            &mut |cmd| sent.push(cmd),
        );
        assert_eq!(sent.len(), acoustics.last_probed());
        acoustics.last_probed()
    };
    let mut out = vec![0.0; 2 * BLOCK];
    let mut blocks = |mixer: &mut Mixer, n: usize| {
        for _ in 0..n {
            mixer.render(&mut out, BLOCK);
        }
    };
    // First sight: probed.
    assert_eq!(update(&mut acoustics, &listener, source), 1);
    // Nothing moved: not probed again.
    assert_eq!(update(&mut acoustics, &listener, source), 0);
    // 0.1 m is not more than 0.1 m; 0.11 m is.
    source[1] += 1.0;
    assert_eq!(update(&mut acoustics, &listener, source), 0);
    source[1] += 0.1;
    assert_eq!(update(&mut acoustics, &listener, source), 1);
    listener.position[2] += 1.1;
    assert_eq!(update(&mut acoustics, &listener, source), 1);
    // 500 ms of audio is 93.75 blocks: probed again at the 94th block, not the 93rd.
    blocks(&mut mixer, 93);
    assert_eq!(update(&mut acoustics, &listener, source), 0);
    blocks(&mut mixer, 1);
    assert_eq!(update(&mut acoustics, &listener, source), 1);
    assert_eq!(mixer.clock().blocks(), 94);
}
