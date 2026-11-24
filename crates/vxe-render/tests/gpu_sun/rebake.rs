// The key light's re-bake as the sun moves (ADR-0079, Plan 0095 Phase 8): a block whose key
// visibilities were baked under a sun the current one has turned past (beyond the disc's radius
// and `Lighting::key_step`, 3 degrees by default) re-bakes its key words alone in the live
// queue; an edit's ring re-bakes them with the sky's. The live queue at 64 samples a brick
// direction, no coarse pass, the world mirrored whole, stepped with the device waited on.

/// The live queue started on `f` over `gpu`, at 64 samples a brick direction, no coarse pass.
fn start_key_queue(queue: &wgpu::Queue, f: &mut Frame, gpu: &GpuWorld) {
    let settings = vxe_render::BakeSettings {
        brick_samples: 64,
        round: 64,
        ..Default::default()
    };
    f.start_bake_queue(queue, gpu, settings, 1.0e9);
    f.set_bake_coarse(0, 0);
}

/// One step of the queue ([`dig_queue_step`] from `cam`), the device waited on: its stats.
fn key_step(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    f: &mut Frame,
    gpu: &mut GpuWorld,
    world: &World,
    cam: &Camera,
) -> vxe_render::QueueStats {
    dig_queue_step(device, queue, f, gpu, world, cam);
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("device poll");
    f.bake_queue().expect("the queue runs").stats()
}

/// Steps the queue until every chunk of `world` holds a block and nothing is pending or
/// fading, 400 steps at most: the steps taken.
fn bake_to_rest(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    f: &mut Frame,
    gpu: &mut GpuWorld,
    world: &World,
    cam: &Camera,
) -> usize {
    let chunks = world_chunks(world);
    for i in 0..400 {
        let st = key_step(device, queue, f, gpu, world, cam);
        let all = chunks.iter().all(|c| f.store().block_words(*c).is_some());
        if all && st.jobs == 0 && st.depth == 0 && st.fading == 0 && st.keys == 0 {
            return i + 1;
        }
    }
    panic!("rig fault: the queue did not come to rest in 400 steps");
}

fn world_chunks(world: &World) -> Vec<ChunkCoord> {
    let e = world.extent();
    let s = e.size();
    let mut out = Vec::new();
    for z in 0..s[2] {
        for y in 0..s[1] {
            for x in 0..s[0] {
                out.push(ChunkCoord([e.min.0[0] + x, e.min.0[1] + y, e.min.0[2] + z]));
            }
        }
    }
    out
}

/// The sun at elevation `el` in the west (the moon below the horizon), the real radius.
fn west_sky(el: f32) -> Sky {
    sky(dir(el, 270.0), dir(-40.0, 0.0), [[1.0; 3], [0.0; 3]])
}

/// The block's sun stamp ([`vxe_render::internals::KEY_SUN_AT`]) as a direction.
fn stamp_of(f: &Frame, c: ChunkCoord) -> Option<[f32; 3]> {
    let block = f.store().block_words(c)?;
    vxe_render::internals::unpack_direction(block[vxe_render::internals::KEY_SUN_AT as usize])
}

fn near_dir(a: [f32; 3], b: [f32; 3]) -> bool {
    (0..3).all(|k| (a[k] - b[k]).abs() < 1e-3)
}

/// Bar: a fixed sun queues no key job over 600 stepped frames after the bake comes to rest; a
/// turn of 1 degree (under the step and the default 3-degree key step) queues none; a turn of
/// 4 degrees from the baked sun queues one for every block (four, the floor of 2 x 2 x 1
/// chunks) at the step it is seen.
#[test]
fn a_fixed_sun_queues_no_key_job_and_a_turn_past_the_step_queues_every_block() {
    let Some((device, queue)) = device() else {
        return;
    };
    let world = floor_world([2, 2, 1], |_| false);
    let mut gpu = mirror(&device, &queue, &world, None);
    let mut f = frame(&device, SIZE, ShadowWalk::Full);
    f.set_sky(Some(west_sky(40.0)));
    start_key_queue(&queue, &mut f, &gpu);
    let cam = camera([64.0, 64.0]);
    bake_to_rest(&device, &queue, &mut f, &mut gpu, &world, &cam);
    let blocks = world_chunks(&world)
        .into_iter()
        .filter(|c| f.store().block_words(*c).is_some())
        .count() as u64;
    assert_eq!(blocks, 4);
    for i in 0..600 {
        let st = key_step(&device, &queue, &mut f, &mut gpu, &world, &cam);
        assert_eq!((st.keys, st.keys_baked), (0, 0), "a fixed sun at step {i}");
    }
    f.set_sky(Some(west_sky(41.0)));
    let st = key_step(&device, &queue, &mut f, &mut gpu, &world, &cam);
    assert_eq!(st.keys, 0, "a 1-degree turn");
    f.set_sky(Some(west_sky(44.0)));
    let st = key_step(&device, &queue, &mut f, &mut gpu, &world, &cam);
    assert_eq!(st.keys, blocks, "a 4-degree turn");
}

