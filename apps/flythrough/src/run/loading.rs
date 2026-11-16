//! Start-up stepped from the frame loop (ADR-0059): the light pre-roll as steps on a
//! [`PreRollRun`], and while a [`Loading`] is present each frame spends `--load-slice` of wall
//! time on steps, then draws a loading frame and returns without touching any per-frame world
//! path (the frame counter, the path player, the recorder, the HUD's wall times).

use crate::app::{diag, ms_since};
use crate::cli::RunMode;
use crate::edits::voxel_of;
use crate::now;
use crate::run::Run;
use crate::run::hud_feed::{band_census_line, field_line, field_pass_line};
use crate::run::queue::{COARSE_PRE_ROLL_MS, PRE_ROLL_SAMPLES, queue_view_angles};
use crate::setup::{Gfx, start_camera};
use std::time::Instant;
use vxe_app::{Flow, Gpu};
use vxe_render::{Camera, Frame, GpuWorld, HudColour, wgpu};
use vxe_world::ChunkCoord;

/// The pre-roll's cap on its step time, in ms.
pub(crate) const PRE_ROLL_CAP_MS: f64 = 120_000.0;

/// Cells of the loading frame's text bar.
const BAR_CELLS: usize = 40;

/// The light pre-roll's state between steps: the camera it bakes for, the near chunks, the
/// moment they settled and the coarse pass's chunks, and the step time spent, which its box
/// and cap count (so a sliced and an unsliced pre-roll stop at the same work).
pub(crate) struct PreRollRun {
    camera: Camera,
    centre: ChunkCoord,
    near: Vec<ChunkCoord>,
    /// The near chunks' moment: the step time, the chunks baked and the samples by then.
    near_done: Option<(f64, u64, u64)>,
    /// The full chunks mirrored at the near chunks' moment, which the coarse pass covers.
    first_pass: Vec<ChunkCoord>,
    /// Near chunks settled at the last step.
    pub(crate) settled: usize,
    boxed: bool,
    /// Step time spent so far, in ms.
    pub(crate) work_ms: f64,
    /// The longest single step, in ms.
    pub(crate) longest_ms: f64,
    /// Steps taken.
    pub(crate) steps: u32,
    pub(crate) done: bool,
}

impl PreRollRun {
    /// The stage the pre-roll is in, and its count: near chunks settled of all of them, then
    /// the coarse pass's chunks.
    pub(crate) fn stage(&self) -> (&'static str, String) {
        match self.near_done {
            None => (
                "lighting near chunks",
                format!("{} / {} chunks", self.settled, self.near.len()),
            ),
            Some(_) => (
                "lighting coarse pass",
                format!("{} chunks", self.first_pass.len()),
            ),
        }
    }

    /// The bar's fraction: the near chunks fill the first half, the coarse pass's time box
    /// the second.
    pub(crate) fn fraction(&self) -> f32 {
        let near = self.settled as f32 / self.near.len().max(1) as f32;
        match self.near_done {
            None => 0.5 * near,
            Some(_) if self.done => 1.0,
            Some((at, _, _)) => {
                0.5 + 0.5 * ((self.work_ms - at) / COARSE_PRE_ROLL_MS).clamp(0.0, 1.0) as f32
            }
        }
    }
}

/// The loading state while start-up steps from the frame loop.
pub(crate) struct Loading {
    pub(crate) pre_roll: PreRollRun,
    /// Wall time a frame spends on steps, in ms.
    slice_ms: f64,
    /// When loading began (device ready).
    began: Instant,
    /// The scene's row.
    title: String,
    /// Loading frames drawn, and the start of the last one.
    pub(crate) frames: u32,
    last: Option<Instant>,
    /// The intervals between loading frames' starts, in ms.
    pub(crate) intervals: Vec<f64>,
}

/// The scene a loading frame names: the castle's or the pose's banner, else the source.
pub(crate) fn scene_title(run: &Run) -> String {
    if let Some((row, _)) = &run.scene {
        return row.clone();
    }
    if let Some((row, _)) = &run.pose {
        return row.clone();
    }
    match run.args.mode {
        RunMode::Cave => "cave".to_owned(),
        RunMode::Level => "level".to_owned(),
        RunMode::Showcase => "showcase".to_owned(),
        RunMode::Room(edge) => format!("room of {edge} voxels"),
        RunMode::ThinRig => "thin rig".to_owned(),
        RunMode::SeamRig => "seam rig".to_owned(),
    }
}

/// A `cells`-wide text bar filled to `fraction`.
pub(crate) fn text_bar(fraction: f32, cells: usize) -> String {
    let filled = ((fraction.clamp(0.0, 1.0) * cells as f32).floor() as usize).min(cells);
    format!("[{}{}]", "#".repeat(filled), ".".repeat(cells - filled))
}

