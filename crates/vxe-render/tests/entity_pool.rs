//! The entity pool's bookkeeping on the CPU (ADR-0060): frees by handle into a coalescing free
//! list, claims per shared volume, generation-tagged handles, and the allocator as a function
//! of its calls. No device.

use std::sync::Arc;

use vxe_entity::{EntitySet, Model, Part, Pose};
use vxe_render::internals::{EntityInstances, RangeAlloc, VolumePool};
use vxe_render::{DEFAULT_ENTITY_POOL_BYTES, EntityMirror, EntityUploadError, FreedBytes};
use vxe_world::{Material, MaterialId, MaterialTable, Prefab};

/// The walker's part sizes in model voxels and their volume bytes (Plan 0062's log).
const TORSO: [u32; 3] = [8, 6, 12];
const HEAD: [u32; 3] = [6, 6, 6];
const LEG: [u32; 3] = [3, 3, 8];
const TORSO_BYTES: u64 = 1708;
const HEAD_BYTES: u64 = 1304;
const LEG_BYTES: u64 = 1144;
const CUBE_BYTES: u64 = 5672;
const WALKER_BYTES: u64 = TORSO_BYTES + HEAD_BYTES + LEG_BYTES;

/// A table of air and two rows, and the two rows' ids.
fn table() -> (MaterialTable, MaterialId, MaterialId) {
    let mut t = MaterialTable::new();
    let a = t.push(Material::named("a"));
    let b = t.push(Material::named("b"));
    (t, a, b)
}

/// A prefab of `size` filled with `m`, voxel `mark` (x fastest) set to `other` when given:
/// the mark makes a prefab of the same size with another id.
fn block(size: [u32; 3], m: MaterialId, mark: Option<(usize, MaterialId)>) -> Arc<Prefab> {
    let mut v = vec![m; (size[0] * size[1] * size[2]) as usize];
    if let Some((i, o)) = mark {
        v[i] = o;
    }
    Arc::new(Prefab::new(size, v, None))
}

fn model(parts: &[Arc<Prefab>]) -> Arc<Model> {
    let parts = parts
        .iter()
        .map(|v| Part {
            volume: v.clone(),
            parent: None,
            pivot: [0; 3],
            detail: 1,
        })
        .collect();
    Arc::new(Model::new(parts).expect("parts"))
}

/// The walker's shape: torso, head, two legs sharing one prefab; the torso marked by `mark`.
fn walker(m: MaterialId, torso_mark: Option<(usize, MaterialId)>) -> Arc<Model> {
    let leg = block(LEG, m, None);
    model(&[
        block(TORSO, m, torso_mark),
        block(HEAD, m, None),
        leg.clone(),
        leg,
    ])
}

#[test]
fn the_part_sizes_are_the_plans() {
    let (_, a, _) = table();
    for (size, bytes) in [
        (TORSO, TORSO_BYTES),
        (HEAD, HEAD_BYTES),
        (LEG, LEG_BYTES),
        ([16; 3], CUBE_BYTES),
    ] {
        assert_eq!(
            EntityMirror::volume_bytes(&block(size, a, None)),
            bytes,
            "{size:?}"
        );
    }
}

