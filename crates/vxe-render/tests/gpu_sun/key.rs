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

// Plan 0095 Phase 4: the far estimate with no record, the column map's horizon toward the sun
// (ADR-0079). The floor of rock under z 16 over one chunk, a key light of a 3-degree radius in
// the west (azimuth 270: the sun's direction points along -x), the camera 8 voxels over the
// floor looking straight down at 4 lighting pixels a voxel (x 28..60 at the floor), the far
// normal's band set past every pixel so its weight is 1 everywhere, no queue: the light pass
// casts no key ray and the temporal pass reads the horizon.

/// The far rig's frame: [`frame`] at 128 x 128 under `key_far`, the far normal's weight 1 at
/// every pixel, a sun at elevation `el` in the west of radius 3 degrees.
fn far_frame(device: &wgpu::Device, key_far: vxe_render::KeyFar, el: f32) -> Frame {
    let mut f = frame(device, [128, 128], ShadowWalk::Full);
    f.set_look(vxe_render::LookParams {
        far_normal: [1000.0, 2000.0],
        ..f.look()
    });
    f.set_lighting(Lighting {
        key_far,
        ..f.lighting()
    });
    let mut s = sky(dir(el, 270.0), dir(-40.0, 0.0), [[1.0; 3], [0.0; 3]]);
    s.sun.angular_radius = 3f32.to_radians();
    f.set_sky(Some(s));
    f
}

/// Straight down from 8 voxels over the floor at x 44, y 32, screen right +x.
fn far_camera() -> Camera {
    Camera {
        position: Vec3::new(44.0, 32.0, FLOOR as f32 + 8.0),
        yaw: FRAC_PI_2,
        pitch: -FRAC_PI_2 + 1e-4,
        vfov: 2.0 * 2f32.atan(),
    }
}

/// A key slot (the visibility pair's g at a pixel with no key ray) as its value and source:
/// `'r'` a record (`-1 - v`), `'h'` the horizon (`2 + v`), `'f'` a landing's fade (`4 + v`),
/// `'m'` a ray's mean (`v`); pair.wgsl's `key_slot_decode`.
fn key_slot(y: f32) -> (f32, char) {
    if (-2.01..=-0.99).contains(&y) {
        (-1.0 - y, 'r')
    } else if (1.99..=3.01).contains(&y) {
        (y - 2.0, 'h')
    } else if (3.99..=5.01).contains(&y) {
        (y - 4.0, 'f')
    } else {
        (y, 'm')
    }
}

/// One frame of `f`: per lighting pixel whose hit is a floor face (z within 0.01 of FLOOR),
/// its hit x, the raw key slot, and the visibility sample's b as a byte (64 for no ray).
fn far_floor(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    f: &mut Frame,
    gpu: &GpuWorld,
) -> Vec<(usize, f32, f32, u8)> {
    f.step(device, queue, gpu, &far_camera(), &LightParams::headlamp());
    let h = vxe_render::debug::read_rgba32f(device, queue, f.light_hit_positions());
    let pair = vxe_render::debug::read_rgba16f(device, queue, f.visibility_pair());
    let vis = vxe_render::debug::read_rgba8(device, queue, f.visibility_samples());
    (0..h.len())
        .filter(|&i| h[i][3] < 1e30 && (h[i][2] - FLOOR as f32).abs() < 0.01)
        .map(|i| (i, h[i][0], pair[i][1], vis[4 * i + 2]))
        .collect()
}

/// The middle row's floor values, linear between the two pixels about `x`, as a level 0..255.
fn far_level_at(px: &[(usize, f32, f32, u8)], row: usize, x: f32) -> f64 {
    let mut r: Vec<(f32, f32)> = px
        .iter()
        .filter(|p| p.0 / 128 == row)
        .map(|p| (p.1, key_slot(p.2).0))
        .collect();
    r.sort_by(|a, b| a.0.total_cmp(&b.0));
    let b = r.iter().position(|p| p.0 >= x).expect("the row reaches x");
    let (a, c) = (r[b - 1], r[b]);
    let t = (x - a.0) / (c.0 - a.0);
    f64::from(a.1 + t * (c.1 - a.1)) * 255.0
}

