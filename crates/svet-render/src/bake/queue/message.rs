//! The frame's message to the bake half (ADR-0083): one owned value a step reads, and the render
//! half's mailbox the edits and the stream workers' results wait in until the next step.

use std::collections::HashSet;

use super::shadow::ShadowNotes;
use super::*;

/// Which chunks a streamed world holds full, as owned data the step reads on whichever thread
/// runs it: a snapshot of the streamer's full tier ([`Residency::of_chunks`]) or a rule of its
/// own ([`Residency::of_rule`], a rig's).
#[derive(Clone)]
pub struct Residency(Arc<dyn Fn(ChunkCoord) -> bool + Send + Sync>);

impl Residency {
    /// The chunks of `full` (the streamer's `full_chunks`, a shared snapshot).
    pub fn of_chunks(full: Arc<HashSet<ChunkCoord>>) -> Self {
        Self(Arc::new(move |c| full.contains(&c)))
    }

    /// The chunks `rule` holds.
    pub fn of_rule(rule: impl Fn(ChunkCoord) -> bool + Send + Sync + 'static) -> Self {
        Self(Arc::new(rule))
    }

    /// Whether `c` is resident full.
    pub fn holds(&self, c: ChunkCoord) -> bool {
        (self.0)(c)
    }

    pub(crate) fn as_fn(&self) -> &dyn Fn(ChunkCoord) -> bool {
        &*self.0
    }
}

impl QueueView {
    /// [`QueueView::resident`] as the walks read it.
    pub(crate) fn resident_fn(&self) -> Option<&dyn Fn(ChunkCoord) -> bool> {
        self.resident.as_ref().map(Residency::as_fn)
    }
}

/// The settings the frame sets each step, from its lighting and look parameters.
#[derive(Clone, Copy, Debug)]
pub(crate) struct StepSettings {
    pub column_wait: bool,
    pub key_step: f32,
    pub landing_fade: bool,
    pub dig_store_hold: bool,
    pub landed_word: Option<u32>,
    pub far_normal: [f32; 2],
}

/// A change to the bake half's knobs (the frame's `set_bake_*`, a caller's
/// [`crate::Frame::set_bake_queue`]): applied to the books in the step that reads its message,
/// in arrival order, never from the render thread directly (ADR-0083).
pub(crate) type KnobSet = Box<dyn FnOnce(&mut BakeQueue) + Send + Sync>;

/// What reached the render half between two steps, in arrival order: an order the books keep,
/// since an edit drops a chunk's masks and a result after it brings new ones.
pub(crate) enum Arrival {
    /// An edit's box (`min ..= max`, voxels).
    Box([[i32; 3]; 2]),
    /// The stream workers' prepared masks.
    Masks(Vec<(ChunkCoord, Arc<PreparedMasks>)>),
    /// A knob set.
    Set(KnobSet),
}

/// What the public reads must count of arrivals the books have not taken: the chunks within a
/// voxel of a box and those with masks.
#[derive(Default)]
struct Overlay {
    /// The chunks within a voxel of a box.
    near: HashSet<[i32; 3]>,
    /// The chunks with masks.
    masked: HashSet<[i32; 3]>,
}

impl Overlay {
    fn is_empty(&self) -> bool {
        self.near.is_empty() && self.masked.is_empty()
    }
}

/// The render half's mailbox: the arrivals since the last message, which the next message
/// carries whole, and what the messages sent since the last applied list carried. The public
/// reads of the books consult it ([`super::QueueRead`]), so an edit reads as it did when its
/// box went into the books at once: the chunks within a voxel of a box not yet answered have no
/// known facelessness, seal or block reason, nor have those with masks not yet answered. A
/// message is answered once a write list made from it or a later one is applied
/// ([`Inbox::answered`]): the books that list came from had taken it.
#[derive(Default)]
pub(crate) struct Inbox {
    arrivals: Vec<Arrival>,
    waiting: Overlay,
    /// The messages sent and not yet answered: their frame and their overlay.
    sent: std::collections::VecDeque<(u64, Overlay)>,
}

