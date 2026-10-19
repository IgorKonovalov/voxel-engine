//! The walk's cost on fixed rays: steps and storage reads read back from the steps view of a
//! one-pixel target, on closure worlds with axis-aligned rays from voxel centres so that
//! every `t` of the walk is exact.

use vxe_render::{
    Camera, GpuWorld, LightParams, LookParams, Marcher, ViewMode, glam::Vec3, readback, wgpu,
};
use vxe_world::*;

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        force_fallback_adapter: true,
        ..Default::default()
    }))
    .or_else(|_| pollster::block_on(instance.request_adapter(&Default::default())));
    let Ok(adapter) = adapter else {
        println!("skipped: no adapter");
        return None;
    };
    let limits = vxe_render::limits_for(&adapter, 16 << 20);
    let desc = wgpu::DeviceDescriptor {
        required_limits: limits,
        ..Default::default()
    };
    Some(pollster::block_on(adapter.request_device(&desc)).expect("device"))
}

/// A world whose voxel at world coordinate `v` is `f(v)`.
struct Voxels<F>(F);

impl<F: Fn([i32; 3]) -> MaterialId + Sync> WorldSource for Voxels<F> {
    fn chunk(&self, coord: ChunkCoord, out: &mut Chunk) {
        let base = coord.min_voxel().0;
        for slot in 0..CHUNK_BRICKS {
            let b = brick_of_slot(slot);
            out.set_brick(
                slot,
                Brick::from_fn(|l| (self.0)([0, 1, 2].map(|k| base[k] + b[k] * BRICK_EDGE + l[k]))),
            );
        }
    }
}

const ROCK: MaterialId = 1;

fn table() -> MaterialTable {
    let mut t = MaterialTable::new();
    t.push(Material::named("rock"));
    t
}

fn world(chunks: [i32; 3], f: impl Fn([i32; 3]) -> MaterialId + Sync) -> World {
    World::generate(ChunkExtent::new([0, 0, 0], chunks), &Voxels(f), &table())
}

/// What the steps view reads for one ray: loop iterations, 32-bit storage loads from the
/// mirror (chunk table, record bits and entries, masks, pool, palette) and the pool loads
/// among them.
#[derive(Debug, PartialEq, Eq)]
struct Cost {
    steps: u8,
    loads: u8,
    pool: u8,
}

/// The steps view's pixel for the ray from `origin` along +x, the one pixel of a 1 × 1
/// target (its centre is the camera's axis), lit by a lamp at the eye.
fn cost_x(device: &wgpu::Device, queue: &wgpu::Queue, world: &World, origin: [f32; 3]) -> Cost {
    cost(device, queue, world, origin, LightParams::at_eye())
}

fn cost(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    world: &World,
    origin: [f32; 3],
    light: LightParams,
) -> Cost {
    let px = pixel(device, queue, world, origin, 0.0, ViewMode::Steps, light);
    Cost {
        steps: px[0],
        loads: px[1],
        pool: px[2],
    }
}

/// The one pixel of a 1 × 1 target for the ray from `origin` at `yaw` (pitch 0), in `view`,
/// lit by `light`. The pixel's centre is the camera's axis, so the ray is the camera's
/// forward vector.
fn pixel(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    world: &World,
    origin: [f32; 3],
    yaw: f32,
    view: ViewMode,
    light: LightParams,
) -> [u8; 4] {
    let mut gpu = GpuWorld::new(device, world.extent());
    gpu.sync(world, queue).expect("sync");
    let mut marcher = Marcher::new(device, 1, 1);
    marcher.set_look(LookParams {
        view,
        ..LookParams::default()
    });
    let camera = Camera {
        position: Vec3::from_array(origin),
        yaw,
        pitch: 0.0,
        vfov: 1.0,
    };
    marcher.render(device, queue, &gpu, &camera, &light);
    let px = readback::read_rgba8(device, queue, marcher.target());
    [px[0], px[1], px[2], px[3]]
}

/// 4 × 1 × 1 chunks: the first three uniform air, the fourth uniform rock.
fn three_air_then_rock() -> World {
    world([4, 1, 1], |v| if v[0] >= 192 { ROCK } else { AIR })
}

/// From the centre of the first chunk's -x face, the +x ray skips each air chunk as one 64³
/// cell on its table entry's `EMPTY_BIT` and hits the rock chunk's face on its 4th step,
/// reading no pool (25 steps before the chunk skip: eight uniform-air bricks per air chunk).
/// Its loads: one table entry per chunk (4), and in the rock chunk the two bits words of
/// brick (0, 4, 4)'s 16³ group (record words 9 and 11) and its one record entry: 7.
#[test]
fn a_ray_through_three_air_chunks_takes_4_steps() {
    let Some((device, queue)) = device() else {
        return;
    };
    let w = three_air_then_rock();
    assert_eq!(
        cost_x(&device, &queue, &w, [0.0, 32.5, 32.5]),
        Cost {
            steps: 4,
            loads: 4 + 2 + 1,
            pool: 0
        }
    );
}

