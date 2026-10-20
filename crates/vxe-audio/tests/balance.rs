//! The balance a listener sets: the echo gain and the tail level each scale only their own
//! part, the pre-delay holds the tail back without changing it, and the table of direct,
//! early and tail energies in closed cubes at the default balance.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use vxe_audio::*;
use vxe_world::*;

/// Counts heap allocations made on a thread that has switched counting on.
struct Counting;

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
}

// SAFETY: forwards every call to `System` unchanged; the thread-locals are `const`-initialised
// `Cell`s, which never allocate.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.with(Cell::get) {
            ALLOCATIONS.with(|a| a.set(a.get() + 1));
        }
        // SAFETY: the caller upholds `GlobalAlloc::alloc`'s contract, passed through.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` came from `System.alloc` with this layout.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if COUNTING.with(Cell::get) {
            ALLOCATIONS.with(|a| a.set(a.get() + 1));
        }
        // SAFETY: as `alloc` and `dealloc`, passed through.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

const MPV: f32 = 0.1;
const FS: f64 = RATE as f64;
/// The cave row: absorption 0.3 per metre, transmission 0.5.
const CAVE: MaterialId = 1;
/// The first 80 ms after the direct sound.
const EARLY_FRAMES: usize = 3_840;

/// A closed cube of air of `edge` voxels, `[0, edge)³`, inside 2 m of `CAVE`, air beyond:
/// bricks wholly inside stay uniform air, so a 100 m cube generates quickly.
struct ShellCube {
    edge: i32,
}

impl WorldSource for ShellCube {
    fn chunk(&self, coord: ChunkCoord, out: &mut Chunk) {
        let base = coord.min_voxel().0;
        let edge = self.edge;
        let material = |v: [i32; 3]| {
            let inside = v.iter().all(|&c| (0..edge).contains(&c));
            let shell = v.iter().all(|&c| (-20..edge + 20).contains(&c));
            if shell && !inside { CAVE } else { AIR }
        };
        for slot in 0..CHUNK_BRICKS {
            let b = brick_of_slot(slot);
            let lo = [0, 1, 2].map(|k| base[k] + b[k] * BRICK_EDGE);
            let brick = if lo.iter().all(|&c| c >= 0 && c + BRICK_EDGE <= edge) {
                Brick::Uniform(AIR)
            } else {
                Brick::from_fn(|l| material([0, 1, 2].map(|k| lo[k] + l[k])))
            };
            out.set_brick(slot, brick);
        }
    }
}

/// The cube of `edge` voxels in its shell.
fn cube(edge: i32) -> World {
    let mut t = MaterialTable::new();
    let mut m = Material::named("cave");
    m.acoustic = [0.3, 0.5, 0.0];
    assert_eq!(t.push(m), CAVE);
    let hi = (edge + 20 + CHUNK_EDGE - 1) / CHUNK_EDGE;
    World::generate(
        ChunkExtent::new([-1, -1, -1], [hi, hi, hi]),
        &ShellCube { edge },
        &t,
    )
}

/// Which parts of the output a render keeps.
#[derive(Clone, Copy)]
struct Parts {
    direct: bool,
    echoes: bool,
    tail: bool,
}

const DIRECT: Parts = Parts {
    direct: true,
    echoes: false,
    tail: false,
};
const ECHOES: Parts = Parts {
    direct: false,
    echoes: true,
    tail: false,
};
const TAIL: Parts = Parts {
    direct: false,
    echoes: false,
    tail: true,
};

