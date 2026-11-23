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

/// Plan 0096 Phase 3 bar 1 (ADR-0080 rule 2): a chunk's band at the fly-through's 70-degree
/// vertical field and the far-normal band `[2, 6]` px a voxel. At 1080p (f = 540 / tan 35
/// degrees, 771.2 px) a chunk whose nearest corner lies 100 voxels (10 m) away reads near (7.7
/// px), 250 (25 m) mid (3.1 px), 500 (50 m) far (1.5 px). At 720p (f 514.1 px) the same chunks
/// read 5.1, 2.1 and 1.0 px, which the band's edges read mid, mid, far.
#[test]
fn the_band_of_a_chunk_follows_the_far_normal_band_at_the_outputs_focal_length() {
    let vfov = 70f32.to_radians();
    let bands = |height: u32| Bands {
        focal_px: Bands::focal_px(height, vfov),
        lo: 2.0,
        hi: 6.0,
    };
    let f1080 = Bands::focal_px(1080, vfov);
    assert!((f1080 - 771.2).abs() < 0.1, "{f1080}");
    assert!((Bands::focal_px(720, vfov) - 514.1).abs() < 0.1);
    // Chunk (8, 0, 0) spans x 512..576; an eye at x 512 - d (y and z inside the chunk's span)
    // has the chunk's nearest point d voxels away.
    let read = |height: u32, d: i32| {
        let b = bands(height);
        let eye = [(512 - d) as f32, 32.0, 32.0];
        let c = [8, 0, 0];
        assert_eq!(chunk_nearest(c, eye), d as f32);
        (b.px_at(d as f32), b.of_chunk(c, eye))
    };
    for (d, px, band) in [
        (100, 7.71, Band::Near),
        (250, 3.08, Band::Mid),
        (500, 1.54, Band::Far),
    ] {
        let (p, b) = read(1080, d);
        assert!((p - px).abs() < 0.01, "1080p at {d}: {p} px");
        assert_eq!(b, band, "1080p at {d}");
    }
    for (d, px, band) in [
        (100, 5.14, Band::Mid),
        (250, 2.06, Band::Mid),
        (500, 1.03, Band::Far),
    ] {
        let (p, b) = read(720, d);
        assert!((p - px).abs() < 0.01, "720p at {d}: {p} px");
        assert_eq!(b, band, "720p at {d}");
    }
    // The band off (`hi <= lo`) reads every chunk near.
    let off = Bands {
        focal_px: f1080,
        lo: 0.0,
        hi: 0.0,
    };
    assert_eq!(off.of_chunk([7, 0, 0], [32.0; 3]), Band::Near);
}

/// ADR-0080 rule 2 on a bare queue: with bands its jobs draw their band's tier past the coarse
/// pass, the record's tier bits follow the count, and a block below its band's count is a
/// refinement's candidate; without a focal length every count is the settings'.
#[test]
fn a_chunks_count_and_tier_follow_its_band() {
    use crate::bake::record::{TIER_FAR, TIER_MID};
    let mut q = BakeQueue::new(BakeSettings::default(), 1.0);
    q.settings.brick_samples = QUEUE_BRICK_SAMPLES;
    q.tiers = DEFAULT_TIERS;
    let eye = [0, 0, 0];
    let (near, mid, far) = ([1, 0, 0], [4, 0, 0], [9, 0, 0]);
    // No focal length: the settings' count, past the coarse pass beyond the full-first shell.
    assert_eq!(q.count_for(near, 1, eye), (QUEUE_BRICK_SAMPLES, 0));
    assert_eq!(q.want(far), QUEUE_BRICK_SAMPLES);
    q.bands = Some(Bands {
        focal_px: Bands::focal_px(1080, 70f32.to_radians()),
        lo: 2.0,
        hi: 6.0,
    });
    q.eye_c = [32.0; 3];
    assert_eq!(q.band_of(near), Band::Near);
    assert_eq!(q.band_of(mid), Band::Mid);
    assert_eq!(q.band_of(far), Band::Far);
    q.full_first = Some(2);
    assert_eq!(q.count_for(near, 0, eye), (4096, 0), "full first");
    assert_eq!(
        q.count_for(mid, 0, eye),
        (256, TIER_FAR),
        "a coarse first pass"
    );
    assert_eq!(
        q.count_for(mid, 1, eye),
        (1024, TIER_MID),
        "the mid refinement"
    );
    assert_eq!(
        q.count_for(far, 0, eye),
        (256, TIER_FAR),
        "the far tier, final"
    );
    assert_eq!(q.count_for(far, 1, eye), (256, TIER_FAR));
    // A block's count against its band's: a mid chunk at 256 is below, a far one is not.
    q.block_count.insert(mid, 256);
    q.block_count.insert(far, 256);
    assert_eq!(q.below(mid), Some(true));
    assert_eq!(q.below(far), Some(false));
    assert_eq!(q.below(near), None);
    q.block_count.insert(mid, 1024);
    assert_eq!(q.below(mid), Some(false));
}

