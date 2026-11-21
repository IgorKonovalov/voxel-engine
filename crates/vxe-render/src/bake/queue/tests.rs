//! The queue's unit tests of the column finality (ADR-0070's amendment).

use super::keys::key_turned;
use super::open::is_reach_bound;
use super::rule::{buried, relisted};
use super::*;
use crate::gpu_world::column_is_final;

/// ADR-0070's amendment: a chunk whose reach's columns are not final does not bake. A
/// window of z -2..6 over a reach of light radius 2 about chunk (0, 0, 0); every chunk of
/// the window mirrored but (2, 1, 5), high in a column over the reach. Wanted by the
/// streamer and absent, the reach is not final; mirrored, or outside the streamer's
/// sphere, it is; pending upload, it is not, wanted or not. A column outside the reach's
/// footprint (x 4) is not read.
#[test]
fn a_chunk_absent_high_in_a_column_holds_the_bake() {
    let high = ChunkCoord([2, 1, 5]);
    let coord = ChunkCoord([0, 0, 0]);
    let lr = 2;
    let fin = |mirrored: bool, pending: bool, wanted: bool| {
        let want = move |_: ChunkCoord| wanted;
        let w: &dyn Fn(ChunkCoord) -> bool = &want;
        reach_columns_final(coord, lr, |x, y| {
            column_is_final(
                x,
                y,
                -2..6,
                |c| pending && c == high,
                |c| mirrored || c != high,
                Some(w),
            )
        })
    };
    assert!(!fin(false, false, true), "absent and wanted: not final");
    assert!(fin(true, false, true), "mirrored: final");
    assert!(
        fin(false, false, false),
        "outside the streamer's sphere: final"
    );
    assert!(!fin(true, true, false), "pending: not final");
    let far = ChunkCoord([4, 0, 5]);
    let read = std::cell::RefCell::new(Vec::new());
    reach_columns_final(coord, lr, |x, y| {
        read.borrow_mut().push([x, y]);
        true
    });
    assert!(read.borrow().contains(&[2, 1]));
    assert!(!read.borrow().contains(&[far.0[0], far.0[1]]));
    let held = held_by_streamer([32.0, 32.0, 32.0], Some(3), Some(&|_| true)).expect("streamed");
    assert!(held(ChunkCoord([3, 0, 0])) && !held(ChunkCoord([3, 1, 0])));
    assert!(
        held_by_streamer([0.0; 3], Some(3), None).is_none(),
        "a bounded world"
    );
}

/// No chunk waits forever under the column wait: a chunk the streamer will never deliver
/// counts as arrived. The eye in chunk (0, 0, 0) with the streamer's sphere of radius 3
/// ([`held_by_streamer`]), a window of z -2..6 and a reach of light radius 2 about chunk
/// (0, 0, 0): with exactly the chunks within the sphere mirrored, the columns over the reach
/// are final, though their tops (z 3 to 5 at the footprint's corners, beyond the sphere)
/// and every chunk above the window's extent (z 6 on) never arrive. One chunk within the
/// sphere absent, or pending upload, holds the bake.
#[test]
fn every_chunk_in_reach_bakes_under_the_wait() {
    let coord = ChunkCoord([0, 0, 0]);
    let lr = 2;
    let eye = [32.0, 32.0, 32.0];
    let resident = |_: ChunkCoord| true;
    let held = held_by_streamer(eye, Some(3), Some(&resident)).expect("streamed");
    let top = ChunkCoord([2, 1, 5]);
    assert!(!held(top), "the column's top lies beyond the sphere");
    let absent = ChunkCoord([1, 1, 1]);
    assert!(held(absent));
    let fin = |missing: Option<ChunkCoord>, pending: Option<ChunkCoord>| {
        let w: &dyn Fn(ChunkCoord) -> bool = &held;
        reach_columns_final(coord, lr, |x, y| {
            column_is_final(
                x,
                y,
                -2..6,
                |c| Some(c) == pending,
                |c| held(c) && Some(c) != missing && Some(c) != pending,
                Some(w),
            )
        })
    };
    assert!(fin(None, None), "the deliverable chunks arrived: final");
    assert!(!fin(Some(absent), None), "one held chunk absent: not final");
    assert!(
        !fin(None, Some(absent)),
        "one held chunk pending: not final"
    );
    assert!(
        !fin(None, Some(top)),
        "a pending chunk beyond the sphere: not final"
    );
}

