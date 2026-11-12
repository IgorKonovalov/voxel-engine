//! The loaded light (ADR-0046, Plan 0059): a bake's brick records and the frames that read them, headless on
//! the software adapter. The readings on a named adapter (lane rule 9c: the live queue's digs,
//! the resolution cut, the budgets at rest) are the `bake` example's, over the rigs of
//! `support::bake`.
//!
//! Every world here is a hand-built box of under 2000 faces at test counts: nothing bakes a whole
//! world in a test.

mod support;

use std::collections::{BTreeMap, HashMap};

use flythrough::cave::{ORE, ROCK};
use support::bake::{
    Rig, StairArm, StairFrame, air_world, brick_value, by_normal, cells_of, fill, hit_face, luma,
    new_frame, rgb, rig_on, stair_cameras, stair_frame, staircase, test_counts, within,
};
use support::cave_table;
use support::rigs::tone_bytes;
use vxe_render::BakeSettings;
use vxe_render::LightingResolution;
use vxe_render::{Camera, Exposure, Frame, Lighting, wgpu};
use vxe_world::{ChunkCoord, ChunkExtent, EditOp, FIXED_ONE, Fixed3, MaterialTable, Mode, World};

/// A rock block under an ore plate over chunks `[0, 2)^3`: about 1700 faces, three of the block's
/// sides in the camera's view, with a 64 x 36 frame looking at it.
fn small_box() -> (World, Camera) {
    let world = air_world([2, 2, 2], &cave_table());
    fill(&world, [40, 40, 40], [56, 56, 56], ROCK);
    fill(&world, [42, 42, 72], [54, 54, 74], ORE);
    let camera = Camera {
        position: [30.5, 28.5, 70.5].into(),
        yaw: 45f32.to_radians(),
        pitch: (-35f32).to_radians(),
        vfov: 70f32.to_radians(),
    };
    (world, camera)
}

/// The records depend on each face's own samples alone: one batch, batches of 7 faces and the
/// jobs reversed read bit-identical; and [`four_ways`].
#[test]
#[ignore = "soak"]
fn a_bake_does_not_depend_on_its_slicing_or_order() {
    let (world, _) = small_box();
    let Some(mut rig) = rig_on(world, [16, 16], None) else {
        return;
    };
    let one = BakeSettings {
        batch_faces: 65_536,
        ..test_counts(1024)
    };
    let report = rig.bake(&one);
    assert!(report.faces > 1000, "{} faces", report.faces);
    assert!(report.faces < 2000, "{} faces", report.faces);
    let first = rig.blocks();
    let held = cells_of(&first);
    let lit = held.values().filter(|c| c[0] != 0).count();
    println!("{} held directions, {lit} lit", held.len());
    // The rock's four top bricks face the ore plate: their +z directions are lit.
    for (x, y) in [(5, 5), (5, 6), (6, 5), (6, 6)] {
        let up = held.get(&([x, y, 6], 5)).map_or(0, |c| c[0]);
        assert_ne!(up, 0, "brick ({x}, {y}, 6) +z unlit");
    }
    rig.bake(&BakeSettings {
        batch_faces: 7,
        ..one.clone()
    });
    assert_eq!(rig.blocks(), first, "batches of 7 faces");
    rig.bake(&BakeSettings {
        reversed: true,
        ..one.clone()
    });
    assert_eq!(rig.blocks(), first, "the jobs reversed");
    four_ways(&mut rig);
}

/// Bakes `rig` at 4096 samples a brick direction, rounds of 64, four ways: several rounds a
/// submit with every job in one submit, one round a submit, each job alone, and each job alone at
/// one round a submit; asserts bit-identical records and samples a face.
fn four_ways(rig: &mut Rig) {
    let stop = BakeSettings {
        keep_faces: true,
        ..test_counts(4096)
    };
    let samples_of = |r: &vxe_render::BakeReport| -> HashMap<([i32; 3], u8), u32> {
        r.outcomes
            .iter()
            .map(|o| ((o.voxel, o.face), o.samples))
            .collect()
    };
    let several = rig.bake(&stop);
    let spent = samples_of(&several);
    let counts: std::collections::BTreeSet<u32> = spent.values().copied().collect();
    println!(
        "{} jobs, {} faces, {} samples, {} wasted, {} submits, most rounds a submit {}, ends at {} counts from {:?} to {:?}",
        several.chunks,
        several.faces,
        several.samples,
        several.wasted,
        several.submits,
        several.max_rounds_in_submit,
        counts.len(),
        counts.first(),
        counts.last()
    );
    assert!(several.chunks > 1, "{} jobs", several.chunks);
    assert_eq!(
        several.max_jobs_in_submit, several.chunks,
        "every job in one submit"
    );
    assert_eq!(spent.len() as u64, several.faces);
    assert!(counts.len() > 2, "faces of different shares: {counts:?}");
    assert!(
        several.max_rounds_in_submit > 1,
        "several rounds a submit: {}",
        several.max_rounds_in_submit
    );
    let stopped = rig.blocks();
    let one_round = rig.bake(&BakeSettings {
        submit_rounds: 1,
        ..stop.clone()
    });
    assert_eq!(one_round.max_rounds_in_submit, 1);
    assert_eq!(
        one_round.wasted, 0,
        "one round a submit throws nothing away"
    );
    assert_eq!(rig.blocks(), stopped, "one round a submit");
    assert_eq!(samples_of(&one_round), spent, "one round a submit");
    let alone = rig.bake(&BakeSettings {
        jobs_alone: true,
        ..stop.clone()
    });
    assert_eq!(alone.max_jobs_in_submit, 1);
    assert!(alone.max_rounds_in_submit > 1);
    assert_eq!(rig.blocks(), stopped, "each job alone");
    assert_eq!(samples_of(&alone), spent, "each job alone");
    let alone_one = rig.bake(&BakeSettings {
        jobs_alone: true,
        submit_rounds: 1,
        ..stop
    });
    assert_eq!(alone_one.max_jobs_in_submit, 1);
    assert_eq!(alone_one.max_rounds_in_submit, 1);
    assert_eq!(rig.blocks(), stopped, "each job alone, one round a submit");
    assert_eq!(
        samples_of(&alone_one),
        spent,
        "each job alone, one round a submit"
    );
    for r in [&one_round, &alone, &alone_one] {
        assert_eq!(r.samples, several.samples);
    }
}

/// After the bake the first frame is the final one: frame 1 and frame 64 are bit-identical, its
/// emissive part is the brick level's value ([`brick_value`]) times the exposure within one half
/// step, the light pass drew nothing, and no face misses the store.
#[test]
fn a_baked_view_is_final_on_its_first_frame() {
    let (world, camera) = small_box();
    let Some(mut rig) = rig_on(world, [64, 36], None) else {
        return;
    };
    rig.bake(&test_counts(1024));
    let cells = rig.cells();
    rig.frame.set_count_store(true);
    rig.step(&camera);
    let first = rig.display();
    let accum = vxe_render::debug::read_float(&rig.device, &rig.queue, rig.frame.accumulated());
    let pos = vxe_render::debug::read_float(&rig.device, &rig.queue, rig.frame.hit_positions());
    let info = vxe_render::debug::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
    let light = vxe_render::debug::read_float(&rig.device, &rig.queue, rig.frame.irradiance());
    let stats = rig.frame.read_store_stats(&rig.device, &rig.queue);
    assert_eq!(stats.missing, 0, "store_missing");
    let eye = camera.position.to_array();
    let (mut checked, mut lit) = (0, 0);
    for i in 0..pos.len() {
        let Some(key) = hit_face(eye, pos[i], info[i]) else {
            continue;
        };
        assert_eq!(
            [light[i][0], light[i][1], light[i][2]],
            [0.0; 3],
            "the light pass's emissive sample at pixel {i}"
        );
        let value =
            brick_value(&cells, key.0, key.1).expect("the face's brick holds its direction");
        let want = value.map(|v| v * 8.0);
        for k in 0..3 {
            let step = (want[k].abs() / 1024.0).max(1.0 / 16384.0);
            assert!(
                (accum[i][k] - want[k]).abs() <= step,
                "pixel {i} channel {k}: {} against {}",
                accum[i][k],
                want[k]
            );
        }
        checked += 1;
        lit += usize::from(value.iter().any(|&v| v > 0.0));
    }
    assert!(checked > 200, "{checked} pixels with a face");
    assert!(lit > 100, "{lit} lit pixels");
    for _ in 1..64 {
        rig.step(&camera);
    }
    assert_eq!(rig.display(), first, "frame 64 against frame 1");
}

