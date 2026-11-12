//! The streamer's residency decisions, apart from its threads and locks: which chunks the focus
//! wants and at which tier, the job queue rebuilt or shifted by a one-chunk focus step, and the
//! sweep's evictions and demotions. The queue's methods write a plain job vector, which the
//! facade passes in under the queue's lock, so every decision here runs without a thread.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::{Residency, Tier, unit_step};
use crate::coords::{ChunkCoord, VoxelCoord};

/// A job's queue key: smaller runs first. Full loads sort before LOD loads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Key {
    pub(super) tier: Tier,
    pub(super) d2: i64,
    pub(super) coord: ChunkCoord,
}

#[derive(Clone)]
pub(super) struct Job {
    pub(super) key: Key,
}

/// A cached shell: a ball's squared radius, the focus step and whether the shell enters it.
type ShellKey = (i64, [i32; 3], bool);

/// How an update's focus chunks differ from the last update's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Refocus {
    /// The same chunks: nothing to do.
    Same,
    /// The single focus moved one chunk step (each axis by at most one): the queue shifts.
    Step([i32; 3]),
    /// Any other change: the queue is rebuilt and the next sweep scans all of residency.
    Jump,
}

/// What the queue and the sweep read and write; see the module docs.
#[derive(Clone)]
pub(super) struct ResidencyPlan {
    pub(super) residency: Residency,
    pub(super) focus: Vec<ChunkCoord>,
    /// Set when the focus chunks change or a chunk arrives outside its band; the next `poll`
    /// sweeps residency for demotions and evictions.
    pub(super) sweep: bool,
    /// Every change goes through [`ResidencyPlan::set_resident`] and
    /// [`ResidencyPlan::unset_resident`], which keep `resident_full`.
    pub(super) resident: HashMap<ChunkCoord, Tier>,
    /// The `Tier::Full` entries of `resident`, kept by count so `stats` walks nothing.
    pub(super) resident_full: usize,
    /// Chunks with a load or demotion job queued or running, and the tier it produces.
    pub(super) in_flight: HashMap<ChunkCoord, Tier>,
    pub(super) load_failed: HashSet<ChunkCoord>,
    /// The tick at which each chunk with a full load was first wanted full.
    pub(super) wanted_since: HashMap<ChunkCoord, u64>,
    /// The next sweep scans all of residency: set by a jump, several focus points, the first
    /// update and `set_radii`. Otherwise a sweep visits the shells the single focus moved
    /// across since `swept_focus`, plus `recheck`.
    sweep_full: bool,
    swept_focus: Option<ChunkCoord>,
    /// Chunks inserted outside their band since the last sweep, which the next one checks.
    recheck: HashSet<ChunkCoord>,
    /// The offsets, from the new focus chunk, of a ball's entering or leaving shell for one
    /// focus step: by squared radius, step and `entering`.
    shells: HashMap<ShellKey, Arc<Vec<[i32; 3]>>>,
    /// Checks every incremental queue update and sweep against the full rebuild and scan.
    #[cfg(any(test, feature = "stream-oracle"))]
    pub(super) oracle: bool,
    #[cfg(any(test, feature = "stream-oracle"))]
    pub(super) oracle_checks: (u64, u64),
    /// `update` calls so far.
    pub(super) tick: u64,
}

impl ResidencyPlan {
    pub(super) fn new(residency: Residency) -> Self {
        Self {
            residency,
            focus: Vec::new(),
            sweep: false,
            resident: HashMap::new(),
            resident_full: 0,
            in_flight: HashMap::new(),
            load_failed: HashSet::new(),
            wanted_since: HashMap::new(),
            sweep_full: true,
            swept_focus: None,
            recheck: HashSet::new(),
            shells: HashMap::new(),
            #[cfg(any(test, feature = "stream-oracle"))]
            oracle: false,
            #[cfg(any(test, feature = "stream-oracle"))]
            oracle_checks: (0, 0),
            tick: 0,
        }
    }

