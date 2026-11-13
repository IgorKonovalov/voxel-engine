//! The loaded light's rigs (ADR-0046, Plan 0059), shared by `tests/gpu_bake.rs` and the `bake`
//! example: a hand-built box mirrored whole under a frame, its bake's brick records read back by
//! brick and face, the CPU's brick-level value of a face, ADR-0049's staircase and its frames.

use std::collections::{BTreeMap, HashMap};

use flythrough::cave::{ORE, ROCK};
use vxe_render::{
    BakeSettings, Camera, Exposure, Frame, GpuWorld, LightParams, Lighting, LightingResolution,
    MirrorConfig, Window, wgpu,
};
use vxe_world::{
    AIR, ChunkCoord, ChunkExtent, EditOp, FIXED_ONE, Fixed3, MaterialTable, Mode, UniformSource,
    World,
};

use super::{SyncNow, cave_table};

/// `world` mirrored whole, a frame over it, and the device they live on.
pub struct Rig {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub gpu: GpuWorld,
    pub world: World,
    pub frame: Frame,
    pub cfg: MirrorConfig,
}

pub fn fill(world: &World, min: [i64; 3], max: [i64; 3], m: vxe_world::MaterialId) {
    world
        .apply(&EditOp::box_(
            Fixed3(min.map(|v| v * FIXED_ONE)),
            Fixed3(max.map(|v| v * FIXED_ONE)),
            Mode::Fill(m),
        ))
        .expect("the box is resident");
}

pub fn air_world(size: [i32; 3], table: &MaterialTable) -> World {
    World::generate(
        ChunkExtent::new([0, 0, 0], size),
        &UniformSource(AIR),
        table,
    )
}

/// `world` mirrored whole under a frame at `size` (full-resolution lighting, no jitter, lamp and
/// probes off, exposure 8).
pub fn rig_on(world: World, size: [u32; 2], light_radius: Option<i32>) -> Option<Rig> {
    let (device, queue) = super::software_device(64 << 20)?;
    let base = MirrorConfig::new(Window::Fixed(world.extent()));
    let cfg = MirrorConfig {
        budget_bytes: 64 << 20,
        light_radius: light_radius.unwrap_or(base.light_radius),
        ..base
    };
    let mut gpu = GpuWorld::with_config(&device, cfg);
    for _ in 0..2 {
        gpu.sync_now(&world, &device, &queue)
            .expect("the mirror fits");
    }
    let frame = new_frame(&device, size);
    Some(Rig {
        device,
        queue,
        gpu,
        world,
        frame,
        cfg,
    })
}

pub fn new_frame(device: &wgpu::Device, size: [u32; 2]) -> Frame {
    let mut frame = Frame::new(device, size[0], size[1]);
    frame.set_jitter(device, false);
    frame.set_resolution(device, LightingResolution::Full);
    frame.set_lighting(Lighting {
        lamp: false,
        lamp_bounce: false,
        exposure: Exposure::Manual(8.0),
        bake_pool_bytes: 16 << 20,
        ..Lighting::default()
    });
    frame.set_dt(1.0 / 60.0);
    frame
}

impl Rig {
    pub fn bake(&mut self, settings: &BakeSettings) -> vxe_render::BakeReport {
        let extent = self.world.extent();
        self.frame
            .bake_until(
                &self.device,
                &self.queue,
                &mut self.gpu,
                &self.world,
                extent,
                settings,
                &mut |_| {},
            )
            .expect("the box fits the pool")
    }

    /// Every chunk's block, for a comparison of the cells.
    pub fn blocks(&self) -> BTreeMap<[i32; 3], Vec<u32>> {
        frame_blocks(&self.device, &self.queue, &self.frame, self.world.extent())
    }

    /// Every held direction of the store's brick records by brick and face ([`cells_of`]).
    pub fn cells(&self) -> HashMap<([i32; 3], u8), [u32; 2]> {
        cells_of(&self.blocks())
    }

