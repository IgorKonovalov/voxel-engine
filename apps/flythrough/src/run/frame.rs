//! The run's per-frame update: the frame, the stream step, the audio step and the draw.

use crate::app::{diag, ms_since};
use crate::audio::{METRES_PER_VOXEL, MIC_VOICE, room_line, room_sent_line};
use crate::autopilot::Leg;
use crate::edits::voxel_of;
use crate::hud::{self};
use crate::now;
use crate::path::{TesterAction, TesterInput, tester_action};
use crate::run::Run;
use crate::run::fade::{FADE_DIGS, FADE_END, FadeSample, fade_lamp, fade_report};
use crate::run::hud_feed::{
    SAMPLES, Samples, band_census_line, belt_line, bins_line, crack_shares, field_line,
    print_moving_exposure, probe_passes_ms, shadow_steps_line, steps_line, window_line,
};
use crate::run::keys::{ORE_PULSE_MEAN, ore_pulse};
use crate::setup::GfxSample;
use crate::streaming::pressure_terms;
use std::time::Instant;
use vxe_app::{Flow, Gpu, Input, KeyCode, MouseButton, PadButton};
use vxe_audio::Command;
use vxe_render::debug::StepHistogram;
use vxe_render::{HudColour, LodParams, MarchTimer, PassTimer, ViewMode, wgpu};

impl Run {
    /// One frame of streaming: update, poll, apply events and sync into the frame's `enc`,
    /// timed together.
    pub(crate) fn stream_step(
        &mut self,
        gpu: &Gpu,
        enc: &mut wgpu::CommandEncoder,
    ) -> Result<(), String> {
        let (Some(stream), Some(gfx)) = (self.stream.as_mut(), self.gfx.as_mut()) else {
            return Ok(());
        };
        let cam = voxel_of(self.fly.camera.position);
        let t = now();
        stream.streamer.update(&self.world, &[cam]);
        let update_ms = ms_since(t);
        let tp = now();
        let ev = stream.streamer.poll(&self.world);
        let poll_ms = ms_since(tp);
        let ta = now();
        gfx.mirror.apply_events(&ev);
        gfx.frame.take_prepared(&ev.prepared, &self.world);
        gfx.mirror.set_focus(cam);
        let apply_ms = ms_since(ta);
        let ts = now();
        let synced = gfx.mirror.sync(&self.world, &gpu.queue, enc);
        let sync_ms = ms_since(ts);
        let cpu = ms_since(t);
        stream.stages_ms = [update_ms, poll_ms, apply_ms, sync_ms];
        stream.tick_ms.push(ms_since(self.start));
        stream.cpu_ms.push(cpu);
        for e in &ev.errors {
            eprintln!("stream: {e:?}");
        }
        // A sync writes the nearest chunks that fit and defers the rest, which stay queued; an
        // empty sync (nothing fit) is an error that wrote nothing. The pressure rule below
        // lowers the radius until they fit.
        let tick = stream.streamer.ticks();
        stream.last_sync = synced.ok();
        if let Some(s) = &stream.last_sync {
            stream.sync_line.push(s);
            self.hud.run.sync.push(s);
            if let (Some(rec), Some(p)) = (self.recorder.as_mut(), self.player.as_ref()) {
                rec.push_sync(p.clock_s, s);
            }
        }
        // The `sync:` line: the syncs' stages since the last, once a second of wall time in a
        // window or once every 60 frames headless, as the queue's lines.
        let t_ms = ms_since(self.start);
        let due = if self.args.headless {
            self.frame >= stream.sync_line_frame + 60
        } else {
            t_ms - stream.sync_line_ms >= 1000.0
        };
        if due {
            if let Some(line) = stream.sync_line.line() {
                println!("sync: at {:.1} s, frame {}, {line}", t_ms / 1e3, self.frame);
            }
            stream.sync_line.clear();
            stream.sync_line_ms = t_ms;
            stream.sync_line_frame = self.frame;
        }
        let (deferred, empty, error) = match synced {
            Ok(s) => {
                self.hud.sync = (s, sync_ms);
                stream.light_ms = s.light_us as f64 / 1e3;
                stream.light_evicted = s.light_evicted;
                stream.light_refused = s.light_refused;
                // The oldest wait in syncs, one sync per tick, as the time since that tick.
                let n = stream.tick_ms.len();
                let w = s.light_oldest_wait as usize;
                stream.light_wait_ms = if w > 0 && w < n {
                    stream.tick_ms[n - 1] - stream.tick_ms[n - 1 - w]
                } else {
                    0.0
                };
                (s.deferred_chunks, false, None)
            }
            Err(e) => (gfx.mirror.pending_chunks() as u64, true, Some(e)),
        };
        let m = &gfx.mirror;
        let [slots, records] = pressure_terms(m);
        if (deferred > 0 || empty) && stream.deferrals.syncs == 0 {
            let what = error.map_or_else(|| "deferred chunks".to_owned(), |e| e.to_string());
            eprintln!(
                "mirror: {deferred} chunks wait for space ({what}) at tick {tick} (pressure {:.4}, units {slots:.4}, records {records:.4}; {} spare slots, {} spare units, {} free pages, {} pages issued, r_full {})",
                m.pressure(),
                m.spare_slots(),
                m.spare_units(),
                m.free_pages().len(),
                m.pages_issued(),
                stream.rule.r_full
            );
        }
        let ended =
            stream
                .deferrals
                .note(deferred, empty, [f64::from(m.pressure()), slots, records]);
        if let Some(n) = ended {
            println!("mirror: deferral streak of {n} syncs ended at tick {tick}");
        }
        let pressure = gfx.mirror.pressure();
        if let Some(r_full) = stream.rule.step(pressure) {
            let r = stream.streamer.config().residency;
            stream.streamer.set_radii(r_full, r.r_lod);
            gfx.frame.set_lod(LodParams {
                r_full,
                forced_level: None,
            });
            stream.radius_changes += 1;
            println!(
                "radius: r_full {} -> {r_full} at mirror pressure {pressure:.3} after {:.0} ms (tick {tick})",
                r.r_full,
                ms_since(self.start)
            );
        }
        stream.track(&ev, &gfx.mirror);
        let st = stream.streamer.stats();
        stream.stats = Some(st);
        if !stream.full_residency
            && st.in_flight == 0
            && st.queued == 0
            && gfx.mirror.pending_chunks() == 0
        {
            stream.full_residency = true;
            println!(
                "full residency after {:.0} ms: {} full, {} LOD-only chunks, mirror {} B live, {} B allocated, pressure {:.3}, r_full {}",
                ms_since(self.start),
                st.resident_full,
                st.resident_lod,
                gfx.mirror.mirror_bytes(),
                gfx.mirror.allocated_bytes(),
                gfx.mirror.pressure(),
                stream.rule.r_full
            );
            println!("{}", bins_line(&gfx.mirror));
            if gfx.mirror.field_surface() {
                println!("{}", field_line(&gfx.mirror));
            }
            if self.args.look.band_census {
                println!("{}", band_census_line(&self.world, &gfx.mirror));
            }
        }
        Ok(())
    }

