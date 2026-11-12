//! The pass timer: timestamp pairs a frame's passes write, resolved and read back.

use super::*;

/// Bytes one resolved timestamp occupies: a `u64` tick count.
pub(super) const QUERY_BYTES: u64 = 8;

/// GPU durations of one or more passes through a query set of two timestamps per pass: pass
/// `i` writes its start and end into timestamps `2 i` and `2 i + 1`, [`PassTimer::resolve`]
/// copies them out after the passes into a [`ReadbackRing`], [`PassTimer::try_read_all_ms`]
/// hands over the newest set that has finished without waiting, and
/// [`PassTimer::read_all_ms`] waits for the last.
///
/// The resolve buffer (`QUERY_RESOLVE | COPY_SRC`) and the ring's buffers (`COPY_DST |
/// MAP_READ`) are separate because wgpu forbids one buffer carrying both usages. One query set
/// and one resolve buffer serve every frame: the queue runs a frame's resolve and copy before
/// the next frame's passes write the queries again. The timestamp period is read off the queue
/// once, at construction.
pub struct PassTimer {
    pub(super) queries: wgpu::QuerySet,
    pub(super) resolve: wgpu::Buffer,
    pub(super) ring: ReadbackRing,
    /// The newest copy [`PassTimer::try_read_all_ms`] handed over, plus one; 0 before any.
    pub(super) handed: std::sync::atomic::AtomicU64,
    /// Nanoseconds per timestamp tick.
    pub(super) period_ns: f32,
    pub(super) passes: u32,
    /// Reads that waited on the GPU so far ([`PassTimer::read_all_ms`]).
    pub(super) blocking_reads: std::sync::atomic::AtomicU64,
}

/// The march pass's timer; the march and the HUD are timed alike.
pub type MarchTimer = PassTimer;

impl PassTimer {
    /// A timer of one pass, or `None` when the device lacks `Features::TIMESTAMP_QUERY` (the
    /// software adapter among them), so a caller there gets no timer rather than a validation
    /// error.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        Self::with_passes(device, queue, 1)
    }

    /// A timer of `passes` passes, such as [`Frame::encode`]'s [`FRAME_PASSES`]; `None` as
    /// [`PassTimer::new`].
    pub fn with_passes(device: &wgpu::Device, queue: &wgpu::Queue, passes: u32) -> Option<Self> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let bytes = 2 * QUERY_BYTES * u64::from(passes);
        Some(Self {
            queries: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("pass timer"),
                ty: wgpu::QueryType::Timestamp,
                count: 2 * passes,
            }),
            resolve: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pass timer resolve"),
                size: bytes,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            ring: ReadbackRing::new(device, "pass timer readback", bytes),
            handed: std::sync::atomic::AtomicU64::new(0),
            period_ns: queue.get_timestamp_period(),
            passes,
            blocking_reads: std::sync::atomic::AtomicU64::new(0),
        })
    }

    pub fn passes(&self) -> u32 {
        self.passes
    }

    /// How many reads waited on the GPU so far: [`PassTimer::read_all_ms`] and
    /// [`PassTimer::read_ms`] each count one.
    pub fn blocking_reads(&self) -> u64 {
        self.blocking_reads
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The timestamp writes for a compute pass timed as pass 0.
    pub fn writes(&self) -> wgpu::ComputePassTimestampWrites<'_> {
        self.writes_at(0)
    }

    /// The timestamp writes for a compute pass timed as pass `i`.
    pub fn writes_at(&self, i: u32) -> wgpu::ComputePassTimestampWrites<'_> {
        assert!(i < self.passes, "pass {i} of {}", self.passes);
        wgpu::ComputePassTimestampWrites {
            query_set: &self.queries,
            beginning_of_pass_write_index: Some(2 * i),
            end_of_pass_write_index: Some(2 * i + 1),
        }
    }

    /// The timestamp writes for a render pass timed as pass 0, such as the HUD overlay's.
    pub fn render_writes(&self) -> wgpu::RenderPassTimestampWrites<'_> {
        wgpu::RenderPassTimestampWrites {
            query_set: &self.queries,
            beginning_of_pass_write_index: Some(0),
            end_of_pass_write_index: Some(1),
        }
    }

    /// Records the resolve of every timestamp and their copy into the ring; call after the
    /// timed passes, in the same or a later encoder, and submit it before the next read.
    pub fn resolve(&self, encoder: &mut wgpu::CommandEncoder) {
        let n = 2 * self.passes;
        encoder.resolve_query_set(&self.queries, 0..n, &self.resolve, 0);
        self.ring.copy(encoder, &self.resolve, 0);
    }

    /// Milliseconds of pass 0 of the last resolve, waiting; see [`PassTimer::read_all_ms`].
    pub fn read_ms(&self, device: &wgpu::Device) -> f64 {
        self.read_all_ms(device)[0]
    }

    /// Milliseconds of pass 0 of the newest finished resolve, without waiting; see
    /// [`PassTimer::try_read_all_ms`].
    pub fn try_read_ms(&self, device: &wgpu::Device) -> Option<f64> {
        self.try_read_all_ms(device).map(|ms| ms[0])
    }

    /// Milliseconds between each pass's two timestamps of the last resolve, by pass; zero
    /// when they are out of order, and every pass zero before any resolve. Blocks until the
    /// GPU has finished the submission that resolved them, and counts in
    /// [`PassTimer::blocking_reads`].
    pub fn read_all_ms(&self, device: &wgpu::Device) -> Vec<f64> {
        self.blocking_reads
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.ring
            .read(device, |b| self.to_ms(b))
            .map_or_else(|| vec![0.0; self.passes as usize], |(ms, _)| ms)
    }

    /// As [`PassTimer::read_all_ms`] for the newest resolve that has finished on the GPU,
    /// without waiting: a non-blocking poll, then the set if it is newer than the last this
    /// call handed over, else `None` (none finished yet, or none new since). With the GPU
    /// keeping up the set is one or two frames old.
    pub fn try_read_all_ms(&self, device: &wgpu::Device) -> Option<Vec<f64>> {
        let (ms, age) = self.ring.try_read(device, |b| self.to_ms(b))?;
        let next = self.ring.lock().copies - age;
        let handed = self
            .handed
            .fetch_max(next, std::sync::atomic::Ordering::Relaxed);
        (next > handed).then_some(ms)
    }

    /// Milliseconds by pass from a resolve's timestamp bytes.
    pub(super) fn to_ms(&self, bytes: &[u8]) -> Vec<f64> {
        let t: &[u64] = bytemuck::cast_slice(bytes);
        t.chunks(2)
            .map(|p| p[1].saturating_sub(p[0]) as f64 * f64::from(self.period_ns) / 1.0e6)
            .collect()
    }
}