/// The store's read counters on a fixed frame: a box of two chunks whose emitter lies in the
/// one-chunk piece that is baked, seen from above, so its cube-face hits split between brick reads
/// (the piece) and no value (the chunk outside, no block). With the counting pipeline
/// (`Frame::set_count_store`) the three counts equal the ones the temporal pass wrote into the
/// store's own words before the counters had a buffer of their own (read at 486a53e through the
/// same `read_store_stats`); with it off the temporal pass writes nothing, so the counter buffer,
/// cleared at the frame's start, still reads zero after it.
#[test]
fn the_store_counters_count_with_the_override_on() {
    let mut t = MaterialTable::new();
    for (id, row) in flythrough::cave_materials()
        .rows()
        .iter()
        .enumerate()
        .skip(1)
    {
        let mut m = row.clone();
        if id == usize::from(ORE) {
            m.emission = [0.05, 0.05, 0.05];
        }
        assert_eq!(usize::from(t.push(m)), id);
    }
    let world = air_world([2, 1, 1], &t);
    fill(&world, [40, 16, 20], [88, 48, 21], ROCK);
    fill(&world, [60, 30, 21], [61, 31, 22], ORE);
    let Some(mut rig) = rig_on(world, [64, 36], None) else {
        return;
    };
    rig.frame
        .bake_until(
            &rig.device,
            &rig.queue,
            &mut rig.gpu,
            &rig.world,
            ChunkExtent::new([1, 0, 0], [2, 1, 1]),
            &dig_counts(),
            &mut |_| {},
        )
        .expect("the piece fits");
    let camera = Camera {
        position: [64.0, 32.0, 40.0].into(),
        yaw: 0.0,
        pitch: (-80f32).to_radians(),
        vfov: 90f32.to_radians(),
    };
    let counts = |s: vxe_render::StoreStats| [s.missing, s.brick_reads, s.coarse_reads];
    rig.frame.set_count_store(true);
    rig.step(&camera);
    let on = counts(rig.frame.read_store_stats(&rig.device, &rig.queue));
    println!("counting: missing, brick reads, coarse reads {on:?}");
    assert_eq!(on, [366, 658, 0], "the counts against the start commit's");
    rig.frame.set_count_store(false);
    rig.step(&camera);
    let off = counts(rig.frame.read_store_stats(&rig.device, &rig.queue));
    assert_eq!(off, [0; 3], "the counters with the override off");
}

/// ADR-0053's rig: chunks A1 (0,0,0) and A2 (0,0,1) baked, each with its own ore emitter over a
/// rock floor (A2's larger), and the blockless floors B (1,0,0), C (1,1,0) and D (4,0,0), under a
/// frame at `size`. z is up; four chunks tall, so the eye above the floors lies inside the
/// mirrored world.
fn fallback_rig(size: [u32; 2]) -> Option<Rig> {
    let world = air_world([5, 2, 4], &cave_table());
    // Floors 4 voxels thick, their top faces at z 8 (layer 0) or 72 (layer 1), inset from the
    // chunk's sides so each floor is its own chunk's.
    let floor = |c: [i64; 3]| {
        let o = c.map(|v| v * 64);
        fill(
            &world,
            [o[0] + 4, o[1] + 4, o[2] + 4],
            [o[0] + 60, o[1] + 60, o[2] + 8],
            ROCK,
        );
    };
    for c in [[0, 0, 0], [0, 0, 1], [1, 0, 0], [1, 1, 0], [4, 0, 0]] {
        floor(c);
    }
    fill(&world, [28, 28, 20], [36, 36, 22], ORE);
    fill(&world, [20, 20, 84], [44, 44, 88], ORE);
    let mut rig = rig_on(world, size, None)?;
    rig.frame
        .bake_until(
            &rig.device,
            &rig.queue,
            &mut rig.gpu,
            &rig.world,
            ChunkExtent::new([0, 0, 0], [1, 1, 2]),
            &dig_counts(),
            &mut |_| {},
        )
        .expect("the piece fits");
    Some(rig)
}

/// The fallback rig's camera: above the floors, looking down.
fn fallback_camera() -> Camera {
    Camera {
        position: [160.0, 64.0, 220.0].into(),
        yaw: 0.0,
        pitch: (-85f32).to_radians(),
        vfov: 90f32.to_radians(),
    }
}

