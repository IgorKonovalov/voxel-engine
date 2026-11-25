//! The live queue on the bake thread (ADR-0083), the render thread's side: the setting, the
//! world handle the thread lists from, its start at the first sliced step, the frame (take the
//! outbox, issue its submits, apply its lists under the cap, send the message), and the stop
//! that gives the books back to lockstep.

use super::*;
use crate::bake::{
    BakeError, BakeSide, FrameMessage, GpuHalf, GpuRun, Lists, OUTBOX_BYTES, PoolApplier,
    QueueStats, THREAD_LIST_CAP, ThreadLink,
};
use std::sync::Arc;

/// The frames [`Frame::bake_render_laps`] keeps, about two minutes at 500 fps.
const READING_FRAMES: usize = 1 << 16;

impl Frame {
    /// Whether the live queue's sliced steps run on the bake thread (off: lockstep, each step
    /// to completion inside [`Frame::bake_queue_step`]). The pre-roll and the one-shot bakes run
    /// in lockstep either way. The thread starts at the next sliced step once the frame holds
    /// the world ([`Frame::share_bake_world`]); off stops and joins it, its books back here and
    /// its write lists applied.
    pub fn set_bake_thread(&mut self, queue: &wgpu::Queue, on: bool) {
        self.baked.thread_on = on;
        if !on {
            self.join_bake(Some(queue));
        }
    }

    /// Whether the live queue's sliced steps run on the bake thread ([`Frame::set_bake_thread`]).
    pub fn bake_thread(&self) -> bool {
        self.baked.thread_on
    }

    /// The submits the bake thread's steps hold unread at most, at least 1 (a reading's knob;
    /// [`crate::THREAD_DEPTH`] by default). Lockstep, the pre-roll and the one-shot bakes hold
    /// two whatever it is. It takes hold at the thread's next start.
    pub fn set_bake_depth(&mut self, depth: usize) {
        self.baked.thread_depth = depth.max(1);
    }

    /// The live queue's table slots the bake thread's steps open jobs into, 16 to 64 (a
    /// reading's knob; [`crate::THREAD_SLOTS`] by default). Lockstep and the pre-roll keep the
    /// last 16 whatever it is; at the thread's stop the loop returns to them, a job open in a
    /// slot outside keeping it until it closes. It takes hold at the thread's next start.
    pub fn set_bake_slots(&mut self, slots: usize) {
        self.baked.thread_slots = slots;
    }

    /// The bake thread's submits on the render side since the frame was made (a reading,
    /// [`crate::BakeRoundTrip`]).
    pub fn bake_round_trip(&self) -> crate::BakeRoundTrip {
        let mut r = self.baked.round_trip.clone();
        r.peak_buffers = r.peak_buffers.max(self.baked.issuer.peak_buffers);
        r
    }

    /// The frames encoded while the bake thread ran, those whose eye lay past the reach of the
    /// books' snapshot of the words under the step's eye, and those where the sky over the
    /// floor read nothing at the encode's eye but an answer at the step's, and the longest run
    /// of consecutive frames past the reach (a reading).
    pub fn bake_snapshot_reach(&self) -> [u64; 4] {
        self.baked.snapshot_reach.read()
    }

    /// The render thread's laps in each frame the bake thread ran, in ms (a reading): the
    /// message's build; the take, the submits issued, the lists applied, the read-backs
    /// collected, the send and an edit's frame's wait for its step; then the frame call's whole.
    pub fn bake_render_laps(&self) -> &[[f32; 8]] {
        &self.baked.render_laps
    }

    /// The edit's frames with the bake thread on (ADR-0083's one wait), each the wait for the
    /// dig's step in ms, the submits the frame issued and the bytes it applied over the frame's
    /// cap (the edit's list and every list made before it go whatever the cap; a reading).
    pub fn bake_dig_waits(&self) -> &[[f32; 3]] {
        &self.baked.dig_waits
    }

    /// The last live-queue frame's wait for an edit's step in ms (0 when it carried no box, and
    /// in lockstep) and the submits it issued, the carried ones with them (a reading).
    pub fn bake_frame_issue(&self) -> (f64, u64) {
        self.baked.last_issue
    }

    /// The last live-queue frame's wait for an edit's step by its parts in ms, with the bake
    /// thread on and when that frame carried a box (a reading): the rest of a step in progress at
    /// the send, the pick-up, the dig's step to the end of its `boxes` stage, the step's rest,
    /// the books' snapshot (with the dig trace when the queue traces digs) and the hand-back;
    /// then the frame's issue lap (its submits). The parts sum to the wait and the send.
    pub fn bake_frame_wait_parts(&self) -> Option<([f64; 6], f64)> {
        let link = self.baked.thread.as_ref().filter(|l| l.waited)?;
        Some((link.parts?.ms, link.laps[1]))
    }

