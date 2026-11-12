//! The run's edits: digs, fills, stamps and the bench, sent through the server tick, and the
//! save.

use crate::app::ms_since;
use crate::cave::{CaveSource, ROCK, vault};
use crate::edits::{
    BenchSum, FILL_RADIUS, edit_record, op_verb, removed_line, unsupported_line,
    wedged_terminal_line,
};
use crate::hud::EditLine;
use crate::logic::{BENCH_CARVES, BENCH_VERB, DIG_VERB, DigInput, REACH};
use crate::logic::{DROP_VERB, PILE_VERB};
use crate::now;
use crate::run::Run;
use crate::run::hud_feed::field_pass_line;
use vxe_app::Gpu;
use vxe_net::Event;
use vxe_render::wgpu;
use vxe_world::{EditResult, FIXED_ONE, Fixed3, LogFile, OpResult};

impl Run {
    /// Prints the edit's line and keeps it for the HUD.
    pub(crate) fn record_edit(&mut self, verb: &'static str, r: &EditResult, ms: f64) {
        println!("{}", self.note_edit(verb, r, ms));
    }

    /// Keeps the edit for the HUD's last-edit row without printing it, for an edit whose
    /// printed line has another form (the collapse's drops and removals); returns the record.
    pub(crate) fn note_edit(&mut self, verb: &'static str, r: &EditResult, ms: f64) -> &EditLine {
        self.hud
            .stats
            .last_edit
            .insert(edit_record(verb, r, self.world.materials(), ms))
    }

    /// Keeps the coarse grid and the walkable map current with `r` (nothing while
    /// streaming).
    pub(crate) fn edited(&mut self, r: &EditResult) {
        if let Some(c) = self.coarse.as_mut() {
            c.update(&self.world, r);
        }
        // The frame's history dies wherever the world changed.
        if let (Some(gfx), Some((min, max))) = (self.gfx.as_mut(), r.aabb) {
            gfx.frame.invalidate(min.0, max.0);
        }
    }

    pub(crate) fn coarse_line(&self) -> String {
        match &self.coarse {
            None => "coarse: not kept while streaming".to_owned(),
            Some(c) => c.line(),
        }
    }

    pub(crate) fn camera_ray(&self) -> (Fixed3, [i64; 3]) {
        let c = &self.fly.camera;
        let origin = c
            .position
            .to_array()
            .map(|v| (v * FIXED_ONE as f32).floor() as i64);
        let dir = c
            .basis()
            .0
            .to_array()
            .map(|v| (v * FIXED_ONE as f32).round() as i64);
        (Fixed3(origin), dir)
    }

    pub(crate) fn send(&self, input: DigInput) {
        if let Some(net) = &self.net {
            net.client.send_input(input.encode());
        }
    }

    /// Left click: a dig along the camera ray.
    pub(crate) fn dig(&mut self) {
        let (origin, dir) = self.camera_ray();
        if let Some(audio) = self.audio.as_mut() {
            let t = self
                .world
                .raycast(origin, dir, REACH)
                .ok()
                .flatten()
                .map_or(REACH, |h| h.t);
            audio.dig_point = Some(
                [0, 1, 2].map(|k| (origin.0[k] + dir[k] * t / FIXED_ONE) as f32 / FIXED_ONE as f32),
            );
        }
        self.send(DigInput::Dig {
            origin,
            dir,
            radius: self.modes.dig_radius,
        });
    }

    /// `P`: a stamp input, the vault (registered on the server by its content id) at the
    /// first solid voxel the camera ray enters.
    pub(crate) fn stamp(&mut self) {
        let (origin, dir) = self.camera_ray();
        self.send(DigInput::Stamp {
            origin,
            dir,
            prefab: vault().id(),
        });
    }

    /// Right click: a fill where the camera ray hits.
    pub(crate) fn fill(&mut self) {
        let (origin, dir) = self.camera_ray();
        self.send(DigInput::Fill {
            origin,
            dir,
            radius: FILL_RADIUS,
            material: ROCK,
        });
    }

    /// `B`: a bench input, its carves seeded from `--seed` and the count of benches so far,
    /// centred on the camera's voxel.
    pub(crate) fn bench(&mut self) {
        let origin = self
            .fly
            .camera
            .position
            .to_array()
            .map(|v| v.floor() as i64);
        let seed = self.args.seed ^ self.bench_runs.wrapping_mul(0x9e37_79b9_7f4a_7c15);
        self.bench_runs += 1;
        self.send(DigInput::Bench { origin, seed });
    }

    /// The frame's server tick, timed, then the client's events: one printed line per dig,
    /// fill, stamp, removal and bench, the logic's other lines (the unsupported regions with
    /// material names), the coarse grid updated from every applied result. A tick that
    /// finished a bench syncs the mirror at once and prints the bench line.
    pub(crate) fn tick(&mut self, gpu: &Gpu) {
        let (benches, refused, ms) = self.tick_server();
        if benches > 0 {
            self.bench_sync(gpu, benches * BENCH_CARVES, refused, ms);
        }
    }