/// ADR-0053's read chain on the GPU. Chunks A1 (0,0,0) and A2 (0,0,1) are baked, each with its
/// own ore emitter over a rock floor (A2's larger), inside level-1 cell (0,0,0); B (1,0,0) is
/// face-adjacent to A1 alone and C (1,1,0) to neither, both blockless floors in the same cell; D
/// (4,0,0) is a blockless floor in a level-2 cell with no block. Seen from above, lamp off, each
/// floor's top face (+z) reads: under `chain` B's stored term A1's summary within one `rgb9e5`
/// step, C's the mean of A1's and A2's (their level-1 record), D's 0; under `parent` B reads that
/// mean; under `neighbour` C reads 0; under `black` B, C and D read 0. Every pixel with no brick
/// value reads the CPU's chain (`LightSummaries::chain`) for its chunk and face; `store_missing`
/// is the same in every mode, and the fallback and still-black counters split it as the chain
/// does (under `chain` B's and C's lit faces are fallback, D's still black). The pool's summary
/// words are the CPU's.
#[test]
fn a_missing_brick_reads_its_neighbour_then_its_parent() {
    use vxe_render::MissingLight;
    let Some(mut rig) = fallback_rig([128, 72]) else {
        return;
    };
    let light = rig.frame.store().light().clone();
    let (a1, a2) = (
        *light.summary([0, 0, 0]).expect("A1 is lit"),
        *light.summary([0, 0, 1]).expect("A2 is lit"),
    );
    // The floors' top faces, +z.
    let top = 5;
    assert_ne!(
        a1[top], a2[top],
        "the two emitters light their floors apart"
    );
    let p1 = *light
        .parent(1, [0, 0, 0])
        .expect("A1 and A2's level-1 cell");
    for (c, s) in [([0, 0, 0], a1), ([0, 0, 1], a2)] {
        let block = rig
            .frame
            .read_store_block(&rig.device, &rig.queue, ChunkCoord(c))
            .expect("a baked block");
        let at = vxe_render::internals::SUMMARY_AT as usize;
        assert_eq!(
            block[at..at + vxe_render::internals::BRICK_WORDS],
            s,
            "{c:?}'s pool summary"
        );
    }
    for c in [[1, 0, 0], [1, 1, 0], [4, 0, 0]] {
        assert!(
            rig.frame.store().block(ChunkCoord(c)).is_none(),
            "{c:?} is blockless"
        );
    }
    let cells = rig.cells();
    let camera = fallback_camera();
    let eye = camera.position.to_array();
    rig.frame.set_count_store(true);
    let mut missing = None;
    for mode in MissingLight::ALL {
        rig.frame.set_missing_light(&rig.queue, mode);
        rig.step(&camera);
        let accum = rig.accum();
        let pos = vxe_render::debug::read_float(&rig.device, &rig.queue, rig.frame.hit_positions());
        let info =
            vxe_render::debug::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
        let stats = rig.frame.read_store_stats(&rig.device, &rig.queue);
        assert_eq!(
            *missing.get_or_insert(stats.missing),
            stats.missing,
            "{mode:?}: store_missing"
        );
        let (mut fallback, mut black) = (0u32, 0u32);
        // Lit +y pixels of B, C and D, and the value each read.
        let mut tops: HashMap<[i32; 3], Vec<[f32; 3]>> = HashMap::new();
        for i in 0..pos.len() {
            let Some((v, f)) = hit_face(eye, pos[i], info[i]) else {
                continue;
            };
            if brick_value(&cells, v, f).is_some() {
                continue;
            }
            let c = v.map(|x| x.div_euclid(64));
            let want = light
                .chain(c, mode)
                .filter(|r| r[6] >> (16 + u32::from(f)) & 1 != 0)
                .map_or([0.0; 3], |r| {
                    vxe_render::internals::rgb9e5_unpack(r[usize::from(f)])
                });
            if want.iter().any(|&x| x > 0.0) {
                fallback += 1;
            } else {
                black += 1;
            }
            let got = [accum[i][0], accum[i][1], accum[i][2]];
            for k in 0..3 {
                let w = want[k] * 8.0;
                let step = (w.abs() / 1024.0).max(1.0 / 16384.0);
                assert!(
                    (got[k] - w).abs() <= step,
                    "{mode:?}: pixel {i} of chunk {c:?} face {f}, channel {k}: {} against {w}",
                    got[k]
                );
            }
            if usize::from(f) == top {
                tops.entry(c).or_default().push(got.map(|x| x / 8.0));
            }
        }
        assert_eq!(
            [stats.fallback, stats.still_black],
            [fallback, black],
            "{mode:?}: the fallback and still-black counters"
        );
        assert_eq!(
            fallback + black,
            stats.missing,
            "{mode:?}: they split the missing"
        );
        let reads = |c: [i32; 3]| -> &Vec<[f32; 3]> {
            let t = tops.get(&c).unwrap_or_else(|| {
                let seen: BTreeMap<_, _> = tops.iter().map(|(k, v)| (*k, v.len())).collect();
                panic!("{mode:?}: no top pixel of {c:?}; seen {seen:?}")
            });
            assert!(t.len() > 20, "{mode:?}: {} top pixels of {c:?}", t.len());
            t
        };
        let all_near = |c: [i32; 3], w: u32| {
            let want = vxe_render::internals::rgb9e5_unpack(w);
            let step = 2f32.powi((w >> 27) as i32 - 24);
            reads(c)
                .iter()
                .all(|g| (0..3).all(|k| (g[k] - want[k]).abs() <= step * 1.01 + want[k] / 1024.0))
        };
        let all_zero = |c: [i32; 3]| reads(c).iter().all(|g| *g == [0.0; 3]);
        match mode {
            MissingLight::Chain => {
                assert!(all_near([1, 0, 0], a1[top]), "chain: B reads A1's summary");
                assert!(
                    all_near([1, 1, 0], p1[top]),
                    "chain: C reads A1's and A2's mean"
                );
                assert!(all_zero([4, 0, 0]), "chain: D reads 0");
            }
            MissingLight::Parent => {
                assert!(all_near([1, 0, 0], p1[top]), "parent: B reads the mean");
                assert!(all_zero([4, 0, 0]), "parent: D reads 0");
            }
            MissingLight::Neighbour => {
                assert!(
                    all_near([1, 0, 0], a1[top]),
                    "neighbour: B reads A1's summary"
                );
                assert!(all_zero([1, 1, 0]), "neighbour: C reads 0");
            }
            MissingLight::Black => {
                for c in [[1, 0, 0], [1, 1, 0], [4, 0, 0]] {
                    assert!(all_zero(c), "black: {c:?} reads 0");
                }
                assert_eq!(stats.fallback, 0, "black: no fallback");
            }
        }
        println!(
            "{mode:?}: missing {}, fallback {}, still black {}",
            stats.missing, stats.fallback, stats.still_black
        );
    }
}

/// The missing light's golden (Plan 0067 Phase 7): the fallback rig at the golden size under the
/// default mode, which the owner picked as `chain` (2026-11-12), lamp off, its first frame after
/// the bake: B lit by A1's summary, C by A1's and A2's level-1 mean, D black. The default is
/// asserted first, so the image is the shipped look.
#[test]
fn the_missing_light_golden() {
    let Some(mut rig) = fallback_rig([support::WIDTH, support::HEIGHT]) else {
        return;
    };
    assert_eq!(
        rig.frame.missing_light(),
        vxe_render::MissingLight::Chain,
        "the default missing light"
    );
    rig.step(&fallback_camera());
    support::assert_golden("missing_light_chain.png", &rig.display());
}

/// The baked light with the filter off (`Lighting::filter`, the fly-through's `--filter off`): on
/// the small box after its bake, lamp off, the mean display luma over the pixels whose cube face
/// holds a lit brick value is within 2 % of the same frame's with the filter on.
#[test]
fn the_baked_light_shows_with_the_filter_off() {
    let (world, camera) = small_box();
    let Some(mut rig) = rig_on(world, [64, 36], None) else {
        return;
    };
    rig.bake(&test_counts(1024));
    let cells = rig.cells();
    let frames = |rig: &mut Rig, filter: bool| {
        rig.frame.set_lighting(Lighting {
            filter,
            ..rig.frame.lighting()
        });
        rig.frame.reset_history();
        for _ in 0..4 {
            rig.step(&camera);
        }
        rig.display()
    };
    let on = frames(&mut rig, true);
    let pos = vxe_render::debug::read_float(&rig.device, &rig.queue, rig.frame.hit_positions());
    let info = vxe_render::debug::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
    let off = frames(&mut rig, false);
    let eye = camera.position.to_array();
    let lit: Vec<usize> = (0..pos.len())
        .filter(|&i| {
            hit_face(eye, pos[i], info[i])
                .and_then(|(v, f)| brick_value(&cells, v, f))
                .is_some_and(|value| value.iter().any(|&c| c > 0.0))
        })
        .collect();
    let mean = |img: &[u8]| {
        let l = luma(img);
        lit.iter().map(|&i| f64::from(l[i])).sum::<f64>() / lit.len() as f64
    };
    let (m_on, m_off) = (mean(&on), mean(&off));
    println!(
        "{} lit pixels: mean luma {m_on:.3} filter on, {m_off:.3} off",
        lit.len()
    );
    assert!(lit.len() > 100, "{} lit pixels", lit.len());
    assert!(m_on > 10.0, "the lit face's luma {m_on}");
    assert!(
        (m_off - m_on).abs() <= 0.02 * m_on,
        "filter off {m_off} against on {m_on}"
    );
}

/// The owner's "moving fast" as a property: a frame reached by a 90 degree yaw in 2 frames reads
/// what a fresh frame at that pose reads, on every pixel with a face.
#[test]
fn a_moving_camera_reads_what_a_still_one_reads() {
    let (world, turned) = small_box();
    let camera = Camera {
        yaw: turned.yaw - 90f32.to_radians(),
        ..turned
    };
    let half = Camera {
        yaw: turned.yaw - 45f32.to_radians(),
        ..turned
    };
    let Some(mut rig) = rig_on(world, [64, 36], None) else {
        return;
    };
    rig.bake(&test_counts(1024));
    let faced = |rig: &Rig, cam: &Camera, img: &[u8]| -> Vec<(usize, [u8; 4])> {
        let pos = vxe_render::debug::read_float(&rig.device, &rig.queue, rig.frame.hit_positions());
        let info =
            vxe_render::debug::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
        (0..pos.len())
            .filter(|&i| hit_face(cam.position.to_array(), pos[i], info[i]).is_some())
            .map(|i| {
                (
                    i,
                    [img[4 * i], img[4 * i + 1], img[4 * i + 2], img[4 * i + 3]],
                )
            })
            .collect()
    };
    rig.step(&camera);
    rig.step(&half);
    rig.step(&turned);
    let moved = rig.display();
    let moved = faced(&rig, &turned, &moved);
    assert!(moved.len() > 100, "{} pixels with a face", moved.len());

    let mut fresh = new_frame(&rig.device, [64, 36]);
    let extent = rig.world.extent();
    fresh
        .bake_until(
            &rig.device,
            &rig.queue,
            &mut rig.gpu,
            &rig.world,
            extent,
            &test_counts(1024),
            &mut |_| {},
        )
        .expect("the box fits the pool");
    std::mem::swap(&mut rig.frame, &mut fresh);
    rig.step(&turned);
    let still = rig.display();
    let still = faced(&rig, &turned, &still);
    assert_eq!(moved, still, "the moved frame against the fresh one");
}

