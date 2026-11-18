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
    /// The fly-through's streamed residency and mirror in place of the rig's own ([`Hold`]).
    pub hold: Option<Hold>,
}

/// [`streamed`] at the fly-through's streamed settings: LOD radius `r_lod`, the mirror's
/// default 3 GiB budget with the fly-through's record count and window, the column wait on or
/// off (`Lighting::column_wait`), and at the end the unbaked chunks within the full radius,
/// each with the chunks that hold its reach's columns from final
/// ([`vxe_render::finality_blockers`]) and why.
#[derive(Clone, Copy, Debug)]
pub struct Hold {
    pub r_lod: i32,
    pub wait: bool,
    /// The live queue's reach mode (`--reach`; the fly-through's default `Full`).
    pub reach: vxe_render::ReachMode,
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
    /// Under a [`Hold`]: the full chunks mirrored whose reach lies within the full radius
    /// (distance from the eye's chunk plus the light radius at most `r_full`), not faceless and
    /// holding no block; 0 without a hold.
    pub unbaked_inner: u32,
    /// The chunks the lighting pixels hit that hold no block.
    pub hit_unbaked: u32,
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
    use vxe_render::{GpuWorld, MirrorConfig, QueueView, Window};
    use vxe_world::{ChunkCoord, PersistConfig, Residency, StreamConfig, Streamer, Tier, World};
    let mut out = Vec::new();
    let r = s.r;
    let residency = Residency {
        r_full: r,
        r_lod: s.hold.map_or(r + 2, |h| h.r_lod),
        hysteresis: 1,
    };
    let side = 2 * (residency.r_lod + residency.hysteresis) + 1;
    let budget = s
        .hold
        .map_or(1u64 << 30, |_| vxe_render::DEFAULT_BUDGET_BYTES);
    let Some((device, queue, adapter)) = super::reading_device(budget) else {
        return out;
    };
    println!("reading: {} on {adapter}, r_full {r}", s.label);
    let eye = s.eye;
    let world = World::new(&flythrough::outdoor_materials());
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
            record_chunks: s.hold.map(|_| flythrough::stream_record_chunks(residency)),
            ..MirrorConfig::new(Window::Follow { side })
        },
    );
    streamer.set_radii(1, 1);
    streamer.update(&world, &[eye]);
    gpu.apply_events(&streamer.wait_idle(&world));
    gpu.set_focus(eye);
    gpu.sync_now(&world, &device, &queue)
        .expect("the mirror fits");
    streamer.set_radii(residency.r_full, residency.r_lod);
    let mut frame = streamed_frame(&device, &queue, &gpu, &s, residency.r_full);
    let camera = s.camera;
    let lum = |p: &[f32; 4]| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
    let last = s.reads.iter().copied().max().unwrap_or(0);
    for f in 0..=last {
        streamer.update(&world, &[eye]);
        gpu.apply_events(&streamer.poll(&world));
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
        if f == last && s.hold.is_some() {
            held_report(&gpu, &frame, &view, eye, residency.r_full);
            held_hits(&device, &queue, &gpu, &frame, &view, &|c| streamer.tier(c));
        }
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
        let (unbaked_inner, hit_unbaked) = unbaked(&gpu, &frame, &hits, eye, residency.r_full);
        let read = StreamedRead {
            frame: f,
            stored: a / n.max(1) as f64,
            direct: d / n.max(1) as f64,
            missing: ss.missing,
            black: ss.still_black,
            unbaked_inner,
            hit_unbaked,
        };
        println!(
            "reading: frame {f}: pending {}, coarse {}, {n} hits, stored {:.4e}, direct {:.4e}, sky ray cast at {cast} (mean open {:.4}), missing {} black {} fallback {}, mirrored {}, queue depth {} baked {} unbaked {} waiting reach {}, unbaked within the full radius by reach {}, hit chunks with no block {}",
            gpu.pending_chunks(),
            st.coarse,
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
            read.unbaked_inner,
            read.hit_unbaked,
        );
        out.push(read);
    }
    out
}

