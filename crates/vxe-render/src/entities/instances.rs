//! The instances and the tree over the entities' boxes, built on the CPU each call
//! ([`EntityInstances::write`]); [`super::EntityMirror`] uploads the words.
//!
//! **Stable slots.** An entity's parts take a run of instance slots from a [`RangeAlloc`] over
//! the [`MAX_ENTITY_PARTS`] slots when it is first listed, and keep it while it stays listed
//! with the same part count; one that leaves the list frees its run. Each slot carries a
//! generation, bumped at most once a write when its occupant changes (a free, or a move by
//! compaction), and an instance's word 25 is `slot | generation << INSTANCE_SLOT_BITS`, which
//! `entity.wgsl` puts in the hit record. The record keeps the slot and the generation's low
//! bit ([`INSTANCE_RECORD_BITS`]): a generation moves at most once between two writes, so two
//! consecutive frames' records of a reused slot always differ in that bit. When no run fits a
//! new entity although enough slots are free, the live runs are packed down from slot 0 in
//! slot order (compaction); every moved slot's generation bumps.
//!
//! **Ghost leaves.** For one write after an entity's run is freed, the tree carries a leaf over
//! its last box with no parts and [`LEAF_MOVED`] set, so a shadow ray crossing where it stood
//! marks its pixel fresh. Ghosts take the tree's spare leaves: past [`MAX_ENTITIES`] leaves in
//! all, the rest are dropped.
//!
//! **No allocation per write** once the scratch has grown to the largest list: the runs live in
//! an ordered map changed only when an entity comes or goes, and every buffer is kept.

use std::collections::BTreeMap;

use vxe_entity::{EntityId, EntitySet};

use super::alloc::RangeAlloc;
use super::display::{DisplayPose, PartXform};
use super::pool::{ModelHandle, VolumePool};
use super::{
    INSTANCE_MOVED, INSTANCE_WORD, INSTANCE_WORDS, LEAF_BIT, LEAF_MOVED, MAX_ENTITIES,
    MAX_ENTITY_PARTS, NODE_WORDS, TREE_HEADER_WORD,
};

/// Bits of an instance's slot in its word 25 and in the hit record's instance field.
pub(crate) const INSTANCE_SLOT_BITS: u32 = 9;
/// Bits of the hit record's instance field: the slot and the generation's low bit.
pub(crate) const INSTANCE_RECORD_BITS: u32 = INSTANCE_SLOT_BITS + 1;
// The record's instance field sits above the hit record flags' bit 6, in their 16 bits
// (`hit.wgsl`), and the slot bits name every slot.
const _: () =
    assert!(6 + INSTANCE_RECORD_BITS <= 16 && 1 << INSTANCE_SLOT_BITS == MAX_ENTITY_PARTS);
/// A slot's generation wraps at this (17 bits: word 25 above the slot).
const GENERATION_MASK: u32 = (1 << 17) - 1;

/// One instance slot: its generation, the write that last bumped it, the part-to-world it was
/// drawn with at the last write, and whether it was allocated at this write (no transform of
/// the write before).
#[derive(Clone, Copy, Debug, Default)]
struct Slot {
    generation: u32,
    bumped: u64,
    prev: [[f32; 4]; 3],
    fresh: bool,
}

/// An entity's run of slots, the write that last listed it, and its box at that write.
#[derive(Clone, Copy, Debug)]
struct Run {
    first: u16,
    count: u16,
    seen: u64,
    bx: [[f32; 3]; 2],
}

/// A listed entity this write: its id, handle, and where its parts' placements start in the
/// kept placements.
#[derive(Clone, Copy, Debug)]
struct Listed {
    id: EntityId,
    handle: ModelHandle,
    at: u32,
    count: u16,
}

/// The instances and the tree of the last write, the slots and runs they were drawn in, and
/// what the write drew and counted.
pub struct EntityInstances {
    slots: Vec<Slot>,
    free: RangeAlloc,
    runs: BTreeMap<EntityId, Run>,
    write: u64,
    listed: Vec<Listed>,
    placements: Vec<(PartXform, u8)>,
    gone: Vec<EntityId>,
    ghosts: Vec<[[f32; 3]; 2]>,
    order_runs: Vec<(u16, EntityId)>,
    /// Every slot's words; the upload covers `[0, high * INSTANCE_WORDS)`.
    inst: Vec<u32>,
    high: usize,
    boxes: Vec<([[f32; 3]; 2], u32, u32)>,
    nodes: Vec<[u32; NODE_WORDS]>,
    order: Vec<usize>,
    tree: Vec<u32>,
    drawn: usize,
    parts: usize,
    over: usize,
    moved: bool,
}

