//! The job loop's render half (ADR-0083): what the bake half plans goes out from here and what
//! the GPU drew comes back. The bake half ([`JobLoop`]) plans a submit's rounds and the entry
//! table's bytes ([`Prepared`]) and places what is read back ([`ReadBack`]); the render half
//! ([`RenderHalf`]) writes the candidate tables, the job uniform, the faces and the
//! parameters, encodes and submits, maps the results, and applies the store's write lists. In
//! lockstep the bake half calls the render half inside the frame's call, at the points the
//! unsplit step made those calls; a test calls a recording half with no device.

use super::*;

/// What the render half reads of the frame: the device, the mirror (the candidate tables' light
/// ranges), the bake's pipeline and groups, the frame's parameters and the store's pool.
pub(crate) struct GpuRun<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub gpu: &'a GpuWorld,
    pub pipeline: &'a BakePipeline,
    pub world_bind: &'a wgpu::BindGroup,
    pub params_buffer: &'a wgpu::Buffer,
    pub params: Params,
    pub pool: &'a wgpu::Buffer,
}

/// A submit the bake half planned: the entry table's rows and what the job uniform reads.
pub(crate) struct Prepared {
    pub input: Vec<[u32; FACE_WORDS]>,
    pub seed: u32,
    pub clamp: bool,
    /// [`BakeRun::sky`] and [`BakeRun::key`] as the bake half read them.
    pub sky: u32,
    pub key: Option<([f32; 3], f32)>,
    /// A timer the caller's settings time the dispatch on, where the loop does not time its own.
    pub timer: Option<Arc<crate::frame::PassTimer>>,
}

/// A submit read back: the entries' words as the GPU left them, whether the map succeeded, the
/// wall from the submit, the dispatch's GPU milliseconds on the loop's timestamps, and on the
/// caller's timer.
pub(crate) struct ReadBack {
    pub words: Vec<u32>,
    pub mapped: bool,
    pub seconds: f64,
    pub gpu_ms: Option<f64>,
    pub timer_ms: Option<f64>,
}

/// The render half as the bake half calls it.
pub(crate) trait RenderHalf {
    /// Lets the device run its callbacks, waiting for nothing.
    fn poll(&mut self);
    /// Writes each `(slot, chunk)`'s candidate table, in order: the jobs opened since the last.
    fn tables(&mut self, opened: &[(usize, ChunkCoord)]);
    /// Writes the open jobs' candidate tables again where the mirror's light ranges moved since
    /// they were last written.
    fn refresh_tables(&mut self, open: &[(usize, ChunkCoord)]);
    /// Writes the frame's parameters the bake dispatches with.
    fn write_params(&mut self);
    /// Issues a planned submit.
    fn issue(&mut self, p: Prepared);
    /// The oldest submit's read-back once its results are mapped; it waits for the device
    /// while more than `unread` submits are unread. `None` when it stops.
    fn collect(&mut self, unread: usize) -> Option<ReadBack>;
    /// The bytes a frame applies at most ([`Lists::cap`]): the bake half closes its lists under
    /// it.
    fn cap(&self) -> Option<usize>;
    /// Applies the step's write lists ([`Lists`]): the landing words written, by chunk.
    fn apply(&mut self, lists: Vec<WriteList>, frame: u64) -> Vec<([i32; 3], u32)>;
}

/// A submit not read back yet: the buffer its results are copied to, mapped when `ready` reads 1.
pub(super) struct InFlight {
    pub(super) count: u64,
    pub(super) staging: wgpu::Buffer,
    pub(super) ready: Arc<std::sync::atomic::AtomicU8>,
    pub(super) timed: bool,
    pub(super) timer: Option<Arc<crate::frame::PassTimer>>,
    pub(super) t: Instant,
}

/// The loop's own timestamps, resolved into each submit's readback.
pub(super) struct LoopTimer {
    pub(super) queries: wgpu::QuerySet,
    pub(super) resolve: wgpu::Buffer,
    pub(super) period_ns: f32,
}

/// The render half's lasting state: the readback buffers, the loop's timer, the submits in
/// flight and the light epoch the candidate tables were last written at.
#[derive(Default)]
pub(crate) struct Issuer {
    pub(super) staging: Vec<wgpu::Buffer>,
    pub(super) in_flight: VecDeque<InFlight>,
    /// Whether the loop times its submits on its own timestamps, and the timer once made
    /// (`Some(None)`: the device has no timestamps).
    pub(super) timed: bool,
    pub(super) timer: Option<Option<LoopTimer>>,
    pub(super) tables_at: Option<u64>,
    /// Read-backs collected for a bake thread that stopped before it read them: handed out
    /// first, in order, by the next collect.
    pub(super) returned: VecDeque<ReadBack>,
    /// The most readback buffers held at once, free and in flight (a reading).
    pub(crate) peak_buffers: usize,
    /// Submits issued so far (a reading: [`crate::Frame::bake_frame_issue`]).
    pub(crate) issued: u64,
}

