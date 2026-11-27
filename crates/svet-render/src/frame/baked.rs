//! The frame's link to the loaded light's store: its bakes, the live queue and the digs,
//! and the store's counters.

use super::*;

/// Bytes of the store's GPU counters that [`Frame::read_store_stats`] reads: five `u32` words
/// (missing, brick reads, coarse reads, fallback, still black).
pub(super) const STORE_STATS_BYTES: u64 = 20;

/// Bytes of the temporal pass's counter buffer: the store's five words, then the history
/// test's four ([`Frame::read_history_rejections`]).
pub(super) const COUNTER_BYTES: u64 = 36;

/// Holds the light-sample ranges out to every chunk of `extent`'s reach (its farthest corner from
/// the focus chunk, plus the light radius and one, for a job's box) and syncs until they settle.
pub(super) fn hold_reach(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    gpu: &mut GpuWorld,
    world: &svet_world::World,
    extent: svet_world::ChunkExtent,
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

/// The frame's link to the loaded light: the store, the bake's pipeline, the dig boxes, the
/// live queue and its world group, and the store's read counters and their readback.
pub(super) struct BakeLink {
    /// The store's books (minimal until the first bake or live queue,
    /// [`Lighting::bake_pool_bytes`]).
    pub(super) store: crate::bake::Store,
    /// The pool the store's writes land in: the render half's buffer (ADR-0083), the store's
    /// capacity in size.
    pub(super) pool: wgpu::Buffer,
    /// The bake's pipeline, built on the first bake.
    pub(super) pipeline: Option<crate::bake::BakePipeline>,
    /// The edit boxes since the last dig re-bake ([`Frame::rebake_digs`]), every one, while the
    /// store holds a piece.
    pub(super) dig_boxes: Vec<[[i32; 3]; 2]>,
    /// The live queue ([`Frame::start_bake_queue`]), when the store fills a frame at a time.
    pub(super) queue: Option<crate::bake::BakeQueue>,
    /// The render half's mailbox (ADR-0083): edits, stream results and knob sets since the last
    /// message, and what the messages not yet answered carried.
    pub(super) inbox: crate::bake::Inbox,
    /// The live queue's messages so far: the next one's frame is this plus one.
    pub(super) messages: u64,
    /// The render half's write lists waiting to apply (ADR-0083); in lockstep with no cap a
    /// step's lists apply in its own frame and none waits.
    pub(super) lists: crate::bake::Lists,
    /// The live queue's submits as the render half holds them: readback buffers, the loop's
    /// timer, the submits in flight.
    pub(super) issuer: crate::bake::Issuer,
    /// The bake half's shadow of the mirror, and the window and light key of its last notes
    /// (ADR-0083).
    pub(super) shadow: crate::bake::Shadow,
    pub(super) shadow_seen: (Option<svet_world::ChunkExtent>, Option<(u64, u64)>),
    /// The live queue's world group and the buffers it binds ([`Frame::queue_world_bind`]).
    pub(super) world_bind: Option<(Vec<wgpu::Buffer>, wgpu::BindGroup)>,
    /// The live queue's knobs, kept while no queue runs and handed to the next one.
    pub(super) knobs: crate::settings::BakeKnobs,
    /// Whether the temporal pass counts the store's reads ([`Frame::set_count_store`]).
    pub(super) count_store: bool,
    /// The store's five read counters (`counters` in `temporal.wgsl`): missing, brick reads,
    /// coarse reads, fallback, still black.
    pub(super) counters: wgpu::Buffer,
    /// Blocking reads of the store's counters ([`Frame::read_store_stats`]) so far.
    pub(super) blocking_reads: std::sync::atomic::AtomicU64,
    /// The store's counters, copied out by [`Frame::copy_store_stats`].
    pub(super) ring: ReadbackRing,
    /// The bake thread (ADR-0083), while it owns the queue's books, the store's copy and the
    /// shadow; `queue` is `None` and `store` a stand-in of the same capacity then.
    pub(super) thread: Option<crate::bake::ThreadLink>,
    /// Whether the live queue's sliced steps run on the bake thread ([`Frame::set_bake_thread`]).
    pub(super) thread_on: bool,
    /// The world the bake thread lists from ([`Frame::share_bake_world`]).
    pub(super) world: Option<std::sync::Arc<svet_world::World>>,
    /// What a stopped bake thread left: its render-half calls not made yet, and the message it
    /// had not read, merged into the next step's.
    pub(super) unissued: Vec<crate::bake::Op>,
    pub(super) leftover: Option<crate::bake::FrameMessage>,
    /// A bake thread's panic read outside a step, for the next step to return.
    pub(super) failed: Option<crate::bake::BakeError>,
    /// The encode's reads of the books' snapshot while the thread runs.
    pub(super) snapshot_reach: crate::bake::SnapshotReach,
    /// The readings [`Frame::bake_render_laps`] and [`Frame::bake_fade_frames`] return.
    pub(super) render_laps: Vec<[f32; 8]>,
    pub(super) fade_frames: [u64; 2],
    /// The edit's frames with the bake thread on: the wait for the dig's step in ms, the
    /// submits the frame issued and the bytes applied over the cap ([`Frame::bake_dig_waits`]).
    pub(super) dig_waits: Vec<[f32; 3]>,
    /// The last live-queue frame's wait for an edit's step in ms (0 with none, or in lockstep)
    /// and the submits it issued ([`Frame::bake_frame_issue`]).
    pub(super) last_issue: (f64, u64),
    /// The last live-queue frame's apply in ms and its bytes over the cap
    /// ([`Frame::bake_frame_apply`]).
    pub(super) last_apply: (f64, usize),
    /// The bake thread's depth in flight ([`Frame::set_bake_depth`]).
    pub(super) thread_depth: usize,
    /// The table slots the bake thread's steps open jobs into ([`Frame::set_bake_slots`]).
    pub(super) thread_slots: usize,
    /// The listing pool's threads at the bake thread's start ([`Frame::set_bake_listers`]).
    pub(super) thread_listers: usize,
    /// The reading [`Frame::bake_round_trip`] returns.
    pub(super) round_trip: crate::BakeRoundTrip,
}

/// The bake pool's buffer of `bytes`.
fn pool_buffer(device: &wgpu::Device, bytes: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("bake pool"),
        size: bytes,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

impl BakeLink {
    /// Writes the store's direct writes ([`crate::bake::Store::take_direct`]) into the pool,
    /// in order: the render half's, before the pool is next read.
    pub(super) fn flush(&mut self, queue: &wgpu::Queue) {
        for w in self.store.take_direct() {
            crate::bake::write(queue, &self.pool, w.offset, &w.data, w.kind);
        }
    }

    pub(super) fn new(device: &wgpu::Device) -> Self {
        Self {
            store: crate::bake::Store::new(crate::bake::MINIMAL_POOL_BYTES),
            pool: pool_buffer(device, crate::bake::MINIMAL_POOL_BYTES),
            pipeline: None,
            dig_boxes: Vec::new(),
            queue: None,
            inbox: crate::bake::Inbox::default(),
            messages: 0,
            lists: crate::bake::Lists::default(),
            issuer: crate::bake::Issuer::timed(),
            shadow: crate::bake::Shadow::default(),
            shadow_seen: (None, None),
            world_bind: None,
            knobs: crate::settings::BakeKnobs::default(),
            count_store: false,
            counters: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("store counters"),
                size: COUNTER_BYTES,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            blocking_reads: std::sync::atomic::AtomicU64::new(0),
            ring: ReadbackRing::new(device, "store stats readback", STORE_STATS_BYTES),
            thread: None,
            thread_on: false,
            world: None,
            unissued: Vec::new(),
            leftover: None,
            failed: None,
            snapshot_reach: Default::default(),
            render_laps: Vec::new(),
            fade_frames: [0; 2],
            dig_waits: Vec::new(),
            last_issue: (0.0, 0),
            last_apply: (0.0, 0),
            thread_depth: crate::THREAD_DEPTH,
            thread_slots: crate::THREAD_SLOTS,
            thread_listers: crate::ThreadPlan::here(None).listers,
            round_trip: Default::default(),
        }
    }
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
            self.state.params.clone(),
            world.pool_shard(0).clone(),
            world.pool_shard(1).clone(),
            world.pool_shard(2).clone(),
            world.pool_shard(3).clone(),
            world.records().clone(),
            world.table().clone(),
            world.materials().clone(),
            world.light_buffer().clone(),
            self.state.emission_scale.clone(),
            self.exposure.state_uniform().clone(),
        ];
        match &self.baked.world_bind {
            Some((k, bind)) if *k == key => bind.clone(),
            _ => {
                let bind = self.world_bind(device, world);
                self.baked.world_bind = Some((key, bind.clone()));
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
        world: &svet_world::World,
        extent: svet_world::ChunkExtent,
        settings: &crate::bake::BakeSettings,
        progress: &mut dyn FnMut(&crate::bake::BakeProgress),
    ) -> Result<crate::bake::BakeReport, crate::bake::BakeError> {
        self.join_bake(Some(queue));
        self.fit_store(true);
        hold_reach(device, queue, gpu, world, extent)?;
        self.baked.dig_boxes.clear();
        let params = self.bake_params(gpu, settings);
        let world_bind = self.world_bind(device, gpu);
        if self.baked.pipeline.is_none() {
            self.baked.pipeline = Some(crate::bake::BakePipeline::new(
                device,
                &self.pipes.world_layout,
            ));
        }
        // A one-shot bake reads no queue books, so no shadow of the mirror.
        let none = crate::bake::Shadow::default();
        let run = crate::bake::BakeRun {
            mirror: &none,
            world,
            sky: self.bake_sky(),
            key: self.bake_key(),
        };
        let gpu_run = crate::bake::GpuRun {
            device,
            queue,
            gpu,
            pipeline: self.baked.pipeline.as_ref().expect("built above"),
            world_bind: &world_bind,
            params_buffer: &self.state.params,
            params,
            pool: &self.baked.pool,
        };
        let report = crate::bake::run(
            &run,
            &gpu_run,
            &mut self.baked.store,
            extent,
            settings,
            progress,
        );
        self.baked.flush(queue);
        report
    }

    /// Starts the live queue (ADR-0046's scheduler): the store emptied for `gpu`'s window, every
    /// block with brick records of `settings`' cell, filled by [`Frame::bake_queue_step`] a slice
    /// of `slice_ms` a frame (the knobs' budget from then on), with the frame's other queue
    /// knobs ([`Frame::bake_knobs`]); the first allocates the pool ([`Lighting::bake_pool_bytes`]).
    pub fn start_bake_queue(
        &mut self,
        queue: &wgpu::Queue,
        gpu: &GpuWorld,
        settings: crate::bake::BakeSettings,
        slice_ms: f64,
    ) {
        // The old queue's books and what its thread made are dropped with it.
        self.join_bake(None);
        self.baked.lists = crate::bake::Lists::default();
        self.baked.unissued.clear();
        self.baked.leftover = None;
        self.baked.failed = None;
        self.fit_store(true);
        self.baked.store.start_queue(gpu.window(), settings.cell);
        self.baked.flush(queue);
        self.baked.dig_boxes.clear();
        // A new queue's loop has nothing in flight: the old one's submits are dropped with it.
        self.baked.issuer = crate::bake::Issuer::timed();
        // A new queue reads no arrival made for the old one; its messages count from 1.
        self.baked.inbox = crate::bake::Inbox::default();
        self.baked.messages = 0;
        let mut q = crate::bake::BakeQueue::new(settings, slice_ms);
        let k = &mut self.baked.knobs;
        k.budget = q.budget;
        (q.coarse_samples, q.near, q.full_first) = (k.coarse_samples, k.near, k.full_first);
        (q.edit_ms, q.reach_ring) = (k.edit_ms, k.dig_reach);
        q.ring1_stretch = k.ring1_stretch;
        q.tiers = k.tiers;
        q.drain = k.drain;
        q.set_reach_mode(k.reach);
        self.baked.queue = Some(q);
    }

    /// Builds the light bake's pipeline unless it is built, so the first bake step (the
    /// pre-roll's, the live queue's or a dig's) builds none: a driver that compiles it takes
    /// hundreds of ms. Call it during start-up; without it the first bake builds it.
    pub fn prepare_bake(&mut self, device: &wgpu::Device) {
        if self.baked.pipeline.is_none() {
            self.baked.pipeline = Some(crate::bake::BakePipeline::new(
                device,
                &self.pipes.world_layout,
            ));
        }
    }

    /// The lighting grid's pixels: what the store's missing count is a share of.
    pub fn light_pixels(&self) -> u32 {
        let [w, h] = self.targets.light_size();
        w * h
    }

    /// The live queue's books, if started, as the render thread reads them
    /// ([`crate::bake::QueueRead`]: with the mailbox's arrivals counted).
    pub fn bake_queue(&self) -> Option<crate::bake::QueueRead<'_>> {
        let src = match (&self.baked.queue, &self.baked.thread) {
            (Some(queue), _) => crate::bake::ReadSource::Live {
                queue,
                store: &self.baked.store,
            },
            (None, Some(link)) => crate::bake::ReadSource::Books(&link.books),
            (None, None) => return None,
        };
        Some(crate::bake::QueueRead {
            src,
            inbox: &self.baked.inbox,
        })
    }

    /// The live queue, if started, to drain its readings
    /// ([`crate::bake::BakeQueue::take_angle_needs`]). A change to its knobs goes through
    /// [`Frame::set_bake_queue`], which the bake half reads with its next message.
    pub fn bake_queue_mut(&mut self) -> Option<&mut crate::bake::BakeQueue> {
        self.baked.queue.as_mut()
    }

    /// Sets the live queue's knobs with `set`, in the step that reads the next message
    /// (ADR-0083: the render thread never writes the bake half's books). Nothing without the
    /// queue.
    pub fn set_bake_queue(
        &mut self,
        set: impl FnOnce(&mut crate::bake::BakeQueue) + Send + Sync + 'static,
    ) {
        if self.queue_running() {
            self.baked.inbox.push_set(Box::new(set));
        }
    }

    /// The live queue's bake budget (ADR-0052).
    pub fn set_bake_budget(&mut self, budget: crate::bake::Budget) {
        self.baked.knobs.budget = budget;
        self.set_bake_queue(move |q| q.budget = budget);
    }

    /// The live queue's coarse pass: `samples` a brick direction (0 none) for a chunk's first
    /// block beyond `near` chunks (Chebyshev) of the camera's; the full-first radius follows
    /// `near`.
    pub fn set_bake_coarse(&mut self, samples: u32, near: i32) {
        let k = &mut self.baked.knobs;
        (k.coarse_samples, k.near, k.full_first) = (samples, near, Some(near));
        self.set_bake_queue(move |q| {
            q.coarse_samples = samples;
            q.near = near;
            q.full_first = Some(near);
        });
    }

    /// The live queue's full-first radius ([`crate::bake::BakeQueue::full_first`]): the chunks
    /// (Chebyshev) around the camera's within which a chunk with no block takes the full count
    /// directly; `None` gives every such chunk the coarse pass first.
    pub fn set_bake_full_first(&mut self, radius: Option<i32>) {
        self.baked.knobs.full_first = radius;
        self.set_bake_queue(move |q| q.full_first = radius);
    }

    /// The live queue's reach mode ([`crate::bake::BakeQueue::reach_mode`]): whether a chunk
    /// whose reach crosses the streamer's full radius bakes from the chunks resident full.
    pub fn set_bake_reach(&mut self, mode: crate::bake::ReachMode) {
        self.baked.knobs.reach = mode;
        self.set_bake_queue(move |q| q.set_reach_mode(mode));
    }

    /// The live queue's sample tiers a brick direction by band, near, mid and far (ADR-0080:
    /// a chunk's record at its band's count by its size on screen).
    pub fn set_bake_tiers(&mut self, tiers: [u32; 3]) {
        self.baked.knobs.tiers = tiers;
        self.set_bake_queue(move |q| q.tiers = tiers);
    }

    /// The live queue's slice's two values (ADR-0080 rule 3): the drain value while a chunk in
    /// view waits for its tier's record, the line's once none does; `None` is the budget alone.
    pub fn set_bake_drain(&mut self, drain: Option<crate::bake::Drain>) {
        self.baked.knobs.drain = drain;
        self.set_bake_queue(move |q| q.drain = drain);
    }

    /// The live queue's digs: the slice's growth while a dig's bricks wait, and whether a dig
    /// that touched lit bricks re-bakes the chunks within the light radius (ring 3).
    pub fn set_bake_digs(&mut self, edit_ms: f64, reach_ring: bool) {
        (self.baked.knobs.edit_ms, self.baked.knobs.dig_reach) = (edit_ms, reach_ring);
        self.set_bake_queue(move |q| (q.edit_ms, q.reach_ring) = (edit_ms, reach_ring));
    }

    /// The live queue's ring-1 stretch ([`crate::bake::BakeQueue::ring1_stretch`]), clamped to
    /// [`crate::RING1_STRETCH_RANGE`]: a dig's ring 1 goes whole in a submit while its samples
    /// are within this many times the priced cap. It reaches the bake half in the next message.
    pub fn set_bake_ring1_stretch(&mut self, stretch: f64) {
        let r = crate::RING1_STRETCH_RANGE;
        let stretch = stretch.clamp(*r.start(), *r.end());
        self.baked.knobs.ring1_stretch = stretch;
        self.set_bake_queue(move |q| q.ring1_stretch = stretch);
    }

    /// The live queue's knobs: the queue's while it runs, else the ones the next queue starts
    /// with.
    pub fn bake_knobs(&self) -> crate::settings::BakeKnobs {
        self.baked.knobs
    }

    /// Whether each of `chunks` has had the live queue's first pass
    /// ([`crate::bake::BakeQueue::passed`]); true without the queue.
    pub fn bake_passed(&self, chunks: &[ChunkCoord]) -> bool {
        self.bake_queue()
            .is_none_or(|q| chunks.iter().all(|&c| q.passed(c)))
    }

    /// Hands the live queue the stream workers' prepared masks ([`crate::bake::FacePrep`]),
    /// into the next step's message; that step judges them against its world's material table,
    /// so `_world` is not read here.
    pub fn take_prepared(
        &mut self,
        prepared: &[(ChunkCoord, svet_world::Prepared)],
        _world: &svet_world::World,
    ) {
        if self.queue_running() && !prepared.is_empty() {
            self.baked.inbox.push_masks(prepared);
        }
    }

    /// Whether `c` holds its block in the live queue's store, or is known to need none (`absent`:
    /// the world holds no chunk there). False without the queue.
    pub fn bake_settled(&self, c: ChunkCoord, absent: bool) -> bool {
        self.bake_queue().is_some_and(|q| q.settled(c, absent))
    }

    /// [`Frame::bake_settled`] as the start's pre-roll reads it: a block at the full count whose
    /// only work is the catch-up it is owed counts as held
    /// ([`crate::bake::QueueRead::held_for_start`]). False without the queue.
    pub fn bake_held_for_start(&self, c: ChunkCoord, absent: bool) -> bool {
        self.bake_queue()
            .is_some_and(|q| q.held_for_start(c, absent))
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
        world: &svet_world::World,
        view: &crate::bake::QueueView,
    ) -> Option<Result<crate::bake::QueueStats, crate::bake::BakeError>> {
        let t0 = crate::bake::StepStart::now();
        if let Some(e) = self.baked.failed.take() {
            return Some(Err(e));
        }
        let issued_before = self.baked.issuer.issued;
        let out = self.bake_queue_frame(device, queue, gpu, world, view, t0);
        if out.is_some() {
            let wait = self
                .baked
                .thread
                .as_ref()
                .filter(|l| l.waited)
                .map_or(0.0, |l| l.laps[5]);
            self.baked.last_issue = (wait, self.baked.issuer.issued - issued_before);
            self.baked.last_apply = match &self.baked.thread {
                Some(l) => (l.laps[2], self.baked.lists.over),
                None => (
                    out.as_ref()
                        .and_then(|r| r.as_ref().ok())
                        .map_or(0.0, |s| s.stages.ms[crate::bake::S_APPLY]),
                    0,
                ),
            };
        }
        out
    }

    /// [`Frame::bake_queue_step`]'s body, after a panic read outside a step.
    fn bake_queue_frame(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gpu: &mut GpuWorld,
        world: &svet_world::World,
        view: &crate::bake::QueueView,
        t0: crate::bake::StepStart,
    ) -> Option<Result<crate::bake::QueueStats, crate::bake::BakeError>> {
        let settings = match (&self.baked.queue, &self.baked.thread) {
            (Some(q), _) => q.settings.clone(),
            (None, Some(link)) => link.settings.clone(),
            (None, None) => return None,
        };
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
        // The bind group is a `wgpu` call: the render thread's share of the step's setup.
        let world_bind = crate::bake::wgpu_call(|| self.queue_world_bind(device, gpu));
        if self.baked.pipeline.is_none() {
            self.baked.pipeline = Some(crate::bake::BakePipeline::new(
                device,
                &self.pipes.world_layout,
            ));
        }
        // The frame's message to the bake half, one owned value: the view, the mirror's changes
        // and its state for them, the sky and key, the settings, and the mailbox's arrivals.
        let (last_window, last_key) = &mut self.baked.shadow_seen;
        let notes = crate::bake::notes(&*gpu, gpu.take_changes(), *last_window, last_key);
        *last_window = notes.window;
        let lighting = &self.state.lighting;
        let settings = crate::bake::StepSettings {
            column_wait: lighting.column_wait,
            key_step: lighting.key_step,
            landing_fade: lighting.landing_fade,
            dig_store_hold: lighting.dig_hold == crate::DigHold::On,
            dig_estimate: lighting.dig_estimate,
            // The frame's number for a first block's landing word (a pre-roll's step, which
            // draws a fixed sample count, writes 0).
            landed_word: lighting.source_fade.then(|| {
                if view.samples.is_some() {
                    0
                } else {
                    self.state.frame.wrapping_add(1)
                }
            }),
            far_normal: self.state.look.far_normal,
        };
        let (sky, key) = (self.bake_sky(), self.bake_key());
        self.baked.messages += 1;
        let frame = self.baked.messages;
        let mut msg = crate::bake::FrameMessage {
            frame,
            start: t0,
            view: view.clone(),
            notes,
            sky,
            key,
            settings,
            arrivals: self.baked.inbox.take(frame),
            landed: self.baked.lists.report(frame),
        };
        // A sliced step with the setting on runs on the bake thread, once a stopped one's
        // leftovers have gone through a lockstep step.
        let sliced = view.samples.is_none();
        if self.baked.unissued.is_empty() && self.baked.leftover.is_none() {
            self.start_bake_thread(world, sliced);
        }
        if self.baked.thread.is_some() {
            return Some(self.thread_frame(device, queue, &*gpu, params, &world_bind, msg));
        }
        if let Some(old) = self.baked.leftover.take() {
            msg = old.coalesce(msg);
        }
        let timer = self
            .baked
            .queue
            .as_ref()
            .and_then(|q| q.settings.timer.clone());
        let gpu_run = crate::bake::GpuRun {
            device,
            queue,
            gpu,
            pipeline: self.baked.pipeline.as_ref().expect("built above"),
            world_bind: &world_bind,
            params_buffer: &self.state.params,
            params,
            pool: &self.baked.pool,
        };
        let mut half = crate::bake::GpuHalf {
            run: &gpu_run,
            issuer: &mut self.baked.issuer,
            lists: &mut self.baked.lists,
        };
        // A stopped bake thread's calls not made yet, ahead of this step's.
        crate::bake::replay(
            std::mem::take(&mut self.baked.unissued),
            &mut half,
            timer.as_ref(),
        );
        let q = self.baked.queue.as_mut()?;
        // The fade reading counts the sliced steps only, as the thread runs no other.
        if q.stats().fading > 0 && sliced {
            self.baked.fade_frames[0] += 1;
            self.baked.fade_frames[1] += 1;
        }
        let out = q.step(
            &mut self.baked.shadow,
            world,
            &mut self.baked.store,
            msg,
            &mut half,
        );
        // In lockstep the step's lists are applied in its call: the message is answered.
        self.baked.inbox.answered(frame);
        Some(out)
    }

    /// The bake's sky-ray bits ([`crate::bake::BakeRun::sky`]): none with no sky set, else on
    /// with the frame's coarse shadow walk and shadow styles.
    pub(super) fn bake_sky(&self) -> u32 {
        use crate::bake::{BAKE_SKY_COARSE, BAKE_SKY_END_IS_SKY, BAKE_SKY_ON, BAKE_SKY_STYLES};
        if self.sky.sky.is_none() {
            return 0;
        }
        BAKE_SKY_ON
            | if self.state.lighting.shadow_walk == ShadowWalk::Coarse {
                BAKE_SKY_COARSE
            } else {
                0
            }
            | if self.state.styles.is_empty() {
                0
            } else {
                BAKE_SKY_STYLES
            }
            | if self.state.hooks.sky_end_is_sky {
                BAKE_SKY_END_IS_SKY
            } else {
                0
            }
    }

    /// The key light the bake's key rays aim at (ADR-0079): the sky's key light above the
    /// horizon, its direction and angular radius; `None` with no sky or none up.
    pub(super) fn bake_key(&self) -> Option<([f32; 3], f32)> {
        let (_, body) = self.sky.sky.as_ref()?.key_light()?;
        Some((body.direction, body.angular_radius))
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
        !self.baked.dig_boxes.is_empty()
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
        world: &svet_world::World,
        settings: &crate::bake::BakeSettings,
        progress: &mut dyn FnMut(&crate::bake::BakeProgress),
    ) -> Option<Result<crate::bake::DigReport, crate::bake::BakeError>> {
        if self.baked.dig_boxes.is_empty() {
            return None;
        }
        self.join_bake(Some(queue));
        let piece = self.baked.store.piece()?;
        let boxes = std::mem::take(&mut self.baked.dig_boxes);
        if let Err(e) = hold_reach(device, queue, gpu, world, piece) {
            return Some(Err(e));
        }
        let params = self.bake_params(gpu, settings);
        let world_bind = self.world_bind(device, gpu);
        if self.baked.pipeline.is_none() {
            self.baked.pipeline = Some(crate::bake::BakePipeline::new(
                device,
                &self.pipes.world_layout,
            ));
        }
        // A one-shot bake reads no queue books, so no shadow of the mirror.
        let none = crate::bake::Shadow::default();
        let run = crate::bake::BakeRun {
            mirror: &none,
            world,
            sky: self.bake_sky(),
            key: self.bake_key(),
        };
        let gpu_run = crate::bake::GpuRun {
            device,
            queue,
            gpu,
            pipeline: self.baked.pipeline.as_ref().expect("built above"),
            world_bind: &world_bind,
            params_buffer: &self.state.params,
            params,
            pool: &self.baked.pool,
        };
        let report = crate::bake::rebake(
            &run,
            &gpu_run,
            &mut self.baked.store,
            &boxes,
            settings,
            progress,
        );
        self.baked.flush(queue);
        Some(report)
    }

    /// The edit boxes waiting for [`Frame::rebake_digs`], `min ..= max` in voxels.
    pub fn pending_digs(&self) -> &[[[i32; 3]; 2]] {
        &self.baked.dig_boxes
    }

    /// Whether the temporal pass counts the store's reads (off by default): on, it runs the
    /// pipeline whose `COUNT_STORE` is true and [`Frame::read_store_stats`] and
    /// [`Frame::try_store_stats`] read its counts; off, the counters stay zero and the pass
    /// does no atomic write.
    pub fn set_count_store(&mut self, on: bool) {
        self.baked.count_store = on;
    }

    /// Whether the temporal pass counts the store's reads ([`Frame::set_count_store`]).
    pub fn counts_store(&self) -> bool {
        self.baked.count_store
    }

    /// The temporal pass's history test over the last submitted encode, a blocking read (zeros
    /// with [`Frame::set_count_store`] off): the hits it tested (a hit with history on, outside
    /// an edit box), then of those whose history reset (no tap accepted, by the bilinear four
    /// or the 3 x 3 search), the nearest previous pixel's first failing test in the order
    /// material, normal, position (any other: no hit there, another thing, or a pixel off the
    /// grid).
    pub fn read_history_rejections(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> [u32; 4] {
        let bytes = crate::readback::read_buffer(
            device,
            queue,
            &self.baked.counters,
            STORE_STATS_BYTES,
            COUNTER_BYTES - STORE_STATS_BYTES,
        );
        std::array::from_fn(|i| {
            u32::from_le_bytes([
                bytes[4 * i],
                bytes[4 * i + 1],
                bytes[4 * i + 2],
                bytes[4 * i + 3],
            ])
        })
    }

    /// The light passes' reading hooks ([`LightHooks`]; all off by default).
    pub fn set_light_hooks(&mut self, hooks: LightHooks) {
        self.state.hooks = hooks;
    }

    /// The store's counters; `missing` is a blocking read of the last submitted encode's
    /// count (0 with [`Frame::set_count_store`] off), counted in
    /// [`Frame::store_blocking_reads`].
    pub fn read_store_stats(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> crate::bake::StoreStats {
        self.baked
            .blocking_reads
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let bytes =
            crate::readback::read_buffer(device, queue, &self.baked.counters, 0, STORE_STATS_BYTES);
        self.store_stats_of(&bytes)
    }

    /// Records a copy of the store's GPU counters into a read-back ring, for
    /// [`Frame::try_store_stats`]; call after [`Frame::encode`] in the same encoder, and submit
    /// it before the next read.
    pub fn copy_store_stats(&self, encoder: &mut wgpu::CommandEncoder) {
        self.baked.ring.copy(encoder, &self.baked.counters, 0);
    }

    /// The store's counters as of the newest [`Frame::copy_store_stats`] that has finished on
    /// the GPU, without waiting, and how many copies older than the last one recorded it is (1
    /// or 2 with the GPU keeping up and a copy a frame); `None` until the first has finished.
    pub fn try_store_stats(&self, device: &wgpu::Device) -> Option<(crate::bake::StoreStats, u64)> {
        self.baked.ring.try_read(device, |b| self.store_stats_of(b))
    }

    /// The store's counts with its GPU counters' five words.
    pub(super) fn store_stats_of(&self, bytes: &[u8]) -> crate::bake::StoreStats {
        let word =
            |i: usize| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
        let counts = match &self.baked.thread {
            Some(link) => crate::bake::StoreStats {
                missing: word(0),
                brick_reads: word(4),
                coarse_reads: word(8),
                ..link.books.store_stats
            },
            None => self.baked.store.stats(word(0), word(4), word(8)),
        };
        crate::bake::StoreStats {
            fallback: word(12),
            still_black: word(16),
            ..counts
        }
    }

    /// What a cube face with no brick value reads (ADR-0053); [`crate::bake::MissingLight::Chain`]
    /// by default.
    pub fn set_missing_light(&mut self, queue: &wgpu::Queue, mode: crate::bake::MissingLight) {
        self.join_bake(Some(queue));
        self.baked.store.set_missing_light(mode);
        self.baked.flush(queue);
    }

    /// What a cube face with no brick value reads.
    pub fn missing_light(&self) -> crate::bake::MissingLight {
        self.baked.store.missing_light()
    }

    /// How many times [`Frame::read_store_stats`] waited on the GPU so far.
    pub fn store_blocking_reads(&self) -> u64 {
        self.baked
            .blocking_reads
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The store's copy, lockstep's.
    ///
    /// # Panics
    ///
    /// With the bake thread running: the copy is its own; [`Frame::bake_queue`] and
    /// [`Frame::read_store_block`] read its snapshot.
    pub fn store(&self) -> &crate::bake::Store {
        assert!(
            self.baked.thread.is_none(),
            "the store's copy is the bake thread's while it runs"
        );
        &self.baked.store
    }

    /// `coord`'s block in the pool as its words, the directory first: blocking reads. The place
    /// is the one the pool's own chunk table names (what the frame reads), read back; the words
    /// are the books' where that is the books' place, else the directory's run. `None` where the
    /// table names no block or the books hold none.
    pub fn read_store_block(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        coord: ChunkCoord,
    ) -> Option<Vec<u32>> {
        let (books_first, books_words) = match &self.baked.thread {
            Some(link) => *link.books.blocks.get(&coord.0)?,
            None => self.baked.store.block(coord)?,
        };
        let read = |first: u64, words: u64| -> Vec<u32> {
            let bytes =
                crate::readback::read_buffer(device, queue, &self.baked.pool, first * 4, words * 4);
            bytemuck::pod_collect_to_vec(&bytes)
        };
        let first =
            u64::from(self.read_table_word(device, queue, coord)? & !crate::bake::TABLE_COARSE);
        if first == 0 {
            return None;
        }
        if first == books_first {
            return Some(read(first, books_words));
        }
        let dir = read(first, crate::bake::DIRECTORY_WORDS);
        let words = dir
            .iter()
            .map(|d| u64::from(d >> 11))
            .max()
            .unwrap_or(0)
            .max(crate::bake::BLOCK_HEAD_WORDS);
        Some(read(first, words))
    }

    /// `coord`'s word of the pool's chunk table, read back with the window the pool's header
    /// holds (a blocking read). `None` outside that window.
    pub fn read_table_word(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        coord: ChunkCoord,
    ) -> Option<u32> {
        let head = crate::readback::read_buffer(device, queue, &self.baked.pool, 0, 16 * 4);
        let head: Vec<u32> = bytemuck::pod_collect_to_vec(&head);
        let min = [0, 1, 2].map(|k| head[4 + k] as i32);
        let size = [0, 1, 2].map(|k| i64::from(head[8 + k]));
        let w = [0, 1, 2].map(|k| i64::from(coord.0[k]) - i64::from(min[k]));
        if (0..3).any(|k| w[k] < 0 || w[k] >= size[k]) {
            return None;
        }
        let index = w[0] + size[0] * (w[1] + size[1] * w[2]);
        let at = (crate::bake::STORE_TABLE + index as u64) * 4;
        let word = crate::readback::read_buffer(device, queue, &self.baked.pool, at, 4);
        Some(u32::from_le_bytes(word[..4].try_into().ok()?))
    }

    /// Rebuilds the store empty where its pool's size changes: [`Lighting::bake_pool_bytes`]
    /// within the device's storage binding, once `allocate` or a bake has allocated it; a minimal
    /// one before.
    pub(super) fn fit_store(&mut self, allocate: bool) {
        if !allocate && self.baked.store.capacity() <= crate::bake::MINIMAL_POOL_BYTES {
            return;
        }
        let limits = self.device.limits();
        let bytes = self
            .state
            .lighting
            .bake_pool_bytes
            .min(limits.max_storage_buffer_binding_size)
            .min(limits.max_buffer_size)
            & !3;
        if bytes != self.baked.store.capacity() {
            // The pool is made again: what the bake thread made for the old one is dropped.
            self.join_bake(None);
            self.baked.lists.waiting.clear();
            let missing = self.baked.store.missing_light();
            self.baked.store = crate::bake::Store::new(bytes);
            self.baked.pool = pool_buffer(&self.device, bytes);
            // The new pool's header, the flags with it, is written at its first reset.
            self.baked.store.keep_missing_light(missing);
        }
    }
}
