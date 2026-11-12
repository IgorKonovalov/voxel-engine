//! The autopilot's steps in the run: the flight, its samples and the report.

use crate::app::{ms_since, peak_rss_bytes};
use crate::autopilot::{Autopilot, Leg, PILOT_LEG_SECS, PILOT_RADIUS, PILOT_REACH, PILOT_SPEED};
use crate::edits::{centre_fixed, edit_line, voxel_of};
use crate::now;
use crate::run::Run;
use crate::run::hud_feed::{belt_line, probe_bytes_line};
use crate::setup::Gfx;
use crate::streaming::pressure_terms;
use crate::{chunks_hash, percentile};
use vxe_app::Flow;
use vxe_render::FRAME_PASSES;
use vxe_render::glam::Vec3;
use vxe_world::{EditOp, FIXED_ONE, Mode, Tier};

impl Run {
    /// Moves the camera along the autopilot's legs; returns `Flow::Exit` once it has printed
    /// its readings.
    pub(crate) fn pilot_step(&mut self) -> Flow {
        let (Some(p), Some(stream), Some(gfx)) =
            (self.pilot.as_mut(), self.stream.as_ref(), self.gfx.as_ref())
        else {
            return Flow::Continue;
        };
        let elapsed = (now() - p.leg_start).as_secs_f64();
        match p.leg {
            Leg::Settle => {
                if !stream.full_residency {
                    let st = stream.stats.unwrap_or_else(|| stream.streamer.stats());
                    let pending =
                        (st.in_flight + st.queued) as u64 + gfx.mirror.pending_chunks() as u64;
                    return p.settle.step(false, pending, &self.failure);
                }
                let o = centre_fixed(self.fly.camera.position);
                let op = EditOp::tool_along_ray(
                    &self.world,
                    o,
                    [FIXED_ONE, 0, 0],
                    PILOT_REACH,
                    PILOT_RADIUS,
                    Mode::Carve,
                );
                match self.world.apply(&op) {
                    Ok(r) => {
                        println!(
                            "autopilot: {}",
                            edit_line("carved", &r, self.world.materials(), 0.0)
                        );
                        p.touched = r.touched.clone();
                        if let (Some(gfx), Some((min, max))) = (self.gfx.as_mut(), r.aabb) {
                            gfx.frame.invalidate(min.0, max.0);
                        }
                    }
                    Err(e) => println!("autopilot: carve refused, {e}"),
                }
                if p.touched.is_empty() {
                    println!("autopilot: nothing carved within 30 m along +x; no return check");
                }
                p.hash = chunks_hash(&self.world, &p.touched).unwrap_or(0);
                p.origin = self.fly.camera.position;
                p.leg = Leg::Out;
                p.leg_start = now();
                p.latency_from = stream.latency_ms.len();
                p.generated_at_takeoff = stream
                    .stats
                    .unwrap_or_else(|| stream.streamer.stats())
                    .generated_chunks;
                println!("autopilot: flying +x at 10 m/s for 30 s");
            }
            Leg::Out | Leg::Back => {
                let s = elapsed.min(PILOT_LEG_SECS) as f32;
                let along = if p.leg == Leg::Out {
                    s
                } else {
                    PILOT_LEG_SECS as f32 - s
                };
                self.fly.camera.position = p.origin + Vec3::X * (PILOT_SPEED * along);
                self.fly.camera.yaw = if p.leg == Leg::Out {
                    0.0
                } else {
                    std::f32::consts::PI
                };
                self.fly.camera.pitch = 0.0;
                if elapsed >= PILOT_LEG_SECS {
                    if p.leg == Leg::Out {
                        p.leg = Leg::Back;
                        println!("autopilot: flying back for 30 s");
                    } else {
                        self.fly.camera.yaw = 0.0;
                        p.leg = Leg::Return;
                        println!("autopilot: back at the dig; waiting for its chunks");
                    }
                    p.leg_start = now();
                }
            }
            Leg::Return => {
                let back = p.touched.iter().all(|&c| {
                    stream.streamer.tier(c) == Some(Tier::Full)
                        && gfx.mirror.is_mirrored(c)
                        && !gfx.mirror.is_lod(c)
                });
                if back {
                    let hash = chunks_hash(&self.world, &p.touched);
                    let verdict = if hash == Some(p.hash) {
                        "hash match"
                    } else {
                        "hash MISMATCH"
                    };
                    println!("return: {} chunks, {verdict}", p.touched.len());
                    self.pilot_report();
                    return Flow::Exit;
                }
            }
        }
        Flow::Continue
    }

