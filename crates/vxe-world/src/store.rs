//! The chunk store and the world built on it.
//!
//! Access model (ADR-0002, ADR-0003): a map from `ChunkCoord` to `Arc<RwLock<Chunk>>`, itself
//! behind a `RwLock` written only on insert and remove. An edit holds the map's read lock for
//! its whole run and takes one chunk write lock at a time in ascending coordinate order, so
//! no chunk can be inserted or removed between its residency check and its last write.
//! Readers take short per-chunk read locks.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::brick::{Brick, SpareArrays};
use crate::chunk::Chunk;
use crate::coords::{BRICK_VOXELS, CHUNK_BRICKS, ChunkCoord, ChunkExtent, VoxelCoord};
use crate::material::{MaterialId, MaterialTable};
use crate::mips::ChunkMips;
use crate::source::WorldSource;

/// Read guard that survives a poisoned lock. A chunk is structurally valid at every point an
/// edit can panic (bricks are replaced whole), so a panic elsewhere does not make it unsafe
/// to read.
pub(crate) fn read<T>(l: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    l.read().unwrap_or_else(PoisonError::into_inner)
}

/// Write guard that survives a poisoned lock; see [`read`].
pub(crate) fn write<T>(l: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    l.write().unwrap_or_else(PoisonError::into_inner)
}

/// Resident chunks, keyed by coordinate, and the set of chunks with a dirty brick.
///
/// The dirty-chunk set has one consumer, the GPU mirror, which drains it on every sync and
/// then visits only those chunks. Inserting a chunk and an edit that changes one add it;
/// removing a chunk drops it. Code that changes a resident chunk's bricks by hand calls
/// [`ChunkStore::mark_dirty`].
#[derive(Default, Debug)]
pub struct ChunkStore {
    pub(crate) chunks: RwLock<HashMap<ChunkCoord, Arc<RwLock<Chunk>>>>,
    dirty: Mutex<HashSet<ChunkCoord>>,
}

impl ChunkStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts or replaces the chunk at `coord`, and marks it dirty.
    pub fn insert(&self, coord: ChunkCoord, chunk: Chunk) {
        self.write_lock()
            .insert(coord, Arc::new(RwLock::new(chunk)));
    }

    /// Removes the chunk at `coord` and hands it back.
    pub fn remove(&self, coord: ChunkCoord) -> Option<Arc<RwLock<Chunk>>> {
        self.write_lock().remove(coord)
    }

    /// The map's write lock, for a batch of inserts and removals under one acquisition. While
    /// the guard lives, edits wait (they hold the read lock for their whole run).
    pub fn write_lock(&self) -> StoreWriteGuard<'_> {
        StoreWriteGuard {
            map: write(&self.chunks),
            dirty: &self.dirty,
        }
    }

    /// Adds `coord` to the dirty-chunk set.
    pub fn mark_dirty(&self, coord: ChunkCoord) {
        lock(&self.dirty).insert(coord);
    }

    /// Empties the dirty-chunk set and returns its coordinates, ascending.
    pub fn take_dirty(&self) -> Vec<ChunkCoord> {
        let mut v: Vec<_> = lock(&self.dirty).drain().collect();
        v.sort_unstable();
        v
    }

    /// Chunks in the dirty-chunk set.
    pub fn dirty_len(&self) -> usize {
        lock(&self.dirty).len()
    }

    /// The chunk at `coord`, if resident.
    pub fn get(&self, coord: ChunkCoord) -> Option<Arc<RwLock<Chunk>>> {
        read(&self.chunks).get(&coord).cloned()
    }

    pub fn contains(&self, coord: ChunkCoord) -> bool {
        read(&self.chunks).contains_key(&coord)
    }

    pub fn len(&self) -> usize {
        read(&self.chunks).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Resident coordinates in ascending order.
    pub fn coords(&self) -> Vec<ChunkCoord> {
        let mut v: Vec<_> = read(&self.chunks).keys().copied().collect();
        v.sort_unstable();
        v
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The store map under its write lock; see [`ChunkStore::write_lock`].
pub struct StoreWriteGuard<'a> {
    map: RwLockWriteGuard<'a, HashMap<ChunkCoord, Arc<RwLock<Chunk>>>>,
    dirty: &'a Mutex<HashSet<ChunkCoord>>,
}

impl StoreWriteGuard<'_> {
    /// Inserts or replaces the chunk at `coord`, and marks it dirty.
    pub fn insert(&mut self, coord: ChunkCoord, chunk: Arc<RwLock<Chunk>>) {
        self.map.insert(coord, chunk);
        lock(self.dirty).insert(coord);
    }

    /// Removes the chunk at `coord`, and drops it from the dirty-chunk set.
    pub fn remove(&mut self, coord: ChunkCoord) -> Option<Arc<RwLock<Chunk>>> {
        lock(self.dirty).remove(&coord);
        self.map.remove(&coord)
    }

    pub fn contains(&self, coord: ChunkCoord) -> bool {
        self.map.contains_key(&coord)
    }

    pub fn get(&self, coord: ChunkCoord) -> Option<Arc<RwLock<Chunk>>> {
        self.map.get(&coord).cloned()
    }
}

