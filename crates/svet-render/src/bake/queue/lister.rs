//! The listing pool: with the bake thread on, a sliced opening hands its ready candidates to a
//! kept pool of threads and takes their faces in a later step, as they finish, so the bake loop
//! does not wait for a batch's listing (Plan 0099 Phase 4, lever (b)). Lockstep lists in the
//! opening's own step across scoped threads, as before, so its order and bytes are unchanged.
//!
//! A job reads the world at its listing, so a job pushed from the pool is stamped with the
//! message the opening had read when it sent the candidate: a change in a later message stales
//! it as it would stale a job listed in its own step.

use super::list::{Held, list_chunk};
use super::*;
use std::sync::mpsc;
use std::thread::JoinHandle;

/// One candidate sent to the pool.
struct Req {
    seq: u64,
    coord: ChunkCoord,
    classes: Arc<[u8]>,
    own: Option<Arc<PreparedMasks>>,
    held: Held,
}

/// One listing back from the pool: the chunk's faces and whether its own masks were prepared,
/// or the text of the panic that ended it.
type Done = (u64, [i32; 3], Result<(ChunkFaces, bool), String>);

/// The threads the pool keeps: the logical cores less the stream workers' half and the render
/// and bake threads, at least one, so the process runs no more busy threads than it has cores.
pub(in crate::bake) fn lister_threads(logical: usize) -> usize {
    logical.saturating_sub(logical / 2).saturating_sub(2).max(1)
}

/// The kept pool of listing threads. Dropping it closes the requests and joins the threads.
pub(in crate::bake) struct Lister {
    tx: Option<mpsc::Sender<Req>>,
    rx: std::sync::Mutex<mpsc::Receiver<Done>>,
    threads: Vec<JoinHandle<()>>,
}

impl Lister {
    /// Starts [`lister_threads`] threads listing from `world`.
    pub(in crate::bake) fn start(world: Arc<World>) -> std::io::Result<Self> {
        let logical = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);
        let (tx, jobs) = mpsc::channel::<Req>();
        let (done, rx) = mpsc::channel::<Done>();
        let jobs = Arc::new(std::sync::Mutex::new(jobs));
        let mut threads = Vec::new();
        for k in 0..lister_threads(logical) {
            let (jobs, done, world) = (jobs.clone(), done.clone(), world.clone());
            threads.push(
                std::thread::Builder::new()
                    .name(format!("svet-bake-list-{k}"))
                    .spawn(move || {
                        loop {
                            let req = {
                                let jobs = jobs.lock().unwrap_or_else(|p| p.into_inner());
                                jobs.recv()
                            };
                            let Ok(r) = req else { return };
                            let listed =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    list_chunk(
                                        &world,
                                        r.coord,
                                        &r.classes,
                                        r.own.as_deref(),
                                        &r.held,
                                    )
                                }))
                                .map_err(|p| crate::bake::thread::panic_text(&*p));
                            if done.send((r.seq, r.coord.0, listed)).is_err() {
                                return;
                            }
                        }
                    })?,
            );
        }
        Ok(Self {
            tx: Some(tx),
            rx: std::sync::Mutex::new(rx),
            threads,
        })
    }
}

