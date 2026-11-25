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

    /// The frames encoded while the bake thread ran, those whose eye lay past the reach of the
    /// books' snapshot of the words under the step's eye, and those where the sky over the
    /// floor read nothing at the encode's eye but an answer at the step's (a reading).
    pub fn bake_snapshot_reach(&self) -> [u64; 3] {
        self.baked.snapshot_reach.read()
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
                b.queue = Some(st.side.queue);
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
        let Some(queue) = b.queue.take() else {
            return;
        };
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
    /// mapped without waiting, and send `msg` with the landing words and the read-backs. Never
    /// waits on a step.
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
        Ok(stats)
    }
}