/// [`streamed`]'s frame: 320 x 180, LOD at the full radius `r_full`, manual exposure 1, the
/// lamp off, `s`'s sky rays, sky and styles, the column wait and reach of `s.hold` (else the
/// defaults), the store's counters on, and the live queue at the fly-through's settings.
fn streamed_frame(
    device: &vxe_render::wgpu::Device,
    queue: &vxe_render::wgpu::Queue,
    gpu: &vxe_render::GpuWorld,
    s: &Streamed,
    r_full: i32,
) -> Frame {
    let mut frame = Frame::new(device, 320, 180);
    frame.set_lod(vxe_render::LodParams {
        r_full,
        forced_level: None,
    });
    frame.set_lighting(Lighting {
        lamp: false,
        exposure: Exposure::Manual(1.0),
        sky_rays: s.rays,
        column_wait: s.hold.is_some_and(|h| h.wait),
        ..flythrough::default_lighting()
    });
    frame.set_sky(Some(s.sky));
    if !s.styles.is_empty() {
        frame.set_material_styles(queue, &s.styles);
    }
    frame.set_count_store(true);
    frame.set_dt(1.0 / 60.0);
    frame.start_bake_queue(
        queue,
        gpu,
        vxe_render::queue_settings(),
        vxe_render::DEFAULT_BAKE_SLICE_MS,
    );
    frame.set_bake_coarse(8, 2);
    if let Some(h) = s.hold {
        frame.set_bake_reach(h.reach);
    }
    frame
}

/// [`StreamedRead::unbaked_inner`] and [`StreamedRead::hit_unbaked`] (`hits`, the lighting
/// grid's hit positions).
fn unbaked(
    gpu: &vxe_render::GpuWorld,
    frame: &vxe_render::Frame,
    hits: &[[f32; 4]],
    eye: vxe_world::VoxelCoord,
    r_full: i32,
) -> (u32, u32) {
    let e = eye.0.map(|v| v.div_euclid(64));
    let lr = f64::from(gpu.light_radius());
    let q = frame.bake_queue().expect("the queue runs");
    let no_block = |c: vxe_world::ChunkCoord| frame.store().block(c).is_none();
    let inner = gpu
        .full_chunks()
        .filter(|c| {
            let d = (0..3)
                .map(|k| f64::from(c.0[k] - e[k]).powi(2))
                .sum::<f64>()
                .sqrt();
            d + lr <= f64::from(r_full) && no_block(*c) && !q.tracks(*c).contains(&"faceless")
        })
        .count() as u32;
    let hit: std::collections::BTreeSet<[i32; 3]> = hits
        .iter()
        .filter(|h| h[3] < 1.0e30)
        .map(|h| [0, 1, 2].map(|k| (h[k].floor() as i32).div_euclid(64)))
        .filter(|c| no_block(vxe_world::ChunkCoord(*c)))
        .collect();
    (inner, hit.len() as u32)
}

/// A shared world source as the streamer takes it.
struct DynSource(std::sync::Arc<dyn vxe_world::WorldSource + Send + Sync>);

impl vxe_world::WorldSource for DynSource {
    fn chunk(&self, coord: vxe_world::ChunkCoord, out: &mut vxe_world::Chunk) {
        self.0.chunk(coord, out);
    }
}

/// [`Hold`]'s end report: every full chunk mirrored within `r_full` of `eye`'s chunk that holds
/// no block and is not known faceless, with its queue tracks and the chunks that hold its
/// reach's columns from final, each named pending, or wanted and absent with the streamer's
/// distance from the eye's chunk and whether it lies inside the mirror's window.
fn held_report(
    gpu: &vxe_render::GpuWorld,
    frame: &vxe_render::Frame,
    view: &vxe_render::QueueView<'_>,
    eye: vxe_world::VoxelCoord,
    r_full: i32,
) {
    let e = eye.0.map(|v| v.div_euclid(64));
    let q = frame.bake_queue().expect("the queue runs");
    let store = frame.store();
    let lr = gpu.light_radius();
    let w = gpu.window();
    let mut unbaked = Vec::new();
    for c in gpu.full_chunks() {
        let d2: i64 = (0..3).map(|k| i64::from(c.0[k] - e[k]).pow(2)).sum();
        if d2 > i64::from(r_full).pow(2) || store.block(c).is_some() {
            continue;
        }
        let tracks = q.tracks(c);
        if tracks.contains(&"faceless") {
            continue;
        }
        unbaked.push((c, tracks));
    }
    println!(
        "reading: held: {} full chunks within r_full {r_full} unbaked (window {:?} + {:?})",
        unbaked.len(),
        w.min.0,
        w.size()
    );
    let mut blockers = std::collections::BTreeMap::<[i32; 3], (bool, usize)>::new();
    for (c, tracks) in &unbaked {
        let b = vxe_render::finality_blockers(gpu, *c, lr, view);
        println!(
            "reading: held: unbaked {:?} tracks {:?}: {} blockers",
            c.0,
            tracks,
            b.len()
        );
        for (bc, pending) in b {
            blockers.entry(bc.0).or_insert((pending, 0)).1 += 1;
        }
    }
    for (bc, (pending, n)) in blockers {
        let d2: i64 = (0..3).map(|k| i64::from(bc[k] - e[k]).pow(2)).sum();
        let inside = (0..3).all(|k| bc[k] >= w.min.0[k] && bc[k] < w.min.0[k] + w.size()[k]);
        println!(
            "reading: held: blocker {bc:?} holds {n}: pending {pending}, d2 {d2}, in window {inside}"
        );
    }
}