/// The slab-edge rig of the far tests (rock to z 26 over x < 24 on the floor of one chunk), the
/// sun in the west: the floor's `+z` key level of brick column x 4 (x 32..40, z brick 1), the
/// mean over its eight y bricks.
fn edge_level(f: &Frame) -> f64 {
    let levels: Vec<u32> = (0..8)
        .map(|y| brick_key(f, [4, y, 1], UP).expect("the floor brick's key word"))
        .collect();
    f64::from(levels.iter().sum::<u32>()) / 8.0
}

/// Bar: on the slab-edge rig baked under a sun at elevation 40 in the west, turning the sun to
/// 36 (4 degrees, past the step) re-bakes the block's key words alone: once the key job lands
/// (`keys_baked` 1, `keys` 0), every brick record's words but its two key words are byte-equal
/// to the baked ones, the block's sun stamp is the new sun, and the floor's `+z` key level at
/// brick column x 4 falls by over 30 levels: the shadow's edge moves away from the slab with
/// the lower sun (arithmetic: the slab's 10 voxels cast `10 / tan 40` = 11.9 voxels, to
/// x 35.9, about half of the brick lit; `10 / tan 36` = 13.8, to x 37.8, about a quarter; 64
/// key rays a brick direction).
#[test]
fn a_turn_past_the_step_rebakes_the_key_words_alone() {
    let Some((device, queue)) = device() else {
        return;
    };
    let slab = floor_world([1, 1, 1], |p| p[2] < FLOOR + 10 && p[0] < 24);
    let mut gpu = mirror(&device, &queue, &slab, None);
    let mut f = frame(&device, SIZE, ShadowWalk::Full);
    f.set_sky(Some(west_sky(40.0)));
    start_key_queue(&queue, &mut f, &gpu);
    let cam = camera([32.0, 32.0]);
    bake_to_rest(&device, &queue, &mut f, &mut gpu, &slab, &cam);
    let c = ChunkCoord([0, 0, 0]);
    let before = f.store().block_words(c).expect("the block").to_vec();
    let level_before = edge_level(&f);
    f.set_sky(Some(west_sky(36.0)));
    let mut landed = None;
    for i in 0..200 {
        let st = key_step(&device, &queue, &mut f, &mut gpu, &slab, &cam);
        if st.keys_baked >= 1 && st.keys == 0 {
            landed = Some(i);
            break;
        }
    }
    let landed = landed.expect("the key job lands within 200 steps");
    let after = f.store().block_words(c).expect("the block").to_vec();
    let level_after = edge_level(&f);
    println!(
        "key job landed at step {landed}; x 4's +z level {level_before:.1} -> {level_after:.1}"
    );
    assert!(near_dir(stamp_of(&f, c).expect("a stamp"), dir(36.0, 270.0)));
    let w = vxe_render::internals::BRICK_WORDS;
    let key = (vxe_render::internals::KEY_SUN_AT - vxe_render::internals::SUMMARY_AT) as usize;
    assert_eq!(before.len(), after.len());
    let dirs = &before[..512];
    assert_eq!(dirs, &after[..512], "the directory");
    let mut records = 0;
    for &d in dirs.iter().filter(|&&d| d != 0) {
        let end = (d >> 11) as usize;
        for i in end - w..end {
            if !(key..key + 2).contains(&(i - (end - w))) {
                assert_eq!(before[i], after[i], "record word {} of brick at {end}", i - (end - w));
            }
        }
        records += 1;
    }
    assert!(records > 40, "{records} brick records");
    assert!(
        level_after < level_before - 30.0,
        "x 4's +z level {level_before:.1} -> {level_after:.1}"
    );
}

/// The far rig's frame ([`far_frame`]'s settings) at 64 x 64, the sun at elevation `el` in the
/// west.
fn small_far_frame(device: &wgpu::Device, el: f32) -> Frame {
    let mut f = frame(device, [64, 64], ShadowWalk::Full);
    f.set_look(vxe_render::LookParams {
        far_normal: [1000.0, 2000.0],
        ..f.look()
    });
    f.set_lighting(Lighting {
        key_far: vxe_render::KeyFar::Baked,
        ..f.lighting()
    });
    f.set_sky(Some(far_sky(el)));
    f
}