/// A unit impulse 2 m ahead (`+x`) of a listener at the centre of the cube of `edge` voxels:
/// the room fan completed, `mix` applied, then 0.5 s of silence, then the impulse played and
/// probed once, `seconds` rendered from the impulse on with `parts` sounding.
fn impulse(world: &World, edge: i32, mix: MixSettings, parts: Parts, seconds: f64) -> Vec<f32> {
    let c = edge as f32 / 2.0;
    let mut mixer = Mixer::new(Hrtf::sadie2_d2(), MPV);
    let listener = Listener {
        position: [c, c, c],
        ..Listener::default()
    };
    mixer.apply(Command::SetListener(listener));
    let mut acoustics = Acoustics::new(mixer.clock());
    acoustics.set_reflections(parts.echoes);
    acoustics.set_reverb(parts.tail);
    for _ in 0..ROOM_RAYS / ROOM_RAYS_PER_UPDATE {
        acoustics.update(world, world.materials(), &listener, &[], MPV, &mut |cmd| {
            mixer.apply(cmd)
        });
    }
    mixer.apply(Command::SetMix(mix));
    let mut silence = vec![0.0; RATE as usize];
    mixer.render(&mut silence, RATE as usize / 2);
    let source = [c + 20.0, c, c];
    let id = VoiceId(1);
    mixer.apply(Command::Play(
        id,
        Sound::new(vec![1.0]),
        PlayParams {
            direct: parts.direct,
            ..PlayParams::at(source)
        },
    ));
    acoustics.update(
        world,
        world.materials(),
        &listener,
        &[(id, source)],
        MPV,
        &mut |cmd| {
            if matches!(cmd, Command::SetAcoustics(..)) {
                mixer.apply(cmd);
            }
        },
    );
    let frames = (seconds * FS) as usize;
    let mut out = vec![0.0; 2 * frames];
    mixer.render(&mut out, frames);
    out
}

/// Energy of interleaved `out` from frame `from` to `to`, averaged over the two ears.
fn energy(out: &[f32], from: usize, to: usize) -> f64 {
    out[2 * from..2 * to.min(out.len() / 2)]
        .iter()
        .map(|&x| f64::from(x) * f64::from(x))
        .sum::<f64>()
        / 2.0
}

fn total(out: &[f32]) -> f64 {
    energy(out, 0, out.len() / 2)
}

fn db(x: f64) -> f64 {
    10.0 * x.max(1e-30).log10()
}

/// The mix with the echo gain and the tail level at `echo_db` and `tail_db`.
fn mix(echo_db: f32, tail_db: f32) -> MixSettings {
    MixSettings {
        echo_gain: 10f32.powf(echo_db / 20.0),
        reverb: ReverbSettings {
            level: 10f32.powf(tail_db / 20.0),
            ..ReverbSettings::default()
        },
        ..MixSettings::default()
    }
}

/// In the 12 m cube: the echo gain at +6 dB multiplies the echoes' energy by `10^0.6 = 3.98`,
/// the tail level at -6 dB the tail's by `10^-0.6 = 0.251`, each within 1 percent, and
/// neither changes the other's render by a bit.
#[test]
fn the_echo_gain_and_the_tail_level_scale_only_their_own_part() {
    let world = cube(120);
    let echoes = impulse(&world, 120, MixSettings::default(), ECHOES, 1.0);
    let echoes_up = impulse(&world, 120, mix(6.0, 0.0), ECHOES, 1.0);
    let echoes_tail_down = impulse(&world, 120, mix(0.0, -6.0), ECHOES, 1.0);
    let tail = impulse(&world, 120, MixSettings::default(), TAIL, 4.0);
    let tail_down = impulse(&world, 120, mix(0.0, -6.0), TAIL, 4.0);
    let tail_echoes_up = impulse(&world, 120, mix(6.0, 0.0), TAIL, 4.0);
    let e = total(&echoes_up) / total(&echoes);
    let t = total(&tail_down) / total(&tail);
    println!(
        "12 m cube: echoes +6 dB multiply their energy by {e:.4} (3.9811); tail -6 dB by {t:.4} \
         (0.2512)"
    );
    assert!((e / 10f64.powf(0.6) - 1.0).abs() <= 0.01, "echoes x{e}");
    assert!((t / 10f64.powf(-0.6) - 1.0).abs() <= 0.01, "tail x{t}");
    assert_eq!(
        echoes, echoes_tail_down,
        "the tail level changed the echoes"
    );
    assert_eq!(tail, tail_echoes_up, "the echo gain changed the tail");
}