    pub(crate) fn frame(
        &mut self,
        gpu: &Gpu,
        input: &Input,
        dt: f32,
        surface: &wgpu::TextureView,
    ) -> Flow {
        let t0 = now();
        // A path run ends through its player, so its report can say how.
        if input.pressed(KeyCode::Escape) && self.player.is_none() {
            return Flow::Exit;
        }
        // The first frame's delta spans startup, not a frame.
        if self.frame > 0 {
            self.hud.frames.record(dt);
            self.hud.run.wall_ms.push(f64::from(dt) * 1e3);
        }
        if self.args.tester {
            if input.pressed(KeyCode::F1)
                && tester_action(TesterInput::Key(KeyCode::F1)) == Some(TesterAction::Help)
            {
                self.help = !self.help;
            }
        } else if input.pressed(KeyCode::F2) || input.pad_pressed(PadButton::Start) {
            self.hud.visible = !self.hud.visible;
        }
        if self.frame == 0 {
            for _ in 0..self.args.bench {
                self.bench();
            }
        }
        if self.args.light.fade_reading && self.frame < FADE_DIGS {
            self.dig();
        }
        if self.args.dig_at.contains(&self.frame) {
            self.dig();
        }
        if self.args.bench_at.contains(&self.frame) {
            self.bench();
        }
        let driven = match self.path_step(input, dt) {
            Some(Flow::Exit) => return Flow::Exit,
            Some(Flow::Continue) => true,
            None => false,
        };
        if driven {
            self.modes
                .cutaway
                .update(&mut self.modes.look, &self.world, self.fly.camera.position);
        } else if self.pilot.is_some() {
            if self.pilot_step() == Flow::Exit {
                return Flow::Exit;
            }
        } else {
            match self.walk.as_mut() {
                Some(w) => {
                    for line in w.update(&self.world, input, &mut self.fly.camera, dt) {
                        println!("{line}");
                    }
                }
                None => self.fly.update(input, dt),
            }
            if let Some(p) = &self.modes.pan {
                self.fly.camera.yaw += p.yaw(dt);
            }
            if self.args.tester {
                // Free flight under `--tester`: the dig is the one action past moving.
                if input.clicked(MouseButton::Left)
                    && tester_action(TesterInput::Click(MouseButton::Left))
                        == Some(TesterAction::Dig)
                {
                    self.dig();
                }
            } else {
                self.handle_input(input);
            }
            self.modes
                .cutaway
                .update(&mut self.modes.look, &self.world, self.fly.camera.position);
        }
        self.audio_step();
        if let Some(n) = self.args.frames
            && self.frame + SAMPLES as u32 == n
        {
            self.samples = Some(Samples::default());
        }
        self.draw(gpu, surface, t0, dt)
    }