    /// Applies `op`, mirrors it, and re-bakes its ring to `settings` as the window's frame does:
    /// the report and the edit's box.
    pub fn dig(
        &mut self,
        op: &EditOp,
        settings: &BakeSettings,
    ) -> (vxe_render::DigReport, [[i32; 3]; 2]) {
        let r = self.world.apply(op).expect("the dig is resident");
        let (min, max) = r.aabb.expect("the dig changed voxels");
        self.gpu
            .sync_now(&self.world, &self.device, &self.queue)
            .expect("the mirror fits");
        self.frame.invalidate(min.0, max.0);
        let report = self
            .frame
            .rebake_digs(
                &self.device,
                &self.queue,
                &mut self.gpu,
                &self.world,
                settings,
                &mut |_| {},
            )
            .expect("a box waits")
            .expect("the ring fits");
        (report, [min.0, max.0])
    }

    /// A fresh mirror and frame of the world as it is now, `piece` baked to `settings`: its records'
    /// directions ([`cells_of`]).
    pub fn fresh_cells(
        &self,
        piece: ChunkExtent,
        settings: &BakeSettings,
    ) -> HashMap<([i32; 3], u8), [u32; 2]> {
        let (_, frame) = self.fresh_bake(piece, settings);
        cells_of(&frame_blocks(
            &self.device,
            &self.queue,
            &frame,
            self.world.extent(),
        ))
    }

    pub fn fresh_bake(&self, piece: ChunkExtent, settings: &BakeSettings) -> (GpuWorld, Frame) {
        let mut gpu = GpuWorld::with_config(&self.device, self.cfg);
        for _ in 0..2 {
            gpu.sync_now(&self.world, &self.device, &self.queue)
                .expect("the mirror fits");
        }
        let mut frame = new_frame(&self.device, [16, 16]);
        frame
            .bake_until(
                &self.device,
                &self.queue,
                &mut gpu,
                &self.world,
                piece,
                settings,
                &mut |_| {},
            )
            .expect("the box fits the pool");
        (gpu, frame)
    }

    pub fn accum(&self) -> Vec<[f32; 4]> {
        vxe_render::debug::read_float(&self.device, &self.queue, self.frame.accumulated())
    }
}

/// `frame`'s block of every chunk of `extent` that has one.
pub fn frame_blocks(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    frame: &Frame,
    e: ChunkExtent,
) -> BTreeMap<[i32; 3], Vec<u32>> {
    let s = e.size();
    let mut out = BTreeMap::new();
    for z in 0..s[2] {
        for y in 0..s[1] {
            for x in 0..s[0] {
                let c = [e.min.0[0] + x, e.min.0[1] + y, e.min.0[2] + z];
                if let Some(b) = frame.read_store_block(device, queue, ChunkCoord(c)) {
                    out.insert(c, b);
                }
            }
        }
    }
    out
}

/// The brick records of `blocks` (cells of 8 voxels, one a brick) by world brick and face, each
/// direction the record holds: its `rgb9e5` word and the record's tail word (`material | mask <<
/// 16`).
pub fn cells_of(blocks: &BTreeMap<[i32; 3], Vec<u32>>) -> HashMap<([i32; 3], u8), [u32; 2]> {
    let mut out = HashMap::new();
    for (&c, block) in blocks {
        for slot in 0..512usize {
            let dir = block[slot];
            if dir == 0 {
                continue;
            }
            let rec = &block
                [(dir >> 11) as usize - vxe_render::internals::BRICK_WORDS..(dir >> 11) as usize];
            let b = vxe_world::brick_of_slot(slot);
            let g = [0, 1, 2].map(|k| c[k] * 8 + b[k]);
            for f in 0..6u8 {
                if rec[6] >> (16 + f) & 1 != 0 {
                    out.insert((g, f), [rec[usize::from(f)], rec[6]]);
                }
            }
        }
    }
    out
}