/// Bar: with no record, an open floor under a sun at elevation 40 reads the horizon's key
/// visibility 255 at every floor pixel, and a floor under a rock slab 10 voxels up (z 26..28,
/// x and y 8..64) reads 0 at every floor pixel more than 12 voxels inside the slab's edge, each
/// with no key ray cast (the sample's b 64, KEY_FROM_STORE).
#[test]
fn the_far_horizon_reads_an_open_floor_lit_and_a_roofed_one_dark() {
    let Some((device, queue)) = device() else {
        return;
    };
    let open = floor_world([1, 1, 1], |_| false);
    let gpu = mirror(&device, &queue, &open, None);
    let mut f = far_frame(&device, vxe_render::KeyFar::Baked, 40.0);
    let px = far_floor(&device, &queue, &mut f, &gpu);
    assert!(px.len() > 10_000, "{} floor pixels", px.len());
    for p in &px {
        assert_eq!(p.3, 64, "no key ray at x {}", p.1);
        assert_eq!(key_slot(p.2), (1.0, 'h'), "the open floor at x {}", p.1);
    }
    let roof = floor_world([1, 1, 1], |p| {
        (26..28).contains(&p[2]) && p[0] >= 8 && (8..64).contains(&p[1])
    });
    let gpu = mirror(&device, &queue, &roof, None);
    let mut f = far_frame(&device, vxe_render::KeyFar::Baked, 40.0);
    let px = far_floor(&device, &queue, &mut f, &gpu);
    let under: Vec<_> = px.iter().filter(|p| p.1 >= 30.0 && p.1 <= 52.0).collect();
    assert!(under.len() > 5000, "{} floor pixels under the slab", under.len());
    for p in under {
        assert_eq!(key_slot(p.2), (0.0, 'h'), "under the slab at x {}", p.1);
    }
}

/// Bar: a floor beside a slab 10 voxels high (rock to z 26 over x < 24) with the sun at
/// elevation 40 in the west reads, along the middle row, within 16 levels of 128 at 8.9 voxels
/// from the slab's edge on the shadow side, 255 at 13 and 0 at 6: the hit is lifted 2.5 voxels
/// (`SEED_LIFT`), so the slab's top stands 7.5 over it and the ramp's middle is
/// `7.5 / tan 40 = 8.94` voxels out, over `7.5 / tan 43` to `7.5 / tan 37` (8.0 to 9.95).
#[test]
fn the_far_horizon_ramps_at_a_slab_edge() {
    let Some((device, queue)) = device() else {
        return;
    };
    let slab = floor_world([1, 1, 1], |p| p[2] < FLOOR + 10 && p[0] < 24);
    let gpu = mirror(&device, &queue, &slab, None);
    let mut f = far_frame(&device, vxe_render::KeyFar::Baked, 40.0);
    let px = far_floor(&device, &queue, &mut f, &gpu);
    let at = |x: f32| far_level_at(&px, 64, 24.0 + x);
    let (mid, lit, dark) = (at(8.9), at(13.0), at(6.0));
    println!("the horizon at 6 / 8.9 / 13 voxels from the edge: {dark:.1} / {mid:.1} / {lit:.1}");
    assert!((mid - 128.0).abs() <= 16.0, "at 8.9: {mid}");
    assert_eq!(lit.round(), 255.0, "at 13: {lit}");
    assert_eq!(dark.round(), 0.0, "at 6: {dark}");
}

