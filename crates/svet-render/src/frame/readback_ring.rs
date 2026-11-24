//! A ring of read-back buffers a frame copies into and reads later, once mapped.

/// Read-back buffers a [`ReadbackRing`] cycles through: with the GPU keeping up, a frame's
/// copy is mapped while the next two are recorded.
pub(crate) const READBACK_RING: usize = 3;

/// A ring slot's progress.
pub(super) enum RingPhase {
    Idle,
    /// Holds the copy of this index; not yet asked to map.
    Copied(u64),
    /// Asked to map the copy of this index: the flag reads 0 pending, 1 mapped, 2 failed.
    Mapping(u64, std::sync::Arc<std::sync::atomic::AtomicU8>),
}

pub(super) struct RingSlot {
    pub(super) buffer: wgpu::Buffer,
    pub(super) phase: RingPhase,
}

pub(super) struct RingState {
    pub(super) slots: Vec<RingSlot>,
    /// Copies recorded so far; the next copy's index.
    pub(super) copies: u64,
    /// The newest copy collected and its bytes (the vector is reused).
    pub(super) newest: Option<u64>,
    pub(super) bytes: Vec<u8>,
}

/// A ring of [`READBACK_RING`] mappable buffers that reads a GPU buffer's bytes without waiting:
/// [`ReadbackRing::copy`] records a copy into a free slot, [`ReadbackRing::try_read`] asks the
/// copied slots to map, polls without blocking and hands over the newest copy that has finished
/// mapping, a frame or two late. [`ReadbackRing::read`] is the blocking form, for tests and
/// tools.
///
/// Every encoder holding a [`ReadbackRing::copy`] must be submitted before the next
/// [`ReadbackRing::try_read`] or [`ReadbackRing::read`]: those ask the copied slots to map, and
/// wgpu refuses a submit that uses a buffer with a map pending. When every slot is mapping (the
/// GPU three copies behind) a copy is skipped; when none is free but one holds a copy not yet
/// asked to map, the oldest such is overwritten.
pub(crate) struct ReadbackRing {
    pub(super) state: std::sync::Mutex<RingState>,
    pub(super) size: u64,
}

impl ReadbackRing {
    /// A ring of `size`-byte slots (`size` a multiple of 4, as a buffer copy needs).
    pub(crate) fn new(device: &wgpu::Device, label: &str, size: u64) -> Self {
        let slots = (0..READBACK_RING)
            .map(|_| RingSlot {
                buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }),
                phase: RingPhase::Idle,
            })
            .collect();
        Self {
            state: std::sync::Mutex::new(RingState {
                slots,
                copies: 0,
                newest: None,
                bytes: Vec::with_capacity(size as usize),
            }),
            size,
        }
    }

    pub(super) fn lock(&self) -> std::sync::MutexGuard<'_, RingState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Records a copy of the ring's size from `src` at `offset` into a slot; first collects the
    /// slots that finished mapping. Returns the copy's index, or `None` when it was skipped.
    pub(crate) fn copy(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        src: &wgpu::Buffer,
        offset: u64,
    ) -> Option<u64> {
        let mut st = self.lock();
        st.collect();
        let at = st.copies;
        let oldest_copied = st
            .slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| match s.phase {
                RingPhase::Copied(c) => Some((c, i)),
                _ => None,
            })
            .min()
            .map(|(_, i)| i);
        let slot = st
            .slots
            .iter()
            .position(|s| matches!(s.phase, RingPhase::Idle))
            .or(oldest_copied)?;
        encoder.copy_buffer_to_buffer(src, offset, &st.slots[slot].buffer, 0, self.size);
        st.slots[slot].phase = RingPhase::Copied(at);
        st.copies += 1;
        Some(at)
    }

    /// Asks the copied slots to map, polls `device` without waiting, collects the finished
    /// ones, and returns `f` of the newest copy collected so far with its age in copies (0 for
    /// the last one recorded); `None` until the first has finished mapping.
    pub(crate) fn try_read<T>(
        &self,
        device: &wgpu::Device,
        f: impl FnOnce(&[u8]) -> T,
    ) -> Option<(T, u64)> {
        let mut st = self.lock();
        st.request_maps();
        let _ = device.poll(wgpu::PollType::Poll);
        st.collect();
        st.newest_with(f)
    }

    /// As [`ReadbackRing::try_read`], but waits for the GPU first, so the newest copy is the
    /// last one recorded (age 0) unless it was skipped.
    pub(crate) fn read<T>(
        &self,
        device: &wgpu::Device,
        f: impl FnOnce(&[u8]) -> T,
    ) -> Option<(T, u64)> {
        let mut st = self.lock();
        st.request_maps();
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("device poll for the read-back ring");
        st.collect();
        st.newest_with(f)
    }
}

impl RingState {
    /// Asks every slot holding a copy to map.
    pub(super) fn request_maps(&mut self) {
        for slot in &mut self.slots {
            if let RingPhase::Copied(at) = slot.phase {
                let done = std::sync::Arc::new(std::sync::atomic::AtomicU8::new(0));
                let flag = done.clone();
                slot.buffer.map_async(wgpu::MapMode::Read, .., move |r| {
                    flag.store(
                        if r.is_ok() { 1 } else { 2 },
                        std::sync::atomic::Ordering::Release,
                    );
                });
                slot.phase = RingPhase::Mapping(at, done);
            }
        }
    }

    /// Takes the bytes of every slot that finished mapping, keeping the newest copy's, and
    /// frees those slots.
    pub(super) fn collect(&mut self) {
        for slot in &mut self.slots {
            let RingPhase::Mapping(at, done) = &slot.phase else {
                continue;
            };
            let at = *at;
            match done.load(std::sync::atomic::Ordering::Acquire) {
                0 => continue,
                1 => {
                    if self.newest.is_none_or(|n| at > n) {
                        let bytes = slot
                            .buffer
                            .get_mapped_range(..)
                            .expect("read-back slot mapped");
                        self.bytes.clear();
                        self.bytes.extend_from_slice(&bytes);
                        self.newest = Some(at);
                    }
                    slot.buffer.unmap();
                }
                _ => {}
            }
            slot.phase = RingPhase::Idle;
        }
    }

    pub(super) fn newest_with<T>(&self, f: impl FnOnce(&[u8]) -> T) -> Option<(T, u64)> {
        let at = self.newest?;
        Some((f(&self.bytes), self.copies - 1 - at))
    }
}