/// The brick level's value of face `face` of voxel `v` (temporal.wgsl's `store_brick_value` at
/// cells of 8 voxels, emission scale 1): the 2 x 2 bricks around the voxel's centre in the face's
/// plane, bilinear over those that hold the direction; its own brick's alone where none has weight.
pub fn brick_value(
    cells: &HashMap<([i32; 3], u8), [u32; 2]>,
    v: [i32; 3],
    face: u8,
) -> Option<[f32; 3]> {
    let axis = usize::from(face / 2);
    let (ta, tb) = ((axis + 1) % 3, (axis + 2) % 3);
    let own = v.map(|x| x.div_euclid(8));
    let g = v.map(|x| (x as f32 + 0.5) / 8.0 - 0.5);
    let lo = g.map(|x| x.floor() as i32);
    let f = [0, 1, 2].map(|k| g[k] - g[k].floor());
    let mut sum = [0f32; 3];
    let mut weight = 0f32;
    for i in 0..4 {
        let (oa, ob) = (i & 1, i >> 1);
        let mut b = own;
        b[ta] = lo[ta] + oa;
        b[tb] = lo[tb] + ob;
        let wa = if oa == 1 { f[ta] } else { 1.0 - f[ta] };
        let wb = if ob == 1 { f[tb] } else { 1.0 - f[tb] };
        if let Some(c) = cells.get(&(b, face)) {
            let x = vxe_render::internals::rgb9e5_unpack(c[0]);
            for k in 0..3 {
                sum[k] += x[k] * (wa * wb);
            }
            weight += wa * wb;
        }
    }
    if weight > 0.0 {
        return Some(sum.map(|x| x / weight));
    }
    cells
        .get(&(own, face))
        .map(|c| vxe_render::internals::rgb9e5_unpack(c[0]))
}

impl Rig {
    pub fn step(&mut self, camera: &Camera) {
        self.frame.step(
            &self.device,
            &self.queue,
            &self.gpu,
            camera,
            &LightParams::default(),
        );
    }

    pub fn display(&self) -> Vec<u8> {
        vxe_render::debug::read_rgba8(&self.device, &self.queue, self.frame.target())
    }
}

/// A hit's cube face, from its position and record: its voxel and entry face.
pub fn hit_face(eye: [f32; 3], pos: [f32; 4], info: [u16; 4]) -> Option<([i32; 3], u8)> {
    let flags = info[3];
    if flags & 1 == 0 || flags & 6 != 0 {
        return None;
    }
    let axis = i32::from((flags >> 3) & 3) - 1;
    if axis < 0 {
        return None;
    }
    let axis = axis as usize;
    let positive = eye[axis] > pos[axis];
    let mut p = [pos[0], pos[1], pos[2]];
    p[axis] += if positive { -0.5 } else { 0.5 };
    Some((
        p.map(|v| v.floor() as i32),
        (2 * axis) as u8 + u8::from(positive),
    ))
}

/// The brick level at `samples` a brick direction, rounds of 64.
pub fn test_counts(samples: u32) -> BakeSettings {
    BakeSettings {
        round: 64,
        brick_samples: samples,
        ..BakeSettings::default()
    }
}

pub fn luma(rgba: &[u8]) -> Vec<f32> {
    rgba.chunks(4)
        .map(|p| 0.2126 * f32::from(p[0]) + 0.7152 * f32::from(p[1]) + 0.0722 * f32::from(p[2]))
        .collect()
}

// ---- ADR-0049: the store read by the far normal and the filters weighed by face ----

/// A 45-degree staircase of 1-voxel steps rising along +x over chunks `[0, 5) x [0, 1) x
/// [0, 5)`: column `x` in 64..320 holds rock up to `z = x - 64`, its tread facing +z and its
/// riser -x, and an ore plate at z 300 over it lights the treads more than the risers. The
/// camera: [`stair_cameras`].
pub fn staircase() -> World {
    let world = air_world([5, 1, 5], &cave_table());
    for x in 64..320i64 {
        fill(&world, [x, 0, 0], [x + 1, 64, x - 63], ROCK);
    }
    fill(&world, [64, 0, 300], [320, 64, 304], ORE);
    world
}

