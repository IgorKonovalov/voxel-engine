//! The queue's side of the stream workers' no-face verdicts ([`crate::bake::seal`]): a chunk
//! the workers sealed is no candidate, so the pool admits it already known faceless and the
//! main thread never lists it. Each verdict carries its point of the shells' order and the
//! later one holds; an edit forgets the shells of the chunks it changed and clears the
//! verdicts within a voxel of it at a new point, so a verdict read before the edit does not
//! hold.

use std::collections::HashMap;

use super::*;
use crate::bake::seal::{self, SharedShells, Verdict};

/// The verdicts the queue holds and the shells' handle.
#[derive(Debug, Default)]
pub(crate) struct Sealed {
    verdicts: HashMap<[i32; 3], (bool, u64)>,
    shells: Option<SharedShells>,
    /// Edits' boxes before a handle arrived (no shell order to place them in yet): forgotten
    /// in the shells when one does, their verdicts held unsealed meanwhile.
    pending: Vec<[[i32; 3]; 2]>,
    /// Verdicts held that read sealed.
    count: u64,
}

/// Applies `verdicts` to `held` (each chunk's verdict and its point of the order): a verdict
/// later than the one held replaces it. The chunks whose sealing changed, and the change to
/// the sealed count.
pub(crate) fn apply(
    held: &mut HashMap<[i32; 3], (bool, u64)>,
    verdicts: &[Verdict],
) -> (Vec<[i32; 3]>, i64) {
    let mut changed = Vec::new();
    let mut delta = 0;
    for v in verdicts {
        let e = held.entry(v.c).or_insert((false, 0));
        if v.seq <= e.1 {
            continue;
        }
        if e.0 != v.sealed {
            changed.push(v.c);
            delta += if v.sealed { 1 } else { -1 };
        }
        *e = (v.sealed, v.seq);
    }
    (changed, delta)
}

/// The chunks an edit's box changed (`edited`) and those within a voxel of it (`near`).
fn box_chunks(min: [i32; 3], max: [i32; 3]) -> (Vec<[i32; 3]>, Vec<[i32; 3]>) {
    let span = |lo: [i32; 3], hi: [i32; 3]| {
        let mut out = Vec::new();
        for z in lo[2]..=hi[2] {
            for y in lo[1]..=hi[1] {
                for x in lo[0]..=hi[0] {
                    out.push([x, y, z]);
                }
            }
        }
        out
    };
    (
        span(min.map(|v| v.div_euclid(64)), max.map(|v| v.div_euclid(64))),
        span(
            min.map(|v| (v - 1).div_euclid(64)),
            max.map(|v| (v + 1).div_euclid(64)),
        ),
    )
}

impl Sealed {
    /// Whether the workers sealed `c` (no exposed face).
    pub(crate) fn holds(&self, c: [i32; 3]) -> bool {
        self.verdicts.get(&c).is_some_and(|v| v.0)
    }

    /// Chunks held sealed.
    pub(crate) fn count(&self) -> u64 {
        self.count
    }

    /// Takes a worker result's verdicts (and its shells' handle, the first time or when the
    /// hook changed): the chunks whose sealing changed.
    pub(crate) fn take(&mut self, m: &PreparedMasks) -> Vec<[i32; 3]> {
        let mut changed = Vec::new();
        if self
            .shells
            .as_ref()
            .is_none_or(|s| !std::sync::Arc::ptr_eq(s, &m.shells))
        {
            if self.shells.is_some() {
                // A new hook's order starts again: what the old one sealed does not hold.
                changed.extend(self.verdicts.drain().filter(|e| e.1.0).map(|e| e.0));
                self.count = 0;
            }
            self.shells = Some(m.shells.clone());
            for [min, max] in std::mem::take(&mut self.pending) {
                self.forget(min, max);
            }
        }
        let (c, delta) = apply(&mut self.verdicts, &m.verdicts);
        self.count = self.count.saturating_add_signed(delta);
        changed.extend(c);
        changed
    }

