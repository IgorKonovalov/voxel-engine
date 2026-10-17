//! The component's WASI: everything it can observe follows from its seed and the tick.
//!
//! - **Clocks.** The monotonic and the wall clock both read `tick × frame_ns` nanoseconds
//!   (the wall clock as a duration since the epoch), with `frame_ns` as their resolution.
//! - **Randomness.** `wasi:random/random` and `wasi:random/insecure` serve one byte stream,
//!   restarted before every call from `mix64(seed ^ tick)`: xorshift64 (13, 7, 17) steps,
//!   each output's eight little-endian bytes in turn. `wasi:random/insecure-seed` (which the
//!   standard library's `HashMap` keys come from) is the first sixteen bytes of the stream
//!   for tick 0, fixed when the component is instantiated.
//! - **Nothing else.** No preopened directories, no environment variables, no arguments,
//!   stdin closed, stdout discarded, stderr kept for a trap's message, no network.

use std::convert::Infallible;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use rand_core::TryRng;
use vxe_world::hash::mix64;
use wasmtime_wasi::p2::pipe::MemoryOutputPipe;
use wasmtime_wasi::{HostMonotonicClock, HostWallClock, WasiCtx};

/// The xorshift state a call's stream starts from: `mix64(seed ^ tick)`, or the golden-ratio
/// constant where that is zero (xorshift sticks at zero).
pub(crate) fn stream_state(seed: u64, tick: u64) -> u64 {
    match mix64(seed ^ tick) {
        0 => 0x9e37_79b9_7f4a_7c15,
        s => s,
    }
}

/// The clocks' reading, in nanoseconds, shared between the store and the host.
#[derive(Clone, Default)]
pub(crate) struct TickClock(Arc<AtomicU64>);

impl TickClock {
    pub(crate) fn set(&self, ns: u64) {
        self.0.store(ns, Ordering::Relaxed);
    }

    fn now(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

struct Monotonic {
    clock: TickClock,
    resolution: u64,
}

impl HostMonotonicClock for Monotonic {
    fn resolution(&self) -> u64 {
        self.resolution
    }

    fn now(&self) -> u64 {
        self.clock.now()
    }
}

struct Wall {
    clock: TickClock,
    resolution: u64,
}

impl HostWallClock for Wall {
    fn resolution(&self) -> Duration {
        Duration::from_nanos(self.resolution)
    }

    fn now(&self) -> Duration {
        Duration::from_nanos(self.clock.now())
    }
}

/// One xorshift64 byte stream.
struct Stream {
    state: u64,
    /// The current output's bytes and how many of them are spent.
    bytes: [u8; 8],
    used: usize,
}

impl Stream {
    fn new(state: u64) -> Self {
        Self {
            state,
            bytes: [0; 8],
            used: 8,
        }
    }

    fn next_byte(&mut self) -> u8 {
        if self.used == 8 {
            let mut x = self.state;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.state = x;
            self.bytes = x.to_le_bytes();
            self.used = 0;
        }
        self.used += 1;
        self.bytes[self.used - 1]
    }
}

/// The stream every random interface serves, restarted by the host before each call.
#[derive(Clone)]
pub(crate) struct TickRandom(Arc<Mutex<Stream>>);

impl TickRandom {
    fn new(state: u64) -> Self {
        Self(Arc::new(Mutex::new(Stream::new(state))))
    }

    pub(crate) fn restart(&self, state: u64) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Stream::new(state);
    }

    fn fill(&self, dst: &mut [u8]) {
        let mut s = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        for b in dst {
            *b = s.next_byte();
        }
    }
}

impl TryRng for TickRandom {
    type Error = Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Infallible> {
        let mut b = [0; 4];
        self.fill(&mut b);
        Ok(u32::from_le_bytes(b))
    }

    fn try_next_u64(&mut self) -> Result<u64, Infallible> {
        let mut b = [0; 8];
        self.fill(&mut b);
        Ok(u64::from_le_bytes(b))
    }

    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Infallible> {
        self.fill(dst);
        Ok(())
    }
}

/// What the host keeps to steer and read the component's WASI.
pub(crate) struct WasiHandles {
    pub clock: TickClock,
    pub random: TickRandom,
    pub stderr: MemoryOutputPipe,
}

/// Most stderr bytes kept, for a trap's message.
const STDERR_BYTES: usize = 64 * 1024;

/// A context for a component initialised with `seed`, its clocks at tick 0's reading.
pub(crate) fn context(seed: u64, frame_ns: u64) -> (WasiCtx, WasiHandles) {
    let clock = TickClock::default();
    let random = TickRandom::new(stream_state(seed, 0));
    let mut insecure_seed = [0; 16];
    random.fill(&mut insecure_seed);
    random.restart(stream_state(seed, 0));
    let stderr = MemoryOutputPipe::new(STDERR_BYTES);
    let ctx = WasiCtx::builder()
        .monotonic_clock(Monotonic {
            clock: clock.clone(),
            resolution: frame_ns,
        })
        .wall_clock(Wall {
            clock: clock.clone(),
            resolution: frame_ns,
        })
        .secure_random(random.clone())
        .insecure_random(random.clone())
        .insecure_random_seed(u128::from_le_bytes(insecure_seed))
        .stderr(stderr.clone())
        .allow_tcp(false)
        .allow_udp(false)
        .allow_ip_name_lookup(false)
        .build();
    (
        ctx,
        WasiHandles {
            clock,
            random,
            stderr,
        },
    )
}