impl Issuer {
    /// An issuer that times each submit on its own GPU timestamps where the device has them
    /// ([`SubmitDone::gpu_ms`]).
    pub(crate) fn timed() -> Self {
        Self {
            timed: true,
            ..Self::default()
        }
    }

    /// Read-backs taken out of flight that the bake half has not read: the next collects hand
    /// them out first.
    pub(crate) fn give_back(&mut self, read_backs: Vec<ReadBack>) {
        self.returned.extend(read_backs);
    }
}

/// The frame's render half: `wgpu` calls on `run`, the issuer's state, and the write lists.
pub(crate) struct GpuHalf<'a> {
    pub run: &'a GpuRun<'a>,
    pub issuer: &'a mut Issuer,
    pub lists: &'a mut Lists,
}

impl RenderHalf for GpuHalf<'_> {
    fn poll(&mut self) {
        let _ = wgpu_call(|| self.run.device.poll(wgpu::PollType::Poll));
    }

    fn tables(&mut self, opened: &[(usize, ChunkCoord)]) {
        for &(slot, coord) in opened {
            write_job_table(self.run, coord, slot);
        }
    }

    fn refresh_tables(&mut self, open: &[(usize, ChunkCoord)]) {
        let epoch = self.run.gpu.light_epoch();
        if self.issuer.tables_at == Some(epoch) {
            return;
        }
        self.issuer.tables_at = Some(epoch);
        self.tables(open);
    }

    fn write_params(&mut self) {
        let run = self.run;
        write(
            run.queue,
            run.params_buffer,
            0,
            bytemuck::bytes_of(&run.params),
            W_JOB,
        );
    }

    fn issue(&mut self, p: Prepared) {
        self.start_timer();
        self.issuer.issued += 1;
        self.send(p);
    }

    fn collect(&mut self, unread: usize) -> Option<ReadBack> {
        use std::sync::atomic::Ordering;
        let device = self.run.device;
        let issuer = &mut *self.issuer;
        if let Some(rb) = issuer.returned.pop_front() {
            return Some(rb);
        }
        loop {
            let front = issuer.in_flight.front()?;
            if front.ready.load(Ordering::Acquire) != 0 {
                break;
            }
            if issuer.in_flight.len() <= unread {
                return None;
            }
            let _ = wgpu_call(|| device.poll(wgpu::PollType::wait_indefinitely()));
        }
        let f = issuer.in_flight.pop_front().expect("front read above");
        let mapped = f.ready.load(Ordering::Acquire) == 1;
        let seconds = seconds_since(f.t);
        let size = STAMP_BYTES + f.count * FACE_BYTES;
        let bytes: Vec<u8> = if mapped {
            wgpu_call(|| {
                let b = f
                    .staging
                    .get_mapped_range(..size)
                    .expect("bake readback mapped")
                    .to_vec();
                f.staging.unmap();
                b
            })
        } else {
            vec![0; size as usize]
        };
        issuer.staging.push(f.staging);
        let gpu_ms = (f.timed && mapped).then(|| {
            let t: &[u64] = bytemuck::cast_slice(&bytes[..STAMP_BYTES as usize]);
            let period = issuer
                .timer
                .as_ref()
                .and_then(Option::as_ref)
                .map_or(1.0, |t| t.period_ns);
            t[1].saturating_sub(t[0]) as f64 * f64::from(period) / 1e6
        });
        let timer_ms = match (&f.timer, f.timed) {
            (Some(timer), false) => Some(timer.read_ms(device)),
            _ => None,
        };
        Some(ReadBack {
            words: bytemuck::pod_collect_to_vec(&bytes[STAMP_BYTES as usize..]),
            mapped,
            seconds,
            gpu_ms,
            timer_ms,
        })
    }

    fn cap(&self) -> Option<usize> {
        self.lists.cap
    }

    fn apply(&mut self, lists: Vec<WriteList>, frame: u64) -> Vec<([i32; 3], u32)> {
        if self.lists.take(lists).is_err() {
            // A landing list over the cap: the maker never makes one (it closes a list before a
            // unit would pass the cap and sends a larger unit as an edit's).
            debug_assert!(false, "a write list over the frame's cap");
        }
        let mut out = PoolApplier {
            queue: self.run.queue,
            pool: self.run.pool,
        };
        let landed = self.lists.apply(frame, &mut out).1;
        // The step writes them into the store's copy at once.
        self.lists.confirm_all();
        landed
    }
}

