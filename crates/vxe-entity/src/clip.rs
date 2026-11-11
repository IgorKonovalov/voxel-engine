//! Clips: keys per part, sampled by integer tick in fixed point, and the blend of two poses.
//!
//! Everything here is a pure function of its arguments: the same clip at the same tick gives
//! the same pose, bit for bit, on every platform (ADR-0051). The consumer owns the clock and
//! hands a tick in; [`Playback`] turns a consumer tick into a clip tick.
//!
//! **The rest layout.** A [`Part`](crate::Part) carries no offset from its parent: a child's
//! rest place is the `t` of its [`PartPose`] in a rest pose the consumer (or a rig file) builds.
//! [`sample`] takes that rest pose, and a key's translation is an offset from it. That keeps the
//! rest layout in one argument; were it to move onto the part, `sample` would drop the argument
//! and read the offset from the model instead.

use std::sync::Arc;

use vxe_world::Fixed3;

use crate::model::Model;
use crate::pose::{PartPose, Pose};
use crate::quat::{Quat, nlerp};

/// One key of a track: where a part is at `tick`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Key {
    /// The clip tick, `0..=length`.
    pub tick: u32,
    /// The offset from the part's rest translation, in 1/256 world voxel, along the parent's
    /// turned axes.
    pub t: [i32; 3],
    /// The part's rotation relative to its parent. It replaces the rest rotation.
    pub q: Quat,
}

/// One part's keys, sorted by tick.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Track {
    /// The part's index in the model.
    pub part: u8,
    /// At least one key, ticks strictly increasing.
    pub keys: Vec<Key>,
}

/// Why [`Clip::new`] refused its tracks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipError {
    /// A length of zero ticks.
    ZeroLength,
    /// A track whose ticks do not strictly increase, at the key `key` (its index).
    UnsortedTrack { part: u8, key: usize },
    /// A key at a tick past the clip's length.
    KeyPastLength { part: u8, tick: u32, length: u32 },
    /// A track for a part the model lacks.
    UnknownPart { part: u8, parts: usize },
    /// A track with no keys.
    EmptyTrack { part: u8 },
    /// Two tracks for one part.
    DuplicateTrack { part: u8 },
}

impl std::fmt::Display for ClipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::ZeroLength => write!(f, "a clip needs a length of at least one tick"),
            Self::UnsortedTrack { part, key } => {
                write!(f, "part {part}'s key {key} is not after the key before it")
            }
            Self::KeyPastLength { part, tick, length } => write!(
                f,
                "part {part} has a key at tick {tick}, past the clip's {length} ticks"
            ),
            Self::UnknownPart { part, parts } => {
                write!(f, "a track for part {part}, the model has {parts} parts")
            }
            Self::EmptyTrack { part } => write!(f, "part {part}'s track has no keys"),
            Self::DuplicateTrack { part } => write!(f, "two tracks for part {part}"),
        }
    }
}

impl std::error::Error for ClipError {}

/// A validated clip: a length in ticks, whether it loops, and a track per animated part.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Clip {
    length: u32,
    looped: bool,
    /// Sorted by part.
    tracks: Vec<Track>,
}

impl Clip {
    /// A clip of `length` ticks for `model`, refused with a typed error when the length is zero,
    /// a track's ticks do not strictly increase, a key lies past the length, a track names a part
    /// the model lacks, a track is empty or two tracks name one part.
    pub fn new(
        model: &Model,
        length: u32,
        looped: bool,
        mut tracks: Vec<Track>,
    ) -> Result<Self, ClipError> {
        if length == 0 {
            return Err(ClipError::ZeroLength);
        }
        let parts = model.parts().len();
        for tr in &tracks {
            let part = tr.part;
            if usize::from(part) >= parts {
                return Err(ClipError::UnknownPart { part, parts });
            }
            if tr.keys.is_empty() {
                return Err(ClipError::EmptyTrack { part });
            }
            for (i, w) in tr.keys.windows(2).enumerate() {
                if w[1].tick <= w[0].tick {
                    return Err(ClipError::UnsortedTrack { part, key: i + 1 });
                }
            }
            if let Some(k) = tr.keys.iter().find(|k| k.tick > length) {
                return Err(ClipError::KeyPastLength {
                    part,
                    tick: k.tick,
                    length,
                });
            }
        }
        tracks.sort_by_key(|t| t.part);
        if let Some(w) = tracks.windows(2).find(|w| w[0].part == w[1].part) {
            return Err(ClipError::DuplicateTrack { part: w[0].part });
        }
        Ok(Self {
            length,
            looped,
            tracks,
        })
    }

    pub fn length(&self) -> u32 {
        self.length
    }

    pub fn looped(&self) -> bool {
        self.looped
    }

    /// The tracks, sorted by part.
    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    /// The clip tick a tick maps to: past the length it wraps when looped (`tick % length`) and
    /// holds at the length otherwise.
    pub fn clip_tick(&self, tick: u64) -> u32 {
        let length = u64::from(self.length);
        if self.looped {
            (tick % length) as u32
        } else {
            tick.min(length) as u32
        }
    }
}

