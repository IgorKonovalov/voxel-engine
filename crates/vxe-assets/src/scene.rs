//! A `.vox` scene as prefabs and placements for the world: the scene graph walked with its
//! hidden nodes and layers left out, reflected frames loaded as mirrored models under a proper
//! rotation, and every model scaled by a whole factor.
//!
//! A frame's `_r` byte is a signed permutation; MagicaVoxel writes a reflection (determinant
//! -1) when a group is mirrored. The composed frame of a placement is a reflection when an odd
//! number of reflections lie on its path: the model is then mirrored along its x axis and
//! placed under the composed frame times that mirror, a proper rotation. A model's box maps
//! onto the placed box the same way for a reflection as for a rotation: destination axis `k`
//! reads source axis `axis[k]`, counted from the far side when the row is negative. Everything
//! here is integer arithmetic.

use std::collections::BTreeMap;
use std::sync::Arc;

use vxe_world::{AIR, MaterialId, Prefab, Rot, VoxelCoord};

use crate::map::{Slot, VoxMap};
use crate::vox::{MAX_PLACEMENTS, Node, Placement, VoxError, VoxFile, VoxModel};

/// The largest whole scale a scene loads at: a 256-voxel model at 4 spans 1024 voxels, the
/// world's default `edit_axis_max`.
pub const MAX_SCALE: u32 = 4;

/// Why a scene did not load.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SceneError {
    /// The scene graph is broken or a composed translation leaves `i32`.
    Vox(VoxError),
    /// A scale outside `1..=MAX_SCALE`.
    Scale(u32),
    /// A placed model uses palette indices the map leaves [`Slot::Unset`], ascending.
    Unmapped { model: usize, indices: Vec<u8> },
}

impl std::fmt::Display for SceneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Vox(e) => e.fmt(f),
            Self::Scale(k) => write!(f, "scale {k} is outside 1..={MAX_SCALE}"),
            Self::Unmapped { model, indices } => {
                write!(
                    f,
                    "model {model} uses palette indices with no mapping: {indices:?}"
                )
            }
        }
    }
}

impl std::error::Error for SceneError {}

impl From<VoxError> for SceneError {
    fn from(e: VoxError) -> Self {
        Self::Vox(e)
    }
}

/// How a scene loads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SceneOptions {
    /// Each `.vox` voxel becomes `scale³` world voxels; `1..=MAX_SCALE`.
    pub scale: u32,
    /// Leave out nodes whose `_hidden` is 1 and transforms on a hidden layer.
    pub honour_hidden: bool,
}

impl Default for SceneOptions {
    /// Scale 1, hidden nodes and layers left out.
    fn default() -> Self {
        Self {
            scale: 1,
            honour_hidden: true,
        }
    }
}

/// A signed permutation matrix, a rotation or a reflection: row `k` is `±e[axis[k]]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Frame {
    axis: [u8; 3],
    neg: [bool; 3],
}

impl Frame {
    const IDENTITY: Self = Self {
        axis: [0, 1, 2],
        neg: [false; 3],
    };

    fn of_rot(r: Rot) -> Self {
        let rows = [0, 1, 2].map(|k| r.row(k));
        Self {
            axis: rows.map(|(a, _)| a),
            neg: rows.map(|(_, n)| n),
        }
    }

    /// A transform's frame: its rotation, or for a reflected `_r` byte its full matrix.
    fn of_node(file: &VoxFile, id: u32, rot: Rot) -> Self {
        match file.reflected.get(&id) {
            Some(&b) => Self {
                axis: [b & 3, (b >> 2) & 3, 3 - (b & 3) - ((b >> 2) & 3)],
                neg: [b >> 4 & 1 != 0, b >> 5 & 1 != 0, b >> 6 & 1 != 0],
            },
            None => Self::of_rot(rot),
        }
    }

    /// This frame followed by `other`, as [`Rot::then`].
    fn then(self, other: Self) -> Self {
        let mut axis = [0; 3];
        let mut neg = [false; 3];
        for k in 0..3 {
            let a = usize::from(other.axis[k]);
            axis[k] = self.axis[a];
            neg[k] = other.neg[k] ^ self.neg[a];
        }
        Self { axis, neg }
    }

    fn apply(self, v: [i32; 3]) -> [i32; 3] {
        [0, 1, 2].map(|k| {
            let c = v[usize::from(self.axis[k])];
            if self.neg[k] { c.wrapping_neg() } else { c }
        })
    }