    /// The last live-queue frame's apply in ms (with the bake thread on, the render thread's
    /// apply lap; in lockstep the step's apply stage) and the bytes it applied over the frame's
    /// cap (a reading).
    pub fn bake_frame_apply(&self) -> (f64, usize) {
        self.baked.last_apply
    }

    /// Of the live queue's frames whose newest books held a block fading, those that applied
    /// a bake step's output (each step moves every fade): lockstep's step runs in its frame,
    /// the bake thread's arrive when they arrive (a reading).
    pub fn bake_fade_frames(&self) -> [u64; 2] {
        self.baked.fade_frames
    }

    /// Whether the store holds a block for `coord` and whether it is the coarse pass's, in either
    /// mode (a reading): lockstep's copy, else the bake thread's newest books, whose coarse marks
    /// are carried only under [`crate::bake::BakeQueue::trace_digs`] (`None` without it).
    pub fn store_holds(&self, coord: svet_world::ChunkCoord) -> Option<(bool, bool)> {
        match &self.baked.thread {
            Some(link) => {
                let coarse = link.books.coarse.as_ref()?;
                Some((
                    link.books.blocks.contains_key(&coord.0),
                    coarse.contains(&coord.0),
                ))
            }
            None => {
                let store = &self.baked.store;
                Some((store.block(coord).is_some(), store.is_coarse(coord)))
            }
        }
    }

    /// Whether the bake thread runs now (on, started, not stopped by a panic).
    pub fn bake_thread_running(&self) -> bool {
        self.baked.thread.is_some()
    }

    /// The world the bake thread lists its chunks from, read through its per-chunk locks: the
    /// world every [`Frame::bake_queue_step`] is given. Without it the queue steps in lockstep.
    pub fn share_bake_world(&mut self, world: Arc<svet_world::World>) {
        self.baked.world = Some(world);
    }

    /// Whether the live queue runs, in lockstep or on the bake thread.
    pub(super) fn queue_running(&self) -> bool {
        self.baked.queue.is_some() || self.baked.thread.is_some()
    }

    /// Stops and joins the bake thread, if it runs: the books, the store's copy and the shadow
    /// back in lockstep's hands, the lists it made applied with `queue` (no cap; without a
    /// queue they wait for the next step), its submits issued at the next step, and the message
    /// it had not read merged into the next step's. A panic that ended it is kept for the next
    /// step to return, and the queue ends.
    pub(super) fn join_bake(&mut self, queue: Option<&wgpu::Queue>) {
        let Some(link) = self.baked.thread.take() else {
            return;
        };
        let b = &mut self.baked;
        match link.stop() {
            Err(e) => self.end_failed(e),
            Ok(st) => {
                let mut books = st.side.queue;
                books.stop_lister();
                b.queue = Some(books);
                b.store = st.side.store;
                b.shadow = st.side.shadow;
                for (ops, lists) in st.steps {
                    b.unissued.extend(ops);
                    let taken = b.lists.take(lists);
                    debug_assert!(taken.is_ok(), "a write list over the frame's cap");
                }
                if let Some(m) = st.msg {
                    b.issuer.give_back(m.read_backs);
                    let mut msg = m.msg;
                    // The words it carried are in `unconfirmed` too, written below.
                    msg.landed.clear();
                    b.leftover = Some(msg);
                }
                // Lockstep's lists apply in their step, and the step takes their words at once.
                b.lists.cap = None;
                if let Some(queue) = queue {
                    let mut out = PoolApplier {
                        queue,
                        pool: &b.pool,
                    };
                    let _ = b.lists.apply(u64::from(self.state.frame), &mut out);
                }
                for (c, (w, _)) in b.lists.unconfirmed.drain() {
                    b.store.landed(c, w);
                }
            }
        }
    }

    /// The bake thread ended by a panic: its books are gone, so the queue ends; the pool keeps
    /// the light it holds. The next step returns the panic.
    fn end_failed(&mut self, e: BakeError) {
        let b = &mut self.baked;
        b.thread = None;
        b.queue = None;
        b.lists = Lists::default();
        b.unissued.clear();
        b.leftover = None;
        b.failed = Some(e);
    }