    /// Sends the camera to the mixer as its listener and probes the voices' occlusion.
    pub(crate) fn audio_step(&mut self) {
        let listener = crate::listener(&self.fly.camera);
        let Some(audio) = self.audio.as_mut() else {
            if let Some(p) = self.room_probe.as_mut() {
                let mut completed = false;
                p.acoustics.update(
                    &self.world,
                    &p.table,
                    &listener,
                    &[],
                    METRES_PER_VOXEL,
                    &mut |cmd| completed |= matches!(cmd, Command::SetRoom(_)),
                );
                if completed && let Some(r) = p.acoustics.room() {
                    println!("{}", room_line(&r));
                    if p.mix.take_room(r.mean_free_path_m) {
                        println!("{}", p.mix.line());
                    }
                }
            }
            return;
        };
        // A full channel drops this frame's listener; the next frame sends a fresh one.
        let _ = audio.output.set_listener(listener);
        let errors = audio.output.stream_errors();
        if errors > audio.stream_errors {
            println!("audio: {errors} stream errors so far (an underrun is heard as a click)");
            audio.stream_errors = errors;
        }
        if ms_since(audio.stats_at) >= 1000.0 {
            let s = audio.output.take_stats();
            audio.callback_max_ms = s.busy_max_ms;
            audio.stats_at = now();
            if audio.debug {
                println!(
                    "audio-debug: {} callbacks of {}..{} frames, busy max {:.3} ms, gap max {:.2} ms, {} late, peak {:.3}, {} clipped, kink max {:.2e}, {} voices",
                    s.callbacks,
                    s.frames_min,
                    s.frames_max,
                    s.busy_max_ms,
                    s.gap_max_ms,
                    s.late,
                    s.peak,
                    s.clipped,
                    s.kink_max,
                    audio.voices.len()
                );
            }
        }
        for id in audio.output.finished() {
            audio.voices.retain(|(v, _)| *v != id);
        }
        if let Some(mic) = audio.mic.as_mut() {
            // The microphone's voice stands at the listener.
            let _ = audio.output.set_position(MIC_VOICE, listener.position);
            if let Some(v) = audio.voices.iter_mut().find(|(v, _)| *v == MIC_VOICE) {
                v.1 = listener.position;
            }
            let period = mic.input.period_frames();
            if !mic.period_printed && period > 0 {
                println!(
                    "mic: period {period} frames ({:.1} ms)",
                    period as f32 * 1e3 / vxe_audio::RATE as f32
                );
                mic.period_printed = true;
            }
            let errors = mic.input.stream_errors();
            if errors > mic.stream_errors {
                println!("mic: {errors} stream errors so far");
                mic.stream_errors = errors;
            }
        }
        let output = &audio.output;
        let t = now();
        let mut completed = false;
        let mut sent_mfp = None;
        let debug = audio.debug;
        let discarded = audio.acoustics.discarded_fans();
        audio.acoustics.update(
            &self.world,
            &audio.table,
            &listener,
            &audio.voices,
            METRES_PER_VOXEL,
            // A full channel drops these targets; the voice is probed again within 500 ms.
            &mut |cmd| {
                if let Command::SetRoom(r) = &cmd {
                    completed = true;
                    sent_mfp = Some(r.mean_free_path_m);
                    if debug {
                        println!("{}", room_sent_line(r));
                    }
                }
                let _ = output.send(cmd);
            },
        );
        audio.update_ms = ms_since(t);
        if let Some(mfp) = sent_mfp
            && audio.mix.take_room(mfp)
        {
            println!("{}", audio.mix.line());
        }
        let now_discarded = audio.acoustics.discarded_fans();
        if debug {
            if now_discarded > discarded && audio.discarded_run == 0 {
                println!("room: fan discarded, the listener is inside a solid");
            }
            if now_discarded > discarded {
                audio.discarded_run += now_discarded - discarded;
            } else if audio.discarded_run > 0 {
                println!(
                    "room: out of the solid after {} discarded fans",
                    audio.discarded_run
                );
                audio.discarded_run = 0;
            }
        }
        if completed
            && (audio.room_line || audio.debug)
            && let Some(r) = audio.acoustics.room()
        {
            println!("{}", room_line(&r));
        }
    }

