//! A part's placement as `f32` affine maps, and [`DisplayPose`]: one entity's parts placed for
//! drawing, from one pose or between two.

use vxe_entity::Model;

/// A part's placement as `f32` affine maps: `part_to_world` takes a point in model voxels to
/// world voxels; `world_to_part` is its inverse. Rows of `[m | t]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PartXform {
    pub(crate) part_to_world: [[f32; 4]; 3],
    pub(crate) world_to_part: [[f32; 4]; 3],
}

impl PartXform {
    /// The part's placement from a composed pose, through `ComposedPose::part_point` (exact in
    /// fixed point): its pivot's world point and the world points 256 model voxels along each
    /// axis from it.
    fn of(pose: &vxe_entity::ComposedPose<'_>, part: usize) -> Option<Self> {
        let p = &pose.model().parts()[part];
        let fx = |l: [i32; 3]| pose.part_point(part, l).map(|f| f.0);
        let base = fx(p.pivot)?;
        const REACH: i32 = 1 << 16;
        let mut m = [[0f32; 3]; 3];
        for (k, col) in (0..3).zip([0usize, 1, 2]) {
            let mut l = p.pivot;
            l[k] += REACH;
            let q = fx(l)?;
            for r in 0..3 {
                // World 1/256 voxels per 256 model voxels: world voxels per model voxel.
                m[r][col] = (q[r] - base[r]) as f32 / (256.0 * 256.0);
            }
        }
        let pivot_w = base.map(|v| v as f32 / 256.0);
        Some(Self::from_parts(
            m,
            p.pivot.map(|v| v as f32 / 256.0),
            pivot_w,
            p.detail,
        ))
    }

    /// The placement whose linear part is `m` (a rotation over `detail`) and which takes the
    /// model point `pivot_m` to the world point `pivot_w`.
    fn from_parts(m: [[f32; 3]; 3], pivot_m: [f32; 3], pivot_w: [f32; 3], detail: u8) -> Self {
        let mut p2w = [[0f32; 4]; 3];
        for r in 0..3 {
            p2w[r][..3].copy_from_slice(&m[r]);
            p2w[r][3] = pivot_w[r] - (0..3).map(|c| m[r][c] * pivot_m[c]).sum::<f32>();
        }
        // The linear part is a rotation over `detail`, so its inverse is its transpose times
        // `detail^2`.
        let d2 = f32::from(detail) * f32::from(detail);
        let mut w2p = [[0f32; 4]; 3];
        for r in 0..3 {
            for c in 0..3 {
                w2p[r][c] = m[c][r] * d2;
            }
            w2p[r][3] = pivot_m[r] - (0..3).map(|c| w2p[r][c] * pivot_w[c]).sum::<f32>();
        }
        Self {
            part_to_world: p2w,
            world_to_part: w2p,
        }
    }

    /// The world box of `[0, size]` in model voxels through `xf`.
    pub(super) fn world_box(xf: &[[f32; 4]; 3], size: [u32; 3]) -> [[f32; 3]; 2] {
        let mut b = [[f32::MAX; 3], [f32::MIN; 3]];
        for i in 0..8 {
            let c = [0, 1, 2].map(|k| if i >> k & 1 == 1 { size[k] as f32 } else { 0.0 });
            for r in 0..3 {
                let w = xf[r][3] + (0..3).map(|k| xf[r][k] * c[k]).sum::<f32>();
                b[0][r] = b[0][r].min(w);
                b[1][r] = b[1][r].max(w);
            }
        }
        b
    }
}

/// One entity's parts placed for drawing, as `f32` affine maps: from one pose
/// ([`DisplayPose::of`]) or between two ([`DisplayPose::between`]), so a viewer draws between
/// two ticks of a fixed-point clip. Drawing only: the entity queries read the pose in the set.
///
/// A display pose kept across frames and refreshed with [`DisplayPose::of_into`] or
/// [`DisplayPose::between_into`] reuses its storage: once grown to its model's part count it
/// allocates nothing.
#[derive(Clone, Debug, Default)]
pub struct DisplayPose {
    /// Each part's placement and its detail.
    pub(super) parts: Vec<(PartXform, u8)>,
    /// The two compositions' storage, kept for the next refresh.
    scratch: [vxe_entity::ComposeScratch; 2],
}

