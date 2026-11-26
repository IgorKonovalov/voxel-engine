//! The live queue's order: the candidate heap, the view angles and the score. Pure CPU.

use super::*;

/// The live queue's angle weight by default (ADR-0048's `W_ANGLE`, [`QueueOrder::Score`]).
pub const QUEUE_ANGLE_WEIGHT: f32 = 4.0;

/// The live queue's refine weight when the score folds the class in ([`Refine::Weight`],
/// ADR-0048's `W_REFINE`). The default order keeps the class as a tier ([`Refine::Tier`]);
/// this is the comparison arm's weight.
pub const QUEUE_REFINE_WEIGHT: f32 = 2.0;

/// How the score treats a chunk that holds a block (a coarse one to refine, or a whole re-bake
/// of ring 3).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Refine {
    /// The score is multiplied by the weight: one list, whatever the class.
    Weight(f32),
    /// The class is the leading key, the score inside each class: every chunk with no block
    /// before any refinement (ADR-0046's coarse-first rule), with its break and its per-class
    /// reach checks.
    Tier,
}

/// The order the live queue opens its candidates in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum QueueOrder {
    /// ADR-0048's score ([`queue_score`]), ties by the view cosine then the coordinate.
    Score { angle: f32, refine: Refine },
    /// The class first, then the squared chunk distance, ties by the view (ADR-0046 as built).
    Distance,
}

/// ADR-0048's score at [`QUEUE_ANGLE_WEIGHT`] with the class as the leading key
/// ([`Refine::Tier`], ADR-0046's coarse-first rule).
impl Default for QueueOrder {
    fn default() -> Self {
        Self::Score {
            angle: QUEUE_ANGLE_WEIGHT,
            refine: Refine::Tier,
        }
    }
}

impl QueueOrder {
    /// The order in words: `order score, angle 4, refine 2`, `order score, angle 4, refine
    /// tier` or `order distance`.
    pub fn words(&self) -> String {
        match self {
            Self::Score { angle, refine } => format!(
                "order score, angle {angle}, refine {}",
                match refine {
                    Refine::Weight(r) => r.to_string(),
                    Refine::Tier => "tier".to_owned(),
                }
            ),
            Self::Distance => "order distance".to_owned(),
        }
    }
}

/// A candidate of an opening: its sort key (the leading class or 0, the score's bits or the
/// squared distance, the view tie, the coordinate), its class (1 when it holds a block), its
/// squared chunk distance and its angle term.
#[derive(Clone, Copy, Debug)]
pub(super) struct Cand {
    pub(super) lead: u8,
    pub(super) key: u64,
    pub(super) tie: i32,
    pub(super) c: [i32; 3],
    pub(super) class: u8,
    /// Whether its block is coarse.
    pub(super) coarse: bool,
    pub(super) d2: i64,
    pub(super) a: f32,
}

impl Cand {
    /// The order's key: unique, since it ends in the coordinate.
    pub(super) fn sort_key(&self) -> (u8, u64, i32, [i32; 3]) {
        (self.lead, self.key, self.tie, self.c)
    }
}

/// A candidate under its key, least first in a [`std::collections::BinaryHeap`].
#[derive(Clone, Copy, Debug)]
pub(super) struct HeapCand(Cand);

impl PartialEq for HeapCand {
    fn eq(&self, other: &Self) -> bool {
        self.0.sort_key() == other.0.sort_key()
    }
}

impl Eq for HeapCand {}
impl PartialOrd for HeapCand {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for HeapCand {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other.0.sort_key().cmp(&self.0.sort_key())
    }
}

/// Chunks a side of a cell of the order's spatial index ([`Ordered`]).
const ORDER_CELL: i32 = 4;

/// The cell of the order's index chunk `c` lies in.
fn order_cell(c: [i32; 3]) -> [i32; 3] {
    c.map(|v| v.div_euclid(ORDER_CELL))
}

/// One lead's walk of the kept order: the walked head in order, the scored candidates of the
/// scanned cells in a heap, and the cells not scanned yet under their least key.
#[derive(Debug, Default)]
struct Walk {
    done: Vec<Cand>,
    heap: std::collections::BinaryHeap<HeapCand>,
    cells: std::collections::BinaryHeap<std::cmp::Reverse<(u64, [i32; 3])>>,
    scanned: std::collections::HashSet<[i32; 3]>,
    /// The class and coarse flag each candidate was scored with since the walk started: a
    /// candidate that leaves and joins again unchanged (a refused place) keeps its entry.
    scored: std::collections::HashMap<[i32; 3], (u8, bool)>,
    /// The cells are read again at the next [`Ordered::get`].
    stale: bool,
}

