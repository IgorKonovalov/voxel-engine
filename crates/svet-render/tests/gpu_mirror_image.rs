//! The mirror's slot images (ADR-0056): a streamed world mirrored with its new full records
//! packed inline and from the stream workers' images reads back the same bytes, and an image
//! the upload must not use (an edited chunk, other material flags, the field surface on) is
//! dropped by its reason and the chunk packed inline.

mod support;

use support::*;

use std::sync::Arc;
use svet_render::{
    ChunkWork, DEFAULT_IMAGE_CAP_BYTES, GpuWorld, ImageHook, MirrorConfig, Packing, SyncStats,
    Window, wgpu,
};
use svet_world::*;

/// Materials 1 to 40 by `(id - 1) mod 4`: emissive opaque, opaque, translucent, emissive
/// translucent.
fn rich_table() -> MaterialTable {
    rich_table_but(None)
}

/// [`rich_table`] with row `flip`'s translucency flipped.
fn rich_table_but(flip: Option<u16>) -> MaterialTable {
    let mut t = MaterialTable::new();
    for id in 1..=40u16 {
        let mut m = Material::named(format!("m{id}"));
        let kind = (id - 1) % 4;
        if kind == 0 || kind == 3 {
            m.emission = [1.0, 0.5, 0.25];
        }
        if (kind >= 2) != (flip == Some(id)) {
            m.flags = MaterialFlags::TRANSLUCENT;
        }
        t.push(m);
    }
    t
}

/// Each brick by a hash of its coordinate: air, a uniform opaque or translucent brick, or a
/// mixed brick of 2, 3, 6, 12 or 40 materials (a planar class each, the last wide) over some
/// air, so a chunk holds translucent voxels and emitters on brick faces and inside.
struct Rich;

impl WorldSource for Rich {
    fn chunk(&self, coord: ChunkCoord, out: &mut Chunk) {
        let base = coord.min_voxel().0;
        for slot in 0..CHUNK_BRICKS {
            let b = brick_of_slot(slot);
            let bmin = [0, 1, 2].map(|k| base[k] + b[k] * BRICK_EDGE);
            let h = hash::hash3(0x0107, bmin.map(|v| v >> 3));
            let n = [2u64, 3, 6, 12, 40][(h >> 8) as usize % 5];
            let first = 1 + (h >> 16) % (41 - n);
            let brick = match h % 8 {
                0 | 1 => Brick::Uniform(AIR),
                2 => Brick::Uniform(2 + 4 * ((h >> 4) % 10) as u16),
                3 => Brick::Uniform(3 + 4 * ((h >> 4) % 10) as u16),
                _ => Brick::from_fn(|l| {
                    let v = hash::hash3(0xF00D, [0, 1, 2].map(|k| bmin[k] + l[k]));
                    if v.is_multiple_of(5) {
                        AIR
                    } else {
                        (first + (v >> 3) % n) as u16
                    }
                }),
            };
            out.set_brick(slot, brick);
        }
    }
}

/// One arm: a world streamed from [`Rich`] at r_full 2 with the workers' hook, its images
/// under `hook`, mirrored by `gpu`.
struct Arm {
    world: World,
    streamer: Streamer,
    gpu: GpuWorld,
    hook: ImageHook,
    stats: Vec<SyncStats>,
}

impl Arm {
    /// An arm packing by `packing`, its hook's flags from `hook_table`, the field surface on
    /// with `field`.
    fn new(
        device: &wgpu::Device,
        packing: Packing,
        hook_table: &MaterialTable,
        field: bool,
    ) -> Self {
        let table = rich_table();
        let world = World::new(&table);
        let hook = ImageHook::new(hook_table, packing, DEFAULT_IMAGE_CAP_BYTES);
        let mut streamer = Streamer::new(
            Arc::new(Rich),
            StreamConfig {
                residency: Residency {
                    r_full: 2,
                    r_lod: 2,
                    hysteresis: 1,
                },
                workers: 4,
                persist: PersistConfig::default(),
            },
        );
        streamer.set_prep(Some(Arc::new(ChunkWork::new(&table, Some(hook.clone())))));
        let cfg = MirrorConfig {
            packing,
            ..small(Window::Follow { side: 7 })
        };
        let gpu = if field {
            GpuWorld::with_field_surface(device, cfg)
        } else {
            GpuWorld::with_config(device, cfg)
        };
        Self {
            world,
            streamer,
            gpu,
            hook,
            stats: Vec::new(),
        }
    }

    /// Streams around `focus` until the workers are idle and hands the events to the mirror.
    fn stream(&mut self, focus: VoxelCoord) {
        self.streamer.update(&self.world, &[focus]);
        let ev = self.streamer.wait_idle(&self.world);
        self.gpu.apply_events(&ev);
        self.gpu.set_focus(focus);
    }

    /// Syncs until nothing is pending.
    fn sync(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        loop {
            let st = self.gpu.sync_now(&self.world, device, queue).unwrap();
            self.stats.push(st);
            if st.pending_chunks == 0 {
                break;
            }
        }
    }

    /// The sum of `f` over every sync's stats.
    fn total(&self, f: impl Fn(&SyncStats) -> u64) -> u64 {
        self.stats.iter().map(f).sum()
    }
}