/// Phase 1 done-when 1: models A (torso, head, leg) and B (A's leg and a 16-voxel cube) take
/// 9828 B; freeing A leaves 6816 B (the leg stays, claimed by B); freeing B leaves 0 and one
/// free range of the whole pool.
#[test]
fn a_freed_model_returns_its_bytes_and_a_shared_volume_stays() {
    let (t, m, _) = table();
    let leg = block(LEG, m, None);
    let a = model(&[block(TORSO, m, None), block(HEAD, m, None), leg.clone()]);
    let b = model(&[leg, block([16; 3], m, None)]);
    let mut pool = VolumePool::new(DEFAULT_ENTITY_POOL_BYTES);
    let ha = pool.upload(&a, &t).expect("room").handle;
    let hb = pool.upload(&b, &t).expect("room").handle;
    assert_eq!(pool.used_bytes(), 9828);
    assert_eq!(
        pool.free_model(ha),
        Ok(FreedBytes(TORSO_BYTES + HEAD_BYTES))
    );
    assert_eq!(pool.used_bytes(), 6816);
    assert_eq!(pool.free_model(hb), Ok(FreedBytes(LEG_BYTES + CUBE_BYTES)));
    assert_eq!(pool.used_bytes(), 0);
    let words = (DEFAULT_ENTITY_POOL_BYTES / 4) as u32;
    assert_eq!(
        pool.free_list().ranges().collect::<Vec<_>>(),
        vec![(0, words)]
    );
}

/// Phase 1 done-when 2: 1000 reloads, each a walker whose torso prefab differs from every
/// cycle before's, uploaded and then the cycle before's model freed: the pool's use is the
/// walker's 4156 B after every cycle, and 4156 + 1708 B between the upload and the free.
#[test]
fn reload_n_times_keeps_the_pool_bounded() {
    let (t, m, o) = table();
    let torso_voxels = (TORSO[0] * TORSO[1] * TORSO[2]) as usize;
    let mut pool = VolumePool::new(DEFAULT_ENTITY_POOL_BYTES);
    let mut live = pool.upload(&walker(m, None), &t).expect("room").handle;
    assert_eq!(pool.used_bytes(), WALKER_BYTES);
    for c in 0..1000usize {
        // Voxel c % 576 marked, with the other row past the first 576 cycles: a new torso id
        // each cycle.
        let mark = (c % torso_voxels, if c < torso_voxels { o } else { 0 });
        let next = pool.upload(&walker(m, Some(mark)), &t).expect("room");
        assert_eq!(next.writes.len(), 1, "cycle {c}: only the torso is new");
        assert_eq!(pool.used_bytes(), WALKER_BYTES + TORSO_BYTES, "cycle {c}");
        assert_eq!(pool.free_model(live), Ok(FreedBytes(TORSO_BYTES)));
        assert_eq!(pool.used_bytes(), WALKER_BYTES, "cycle {c}");
        live = next.handle;
    }
}

/// Phase 1 done-when 3: a freed handle is `UnknownModel` to a second free; the next upload
/// reuses its slot under another generation, so the two handles differ; an entity listed under
/// the stale handle is counted over and draws nothing.
#[test]
fn a_stale_handle_is_refused_and_a_reused_index_differs() {
    let (t, m, _) = table();
    let w = walker(m, None);
    let mut pool = VolumePool::new(DEFAULT_ENTITY_POOL_BYTES);
    let old = pool.upload(&w, &t).expect("room").handle;
    let mut set = EntitySet::new();
    let id = set.insert(w.clone(), Pose::rest(&w)).expect("fits");
    let mut inst = EntityInstances::new();
    inst.set_entities(&pool, &set, &[(id, old)]);
    assert_eq!((inst.entities_drawn(), inst.parts_drawn()), (1, 4));

    assert_eq!(pool.free_model(old), Ok(FreedBytes(WALKER_BYTES)));
    assert_eq!(pool.free_model(old), Err(EntityUploadError::UnknownModel));
    let new = pool.upload(&w, &t).expect("room").handle;
    assert_ne!(new, old);
    assert_eq!(pool.part_words(old), None);
    assert!(pool.part_words(new).is_some());

    inst.set_entities(&pool, &set, &[(id, old)]);
    assert_eq!(
        (
            inst.entities_drawn(),
            inst.parts_drawn(),
            inst.entities_over_cap()
        ),
        (0, 0, 1)
    );
    assert!(inst.instance_words().is_empty());
}

/// A small deterministic generator: splitmix64.
struct Mix(u64);

impl Mix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