/// [`far_frame`]'s sky: the sun at elevation `el` in the west, of radius 3 degrees.
fn far_sky(el: f32) -> Sky {
    let mut s = west_sky(el);
    s.sun.angular_radius = 3f32.to_radians();
    s
}

/// Rule 3 at a key re-bake's landing: the slab-edge rig under `key_far baked` at the far (the
/// far tests' camera, look and 3-degree sun, at 64 x 64), the live queue baked to rest under
/// the sun at elevation 40, two frames on the records; then the sun drifting down 1 degree a
/// frame (under its radius: a drift, not a jump) to 36, past the disc and the step, and 22
/// frames in all, the queue stepped each: at every floor pixel that ends on a record and whose
/// value changes by over a level, the trace from the last frame before the drift is monotonic,
/// moves at most 1/16 a frame (plus the f16 slot's step), and reaches the new record's value
/// within FADE (16) frames of its first move; at least 50 pixels move.
#[test]
fn a_key_rebakes_landing_at_the_far_is_one_fade() {
    let Some((device, queue)) = device() else {
        return;
    };
    let slab = floor_world([1, 1, 1], |p| p[2] < FLOOR + 10 && p[0] < 24);
    let mut gpu = mirror(&device, &queue, &slab, None);
    let mut f = small_far_frame(&device, 40.0);
    start_key_queue(&queue, &mut f, &gpu);
    let cam = far_camera();
    bake_to_rest(&device, &queue, &mut f, &mut gpu, &slab, &cam);
    let mut shots = Vec::new();
    for _ in 0..2 {
        key_step(&device, &queue, &mut f, &mut gpu, &slab, &cam);
        shots.push(far_floor(&device, &queue, &mut f, &gpu));
    }
    let last = shots.pop().expect("two frames");
    assert!(
        last.iter().all(|p| key_slot(p.2).1 == 'r') && last.len() > 1000,
        "rig fault: {} floor pixels, some off their record before the drift",
        last.len()
    );
    let start: std::collections::HashMap<usize, f32> =
        last.iter().map(|p| (p.0, key_slot(p.2).0)).collect();
    let mut traces: std::collections::HashMap<usize, Vec<(f32, char)>> =
        std::collections::HashMap::new();
    let mut landed_at = None;
    for k in 0..22 {
        if k < 4 {
            f.set_sky(Some(far_sky(39.0 - k as f32)));
        }
        let st = key_step(&device, &queue, &mut f, &mut gpu, &slab, &cam);
        if st.keys_baked >= 1 && landed_at.is_none() {
            landed_at = Some(k);
        }
        for p in far_floor(&device, &queue, &mut f, &gpu) {
            traces.entry(p.0).or_default().push(key_slot(p.2));
        }
    }
    let landed_at = landed_at.expect("the block's key job landed");
    let (mut moved, mut long) = (0, 0);
    // The moved pixels' hit x range and their values' range before and after, for the log.
    let mut range = [f32::MAX, f32::MIN, f32::MAX, f32::MIN, f32::MAX, f32::MIN];
    let hit_x: std::collections::HashMap<usize, f32> = last.iter().map(|p| (p.0, p.1)).collect();
    for (i, t) in &traces {
        let Some(&s) = start.get(i) else { continue };
        let (end, src) = *t.last().unwrap();
        if src != 'r' || (end - s).abs() <= 1.0 / 255.0 {
            continue;
        }
        moved += 1;
        for (k, v) in [hit_x[i], s, end].into_iter().enumerate() {
            range[2 * k] = range[2 * k].min(v);
            range[2 * k + 1] = range[2 * k + 1].max(v);
        }
        let first = t
            .iter()
            .position(|v| (v.0 - s).abs() > 1e-3)
            .expect("moves");
        let reached = t
            .iter()
            .position(|v| (v.0 - end).abs() <= 1.0 / 255.0)
            .expect("reaches its end");
        assert!(
            reached - first < 16,
            "pixel {i} moves at frame {first}, reaches the record at {reached}: {t:?}"
        );
        long += usize::from(reached - first > 2);
        let up = end > s;
        let mut prev = s;
        for v in t {
            let d = v.0 - prev;
            assert!(
                if up { d >= -1e-3 } else { d <= 1e-3 } && d.abs() <= 1.0 / 16.0 + 1.0 / 256.0 + 1e-3,
                "pixel {i} from {s}: {t:?}"
            );
            prev = v.0;
        }
    }
    println!(
        "key landing at frame {landed_at}: {moved} of {} floor pixels moved, {long} over more \
         than two frames; hit x {:.1}..{:.1}, from {:.3}..{:.3} to {:.3}..{:.3}",
        last.len(),
        range[0],
        range[1],
        range[2],
        range[3],
        range[4],
        range[5]
    );
    assert!(moved > 50, "{moved} moved");
}