/// A pre-delay of 40 ms (1 920 frames) holds the tail back: nothing before frame `1 920 + 31`
/// (31 the shortest line any room gives), and the same energy within 0.1 dB.
#[test]
fn a_pre_delay_holds_the_tail_back_without_changing_it() {
    let world = cube(120);
    let held = MixSettings {
        reverb: ReverbSettings {
            predelay_s: 0.04,
            ..ReverbSettings::default()
        },
        ..MixSettings::default()
    };
    assert_eq!(held.reverb.predelay_frames(), 1_920);
    let tail = impulse(&world, 120, MixSettings::default(), TAIL, 4.0);
    let late = impulse(&world, 120, held, TAIL, 4.04);
    let first = |out: &[f32]| out.chunks_exact(2).position(|f| f[0] != 0.0 || f[1] != 0.0);
    let d = db(total(&late) / total(&tail));
    println!(
        "pre-delay 40 ms: first nonzero frame {:?} (without {:?}); energy {d:+.4} dB",
        first(&late),
        first(&tail)
    );
    assert!(
        late[..2 * (1_920 + 31)].iter().all(|&x| x == 0.0),
        "the tail sounds before frame 1 951"
    );
    assert!(d.abs() <= 0.1, "energy {d} dB");
}

/// The room the fan reads from the centre of the cube of `edge` voxels, and the tail's lines a
/// mixer takes from it.
fn fan_room(world: &World, edge: i32) -> (RoomEstimate, [ReverbLine; REVERB_LINES]) {
    let c = edge as f32 / 2.0;
    let mut mixer = Mixer::new(Hrtf::sadie2_d2(), MPV);
    let listener = Listener {
        position: [c, c, c],
        ..Listener::default()
    };
    let mut acoustics = Acoustics::new(mixer.clock());
    for _ in 0..ROOM_RAYS / ROOM_RAYS_PER_UPDATE {
        acoustics.update(world, world.materials(), &listener, &[], MPV, &mut |cmd| {
            mixer.apply(cmd)
        });
    }
    (
        acoustics.room().expect("the fan completed"),
        mixer.reverb_lines(),
    )
}

fn is_prime(n: u32) -> bool {
    n >= 2
        && (2..)
            .take_while(|d| d * d <= n)
            .all(|d| !n.is_multiple_of(d))
}

/// From the 100 m cube's centre (mean free path about 65.6 m) the tail's 16 lines are distinct
/// primes, strictly increasing and under the cap: none clamps to a shared length.
#[test]
fn the_100_m_cube_gives_sixteen_distinct_lines() {
    let world = cube(1000);
    let (room, lines) = fan_room(&world, 1000);
    let targets: Vec<u32> = lines.iter().map(|l| l.target).collect();
    println!(
        "100 m cube: mfp {:.3} m; line targets {targets:?} (under a 4 800-frame cap: 4 591 and \
         fifteen 4 799)",
        room.mean_free_path_m
    );
    assert!(targets.iter().all(|&m| is_prime(m)), "{targets:?}");
    assert!(targets.windows(2).all(|w| w[0] < w[1]), "{targets:?}");
    assert!(
        targets
            .iter()
            .all(|&m| (m as usize) < REVERB_MAX_DELAY_FRAMES),
        "{targets:?}"
    );
}

/// Energy of one ear of interleaved `out`.
fn ear_energy(out: &[f32], ear: usize) -> f64 {
    out.iter()
        .skip(ear)
        .step_by(2)
        .map(|&x| f64::from(x) * f64::from(x))
        .sum()
}