impl PartialEq for DisplayPose {
    /// Two display poses are equal when they place the same parts alike; the kept storage
    /// does not count.
    fn eq(&self, o: &Self) -> bool {
        self.parts == o.parts
    }
}

impl DisplayPose {
    /// `model`'s parts as `pose` places them; `None` when the pose does not fit the model.
    pub fn of(model: &Model, pose: &vxe_entity::Pose) -> Option<Self> {
        let mut d = Self::default();
        d.of_into(model, pose).then_some(d)
    }

    /// [`DisplayPose::of`] into `self`, reusing its storage; `false`, and `self` emptied, when
    /// the pose does not fit the model.
    pub fn of_into(&mut self, model: &Model, pose: &vxe_entity::Pose) -> bool {
        self.parts.clear();
        let scratch = std::mem::take(&mut self.scratch[0]);
        let c = match pose.compose_with(model, scratch) {
            Ok(c) => c,
            Err(s) => {
                self.scratch[0] = s;
                return false;
            }
        };
        let mut fits = true;
        for k in 0..model.parts().len() {
            let Some(x) = PartXform::of(&c, k) else {
                fits = false;
                break;
            };
            self.parts.push((x, model.parts()[k].detail));
        }
        self.scratch[0] = c.into_scratch();
        if !fits {
            self.parts.clear();
        }
        fits
    }

    /// `model`'s parts a fraction `alpha` (0 to 1) of the way from pose `a` to pose `b`: per
    /// part, its pivot's world point linear between the two, and its turn a normalised linear
    /// blend of the two turns along the shorter arc. Each part blends its own composed
    /// placement, so a child stays on its parent's pivot only to the blend's accuracy. `None`
    /// when a pose does not fit the model.
    pub fn between(
        model: &Model,
        a: &vxe_entity::Pose,
        b: &vxe_entity::Pose,
        alpha: f32,
    ) -> Option<Self> {
        let mut d = Self::default();
        d.between_into(model, a, b, alpha).then_some(d)
    }

    /// [`DisplayPose::between`] into `self`, reusing its storage; `false`, and `self` emptied,
    /// when a pose does not fit the model.
    pub fn between_into(
        &mut self,
        model: &Model,
        a: &vxe_entity::Pose,
        b: &vxe_entity::Pose,
        alpha: f32,
    ) -> bool {
        self.parts.clear();
        let [sa, sb] = std::mem::take(&mut self.scratch);
        let (ca, cb) = match (a.compose_with(model, sa), b.compose_with(model, sb)) {
            (Ok(ca), Ok(cb)) => (ca, cb),
            (ra, rb) => {
                self.scratch = [
                    ra.map_or_else(|s| s, |c| c.into_scratch()),
                    rb.map_or_else(|s| s, |c| c.into_scratch()),
                ];
                return false;
            }
        };
        let fits = self.blend_parts(model, &ca, &cb, f64::from(alpha));
        self.scratch = [ca.into_scratch(), cb.into_scratch()];
        if !fits {
            self.parts.clear();
        }
        fits
    }

    /// Pushes each part of `model` blended `alpha` of the way from `ca` to `cb`; `false` at
    /// the first part either composition cannot place.
    fn blend_parts(
        &mut self,
        model: &Model,
        ca: &vxe_entity::ComposedPose<'_>,
        cb: &vxe_entity::ComposedPose<'_>,
        alpha: f64,
    ) -> bool {
        for (k, p) in model.parts().iter().enumerate() {
            let (Some(xa), Some(xb)) = (PartXform::of(ca, k), PartXform::of(cb, k)) else {
                return false;
            };
            let d = f64::from(p.detail);
            let pivot_m = p.pivot.map(|v| v as f32 / 256.0);
            let qa = quat_of(&xa.part_to_world, d);
            let mut qb = quat_of(&xb.part_to_world, d);
            if (0..4).map(|i| qa[i] * qb[i]).sum::<f64>() < 0.0 {
                qb = qb.map(|v| -v);
            }
            let q: [f64; 4] = std::array::from_fn(|i| qa[i] + (qb[i] - qa[i]) * alpha);
            let r = matrix_of(q);
            let m = r.map(|row| row.map(|v| (v / d) as f32));
            let (pa, pb) = (
                apply(&xa.part_to_world, pivot_m),
                apply(&xb.part_to_world, pivot_m),
            );
            let pivot_w: [f32; 3] = std::array::from_fn(|i| {
                (f64::from(pa[i]) + (f64::from(pb[i]) - f64::from(pa[i])) * alpha) as f32
            });
            self.parts.push((
                PartXform::from_parts(m, pivot_m, pivot_w, p.detail),
                p.detail,
            ));
        }
        true
    }