// ---- The dig, the scale, the lamp, the file and the piece (Phase 2) ----

/// A dig's settings beside a bake's: the test counts, the dig's `n_max` the same.
fn dig_counts() -> BakeSettings {
    test_counts(1024)
}

fn sphere(c: [i64; 3], r: i64, mode: Mode) -> EditOp {
    EditOp::sphere(Fixed3(c.map(|v| v * FIXED_ONE)), r * FIXED_ONE, mode)
}

fn box_op(min: [i64; 3], max: [i64; 3], mode: Mode) -> EditOp {
    EditOp::box_(
        Fixed3(min.map(|v| v * FIXED_ONE)),
        Fixed3(max.map(|v| v * FIXED_ONE)),
        mode,
    )
}

/// The directions of `cells` ([`cells_of`]) whose brick is one of `ring`.
fn in_ring(
    cells: &HashMap<([i32; 3], u8), [u32; 2]>,
    ring: &BTreeMap<[i32; 3], Vec<u16>>,
) -> BTreeMap<([i32; 3], u8), [u32; 2]> {
    cells
        .iter()
        .filter(|((g, _), _)| {
            let c = g.map(|x| x.div_euclid(8));
            let b = g.map(|x| x.rem_euclid(8));
            ring.get(&c)
                .is_some_and(|s| s.contains(&(vxe_world::brick_slot(b) as u16)))
        })
        .map(|(k, v)| (*k, *v))
        .collect()
}

/// A lit wall, a radius 3 carve: on the first frame the hole is on screen, every record of the
/// ring is bit-identical to a fresh bake of the edited world (the same counts), every record
/// beyond it is as before, and that frame and frame +64 are bit-identical.
#[test]
fn a_dug_hole_is_final_on_its_first_frame() {
    let (world, camera) = small_box();
    let Some(mut rig) = rig_on(world, [64, 36], None) else {
        return;
    };
    rig.bake(&dig_counts());
    let before = rig.cells();
    let (dig, aabb) = rig.dig(&sphere([48, 48, 56], 3, Mode::Carve), &dig_counts());
    assert!(dig.bake.faces > 0, "the dig baked no face");
    let piece = rig.world.extent();
    let ring = vxe_render::internals::dig_ring(&[aabb], piece);
    let after = rig.cells();
    let fresh = rig.fresh_cells(piece, &dig_counts());
    let (a, f) = (in_ring(&after, &ring), in_ring(&fresh, &ring));
    println!("{} directions in the ring of {}", a.len(), after.len());
    // The ring reaches every brick of the box: its 12 bricks hold over 40 directions.
    assert!(a.len() > 40, "{} directions in the ring", a.len());
    assert_eq!(a, f, "the ring against a fresh bake");
    for (k, v) in &after {
        if !a.contains_key(k) {
            assert_eq!(before.get(k), Some(v), "{k:?} beyond the ring moved");
        }
    }
    // The hole's new faces: directions of bricks within the carve's reach that held none before.
    let new: Vec<_> = after.keys().filter(|k| !before.contains_key(k)).collect();
    assert!(!new.is_empty(), "the carve exposed no face");
    rig.frame.set_count_store(true);
    rig.step(&camera);
    let first = rig.display();
    let pos = vxe_render::debug::read_float(&rig.device, &rig.queue, rig.frame.hit_positions());
    let info = vxe_render::debug::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
    let eye = camera.position.to_array();
    let on_screen = (0..pos.len())
        .filter_map(|i| hit_face(eye, pos[i], info[i]))
        .filter(|(v, f)| !before.contains_key(&(v.map(|x| x.div_euclid(8)), *f)))
        .count();
    println!(
        "dig: {} chunks, {} faces, {} samples; {} new faces, {} hole pixels",
        dig.bake.chunks,
        dig.bake.faces,
        dig.bake.samples,
        new.len(),
        on_screen
    );
    assert!(on_screen > 0, "the hole is on screen");
    let stats = rig.frame.read_store_stats(&rig.device, &rig.queue);
    assert_eq!(stats.missing, 0, "store_missing");
    for _ in 0..64 {
        rig.step(&camera);
    }
    assert_eq!(rig.display(), first, "frame +64 against the dig's frame");
}

/// Carving the box's one emitter brings every record of the ring to exactly 0 on that frame,
/// bit-identical to a fresh bake.
#[test]
fn a_dug_emitter_leaves_the_near_ring_at_once() {
    let world = air_world([1, 1, 1], &cave_table());
    fill(&world, [8, 8, 20], [40, 40, 21], ROCK);
    fill(&world, [22, 22, 30], [24, 24, 32], ORE);
    let Some(mut rig) = rig_on(world, [16, 16], None) else {
        return;
    };
    rig.bake(&dig_counts());
    let lit = rig.cells().values().filter(|c| c[0] != 0).count();
    println!("{lit} lit directions before");
    // The slab's 16 bricks face the ore with their +z.
    assert!(lit >= 16, "{lit} lit directions before");
    let (_, aabb) = rig.dig(
        &box_op([22, 22, 30], [24, 24, 32], Mode::Carve),
        &dig_counts(),
    );
    let piece = rig.world.extent();
    let ring = vxe_render::internals::dig_ring(&[aabb], piece);
    let after = in_ring(&rig.cells(), &ring);
    println!("{} directions in the ring", after.len());
    // The slab's 16 bricks, each with its +z and -z.
    assert!(after.len() >= 32, "{} directions in the ring", after.len());
    let lit: Vec<_> = after.iter().filter(|(_, c)| c[0] != 0).collect();
    assert!(
        lit.is_empty(),
        "{} directions still lit, first {:?}",
        lit.len(),
        lit.first()
    );
    let fresh = in_ring(&rig.fresh_cells(piece, &dig_counts()), &ring);
    assert_eq!(after, fresh, "the ring against a fresh bake");
}

/// `Frame::set_emission_scales` at 0.5 halves a lit wall's displayed irradiance on that frame,
/// within one `rgb9e5` step, the records unchanged; a material at 0 when baked reads its light
/// when raised.
#[test]
fn a_scale_change_moves_no_bake() {
    let (world, camera) = small_box();
    let Some(mut rig) = rig_on(world, [64, 36], None) else {
        return;
    };
    rig.frame.set_emission_scale(&rig.queue, ORE, 0.0);
    rig.bake(&test_counts(1024));
    let cells = rig.cells();
    rig.step(&camera);
    let dark = rig.accum();
    assert!(
        dark.iter()
            .all(|p| p[0] == 0.0 && p[1] == 0.0 && p[2] == 0.0),
        "the ore at 0 lights nothing"
    );
    rig.frame.set_emission_scale(&rig.queue, ORE, 1.0);
    rig.step(&camera);
    let full = rig.accum();
    rig.frame.set_emission_scale(&rig.queue, ORE, 0.5);
    rig.step(&camera);
    let half = rig.accum();
    assert_eq!(rig.cells(), cells, "the records after the scale changes");
    let mut lit = 0;
    for i in 0..full.len() {
        for k in 0..3 {
            let step = (full[i][k].abs() / 512.0).max(1.0 / 16384.0);
            assert!(
                (half[i][k] - 0.5 * full[i][k]).abs() <= step,
                "pixel {i} channel {k}: {} against half of {}",
                half[i][k],
                full[i][k]
            );
        }
        lit += usize::from(full[i][0] > 0.0);
    }
    assert!(lit > 100, "{lit} lit pixels at scale 1");
}

