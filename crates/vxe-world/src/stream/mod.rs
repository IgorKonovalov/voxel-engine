//! Streaming residency (ADR-0003): chunks resident around focus points in two tiers,
//! generated on a worker pool and inserted, demoted and evicted on the caller's thread.
//!
//! Distances are integer chunk units. With `d2` the smallest squared distance from a chunk to
//! any focus chunk, a chunk is wanted *full* when `d2 <= r_full^2` and wanted *LOD-only*
//! when it is not full and `d2 <= r_lod^2`. A full chunk is demoted to LOD-only once
//! `d2 > (r_full + h)^2`, and a resident chunk of either tier is evicted once
//! `d2 > (r_lod + h)^2`. A full chunk is a store chunk; a LOD-only chunk keeps only its
//! mips ([`ChunkMips`]), which the world holds beside the store.
//!
//! The caller drives everything: [`Streamer::update`] takes the focus points and, when a
//! focus chunk moved, rebuilds the job queue; [`Streamer::poll`] inserts finished chunks and
//! performs demotions and evictions. When a single focus moves one chunk step (each axis by
//! at most one), the queue and the sweep are updated from the shells the step moves across
//! instead of rebuilt; the result is the rebuild's, job for job. The queue is ordered by tier (full loads and promotions
//! first), then `d2`, then coordinate, so the nearest chunks load first and ties fall to
//! coordinate order. A queued job whose chunk stops being wanted is dropped at the next
//! rebuild. Nothing here reads a clock: latency is counted in `update` calls.
//!
//! A LOD-only chunk is built from a full chunk: generated (and dropped once its mips are
//! built), read from persistence (which keeps it), or demoted from residency, in which case a
//! worker builds its mips after the chunk has left the store. Promotion loads the full chunk
//! again, from persistence or the source.
//!
//! An evicted or demoted chunk that was ever edited goes to the persistence map
//! (`persist.rs`) and a full load of its coordinate takes it back from there, from RAM or
//! from its spill file, instead of generating it. A spill file that fails its checks is
//! reported and its chunk stays non-resident for the streamer's life: an edit is never
//! silently regenerated away.

mod residency;
mod workers;

use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Condvar, Mutex, PoisonError, RwLock};
use std::thread::JoinHandle;

use crate::chunk::Chunk;
use crate::coords::{ChunkCoord, VoxelCoord};
#[cfg(doc)]
use crate::mips::ChunkMips;
use crate::persist::{PersistConfig, Persisted, SpillError};
use crate::source::WorldSource;
use crate::store::{World, read};
use residency::{Job, Key, Refocus, ResidencyPlan};
pub use workers::{ChunkPrep, Prepared};
use workers::{DemoteJob, Done, Queue, Shared, SpillJob, lock, worker};

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

    fn lod2(&self) -> i64 {
        i64::from(self.r_lod.max(self.r_full)).pow(2)
    }

    fn keep_full2(&self) -> i64 {
        i64::from(self.r_full + self.hysteresis).pow(2)
    }

    fn keep_lod2(&self) -> i64 {
        i64::from(self.r_lod.max(self.r_full) + self.hysteresis).pow(2)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamConfig {
    pub residency: Residency,
    /// Worker threads; at least one is started.
    pub workers: usize,
    pub persist: PersistConfig,
}

/// The tier a chunk is resident in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Tier {
    /// Every voxel, in the world's store: edits, queries and raycasts see it.
    Full,
    /// Only its mips, in [`World::lod`]: drawn by the mirror, invisible to edits and queries.
    LodOnly,
}

/// A failure the streamer reports instead of acting on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StreamError {
    /// Writing or reading `coord`'s spill file failed. On a failed write the chunk stays in
    /// RAM; on a failed read it stays non-resident and the file is left in place.
    Spill { coord: ChunkCoord, kind: SpillError },
}

/// What one `poll` (or `wait_idle`) changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StreamEvents {
    /// Chunks inserted, with their tier, in insertion order. A promotion is a `Full` load of
    /// a LOD-only chunk.
    pub loaded: Vec<(ChunkCoord, Tier)>,
    /// Full chunks that became LOD-only.
    pub demoted: Vec<ChunkCoord>,
    /// Chunks removed from residency.
    pub evicted: Vec<ChunkCoord>,
    pub errors: Vec<StreamError>,
    /// For each chunk in `loaded` as `Full`, the `update` calls between the one that first
    /// wanted it full and its insertion.
    pub latency_ticks: Vec<u32>,
    /// The hook's results ([`Streamer::set_prep`]) for chunks in `loaded` as `Full`, in the
    /// same order; a chunk loaded before the hook was set has none.
    pub prepared: Vec<(ChunkCoord, Prepared)>,
}