/// The default balance, for the listener's reference: per cube, a unit impulse 2 m ahead,
/// the direct path, the echoes (the buses alone) and the tail, each alone, in dB of energy,
/// whole and over the first 80 ms after the direct sound. The tail's energy (both ears
/// averaged) is within 1.0 dB of the room's own `energy` in every cube, the 100 m one
/// included, and each ear's tail sounds.
#[test]
fn the_balance_table_at_the_defaults() {
    let mut rows = Vec::new();
    let mut tails = Vec::new();
    for metres in [4, 12, 36, 100] {
        let edge = metres * 10;
        let world = cube(edge);
        let (room, lines) = fan_room(&world, edge);
        let direct = impulse(&world, edge, MixSettings::default(), DIRECT, 0.1);
        let echoes = impulse(&world, edge, MixSettings::default(), ECHOES, 0.6);
        let seconds = match metres {
            4 => 1.5,
            12 => 3.5,
            _ => 9.0,
        };
        let tail = impulse(&world, edge, MixSettings::default(), TAIL, seconds);
        let row = format!(
            "| {metres} m | {:+.1} | {:+.1} | {:+.1} | {:+.1} | {:+.1} | {:+.1} |",
            db(total(&direct)),
            db(total(&echoes)),
            db(total(&tail)),
            db(energy(&direct, 0, EARLY_FRAMES)),
            db(energy(&echoes, 0, EARLY_FRAMES)),
            db(energy(&tail, 0, EARLY_FRAMES)),
        );
        rows.push(row);
        assert!(total(&direct) > 0.0 && total(&echoes) > 0.0 && total(&tail) > 0.0);
        let (want, got) = (db(f64::from(room.energy)), db(total(&tail)));
        let (left, right) = (ear_energy(&tail, 0), ear_energy(&tail, 1));
        tails.push(format!(
            "{metres} m: tail {got:+.2} dB against the room's {want:+.2} dB ({:+.2}); left \
             {:+.2} dB, right {:+.2} dB, left - right {:+.2} dB; lines {:?}",
            got - want,
            db(left),
            db(right),
            db(left) - db(right),
            lines.iter().map(|l| l.target).collect::<Vec<_>>()
        ));
        assert!(
            right > 0.0 && left > 0.0,
            "{metres} m: an ear's tail is silent"
        );
        assert!(
            (got - want).abs() <= 1.0,
            "{metres} m: tail {got:+.2} dB against {want:+.2} dB"
        );
    }
    println!("{}", tails.join("\n"));
    println!(
        "balance at the defaults, unit impulse 2 m ahead, dB of energy:\n| cube | direct | early \
         | tail | direct, 80 ms | early, 80 ms | tail, 80 ms |\n{}",
        rows.join("\n")
    );
}

/// Rendering while the balance changes every block allocates nothing.
#[test]
fn a_changing_balance_renders_without_allocating() {
    let mut m = Mixer::new(Hrtf::sadie2_d2(), MPV);
    let hum = Sound::new(
        (0..4800)
            .map(|n| (n as f32 * 0.05).sin())
            .collect::<Vec<_>>(),
    );
    m.apply(Command::Play(
        VoiceId(1),
        hum,
        PlayParams {
            looping: true,
            ..PlayParams::at([20.0, 0.0, 0.0])
        },
    ));
    let mut taps = [Tap::default(); MAX_TAPS];
    taps[0] = Tap {
        delay_frames: 900,
        gain: 0.1,
    };
    m.apply(Command::SetAcoustics(
        VoiceId(1),
        AcousticTargets {
            taps,
            ..AcousticTargets::default()
        },
    ));
    m.apply(Command::SetRoom(RoomTargets {
        t60_s: 2.0,
        mean_free_path_m: 8.0,
        energy: 0.4,
    }));
    let mut out = vec![0.0; 2 * BLOCK];
    m.render(&mut out, BLOCK);
    COUNTING.with(|c| c.set(true));
    for b in 0..64 {
        // A room every 8 blocks, alternating 8 m and 10 m: a handover clears a network.
        if b % 8 == 0 {
            m.apply(Command::SetRoom(RoomTargets {
                t60_s: 2.0,
                mean_free_path_m: if b % 16 == 0 { 10.0 } else { 8.0 },
                energy: 0.4,
            }));
        }
        m.apply(Command::SetMix(MixSettings {
            echo_gain: 1.0 + b as f32 * 0.1,
            direct_delay: b % 2 == 0,
            reverb: ReverbSettings {
                level: 0.5 + b as f32 * 0.01,
                predelay_s: (b % 5) as f32 * 0.03,
                lines: if b % 3 == 0 {
                    TailLines::Glide
                } else {
                    TailLines::Handover
                },
                ..ReverbSettings::default()
            },
        }));
        m.render(&mut out, BLOCK);
    }
    COUNTING.with(|c| c.set(false));
    assert_eq!(ALLOCATIONS.with(Cell::get), 0);
    assert!(out.iter().all(|s| s.is_finite()));
}