/// Plan 0096 Phase 3 bar 4 (ADR-0080 rule 4): the reach rule by tier over a streamed view with
/// `r_full` 16 and light radius 4, the candidates the full sphere's octant of non-negative
/// offsets about the eye's chunk, the resident chunks exactly the sphere's and nothing pending,
/// the bands at 1080p and `[2, 6]`. Every chunk whose reach crosses the full radius
/// ([`reach_crosses`]: the chunks past 16 less the reach's extent, 5.39 chunks, out) is mid or
/// far band and bakes flagged partial ([`reach_verdict`] `Some(true)`); with a chunk of its
/// reach pending (one in 8 read) it waits; every other chunk's reach is ready. A near-band
/// chunk whose reach crosses a shrunk `r_full` of 4 waits under the default
/// [`ReachMode::Full`] and bakes partial under [`ReachMode::Partial`]. A chunk arriving in a
/// flagged far record's reach re-queues nothing; the same arrival in a flagged dig ring's reach
/// (a record at the near count) re-queues the ring.
#[test]
fn a_far_band_chunk_whose_reach_crosses_the_full_radius_bakes_from_the_chunks_resident() {
    use super::reach::{reach_by, reach_verdict};
    let (lr, r) = (4, 16);
    let eye = [0, 0, 0];
    let bands = Bands {
        focal_px: Bands::focal_px(1080, 70f32.to_radians()),
        lo: 2.0,
        hi: 6.0,
    };
    let eye_c = [32.0; 3];
    let within = |c: ChunkCoord, r: i32| c.0.iter().map(|&d| d * d).sum::<i32>() <= r * r;
    let verdict = |c: ChunkCoord, r: i32, mode: ReachMode, pending: Option<ChunkCoord>| {
        let mode = mode.for_band(bands.of_chunk(c.0, eye_c) == Band::Near);
        let reach = reach_by(
            c,
            lr,
            |n| Some(n) == pending,
            |n| within(n, r),
            mode == ReachMode::Full,
        );
        reach_verdict(reach, mode, reach_crosses(c, eye, lr, r))
    };
    // The sphere's octant of non-negative offsets (the rule is symmetric; a debug build's
    // walks over the whole sphere take about 10 s).
    let sphere: Vec<ChunkCoord> = (0..=r)
        .flat_map(|z| (0..=r).flat_map(move |y| (0..=r).map(move |x| ChunkCoord([x, y, z]))))
        .filter(|&c| within(c, r))
        .collect();
    let mut partial = 0;
    for &c in &sphere {
        let len = (c.0.iter().map(|&d| f64::from(d * d)).sum::<f64>()).sqrt();
        let v = verdict(c, r, ReachMode::Full, None);
        if reach_crosses(c, eye, lr, r) {
            assert!(len > f64::from(r) - reach_extent(lr), "{c:?} at {len}");
            assert_ne!(bands.of_chunk(c.0, eye_c), Band::Near, "{c:?}");
            assert_eq!(v, Some(true), "{c:?} bakes partial");
            // One in 8 again with a chunk of its reach pending.
            if partial % 8 == 0 {
                let pending = ChunkCoord([c.0[0] + 1, c.0[1], c.0[2]]);
                assert_eq!(
                    verdict(c, r, ReachMode::Full, Some(pending)),
                    None,
                    "{c:?} waits for a pending chunk"
                );
            }
            partial += 1;
        } else {
            assert_eq!(v, Some(false), "{c:?}'s reach is resident");
        }
    }
    assert!(partial > 0);
    // A near-band chunk whose reach crosses a shrunk full radius of 4.
    let near = ChunkCoord([1, 0, 0]);
    assert_eq!(bands.of_chunk(near.0, eye_c), Band::Near);
    assert!(reach_crosses(near, eye, lr, 4));
    assert_eq!(verdict(near, 4, ReachMode::Full, None), None, "it waits");
    assert_eq!(verdict(near, 4, ReachMode::Partial, None), Some(true));
    // The re-queue on arrival: a far record at 256, a dig's ring at the near count.
    let mut q = BakeQueue::new(BakeSettings::default(), 1.0);
    q.settings.brick_samples = QUEUE_BRICK_SAMPLES;
    q.tiers = DEFAULT_TIERS;
    q.bands = Some(bands);
    let (far, ring) = ([14, 0, 0], [-14, 0, 0]);
    q.partial.insert(far);
    q.block_count.insert(far, DEFAULT_TIERS[2]);
    q.partial.insert(ring);
    q.block_count.insert(ring, QUEUE_BRICK_SAMPLES);
    let arrived = |c: [i32; 3]| -> ChunkSet { [c].into_iter().collect() };
    q.requeue_arrived(Some(&arrived([17, 0, 0])), lr);
    assert!(q.partial_due.is_empty(), "the far record is not re-queued");
    q.requeue_arrived(Some(&arrived([-17, 0, 0])), lr);
    assert_eq!(
        q.partial_due.iter().copied().collect::<Vec<_>>(),
        vec![ring]
    );
}

