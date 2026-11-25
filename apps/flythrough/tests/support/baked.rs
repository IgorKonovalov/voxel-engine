//! The wobble rigs' shared settle (Plan 0104 Phase 0): the live bake queue stepped as the
//! fly-through steps it, with a device wait every frame, until the store is baked in view, and a
//! census of the store before any read.
//!
//! **Why the wait.** The queue learns that a submit finished from a readback's map callback, and
//! its step polls the device without blocking; it sends nothing new while its submits in flight
//! are at their cap. A rig frame that is neither read back nor waited on submits and returns, so
//! on a fast adapter the frames outrun the bake and step after step finds the cap full: a count of
//! frames is not a measure of baking. [`queue_step`] waits on the device before it steps, which
//! finishes the frame before and runs its callbacks.
//!
//! **The census.** The distinct chunks the lighting pixels' hits lie in (the hit nudged a hundredth
//! of a voxel along the view ray into the solid), against those of them the queue reports settled
//! (`BakeQueue::settled`: the block at its tier's count with nothing pending, or a chunk with no
//! face to bake); the coarse blocks among them counted apart. A census is whole when every chunk in
//! view is settled.

use svet_render::{Camera, Frame, QueueStats, wgpu};

use super::terms::Rig;

/// The frame step the queue is told, in seconds (the rigs' 60 Hz).
pub const INTERVAL_S: f64 = 1.0 / 60.0;

/// The settle's frames between two censuses while the queue still reports a chunk in view
/// waiting (a bounded world's edge chunks can keep it waiting for good), and the most frames.
pub const CENSUS_EVERY: usize = 600;
pub const SETTLE_MAX: usize = 36_000;

/// The fly-through's bake knobs on the frame's queue: the coarse pass's samples and the near
/// radius, the full-first radius, the tiers by size on screen and the pre-roll drain's radius
/// (as `run::loading` sets them from the default flags).
pub fn fly_through_knobs(f: &mut Frame) {
    f.set_bake_coarse(
        svet_render::DEFAULT_COARSE_SAMPLES,
        svet_render::DEFAULT_NEAR,
    );
    f.set_bake_full_first(Some(svet_render::DEFAULT_NEAR));
    f.set_bake_tiers(svet_render::DEFAULT_TIERS);
    f.set_bake_drain(Some(svet_render::DEFAULT_DRAIN));
}

/// The queue's view of `cam` as the fly-through gives it: the view's half angles (the frustum)
/// and focal length, so the tiers by size on screen apply.
pub fn view(f: &Frame, cam: &Camera, interval_s: f64) -> svet_render::QueueView<'static> {
    let size = f.output_size();
    let aspect = size[0] as f32 / size[1] as f32;
    svet_render::QueueView {
        eye: cam.position.to_array(),
        forward: cam.basis().0.to_array(),
        resident: None,
        samples: None,
        interval_s,
        hold: None,
        half_angle: svet_render::internals::view_half_angle(cam.vfov, aspect),
        frustum: Some(svet_render::internals::view_half_angles(cam.vfov, aspect)),
        full_radius: None,
        lod_radius: None,
        focal_px: Some(svet_render::Bands::focal_px(size[1], cam.vfov)),
    }
}

/// Waits until the device has finished every submit, running their callbacks.
pub fn wait(rig: &Rig) {
    rig.device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("the device polls");
}

/// One step of the live queue at `cam`: a device wait first (so the frame before has finished
/// and the queue sees its readbacks), then the step with [`view`] at [`INTERVAL_S`].
pub fn queue_step(rig: &mut Rig, f: &mut Frame, cam: &Camera) -> QueueStats {
    queue_step_at(rig, f, cam, INTERVAL_S)
}

/// [`queue_step`] with the frame step `interval_s`.
pub fn queue_step_at(rig: &mut Rig, f: &mut Frame, cam: &Camera, interval_s: f64) -> QueueStats {
    wait(rig);
    let v = view(f, cam, interval_s);
    f.bake_queue_step(&rig.device, &rig.queue, &mut rig.gpu, &rig.world, &v)
        .expect("the queue runs")
        .expect("the queue's step")
}

