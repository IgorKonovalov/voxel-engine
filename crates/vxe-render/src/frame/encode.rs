//! The frame's encode: its uniforms, its bind groups and the passes it records.

use super::*;

/// One dispatch of a frame pass: its pipeline, the bind groups at indices 0 to 3, and the
/// workgroups in x and y.
pub(crate) struct Dispatch<'a> {
    pub pipeline: &'a wgpu::ComputePipeline,
    pub groups: [Option<&'a wgpu::BindGroup>; 4],
    pub workgroups: [u32; 2],
    /// Dispatched with the workgroup counts this buffer holds at this byte offset instead.
    pub indirect: Option<(&'a wgpu::Buffer, u64)>,
}

impl Dispatch<'_> {
    /// Sets the pipeline and the bind groups on `pass` and dispatches.
    pub(super) fn record(&self, pass: &mut wgpu::ComputePass<'_>) {
        pass.set_pipeline(self.pipeline);
        for (g, bind) in self.groups.iter().enumerate() {
            if let Some(b) = bind {
                pass.set_bind_group(g as u32, *b, &[]);
            }
        }
        match self.indirect {
            Some((args, offset)) => pass.dispatch_workgroups_indirect(args, offset),
            None => pass.dispatch_workgroups(self.workgroups[0], self.workgroups[1], 1),
        }
    }
}

/// Passes of a frame in encode order, as [`Frame::encode`] times them: pass `i` writes
/// timestamps `2 i` and `2 i + 1` of the timer it is given.
/// The nine before the last are FSR 2's ([`crate::fsr2::FSR2_PASSES`]); in FSR mode `upscale`
/// and `sharpen` are empty, and otherwise FSR's are. `exposure` runs last, after either.
pub const FRAME_PASSES: [&str; 20] = [
    "hit",
    "gradient",
    "light",
    "temporal",
    "atrous",
    "probes",
    "gather",
    "composite",
    "upscale",
    "sharpen",
    "fsr2 input",
    "fsr2 spd",
    "fsr2 spd tail",
    "fsr2 reconstruct",
    "fsr2 depth clip",
    "fsr2 lock",
    "fsr2 accumulate",
    "fsr2 rcas",
    "fsr2 tone",
    "exposure",
];