/// The volume sizes the sequence draws from, in words: a one-voxel part, the walker's leg, head
/// and torso, a 16-voxel cube and a 64-voxel cube.
const SIZES: [u32; 6] = [
    1068 / 4,
    1144 / 4,
    1304 / 4,
    1708 / 4,
    5672 / 4,
    296_480 / 4,
];

/// The starts a fresh default-sized allocator gives a seeded sequence of 10 000 calls: an
/// alloc of a drawn size (two calls in three), or a free of a drawn live range; `None` for a refusal.
fn run(seed: u64) -> Vec<Option<u32>> {
    let mut a = RangeAlloc::new((DEFAULT_ENTITY_POOL_BYTES / 4) as u32);
    let mut r = Mix(seed);
    let mut live: Vec<(u32, u32)> = Vec::new();
    let mut out = Vec::with_capacity(10_000);
    for _ in 0..10_000 {
        if !live.is_empty() && r.next().is_multiple_of(3) {
            let (s, l) = live.swap_remove((r.next() % live.len() as u64) as usize);
            a.free(s, l);
            out.push(Some(s));
        } else {
            let len = SIZES[(r.next() % SIZES.len() as u64) as usize];
            let got = a.alloc(len);
            if let Some(s) = got {
                live.push((s, len));
            }
            out.push(got);
        }
    }
    out
}

/// Phase 1 done-when 4: two allocators fed the same seeded calls give the same start at every
/// step; and a free of `b` then `a` after `alloc a, b, c` leaves one range of `a + b` at
/// `a`'s start, which the next `alloc(a + b)` takes.
#[test]
fn the_allocator_is_a_function_of_its_calls() {
    let one = run(0x0078);
    assert_eq!(one, run(0x0078));
    assert!(
        one.iter().any(|s| s.is_none()),
        "the sequence reaches a refusal"
    );

    let (la, lb, lc) = (SIZES[0], SIZES[3], SIZES[4]);
    let mut x = RangeAlloc::new(la + lb + lc);
    let a = x.alloc(la).expect("room");
    let b = x.alloc(lb).expect("room");
    let c = x.alloc(lc).expect("room");
    assert_eq!((a, b, c), (0, la, la + lb));
    x.free(b, lb);
    x.free(a, la);
    assert_eq!(x.ranges().collect::<Vec<_>>(), vec![(a, la + lb)]);
    assert_eq!(x.alloc(la + lb), Some(a));
    assert_eq!(x.free_total(), 0);
}

/// Phase 1 done-when 5: a pool of four 16-voxel cubes, full, its first and third freed,
/// refuses a model of two new cubes (needed in one range) naming 11 344 B free and 5672 B in
/// the largest hole; a one-cube model lands at the pool's first word.
#[test]
fn a_fragmented_pool_names_its_largest_hole() {
    let (t, m, o) = table();
    let cube = |i: usize| block([16; 3], m, Some((i, o)));
    let mut pool = VolumePool::new(4 * CUBE_BYTES);
    let h: Vec<_> = (0..4)
        .map(|i| pool.upload(&model(&[cube(i)]), &t).expect("room").handle)
        .collect();
    assert_eq!(pool.free_bytes(), 0);
    pool.free_model(h[0]).expect("live");
    pool.free_model(h[2]).expect("live");
    let two = model(&[cube(10), cube(11)]);
    assert_eq!(
        pool.upload(&two, &t).err(),
        Some(EntityUploadError::PoolFull {
            needed: 2 * CUBE_BYTES,
            free: 11_344,
            largest: CUBE_BYTES,
        })
    );
    assert_eq!(pool.used_bytes(), 2 * CUBE_BYTES, "nothing changed");
    let one = pool.upload(&model(&[cube(12)]), &t).expect("room").handle;
    assert_eq!(pool.part_words(one), Some(vec![0]));
}

