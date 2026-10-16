//! Distance, overlap and the conservative-advancement sweep (ADR-0005).

use vxe_world::{FIXED_ONE, Fixed3, QueryStats, World};

use crate::body::{Body, Contact, Key, SolidBox, collect, isqrt};

/// The sweep stops after this many advances, whatever is left of the delta.
pub const MAX_SWEEP_ITERATIONS: u32 = 64;

/// What a sweep did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sweep {
    /// How far the foot moved; never more than the delta on any axis.
    pub moved: Fixed3,
    /// The box that stopped the sweep, or `None` when the whole delta was spent (even if the
    /// body ends touching something).
    pub contact: Option<Contact>,
    /// The body overlapped a solid box before moving; it was not moved.
    pub started_overlapping: bool,
    /// Advances computed, at most [`MAX_SWEEP_ITERATIONS`].
    pub iterations: u32,
}

/// The half-open box the body occupies at `foot` and at `foot + delta`, and everything between:
/// the body's bounding box extended along the delta. A box touching the body from a side the
/// delta does not move toward lies outside it.
fn swept_region(b: &Body, delta: [i64; 3]) -> ([i64; 3], [i64; 3]) {
    let f = b.foot.0;
    let lo0 = [f[0] - b.radius, f[1] - b.radius, f[2]];
    let hi0 = [f[0] + b.radius, f[1] + b.radius, f[2] + b.height];
    let lo = [0, 1, 2].map(|k| lo0[k].min(lo0[k] + delta[k]));
    let hi = [0, 1, 2].map(|k| hi0[k].max(hi0[k] + delta[k]));
    (lo, hi)
}

/// The nearest of `boxes` meeting `[lo, hi)`, by [`Key`].
fn nearest(boxes: &[SolidBox], b: &Body, lo: [i64; 3], hi: [i64; 3]) -> Option<(Key, Contact)> {
    let (key, bx) = boxes
        .iter()
        .filter(|x| x.intersects(lo, hi))
        .map(|x| (x.key(b), x))
        .min_by_key(|(k, _)| *k)?;
    Some((key, bx.contact(b, key.voxel)))
}

/// The nearest of `boxes` meeting `[lo, hi)` that a move by `delta` closes on
/// ([`SolidBox::closes_on`]), by [`Key`]. A box the move runs along or away from cannot stop
/// it. Without that filter a box within the floored gap of zero, or within the gap at which
/// every advance rounds to zero, stops every move from there, including one straight away
/// from it, and the body never moves again.
fn nearest_blocking(
    boxes: &[SolidBox],
    b: &Body,
    lo: [i64; 3],
    hi: [i64; 3],
    delta: [i64; 3],
) -> Option<(Key, Contact)> {
    let (key, bx) = boxes
        .iter()
        .filter(|x| x.intersects(lo, hi))
        .map(|x| (x.key(b), x))
        .filter(|(_, x)| x.closes_on(b, delta))
        .min_by_key(|(k, _)| *k)?;
    Some((key, bx.contact(b, key.voxel)))
}

fn to_i64(v: i128) -> i64 {
    v.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}

/// The smallest squared distance from `body` to a solid box within one voxel of its bounding
/// box, and the contact that box makes; `None` when that region holds no solid.
pub fn distance2(world: &World, body: &Body) -> Option<(i64, Contact)> {
    distance2_with_stats(world, body).0
}

/// [`distance2`] plus the work its region scan did.
pub fn distance2_with_stats(world: &World, body: &Body) -> (Option<(i64, Contact)>, QueryStats) {
    let mut stats = QueryStats::default();
    let (lo, hi) = swept_region(body, [0; 3]);
    let lo = lo.map(|v| v - FIXED_ONE);
    let hi = hi.map(|v| v + FIXED_ONE);
    let boxes = collect(world, lo, hi, &mut stats);
    let n = nearest(&boxes, body, lo, hi).map(|(k, c)| (to_i64(k.d2), c));
    (n, stats)
}

/// Whether `body` strictly overlaps a solid voxel. Touching is not overlapping.
pub fn overlaps(world: &World, body: &Body) -> bool {
    let (lo, hi) = swept_region(body, [0; 3]);
    collect(world, lo, hi, &mut QueryStats::default())
        .iter()
        .any(|x| x.overlaps(body))
}

