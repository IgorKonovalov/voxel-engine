//! The attribute pass's work list (ADR-0011): which `Full` bricks of mirrored full chunks need
//! their contour attribute recomputed after a sync, built from the mirror's change sets over a
//! read-only view of its CPU model. Pure: no device, no queue, no `GpuWorld`.
//!
//! Three change sets feed it:
//! - chunks whose record was written whole: every brick is taken;
//! - re-uploaded bricks (world brick coordinates): each brick's 3 x 3 x 3 neighbourhood;
//! - shell chunks, whose record was written new, dropped or replaced: the bricks within one
//!   brick of the chunk's box that belong to other chunks. Only a neighbour chunk mirrored
//!   full can hold such a brick, so the shell costs one lookup per neighbour and visits only
//!   the bricks of full neighbours that border the chunk.
//!
//! Each brick enters the list at most once. A brick of a chunk taken whole is not visited again
//! by the other two sources.

use std::collections::HashSet;

use vxe_world::{BRICK_EDGE, CHUNK_BRICKS, CHUNK_EDGE_BRICKS, ChunkCoord, brick_of_slot};

use crate::attrib::AttribItem;

/// What the builder reads of the mirror's CPU model.
pub trait MirrorView {
    /// Whether `c` is mirrored from its full voxels (not a LOD record, not absent).
    fn is_full_chunk(&self, c: ChunkCoord) -> bool;
    /// The pool slot of world brick `b` if it is a `Full` brick of a mirrored full chunk.
    fn full_slot(&self, b: [i32; 3]) -> Option<u32>;
}

/// What one build did: the sizes of its inputs, the calls it made and the items it returned.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AttrWorkStats {
    /// Chunks taken whole.
    pub whole_chunks: u32,
    /// Re-uploaded bricks whose neighbourhoods were visited.
    pub dirty_bricks: u32,
    /// Shell chunks.
    pub shell_chunks: u32,
    /// `full_slot` calls for the bricks of chunks taken whole.
    pub item_calls: u64,
    /// Brick visits from the re-uploaded bricks' neighbourhoods.
    pub dirty_visits: u64,
    /// Neighbour-chunk lookups from the shells, 26 per shell chunk.
    pub neighbour_lookups: u64,
    /// Brick visits from the shells: the bordering bricks of full neighbours.
    pub shell_visits: u64,
    /// Hash-table operations: the view's lookups and the deduplication's.
    pub lookups: u64,
    /// Items returned.
    pub items: u32,
}

impl AttrWorkStats {
    /// Every brick visit, from both sources that visit.
    pub fn visits(&self) -> u64 {
        self.dirty_visits + self.shell_visits
    }
}

/// Appends to `items` the work list of the three change sets over `view`, and counts it.
pub fn build(
    view: &impl MirrorView,
    chunks: &HashSet<ChunkCoord>,
    bricks: &HashSet<[i32; 3]>,
    shells: &HashSet<ChunkCoord>,
    items: &mut Vec<AttribItem>,
) -> AttrWorkStats {
    let start = items.len();
    let mut st = AttrWorkStats {
        whole_chunks: chunks.len() as u32,
        dirty_bricks: bricks.len() as u32,
        shell_chunks: shells.len() as u32,
        ..Default::default()
    };
    whole_pass(view, chunks, &mut st, items);
    // Bricks outside the chunks taken whole, each once.
    let mut v = Visitor {
        view,
        chunks,
        seen: HashSet::new(),
    };
    dirty_pass(&mut v, bricks, &mut st, items);
    shell_pass(&mut v, shells, &mut st, items);
    st.items = (items.len() - start) as u32;
    st
}

/// The work-list item of world brick `b` at pool slot `slot`.
fn item(slot: u32, b: [i32; 3]) -> AttribItem {
    AttribItem {
        slot,
        origin: b.map(|v| v * BRICK_EDGE),
    }
}

/// Every brick of the chunks taken whole. Each pass is kept out of line so a profile
/// attributes each source's cost to its own symbol.
#[inline(never)]
fn whole_pass(
    view: &impl MirrorView,
    chunks: &HashSet<ChunkCoord>,
    st: &mut AttrWorkStats,
    items: &mut Vec<AttribItem>,
) {
    let e = CHUNK_EDGE_BRICKS;
    for &c in chunks {
        for i in 0..CHUNK_BRICKS {
            let b = brick_of_slot(i);
            let b = [0, 1, 2].map(|k| c.0[k] * e + b[k]);
            st.item_calls += 1;
            st.lookups += 1;
            if let Some(s) = view.full_slot(b) {
                items.push(item(s, b));
            }
        }
    }
}

/// Visits bricks outside the chunks taken whole, each once.
struct Visitor<'a, V> {
    view: &'a V,
    chunks: &'a HashSet<ChunkCoord>,
    seen: HashSet<[i32; 3]>,
}