/// With the lamp on a frame's emissive part equals the lamp-off frame's and the records are
/// unchanged; off again, the frame is bit-identical to the first lamp-off frame once the lamp's
/// history has emptied, [`LAMP_EMPTY_FRAMES`] frames after the lamp goes off.
#[test]
fn the_lamp_adds_over_the_bake() {
    let (world, camera) = small_box();
    let Some(mut rig) = rig_on(world, [64, 36], None) else {
        return;
    };
    rig.bake(&test_counts(1024));
    let cells = rig.cells();
    rig.step(&camera);
    let off = rig.display();
    let off_accum = rig.accum();
    let lighting = |lamp: bool| Lighting {
        lamp,
        lamp_bounce: false,
        exposure: Exposure::Manual(8.0),
        bake_pool_bytes: 16 << 20,
        ..Lighting::default()
    };
    rig.frame.set_lighting(lighting(true));
    for _ in 0..4 {
        rig.step(&camera);
    }
    assert_ne!(rig.display(), off, "the lamp shows");
    let rgb = |a: &[[f32; 4]]| -> Vec<[f32; 3]> { a.iter().map(|p| [p[0], p[1], p[2]]).collect() };
    let differ = rgb(&rig.accum())
        .iter()
        .zip(rgb(&off_accum))
        .filter(|(a, b)| **a != *b)
        .count();
    assert_eq!(differ, 0, "pixels whose emissive part moved under the lamp");
    assert_eq!(rig.cells(), cells, "the records under the lamp");
    rig.frame.set_lighting(lighting(false));
    let mut at = None;
    for n in 1..=256 {
        rig.step(&camera);
        if rig.display() == off {
            at = Some(n);
            break;
        }
    }
    println!("lamp off: the first lamp-off frame again at +{at:?}");
    assert_eq!(
        at,
        Some(LAMP_EMPTY_FRAMES),
        "frames after the lamp goes off"
    );
}

/// Frames after the lamp goes off at which the frame is the lamp-off frame again.
const LAMP_EMPTY_FRAMES: u32 = 1;

/// A pane of opacity 0.6 and a dark albedo, one voxel thick, 2.5 voxels ahead of
/// [`small_box`]'s eye, over the block lit by its ore plate: with the lamp off (the headlamp's
/// parameters set, as the window's frames pass them), every pixel and channel reads no brighter
/// than the larger of the block alone and the pane drawn opaque (its own lit face). One rig: the
/// block alone, then the pane dug in as translucent glass, then the same box's glass replaced by its
/// opaque twin, each dig re-baking its ring as the window's frame does.
#[test]
fn a_glass_pane_does_not_brighten_what_it_covers() {
    let mut t = cave_table();
    let rock = t.get(ROCK).expect("rock").clone();
    for translucent in [true, false] {
        let flags = if translucent {
            rock.flags | vxe_world::MaterialFlags::TRANSLUCENT
        } else {
            rock.flags
        };
        t.push(vxe_world::Material {
            name: format!("pane {}", if translucent { "glass" } else { "opaque" }),
            albedo: [0.10, 0.10, 0.14],
            emission: [0.0; 3],
            opacity: 0.6,
            flags,
            ..rock.clone()
        });
    }
    let (glass_id, opaque_id) = (
        (t.len() - 2) as vxe_world::MaterialId,
        (t.len() - 1) as vxe_world::MaterialId,
    );
    let (_, camera) = small_box();
    let world = air_world([2, 2, 2], &t);
    fill(&world, [40, 40, 40], [56, 56, 56], ROCK);
    fill(&world, [42, 42, 72], [54, 54, 74], ORE);
    let Some(mut rig) = rig_on(world, [64, 36], None) else {
        return;
    };
    let settings = test_counts(1024);
    rig.bake(&settings);
    let frame_of = |rig: &mut Rig, name: &str| -> (Vec<u8>, usize) {
        for _ in 0..16 {
            rig.step(&camera);
        }
        let info =
            vxe_render::debug::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
        let covered = info.iter().filter(|r| r[3] & 4 != 0).count();
        println!("{name}: {covered} pixels behind a translucent voxel");
        (rig.display(), covered)
    };
    let pane = |mode: Mode| {
        EditOp::box_(
            Fixed3([33, 16, 40].map(|v: i64| v * FIXED_ONE)),
            Fixed3([34, 72, 80].map(|v: i64| v * FIXED_ONE)),
            mode,
        )
    };
    let (wall, _) = frame_of(&mut rig, "the block alone");
    rig.dig(&pane(Mode::Fill(glass_id)), &settings);
    let (glass, covered) = frame_of(&mut rig, "the glass pane");
    rig.dig(
        &pane(Mode::Replace {
            from: glass_id,
            to: opaque_id,
        }),
        &settings,
    );
    let (opaque, _) = frame_of(&mut rig, "the opaque pane");
    assert!(covered > 1000, "{covered} pixels behind the pane");
    let lit = wall.chunks(4).filter(|p| p[..3] != [0, 0, 0]).count();
    assert!(lit > 100, "{lit} lit pixels of the block alone");
    let mut over = 0;
    let mut first = None;
    for (i, ((g, w), o)) in glass
        .chunks(4)
        .zip(wall.chunks(4))
        .zip(opaque.chunks(4))
        .enumerate()
    {
        for k in 0..3 {
            if g[k] > w[k].max(o[k]) {
                over += 1;
                first.get_or_insert((i, k, g[k], w[k], o[k]));
            }
        }
    }
    assert_eq!(
        over, 0,
        "channels brighter behind the pane than the block alone and the pane's own face; the first (pixel, channel, pane, block, own face): {first:?}"
    );
}

/// The composite's radiance of each pixel's hit before the display and before any translucent
/// solid in front of it (pre-exposed): `albedo * E / PI` plus the emission at exposure `ex`, `E`
/// the irradiance the composite reads (the filter's rgb, or with the filter off the light pass's
/// rgb plus its lamp's white). 0 on a miss.
fn hit_radiance(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    frame: &Frame,
    mats: &MaterialTable,
    ex: f32,
) -> Vec<[f32; 3]> {
    let filter = frame.lighting().filter;
    let info = vxe_render::debug::read_rgba16u(device, queue, frame.hit_records());
    let lit_tex = if filter {
        frame.filtered()
    } else {
        frame.irradiance()
    };
    let lit = vxe_render::debug::read_float(device, queue, lit_tex);
    info.iter()
        .zip(&lit)
        .map(|(r, l)| {
            if r[3] & 1 == 0 {
                return [0.0; 3];
            }
            let m = mats.get(r[2]).expect("a material");
            let s = frame.emission_scale(r[2]);
            let w = if filter { 0.0 } else { l[3] };
            [0, 1, 2]
                .map(|k| m.albedo[k] * (l[k] + w) / std::f32::consts::PI + m.emission[k] * s * ex)
        })
        .collect()
}

/// The length of `o + d s`, `0 <= s < s_end`, inside the boxes `[min, max)` (disjoint).
fn path_in_boxes(o: [f64; 3], d: [f64; 3], s_end: f64, boxes: &[([i32; 3], [i32; 3])]) -> f64 {
    let mut sum = 0.0;
    for (min, max) in boxes {
        let (mut a, mut b) = (0.0f64, s_end);
        for k in 0..3 {
            let (lo, hi) = (f64::from(min[k]), f64::from(max[k]));
            if d[k] == 0.0 {
                if o[k] < lo || o[k] >= hi {
                    b = a;
                }
                continue;
            }
            let (p, q) = ((lo - o[k]) / d[k], (hi - o[k]) / d[k]);
            a = a.max(p.min(q));
            b = b.min(p.max(q));
        }
        sum += (b - a).max(0.0);
    }
    sum
}