    /// Per-frame flight readings: unready-near count, streaming CPU, memory peaks.
    pub(crate) fn pilot_sample(&mut self) {
        let in_flight = self
            .pilot
            .as_ref()
            .is_some_and(|p| matches!(p.leg, Leg::Out | Leg::Back));
        if !in_flight {
            return;
        }
        let resident = if self.frame.is_multiple_of(60) {
            Some(self.resident_ram())
        } else {
            None
        };
        let now_ms = ms_since(self.start);
        let queue_ms = self.queue.last_step_cpu;
        let (Some(p), Some(stream), Some(gfx)) =
            (self.pilot.as_mut(), self.stream.as_ref(), self.gfx.as_ref())
        else {
            return;
        };
        p.flight_frames += 1;
        p.flight_span_ms = Some(
            p.flight_span_ms
                .map_or((now_ms, now_ms), |(a, _)| (a, now_ms)),
        );
        let cam = voxel_of(self.fly.camera.position).chunk();
        p.near.follow(stream.streamer.config().residency.r_full);
        let unready = p.near.unready(cam, |c| {
            stream.streamer.tier(c) == Some(Tier::Full)
                && gfx.mirror.is_mirrored(c)
                && !gfx.mirror.is_lod(c)
        });
        p.unready_frames += u64::from(unready);
        if let Some(&cpu) = stream.cpu_ms.last() {
            p.cpu_ms.push(cpu);
            for (v, &ms) in p.stages_ms.iter_mut().zip(&stream.stages_ms) {
                v.push(ms);
            }
            p.light_ms.push(stream.light_ms);
            p.queue_ms.push(queue_ms);
            p.light_evicted += stream.light_evicted;
            p.light_refused += stream.light_refused;
            p.light_wait_ms = p.light_wait_ms.max(stream.light_wait_ms);
            p.peak_light = p.peak_light.max(gfx.mirror.light_samples());
        }
        p.peak_mirror = p.peak_mirror.max(gfx.mirror.mirror_bytes());
        p.peak_allocated = p.peak_allocated.max(gfx.mirror.allocated_bytes());
        p.peak_pressure = p.peak_pressure.max(gfx.mirror.pressure());
        p.peak_belt = p.peak_belt.max(gfx.mirror.belt_chunks());
        let terms = pressure_terms(&gfx.mirror);
        p.peak_terms = [0, 1].map(|k| p.peak_terms[k].max(terms[k]));
        p.peak_persisted = p.peak_persisted.max(
            stream
                .stats
                .unwrap_or_else(|| stream.streamer.stats())
                .persisted_ram_bytes,
        );
        if let Some(r) = resident {
            p.peak_resident = p.peak_resident.max(r);
        }
    }