impl<V: MirrorView> Visitor<'_, V> {
    fn visit(&mut self, b: [i32; 3], st: &mut AttrWorkStats, items: &mut Vec<AttribItem>) {
        let c = ChunkCoord(b.map(|v| v.div_euclid(CHUNK_EDGE_BRICKS)));
        st.lookups += 1;
        if self.chunks.contains(&c) {
            return;
        }
        self.visit_outside_whole(b, st, items);
    }

    /// [`Visitor::visit`] for a brick already known to lie outside the chunks taken whole.
    fn visit_outside_whole(
        &mut self,
        b: [i32; 3],
        st: &mut AttrWorkStats,
        items: &mut Vec<AttribItem>,
    ) {
        st.lookups += 1;
        if !self.seen.insert(b) {
            return;
        }
        st.lookups += 1;
        if let Some(s) = self.view.full_slot(b) {
            items.push(item(s, b));
        }
    }
}

/// The 3 x 3 x 3 neighbourhood of each re-uploaded brick.
#[inline(never)]
fn dirty_pass<V: MirrorView>(
    v: &mut Visitor<'_, V>,
    bricks: &HashSet<[i32; 3]>,
    st: &mut AttrWorkStats,
    items: &mut Vec<AttribItem>,
) {
    for &b in bricks {
        for z in -1..=1 {
            for y in -1..=1 {
                for x in -1..=1 {
                    st.dirty_visits += 1;
                    v.visit([b[0] + x, b[1] + y, b[2] + z], st, items);
                }
            }
        }
    }
}

/// The 26 neighbour offsets of a chunk.
const NEIGHBOURS: [[i32; 3]; 26] = {
    let mut out = [[0; 3]; 26];
    let mut n = 0;
    let mut i = 0;
    while i < 27 {
        let d = [i % 3 - 1, i / 3 % 3 - 1, i / 9 - 1];
        if d[0] != 0 || d[1] != 0 || d[2] != 0 {
            out[n] = d;
            n += 1;
        }
        i += 1;
    }
    out
};

