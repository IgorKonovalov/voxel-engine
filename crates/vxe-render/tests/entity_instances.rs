//! The instance table on the CPU (Plan 0078 Phase 3): stable runs of slots per entity, a
//! generation per slot, compaction when no run fits, ghost leaves for one write after a
//! removal, and `EntitySet::replace` keeping an entity's slots. No device.

use std::sync::Arc;

use vxe_entity::{EntityId, EntitySet, Model, Part, Pose, PoseError};
use vxe_render::internals::{EntityInstances, VolumePool};
use vxe_render::{DEFAULT_ENTITY_POOL_BYTES, ModelHandle};
use vxe_world::{Fixed3, Material, MaterialId, MaterialTable, Prefab};

/// A leaf's count word carries this bit when its entity moved, or for a ghost.
const LEAF_MOVED: u32 = 1 << 31;

fn table() -> (MaterialTable, MaterialId, MaterialId) {
    let mut t = MaterialTable::new();
    let a = t.push(Material::named("a"));
    let b = t.push(Material::named("b"));
    (t, a, b)
}

/// A model of `n` root parts, each a one-voxel cube of `m`.
fn parts(n: usize, m: MaterialId) -> Arc<Model> {
    let one = Arc::new(Prefab::new([1; 3], vec![m], None));
    let p = |_| Part {
        volume: one.clone(),
        parent: None,
        pivot: [128; 3],
        detail: 1,
    };
    Arc::new(Model::new((0..n).map(p).collect()).expect("parts"))
}

/// The pool, the set and the instances, with each entity's handle.
struct Rig {
    t: MaterialTable,
    pool: VolumePool,
    set: EntitySet,
    inst: EntityInstances,
    list: Vec<(EntityId, ModelHandle)>,
}

impl Rig {
    fn new() -> Self {
        Self {
            t: table().0,
            pool: VolumePool::new(DEFAULT_ENTITY_POOL_BYTES),
            set: EntitySet::new(),
            inst: EntityInstances::new(),
            list: Vec::new(),
        }
    }

    /// Places `model` at `x` voxels along x, listed last.
    fn place(&mut self, model: &Arc<Model>, x: i64) -> EntityId {
        let h = self.pool.upload(model, &self.t).expect("room").handle;
        let pose = Pose {
            root: Fixed3([x * 256, 0, 0]),
            ..Pose::rest(model)
        };
        let id = self.set.insert(model.clone(), pose).expect("fits");
        self.list.push((id, h));
        id
    }

    fn remove(&mut self, id: EntityId) {
        self.set.remove(id);
        self.list.retain(|&(e, _)| e != id);
    }

    fn write(&mut self) {
        self.inst.set_entities(&self.pool, &self.set, &self.list);
    }

    fn run(&self, id: EntityId) -> (u16, u16) {
        self.inst.run_of(id).expect("a run")
    }

    /// The generations of `id`'s slots.
    fn generations(&self, id: EntityId) -> Vec<u32> {
        let (first, n) = self.run(id);
        (first..first + n)
            .map(|s| self.inst.generation(usize::from(s)))
            .collect()
    }
}

/// Phase 3 done-when 1: three four-part entities take slots 0..4, 4..8, 8..12; removing the
/// first leaves the others where they were with their generations; the next four-part entity
/// takes 0..4 at generation 1, and a two-part entity after it 12..14.
#[test]
fn an_entity_keeps_its_slots_when_another_is_removed() {
    let mut r = Rig::new();
    let four = parts(4, 1);
    let ids: Vec<_> = (0..3).map(|i| r.place(&four, i * 10)).collect();
    r.write();
    assert_eq!(
        ids.iter().map(|&id| r.run(id)).collect::<Vec<_>>(),
        vec![(0, 4), (4, 4), (8, 4)]
    );
    r.remove(ids[0]);
    r.write();
    assert_eq!(r.inst.run_of(ids[0]), None);
    assert_eq!((r.run(ids[1]), r.run(ids[2])), ((4, 4), (8, 4)));
    assert_eq!(r.generations(ids[1]), vec![0; 4]);
    assert_eq!(r.generations(ids[2]), vec![0; 4]);
    let next = r.place(&four, 40);
    r.write();
    assert_eq!(r.run(next), (0, 4));
    assert_eq!(r.generations(next), vec![1; 4]);
    let two = r.place(&parts(2, 1), 50);
    r.write();
    assert_eq!(r.run(two), (12, 2));
    assert_eq!(r.generations(two), vec![0; 2]);
    assert_eq!(r.inst.entities_drawn(), 4);
}

/// Phase 3 done-when 2: 64 entities alternating 4 and 12 parts fill the 512 slots; the 32
/// four-part ones leave and a sixteen-part entity comes in the same write, so 128 slots are
/// free in runs of 4: the table compacts. Nothing is over, every twelve-part entity's run moved
/// and each of its slots' generations is one above its old slot's, the sixteen-part entity is
/// drawn, and 33 entities hold 400 slots.
#[test]
fn a_full_table_compacts_and_bumps_generations() {
    let mut r = Rig::new();
    let (four, twelve) = (parts(4, 1), parts(12, 1));
    let ids: Vec<_> = (0..64)
        .map(|i| r.place(if i % 2 == 0 { &four } else { &twelve }, i * 10))
        .collect();
    r.write();
    assert_eq!(r.inst.parts_drawn(), 512);
    assert_eq!(r.inst.entities_over_cap(), 0);
    let before: Vec<_> = ids
        .iter()
        .skip(1)
        .step_by(2)
        .map(|&id| (r.run(id), r.generations(id)))
        .collect();
    for &id in ids.iter().step_by(2) {
        r.remove(id);
    }
    let sixteen = r.place(&parts(16, 1), 700);
    r.write();
    assert_eq!(r.inst.entities_over_cap(), 0);
    assert_eq!((r.inst.entities_drawn(), r.inst.parts_drawn()), (33, 400));
    for (&id, (run, gens)) in ids.iter().skip(1).step_by(2).zip(&before) {
        assert_ne!(r.run(id), *run, "{id:?} moved");
        let rose: Vec<u32> = gens.iter().map(|g| g + 1).collect();
        assert_eq!(r.generations(id), rose, "{id:?}");
    }
    assert_eq!(r.run(sixteen), (384, 16));
    assert_eq!(r.inst.instance_words().len(), 400 * 32);
}