/// The kept candidates in their order, scored only as far as an opening walks (Plan 0096 Phase
/// 5): the pool in a spatial index of [`ORDER_CELL`]-chunk cells per lead, each lead walked
/// from the camera outward. A cell's least key (the key of a chunk at its nearest point, no
/// key of a chunk in it is less: the score is at least the distance) orders the cells; a
/// candidate leaves the heap only once its key is under every unscanned cell's, so the walk
/// sees the order a full sort gives. A still camera with no candidate joining scores nothing;
/// a view change or a candidate joining a scanned cell starts its lead's walk again, which
/// scores only the cells it reaches.
#[derive(Debug, Default)]
pub(super) struct Ordered {
    index: [std::collections::HashMap<[i32; 3], Vec<[i32; 3]>>; 2],
    walks: [Walk; 2],
    view: Option<OrderView>,
    forward: [f32; 3],
    half_angle: f32,
    /// Candidates scored since the last [`Ordered::take_visited`].
    visited: u64,
}

impl Ordered {
    /// The lead (the order's leading key) of a candidate of `class` under `order`.
    fn lead(order: QueueOrder, class: u8) -> usize {
        match order {
            QueueOrder::Score {
                refine: Refine::Weight(_),
                ..
            } => 0,
            _ => usize::from(class.min(1)),
        }
    }

    /// The order's view: a change (or a new order) starts both walks again. True when the
    /// order's leads moved, so the index is to be read again ([`Ordered::rebuild`]).
    pub(super) fn set_view(&mut self, key: OrderView, forward: [f32; 3], half_angle: f32) -> bool {
        if self.view == Some(key) {
            return false;
        }
        let relead = self
            .view
            .is_some_and(|v| Self::lead(v.order, 1) != Self::lead(key.order, 1));
        self.view = Some(key);
        self.forward = forward;
        self.half_angle = half_angle;
        for w in &mut self.walks {
            w.stale = true;
        }
        relead
    }