impl Default for EntityInstances {
    fn default() -> Self {
        Self::new()
    }
}

impl EntityInstances {
    pub fn new() -> Self {
        Self {
            slots: vec![Slot::default(); MAX_ENTITY_PARTS],
            free: RangeAlloc::new(MAX_ENTITY_PARTS as u32),
            runs: BTreeMap::new(),
            write: 0,
            listed: Vec::new(),
            placements: Vec::new(),
            gone: Vec::new(),
            ghosts: Vec::new(),
            order_runs: Vec::new(),
            inst: vec![0; MAX_ENTITY_PARTS * INSTANCE_WORDS],
            high: 0,
            boxes: Vec::new(),
            nodes: Vec::new(),
            order: Vec::new(),
            tree: vec![0; INSTANCE_WORD],
            drawn: 0,
            parts: 0,
            over: 0,
            moved: false,
        }
    }

    /// [`EntityInstances::write`] with each entity's parts placed as `set` poses them.
    pub fn set_entities(
        &mut self,
        pool: &VolumePool,
        set: &EntitySet,
        list: &[(EntityId, ModelHandle)],
    ) {
        let shown: Vec<_> = list
            .iter()
            .map(|&(id, h)| {
                let d = set.get(id).and_then(|(m, p)| DisplayPose::of(m, p));
                (id, h, d)
            })
            .collect();
        self.write(pool, shown.iter().map(|(id, h, d)| (*id, *h, d.as_ref())));
    }

    /// The listed entities' instances, each in its run of slots (module docs), and the tree
    /// over their boxes and the ghosts of those that left. An entity past [`MAX_ENTITIES`], or
    /// whose parts would pass [`MAX_ENTITY_PARTS`] over the list so far, is counted and not
    /// drawn; so is an entry with no display pose, one whose handle names no model of `pool`,
    /// one whose display pose has not its model's part count, and an entity listed twice. An
    /// entity's box holds its parts now and at the write before.
    pub fn write<'a>(
        &mut self,
        pool: &VolumePool,
        list: impl Iterator<Item = (EntityId, ModelHandle, Option<&'a DisplayPose>)>,
    ) {
        self.write += 1;
        let now = self.write;
        self.listed.clear();
        self.placements.clear();
        let (mut over, mut parts) = (0usize, 0usize);
        for (id, handle, shown) in list {
            let fits = shown
                .zip(pool.model(handle))
                .filter(|(d, v)| d.parts.len() == v.len());
            let Some((shown, _)) = fits else {
                over += 1;
                continue;
            };
            let n = shown.parts.len();
            if self.listed.len() == MAX_ENTITIES
                || parts + n > MAX_ENTITY_PARTS
                || self.listed.iter().any(|l| l.id == id)
            {
                over += 1;
                continue;
            }
            self.listed.push(Listed {
                id,
                handle,
                at: self.placements.len() as u32,
                count: n as u16,
            });
            self.placements.extend_from_slice(&shown.parts);
            parts += n;
        }
        self.keep_and_free(now);
        self.allocate(now);
        self.fill(pool);
        self.over = over;
    }

    /// Marks the runs of the entities listed again with their part count, and frees every
    /// other run, keeping its box as a ghost.
    fn keep_and_free(&mut self, now: u64) {
        for l in &self.listed {
            if let Some(r) = self.runs.get_mut(&l.id)
                && r.count == l.count
            {
                r.seen = now;
            }
        }
        self.gone.clear();
        self.gone.extend(
            self.runs
                .iter()
                .filter(|(_, r)| r.seen != now)
                .map(|(&id, _)| id),
        );
        self.ghosts.clear();
        for i in 0..self.gone.len() {
            let r = self.runs.remove(&self.gone[i]).expect("a run listed above");
            for s in r.first..r.first + r.count {
                self.bump(usize::from(s), now);
            }
            self.free.free(u32::from(r.first), u32::from(r.count));
            self.ghosts.push(r.bx);
        }
    }

    /// A run for each listed entity without one, in the list's order, compacting the table
    /// when no free run is long enough.
    fn allocate(&mut self, now: u64) {
        for i in 0..self.listed.len() {
            let l = self.listed[i];
            if self.runs.contains_key(&l.id) {
                continue;
            }
            let n = u32::from(l.count);
            let first = match self.free.alloc(n) {
                Some(f) => f,
                None => {
                    self.compact(now);
                    // The listed parts are at most the table's slots, so packed they fit.
                    self.free.alloc(n).expect("the listed parts fit the table")
                }
            };
            for s in &mut self.slots[first as usize..(first + n) as usize] {
                s.fresh = true;
            }
            self.runs.insert(
                l.id,
                Run {
                    first: first as u16,
                    count: l.count,
                    seen: now,
                    bx: [[0.0; 3]; 2],
                },
            );
        }
    }