    pub(crate) fn pilot_report(&mut self) {
        let resident = self.resident_ram();
        let (Some(p), Some(stream), Some(gfx)) =
            (self.pilot.as_mut(), self.stream.as_mut(), self.gfx.as_ref())
        else {
            return;
        };
        let r = self.args.residency;
        // The frame bar's credit: the flight's frames had the field pass's GPU time not
        // been spent, `F_d* = F_d T / (T - G)`, `T` the first to last flight frame and `G` the
        // pass's GPU milliseconds over the same frames.
        let t = p.flight_span_ms.map_or(0.0, |(a, b)| b - a);
        let g: f64 = p.field.gpu_ms.iter().sum();
        let f = p.flight_frames as f64;
        let credited = if t > g { f * t / (t - g) } else { f };
        println!(
            "autopilot: frame bar: T {t:.1} ms, G {g:.1} ms, F_d {} frames, F_d* {credited:.1}",
            p.flight_frames
        );
        println!(
            "autopilot: unready-near frames {} of {} (chunks wanted full within r_full - 2 of the camera's, {} at the end; {} ring recomputes)",
            p.unready_frames,
            p.flight_frames,
            p.near.r_full - 2,
            p.near.recomputes
        );
        let mut cpu = p.cpu_ms.clone();
        println!(
            "autopilot: streaming CPU per frame (update + poll + sync): p50 {:.3} ms, p99 {:.3} ms, max {:.3} ms over {} frames (line 1.0 ms)",
            percentile(&mut cpu, 50.0),
            percentile(&mut cpu, 99.0),
            percentile(&mut cpu, 100.0),
            cpu.len()
        );
        for (name, v) in ["update", "poll", "apply_events", "sync"]
            .iter()
            .zip(&mut p.stages_ms)
        {
            println!(
                "autopilot: streaming stage {name}: p50 {:.3} ms, p99 {:.3} ms, max {:.3} ms",
                percentile(v, 50.0),
                percentile(v, 99.0),
                percentile(v, 100.0)
            );
        }
        println!(
            "autopilot: light extraction in sync: p50 {:.3} ms, p99 {:.3} ms, max {:.3} ms over {} frames; light samples peak {}, now {}; ranges evicted {}, refused {}; longest wait for a range inside the radius {:.0} ms",
            percentile(&mut p.light_ms, 50.0),
            percentile(&mut p.light_ms, 99.0),
            percentile(&mut p.light_ms, 100.0),
            p.light_ms.len(),
            p.peak_light,
            gfx.mirror.light_samples(),
            p.light_evicted,
            p.light_refused,
            p.light_wait_ms
        );
        println!(
            "autopilot: bake queue step CPU per frame: p50 {:.3} ms, p99 {:.3} ms, max {:.3} ms over {} frames",
            percentile(&mut p.queue_ms, 50.0),
            percentile(&mut p.queue_ms, 99.0),
            percentile(&mut p.queue_ms, 100.0),
            p.queue_ms.len()
        );
        if let Some(st) = self.queue.stats {
            println!(
                "autopilot: the workers' face masks per chunk: mean {:.1} us, max {:.1} us over {} chunks since the start",
                st.hook_ns as f64 / st.hook_chunks.max(1) as f64 / 1e3,
                st.hook_max_ns as f64 / 1e3,
                st.hook_chunks
            );
        }
        let mut lat = stream.latency_ms[p.latency_from..].to_vec();
        println!(
            "autopilot: residency latency wanted to mirrored: p95 {:.0} ms, max {:.0} ms over {} chunks (proposal 500 ms)",
            percentile(&mut lat, 95.0),
            percentile(&mut lat, 100.0),
            lat.len()
        );
        println!(
            "autopilot: {}; peak {} chunks over the flight",
            belt_line(&gfx.mirror),
            p.peak_belt
        );
        let st = stream.streamer.stats();
        println!(
            "autopilot: peak mirror {} B live, {} B allocated, resident RAM peak {} B (now {} B), process peak RSS {} (dev reference), persisted RAM peak {} B",
            p.peak_mirror,
            p.peak_allocated,
            p.peak_resident.max(resident),
            resident,
            peak_rss_bytes().map_or_else(|| "n/a".to_owned(), |b| format!("{b} B")),
            p.peak_persisted.max(st.persisted_ram_bytes)
        );
        println!(
            "autopilot: mirror budget {} B ({} units, {} records), peak live / budget {:.4}, peak pressure {:.4} (units {:.4}, records {:.4}), {} radius changes (r_full {} of {}), {} syncs with a deferral",
            gfx.mirror.allocated_bytes(),
            gfx.mirror.reserved_units(),
            gfx.mirror.reserved_records(),
            p.peak_mirror as f64 / gfx.mirror.allocated_bytes() as f64,
            p.peak_pressure,
            p.peak_terms[0],
            p.peak_terms[1],
            stream.radius_changes,
            stream.rule.r_full,
            r.r_full,
            stream.deferrals.syncs
        );
        if stream.deferrals.syncs > 0 {
            let f = stream.deferrals;
            println!(
                "autopilot: deferrals: {} chunks over {} syncs, {} of them empty; pressure {:.4} to {:.4} (slots {:.4} to {:.4}, records {:.4} to {:.4}); longest streak {} syncs ({} open at the end)",
                f.chunks,
                f.syncs,
                f.empty,
                f.low[0],
                f.high[0],
                f.low[1],
                f.high[1],
                f.low[2],
                f.high[2],
                f.longest,
                f.streak
            );
        }
        pilot_gpu_lines(p, gfx);
        let generated = st.generated_chunks - p.generated_at_takeoff;
        let secs = 2.0 * PILOT_LEG_SECS + (now() - p.leg_start).as_secs_f64();
        println!(
            "autopilot: generated {generated} chunks in about {secs:.1} s on {} workers: {:.3e} voxels/s/worker",
            stream.workers,
            generated as f64 * 262_144.0 / secs / stream.workers as f64
        );
    }
}

/// Prints the autopilot's GPU readings over the flight: the march's time, each pass's, the
/// frame targets' bytes and the probe volume's line, and the field pass's lines when it ran.
fn pilot_gpu_lines(p: &mut Autopilot, gfx: &Gfx) {
    if p.gpu_ms.is_empty() {
        println!("autopilot: march GPU n/a (no TIMESTAMP_QUERY)");
    } else {
        let [w, h] = [gfx.frame.target().width(), gfx.frame.target().height()];
        println!(
            "autopilot: march GPU at {w}x{h} over the flight: p50 {:.3} ms, p99 {:.3} ms, max {:.3} ms over {} frames",
            percentile(&mut p.gpu_ms, 50.0),
            percentile(&mut p.gpu_ms, 99.0),
            percentile(&mut p.gpu_ms, 100.0),
            p.gpu_ms.len()
        );
        for (name, v) in FRAME_PASSES.iter().zip(&mut p.pass_ms) {
            println!(
                "autopilot: pass {name}: p50 {:.3} ms, p99 {:.3} ms",
                percentile(v, 50.0),
                percentile(v, 99.0)
            );
        }
    }
    let [iw, ih] = gfx.frame.internal_size();
    println!(
        "autopilot: frame targets {} B at {}x{} from {iw}x{ih} ({} lighting)",
        gfx.frame.frame_bytes(),
        gfx.frame.target().width(),
        gfx.frame.target().height(),
        gfx.frame.resolution().name()
    );
    println!("autopilot: {}", probe_bytes_line(gfx.frame.probes()));
    if !p.field.items.is_empty() {
        for line in p.field.lines() {
            println!("{line}");
        }
    }
}