    /// The index read whole from `pool`, both walks started again.
    pub(super) fn rebuild<'a>(
        &mut self,
        pool: impl Iterator<Item = (&'a [i32; 3], &'a (u8, bool))>,
    ) {
        self.index = Default::default();
        let order = self.view.map_or(QueueOrder::default(), |v| v.order);
        for (&c, &(class, _)) in pool {
            self.index[Self::lead(order, class)]
                .entry(order_cell(c))
                .or_default()
                .push(c);
        }
        for w in &mut self.walks {
            w.stale = true;
        }
    }

    /// `c` joined the pool, or its value changed, as `class`: in the index, and its lead's walk
    /// started again where it already scanned `c`'s cell.
    pub(super) fn insert(&mut self, c: [i32; 3], class: u8, coarse: bool, was: Option<u8>) {
        let order = self.view.map_or(QueueOrder::default(), |v| v.order);
        if let Some(w) = was {
            self.remove(c, w);
        }
        let lead = Self::lead(order, class);
        let cell = order_cell(c);
        let members = self.index[lead].entry(cell).or_default();
        let new_cell = members.is_empty();
        members.push(c);
        let key = self.cell_key(cell);
        let walk = &mut self.walks[lead];
        if walk.scanned.contains(&cell) {
            walk.stale |= walk.scored.get(&c) != Some(&(class, coarse));
        } else if new_cell && !walk.stale {
            // A cell the walk's heap of cells did not hold joins it (a cell held twice is
            // scanned once), unless the walk already passed its key.
            if walk.done.last().is_some_and(|d| key < d.key) {
                walk.stale = true;
            } else {
                walk.cells.push(std::cmp::Reverse((key, cell)));
            }
        }
    }

    /// `c` of `class` left the pool: out of the index (a walked entry is skipped by the
    /// walker's pool check).
    pub(super) fn remove(&mut self, c: [i32; 3], class: u8) {
        let order = self.view.map_or(QueueOrder::default(), |v| v.order);
        let cell = order_cell(c);
        let m = &mut self.index[Self::lead(order, class)];
        if let Some(v) = m.get_mut(&cell)
            && let Some(i) = v.iter().position(|&x| x == c)
        {
            v.swap_remove(i);
            if v.is_empty() {
                m.remove(&cell);
            }
        }
    }

    /// The key no chunk of `cell` scores under from the view's eye.
    fn cell_key(&self, cell: [i32; 3]) -> u64 {
        let Some(v) = self.view else { return 0 };
        let d2: i64 = (0..3)
            .map(|k| {
                let lo = cell[k] * ORDER_CELL;
                let hi = lo + ORDER_CELL - 1;
                let e = v.eye[k];
                i64::from((lo - e).max(e - hi).max(0)).pow(2)
            })
            .sum();
        match v.order {
            QueueOrder::Distance => d2 as u64,
            QueueOrder::Score { angle, refine } => {
                let r = match refine {
                    Refine::Weight(r) => r.min(1.0),
                    Refine::Tier => 1.0,
                };
                if angle < 0.0 || r < 0.0 {
                    return 0;
                }
                let o = (d2 as f32).sqrt();
                u64::from((o * r).to_bits())
            }
        }
    }

    /// The `i`th candidate of lead `lead` in order, scanning cells and sorting the heap's head
    /// as far as `i`. Every candidate a scanned cell holds is scored, and counted.
    pub(super) fn get(
        &mut self,
        lead: usize,
        i: usize,
        pool: &std::collections::HashMap<[i32; 3], (u8, bool)>,
    ) -> Option<Cand> {
        let v = self.view?;
        if self.walks[lead].stale {
            let cells: Vec<_> = self.index[lead]
                .keys()
                .map(|&cell| std::cmp::Reverse((self.cell_key(cell), cell)))
                .collect();
            let w = &mut self.walks[lead];
            w.done.clear();
            w.heap.clear();
            w.scanned.clear();
            w.scored.clear();
            w.cells = cells.into();
            w.stale = false;
        }
        while self.walks[lead].done.len() <= i {
            let w = &mut self.walks[lead];
            let head = w.heap.peek().map(|h| h.0.key);
            let bound = w.cells.peek().map(|r| r.0.0);
            match (head, bound) {
                (Some(k), b) if b.is_none_or(|b| k < b) => {
                    let c = w.heap.pop()?.0;
                    w.done.push(c);
                }
                (_, Some(_)) => {
                    let std::cmp::Reverse((_, cell)) = w.cells.pop()?;
                    if !w.scanned.insert(cell) {
                        continue;
                    }
                    let members = self.index[lead].get(&cell).cloned().unwrap_or_default();
                    for c in members {
                        let Some(&(class, coarse)) = pool.get(&c) else {
                            continue;
                        };
                        let k = cand_at(
                            c,
                            class,
                            coarse,
                            v.eye,
                            self.forward,
                            self.half_angle,
                            v.order,
                            v.near,
                        );
                        self.visited += 1;
                        let w = &mut self.walks[lead];
                        w.scored.insert(c, (class, coarse));
                        w.heap.push(HeapCand(k));
                    }
                }
                _ => return None,
            }
        }
        Some(self.walks[lead].done[i])
    }

    /// The candidates scored since the last call.
    pub(super) fn take_visited(&mut self) -> u64 {
        std::mem::take(&mut self.visited)
    }
}

/// What the kept order's scores read: the camera's chunk, the forward direction and the
/// half-angle (bits), the order and the near shell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct OrderView {
    pub(super) eye: [i32; 3],
    pub(super) forward: [u32; 3],
    pub(super) half_angle: u32,
    pub(super) order: QueueOrder,
    pub(super) near: i32,
}

/// The candidate `c` of `class` (and whether its block is `coarse`) scored under `order` from
/// the camera's chunk `eye` and `view`.
pub(super) fn cand_of(
    c: [i32; 3],
    class: u8,
    coarse: bool,
    eye: [i32; 3],
    view: &QueueView,
    order: QueueOrder,
    near: i32,
) -> Cand {
    cand_at(
        c,
        class,
        coarse,
        eye,
        view.forward,
        view.half_angle,
        order,
        near,
    )
}

/// [`cand_of`] from the view's forward direction and half-angle.
#[allow(
    clippy::too_many_arguments,
    reason = "the candidate, its class and the view's five terms are independent"
)]
pub(super) fn cand_at(
    c: [i32; 3],
    class: u8,
    coarse: bool,
    eye: [i32; 3],
    forward: [f32; 3],
    half_angle: f32,
    order: QueueOrder,
    near: i32,
) -> Cand {
    let d = [0, 1, 2].map(|k| c[k] - eye[k]);
    let d2: i64 = d.iter().map(|&v| i64::from(v).pow(2)).sum();
    let len = (d2 as f32).sqrt().max(1.0);
    let dot: f32 = (0..3).map(|k| d[k] as f32 * forward[k]).sum::<f32>() / len;
    let a = angle_term(d, forward, half_angle, near);
    let (lead, key) = match order {
        QueueOrder::Distance => (class, d2 as u64),
        QueueOrder::Score {
            angle,
            refine: Refine::Weight(r),
        } => (
            0,
            u64::from(queue_score(d, a, class == 1, angle, r).to_bits()),
        ),
        QueueOrder::Score {
            angle,
            refine: Refine::Tier,
        } => (
            class,
            u64::from(queue_score(d, a, false, angle, 1.0).to_bits()),
        ),
    };
    Cand {
        lead,
        key,
        tie: -(dot * 1000.0) as i32,
        c,
        class,
        coarse,
        d2,
        a,
    }
}