    pub(crate) fn draw(
        &mut self,
        gpu: &Gpu,
        surface: &wgpu::TextureView,
        t0: Instant,
        dt: f32,
    ) -> Flow {
        let (mut enc, stream_ms, hud) = match self.prepare_frame(gpu, dt) {
            Ok(v) => v,
            Err(flow) => return flow,
        };
        let flying = self
            .pilot
            .as_ref()
            .is_some_and(|p| matches!(p.leg, Leg::Out | Leg::Back));
        if self.gfx.is_none() {
            return Flow::Exit;
        }
        self.configure_frame(gpu, dt, hud);
        let Some(gfx) = self.gfx.as_mut() else {
            return Flow::Exit;
        };
        let params = gfx.frame.params(&gfx.mirror, &self.fly.camera, &self.light);
        let timer = gfx.timer.as_ref().filter(|_| {
            self.samples.is_some()
                || flying
                || hud
                || self.recorder.is_some()
                || self.args.light.lamp_toggle.is_some()
                || self.args.light.fade_reading
        });
        let hud_timer = gfx.hud_timer.as_ref().filter(|_| hud);
        // Under the autopilot's flight the field pass is timed: its GPU time when it has band
        // slots to fill, and the render thread's CPU time recording it.
        let field_timer = gfx.field_timer.as_ref().filter(|_| flying);
        let t_field = now();
        let field_items = gfx
            .mirror
            .update_field_timed(&mut enc, field_timer.map(MarchTimer::writes));
        let field_cpu_ms = ms_since(t_field);
        // Read on every flying frame: a resolve finishes a frame or two after it was recorded.
        let field_reader = field_timer;
        let field_timer = field_timer.filter(|_| field_items > 0);
        if let Some(t) = field_timer {
            t.resolve(&mut enc);
        }
        gfx.frame.encode(
            &gpu.device,
            &gpu.queue,
            &mut enc,
            &gfx.mirror,
            &params,
            timer,
        );
        if let Some(t) = timer {
            t.resolve(&mut enc);
        }
        gfx.frame.copy_store_stats(&mut enc);
        gfx.frame.advance();
        gfx.presenter
            .encode(&gpu.device, &mut enc, gfx.frame.target(), surface);
        // The overlay draws on every frame: the stats HUD, or with it off the banner, whose
        // live queue row always shows.
        {
            let [w, h] = gpu.size;
            gfx.hud.encode_timed(
                &mut enc,
                surface,
                w,
                h,
                hud_timer.map(PassTimer::render_writes),
            );
            if let Some(t) = hud_timer {
                t.resolve(&mut enc);
            }
        }
        gpu.queue.submit([enc.finish()]);
        if self.args.headless {
            diag(format_args!("frame {}: submitted", self.frame));
        }
        let cpu_ms = ms_since(t0);
        self.hud.exposure = gfx.frame.exposure_readout(&gpu.device);
        self.modes.set_readout(self.hud.exposure);
        print_moving_exposure(
            &mut self.exposure_print,
            &self.modes,
            self.hud.exposure,
            ms_since(self.start),
        );

        if self.frame == 0 {
            wait_first_frame(gpu, self.start);
        }
        // The timers hand over their newest finished set without waiting (a frame or two old),
        // `None` on a frame with nothing new. `--lamp-toggle` and `--fade-reading` file each set
        // under the frame that drew it, so they wait for this frame's.
        let exact = self.args.light.lamp_toggle.is_some() || self.args.light.fade_reading;
        let pass_ms = timer.and_then(|t| {
            if exact {
                Some(t.read_all_ms(&gpu.device))
            } else {
                t.try_read_all_ms(&gpu.device)
            }
        });
        let gpu_ms = pass_ms.as_ref().map(|p| p.iter().sum::<f64>());
        let field_gpu_ms = field_reader.and_then(|t| t.try_read_ms(&gpu.device));
        let hud_ms = hud_timer.and_then(|t| t.try_read_ms(&gpu.device));
        self.hud.run.push(cpu_ms, stream_ms, gpu_ms);
        if self.frame > 0
            && let (Some(rec), Some(p)) = (self.recorder.as_mut(), self.player.as_ref())
        {
            rec.push(p.clock_s, f64::from(dt) * 1e3, cpu_ms, gpu_ms);
        }
        self.hud.cpu_ms = cpu_ms as f32;
        // The HUD keeps the last set read until a newer one finishes, and shows none once its
        // timer is off.
        if timer.is_none() {
            self.hud.march_ms = None;
            self.hud.pass_ms = None;
        } else if pass_ms.is_some() {
            self.hud.march_ms = gpu_ms.map(|v| v as f32);
            self.hud.pass_ms.clone_from(&pass_ms);
        }
        if self.args.light.lamp_toggle.is_some()
            && let Some(p) = &pass_ms
        {
            let window = usize::from(gfx.frame.probe_light_window());
            self.window_ms[window].push(probe_passes_ms(p));
        }
        if self.args.light.fade_reading && self.frame >= FADE_DIGS {
            self.fade
                .push(FadeSample::read(gpu, &gfx.frame, gpu_ms.unwrap_or(0.0)));
        }
        if hud_timer.is_none() {
            self.hud.hud_ms = None;
        } else if let Some(ms) = hud_ms {
            self.hud.hud_ms = Some(ms as f32);
        }
        if flying && let (Some(p), Some(ms)) = (self.pilot.as_mut(), gpu_ms) {
            p.gpu_ms.push(ms);
            if let Some(passes) = &pass_ms {
                p.pass_ms.resize(passes.len(), Vec::new());
                for (v, &ms) in p.pass_ms.iter_mut().zip(passes) {
                    v.push(ms);
                }
            }
        }
        if flying
            && gfx.mirror.field_surface()
            && let Some(p) = self.pilot.as_mut()
        {
            p.field.push(field_items, field_cpu_ms, field_gpu_ms);
        }
        self.push_samples(gpu, cpu_ms, gpu_ms, pass_ms.as_deref(), hud_ms);
        self.frame += 1;
        if self.args.light.fade_reading && self.frame >= FADE_END {
            for line in fade_report(&self.fade) {
                println!("{line}");
            }
            return Flow::Exit;
        }
        if self.args.frames.is_some_and(|n| self.frame >= n) {
            self.print_exit_readings(gpu);
            return Flow::Exit;
        }
        Flow::Continue
    }