impl StreamEvents {
    fn append(&mut self, mut other: StreamEvents) {
        self.loaded.append(&mut other.loaded);
        self.demoted.append(&mut other.demoted);
        self.evicted.append(&mut other.evicted);
        self.errors.append(&mut other.errors);
        self.latency_ticks.append(&mut other.latency_ticks);
        self.prepared.append(&mut other.prepared);
    }

    pub fn is_empty(&self) -> bool {
        self.loaded.is_empty()
            && self.demoted.is_empty()
            && self.evicted.is_empty()
            && self.errors.is_empty()
    }
}

/// Counters for callers that measure the streamer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StreamStats {
    /// `update` calls so far.
    pub ticks: u64,
    pub resident_full: usize,
    pub resident_lod: usize,
    /// Jobs queued and not yet taken by a worker.
    pub queued: usize,
    /// Load and demotion jobs queued or running.
    pub in_flight: usize,
    /// Chunks generated from the source over the streamer's life, for either tier.
    pub generated_chunks: u64,
    /// Chunks taken back from persistence over the streamer's life.
    pub restored_chunks: u64,
    /// Persisted chunks held in RAM, and their bytes.
    pub persisted_ram_chunks: usize,
    pub persisted_ram_bytes: u64,
    /// Persisted chunks held in spill files.
    pub spilled_chunks: usize,
    /// Persisted RAM exceeds `PersistConfig::ram_cap_bytes`.
    pub persist_over_cap: bool,
}

/// Keeps a [`World`] resident around focus points; see the module docs.
pub struct Streamer {
    cfg: StreamConfig,
    shared: Arc<Shared>,
    workers: Vec<JoinHandle<()>>,
    results: Receiver<Done>,
    /// The focus, residency, in-flight and sweep state the queue and sweep decisions read.
    plan: ResidencyPlan,
    /// Chunks with a spill write queued or running.
    spilling: HashSet<ChunkCoord>,
    /// Chunks whose spill write or read failed: not spilled, or not loaded, again.
    spill_failed: HashSet<ChunkCoord>,
    generated: u64,
    restored: u64,
}

/// A chunk and its coordinate, held outside the store: loaded, or leaving it.
type Held = (ChunkCoord, Arc<RwLock<Chunk>>);

/// What one batch of finished jobs leaves for the store and the queue: full chunks to insert,
/// those of them inserted outside the full band, and chunks to queue again if still wanted.
struct Absorbed {
    inserts: Vec<Held>,
    outside: Vec<ChunkCoord>,
    requeue: Vec<ChunkCoord>,
}