    /// The step's start on the bake thread, if it should run there: the setting on, a sliced
    /// step, the world shared. The books, the store's copy and the shadow move to the thread;
    /// the store here is a stand-in of the same capacity and missing-light mode.
    pub(super) fn start_bake_thread(&mut self, world: &svet_world::World, sliced: bool) {
        let b = &mut self.baked;
        if b.thread.is_some() || !b.thread_on || !sliced {
            return;
        }
        let Some(shared) = b.world.clone() else {
            return;
        };
        debug_assert!(
            std::ptr::eq(Arc::as_ptr(&shared), world),
            "the shared world is the one stepped"
        );
        let Some(mut queue) = b.queue.take() else {
            return;
        };
        // The bake thread's openings list through a kept pool and do not wait for it, and its
        // steps hold its depth of submits unread and open jobs into its table slots.
        queue.set_thread_depth(b.thread_depth);
        queue.set_thread_slots(b.thread_slots);
        if let Err(e) = queue.start_lister(shared.clone()) {
            self.end_failed(BakeError::Thread(format!("spawn: {e}")));
            return;
        }
        let mut stand_in = crate::bake::Store::new(b.store.capacity());
        stand_in.keep_missing_light(b.store.missing_light());
        let side = BakeSide {
            queue,
            store: std::mem::replace(&mut b.store, stand_in),
            shadow: std::mem::take(&mut b.shadow),
            world: shared,
            #[cfg(test)]
            hook: None,
        };
        b.lists.cap = Some(THREAD_LIST_CAP);
        match ThreadLink::start(side, Some(THREAD_LIST_CAP), OUTBOX_BYTES) {
            Ok(link) => b.thread = Some(link),
            // No thread to be had: the books are lost with the closure that failed to spawn.
            Err(e) => self.end_failed(BakeError::Thread(format!("spawn: {e}"))),
        }
    }

    /// The render thread's frame with the bake thread on: take what it made, issue its submits
    /// and calls through the frame's render half, apply the lists under the cap, collect what
    /// mapped without waiting, and send `msg` with the landing words and the read-backs. Waits on
    /// a step only when `msg` carries an edit's box ([`crate::bake::render_frame`]).
    #[allow(
        clippy::too_many_arguments,
        reason = "the device and queue, the mirror, the bake's parameters and group, and the message are independent"
    )]
    pub(super) fn thread_frame(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gpu: &GpuWorld,
        params: Params,
        world_bind: &wgpu::BindGroup,
        msg: FrameMessage,
    ) -> Result<QueueStats, BakeError> {
        let b = &mut self.baked;
        let link = b.thread.as_mut().expect("the bake thread runs");
        let start = msg.start;
        let gpu_run = GpuRun {
            device,
            queue,
            gpu,
            pipeline: b.pipeline.as_ref().expect("built by the step"),
            world_bind,
            params_buffer: &self.state.params,
            params,
            pool: &b.pool,
        };
        let mut none = Lists::default();
        let mut half = GpuHalf {
            run: &gpu_run,
            issuer: &mut b.issuer,
            lists: &mut none,
        };
        let mut out = PoolApplier {
            queue,
            pool: &b.pool,
        };
        let stats =
            crate::bake::render_frame(link, &mut half, &mut b.lists, &mut out, &mut b.inbox, msg);
        let mut stats = match stats {
            Err(BakeError::Thread(m)) => {
                self.end_failed(BakeError::Thread(m.clone()));
                self.baked.failed = None;
                return Err(BakeError::Thread(m));
            }
            other => other?,
        };
        // The render thread's share is all of its time in the frame's call, filed under the
        // apply; the stages' own times are the bake thread's.
        let wall = crate::bake::stopwatch()
            .duration_since(start.at)
            .as_secs_f64()
            * 1e3;
        stats.stages.total_ms = wall;
        stats.stages.render_ms = [0.0; crate::bake::STEP_STAGES.len()];
        stats.stages.render_ms[crate::bake::S_APPLY] = wall;
        let b_over = self.baked.lists.over;
        if let Some(link) = self.baked.thread.as_mut() {
            let trip = std::mem::take(&mut link.round_trip);
            self.baked.round_trip.merge(&trip);
            let l = link.laps;
            let in_call: f64 = l.iter().sum();
            if self.baked.render_laps.len() < READING_FRAMES {
                self.baked.render_laps.push([
                    (wall - in_call) as f32,
                    l[0] as f32,
                    l[1] as f32,
                    l[2] as f32,
                    l[3] as f32,
                    l[4] as f32,
                    l[5] as f32,
                    wall as f32,
                ]);
            }
            if link.waited && self.baked.dig_waits.len() < READING_FRAMES {
                self.baked
                    .dig_waits
                    .push([l[5] as f32, link.issued_now as f32, b_over as f32]);
            }
            if link.fading_before > 0 {
                self.baked.fade_frames[0] += 1;
                self.baked.fade_frames[1] += u64::from(link.took > 0);
            }
        }
        Ok(stats)
    }
}