impl Frame {
    pub(super) fn world_bind(&self, device: &wgpu::Device, world: &GpuWorld) -> wgpu::BindGroup {
        let noise = view(&self.blue_noise);
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("world"),
            layout: &self.world_layout,
            entries: &[
                buffer_entry(0, &self.params),
                buffer_entry(1, world.pool_shard(0)),
                buffer_entry(7, world.pool_shard(1)),
                buffer_entry(8, world.pool_shard(2)),
                buffer_entry(9, world.pool_shard(3)),
                buffer_entry(2, world.records()),
                buffer_entry(3, world.table()),
                buffer_entry(4, world.materials()),
                buffer_entry(10, world.light_buffer()),
                buffer_entry(11, &self.emission_scale),
                wgpu::BindGroupEntry {
                    binding: 12,
                    resource: wgpu::BindingResource::TextureView(&noise),
                },
                buffer_entry(13, self.exposure.state_uniform()),
            ],
        })
    }

    /// The temporal pass's pipeline: with [`Frame::set_count_store`] on, the one whose
    /// `COUNT_STORE` is true, built on its first use.
    pub(super) fn temporal_pipeline(&self, device: &wgpu::Device) -> &wgpu::ComputePipeline {
        if !self.count_store {
            return &self.temporal;
        }
        self.temporal_counting.get_or_init(|| {
            compute_pipeline_with(
                device,
                "temporal counting",
                format!("{MARCH_WGSL}\n{TEMPORAL_WGSL}"),
                &[Some(&self.world_layout), None, Some(&self.temporal_layout)],
                "temporal",
                &[("COUNT_STORE", 1.0)],
            )
        })
    }

    /// The light pass's pipeline (`shade`), built on its first use.
    pub(super) fn light_pipeline(&self, device: &wgpu::Device) -> &wgpu::ComputePipeline {
        self.light.get_or_init(|| {
            compute_pipeline(
                device,
                "light",
                format!("{MARCH_WGSL}\n{LIGHT_WGSL}"),
                &[Some(&self.world_layout), None, Some(&self.light_layout)],
                "shade",
            )
        })
    }

    /// The uniform block for `camera` over `world`, lit by `light`: one view filling the
    /// target.
    pub fn params(&self, world: &GpuWorld, camera: &Camera, light: &LightParams) -> Params {
        let rect = [0, 0, self.targets.width, self.targets.height];
        self.params_for(world, &[View::of_camera(camera, rect)], camera, light)
    }

    /// The uniform block for `views` (at most [`MAX_VIEWS`]; a pixel outside every rectangle
    /// takes the first) over `world`, lit by `light` placed relative to `camera`. Each view's
    /// jitter is the frame's ([`Frame::set_jitter`]). [`Frame::encode`] fills in the history
    /// fields.
    pub fn params_for(
        &self,
        world: &GpuWorld,
        views: &[View],
        camera: &Camera,
        light: &LightParams,
    ) -> Params {
        assert!(
            (1..=MAX_VIEWS).contains(&views.len()),
            "{} views",
            views.len()
        );
        let (fwd, right, up) = camera.basis();
        let mut vs = [View::default(); MAX_VIEWS];
        vs[..views.len()].copy_from_slice(views);
        let fsr = self
            .fsr
            .as_ref()
            .filter(|f| !f.bypassed())
            .map(Fsr2::settings);
        let jitter = if let Some(f) = self.fsr.as_ref().filter(|f| !f.bypassed()) {
            // FSR's sample of pixel p lies at p + 0.5 - offset (`fsr2::jitter_offset`).
            let [w, _] = self.internal_size();
            let [x, y] = jitter_at(self.frame, f.settings().phases(w, self.output[0]));
            [-x, -y]
        } else if self.jitter {
            jitter_at(self.frame, JITTER_PHASES)
        } else {
            [0.0; 2]
        };
        for v in &mut vs[..views.len()] {
            v.jitter = jitter;
        }
        let e = world.window();
        Params {
            views: vs,
            view_count: views.len() as u32,
            translucent_layers: self.look.translucent_layers,
            window_min: e.min.0,
            face_mix: self.look.tuning.face_mix,
            far_normal: self.look.far_normal,
            window_size: e.size(),
            light_pos: (camera.position + right * light.offset_right - up * light.offset_down)
                .to_array(),
            lamp_pose: [light.offset_right, light.offset_down, light.turn],
            light_intensity: light.intensity,
            light_dir: (fwd * light.turn.cos() + right * light.turn.sin()).to_array(),
            cos_outer: light.outer.cos(),
            size: [self.targets.width, self.targets.height],
            cos_inner: light.inner.cos(),
            flags: if light.shadows { FLAG_SHADOWS } else { 0 }
                | if light.masks { FLAG_MASKS } else { 0 }
                | if light.skips { FLAG_SKIPS } else { 0 }
                | self.look.flags(),
            units_per_shard: world.shard_layout().units_per_shard,
            forced_level: self.lod.forced_level.unwrap_or(0),
            lod1_r2: (2 * self.lod.r_full).pow(2),
            lod2_r2: (4 * self.lod.r_full).pow(2),
            exposure: self.lighting.manual_exposure(),
            dither: u32::from(fsr.is_some_and(|f| f.dither)),
            output_size: if self.upscaling() {
                self.output
            } else {
                [self.targets.width, self.targets.height]
            },
            lighting: self.lighting.flags()
                | if self
                    .emission_changed_at
                    .is_some_and(|f| self.frame.wrapping_sub(f) <= EMISSION_CHANGE_FRAMES)
                {
                    LIGHTING_EMISSION_CHANGED
                } else {
                    0
                },
            light_table_base: world.light_table_base(),
            candidates: self.lighting.candidates,
            frame: self.frame,
            light_size: self.targets.light_size(),
            light_scale: self.targets.scale,
            light_offset: if self.targets.scale > 1 {
                let [x, y] = HALF_OFFSET;
                x | y << 8
            } else {
                0
            },
            ..Params::default()
        }
    }

    /// Records the frame's passes over `world` into `encoder`, with history: the temporal
    /// pass reads the previous frame's hit buffer and history unless they are invalid, and
    /// takes the edit boxes collected since the last encode. With `timer` (of at least
    /// `FRAME_PASSES.len()` passes) pass `i` writes its start and end into timestamps `2 i`
    /// and `2 i + 1`; the caller resolves them.
    pub fn encode(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        world: &GpuWorld,
        params: &Params,
        timer: Option<&PassTimer>,
    ) {
        self.encode_with(device, queue, encoder, world, params, timer, true);
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "encode's arguments plus whether the frame keeps history"
    )]
    pub(super) fn encode_with(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        world: &GpuWorld,
        params: &Params,
        timer: Option<&PassTimer>,
        keep_history: bool,
    ) {
        let mut p = *params;
        // The probe passes run in the shaded and cracks views outside the cutaway, with the
        // lamp's bounce on and the lamp on (or a preset volume, whose updates are off).
        let presets = self.probes.get().is_some_and(|v| !v.updates());
        let indirect = self.lighting.lamp_bounce
            && (self.lighting.lamp || presets)
            && self.look.view.shades()
            && !self.look.cutaway;
        // The volume exists from the first encode with the channel running.
        let volume = indirect.then(|| self.probes());
        let probe_update = volume.is_some_and(ProbeVolume::updates);
        // The channel starts from zero when it was not running at the last encode or the
        // lamp's light changed: every probe's state is cleared and a window opens.
        let channel_was = self
            .probe_channel_ran
            .swap(probe_update, std::sync::atomic::Ordering::Relaxed);
        let mut probe_restart = probe_update && !channel_was;
        let mut light_window = false;
        let mut lamp_moved = false;
        let mut boxes: Vec<[[i32; 3]; 2]> = Vec::new();
        let mut edits = EditsUniform {
            count: 0,
            _pad: [0; 3],
            boxes: [[0; 4]; 2 * MAX_EDITS],
        };
        let mut old_lamp = Lamp::of(params);
        if keep_history {
            let mut h = self
                .history
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            p.prev_views = h.prev_views;
            p.prev_light_offset = h.prev_offset;
            h.last_offset = params.light_offset;
            p.history = u32::from(h.valid && !h.overflow);
            // Bit 1: the upscaler's history holds this view's image.
            p.history |= u32::from(h.valid && !h.overflow && !h.view_changed) << 1;
            h.view_changed = false;
            for (i, [min, max]) in h.edits.iter().enumerate() {
                edits.boxes[2 * i] = [min[0], min[1], min[2], 0];
                edits.boxes[2 * i + 1] = [max[0], max[1], max[2], 0];
            }
            edits.count = h.edits.len() as u32;
            boxes = std::mem::take(&mut h.edits);
            h.overflow = false;
            h.last_views = params.views;
            h.encoded = true;
            let key = LightKey {
                lamp: self.lighting.lamp,
                params: LampKey::of(params),
                shadow_walk: self.lighting.shadow_walk,
            };
            let lamp_changed = h.light_key.is_some_and(|k| k.lamp_differs(&key));
            if lamp_changed {
                p.lighting |= LIGHTING_LAMP_CHANGED;
            }
            self.lamp_changed
                .store(lamp_changed, std::sync::atomic::Ordering::Relaxed);
            // The probes see the lamp alone: its switch, parameters or shadow walk restart them.
            probe_restart |= probe_update && lamp_changed;
            if probe_restart {
                h.light_window = crate::probes::WINDOW_FRAMES;
            }
            h.light_key = Some(key);
            if probe_update && h.light_window > 0 {
                light_window = true;
                h.light_window -= 1;
            }
            let back = (self.upscaler.change_frames.max(1) as usize).min(LAMP_FRAMES);
            old_lamp = h
                .lamps
                .len()
                .checked_sub(back)
                .and_then(|i| h.lamps.get(i))
                .or(h.lamps.front())
                .copied()
                .unwrap_or_else(|| Lamp::of(params));
            let now = Lamp::of(params);
            lamp_moved = h
                .lamps
                .back()
                .is_some_and(|l| l.pos != now.pos || l.dir != now.dir);
            if h.lamps.len() == LAMP_FRAMES {
                h.lamps.pop_front();
            }
            h.lamps.push_back(Lamp::of(params));
        } else {
            p.history = 0;
        }
        queue.write_buffer(&self.edits, 0, bytemuck::bytes_of(&edits));
        let scale = self.output[0] as f32 / self.targets.width as f32;
        queue.write_buffer(
            &self.upscale_params,
            0,
            bytemuck::bytes_of(&self.upscaler.uniform(scale, old_lamp)),
        );
        let table = light_table(world, p.views[0].origin);
        let used = (1 + table.count as usize) * 16;
        queue.write_buffer(
            world.light_buffer(),
            u64::from(world.light_table_base()) * crate::gpu_world::LIGHT_SAMPLE_BYTES,
            &bytemuck::bytes_of(&table)[..used],
        );
        if !self
            .blue_noise_written
            .swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            queue.write_texture(
                self.blue_noise.as_image_copy(),
                BLUE_NOISE,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(128),
                    rows_per_image: Some(128),
                },
                self.blue_noise.size(),
            );
        }
        let t = &self.targets;
        let world_bind = self.world_bind(device, world);
        // The cutaway and a mirror without band slots walk the voxels.
        let field = self.look.normal_mode == NormalMode::Contour
            && !self.look.cutaway
            && world.field_surface();
        // The split walk pays only where a brick walks as cubes: with no crisp row every band
        // brick is a field brick. The cracks and steps views read the field walk's own leaks
        // and counters, so they walk it unsplit.
        let split = field
            && self.field_split
            && world.crisp_rows()
            && !matches!(self.look.view, ViewMode::Cracks | ViewMode::Steps)
            && u64::from(t.width) * u64::from(t.height)
                <= u64::from(SPLIT_GROUP) * u64::from(MAX_WORKGROUPS);
        // The hit pass binds the full list only in a frame that splits; the unsplit walk
        // leaves the stub untouched.
        let split_list = if split {
            t.split_list(device)
        } else {
            &t.split_stub
        };
        let (c, q) = (self.cur, self.cur ^ 1);
        let hit_bind = bind_io(
            device,
            &self.hit.out_layout,
            &[
                Res::Tex(&t.hit_pos[c]),
                Res::Tex(&t.hit_info[c]),
                Res::Tex(&t.layer),
                Res::Buf(split_list),
                Res::Buf(&t.split_args),
                Res::Tex(&t.transmit),
                Res::Buf(self.store.pool()),
            ],
        );
        let half = t.half_pos.is_some();
        let downsample_bind = bind_io(
            device,
            &self.downsample_layout,
            &[
                Res::Tex(&t.hit_pos[c]),
                Res::Tex(&t.hit_info[c]),
                Res::Tex(t.lit_pos(c)),
                Res::Tex(t.lit_info(c)),
            ],
        );
        // The store's counters: missing, brick reads, coarse reads.
        encoder.clear_buffer(&self.store_counters, 0, None);
        let light_bind = bind_io(
            device,
            &self.light_layout,
            &[
                Res::Tex(t.lit_pos(c)),
                Res::Tex(t.lit_info(c)),
                Res::Tex(&t.irradiance),
            ],
        );
        let temporal_bind = bind_io(
            device,
            &self.temporal_layout,
            &[
                Res::Tex(t.lit_pos(c)),
                Res::Tex(t.lit_info(c)),
                Res::Tex(t.lit_pos(q)),
                Res::Tex(t.lit_info(q)),
                Res::Tex(&t.irradiance),
                Res::Tex(&t.accum[q]),
                Res::Tex(&t.moments[q]),
                Res::Tex(&t.accum[c]),
                Res::Tex(&t.moments[c]),
                Res::Buf(&self.edits),
                Res::Tex(&t.strata),
                Res::Buf(self.store.pool()),
                Res::Buf(&self.store_counters),
            ],
        );
        // The filter runs in the shaded and cracks views unless switched off; the variance pass
        // writes `filter[0]` and the iterations ping-pong, ending in `filter[0]`.
        let filtering = self.lighting.filter && self.look.view.shades();
        let variance_bind = bind_io(
            device,
            &self.variance_layout,
            &[
                Res::Tex(t.lit_pos(c)),
                Res::Tex(t.lit_info(c)),
                Res::Tex(&t.accum[c]),
                Res::Tex(&t.moments[c]),
                Res::Tex(&t.filter[0]),
            ],
        );
        let atrous_binds: Vec<wgpu::BindGroup> = (0..ATROUS_ITERATIONS)
            .map(|i| {
                bind_io_at(
                    device,
                    &self.atrous_layout,
                    &[0, 1, 2, 5, 6, 7, 8],
                    &[
                        Res::Tex(t.lit_pos(c)),
                        Res::Tex(t.lit_info(c)),
                        Res::Tex(&t.filter[i % 2]),
                        Res::Tex(&t.filter[(i + 1) % 2]),
                        Res::Buf(&self.steps[i]),
                        Res::Tex(&t.accum[c]),
                        Res::Tex(&t.moments[c]),
                    ],
                )
            })
            .collect();
        let lit = if filtering {
            &t.filter[ATROUS_ITERATIONS % 2]
        } else {
            &t.accum[c]
        };
        // In FSR mode the composite writes FSR's colour input at the internal size.
        let composite_out = self.fsr.as_ref().map_or(&t.output, Fsr2::colour);
        // FSR 2's composite stores linear radiance (`composite_hdr`).
        let (composite_layout, composite) = self
            .composite_hdr
            .as_ref()
            .filter(|_| self.fsr.is_some())
            .map_or((&self.composite_layout, &self.composite), |(l, p)| (l, p));
        let composite_bind = bind_io(
            device,
            composite_layout,
            &[
                Res::Tex(&t.hit_pos[c]),
                Res::Tex(&t.hit_info[c]),
                Res::Tex(&t.layer),
                Res::Tex(lit),
                Res::Tex(composite_out),
                Res::Tex(t.lit_pos(c)),
                Res::Tex(t.lit_info(c)),
                Res::Tex(&t.indirect),
                Res::Buf(&t.tiles),
                Res::Tex(&t.transmit),
            ],
        );
        let gradient_bind = bind_io(
            device,
            &self.gradient_layout,
            &[
                Res::Tex(t.lit_pos(q)),
                Res::Tex(t.lit_info(q)),
                Res::Tex(&t.irradiance),
                Res::Tex(&t.strata),
            ],
        );
        let upscale_bind = t.history.as_ref().map(|hist| {
            bind_io(
                device,
                &self.upscale_layout,
                &[
                    Res::Tex(&t.hit_pos[c]),
                    Res::Tex(&t.hit_info[c]),
                    Res::Tex(&t.output),
                    Res::Tex(&t.hit_pos[q]),
                    Res::Tex(&t.hit_info[q]),
                    Res::Tex(&hist[q]),
                    Res::Tex(&hist[c]),
                    Res::Buf(&self.edits),
                    Res::Buf(&self.upscale_params),
                ],
            )
        });
        let sharpen_bind = t.history.as_ref().map(|hist| {
            bind_io(
                device,
                &self.sharpen_layout,
                &[
                    Res::Tex(&hist[c]),
                    Res::Tex(&t.output),
                    Res::Buf(&self.sharpen_params),
                ],
            )
        });
        // The gradient pass runs in a shaded view for the gradient mode's lambda.
        let gradient_on = self.look.view.shades() && p.lighting & LIGHTING_GRADIENT != 0;
        let full = [t.width.div_ceil(WORKGROUP), t.height.div_ceil(WORKGROUP)];
        let lw = t.light_size().map(|n| n.div_ceil(WORKGROUP));
        let strata = [t.strata.width(), t.strata.height()].map(|n| n.div_ceil(WORKGROUP));
        let fsr_binds = self.fsr.as_ref().map(|f| {
            (
                f,
                f.prepare(
                    device,
                    queue,
                    &p,
                    &crate::fsr2::FrameInputs {
                        hit_pos: &t.hit_pos[c],
                        hit_info: &t.hit_info[c],
                        layer: &t.layer,
                        edits: &self.edits,
                        lamp: &self.upscale_params,
                        output: &t.output,
                        blue_noise: &self.blue_noise,
                        cur: c,
                        dt: self.dt,
                    },
                ),
            )
        });
        if let Some(f) = &self.fsr {
            f.copy_exposure(
                encoder,
                self.exposure.state(),
                matches!(self.lighting.exposure, Exposure::Auto(_)),
                p.history & 2 == 0,
            );
        }
        // A frame pass's dispatch with the world's group 0 and its own group `group`.
        let frame_pass = |pipeline, bind, group: usize, workgroups| {
            let mut groups = [Some(&world_bind), None, None, None];
            groups[group] = Some(bind);
            Dispatch {
                pipeline,
                groups,
                workgroups,
                indirect: None,
            }
        };
        let hit_pipeline = if split {
            p.flags |= FLAG_FIELD_SPLIT;
            &self.hit.pipeline
        } else if field {
            &self.hit.field
        } else {
            &self.hit.pipeline
        };
        let resume_bind = split.then(|| {
            encoder.clear_buffer(&t.split_args, 0, None);
            encoder.clear_buffer(split_list, 0, Some(16));
            bind_io(
                device,
                &self.hit.resume_layout,
                &[
                    Res::Tex(&t.hit_pos[c]),
                    Res::Tex(&t.hit_info[c]),
                    Res::Tex(&t.layer),
                    Res::Buf(split_list),
                ],
            )
        });
        let mut hit: Vec<Dispatch<'_>> = vec![frame_pass(hit_pipeline, &hit_bind, 1, full)];
        if let Some(b) = &resume_bind {
            hit.push(Dispatch {
                indirect: Some((&t.split_args, 0)),
                ..frame_pass(&self.hit.resume, b, 1, [0, 0])
            });
        }
        if half {
            hit.push(frame_pass(&self.downsample, &downsample_bind, 3, lw));
        }
        let mut gradient: Vec<Dispatch<'_>> = Vec::new();
        if gradient_on {
            gradient.push(frame_pass(&self.gradient, &gradient_bind, 3, strata));
        }
        let mut atrous: Vec<Dispatch<'_>> = Vec::new();
        if filtering {
            atrous.push(frame_pass(&self.variance, &variance_bind, 2, lw));
            atrous.extend(
                atrous_binds
                    .iter()
                    .map(|b| frame_pass(&self.atrous, b, 2, lw)),
            );
        }
        p.lighting &= !LIGHTING_INDIRECT;
        let mut gather: Vec<Dispatch<'_>> = Vec::new();
        let mut probes: Vec<Dispatch<'_>> = Vec::new();
        let mut updates = 0;
        self.traced_probe_rays
            .store(0, std::sync::atomic::Ordering::Relaxed);
        self.probe_light_window
            .store(light_window, std::sync::atomic::Ordering::Relaxed);
        let probe_bind = volume.map(|volume| {
            p.lighting |= LIGHTING_INDIRECT;
            if probe_update {
                let slices = if light_window {
                    crate::probes::WINDOW_SLICES
                } else {
                    self.lighting.slices()
                };
                let origin = volume.origin_for(p.views[0].origin);
                let touched: Vec<[i32; 3]> = boxes
                    .iter()
                    .flat_map(|&[min, max]| volume.touched(origin, min, max))
                    .collect();
                if probe_restart {
                    let base = u64::from(volume.state_base()) * 4;
                    encoder.clear_buffer(volume.data(), base, Some(volume.state_bytes()));
                    volume.forget_reach();
                }
                let reach = self.lighting.lamp_reach;
                let lamp = p.light_pos;
                let moved = lamp_moved && self.lighting.lamp;
                let list = volume.update_list(origin, p.frame, slices, &touched, moved, |w| {
                    let q = volume.probe_position(w);
                    let d2: f32 = (0..3).map(|k| (q[k] - lamp[k]).powi(2)).sum();
                    reach <= 0.0 || d2 <= reach * reach
                });
                queue.write_buffer(volume.scratch(), 0, bytemuck::cast_slice(&list));
                updates = list.len() as u32;
            }
            self.traced_probe_rays.store(
                u64::from(updates) * u64::from(self.lighting.rays()),
                std::sync::atomic::Ordering::Relaxed,
            );
            self.write_probe_uniform(queue, volume, &p, updates);
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("probes"),
                layout: &self.probe_layout,
                entries: &[
                    buffer_entry(0, volume.data()),
                    buffer_entry(1, volume.scratch()),
                    buffer_entry(2, volume.uniform()),
                ],
            })
        });
        let gather_bind = probe_bind.is_some().then(|| {
            bind_io(
                device,
                &self.gather_layout,
                &[
                    Res::Tex(t.lit_pos(c)),
                    Res::Tex(t.lit_info(c)),
                    Res::Tex(&t.indirect),
                ],
            )
        });
        if let (Some(b), Some(g)) = (&probe_bind, &gather_bind) {
            gather.push(Dispatch {
                pipeline: &self.gather,
                groups: [Some(&world_bind), None, Some(g), Some(b)],
                workgroups: lw,
                indirect: None,
            });
            if updates > 0 {
                probes.push(frame_pass(&self.probe_trace, b, 3, [updates, 1]));
                probes.push(frame_pass(&self.probe_blend, b, 3, [updates, 1]));
            }
        }
        self.probe_dispatches.store(
            (gather.len() + probes.len()) as u32,
            std::sync::atomic::Ordering::Relaxed,
        );
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&p));
        let mut upscale: Vec<Dispatch<'_>> = Vec::new();
        let mut sharpen: Vec<Dispatch<'_>> = Vec::new();
        if let (Some(u), Some(r)) = (&upscale_bind, &sharpen_bind) {
            let out = self.output.map(|n| n.div_ceil(WORKGROUP));
            upscale.push(frame_pass(&self.upscale, u, 2, out));
            sharpen.push(frame_pass(&self.sharpen, r, 3, out));
        }
        let fsr_passes = fsr_binds
            .as_ref()
            .map(|(f, b)| b.passes(f, &world_bind))
            .unwrap_or_default();
        let passes = [
            hit,
            gradient,
            vec![frame_pass(self.light_pipeline(device), &light_bind, 2, lw)],
            vec![frame_pass(
                self.temporal_pipeline(device),
                &temporal_bind,
                2,
                lw,
            )],
            atrous,
            probes,
            gather,
            vec![frame_pass(composite, &composite_bind, 2, full)],
            upscale,
            sharpen,
        ]
        .into_iter()
        .chain(fsr_passes);
        for (i, dispatches) in passes.enumerate() {
            // An empty slot is still recorded under a timer, so every timestamp pair is written.
            if dispatches.is_empty() && timer.is_none() {
                continue;
            }
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(FRAME_PASSES[i]),
                timestamp_writes: timer.map(|t| t.writes_at(i as u32)),
            });
            for d in dispatches {
                d.record(&mut pass);
            }
            drop(pass);
            if FRAME_PASSES[i] == "composite"
                && self.fsr.is_none()
                && let Some(capture) = &self.composite_capture
            {
                encoder.copy_texture_to_texture(
                    t.output.as_image_copy(),
                    capture.as_image_copy(),
                    capture.size(),
                );
            }
        }
        let exposure_pass = FRAME_PASSES.len() as u32 - 1;
        self.exposure.encode(
            device,
            queue,
            encoder,
            &t.tiles,
            &self.exposure_uniform(&p),
            timer
                .filter(|t| t.passes() > exposure_pass)
                .map(|t| t.writes_at(exposure_pass)),
        );
    }

    /// The exposure pass's parameters for the frame `p`.
    pub(super) fn exposure_uniform(&self, p: &Params) -> ExposureUniform {
        let metering = match self.lighting.exposure {
            Exposure::Auto(a) => a.metering,
            Exposure::Manual(_) => self.metering,
        };
        let (strength, sigma) = metering.params();
        let meter = self.look.view.shades();
        let mut mode = if meter { EX_METER } else { 0 };
        if let Exposure::Auto(_) = self.lighting.exposure {
            mode |= EX_AUTO;
            if meter
                && self
                    .reset_exposure
                    .swap(false, std::sync::atomic::Ordering::Relaxed)
            {
                mode |= EX_RESET;
            }
        }
        let mut rects = [[0.0; 4]; 2];
        for (r, v) in rects.iter_mut().zip(&p.views) {
            *r = v.rect.map(|x| x as f32);
        }
        let mut u = ExposureUniform {
            tiles: self.targets.tile_counts(),
            view_count: p.view_count,
            mode,
            rects,
            manual: self.lighting.manual_exposure(),
            strength,
            sigma,
            dt: self.dt,
            ..ExposureUniform::default()
        };
        if let Exposure::Auto(a) = &self.lighting.exposure {
            u.set_auto(a);
        }
        u
    }

    /// Renders `camera` over `world` lit by `light` as one frame alone, with no history,
    /// and submits the work; the frame loop's history is left as it was.
    pub fn render(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        world: &GpuWorld,
        camera: &Camera,
        light: &LightParams,
    ) {
        let params = self.params(world, camera, light);
        self.submit(device, queue, world, &params);
    }

    /// Renders the frame `params` describes over `world` alone, with no history, and submits
    /// the work.
    pub fn submit(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        world: &GpuWorld,
        params: &Params,
    ) {
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        self.encode_with(device, queue, &mut enc, world, params, None, false);
        queue.submit([enc.finish()]);
    }
}