    /// The proper rotation and whether the model is mirrored along x first: the frame itself,
    /// or the frame times `diag(-1, 1, 1)`.
    fn split(self) -> (Rot, bool) {
        if let Some(r) = Rot::from_rows(self.axis, self.neg) {
            return (r, false);
        }
        let mut neg = self.neg;
        let k = self
            .axis
            .iter()
            .position(|&a| a == 0)
            .expect("a permutation");
        neg[k] = !neg[k];
        let r = Rot::from_rows(self.axis, neg).expect("one sign flipped makes it proper");
        (r, true)
    }
}

/// A `.vox` scene for the world: one prefab per placed model (and per mirrored model), and the
/// placements, each the prefab's index, the minimum corner of its turned box and the turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VoxScene {
    pub prefabs: Vec<Arc<Prefab>>,
    pub placements: Vec<(usize, VoxelCoord, Rot)>,
    /// Each layer that holds a placement, by ascending layer id: its name (`layer <id>` when
    /// it has none) and the indices into `placements` of the placements under it.
    pub layers: Vec<(String, Vec<usize>)>,
    /// The inclusive voxel box every placement fits in; `([0; 3], [-1; 3])` with none.
    pub bounds: ([i32; 3], [i32; 3]),
}

/// One placement found by the walk.
struct Found {
    model: usize,
    translation: [i32; 3],
    frame: Frame,
    layer: Option<i32>,
}

impl VoxScene {
    /// Loads `file`'s scene with `map` deciding each palette index (index 0, the empty cells,
    /// included). Fails on a scale outside `1..=MAX_SCALE`, a placed model with an undecided
    /// index, or a scene graph the walk refuses.
    pub fn load(file: &VoxFile, map: &VoxMap, opts: SceneOptions) -> Result<Self, SceneError> {
        if !(1..=MAX_SCALE).contains(&opts.scale) {
            return Err(SceneError::Scale(opts.scale));
        }
        let found = walk(file, opts.honour_hidden)?;
        let k = opts.scale;
        let mut prefabs: Vec<Arc<Prefab>> = Vec::new();
        let mut by_model: BTreeMap<(usize, bool), usize> = BTreeMap::new();
        let mut placements = Vec::with_capacity(found.len());
        let mut layers: BTreeMap<i32, Vec<usize>> = BTreeMap::new();
        let mut bounds: Option<([i32; 3], [i32; 3])> = None;
        for f in found {
            let (rot, mirror) = f.frame.split();
            let index = match by_model.get(&(f.model, mirror)) {
                Some(&i) => i,
                None => {
                    let p =
                        model_prefab(&file.models[f.model], map, mirror, k).map_err(|indices| {
                            SceneError::Unmapped {
                                model: f.model,
                                indices,
                            }
                        })?;
                    prefabs.push(Arc::new(p));
                    by_model.insert((f.model, mirror), prefabs.len() - 1);
                    prefabs.len() - 1
                }
            };
            let size = prefabs[index].size();
            let turned = rot.apply_size(size);
            let mut t = [0i32; 3];
            for (tc, &fc) in t.iter_mut().zip(&f.translation) {
                *tc = fc
                    .checked_mul(k as i32)
                    .ok_or(SceneError::Vox(VoxError::Malformed {
                        chunk: *b"nTRN",
                        what: "a scaled _t overflows i32",
                    }))?;
            }
            let origin = Placement {
                model: f.model,
                translation: t,
                rot,
            }
            .min_corner(turned);
            if let Some(l) = f.layer {
                layers.entry(l).or_default().push(placements.len());
            }
            if !turned.contains(&0) {
                let hi = [0, 1, 2].map(|c| origin.0[c].saturating_add(turned[c] as i32 - 1));
                bounds = Some(match bounds {
                    None => (origin.0, hi),
                    Some((a, b)) => (
                        [0, 1, 2].map(|c| a[c].min(origin.0[c])),
                        [0, 1, 2].map(|c| b[c].max(hi[c])),
                    ),
                });
            }
            placements.push((index, origin, rot));
        }
        let layers = layers
            .into_iter()
            .map(|(l, v)| {
                let name = file
                    .layers
                    .get(&l)
                    .map(|x| x.name.clone())
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| format!("layer {l}"));
                (name, v)
            })
            .collect();
        Ok(Self {
            prefabs,
            placements,
            layers,
            bounds: bounds.unwrap_or(([0; 3], [-1; 3])),
        })
    }

    /// The box's size in voxels: zero with no placement.
    pub fn size(&self) -> [u32; 3] {
        let (a, b) = self.bounds;
        [0, 1, 2].map(|k| (i64::from(b[k]) - i64::from(a[k]) + 1).max(0) as u32)
    }

    /// The placements moved by `by` voxels, as `vxe_world::StampedSource` and
    /// `World::stamp` take them.
    pub fn placed_at(&self, by: [i32; 3]) -> Vec<(Arc<Prefab>, VoxelCoord, Rot)> {
        self.placements
            .iter()
            .map(|&(i, o, r)| {
                let o = VoxelCoord([0, 1, 2].map(|k| o.0[k].wrapping_add(by[k])));
                (self.prefabs[i].clone(), o, r)
            })
            .collect()
    }
}