/// Plan 0096 Phase 4 bar 1 (ADR-0080 rule 3): a queue with 3 chunks in view waiting for their
/// tier's record reads the drain value (4.0 ms a frame at 60 Hz, over the budget's 1.5) while
/// any waits, and the line's 1.0 at the step after the third lands; without a drain, the
/// budget's 1.5 throughout.
#[test]
fn the_slice_is_the_drain_value_while_in_view_work_waits_and_the_line_after() {
    let hz60 = 1.0 / 60.0;
    let mut q = BakeQueue::new(BakeSettings::default(), 1.5);
    q.drain = Some(Drain {
        drain_ms: 4.0,
        line_ms: 1.0,
    });
    let chunks = [[3, 0, 0], [4, 0, 0], [5, 0, 0]];
    q.view_waiting = chunks.iter().copied().collect();
    let mut landed: Vec<[i32; 3]> = Vec::new();
    for c in chunks {
        assert_eq!(q.step_slice(hz60), 4.0, "{} landed", landed.len());
        assert!(q.stats.draining);
        landed.push(c);
        q.retain_waiting(|x| landed.contains(&x));
    }
    assert_eq!(q.stats.drain_waiting, 0);
    assert_eq!(q.step_slice(hz60), 1.0, "the line once the third landed");
    assert!(!q.stats.draining);
    q.drain = None;
    q.view_waiting = chunks.iter().copied().collect();
    assert_eq!(q.step_slice(hz60), 1.5, "the budget alone");
}

/// A synthetic pool of the cube of chunks `-r..=r` about the origin, about one in eleven (by
/// its coordinate) a class 1 candidate with a coarse block, the rest class 0.
fn cube_pool(r: i32) -> std::collections::HashMap<[i32; 3], (u8, bool)> {
    let mut pool = std::collections::HashMap::new();
    for x in -r..=r {
        for y in -r..=r {
            for z in -r..=r {
                let class = u8::from((7 * x + 13 * y + 29 * z).rem_euclid(11) == 0);
                pool.insert([x, y, z], (class, class == 1));
            }
        }
    }
    pool
}

/// The order's view from chunk `eye` looking along +x, at a 70 degree, 16:9 view.
fn order_view(eye: [i32; 3]) -> (OrderView, [f32; 3], f32) {
    let forward = [1.0f32, 0.0, 0.0];
    let half = view_half_angle(70f32.to_radians(), 16.0 / 9.0);
    let key = OrderView {
        eye,
        forward: forward.map(f32::to_bits),
        half_angle: half.to_bits(),
        order: QueueOrder::default(),
        near: 1,
    };
    (key, forward, half)
}