/// As `three_air_then_rock`, with a floor of rock bricks (brick row y = 0) in the three air
/// chunks: none of them is empty, but a +x ray at y = 32.5 runs through brick rows 4, in
/// 16³ groups whose eight brick bits are clear, so it crosses each chunk in 4 steps and hits
/// the rock chunk on its 13th. The four groups of a row along x share two bits words, so each
/// air chunk costs its table entry and two bits words, and the rock chunk one entry more: 13.
#[test]
fn a_ray_above_a_floor_of_bricks_takes_4_steps_per_chunk() {
    let Some((device, queue)) = device() else {
        return;
    };
    let w = world(
        [4, 1, 1],
        |v| if v[0] >= 192 || v[1] < 8 { ROCK } else { AIR },
    );
    assert_eq!(
        cost_x(&device, &queue, &w, [0.0, 32.5, 32.5]),
        Cost {
            steps: 3 * 4 + 1,
            loads: 3 * 3 + 4,
            pool: 0
        }
    );
}

/// With the skips flag clear the same ray steps every voxel of the three air chunks: 192
/// steps, then the hit on the rock chunk's face, with no pool read (the rock chunk's bricks
/// are uniform). No bits word is read: the skips are off. Each chunk costs its table entry and
/// one record entry per brick entered, eight along x in an air chunk and one in the rock: 29.
#[test]
fn without_skips_every_voxel_of_an_air_chunk_is_a_step() {
    let Some((device, queue)) = device() else {
        return;
    };
    let w = three_air_then_rock();
    let light = LightParams {
        skips: false,
        ..LightParams::at_eye()
    };
    assert_eq!(
        cost(&device, &queue, &w, [0.0, 32.5, 32.5], light),
        Cost {
            steps: 3 * 64 + 1,
            loads: 3 * (1 + 8) + 2,
            pool: 0
        }
    );
}

/// One chunk of air but for brick 0, a `Full` brick holding one solid voxel at its far
/// corner (7, 7, 7).
fn far_corner_voxel() -> World {
    world([1, 1, 1], |v| if v == [7, 7, 7] { ROCK } else { AIR })
}

/// The +x ray through the corner voxel crosses brick 0 from its -x face: the empty 4³ block,
/// the empty 2³ block, the empty voxel (6, 7, 7) from the opaque line, then the hit, the only
/// pool read. Its loads: 1 table entry, the 2 bits words of brick 0's group, 1 record entry,
/// the 4 words of the slab (layers 4..8, rows 4..8) that the 4³, 2³ and voxel tests all read
/// from, 1 pool word and 1 palette word: 10.
#[test]
fn a_hit_in_a_full_brick_reads_the_pool_once() {
    let Some((device, queue)) = device() else {
        return;
    };
    let w = far_corner_voxel();
    assert_eq!(
        cost_x(&device, &queue, &w, [0.0, 7.5, 7.5]),
        Cost {
            steps: 4,
            loads: 1 + 2 + 1 + 4 + 1 + 1,
            pool: 1
        }
    );
}

/// With the masks flag clear the same ray reads each of the brick's 8 voxels on its row
/// through the pool, one step each; only the hit voxel's palette index is non-zero, so the
/// palette is read once: 1 table, 2 bits, 1 entry, 8 pool, 1 palette.
#[test]
fn without_masks_every_voxel_of_a_full_brick_is_read_through_the_pool() {
    let Some((device, queue)) = device() else {
        return;
    };
    let w = far_corner_voxel();
    let light = LightParams {
        masks: false,
        ..LightParams::at_eye()
    };
    assert_eq!(
        cost(&device, &queue, &w, [0.0, 7.5, 7.5], light),
        Cost {
            steps: 8,
            loads: 1 + 2 + 1 + 8 + 1,
            pool: 8
        }
    );
}

/// A +x ray through the far-corner chunk that misses brick 0. At y = z = 20.5 it runs along a
/// row of clear 16³ groups (bricks y = z = 2): one table entry and the row's two bits words,
/// held across the four groups, then the step out of the window: 3 loads. At y = z = 12.5 it
/// runs along brick 0's own row of groups (bricks y = z = 1): the first group holds brick 0,
/// so its two bricks (0, 1, 1) and (1, 1, 1) are entered and their uniform-air entries read,
/// then the three clear groups cost nothing more: 5 loads, 0 pool.
#[test]
fn a_ray_past_the_full_brick_reads_the_table_once_and_two_bits_words() {
    let Some((device, queue)) = device() else {
        return;
    };
    let w = far_corner_voxel();
    assert_eq!(
        cost_x(&device, &queue, &w, [0.0, 20.5, 20.5]),
        Cost {
            steps: 4 + 1,
            loads: 1 + 2,
            pool: 0
        }
    );
    assert_eq!(
        cost_x(&device, &queue, &w, [0.0, 12.5, 12.5]),
        Cost {
            steps: 2 + 3 + 1,
            loads: 1 + 2 + 2,
            pool: 0
        }
    );
}

