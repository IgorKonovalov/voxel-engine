//! The instances and the tree over the entities' boxes, built on the CPU each call
//! ([`EntityInstances::write`]); [`super::EntityMirror`] uploads the words.

use std::collections::HashMap;

use vxe_entity::{EntityId, EntitySet};

use super::display::{DisplayPose, PartXform};
use super::pool::{ModelHandle, VolumePool};
use super::{
    INSTANCE_MOVED, INSTANCE_WORD, INSTANCE_WORDS, LEAF_BIT, LEAF_MOVED, MAX_ENTITIES,
    MAX_ENTITY_PARTS, NODE_WORDS, TREE_HEADER_WORD,
};

/// The working buffers of one write. `prev` fills with this call's transforms and is swapped
/// with the kept map at the end, so the two maps trade places each frame instead of one being
/// built fresh.
#[derive(Default)]
struct Scratch {
    inst: Vec<u32>,
    boxes: Vec<([[f32; 3]; 2], u32, u32)>,
    prev: HashMap<(EntityId, usize), [[f32; 4]; 3]>,
    nodes: Vec<[u32; NODE_WORDS]>,
    order: Vec<usize>,
    tree: Vec<u32>,
}

/// The instances and the tree of the last write, the transforms they were drawn with, and
/// what the write drew and counted.
#[derive(Default)]
pub struct EntityInstances {
    /// Each drawn part's part-to-world of the last write.
    prev: HashMap<(EntityId, usize), [[f32; 4]; 3]>,
    scratch: Scratch,
    drawn: usize,
    parts: usize,
    over: usize,
    moved: bool,
}

impl EntityInstances {
    pub fn new() -> Self {
        Self::default()
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

    /// One instance a part of the listed entities (in the list's order) and the tree over
    /// their boxes. An entity past [`MAX_ENTITIES`], or whose parts would pass
    /// [`MAX_ENTITY_PARTS`], is counted and not drawn; so is an entry with no display pose,
    /// one whose handle names no model of `pool`, or one whose display pose has not its
    /// model's part count. An entity's box holds its parts now and in the call before.
    pub fn write<'a>(
        &mut self,
        pool: &VolumePool,
        list: impl Iterator<Item = (EntityId, ModelHandle, Option<&'a DisplayPose>)>,
    ) {
        // Taken out of `self` for the call so `self.prev` stays readable; `mem::take` of the
        // empty default allocates nothing, and the buffers go back below.
        let mut s = std::mem::take(&mut self.scratch);
        let Scratch {
            inst,
            boxes,
            prev,
            nodes,
            order,
            tree,
        } = &mut s;
        inst.clear();
        boxes.clear();
        prev.clear();
        nodes.clear();
        let mut over = 0;
        let mut moved = false;
        for (id, handle, shown) in list {
            let (Some(shown), Some(vols)) = (shown, pool.model(handle)) else {
                over += 1;
                continue;
            };
            let n = shown.parts.len();
            if boxes.len() == MAX_ENTITIES
                || inst.len() / INSTANCE_WORDS + n > MAX_ENTITY_PARTS
                || vols.len() != n
            {
                over += 1;
                continue;
            }
            let first = (inst.len() / INSTANCE_WORDS) as u32;
            let mut entity_moved = false;
            let mut bx = [[f32::MAX; 3], [f32::MIN; 3]];
            for (k, ((xf, detail), (_, vol))) in shown.parts.iter().zip(vols).enumerate() {
                let before = self.prev.get(&(id, k)).copied().unwrap_or(xf.part_to_world);
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
                prev.insert((id, k), xf.part_to_world);
                let detail = f32::from(*detail);
                for row in xf.world_to_part.iter().chain(&before) {
                    inst.extend(row.map(f32::to_bits));
                }
                inst.extend([
                    vol.word,
                    boxes.len() as u32,
                    u32::from(part_moved) * INSTANCE_MOVED,
                    (1.0 / detail).to_bits(),
                    vol.size[0],
                    vol.size[1],
                    vol.size[2],
                    0,
                ]);
            }
            moved |= entity_moved;
            let count = n as u32 | if entity_moved { LEAF_MOVED } else { 0 };
            boxes.push((bx, first, count));
        }
        if !boxes.is_empty() {
            order.clear();
            order.extend(0..boxes.len());
            build(boxes, order, nodes);
        }
        tree.clear();
        tree.resize(INSTANCE_WORD, 0);
        for (i, n) in nodes.iter().enumerate() {
            tree[i * NODE_WORDS..(i + 1) * NODE_WORDS].copy_from_slice(n);
        }
        let parts = inst.len() / INSTANCE_WORDS;
        tree[TREE_HEADER_WORD..TREE_HEADER_WORD + 4].copy_from_slice(&[
            boxes.len() as u32,
            nodes.len() as u32,
            parts as u32,
            u32::from(moved),
        ]);
        std::mem::swap(&mut self.prev, prev);
        self.drawn = boxes.len();
        self.parts = parts;
        self.over = over;
        self.moved = moved;
        self.scratch = s;
    }

    /// The tree's words of the last write, from the buffer's word 0 to [`INSTANCE_WORD`].
    pub fn tree_words(&self) -> &[u32] {
        &self.scratch.tree
    }

    /// The instances' words of the last write, from [`INSTANCE_WORD`].
    pub fn instance_words(&self) -> &[u32] {
        &self.scratch.inst
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
    order.sort_by(|&a, &k| centre(a).total_cmp(&centre(k)).then(a.cmp(&k)));
    let mid = order.len() / 2;
    let (lo, hi) = order.split_at_mut(mid);
    let l = build(boxes, lo, nodes);
    let r = build(boxes, hi, nodes);
    nodes[at][6] = l;
    nodes[at][7] = r;
    at as u32
}