    /// An edit's box (`min ..= max`, voxels): the edited chunks' shells go and the verdicts
    /// within a voxel of it read unsealed from a new point of the order.
    pub(crate) fn forget(&mut self, min: [i32; 3], max: [i32; 3]) {
        let (edited, near) = box_chunks(min, max);
        let at = match &self.shells {
            Some(s) => seal::lock(s).forget(&edited, &near),
            None => {
                self.pending.push([min, max]);
                u64::MAX
            }
        };
        for c in near {
            let e = self.verdicts.entry(c).or_insert((false, 0));
            if e.0 {
                self.count -= 1;
            }
            // A pending edit's `u64::MAX` gives way to the point its forget takes later.
            *e = (false, if e.1 == u64::MAX { at } else { at.max(e.1) });
        }
    }

    /// When the verdicts outnumber twice the mirror's chunks (and a floor), keeps only those of
    /// chunks `keep` holds, in the queue and in the workers' shells.
    pub(crate) fn prune(&mut self, mirrored: usize, keep: impl Fn([i32; 3]) -> bool) {
        if self.verdicts.len() <= 2 * mirrored + 8192 || !self.pending.is_empty() {
            return;
        }
        self.verdicts.retain(|c, _| keep(*c));
        self.count = self.verdicts.values().filter(|v| v.0).count() as u64;
        if let Some(s) = &self.shells {
            seal::lock(s).prune(&keep);
        }
    }

    /// Shells held by the workers' map and their bytes (0, 0 with no handle yet).
    pub(crate) fn shells(&self) -> (usize, usize) {
        self.shells.as_ref().map_or((0, 0), |s| {
            let s = seal::lock(s);
            (s.len(), s.bytes())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(c: [i32; 3], sealed: bool, seq: u64) -> Verdict {
        Verdict { c, sealed, seq }
    }

    /// The later verdict on a chunk holds whatever the order the results arrive in; an earlier
    /// one arriving late changes nothing.
    #[test]
    fn the_later_verdict_holds_whatever_the_arrival_order() {
        let mut held = HashMap::new();
        let a = [1, 2, 3];
        let (ch, d) = apply(&mut held, &[v(a, true, 7)]);
        assert_eq!((ch, d), (vec![a], 1));
        let (ch, d) = apply(&mut held, &[v(a, false, 5)]);
        assert!(ch.is_empty() && d == 0, "an earlier verdict arriving late");
        assert_eq!(held[&a], (true, 7));
        let (ch, d) = apply(&mut held, &[v(a, false, 9)]);
        assert_eq!((ch, d), (vec![a], -1));
        let (ch, d) = apply(&mut held, &[v(a, false, 11), v([0, 0, 0], false, 11)]);
        assert!(ch.is_empty() && d == 0, "no change in sealing");
    }

    /// An edit before the handle arrives holds its chunks unsealed past every verdict until the
    /// handle's forget gives them a point of the order; a verdict after that point applies.
    #[test]
    fn an_edit_before_the_handle_holds_its_chunks_unsealed() {
        let mut s = Sealed::default();
        s.forget([0, 0, 0], [3, 3, 3]);
        assert_eq!(s.pending.len(), 1);
        let shells: SharedShells = Default::default();
        // Three loads before the result: points 1 to 3 of the order.
        for _ in 0..3 {
            seal::lock(&shells).admit([9, 9, 9], seal::Shell::of(&rock_masks()));
        }
        let m = PreparedMasks {
            classes: Default::default(),
            masks: rock_masks(),
            ns: 0,
            seal_ns: 0,
            verdicts: vec![v([0, 0, 0], true, 2), v([5, 5, 5], true, 3)],
            shells: shells.clone(),
        };
        let changed = s.take(&m);
        assert!(s.pending.is_empty());
        assert!(
            !s.holds([0, 0, 0]),
            "a verdict from before the edit's forget"
        );
        assert!(s.holds([5, 5, 5]));
        assert_eq!(changed, vec![[5, 5, 5]]);
        assert_eq!(s.count(), 1);
        // The forget took point 4: a later verdict applies.
        let (ch, _) = apply(&mut s.verdicts, &[v([0, 0, 0], true, 5)]);
        assert_eq!(ch, vec![[0, 0, 0]]);
    }

    fn rock_masks() -> crate::bake::faces::ChunkMasks {
        let mut classes = vec![0u8; 1 << 16];
        classes[1] = 1;
        crate::bake::faces::ChunkMasks::of(&svet_world::Chunk::uniform(1), &classes)
    }
}