    /// Takes new radii: the next sweep scans all of residency. The caller rebuilds the queue.
    pub(super) fn set_residency(&mut self, residency: Residency) {
        self.residency = residency;
        self.sweep = true;
        self.sweep_full = true;
    }

    /// Counts an update and takes its focus points. When the focus chunks changed, the next
    /// `poll` sweeps; a jump also makes that sweep a full scan.
    pub(super) fn refocus(&mut self, focus: &[VoxelCoord]) -> Refocus {
        self.tick += 1;
        let mut chunks: Vec<ChunkCoord> = focus.iter().map(|v| v.chunk()).collect();
        chunks.sort_unstable();
        chunks.dedup();
        if chunks == self.focus {
            return Refocus::Same;
        }
        let step = match (self.focus.as_slice(), chunks.as_slice()) {
            ([a], [b]) => Some([0, 1, 2].map(|k| b.0[k] - a.0[k])).filter(|d| unit_step(*d)),
            _ => None,
        };
        self.focus = chunks;
        self.sweep = true;
        match step {
            Some(d) => Refocus::Step(d),
            None => {
                self.sweep_full = true;
                Refocus::Jump
            }
        }
    }

    /// The offsets from the new focus chunk of the chunks entering (`entering`) or leaving
    /// the ball of squared radius `r2` when the focus steps by `d`: entering, `|o|^2 <= r2 <
    /// |o + d|^2`; leaving, `|o + d|^2 <= r2 < |o|^2`.
    fn shell(&mut self, r2: i64, d: [i32; 3], entering: bool) -> Arc<Vec<[i32; 3]>> {
        self.shells
            .entry((r2, d, entering))
            .or_insert_with(|| {
                let r = (r2.max(0) as f64).sqrt() as i32 + 2;
                let len2 = |o: [i32; 3]| o.iter().map(|&v| i64::from(v).pow(2)).sum::<i64>();
                let mut out = Vec::new();
                for x in -r..=r {
                    for y in -r..=r {
                        for z in -r..=r {
                            let o = [x, y, z];
                            let (now, was) = (len2(o), len2([x + d[0], y + d[1], z + d[2]]));
                            let hit = if entering {
                                now <= r2 && was > r2
                            } else {
                                was <= r2 && now > r2
                            };
                            if hit {
                                out.push(o);
                            }
                        }
                    }
                }
                Arc::new(out)
            })
            .clone()
    }

    /// The queue after a one-step move of the single focus by `d`: [`ResidencyPlan::rebuild_jobs`]'s
    /// result, from the chunks whose want could change. With the oracle on, the rebuild is
    /// computed from the same state as well and a difference panics.
    pub(super) fn shift_queue(&mut self, jobs: &mut Vec<Job>, d: [i32; 3]) {
        #[cfg(any(test, feature = "stream-oracle"))]
        if self.oracle {
            let before = (
                jobs.clone(),
                self.in_flight.clone(),
                self.wanted_since.clone(),
            );
            self.shift_jobs(jobs, d);
            let shifted = (
                jobs.clone(),
                self.in_flight.clone(),
                self.wanted_since.clone(),
            );
            (*jobs, self.in_flight, self.wanted_since) = before;
            self.rebuild_jobs(jobs);
            let keys = |jobs: &[Job]| jobs.iter().map(|j| j.key).collect::<Vec<_>>();
            assert_eq!(keys(&shifted.0), keys(jobs), "incremental queue");
            assert_eq!(shifted.1, self.in_flight, "incremental in-flight set");
            assert_eq!(shifted.2, self.wanted_since, "incremental wanted_since");
            self.oracle_checks.0 += 1;
            return;
        }
        self.shift_jobs(jobs, d);
    }

