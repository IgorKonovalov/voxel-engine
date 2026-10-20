//! The late tail: its decay time and energy against the room it is given, the cubes the fan
//! measures, a room that changes under a hum (gliding lines, and two networks handing over
//! without moving a line that holds signal), and no allocation while it runs.

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
const CAVE: MaterialId = 1;
const FS: f64 = RATE as f64;
const MODES: [TailLines; 2] = [TailLines::Handover, TailLines::Glide];

/// A mixer whose tail takes rooms as `lines` says.
fn mixer(lines: TailLines) -> Mixer {
    let mut m = Mixer::new(Hrtf::sadie2_d2(), MPV);
    m.set_reverb_settings(ReverbSettings {
        lines,
        ..ReverbSettings::default()
    });
    m
}

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

/// A closed cube of air of `edge` voxels, `[0, edge)³`, in rock of the cave row (`A = 0.3`)
/// filling the chunks around it.
fn cube(edge: i32) -> World {
    let mut t = MaterialTable::new();
    let mut m = Material::named("cave");
    m.acoustic = [0.3, 0.5, 0.0];
    assert_eq!(t.push(m), CAVE);
    let hi = (edge + CHUNK_EDGE - 1) / CHUNK_EDGE + 1;
    World::generate(
        ChunkExtent::new([-1, -1, -1], [hi, hi, hi]),
        &FnSource(move |v: [i32; 3]| {
            if v.iter().any(|&c| !(0..edge).contains(&c)) {
                CAVE
            } else {
                AIR
            }
        }),
        &t,
    )
}

/// A one-sample unit impulse.
fn impulse() -> Sound {
    Sound::new(vec![1.0])
}

/// Per-frame energy averaged over the two ears.
fn frame_energy(out: &[f32]) -> Vec<f64> {
    out.chunks_exact(2)
        .map(|f| (f64::from(f[0]).powi(2) + f64::from(f[1]).powi(2)) / 2.0)
        .collect()
}

/// The decay time from the Schroeder backward integral of `e` (per-frame energy): the line
/// fitted to `10 log10` of the integral between 10 and 30 dB below its value at 50 ms,
/// extrapolated to 60 dB.
fn schroeder_t60(e: &[f64]) -> f64 {
    let mut edc = vec![0.0f64; e.len() + 1];
    for i in (0..e.len()).rev() {
        edc[i] = edc[i + 1] + e[i];
    }
    let at50 = (0.05 * FS) as usize;
    let db: Vec<f64> = edc[..e.len()]
        .iter()
        .map(|&v| 10.0 * (v / edc[at50]).log10())
        .collect();
    let (mut sx, mut sy, mut sxx, mut sxy, mut n) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for (i, &d) in db.iter().enumerate().skip(at50) {
        if (-30.0..=-10.0).contains(&d) {
            let t = i as f64 / FS;
            sx += t;
            sy += d;
            sxx += t * t;
            sxy += t * d;
            n += 1.0;
        }
    }
    assert!(n > 100.0, "the decay reaches 30 dB down within the render");
    let slope = (n * sxy - sx * sy) / (n * sxx - sx * sx);
    -60.0 / slope
}

/// The normalized echo density (Abel and Huang) of one ear around `t` seconds: over a 20 ms
/// window, the fraction of samples beyond one standard deviation, over `erfc(1 / sqrt 2)`.
fn echo_density(out: &[f32], ear: usize, t: f64) -> f64 {
    let half = (0.01 * FS) as usize;
    let c = (t * FS) as usize;
    let w: Vec<f64> = out
        .chunks_exact(2)
        .skip(c - half)
        .take(2 * half)
        .map(|f| f64::from(f[ear]))
        .collect();
    let sd = (w.iter().map(|x| x * x).sum::<f64>() / w.len() as f64).sqrt();
    let beyond = w.iter().filter(|x| x.abs() > sd).count() as f64 / w.len() as f64;
    beyond / 0.317_310_507_862_914_1
}