/// `model` as a prefab through `map`, mirrored along x when `mirror`, each voxel repeated
/// `k` times per axis. Fails with the undecided indices it uses, ascending.
fn model_prefab(model: &VoxModel, map: &VoxMap, mirror: bool, k: u32) -> Result<Prefab, Vec<u8>> {
    let s = model.size.map(|v| v as usize);
    let mut index = vec![0u8; s[0] * s[1] * s[2]];
    for &[x, y, z, i] in &model.voxels {
        if (0..3).any(|a| u32::from([x, y, z][a]) >= model.size[a]) {
            continue;
        }
        let x = if mirror {
            s[0] - 1 - usize::from(x)
        } else {
            usize::from(x)
        };
        index[x + s[0] * (usize::from(y) + s[1] * usize::from(z))] = i;
    }
    let mut used = [false; 256];
    for &i in &index {
        used[usize::from(i)] = true;
    }
    let unset: Vec<u8> = (0..=255u8)
        .filter(|&i| used[usize::from(i)] && map.get(i) == Slot::Unset)
        .collect();
    if !unset.is_empty() {
        return Err(unset);
    }
    let slot = |i: u8| -> Option<MaterialId> {
        match map.get(i) {
            Slot::Air => Some(AIR),
            Slot::Material(m) => Some(m),
            Slot::Skip | Slot::Unset => None,
        }
    };
    let size = model.size.map(|v| v * k);
    Ok(Prefab::from_fn(size, |l| {
        let [x, y, z] = l.map(|v| (v / k) as usize);
        slot(index[x + s[0] * (y + s[1] * z)])
    }))
}

/// Every placement of `file`'s scene graph with its composed frame, depth first in child
/// order as [`VoxFile::placements`], reflections allowed, leaving out (when `honour_hidden`)
/// every node whose `_hidden` is 1 and every transform on a hidden layer. A placement's layer
/// is that of the first transform on its path, from the root, with a layer id of 0 or more.
fn walk(file: &VoxFile, honour_hidden: bool) -> Result<Vec<Found>, SceneError> {
    let mut out = Vec::new();
    if file.nodes.is_empty() {
        for model in 0..file.models.len() {
            out.push(Found {
                model,
                translation: [0; 3],
                frame: Frame::IDENTITY,
                layer: None,
            });
        }
        return Ok(out);
    }
    // Subtrees that place nothing are not entered, so the walk is bounded by the placements
    // times the graph's depth, as `VoxFile::placements`'s.
    let counts = file.check_graph()?;
    let places = |c: &u32| counts.get(c).is_some_and(|&n| n > 0);
    let hidden_layer = |l: i32| file.layers.get(&l).is_some_and(|x| x.hidden);
    let mut stack = vec![(0u32, [0i32; 3], Frame::IDENTITY, None::<i32>)];
    while let Some((id, t, frame, layer)) = stack.pop() {
        if honour_hidden && file.hidden.contains(&id) {
            continue;
        }
        match &file.nodes[&id] {
            Node::Transform {
                child,
                translation,
                rot,
            } => {
                if !places(child) {
                    continue;
                }
                let mut layer = layer;
                if layer.is_none() && file.layer_of(id) >= 0 {
                    let l = file.layer_of(id);
                    if honour_hidden && hidden_layer(l) {
                        continue;
                    }
                    layer = Some(l);
                }
                let d = frame.apply(*translation);
                let mut sum = [0; 3];
                for k in 0..3 {
                    sum[k] = t[k].checked_add(d[k]).ok_or(VoxError::Malformed {
                        chunk: *b"nTRN",
                        what: "composed _t overflows i32",
                    })?;
                }
                let own = Frame::of_node(file, id, *rot);
                stack.push((*child, sum, own.then(frame), layer));
            }
            Node::Group { children } => {
                stack.extend(
                    children
                        .iter()
                        .rev()
                        .filter(|c| places(c))
                        .map(|&c| (c, t, frame, layer)),
                );
            }
            Node::Shape { models } => {
                for &m in models {
                    if out.len() == MAX_PLACEMENTS {
                        return Err(VoxError::Malformed {
                            chunk: *b"nSHP",
                            what: "places more models than MAX_PLACEMENTS",
                        }
                        .into());
                    }
                    out.push(Found {
                        model: m as usize,
                        translation: t,
                        frame,
                        layer,
                    });
                }
            }
        }
    }
    Ok(out)
}