    /// The parts placed.
    pub fn len(&self) -> usize {
        self.parts.len()
    }

    /// Whether no part is placed.
    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }

    /// Part `part`'s map from model voxels to world voxels, rows of `[m | t]`.
    pub fn part_to_world(&self, part: usize) -> Option<[[f32; 4]; 3]> {
        self.parts.get(part).map(|(x, _)| x.part_to_world)
    }
}

/// `xf` applied to the point `p`.
fn apply(xf: &[[f32; 4]; 3], p: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|r| xf[r][3] + (0..3).map(|c| xf[r][c] * p[c]).sum::<f32>())
}

/// The unit quaternion `(w, x, y, z)` of the rotation `detail` times `xf`'s linear part
/// (Shepperd's method: the largest of the four diagonal sums is the one divided by).
fn quat_of(xf: &[[f32; 4]; 3], detail: f64) -> [f64; 4] {
    let m = |r: usize, c: usize| f64::from(xf[r][c]) * detail;
    let tr = m(0, 0) + m(1, 1) + m(2, 2);
    let q = if tr > 0.0 {
        let s = (tr + 1.0).sqrt() * 2.0;
        [
            0.25 * s,
            (m(2, 1) - m(1, 2)) / s,
            (m(0, 2) - m(2, 0)) / s,
            (m(1, 0) - m(0, 1)) / s,
        ]
    } else if m(0, 0) > m(1, 1) && m(0, 0) > m(2, 2) {
        let s = (1.0 + m(0, 0) - m(1, 1) - m(2, 2)).sqrt() * 2.0;
        [
            (m(2, 1) - m(1, 2)) / s,
            0.25 * s,
            (m(0, 1) + m(1, 0)) / s,
            (m(0, 2) + m(2, 0)) / s,
        ]
    } else if m(1, 1) > m(2, 2) {
        let s = (1.0 + m(1, 1) - m(0, 0) - m(2, 2)).sqrt() * 2.0;
        [
            (m(0, 2) - m(2, 0)) / s,
            (m(0, 1) + m(1, 0)) / s,
            0.25 * s,
            (m(1, 2) + m(2, 1)) / s,
        ]
    } else {
        let s = (1.0 + m(2, 2) - m(0, 0) - m(1, 1)).sqrt() * 2.0;
        [
            (m(1, 0) - m(0, 1)) / s,
            (m(0, 2) + m(2, 0)) / s,
            (m(1, 2) + m(2, 1)) / s,
            0.25 * s,
        ]
    };
    let n = q.iter().map(|v| v * v).sum::<f64>().sqrt();
    q.map(|v| v / n)
}