    /// Packs the live runs down from slot 0 in slot order, each slot's transform moving with
    /// it; every slot a run leaves or enters bumps its generation.
    fn compact(&mut self, now: u64) {
        self.order_runs.clear();
        self.order_runs
            .extend(self.runs.iter().map(|(&id, r)| (r.first, id)));
        self.order_runs.sort_unstable();
        self.free = RangeAlloc::new(MAX_ENTITY_PARTS as u32);
        for i in 0..self.order_runs.len() {
            let (old, id) = self.order_runs[i];
            let count = self.runs[&id].count;
            let new = self
                .free
                .alloc(u32::from(count))
                .expect("packed at or below where it was") as u16;
            if new == old {
                continue;
            }
            // `new < old`: copying upward from the run's first slot reads each source slot
            // before a later copy overwrites it.
            for k in 0..usize::from(count) {
                let (o, n) = (usize::from(old) + k, usize::from(new) + k);
                let (prev, fresh) = (self.slots[o].prev, self.slots[o].fresh);
                self.bump(o, now);
                self.bump(n, now);
                self.slots[n].prev = prev;
                self.slots[n].fresh = fresh;
            }
            self.runs.get_mut(&id).expect("listed").first = new;
        }
    }

    /// Bumps slot `s`'s generation, once a write.
    fn bump(&mut self, s: usize, now: u64) {
        let slot = &mut self.slots[s];
        if slot.bumped != now {
            slot.generation = (slot.generation + 1) & GENERATION_MASK;
            slot.bumped = now;
        }
    }

    /// Writes each listed entity's instances into its slots and builds the tree over the
    /// entities' boxes and the ghosts.
    fn fill(&mut self, pool: &VolumePool) {
        let Self {
            slots,
            runs,
            listed,
            placements,
            ghosts,
            inst,
            high,
            boxes,
            nodes,
            order,
            tree,
            ..
        } = self;
        boxes.clear();
        *high = 0;
        let mut moved = false;
        let mut parts = 0;
        for l in listed.iter() {
            let vols = pool.model(l.handle).expect("checked when listed");
            let run = runs
                .get_mut(&l.id)
                .expect("allocated for every listed entity");
            let first = usize::from(run.first);
            let n = usize::from(l.count);
            let mut entity_moved = false;
            let mut bx = [[f32::MAX; 3], [f32::MIN; 3]];
            for k in 0..n {
                let (xf, detail) = &placements[l.at as usize + k];
                let vol = vols[k].1;
                let s = &mut slots[first + k];
                let before = if s.fresh { xf.part_to_world } else { s.prev };
                let part_moved = before != xf.part_to_world;
                entity_moved |= part_moved;
                for b in [
                    PartXform::world_box(&xf.part_to_world, vol.size),
                    PartXform::world_box(&before, vol.size),
                ] {
                    for r in 0..3 {
                        bx[0][r] = bx[0][r].min(b[0][r]);
                        bx[1][r] = bx[1][r].max(b[1][r]);
                    }
                }
                (s.prev, s.fresh) = (xf.part_to_world, false);
                let w = &mut inst[(first + k) * INSTANCE_WORDS..(first + k + 1) * INSTANCE_WORDS];
                for (j, v) in xf.world_to_part.iter().chain(&before).flatten().enumerate() {
                    w[j] = v.to_bits();
                }
                w[24..].copy_from_slice(&[
                    vol.word,
                    (first + k) as u32 | s.generation << INSTANCE_SLOT_BITS,
                    u32::from(part_moved) * INSTANCE_MOVED,
                    (1.0 / f32::from(*detail)).to_bits(),
                    vol.size[0],
                    vol.size[1],
                    vol.size[2],
                    0,
                ]);
            }
            run.bx = bx;
            *high = (*high).max(first + n);
            parts += n;
            moved |= entity_moved;
            let count = n as u32 | if entity_moved { LEAF_MOVED } else { 0 };
            boxes.push((bx, first as u32, count));
        }
        let drawn = boxes.len();
        for g in ghosts.iter() {
            if boxes.len() == MAX_ENTITIES {
                break;
            }
            boxes.push((*g, 0, LEAF_MOVED));
        }
        nodes.clear();
        if !boxes.is_empty() {
            order.clear();
            order.extend(0..boxes.len());
            build(boxes, order, nodes);
        }
        tree.fill(0);
        for (i, n) in nodes.iter().enumerate() {
            tree[i * NODE_WORDS..(i + 1) * NODE_WORDS].copy_from_slice(n);
        }
        tree[TREE_HEADER_WORD..TREE_HEADER_WORD + 4].copy_from_slice(&[
            boxes.len() as u32,
            nodes.len() as u32,
            parts as u32,
            u32::from(moved),
        ]);
        self.drawn = drawn;
        self.parts = parts;
        self.moved = moved;
    }

