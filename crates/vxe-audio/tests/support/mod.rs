//! The counting allocator the allocation tests share: it counts the heap allocations and frees
//! a thread makes while [`count`] runs on it.
//!
//! Also the fixtures the integration tests share: the cave material and its worlds, a test
//! tone and its stereo tail level, and the two decibel conversions.
#![allow(dead_code, reason = "shared across test binaries; each uses a subset")]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::f32::consts::PI;

use vxe_audio::{RATE, Sound};
use vxe_world::{
    AIR, BRICK_EDGE, Brick, CHUNK_BRICKS, Chunk, ChunkCoord, ChunkExtent, FnSource, Material,
    MaterialId, MaterialTable, World, WorldSource, brick_of_slot,
};

/// Forwards to `System`, counting on threads inside [`count`].
pub struct Counting;

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
    static FREES: Cell<u64> = const { Cell::new(0) };
}

fn bump(counter: &'static std::thread::LocalKey<Cell<u64>>) {
    if COUNTING.with(Cell::get) {
        counter.with(|c| c.set(c.get() + 1));
    }
}

// SAFETY: forwards every call to `System` unchanged; the thread-locals are `const`-initialised
// `Cell`s, which never allocate.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        bump(&ALLOCATIONS);
        // SAFETY: the caller upholds `GlobalAlloc::alloc`'s contract, passed through.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        bump(&FREES);
        // SAFETY: `ptr` came from `System.alloc` with this layout.
        unsafe { System.dealloc(ptr, layout) }
    }

    /// One allocation: a grown block, moved or not.
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        bump(&ALLOCATIONS);
        // SAFETY: as `alloc` and `dealloc`, passed through.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

/// Allocations and frees counted over one [`count`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub allocations: u64,
    pub frees: u64,
}

impl Counts {
    /// Nothing allocated and nothing freed.
    pub const NONE: Counts = Counts {
        allocations: 0,
        frees: 0,
    };
}

/// Runs `f` on this thread with counting on, and returns its value and what it allocated and
/// freed. Counts made on other threads are not seen.
pub fn count<R>(f: impl FnOnce() -> R) -> (R, Counts) {
    let read = || Counts {
        allocations: ALLOCATIONS.with(Cell::get),
        frees: FREES.with(Cell::get),
    };
    let before = read();
    COUNTING.with(|c| c.set(true));
    let r = f();
    COUNTING.with(|c| c.set(false));
    let after = read();
    (
        r,
        Counts {
            allocations: after.allocations - before.allocations,
            frees: after.frees - before.frees,
        },
    )
}

/// The cave material's id in every fixture table: absorption 0.3 per metre, transmission 0.5.
pub const CAVE: MaterialId = 1;

/// A table of air and [`CAVE`].
pub fn cave_table() -> MaterialTable {
    let mut t = MaterialTable::new();
    let mut m = Material::named("cave");
    m.acoustic = [0.3, 0.5, 0.0];
    assert_eq!(t.push(m), CAVE);
    t
}

/// Chunks `[min, max)`, [`CAVE`] wherever `solid` holds and air elsewhere.
pub fn world(min: [i32; 3], max: [i32; 3], solid: impl Fn([i32; 3]) -> bool + Sync) -> World {
    World::generate(
        ChunkExtent::new(min, max),
        &FnSource(move |v| if solid(v) { CAVE } else { AIR }),
        &cave_table(),
    )
}

/// An air cube of voxels `[0, edge)³` inside a 20-voxel [`CAVE`] shell, air beyond; bricks
/// wholly inside the cube are generated `Uniform`.
pub struct ShellCube {
    pub edge: i32,
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

/// A sine of `freq` Hz at `amplitude`, `len` samples at [`RATE`].
pub fn tone(freq: f32, amplitude: f32, len: usize) -> Sound {
    Sound::new(
        (0..len)
            .map(|n| amplitude * (2.0 * PI * freq * n as f32 / RATE as f32).sin())
            .collect::<Vec<_>>(),
    )
}

/// The RMS of each channel over the last `tail` frames of interleaved stereo `out`.
pub fn tail_rms(out: &[f32], tail: usize) -> [f32; 2] {
    let tail_samples = &out[out.len() - 2 * tail..];
    [0, 1].map(|c| {
        let sum: f64 = tail_samples
            .iter()
            .skip(c)
            .step_by(2)
            .map(|&s| f64::from(s) * f64::from(s))
            .sum();
        (sum / tail as f64).sqrt() as f32
    })
}

/// A power ratio in decibels, `10 log10(x)`, with `x` floored at `1e-30`.
pub fn db_power(x: f64) -> f64 {
    10.0 * x.max(1e-30).log10()
}

/// An amplitude ratio `a / b` in decibels, `20 log10(a / b)`.
pub fn db_amplitude(a: f32, b: f32) -> f32 {
    20.0 * (a / b).log10()
}
