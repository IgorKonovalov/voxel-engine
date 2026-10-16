//! The chunk store and the world built on it.
//!
//! Access model (ADR-0002, ADR-0003): a map from `ChunkCoord` to `Arc<RwLock<Chunk>>`, itself
//! behind a `RwLock` written only on insert and remove. An edit holds the map's read lock for
//! its whole run and takes one chunk write lock at a time in ascending coordinate order, so
//! no chunk can be inserted or removed between its residency check and its last write.
//! Readers take short per-chunk read locks.

use std::collections::HashMap;
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::chunk::Chunk;
use crate::coords::{ChunkCoord, ChunkExtent, VoxelCoord};
use crate::material::{MaterialId, MaterialTable};
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

/// Resident chunks, keyed by coordinate.
#[derive(Default, Debug)]
pub struct ChunkStore {
    pub(crate) chunks: RwLock<HashMap<ChunkCoord, Arc<RwLock<Chunk>>>>,
}

impl ChunkStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts or replaces the chunk at `coord`.
    pub fn insert(&self, coord: ChunkCoord, chunk: Chunk) {
        write(&self.chunks).insert(coord, Arc::new(RwLock::new(chunk)));
    }

    /// Removes the chunk at `coord` and hands it back.
    pub fn remove(&self, coord: ChunkCoord) -> Option<Arc<RwLock<Chunk>>> {
        write(&self.chunks).remove(&coord)
    }

    /// The map's write lock, for a batch of inserts and removals under one acquisition. While
    /// the guard lives, edits wait (they hold the read lock for their whole run).
    pub fn write_lock(&self) -> StoreWriteGuard<'_> {
        StoreWriteGuard(write(&self.chunks))
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

/// The store map under its write lock; see [`ChunkStore::write_lock`].
pub struct StoreWriteGuard<'a>(RwLockWriteGuard<'a, HashMap<ChunkCoord, Arc<RwLock<Chunk>>>>);

impl StoreWriteGuard<'_> {
    pub fn insert(&mut self, coord: ChunkCoord, chunk: Arc<RwLock<Chunk>>) {
        self.0.insert(coord, chunk);
    }

    pub fn remove(&mut self, coord: ChunkCoord) -> Option<Arc<RwLock<Chunk>>> {
        self.0.remove(&coord)
    }

    pub fn contains(&self, coord: ChunkCoord) -> bool {
        self.0.contains_key(&coord)
    }

    pub fn get(&self, coord: ChunkCoord) -> Option<Arc<RwLock<Chunk>>> {
        self.0.get(&coord).cloned()
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
#[derive(Debug)]
pub struct World {
    pub(crate) store: ChunkStore,
    extent: ChunkExtent,
    pub(crate) materials: MaterialTable,
}

impl World {
    /// An empty, unbounded world. Its [`World::extent`] is empty.
    pub fn new(table: &MaterialTable) -> Self {
        Self {
            store: ChunkStore::new(),
            extent: ChunkExtent::new([0, 0, 0], [0, 0, 0]),
            materials: table.clone(),
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
            extent,
            materials: table.clone(),
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
