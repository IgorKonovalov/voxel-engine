//! The room fan: the volume, surface, absorption and decay time a fan of rays from the listener
//! reads in closed cubes and in open air, and when it sends them.

use vxe_audio::*;
use vxe_world::*;

const MPV: f32 = 0.1;
/// The cave row: absorption 0.3 per metre, transmission 0.5.
const CAVE: MaterialId = 1;

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

/// A closed cube of air of `edge` voxels, `[0, edge)³`, in rock filling chunks `[-1, 3)³`.
fn cube(edge: i32) -> World {
    world([-1, -1, -1], [3, 3, 3], move |v| {
        v.iter().any(|&c| !(0..edge).contains(&c))
    })
}

/// Runs `calls` updates for a still listener at `at` (voxel units) with no voices and returns
/// the room commands sent.
fn fan(world: &World, at: [f32; 3], calls: usize) -> (Acoustics, Vec<RoomTargets>) {
    let mixer = Mixer::new(Hrtf::sadie2_d2(), MPV);
    let mut acoustics = Acoustics::new(mixer.clock());
    let listener = Listener {
        position: at,
        ..Listener::default()
    };
    let mut rooms = Vec::new();
    for _ in 0..calls {
        acoustics.update(world, world.materials(), &listener, &[], MPV, &mut |cmd| {
            if let Command::SetRoom(r) = cmd {
                rooms.push(r);
            }
        });
    }
    (acoustics, rooms)
}

fn within(actual: f32, expected: f32, rel: f32, what: &str) {
    assert!(
        (actual - expected).abs() <= rel * expected,
        "{what}: {actual} against {expected} (tolerance {rel})"
    );
}

/// The 12 m cube read from its centre and from `(8 m, 6.3 m, 5.8 m)`: `V = 1728 m^3`,
/// `S = 864 m^2`, mean free path 8 m, `alpha = 1 - 0.7 ^ 0.4 = 0.1330` and Eyring's
/// `T60 = 0.161 x 1728 / (-864 ln(1 - 0.1330)) = 2.26 s`.
#[test]
fn a_12_m_cube_reads_its_volume_surface_and_decay() {
    let room = cube(120);
    let alpha = 1.0 - 0.7f32.powf(0.4);
    let t60 = 0.161 * 1728.0 / (-864.0 * (1.0 - alpha).ln());
    for at in [[60.0, 60.0, 60.0], [80.0, 63.0, 58.0]] {
        let (acoustics, rooms) = fan(&room, at, 8);
        let r = acoustics.room().expect("a fan completes in 8 calls");
        println!("12 m cube from {at:?}: {r:?} (alpha {alpha}, T60 {t60})");
        assert_eq!(rooms.len(), 1);
        within(r.volume_m3, 1728.0, 0.04, "volume");
        within(r.surface_m2, 864.0, 0.04, "surface");
        within(r.mean_free_path_m, 8.0, 0.04, "mean free path");
        assert!(
            (r.absorption - 0.1330).abs() <= 0.001,
            "alpha {}",
            r.absorption
        );
        within(r.t60_s, 2.26, 0.04, "T60");
        assert_eq!(rooms[0].t60_s, r.t60_s);
        assert_eq!(rooms[0].mean_free_path_m, r.mean_free_path_m);
        assert_eq!(rooms[0].energy, r.energy);
    }
}

/// The 4 m cube from 1.7 m off its centre along `x`: mean free path `4V / S = 2.667 m`.
#[test]
fn a_4_m_cube_off_centre_reads_its_mean_free_path() {
    let room = cube(40);
    let (acoustics, _) = fan(&room, [37.0, 20.0, 20.0], 8);
    let r = acoustics.room().expect("complete");
    println!("4 m cube from 1.7 m off centre: {r:?}");
    within(
        r.mean_free_path_m,
        4.0 * 64.0 / 96.0,
        0.05,
        "mean free path",
    );
}

/// Open air: every ray misses, so the fan reads a sphere of fully absorbing faces at the
/// range, its absorption clamped to 0.98, and no tail at all: energy exactly 0.
#[test]
fn open_air_gives_no_tail() {
    let air = world([-1, -1, -1], [1, 1, 1], |_| false);
    let (acoustics, rooms) = fan(&air, [0.0, 0.0, 0.0], 8);
    let r = acoustics.room().expect("complete");
    println!("open air: {r:?}");
    assert_eq!(r.absorption, 0.98);
    assert_eq!(r.energy, 0.0);
    assert_eq!(rooms[0].energy, 0.0);
}

/// A listener inside a solid block: a fan due there is discarded, so no `SetRoom` goes out over
/// 16 calls and the estimate from the chamber stands.
#[test]
fn a_listener_inside_a_solid_sends_no_room() {
    let room = cube(120);
    let (mut acoustics, rooms) = fan(&room, [60.0, 60.0, 60.0], 8);
    assert_eq!(rooms.len(), 1);
    let before = acoustics.room().expect("complete");
    assert_eq!(acoustics.discarded_fans(), 0);
    // 1.5 m into the rock beyond the face at x = 0: more than a fan's move away.
    let listener = Listener {
        position: [-15.0, 60.0, 60.0],
        ..Listener::default()
    };
    let mut sent = 0;
    for _ in 0..16 {
        acoustics.update(&room, room.materials(), &listener, &[], MPV, &mut |cmd| {
            sent += usize::from(matches!(cmd, Command::SetRoom(_)));
        });
        assert_eq!(acoustics.last_room_rays(), 0);
    }
    println!(
        "inside a solid: {sent} rooms sent over 16 calls, {} fans discarded",
        acoustics.discarded_fans()
    );
    assert_eq!(sent, 0);
    assert_eq!(acoustics.room(), Some(before));
    assert!(acoustics.discarded_fans() > 0);
}

