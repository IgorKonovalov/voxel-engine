//! The queue's unit tests of the column finality (ADR-0070's amendment).

use super::keys::key_turned;
use super::open::is_reach_bound;
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

/// A block of two one-cell bricks (cells of 8 voxels): each cell's six direction values,
/// its mask of all six and material 3, its sky and key visibilities, baked.
fn two_cell_block(dirs: [[f32; 3]; 2], vis: [[f32; 6]; 2], key: [[f32; 6]; 2]) -> Vec<u32> {
    let faces = ChunkFaces {
        bricks: vec![(3, vec![1]), (9, vec![2])],
        glass: Vec::new(),
    };
    let (dir, words) = layout(&faces, BRICK_WORDS);
    let mut b = dir;
    b.resize(words as usize, 0);
    for (i, &slot) in [3usize, 9].iter().enumerate() {
        let cell = (b[slot] >> 11) as usize - BRICK_WORDS;
        for f in 0..6 {
            let s = 1.0 + f as f32 / 4.0;
            b[cell + f] = rgb9e5_pack(dirs[i].map(|v| v * s));
        }
        b[cell + 6] = 3 | 0x3F << 16;
        b[cell + VIS_WORD..cell + VIS_WORD + 2].copy_from_slice(&visibility_words(vis[i]));
        b[cell + KEY_WORD..cell + KEY_WORD + 2].copy_from_slice(&key_words(key[i]));
    }
    b
}

/// The frames a landing shows from `from` to `to` under the queue's fade: `from` as it lands,
/// then [`fade_mix`] at `k / FADE` for `k` 1 to `FADE - 1`, then `to` itself (the fades'
/// last frame writes the store's copy).
fn fade_frames(from: &[u32], to: &[u32], vis: bool) -> Vec<Vec<u32>> {
    let mut out = vec![from.to_vec()];
    for k in 1..FADE {
        let mut shown = Vec::new();
        fade_mix(
            &mut shown,
            from,
            to,
            k as f32 / FADE as f32,
            BRICK_WORDS,
            vis,
        );
        out.push(shown);
    }
    out.push(to.to_vec());
    out
}

/// Every word the light pass reads of `to`'s cells, frame to frame over `frames`: the six
/// direction values (each channel in linear light) and the six sky and six key visibility
/// bytes. Panics where one moves further than its share of the whole change, `1 / FADE`, plus
/// one unit of rounding (one `rgb9e5` step at the larger exponent of the two words, one in a
/// byte), or where the first frame is not `from` or the last not `to`.
fn assert_rides_the_fade(frames: &[Vec<u32>], from: &[u32], to: &[u32], what: &str) {
    assert_eq!(
        frames[0], from,
        "{what}: the landing frame shows the old values"
    );
    assert_eq!(
        frames.last().unwrap(),
        to,
        "{what}: the fade ends on the new"
    );
    let bytes = |w: &[u32]| -> [u32; 6] {
        [
            w[0] & 255,
            w[0] >> 8 & 255,
            w[0] >> 16 & 255,
            w[0] >> 24,
            w[1] & 255,
            w[1] >> 8 & 255,
        ]
    };
    for &dir in &to[..CHUNK_BRICKS] {
        if dir == 0 {
            continue;
        }
        let cell = (dir >> 11) as usize - BRICK_WORDS;
        for pair in frames.windows(2) {
            let (a, b) = (&pair[0], &pair[1]);
            for f in cell..cell + 6 {
                let (x, y) = (rgb9e5_unpack(a[f]), rgb9e5_unpack(b[f]));
                let (p, q) = (rgb9e5_unpack(from[f]), rgb9e5_unpack(to[f]));
                let step = 2f32.powi((a[f] >> 27).max(b[f] >> 27) as i32 - 24);
                for k in 0..3 {
                    let share = (q[k] - p[k]).abs() / FADE as f32;
                    assert!(
                        (y[k] - x[k]).abs() <= share + step * 1.0001,
                        "{what}: word {f} channel {k} moved {} against {share} + {step}",
                        (y[k] - x[k]).abs()
                    );
                }
            }
            for w in [VIS_WORD, KEY_WORD] {
                let w = cell + w;
                let (x, y) = (bytes(&a[w..w + 2]), bytes(&b[w..w + 2]));
                let (p, q) = (bytes(&from[w..w + 2]), bytes(&to[w..w + 2]));
                for k in 0..6 {
                    let share = p[k].abs_diff(q[k]) as f32 / FADE as f32;
                    assert!(
                        x[k].abs_diff(y[k]) as f32 <= share + 1.0,
                        "{what}: word {w} byte {k} moved {} against {share} + 1",
                        x[k].abs_diff(y[k])
                    );
                }
                assert_eq!(b[w + 1] & !0xFFFF, to[w + 1] & !0xFFFF, "{what}: the flags");
            }
        }
    }
}