    /// [`ResidencyPlan::shift_queue`]'s update itself. The chunks whose want could change are
    /// the queued jobs (re-keyed or dropped), the chunks entering the LOD ball, and those
    /// entering or leaving the full ball; every other wanted chunk keeps its tier and has a
    /// job, or needs none.
    pub(super) fn shift_jobs(&mut self, jobs: &mut Vec<Job>, d: [i32; 3]) {
        let f = self.focus[0];
        let (full2, lod2) = (self.residency.full2(), self.residency.lod2());
        let at = |o: &[i32; 3]| ChunkCoord([0, 1, 2].map(|k| f.0[k] + o[k]));
        let mut eval: HashSet<ChunkCoord> = HashSet::new();
        for job in jobs.drain(..) {
            self.in_flight.remove(&job.key.coord);
            eval.insert(job.key.coord);
        }
        let leaving_full = self.shell(full2, d, false);
        for o in leaving_full.iter() {
            self.wanted_since.remove(&at(o));
            eval.insert(at(o));
        }
        for o in self.shell(lod2, d, true).iter() {
            eval.insert(at(o));
        }
        for o in self.shell(full2, d, true).iter() {
            eval.insert(at(o));
        }
        let mut shifted = Vec::new();
        for c in eval {
            let Some(tier) = self.wanted_tier(c) else {
                continue;
            };
            let key = Key {
                tier,
                d2: self.d2(c),
                coord: c,
            };
            if !self.needs_job(&key) {
                continue;
            }
            if tier == Tier::Full {
                self.wanted_since.entry(c).or_insert(self.tick);
            }
            self.in_flight.insert(c, tier);
            shifted.push(Job { key });
        }
        shifted.sort_unstable_by_key(|j| std::cmp::Reverse(j.key));
        *jobs = shifted;
    }

    /// Smallest squared chunk distance from `c` to a focus chunk; `i64::MAX` with no focus.
    pub(super) fn d2(&self, c: ChunkCoord) -> i64 {
        self.focus
            .iter()
            .map(|f| c.distance2(*f))
            .min()
            .unwrap_or(i64::MAX)
    }

    /// The tier `c` is wanted in at the current focus, if any.
    pub(super) fn wanted_tier(&self, c: ChunkCoord) -> Option<Tier> {
        let d = self.d2(c);
        if d <= self.residency.full2() {
            Some(Tier::Full)
        } else if d <= self.residency.lod2() {
            Some(Tier::LodOnly)
        } else {
            None
        }
    }