/// The staircase's two cameras at `size` (70 degrees horizontal): far, 194 voxels above the
/// slope at x 70 looking down 10 degrees along +x, whose rays descend so they see treads and
/// risers, from about 125 to 245 voxels away; near, 24 voxels above it at x 100 looking up
/// 25 degrees, whose hits start about 25 voxels away (risers only past the slope's angle).
pub fn stair_cameras(size: [u32; 2]) -> [Camera; 2] {
    let vfov = vfov_of(size);
    [
        Camera {
            position: [70.5, 32.5, 200.5].into(),
            yaw: 0.0,
            pitch: (-10f32).to_radians(),
            vfov,
        },
        Camera {
            position: [100.5, 32.5, 60.5].into(),
            yaw: 0.0,
            pitch: 25f32.to_radians(),
            vfov,
        },
    ]
}

/// The vertical field of view of a horizontal 70 degrees at `size`.
pub fn vfov_of(size: [u32; 2]) -> f32 {
    2.0 * (35f32.to_radians().tan() * size[1] as f32 / size[0] as f32).atan()
}

/// One baked frame of the staircase per arm: (store read, far normal, look, weights,
/// resolution) and its read-backs.
pub struct StairFrame {
    pub accum: Vec<[f32; 4]>,
    pub pos: Vec<[f32; 4]>,
    pub info: Vec<[u16; 4]>,
    pub display: Vec<u8>,
}

pub struct StairArm {
    pub read: vxe_render::StoreRead,
    pub weights: vxe_render::LightWeights,
    pub far_normal: [f32; 2],
    pub mode: vxe_render::NormalMode,
    pub resolution: LightingResolution,
}

pub fn stair_frame(rig: &mut Rig, camera: &Camera, arm: &StairArm) -> StairFrame {
    rig.frame.set_resolution(&rig.device, arm.resolution);
    rig.frame.set_lighting(Lighting {
        lamp: false,
        lamp_bounce: false,
        exposure: Exposure::Manual(8.0),
        bake_pool_bytes: 64 << 20,
        store_read: arm.read,
        light_weights: arm.weights,
        ..Lighting::default()
    });
    rig.frame.set_look(vxe_render::LookParams {
        normal_mode: arm.mode,
        far_normal: arm.far_normal,
        ..vxe_render::LookParams::default()
    });
    // Two frames: the first after a setting change carries no history to differ by.
    rig.step(camera);
    rig.step(camera);
    StairFrame {
        accum: vxe_render::debug::read_float(&rig.device, &rig.queue, rig.frame.accumulated()),
        pos: vxe_render::debug::read_float(&rig.device, &rig.queue, rig.frame.hit_positions()),
        info: vxe_render::debug::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records()),
        display: rig.display(),
    }
}

/// The stored part ADR-0049's read gives a hit of normal `n` on face `face` of voxel `v`:
/// the brick level's direction values the normal leans toward, mixed by `n_a^2` over those
/// held, renormalised; the face's own where none is.
pub fn by_normal(
    cells: &HashMap<([i32; 3], u8), [u32; 2]>,
    v: [i32; 3],
    face: u8,
    n: [f32; 3],
) -> Option<[f32; 3]> {
    let own = brick_value(cells, v, face);
    let mut sum = [0f32; 3];
    let mut weight = 0f32;
    for (a, &na) in n.iter().enumerate() {
        let w = na * na;
        if w < 1e-4 {
            continue;
        }
        let dir = 2 * a as u8 + u8::from(na > 0.0);
        if let Some(r) = if dir == face {
            own
        } else {
            brick_value(cells, v, dir)
        } {
            for k in 0..3 {
                sum[k] += r[k] * w;
            }
            weight += w;
        }
    }
    if weight > 0.0 {
        Some(sum.map(|x| x / weight))
    } else {
        own
    }
}

pub fn within(a: [f32; 3], b: [f32; 3], rel: f32) -> bool {
    (0..3).all(|k| (a[k] - b[k]).abs() <= rel * b[k].abs().max(1e-3))
}

pub fn rgb(p: [f32; 4]) -> [f32; 3] {
    [p[0], p[1], p[2]].map(|x| x / 8.0)
}