/// An opening's walk of the kept order: the first `n` of lead 0, then the first `n` of lead 1.
fn walk(
    ordered: &mut Ordered,
    pool: &std::collections::HashMap<[i32; 3], (u8, bool)>,
    n: usize,
) -> Vec<[i32; 3]> {
    let mut out = Vec::new();
    for lead in 0..2 {
        for i in 0..n {
            match ordered.get(lead, i, pool) {
                Some(k) => out.push(k.c),
                None => break,
            }
        }
    }
    out
}

/// Plan 0096 Phase 5 bar 1: over a pool of 20 000 candidates (the cube -13..=13, 19 683) a
/// second opening at a still camera scores no candidate, and one after the camera moves a
/// chunk scores at most the view cone's members (the chunks of the pool whose angle term reads
/// 0 from the new eye: the cone and the near shell), never the pool.
#[test]
fn a_still_step_visits_no_candidate_and_a_one_chunk_move_visits_only_the_cone() {
    let pool = cube_pool(13);
    assert_eq!(pool.len(), 19_683);
    let mut ordered = Ordered::default();
    let (key, f, h) = order_view([0, 0, 0]);
    ordered.set_view(key, f, h);
    ordered.rebuild(pool.iter());
    let first = walk(&mut ordered, &pool, QUEUE_CHECKS);
    assert_eq!(first.len(), 2 * QUEUE_CHECKS);
    let opened = ordered.take_visited();
    assert!(opened > 0 && opened < pool.len() as u64, "{opened}");
    let (key, f, h) = order_view([0, 0, 0]);
    assert!(!ordered.set_view(key, f, h));
    assert_eq!(walk(&mut ordered, &pool, QUEUE_CHECKS), first);
    assert_eq!(ordered.take_visited(), 0, "a still camera's second opening");
    let eye = [1, 0, 0];
    let (key, f, h) = order_view(eye);
    ordered.set_view(key, f, h);
    walk(&mut ordered, &pool, QUEUE_CHECKS);
    let moved = ordered.take_visited();
    let cone = pool
        .keys()
        .filter(|c| angle_term([0, 1, 2].map(|k| c[k] - eye[k]), f, h, 1) <= 0.0)
        .count() as u64;
    assert!(moved > 0 && moved <= cone, "moved {moved}, cone {cone}");
    assert!(moved < pool.len() as u64);
}

/// Plan 0096 Phase 5 bar 2: the pool at about 2 000 (the cube -6..=6, 2 197) and at about
/// 20 000 (-13..=13) opens the same first 16 candidates of each class, in the order a full
/// sort of ADR-0048's keys gives.
#[test]
fn the_opened_jobs_are_the_nearest_of_their_band_whatever_the_depth() {
    let eye = [0, 0, 0];
    let (key, f, h) = order_view(eye);
    let mut opened = Vec::new();
    for r in [6, 13] {
        let pool = cube_pool(r);
        let mut ordered = Ordered::default();
        ordered.set_view(key, f, h);
        ordered.rebuild(pool.iter());
        let got = walk(&mut ordered, &pool, OPENED);
        let mut full: Vec<Cand> = pool
            .iter()
            .map(|(&c, &(class, coarse))| {
                cand_at(c, class, coarse, eye, f, h, QueueOrder::default(), 1)
            })
            .collect();
        full.sort_by_key(Cand::sort_key);
        let want: Vec<[i32; 3]> = [0u8, 1]
            .iter()
            .flat_map(|&l| full.iter().filter(move |k| k.lead == l).take(OPENED))
            .map(|k| k.c)
            .collect();
        assert_eq!(got, want, "the cube of side {}", 2 * r + 1);
        opened.push(got);
    }
    assert_eq!(opened[0], opened[1]);
}

/// The jobs an opening's first walk takes in the order tests.
const OPENED: usize = 16;