    /// The frame's work before the encode: the tick, the glide, the mirror's sync (streamed or
    /// bounded) recorded into a new encoder, the live queue's step, the autopilot's sample, the
    /// HUD's samples and rows (or the banner alone), and the lamp's scripted presses. Gives the
    /// encoder, the sync's CPU time and whether the stats HUD shows; a failed sync gives the
    /// `Flow` that `mirror_failed` ends the session with.
    fn prepare_frame(
        &mut self,
        gpu: &Gpu,
        dt: f32,
    ) -> Result<(wgpu::CommandEncoder, Option<f64>, bool), Flow> {
        self.tick(gpu);
        if let Some(v) = self.args.glide {
            let step = if self.args.headless {
                1.0 / 60.0
            } else {
                dt.clamp(0.0, 0.1)
            };
            let (forward, _, _) = self.fly.camera.basis();
            self.fly.camera.position += forward * (v * step);
        }
        // The sync's copies are recorded first, so the field pass and the march read
        // them in the same submission.
        let mut enc = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let mut stream_ms = None;
        if self.stream.is_some() {
            if let Err(e) = self.stream_step(gpu, &mut enc) {
                return Err(self.mirror_failed(e));
            }
            stream_ms = self.stream.as_ref().and_then(|s| s.cpu_ms.last().copied());
        } else if let Some(gfx) = self.gfx.as_mut() {
            // The light radius follows the camera on the bounded level too.
            gfx.mirror.set_focus(voxel_of(self.fly.camera.position));
            let t = now();
            match gfx.mirror.sync(&self.world, &gpu.queue, &mut enc) {
                Ok(s) => {
                    let ms = ms_since(t);
                    self.hud.sync = (s, ms);
                    stream_ms = Some(ms);
                }
                Err(e) => return Err(self.mirror_failed(e)),
            }
        }
        let enc = self.step_bake_queue(gpu, enc);
        self.pilot_sample();
        let hud = self.hud.visible;
        if self.hud.cadence.due(hud) {
            self.sample_world();
            self.sample_probes(gpu);
        }
        // The steps row reads the last frame's target back before this frame's march
        // overwrites it: a blocking copy, paid only with the HUD and that view both on.
        self.hud.steps = None;
        if hud
            && self.modes.look.view == ViewMode::Steps
            && let Some(gfx) = &self.gfx
        {
            let px = vxe_render::debug::read_rgba8(&gpu.device, &gpu.queue, gfx.frame.target());
            let width = gfx.frame.target().width() as usize;
            let h = StepHistogram::from_rgba8(&px, width);
            self.hud.steps = Some(hud::StepsSample::from_histogram(&h));
        }
        self.hud.cracks = None;
        if hud
            && self.modes.look.view == ViewMode::Cracks
            && let Some(gfx) = &self.gfx
        {
            self.hud.cracks = Some(crack_shares(gpu, &gfx.frame));
        }
        // With the stats HUD off the overlay draws the mode banner alone while it shows.
        if !hud {
            self.write_banner();
        }
        if hud {
            self.update_hud(gpu);
        } else if let Some(gfx) = self.gfx.as_mut() {
            gfx.hud.set_lines(
                &gpu.queue,
                &self.hud.banner,
                &[HudColour::Yellow; hud::BANNER_ROWS + 1],
            );
            gfx.hud.set_samples(&gpu.queue, &[], 0.0);
        }
        if let Some(n) = self.args.light.lamp_toggle
            && self.frame > 0
            && self.frame.is_multiple_of(n)
        {
            self.modes.press(hud::ModeKey::Lamp, ms_since(self.start));
        }
        if self.args.light.fade_reading && self.modes.lighting.lamp != fade_lamp(self.frame) {
            self.modes.press(hud::ModeKey::Lamp, ms_since(self.start));
        }
        Ok((enc, stream_ms, hud))
    }

