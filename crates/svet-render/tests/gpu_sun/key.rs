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
    let settings = svet_render::BakeSettings {
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
    let w = svet_render::internals::BRICK_WORDS;
    let block = f.store().block_words(ChunkCoord(b.map(|v| v.div_euclid(8))))?;
    let dir = block[brick_slot(b.map(|v| v.rem_euclid(8)))];
    if dir == 0 {
        return None;
    }
    let rec = &block[(dir >> 11) as usize - w..(dir >> 11) as usize];
    svet_render::internals::record_key_visibility(rec, usize::from(face))
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
fn far_frame(device: &wgpu::Device, key_far: svet_render::KeyFar, el: f32) -> Frame {
    let mut f = frame(device, [128, 128], ShadowWalk::Full);
    f.set_look(svet_render::LookParams {
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
    let h = svet_render::debug::read_rgba32f(device, queue, f.light_hit_positions());
    let pair = svet_render::debug::read_rgba16f(device, queue, f.visibility_pair());
    let vis = svet_render::debug::read_rgba8(device, queue, f.visibility_samples());
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
#[ignore = "soak"]
fn the_far_horizon_reads_an_open_floor_lit_and_a_roofed_one_dark() {
    let Some((device, queue)) = device() else {
        return;
    };
    let open = floor_world([1, 1, 1], |_| false);
    let gpu = mirror(&device, &queue, &open, None);
    let mut f = far_frame(&device, svet_render::KeyFar::Baked, 40.0);
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
    let mut f = far_frame(&device, svet_render::KeyFar::Baked, 40.0);
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
    let mut f = far_frame(&device, svet_render::KeyFar::Baked, 40.0);
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
    let mut f = far_frame(&device, svet_render::KeyFar::Column, 40.0);
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

/// The default's near key shadow (ADR-0082): on the slab-edge rig at the near (the far normal's
/// band off, its weight 0) under `Lighting::default()`'s light history, key far and key far mix
/// (no history, `column`, `step`) with the disc's jitter on: the key light's term shown at every
/// floor pixel (the direct target's rgb: the rig has no lamp, no list light and no sky ray) is
/// the same f16 on frames 1, 2 and 3, and so is the key slot on frames 2 and 3; some pixel's
/// slot value lies strictly between 0 and 1 (the horizon's ramp is in view); and every pixel's
/// slot value on each frame is the same frame's on a fourth frame under the far estimate alone
/// (the far normal's weight 1 at every pixel, as [`far_frame`]'s, no key ray cast). The slot
/// values are compared within the slot's f16 step at 2 + v (1/512): a frame whose history is
/// invalid (the first) leaves the value untagged in `[0, 1]`, a frame with no key ray tagged by
/// its source at `2 + v`. The near key shadow is the column map's horizon, the same every frame.
#[test]
fn the_defaults_near_key_shadow_is_the_column_horizon() {
    let Some((device, queue)) = device() else {
        return;
    };
    let slab = floor_world([1, 1, 1], |p| p[2] < FLOOR + 10 && p[0] < 24);
    let gpu = mirror(&device, &queue, &slab, None);
    let mut near = far_frame(&device, svet_render::KeyFar::Column, 40.0);
    let d = Lighting::default();
    assert!(!d.light_history && !d.light_history_far);
    near.set_lighting(Lighting {
        light_history: d.light_history,
        key_far: d.key_far,
        key_far_mix: d.key_far_mix,
        penumbra: d.penumbra,
        ..near.lighting()
    });
    near.set_look(svet_render::LookParams {
        far_normal: [0.0, 0.0],
        ..near.look()
    });
    let mut shown = Vec::new();
    let shots: Vec<Vec<(usize, f32)>> = (0..3)
        .map(|_| {
            let px = far_floor(&device, &queue, &mut near, &gpu);
            let d = svet_render::debug::read_rgba16f(&device, &queue, near.direct_accumulated());
            shown.push(px.iter().map(|p| (p.0, d[p.0][0..3].to_vec())).collect::<Vec<_>>());
            px.iter().map(|p| (p.0, key_slot(p.2).0)).collect()
        })
        .collect();
    assert!(shots[0].len() > 10_000, "{} floor pixels", shots[0].len());
    assert!(shots[0].iter().any(|p| p.1 > 0.0 && p.1 < 1.0), "no ramp in view");
    assert!(
        shown[0].iter().any(|p| p.1.iter().any(|c| *c > 0.0)),
        "no key light shown"
    );
    assert_eq!(shown[0], shown[1]);
    assert_eq!(shown[1], shown[2]);
    assert_eq!(shots[1], shots[2]);
    // The far estimate alone: the same frame with the far normal's weight 1 at every pixel (as
    // in [`far_frame`]), where no key ray is cast.
    near.set_look(svet_render::LookParams {
        far_normal: [1000.0, 2000.0],
        ..near.look()
    });
    let far: std::collections::HashMap<usize, f32> = far_floor(&device, &queue, &mut near, &gpu)
        .iter()
        .map(|p| {
            assert_eq!(p.3, 64, "a key ray at the far, x {}", p.1);
            (p.0, key_slot(p.2).0)
        })
        .collect();
    for (k, shot) in shots.iter().enumerate() {
        for &(i, v) in shot {
            let f = far[&i];
            assert!(
                (v - f).abs() <= 1.0 / 512.0 + 1e-6,
                "frame {}, pixel {i}: near {v} against the far estimate's {f}",
                k + 1
            );
        }
    }
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
    let mut f = far_frame(&device, svet_render::KeyFar::Baked, 40.0);
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
        &svet_render::BakeSettings {
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

// Plan 0095 Phase 5: the near key light under `penumbra approach` is the frame's approach
// visibility times the ray's transmittance, taken as the value (no running mean), and the
// approach probes skip voxels of a SHADOW_ABSORB material. The 128 x 128 rig of the far
// tests' camera, the far normal's band off (its weight 0: the near).

/// A rock floor (id 1) under z 16 over one chunk plus `extra` voxels of material `m(p)`
/// (0 air), and a material table of rock and leaf (id 2).
fn styled_world(m: impl Fn([i32; 3]) -> MaterialId + Sync) -> World {
    let f = move |p: [i32; 3]| if p[2] < FLOOR { ROCK } else { m(p) };
    let table: MaterialTable = [Material::named("rock"), Material::named("leaf")]
        .into_iter()
        .collect();
    World::generate(ChunkExtent::new([0, 0, 0], [1, 1, 1]), &FnSource(f), &table)
}

/// The near rig's frame: [`frame`] at 128 x 128 under `penumbra approach`, the leaf (id 2)
/// absorbing `sigma` a voxel, a sun at elevation `el` and azimuth `az` of the real radius.
fn near_frame(device: &wgpu::Device, queue: &wgpu::Queue, el: f32, az: f32, sigma: f32) -> Frame {
    let mut f = frame(device, [128, 128], ShadowWalk::Full);
    f.set_lighting(Lighting {
        penumbra: svet_render::Penumbra::Approach,
        ..f.lighting()
    });
    f.set_material_styles(queue, &[0, 0, svet_render::shadow_absorb(sigma)]);
    f.set_sky(Some(sky(dir(el, az), dir(-40.0, 0.0), [[1.0; 3], [0.0; 3]])));
    f
}

/// One frame of `f` from `cam`: per lighting pixel whose hit is a floor face, its index, hit
/// x and y, and the key slot (a ray's value under `approach`).
fn near_floor(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    f: &mut Frame,
    gpu: &GpuWorld,
    cam: &Camera,
) -> Vec<(usize, f32, f32, f32)> {
    f.step(device, queue, gpu, cam, &LightParams::headlamp());
    let h = svet_render::debug::read_rgba32f(device, queue, f.light_hit_positions());
    let pair = svet_render::debug::read_rgba16f(device, queue, f.visibility_pair());
    (0..h.len())
        .filter(|&i| h[i][3] < 1e30 && (h[i][2] - FLOOR as f32).abs() < 0.01)
        .map(|i| (i, h[i][0], h[i][1], pair[i][1]))
        .collect()
}

/// Straight down from 8 voxels over the floor at `at`, screen right +x.
fn down_at(at: [f32; 2]) -> Camera {
    Camera {
        position: Vec3::new(at[0], at[1], FLOOR as f32 + 8.0),
        ..far_camera()
    }
}

/// Determinism: under `approach` the near key slot at every floor pixel of the slab-edge rig
/// (rock to z 26 over x < 24, the sun at elevation 40 in the west) is the same f16 on frames
/// 1, 2 and 3: the value of the frame's ray, which a reset does not change.
#[test]
fn the_approach_penumbra_is_the_same_every_frame() {
    let Some((device, queue)) = device() else {
        return;
    };
    let slab = styled_world(|p| if p[2] < FLOOR + 10 && p[0] < 24 { ROCK } else { AIR });
    let gpu = mirror(&device, &queue, &slab, None);
    let mut f = near_frame(&device, &queue, 40.0, 270.0, 0.15);
    let cam = far_camera();
    let shots: Vec<Vec<f32>> = (0..3)
        .map(|_| {
            near_floor(&device, &queue, &mut f, &gpu, &cam)
                .iter()
                .map(|p| p.3)
                .collect()
        })
        .collect();
    assert!(shots[0].contains(&0.0) && shots[0].contains(&1.0));
    assert_eq!(shots[0], shots[1]);
    assert_eq!(shots[1], shots[2]);
}

/// Bar: under a zenith sun, a floor under a one-voxel layer of leaf (z 28, the whole chunk)
/// absorbing 0.15 a voxel reads, under `approach` with the probes' skip, within 8 levels of
/// `exp(-0.15) 255 = 219.5` at every floor pixel (the transmittance alone). 15.0 s on lavapipe
/// at load 1.1 (over the soak line).
#[test]
#[ignore = "soak"]
fn a_floor_under_a_leaf_layer_reads_its_transmittance() {
    let Some((device, queue)) = device() else {
        return;
    };
    let leaf = styled_world(|p| if p[2] == FLOOR + 12 { 2 } else { AIR });
    let gpu = mirror(&device, &queue, &leaf, None);
    let mut f = near_frame(&device, &queue, 90.0, 0.0, 0.15);
    let px = near_floor(&device, &queue, &mut f, &gpu, &far_camera());
    assert!(px.len() > 10_000, "{} floor pixels", px.len());
    let want = (-0.15f64).exp() * 255.0;
    for p in &px {
        let got = f64::from(p.3) * 255.0;
        assert!((got - want).abs() <= 8.0, "at ({}, {}): {got} against {want}", p.1, p.2);
    }
}

/// Bar: a rock post (x 28..32, y 30..34, to z 26) under a sun at elevation 45 in the west:
/// under `approach` the floor in its shadow (x 33..40, y 30.5..33.5) reads 0, and the floor
/// two voxels past the shadow's side (x 33..40, y 36.1..40), past a ramp of `t tan r` (under
/// 0.07 voxels for the real radius at the post's 14 voxels), reads 255.
#[test]
fn a_post_casts_a_dark_shadow_and_a_lit_side_under_the_approach() {
    let Some((device, queue)) = device() else {
        return;
    };
    let post = styled_world(|p| {
        if (28..32).contains(&p[0]) && (30..34).contains(&p[1]) && p[2] < FLOOR + 10 {
            ROCK
        } else {
            AIR
        }
    });
    let gpu = mirror(&device, &queue, &post, None);
    let mut f = near_frame(&device, &queue, 45.0, 270.0, 0.15);
    let px = near_floor(&device, &queue, &mut f, &gpu, &down_at([36.0, 34.0]));
    let (mut dark, mut lit) = (0, 0);
    for p in &px {
        if (33.0..40.0).contains(&p.1) && (30.5..33.5).contains(&p.2) {
            assert_eq!(p.3, 0.0, "in the shadow at ({}, {})", p.1, p.2);
            dark += 1;
        }
        if (33.0..40.0).contains(&p.1) && (36.1..40.0).contains(&p.2) {
            assert_eq!(p.3, 1.0, "past the side at ({}, {})", p.1, p.2);
            lit += 1;
        }
    }
    assert!(dark > 300 && lit > 300, "{dark} dark, {lit} lit pixels");
}

/// One 320 x 180 frame's light pass of the mirrored world `gpu` under a noon sun (no sky ray,
/// the filter off, the disc penumbra, `key_far ray` so every pixel's key is its ray with no far
/// estimate to seed or replace it) with the key cap `cap` (0: none), from 4 voxels over the floor at x 48 looking along -x, 10 degrees
/// up: per lighting pixel its hit position and its visibility pair (`y` the key light's ray
/// mean, one frame: the ray's value).
fn lever_shot(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    gpu: &GpuWorld,
    cap: f32,
) -> (Vec<[f32; 4]>, Vec<[f32; 4]>) {
    let mut f = frame(device, [320, 180], ShadowWalk::Full);
    f.set_lighting(Lighting {
        key_far: svet_render::KeyFar::Ray,
        key_cap: cap,
        ..f.lighting()
    });
    f.set_sky(Some(sky(dir(90.0, 0.0), dir(-40.0, 0.0), [[1.0; 3], [0.0; 3]])));
    let cam = Camera {
        position: Vec3::new(48.0, 32.0, FLOOR as f32 + 4.0),
        yaw: std::f32::consts::PI,
        pitch: 10f32.to_radians(),
        vfov: 60f32.to_radians(),
    };
    f.step(device, queue, gpu, &cam, &LightParams::headlamp());
    let h = svet_render::debug::read_rgba32f(device, queue, f.light_hit_positions());
    let pair = svet_render::debug::read_rgba16f(device, queue, f.visibility_pair());
    (h, pair)
}

/// Plan 0096 Phase 6 bar 1: the slow adapter's key cap counts a key ray clear for `cap` voxels
/// lit, so it changes the image only where a ray is longer than the cap. An open floor at noon
/// (every ray clear) renders bit-identical with the cap at 4 voxels and with none. Under a slab
/// at z 26..28 over x < 24 (an overhang 10 voxels over the floor, no post), the cap at 4 voxels,
/// under that clearance, lifts the floor under the slab from dark to lit; every pixel that
/// differs is a floor hit (z 16) at x < 24.1 (the slab's edge plus the sun disc's lean over the
/// 10 voxels, about 0.05), and the slab's own faces and the open floor past it are unchanged.
#[test]
#[ignore = "soak"]
fn the_key_cap_leaves_the_lit_image_unchanged_where_no_ray_is_longer() {
    let Some((device, queue)) = device() else {
        return;
    };
    const CAP: f32 = 4.0;
    let under = FLOOR + 10;
    let bits = |v: &[f32; 4]| v.map(f32::to_bits);
    let open = mirror(&device, &queue, &styled_world(|_| AIR), None);
    let base = lever_shot(&device, &queue, &open, 0.0).1;
    let capped = lever_shot(&device, &queue, &open, CAP).1;
    let moved = (0..base.len()).filter(|&i| bits(&base[i]) != bits(&capped[i]));
    assert_eq!(moved.count(), 0, "the open floor under the cap");
    let slab_world = styled_world(move |p| {
        let deck = (under..under + 2).contains(&p[2]) && p[0] < 24;
        if deck { ROCK } else { AIR }
    });
    let slab = mirror(&device, &queue, &slab_world, None);
    let (h, plain) = lever_shot(&device, &queue, &slab, 0.0);
    let (hc, cap) = lever_shot(&device, &queue, &slab, CAP);
    assert!(h.iter().zip(&hc).all(|(a, b)| bits(a) == bits(b)), "the cap moves a hit");
    let floor = |i: usize| h[i][3] < 1e30 && (h[i][2] - FLOOR as f32).abs() < 0.01;
    let shaded: Vec<usize> = (0..h.len())
        .filter(|&i| floor(i) && h[i][0] < 23.0)
        .collect();
    let differ: Vec<usize> = (0..h.len())
        .filter(|&i| bits(&plain[i]) != bits(&cap[i]))
        .collect();
    println!(
        "{} pixels differ, {} on the floor well under the slab",
        differ.len(),
        shaded.len()
    );
    assert!(shaded.len() > 300, "{} floor pixels under the slab", shaded.len());
    for &i in &shaded {
        let pair = (plain[i][1], cap[i][1]);
        assert_eq!(pair, (0.0, 1.0), "pixel {i} under the slab, hit {:?}", h[i]);
    }
    for i in differ {
        let under_slab = floor(i) && h[i][0] < 24.1;
        assert!(under_slab, "pixel {i} differs off the floor under the slab: hit {:?}", h[i]);
    }
}
