//! The frame's link to the loaded light's store: its bakes, the live queue and the digs,
//! and the store's counters.

use super::*;

/// Bytes of the store's GPU counters that [`Frame::read_store_stats`] reads: five `u32` words
/// (missing, brick reads, coarse reads, fallback, still black).
pub(super) const STORE_STATS_BYTES: u64 = 20;

/// Holds the light-sample ranges out to every chunk of `extent`'s reach (its farthest corner from
/// the focus chunk, plus the light radius and one, for a job's box) and syncs until they settle.
pub(super) fn hold_reach(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    gpu: &mut GpuWorld,
    world: &vxe_world::World,
    extent: vxe_world::ChunkExtent,
) -> Result<(), crate::bake::BakeError> {
    let f = gpu.focus().0;
    let size = extent.size();
    let far: i64 = (0..3)
        .map(|k| {
            let lo = i64::from(extent.min.0[k] - f[k]).abs();
            let hi = i64::from(extent.min.0[k] + size[k] - 1 - f[k]).abs();
            lo.max(hi).pow(2)
        })
        .sum();
    let reach = (far as f64).sqrt().ceil() as i32 + gpu.light_radius() + 1;
    gpu.hold_lights(reach);
    let mut refused = 0;
    loop {
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let synced = gpu.sync(world, queue, &mut enc);
        queue.submit([enc.finish()]);
        if let Ok(s) = synced {
            refused += s.light_refused;
        }
        if gpu.lights_settled() || refused > 0 {
            break;
        }
    }
    if refused > 0 {
        return Err(crate::bake::BakeError::LightsRefused(refused));
    }
    Ok(())
}

impl Frame {
    /// Group 0 of the frame's shaders over `world`.
    /// [`Frame::world_bind`] for the live queue's step, made again only when one of the buffers
    /// it binds was replaced (a mirror shard or the light-sample buffer grown, say).
    pub(super) fn queue_world_bind(
        &mut self,
        device: &wgpu::Device,
        world: &GpuWorld,
    ) -> wgpu::BindGroup {
        let key = vec![
            self.params.clone(),
            world.pool_shard(0).clone(),
            world.pool_shard(1).clone(),
            world.pool_shard(2).clone(),
            world.pool_shard(3).clone(),
            world.records().clone(),
            world.table().clone(),
            world.materials().clone(),
            world.light_buffer().clone(),
            self.emission_scale.clone(),
            self.exposure.state_uniform().clone(),
        ];
        match &self.queue_world_bind {
            Some((k, bind)) if *k == key => bind.clone(),
            _ => {
                let bind = self.world_bind(device, world);
                self.queue_world_bind = Some((key, bind.clone()));
                bind
            }
        }
    }