/// ADR-0047's filter (lavapipe, a 64 x 36 frame, lamp off, exposure 8). A wall lit by an ore
/// strip on its own side, behind a pane 1 voxel thick on one half and 2 on the other; no
/// emitter sees the pane's camera-side faces, so their records are 0 and each pixel is `T(L)` of
/// what it showed before the pane: `L` the exact path through the glass boxes along the pixel's
/// ray, `T = tint^L` per channel, `tint = 0.8 (0.5 + 0.5 albedo / max(albedo))` from the row's
/// albedo. Within 2 levels, head on (the halves at `L` near 1 and 2: `T` and `T^2`) and at 30
/// degrees of yaw (`L` near 2.3 through the 2-voxel half). Then a pane lit from the camera's side
/// over a black wall: each pixel's reflected light before exposure (the layer target's rgb) is
/// Schlick's `R` at F0 0.04 at its angle times `E / PI`, `E` the brick level's value of its entry
/// face ([`brick_value`] of the store's records, times the ore's emission scale), above 0 and
/// within 10 %, and the pixel shows it. With the ore strip carved away (its ring re-baked), the
/// same pixels reflect 0. The per-layer composite fails the first.
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one rig read four ways (each pane, the carve, the composite) against one baked store"
)]
fn a_glass_pane_filters_by_path_length() {
    let mut t = cave_table();
    let rock = t.get(ROCK).expect("rock").clone();
    let albedo = [0.25, 0.35, 0.85];
    t.push(vxe_world::Material {
        name: "glass".to_owned(),
        albedo,
        emission: [0.0; 3],
        opacity: 0.6,
        flags: rock.flags | vxe_world::MaterialFlags::TRANSLUCENT,
        ..rock.clone()
    });
    let glass_id = (t.len() - 1) as vxe_world::MaterialId;
    t.push(vxe_world::Material {
        name: "black".to_owned(),
        albedo: [0.0; 3],
        emission: [0.0; 3],
        ..rock.clone()
    });
    let black_id = (t.len() - 1) as vxe_world::MaterialId;
    let mats = t.clone();
    let world = air_world([2, 2, 2], &t);
    // The wall and its ore strip above, out of the views.
    fill(&world, [40, 52, 22], [41, 100, 40], ROCK);
    fill(&world, [34, 52, 42], [36, 100, 44], ORE);
    // The lit pane's black wall and its ore strip, on the camera's side of the pane.
    fill(&world, [84, 54, 92], [85, 88, 118], black_id);
    fill(&world, [100, 56, 114], [104, 84, 116], ORE);
    let halves: [([i32; 3], [i32; 3]); 2] =
        [([30, 56, 20], [31, 70, 42]), ([30, 70, 20], [32, 86, 42])];
    let lit_pane = ([96, 56, 96], [98, 84, 112]);
    let size = [64u32, 36];
    let Some(mut rig) = rig_on(world, size, None) else {
        return;
    };
    let settings = test_counts(1024);
    rig.bake(&settings);
    let ex = 8.0f32;
    let head_on = Camera {
        position: [20.0, 70.0, 31.0].into(),
        yaw: 0.0,
        pitch: 0.0,
        vfov: 30f32.to_radians(),
    };
    let oblique = Camera {
        position: [31.0 - 8.66, 72.0, 31.0].into(),
        yaw: 30f32.to_radians(),
        ..head_on
    };
    let lit_view = Camera {
        position: [108.0, 70.0, 104.0].into(),
        yaw: 180f32.to_radians(),
        ..head_on
    };
    let lighting = rig.frame.lighting();
    let level = |c: [f32; 3]| {
        let b = tone_bytes([c[0], c[1], c[2], 1.0], &lighting);
        0.2126 * f32::from(b[0]) + 0.7152 * f32::from(b[1]) + 0.0722 * f32::from(b[2])
    };
    let settle = |rig: &mut Rig, camera: &Camera| {
        for _ in 0..8 {
            rig.step(camera);
        }
    };
    let mut before = Vec::new();
    for camera in [&head_on, &oblique] {
        settle(&mut rig, camera);
        before.push(hit_radiance(&rig.device, &rig.queue, &rig.frame, &mats, ex));
    }
    for (min, max) in halves.iter().chain([&lit_pane]) {
        rig.dig(
            &EditOp::box_(
                Fixed3(min.map(|v| i64::from(v) * FIXED_ONE)),
                Fixed3(max.map(|v| i64::from(v) * FIXED_ONE)),
                Mode::Fill(glass_id),
            ),
            &settings,
        );
    }
    let top = albedo.iter().fold(0f32, |a, b| a.max(*b));
    let tint = albedo.map(|a| f64::from(0.8 * (0.5 + 0.5 * a / top)));
    for (camera, c0, name) in [
        (&head_on, &before[0], "head on"),
        (&oblique, &before[1], "30 degrees"),
    ] {
        settle(&mut rig, camera);
        let shown = luma(&rig.display());
        let info =
            vxe_render::debug::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
        let pos = vxe_render::debug::read_float(&rig.device, &rig.queue, rig.frame.hit_positions());
        let eye = camera.position.to_array().map(f64::from);
        let (mut worst, mut n) = (0f32, 0usize);
        let mut by_half: [Vec<f64>; 2] = [Vec::new(), Vec::new()];
        for i in 0..info.len() {
            if info[i][3] & 1 == 0 || info[i][3] & 4 == 0 {
                continue;
            }
            let to = [0, 1, 2].map(|k| f64::from(pos[i][k]) - eye[k]);
            let s_end = (to[0] * to[0] + to[1] * to[1] + to[2] * to[2]).sqrt();
            let d = to.map(|v| v / s_end);
            let lens = halves.map(|b| path_in_boxes(eye, d, s_end, &[b]));
            let l = lens[0] + lens[1];
            let tr = tint.map(|x| x.powf(l) as f32);
            let want = level([0, 1, 2].map(|k| tr[k] * c0[i][k]));
            let off = (shown[i] - want).abs();
            if off > worst {
                worst = off;
            }
            n += 1;
            if lens[1] == 0.0 {
                by_half[0].push(l);
            } else if lens[0] == 0.0 {
                by_half[1].push(l);
            }
        }
        let median = |v: &mut Vec<f64>| {
            v.sort_by(f64::total_cmp);
            v.get(v.len() / 2).copied().unwrap_or(0.0)
        };
        let (n1, n2) = (by_half[0].len(), by_half[1].len());
        let (l1, l2) = (median(&mut by_half[0]), median(&mut by_half[1]));
        println!(
            "{name}: {n} pane pixels; L median {l1:.3} over {n1} (1 voxel), {l2:.3} over {n2} (2 voxels); worst |pixel - T(L) behind| {worst:.2} levels"
        );
        assert!(n > 1000, "{name}: {n} pixels behind the pane");
        assert!(
            worst <= 2.0,
            "{name}: a pixel {worst:.2} levels from T(L) of what it showed before the pane"
        );
        if camera.yaw == 0.0 {
            assert!(n1 > 300 && n2 > 300, "{name}: halves {n1} and {n2}");
            assert!(
                (1.0..1.1).contains(&l1) && (2.0..2.2).contains(&l2),
                "{l1} {l2}"
            );
        } else {
            assert!(
                n2 > 300 && (2.25..2.4).contains(&l2),
                "{name}: {n2} at {l2}"
            );
        }
    }
    // The lit pane over the black wall: its reflected share from its +x face's brick record.
    settle(&mut rig, &lit_view);
    let shown = luma(&rig.display());
    let info = vxe_render::debug::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
    let pos = vxe_render::debug::read_float(&rig.device, &rig.queue, rig.frame.hit_positions());
    let layer = vxe_render::debug::read_float(&rig.device, &rig.queue, rig.frame.layers());
    let cells = rig.cells();
    let scale = rig.frame.emission_scale(ORE);
    let eye = lit_view.position.to_array().map(f64::from);
    let lum = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let (mut lit, mut dark, mut worst, mut worst_shown) = (Vec::new(), 0usize, 0f32, 0f32);
    for i in 0..info.len() {
        if info[i][3] & 1 == 0 || info[i][3] & 4 == 0 || info[i][2] != black_id {
            continue;
        }
        let to = [0, 1, 2].map(|k| f64::from(pos[i][k]) - eye[k]);
        let len = (to[0] * to[0] + to[1] * to[1] + to[2] * to[2]).sqrt();
        let d = to.map(|v| v / len);
        // The entry: the plane x = 98, the pane's +x face; its voxel is x = 97.
        let s = (98.0 - eye[0]) / d[0];
        let p = [0, 1, 2].map(|k| eye[k] + d[k] * s);
        let v = [97, p[1].floor() as i32, p[2].floor() as i32];
        let Some(e) = brick_value(&cells, v, 1) else {
            panic!("no record at the pane's face {v:?}");
        };
        let c = (1.0 - d[0].abs()) as f32;
        let r = 0.04 + 0.96 * c.powi(5);
        let want = e.map(|x| r * x * scale / std::f32::consts::PI);
        let got = [layer[i][0], layer[i][1], layer[i][2]];
        if lum(want) <= 0.0 {
            dark += 1;
            continue;
        }
        lit.push(i);
        worst = worst.max((lum(got) / lum(want) - 1.0).abs());
        worst_shown = worst_shown.max((shown[i] - level(got.map(|x| x * ex))).abs());
    }
    println!(
        "lit pane: {} pixels over the black wall, {dark} with E 0; worst |reflected / (R E / PI) - 1| {worst:.4}; worst |pixel - display(reflected)| {worst_shown:.2} levels",
        lit.len()
    );
    assert!(lit.len() > 1000, "{} lit pane pixels", lit.len());
    assert_eq!(dark, 0, "pane pixels whose entry face's record reads 0");
    assert!(worst <= 0.10, "reflected light {worst:.4} off R E / PI");
    assert!(
        worst_shown <= 2.0,
        "a pixel {worst_shown:.2} levels off its reflected light"
    );
    // The control: no emitter sees the pane, so the same pixels reflect nothing.
    rig.dig(
        &EditOp::box_(
            Fixed3([100, 56, 114].map(|v| v * FIXED_ONE)),
            Fixed3([104, 84, 116].map(|v| v * FIXED_ONE)),
            Mode::Carve,
        ),
        &settings,
    );
    settle(&mut rig, &lit_view);
    let layer = vxe_render::debug::read_float(&rig.device, &rig.queue, rig.frame.layers());
    let unlit = lit
        .iter()
        .filter(|&&i| layer[i][0] != 0.0 || layer[i][1] != 0.0 || layer[i][2] != 0.0)
        .count();
    println!("no emitter: {unlit} of {} pane pixels reflect", lit.len());
    assert_eq!(unlit, 0, "pane pixels reflecting with no emitter");
}