/// Candidates in the view the reach census reads at most ([`QueueStats::view_reach`]).
pub(crate) const VIEW_REACH_SAMPLE: usize = 64;

/// Steps between two reach censuses of the view.
pub(crate) const VIEW_REACH_EVERY: u64 = 60;

/// Degrees the queue's view cone widens the view's half-angle by (ADR-0048's margin).
pub const QUEUE_VIEW_MARGIN_DEG: f32 = 15.0;

/// The wider of a view's half-angles, in radians: the vertical one (half of `vfov`) or the
/// horizontal one at `aspect` (width over height), never the corner's.
pub fn view_half_angle(vfov: f32, aspect: f32) -> f32 {
    let v = vfov * 0.5;
    let h = (v.tan() * aspect).atan();
    v.max(h)
}

/// The horizontal and vertical half-angles of a view of `vfov` at `aspect`, in radians.
pub fn view_half_angles(vfov: f32, aspect: f32) -> [f32; 2] {
    let v = vfov * 0.5;
    [(v.tan() * aspect).atan(), v]
}

/// Whether a chunk at integer chunk offset `offset` from the camera's lies in the near shell:
/// within `near` chunks (Chebyshev) every way.
pub fn in_near_shell(offset: [i32; 3], near: i32) -> bool {
    offset.iter().all(|v| v.abs() <= near)
}

/// ADR-0048's angle term `a` of a chunk at integer chunk offset `offset` from the camera's: its
/// angle to `forward` beyond the view cone (`half_angle` plus [`QUEUE_VIEW_MARGIN_DEG`]), over
/// pi; 0 inside the cone and in the near shell ([`in_near_shell`]) whatever its direction.
pub fn angle_term(offset: [i32; 3], forward: [f32; 3], half_angle: f32, near: i32) -> f32 {
    if in_near_shell(offset, near) {
        return 0.0;
    }
    let o = offset.map(|v| v as f32);
    let len = (o[0] * o[0] + o[1] * o[1] + o[2] * o[2]).sqrt();
    let flen = (forward[0] * forward[0] + forward[1] * forward[1] + forward[2] * forward[2])
        .sqrt()
        .max(f32::MIN_POSITIVE);
    let cos = ((o[0] * forward[0] + o[1] * forward[1] + o[2] * forward[2]) / (len * flen))
        .clamp(-1.0, 1.0);
    let cone = half_angle + QUEUE_VIEW_MARGIN_DEG.to_radians();
    if cos >= cone.cos() {
        return 0.0;
    }
    ((cos.acos() - cone) / std::f32::consts::PI).max(0.0)
}

/// ADR-0048's score of a candidate at integer chunk offset `offset` with angle term `a`
/// ([`angle_term`]): `|o| x (1 + angle_weight x a) x (refine_weight for a coarse block, else
/// 1)`. Lower opens first.
pub(crate) fn queue_score(
    offset: [i32; 3],
    a: f32,
    coarse: bool,
    angle_weight: f32,
    refine_weight: f32,
) -> f32 {
    let o = offset.map(|v| v as f32);
    let len = (o[0] * o[0] + o[1] * o[1] + o[2] * o[2]).sqrt();
    len * (1.0 + angle_weight * a) * if coarse { refine_weight } else { 1.0 }
}

/// Where a chunk lies against the queue's view ([`QueueStats::view_waiting`]): the near shell,
/// in the view cone beyond it, or out of view.
pub const VIEW_SHELL: usize = 0;

pub const VIEW_IN: usize = 1;

pub const VIEW_OUT: usize = 2;

/// Whether `offset` lies inside the frustum of half-angles `half` (horizontal, vertical, each
/// widened by [`QUEUE_VIEW_MARGIN_DEG`]) about `forward`, its up the camera's with no roll.
pub(super) fn in_widened_frustum(offset: [i32; 3], forward: [f32; 3], half: [f32; 2]) -> bool {
    let f = glam::Vec3::from(forward).normalize_or_zero();
    let right = f.cross(glam::Vec3::Z).normalize_or(glam::Vec3::NEG_Y);
    let up = right.cross(f);
    let o = glam::Vec3::from(offset.map(|v| v as f32));
    let z = o.dot(f);
    if z <= 0.0 {
        return false;
    }
    let m = QUEUE_VIEW_MARGIN_DEG.to_radians();
    o.dot(right).abs().atan2(z) <= half[0] + m && o.dot(up).abs().atan2(z) <= half[1] + m
}

