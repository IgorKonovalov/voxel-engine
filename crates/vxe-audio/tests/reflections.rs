//! Early reflections: the fixed direction set, the tap a wall gives, and what the mixer renders
//! from it.

use vxe_audio::*;
use vxe_world::*;

const MPV: f32 = 0.1;
/// 100 ms.
const FRAMES: usize = 4_800;
/// The wall material's row: absorption 0.3 per metre, transmission 0.5.
const W: MaterialId = 1;
/// The amplitude the wall's face keeps: `(1 - 0.3) ^ 0.2`, one 10 cm skin in and out.
const KEEP: f32 = 0.931_149_5;

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

/// Air over chunks `[0, 4)³` (voxels `[0, 256)³`), `W` wherever `solid` holds.
fn world(solid: impl Fn([i32; 3]) -> bool + Sync) -> World {
    let mut t = MaterialTable::new();
    let mut w = Material::named("wall");
    w.acoustic = [0.3, 0.5, 0.0];
    assert_eq!(t.push(w), W);
    World::generate(
        ChunkExtent::new([0, 0, 0], [4, 4, 4]),
        &FnSource(move |v| if solid(v) { W } else { AIR }),
        &t,
    )
}

fn centre(v: [i32; 3]) -> [f32; 3] {
    v.map(|c| c as f32 + 0.5)
}

/// Listener at voxel `(0, 128, 128)`, 4.95 m from the wall face at `x = 50`.
const LISTENER: [i32; 3] = [0, 128, 128];
/// 2 m along `+y` from the listener, also 4.95 m from the face.
const SOURCE: [i32; 3] = [0, 148, 128];

/// The left channel (the source is on the listener's left) of 100 ms of a one-sample impulse
/// of amplitude 1 at `SOURCE`, with the acoustics probed once through `world` before the first
/// block.
fn impulse_response(world: &World) -> (Vec<f32>, AcousticTargets) {
    let mut mixer = Mixer::new(Hrtf::sadie2_d2(), MPV);
    let listener = Listener {
        position: centre(LISTENER),
        ..Listener::default()
    };
    mixer.apply(Command::SetListener(listener));
    let id = VoiceId(3);
    let mut impulse = vec![0.0; FRAMES];
    impulse[0] = 1.0;
    mixer.apply(Command::Play(
        id,
        Sound::new(impulse),
        PlayParams::at(centre(SOURCE)),
    ));
    let mut acoustics = Acoustics::new(mixer.clock());
    let mut targets = None;
    acoustics.update(
        world,
        world.materials(),
        &listener,
        &[(id, centre(SOURCE))],
        MPV,
        &mut |cmd| {
            if let Command::SetAcoustics(_, t) = &cmd {
                targets = Some(*t);
            }
            mixer.apply(cmd);
        },
    );
    let mut out = vec![0.0; 2 * FRAMES];
    mixer.render(&mut out, FRAMES);
    (
        out.chunks_exact(2).map(|f| f[0]).collect(),
        targets.expect("probed"),
    )
}

/// Index and value of the largest magnitude in `x[from..]`.
fn peak(x: &[f32], from: usize) -> (usize, f32) {
    x.iter()
        .enumerate()
        .skip(from)
        .fold((from, 0.0f32), |best, (i, &s)| {
            if s.abs() > best.1.abs() { (i, s) } else { best }
        })
}

#[test]
fn the_directions_are_a_fibonacci_sphere_with_poles_on_x() {
    let d = reflection_directions();
    let round = |v: [f32; 3]| v.map(|c| (c * 1e4).round() / 1e4);
    assert_eq!(d[0], [1.0, 0.0, 0.0]);
    assert_eq!(round(d[1]), [0.8667, -0.3679, 0.3370]);
    assert_eq!(round(d[2]), [0.7333, 0.0594, -0.6773]);
    assert_eq!(d[15][0], -1.0);
    assert_eq!(d[15][1].abs() + d[15][2].abs(), 0.0);
    for v in d {
        let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        assert!((len - 1.0).abs() < 1e-6, "{v:?} has length {len}");
    }
}

#[test]
fn the_x_ray_hits_the_face_and_gives_the_1160_sample_tap() {
    let walled = world(|v| (50..60).contains(&v[0]));
    let hits = reflections(
        &walled,
        walled.materials(),
        centre(LISTENER),
        &reflection_directions(),
        MPV,
    );
    // The +x ray enters the wall exactly at its face.
    let hit = hits[0].expect("the +x ray hits the wall");
    assert_eq!(hit.point, [50.0, 128.5, 128.5]);
    assert!((hit.keep - KEEP).abs() < 1e-6, "keep {}", hit.keep);
    assert!((reflection_keep(0.3) - 0.7f32.powf(0.2)).abs() < 1e-7);
    let taps = taps(&hits, centre(LISTENER), centre(SOURCE), MPV, 1.0);
    // A slot sounds exactly when its ray hits; the -x ray, pointing away from the wall, hits
    // nothing.
    for (t, h) in taps.iter().zip(&hits) {
        assert_eq!(t.gain != 0.0, h.is_some(), "{t:?} for {h:?}");
    }
    assert_eq!(hits[15], None);
    let path = 4.95 + (4.95f32 * 4.95 + 4.0).sqrt();
    assert_eq!(taps[0].delay_frames, 1160);
    assert!((taps[0].gain - KEEP / path).abs() < 1e-6);
    let delays: Vec<u32> = taps.iter().map(|t| t.delay_frames).collect();
    println!("tap delays by slot {delays:?}");
}

