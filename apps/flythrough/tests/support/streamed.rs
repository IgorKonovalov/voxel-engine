//! A world streamed in process around an eye, reached as by a jump, with the live bake queue
//! and a frame stepped each frame (`streamed`): the readings of the first seconds after a jump.

use vxe_render::{Exposure, Frame, LightParams, Lighting};

/// A streamed rig ([`streamed`]): the world's source, the eye the streamer follows, the
/// camera, the sky, the material styles, the full radius, the sky rays and the frames read.
pub struct Streamed {
    pub label: &'static str,
    pub source: std::sync::Arc<dyn vxe_world::WorldSource + Send + Sync>,
    pub eye: vxe_world::VoxelCoord,
    pub camera: vxe_render::Camera,
    pub sky: vxe_render::sky::Sky,
    pub styles: Vec<u32>,
    pub r: i32,
    pub rays: u32,
    pub reads: Vec<u32>,
}

/// What [`streamed`] read at a frame: the hit pixels' mean luminance of the accumulated
/// stored and direct parts, the store's missing and black counts (`StoreStats`), and the
/// mirrored chunks.
#[derive(Clone, Copy, Debug)]
pub struct StreamedRead {
    pub frame: u32,
    pub stored: f64,
    pub direct: f64,
    pub missing: u32,
    pub black: u32,
}