/// A bounded free list of boxed brick arrays, shared by every edit on a world. An edit takes
/// a batch under one lock and gives back the unused rest and the arrays of the bricks it
/// collapsed under a second; [`World::reserve_brick_arrays`] fills it off the edit path, and
/// a chunk leaving residency gives back its `Full` bricks' arrays. The arrays it holds carry
/// stale voxels: a taker overwrites all 512 before use. It never decides a result: an empty
/// pool makes `expand` allocate.
#[derive(Default)]
pub(crate) struct BrickArrays(Mutex<SpareArrays>);

impl std::fmt::Debug for BrickArrays {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "BrickArrays({})", lock(&self.0).len())
    }
}

impl BrickArrays {
    fn len(&self) -> usize {
        lock(&self.0).len()
    }

    /// Up to `n` arrays, under one lock.
    pub(crate) fn take(&self, n: usize) -> SpareArrays {
        if n == 0 {
            return Vec::new();
        }
        let mut pool = lock(&self.0);
        let keep = pool.len().saturating_sub(n);
        pool.split_off(keep)
    }

    /// Gives `arrays` back under one lock, up to [`World::BRICK_ARRAY_POOL_CAP`] in the pool;
    /// the rest are freed after the lock is released.
    pub(crate) fn give(&self, mut arrays: SpareArrays) {
        if arrays.is_empty() {
            return;
        }
        {
            let mut pool = lock(&self.0);
            let room = World::BRICK_ARRAY_POOL_CAP.saturating_sub(pool.len());
            let n = room.min(arrays.len());
            pool.extend(arrays.drain(..n));
        }
        drop(arrays);
    }
}

/// Brick counts over the resident world.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct WorldStats {
    pub chunks: usize,
    pub uniform_bricks: usize,
    pub full_bricks: usize,
}

/// A world of resident chunks. [`World::generate`] makes a bounded one, every chunk of an
/// extent generated up front and resident for good; [`World::new`] makes an empty, unbounded
/// one that a [`crate::Streamer`] fills and empties around focus points.
///
/// Besides the full chunks of the store, a world holds the mips of its LOD-only chunks
/// (ADR-0003). Edits, queries and raycasts see only the store, so to them a LOD-only chunk is
/// not resident; the GPU mirror draws it from its mips.
#[derive(Debug)]
pub struct World {
    pub(crate) store: ChunkStore,
    lods: RwLock<HashMap<ChunkCoord, Arc<ChunkMips>>>,
    extent: ChunkExtent,
    pub(crate) materials: MaterialTable,
    pub(crate) brick_arrays: BrickArrays,
}

impl World {
    /// The most brick arrays the pool holds: 8192 arrays of 512 `u16`, 8 MiB.
    pub const BRICK_ARRAY_POOL_CAP: usize = 8192;

    /// Tops the brick-array pool up to `min(n, BRICK_ARRAY_POOL_CAP)` arrays. The arrays are
    /// allocated and written on the caller's thread outside the pool's lock, so their pages
    /// are touched before an edit takes them; call it off the edit path (after a frame's
    /// edits, or on a worker).
    pub fn reserve_brick_arrays(&self, n: usize) {
        let want = n.min(Self::BRICK_ARRAY_POOL_CAP);
        let need = want.saturating_sub(self.brick_arrays.len());
        if need == 0 {
            return;
        }
        // A non-zero fill, so the allocation is not a `calloc` that leaves fresh pages
        // untouched.
        let fresh: SpareArrays = (0..need)
            .map(|_| Box::new([MaterialId::MAX; BRICK_VOXELS]))
            .collect();
        self.brick_arrays.give(fresh);
    }

    /// Arrays in the brick-array pool.
    pub fn pool_len(&self) -> usize {
        self.brick_arrays.len()
    }

    /// Gives the arrays of `chunk`'s `Full` bricks to the pool when no one else holds the
    /// chunk; a chunk held elsewhere is left to its last holder, which frees it.
    pub(crate) fn recycle_chunk(&self, chunk: Arc<RwLock<Chunk>>) {
        let Ok(chunk) = Arc::try_unwrap(chunk) else {
            return;
        };
        let mut chunk = chunk.into_inner().unwrap_or_else(PoisonError::into_inner);
        let mut arrays = Vec::new();
        for slot in 0..CHUNK_BRICKS {
            if let Brick::Full(v) = std::mem::take(chunk.brick_mut(slot)) {
                arrays.push(v);
            }
        }
        self.brick_arrays.give(arrays);
    }