/// Phase 1 done-when 6: after `clear` the pool holds nothing and every earlier handle is
/// `UnknownModel`.
#[test]
fn clear_frees_everything_and_every_handle() {
    let (t, m, o) = table();
    let mut pool = VolumePool::new(DEFAULT_ENTITY_POOL_BYTES);
    let handles: Vec<_> = (0..5)
        .map(|i| {
            let w = walker(m, Some((i, o)));
            pool.upload(&w, &t).expect("room").handle
        })
        .collect();
    assert_eq!(pool.used_bytes(), WALKER_BYTES + 4 * TORSO_BYTES);
    pool.clear();
    assert_eq!(pool.used_bytes(), 0);
    assert_eq!(pool.largest_free_bytes(), DEFAULT_ENTITY_POOL_BYTES);
    for h in handles {
        assert_eq!(pool.free_model(h), Err(EntityUploadError::UnknownModel));
        assert_eq!(pool.part_words(h), None);
    }
}

/// Readings, run by hand (`--run-ignored only -E 'test(=reading::<name>)'`).
mod reading {
    use super::*;

    /// Plan 0078 Phase 4's waste reading: the default pool's allocator under a seeded sequence
    /// of 10 000 calls, each an upload of a one-volume model (a quarter of them the largest
    /// volume, the rest one of the other five sizes) or, one call in three while anything is
    /// live, the free of a live one, so the pool fills and stays near full. A model's volumes
    /// take one range, so an upload of one volume is one `alloc`. Reports, at the step where
    /// the live bytes peak and as the largest over the steps with the pool at least half live,
    /// `used + free - largest` over the live bytes and `free - largest` over the live bytes;
    /// and the refusals a `clear` would have answered (`largest < needed <= free`) against
    /// those of a pool simply full.
    #[test]
    #[ignore]
    fn the_waste_of_a_seeded_sequence() {
        let total = (DEFAULT_ENTITY_POOL_BYTES / 4) as u32;
        let mut a = RangeAlloc::new(total);
        let mut r = Mix(0x0078_0004);
        let mut live: Vec<(u32, u32)> = Vec::new();
        let mut used = 0u32;
        let ratios = |a: &RangeAlloc, used: u32| {
            (
                f64::from(total - a.largest()) / f64::from(used),
                f64::from(a.free_total() - a.largest()) / f64::from(used),
            )
        };
        let (mut peak_used, mut at_peak) = (0u32, (0f64, 0f64));
        let mut worst_half = (0f64, 0f64);
        let (mut fragmented, mut full, mut uploads) = (0, 0, 0);
        for _ in 0..10_000 {
            if !live.is_empty() && r.next().is_multiple_of(3) {
                let (s, l) = live.swap_remove((r.next() % live.len() as u64) as usize);
                a.free(s, l);
                used -= l;
            } else {
                uploads += 1;
                let len = if r.next().is_multiple_of(4) {
                    SIZES[5]
                } else {
                    SIZES[(r.next() % 5) as usize]
                };
                match a.alloc(len) {
                    Some(s) => {
                        live.push((s, len));
                        used += len;
                    }
                    None if a.largest() < len && len <= a.free_total() => fragmented += 1,
                    None => full += 1,
                }
            }
            if used > peak_used {
                (peak_used, at_peak) = (used, ratios(&a, used));
            }
            if used >= total / 2 {
                let (s, h) = ratios(&a, used);
                worst_half = (worst_half.0.max(s), worst_half.1.max(h));
            }
        }
        println!(
            "10 000 calls, {uploads} uploads; at the live peak ({} B of {} B): \
             (used + free - largest) / live {:.4}, (free - largest) / live {:.4}; worst with the \
             pool at least half live: {:.4} and {:.4}; refusals a clear answers {fragmented}, \
             refusals of a full pool {full}",
            u64::from(peak_used) * 4,
            u64::from(total) * 4,
            at_peak.0,
            at_peak.1,
            worst_half.0,
            worst_half.1
        );
    }
}