    /// The frame's settings ahead of its `params`: an upscaler or sharpening change, the
    /// lighting modes, the exposure's step, the resolution, the look, the store's read count,
    /// the missing light and the ore's emission pulse.
    fn configure_frame(&mut self, gpu: &Gpu, dt: f32, hud: bool) {
        let Some(gfx) = self.gfx.as_mut() else {
            return;
        };
        if std::mem::take(&mut self.upscaler_changed) {
            let (kind, scale, jitter) = self.args.upscaler_settings(self.modes.upscaler.mode);
            let output = gfx.frame.output_size();
            gfx.frame.set_jitter(&gpu.device, jitter);
            gfx.frame.set_upscaler_kind(&gpu.device, kind);
            gfx.frame
                .set_sizes(&gpu.device, scale.internal(output), output);
        }
        if std::mem::take(&mut self.sharpen_changed) {
            gfx.frame.set_sharpness(&gpu.queue, self.args.look.sharpen);
            if self.modes.upscaler.mode == hud::UpscalerMode::Fsr2 {
                let (kind, _, _) = self.args.upscaler_settings(self.modes.upscaler.mode);
                gfx.frame.set_upscaler_kind(&gpu.device, kind);
            }
        }
        gfx.frame.set_lighting(self.modes.lighting);
        // Headless frames step the exposure by exactly 1/60 s; the window by its measured
        // interval, at most 0.1 s, so a hitch adapts by no more than that.
        gfx.frame.set_dt(if self.args.headless {
            1.0 / 60.0
        } else {
            dt.clamp(0.0, 0.1)
        });
        gfx.frame.set_resolution(&gpu.device, self.modes.resolution);
        gfx.frame.set_look(self.modes.look);
        // The temporal pass counts the store's reads only for a reader of them: the HUD's
        // store row and `--read-missing`.
        gfx.frame
            .set_count_store(hud || self.args.bake.read_missing);
        if gfx.frame.missing_light() != self.args.bake.missing_light {
            gfx.frame
                .set_missing_light(&gpu.queue, self.args.bake.missing_light);
        }
        let scale = if self.modes.pulse {
            ore_pulse(ms_since(self.start))
        } else {
            1.0
        };
        // The probes see the pulse at its average, so it opens no light-change window; the
        // pixels' light keeps its full swing.
        gfx.frame
            .set_emission_scales(&gpu.queue, crate::cave::ORE, scale, ORE_PULSE_MEAN);
    }