/// No `SetRoom` before the first fan completes (8 calls of 16 rays), none while the listener
/// stands still and less than 0.5 s of audio has passed, one within 8 calls of a 1.1 m move,
/// and one again once 0.5 s of audio has passed.
#[test]
fn the_fan_follows_its_cadence() {
    let room = cube(120);
    let mut mixer = Mixer::new(Hrtf::sadie2_d2(), MPV);
    let mut acoustics = Acoustics::new(mixer.clock());
    let mut listener = Listener {
        position: [60.0, 60.0, 60.0],
        ..Listener::default()
    };
    let call = |acoustics: &mut Acoustics, listener: &Listener| {
        let mut sent = 0;
        acoustics.update(&room, room.materials(), listener, &[], MPV, &mut |cmd| {
            sent += usize::from(matches!(cmd, Command::SetRoom(_)));
        });
        (sent, acoustics.last_room_rays())
    };
    for i in 0..7 {
        assert_eq!(call(&mut acoustics, &listener), (0, 16), "call {i}");
        assert!(acoustics.room().is_none());
    }
    assert_eq!(
        call(&mut acoustics, &listener),
        (1, 16),
        "the eighth call completes"
    );
    // Still, and no audio time passed: nothing traced, nothing sent.
    for i in 0..20 {
        assert_eq!(call(&mut acoustics, &listener), (0, 0), "still, call {i}");
    }
    // 1.1 m: a new fan, complete within 8 calls.
    listener.position[0] += 11.0;
    let sent: Vec<_> = (0..8).map(|_| call(&mut acoustics, &listener)).collect();
    assert_eq!(sent.iter().map(|s| s.0).sum::<usize>(), 1, "{sent:?}");
    assert_eq!(sent[7], (1, 16));
    assert_eq!(call(&mut acoustics, &listener), (0, 0));
    // 0.5 s of audio is 93.75 blocks: a new fan begins at the 94th block.
    let mut out = vec![0.0; 2 * BLOCK];
    for _ in 0..93 {
        mixer.render(&mut out, BLOCK);
    }
    assert_eq!(call(&mut acoustics, &listener), (0, 0));
    mixer.render(&mut out, BLOCK);
    assert_eq!(call(&mut acoustics, &listener), (0, 16));
}

/// The tail switched off sends the last estimate again with energy 0, and on again with its
/// energy.
#[test]
fn switching_the_tail_resends_the_room() {
    let room = cube(120);
    let (mut acoustics, rooms) = fan(&room, [60.0, 60.0, 60.0], 8);
    let energy = rooms[0].energy;
    assert!(energy > 0.0);
    let listener = Listener {
        position: [60.0, 60.0, 60.0],
        ..Listener::default()
    };
    let mut sent = Vec::new();
    acoustics.set_reverb(false);
    assert!(!acoustics.reverb_on());
    acoustics.update(&room, room.materials(), &listener, &[], MPV, &mut |cmd| {
        sent.push(cmd)
    });
    acoustics.set_reverb(true);
    acoustics.update(&room, room.materials(), &listener, &[], MPV, &mut |cmd| {
        sent.push(cmd)
    });
    let energies: Vec<f32> = sent
        .iter()
        .map(|c| match c {
            Command::SetRoom(r) => r.energy,
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(energies, [0.0, energy]);
}

/// A closed cube of air of `edge` voxels, `[0, edge)³`, inside 2 m of `CAVE`, air beyond: bricks
/// wholly inside stay uniform air, so a 100 m cube generates quickly.
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

/// The 100 m cube from its centre: every room ray hits within the 100 m range (the corners are
/// 86.6 m away), so the fan reads `V = 1e6 m^3`, Eyring's 18.8 s clamped to the 8 s cap, and the
/// tail's energy `16 pi (1 - 0.1330) / (6e4 x 0.1330) = 5.46e-3` (-22.6 dB).
#[test]
fn a_100_m_cube_reads_its_volume_and_the_capped_decay() {
    let mut t = MaterialTable::new();
    let mut m = Material::named("cave");
    m.acoustic = [0.3, 0.5, 0.0];
    assert_eq!(t.push(m), CAVE);
    let cube = World::generate(
        ChunkExtent::new([-1, -1, -1], [17, 17, 17]),
        &ShellCube { edge: 1000 },
        &t,
    );
    let (acoustics, rooms) = fan(&cube, [500.0, 500.0, 500.0], 8);
    let r = acoustics.room().expect("complete");
    let alpha = 1.0 - 0.7f32.powf(0.4);
    let energy = 16.0 * std::f32::consts::PI * (1.0 - alpha) / (6e4 * alpha);
    println!("100 m cube from the centre: {r:?} (energy {energy:e})");
    assert_eq!(rooms.len(), 1);
    within(r.volume_m3, 1e6, 0.04, "volume");
    assert_eq!(r.t60_s, MAX_T60_SECONDS);
    within(r.energy, energy, 0.04, "energy");
}