/// A box of two chunks whose emitter lies outside a one-chunk piece: the piece's records are a
/// whole bake's, the chunk outside has no block and its hits count in `store_missing`; a carve
/// that straddles the piece's edge re-bakes only records inside it, bit-identical to a fresh piece bake
/// of the edited world, and makes no block outside.
#[test]
fn a_piece_reads_what_the_whole_world_reads() {
    let mut t = MaterialTable::new();
    for (id, row) in flythrough::cave_materials()
        .rows()
        .iter()
        .enumerate()
        .skip(1)
    {
        let mut m = row.clone();
        if id == usize::from(ORE) {
            m.emission = [0.05, 0.05, 0.05];
        }
        assert_eq!(usize::from(t.push(m)), id);
    }
    let world = air_world([2, 1, 1], &t);
    fill(&world, [40, 16, 20], [88, 48, 21], ROCK);
    fill(&world, [60, 30, 21], [61, 31, 22], ORE);
    let Some(mut rig) = rig_on(world, [64, 36], None) else {
        return;
    };
    let settings = dig_counts();
    rig.bake(&settings);
    let whole = rig.cells();
    let piece = ChunkExtent::new([1, 0, 0], [2, 1, 1]);
    rig.frame
        .bake_until(
            &rig.device,
            &rig.queue,
            &mut rig.gpu,
            &rig.world,
            piece,
            &settings,
            &mut |_| {},
        )
        .expect("the piece fits");
    let blocks = rig.blocks();
    assert!(
        !blocks.contains_key(&[0, 0, 0]),
        "a block outside the piece"
    );
    let cells = cells_of(&blocks);
    let inside: BTreeMap<_, _> = whole.iter().filter(|((g, _), _)| g[0] >= 8).collect();
    let lit = cells.values().filter(|c| c[0] != 0).count();
    println!("{lit} lit directions in the piece");
    // The piece's 12 slab bricks face the ore with their +z.
    assert!(lit >= 12, "{lit} lit directions in the piece");
    assert_eq!(
        cells.iter().collect::<BTreeMap<_, _>>(),
        inside,
        "the piece against the whole bake"
    );
    let camera = Camera {
        position: [64.0, 32.0, 40.0].into(),
        yaw: 0.0,
        pitch: (-80f32).to_radians(),
        vfov: 90f32.to_radians(),
    };
    rig.frame.set_count_store(true);
    rig.step(&camera);
    let stats = rig.frame.read_store_stats(&rig.device, &rig.queue);
    println!("store_missing {} over the chunk outside", stats.missing);
    assert!(stats.missing > 100, "store_missing {}", stats.missing);
    let (dig, _) = rig.dig(&sphere([64, 30, 20], 3, Mode::Carve), &settings);
    assert!(dig.bake.faces > 0);
    let blocks = rig.blocks();
    assert!(
        !blocks.contains_key(&[0, 0, 0]),
        "a block outside after the dig"
    );
    let fresh = rig.fresh_cells(piece, &settings);
    assert_eq!(
        cells_of(&blocks),
        fresh,
        "the dug piece against a fresh piece bake"
    );
}

/// The staircase at 96 x 54 (Plan 0061 Phase 2 done-when 1b, at Phase 5's defaults): the
/// fly-through's far normal (`LookPreset::DEFAULT`, `[2, 6]`) gives a voxel the far normal's
/// full weight where it covers at most 2 pixels, `t >= 96 / (2 tan 35 deg) / 2 = 34.3` voxels;
/// every rock riser or tread pixel the far camera hits past `t = 36` (all of them: its hits start
/// about 125 voxels away) reads, with the fly-through's store read and weights at full light
/// resolution, ADR-0049's mix within 1 % (the CPU's `by_normal` over the bake's records), and
/// with the library's (`Face`, `Normal`) its own face's value within 1 %, risers and treads
/// both occurring. The frame at the fly-through's defaults (half light resolution) matches
/// `far_staircase_store_normal.png` at the harness's floor (at most 0.1 % of pixels over
/// 2/255; lavapipe).
#[test]
fn a_small_far_staircase_reads_the_mix_and_matches_its_golden() {
    use vxe_render::{LightWeights, NormalMode, StoreRead};
    const SIZE: [u32; 2] = [96, 54];
    const LINE: f32 = 36.0;
    let [camera, _] = stair_cameras(SIZE);
    let Some(mut rig) = rig_on(staircase(), SIZE, None) else {
        return;
    };
    rig.frame.set_lighting(Lighting {
        lamp: false,
        lamp_bounce: false,
        exposure: Exposure::Manual(8.0),
        bake_pool_bytes: 64 << 20,
        ..Lighting::default()
    });
    rig.bake(&test_counts(256));
    let cells = rig.cells();
    let eye = camera.position.to_array();
    let far_normal = flythrough::LookPreset::DEFAULT.far_normal;
    let arm = |read, weights, resolution| StairArm {
        read,
        weights,
        far_normal,
        mode: NormalMode::Face,
        resolution,
    };
    let on = stair_frame(
        &mut rig,
        &camera,
        &arm(
            flythrough::DEFAULT_STORE_READ,
            flythrough::DEFAULT_LIGHT_WEIGHTS,
            LightingResolution::Full,
        ),
    );
    let off = stair_frame(
        &mut rig,
        &camera,
        &arm(
            StoreRead::Face,
            LightWeights::Normal,
            LightingResolution::Full,
        ),
    );
    let (mut checked, mut bad_on, mut bad_off, mut risers, mut treads) = (0, 0, 0, 0, 0);
    for i in 0..on.pos.len() {
        let Some((v, face)) = hit_face(eye, on.pos[i], on.info[i]) else {
            continue;
        };
        if on.info[i][2] != ROCK || on.pos[i][3] < LINE || !(face == 0 || face == 5) {
            continue;
        }
        let n = support::rigs::octa16_normal(on.info[i][0], on.info[i][1]);
        let Some(want) = by_normal(&cells, v, face, n) else {
            continue;
        };
        checked += 1;
        if !within(rgb(on.accum[i]), want, 0.01) {
            bad_on += 1;
        }
        let own = brick_value(&cells, v, face).expect("a mixed face has its own value");
        if !within(rgb(off.accum[i]), own, 0.01) {
            bad_off += 1;
        }
        if face == 0 {
            risers += 1;
        } else {
            treads += 1;
        }
    }
    println!(
        "{checked} rock pixels past t {LINE}: {risers} risers, {treads} treads; {bad_on} off the mix, {bad_off} off their face's value"
    );
    assert!(risers > 0 && treads > 0, "{risers} risers, {treads} treads");
    assert_eq!((bad_on, bad_off), (0, 0), "of {checked}");
    let defaults = stair_frame(
        &mut rig,
        &camera,
        &arm(
            flythrough::DEFAULT_STORE_READ,
            flythrough::DEFAULT_LIGHT_WEIGHTS,
            LightingResolution::default(),
        ),
    );
    let path = support::golden_path("far_staircase_store_normal.png");
    if std::env::var("VXE_BLESS").as_deref() == Ok("1") {
        support::write_png(&path, &defaults.display, SIZE[0], SIZE[1]);
        println!("blessed {path:?}");
        return;
    }
    let (golden, w, h) = support::read_png(&path);
    assert_eq!([w, h], SIZE);
    let f = support::fraction_differing(&defaults.display, &golden, 2);
    println!(
        "far_staircase_store_normal.png: {:.4} % of pixels differ by more than 2/255",
        f * 100.0
    );
    assert!(f <= 0.001, "{:.4} % of pixels differ", f * 100.0);
}