/// Plan 0095 Phase 6: the census's reach-bound test ([`is_reach_bound`]) over a candidate list of
/// every chunk of the full sphere (`d2 <= 16^2` about the eye's chunk) at light radius 4 counts
/// the chunks [`reach_crosses`] marks, chunk for chunk: 11 814 of them. A chunk 11 chunks out
/// along an axis is not bound (its reach's farthest chunk, at offset (4, 3, 2), lies 15.43
/// out), and one at (11, 3, 1), 11.45 out, is (16.43).
#[test]
fn the_reach_bound_count_matches_the_exact_crossing_test() {
    let (lr, r) = (4, 16);
    let offsets: Vec<[i32; 3]> = reach_offsets(lr).collect();
    let extent = reach_extent(lr);
    assert!((extent - 29f64.sqrt()).abs() < 1e-9, "{extent}");
    let eye = [3, -2, 1];
    let sphere: Vec<[i32; 3]> = (-r..=r)
        .flat_map(|z| (-r..=r).flat_map(move |y| (-r..=r).map(move |x| [x, y, z])))
        .filter(|o| o.iter().map(|&d| d * d).sum::<i32>() <= r * r)
        .collect();
    let (mut census, mut exact) = (0, 0);
    for &o in &sphere {
        let bound = is_reach_bound(o, &offsets, extent, r);
        let crosses = reach_crosses(ChunkCoord([0, 1, 2].map(|k| eye[k] + o[k])), eye, lr, r);
        assert_eq!(bound, crosses, "offset {o:?}");
        census += usize::from(bound);
        exact += usize::from(crosses);
    }
    assert_eq!((census, exact), (11_814, 11_814));
    assert!(!is_reach_bound([11, 0, 0], &offsets, extent, r));
    assert!(is_reach_bound([11, 3, 1], &offsets, extent, r));
}

/// A block re-bakes its key words once the light has turned past both the disc's radius and
/// the step, or when it was baked with no key light.
#[test]
fn a_block_is_turned_past_the_radius_and_the_step() {
    let at = |deg: f32| {
        let r = deg.to_radians();
        [r.sin(), 0.0, r.cos()]
    };
    let stamp = pack_direction(at(0.0));
    let (radius, step) = (3f32.to_radians(), 3f32.to_radians());
    assert!(!key_turned(stamp, at(1.0), radius, step));
    assert!(!key_turned(stamp, at(2.9), radius, step));
    assert!(key_turned(stamp, at(4.0), radius, step));
    assert!(!key_turned(stamp, at(4.0), 5f32.to_radians(), step));
    assert!(key_turned(0, at(0.0), radius, step));
}

/// Plan 0096 Phase 2 bar 2 (ADR-0080): the rule skips no chunk it cannot answer. A chunk at
/// (3, 3, 1) at light radius 2 under a flat column top of 319 is buried (its box's top 255);
/// one column of its footprint whose lowest top is 191 (the box's top one chunk over it), or
/// 255 (level with it), or holding no opaque voxel, and it is not. An emitter chunk 3 chunks
/// away along x lies within the light radius 4 (a gap of 2 chunks), so the chunk bakes with a
/// job; one 6 chunks away (a gap of 5) does not reach it.
#[test]
fn a_chunk_with_an_emitter_in_reach_or_a_box_crossing_the_column_top_is_not_skipped() {
    let c = ChunkCoord([3, 3, 1]);
    assert!(buried(c, 2, |_, _| 319));
    for low in [191, 255, crate::gpu_world::COLUMN_NONE] {
        assert!(
            !buried(c, 2, |x, y| if [x, y] == [5, 1] { low } else { 319 }),
            "a column at {low}"
        );
    }
    let mut read = Vec::new();
    buried(c, 2, |x, y| {
        read.push([x, y]);
        319
    });
    assert_eq!(read.len(), 25, "the footprint's 5 x 5 columns");
    use crate::gpu_world::boxes_within;
    let o = ChunkCoord([0, 0, 0]);
    assert!(
        boxes_within(ChunkCoord([3, 0, 0]), o, 4),
        "an emitter 3 chunks away"
    );
    assert!(!boxes_within(ChunkCoord([6, 0, 0]), o, 4));
}

/// Plan 0096 Phase 2 bar 3: a column carved from the surface into a cave re-lists the cave's
/// zeroed chunks. Three zeroed chunks (2..=4, 3, 1) and a far one (20, 20, 1) under a flat
/// column top of 319 at light radius 2: no change re-lists nothing; a carve that lowers
/// column (4, 3) to 100 re-lists the three, whose footprints hold it, and reads the rule of
/// none but those within 2 columns of the change.
#[test]
fn a_dig_to_the_sky_over_a_buried_chunk_relists_it() {
    let cave = [[2, 3, 1], [3, 3, 1], [4, 3, 1]];
    let far = [20, 20, 1];
    let zeroed: ChunkSet = cave.iter().copied().chain([far]).collect();
    let flat = |_: i32, _: i32| 319;
    let still = |z: [i32; 3]| buried(ChunkCoord(z), 2, flat);
    assert!(relisted(&zeroed, Some(&[]), 2, still).is_empty());
    assert!(
        relisted(&zeroed, None, 2, still).is_empty(),
        "every one still buried"
    );
    let dug = |x: i32, y: i32| if [x, y] == [4, 3] { 100 } else { 319 };
    let mut asked = Vec::new();
    let due = relisted(&zeroed, Some(&[[4, 3]]), 2, |z| {
        asked.push(z);
        buried(ChunkCoord(z), 2, dug)
    });
    assert_eq!(due, cave.to_vec());
    assert!(!asked.contains(&far), "the far chunk is not read");
}