impl Drop for Lister {
    fn drop(&mut self) {
        self.tx = None;
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

/// A candidate in the pool's hands: what its opening's checks found, the bound it was admitted
/// at, and the message it was sent under (its job's stamp).
pub(in crate::bake) struct Listing {
    seq: u64,
    class: u8,
    coarse: bool,
    bound: u64,
    stamp: u64,
    partial: bool,
    provisional: bool,
    ruled: bool,
    missed: Option<(usize, bool)>,
    /// Its faces, once back and not yet pushed (a box waited).
    listed: Option<(ChunkFaces, bool)>,
}

impl BakeQueue {
    /// Starts the listing pool on `world`; the bake thread's openings list through it.
    pub(crate) fn start_lister(&mut self, world: Arc<World>) -> std::io::Result<()> {
        self.lister = Some(Lister::start(world)?);
        Ok(())
    }

    /// Stops the listing pool and forgets what it held: those candidates are still in the
    /// candidate pool, and the next opening lists them again.
    pub(crate) fn stop_lister(&mut self) {
        self.lister = None;
        self.listing.clear();
    }

    /// Whether the openings list through the pool.
    pub(in crate::bake) fn lists_off_the_loop(&self) -> bool {
        self.lister.is_some()
    }

    /// Whether `c` is in the pool's hands.
    pub(in crate::bake) fn in_listing(&self, c: [i32; 3]) -> bool {
        self.listing.contains_key(&c)
    }

    /// The bounds of the candidates in the pool's hands: samples the opening counts as open.
    pub(in crate::bake) fn listing_samples(&self) -> u64 {
        self.listing.values().map(|l| l.bound).sum()
    }

    /// Hands `batch`'s candidates to the pool, each with its listing bound and the flags its
    /// checks left; empties `batch`. The samples handed.
    pub(in crate::bake) fn send_batch(
        &mut self,
        eye: [i32; 3],
        classes: &[u8],
        batch: &mut Vec<(u8, [i32; 3])>,
    ) -> u64 {
        let classes: Arc<[u8]> = classes.into();
        let mut sent = 0;
        for (class, c) in batch.drain(..) {
            let Some(&(_, coarse)) = self.pool.get(&c) else {
                continue;
            };
            let coord = ChunkCoord(c);
            let (own, held) = self.listing_masks(coord);
            let Some(tx) = self.lister.as_ref().and_then(|l| l.tx.as_ref()) else {
                return sent;
            };
            self.listing_seq += 1;
            let seq = self.listing_seq;
            if tx
                .send(Req {
                    seq,
                    coord,
                    classes: classes.clone(),
                    own,
                    held,
                })
                .is_err()
            {
                return sent;
            }
            let bound = self.listing_bound(c, class, eye);
            sent += bound;
            self.listing.insert(
                c,
                Listing {
                    seq,
                    class,
                    coarse,
                    bound,
                    stamp: self.lp.stamp,
                    partial: self.partial_checked.contains(&c),
                    provisional: self.provisional_checked.contains(&c),
                    ruled: self.ruled_checked.contains(&c),
                    missed: self.rule_missed_checked.get(&c).copied(),
                    listed: None,
                },
            );
        }
        sent
    }

    /// Pushes the listings the pool has finished, as [`BakeQueue::push_batch`] pushes a batch's,
    /// each job stamped with the message its candidate was sent under. A candidate that left
    /// the candidate pool, or changed its class, since it was sent is dropped (a later opening
    /// lists it again if it is back); once a box waits the rest wait for the next step.
    pub(in crate::bake) fn take_listed(
        &mut self,
        store: &mut Store,
        eye: [i32; 3],
    ) -> Result<(), BakeError> {
        let Some(lister) = self.lister.as_ref() else {
            return Ok(());
        };
        let rx = lister.rx.lock().unwrap_or_else(|p| p.into_inner());
        let back: Vec<Done> = rx.try_iter().collect();
        drop(rx);
        for (seq, c, listed) in back {
            let listed = listed.map_err(|m| BakeError::Thread(format!("a listing thread: {m}")))?;
            // A listing an edit's box dropped, or one sent again since, is not this one.
            if let Some(l) = self.listing.get_mut(&c).filter(|l| l.seq == seq) {
                l.listed = Some(listed);
            }
        }
        let back: Vec<[i32; 3]> = self
            .listing
            .iter()
            .filter(|(_, l)| l.listed.is_some())
            .map(|(c, _)| *c)
            .collect();
        for c in back {
            if self.box_waits() {
                break;
            }
            let l = self.listing.remove(&c).expect("collected above");
            if self.pool.get(&c) != Some(&(l.class, l.coarse)) {
                continue;
            }
            for (on, set) in [
                (l.partial, &mut self.partial_checked),
                (l.provisional, &mut self.provisional_checked),
                (l.ruled, &mut self.ruled_checked),
            ] {
                if on {
                    set.insert(c);
                }
            }
            if let Some(m) = l.missed {
                self.rule_missed_checked.insert(c, m);
            }
            let listed = l.listed.expect("collected above");
            match self.push_listed(store, eye, l.class, ChunkCoord(c), listed) {
                Ok(samples) => {
                    if samples > 0 && self.lp.stamps.contains_key(&c) {
                        self.lp.stamps.insert(c, l.stamp);
                    }
                }
                Err(BakeError::PoolFull { .. }) => self.stats.pool_full += 1,
                Err(e) => return Err(e),
            }
        }
        self.partial_checked.clear();
        self.provisional_checked.clear();
        self.ruled_checked.clear();
        self.rule_missed_checked.clear();
        Ok(())
    }
}