/// The staircase baked for the near-face tests at `size`, lit as the staircase tests are.
fn baked_staircase(size: [u32; 2]) -> Option<Rig> {
    let mut rig = rig_on(staircase(), size, None)?;
    rig.frame.set_lighting(Lighting {
        lamp: false,
        lamp_bounce: false,
        exposure: Exposure::Manual(8.0),
        bake_pool_bytes: 64 << 20,
        ..Lighting::default()
    });
    rig.bake(&test_counts(256));
    Some(rig)
}

/// The pixels of two staircase frames whose read-backs differ: (display bytes, accumulation
/// pixels).
fn frame_diff(a: &StairFrame, b: &StairFrame) -> (usize, usize) {
    (
        a.display
            .iter()
            .zip(&b.display)
            .filter(|(x, y)| x != y)
            .count(),
        a.accum.iter().zip(&b.accum).filter(|(x, y)| x != y).count(),
    )
}

/// ADR-0049's read leaves a near face to the bit (Plan 0061 Phase 2 done-when 2). At 192 x 108
/// under `--look face` with far normal `[1, 2]` the far normal's weight is 0 where a voxel spans
/// 2 pixels or more, `t <= 192 / (2 tan 35 deg) / 2 = 68.6` voxels, so the hit's normal is its
/// face's there and the read takes the face's own value: every hit pixel of the near camera at
/// `t <= 64` holds the same accumulated (stored) value bit for bit with the store read `Normal`
/// and `Face` (full light resolution, weights `Normal`), over at least 10 000 such pixels. With
/// the far normal off the far camera's display and accumulation are bit-identical, read
/// `Normal` against `Face`. Under `gradient27` and `trilinear8` (the normal leaves the face at
/// every distance) with the read on, every stored part of the far camera's frame is finite.
#[test]
fn the_store_read_by_normal_leaves_near_faces_alone() {
    use vxe_render::{LightWeights, NormalMode, StoreRead};
    const SIZE: [u32; 2] = [192, 108];
    const NEAR: f32 = 64.0;
    let [camera, near_camera] = stair_cameras(SIZE);
    let Some(mut rig) = baked_staircase(SIZE) else {
        return;
    };
    let arm = |read, far_normal, mode| StairArm {
        read,
        weights: LightWeights::Normal,
        far_normal,
        mode,
        resolution: LightingResolution::Full,
    };
    let face = stair_frame(
        &mut rig,
        &near_camera,
        &arm(StoreRead::Face, [1.0, 2.0], NormalMode::Face),
    );
    let normal = stair_frame(
        &mut rig,
        &near_camera,
        &arm(StoreRead::Normal, [1.0, 2.0], NormalMode::Face),
    );
    let near: Vec<usize> = (0..normal.pos.len())
        .filter(|&i| normal.info[i][3] & 1 != 0 && normal.pos[i][3] <= NEAR)
        .collect();
    let moved = near
        .iter()
        .filter(|&&i| normal.accum[i] != face.accum[i])
        .count();
    println!(
        "near camera, far normal [1, 2]: {moved} of {} hit pixels at t <= {NEAR} differ read Normal against Face",
        near.len()
    );
    assert!(near.len() >= 10_000, "{} near hit pixels", near.len());
    assert_eq!(moved, 0, "of {}", near.len());

    let face = stair_frame(
        &mut rig,
        &camera,
        &arm(StoreRead::Face, [0.0, 0.0], NormalMode::Face),
    );
    let normal = stair_frame(
        &mut rig,
        &camera,
        &arm(StoreRead::Normal, [0.0, 0.0], NormalMode::Face),
    );
    let diff = frame_diff(&normal, &face);
    println!(
        "far camera, far normal off: read Normal against Face differs in {} display bytes, {} accumulated pixels",
        diff.0, diff.1
    );
    assert_eq!(diff, (0, 0));

    for mode in [NormalMode::Gradient27, NormalMode::Trilinear8] {
        let f = stair_frame(&mut rig, &camera, &arm(StoreRead::Normal, [1.0, 2.0], mode));
        let nonfinite = f
            .accum
            .iter()
            .filter(|p| p.iter().any(|x| !x.is_finite()))
            .count();
        println!("{mode:?}, read Normal: {nonfinite} non-finite stored parts");
        assert_eq!(nonfinite, 0, "{mode:?}");
    }
}

/// The face weights leave a frame with the far normal off to the bit (Plan 0061 Phase 2
/// done-when 2b): there every hit's normal is its face's, so the 2 x 2 upsample's and the
/// a-trous pass's weights read the same dot either way. At 96 x 54 under `--look face`, for the
/// far and the near camera, the display is bit-identical with weights `Face` against `Normal`
/// (store read `Face`) at half and at full light resolution, and with the fly-through's store
/// read and weights against the library's (`Face`, `Normal`) at half.
#[test]
fn the_face_weights_leave_near_faces_alone() {
    use vxe_render::{LightWeights, NormalMode, StoreRead};
    const SIZE: [u32; 2] = [96, 54];
    let Some(mut rig) = baked_staircase(SIZE) else {
        return;
    };
    let arm = |read, weights, resolution| StairArm {
        read,
        weights,
        far_normal: [0.0, 0.0],
        mode: NormalMode::Face,
        resolution,
    };
    let half = LightingResolution::Half;
    let pairs = [
        (
            "weights Face against Normal, half",
            arm(StoreRead::Face, LightWeights::Face, half),
            arm(StoreRead::Face, LightWeights::Normal, half),
        ),
        (
            "weights Face against Normal, full",
            arm(
                StoreRead::Face,
                LightWeights::Face,
                LightingResolution::Full,
            ),
            arm(
                StoreRead::Face,
                LightWeights::Normal,
                LightingResolution::Full,
            ),
        ),
        (
            "the fly-through's read and weights against the library's, half",
            arm(
                flythrough::DEFAULT_STORE_READ,
                flythrough::DEFAULT_LIGHT_WEIGHTS,
                half,
            ),
            arm(StoreRead::Face, LightWeights::Normal, half),
        ),
    ];
    for (c, camera) in stair_cameras(SIZE).iter().enumerate() {
        for (name, a, b) in &pairs {
            let fa = stair_frame(&mut rig, camera, a);
            let fb = stair_frame(&mut rig, camera, b);
            let lit = fa.display.chunks(4).filter(|p| p[..3] != [0, 0, 0]).count();
            let (bytes, _) = frame_diff(&fa, &fb);
            println!(
                "camera {c}, far normal off, {name}: {bytes} display bytes differ ({lit} lit pixels)"
            );
            assert!(lit > 0, "camera {c}, {name}: a black frame");
            assert_eq!(bytes, 0, "camera {c}, {name}");
        }
    }
}