impl Run {
    /// Starts the pre-roll: the live queue started on an empty store with the run's settings,
    /// and the near chunks around the start camera listed.
    pub(crate) fn start_pre_roll(
        &mut self,
        gpu: &Gpu,
        frame: &mut Frame,
        mirror: &GpuWorld,
    ) -> PreRollRun {
        let settings = vxe_render::queue_settings();
        frame.start_bake_queue(
            &gpu.queue,
            mirror,
            settings,
            vxe_render::DEFAULT_BAKE_SLICE_MS,
        );
        frame.set_bake_budget(self.args.bake.budget);
        frame.set_bake_coarse(self.args.bake.coarse, self.args.bake.near);
        frame.set_bake_digs(self.args.bake.edit_ms, self.args.bake.dig_reach);
        frame.set_bake_reach(self.args.bake.reach);
        if let Some(q) = frame.bake_queue_mut() {
            q.order = self.args.queue_order();
            q.withdraw = self.args.bake.queue_withdraw;
            q.census_every_opening = self.args.bake.queue_census;
        }
        println!("{}", self.queue_row());
        frame.take_prepared(&std::mem::take(&mut self.queue.prepared), &self.world);
        let camera = start_camera(&self.args, self.spawn);
        let eye = camera.position.to_array();
        let centre = ChunkCoord(eye.map(|v| (v / 64.0).floor() as i32));
        let r = self.args.bake.near;
        let near: Vec<ChunkCoord> = (-r..=r)
            .flat_map(|z| {
                (-r..=r).flat_map(move |y| {
                    (-r..=r).map(move |x| {
                        ChunkCoord([centre.0[0] + x, centre.0[1] + y, centre.0[2] + z])
                    })
                })
            })
            .collect();
        PreRollRun {
            camera,
            centre,
            near,
            near_done: None,
            first_pass: Vec::new(),
            settled: 0,
            boxed: false,
            work_ms: 0.0,
            longest_ms: 0.0,
            steps: 0,
            done: false,
        }
    }

