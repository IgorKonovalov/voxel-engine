// The key light's visibility baked per brick direction over its disc (ADR-0079, Plan 0095
// Phase 1): a floor of rock under a sun of the real radius, each rig's bricks baked alone (the
// bake's face filter) at 256 samples a brick direction, read from the store's copy of the block.

/// Bakes `world`, mirrored whole, under a sky whose sun is at `sun` (the moon below the
/// horizon), only the faces whose brick (voxel / 8) and face are in `keep`, at 256 samples a
/// brick direction: the frame, its store holding the blocks.
fn key_bake(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    world: &World,
    sun: [f32; 3],
    keep: Vec<([i32; 3], u8)>,
) -> Frame {
    let mut gpu = mirror(device, queue, world, None);
    let mut f = frame(device, SIZE, ShadowWalk::Full);
    f.set_sky(Some(sky(sun, dir(-40.0, 0.0), [[1.0; 3], [0.0; 3]])));
    let settings = vxe_render::BakeSettings {
        brick_samples: 256,
        round: 64,
        filter: Some(std::sync::Arc::new(move |v: [i32; 3], face: u8| {
            keep.contains(&(v.map(|x| x.div_euclid(8)), face))
        })),
        ..Default::default()
    };
    f.bake_until(
        device,
        queue,
        &mut gpu,
        world,
        world.extent(),
        &settings,
        &mut |_| {},
    )
    .expect("the floor fits the pool");
    f
}

/// Brick `b`'s (world brick coordinates) key visibility for `face` as the store holds it, a
/// `u8` level; `None` where its record lacks it.
fn brick_key(f: &Frame, b: [i32; 3], face: u8) -> Option<u32> {
    let w = vxe_render::internals::BRICK_WORDS;
    let block = f.store().block_words(ChunkCoord(b.map(|v| v.div_euclid(8))))?;
    let dir = block[brick_slot(b.map(|v| v.rem_euclid(8)))];
    if dir == 0 {
        return None;
    }
    let rec = &block[(dir >> 11) as usize - w..(dir >> 11) as usize];
    vxe_render::internals::record_key_visibility(rec, usize::from(face))
        .map(|v| (v * 255.0).round() as u32)
}

// Plan 0095 Phase 1 bar 2: the key light's baked visibility, one bake a test on a floor of
// 2 x 2 x 1 chunks (rock under z 16, the floor's `+z` faces in brick row z 1).

const UP: u8 = 5;

/// The open floor under a sun at elevation 40 bakes `+z` 255.
#[test]
fn the_open_floor_bakes_its_key_visibility_lit() {
    let Some((device, queue)) = device() else {
        return;
    };
    let open = floor_world([2, 2, 1], |_| false);
    let f = key_bake(&device, &queue, &open, dir(40.0, 200.0), vec![([8, 8, 1], UP)]);
    assert_eq!(brick_key(&f, [8, 8, 1], UP), Some(255), "the open floor");
}

/// A floor brick under a rock slab 10 voxels up (z 26..28, x and y 16..112) bakes `+z` 0
/// under a sun at elevation 40 (its rays climb 10 voxels in 12 across, inside the slab).
#[test]
fn a_floor_under_a_slab_bakes_its_key_visibility_dark() {
    let Some((device, queue)) = device() else {
        return;
    };
    let slab = floor_world([2, 2, 1], |p| {
        (26..28).contains(&p[2]) && (16..112).contains(&p[0]) && (16..112).contains(&p[1])
    });
    let f = key_bake(&device, &queue, &slab, dir(40.0, 200.0), vec![([8, 8, 1], UP)]);
    assert_eq!(brick_key(&f, [8, 8, 1], UP), Some(0), "under the slab");
}

/// Under a zenith sun, the floor brick at x 64..72 under a slab whose edge is at x 68 (the
/// slab over x < 68, 10 voxels up) bakes `+z` within 16 of 128: half its faces' rays end on
/// the slab, and the binomial standard deviation at 64 rays is 1/16.
#[test]
fn a_floor_half_under_a_slab_edge_bakes_half() {
    let Some((device, queue)) = device() else {
        return;
    };
    let edge = floor_world([2, 2, 1], |p| {
        (26..28).contains(&p[2]) && p[0] < 68 && (16..112).contains(&p[1])
    });
    let f = key_bake(&device, &queue, &edge, [0.0, 0.0, 1.0], vec![([8, 8, 1], UP)]);
    let half = brick_key(&f, [8, 8, 1], UP).expect("baked");
    println!("half under the edge: {half}");
    assert!(half.abs_diff(128) <= 16, "half under the edge: {half}");
}

/// A wall's `+x` faces (x 56..64 up to z 40; the faces at x 63, brick x 7) under a sun in the
/// west (azimuth 270, elevation 40) bake 0: they look away from it.
#[test]
fn a_wall_facing_away_from_the_sun_bakes_dark() {
    let Some((device, queue)) = device() else {
        return;
    };
    let wall = floor_world([2, 2, 1], |p| {
        (56..64).contains(&p[0]) && (40..90).contains(&p[1]) && p[2] < 40
    });
    let f = key_bake(&device, &queue, &wall, dir(40.0, 270.0), vec![([7, 8, 3], 1)]);
    assert_eq!(brick_key(&f, [7, 8, 3], 1), Some(0), "a wall facing away");
}