impl Streamer {
    /// Starts `cfg.workers` worker threads generating from `source`.
    pub fn new<S: WorldSource + Send + Sync + 'static>(source: Arc<S>, cfg: StreamConfig) -> Self {
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue::default()),
            wake: Condvar::new(),
            persisted: Mutex::new(Persisted::default()),
            spill_dir: cfg.persist.spill_dir.clone(),
            world_key: cfg.persist.world_key,
            prep: RwLock::new(None),
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
            plan: ResidencyPlan::new(cfg.residency),
            cfg,
            shared,
            workers,
            results,
            spilling: HashSet::new(),
            spill_failed: HashSet::new(),
            generated: 0,
            restored: 0,
        }
    }

    pub fn config(&self) -> &StreamConfig {
        &self.cfg
    }

    /// Sets the hook the workers run on each full chunk they load from now on (ADR-0017), or
    /// none. Its results arrive in [`StreamEvents::prepared`].
    pub fn set_prep(&mut self, prep: Option<Arc<dyn ChunkPrep>>) {
        *self
            .shared
            .prep
            .write()
            .unwrap_or_else(PoisonError::into_inner) = prep;
    }

    /// Changes the full and LOD radii, keeping the hysteresis. The queue is rebuilt for the
    /// current focus and the next `poll` sweeps residency: full chunks now past
    /// `r_full + h` are demoted, chunks past `r_lod + h` evicted, and chunks now wanted at a
    /// tier they lack are loaded, an edited one from persistence. A call with the current
    /// radii changes nothing.
    pub fn set_radii(&mut self, r_full: i32, r_lod: i32) {
        let r = &mut self.cfg.residency;
        if (r.r_full, r.r_lod) == (r_full, r_lod) {
            return;
        }
        r.r_full = r_full;
        r.r_lod = r_lod;
        self.plan.set_residency(*r);
        self.rebuild_queue();
    }

    /// Test support: when on, every incremental queue update and sweep is recomputed the full
    /// way from the same state, and a difference panics. [`Streamer::oracle_checks`] counts
    /// the checks. Feature `stream-oracle`.
    #[cfg(any(test, feature = "stream-oracle"))]
    #[doc(hidden)]
    pub fn set_incremental_oracle(&mut self, on: bool) {
        self.plan.oracle = on;
    }

    /// Test support: the incremental queue updates and sweeps checked so far. Feature
    /// `stream-oracle`.
    #[cfg(any(test, feature = "stream-oracle"))]
    #[doc(hidden)]
    pub fn oracle_checks(&self) -> (u64, u64) {
        self.plan.oracle_checks
    }

    /// Takes this tick's focus points. When the set of focus chunks differs from the last
    /// call's, the queue is rebuilt from the chunks now wanted and not resident at their
    /// tier, and the next `poll` sweeps for demotions and evictions.
    pub fn update(&mut self, _world: &World, focus: &[VoxelCoord]) {
        match self.plan.refocus(focus) {
            Refocus::Same => {}
            Refocus::Step(d) => self.shift_queue(d),
            Refocus::Jump => self.rebuild_queue(),
        }
    }

    /// The queue after a one-step move of the single focus by `d`, under the queue's lock.
    fn shift_queue(&mut self, d: [i32; 3]) {
        let mut q = lock(&self.shared.queue);
        self.plan.shift_queue(&mut q.jobs, d);
        drop(q);
        self.shared.wake.notify_all();
    }

    fn rebuild_queue(&mut self) {
        let mut q = lock(&self.shared.queue);
        self.plan.rebuild_jobs(&mut q.jobs);
        drop(q);
        self.shared.wake.notify_all();
    }

    /// Queues a load for `c` if the current focus wants it at a tier it lacks; for chunks
    /// whose job finished without leaving them at their wanted tier.
    fn requeue(&mut self, c: ChunkCoord) {
        let Some(key) = self.plan.requeue_key(c) else {
            return;
        };
        let mut q = lock(&self.shared.queue);
        // Descending order: insert before the first job with a smaller key.
        let at = q.jobs.partition_point(|j| j.key > key);
        q.jobs.insert(at, Job { key });
        drop(q);
        self.shared.wake.notify_one();
    }

    /// Inserts every finished chunk and performs demotions and evictions. Never blocks on a
    /// worker.
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
        while !self.plan.in_flight.is_empty() || !self.spilling.is_empty() {
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

    /// Hands a chunk leaving residency to persistence if it holds edits; otherwise drops it.
    fn persist(&self, coord: ChunkCoord, chunk: Arc<RwLock<Chunk>>) {
        if read(&chunk).is_edited() {
            lock(&self.shared.persisted).insert(coord, chunk);
        }
    }

    /// Applies one batch of finished jobs: [`Streamer::absorb_done`], then
    /// [`Streamer::commit_residency`], then [`Streamer::schedule_io`].
    fn apply(&mut self, world: &World, done: Vec<Done>) -> StreamEvents {
        let mut ev = StreamEvents::default();
        let absorbed = self.absorb_done(world, done, &mut ev);
        let (evict, demoting) = self.commit_residency(world, absorbed.inserts, absorbed.outside);
        self.schedule_io(demoting, absorbed.requeue);
        ev.evicted.extend(evict.into_iter().map(|(c, _)| c));
        ev
    }

    /// Takes each finished job's result in order: LOD-only chunks and demotions are applied
    /// here, full chunks are returned for the store's write lock, failures are recorded.
    fn absorb_done(&mut self, world: &World, done: Vec<Done>, ev: &mut StreamEvents) -> Absorbed {
        let keep_full2 = self.plan.residency.keep_full2();
        let keep_lod2 = self.plan.residency.keep_lod2();
        let mut inserts: Vec<Held> = Vec::new();
        let mut outside = Vec::new();
        let mut requeue = Vec::new();
        for d in done {
            match d {
                Done::Full {
                    coord,
                    chunk,
                    restored,
                    prepared,
                } => {
                    if restored {
                        self.restored += 1;
                    } else {
                        self.generated += 1;
                    }
                    self.plan.in_flight.remove(&coord);
                    let since = self.plan.wanted_since.remove(&coord);
                    let d = self.plan.d2(coord);
                    // A chunk that left both bands while it loaded is dropped, or goes back
                    // to persistence if it holds edits.
                    if d > keep_lod2 {
                        self.persist(coord, chunk);
                        requeue.push(coord);
                        continue;
                    }
                    // Past the full band it is inserted anyway; the next poll's sweep demotes
                    // it (this poll's sweep runs before the insert, so it cannot).
                    if d > keep_full2 {
                        outside.push(coord);
                    }
                    inserts.push((coord, chunk));
                    ev.loaded.push((coord, Tier::Full));
                    if let Some(p) = prepared {
                        ev.prepared.push((coord, p));
                    }
                    let since = since.unwrap_or(self.plan.tick);
                    ev.latency_ticks
                        .push(u32::try_from(self.plan.tick - since).unwrap_or(u32::MAX));
                }
                Done::Lod {
                    coord,
                    mips,
                    generated,
                } => {
                    if generated {
                        self.generated += 1;
                    }
                    self.plan.in_flight.remove(&coord);
                    if self.plan.d2(coord) <= keep_lod2 && !self.plan.resident.contains_key(&coord)
                    {
                        world.insert_lod(coord, mips);
                        self.plan.set_resident(coord, Tier::LodOnly);
                        ev.loaded.push((coord, Tier::LodOnly));
                    }
                    requeue.push(coord);
                }
                Done::Demoted { coord, mips } => {
                    self.plan.in_flight.remove(&coord);
                    if self.plan.wanted_tier(coord) != Some(Tier::Full)
                        && self.plan.d2(coord) <= keep_lod2
                    {
                        world.insert_lod(coord, mips);
                        self.plan.set_resident(coord, Tier::LodOnly);
                        ev.demoted.push(coord);
                    } else {
                        // Wanted full again, or out of range: the mirror still holds the
                        // full chunk it had, so report it gone.
                        self.plan.unset_resident(coord);
                        ev.evicted.push(coord);
                        requeue.push(coord);
                    }
                }
                Done::LoadFailed { coord, kind } => {
                    self.plan.in_flight.remove(&coord);
                    self.plan.wanted_since.remove(&coord);
                    self.plan.load_failed.insert(coord);
                    ev.errors.push(StreamError::Spill { coord, kind });
                }
                Done::Spilled { coord, result } => {
                    self.spilling.remove(&coord);
                    if let Err(kind) = result {
                        self.spill_failed.insert(coord);
                        ev.errors.push(StreamError::Spill { coord, kind });
                    }
                }
                Done::Panicked(e) => std::panic::resume_unwind(e),
            }
        }
        Absorbed {
            inserts,
            outside,
            requeue,
        }
    }

    /// The sweep, then under the store's write lock the inserts, the evictions and the full
    /// chunks of the demotions leaving the store; then persisting outside the lock. Returns
    /// the evictions and the demoted chunks, whose mips a worker builds.
    fn commit_residency(
        &mut self,
        world: &World,
        inserts: Vec<Held>,
        outside: Vec<ChunkCoord>,
    ) -> (Vec<(ChunkCoord, Tier)>, Vec<Held>) {
        let mut evict = Vec::new();
        let mut demote = Vec::new();
        if self.plan.sweep {
            self.plan.sweep = false;
            (evict, demote) = self.plan.sweep_lists();
        }
        // Checked by the next sweep, which a full scan would find them in. The sweep above ran
        // before these chunks are inserted below.
        self.plan.recheck_later(outside);
        // Chunks inserted now that the sweep found outside their bands are left for the next
        // sweep; none is both inserted and removed in one poll.
        let mut removed = Vec::new();
        let mut demoting = Vec::new();
        if !inserts.is_empty() || !evict.is_empty() || !demote.is_empty() {
            let mut map = world.store.write_lock();
            for (coord, chunk) in inserts {
                map.insert(coord, chunk);
                self.plan.set_resident(coord, Tier::Full);
                world.remove_lod(coord);
            }
            for &(coord, tier) in &evict {
                if tier == Tier::Full {
                    if let Some(chunk) = map.remove(coord) {
                        removed.push((coord, chunk));
                    }
                } else {
                    world.remove_lod(coord);
                }
                self.plan.unset_resident(coord);
            }
            for &coord in &demote {
                if let Some(chunk) = map.remove(coord) {
                    demoting.push((coord, chunk));
                }
            }
        }
        // Outside the map lock: persisting reads each chunk's flag under its own lock.
        for (coord, chunk) in removed {
            self.persist(coord, chunk);
        }
        for (coord, chunk) in &demoting {
            self.persist(*coord, chunk.clone());
            // Neither in the store nor LOD-only until its mips arrive; the mirror keeps the
            // full chunk it has meanwhile.
            self.plan.unset_resident(*coord);
            self.plan.in_flight.insert(*coord, Tier::LodOnly);
        }
        (evict, demoting)
    }

    /// Queues the spill writes now due and the demotions' mips, then the loads of chunks
    /// still wanted at a tier they lack.
    fn schedule_io(&mut self, demoting: Vec<Held>, requeue: Vec<ChunkCoord>) {
        let spills = {
            let persisted = lock(&self.shared.persisted);
            self.spills_due(&persisted)
        };
        if !spills.is_empty() || !demoting.is_empty() {
            let mut q = lock(&self.shared.queue);
            for job in spills {
                self.spilling.insert(job.coord);
                q.io.push_back(job);
            }
            for (coord, chunk) in demoting {
                q.demote.push_back(DemoteJob { coord, chunk });
            }
            drop(q);
            self.shared.wake.notify_all();
        }
        for c in requeue {
            self.requeue(c);
        }
    }

    /// Spill writes that bring persisted RAM under the cap, oldest entries first, skipping
    /// entries already being written or whose write failed.
    fn spills_due(&self, p: &Persisted) -> Vec<SpillJob> {
        let mut jobs = Vec::new();
        if self.shared.spill_dir.is_none() {
            return jobs;
        }
        let cap = self.cfg.persist.ram_cap_bytes;
        let mut bytes: u64 = p
            .ram
            .iter()
            .filter(|(c, _)| !self.spilling.contains(c))
            .map(|(_, e)| e.bytes)
            .sum();
        for (&seq, &coord) in &p.by_seq {
            if bytes <= cap {
                break;
            }
            if self.spilling.contains(&coord) || self.spill_failed.contains(&coord) {
                continue;
            }
            bytes -= p.ram[&coord].bytes;
            jobs.push(SpillJob { coord, seq });
        }
        jobs
    }

    /// `update` calls so far, as [`StreamStats::ticks`], without the rest of the counters'
    /// locks.
    pub fn ticks(&self) -> u64 {
        self.plan.tick
    }

    /// The counters: the resident counts are kept as chunks change tier, so this walks
    /// nothing; it takes the persistence and job queue locks once each.
    pub fn stats(&self) -> StreamStats {
        let p = lock(&self.shared.persisted);
        let full = self.plan.resident_full;
        StreamStats {
            ticks: self.plan.tick,
            resident_full: full,
            resident_lod: self.plan.resident.len() - full,
            queued: lock(&self.shared.queue).jobs.len(),
            in_flight: self.plan.in_flight.len(),
            generated_chunks: self.generated,
            restored_chunks: self.restored,
            persisted_ram_chunks: p.ram.len(),
            persisted_ram_bytes: p.ram_bytes,
            spilled_chunks: p.spilled.len(),
            persist_over_cap: p.ram_bytes > self.cfg.persist.ram_cap_bytes,
        }
    }

    /// The tier `coord` is resident in, if any.
    pub fn tier(&self, coord: ChunkCoord) -> Option<Tier> {
        self.plan.resident.get(&coord).copied()
    }

    /// Whether `coord` is held by persistence, in RAM or in a spill file.
    pub fn is_persisted(&self, coord: ChunkCoord) -> bool {
        let p = lock(&self.shared.persisted);
        p.ram.contains_key(&coord) || p.spilled.contains_key(&coord)
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

/// Lattice chunks within `r` chunks of a focus chunk in the streamer's metric, `d2 <= r^2`:
/// the most chunks one focus keeps resident after a poll when `r` is `r_lod + h`. 0 for a
/// negative `r`.
pub fn chunks_in_ball(r: i32) -> u64 {
    if r < 0 {
        return 0;
    }
    let r2 = i64::from(r) * i64::from(r);
    let mut n = 0u64;
    for x in -r..=r {
        for y in -r..=r {
            let rest = r2 - i64::from(x) * i64::from(x) - i64::from(y) * i64::from(y);
            if rest >= 0 {
                // z in [-z, z] with z the integer square root of `rest`.
                let mut z = (rest as f64).sqrt() as i64;
                while z * z > rest {
                    z -= 1;
                }
                while (z + 1) * (z + 1) <= rest {
                    z += 1;
                }
                n += 2 * z as u64 + 1;
            }
        }
    }
    n
}

/// Whether `d` moves each axis by at most one chunk.
fn unit_step(d: [i32; 3]) -> bool {
    d.iter().all(|v| v.abs() <= 1)
}