/// [`Hold`]'s report of the hit chunks with no block at the last frame: the fly-through's
/// `--dump-black` lines ([`flythrough::black_dump_lines`], frame 0 on them), then per chunk the
/// lighting pixels that hit it, its queue tracks, its streamer tier, whether the mirror holds it
/// full, and the chunks that hold its columns from final.
fn held_hits(
    device: &vxe_render::wgpu::Device,
    queue: &vxe_render::wgpu::Queue,
    gpu: &vxe_render::GpuWorld,
    frame: &vxe_render::Frame,
    view: &vxe_render::QueueView<'_>,
    tier: &dyn Fn(vxe_world::ChunkCoord) -> Option<vxe_world::Tier>,
) {
    let hits = vxe_render::debug::read_rgba32f(device, queue, frame.light_hit_positions());
    let mut by = std::collections::BTreeMap::<[i32; 3], usize>::new();
    for h in hits.iter().filter(|h| h[3] < 1.0e30) {
        let c = [0, 1, 2].map(|k| (h[k].floor() as i32).div_euclid(64));
        if frame.store().block(vxe_world::ChunkCoord(c)).is_none() {
            *by.entry(c).or_default() += 1;
        }
    }
    if let Ok(path) = std::env::var("VXE_HELD_PNG") {
        // The display, and beside it the lighting grid: red where the hit's chunk holds no
        // block, grey where it does, black at a miss.
        let t = frame.target();
        let disp = vxe_render::debug::read_rgba8(device, queue, t);
        super::write_png(&path.clone().into(), &disp, t.width(), t.height());
        let g = frame.light_hit_positions();
        let mask: Vec<u8> = hits
            .iter()
            .flat_map(|h| {
                if h[3] >= 1.0e30 {
                    return [0, 0, 0, 255];
                }
                let c = [0, 1, 2].map(|k| (h[k].floor() as i32).div_euclid(64));
                if frame.store().block(vxe_world::ChunkCoord(c)).is_none() {
                    [255, 0, 0, 255]
                } else {
                    [128, 128, 128, 255]
                }
            })
            .collect();
        super::write_png(
            &format!("{path}.mask.png").into(),
            &mask,
            g.width(),
            g.height(),
        );
    }
    // The fly-through's `--dump-black` lines on the same read-back.
    let black = frame.read_store_stats(device, queue).still_black;
    for l in flythrough::black_dump_lines(gpu, frame, &hits, view, 0, black) {
        println!("reading: {l}");
    }
    let q = frame.bake_queue().expect("the queue runs");
    let jobs = q.jobs();
    println!("reading: held hits: {} chunks hit with no block", by.len());
    for (c, n) in by {
        let cc = vxe_world::ChunkCoord(c);
        let b = vxe_render::finality_blockers(gpu, cc, gpu.light_radius(), view);
        println!(
            "reading: held hit chunk {c:?}: {n} px, tracks {:?}, in a job {}, tier {:?}, mirrored {} lod {} pending {}, {} blockers {:?}",
            q.tracks(cc),
            jobs.contains(&cc),
            tier(cc),
            gpu.is_mirrored(cc),
            gpu.is_lod(cc),
            gpu.is_pending(cc),
            b.len(),
            b.iter().take(4).collect::<Vec<_>>()
        );
    }
}