/// One frame at `cam`: the queue's step ([`queue_step`]), the frame (the headlamp's light
/// params, as `Stepper::run`), a device wait.
pub fn frame(rig: &mut Rig, f: &mut Frame, cam: &Camera) -> QueueStats {
    let q = queue_step(rig, f, cam);
    f.step(
        &rig.device,
        &rig.queue,
        &rig.gpu,
        cam,
        &svet_render::LightParams::headlamp(),
    );
    wait(rig);
    q
}

/// The store's state in view: chunks settled, chunks in view, coarse blocks among them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Census {
    pub settled: usize,
    pub chunks: usize,
    pub coarse: usize,
}

impl Census {
    pub fn whole(&self) -> bool {
        self.settled == self.chunks
    }

    /// The census as one line: `census S/N (whole; coarse C)` or `(NOT WHOLE; ...)`.
    pub fn line(&self) -> String {
        format!(
            "census {}/{} ({}; coarse {})",
            self.settled,
            self.chunks,
            if self.whole() { "whole" } else { "NOT WHOLE" },
            self.coarse
        )
    }
}

/// The census of `f`'s store at its last frame from `cam` (see the module's doc).
pub fn census(rig: &Rig, f: &Frame, cam: &Camera) -> Census {
    let hits = svet_render::debug::read_rgba32f(&rig.device, &rig.queue, f.light_hit_positions());
    let eye = cam.position.to_array();
    let mut chunks = std::collections::HashSet::new();
    for h in &hits {
        if h[3] >= 1e30 || !h[3].is_finite() {
            continue;
        }
        let d = [h[0] - eye[0], h[1] - eye[1], h[2] - eye[2]];
        let t = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-3);
        let v = [0, 1, 2].map(|k| (h[k] + 0.01 * d[k] / t).floor() as i32);
        chunks.insert(v.map(|x| x.div_euclid(svet_world::CHUNK_EDGE)));
    }
    let Some(q) = f.bake_queue() else {
        return Census {
            chunks: chunks.len(),
            ..Census::default()
        };
    };
    let store = f.store();
    Census {
        settled: chunks
            .iter()
            .filter(|&&c| q.settled(store, svet_world::ChunkCoord(c), false))
            .count(),
        chunks: chunks.len(),
        coarse: chunks
            .iter()
            .filter(|&&c| store.is_coarse(svet_world::ChunkCoord(c)))
            .count(),
    }
}

/// What [`settle`] ended on.
#[derive(Clone, Debug)]
pub struct Settled {
    pub frames: usize,
    pub secs: f64,
    pub census: Census,
    pub stats: QueueStats,
}

impl Settled {
    /// The settle as one line, the census last.
    pub fn line(&self) -> String {
        format!(
            "settled {} frames, {:.1} s: depth {} jobs {} fading {} waiting in view {}; {}",
            self.frames,
            self.secs,
            self.stats.depth,
            self.stats.jobs,
            self.stats.fading,
            self.stats.drain_waiting,
            self.census.line()
        )
    }
}

/// The shared settle at `cam`: at least `min` frames of [`frame`] (each with its device wait),
/// then on until no job is open and no block fades and the census is whole. The census is read
/// when the queue reports nothing waiting in view, else every [`CENSUS_EVERY`] frames; at
/// [`SETTLE_MAX`] frames the settle ends whatever the census, and its line says NOT WHOLE.
/// Prints the line ([`Settled::line`]) under `label`.
#[allow(
    clippy::disallowed_methods,
    reason = "the settle prints its own wall time, a reading only"
)]
pub fn settle(rig: &mut Rig, f: &mut Frame, cam: &Camera, min: usize, label: &str) -> Settled {
    let t0 = std::time::Instant::now();
    let mut n = 0usize;
    loop {
        let q = frame(rig, f, cam);
        n += 1;
        let quiet = q.fading == 0 && q.jobs == 0;
        let drained = quiet && q.drain_waiting == 0 && !q.draining;
        if n < min || !(drained || n.is_multiple_of(CENSUS_EVERY) || n >= SETTLE_MAX) {
            continue;
        }
        let c = census(rig, f, cam);
        if (quiet && c.whole()) || n >= SETTLE_MAX {
            let s = Settled {
                frames: n,
                secs: t0.elapsed().as_secs_f64(),
                census: c,
                stats: q,
            };
            println!("{label}: {}", s.line());
            return s;
        }
    }
}