impl GpuHalf<'_> {
    /// The loop's own timestamp queries, made at its first timed dispatch where the device
    /// has them.
    fn start_timer(&mut self) {
        let (device, queue) = (self.run.device, self.run.queue);
        if self.issuer.timed && self.issuer.timer.is_none() {
            self.issuer.timer = Some(
                device
                    .features()
                    .contains(wgpu::Features::TIMESTAMP_QUERY)
                    .then(|| LoopTimer {
                        queries: device.create_query_set(&wgpu::QuerySetDescriptor {
                            label: Some("bake loop timer"),
                            ty: wgpu::QueryType::Timestamp,
                            count: 2,
                        }),
                        resolve: device.create_buffer(&wgpu::BufferDescriptor {
                            label: Some("bake loop timer resolve"),
                            size: STAMP_BYTES,
                            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                            mapped_at_creation: false,
                        }),
                        period_ns: queue.get_timestamp_period(),
                    }),
            );
        }
    }

    /// Writes the dispatch's input, records the pass and its readback, submits and maps it,
    /// and puts it in flight.
    fn send(&mut self, p: Prepared) {
        let run = self.run;
        let (device, queue) = (run.device, run.queue);
        let count = p.input.len() as u32;
        let (key_dir, key_radius) = p.key.unwrap_or(([0.0; 3], 0.0));
        let uniform = [
            count,
            p.seed,
            ((run.gpu.light_radius() * 64) as f32).to_bits(),
            u32::from(p.clamp),
            p.sky,
            u32::from(p.key.is_some()),
            0,
            0,
            key_dir[0].to_bits(),
            key_dir[1].to_bits(),
            key_dir[2].to_bits(),
            key_radius.to_bits(),
        ];
        write(
            queue,
            &run.pipeline.job,
            0,
            bytemuck::cast_slice(&uniform),
            W_JOB,
        );
        write(
            queue,
            &run.pipeline.faces,
            0,
            bytemuck::cast_slice(&p.input),
            W_JOB,
        );
        // The encode, the submit and the map's request: `wgpu` calls, the render thread's
        // (ADR-0083).
        let calls_at = stopwatch();
        let issuer = &mut *self.issuer;
        let own = issuer.timer.as_ref().and_then(Option::as_ref);
        let size = STAMP_BYTES + u64::from(count) * FACE_BYTES;
        let staging = match issuer.staging.iter().position(|b| b.size() >= size) {
            Some(k) => issuer.staging.swap_remove(k),
            None => device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("bake readback"),
                size: STAMP_BYTES + BAKE_BATCH_FACES as u64 * FACE_BYTES,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
        };
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let writes = match own {
                Some(t) => Some(wgpu::ComputePassTimestampWrites {
                    query_set: &t.queries,
                    beginning_of_pass_write_index: Some(0),
                    end_of_pass_write_index: Some(1),
                }),
                None => p.timer.as_ref().map(|t| t.writes()),
            };
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("bake"),
                timestamp_writes: writes,
            });
            pass.set_pipeline(&run.pipeline.pipeline);
            pass.set_bind_group(0, run.world_bind, &[]);
            pass.set_bind_group(3, run.pipeline.bind(), &[]);
            pass.dispatch_workgroups(count.min(BAKE_ROW), count.div_ceil(BAKE_ROW), 1);
        }
        match own {
            Some(t) => {
                enc.resolve_query_set(&t.queries, 0..2, &t.resolve, 0);
                enc.copy_buffer_to_buffer(&t.resolve, 0, &staging, 0, STAMP_BYTES);
            }
            None => {
                if let Some(timer) = &p.timer {
                    timer.resolve(&mut enc);
                }
            }
        }
        enc.copy_buffer_to_buffer(
            &run.pipeline.faces,
            0,
            &staging,
            STAMP_BYTES,
            u64::from(count) * FACE_BYTES,
        );
        let t = stopwatch();
        queue.submit([enc.finish()]);
        let ready = Arc::new(std::sync::atomic::AtomicU8::new(0));
        let flag = ready.clone();
        staging.map_async(wgpu::MapMode::Read, ..size, move |r| {
            flag.store(
                if r.is_ok() { 1 } else { 2 },
                std::sync::atomic::Ordering::Release,
            );
        });
        add_call_ms(seconds_since(calls_at) * 1e3);
        let timed = own.is_some();
        issuer.in_flight.push_back(InFlight {
            count: u64::from(count),
            staging,
            ready,
            timed,
            timer: p.timer,
            t,
        });
        issuer.peak_buffers = issuer
            .peak_buffers
            .max(issuer.staging.len() + issuer.in_flight.len());
    }
}

/// Stages `coord`'s candidate table and its chunk in table slot `slot`.
/// The table is a function of the mirror's light ranges, so its build counts with its writes
/// as the render thread's (ADR-0083).
pub(super) fn write_job_table(run: &GpuRun<'_>, coord: ChunkCoord, slot: usize) {
    let table = wgpu_call(|| job_table(run.gpu, coord));
    let used = (1 + table.count as usize) * 16;
    let at = slot as u64 * TABLE_SLOT_BYTES;
    let tables = &run.pipeline.tables;
    write(
        run.queue,
        tables,
        at,
        &bytemuck::bytes_of(&table)[..used],
        W_JOB,
    );
    let chunk = [coord.0[0], coord.0[1], coord.0[2], 0];
    write(
        run.queue,
        tables,
        at + (TABLE_SLOT_WORDS - 1) * 16,
        bytemuck::cast_slice(&chunk),
        W_JOB,
    );
}
