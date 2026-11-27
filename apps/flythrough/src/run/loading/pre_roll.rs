//! The loading's pre-roll stage: the live queue started on an empty store with the near chunks
//! around the start camera listed, stepped until its chunks or its box end it, and its lines.

use super::*;

/// The `first frame light:` line: the candidates with no block and with a coarse one by where
/// they lie against the view (the `bake queue view:` line's counts), read at the first opening
/// after the pre-roll, at frame `frame`.
pub(crate) fn first_light_line(st: &svet_render::QueueStats, frame: u32) -> String {
    let w = st.view_waiting;
    format!(
        "first frame light: waiting (blank/coarse) shell {}/{} in view {}/{} out {}/{}, read at frame {frame} (the first opening after the pre-roll)",
        w[0][0], w[0][1], w[1][0], w[1][1], w[2][0], w[2][1],
    )
}

impl Run {
    /// Starts the pre-roll: the live queue started on an empty store with the run's settings,
    /// and the near chunks around the start camera listed.
    pub(super) fn enter_pre_roll(&mut self, gpu: &Gpu, gfx: &mut Gfx, l: &mut Loading) -> Stage {
        // The run's own sync budget back: the streamed mirror's `--sync-ms`, none on the
        // level's, so the pre-roll streams as the unsliced start does.
        gfx.mirror.set_sync_budget_time(if self.stream.is_some() {
            self.args.sync_time
        } else {
            None
        });
        // The pre-roll's bakes read the sky and the canopy's style as the frame loop sets them,
        // so a pre-rolled block holds its sky visibility and a column's finality marks it.
        crate::run::frame::set_sky_and_styles(
            &self.modes,
            self.args.mode.is_outdoor(),
            gpu,
            &mut gfx.frame,
        );
        let frame = &mut gfx.frame;
        let settings = svet_render::queue_settings();
        frame.start_bake_queue(
            &gpu.queue,
            &gfx.mirror,
            settings,
            svet_render::DEFAULT_BAKE_SLICE_MS,
        );
        frame.set_bake_budget(self.args.bake.budget);
        frame.set_bake_coarse(self.args.bake.coarse, self.args.bake.near);
        frame.set_bake_digs(self.args.bake.edit_ms, self.args.bake.dig_reach);
        frame.set_bake_ring1_stretch(self.args.bake.ring1_stretch);
        frame.set_bake_reach(self.args.bake.reach);
        frame.set_bake_tiers(self.args.bake.tiers);
        frame.set_bake_drain(self.args.bake.drain);
        frame.share_bake_world(self.world.clone());
        frame.set_bake_thread(&gpu.queue, self.args.bake_thread());
        frame.set_bake_depth(self.args.bake.depth);
        frame.set_bake_slots(self.args.bake.slots);
        if frame.bake_queue().is_some() {
            let (order, withdraw) = (self.args.queue_order(), self.args.bake.queue_withdraw);
            let (census, rule, audit) = (
                self.args.bake.queue_census,
                self.args.bake.queue_rule_reading,
                self.args.bake.queue_rule_audit,
            );
            frame.set_bake_queue(move |q| {
                q.order = order;
                q.withdraw = withdraw;
                q.census_every_opening = census;
                q.rule_reading = rule;
                q.rule_audit = audit;
            });
            self.queue.listing_due = self.args.bake.queue_census;
        }
        println!("{}", self.queue_row());
        frame.take_prepared(&std::mem::take(&mut self.queue.prepared), &self.world);
        let camera = start_camera(&self.args, self.spawn);
        let centre = ChunkCoord(
            camera
                .position
                .to_array()
                .map(|v| (v / 64.0).floor() as i32),
        );
        let r = self.args.bake.near;
        let mode = self.args.pre_roll();
        let mut near: Vec<ChunkCoord> = (-r..=r)
            .flat_map(|z| {
                (-r..=r).flat_map(move |y| {
                    (-r..=r).map(move |x| {
                        ChunkCoord([centre.0[0] + x, centre.0[1] + y, centre.0[2] + z])
                    })
                })
            })
            .collect();
        // `--pre-roll off` holds no near chunk: the near stage is over at the first step.
        if mode == PreRollMode::Off {
            near.clear();
        }
        l.progress.enter(NEAR);
        l.progress.set(0, Some(near.len() as u64));
        let (box_ms, cap_ms) = pre_roll_limits(&self.args);
        Stage::PreRoll(Box::new(PreRollRun {
            pre: PreRoll::new(PreRollConfig {
                near,
                coarse: self.args.bake.coarse,
                box_ms,
                cap_ms,
                samples: PRE_ROLL_SAMPLES,
            }),
            camera,
            centre,
            work_ms: 0.0,
            longest_ms: 0.0,
        }))
    }

