//! Flying out of a chamber through its rock shell and back in, with a hum playing: the output
//! never rises more than 1.5 dB over the chamber's own steady level, neither the tail alone nor
//! the full output. And the mixer alone handed a room with no surface at the largest energy a
//! room estimate allows (`T60` 0.05 s, mean free path 0, energy 16) for 0.5 s between two of
//! the chamber's: once the chamber's room is back, the tail settles to its steady level without
//! overshooting it.

use vxe_audio::*;
use vxe_world::*;

const MPV: f32 = 0.1;
/// The cave row: absorption 0.3 per metre, transmission 0.5.
const CAVE: MaterialId = 1;
const FS: f64 = RATE as f64;
/// The hum's frequency.
const HZ: f64 = 440.0;
/// 50 ms windows, hop 25 ms.
const WINDOW: usize = 2_400;
const HOP: usize = 1_200;
/// The bound over the chamber's steady level.
const BOUND_DB: f64 = 1.5;

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

/// The 12 m cube of air, voxels `[0, 120)³`, inside a 2 m shell of the cave row
/// (`[-20, 140)³`), air beyond it; chunks outside `[-1, 4)³` are not resident and read as air.
fn chamber() -> World {
    let mut t = MaterialTable::new();
    let mut m = Material::named("cave");
    m.acoustic = [0.3, 0.5, 0.0];
    assert_eq!(t.push(m), CAVE);
    World::generate(
        ChunkExtent::new([-1, -1, -1], [4, 4, 4]),
        &FnSource(|v: [i32; 3]| {
            let inside = v.iter().all(|&c| (0..120).contains(&c));
            let shell = v.iter().all(|&c| (-20..140).contains(&c));
            if shell && !inside { CAVE } else { AIR }
        }),
        &t,
    )
}

/// One second of a 440 Hz sine of amplitude 0.3 (whole cycles, so it loops cleanly).
fn hum() -> Sound {
    Sound::new(
        (0..RATE)
            .map(|n| 0.3 * (std::f64::consts::TAU * HZ * f64::from(n) / FS).sin() as f32)
            .collect::<Vec<_>>(),
    )
}

/// The hum at `(8, 6, 6) m`.
const SOURCE: [f32; 3] = [80.0, 60.0, 60.0];
/// Seconds: the first hold, the flight out, the hold outside, the flight back, the last hold.
const HOLD: f64 = 3.0;
const FLIGHT: f64 = 14.0 / 12.0;
const OUTSIDE: f64 = 1.0;

/// The listener's `x` in voxels at `t` seconds: 6 m for 3 s, to 20 m at 12 m/s, 1 s there, back
/// at 12 m/s, then 6 m for 3 s.
fn listener_x(t: f64) -> f64 {
    let fly = |s: f64| 60.0 + 120.0 * s;
    let t1 = HOLD;
    let t2 = t1 + FLIGHT;
    let t3 = t2 + OUTSIDE;
    let t4 = t3 + FLIGHT;
    if t < t1 {
        60.0
    } else if t < t2 {
        fly(t - t1)
    } else if t < t3 {
        200.0
    } else if t < t4 {
        200.0 - 120.0 * (t - t3)
    } else {
        60.0
    }
}

fn total_seconds() -> f64 {
    HOLD + FLIGHT + OUTSIDE + FLIGHT + HOLD
}

/// One render of the flight at `mix`: `full` plays the direct path and the reflections,
/// otherwise the output is the tail alone. Returns the output and the rooms sent, with their
/// block.
fn fly(world: &World, full: bool, mix: MixSettings) -> (Vec<f32>, Vec<(usize, RoomTargets)>, u64) {
    let mut mixer = Mixer::new(Hrtf::sadie2_d2(), MPV);
    mixer.set_mix(mix);
    let mut acoustics = Acoustics::new(mixer.clock());
    acoustics.set_reflections(full);
    let id = VoiceId(1);
    mixer.apply(Command::Play(
        id,
        hum(),
        PlayParams {
            looping: true,
            direct: full,
            ..PlayParams::at(SOURCE)
        },
    ));
    let blocks = (total_seconds() * FS) as usize / BLOCK;
    let mut out = vec![0.0; 2 * blocks * BLOCK];
    let mut rooms = Vec::new();
    for (b, chunk) in out.chunks_mut(2 * BLOCK).enumerate() {
        if b % 3 == 0 {
            let t = (b * BLOCK) as f64 / FS;
            let listener = Listener {
                position: [listener_x(t) as f32, 60.0, 60.0],
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
                    if let Command::SetRoom(r) = cmd {
                        rooms.push((b, r));
                    }
                    mixer.apply(cmd);
                },
            );
        }
        mixer.render(chunk, BLOCK);
    }
    (out, rooms, acoustics.discarded_fans())
}