    /// An empty, unbounded world. Its [`World::extent`] is empty.
    pub fn new(table: &MaterialTable) -> Self {
        Self {
            store: ChunkStore::new(),
            lods: RwLock::default(),
            extent: ChunkExtent::new([0, 0, 0], [0, 0, 0]),
            materials: table.clone(),
            brick_arrays: BrickArrays::default(),
        }
    }

    /// Generates every chunk of `extent` from `source`, in parallel over chunks with one
    /// thread per available core. The result does not depend on the thread count.
    pub fn generate<S: WorldSource + Sync>(
        extent: ChunkExtent,
        source: &S,
        table: &MaterialTable,
    ) -> Self {
        let threads = std::thread::available_parallelism().map_or(1, usize::from);
        Self::generate_with_threads(extent, source, table, threads)
    }

    /// [`World::generate`] on exactly `threads` threads (at least one).
    pub fn generate_with_threads<S: WorldSource + Sync>(
        extent: ChunkExtent,
        source: &S,
        table: &MaterialTable,
        threads: usize,
    ) -> Self {
        let coords = extent.coords();
        let threads = threads.clamp(1, coords.len().max(1));
        let store = ChunkStore::new();
        std::thread::scope(|s| {
            let workers: Vec<_> = (0..threads)
                .map(|t| {
                    let coords = &coords;
                    s.spawn(move || {
                        coords
                            .iter()
                            .skip(t)
                            .step_by(threads)
                            .map(|&c| {
                                let mut chunk = Chunk::default();
                                source.chunk(c, &mut chunk);
                                (c, chunk)
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            for w in workers {
                // A panicking source is a bug in the source; re-raise it on the caller.
                let chunks = w.join().unwrap_or_else(|e| std::panic::resume_unwind(e));
                for (c, chunk) in chunks {
                    store.insert(c, chunk);
                }
            }
        });
        Self {
            store,
            lods: RwLock::default(),
            extent,
            materials: table.clone(),
            brick_arrays: BrickArrays::default(),
        }
    }

    /// The extent generated up front; empty for a world made by [`World::new`].
    pub fn extent(&self) -> ChunkExtent {
        self.extent
    }

    pub fn materials(&self) -> &MaterialTable {
        &self.materials
    }

    pub fn store(&self) -> &ChunkStore {
        &self.store
    }

    /// The mips of LOD-only chunk `c`, if it is one.
    pub fn lod(&self, c: ChunkCoord) -> Option<Arc<ChunkMips>> {
        read(&self.lods).get(&c).cloned()
    }

    /// Makes `c` LOD-only with `mips`, and marks it in the dirty-chunk set so the mirror
    /// uploads it. The streamer calls it; tests call it to build LOD scenes. A full chunk at
    /// `c` should be removed from the store first: the mirror prefers the full chunk.
    pub fn insert_lod(&self, c: ChunkCoord, mips: Arc<ChunkMips>) {
        write(&self.lods).insert(c, mips);
        self.store.mark_dirty(c);
    }

    /// Drops `c`'s mips.
    pub fn remove_lod(&self, c: ChunkCoord) -> Option<Arc<ChunkMips>> {
        write(&self.lods).remove(&c)
    }

    /// LOD-only chunks, ascending.
    pub fn lod_coords(&self) -> Vec<ChunkCoord> {
        let mut v: Vec<_> = read(&self.lods).keys().copied().collect();
        v.sort_unstable();
        v
    }

    /// Chunk and brick counts, read one chunk lock at a time.
    pub fn stats(&self) -> WorldStats {
        let mut s = WorldStats::default();
        for c in self.store.coords() {
            let Some(chunk) = self.store.get(c) else {
                continue;
            };
            let chunk = read(&chunk);
            s.chunks += 1;
            for b in chunk.bricks() {
                if b.is_full() {
                    s.full_bricks += 1;
                } else {
                    s.uniform_bricks += 1;
                }
            }
        }
        s
    }

    /// The material of voxel `v`, or `None` when its chunk is not resident.
    pub fn material_at(&self, v: impl Into<VoxelCoord>) -> Option<MaterialId> {
        let v = v.into();
        let chunk = self.store.get(v.chunk())?;
        let chunk = read(&chunk);
        Some(chunk.brick(v.brick_slot()).get(v.index_in_brick()))
    }
}