/// Phase 3 done-when 3: `replace` with a model of the same part count keeps the entity's id,
/// and the instances keep its run and its generations; with another part count the run is
/// allocated again and the generations bump. An unknown entity and a pose of the wrong part
/// count are refused.
#[test]
fn replace_keeps_the_entity_and_its_slots() {
    let mut r = Rig::new();
    let (_, a, b) = table();
    let id = r.place(&parts(4, a), 0);
    let other = r.place(&parts(3, a), 20);
    r.write();
    assert_eq!((r.run(id), r.generations(id)), ((0, 4), vec![0; 4]));

    let same = parts(4, b);
    let h = r.pool.upload(&same, &r.t).expect("room").handle;
    let pose = r.set.get(id).expect("placed").1.clone();
    r.set.replace(id, same.clone(), pose.clone()).expect("fits");
    assert_eq!(r.set.len(), 2);
    r.list[0] = (id, h);
    r.write();
    assert_eq!((r.run(id), r.generations(id)), ((0, 4), vec![0; 4]));
    assert_eq!(r.inst.entities_drawn(), 2);

    let fewer = parts(2, b);
    let h = r.pool.upload(&fewer, &r.t).expect("room").handle;
    r.set
        .replace(id, fewer.clone(), Pose::rest(&fewer))
        .expect("fits");
    r.list[0] = (id, h);
    r.write();
    assert_eq!((r.run(id), r.generations(id)), ((0, 2), vec![1; 2]));
    assert_eq!(r.run(other), (4, 3), "the other entity stays");

    let ghost = EntityId(99);
    assert_eq!(
        r.set.replace(ghost, same.clone(), pose.clone()),
        Err(PoseError::UnknownEntity { entity: ghost })
    );
    assert_eq!(
        r.set.replace(id, same, Pose::rest(&fewer)),
        Err(PoseError::PartCount {
            expected: 4,
            got: 2
        })
    );
}

/// Phase 3 done-when 5: the write after a removal holds the live entities' leaves and one
/// more, a ghost over the removed entity's last box with no part and `LEAF_MOVED` set; the
/// write after that holds no ghost.
#[test]
fn a_removed_entity_leaves_a_ghost_leaf_once() {
    let mut r = Rig::new();
    let four = parts(4, 1);
    let ids: Vec<_> = (0..3).map(|i| r.place(&four, i * 10)).collect();
    r.write();
    r.write();
    let gone_box = r
        .inst
        .leaves()
        .find(|&(_, first, _)| first == 4)
        .expect("the second entity's leaf")
        .0;
    r.remove(ids[1]);
    r.write();
    let leaves: Vec<_> = r.inst.leaves().collect();
    assert_eq!(leaves.len(), 3, "two live and a ghost");
    assert_eq!(r.inst.entities_drawn(), 2);
    let ghosts: Vec<_> = leaves.iter().filter(|l| l.2 & !LEAF_MOVED == 0).collect();
    assert_eq!(ghosts.len(), 1);
    assert_eq!((ghosts[0].0, ghosts[0].2), (gone_box, LEAF_MOVED));
    assert!(r.inst.walks());
    // The header's leaf count and the tree's node count: three leaves, five nodes.
    let words = r.inst.tree_words();
    let header = words.len() - 8;
    assert_eq!(&words[header..header + 3], &[3, 5, 8]);

    r.write();
    assert_eq!(r.inst.leaves().count(), 2, "the ghost is gone");
    for id in [ids[0], ids[2]] {
        r.remove(id);
    }
    r.write();
    assert_eq!(r.inst.parts_drawn(), 0);
    assert!(r.inst.walks(), "the last two leave ghosts");
    r.write();
    assert!(!r.inst.walks());
}

/// Readings, run by hand (`--run-ignored only -E 'test(=reading::<name>)'`).
mod reading {
    use super::*;

    /// Plan 0078 Phase 3's reading: the instance upload's bytes per write (the slots up to the
    /// highest live one) with 64 four-part entities, through 200 seeded steps that each remove
    /// one at random and place a new one: the bytes after each step, their peak and their last.
    #[test]
    #[ignore]
    fn the_instance_upload_after_removals_and_placements() {
        let mut r = Rig::new();
        let four = parts(4, 1);
        let mut ids: Vec<EntityId> = (0..64).map(|i| r.place(&four, i * 10)).collect();
        r.write();
        let mut seed = 0x0078_u64;
        let mut next = || {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            seed >> 33
        };
        let mut bytes = Vec::new();
        for step in 0..200 {
            let k = (next() % ids.len() as u64) as usize;
            r.remove(ids.swap_remove(k));
            ids.push(r.place(&four, 1000 + step * 10));
            r.write();
            assert_eq!(r.inst.entities_drawn(), 64);
            bytes.push(r.inst.instance_words().len() * 4);
        }
        println!(
            "instance upload at 64 four-part entities over 200 removals and placements: \
             first {} B, peak {} B, last {} B",
            bytes[0],
            bytes.iter().max().expect("200"),
            bytes[199]
        );
    }
}