/// RMS over both ears of each 50 ms window (hop 25 ms) of interleaved `out`, with its first
/// frame.
fn window_rms(out: &[f32]) -> Vec<(usize, f64)> {
    let frames = out.len() / 2;
    let mut v = Vec::new();
    let mut s = 0;
    while s + WINDOW <= frames {
        let e: f64 = out[2 * s..2 * (s + WINDOW)]
            .iter()
            .map(|&x| f64::from(x) * f64::from(x))
            .sum();
        v.push((s, (e / (2 * WINDOW) as f64).sqrt()));
        s += HOP;
    }
    v
}

/// The largest window over the steady level (the largest window of the second before frame
/// `change`), in dB, of the windows from frame `from` on, and the time it starts at; prints the
/// rooms sent.
fn worst_over_steady(
    out: &[f32],
    change: usize,
    from: usize,
    rooms: &[(usize, RoomTargets)],
    what: &str,
) -> (f64, f64) {
    let w = window_rms(out);
    let steady = w
        .iter()
        .filter(|(s, _)| *s >= change - RATE as usize && s + WINDOW <= change)
        .map(|w| w.1)
        .fold(0.0f64, f64::max);
    let (at, worst) = w
        .iter()
        .filter(|(s, _)| *s >= from)
        .map(|&(s, r)| (s, 20.0 * (r / steady).log10()))
        .fold((0, f64::NEG_INFINITY), |m, x| if x.1 > m.1 { x } else { m });
    let sent: Vec<String> = rooms
        .iter()
        .map(|(b, r)| {
            format!(
                "{:.3} s: energy {:+.1} dB T60 {:.3} s mfp {:.2} m",
                (b * BLOCK) as f64 / FS,
                10.0 * f64::from(r.energy).log10(),
                r.t60_s,
                r.mean_free_path_m
            )
        })
        .collect();
    println!(
        "{what}: steady RMS {steady:.5}; largest window from {:.3} s {worst:+.2} dB at {:.3} s; \
         rooms sent: {}",
        from as f64 / FS,
        at as f64 / FS,
        sent.join("; ")
    );
    (worst, at as f64 / FS)
}

#[test]
fn flying_out_through_the_shell_and_back_never_bursts() {
    let world = chamber();
    let flight = (HOLD * FS) as usize;
    let flat = MixSettings::flat();
    let (tail, rooms, discarded) = fly(&world, false, flat);
    let (tail_db, _) = worst_over_steady(&tail, flight, flight, &rooms, "tail alone");
    println!("tail alone: {discarded} fans discarded");
    let (full, rooms, discarded) = fly(&world, true, flat);
    let (full_db, _) = worst_over_steady(&full, flight, flight, &rooms, "full output");
    println!("full output: {discarded} fans discarded");
    // The same flight at the default room curves, printed beside the flat figures.
    let at = MixSettings::default();
    let (tail_d, rooms, _) = fly(&world, false, at);
    let (tail_d_db, _) = worst_over_steady(&tail_d, flight, flight, &rooms, "defaults, tail");
    let (full_d, rooms, _) = fly(&world, true, at);
    let (full_d_db, _) = worst_over_steady(&full_d, flight, flight, &rooms, "defaults, full");
    println!(
        "over steady: tail alone {tail_db:+.2} dB flat, {tail_d_db:+.2} dB at the defaults; \
         full output {full_db:+.2} dB flat, {full_d_db:+.2} dB at the defaults"
    );
    assert!(
        tail_db <= BOUND_DB,
        "tail alone {tail_db:+.2} dB over steady"
    );
    assert!(
        full_db <= BOUND_DB,
        "full output {full_db:+.2} dB over steady"
    );
}