/// Determinism: under `key_far column` the slab-edge rig's far key slot at every floor pixel is
/// the same f16 on frames 1, 2 and 3 (no history, no fade in flight).
#[test]
fn the_far_horizon_is_the_same_every_frame() {
    let Some((device, queue)) = device() else {
        return;
    };
    let slab = floor_world([1, 1, 1], |p| p[2] < FLOOR + 10 && p[0] < 24);
    let gpu = mirror(&device, &queue, &slab, None);
    let mut f = far_frame(&device, vxe_render::KeyFar::Column, 40.0);
    let shots: Vec<Vec<f32>> = (0..3)
        .map(|_| {
            far_floor(&device, &queue, &mut f, &gpu)
                .iter()
                .map(|p| p.2)
                .collect()
        })
        .collect();
    assert!(shots[0].iter().any(|y| key_slot(*y).0 > 0.0 && key_slot(*y).0 < 1.0));
    assert_eq!(shots[0], shots[1]);
    assert_eq!(shots[1], shots[2]);
}

/// Rule 3 at a record's landing: on the slab-edge rig under `key_far baked`, 4 frames on the
/// horizon, then the floor's `+z` faces of bricks x 3..6 (x 24..48) baked (their records land
/// together, as one stepped job's would), then 20 frames: at every floor pixel that ends on a
/// record (the slot's source `'r'`) and whose value changes by over a level, the trace from
/// the last horizon frame is monotonic, moves at most 1/16 a frame (plus the f16 slot's step,
/// 1/256, at 4 + v), and reaches the record's value within FADE (16) frames; at least one
/// pixel takes more than two frames.
#[test]
fn a_records_landing_at_the_far_is_one_fade() {
    let Some((device, queue)) = device() else {
        return;
    };
    let slab = floor_world([1, 1, 1], |p| p[2] < FLOOR + 10 && p[0] < 24);
    let mut gpu = mirror(&device, &queue, &slab, None);
    let mut f = far_frame(&device, vxe_render::KeyFar::Baked, 40.0);
    let mut last = Vec::new();
    for _ in 0..4 {
        last = far_floor(&device, &queue, &mut f, &gpu);
    }
    assert!(last.iter().all(|p| key_slot(p.2).1 == 'h'));
    f.bake_until(
        &device,
        &queue,
        &mut gpu,
        &slab,
        slab.extent(),
        &vxe_render::BakeSettings {
            brick_samples: 64,
            round: 64,
            filter: Some(std::sync::Arc::new(|v: [i32; 3], face: u8| {
                face == UP && v[2].div_euclid(8) == 1 && (3..6).contains(&v[0].div_euclid(8))
            })),
            ..Default::default()
        },
        &mut |_| {},
    )
    .expect("the floor fits the pool");
    let start: std::collections::HashMap<usize, f32> =
        last.iter().map(|p| (p.0, key_slot(p.2).0)).collect();
    let mut traces: std::collections::HashMap<usize, Vec<(f32, char)>> =
        std::collections::HashMap::new();
    for _ in 0..20 {
        for p in far_floor(&device, &queue, &mut f, &gpu) {
            traces.entry(p.0).or_default().push(key_slot(p.2));
        }
    }
    let (mut moved, mut long) = (0, 0);
    for (i, t) in &traces {
        let Some(&s) = start.get(i) else { continue };
        let (end, src) = *t.last().unwrap();
        if src != 'r' || (end - s).abs() <= 1.0 / 255.0 {
            continue;
        }
        moved += 1;
        let mut prev = s;
        let up = end > s;
        let reached = t
            .iter()
            .position(|v| (v.0 - end).abs() <= 1.0 / 255.0)
            .expect("reaches its end");
        assert!(reached < 16, "pixel {i} reaches the record at frame {reached}: {t:?}");
        long += usize::from(reached > 2);
        for v in t {
            let d = v.0 - prev;
            assert!(
                if up { d >= -1e-3 } else { d <= 1e-3 } && d.abs() <= 1.0 / 16.0 + 1.0 / 256.0 + 1e-3,
                "pixel {i} from {s}: {t:?}"
            );
            prev = v.0;
        }
    }
    println!("landing: {moved} floor pixels moved, {long} over more than two frames");
    assert!(moved > 100, "{moved} moved");
    assert!(long > 0, "{moved} moved");
}
