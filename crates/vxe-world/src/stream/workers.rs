//! The stream's worker pool: the shared queue the workers take jobs from, the loads from
//! persistence or the source, the spill writes and the demotions' mips.

use std::collections::VecDeque;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex, PoisonError, RwLock};

use super::{Job, Key, Tier};
use crate::chunk::Chunk;
use crate::coords::ChunkCoord;
use crate::mips::ChunkMips;
use crate::persist::{self, Persisted, SpillError};
use crate::source::WorldSource;
use crate::store::read;

#[cfg(doc)]
use super::{StreamEvents, Streamer};

/// Write `coord`'s RAM entry of sequence number `seq` to a spill file.
pub(super) struct SpillJob {
    pub(super) coord: ChunkCoord,
    pub(super) seq: u64,
}

/// Build the mips of a chunk that just left the store.
pub(super) struct DemoteJob {
    pub(super) coord: ChunkCoord,
    pub(super) chunk: Arc<RwLock<Chunk>>,
}

pub(super) enum Done {
    /// A full chunk, generated from the source (`restored` false) or taken back from
    /// persistence.
    Full {
        coord: ChunkCoord,
        chunk: Arc<RwLock<Chunk>>,
        restored: bool,
        /// The caller's hook's result on it ([`Streamer::set_prep`]), if one is set.
        prepared: Option<Prepared>,
    },
    /// Mips for a LOD-only load; `generated` when the full chunk came from the source.
    Lod {
        coord: ChunkCoord,
        mips: Arc<ChunkMips>,
        generated: bool,
    },
    Demoted {
        coord: ChunkCoord,
        mips: Arc<ChunkMips>,
    },
    LoadFailed {
        coord: ChunkCoord,
        kind: SpillError,
    },
    Spilled {
        coord: ChunkCoord,
        result: Result<(), SpillError>,
    },
    Panicked(Box<dyn std::any::Any + Send>),
}

#[derive(Default)]
pub(super) struct Queue {
    /// Sorted descending by key, so `pop` takes the nearest.
    pub(super) jobs: Vec<Job>,
    /// Spill writes and demotions, taken before any load.
    pub(super) io: VecDeque<SpillJob>,
    pub(super) demote: VecDeque<DemoteJob>,
    pub(super) shutdown: bool,
}

pub(super) struct Shared {
    pub(super) queue: Mutex<Queue>,
    pub(super) wake: Condvar,
    pub(super) persisted: Mutex<Persisted>,
    pub(super) spill_dir: Option<PathBuf>,
    pub(super) world_key: u64,
    /// The caller's hook, run on each full chunk a worker loads ([`Streamer::set_prep`]).
    pub(super) prep: RwLock<Option<Arc<dyn ChunkPrep>>>,
}

pub(super) fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

enum Work {
    Load(Key),
    Spill(SpillJob),
    Demote(DemoteJob),
}

/// Work the caller has the stream workers do on each full chunk they load (ADR-0017): a pure
/// function of the chunk's content, whose result the world carries to [`Streamer::poll`]'s
/// events and never reads. A caller keeps an inline path for a chunk with no result, and
/// throws a result away once the chunk is edited.
pub trait ChunkPrep: Send + Sync {
    fn prepare(&self, coord: ChunkCoord, chunk: &Chunk) -> Prepared;
}

/// A [`ChunkPrep`] result, opaque to the world. Two are equal when they are the same value.
#[derive(Clone)]
pub struct Prepared(pub Arc<dyn std::any::Any + Send + Sync>);

impl std::fmt::Debug for Prepared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Prepared(..)")
    }
}