    /// The tree's words of the last write, from the buffer's word 0 to [`INSTANCE_WORD`].
    pub fn tree_words(&self) -> &[u32] {
        &self.tree
    }

    /// The instances' words of the last write, from [`INSTANCE_WORD`] to the highest live
    /// slot's end.
    pub fn instance_words(&self) -> &[u32] {
        &self.inst[..self.high * INSTANCE_WORDS]
    }

    /// The tree's leaves of the last write: each box, its first slot and its count word (the
    /// part count, [`LEAF_MOVED`] when a part moved; a ghost's count is 0 with the bit set).
    pub fn leaves(&self) -> impl Iterator<Item = ([[f32; 3]; 2], u32, u32)> + '_ {
        self.boxes.iter().copied()
    }

    /// The run of slots `entity` holds: its first slot and its count.
    pub fn run_of(&self, entity: EntityId) -> Option<(u16, u16)> {
        self.runs.get(&entity).map(|r| (r.first, r.count))
    }

    /// Slot `slot`'s generation.
    pub fn generation(&self, slot: usize) -> u32 {
        self.slots[slot].generation
    }

    /// Whether the last write's tree holds any leaf: an entity drawn or a ghost.
    pub fn walks(&self) -> bool {
        !self.boxes.is_empty()
    }

    /// Entities the last write drew.
    pub fn entities_drawn(&self) -> usize {
        self.drawn
    }

    /// Parts the last write drew.
    pub fn parts_drawn(&self) -> usize {
        self.parts
    }

    /// Entities the last write was given and did not draw.
    pub fn entities_over_cap(&self) -> usize {
        self.over
    }

    /// Whether any part the last write drew moved since the write before.
    pub fn entities_moved(&self) -> bool {
        self.moved
    }
}

/// Appends the subtree over `order`'s boxes to `nodes`, by median split on the longest axis
/// of their centres' extent, one entity a leaf; returns its root's index.
fn build(
    boxes: &[([[f32; 3]; 2], u32, u32)],
    order: &mut [usize],
    nodes: &mut Vec<[u32; NODE_WORDS]>,
) -> u32 {
    let mut b = [[f32::MAX; 3], [f32::MIN; 3]];
    let mut c = [[f32::MAX; 3], [f32::MIN; 3]];
    for &i in order.iter() {
        for r in 0..3 {
            b[0][r] = b[0][r].min(boxes[i].0[0][r]);
            b[1][r] = b[1][r].max(boxes[i].0[1][r]);
            let m = 0.5 * (boxes[i].0[0][r] + boxes[i].0[1][r]);
            c[0][r] = c[0][r].min(m);
            c[1][r] = c[1][r].max(m);
        }
    }
    let at = nodes.len();
    let mut node = [0u32; NODE_WORDS];
    for r in 0..3 {
        node[r] = b[0][r].to_bits();
        node[3 + r] = b[1][r].to_bits();
    }
    nodes.push(node);
    if let [one] = order {
        nodes[at][6] = boxes[*one].1 | LEAF_BIT;
        nodes[at][7] = boxes[*one].2;
        return at as u32;
    }
    let axis = (0..3)
        .max_by(|&a, &k| (c[1][a] - c[0][a]).total_cmp(&(c[1][k] - c[0][k])))
        .unwrap_or(0);
    let centre = |i: usize| boxes[i].0[0][axis] + boxes[i].0[1][axis];
    // Unstable sort, ties broken by index: the same order as a stable sort, with no buffer.
    order.sort_unstable_by(|&a, &k| centre(a).total_cmp(&centre(k)).then(a.cmp(&k)));
    let mid = order.len() / 2;
    let (lo, hi) = order.split_at_mut(mid);
    let l = build(boxes, lo, nodes);
    let r = build(boxes, hi, nodes);
    nodes[at][6] = l;
    nodes[at][7] = r;
    at as u32
}
