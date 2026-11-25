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
use std::collections::VecDeque;
use std::sync::mpsc;
use std::thread::JoinHandle;

/// The level a pooled opening admits to, in caps: the loop's samples left and the pool's
/// estimates under `POOLED_LEVEL x cap` (Plan 0099 Phase 4b). What an opening sends is pushed a
/// step later, so with a draw of at most one cap the next step still holds two slices.
pub(in crate::bake) const POOLED_LEVEL: u64 = 3;

/// The candidates the pool may hold at once: a guard, counted ([`QueueStats::pool_guard`]),
/// not the rule the level is.
pub(in crate::bake) const POOL_GUARD: usize = 64;

/// The listings a fill is kept over, and how many of a kind must have returned before it is
/// read (before that a candidate counts at its bound).
const FILL_KEPT: usize = 64;
const FILL_FIRST: usize = 16;

/// The measured fill of the listing bounds ([`BakeQueue::listing_bound`]): the samples the
/// returned listings pushed over their bounds, over the last [`FILL_KEPT`] of each kind, one
/// for candidates listed from prepared masks and one for those listed from the world (the
/// bound's two formulas). A faceless or zero-by-rule listing counts its 0; one the store
/// refused is not counted.
#[derive(Default)]
pub(in crate::bake) struct Fill {
    kept: [VecDeque<(u64, u64)>; 2],
}

impl Fill {
    /// A listing of `bound` returned and pushed `samples`, from prepared masks or not.
    pub(in crate::bake) fn note(&mut self, prepared: bool, bound: u64, samples: u64) {
        let k = &mut self.kept[usize::from(prepared)];
        k.push_back((bound, samples));
        if k.len() > FILL_KEPT {
            k.pop_front();
        }
    }

    /// The fill of a kind: 1 until [`FILL_FIRST`] have returned, else the kept samples over
    /// the kept bounds.
    pub(in crate::bake) fn of(&self, prepared: bool) -> f64 {
        let k = &self.kept[usize::from(prepared)];
        let bounds: u64 = k.iter().map(|p| p.0).sum();
        if k.len() < FILL_FIRST || bounds == 0 {
            return 1.0;
        }
        k.iter().map(|p| p.1).sum::<u64>() as f64 / bounds as f64
    }

    /// The two fills, listed from the world and from prepared masks.
    pub(in crate::bake) fn figures(&self) -> [f64; 2] {
        [self.of(false), self.of(true)]
    }
}

/// The estimates of the candidates in `listing`, each its bound at its kind's `fill`.
pub(in crate::bake) fn estimate_of(
    listing: &std::collections::HashMap<[i32; 3], Listing>,
    fill: &Fill,
) -> u64 {
    listing
        .values()
        .map(|l| at_fill(l.bound, fill.of(l.prepared)))
        .sum()
}

/// `bound` counted at `fill`.
fn at_fill(bound: u64, fill: f64) -> u64 {
    (bound as f64 * fill).round() as u64
}

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
    /// Whether its own masks were prepared when it was sent: which fill its estimate reads.
    prepared: bool,
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

    /// The submits a step holds unread at most: the bake thread's depth while the pool runs
    /// (the bake thread's steps), [`IN_FLIGHT`] in lockstep, the pre-roll and the one-shot bakes.
    pub(in crate::bake) fn in_flight_bound(&self) -> usize {
        if self.lists_off_the_loop() {
            self.thread_depth.max(1)
        } else {
            IN_FLIGHT
        }
    }

    /// Sets the bake thread's depth ([`BakeQueue::in_flight_bound`]), at least 1.
    pub(crate) fn set_thread_depth(&mut self, depth: usize) {
        self.thread_depth = depth.max(1);
    }

    /// Whether `c` is in the pool's hands.
    pub(in crate::bake) fn in_listing(&self, c: [i32; 3]) -> bool {
        self.listing.contains_key(&c)
    }

    /// The estimates of the candidates in the pool's hands, each its bound at its kind's fill
    /// ([`Fill`]): samples a pooled opening counts as open.
    pub(in crate::bake) fn pooled_estimate(&self) -> u64 {
        estimate_of(&self.listing, &self.fill)
    }

    /// What a pooled opening projects the bake half holds: the loop's samples left and the
    /// pool's estimates.
    pub(in crate::bake) fn projection(&self) -> u64 {
        self.lp.samples_left() + self.pooled_estimate()
    }

    /// The estimate a pooled opening admits `c` at: its bound at the fill of its kind.
    pub(in crate::bake) fn pooled_bound(&self, c: [i32; 3], class: u8, eye: [i32; 3]) -> u64 {
        let fill = self.fill.of(self.prepared.contains_key(&c));
        at_fill(self.listing_bound(c, class, eye), fill)
    }

    /// Whether the pool holds [`POOL_GUARD`] candidates with `more` about to be sent.
    pub(in crate::bake) fn pool_guard_full(&self, off_loop: bool, more: usize) -> bool {
        off_loop && self.listing.len() + more >= POOL_GUARD
    }

    /// Hands `batch`'s candidates to the pool, each with its listing bound and the flags its
    /// checks left; empties `batch`. The samples handed, at their estimates.
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
            let own_prepared = own.is_some();
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
            sent += at_fill(bound, self.fill.of(own_prepared));
            self.listing.insert(
                c,
                Listing {
                    seq,
                    class,
                    coarse,
                    bound,
                    prepared: own_prepared,
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

    /// Keeps the listings the pool has finished by now with their candidates.
    fn receive_listed(&mut self) -> Result<(), BakeError> {
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
        Ok(())
    }

    /// A test's wait: until every candidate in the pool's hands is listed.
    #[cfg(test)]
    pub(in crate::bake) fn wait_listed(&mut self) {
        while self.listing.values().any(|l| l.listed.is_none()) {
            std::thread::sleep(std::time::Duration::from_micros(100));
            self.receive_listed().expect("the listings run");
        }
    }

    /// A test's read: the candidates in the pool's hands in the order they were sent.
    #[cfg(test)]
    pub(in crate::bake) fn listing_order(&self) -> Vec<[i32; 3]> {
        let mut v: Vec<(u64, [i32; 3])> = self.listing.iter().map(|(c, l)| (l.seq, *c)).collect();
        v.sort_unstable();
        v.into_iter().map(|(_, c)| c).collect()
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
        if self.lister.is_none() {
            return Ok(());
        }
        self.receive_listed()?;
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
                    self.fill.note(l.prepared, l.bound, samples);
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
        self.stats.fill = self.fill.figures();
        Ok(())
    }
}