    /// Bakes every chunk of `extent` into the store, emptied first, before any frame reads it,
    /// blocking: submits of at most [`crate::bake::SUBMIT_SAMPLES`] samples, each waited on, with
    /// `progress` after each. The light-sample buffer's ranges are first held and written out to
    /// the bake's reach. `world` is the voxels `gpu` mirrors, which `gpu` must hold already. The
    /// first bake allocates the pool ([`Lighting::bake_pool_bytes`]). `Err` when the pool or the
    /// light samples cannot hold the extent.
    #[allow(
        clippy::too_many_arguments,
        reason = "the device and queue, the two forms of the world, the extent, the settings and the progress are all independent"
    )]
    pub fn bake_until(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gpu: &mut GpuWorld,
        world: &vxe_world::World,
        extent: vxe_world::ChunkExtent,
        settings: &crate::bake::BakeSettings,
        progress: &mut dyn FnMut(&crate::bake::BakeProgress),
    ) -> Result<crate::bake::BakeReport, crate::bake::BakeError> {
        self.fit_store(true);
        hold_reach(device, queue, gpu, world, extent)?;
        self.dig_boxes.clear();
        let params = self.bake_params(gpu, settings);
        let world_bind = self.world_bind(device, gpu);
        if self.bake.is_none() {
            self.bake = Some(crate::bake::BakePipeline::new(device, &self.world_layout));
        }
        let run = crate::bake::BakeRun {
            device,
            queue,
            gpu,
            world,
            pipeline: self.bake.as_ref().expect("built above"),
            world_bind: &world_bind,
            params_buffer: &self.params,
            params,
        };
        crate::bake::run(&run, &mut self.store, extent, settings, progress)
    }

    /// Starts the live queue (ADR-0046's scheduler): the store emptied for `gpu`'s window, every
    /// block with brick records of `settings`' cell, filled by [`Frame::bake_queue_step`] a slice
    /// of `slice_ms` a frame; the first allocates the pool ([`Lighting::bake_pool_bytes`]).
    pub fn start_bake_queue(
        &mut self,
        queue: &wgpu::Queue,
        gpu: &GpuWorld,
        settings: crate::bake::BakeSettings,
        slice_ms: f64,
    ) {
        self.fit_store(true);
        self.store.start_queue(queue, gpu.window(), settings.cell);
        self.dig_boxes.clear();
        self.bake_queue = Some(crate::bake::BakeQueue::new(settings, slice_ms));
    }

    /// The lighting grid's pixels: what the store's missing count is a share of.
    pub fn light_pixels(&self) -> u32 {
        let [w, h] = self.targets.light_size();
        w * h
    }

    /// The live queue, if started.
    pub fn bake_queue(&self) -> Option<&crate::bake::BakeQueue> {
        self.bake_queue.as_ref()
    }

    /// The live queue, if started, to drain its readings
    /// ([`crate::bake::BakeQueue::take_angle_needs`]).
    pub fn bake_queue_mut(&mut self) -> Option<&mut crate::bake::BakeQueue> {
        self.bake_queue.as_mut()
    }

    /// The live queue's slice, in milliseconds a frame ([`crate::bake::Budget::PerFrame`]).
    pub fn set_bake_slice(&mut self, ms: f64) {
        self.set_bake_budget(crate::bake::Budget::PerFrame { ms });
    }

    /// The live queue's bake budget (ADR-0052).
    pub fn set_bake_budget(&mut self, budget: crate::bake::Budget) {
        if let Some(q) = self.bake_queue.as_mut() {
            q.budget = budget;
        }
    }

    /// The live queue's coarse pass: `samples` a brick direction (0 none) for a chunk's first
    /// block beyond `near` chunks (Chebyshev) of the camera's.
    pub fn set_bake_coarse(&mut self, samples: u32, near: i32) {
        if let Some(q) = self.bake_queue.as_mut() {
            q.coarse_samples = samples;
            q.near = near;
            q.full_first = Some(near);
        }
    }

    /// The live queue's full-first radius ([`crate::bake::BakeQueue::full_first`]): the chunks
    /// (Chebyshev) around the camera's within which a chunk with no block takes the full count
    /// directly; `None` gives every such chunk the coarse pass first.
    pub fn set_bake_full_first(&mut self, radius: Option<i32>) {
        if let Some(q) = self.bake_queue.as_mut() {
            q.full_first = radius;
        }
    }

    /// The live queue's reach mode ([`crate::bake::BakeQueue::reach_mode`]): whether a chunk
    /// whose reach crosses the streamer's full radius bakes from the chunks resident full.
    pub fn set_bake_reach(&mut self, mode: crate::bake::ReachMode) {
        if let Some(q) = self.bake_queue.as_mut() {
            q.set_reach_mode(mode);
        }
    }

    /// The live queue's digs: the slice's growth while a dig's bricks wait, and whether a dig
    /// that touched lit bricks re-bakes the chunks within the light radius (ring 3).
    pub fn set_bake_digs(&mut self, edit_ms: f64, reach_ring: bool) {
        if let Some(q) = self.bake_queue.as_mut() {
            q.edit_ms = edit_ms;
            q.reach_ring = reach_ring;
        }
    }

    /// Whether each of `chunks` has had the live queue's first pass
    /// ([`crate::bake::BakeQueue::passed`]); true without the queue.
    pub fn bake_passed(&self, chunks: &[ChunkCoord]) -> bool {
        self.bake_queue
            .as_ref()
            .is_none_or(|q| chunks.iter().all(|&c| q.passed(&self.store, c)))
    }

    /// Hands the live queue the stream workers' prepared masks ([`crate::bake::FacePrep`]).
    pub fn take_prepared(
        &mut self,
        prepared: &[(ChunkCoord, vxe_world::Prepared)],
        world: &vxe_world::World,
    ) {
        if let Some(q) = self.bake_queue.as_mut()
            && !prepared.is_empty()
        {
            q.take_prepared(prepared, world.materials());
        }
    }

    /// Whether `c` holds its block in the live queue's store, or is known to need none (`absent`:
    /// the world holds no chunk there). False without the queue.
    pub fn bake_settled(&self, c: ChunkCoord, absent: bool) -> bool {
        self.bake_queue
            .as_ref()
            .is_some_and(|q| q.settled(&self.store, c, absent))
    }

    /// One frame of the live queue, ahead of the encode: the light-sample ranges held out to
    /// `view.hold` (by default the window's far corner plus the light radius), then [`crate::bake::BakeQueue::step`], which
    /// waits on the GPU only without a slice (`view.samples`). `gpu` must already hold this frame's sync, submitted. `None` without
    /// the queue.
    pub fn bake_queue_step(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gpu: &mut GpuWorld,
        world: &vxe_world::World,
        view: &crate::bake::QueueView<'_>,
    ) -> Option<Result<crate::bake::QueueStats, crate::bake::BakeError>> {
        let t0 = crate::bake::stopwatch();
        let settings = self.bake_queue.as_ref()?.settings.clone();
        let w = gpu.window();
        let f = gpu.focus().0;
        let size = w.size();
        let far: i64 = (0..3)
            .map(|k| {
                let lo = i64::from(w.min.0[k] - f[k]).abs();
                let hi = i64::from(w.min.0[k] + size[k] - 1 - f[k]).abs();
                lo.max(hi).pow(2)
            })
            .sum();
        gpu.hold_lights(
            view.hold
                .unwrap_or((far as f64).sqrt().ceil() as i32 + gpu.light_radius() + 1),
        );
        let params = self.bake_params(gpu, &settings);
        let world_bind = self.queue_world_bind(device, gpu);
        if self.bake.is_none() {
            self.bake = Some(crate::bake::BakePipeline::new(device, &self.world_layout));
        }
        let run = crate::bake::BakeRun {
            device,
            queue,
            gpu,
            world,
            pipeline: self.bake.as_ref().expect("built above"),
            world_bind: &world_bind,
            params_buffer: &self.params,
            params,
        };
        let q = self.bake_queue.as_mut()?;
        Some(q.step(&run, &mut self.store, view, t0))
    }

    /// The frame parameters a bake dispatches with: no view but the LOD walk's camera chunk,
    /// which each face takes from its job.
    pub(super) fn bake_params(
        &self,
        gpu: &GpuWorld,
        settings: &crate::bake::BakeSettings,
    ) -> Params {
        let camera = Camera {
            position: [0.0, 0.0, 0.0].into(),
            yaw: 0.0,
            pitch: 0.0,
            vfov: 1.0,
        };
        let mut params = self.params(gpu, &camera, &LightParams::default());
        params.flags = FLAG_SHADOWS | FLAG_MASKS | FLAG_SKIPS;
        params.lighting = 0;
        params.history = 0;
        params.forced_level = 0;
        params.candidates = settings.candidates;
        params
    }

    /// Whether edit boxes wait for [`Frame::rebake_digs`].
    pub fn digs_pending(&self) -> bool {
        !self.dig_boxes.is_empty()
    }

    /// Re-bakes the ring of every edit box since the last call (ADR-0046's rings 1 and 2,
    /// [`crate::bake::dig_ring`]) to `settings`' stop, blocking, ahead of the next encode: the
    /// light-sample ranges are first held out to the piece's reach from the focus and synced.
    /// `gpu` must already mirror the edits. `None` when no box waits or the store holds no piece.
    pub fn rebake_digs(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gpu: &mut GpuWorld,
        world: &vxe_world::World,
        settings: &crate::bake::BakeSettings,
        progress: &mut dyn FnMut(&crate::bake::BakeProgress),
    ) -> Option<Result<crate::bake::DigReport, crate::bake::BakeError>> {
        if self.dig_boxes.is_empty() {
            return None;
        }
        let piece = self.store.piece()?;
        let boxes = std::mem::take(&mut self.dig_boxes);
        if let Err(e) = hold_reach(device, queue, gpu, world, piece) {
            return Some(Err(e));
        }
        let params = self.bake_params(gpu, settings);
        let world_bind = self.world_bind(device, gpu);
        if self.bake.is_none() {
            self.bake = Some(crate::bake::BakePipeline::new(device, &self.world_layout));
        }
        let run = crate::bake::BakeRun {
            device,
            queue,
            gpu,
            world,
            pipeline: self.bake.as_ref().expect("built above"),
            world_bind: &world_bind,
            params_buffer: &self.params,
            params,
        };
        Some(crate::bake::rebake(
            &run,
            &mut self.store,
            &boxes,
            settings,
            progress,
        ))
    }

    /// The edit boxes waiting for [`Frame::rebake_digs`], `min ..= max` in voxels.
    pub fn pending_digs(&self) -> &[[[i32; 3]; 2]] {
        &self.dig_boxes
    }

    /// Whether the temporal pass counts the store's reads (off by default): on, it runs the
    /// pipeline whose `COUNT_STORE` is true and [`Frame::read_store_stats`] and
    /// [`Frame::try_store_stats`] read its counts; off, the counters stay zero and the pass
    /// does no atomic write.
    pub fn set_count_store(&mut self, on: bool) {
        self.count_store = on;
    }

    /// Whether the temporal pass counts the store's reads ([`Frame::set_count_store`]).
    pub fn counts_store(&self) -> bool {
        self.count_store
    }

    /// The store's counters; `missing` is a blocking read of the last submitted encode's
    /// count (0 with [`Frame::set_count_store`] off), counted in
    /// [`Frame::store_blocking_reads`].
    pub fn read_store_stats(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> crate::bake::StoreStats {
        self.store_blocking_reads
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let bytes =
            crate::readback::read_buffer(device, queue, &self.store_counters, 0, STORE_STATS_BYTES);
        self.store_stats_of(&bytes)
    }

    /// Records a copy of the store's GPU counters into a read-back ring, for
    /// [`Frame::try_store_stats`]; call after [`Frame::encode`] in the same encoder, and submit
    /// it before the next read.
    pub fn copy_store_stats(&self, encoder: &mut wgpu::CommandEncoder) {
        self.store_ring.copy(encoder, &self.store_counters, 0);
    }

    /// The store's counters as of the newest [`Frame::copy_store_stats`] that has finished on
    /// the GPU, without waiting, and how many copies older than the last one recorded it is (1
    /// or 2 with the GPU keeping up and a copy a frame); `None` until the first has finished.
    pub fn try_store_stats(&self, device: &wgpu::Device) -> Option<(crate::bake::StoreStats, u64)> {
        self.store_ring.try_read(device, |b| self.store_stats_of(b))
    }

    /// The store's counts with its GPU counters' five words.
    pub(super) fn store_stats_of(&self, bytes: &[u8]) -> crate::bake::StoreStats {
        let word =
            |i: usize| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
        crate::bake::StoreStats {
            fallback: word(12),
            still_black: word(16),
            ..self.store.stats(word(0), word(4), word(8))
        }
    }

    /// What a cube face with no brick value reads (ADR-0053); [`crate::bake::MissingLight::Chain`]
    /// by default.
    pub fn set_missing_light(&mut self, queue: &wgpu::Queue, mode: crate::bake::MissingLight) {
        self.store.set_missing_light(queue, mode);
    }

    /// What a cube face with no brick value reads.
    pub fn missing_light(&self) -> crate::bake::MissingLight {
        self.store.missing_light()
    }

    /// How many times [`Frame::read_store_stats`] waited on the GPU so far.
    pub fn store_blocking_reads(&self) -> u64 {
        self.store_blocking_reads
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The store.
    pub fn store(&self) -> &crate::bake::Store {
        &self.store
    }

    /// `coord`'s block in the pool as its words, the directory first: a blocking read.
    pub fn read_store_block(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        coord: ChunkCoord,
    ) -> Option<Vec<u32>> {
        let (first, words) = self.store.block(coord)?;
        let bytes =
            crate::readback::read_buffer(device, queue, self.store.pool(), first * 4, words * 4);
        Some(bytemuck::pod_collect_to_vec(&bytes))
    }

    /// Rebuilds the store empty where its pool's size changes: [`Lighting::bake_pool_bytes`]
    /// within the device's storage binding, once `allocate` or a bake has allocated it; a minimal
    /// one before.
    pub(super) fn fit_store(&mut self, allocate: bool) {
        if !allocate && self.store.pool().size() <= crate::bake::MINIMAL_POOL_BYTES {
            return;
        }
        let limits = self.device.limits();
        let bytes = self
            .lighting
            .bake_pool_bytes
            .min(limits.max_storage_buffer_binding_size)
            .min(limits.max_buffer_size)
            & !3;
        if bytes != self.store.pool().size() {
            let missing = self.store.missing_light();
            self.store = crate::bake::Store::new(&self.device, bytes);
            // The new pool's header, the flags with it, is written at its first reset.
            self.store.keep_missing_light(missing);
        }
    }
}
