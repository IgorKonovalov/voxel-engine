//! Streaming residency (ADR-0003): chunks resident in spheres around focus points, generated
//! on a worker pool and inserted and evicted on the caller's thread.
//!
//! Distances are integer chunk units. With `d2` the smallest squared distance from a chunk to
//! any focus chunk, a chunk is wanted when `d2 <= r_full^2`, and a resident chunk is evicted
//! once `d2 > (r_full + h)^2`.
//!
//! The caller drives everything: [`Streamer::update`] takes the focus points and, when a
//! focus chunk moved, rebuilds the job queue; [`Streamer::poll`] inserts finished chunks and
//! performs evictions. The queue is ordered by `(d2, coord)`, so the nearest chunks load first
//! and ties fall to coordinate order. A queued job whose chunk stops being wanted is dropped
//! at the next rebuild. Nothing here reads a clock: latency is counted in `update` calls.

use std::collections::{HashMap, HashSet};
use std::panic::AssertUnwindSafe;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, PoisonError, RwLock};
use std::thread::JoinHandle;

use crate::chunk::Chunk;
use crate::coords::{ChunkCoord, VoxelCoord};
use crate::source::WorldSource;
use crate::store::World;

/// Residency radii in chunks, and the hysteresis applied past each.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Residency {
    pub r_full: i32,
    pub r_lod: i32,
    pub hysteresis: i32,
}

impl Residency {
    fn full2(&self) -> i64 {
        i64::from(self.r_full).pow(2)
    }

    fn keep_full2(&self) -> i64 {
        i64::from(self.r_full + self.hysteresis).pow(2)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamConfig {
    pub residency: Residency,
    /// Worker threads; at least one is started.
    pub workers: usize,
}

/// The tier a chunk is resident in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Tier {
    /// Every voxel, in the world's store: edits, queries and raycasts see it.
    Full,
}

/// What one `poll` (or `wait_idle`) changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StreamEvents {
    /// Chunks inserted, with their tier, in insertion order.
    pub loaded: Vec<(ChunkCoord, Tier)>,
    /// Chunks removed from residency.
    pub evicted: Vec<ChunkCoord>,
    /// For each chunk in `loaded` as `Full`, the `update` calls between the one that first
    /// wanted it and its insertion.
    pub latency_ticks: Vec<u32>,
}

impl StreamEvents {
    fn append(&mut self, mut other: StreamEvents) {
        self.loaded.append(&mut other.loaded);
        self.evicted.append(&mut other.evicted);
        self.latency_ticks.append(&mut other.latency_ticks);
    }

    pub fn is_empty(&self) -> bool {
        self.loaded.is_empty() && self.evicted.is_empty()
    }
}

/// Counters for callers that measure the streamer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StreamStats {
    /// `update` calls so far.
    pub ticks: u64,
    pub resident_full: usize,
    /// Jobs queued and not yet taken by a worker.
    pub queued: usize,
    /// Jobs queued or running.
    pub in_flight: usize,
    /// Chunks generated from the source over the streamer's life.
    pub generated_chunks: u64,
}

/// A job's queue key: smaller runs first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    d2: i64,
    coord: ChunkCoord,
}

struct Job {
    key: Key,
}

enum Done {
    Loaded {
        coord: ChunkCoord,
        chunk: Box<Chunk>,
    },
    Panicked(Box<dyn std::any::Any + Send>),
}

#[derive(Default)]
struct Queue {
    /// Sorted descending by key, so `pop` takes the nearest.
    jobs: Vec<Job>,
    shutdown: bool,
}