/// A world streamed in process around `s.eye`, reached as by a jump: the spawn's block first
/// (radii 1), then each frame the stream updated (full radius `s.r`, 2 more LOD, hysteresis 1,
/// 8 workers, a following window), polled and synced with no budget, the live queue stepped at
/// the fly-through's settings (coarse 8, near 2, the default slice) and the frame rendered at
/// 320 x 180, manual exposure 1, the lamp off, the store's counters on. At each frame of
/// `s.reads`: the hit pixels' mean luminance of the accumulated stored part and of the direct
/// part, the share of lighting pixels that cast the sky ray and their mean visibility, the
/// store's missing and black counts, the mirrored chunks and the queue's depth. Printed and
/// returned; empty with no adapter.
pub fn streamed(s: Streamed) -> Vec<StreamedRead> {
    use super::SyncNow;
    use vxe_render::{GpuWorld, LodParams, MirrorConfig, QueueView, Window};
    use vxe_world::{ChunkCoord, PersistConfig, Residency, StreamConfig, Streamer, Tier, World};
    let mut out = Vec::new();
    let r = s.r;
    let residency = Residency {
        r_full: r,
        r_lod: r + 2,
        hysteresis: 1,
    };
    let side = 2 * (residency.r_lod + residency.hysteresis) + 1;
    let budget = 1u64 << 30;
    let Some((device, queue, adapter)) = super::reading_device(budget) else {
        return out;
    };
    println!("reading: {} on {adapter}, r_full {r}", s.label);
    let eye = s.eye;
    let world = World::new(&flythrough::outdoor_materials());
    struct DynSource(std::sync::Arc<dyn vxe_world::WorldSource + Send + Sync>);
    impl vxe_world::WorldSource for DynSource {
        fn chunk(&self, coord: ChunkCoord, out: &mut vxe_world::Chunk) {
            self.0.chunk(coord, out);
        }
    }
    let mut streamer = Streamer::new(
        std::sync::Arc::new(DynSource(s.source.clone())),
        StreamConfig {
            residency,
            workers: 8,
            persist: PersistConfig::default(),
        },
    );
    let mut gpu = GpuWorld::with_config(
        &device,
        MirrorConfig {
            budget_bytes: budget,
            ..MirrorConfig::new(Window::Follow { side })
        },
    );
    streamer.set_radii(1, 1);
    streamer.update(&world, &[eye]);
    let ev = streamer.wait_idle(&world);
    gpu.apply_events(&ev);
    gpu.set_focus(eye);
    gpu.sync_now(&world, &device, &queue)
        .expect("the mirror fits");
    streamer.set_radii(residency.r_full, residency.r_lod);
    let mut frame = Frame::new(&device, 320, 180);
    frame.set_lod(LodParams {
        r_full: residency.r_full,
        forced_level: None,
    });
    frame.set_lighting(Lighting {
        lamp: false,
        exposure: Exposure::Manual(1.0),
        sky_rays: s.rays,
        ..flythrough::default_lighting()
    });
    frame.set_sky(Some(s.sky));
    if !s.styles.is_empty() {
        frame.set_material_styles(&queue, &s.styles);
    }
    frame.set_count_store(true);
    frame.set_dt(1.0 / 60.0);
    frame.start_bake_queue(
        &queue,
        &gpu,
        vxe_render::queue_settings(),
        vxe_render::DEFAULT_BAKE_SLICE_MS,
    );
    frame.set_bake_coarse(8, 2);
    let camera = s.camera;
    let lum = |p: &[f32; 4]| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
    let last = s.reads.iter().copied().max().unwrap_or(0);
    for f in 0..=last {
        streamer.update(&world, &[eye]);
        let ev = streamer.poll(&world);
        gpu.apply_events(&ev);
        gpu.set_focus(eye);
        let mut enc = device.create_command_encoder(&Default::default());
        gpu.sync(&world, &queue, &mut enc).expect("the mirror fits");
        queue.submit([enc.finish()]);
        let resident_fn = |c: ChunkCoord| streamer.tier(c) == Some(Tier::Full);
        let view = QueueView {
            eye: camera.position.to_array(),
            forward: camera.basis().0.to_array(),
            resident: Some(&resident_fn),
            samples: None,
            interval_s: 1.0 / 60.0,
            hold: Some(residency.r_full + gpu.light_radius() + 1),
            half_angle: {
                let [w, h] = frame.output_size();
                vxe_render::internals::view_half_angle(camera.vfov, w as f32 / h as f32)
            },
            frustum: None,
            full_radius: Some(residency.r_full),
            lod_radius: Some(residency.r_lod.max(residency.r_full)),
        };
        let st = frame
            .bake_queue_step(&device, &queue, &mut gpu, &world, &view)
            .expect("the queue runs")
            .expect("the queue's step");
        frame.step(&device, &queue, &gpu, &camera, &LightParams::headlamp());
        if !s.reads.contains(&f) {
            continue;
        }
        let hits = vxe_render::debug::read_rgba32f(&device, &queue, frame.light_hit_positions());
        let acc = vxe_render::debug::read_rgba16f(&device, &queue, frame.accumulated());
        let dir = vxe_render::debug::read_rgba16f(&device, &queue, frame.direct_accumulated());
        let vis = vxe_render::debug::read_rgba8(&device, &queue, frame.visibility_samples());
        let ss = frame.read_store_stats(&device, &queue);
        let (mut n, mut a, mut d, mut cast, mut open) = (0usize, 0.0f64, 0.0f64, 0usize, 0.0f64);
        for i in 0..hits.len() {
            if hits[i][3] < 1.0e30 {
                n += 1;
                a += f64::from(lum(&acc[i]));
                d += f64::from(dir[i][3]);
                if vis[4 * i + 3] > 0 {
                    cast += 1;
                    open += f64::from(vis[4 * i]) / 255.0;
                }
            }
        }
        let read = StreamedRead {
            frame: f,
            stored: a / n.max(1) as f64,
            direct: d / n.max(1) as f64,
            missing: ss.missing,
            black: ss.still_black,
        };
        println!(
            "reading: frame {f}: {n} hits, stored {:.4e}, direct {:.4e}, sky ray cast at {cast} (mean open {:.4}), missing {} black {} fallback {}, mirrored {}, queue depth {} baked {} unbaked {} waiting reach {}",
            read.stored,
            read.direct,
            open / cast.max(1) as f64,
            ss.missing,
            ss.still_black,
            ss.fallback,
            gpu.mirrored_chunks(),
            st.depth,
            st.baked,
            st.unbaked,
            st.waiting_reach,
        );
        out.push(read);
    }
    out
}