/// The chunks within a voxel of the box `min ..= max`.
pub(super) fn box_near(min: [i32; 3], max: [i32; 3]) -> impl Iterator<Item = [i32; 3]> {
    let lo = min.map(|v| (v - 1).div_euclid(64));
    let hi = max.map(|v| (v + 1).div_euclid(64));
    (lo[2]..=hi[2]).flat_map(move |z| {
        (lo[1]..=hi[1]).flat_map(move |y| (lo[0]..=hi[0]).map(move |x| [x, y, z]))
    })
}

impl Inbox {
    /// An edit's box (`min ..= max`, voxels) into the next message, which the step takes first:
    /// the chunks within a voxel of it lose their prepared masks and their known facelessness,
    /// and a running job's block for them is thrown away; the box's rings are read in that step.
    pub(crate) fn push_box(&mut self, min: [i32; 3], max: [i32; 3]) {
        self.waiting.near.extend(box_near(min, max));
        self.arrivals.push(Arrival::Box([min, max]));
    }

    /// The stream workers' results into the next message; one made under other classes than
    /// the step's material table is dropped there.
    pub(crate) fn push_masks(&mut self, prepared: &[(ChunkCoord, svet_world::Prepared)]) {
        let masks: Vec<_> = prepared
            .iter()
            .filter_map(|(c, p)| Some((*c, p.0.clone().downcast::<PreparedMasks>().ok()?)))
            .collect();
        if !masks.is_empty() {
            self.waiting.masked.extend(masks.iter().map(|(c, _)| c.0));
            self.arrivals.push(Arrival::Masks(masks));
        }
    }

    /// A knob set into the next message.
    pub(crate) fn push_set(&mut self, set: KnobSet) {
        self.arrivals.push(Arrival::Set(set));
    }

    /// The arrivals, for the message of `frame`; what they carry stays counted until a list
    /// made from that message or a later one is applied.
    pub(crate) fn take(&mut self, frame: u64) -> Vec<Arrival> {
        let o = std::mem::take(&mut self.waiting);
        if !o.is_empty() {
            self.sent.push_back((frame, o));
        }
        std::mem::take(&mut self.arrivals)
    }

    /// A list made from the message of `frame` was applied: every message up to it is answered.
    pub(crate) fn answered(&mut self, frame: u64) {
        while self.sent.front().is_some_and(|(f, _)| *f <= frame) {
            self.sent.pop_front();
        }
    }

    fn overlays(&self) -> impl Iterator<Item = &Overlay> {
        std::iter::once(&self.waiting).chain(self.sent.iter().map(|(_, o)| o))
    }

    /// Whether an edit's box waits or is not yet answered.
    pub(crate) fn has_box(&self) -> bool {
        self.overlays().any(|o| !o.near.is_empty())
    }

    /// Whether `c` lies within a voxel of a box not yet answered.
    pub(crate) fn near(&self, c: [i32; 3]) -> bool {
        self.overlays().any(|o| o.near.contains(&c))
    }

    /// Whether a box or masks not yet answered will clear `c`'s facelessness and block reason.
    pub(crate) fn unknown(&self, c: [i32; 3]) -> bool {
        self.overlays()
            .any(|o| o.near.contains(&c) || o.masked.contains(&c))
    }
}

/// The frame's message: everything the bake half reads of the frame in a step, owned.
pub(crate) struct FrameMessage {
    /// The message's frame: the render half's count of messages since the queue started, one a
    /// frame call. A job's stamp, an edit's stale mark and a write list's name read it
    /// (ADR-0083); in lockstep it is the step's number.
    pub frame: u64,
    /// When the frame's call began: the time to the step is its setup stage.
    pub start: StepStart,
    pub view: QueueView,
    /// The mirror's change list with each changed chunk's state ([`super::notes`]).
    pub notes: ShadowNotes,
    /// [`BakeRun::sky`] and [`BakeRun::key`].
    pub sky: u32,
    pub key: Option<([f32; 3], f32)>,
    pub settings: StepSettings,
    pub arrivals: Vec<Arrival>,
    /// The landing words the render half wrote at apply since the last message, by chunk, for
    /// the store's copy ([`Store::landed`]); in lockstep the step takes them itself.
    pub landed: Vec<([i32; 3], u32)>,
}