/// The neighbours' bricks within one brick of each shell chunk's box: for each of the chunk's
/// 26 neighbour chunks mirrored full, the bricks of that neighbour that border the chunk (64
/// across a face, 8 along an edge, 1 at a corner). A neighbour that is not full, or that is
/// taken whole, holds none of the list's bricks, so it costs one lookup.
#[inline(never)]
fn shell_pass<V: MirrorView>(
    v: &mut Visitor<'_, V>,
    shells: &HashSet<ChunkCoord>,
    st: &mut AttrWorkStats,
    items: &mut Vec<AttribItem>,
) {
    let e = CHUNK_EDGE_BRICKS;
    for &c in shells {
        for d in NEIGHBOURS {
            let n = ChunkCoord([0, 1, 2].map(|k| c.0[k] + d[k]));
            st.neighbour_lookups += 1;
            st.lookups += 1;
            if !v.view.is_full_chunk(n) {
                continue;
            }
            st.lookups += 1;
            if v.chunks.contains(&n) {
                continue;
            }
            // The bricks of `n` on the side facing `c`, per axis: its last layer when `c`
            // lies below, its first when above, all of them when level.
            let span = d.map(|dk| match dk {
                -1 => e - 1..e,
                1 => 0..1,
                _ => 0..e,
            });
            for z in span[2].clone() {
                for y in span[1].clone() {
                    for x in span[0].clone() {
                        st.shell_visits += 1;
                        let b = [n.0[0] * e + x, n.0[1] * e + y, n.0[2] * e + z];
                        v.visit_outside_whole(b, st, items);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    /// The fly-through's residency around the origin, worst case: every chunk within 17
    /// chunks is full with every brick `Full`, every other chunk within 25 is LOD-only.
    struct Synthetic {
        /// Full or LOD-only, and the chunk's first slot.
        chunks: HashMap<ChunkCoord, (bool, u32)>,
    }

    fn d2(c: [i32; 3]) -> i32 {
        c.iter().map(|v| v * v).sum()
    }

    fn ball(r2: i32, at: [i32; 3]) -> HashSet<ChunkCoord> {
        let r = (f64::from(r2)).sqrt() as i32 + 1;
        let mut out = HashSet::new();
        for z in -r..=r {
            for y in -r..=r {
                for x in -r..=r {
                    if d2([x, y, z]) <= r2 {
                        out.insert(ChunkCoord([x + at[0], y + at[1], z + at[2]]));
                    }
                }
            }
        }
        out
    }

    impl Synthetic {
        fn new() -> Self {
            let mut chunks = HashMap::new();
            let mut coords: Vec<ChunkCoord> = ball(25 * 25, [0; 3]).into_iter().collect();
            coords.sort_by_key(|c| c.0);
            for (n, c) in coords.into_iter().enumerate() {
                chunks.insert(c, (d2(c.0) <= 17 * 17, n as u32 * CHUNK_BRICKS as u32));
            }
            Self { chunks }
        }
    }

    impl MirrorView for Synthetic {
        fn is_full_chunk(&self, c: ChunkCoord) -> bool {
            self.chunks.get(&c).is_some_and(|m| m.0)
        }

        fn full_slot(&self, b: [i32; 3]) -> Option<u32> {
            let e = CHUNK_EDGE_BRICKS;
            let c = ChunkCoord(b.map(|v| v.div_euclid(e)));
            let &(full, first) = self.chunks.get(&c).filter(|m| m.0)?;
            debug_assert!(full);
            Some(first + vxe_world::brick_slot(b.map(|v| v.rem_euclid(e))) as u32)
        }
    }

    /// The chunks a +x focus step evicts at the fly-through's residency: ball(25) minus its
    /// +x shift.
    fn evicted() -> HashSet<ChunkCoord> {
        let shifted = ball(25 * 25, [1, 0, 0]);
        ball(25 * 25, [0; 3])
            .into_iter()
            .filter(|c| !shifted.contains(c))
            .collect()
    }

    /// The chunks a +x focus step promotes: ball(16) shifted +x, minus ball(16).
    fn promoted() -> HashSet<ChunkCoord> {
        let before = ball(16 * 16, [0; 3]);
        ball(16 * 16, [1, 0, 0])
            .into_iter()
            .filter(|c| !before.contains(c))
            .collect()
    }

    #[test]
    fn an_eviction_frame_looks_up_each_neighbour_once_and_visits_nothing() {
        let view = Synthetic::new();
        let shells = evicted();
        assert_eq!(shells.len(), 1961);
        let mut items = Vec::new();
        let st = build(&view, &HashSet::new(), &HashSet::new(), &shells, &mut items);
        assert_eq!(st.shell_chunks, 1961);
        assert_eq!(st.neighbour_lookups, 1961 * 26);
        assert_eq!(st.neighbour_lookups, 50_986);
        assert_eq!(st.shell_visits, 0);
        assert_eq!(st.visits(), 0);
        assert_eq!(st.items, 0);
        assert!(items.is_empty());
    }

    #[test]
    fn the_volume_shell_visited_every_shell_brick_of_an_eviction_frame() {
        let view = Synthetic::new();
        let shells = evicted();
        let mut items = Vec::new();
        let visits = reference::build(&view, &HashSet::new(), &HashSet::new(), &shells, &mut items);
        assert_eq!(visits, 1961 * 488);
        assert_eq!(visits, 956_968);
        assert!(items.is_empty());
    }

    #[test]
    fn a_promotion_frame_takes_every_brick_of_every_promoted_chunk() {
        let view = Synthetic::new();
        let chunks = promoted();
        assert_eq!(chunks.len(), 797);
        let mut items = Vec::new();
        let st = build(&view, &chunks, &HashSet::new(), &HashSet::new(), &mut items);
        assert_eq!(st.item_calls, 408_064);
        assert_eq!(st.items, 408_064);
        assert_eq!(items.len(), 408_064);
        assert_eq!(st.visits(), 0);
    }

    /// The volume-shell builder: every brick of a one-brick shell around each shell chunk,
    /// 10^3 - 8^3 = 488 per chunk. The oracle the neighbour shell is held to.
    mod reference {
        use std::collections::HashSet;

        use vxe_world::{BRICK_EDGE, CHUNK_BRICKS, CHUNK_EDGE_BRICKS, ChunkCoord, brick_of_slot};

        use super::super::MirrorView;
        use crate::attrib::AttribItem;

        /// Appends the work list to `items`; returns the shell's brick visits.
        pub fn build(
            view: &impl MirrorView,
            chunks: &HashSet<ChunkCoord>,
            bricks: &HashSet<[i32; 3]>,
            shells: &HashSet<ChunkCoord>,
            items: &mut Vec<AttribItem>,
        ) -> u64 {
            let e = CHUNK_EDGE_BRICKS;
            let item = |view: &dyn Fn([i32; 3]) -> Option<u32>, b: [i32; 3]| {
                view(b).map(|slot| AttribItem {
                    slot,
                    origin: b.map(|v| v * BRICK_EDGE),
                })
            };
            let slot = |b| view.full_slot(b);
            for &c in chunks {
                for i in 0..CHUNK_BRICKS {
                    let b = brick_of_slot(i);
                    items.extend(item(&slot, [0, 1, 2].map(|k| c.0[k] * e + b[k])));
                }
            }
            let mut seen = HashSet::new();
            let mut visit = |b: [i32; 3], items: &mut Vec<AttribItem>| {
                let c = ChunkCoord(b.map(|v| v.div_euclid(e)));
                if !chunks.contains(&c) && seen.insert(b) {
                    items.extend(item(&slot, b));
                }
            };
            for &b in bricks {
                for z in -1..=1 {
                    for y in -1..=1 {
                        for x in -1..=1 {
                            visit([b[0] + x, b[1] + y, b[2] + z], items);
                        }
                    }
                }
            }
            let mut visits = 0;
            for &c in shells {
                let lo = c.0.map(|v| v * e - 1);
                let hi = c.0.map(|v| v * e + e);
                for z in lo[2]..=hi[2] {
                    for y in lo[1]..=hi[1] {
                        for x in lo[0]..=hi[0] {
                            let p = [x, y, z];
                            if (0..3).any(|k| p[k] == lo[k] || p[k] == hi[k]) {
                                visits += 1;
                                visit(p, items);
                            }
                        }
                    }
                }
            }
            visits
        }
    }

    /// A seeded small view: a 6^3-chunk region with a mix of full, LOD-only and absent chunks,
    /// and of `Full` and uniform bricks in the full ones. The view reads LOD-only and absent
    /// chunks alike (not full), so only the full ones are stored.
    struct Small {
        /// Per full chunk, the slot of each `Full` brick (`None` for a uniform one).
        full: HashMap<ChunkCoord, Vec<Option<u32>>>,
    }

    /// A splitmix stream.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            vxe_world::hash::mix64(self.0)
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    const SIDE: i32 = 6;

    impl Small {
        fn new(rng: &mut Rng) -> Self {
            let mut full = HashMap::new();
            let mut slot = 0;
            for z in 0..SIDE {
                for y in 0..SIDE {
                    for x in 0..SIDE {
                        let c = ChunkCoord([x, y, z]);
                        // 0 absent, 1 LOD-only, 2 full.
                        match rng.below(3) {
                            0 | 1 => {}
                            _ => {
                                let bricks = (0..CHUNK_BRICKS)
                                    .map(|_| {
                                        (rng.below(2) == 0).then(|| {
                                            slot += 1;
                                            slot
                                        })
                                    })
                                    .collect();
                                full.insert(c, bricks);
                            }
                        }
                    }
                }
            }
            Self { full }
        }
    }

    impl MirrorView for Small {
        fn is_full_chunk(&self, c: ChunkCoord) -> bool {
            self.full.contains_key(&c)
        }

        fn full_slot(&self, b: [i32; 3]) -> Option<u32> {
            let e = CHUNK_EDGE_BRICKS;
            let c = ChunkCoord(b.map(|v| v.div_euclid(e)));
            self.full.get(&c)?[vxe_world::brick_slot(b.map(|v| v.rem_euclid(e)))]
        }
    }

    fn sorted(items: &[AttribItem]) -> Vec<(u32, [i32; 3])> {
        let mut v: Vec<_> = items.iter().map(|i| (i.slot, i.origin)).collect();
        v.sort_unstable();
        v
    }

    #[test]
    fn the_neighbour_shell_lists_what_the_volume_shell_listed() {
        let e = CHUNK_EDGE_BRICKS;
        let mut shell_items = 0;
        for seed in 0..64u64 {
            let mut rng = Rng(seed);
            let view = Small::new(&mut rng);
            let any_chunk =
                |rng: &mut Rng| ChunkCoord([0; 3].map(|_| rng.below(SIDE as u64 + 2) as i32 - 1));
            let chunks: HashSet<ChunkCoord> =
                (0..rng.below(4)).map(|_| any_chunk(&mut rng)).collect();
            let bricks: HashSet<[i32; 3]> = (0..rng.below(40))
                .map(|_| [0; 3].map(|_| rng.below((SIDE * e) as u64 + 2) as i32 - 1))
                .collect();
            let shells: HashSet<ChunkCoord> =
                (0..rng.below(30)).map(|_| any_chunk(&mut rng)).collect();
            let mut got = Vec::new();
            build(&view, &chunks, &bricks, &shells, &mut got);
            let mut want = Vec::new();
            reference::build(&view, &chunks, &bricks, &shells, &mut want);
            assert_eq!(sorted(&got), sorted(&want), "seed {seed}");
            assert_eq!(got.len(), want.len(), "seed {seed}: a duplicate item");
            // The shells alone, so the comparison covers the path this builder changed.
            let none_c = HashSet::new();
            let none_b = HashSet::new();
            let mut got = Vec::new();
            build(&view, &none_c, &none_b, &shells, &mut got);
            let mut want = Vec::new();
            reference::build(&view, &none_c, &none_b, &shells, &mut want);
            assert_eq!(sorted(&got), sorted(&want), "seed {seed}, shells alone");
            shell_items += got.len();
        }
        println!("shell-only items over 64 seeds: {shell_items}");
        assert!(shell_items > 1000, "the shells listed {shell_items} items");
    }
}