    /// One server tick and the client's events: prints the logic's lines, keeps the coarse
    /// grid and the frame's history current with every applied op (those a logic line names
    /// and those none does, such as the ops a logic applied in a tick it then failed), and
    /// returns the benches completed, their refused carves and the tick's milliseconds.
    pub(crate) fn tick_server(&mut self) -> (usize, usize, f64) {
        let Some(net) = self.net.as_mut() else {
            return (0, 0, 0.0);
        };
        let t = now();
        let report = net.server.tick();
        let ms = ms_since(t);
        for e in [report.restart, report.logic_error].into_iter().flatten() {
            if let Some(line) = net.logic_failure(&e) {
                println!("{line}");
            }
        }
        let events = net.client.poll();

        let mut ops = std::collections::VecDeque::new();
        let mut bench = BenchSum::default();
        let mut benches = (0, 0);
        for event in events {
            let Event::Logic { bytes, .. } = event else {
                ops.push_back(event);
                continue;
            };
            for line in String::from_utf8_lossy(&bytes).lines() {
                let Some(verb) = line.strip_prefix("op ").and_then(op_verb) else {
                    match unsupported_line(self.world.materials(), line) {
                        Some(l) => {
                            if let Some(c) = self.coarse.as_mut() {
                                c.query = Some(c.query.map_or(1, |n| n + 1));
                            }
                            println!("{l}");
                        }
                        None => match wedged_terminal_line(self.world.materials(), line) {
                            Some(l) => println!("{l}"),
                            None => println!("{line}"),
                        },
                    }
                    continue;
                };
                let result = match ops.pop_front() {
                    Some(Event::Applied { result, .. }) => Ok(result),
                    Some(Event::Refused { error, .. }) => Err(error),
                    _ => continue,
                };
                if let Ok(r) = &result {
                    self.edited(&r.edit_result());
                }
                if verb == DIG_VERB
                    && result.is_ok()
                    && let Some(c) = self.coarse.as_mut()
                {
                    // The region lines that follow this dig's are what its query found.
                    c.query = Some(0);
                }
                if verb != BENCH_VERB {
                    match result {
                        // A drop happens every tick a region falls, and a pile ends a fall the
                        // logic's line reports: kept for the HUD only.
                        Ok(r) if verb == DROP_VERB || verb == PILE_VERB => {
                            if r.aabb().is_some() {
                                self.note_edit(verb, &r.edit_result(), ms);
                            }
                        }
                        // The logic's `drop refused` or `pile refused` line reports it.
                        Err(_) if verb == DROP_VERB || verb == PILE_VERB => {}
                        Ok(OpResult::Remove(r)) => {
                            println!("{}", removed_line(self.world.materials(), &r));
                            self.note_edit(verb, &r.edit_result(), ms);
                        }
                        Ok(r) => self.record_edit(verb, &r.edit_result(), ms),
                        Err(e) => println!("{verb}: refused, {e}"),
                    }
                    continue;
                }
                let result = result.map(|r| r.edit_result());
                match result {
                    Ok(r) => {
                        for (m, n) in r.changed_from {
                            *bench.counts.entry(m).or_default() += n;
                        }
                        bench.touched.extend(r.touched);
                    }
                    Err(_) => bench.refused += 1,
                }
                bench.carves += 1;
                if bench.carves == BENCH_CARVES {
                    let done = std::mem::take(&mut bench);
                    let total = EditResult {
                        changed_from: done.counts.into_iter().collect(),
                        touched: done.touched.into_iter().collect(),
                        aabb: None,
                    };
                    self.record_edit("carved", &total, ms);
                    benches = (benches.0 + 1, benches.1 + done.refused);
                }
            }
        }
        // Applied ops no logic line named still changed the world.
        for event in ops {
            if let Event::Applied { result, .. } = event {
                self.edited(&result.edit_result());
            }
        }
        (benches.0, benches.1, ms)
    }

    /// The mirror sync after a bench, timed, with the bench line; with the field's band slots,
    /// the field pass over the carves timed alone.
    pub(crate) fn bench_sync(&mut self, gpu: &Gpu, carves: usize, refused: usize, edit_ms: f64) {
        let Some(gfx) = self.gfx.as_mut() else { return };
        let t = now();
        let mut enc = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let synced = gfx.mirror.sync(&self.world, &gpu.queue, &mut enc);
        let sync_ms = ms_since(t);
        gpu.queue.submit([enc.finish()]);
        match synced {
            Ok(s) => println!(
                "bench: {carves} carves of radius 1 m within 10 m ({refused} refused): \
                 edits {edit_ms:.2} ms CPU; sync uploaded {} B ({} bricks, \
                 {} chunks, {} visited, {} writes) in {:.2} ms",
                s.bytes, s.bricks, s.chunks, s.chunks_visited, s.writes, sync_ms
            ),
            Err(e) => println!("bench: sync failed, {e}"),
        }
        if gfx.mirror.field_surface() {
            let what = format!("{carves} carves of radius 1 m");
            println!("{}", field_pass_line(&what, gfx.timed_field(gpu)));
        }
    }

    /// `--save`: the session's log file, and the world's hash.
    pub(crate) fn save(&self) {
        let (Some(path), Some(net)) = (&self.args.save, &self.net) else {
            return;
        };
        let file = LogFile {
            seed: self.args.seed,
            source_version: CaveSource::VERSION,
            extent: self.world.extent(),
            ops: net.server.ops().clone(),
            inputs: net.server.inputs().clone(),
            prefabs: net.server.prefabs().subset(net.server.ops().prefab_ids()),
        };
        println!("world hash 0x{:016x}", self.world.voxel_hash());
        let written = std::fs::File::create(path).and_then(|mut f| file.write_to(&mut f));
        match written {
            Ok(()) => println!(
                "saved {} inputs, {} ops to {}",
                file.inputs.len(),
                file.ops.len(),
                path.display()
            ),
            Err(e) => eprintln!("save to {} failed: {e}", path.display()),
        }
    }
}