/// The distance bands of the weight log ([`QueueStats::weight_census`]): chunks 0 to 2, 3 to
/// 5, 6 to 10, over 10.
pub(super) fn distance_band(len: f32) -> usize {
    if len < 2.5 {
        0
    } else if len < 5.5 {
        1
    } else if len < 10.5 {
        2
    } else {
        3
    }
}

/// The angle bands of the weight log: 0, under 0.2, 0.2 to 0.4, over 0.4.
pub(super) fn angle_band(a: f32) -> usize {
    if a <= 0.0 {
        0
    } else if a < 0.2 {
        1
    } else if a <= 0.4 {
        2
    } else {
        3
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ADR-0048's angle term at the fly-through's view: a 70 degree vertical field at 16:9 is
    /// 51.2 degrees horizontally, the cone 66.2 degrees every way. A chunk ahead reads 0, one
    /// straight behind (180 - 66.2) / 180, one behind in the near shell 0; at an angle weight
    /// of 0 every score is the distance (times the refine weight for a coarse block).
    #[test]
    fn the_view_cone_scores_a_chunk_by_its_angle() {
        let half = view_half_angle(70f32.to_radians(), 16.0 / 9.0);
        let want = (35f32.to_radians().tan() * 16.0 / 9.0).atan();
        assert!((half - want).abs() < 1e-6, "{half} {want}");
        assert!(
            (half.to_degrees() - 51.2).abs() < 0.05,
            "{}",
            half.to_degrees()
        );
        // The wider of the two, not the corner's: a tall view takes its vertical half-angle.
        assert!((view_half_angle(70f32.to_radians(), 0.5) - 35f32.to_radians()).abs() < 1e-6);
        assert!((view_half_angles(70f32.to_radians(), 16.0 / 9.0)[0] - want).abs() < 1e-6);
        let cam = [2, 0, 0];
        let forward = [1.0, 0.0, 0.0];
        let off = |c: [i32; 3]| [0, 1, 2].map(|k| c[k] - cam[k]);
        let a = |c: [i32; 3]| angle_term(off(c), forward, half, 1);
        assert_eq!(a([7, 0, 0]), 0.0);
        let behind = a([-3, 0, 0]);
        let cone = half.to_degrees() + QUEUE_VIEW_MARGIN_DEG;
        assert!((cone - 66.2).abs() < 0.05, "{cone}");
        assert!((behind - (180.0 - 66.2) / 180.0).abs() < 0.005, "{behind}");
        assert!((behind - 0.632).abs() < 0.005, "{behind}");
        assert_eq!(a([1, 0, 0]), 0.0, "behind, in the shell");
        // Just inside and just outside the cone, off to the side.
        let side = |deg: f32| {
            let r = deg.to_radians();
            angle_term(
                [(100.0 * r.cos()) as i32, (100.0 * r.sin()) as i32, 0],
                forward,
                half,
                1,
            )
        };
        assert_eq!(side(64.0), 0.0);
        assert!(side(70.0) > 0.0 && side(70.0) < 0.03, "{}", side(70.0));
        for c in [[7, 0, 0], [-3, 0, 0], [1, 0, 0], [2, 5, -3]] {
            let o = off(c);
            let len = (o.iter().map(|v| (v * v) as f32).sum::<f32>()).sqrt();
            let t = angle_term(o, forward, half, 1);
            assert_eq!(queue_score(o, t, false, 0.0, 2.0), len, "{c:?}");
            assert_eq!(queue_score(o, t, true, 0.0, 2.0), 2.0 * len, "{c:?}");
        }
        // The weight scales the angle: behind at |o| 5 and weight 4 scores 5 x (1 + 4 x 0.632).
        let s = queue_score(off([-3, 0, 0]), behind, false, 4.0, 2.0);
        assert!((s - 5.0 * (1.0 + 4.0 * behind)).abs() < 1e-4, "{s}");
        // The widened frustum is narrower than the cone up and down: 60 degrees up is inside
        // the cone (66.2) and outside the frustum (35 + 15).
        let up = [100, 0, 173];
        assert_eq!(angle_term(up, forward, half, 1), 0.0);
        assert!(!in_widened_frustum(
            up,
            forward,
            view_half_angles(70f32.to_radians(), 16.0 / 9.0)
        ));
        assert!(in_widened_frustum(
            [100, 0, 50],
            forward,
            view_half_angles(70f32.to_radians(), 16.0 / 9.0)
        ));
    }
}
