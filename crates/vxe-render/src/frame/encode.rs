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

/// Whether an encode reads and advances the frame's history ([`Frame::encode`]) or draws one
/// frame alone ([`Frame::submit`]).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Keep {
    History,
    Alone,
}

/// What an encode's uniform stage hands on: the frame's parameters as the passes read them,
/// the edit boxes it took, and the probe channel's state this frame.
struct Uniforms<'a> {
    p: Params,
    boxes: Vec<[[i32; 3]; 2]>,
    volume: Option<&'a ProbeVolume>,
    probe_update: bool,
    probe_restart: bool,
    light_window: bool,
    lamp_moved: bool,
}

/// The encode's bind groups, and the walk the hit pass takes: the field surface, the split
/// walk and its list.
struct Binds<'a> {
    world_bind: wgpu::BindGroup,
    field: bool,
    split: bool,
    split_list: &'a wgpu::Buffer,
    hit_bind: wgpu::BindGroup,
    half: bool,
    downsample_bind: wgpu::BindGroup,
    light_bind: wgpu::BindGroup,
    temporal_bind: wgpu::BindGroup,
    filtering: bool,
    variance_bind: wgpu::BindGroup,
    atrous_binds: Vec<wgpu::BindGroup>,
    out: OutputBinds<'a>,
}

/// The composite's pipeline and the output passes' bind groups.
struct OutputBinds<'a> {
    composite: &'a wgpu::ComputePipeline,
    composite_bind: wgpu::BindGroup,
    gradient_bind: wgpu::BindGroup,
    upscale_bind: Option<wgpu::BindGroup>,
    sharpen_bind: Option<wgpu::BindGroup>,
}

/// A frame pass's dispatch with the world's group 0 and its own group `group`.
fn frame_pass<'a>(
    world: &'a wgpu::BindGroup,
    pipeline: &'a wgpu::ComputePipeline,
    bind: &'a wgpu::BindGroup,
    group: usize,
    workgroups: [u32; 2],
) -> Dispatch<'a> {
    let mut groups = [Some(world), None, None, None];
    groups[group] = Some(bind);
    Dispatch {
        pipeline,
        groups,
        workgroups,
        indirect: None,
    }
}