/// The kept order after candidates join and leave between openings (in a scanned cell, an
/// unscanned one and a cell new to the index, near and far) walks whole as a full sort of the
/// pool does.
#[test]
fn the_kept_order_walks_as_a_full_sort_after_joins_and_leaves() {
    let eye = [0, 0, 0];
    let (key, f, h) = order_view(eye);
    let mut pool = cube_pool(6);
    let mut ordered = Ordered::default();
    ordered.set_view(key, f, h);
    ordered.rebuild(pool.iter());
    walk(&mut ordered, &pool, OPENED);
    let joins = [[1, 1, 0], [5, 0, 0], [20, 0, 0], [-30, 2, 1], [0, 0, 9]];
    let leaves = [[0, 1, 0], [6, 6, 6], [2, 0, 0]];
    for (n, c) in joins.into_iter().enumerate() {
        let v = (u8::from(n % 2 == 1), n % 2 == 1);
        let was = pool.insert(c, v).map(|w| w.0);
        ordered.insert(c, v.0, v.1, was);
        walk(&mut ordered, &pool, OPENED);
    }
    for c in leaves {
        let (class, _) = pool.remove(&c).expect("in the pool");
        ordered.remove(c, class);
    }
    // The whole order, so the far joins are walked too. The walk yields what left as it
    // stands; the opening skips them by the pool's check.
    let n = pool.len();
    let got: Vec<[i32; 3]> = walk(&mut ordered, &pool, n + leaves.len())
        .into_iter()
        .filter(|c| pool.contains_key(c))
        .collect();
    let mut full: Vec<Cand> = pool
        .iter()
        .map(|(&c, &(class, coarse))| {
            cand_at(c, class, coarse, eye, f, h, QueueOrder::default(), 1)
        })
        .collect();
    full.sort_by_key(Cand::sort_key);
    for lead in 0..2u8 {
        let want: Vec<[i32; 3]> = full
            .iter()
            .filter(|k| k.lead == lead)
            .take(n)
            .map(|k| k.c)
            .collect();
        let have: Vec<[i32; 3]> = got
            .iter()
            .copied()
            .filter(|c| pool[c].0 == lead)
            .take(n)
            .collect();
        assert_eq!(have, want, "lead {lead}");
    }
}

/// The block counts follow the store's window: a queue holding counts for a 4 x 4 x 1 window's
/// blocks, the window moved two chunks along x (the same size), keeps the 8 counts still inside
/// and drops the 8 the move left behind, which are read again at the next opening; a window of
/// another size drops every count.
#[test]
fn a_window_move_drops_the_block_counts_it_leaves_behind() {
    use crate::bake::store::dropped_by_move;
    let a = vxe_world::ChunkExtent::new([0, 0, 0], [4, 4, 1]);
    let b = vxe_world::ChunkExtent::new([2, 0, 0], [6, 4, 1]);
    let mut q = BakeQueue::new(BakeSettings::default(), 1.5);
    for x in 0..4 {
        for y in 0..4 {
            q.block_count.insert([x, y, 0], 256);
        }
    }
    let held: Vec<[i32; 3]> = q.block_count.keys().copied().collect();
    q.rewindowed(dropped_by_move(Some(a), b, held.iter().copied()));
    assert_eq!(q.block_count.len(), 8);
    assert!(
        q.block_count.keys().all(|c| c[0] >= 2),
        "{:?}",
        q.block_count
    );
    assert_eq!(q.touched.len(), 8);
    assert!(q.touched.iter().all(|c| c[0] < 2));
    let c = vxe_world::ChunkExtent::new([2, 0, 0], [10, 4, 1]);
    let held: Vec<[i32; 3]> = q.block_count.keys().copied().collect();
    q.rewindowed(dropped_by_move(Some(b), c, held.iter().copied()));
    assert!(q.block_count.is_empty(), "another size empties the store");
}

/// The rule reading counts a job by its exclusion whole, and before the in-view landing only
/// when it lies in view; every job counts among those opened, and before the landing until it.
#[test]
fn the_rule_reading_splits_the_jobs_by_exclusion_and_the_landing() {
    let mut r = RuleMissed::default();
    r.note(Some((0, true)), 100);
    r.note(Some((1, false)), 50);
    r.note(None, 10);
    r.landed_at = Some(7);
    r.note(Some((2, true)), 30);
    assert_eq!((r.jobs, r.samples), ([1, 1, 1], [100, 50, 30]));
    assert_eq!((r.before_jobs, r.before_samples), ([1, 0, 0], [100, 0, 0]));
    assert_eq!((r.opened, r.opened_before), ([4, 190], [3, 160]));
    assert_eq!(r.before_shares(), [1.0 / 3.0, 100.0 / 160.0]);
}