/// A unit impulse at `(1 m, 0, 0)`, no acoustics, the room `room` given straight to the mixer
/// 0.5 s before (the tail's input gain fades in from 0 over its 50 ms smoother): `seconds` of
/// output from the impulse on.
fn impulse_in_room(lines: TailLines, room: Option<RoomTargets>, seconds: f64) -> Vec<f32> {
    let mut m = mixer(lines);
    if let Some(r) = room {
        m.apply(Command::SetRoom(r));
    }
    let mut silence = vec![0.0; 2 * RATE as usize / 2];
    m.render(&mut silence, RATE as usize / 2);
    m.apply(Command::Play(
        VoiceId(1),
        impulse(),
        PlayParams::at([10.0, 0.0, 0.0]),
    ));
    let frames = (seconds * FS) as usize;
    let mut out = vec![0.0; 2 * frames];
    m.render(&mut out, frames);
    out
}

#[test]
fn the_tail_decays_in_its_t60_with_its_energy() {
    let room = RoomTargets {
        t60_s: 2.26,
        mean_free_path_m: 8.0,
        energy: 0.379,
    };
    for mode in MODES {
        let out = impulse_in_room(mode, Some(room), 4.0);
        let e = frame_energy(&out);
        let t60 = schroeder_t60(&e);
        let tail: f64 = e[128..].iter().sum();
        let db = 10.0 * (tail / 0.379).log10();
        let dens: Vec<String> = [0.1, 0.3]
            .iter()
            .flat_map(|&t| (0..2).map(move |ear| (t, ear)))
            .map(|(t, ear)| {
                format!(
                    "{:.0} ms ear {ear} {:.3}",
                    t * 1e3,
                    echo_density(&out, ear, t)
                )
            })
            .collect();
        println!(
            "{mode:?}: tail T60 {t60:.3} s (2.26), energy from frame 128 {tail:.4} (0.379, \
         {db:+.2} dB); echo density {}",
            dens.join(", ")
        );
        assert!((t60 - 2.26).abs() <= 0.1 * 2.26, "{mode:?}: T60 {t60}");
        assert!(
            db.abs() <= 2.0,
            "{mode:?}: tail energy {tail} is {db} dB from 0.379"
        );
    }
}