struct Shared {
    queue: Mutex<Queue>,
    wake: Condvar,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Keeps a [`World`] resident around focus points; see the module docs.
pub struct Streamer {
    cfg: StreamConfig,
    shared: Arc<Shared>,
    workers: Vec<JoinHandle<()>>,
    results: Receiver<Done>,
    focus: Vec<ChunkCoord>,
    /// Set when the focus chunks change; the next `poll` sweeps residency for evictions.
    sweep: bool,
    resident: HashMap<ChunkCoord, Tier>,
    /// Chunks with a job queued or running.
    in_flight: HashSet<ChunkCoord>,
    /// The tick at which each chunk with a job was first wanted.
    wanted_since: HashMap<ChunkCoord, u64>,
    tick: u64,
    generated: u64,
}

impl Streamer {
    /// Starts `cfg.workers` worker threads generating from `source`.
    pub fn new<S: WorldSource + Send + Sync + 'static>(source: Arc<S>, cfg: StreamConfig) -> Self {
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue::default()),
            wake: Condvar::new(),
        });
        let (tx, results) = mpsc::channel();
        let workers = (0..cfg.workers.max(1))
            .map(|_| {
                let shared = shared.clone();
                let source = source.clone();
                let tx = tx.clone();
                std::thread::spawn(move || worker(&shared, &*source, &tx))
            })
            .collect();
        Self {
            cfg,
            shared,
            workers,
            results,
            focus: Vec::new(),
            sweep: false,
            resident: HashMap::new(),
            in_flight: HashSet::new(),
            wanted_since: HashMap::new(),
            tick: 0,
            generated: 0,
        }
    }

    pub fn config(&self) -> &StreamConfig {
        &self.cfg
    }

    /// Takes this tick's focus points. When the set of focus chunks differs from the last
    /// call's, the queue is rebuilt from the chunks now wanted and not resident, and the next
    /// `poll` sweeps for evictions.
    pub fn update(&mut self, _world: &World, focus: &[VoxelCoord]) {
        self.tick += 1;
        let mut chunks: Vec<ChunkCoord> = focus.iter().map(|v| v.chunk()).collect();
        chunks.sort_unstable();
        chunks.dedup();
        if chunks == self.focus {
            return;
        }
        self.focus = chunks;
        self.sweep = true;
        self.rebuild_queue();
    }

    /// Smallest squared chunk distance from `c` to a focus chunk; `i64::MAX` with no focus.
    fn d2(&self, c: ChunkCoord) -> i64 {
        self.focus
            .iter()
            .map(|f| d2(c, *f))
            .min()
            .unwrap_or(i64::MAX)
    }

    /// Every chunk wanted at the current focus with its queue key, ascending.
    fn wanted(&self) -> Vec<Key> {
        let r = self.cfg.residency.r_full;
        let r2 = self.cfg.residency.full2();
        let mut keys = Vec::new();
        let mut seen = HashSet::new();
        for f in &self.focus {
            for dx in -r..=r {
                for dy in -r..=r {
                    for dz in -r..=r {
                        let o = [dx, dy, dz].map(i64::from);
                        if o.iter().map(|v| v * v).sum::<i64>() > r2 {
                            continue;
                        }
                        let coord = ChunkCoord([f.0[0] + dx, f.0[1] + dy, f.0[2] + dz]);
                        if self.focus.len() > 1 && !seen.insert(coord) {
                            continue;
                        }
                        keys.push(Key {
                            d2: self.d2(coord),
                            coord,
                        });
                    }
                }
            }
        }
        keys.sort_unstable();
        keys
    }

    fn rebuild_queue(&mut self) {
        let wanted = self.wanted();
        let mut q = lock(&self.shared.queue);
        // Jobs still queued are not running: forget them, then queue what is wanted now.
        for job in q.jobs.drain(..) {
            self.in_flight.remove(&job.key.coord);
        }
        let mut jobs = Vec::new();
        let mut still_wanted = HashSet::new();
        for key in wanted {
            still_wanted.insert(key.coord);
            if self.resident.contains_key(&key.coord) || self.in_flight.contains(&key.coord) {
                continue;
            }
            self.wanted_since.entry(key.coord).or_insert(self.tick);
            self.in_flight.insert(key.coord);
            jobs.push(Job { key });
        }
        self.wanted_since
            .retain(|c, _| still_wanted.contains(c) || self.in_flight.contains(c));
        jobs.reverse();
        q.jobs = jobs;
        drop(q);
        self.shared.wake.notify_all();
    }

    /// Inserts every finished chunk and performs evictions. Never blocks on a worker.
    pub fn poll(&mut self, world: &World) -> StreamEvents {
        let mut done = Vec::new();
        while let Ok(d) = self.results.try_recv() {
            done.push(d);
        }
        self.apply(world, done)
    }

    /// Blocks until no job is queued or running and every result is applied; returns what
    /// changed meanwhile. For tests and startup.
    pub fn wait_idle(&mut self, world: &World) -> StreamEvents {
        let mut events = self.poll(world);
        while !self.in_flight.is_empty() {
            let Ok(d) = self.results.recv() else {
                break;
            };
            let mut done = vec![d];
            while let Ok(d) = self.results.try_recv() {
                done.push(d);
            }
            events.append(self.apply(world, done));
        }
        events
    }

    fn apply(&mut self, world: &World, done: Vec<Done>) -> StreamEvents {
        let mut ev = StreamEvents::default();
        let keep2 = self.cfg.residency.keep_full2();
        let mut inserts = Vec::new();
        for d in done {
            match d {
                Done::Loaded { coord, chunk } => {
                    self.in_flight.remove(&coord);
                    self.generated += 1;
                    let since = self.wanted_since.remove(&coord);
                    // A chunk that left the keep radius while it was generated is dropped.
                    if self.d2(coord) > keep2 {
                        continue;
                    }
                    inserts.push((coord, *chunk));
                    ev.loaded.push((coord, Tier::Full));
                    let since = since.unwrap_or(self.tick);
                    ev.latency_ticks
                        .push(u32::try_from(self.tick - since).unwrap_or(u32::MAX));
                }
                Done::Panicked(e) => std::panic::resume_unwind(e),
            }
        }
        let mut evict = Vec::new();
        if self.sweep {
            self.sweep = false;
            evict = self
                .resident
                .keys()
                .copied()
                .filter(|&c| self.d2(c) > keep2)
                .collect();
            evict.sort_unstable();
        }
        if !inserts.is_empty() || !evict.is_empty() {
            let mut map = world.store.write_lock();
            for (coord, chunk) in inserts {
                map.insert(coord, Arc::new(RwLock::new(chunk)));
                self.resident.insert(coord, Tier::Full);
            }
            for &coord in &evict {
                map.remove(coord);
                self.resident.remove(&coord);
            }
        }
        ev.evicted = evict;
        ev
    }

    pub fn stats(&self) -> StreamStats {
        StreamStats {
            ticks: self.tick,
            resident_full: self.resident.len(),
            queued: lock(&self.shared.queue).jobs.len(),
            in_flight: self.in_flight.len(),
            generated_chunks: self.generated,
        }
    }

    /// The tier `coord` is resident in, if any.
    pub fn tier(&self, coord: ChunkCoord) -> Option<Tier> {
        self.resident.get(&coord).copied()
    }
}