    /// `--frames`' readings at the exit: the walk's foot, the coarse grid, the cracks or steps
    /// view's line, the mirror's belt, the lamp toggle's windows and the stats HUD's rows.
    fn print_exit_readings(&mut self, gpu: &Gpu) {
        if let Some(w) = &self.walk {
            println!(
                "walk: foot at {:?} (fixed), {}",
                w.state.foot.0,
                if w.state.grounded {
                    "grounded"
                } else {
                    "airborne"
                }
            );
        }
        if self.coarse.is_some() {
            println!("{}", self.coarse_line());
        }
        if self.modes.look.view == ViewMode::Cracks
            && let Some(gfx) = &self.gfx
        {
            println!("{}", hud::cracks_row(&crack_shares(gpu, &gfx.frame)));
        }
        if self.modes.look.view == ViewMode::Steps
            && let Some(gfx) = &self.gfx
        {
            let px = vxe_render::debug::read_rgba8(&gpu.device, &gpu.queue, gfx.frame.target());
            let width = gfx.frame.target().width() as usize;
            println!("{}", steps_line(&StepHistogram::from_rgba8(&px, width)));
            let shadow =
                vxe_render::debug::read_rgba16f(&gpu.device, &gpu.queue, gfx.frame.irradiance());
            println!(
                "{}",
                shadow_steps_line(&shadow, gfx.frame.lighting().shadow_walk)
            );
        }
        if let Some(gfx) = &self.gfx {
            println!("mirror: {}", belt_line(&gfx.mirror));
        }
        if let Some(n) = self.args.light.lamp_toggle {
            println!("{}", window_line(n, &self.window_ms));
        }
        self.sample_world();
        self.sample_probes(gpu);
        self.refresh_hud_stats(self.gfx.as_ref().map(GfxSample::of));
        let mut stats = self.hud.stats.clone();
        stats.world.age_frames = 0;
        println!("hud:\n{stats}");
    }

    /// Files this frame's times into the `F1` samples, when they are being taken, and prints
    /// their report once they hold `SAMPLES` frames.
    fn push_samples(
        &mut self,
        gpu: &Gpu,
        cpu_ms: f64,
        gpu_ms: Option<f64>,
        pass_ms: Option<&[f64]>,
        hud_ms: Option<f64>,
    ) {
        if let Some(s) = self.samples.as_mut() {
            s.cpu_ms.push(cpu_ms);
            if let Some(ms) = gpu_ms {
                s.gpu_ms.push(ms);
            }
            if let Some(p) = pass_ms {
                s.pass_ms.resize(p.len(), Vec::new());
                for (v, &ms) in s.pass_ms.iter_mut().zip(p) {
                    v.push(ms);
                }
            }
            if let Some(ms) = hud_ms {
                s.hud_ms.push(ms);
            }
            if s.cpu_ms.len() == SAMPLES {
                let s = self.samples.take().unwrap_or_default();
                self.report(gpu, s);
            }
        }
    }
}

/// Waits for the GPU to finish the first frame, which counts from then: prints when, from the
/// session's `start`.
fn wait_first_frame(gpu: &Gpu, start: Instant) {
    diag(format_args!("frame 0: waiting for the GPU"));
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    diag(format_args!("frame 0: the GPU finished"));
    println!("first frame after {:.0} ms", ms_since(start));
}
