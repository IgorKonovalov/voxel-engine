//! A streamed voice: a writer thread feeding the ring while the mixer renders, an underrun,
//! a voice heard only off the walls, and no allocation while a stream plays.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

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
const W: MaterialId = 1;
const PIECE: usize = 480;
const POSITION: [f32; 3] = [20.0, 5.0, 0.0];

fn tone(len: usize) -> Vec<f32> {
    (0..len)
        .map(|n| 0.5 * (std::f32::consts::TAU * 440.0 * n as f32 / RATE as f32).sin())
        .collect()
}

/// `frames` frames of `samples` played as a stream at `POSITION`, fed by a writer thread in
/// 480-frame pieces. Before each block the mixer waits until the writer has pushed the frames it
/// needs, except over `gap` (frames from the start; it begins on a block edge), where the
/// writer holds off until the mixer has rendered past it: an underrun of that length.
fn streamed(samples: Vec<f32>, frames: usize, gap: Option<std::ops::Range<usize>>) -> Vec<f32> {
    let (mut writer, reader) = Stream::new();
    let pushed = Arc::new(AtomicUsize::new(0));
    let resume = Arc::new(AtomicBool::new(false));
    let feeder = {
        let (pushed, resume, gap) = (Arc::clone(&pushed), Arc::clone(&resume), gap.clone());
        std::thread::spawn(move || {
            let mut at = 0;
            while at < samples.len() {
                if let Some(g) = &gap
                    && at == g.start
                {
                    while !resume.load(Ordering::Acquire) {
                        std::thread::yield_now();
                    }
                    at = g.end;
                    continue;
                }
                let end = (at + PIECE).min(samples.len());
                let end = gap
                    .as_ref()
                    .map_or(end, |g| if at < g.start { end.min(g.start) } else { end });
                while writer.space() < end - at {
                    std::thread::yield_now();
                }
                assert_eq!(writer.push(&samples[at..end]), 0);
                at = end;
                pushed.store(at, Ordering::Release);
            }
        })
    };
    let mut mixer = Mixer::new(Hrtf::sadie2_d2(), MPV);
    mixer.apply(Command::PlayStream(
        VoiceId(1),
        reader,
        PlayParams::at(POSITION),
    ));
    let mut out = vec![0.0; 2 * frames];
    for (b, chunk) in out.chunks_mut(2 * BLOCK).enumerate() {
        let (start, end) = (b * BLOCK, ((b + 1) * BLOCK).min(frames));
        let in_gap = gap.as_ref().is_some_and(|g| start < g.end && end > g.start);
        if gap.as_ref().is_some_and(|g| start >= g.end) {
            resume.store(true, Ordering::Release);
        }
        if !in_gap {
            while pushed.load(Ordering::Acquire) < end {
                std::thread::yield_now();
            }
        }
        mixer.render(chunk, BLOCK);
    }
    resume.store(true, Ordering::Release);
    feeder.join().expect("the writer thread");
    out
}

/// The same samples played from a `Sound`.
fn from_sound(samples: Vec<f32>, frames: usize) -> Vec<f32> {
    let mut mixer = Mixer::new(Hrtf::sadie2_d2(), MPV);
    mixer.apply(Command::Play(
        VoiceId(1),
        Sound::new(samples),
        PlayParams::at(POSITION),
    ));
    let mut out = vec![0.0; 2 * frames];
    mixer.render(&mut out, frames);
    out
}

#[test]
fn a_stream_fed_by_a_thread_plays_as_its_sound_does() {
    let frames = RATE as usize;
    let out = streamed(tone(frames), frames, None);
    let reference = from_sound(tone(frames), frames);
    let worst = out[2 * BLOCK..]
        .iter()
        .zip(&reference[2 * BLOCK..])
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    println!(
        "streamed against played from a Sound, from the second block: largest difference {worst:e}"
    );
    assert!(worst <= 1e-6, "differs by {worst}");
    assert!(reference.iter().any(|&s| s != 0.0));
}

fn block_rms(ch: &[f32]) -> Vec<f64> {
    ch.chunks_exact(BLOCK)
        .map(|b| (b.iter().map(|&x| f64::from(x).powi(2)).sum::<f64>() / BLOCK as f64).sqrt())
        .collect()
}