#[test]
fn a_wall_4_95_m_away_echoes_1160_samples_after_the_direct_peak() {
    let (left, targets) = impulse_response(&world(|v| (50..60).contains(&v[0])));
    println!("taps by slot: {:?}", targets.taps);
    let (direct_at, direct) = peak(&left, 0);
    // The HRIR spans 128 taps; past them the direct sound is silent.
    let (echo_at, echo) = peak(&left, direct_at + 128);
    let delay = echo_at - direct_at;
    let ratio = echo.abs() / direct.abs();
    println!(
        "direct peak {direct} at {direct_at}, echo {echo} at {echo_at}: delay {delay}, ratio {ratio}"
    );
    assert!(delay.abs_diff(1160) <= 2, "echo delay {delay} samples");
    let expected = KEEP * (1.0 / 10.2888) / (1.0 / 2.0);
    assert!(
        (ratio - expected).abs() <= 0.1 * expected,
        "echo over direct {ratio} vs {expected}"
    );
}

#[test]
fn without_the_wall_nothing_echoes() {
    let (left, targets) = impulse_response(&world(|_| false));
    assert!(
        targets.taps.iter().all(|t| t.gain == 0.0),
        "{:?}",
        targets.taps
    );
    let (direct_at, direct) = peak(&left, 0);
    let (_, rest) = peak(&left, direct_at + 128);
    println!("direct {direct} at {direct_at}, largest after the HRIR {rest}");
    assert!(rest.abs() <= 0.01 * direct.abs());
}

/// Phase 3's occlusion scenes: listener at voxel `(50, 128, 128)`, source at `(200, 128, 128)`,
/// a wall of `W` over `x in [x0, x1)`.
const OCC_LISTENER: [i32; 3] = [50, 128, 128];
const OCC_SOURCE: [i32; 3] = [200, 128, 128];

/// One probe of the source at `OCC_SOURCE` from `OCC_LISTENER` through `world`, reflections
/// on: the targets it sends.
fn occluded_targets(world: &World) -> AcousticTargets {
    let mixer = Mixer::new(Hrtf::sadie2_d2(), MPV);
    let mut acoustics = Acoustics::new(mixer.clock());
    assert!(acoustics.reflections_on());
    let listener = Listener {
        position: centre(OCC_LISTENER),
        ..Listener::default()
    };
    let mut targets = None;
    acoustics.update(
        world,
        world.materials(),
        &listener,
        &[(VoiceId(7), centre(OCC_SOURCE))],
        MPV,
        &mut |cmd| {
            if let Command::SetAcoustics(_, t) = cmd {
                targets = Some(t);
            }
        },
    );
    targets.expect("probed")
}

/// RMS over both channels of the last 0.1 s of 0.5 s of a tone of `freq` and amplitude 0.5 at
/// `OCC_SOURCE`, heard at `OCC_LISTENER`, probed once before the first block with reflections
/// `on` or off.
fn occluded_render(world: &World, freq: f32, on: bool) -> f32 {
    const LEN: usize = 24_000;
    const TAIL: usize = 4_800;
    let mut mixer = Mixer::new(Hrtf::sadie2_d2(), MPV);
    let listener = Listener {
        position: centre(OCC_LISTENER),
        ..Listener::default()
    };
    mixer.apply(Command::SetListener(listener));
    let id = VoiceId(7);
    let tone: Vec<f32> = (0..LEN)
        .map(|n| 0.5 * (std::f32::consts::TAU * freq * n as f32 / RATE as f32).sin())
        .collect();
    mixer.apply(Command::Play(
        id,
        Sound::new(tone),
        PlayParams::at(centre(OCC_SOURCE)),
    ));
    let mut acoustics = Acoustics::new(mixer.clock());
    acoustics.set_reflections(on);
    acoustics.update(
        world,
        world.materials(),
        &listener,
        &[(id, centre(OCC_SOURCE))],
        MPV,
        &mut |cmd| mixer.apply(cmd),
    );
    let mut out = vec![0.0; 2 * LEN];
    mixer.render(&mut out, LEN);
    let tail = &out[out.len() - 2 * TAIL..];
    let sum: f64 = tail.iter().map(|&s| f64::from(s) * f64::from(s)).sum();
    (sum / tail.len() as f64).sqrt() as f32
}

fn wall(x0: i32, x1: i32) -> impl Fn([i32; 3]) -> bool + Sync + Copy {
    move |v| (x0..x1).contains(&v[0])
}