impl FrameMessage {
    /// `self` followed by the `newer` message the bake half had not read yet, as one: the newest
    /// view, notes' state, sky, key and settings, the change lists, arrivals and landing words
    /// appended in order.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the bake thread's inbox coalesces; until it runs, the tests"
        )
    )]
    pub(crate) fn coalesce(mut self, newer: FrameMessage) -> FrameMessage {
        let FrameMessage {
            frame,
            start,
            view,
            notes,
            sky,
            key,
            settings,
            arrivals,
            landed,
        } = newer;
        self.notes = self.notes.then(notes);
        self.arrivals.extend(arrivals);
        self.landed.extend(landed);
        FrameMessage {
            frame,
            start,
            view,
            sky,
            key,
            settings,
            ..self
        }
    }
}

impl BakeQueue {
    /// The message's settings and arrivals into the books, ahead of everything else the step
    /// does: the books stand as they would have had each arrival gone in where it reached the
    /// frame.
    pub(super) fn take_message(
        &mut self,
        frame: u64,
        settings: StepSettings,
        arrivals: Vec<Arrival>,
        materials: &MaterialTable,
    ) {
        let s = settings;
        self.frame = frame;
        self.column_wait = s.column_wait;
        self.key_step = s.key_step;
        self.landing_fade = s.landing_fade;
        self.dig_store_hold = s.dig_store_hold;
        self.landed_word = s.landed_word;
        self.far_normal = s.far_normal;
        for a in arrivals {
            match a {
                Arrival::Box([min, max]) => self.take_box(min, max),
                Arrival::Masks(m) => self.take_masks(m, materials),
                Arrival::Set(set) => set(self),
            }
        }
    }

    /// A stream result: one made under other classes than `materials`' is dropped.
    fn take_masks(
        &mut self,
        prepared: Vec<(ChunkCoord, Arc<PreparedMasks>)>,
        materials: &MaterialTable,
    ) {
        let classes = self.classes(materials);
        let classes = classes.as_slice();
        for (c, m) in prepared {
            self.stats.hook_chunks += 1;
            self.stats.hook_ns += m.ns;
            self.stats.hook_max_ns = self.stats.hook_max_ns.max(m.ns);
            self.stats.seal_ns += m.seal_ns;
            self.stats.seal_max_ns = self.stats.seal_max_ns.max(m.seal_ns);
            if m.classes.as_slice() != classes {
                continue;
            }
            let changed = self.sealed.take(&m);
            self.touched.extend(changed);
            self.prepared_bytes += m.masks.bytes();
            if let Some(old) = self.prepared.insert(c.0, m) {
                self.prepared_bytes -= old.masks.bytes();
            }
            self.faceless.remove(&c.0);
            self.touched.insert(c.0);
            self.blocked.remove(&c.0);
        }
        if self.prepared_bytes > PREPARED_BYTES_MAX {
            // The farthest from the last step's eye go first, to nine tenths of the cap.
            let eye = self.eye;
            let mut by: Vec<(i64, [i32; 3])> = self
                .prepared
                .keys()
                .map(|c| {
                    let d: i64 = (0..3).map(|k| i64::from(c[k] - eye[k]).pow(2)).sum();
                    (d, *c)
                })
                .collect();
            by.sort_unstable();
            while self.prepared_bytes > PREPARED_BYTES_MAX / 10 * 9 {
                let Some((_, c)) = by.pop() else { break };
                if let Some(m) = self.prepared.remove(&c) {
                    self.prepared_bytes -= m.masks.bytes();
                }
            }
        }
    }

    /// An edit's box: the chunks within a voxel of it lose their prepared masks and their known
    /// facelessness, and a running job's block for them is thrown away; the box's rings are read
    /// in this step.
    fn take_box(&mut self, min: [i32; 3], max: [i32; 3]) {
        self.boxes.push([min, max]);
        self.sealed.forget(min, max);
        for c in box_near(min, max) {
            if let Some(m) = self.prepared.remove(&c) {
                self.prepared_bytes -= m.masks.bytes();
            }
            self.faceless.remove(&c);
            self.touched.insert(c);
            self.blocked.remove(&c);
            if self.lp.chunks().any(|j| j.0 == c) {
                // The box is in this step's message: it stales a job stamped before that.
                self.stale.insert(c, self.frame);
            }
        }
    }
}