impl Drop for Streamer {
    fn drop(&mut self) {
        lock(&self.shared.queue).shutdown = true;
        self.shared.wake.notify_all();
        for w in self.workers.drain(..) {
            let _ = w.join();
        }
    }
}

fn d2(a: ChunkCoord, b: ChunkCoord) -> i64 {
    (0..3)
        .map(|k| (i64::from(a.0[k]) - i64::from(b.0[k])).pow(2))
        .sum()
}

fn worker<S: WorldSource + ?Sized>(shared: &Shared, source: &S, tx: &Sender<Done>) {
    loop {
        let job = {
            let mut q = lock(&shared.queue);
            loop {
                if q.shutdown {
                    return;
                }
                if let Some(job) = q.jobs.pop() {
                    break job;
                }
                q = shared.wake.wait(q).unwrap_or_else(PoisonError::into_inner);
            }
        };
        let coord = job.key.coord;
        let r = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let mut chunk = Box::<Chunk>::default();
            source.chunk(coord, &mut chunk);
            chunk
        }));
        let done = match r {
            Ok(chunk) => Done::Loaded { coord, chunk },
            Err(e) => Done::Panicked(e),
        };
        if tx.send(done).is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::UniformSource;

    fn streamer(r: i32) -> Streamer {
        Streamer::new(
            Arc::new(UniformSource(3)),
            StreamConfig {
                residency: Residency {
                    r_full: r,
                    r_lod: r,
                    hysteresis: 1,
                },
                workers: 1,
            },
        )
    }

    #[test]
    fn queue_runs_nearest_first_with_ties_by_coordinate() {
        let mut s = streamer(2);
        s.focus = vec![ChunkCoord([0, 0, 0])];
        let order: Vec<[i32; 3]> = s.wanted().iter().map(|k| k.coord.0).collect();
        assert_eq!(order.len(), 33);
        assert_eq!(order[0], [0, 0, 0]);
        assert_eq!(
            order[1..7],
            [
                [-1, 0, 0],
                [0, -1, 0],
                [0, 0, -1],
                [0, 0, 1],
                [0, 1, 0],
                [1, 0, 0]
            ]
        );
        let ring2: Vec<[i32; 3]> = order[7..19].to_vec();
        let mut expect: Vec<[i32; 3]> = Vec::new();
        for x in -1..=1 {
            for y in -1..=1 {
                for z in -1..=1i32 {
                    if x * x + y * y + z * z == 2 {
                        expect.push([x, y, z]);
                    }
                }
            }
        }
        expect.sort_unstable();
        assert_eq!(ring2, expect);

        // The queue the workers pop from holds the same order. The worker is stopped first so
        // it cannot take a job before the queue is read.
        lock(&s.shared.queue).shutdown = true;
        s.shared.wake.notify_all();
        for w in s.workers.drain(..) {
            w.join().unwrap();
        }
        s.rebuild_queue();
        let q = lock(&s.shared.queue);
        let popped: Vec<[i32; 3]> = q.jobs.iter().rev().map(|j| j.key.coord.0).collect();
        assert_eq!(popped, order);
    }
}