    /// One step of the pre-roll: streaming on the streamed world, then [`PreRoll::step`]. A
    /// failed sync or step records the session's failure and ends loading.
    pub(super) fn pre_roll_stage(
        &mut self,
        gpu: &Gpu,
        gfx: &mut Gfx,
        mut p: Box<PreRollRun>,
        l: &mut Loading,
    ) -> Stage {
        // `--pre-roll-stepped`: every submit of the earlier steps has finished before this
        // step polls, so the step reads the same read-backs under either driver (the frame
        // loop's slices and `load_all`'s loop); outside the step's timed span.
        if self.args.loading.pre_roll_stepped {
            let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        }
        let t = now();
        let camera = p.camera;
        let frame = &mut gfx.frame;
        let mirror = &mut gfx.mirror;
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
        let stream = self.stream.as_ref();
        let resident = stream.map(|s| svet_render::Residency::of_chunks(s.streamer.full_chunks()));
        let (forward, _, _) = camera.basis();
        let (half_angle, frustum, focal) = queue_view_angles(camera.vfov, frame.output_size());
        let view = svet_render::QueueView {
            eye: camera.position.to_array(),
            forward: forward.to_array(),
            resident,
            samples: Some(PRE_ROLL_SAMPLES),
            interval_s: 0.0,
            hold: stream.map(|s| s.rule.r_full + self.args.light.radius() + 1),
            half_angle,
            frustum: Some(frustum),
            full_radius: stream.map(|s| s.rule.r_full),
            lod_radius: stream.map(|s| {
                let r = s.streamer.config().residency;
                r.r_lod.max(r.r_full)
            }),
            focal_px: Some(focal),
        };
        let state = p.pre.step(
            frame,
            &gpu.device,
            &gpu.queue,
            mirror,
            &self.world,
            &view,
            p.work_ms,
        );
        if stream.is_some() {
            std::thread::sleep(Duration::from_millis(1));
        }
        let step_ms = ms_since(t);
        p.work_ms += step_ms;
        p.longest_ms = p.longest_ms.max(step_ms);
        let state = match state {
            Ok(s) => s,
            Err(e) => {
                self.failure.set(e.to_string());
                return Stage::Done;
            }
        };
        match state {
            PreRollState::Near { settled, of } => {
                l.progress.set(settled as u64, Some(of as u64));
                Stage::PreRoll(p)
            }
            PreRollState::Coarse { passed, of } => {
                l.progress.enter(COARSE);
                let by_count = (passed as u64 * COARSE_UNITS) / (of.max(1) as u64);
                let box_ms = p.pre.config().box_ms;
                let by_box = p.pre.near_ms().map_or(0.0, |at| {
                    if box_ms > 0.0 {
                        ((p.work_ms - at) / box_ms).clamp(0.0, 1.0)
                    } else {
                        1.0
                    }
                });
                let by_box = (by_box * COARSE_UNITS as f64) as u64;
                l.progress
                    .set(by_count.max(by_box).min(COARSE_UNITS), Some(COARSE_UNITS));
                Stage::PreRoll(p)
            }
            PreRollState::Done(s) => {
                self.end_pre_roll(frame, &p, &s);
                l.progress.enter(COARSE);
                l.progress.next();
                diag(format_args!("pre-roll done"));
                if self.args.still.settled {
                    Stage::Settle(Box::default())
                } else {
                    Stage::Field
                }
            }
        }
    }

    /// Prints the pre-roll's lines and hands the live queue its full-first radius.
    fn end_pre_roll(
        &mut self,
        frame: &mut svet_render::Frame,
        p: &PreRollRun,
        s: &svet_render::PreRollSummary,
    ) {
        let st = frame.bake_queue().map(|q| q.stats()).unwrap_or_default();
        if s.capped {
            println!("bake queue: the pre-roll did not end in 120 s; going on");
        }
        println!(
            "bake queue: the camera's chunk {:?} and the {} within {} at 4096 by {:.0} ms ({} chunks, {:.3e} samples); the coarse pass at {} by {:.0} ms (+{:.0} ms, +{} chunks, +{:.3e} samples, {} coarse{}); {} listed from the workers; {:.0} ms since the start",
            p.centre.0,
            p.pre.config().near.len(),
            self.args.bake.near,
            s.near_ms,
            s.near_baked,
            s.near_samples as f64,
            self.args.bake.coarse,
            p.work_ms,
            p.work_ms - s.near_ms,
            s.baked - s.near_baked,
            (s.samples - s.near_samples) as f64,
            st.coarse,
            if s.boxed {
                format!(
                    ", ended by its {:.0} s box with the rest of the {} chunks to the queue",
                    p.pre.config().box_ms / 1e3,
                    s.coarse_chunks
                )
            } else {
                String::new()
            },
            st.prepared,
            ms_since(self.start)
        );
        println!(
            "pre-roll: {} steps, {} chunks baked, {} samples, {:.0} ms of steps, the longest {:.1} ms",
            s.steps, s.baked, s.samples, p.work_ms, p.longest_ms
        );
        if let Some(q) = frame.bake_queue() {
            let qs = q.stats();
            println!(
                "pre-roll: column wait {}, {} jobs opened provisional, {} catch-ups queued, {} chunks provisional",
                if frame.lighting().column_wait {
                    "on"
                } else {
                    "off"
                },
                qs.provisional_opened,
                qs.provisional_requeued,
                qs.provisional
            );
        }
        // The pre-roll's near chunks take the full count under `--near`; the live queue's
        // full-first radius starts here.
        frame.set_bake_full_first(self.args.full_first());
        // The `first frame light:` line reads the census of the first opening after the
        // pre-roll, asked for here.
        self.queue.first_light = Some(st.openings);
        frame.set_bake_queue(svet_render::BakeQueue::request_census);
    }
}