/// `(a * (den - num) + b * num) / den`, rounded toward zero.
fn lerp_t(a: [i32; 3], b: [i32; 3], num: i64, den: i64) -> [i32; 3] {
    [0, 1, 2].map(|k| ((i64::from(a[k]) * (den - num) + i64::from(b[k]) * num) / den) as i32)
}

/// The key a track gives at clip tick `tick`: the nearest key before its first and after its
/// last, and between two keys a linear translation and a normalised linear blend of the
/// rotations along the shorter arc.
fn sample_track(keys: &[Key], tick: u32) -> Key {
    let first = keys[0];
    let last = keys[keys.len() - 1];
    if tick <= first.tick {
        return first;
    }
    if tick >= last.tick {
        return last;
    }
    // The first key after `tick`; it has a predecessor at or before `tick`.
    let i = keys.partition_point(|k| k.tick <= tick);
    let (k0, k1) = (keys[i - 1], keys[i]);
    let num = i64::from(tick - k0.tick);
    let den = i64::from(k1.tick - k0.tick);
    Key {
        tick,
        t: lerp_t(k0.t, k1.t, num, den),
        q: nlerp(k0.q, k1.q, num, den),
    }
}

/// The pose of `model` at `tick` of `clip`, the root at the origin with the identity rotation
/// for the caller to place.
///
/// A part with a track takes the rest translation plus the sampled key's offset, and the
/// sampled key's rotation. A part with no track keeps its rest [`PartPose`]. `rest` gives one
/// part pose per part (a missing one reads as the default); tracks for parts past the model's
/// end are skipped, which a clip built for this model never has.
pub fn sample(model: &Model, rest: &Pose, clip: &Clip, tick: u64) -> Pose {
    let tick = clip.clip_tick(tick);
    let mut parts: Vec<PartPose> = (0..model.parts().len())
        .map(|i| rest.parts.get(i).copied().unwrap_or_default())
        .collect();
    for tr in &clip.tracks {
        let Some(pp) = parts.get_mut(usize::from(tr.part)) else {
            continue;
        };
        let k = sample_track(&tr.keys, tick);
        pp.t = [0, 1, 2].map(|c| pp.t[c].saturating_add(k.t[c]));
        pp.q = k.q;
    }
    Pose {
        root: Fixed3::default(),
        root_q: Quat::IDENTITY,
        parts,
    }
}

/// The blend of two poses at `w` in 1/256: `w` 0 is `a`, 256 is `b` (a larger `w` reads as
/// 256). The root and every part blend by the sampling rules: translations linear, rounded
/// toward zero; rotations a normalised linear blend along the shorter arc. `w` 0 and 256 return
/// `a` and `b` bit for bit.
///
/// # Panics
///
/// When the two poses hold different numbers of part poses.
pub fn blend(a: &Pose, b: &Pose, w: u16) -> Pose {
    assert_eq!(
        a.parts.len(),
        b.parts.len(),
        "blend needs two poses of one model"
    );
    match w {
        0 => return a.clone(),
        256.. => return b.clone(),
        _ => {}
    }
    let (num, den) = (i64::from(w), 256);
    let root = [0, 1, 2].map(|k| {
        let s = i128::from(a.root.0[k]) * i128::from(den - num)
            + i128::from(b.root.0[k]) * i128::from(num);
        (s / i128::from(den)) as i64
    });
    Pose {
        root: Fixed3(root),
        root_q: nlerp(a.root_q, b.root_q, num, den),
        parts: a
            .parts
            .iter()
            .zip(&b.parts)
            .map(|(pa, pb)| PartPose {
                t: lerp_t(pa.t, pb.t, num, den),
                q: nlerp(pa.q, pb.q, num, den),
            })
            .collect(),
    }
}

/// A clip playing from a consumer tick at a rate. The consumer owns it and its clock; the crate
/// keeps none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Playback {
    pub clip: Arc<Clip>,
    /// The consumer tick the clip's tick 0 falls on.
    pub start_tick: u64,
    /// Clip ticks per consumer tick, in 1/256: 256 plays at the clip's own speed.
    pub rate: u32,
}

impl Playback {
    /// The clip tick at consumer tick `now`: `(now - start_tick) * rate / 256`, rounded down,
    /// and 0 before the start. Not yet wrapped or held; [`sample`] does that.
    pub fn tick_at(&self, now: u64) -> u64 {
        let elapsed = u128::from(now.saturating_sub(self.start_tick));
        let t = elapsed * u128::from(self.rate) / 256;
        u64::try_from(t).unwrap_or(u64::MAX)
    }

    /// The pose at consumer tick `now`: [`sample`] at [`Self::tick_at`].
    pub fn pose(&self, model: &Model, rest: &Pose, now: u64) -> Pose {
        sample(model, rest, &self.clip, self.tick_at(now))
    }
}