/// Every tap carries the source's occlusion multiplier. In Phase 3's two wall scenes the sent
/// multiplier is Phase 3's figure, and each sent tap is that multiplier times the wall's keep
/// factor `(1 - A) ^ 0.2` over the reflected path `|L - H| + |H - S|` (at least 1 m), delayed
/// by the path's excess over the direct one, all computed here in `f64` from each hit's point,
/// in the slot of the hit's ray; every other slot is silent.
#[test]
fn every_tap_carries_the_source_occlusion_multiplier() {
    /// The wall row's absorption per metre.
    const ABSORPTION: f64 = 0.3;
    let dist = |a: [f32; 3], b: [f32; 3]| -> f64 {
        let d = [0, 1, 2].map(|k| f64::from(a[k]) - f64::from(b[k]));
        (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() * f64::from(MPV)
    };
    let (l, s) = (centre(OCC_LISTENER), centre(OCC_SOURCE));
    for ((x0, x1), multiplier) in [((100, 150), 0.08404), ((100, 105), 0.4183)] {
        let walled = world(wall(x0, x1));
        let sent = occluded_targets(&walled);
        assert!(
            (f64::from(sent.gain) - multiplier).abs() <= 0.01 * multiplier,
            "wall [{x0}, {x1}): multiplier {} against {multiplier}",
            sent.gain
        );
        let hits = reflections(
            &walled,
            walled.materials(),
            l,
            &reflection_directions(),
            MPV,
        );
        assert_eq!(
            sent.send, sent.gain,
            "the tail's send is the occlusion multiplier"
        );
        let direct = dist(l, s);
        let keep = (1.0 - ABSORPTION).powf(0.2);
        let expected: Vec<Option<(u32, f64)>> = hits
            .iter()
            .map(|h| {
                let h = h.as_ref()?;
                let path = dist(l, h.point) + dist(h.point, s);
                let delay = ((path - direct) / 343.0 * 48_000.0).round();
                (delay <= MAX_DELAY_FRAMES as f64).then(|| {
                    let gain = f64::from(sent.gain) * keep / path.max(1.0);
                    (delay as u32, gain)
                })
            })
            .collect();
        assert!(expected.iter().any(Option::is_some), "the wall gives taps");
        let mut worst = 0.0f64;
        for (t, e) in sent.taps.iter().zip(&expected) {
            let Some((delay, gain)) = *e else {
                assert_eq!(*t, Tap::default(), "wall [{x0}, {x1}): a silent slot");
                continue;
            };
            assert_eq!(t.delay_frames, delay, "wall [{x0}, {x1}): tap {t:?}");
            let rel = (f64::from(t.gain) - gain).abs() / gain;
            assert!(
                rel <= 1e-6,
                "wall [{x0}, {x1}): tap {t:?} against {gain}, relative {rel}"
            );
            worst = worst.max(rel);
        }
        println!(
            "wall [{x0}, {x1}): multiplier {}, {} sounding slots, gains {:?}, worst relative \
             {worst:e}",
            sent.gain,
            expected.iter().flatten().count(),
            sent.taps.iter().map(|t| t.gain).collect::<Vec<_>>()
        );
    }
}

/// Phase 3's two wall scenes with reflections on: the rendered ratios to the open scene, with
/// each face keeping `(1 - A) ^ 0.2`. These are measured, not derived: they depend on where the
/// sixteen rays hit the walls.
#[test]
fn the_occlusion_scenes_with_reflections_on_render_at_0_1272_and_0_7773() {
    for ((x0, x1), freq, expected) in [((100, 150), 100.0, 0.1272), ((100, 105), 2000.0, 0.7773)] {
        let open = occluded_render(&world(|_| false), freq, true);
        let behind = occluded_render(&world(wall(x0, x1)), freq, true);
        let ratio = behind / open;
        println!("wall [{x0}, {x1}), {freq} Hz, reflections on: {ratio} (expected {expected})");
        assert!(
            (ratio - expected).abs() <= 0.02 * expected,
            "wall [{x0}, {x1}): {ratio} against {expected}"
        );
    }
}

#[test]
fn with_reflections_off_no_taps_are_sent() {
    let walled = world(wall(100, 150));
    let mixer = Mixer::new(Hrtf::sadie2_d2(), MPV);
    let mut acoustics = Acoustics::new(mixer.clock());
    acoustics.set_reflections(false);
    assert!(!acoustics.reflections_on());
    let mut sent = Vec::new();
    acoustics.update(
        &walled,
        walled.materials(),
        &Listener {
            position: centre(OCC_LISTENER),
            ..Listener::default()
        },
        &[(VoiceId(7), centre(OCC_SOURCE))],
        MPV,
        &mut |cmd| sent.push(cmd),
    );
    assert_eq!(sent.len(), 1);
    let Command::SetAcoustics(_, t) = &sent[0] else {
        panic!("expected SetAcoustics, got {:?}", sent[0]);
    };
    assert!(t.taps.iter().all(|t| t.gain == 0.0), "{:?}", t.taps);
    assert_eq!(acoustics.active_taps(), 0);
}
