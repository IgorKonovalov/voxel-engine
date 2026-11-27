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

/// One arm: a world streamed from [`Rich`] (at r_full 2, r_lod 2 unless built
/// [`Arm::with_ring`]) with the workers' hook, its images under `hook`, mirrored by `gpu`.
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
        Self::with_budget(device, packing, hook_table, field, SMALL_BUDGET)
    }

    /// [`Arm::new`] with the mirror's reservation at `budget` bytes.
    fn with_budget(
        device: &wgpu::Device,
        packing: Packing,
        hook_table: &MaterialTable,
        field: bool,
        budget: u64,
    ) -> Self {
        Self::with_ring(device, packing, hook_table, field, budget, 2)
    }

    /// [`Arm::with_budget`] streaming LOD records out to `r_lod` around the full radius of 2,
    /// in a window of side `2 * (r_lod + 1) + 1` (the residency and its hysteresis).
    fn with_ring(
        device: &wgpu::Device,
        packing: Packing,
        hook_table: &MaterialTable,
        field: bool,
        budget: u64,
        r_lod: i32,
    ) -> Self {
        let table = rich_table();
        let world = World::new(&table);
        let hook = ImageHook::new(hook_table, packing, DEFAULT_IMAGE_CAP_BYTES);
        let mut streamer = Streamer::new(
            Arc::new(Rich),
            StreamConfig {
                residency: Residency {
                    r_full: 2,
                    r_lod,
                    hysteresis: 1,
                },
                workers: 4,
                persist: PersistConfig::default(),
            },
        );
        streamer.set_prep(Some(Arc::new(ChunkWork::new(&table, Some(hook.clone())))));
        let cfg = MirrorConfig {
            packing,
            budget_bytes: budget,
            ..small(Window::Follow {
                side: 2 * (r_lod + 1) + 1,
            })
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

/// Asserts the two mirrors hold the same light lists per chunk and brick, place the same
/// chunks' ranges (by range start), and read back the same light-sample buffer up to its high-water mark.
fn assert_same_lights(device: &wgpu::Device, queue: &wgpu::Queue, a: &GpuWorld, b: &GpuWorld) {
    assert!(
        a.emitter_lists() == b.emitter_lists(),
        "emitter lists differ"
    );
    let placed = |g: &GpuWorld| {
        let mut v: Vec<_> = g.light_chunks().collect();
        v.sort_by_key(|&(_, start, _, _)| start);
        v
    };
    assert_eq!(placed(a), placed(b));
    assert_eq!(a.light_samples(), b.light_samples());
    assert_eq!(a.light_span(), b.light_span());
    let len = u64::from(a.light_span()) * svet_render::internals::LIGHT_SAMPLE_BYTES;
    let read =
        |g: &GpuWorld| svet_render::debug::read_buffer(device, queue, g.light_buffer(), 0, len);
    assert!(read(a) == read(b), "light-sample buffers differ");
}

/// Plan 0107 Phase 3. One streamed world mirrored twice, inline and from the workers' images,
/// at a first focus and after a move that evicts and loads chunks (so the second batch reuses
/// freed slots): after each, the two mirrors' pool shards, records and table read back equal
/// byte for byte, with the same live and allocated bytes (VRAM unchanged), and the same light
/// lists per chunk and brick and light-sample buffer ([`Rich`] holds emissive bricks). Every
/// new record of the `Workers` arm came from an image and none of the `Inline` arm's did; the
/// `Workers` arm sized no chunk and scanned no brick for emitters (Plan 0107 Phase 4), which
/// the `Inline` arm did; no image is held once the syncs are done.
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
        assert!(workers.gpu.light_samples() > 0, "the world holds emitters");
        assert_same_lights(&device, &queue, &inline.gpu, &workers.gpu);
    }
    assert_eq!(workers.total(|s| s.sized_chunks), 0, "no chunk sized");
    assert_eq!(workers.total(|s| s.light_bricks), 0, "no brick scanned");
    assert!(inline.total(|s| s.sized_chunks) > 0);
    assert!(inline.total(|s| s.light_bricks) > 0);
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

/// Plan 0111 Phase 6d (ADR-0089: `Workers` is the default). The world streamed at r_full 2
/// and r_lod 4, mirrored inline and from the workers' images: at the first focus, then after a
/// move of three chunks along x that promotes LOD records to full ones (their images held from
/// the promoting load) and demotes full records to LOD ones. After every sync, the two arms'
/// syncs write, defer and leave pending the same chunks, and their pool shards, records, table,
/// light lists and light-sample buffer read back equal byte for byte. The `Workers` arm's
/// promoted records came from images, and it sized every chunk the `Inline` arm sized but
/// those its images carried (a LOD record has no image).
#[test]
fn a_promoted_and_a_demoted_record_are_byte_identical_whichever_thread_packed() {
    let Some((device, queue)) = device() else {
        return;
    };
    let table = rich_table();
    let budget = 256 << 20;
    let mut inline = Arm::with_ring(&device, Packing::Inline, &table, false, budget, 4);
    let mut workers = Arm::with_ring(&device, Packing::Workers, &table, false, budget, 4);
    let mut before: Option<(Vec<ChunkCoord>, Vec<ChunkCoord>)> = None;
    for focus in [VoxelCoord([0, 0, 0]), VoxelCoord([192, 0, 0])] {
        for arm in [&mut inline, &mut workers] {
            arm.stream(focus);
        }
        let used_before = workers.total(|s| s.images_used);
        let mut syncs = 0;
        loop {
            let a = sync_line(&mut inline, &device, &queue);
            let b = sync_line(&mut workers, &device, &queue);
            assert_eq!(a, b, "focus {focus:?}: (error, written, deferred, pending)");
            assert!(!b.0, "focus {focus:?}: a sync failed");
            assert_same_bytes(&device, &queue, &inline.gpu, &workers.gpu);
            assert_same_lights(&device, &queue, &inline.gpu, &workers.gpu);
            syncs += 1;
            if b.3 == 0 {
                break;
            }
            assert!(
                syncs < 16,
                "focus {focus:?}: still pending after {syncs} syncs"
            );
        }
        let g = &workers.gpu;
        let mirrored: Vec<ChunkCoord> = g.full_chunks().collect();
        let lod: Vec<ChunkCoord> = g
            .window()
            .coords()
            .into_iter()
            .filter(|&c| g.is_lod(c))
            .collect();
        assert!(!lod.is_empty(), "focus {focus:?}: no LOD record");
        if let Some((full0, lod0)) = &before {
            let promoted: Vec<_> = mirrored.iter().filter(|c| lod0.contains(c)).collect();
            let demoted: Vec<_> = lod.iter().filter(|c| full0.contains(c)).collect();
            assert!(!promoted.is_empty(), "no LOD record promoted");
            assert!(!demoted.is_empty(), "no full record demoted");
            let used = workers.total(|s| s.images_used) - used_before;
            assert!(
                used >= promoted.len() as u64,
                "{used} images used for {} promotions",
                promoted.len()
            );
        }
        before = Some((mirrored, lod));
    }
    assert!(workers.gpu.light_samples() > 0, "the world holds emitters");
    // A LOD record has no image (its bricks are mips), so both arms size and pack it inline;
    // the `Workers` arm sizes every chunk the `Inline` arm sizes but those its images carried.
    let used = workers.total(|s| s.images_used);
    assert!(used > 0);
    assert_eq!(
        workers.total(|s| s.sized_chunks) + used,
        inline.total(|s| s.sized_chunks),
        "(sized, images used)"
    );
    assert!(workers.total(|s| s.packed_bricks) < inline.total(|s| s.packed_bricks));
    assert_eq!(inline.total(|s| s.images_used), 0);
}

/// What one sync of an arm did, for comparing two arms' syncs: whether it returned an error,
/// records written, chunks deferred and chunks pending after it.
fn sync_line(arm: &mut Arm, device: &wgpu::Device, queue: &wgpu::Queue) -> (bool, u64, u64, u64) {
    let r = arm.gpu.sync_now(&arm.world, device, queue);
    let st = r.as_ref().copied().unwrap_or_default();
    arm.stats.push(st);
    (
        r.is_err(),
        st.chunks,
        st.deferred_chunks,
        arm.gpu.pending_chunks() as u64,
    )
}

/// Plan 0107 Phase 4. On a pool too small for the nearest chunk (one page), and on one that
/// holds a few chunks and defers the rest (eight pages), the `Workers` arm, sizing from the
/// images, defers the same chunks in the same order as the `Inline` arm, sizing inline: each of
/// three syncs returns the same error or none, writes and defers as many records, and leaves the
/// same pool, records and table. A deferred chunk is written nothing (on one page no byte at
/// all) and keeps its image for the sync that takes it: the images held are the chunks pending.
#[test]
fn a_deferred_chunk_with_an_image_is_deferred_whole() {
    let Some((device, queue)) = device() else {
        return;
    };
    let table = rich_table();
    for pages in [1, 8] {
        let budget = pages_budget(pages, 7 * 7 * 7);
        let mut inline = Arm::with_budget(&device, Packing::Inline, &table, false, budget);
        let mut workers = Arm::with_budget(&device, Packing::Workers, &table, false, budget);
        for arm in [&mut inline, &mut workers] {
            arm.stream(VoxelCoord([0, 0, 0]));
        }
        let loaded = workers.gpu.held_images();
        assert!(loaded > 10, "{loaded} images");
        let mut lines = Vec::new();
        for _ in 0..3 {
            let a = sync_line(&mut inline, &device, &queue);
            let b = sync_line(&mut workers, &device, &queue);
            assert_eq!(a, b, "{pages} pages: (error, written, deferred, pending)");
            lines.push(b);
            assert_same_bytes(&device, &queue, &inline.gpu, &workers.gpu);
            assert_eq!(workers.gpu.held_images() as u64, b.3, "{pages} pages");
        }
        let written: u64 = lines.iter().map(|l| l.1).sum();
        assert_eq!(workers.total(|s| s.images_used), written);
        assert_eq!(workers.total(|s| s.sized_chunks), 0);
        if pages == 1 {
            // An empty sync returns the deferral as its error, its stats unread.
            assert!(lines.iter().all(|l| l.0), "{lines:?}");
            assert_eq!(
                workers.gpu.bytes_written(),
                0,
                "nothing written on one page"
            );
            assert_eq!(workers.gpu.held_images(), loaded);
        } else {
            let first = lines[0];
            assert!(!first.0 && first.1 > 0 && first.2 > 0, "{lines:?}");
        }
    }
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