/// Plan 0098 Phase 6: under `Lighting::landing_fade` a landing block's every word the light
/// pass reads moves from its old value to its new one over the queue's [`FADE`] frames, the
/// sky visibilities with the direction values and the key visibilities, and no frame moves a
/// word further than its share of the whole change plus one unit of rounding: after a
/// refinement lands over the chunk's block of the same layout, and after a first block lands
/// over the missing-light chain ([`fallback_block`], whose sky visibilities are then the
/// chain's). With the switch off the sky visibilities change in the landing's first frame.
/// The block's summary is recomputed from the frame's words at each write (`Store::show`).
#[test]
fn a_landing_moves_every_read_word_over_the_fade() {
    let old = two_cell_block(
        [[0.2, 0.4, 0.1], [3.0, 2.0, 1.0]],
        [[1.0, 0.0, 0.5, 0.25, 1.0, 0.1], [0.0; 6]],
        [[0.0, 1.0, 0.5, 0.3, 0.0, 1.0], [1.0; 6]],
    );
    let new = two_cell_block(
        [[0.8, 0.1, 0.3], [0.5, 2.5, 4.0]],
        [[0.0, 1.0, 0.5, 0.75, 0.2, 0.9], [1.0; 6]],
        [[1.0, 0.0, 0.5, 0.9, 1.0, 0.0], [0.0; 6]],
    );
    assert_rides_the_fade(&fade_frames(&old, &new, true), &old, &new, "a refinement");

    let mut chain = [0u32; BRICK_WORDS];
    chain[..6].fill(rgb9e5_pack([1.0, 1.5, 0.5]));
    chain[6] = 0x3F << 16;
    chain[VIS_WORD..VIS_WORD + 2].copy_from_slice(&visibility_words([0.4; 6]));
    let from = fallback_block(&new, &chain, BRICK_WORDS, true);
    let cell = (new[3] >> 11) as usize - BRICK_WORDS;
    assert_eq!(
        from[cell + VIS_WORD],
        chain[VIS_WORD],
        "a first block's sky visibilities fade from the chain's"
    );
    assert_rides_the_fade(
        &fade_frames(&from, &new, true),
        &from,
        &new,
        "a first block",
    );

    let off = fade_frames(&old, &new, false);
    assert_eq!(
        off[1][cell + VIS_WORD..cell + VIS_WORD + 2],
        new[cell + VIS_WORD..cell + VIS_WORD + 2],
        "with the switch off the sky visibilities land in one frame"
    );
    let from = fallback_block(&new, &chain, BRICK_WORDS, false);
    assert_eq!(from[cell + VIS_WORD], new[cell + VIS_WORD]);
}

/// Plan 0098 Phase 6b: under `Lighting::source_fade` a chunk's first block, landing over the
/// missing-light chain or bare, writes the landing frame plus one at [`LANDED_AT`]; a
/// refinement and a key re-bake landing on the chunk keep that first value; with the switch off,
/// and for a block placed with no frame (the pre-roll), the word is 0. The word rides the
/// block's fade unchanged in every frame (the fade stage writes nothing new).
#[test]
fn a_first_landing_writes_its_frame_and_later_landings_keep_it() {
    use super::step::landed_word;
    let at = LANDED_AT as usize;
    let land = |old: Option<&[u32]>, word: Option<u32>, vis: [f32; 6]| {
        let mut b = two_cell_block([[0.5, 0.5, 0.5]; 2], [vis; 2], [[0.5; 6]; 2]);
        b[at] = landed_word(old, word);
        b
    };
    let mut chain = [0u32; BRICK_WORDS];
    chain[..6].fill(rgb9e5_pack([1.0, 1.5, 0.5]));
    chain[6] = 0x3F << 16;
    chain[VIS_WORD..VIS_WORD + 2].copy_from_slice(&visibility_words([0.4; 6]));

    // A first block over the chain, at frame 41: the word is 42, and its fade from the chain's
    // values shows 42 in every frame.
    let first = land(None, Some(42), [0.2; 6]);
    assert_eq!(first[at], 42, "a first block over the chain");
    let from = fallback_block(&first, &chain, BRICK_WORDS, true);
    for f in fade_frames(&from, &first, true) {
        assert_eq!(f[at], 42, "the fade carries the word");
    }
    // A bare first block (no chain) writes the same.
    assert_eq!(land(None, Some(7), [0.2; 6])[at], 7, "a bare first block");

    // A refinement landing frames later keeps the first value, and so does its fade.
    let refined = land(Some(&first), Some(90), [0.6; 6]);
    assert_eq!(
        refined[at], 42,
        "a refinement keeps the first landing's word"
    );
    for f in fade_frames(&first, &refined, true) {
        assert_eq!(f[at], 42);
    }
    // A key re-bake keeps the block's words but its key visibilities, the word with them.
    let mut keyed = two_cell_block([[0.5, 0.5, 0.5]; 2], [[0.6; 6]; 2], [[0.9; 6]; 2]);
    keyed[at] = landed_word(Some(&refined), Some(120));
    assert_eq!(
        keyed[at], 42,
        "a key re-bake keeps the first landing's word"
    );

    // Switch off: 0, first or later; a block placed with no frame (the pre-roll): 0, kept.
    assert_eq!(land(None, None, [0.2; 6])[at], 0, "off: a first block");
    assert_eq!(
        land(Some(&first), None, [0.6; 6])[at],
        0,
        "off: a refinement"
    );
    let pre = land(None, Some(0), [0.2; 6]);
    assert_eq!(pre[at], 0, "the pre-roll's block");
    assert_eq!(
        land(Some(&pre), Some(300), [0.6; 6])[at],
        0,
        "a refinement of a pre-roll's block keeps 0"
    );
}