    /// Every chunk wanted at the current focus with its queue key, ascending.
    fn wanted(&self) -> Vec<Key> {
        let r = self.residency.r_lod.max(self.residency.r_full);
        let r2 = self.residency.lod2();
        let full2 = self.residency.full2();
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
                        let d2 = self.d2(coord);
                        let tier = if d2 <= full2 {
                            Tier::Full
                        } else {
                            Tier::LodOnly
                        };
                        keys.push(Key { tier, d2, coord });
                    }
                }
            }
        }
        keys.sort_unstable();
        keys
    }

    /// Whether `key` needs a job: its chunk is not resident at the wanted tier (a full chunk
    /// also serves a LOD-only want), has no job, and has not failed to load.
    fn needs_job(&self, key: &Key) -> bool {
        let resident = self.resident.get(&key.coord);
        let satisfied = match key.tier {
            Tier::Full => resident == Some(&Tier::Full),
            Tier::LodOnly => resident.is_some(),
        };
        !satisfied
            && !self.in_flight.contains_key(&key.coord)
            && !self.load_failed.contains(&key.coord)
    }

    /// The queue for the current focus, replacing `jobs`: every wanted chunk that needs a job,
    /// sorted descending by key so a worker's `pop` takes the nearest.
    pub(super) fn rebuild_jobs(&mut self, jobs: &mut Vec<Job>) {
        let wanted = self.wanted();
        // Jobs still queued are not running: forget them, then queue what is wanted now.
        for job in jobs.drain(..) {
            self.in_flight.remove(&job.key.coord);
        }
        let mut rebuilt = Vec::new();
        let mut still_wanted = HashSet::new();
        for key in wanted {
            if key.tier == Tier::Full {
                still_wanted.insert(key.coord);
            }
            if !self.needs_job(&key) {
                continue;
            }
            if key.tier == Tier::Full {
                self.wanted_since.entry(key.coord).or_insert(self.tick);
            }
            self.in_flight.insert(key.coord, key.tier);
            rebuilt.push(Job { key });
        }
        self.wanted_since.retain(|c, _| still_wanted.contains(c));
        rebuilt.reverse();
        *jobs = rebuilt;
    }

    /// The key of a load for `c` if the current focus wants it at a tier it lacks, with the
    /// load marked in flight; for chunks whose job finished without leaving them at their
    /// wanted tier. The caller queues it.
    pub(super) fn requeue_key(&mut self, c: ChunkCoord) -> Option<Key> {
        let tier = self.wanted_tier(c)?;
        let key = Key {
            tier,
            d2: self.d2(c),
            coord: c,
        };
        if !self.needs_job(&key) {
            return None;
        }
        if tier == Tier::Full {
            self.wanted_since.entry(c).or_insert(self.tick);
        }
        self.in_flight.insert(c, tier);
        Some(key)
    }

    /// Chunks inserted outside their band, for the next sweep. The flag is set after this
    /// poll's sweep ran, so the next poll sweeps them even when nothing else moves.
    pub(super) fn recheck_later(&mut self, outside: Vec<ChunkCoord>) {
        self.sweep |= !outside.is_empty();
        self.recheck.extend(outside);
    }

    /// The sweep's evictions and demotions, each sorted. A sweep after one-step moves of a
    /// single focus visits the chunks that crossed out of the keep balls since the last sweep
    /// and those inserted outside their band since: every other resident chunk is inside its
    /// band, as the last sweep left it.
    pub(super) fn sweep_lists(&mut self) -> (Vec<(ChunkCoord, Tier)>, Vec<ChunkCoord>) {
        let keep_full2 = self.residency.keep_full2();
        let keep_lod2 = self.residency.keep_lod2();
        let step = match (self.focus.as_slice(), self.swept_focus) {
            ([f], Some(g)) if !self.sweep_full => {
                Some((*f, [0, 1, 2].map(|k| f.0[k] - g.0[k]))).filter(|(_, d)| unit_step(*d))
            }
            _ => None,
        };
        let classify =
            |s: &Self, c: ChunkCoord, tier: Tier, evict: &mut Vec<_>, demote: &mut Vec<_>| {
                let d = s.d2(c);
                if d > keep_lod2 {
                    evict.push((c, tier));
                } else if tier == Tier::Full && d > keep_full2 {
                    demote.push(c);
                }
            };
        let full_scan = |s: &Self| {
            let (mut evict, mut demote) = (Vec::new(), Vec::new());
            for (&c, &tier) in &s.resident {
                classify(s, c, tier, &mut evict, &mut demote);
            }
            evict.sort_unstable();
            demote.sort_unstable();
            (evict, demote)
        };
        let lists = match step {
            Some((f, d)) => {
                let mut visit: HashSet<ChunkCoord> = std::mem::take(&mut self.recheck);
                if d != [0; 3] {
                    for r2 in [keep_lod2, keep_full2] {
                        for o in self.shell(r2, d, false).iter() {
                            visit.insert(ChunkCoord([0, 1, 2].map(|k| f.0[k] + o[k])));
                        }
                    }
                }
                let (mut evict, mut demote) = (Vec::new(), Vec::new());
                for c in visit {
                    if let Some(&tier) = self.resident.get(&c) {
                        classify(self, c, tier, &mut evict, &mut demote);
                    }
                }
                evict.sort_unstable();
                demote.sort_unstable();
                #[cfg(any(test, feature = "stream-oracle"))]
                if self.oracle {
                    let full = full_scan(self);
                    assert_eq!((&evict, &demote), (&full.0, &full.1), "incremental sweep");
                    self.oracle_checks.1 += 1;
                }
                (evict, demote)
            }
            None => full_scan(self),
        };
        self.recheck.clear();
        self.sweep_full = false;
        self.swept_focus = match self.focus.as_slice() {
            [f] => Some(*f),
            _ => None,
        };
        lists
    }

    /// Makes `coord` resident in `tier`, keeping the full count.
    pub(super) fn set_resident(&mut self, coord: ChunkCoord, tier: Tier) {
        let old = self.resident.insert(coord, tier);
        self.resident_full += usize::from(tier == Tier::Full);
        self.resident_full -= usize::from(old == Some(Tier::Full));
    }

    /// Makes `coord` not resident, keeping the full count.
    pub(super) fn unset_resident(&mut self, coord: ChunkCoord) {
        let old = self.resident.remove(&coord);
        self.resident_full -= usize::from(old == Some(Tier::Full));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(r_full: i32, r_lod: i32) -> ResidencyPlan {
        ResidencyPlan::new(Residency {
            r_full,
            r_lod,
            hysteresis: 1,
        })
    }

    #[test]
    fn queue_runs_nearest_first_with_ties_by_coordinate() {
        let mut p = plan(2, 2);
        p.focus = vec![ChunkCoord([0, 0, 0])];
        let order: Vec<[i32; 3]> = p.wanted().iter().map(|k| k.coord.0).collect();
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

        // The queue the workers pop from holds the same order.
        let mut jobs = Vec::new();
        p.rebuild_jobs(&mut jobs);
        let popped: Vec<[i32; 3]> = jobs.iter().rev().map(|j| j.key.coord.0).collect();
        assert_eq!(popped, order);
    }

    #[test]
    fn full_loads_come_before_every_lod_load() {
        let mut p = plan(1, 2);
        p.focus = vec![ChunkCoord([0, 0, 0])];
        let mut jobs = Vec::new();
        p.rebuild_jobs(&mut jobs);
        let tiers: Vec<Tier> = jobs.iter().rev().map(|j| j.key.tier).collect();
        assert_eq!(tiers.len(), 33);
        assert!(tiers[..7].iter().all(|&t| t == Tier::Full));
        assert!(tiers[7..].iter().all(|&t| t == Tier::LodOnly));
    }

    /// One focus stepped by one chunk along each kind of step (an axis, a face diagonal, a
    /// corner diagonal), with chunks resident, running and queued: the shifted queue, the
    /// in-flight set and `wanted_since` equal a rebuild's from the same state.
    #[test]
    fn a_one_step_shift_gives_the_rebuilds_queue() {
        let mut p = plan(2, 4);
        assert_eq!(p.refocus(&[VoxelCoord([0, 0, 0])]), Refocus::Jump);
        let mut jobs = Vec::new();
        p.rebuild_jobs(&mut jobs);
        // The nearest twelve jobs taken by workers: nine loaded, three still running.
        for i in 0..12 {
            let key = jobs.pop().expect("a queued job").key;
            if i < 9 {
                p.in_flight.remove(&key.coord);
                p.wanted_since.remove(&key.coord);
                p.set_resident(key.coord, key.tier);
            }
        }
        let mut at = [0, 0, 0];
        for d in [[1, 0, 0], [0, -1, 1], [1, 1, 1]] {
            at = [0, 1, 2].map(|k| at[k] + d[k]);
            assert_eq!(
                p.refocus(&[VoxelCoord(at.map(|c| c * 64))]),
                Refocus::Step(d)
            );
            let (mut fresh, mut fresh_jobs) = (p.clone(), jobs.clone());
            p.shift_jobs(&mut jobs, d);
            fresh.rebuild_jobs(&mut fresh_jobs);
            let keys = |jobs: &[Job]| jobs.iter().map(|j| j.key).collect::<Vec<_>>();
            assert!(!jobs.is_empty(), "step {d:?}");
            assert_eq!(keys(&jobs), keys(&fresh_jobs), "step {d:?}");
            assert_eq!(p.in_flight, fresh.in_flight, "step {d:?}");
            assert_eq!(p.wanted_since, fresh.wanted_since, "step {d:?}");
        }
    }
}