/// The rotation matrix of the quaternion `(w, x, y, z)`, normalised first.
fn matrix_of(q: [f64; 4]) -> [[f64; 3]; 3] {
    let n = q.iter().map(|v| v * v).sum::<f64>().sqrt();
    let [w, x, y, z] = q.map(|v| v / n);
    [
        [
            1.0 - 2.0 * (y * y + z * z),
            2.0 * (x * y - w * z),
            2.0 * (x * z + w * y),
        ],
        [
            2.0 * (x * y + w * z),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z - w * x),
        ],
        [
            2.0 * (x * z - w * y),
            2.0 * (y * z + w * x),
            1.0 - 2.0 * (x * x + y * y),
        ],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use vxe_world::{AIR, Prefab};

    /// A one-part model of a 2-voxel cube at detail 1, its pivot at its corner, and the pose
    /// that translates the part by `t` (1/256 world voxel) and turns it `q`.
    fn one_part(t: [i32; 3], q: vxe_entity::Quat) -> (Model, vxe_entity::Pose) {
        let part = vxe_entity::Part {
            volume: std::sync::Arc::new(Prefab::new([2; 3], vec![AIR; 8], None)),
            parent: None,
            pivot: [0; 3],
            detail: 1,
        };
        let model = Model::new(vec![part]).expect("one part");
        let mut pose = vxe_entity::Pose::rest(&model);
        pose.parts[0].t = t;
        pose.parts[0].q = q;
        (model, pose)
    }

    fn close(a: &[[f32; 4]; 3], b: &[[f32; 4]; 3], tol: f32) -> bool {
        a.iter()
            .flatten()
            .zip(b.iter().flatten())
            .all(|(x, y)| (x - y).abs() <= tol)
    }

    /// Plan 0063 Phase 3 done-when 1: a part translated 0 at one tick and 256 (one voxel) at the
    /// next sits a quarter voxel along at `alpha` 0.25, and `alpha` 0 and 1 give the two ticks'
    /// transforms; a quarter turn about z between the ticks is an eighth turn at 0.5.
    #[test]
    fn display_between_two_ticks_is_between_them() {
        let id = vxe_entity::Quat::IDENTITY;
        let (model, a) = one_part([0; 3], id);
        let (_, b) = one_part([256, 0, 0], id);
        let xa = DisplayPose::of(&model, &a)
            .expect("fits")
            .part_to_world(0)
            .expect("part");
        let xb = DisplayPose::of(&model, &b)
            .expect("fits")
            .part_to_world(0)
            .expect("part");
        let at = |alpha: f32| {
            DisplayPose::between(&model, &a, &b, alpha)
                .expect("fits")
                .part_to_world(0)
                .expect("part")
        };
        let q = at(0.25);
        assert!((q[0][3] - 0.25).abs() <= 1e-4, "{q:?}");
        assert!(q[1][3].abs() <= 1e-4 && q[2][3].abs() <= 1e-4, "{q:?}");
        assert!(close(&at(0.0), &xa, 1e-6), "{:?} against {xa:?}", at(0.0));
        assert!(close(&at(1.0), &xb, 1e-6), "{:?} against {xb:?}", at(1.0));

        // (cos 45, 0, 0, sin 45) in Q14: a quarter turn about z.
        let (_, c) = one_part([0; 3], vxe_entity::Quat([11585, 0, 0, 11585]));
        let h = DisplayPose::between(&model, &a, &c, 0.5)
            .expect("fits")
            .part_to_world(0)
            .expect("part");
        let (s, k) = (
            std::f32::consts::FRAC_PI_8.sin(),
            std::f32::consts::FRAC_PI_8.cos(),
        );
        // An eighth turn: x maps to (cos 45, sin 45), so m00 = cos^2 - sin^2 of 22.5 degrees.
        let c45 = k * k - s * s;
        assert!(
            (h[0][0] - c45).abs() <= 1e-4 && (h[1][0] - c45).abs() <= 1e-4,
            "{h:?}"
        );
    }

    /// A display pose refreshed in place equals the one built new, keeps its parts' storage
    /// (the same buffer, so no allocation) from the second refresh on, and empties itself on
    /// a pose that does not fit the model.
    #[test]
    fn a_display_refreshed_in_place_reuses_its_storage() {
        let id = vxe_entity::Quat::IDENTITY;
        let (model, a) = one_part([0; 3], id);
        let (_, b) = one_part([256, 0, 0], vxe_entity::Quat([11585, 0, 0, 11585]));
        let mut d = DisplayPose::default();
        assert!(d.between_into(&model, &a, &b, 0.25));
        assert_eq!(
            Some(&d),
            DisplayPose::between(&model, &a, &b, 0.25).as_ref()
        );
        let held = (d.parts.as_ptr(), d.parts.capacity());
        for k in 0..4 {
            let alpha = k as f32 / 4.0;
            assert!(d.between_into(&model, &a, &b, alpha));
            assert_eq!(
                Some(&d),
                DisplayPose::between(&model, &a, &b, alpha).as_ref()
            );
            assert!(d.of_into(&model, &b));
            assert_eq!(Some(&d), DisplayPose::of(&model, &b).as_ref());
            assert_eq!((d.parts.as_ptr(), d.parts.capacity()), held, "refresh {k}");
        }
        let unfit = vxe_entity::Pose {
            parts: Vec::new(),
            ..a.clone()
        };
        assert!(!d.of_into(&model, &unfit));
        assert!(d.is_empty());
        assert!(!d.between_into(&model, &a, &unfit, 0.5));
        assert!(d.is_empty());
        assert!(d.of_into(&model, &a), "the storage survives a refusal");
    }
}