/// A `Full` brick of air but for the 2³ block at its origin: the +x ray at y = z = 7.5 misses
/// it and reads no pool word. It takes two 4³ blocks, brick 1 (its 16³ group holds brick 0),
/// three clear 16³ groups, and the step that leaves the window. Loads: 1 table, 2 bits (one
/// row of groups), brick 0's entry, its slab's 4 mask words (both 4³ blocks lie in it), brick
/// 1's entry: 9.
#[test]
fn a_ray_past_the_one_solid_block_of_a_full_brick_reads_no_pool() {
    let Some((device, queue)) = device() else {
        return;
    };
    let w = world(
        [1, 1, 1],
        |v| if v.iter().all(|&c| c < 2) { ROCK } else { AIR },
    );
    assert_eq!(
        cost_x(&device, &queue, &w, [0.0, 7.5, 7.5]),
        Cost {
            steps: 7,
            loads: 1 + 2 + 1 + 4 + 1,
            pool: 0
        }
    );
}

/// One chunk of air but for the solid voxel `at`.
fn lone_voxel(at: [i32; 3]) -> World {
    world([1, 1, 1], move |v| if v == at { ROCK } else { AIR })
}

/// The ray from `(0.5, 0.5, 0.5)` along the normalized `(1, 1, 0)` crosses a lattice corner
/// at every voxel: the planes `x = n` and `y = n` are met at the same `t`, bit for bit, since
/// `sin` and `cos` of `FRAC_PI_4` agree in `f32` and the shader normalizes equal components
/// alike. By plane order the x plane is crossed first, so the ray enters voxel `(n, n - 1)`
/// before `(n, n)`: the lone voxel at `(4, 3, 0)` is hit through its -x face at
/// `t = 3.5 sqrt 2`, both by the walk that skips the empty 4³ block `[0, 4)²` and the 2³
/// block `[4, 6) × [2, 4)` and by the walk over single voxels, and both read the same hit
/// mask, face normal and lit value (lamp at the eye, intensity 10, albedo 0.5: `0.5 × 10 ×
/// cos 45° / 24.5` through the 1 / 2.2 transfer, 106 of 255; a hit one voxel on would read
/// 84). With the voxel at `(3, 4, 0)` instead the ray only touches its corner, and both
/// walks miss it.
#[test]
fn a_corner_tie_ray_hits_the_same_voxel_through_the_same_face_with_and_without_skips() {
    let Some((device, queue)) = device() else {
        return;
    };
    let origin = [0.5, 0.5, 0.5];
    let yaw = std::f32::consts::FRAC_PI_4;
    let on = LightParams {
        intensity: 10.0,
        ..LightParams::at_eye()
    };
    let off = LightParams { skips: false, ..on };
    let expected = (0.5f32 * 10.0 * std::f32::consts::FRAC_1_SQRT_2 / 24.5).powf(1.0 / 2.2) * 255.0;
    assert_eq!(expected.round() as u8, 106);
    for (at, hits) in [([4, 3, 0], true), ([3, 4, 0], false)] {
        let w = lone_voxel(at);
        let read = |light, view| pixel(&device, &queue, &w, origin, yaw, view, light);
        let (mask_on, mask_off) = (read(on, ViewMode::HitMask), read(off, ViewMode::HitMask));
        assert_eq!(mask_on, mask_off, "{at:?}: hit masks differ");
        assert_eq!(mask_on[0] == 255, hits, "{at:?}: hit mask {}", mask_on[0]);
        let (n_on, n_off) = (read(on, ViewMode::Normals), read(off, ViewMode::Normals));
        assert_eq!(n_on, n_off, "{at:?}: normals differ");
        let (lit_on, lit_off) = (read(on, ViewMode::Shaded), read(off, ViewMode::Shaded));
        assert_eq!(lit_on, lit_off, "{at:?}: shaded pixels differ");
        if hits {
            // The -x face: n = (-1, 0, 0) encodes as (0, 0.5, 0.5).
            assert_eq!(n_on[0], 0, "{at:?}: normal {n_on:?}");
            assert!(
                (127..=128).contains(&n_on[1]) && (127..=128).contains(&n_on[2]),
                "{at:?}: normal {n_on:?}"
            );
            for k in 0..3 {
                assert!(
                    (lit_on[k] as f32 - expected).abs() <= 2.0,
                    "{at:?}: lit {lit_on:?}, expected {expected:.1} in every channel"
                );
            }
        } else {
            assert_eq!(lit_on, [0, 0, 0, 255], "{at:?}: a miss shades black");
        }
    }
}