impl Frame {
    pub(super) fn world_bind(&self, device: &wgpu::Device, world: &GpuWorld) -> wgpu::BindGroup {
        let noise = view(&self.state.blue_noise);
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("world"),
            layout: &self.pipes.world_layout,
            entries: &[
                buffer_entry(0, &self.state.params),
                buffer_entry(1, world.pool_shard(0)),
                buffer_entry(7, world.pool_shard(1)),
                buffer_entry(8, world.pool_shard(2)),
                buffer_entry(9, world.pool_shard(3)),
                buffer_entry(2, world.records()),
                buffer_entry(3, world.table()),
                buffer_entry(4, world.materials()),
                buffer_entry(10, world.light_buffer()),
                buffer_entry(11, &self.state.emission_scale),
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
        let pass = if self.baked.count_store {
            P_TEMPORAL_COUNTING
        } else {
            P_TEMPORAL
        };
        self.pipes.pipeline(device, pass)
    }

    /// The light pass's pipeline (`shade`), built on its first use.
    pub(super) fn light_pipeline(&self, device: &wgpu::Device) -> &wgpu::ComputePipeline {
        self.pipes.pipeline(device, P_LIGHT)
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
            .up
            .fsr
            .as_ref()
            .filter(|f| !f.bypassed())
            .map(Fsr2::settings);
        let jitter = if let Some(f) = self.up.fsr.as_ref().filter(|f| !f.bypassed()) {
            // FSR's sample of pixel p lies at p + 0.5 - offset (`fsr2::jitter_offset`).
            let [w, _] = self.internal_size();
            let [x, y] = jitter_at(self.state.frame, f.settings().phases(w, self.output[0]));
            [-x, -y]
        } else if self.up.jitter {
            jitter_at(self.state.frame, JITTER_PHASES)
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
            translucent_layers: self.state.look.translucent_layers,
            window_min: e.min.0,
            face_mix: self.state.look.tuning.face_mix,
            far_normal: self.state.look.far_normal,
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
                | self.state.look.flags(),
            units_per_shard: world.shard_layout().units_per_shard,
            forced_level: self.state.lod.forced_level.unwrap_or(0),
            lod1_r2: (2 * self.state.lod.r_full).pow(2),
            lod2_r2: (4 * self.state.lod.r_full).pow(2),
            exposure: self.state.lighting.manual_exposure(),
            dither: u32::from(fsr.is_some_and(|f| f.dither)),
            output_size: if self.upscaling() {
                self.output
            } else {
                [self.targets.width, self.targets.height]
            },
            lighting: self.state.lighting.flags()
                | if self
                    .state
                    .emission_changed_at
                    .is_some_and(|f| self.state.frame.wrapping_sub(f) <= EMISSION_CHANGE_FRAMES)
                {
                    LIGHTING_EMISSION_CHANGED
                } else {
                    0
                },
            light_table_base: world.light_table_base(),
            candidates: self.state.lighting.candidates,
            frame: self.state.frame,
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
        self.encode_with(device, queue, encoder, world, params, timer, Keep::History);
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
        keep: Keep,
    ) {
        let uni = self.write_uniforms(queue, world, params, keep);
        let binds = self.bind_groups(device, encoder, world);
        self.record(device, queue, encoder, &uni, &binds, timer);
    }

    /// The encode's uniform stage: the frame's parameters with the history's fields (with
    /// [`Keep::History`]), the probe channel's state, and the edit boxes, the upscaler's
    /// tuning, the light table and the blue noise written.
    fn write_uniforms(
        &self,
        queue: &wgpu::Queue,
        world: &GpuWorld,
        params: &Params,
        keep: Keep,
    ) -> Uniforms<'_> {
        let mut p = *params;
        // The probe passes run in the shaded and cracks views outside the cutaway, with the
        // lamp's bounce on and the lamp on (or a preset volume, whose updates are off).
        let presets = self.probe.volume.get().is_some_and(|v| !v.updates());
        let indirect = self.state.lighting.lamp_bounce
            && (self.state.lighting.lamp || presets)
            && self.state.look.view.shades()
            && !self.state.look.cutaway;
        // The volume exists from the first encode with the channel running.
        let volume = indirect.then(|| self.probes());
        let probe_update = volume.is_some_and(ProbeVolume::updates);
        // The channel starts from zero when it was not running at the last encode or the
        // lamp's light changed: every probe's state is cleared and a window opens.
        let channel_was = self
            .probe
            .ran
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
        if keep == Keep::History {
            let mut h = self
                .state
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
                lamp: self.state.lighting.lamp,
                params: LampKey::of(params),
                shadow_walk: self.state.lighting.shadow_walk,
            };
            let lamp_changed = h.light_key.is_some_and(|k| k.lamp_differs(&key));
            if lamp_changed {
                p.lighting |= LIGHTING_LAMP_CHANGED;
            }
            self.state
                .lamp_changed
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
            let back = (self.up.upscaler.change_frames.max(1) as usize).min(LAMP_FRAMES);
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
        queue.write_buffer(&self.state.edits, 0, bytemuck::bytes_of(&edits));
        let scale = self.output[0] as f32 / self.targets.width as f32;
        queue.write_buffer(
            &self.up.params,
            0,
            bytemuck::bytes_of(&self.up.upscaler.uniform(scale, old_lamp)),
        );
        let table = light_table(world, p.views[0].origin);
        let used = (1 + table.count as usize) * 16;
        queue.write_buffer(
            world.light_buffer(),
            u64::from(world.light_table_base()) * crate::gpu_world::LIGHT_SAMPLE_BYTES,
            &bytemuck::bytes_of(&table)[..used],
        );
        if !self
            .state
            .blue_noise_written
            .swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            queue.write_texture(
                self.state.blue_noise.as_image_copy(),
                BLUE_NOISE,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(128),
                    rows_per_image: Some(128),
                },
                self.state.blue_noise.size(),
            );
        }
        Uniforms {
            p,
            boxes,
            volume,
            probe_update,
            probe_restart,
            light_window,
            lamp_moved,
        }
    }

    /// The encode's bind groups of the hit, light and temporal passes, of the filter and of
    /// the output passes, with the walk the hit pass takes. Clears the store's counters.
    fn bind_groups(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        world: &GpuWorld,
    ) -> Binds<'_> {
        let t = &self.targets;
        let world_bind = self.world_bind(device, world);
        // The cutaway and a mirror without band slots walk the voxels.
        let field = self.state.look.normal_mode == NormalMode::Contour
            && !self.state.look.cutaway
            && world.field_surface();
        // The split walk pays only where a brick walks as cubes: with no crisp row every band
        // brick is a field brick. The cracks and steps views read the field walk's own leaks
        // and counters, so they walk it unsplit.
        let split = field
            && self.state.field_split
            && world.crisp_rows()
            && !matches!(self.state.look.view, ViewMode::Cracks | ViewMode::Steps)
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
            &self.pipes.hit.out_layout,
            &[
                Res::Tex(&t.hit_pos[c]),
                Res::Tex(&t.hit_info[c]),
                Res::Tex(&t.layer),
                Res::Buf(split_list),
                Res::Buf(&t.split_args),
                Res::Tex(&t.transmit),
                Res::Buf(self.baked.store.pool()),
            ],
        );
        let half = t.half_pos.is_some();
        let downsample_bind = bind_io(
            device,
            self.pipes.layout(P_DOWNSAMPLE),
            &[
                Res::Tex(&t.hit_pos[c]),
                Res::Tex(&t.hit_info[c]),
                Res::Tex(t.lit_pos(c)),
                Res::Tex(t.lit_info(c)),
            ],
        );
        // The store's counters: missing, brick reads, coarse reads.
        encoder.clear_buffer(&self.baked.counters, 0, None);
        let light_bind = bind_io(
            device,
            self.pipes.layout(P_LIGHT),
            &[
                Res::Tex(t.lit_pos(c)),
                Res::Tex(t.lit_info(c)),
                Res::Tex(&t.irradiance),
            ],
        );
        let temporal_bind = bind_io(
            device,
            self.pipes.layout(P_TEMPORAL),
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
                Res::Buf(&self.state.edits),
                Res::Tex(&t.strata),
                Res::Buf(self.baked.store.pool()),
                Res::Buf(&self.baked.counters),
            ],
        );
        // The filter runs in the shaded and cracks views unless switched off; the variance pass
        // writes `filter[0]` and the iterations ping-pong, ending in `filter[0]`.
        let (filtering, variance_bind, atrous_binds) = self.filter_binds(device);
        let out = self.output_binds(device, filtering);
        Binds {
            world_bind,
            field,
            split,
            split_list,
            hit_bind,
            half,
            downsample_bind,
            light_bind,
            temporal_bind,
            filtering,
            variance_bind,
            atrous_binds,
            out,
        }
    }

    /// The filter's bind groups: whether it runs, the variance pass's and each a-trous
    /// iteration's.
    fn filter_binds(&self, device: &wgpu::Device) -> (bool, wgpu::BindGroup, Vec<wgpu::BindGroup>) {
        let t = &self.targets;
        let c = self.cur;
        let filtering = self.state.lighting.filter && self.state.look.view.shades();
        let variance_bind = bind_io(
            device,
            self.pipes.layout(P_VARIANCE),
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
                    self.pipes.layout(P_ATROUS),
                    &[0, 1, 2, 5, 6, 7, 8],
                    &[
                        Res::Tex(t.lit_pos(c)),
                        Res::Tex(t.lit_info(c)),
                        Res::Tex(&t.filter[i % 2]),
                        Res::Tex(&t.filter[(i + 1) % 2]),
                        Res::Buf(&self.state.atrous_steps[i]),
                        Res::Tex(&t.accum[c]),
                        Res::Tex(&t.moments[c]),
                    ],
                )
            })
            .collect();
        (filtering, variance_bind, atrous_binds)
    }

    /// The bind groups of the composite (and its pipeline, FSR 2's variant in FSR mode), the
    /// gradient, and the engine's own upscale and sharpen passes.
    fn output_binds(&self, device: &wgpu::Device, filtering: bool) -> OutputBinds<'_> {
        let t = &self.targets;
        let (c, q) = (self.cur, self.cur ^ 1);
        let lit = if filtering {
            &t.filter[ATROUS_ITERATIONS % 2]
        } else {
            &t.accum[c]
        };
        // In FSR mode the composite writes FSR's colour input at the internal size.
        let composite_out = self.up.fsr.as_ref().map_or(&t.output, Fsr2::colour);
        // FSR 2's composite stores linear radiance (`composite_hdr`).
        let (composite_layout, composite) = self
            .up
            .composite_hdr
            .as_ref()
            .filter(|_| self.up.fsr.is_some())
            .map_or(
                (
                    self.pipes.layout(P_COMPOSITE),
                    self.pipes.pipeline(device, P_COMPOSITE),
                ),
                |(l, p)| (l, p),
            );
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
            self.pipes.layout(P_GRADIENT),
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
                self.pipes.layout(P_UPSCALE),
                &[
                    Res::Tex(&t.hit_pos[c]),
                    Res::Tex(&t.hit_info[c]),
                    Res::Tex(&t.output),
                    Res::Tex(&t.hit_pos[q]),
                    Res::Tex(&t.hit_info[q]),
                    Res::Tex(&hist[q]),
                    Res::Tex(&hist[c]),
                    Res::Buf(&self.state.edits),
                    Res::Buf(&self.up.params),
                ],
            )
        });
        let sharpen_bind = t.history.as_ref().map(|hist| {
            bind_io(
                device,
                self.pipes.layout(P_SHARPEN),
                &[
                    Res::Tex(&hist[c]),
                    Res::Tex(&t.output),
                    Res::Buf(&self.up.sharpen_params),
                ],
            )
        });
        // The gradient pass runs in a shaded view for the gradient mode's lambda.
        OutputBinds {
            composite,
            composite_bind,
            gradient_bind,
            upscale_bind,
            sharpen_bind,
        }
    }

    /// The encode's record stage: FSR 2's inputs, the dispatches of every pass, the frame's
    /// parameters written, each pass recorded in [`FRAME_PASSES`] order, and the exposure.
    fn record(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        uni: &Uniforms<'_>,
        binds: &Binds<'_>,
        timer: Option<&PassTimer>,
    ) {
        let t = &self.targets;
        let c = self.cur;
        let mut p = uni.p;
        let world = &binds.world_bind;
        let full = [t.width.div_ceil(WORKGROUP), t.height.div_ceil(WORKGROUP)];
        let lw = t.light_size().map(|n| n.div_ceil(WORKGROUP));
        let fsr_binds = self.fsr_frame(device, queue, encoder, &p);
        let hit_pipeline = if binds.split {
            p.flags |= FLAG_FIELD_SPLIT;
            &self.pipes.hit.pipeline
        } else if binds.field {
            &self.pipes.hit.field
        } else {
            &self.pipes.hit.pipeline
        };
        let resume_bind = binds.split.then(|| {
            encoder.clear_buffer(&t.split_args, 0, None);
            encoder.clear_buffer(binds.split_list, 0, Some(16));
            bind_io(
                device,
                &self.pipes.hit.resume_layout,
                &[
                    Res::Tex(&t.hit_pos[c]),
                    Res::Tex(&t.hit_info[c]),
                    Res::Tex(&t.layer),
                    Res::Buf(binds.split_list),
                ],
            )
        });
        let mut hit: Vec<Dispatch<'_>> =
            vec![frame_pass(world, hit_pipeline, &binds.hit_bind, 1, full)];
        if let Some(b) = &resume_bind {
            hit.push(Dispatch {
                indirect: Some((&t.split_args, 0)),
                ..frame_pass(world, &self.pipes.hit.resume, b, 1, [0, 0])
            });
        }
        if binds.half {
            hit.push(frame_pass(
                world,
                self.pipes.pipeline(device, P_DOWNSAMPLE),
                &binds.downsample_bind,
                3,
                lw,
            ));
        }
        let [gradient, atrous] = self.filter_dispatches(device, binds, &p);
        p.lighting &= !LIGHTING_INDIRECT;
        let mut gather: Vec<Dispatch<'_>> = Vec::new();
        let mut probes: Vec<Dispatch<'_>> = Vec::new();
        let (probe_bind, gather_bind, updates) =
            self.probe_binds(device, queue, encoder, uni, &mut p);
        if let (Some(b), Some(g)) = (&probe_bind, &gather_bind) {
            gather.push(Dispatch {
                pipeline: self.pipes.pipeline(device, P_GATHER),
                groups: [Some(world), None, Some(g), Some(b)],
                workgroups: lw,
                indirect: None,
            });
            if updates > 0 {
                probes.push(frame_pass(
                    world,
                    self.pipes.pipeline(device, P_PROBE_TRACE),
                    b,
                    3,
                    [updates, 1],
                ));
                probes.push(frame_pass(
                    world,
                    self.pipes.pipeline(device, P_PROBE_BLEND),
                    b,
                    3,
                    [updates, 1],
                ));
            }
        }
        self.probe.dispatches.store(
            (gather.len() + probes.len()) as u32,
            std::sync::atomic::Ordering::Relaxed,
        );
        queue.write_buffer(&self.state.params, 0, bytemuck::bytes_of(&p));
        let [upscale, sharpen] = self.upscale_dispatches(device, binds);
        let fsr_passes = fsr_binds
            .as_ref()
            .map(|(f, b)| b.passes(f, world))
            .unwrap_or_default();
        let out = &binds.out;
        let passes = [
            hit,
            gradient,
            vec![frame_pass(
                world,
                self.light_pipeline(device),
                &binds.light_bind,
                2,
                lw,
            )],
            vec![frame_pass(
                world,
                self.temporal_pipeline(device),
                &binds.temporal_bind,
                2,
                lw,
            )],
            atrous,
            probes,
            gather,
            vec![frame_pass(
                world,
                out.composite,
                &out.composite_bind,
                2,
                full,
            )],
            upscale,
            sharpen,
        ]
        .into_iter()
        .chain(fsr_passes);
        self.record_passes(device, queue, encoder, passes, &p, timer);
    }

    /// FSR 2's bind groups for the frame `p`, its inputs prepared and its exposure copied in;
    /// `None` outside FSR mode.
    fn fsr_frame(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        p: &Params,
    ) -> Option<(&Fsr2, crate::fsr2::Fsr2Binds)> {
        let t = &self.targets;
        let c = self.cur;
        let fsr_binds = self.up.fsr.as_ref().map(|f| {
            (
                f,
                f.prepare(
                    device,
                    queue,
                    p,
                    &crate::fsr2::FrameInputs {
                        hit_pos: &t.hit_pos[c],
                        hit_info: &t.hit_info[c],
                        layer: &t.layer,
                        edits: &self.state.edits,
                        lamp: &self.up.params,
                        output: &t.output,
                        blue_noise: &self.state.blue_noise,
                        cur: c,
                        dt: self.dt,
                    },
                ),
            )
        });
        if let Some(f) = &self.up.fsr {
            f.copy_exposure(
                encoder,
                self.exposure.state(),
                matches!(self.state.lighting.exposure, Exposure::Auto(_)),
                p.history & 2 == 0,
            );
        }
        fsr_binds
    }

    /// The gradient pass's dispatch (in a shaded view for the gradient mode's lambda) and the
    /// filter's (the variance pass and the a-trous iterations), each empty when it does not run.
    fn filter_dispatches<'a>(
        &'a self,
        device: &wgpu::Device,
        binds: &'a Binds<'_>,
        p: &Params,
    ) -> [Vec<Dispatch<'a>>; 2] {
        let t = &self.targets;
        let world = &binds.world_bind;
        let lw = t.light_size().map(|n| n.div_ceil(WORKGROUP));
        let strata = [t.strata.width(), t.strata.height()].map(|n| n.div_ceil(WORKGROUP));
        // The gradient pass runs in a shaded view for the gradient mode's lambda.
        let gradient_on = self.state.look.view.shades() && p.lighting & LIGHTING_GRADIENT != 0;
        let mut gradient: Vec<Dispatch<'_>> = Vec::new();
        if gradient_on {
            gradient.push(frame_pass(
                world,
                self.pipes.pipeline(device, P_GRADIENT),
                &binds.out.gradient_bind,
                3,
                strata,
            ));
        }
        let mut atrous: Vec<Dispatch<'_>> = Vec::new();
        if binds.filtering {
            atrous.push(frame_pass(
                world,
                self.pipes.pipeline(device, P_VARIANCE),
                &binds.variance_bind,
                2,
                lw,
            ));
            atrous.extend(
                binds
                    .atrous_binds
                    .iter()
                    .map(|b| frame_pass(world, self.pipes.pipeline(device, P_ATROUS), b, 2, lw)),
            );
        }
        [gradient, atrous]
    }

    /// The engine's own upscale and sharpen dispatches at the output size, empty without its
    /// history (FSR mode or no upscaling).
    fn upscale_dispatches<'a>(
        &'a self,
        device: &wgpu::Device,
        binds: &'a Binds<'_>,
    ) -> [Vec<Dispatch<'a>>; 2] {
        let world = &binds.world_bind;
        let mut upscale: Vec<Dispatch<'_>> = Vec::new();
        let mut sharpen: Vec<Dispatch<'_>> = Vec::new();
        if let (Some(u), Some(r)) = (&binds.out.upscale_bind, &binds.out.sharpen_bind) {
            let out = self.output.map(|n| n.div_ceil(WORKGROUP));
            upscale.push(frame_pass(
                world,
                self.pipes.pipeline(device, P_UPSCALE),
                u,
                2,
                out,
            ));
            sharpen.push(frame_pass(
                world,
                self.pipes.pipeline(device, P_SHARPEN),
                r,
                3,
                out,
            ));
        }
        [upscale, sharpen]
    }

    /// Records each pass of `passes` in [`FRAME_PASSES`] order (an empty one only under a
    /// timer), the composite's capture after the composite, and the exposure pass last.
    fn record_passes<'a>(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        passes: impl Iterator<Item = Vec<Dispatch<'a>>>,
        p: &Params,
        timer: Option<&PassTimer>,
    ) {
        let t = &self.targets;
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
                && self.up.fsr.is_none()
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
            &self.exposure_uniform(p),
            timer
                .filter(|t| t.passes() > exposure_pass)
                .map(|t| t.writes_at(exposure_pass)),
        );
    }

    /// The probe passes' bind groups (the volume's, and the gather's) when the channel runs,
    /// with the update list and the probe uniform written, and the update list's length.
    fn probe_binds(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        uni: &Uniforms<'_>,
        p: &mut Params,
    ) -> (Option<wgpu::BindGroup>, Option<wgpu::BindGroup>, u32) {
        let t = &self.targets;
        let c = self.cur;
        let Uniforms {
            volume,
            probe_update,
            probe_restart,
            light_window,
            lamp_moved,
            ref boxes,
            ..
        } = *uni;
        let mut updates = 0;
        self.probe
            .traced_rays
            .store(0, std::sync::atomic::Ordering::Relaxed);
        self.probe
            .light_window
            .store(light_window, std::sync::atomic::Ordering::Relaxed);
        let probe_bind = volume.map(|volume| {
            p.lighting |= LIGHTING_INDIRECT;
            if probe_update {
                let slices = if light_window {
                    crate::probes::WINDOW_SLICES
                } else {
                    self.state.lighting.slices()
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
                let reach = self.state.lighting.lamp_reach;
                let lamp = p.light_pos;
                let moved = lamp_moved && self.state.lighting.lamp;
                let list = volume.update_list(origin, p.frame, slices, &touched, moved, |w| {
                    let q = volume.probe_position(w);
                    let d2: f32 = (0..3).map(|k| (q[k] - lamp[k]).powi(2)).sum();
                    reach <= 0.0 || d2 <= reach * reach
                });
                queue.write_buffer(volume.scratch(), 0, bytemuck::cast_slice(&list));
                updates = list.len() as u32;
            }
            self.probe.traced_rays.store(
                u64::from(updates) * u64::from(self.state.lighting.rays()),
                std::sync::atomic::Ordering::Relaxed,
            );
            self.write_probe_uniform(queue, volume, p, updates);
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("probes"),
                layout: &self.pipes.probe_layout,
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
                self.pipes.layout(P_GATHER),
                &[
                    Res::Tex(t.lit_pos(c)),
                    Res::Tex(t.lit_info(c)),
                    Res::Tex(&t.indirect),
                ],
            )
        });
        (probe_bind, gather_bind, updates)
    }

    /// The exposure pass's parameters for the frame `p`.
    pub(super) fn exposure_uniform(&self, p: &Params) -> ExposureUniform {
        let metering = match self.state.lighting.exposure {
            Exposure::Auto(a) => a.metering,
            Exposure::Manual(_) => self.metering,
        };
        let (strength, sigma) = metering.params();
        let meter = self.state.look.view.shades();
        let mut mode = if meter { EX_METER } else { 0 };
        if let Exposure::Auto(_) = self.state.lighting.exposure {
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
            manual: self.state.lighting.manual_exposure(),
            strength,
            sigma,
            dt: self.dt,
            ..ExposureUniform::default()
        };
        if let Exposure::Auto(a) = &self.state.lighting.exposure {
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
        self.encode_with(device, queue, &mut enc, world, params, None, Keep::Alone);
        queue.submit([enc.finish()]);
    }
}