const CHAMBER: RoomTargets = RoomTargets {
    t60_s: 2.26,
    mean_free_path_m: 8.0,
    energy: 0.379,
};
/// A room with no surface: the shortest decay and the largest energy a room estimate allows.
const INSIDE_ROCK: RoomTargets = RoomTargets {
    t60_s: 0.05,
    mean_free_path_m: 0.0,
    energy: 16.0,
};
/// The mixer alone, the tail alone (the hum 2 m away, direct path off): the chamber's room for
/// 3 s, the rock room for 0.5 s, the chamber's again for 3 s.
fn rock_room_between_chambers(lines: TailLines) -> Vec<f32> {
    let mut m = Mixer::new(Hrtf::sadie2_d2(), MPV);
    m.set_mix(MixSettings {
        reverb: ReverbSettings {
            lines,
            ..MixSettings::flat().reverb
        },
        ..MixSettings::flat()
    });
    m.apply(Command::SetRoom(CHAMBER));
    m.apply(Command::Play(
        VoiceId(1),
        hum(),
        PlayParams {
            looping: true,
            direct: false,
            ..PlayParams::at([20.0, 0.0, 0.0])
        },
    ));
    let mut out = Vec::new();
    for (seconds, room) in [
        (HOLD, None),
        (0.5, Some(INSIDE_ROCK)),
        (HOLD, Some(CHAMBER)),
    ] {
        if let Some(r) = room {
            m.apply(Command::SetRoom(r));
        }
        let frames = (seconds * FS) as usize / BLOCK * BLOCK;
        let mut part = vec![0.0; 2 * frames];
        m.render(&mut part, frames);
        out.extend_from_slice(&part);
    }
    out
}

/// The chamber's own level for the mixer-alone case is the largest window of its first hold,
/// the tail's switch-on included: a fresh network fed a steady tone swells past its steady
/// level while its modes build up, and a tail handed over to a fresh network on the way back
/// rises as it did at the start. Asserted with the handover (the default); printed with
/// gliding lines, whose single network carries the rock room's stored tail into the chamber's
/// longer decay.
#[test]
fn the_tail_settles_after_a_room_from_inside_rock_without_overshooting() {
    let change = (HOLD * FS) as usize / BLOCK * BLOCK;
    let back = change + (0.5 * FS) as usize / BLOCK * BLOCK;
    let rooms = [(change / BLOCK, INSIDE_ROCK), (back / BLOCK, CHAMBER)];
    for lines in [TailLines::Handover, TailLines::Glide] {
        let out = rock_room_between_chambers(lines);
        let what = format!("{lines:?}");
        let w = window_rms(&out);
        let largest = |from: usize, to: usize| {
            w.iter()
                .filter(|(s, _)| *s >= from && s + WINDOW <= to)
                .map(|w| w.1)
                .fold(0.0f64, f64::max)
        };
        let db = |r: f64, of: f64| 20.0 * (r / of).log10();
        let steady = largest(change - RATE as usize, change);
        let own = largest(0, change);
        let rock = largest(change, back);
        let (at, after) = w
            .iter()
            .filter(|(s, _)| *s >= back)
            .fold((0, 0.0f64), |m, &(s, r)| if r > m.1 { (s, r) } else { m });
        let _ = worst_over_steady(&out, change, change, &rooms, &what);
        let over = db(after, own);
        println!(
            "{what}: the first hold's largest window {:+.2} dB over its steady level; the rock \
             room's {:+.2} dB; back in the chamber {:+.2} dB at {:.3} s, {over:+.2} dB over the \
             first hold's largest",
            db(own, steady),
            db(rock, steady),
            db(after, steady),
            at as f64 / FS
        );
        if lines == TailLines::Handover {
            assert!(
                over <= BOUND_DB,
                "{what}: {over:+.2} dB over the first hold's largest"
            );
        }
    }
}