/// Asserts the two mirrors read back the same pool shards, records and table, and account
/// the same live and allocated bytes.
fn assert_same_bytes(device: &wgpu::Device, queue: &wgpu::Queue, a: &GpuWorld, b: &GpuWorld) {
    let read = |x: &wgpu::Buffer| svet_render::debug::read_buffer(device, queue, x, 0, x.size());
    for k in 0..4 {
        assert!(
            read(a.pool_shard(k)) == read(b.pool_shard(k)),
            "pool shard {k} differs"
        );
    }
    assert!(read(a.records()) == read(b.records()), "records differ");
    assert!(read(a.table()) == read(b.table()), "tables differ");
    assert_eq!(a.live_units(), b.live_units());
    assert_eq!(a.live_bricks(), b.live_bricks());
    assert_eq!(a.mirror_bytes(), b.mirror_bytes());
    assert_eq!(a.allocated_bytes(), b.allocated_bytes());
    assert_eq!(a.mirrored_chunks(), b.mirrored_chunks());
}

/// Plan 0107 Phase 3. One streamed world mirrored twice, inline and from the workers' images,
/// at a first focus and after a move that evicts and loads chunks (so the second batch reuses
/// freed slots): after each, the two mirrors' pool shards, records and table read back equal
/// byte for byte, with the same live and allocated bytes (VRAM unchanged). Every new record of
/// the `Workers` arm came from an image and none of the `Inline` arm's did; no image is held
/// once the syncs are done.
#[test]
fn the_mirror_is_byte_identical_whichever_thread_packed() {
    let Some((device, queue)) = device() else {
        return;
    };
    let table = rich_table();
    let mut inline = Arm::new(&device, Packing::Inline, &table, false);
    let mut workers = Arm::new(&device, Packing::Workers, &table, false);
    for focus in [VoxelCoord([0, 0, 0]), VoxelCoord([192, 0, 0])] {
        for arm in [&mut inline, &mut workers] {
            arm.stream(focus);
            arm.sync(&device, &queue);
        }
        assert_same_bytes(&device, &queue, &inline.gpu, &workers.gpu);
    }
    let chunks = workers.total(|s| s.chunks);
    assert!(chunks > 40, "{chunks} records written");
    assert_eq!(workers.total(|s| s.images_used), chunks);
    assert_eq!(
        workers.total(|s| s.packed_bricks),
        0,
        "nothing packed inline"
    );
    assert_eq!(inline.total(|s| s.images_used), 0);
    assert_eq!(inline.hook.stats().made, 0);
    let h = workers.hook.stats();
    assert_eq!(h.made, chunks, "{h:?}");
    assert_eq!((h.refused, h.held_bytes), (0, 0), "{h:?}");
    assert_eq!(workers.gpu.held_images(), 0);
}

/// A chunk carved between its load and its first sync is uploaded from the chunk, not its
/// image: the bytes equal the inline arm's after the same carve, and the stats count one image
/// dropped as edited.
#[test]
fn an_edited_chunk_ignores_its_image() {
    let Some((device, queue)) = device() else {
        return;
    };
    let table = rich_table();
    let mut inline = Arm::new(&device, Packing::Inline, &table, false);
    let mut workers = Arm::new(&device, Packing::Workers, &table, false);
    let focus = VoxelCoord([0, 0, 0]);
    for arm in [&mut inline, &mut workers] {
        arm.stream(focus);
        let r = arm
            .world
            .apply(&EditOp::box_(
                centre_of_voxel([1, 1, 1]),
                centre_of_voxel([20, 20, 20]),
                Mode::Carve,
            ))
            .unwrap();
        assert!(
            r.changed_from
                .iter()
                .map(|&(_, n)| u64::from(n))
                .sum::<u64>()
                > 1000
        );
        arm.sync(&device, &queue);
    }
    assert_same_bytes(&device, &queue, &inline.gpu, &workers.gpu);
    assert_eq!(workers.total(|s| s.images_edited), 1);
    assert_eq!(
        workers.total(|s| s.images_used),
        workers.total(|s| s.chunks) - 1
    );
    assert_eq!(inline.total(|s| s.images_edited), 0);
}

/// Images packed under a table whose flags differ from the world's (one row's translucency) are
/// dropped at the upload: the bytes equal the inline arm's, and every record counts its image
/// dropped for its flags.
#[test]
fn an_image_under_other_flags_is_ignored() {
    let Some((device, queue)) = device() else {
        return;
    };
    let table = rich_table();
    let other = rich_table_but(Some(2));
    let mut inline = Arm::new(&device, Packing::Inline, &table, false);
    let mut workers = Arm::new(&device, Packing::Workers, &other, false);
    for arm in [&mut inline, &mut workers] {
        arm.stream(VoxelCoord([0, 0, 0]));
        arm.sync(&device, &queue);
    }
    assert_same_bytes(&device, &queue, &inline.gpu, &workers.gpu);
    let chunks = workers.total(|s| s.chunks);
    assert!(workers.hook.stats().made >= chunks);
    assert_eq!(workers.total(|s| s.images_other_flags), chunks);
    assert_eq!(workers.total(|s| s.images_used), 0);
}

/// With the field surface on no image is used: the bytes equal the inline arm's, band slots
/// included, and every record counts its image dropped for the field.
#[test]
fn no_image_is_used_with_the_field_surface_on() {
    let Some((device, queue)) = device() else {
        return;
    };
    let table = rich_table();
    let mut inline = Arm::new(&device, Packing::Inline, &table, true);
    let mut workers = Arm::new(&device, Packing::Workers, &table, true);
    for arm in [&mut inline, &mut workers] {
        arm.stream(VoxelCoord([0, 0, 0]));
        arm.sync(&device, &queue);
    }
    assert_same_bytes(&device, &queue, &inline.gpu, &workers.gpu);
    assert!(workers.gpu.band_bricks() > 0);
    let chunks = workers.total(|s| s.chunks);
    assert_eq!(workers.total(|s| s.images_field), chunks);
    assert_eq!(workers.total(|s| s.images_used), 0);
}