impl PartialEq for Prepared {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Prepared {}

/// `done` with the hook's result where it is a full chunk and a hook is set.
fn prepare(shared: &Shared, mut done: Done) -> Done {
    if let Done::Full {
        coord,
        chunk,
        prepared,
        ..
    } = &mut done
    {
        let hook = shared
            .prep
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(h) = hook {
            *prepared = Some(h.prepare(*coord, &read(chunk)));
        }
    }
    done
}

pub(super) fn worker<S: WorldSource + ?Sized>(shared: &Shared, source: &S, tx: &Sender<Done>) {
    loop {
        let work = {
            let mut q = lock(&shared.queue);
            loop {
                if q.shutdown {
                    return;
                }
                if let Some(job) = q.io.pop_front() {
                    break Work::Spill(job);
                }
                if let Some(job) = q.demote.pop_front() {
                    break Work::Demote(job);
                }
                if let Some(job) = q.jobs.pop() {
                    break Work::Load(job.key);
                }
                q = shared.wake.wait(q).unwrap_or_else(PoisonError::into_inner);
            }
        };
        let r = std::panic::catch_unwind(AssertUnwindSafe(|| match work {
            Work::Load(key) if key.tier == Tier::Full => {
                prepare(shared, load_full(shared, source, key.coord))
            }
            Work::Load(key) => load_lod(shared, source, key.coord),
            Work::Spill(job) => spill(shared, job),
            Work::Demote(DemoteJob { coord, chunk }) => Done::Demoted {
                coord,
                mips: Arc::new(ChunkMips::build(&read(&chunk))),
            },
        }));
        let done = r.unwrap_or_else(Done::Panicked);
        if tx.send(done).is_err() {
            return;
        }
    }
}

/// Where a persisted chunk was found: in RAM (taken out, or shared when kept), or a file.
enum Found {
    Ram(Arc<RwLock<Chunk>>),
    File(PathBuf),
}

/// Looks `coord` up in persistence. A full load takes a RAM entry out; a LOD load (`keep`)
/// leaves it, since a LOD-only chunk does not hold the voxels.
fn find_persisted(shared: &Shared, coord: ChunkCoord, keep: bool) -> Option<Found> {
    let mut p = lock(&shared.persisted);
    if keep {
        if let Some(e) = p.ram.get(&coord) {
            return Some(Found::Ram(e.chunk.clone()));
        }
    } else if let Some(e) = p.take(coord) {
        return Some(Found::Ram(e.chunk));
    }
    p.spilled.get(&coord).cloned().map(Found::File)
}

fn read_spill(shared: &Shared, coord: ChunkCoord, path: &PathBuf) -> Result<Chunk, SpillError> {
    let bytes = std::fs::read(path).map_err(|_| SpillError::Io)?;
    match persist::decode_spill(&bytes, shared.world_key)? {
        (c, chunk) if c == coord => Ok(chunk),
        _ => Err(SpillError::Corrupt),
    }
}

/// Loads `coord` full from persistence if it is there, else from the source.
fn load_full<S: WorldSource + ?Sized>(shared: &Shared, source: &S, coord: ChunkCoord) -> Done {
    match find_persisted(shared, coord, false) {
        Some(Found::Ram(chunk)) => {
            crate::store::write(&chunk).mark_all_dirty();
            Done::Full {
                coord,
                chunk,
                restored: true,
                prepared: None,
            }
        }
        Some(Found::File(path)) => match read_spill(shared, coord, &path) {
            Ok(chunk) => {
                lock(&shared.persisted).spilled.remove(&coord);
                // The chunk is restored either way; a file left behind is only disk.
                let _ = std::fs::remove_file(&path);
                Done::Full {
                    coord,
                    chunk: Arc::new(RwLock::new(chunk)),
                    restored: true,
                    prepared: None,
                }
            }
            Err(kind) => Done::LoadFailed { coord, kind },
        },
        None => {
            let mut chunk = Chunk::default();
            source.chunk(coord, &mut chunk);
            Done::Full {
                coord,
                chunk: Arc::new(RwLock::new(chunk)),
                restored: false,
                prepared: None,
            }
        }
    }
}

/// Builds `coord`'s mips from its persisted copy if there is one, else from a freshly
/// generated chunk that is then dropped.
fn load_lod<S: WorldSource + ?Sized>(shared: &Shared, source: &S, coord: ChunkCoord) -> Done {
    let (mips, generated) = match find_persisted(shared, coord, true) {
        Some(Found::Ram(chunk)) => (ChunkMips::build(&read(&chunk)), false),
        Some(Found::File(path)) => match read_spill(shared, coord, &path) {
            Ok(chunk) => (ChunkMips::build(&chunk), false),
            Err(kind) => return Done::LoadFailed { coord, kind },
        },
        None => {
            let mut chunk = Chunk::default();
            source.chunk(coord, &mut chunk);
            (ChunkMips::build(&chunk), true)
        }
    };
    Done::Lod {
        coord,
        mips: Arc::new(mips),
        generated,
    }
}

/// Writes a RAM entry to its spill file, then drops it from RAM if it is still the same
/// entry. An entry a load took back meanwhile keeps no file.
fn spill(shared: &Shared, job: SpillJob) -> Done {
    let SpillJob { coord, seq } = job;
    let Some(dir) = &shared.spill_dir else {
        return Done::Spilled {
            coord,
            result: Ok(()),
        };
    };
    let chunk = {
        let p = lock(&shared.persisted);
        match p.ram.get(&coord) {
            Some(e) if e.seq == seq => e.chunk.clone(),
            _ => {
                return Done::Spilled {
                    coord,
                    result: Ok(()),
                };
            }
        }
    };
    let bytes = persist::encode_spill(coord, &read(&chunk), shared.world_key);
    let path = persist::spill_path(dir, coord, seq);
    if let Err(kind) = persist::write_file(&path, &bytes) {
        return Done::Spilled {
            coord,
            result: Err(kind),
        };
    }
    let mut p = lock(&shared.persisted);
    if p.ram.get(&coord).is_some_and(|e| e.seq == seq) {
        p.take(coord);
        p.spilled.insert(coord, path);
    } else {
        drop(p);
        let _ = std::fs::remove_file(&path);
    }
    Done::Spilled {
        coord,
        result: Ok(()),
    }
}