/// Bar: a dig under a fully lit floor re-bakes the floor bricks' key words with the sky's
/// ring: the floor of one chunk baked to rest under the sun at elevation 40, the sun turned 1
/// degree (no key job), a box carved inside the rock under the floor (x and y 28..36, z
/// 4..8): at the dig's step the floor brick over it (brick 4, 4, 1) holds its record: not
/// stale (`VIS_BAKED`), its `+z` sky value kept at 1.0 (the face lies outside the dig's box
/// dilated by one voxel), and `KEY_BAKED` clear, so its key reads as none, never as 0; once
/// the ring lands it is clear, its `+z` key level is 255 and the block's sun stamp is the
/// turned sun, under which only the ring's job baked.
#[test]
fn a_dig_rebakes_the_key_words_with_the_sky_ring() {
    let Some((device, queue)) = device() else {
        return;
    };
    let world = floor_world([1, 1, 1], |_| false);
    let mut gpu = mirror(&device, &queue, &world, None);
    let mut f = frame(&device, SIZE, ShadowWalk::Full);
    f.set_sky(Some(west_sky(40.0)));
    start_key_queue(&queue, &mut f, &gpu);
    let cam = camera([32.0, 32.0]);
    bake_to_rest(&device, &queue, &mut f, &mut gpu, &world, &cam);
    let c = ChunkCoord([0, 0, 0]);
    let floor_record = |f: &Frame| {
        let w = vxe_render::internals::BRICK_WORDS;
        let block = f.store().block_words(c).expect("the block");
        let d = block[brick_slot([4, 4, 1])];
        assert_ne!(d, 0, "the floor brick has a record");
        block[(d >> 11) as usize - w..(d >> 11) as usize].to_vec()
    };
    assert_eq!(brick_key(&f, [4, 4, 1], UP), Some(255));
    f.set_sky(Some(west_sky(41.0)));
    let st = key_step(&device, &queue, &mut f, &mut gpu, &world, &cam);
    assert_eq!(st.keys, 0, "a 1-degree turn");
    let lo = [28, 28, 4].map(|v: i64| v * FIXED_ONE);
    let hi = [36, 36, 8].map(|v: i64| v * FIXED_ONE);
    let r = world
        .apply(&EditOp::box_(Fixed3(lo), Fixed3(hi), Mode::Carve))
        .expect("the dig is resident");
    let (a, b) = r.aabb.expect("the dig changed voxels");
    let mut enc = device.create_command_encoder(&Default::default());
    gpu.sync(&world, &queue, &mut enc).expect("the mirror fits");
    queue.submit([enc.finish()]);
    f.invalidate(a.0, b.0);
    key_step(&device, &queue, &mut f, &mut gpu, &world, &cam);
    let rec = floor_record(&f);
    assert!(
        !vxe_render::internals::record_sky_stale(&rec),
        "held, not stale, at the dig"
    );
    assert_eq!(
        vxe_render::internals::record_visibility(&rec, usize::from(UP)),
        Some(1.0),
        "the held record keeps its +z sky value (VIS_BAKED)"
    );
    assert_eq!(
        vxe_render::internals::record_key_visibility(&rec, usize::from(UP)),
        None,
        "KEY_BAKED clear at the dig"
    );
    let mut landed = false;
    for _ in 0..200 {
        let st = key_step(&device, &queue, &mut f, &mut gpu, &world, &cam);
        if st.digs == 0 && st.jobs == 0 {
            landed = true;
            break;
        }
    }
    assert!(landed, "the ring lands within 200 steps");
    let rec = floor_record(&f);
    assert!(!vxe_render::internals::record_sky_stale(&rec), "clear at the landing");
    assert_eq!(brick_key(&f, [4, 4, 1], UP), Some(255));
    assert!(near_dir(stamp_of(&f, c).expect("a stamp"), dir(41.0, 270.0)));
    assert_eq!(
        f.bake_queue().expect("the queue").stats().keys_baked,
        0,
        "no key-only job ran"
    );
}