    /// One step of the pre-roll: streaming on the streamed world, the mirror's sync, then one
    /// bake step of [`PRE_ROLL_SAMPLES`]. Marks the pre-roll done once the near chunks hold
    /// their full blocks (or are known to need none) and, under `--coarse`, every full chunk
    /// mirrored at that moment has had its first pass, or the coarse pass ran its box, or the
    /// steps their cap. A failed sync or step records the session's failure and ends it.
    pub(crate) fn pre_roll_step(
        &mut self,
        gpu: &Gpu,
        frame: &mut Frame,
        mirror: &mut GpuWorld,
        pr: &mut PreRollRun,
    ) {
        let t = now();
        let camera = pr.camera;
        let mut enc = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        if let Some(stream) = self.stream.as_mut() {
            stream
                .streamer
                .update(&self.world, &[voxel_of(camera.position)]);
            // Each tick's time, as the frame loop records it: `track` reads a chunk's wanted
            // tick through it.
            stream.tick_ms.push(ms_since(self.start));
            let ev = stream.streamer.poll(&self.world);
            mirror.apply_events(&ev);
            frame.take_prepared(&ev.prepared, &self.world);
            stream.track(&ev, mirror);
        }
        mirror.set_focus(voxel_of(camera.position));
        if let Err(e) = mirror.sync(&self.world, &gpu.queue, &mut enc) {
            self.failure.set(format!("mirror sync failed: {e}"));
            pr.done = true;
            return;
        }
        gpu.queue.submit([enc.finish()]);
        let stream = self.stream.as_ref();
        let resident_fn = |c: ChunkCoord| {
            stream.is_some_and(|s| s.streamer.tier(c) == Some(vxe_world::Tier::Full))
        };
        let resident: Option<&dyn Fn(ChunkCoord) -> bool> = stream
            .is_some()
            .then_some(&resident_fn as &dyn Fn(ChunkCoord) -> bool);
        let (forward, _, _) = camera.basis();
        let (half_angle, frustum) = queue_view_angles(camera.vfov, frame.output_size());
        let view = vxe_render::QueueView {
            eye: camera.position.to_array(),
            forward: forward.to_array(),
            resident,
            samples: Some(PRE_ROLL_SAMPLES),
            interval_s: 0.0,
            hold: stream.map(|s| s.rule.r_full + self.args.light.radius() + 1),
            half_angle,
            frustum: Some(frustum),
            full_radius: stream.map(|s| s.rule.r_full),
        };
        if let Some(Err(e)) =
            frame.bake_queue_step(&gpu.device, &gpu.queue, mirror, &self.world, &view)
        {
            self.failure.set(format!("bake queue: {e}"));
            pr.done = true;
            return;
        }
        let bounded = stream.is_none();
        if stream.is_some() {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let step_ms = ms_since(t);
        pr.work_ms += step_ms;
        pr.steps += 1;
        pr.longest_ms = pr.longest_ms.max(step_ms);
        if pr.near_done.is_none() {
            pr.settled = pr
                .near
                .iter()
                .filter(|&&c| {
                    let absent = bounded && self.world.store().get(c).is_none();
                    frame.bake_settled(c, absent)
                })
                .count();
            if pr.settled == pr.near.len() {
                let st = frame.bake_queue().map(|q| q.stats()).unwrap_or_default();
                pr.near_done = Some((pr.work_ms, st.baked, st.total_samples));
                pr.first_pass = mirror.full_chunks().collect();
            }
        }
        if let Some((near_ms, _, _)) = pr.near_done {
            if self.args.bake.coarse == 0 || frame.bake_passed(&pr.first_pass) {
                pr.done = true;
            } else if pr.work_ms - near_ms > COARSE_PRE_ROLL_MS {
                pr.boxed = true;
                pr.done = true;
            }
        }
        if !pr.done && pr.work_ms > PRE_ROLL_CAP_MS {
            println!("bake queue: the pre-roll did not end in 120 s; going on");
            pr.done = true;
        }
    }

    /// Prints the pre-roll's line and hands the live queue its full-first radius.
    pub(crate) fn end_pre_roll(&mut self, frame: &mut Frame, pr: &PreRollRun) {
        let st = frame.bake_queue().map(|q| q.stats()).unwrap_or_default();
        let total = pr.work_ms;
        let (near_ms, near_baked, near_samples) =
            pr.near_done.unwrap_or((total, st.baked, st.total_samples));
        println!(
            "bake queue: the camera's chunk {:?} and the {} within {} at 4096 by {:.0} ms ({} chunks, {:.3e} samples); the coarse pass at {} by {:.0} ms (+{:.0} ms, +{} chunks, +{:.3e} samples, {} coarse{}); {} listed from the workers; {:.0} ms since the start",
            pr.centre.0,
            pr.near.len(),
            self.args.bake.near,
            near_ms,
            near_baked,
            near_samples as f64,
            self.args.bake.coarse,
            total,
            total - near_ms,
            st.baked - near_baked,
            (st.total_samples - near_samples) as f64,
            st.coarse,
            if pr.boxed {
                format!(
                    ", ended by its {:.0} s box with the rest of the {} chunks to the queue",
                    COARSE_PRE_ROLL_MS / 1e3,
                    pr.first_pass.len()
                )
            } else {
                String::new()
            },
            st.prepared,
            ms_since(self.start)
        );
        println!(
            "pre-roll: {} steps, {} chunks baked, {} samples, {:.0} ms of steps, the longest {:.1} ms",
            pr.steps, st.baked, st.total_samples, pr.work_ms, pr.longest_ms
        );
        // The pre-roll's near chunks take the full count under `--near`; the live queue's
        // full-first radius starts here.
        frame.set_bake_full_first(self.args.full_first());
    }

    /// The pre-roll run to its end in one call: today's start, under `--loading-screen off`.
    pub(crate) fn queue_pre_roll(&mut self, gpu: &Gpu, frame: &mut Frame, mirror: &mut GpuWorld) {
        let mut pr = self.start_pre_roll(gpu, frame, mirror);
        while !pr.done {
            self.pre_roll_step(gpu, frame, mirror, &mut pr);
        }
        if !self.failure.is_some() {
            self.end_pre_roll(frame, &pr);
        }
    }

    /// The start-up's work after the pre-roll: the field's initial fill and the band census.
    pub(crate) fn finish_load(&mut self, gpu: &Gpu) {
        let Some(gfx) = self.gfx.as_mut() else {
            return;
        };
        if gfx.mirror.field_surface() {
            println!("{}", field_pass_line("initial fill", gfx.timed_field(gpu)));
            println!("{}", field_line(&gfx.mirror));
        }
        diag(format_args!("field filled"));
        if self.args.look.band_census && self.stream.is_none() {
            println!("{}", band_census_line(&self.world, &gfx.mirror));
        }
        diag(format_args!("init done"));
    }

    /// Starts loading behind loading frames: the pre-roll, stepped from [`Run::loading_frame`].
    pub(crate) fn begin_loading(&mut self, gpu: &Gpu) {
        let Some(mut gfx) = self.gfx.take() else {
            return;
        };
        let pre_roll = self.start_pre_roll(gpu, &mut gfx.frame, &gfx.mirror);
        self.gfx = Some(gfx);
        self.loading = Some(Loading {
            pre_roll,
            slice_ms: self.args.loading.slice_ms,
            began: now(),
            title: scene_title(self),
            frames: 0,
            last: None,
            intervals: Vec::new(),
        });
    }

    /// One loading frame: steps until the slice is spent or the pre-roll ends, then draws the
    /// loading rows over a cleared surface. Once the pre-roll ends the rest of the start-up
    /// runs and the next frame is the first world frame.
    pub(crate) fn loading_frame(&mut self, gpu: &Gpu, surface: &wgpu::TextureView) -> Flow {
        let (Some(mut loading), Some(mut gfx)) = (self.loading.take(), self.gfx.take()) else {
            return Flow::Exit;
        };
        let t0 = now();
        if let Some(last) = loading.last.replace(t0) {
            loading.intervals.push((t0 - last).as_secs_f64() * 1e3);
        }
        while !loading.pre_roll.done && ms_since(t0) < loading.slice_ms {
            self.pre_roll_step(gpu, &mut gfx.frame, &mut gfx.mirror, &mut loading.pre_roll);
        }
        if self.failure.is_some() {
            self.gfx = Some(gfx);
            return Flow::Exit;
        }
        draw_loading(gpu, &mut gfx, surface, &loading);
        loading.frames += 1;
        if loading.pre_roll.done {
            self.end_pre_roll(&mut gfx.frame, &loading.pre_roll);
            self.gfx = Some(gfx);
            self.finish_load(gpu);
            let longest = loading.intervals.iter().copied().fold(0.0, f64::max);
            println!(
                "loading: pre-roll {:.0} ms, {} loading frames, longest frame {:.1} ms; the longest step {:.1} ms; {}",
                loading.pre_roll.work_ms,
                loading.frames,
                longest,
                loading.pre_roll.longest_ms,
                interval_line(&loading.intervals)
            );
            println!(
                "loading: {:.0} ms from device ready to the last loading frame",
                ms_since(loading.began)
            );
        } else {
            self.gfx = Some(gfx);
            self.loading = Some(loading);
        }
        Flow::Continue
    }
}

/// The loading frames' intervals as `interval p50 .. p95 .. max .. ms over n`.
fn interval_line(intervals: &[f64]) -> String {
    if intervals.is_empty() {
        return "no intervals".to_owned();
    }
    let mut v = intervals.to_vec();
    v.sort_by(f64::total_cmp);
    let p = |q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
    format!(
        "interval p50 {:.1} p95 {:.1} max {:.1} ms over {}",
        p(0.5),
        p(0.95),
        v[v.len() - 1],
        v.len()
    )
}

/// Clears `surface` and draws the loading rows: the scene, the stage and its count, the text
/// bar and the seconds since loading began.
fn draw_loading(gpu: &Gpu, gfx: &mut Gfx, surface: &wgpu::TextureView, loading: &Loading) {
    let (stage, detail) = loading.pre_roll.stage();
    let fraction = loading.pre_roll.fraction();
    let rows = [
        format!("loading {}", loading.title),
        format!("{stage}: {detail}"),
        format!(
            "{} {:3.0} %",
            text_bar(fraction, BAR_CELLS),
            fraction * 100.0
        ),
        format!("{:.1} s", ms_since(loading.began) / 1e3),
    ];
    let rows: [(&[u8], HudColour); 4] = [
        (rows[0].as_bytes(), HudColour::Yellow),
        (rows[1].as_bytes(), HudColour::White),
        (rows[2].as_bytes(), HudColour::White),
        (rows[3].as_bytes(), HudColour::White),
    ];
    gfx.hud.set_rows(&gpu.queue, &rows);
    let mut enc = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    {
        let _clear = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("loading clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: surface,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.02,
                        g: 0.02,
                        b: 0.03,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
    }
    let [w, h] = gpu.size;
    gfx.hud.encode(&mut enc, surface, w, h);
    gpu.queue.submit([enc.finish()]);
}

#[cfg(test)]
mod tests {
    use super::text_bar;

    /// The text bar fills `floor(fraction * cells)` cells and clamps outside 0 to 1.
    #[test]
    fn the_text_bar_fills_its_fraction() {
        assert_eq!(text_bar(0.0, 4), "[....]");
        assert_eq!(text_bar(0.5, 4), "[##..]");
        assert_eq!(text_bar(0.99, 4), "[###.]");
        assert_eq!(text_bar(1.0, 4), "[####]");
        assert_eq!(text_bar(1.5, 4), "[####]");
        assert_eq!(text_bar(-1.0, 4), "[....]");
    }
}