/// A unit impulse 2 m ahead of a listener at the centre of a closed cube of `edge` voxels,
/// played 0.5 s after the fan completed and probed once, reflections on, the tail `on` or off:
/// the output from the impulse on and the room the fan read.
fn cube_impulse(
    lines: TailLines,
    world: &World,
    edge: i32,
    on: bool,
    seconds: f64,
) -> (Vec<f32>, RoomEstimate) {
    let c = edge as f32 / 2.0;
    let mut mixer = mixer(lines);
    let listener = Listener {
        position: [c, c, c],
        ..Listener::default()
    };
    mixer.apply(Command::SetListener(listener));
    let source = [c + 20.0, c, c];
    let mut acoustics = Acoustics::new(mixer.clock());
    acoustics.set_reverb(on);
    for _ in 0..ROOM_RAYS / ROOM_RAYS_PER_UPDATE {
        acoustics.update(world, world.materials(), &listener, &[], MPV, &mut |cmd| {
            mixer.apply(cmd)
        });
    }
    let room = acoustics.room().expect("the fan completed");
    // The tail's input gain fades in over its smoother before the impulse plays.
    let mut silence = vec![0.0; RATE as usize];
    mixer.render(&mut silence, RATE as usize / 2);
    mixer.apply(Command::Play(VoiceId(1), impulse(), PlayParams::at(source)));
    acoustics.update(
        world,
        world.materials(),
        &listener,
        &[(VoiceId(1), source)],
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
    (out, room)
}

/// Peak magnitude over both ears.
fn peak(out: &[f32]) -> f32 {
    out.iter().fold(0.0f32, |m, &s| m.max(s.abs()))
}

#[test]
fn bigger_cubes_decay_longer_and_hold_their_mode_density() {
    for mode in MODES {
        let mut last = 0.0;
        for (edge, expected) in [(40, 0.75f64), (120, 2.26), (360, 6.77)] {
            let world = cube(edge);
            let seconds = (expected * 1.3).max(1.5);
            let (out, room) = cube_impulse(mode, &world, edge, true, seconds);
            let (dry, _) = cube_impulse(mode, &world, edge, false, 0.2);
            let t60 = schroeder_t60(&frame_energy(&out));
            // The direct impulse: the HRIR's 128 frames, before the shortest line (at least 187
            // frames here) returns anything.
            let (p_on, p_off) = (peak(&out[..2 * 128]), peak(&dry[..2 * 128]));
            println!(
                "{mode:?}, {} m cube: fan T60 {:.3} s, mfp {:.3} m, energy {:.4}; fitted T60 \
             {t60:.3} s (Eyring {expected}); direct peak {p_on} with the tail, {p_off} without",
                edge / 10,
                room.t60_s,
                room.mean_free_path_m,
                room.energy
            );
            assert!(
                (t60 - expected).abs() <= 0.2 * expected,
                "{edge}: T60 {t60}"
            );
            assert!(t60 > last, "{edge}: T60 {t60} after {last}");
            last = t60;
            assert!(
                (p_on - p_off).abs() <= 0.01 * p_off,
                "{edge}: direct peak {p_on} against {p_off}"
            );
            // The mode-density floor: the lines' total length against 0.15 T60 f_s.
            let mut m = mixer(mode);
            m.apply(Command::SetRoom(RoomTargets {
                t60_s: room.t60_s,
                mean_free_path_m: room.mean_free_path_m,
                energy: room.energy,
            }));
            let total: u32 = m.reverb_lines().iter().map(|l| l.target).sum();
            let floor = 0.15 * f64::from(room.t60_s) * FS;
            println!(
                "{} m cube: line lengths sum {total}, floor {floor:.0}",
                edge / 10
            );
            assert!(f64::from(total) > floor, "{edge}: {total} against {floor}");
        }
    }
}

/// With gliding lines: a 440 Hz hum 2 m ahead in the room `(8 m, 2.26 s)` for 4 s, then
/// `next` for 4 s more, with line `line`'s read point moved by `delta` frames at each
/// `(frame, line, delta)` of `jumps` (frames counted from the step): the output of the second
/// 4 s and every line's state at each block edge of it.
fn hum_with_room_change(
    next: RoomTargets,
    jumps: &[(usize, usize, f32)],
) -> (Vec<f32>, Vec<[ReverbLine; REVERB_LINES]>) {
    let hum: Vec<f32> = (0..RATE)
        .map(|n| 0.3 * (std::f64::consts::TAU * 440.0 * f64::from(n) / FS).sin() as f32)
        .collect();
    let mut m = mixer(TailLines::Glide);
    m.apply(Command::SetRoom(RoomTargets {
        t60_s: 2.26,
        mean_free_path_m: 8.0,
        energy: 0.379,
    }));
    m.apply(Command::Play(
        VoiceId(1),
        Sound::new(hum),
        PlayParams {
            looping: true,
            ..PlayParams::at([20.0, 0.0, 0.0])
        },
    ));
    let blocks = 4 * RATE as usize / BLOCK;
    let mut out = vec![0.0; 2 * BLOCK];
    for _ in 0..blocks {
        m.render(&mut out, BLOCK);
    }
    m.apply(Command::SetRoom(next));
    let mut after = Vec::with_capacity(2 * blocks * BLOCK);
    let mut lines = vec![m.reverb_lines()];
    for b in 0..blocks {
        let start = b * BLOCK;
        // A jump inside this block splits it there.
        let mut at = start;
        for &(frame, line, delta) in jumps
            .iter()
            .filter(|j| (start..start + BLOCK).contains(&j.0))
        {
            let n = frame - at;
            if n > 0 {
                m.render(&mut out, n);
                after.extend_from_slice(&out[..2 * n]);
            }
            m.jump_reverb_line(line, delta);
            at = frame;
        }
        let n = start + BLOCK - at;
        m.render(&mut out, n);
        after.extend_from_slice(&out[..2 * n]);
        lines.push(m.reverb_lines());
    }
    (after, lines)
}

/// Per 200 ms window (hop 100 ms) of one channel: the walking test's click index `K_w` (the
/// largest second difference over `w^2 sqrt(2) rms`, `w` 440 Hz in radians per frame) and the
/// crest factor `C_w = max |x| / (sqrt 2 rms)`. A pure tone scores 1 on both.
fn windows_k_and_crest(x: &[f64]) -> Vec<(f64, f64)> {
    const WINDOW: usize = 9_600;
    const HOP: usize = 4_800;
    let w = std::f64::consts::TAU * 440.0 / FS;
    let mut v = Vec::new();
    let mut s = 0;
    while s + WINDOW <= x.len() {
        let win = &x[s..s + WINDOW];
        let rms = (win.iter().map(|v| v * v).sum::<f64>() / WINDOW as f64).sqrt();
        let d2 = win
            .windows(3)
            .map(|t| (t[2] - 2.0 * t[1] + t[0]).abs())
            .fold(0.0, f64::max);
        let peak = win.iter().fold(0.0f64, |m, v| m.max(v.abs()));
        let root2_rms = std::f64::consts::SQRT_2 * rms;
        v.push((d2 / (w * w * root2_rms), peak / root2_rms));
        s += HOP;
    }
    v
}

fn ear(out: &[f32], c: usize) -> Vec<f64> {
    out.iter()
        .skip(c)
        .step_by(2)
        .map(|&x| f64::from(x))
        .collect()
}

/// The largest `K_w / (1.25 C_w)` over the windows and ears of `out`, printing each window.
fn worst_ratio(out: &[f32], what: &str) -> f64 {
    let mut worst = 0.0f64;
    for c in 0..2 {
        let v = windows_k_and_crest(&ear(out, c));
        let line: Vec<String> = v
            .iter()
            .map(|(k, cr)| format!("{k:.3}/{cr:.3}={:.3}", k / cr))
            .collect();
        println!("{what}, ear {c}, K_w/C_w per window: {}", line.join(" "));
        worst = v
            .iter()
            .map(|(k, cr)| k / (1.25 * cr))
            .fold(worst, f64::max);
    }
    worst
}

const BIG: RoomTargets = RoomTargets {
    t60_s: 6.77,
    mean_free_path_m: 24.0,
    energy: 0.0421,
};

#[test]
fn a_room_that_grows_under_a_hum_glides_its_lines() {
    let (out, lines) = hum_with_room_change(BIG, &[]);
    let (held, _) = hum_with_room_change(
        RoomTargets {
            t60_s: 2.26,
            mean_free_path_m: 8.0,
            energy: 0.379,
        },
        &[],
    );
    let step = REVERB_DELAY_SLEW_FRAMES_PER_FRAME * BLOCK as f32;
    let mut worst = 0.0f32;
    for pair in lines.windows(2) {
        for (a, b) in pair[0].iter().zip(&pair[1]) {
            worst = worst.max((b.length - a.length).abs());
        }
    }
    let last = lines.last().expect("blocks rendered");
    let far = last
        .iter()
        .map(|l| (l.length - l.target as f32).abs())
        .fold(0.0, f32::max);
    let k_held: Vec<f64> = (0..2)
        .map(|c| {
            windows_k_and_crest(&ear(&held, c))
                .iter()
                .fold(0.0f64, |m, v| m.max(v.0))
        })
        .collect();
    let ratio = worst_ratio(&out, "room stepped");
    // The bite: line 15 jumped by half a 440 Hz period at 0.5, 1.5, 2.5 and 3.5 s, each a
    // further quarter period on, alternating in sign.
    let jumps: Vec<(usize, usize, f32)> = (0..4)
        .map(|j| {
            let frame = ((j as f64 + 0.5) * FS) as usize + 27 * j;
            (frame, 15, if j % 2 == 0 { 55.0 } else { -55.0 })
        })
        .collect();
    let (jumped, _) = hum_with_room_change(BIG, &jumps);
    let bite = worst_ratio(&jumped, "line 15 jumped");
    println!(
        "room stepped to 24 m, 6.77 s: largest line move {worst:.3} frames per block, farthest \
         from target after 4 s {far:.3}; held render K {k_held:?}; largest K_w / (1.25 C_w) \
         {ratio:.4}; with line 15 jumped {bite:.3}"
    );
    assert!(
        worst <= step + 1e-3,
        "a line moved {worst} frames in a block"
    );
    assert!(far <= 1.0, "a line is {far} frames from its target");
    assert!(ratio <= 1.0, "a window's K_w is {ratio} of 1.25 C_w");
    assert!(
        bite >= 4.0,
        "the jumped render scores only {bite} of the bound"
    );
}

/// The fraction of `x`'s energy within 3 Hz of 440 Hz over 1 s Hann windows, hop 0.5 s: the
/// DFT bins 437 to 443 Hz (1 Hz apart at 48 000 frames), both signs, over Parseval's total.
fn energy_near_440(x: &[f64]) -> f64 {
    let n = RATE as usize;
    let hann: Vec<f64> = (0..n)
        .map(|i| 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos())
        .collect();
    let (mut band, mut total) = (0.0f64, 0.0f64);
    let mut s = 0;
    while s + n <= x.len() {
        let w: Vec<f64> = x[s..s + n].iter().zip(&hann).map(|(a, h)| a * h).collect();
        total += n as f64 * w.iter().map(|v| v * v).sum::<f64>();
        for k in 437..=443 {
            let step = std::f64::consts::TAU * k as f64 / n as f64;
            let (mut re, mut im) = (0.0, 0.0);
            for (i, v) in w.iter().enumerate() {
                let (sin, cos) = (step * i as f64).sin_cos();
                re += v * cos;
                im -= v * sin;
            }
            band += 2.0 * (re * re + im * im);
        }
        s += n / 2;
    }
    band / total
}

/// A hum 2 m away heard only through the tail, the room alternating every 0.33 s for 4 s
/// between `(2.26 s, 8 m, 0.379)` and `(2.83 s, 10 m, 0.243)`, as a walk through a cave hands
/// it rooms (10 / 8 = 1.25, past the handover ratio): the output, and both networks at every
/// block edge.
fn hum_through_alternating_rooms(lines: TailLines) -> (Vec<f32>, Vec<[ReverbNetwork; 2]>) {
    const ROOMS: [RoomTargets; 2] = [
        RoomTargets {
            t60_s: 2.26,
            mean_free_path_m: 8.0,
            energy: 0.379,
        },
        RoomTargets {
            t60_s: 2.83,
            mean_free_path_m: 10.0,
            energy: 0.243,
        },
    ];
    let hum: Vec<f32> = (0..RATE)
        .map(|n| 0.3 * (std::f64::consts::TAU * 440.0 * f64::from(n) / FS).sin() as f32)
        .collect();
    let mut m = mixer(lines);
    m.apply(Command::SetRoom(ROOMS[0]));
    m.apply(Command::Play(
        VoiceId(1),
        Sound::new(hum),
        PlayParams {
            looping: true,
            direct: false,
            ..PlayParams::at([20.0, 0.0, 0.0])
        },
    ));
    let blocks = 4 * RATE as usize / BLOCK;
    let mut out = vec![0.0; 2 * blocks * BLOCK];
    let mut nets = vec![m.reverb_networks()];
    let mut room = 0;
    for (b, chunk) in out.chunks_mut(2 * BLOCK).enumerate() {
        let r = ((b * BLOCK) as f64 / FS / 0.33) as usize % 2;
        if r != room {
            room = r;
            m.apply(Command::SetRoom(ROOMS[r]));
        }
        m.render(chunk, BLOCK);
        nets.push(m.reverb_networks());
    }
    (out, nets)
}

/// Handing over between two networks never moves a line that holds signal, so the tail of a
/// hum stays at the hum's pitch; gliding lines shift it on every pass.
#[test]
fn a_walk_through_rooms_hands_the_tail_over_without_repitching_it() {
    let (out, nets) = hum_through_alternating_rooms(TailLines::Handover);
    let mut handovers = 0;
    let mut moves = 0;
    for (e, pair) in nets.windows(2).enumerate() {
        handovers += usize::from(pair[0][0].fed != pair[1][0].fed);
        for (k, (before, after)) in pair[0].iter().zip(&pair[1]).enumerate() {
            let moved = before
                .lines
                .iter()
                .zip(&after.lines)
                .any(|(a, b)| a.length != b.length);
            if moved {
                moves += 1;
                assert!(
                    !before.holds_signal,
                    "block {e}: network {k}'s lines moved while it held signal"
                );
            }
        }
    }
    let last2 = (2 * RATE) as usize;
    let frac = |out: &[f32]| {
        (0..2)
            .map(|c| energy_near_440(&ear(&out[out.len() - 2 * last2..], c)))
            .sum::<f64>()
            / 2.0
    };
    let (glide, _) = hum_through_alternating_rooms(TailLines::Glide);
    let (h, g) = (frac(&out), frac(&glide));
    println!(
        "rooms alternating 8 m / 10 m every 0.33 s: {handovers} handovers, {moves} length \
         changes, each on a silent network; energy within 3 Hz of 440 Hz over the last 2 s: \
         Handover {h:.4}, Glide {g:.4}"
    );
    assert!(handovers >= 10, "{handovers} handovers");
}

/// Rendering with the tail on, its room changing every block (a handover at nearly every
/// one), allocates nothing.
#[test]
fn the_tail_renders_without_allocating() {
    let mut m = Mixer::new(Hrtf::sadie2_d2(), MPV);
    let hum = Sound::new(
        (0..4800)
            .map(|n| (n as f32 * 0.05).sin())
            .collect::<Vec<_>>(),
    );
    for i in 0..8u32 {
        m.apply(Command::Play(
            VoiceId(i),
            hum.clone(),
            PlayParams {
                looping: true,
                ..PlayParams::at([20.0, i as f32, 0.0])
            },
        ));
    }
    let mut out = vec![0.0; 2 * BLOCK];
    m.apply(Command::SetRoom(RoomTargets {
        t60_s: 2.0,
        mean_free_path_m: 8.0,
        energy: 0.4,
    }));
    m.render(&mut out, BLOCK);
    COUNTING.with(|c| c.set(true));
    for b in 0..32 {
        m.apply(Command::SetRoom(RoomTargets {
            t60_s: 1.0 + b as f32 * 0.2,
            mean_free_path_m: 4.0 + b as f32,
            energy: 0.1 + b as f32 * 0.01,
        }));
        m.render(&mut out, BLOCK);
    }
    COUNTING.with(|c| c.set(false));
    assert_eq!(ALLOCATIONS.with(Cell::get), 0);
    assert!(out.iter().all(|s| s.is_finite()));
}

/// A tail left to ring out in silence settles to exact zeros instead of sinking into
/// subnormal floats, which cost every block far more than normal arithmetic: an impulse in a
/// 0.75 s room, then 30 s of silence, the last 10 s all `0.0` or normal and the last block
/// exactly zero.
#[test]
fn a_silent_tail_rings_out_to_zero_not_subnormals() {
    let mut m = Mixer::new(Hrtf::sadie2_d2(), MPV);
    m.apply(Command::SetRoom(RoomTargets {
        t60_s: 0.75,
        mean_free_path_m: 2.667,
        energy: 1.0,
    }));
    let mut out = vec![0.0; RATE as usize];
    // The input gain fades in before the impulse.
    m.render(&mut out, RATE as usize / 2);
    m.apply(Command::Play(
        VoiceId(1),
        impulse(),
        PlayParams::at([10.0, 0.0, 0.0]),
    ));
    let seconds = 30;
    let mut subnormal = 0usize;
    let mut last = Vec::new();
    for s in 0..seconds {
        m.render(&mut out, RATE as usize / 2);
        let mut second = out.clone();
        m.render(&mut out, RATE as usize / 2);
        second.extend_from_slice(&out);
        if s >= seconds - 10 {
            subnormal += second
                .iter()
                .filter(|x| **x != 0.0 && !x.is_normal())
                .count();
        }
        last = second;
    }
    let tail_block = &last[last.len() - 2 * BLOCK..];
    println!(
        "30 s of silence after an impulse in a 0.75 s room: {subnormal} subnormal samples in \
         the last 10 s; last block peak {:e}",
        tail_block.iter().fold(0.0f32, |p, x| p.max(x.abs()))
    );
    assert_eq!(subnormal, 0, "subnormal samples in the last 10 s");
    assert!(
        tail_block.iter().all(|&x| x == 0.0),
        "the last block is zero"
    );
}