#[test]
fn a_writer_that_pauses_leaves_a_gap_of_silence() {
    let frames = RATE as usize;
    // 50 ms starting on a block edge, 0.5 s in.
    let gap = 24_064..24_064 + 2_400;
    let out = streamed(tone(frames), frames, Some(gap.clone()));
    let taps = Hrtf::sadie2_d2().taps();
    // Silence once the last frame before the gap has left the HRIR, until the next block the
    // writer resumed in.
    let silent_from = gap.start + taps;
    let resumed = gap.end.div_ceil(BLOCK) * BLOCK;
    assert!(
        out[2 * silent_from..2 * resumed].iter().all(|&s| s == 0.0),
        "the gap is silent"
    );
    assert!(
        out[2 * resumed..2 * (resumed + BLOCK)]
            .iter()
            .any(|&s| s != 0.0),
        "it resumes"
    );
    let left: Vec<f32> = out.iter().step_by(2).copied().collect();
    let rms = block_rms(&left);
    let (s0, r0) = (gap.start / BLOCK, resumed / BLOCK);
    let step = |a: f64, b: f64| {
        if a > 0.0 && b > 0.0 {
            format!("{:+.1} dB", 20.0 * (b / a).log10())
        } else {
            "to or from silence".to_owned()
        }
    };
    println!(
        "underrun of 2400 frames, left ear block RMS (reported, not bounded): into the gap \
         {:.5} -> {:.5} -> {:.5} ({}), out of it {:.5} -> {:.5} ({})",
        rms[s0 - 1],
        rms[s0],
        rms[s0 + 1],
        step(rms[s0], rms[s0 + 1]),
        rms[r0 - 1],
        rms[r0],
        step(rms[r0 - 1], rms[r0])
    );
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

fn world_around(solid: impl Fn([i32; 3]) -> bool + Sync) -> World {
    let mut t = MaterialTable::new();
    let mut w = Material::named("wall");
    w.acoustic = [0.3, 0.5, 0.0];
    assert_eq!(t.push(w), W);
    World::generate(
        ChunkExtent::new([-2, -2, -2], [2, 2, 2]),
        &FnSource(move |v| if solid(v) { W } else { AIR }),
        &t,
    )
}

/// A 100 ms 2 kHz burst pushed into a stream played at the listener with `direct: false`,
/// probed once through `world` (no room fan completes): the output and the first echo's
/// delay.
fn undirected_burst(world: &World) -> (Vec<f32>, usize) {
    let (mut writer, reader) = Stream::new();
    let burst: Vec<f32> = (0..4_800)
        .map(|n| 0.5 * (std::f32::consts::TAU * 2000.0 * n as f32 / RATE as f32).sin())
        .collect();
    assert_eq!(writer.push(&burst), 0);
    let mut mixer = Mixer::new(Hrtf::sadie2_d2(), MPV);
    let listener = Listener::default();
    mixer.apply(Command::SetListener(listener));
    let at = listener.position;
    mixer.apply(Command::PlayStream(
        VoiceId(2),
        reader,
        PlayParams {
            direct: false,
            ..PlayParams::at(at)
        },
    ));
    let mut acoustics = Acoustics::new(mixer.clock());
    let mut first = usize::MAX;
    acoustics.update(
        world,
        world.materials(),
        &listener,
        &[(VoiceId(2), at)],
        MPV,
        &mut |cmd| {
            if let Command::SetAcoustics(_, t) = &cmd {
                first = t
                    .taps
                    .iter()
                    .filter(|t| t.gain != 0.0)
                    .map(|t| t.delay_frames as usize)
                    .min()
                    .unwrap_or(usize::MAX);
            }
            mixer.apply(cmd);
        },
    );
    let frames = 14_400;
    let mut out = vec![0.0; 2 * frames];
    mixer.render(&mut out, frames);
    (out, first)
}

#[test]
fn a_stream_without_its_direct_path_is_heard_only_off_the_walls() {
    let (open, first) = undirected_burst(&world_around(|_| false));
    assert_eq!(first, usize::MAX, "no wall, no tap");
    assert!(open.iter().all(|&s| s == 0.0), "all air renders silence");
    let (walled, first) = undirected_burst(&world_around(|v| v[1] >= 50));
    let before = &walled[..2 * first];
    let energy: f64 = walled[2 * first..]
        .iter()
        .map(|&s| f64::from(s).powi(2))
        .sum();
    println!("left wall: first echo at frame {first}, energy after it {energy:.4}");
    assert!(
        before.iter().all(|&s| s == 0.0),
        "nothing before the first echo"
    );
    assert!(energy > 0.0);
}

#[test]
fn a_stream_renders_without_allocating() {
    let walled = world_around(|v| v[1] >= 50);
    let (mut writer, reader) = Stream::new();
    let mut mixer = Mixer::new(Hrtf::sadie2_d2(), MPV);
    let listener = Listener::default();
    mixer.apply(Command::PlayStream(
        VoiceId(3),
        reader,
        PlayParams {
            direct: false,
            ..PlayParams::at([0.0; 3])
        },
    ));
    let mut acoustics = Acoustics::new(mixer.clock());
    for _ in 0..ROOM_RAYS / ROOM_RAYS_PER_UPDATE {
        acoustics.update(
            &walled,
            walled.materials(),
            &listener,
            &[(VoiceId(3), [0.0; 3])],
            MPV,
            &mut |cmd| mixer.apply(cmd),
        );
    }
    let piece = tone(BLOCK);
    let mut out = vec![0.0; 2 * BLOCK];
    COUNTING.with(|c| c.set(true));
    for _ in 0..64 {
        writer.push(&piece);
        mixer.render(&mut out, BLOCK);
    }
    COUNTING.with(|c| c.set(false));
    assert_eq!(ALLOCATIONS.with(Cell::get), 0);
    assert!(out.iter().any(|&s| s != 0.0));
}
