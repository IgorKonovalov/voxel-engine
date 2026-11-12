//! The loaded light's store and its bake (ADR-0046): every surface brick of a chunk holds its
//! emissive irradiance at unit emission, a value a direction for each cell of [`BRICK_CELL`] (or
//! 4) voxels, in one pool, computed ahead of the frames that read it.
//!
//! **The list.** [`list_chunk_faces`] is a pure function of a chunk's voxels, its six neighbours'
//! and each material's class ([`face_classes`]): each surface brick's sorted face keys, an opaque
//! voxel's faces toward a voxel that is not opaque and a translucent voxel's faces toward air
//! (ADR-0047: a pane's two sides), the latter listed in [`ChunkFaces::glass`] and baked as the
//! opaque ones are: a pane's entry face reads its brick's record (`hit.wgsl`'s
//! `glass_reflected`).
//!
//! **The pool** ([`Store`]), in `u32` words: words 0 to 2 unused (the frame's read counters,
//! no value, brick reads and [`STORE_COARSE_READS`], are a buffer of their own that only the
//! counting temporal pass writes), the mirror window's origin (word 4) and size (word 8) in
//! chunks, the flags ([`STORE_FLAGS`]), then from word [`STORE_TABLE`] one word per window chunk
//! naming its block's first word (0 none), bit 31 ([`TABLE_COARSE`]) set on the live queue's
//! coarse blocks, then the two levels of the [`ParentTable`] (ADR-0053). A block is 512 directory
//! words, a brick's `offset from the block's start << 11` just past its record (0 no surface),
//! then at [`SUMMARY_AT`], just past the directory, the block's light summary (one record in the
//! cell's format: each direction the mean over the cells that have it), then each surface
//! brick's record ([`BRICK_WORDS`] a cell, [`record_words`]). A record's direction is the mean of the cell's faces of that direction,
//! opaque and translucent, each face drawing its share of [`BakeSettings::brick_samples`].
//!
//! **The job** ([`Frame::bake_until`](crate::Frame::bake_until)): a chunk's faces, its own
//! candidate table ([`job_table`]) and its frame parameters, staged once in a slot when the job
//! opens. A submit holds the live faces of up to [`OPEN_JOBS`] open jobs, at most
//! [`SUBMIT_SAMPLES`] samples, in one dispatch whose entries name their jobs' slots, so every face
//! samples its own job's table. A one-off bake waits on each submit; the live queue reads a submit
//! back frames later, once mapped, with at most [`IN_FLIGHT`] unread, and a face's next round
//! starts after the samples it has in flight. An entry of the working buffer is one round of one
//! face, its samples spread over a workgroup's [`BAKE_LANES`] lanes: a
//! face takes the rounds that cannot end it (those up to `n_min`) at once, one round past that,
//! and, when the submit still has room and no job is left to open, further rounds speculatively.
//! `bake.wgsl` returns each entry's sums; the running sums are kept here in `f64` and added round
//! by round in order, the stop is read after each round, and a round past a face's stop is thrown
//! away ([`BakeReport::wasted`]). A face's samples are a function of its key and the sample's
//! index alone, so its rounds, its stop and its record depend neither on the batching, nor on the
//! rounds a submit, nor on which jobs share a submit, nor on the jobs' order.
//!
//! The stop ([`se_levels`]): a face ends when the standard error of its mean luminance, through the
//! display transform at [`BAKE_EXPOSURE`], is at most [`BAKE_SE`] levels, not before `n_min` samples
//! and not after `n_max`. A face's `n_min` and `n_max` are both its share of its brick direction's
//! samples, so it ends at that share.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use vxe_world::{
    AIR, BRICK_VOXELS, Brick, CHUNK_BRICKS, Chunk, ChunkCoord, ChunkExtent, MaterialTable, World,
    brick_of_slot, brick_slot,
};

use crate::frame::{LIGHT_TABLE_ENTRIES, LightTableUniform, Params};
use crate::gpu_util::{Io, compute_pipeline_with, io_layout};
use crate::gpu_world::{CHANGES_MAX, GpuWorld};

mod budget;
mod faces;
mod job;
mod job_loop;
mod order;
mod pipeline;
mod queue;
mod record;
mod stages;
mod store;
mod summary;

pub use self::budget::*;
pub use self::faces::*;
pub use self::job::*;
use self::job_loop::*;
pub use self::order::*;
pub use self::pipeline::*;
pub use self::queue::*;
pub use self::record::*;
pub use self::stages::*;
pub use self::store::*;
pub use self::summary::*;

/// One face after its bake.
#[derive(Clone, Debug)]
pub struct FaceOutcome {
    pub voxel: [i32; 3],
    pub face: u8,
    pub samples: u32,
    /// The mean irradiance, unclamped and clamped.
    pub mean: [[f32; 3]; 2],
    /// The stop's standard error in levels at the end.
    pub se: f32,
    /// The dominant material.
    pub material: u16,
    /// `(samples, mean unclamped, mean clamped)` at each of [`BakeSettings::snapshots`] reached.
    pub snapshots: Vec<(u32, [f32; 3], [f32; 3])>,
}

/// What a bake did.
#[derive(Clone, Debug, Default)]
pub struct BakeReport {
    pub chunks: u64,
    pub faces: u64,
    /// Samples kept: the rounds up to each face's stop.
    pub samples: u64,
    /// Samples of rounds drawn past a face's stop in the same submit, thrown away.
    pub wasted: u64,
    pub submits: u64,
    /// The most jobs one submit held.
    pub max_jobs_in_submit: u64,
    /// The most rounds one face took in one submit.
    pub max_rounds_in_submit: u32,
    /// Seconds from each submit to its readback, over the bake.
    pub gpu_seconds: f64,
    /// Seconds of the dispatches on the GPU's timestamps, under [`BakeSettings::timer`].
    pub dispatch_seconds: f64,
    pub faces_at_min: u64,
    pub faces_at_max: u64,
    pub rays: u64,
    pub ray_len: f64,
    /// Every face, under [`BakeSettings::keep_faces`].
    pub outcomes: Vec<FaceOutcome>,
}

/// The bake's progress, after a submit.
#[derive(Clone, Copy, Debug, Default)]
pub struct BakeProgress {
    pub chunks_done: u64,
    pub chunks: u64,
    pub faces_done: u64,
    pub faces: u64,
    pub samples: u64,
    pub gpu_seconds: f64,
}

/// Why a bake did not run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BakeError {
    /// The pool cannot hold the extent's blocks.
    PoolFull { needed: u64, capacity: u64 },
    /// The light-sample buffer refused a chunk in the bake's reach.
    LightsRefused(u64),
}

impl std::fmt::Display for BakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PoolFull { needed, capacity } => {
                write!(f, "the bake pool needs {needed} B and holds {capacity} B")
            }
            Self::LightsRefused(n) => {
                write!(f, "the light-sample buffer refused {n} chunks in reach")
            }
        }
    }
}

impl std::error::Error for BakeError {}