/// Moves `body` along `delta` until it touches a solid box, by conservative advancement.
///
/// Each iteration takes `g`, the floor of the root of the smallest squared distance to a
/// solid box in the region swept along the remaining delta, and advances by the whole
/// remainder when `g² >= |remaining|²`, else by `g` along it (scaled by `|remaining|` rounded
/// up, every component rounded toward zero), so no advance is longer than `g`. Boxes the
/// remaining delta does not close on (it runs along or away from them) are left out of `g`.
/// It stops on `g == 0` (that box is the contact), when an advance would overlap a box (that
/// box is the contact; the advance is not taken), when the delta is spent (no contact), when
/// every component rounds to zero, or after [`MAX_SWEEP_ITERATIONS`] (the nearest box the
/// remaining delta closes on is the contact). Delta components are bounded to `2^31` fixed
/// units.
pub fn sweep(world: &World, body: &Body, delta: Fixed3) -> Sweep {
    sweep_with_stats(world, body, delta).0
}

/// [`sweep`] plus the work its region scan did.
pub fn sweep_with_stats(world: &World, body: &Body, delta: Fixed3) -> (Sweep, QueryStats) {
    let mut stats = QueryStats::default();
    let start = body.foot.0;
    let (lo, hi) = swept_region(body, delta.0);
    // Every later region is inside this one: it starts at the body's current box, which lies
    // on the segment, and extends along what is left of the same delta.
    let boxes = collect(world, lo, hi, &mut stats);
    let mut out = Sweep {
        moved: Fixed3::default(),
        contact: None,
        started_overlapping: false,
        iterations: 0,
    };
    if boxes.iter().any(|x| x.overlaps(body)) {
        out.started_overlapping = true;
        out.contact = nearest(&boxes, body, lo, hi).map(|(_, c)| c);
        return (out, stats);
    }
    let mut pos = start;
    let mut rem = delta.0;
    while out.iterations < MAX_SWEEP_ITERATIONS && rem != [0; 3] {
        out.iterations += 1;
        let b = body.at(pos);
        let (rlo, rhi) = swept_region(&b, rem);
        let Some((key, contact)) = nearest_blocking(&boxes, &b, rlo, rhi, rem) else {
            pos = [0, 1, 2].map(|k| pos[k] + rem[k]);
            rem = [0; 3];
            break;
        };
        let g = isqrt(key.d2 as u128) as i128;
        if g == 0 {
            out.contact = Some(contact);
            break;
        }
        let r = rem.map(i128::from);
        let rr = r[0] * r[0] + r[1] * r[1] + r[2] * r[2];
        // The whole remainder when it is no longer than `g`; else `g` along it, scaled by the
        // root of `|rem|²` rounded up, so the advance is never longer than `g`. With the
        // root rounded down, a remainder up to one unit longer than `g` passed for `g` and
        // was taken whole, ending inside the box.
        let adv = if g * g >= rr {
            rem
        } else {
            let len = isqrt(rr as u128) as i128;
            let len = if len * len < rr { len + 1 } else { len };
            // `/` on integers rounds toward zero.
            r.map(|c| (c * g / len) as i64)
        };
        if adv == [0; 3] {
            out.contact = Some(contact);
            break;
        }
        let next = [0, 1, 2].map(|k| pos[k] + adv[k]);
        // A box left out as not closed on can lie under one unit from the body, and an
        // advance rounded off the line of the delta can reach into it. Stop short of that.
        let nb = body.at(next);
        if let Some(x) = boxes.iter().find(|x| x.overlaps(&nb)) {
            out.contact = Some(x.contact(&b, x.key(&b).voxel));
            break;
        }
        pos = next;
        rem = [0, 1, 2].map(|k| rem[k] - adv[k]);
    }
    if rem != [0; 3] && out.contact.is_none() {
        let b = body.at(pos);
        let (rlo, rhi) = swept_region(&b, rem);
        out.contact = nearest_blocking(&boxes, &b, rlo, rhi, rem).map(|(_, c)| c);
    }
    out.moved = Fixed3([0, 1, 2].map(|k| pos[k] - start[k]));
    (out, stats)
}
