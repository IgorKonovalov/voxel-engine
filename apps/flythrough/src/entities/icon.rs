//! A still picture per library entry (ADR-0075): the entry's model in its rest pose, traced on
//! the CPU through `EntitySet::raycast` from a fixed orthographic view 30 degrees down and 225
//! degrees round (the camera off the model's -x, -y side, so a model facing -y shows its front
//! and its -x flank), fitted to 5/8 of a [`ICON_PX`]-pixel tile, each pixel the hit voxel's
//! material albedo shaded by a fixed Lambert from the entered face's world normal. Integer
//! arithmetic from the model to the bytes, so an icon is the same on every platform. Icons are
//! made on a worker thread ([`Icons`]) and drawn as overlay tiles once they arrive; until then
//! the chooser's row and the belt's slot show the entry's words.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use vxe_entity::{ComposedPose, EntitySet, Model, Pose};
use vxe_world::{Face, Fixed3, MaterialTable};

/// An icon's side in pixels: the overlay's tile.
pub const ICON_PX: u32 = vxe_render::TILE_PX;
/// The model's larger projected side, in pixels: 5/8 of the tile.
pub const FIT_PX: i64 = (ICON_PX as i64) * 5 / 8;

/// The view's axes in 1/4096 (`round(4096 x` the unit vector`)`): `FORWARD` from the camera
/// into the scene, 30 degrees down and toward +x, +y; `RIGHT` horizontal; `UP` their cross.
const FORWARD: [i64; 3] = [2508, 2508, -2048];
const RIGHT: [i64; 3] = [2896, -2896, 0];
const UP: [i64; 3] = [1448, 1448, 3547];
/// The light toward which a face is lit, `(-2, -3, 6) / 7`: a unit vector in sevenths, so the
/// three faces the view shows (-x, -y, +z) take three distinct shades.
const LIGHT: [i64; 3] = [-2, -3, 6];
/// A face's shade in 256ths: `AMBIENT` plus `DIFFUSE` times its cosine to the light.
const AMBIENT: i64 = 96;
const DIFFUSE: i64 = 160;
/// The length, in 1/256 model voxel, along which a face's normal is carried into the world.
const NORMAL_REACH: i32 = 1 << 12;

/// The icon of `model` against `table`: [`ICON_PX`] x [`ICON_PX`] RGBA8 pixels, row-major from
/// the top-left; a pixel with no hit is transparent (all zero). `None` for a model with no
/// solid voxel the view meets.
pub fn icon(model: &Arc<Model>, table: &MaterialTable) -> Option<Vec<u8>> {
    let pose = Pose::rest(model);
    let composed = pose.compose(model)?;
    // The corners of every solid voxel in world 1/256 voxel: the fit is the silhouette's.
    let mut corners = Vec::new();
    for (k, part) in model.parts().iter().enumerate() {
        let v = &part.volume;
        let [sx, sy, sz] = v.size().map(|n| n as i32);
        for z in 0..sz {
            for y in 0..sy {
                for x in 0..sx {
                    if v.get([x, y, z]).is_none_or(|m| m == vxe_world::AIR) {
                        continue;
                    }
                    for c in 0..8 {
                        let local = [x, y, z].map(|n| n * 256);
                        let local = [0, 1, 2].map(|a| local[a] + 256 * (c >> a & 1));
                        corners.push(composed.part_point(k, local)?.0);
                    }
                }
            }
        }
    }
    let (mut lo, mut hi) = ([i64::MAX; 3], [i64::MIN; 3]);
    for p in &corners {
        for a in 0..3 {
            lo[a] = lo[a].min(p[a]);
            hi[a] = hi[a].max(p[a]);
        }
    }
    if corners.is_empty() {
        return None;
    }
    let centre = [0, 1, 2].map(|a| (lo[a] + hi[a]) / 2);
    // The silhouette's extents on the view's axes, in 1/256 voxel x 4096, about the centre.
    let dot = |a: [i64; 3], b: [i64; 3]| (0..3).map(|i| a[i] * b[i]).sum::<i64>();
    let (mut r, mut u) = ([i64::MAX, i64::MIN], [i64::MAX, i64::MIN]);
    for c in &corners {
        let p = [0, 1, 2].map(|a| c[a] - centre[a]);
        let (pr, pu) = (dot(p, RIGHT), dot(p, UP));
        r = [r[0].min(pr), r[1].max(pr)];
        u = [u[0].min(pu), u[1].max(pu)];
    }
    let extent = (r[1] - r[0]).max(u[1] - u[0]).max(1);
    let (rc, uc) = ((r[0] + r[1]) / 2, (u[0] + u[1]) / 2);
    let mut set = EntitySet::new();
    set.insert(model.clone(), pose.clone()).ok()?;
    // A start well behind the box along the view: its diagonal in 1/256 voxel.
    let back = (0..3).map(|a| hi[a] - lo[a]).sum::<i64>() + 256;
    let px = i64::from(ICON_PX);
    let mut out = vec![0u8; (ICON_PX * ICON_PX * 4) as usize];
    let mut any = false;
    for y in 0..px {
        for x in 0..px {
            // The pixel centre's offsets along RIGHT and UP, in the projections' units.
            let sr = rc + (2 * x + 1 - px) * extent / (2 * FIT_PX);
            let su = uc + (px - 2 * y - 1) * extent / (2 * FIT_PX);
            let origin: [i64; 3] = [0, 1, 2].map(|a| {
                let on_plane = (RIGHT[a] * sr + UP[a] * su) / (4096 * 4096);
                centre[a] + on_plane - FORWARD[a] * back / 4096
            });
            let Some(hit) = set.raycast(Fixed3(origin), FORWARD, 1 << 40) else {
                continue;
            };
            let Some(row) = table.get(hit.material) else {
                continue;
            };
            let shade = match hit.face {
                Some(f) => face_shade(&composed, hit.part.into(), f)?,
                None => AMBIENT + DIFFUSE,
            };
            let at = ((y * px + x) * 4) as usize;
            for (c, &a) in row.albedo.iter().enumerate() {
                let v = i64::from((a.clamp(0.0, 1.0) * 255.0).round() as u8);
                out[at + c] = (v * shade / 256) as u8;
            }
            out[at + 3] = 255;
            any = true;
        }
    }
    any.then_some(out)
}

/// The shade in 256ths of part `part`'s face `face` in the rest pose: its outward normal
/// carried into the world by the part's placement, against [`LIGHT`].
fn face_shade(composed: &ComposedPose<'_>, part: usize, face: Face) -> Option<i64> {
    let (axis, sign) = match face {
        Face::NegX => (0, -1),
        Face::PosX => (0, 1),
        Face::NegY => (1, -1),
        Face::PosY => (1, 1),
        Face::NegZ => (2, -1),
        Face::PosZ => (2, 1),
    };
    let mut tip = [0i32; 3];
    tip[axis] = sign * NORMAL_REACH;
    let (a, b) = (
        composed.part_point(part, [0; 3])?.0,
        composed.part_point(part, tip)?.0,
    );
    let n = [0, 1, 2].map(|i| b[i] - a[i]);
    let len = n.iter().map(|v| v.abs()).max().unwrap_or(1).max(1);
    // The cosine in 256ths over the normal's longest component, exact for an axis normal.
    let cos = (0..3).map(|i| n[i] * LIGHT[i]).sum::<i64>() * 256 / (len * 7);
    Some(AMBIENT + DIFFUSE * cos.clamp(0, 256) / 256)
}

/// One entry's icon: the model it was asked for (by [`Model::id`]), whether the worker's
/// answer has arrived and held an icon, and its pixels waiting for the overlay.
#[derive(Clone, Debug, Default)]
struct Slot {
    model: Option<u64>,
    arrived: bool,
    made: bool,
    upload: Option<Vec<u8>>,
}

impl Slot {
    /// Takes the worker's answer for model `id`, unless the slot has moved on.
    fn take(&mut self, id: u64, pixels: Option<Vec<u8>>) {
        if self.model == Some(id) {
            self.arrived = true;
            self.made = pixels.is_some();
            self.upload = pixels;
        }
    }
}

type Job = (usize, u64, Arc<Model>, Arc<MaterialTable>);
type Done = (usize, u64, Option<Vec<u8>>);
/// The worker's ends: the jobs it takes and the icons it gives back.
type Worker = (Sender<Job>, Arc<Mutex<Receiver<Done>>>);

/// The icons of the library's entries and the worker that makes them: an entry's icon is
/// asked for when its model is first built and again when it changes ([`Icons::want`]),
/// made off the frame thread, and picked up by [`Icons::poll`].
#[derive(Clone, Default)]
pub struct Icons {
    slots: Vec<Slot>,
    worker: Option<Worker>,
}

impl std::fmt::Debug for Icons {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Icons").field("slots", &self.slots).finish()
    }
}

impl Icons {
    /// Asks for entry `i`'s icon of `model` unless that model's was asked for already; a
    /// changed model forgets the old icon until the new one arrives.
    pub fn want(&mut self, i: usize, model: &Arc<Model>, table: &MaterialTable) {
        if self.slots.len() <= i {
            self.slots.resize(i + 1, Slot::default());
        }
        let slot = &mut self.slots[i];
        if slot.model == Some(model.id()) {
            return;
        }
        *slot = Slot {
            model: Some(model.id()),
            ..Slot::default()
        };
        let (tx, _) = self.worker.get_or_insert_with(spawn);
        // A worker gone (a panic in a trace) leaves the words in place of the icon.
        let _ = tx.send((i, model.id(), model.clone(), Arc::new(table.clone())));
    }

    /// Takes the icons the worker has made since the last call; one made for a model other
    /// than the entry's current one is dropped.
    pub fn poll(&mut self) {
        let Some((_, rx)) = &self.worker else { return };
        let Ok(rx) = rx.lock() else { return };
        while let Ok((i, id, pixels)) = rx.try_recv() {
            if let Some(slot) = self.slots.get_mut(i) {
                slot.take(id, pixels);
            }
        }
    }

    /// [`Icons::poll`] until every icon asked for has arrived, for the tests.
    pub fn wait(&mut self) {
        while self.slots.iter().any(|s| s.model.is_some() && !s.arrived) {
            let Some((_, rx)) = &self.worker else { return };
            let got = rx.lock().ok().and_then(|rx| rx.recv().ok());
            let Some((i, id, pixels)) = got else { return };
            if let Some(slot) = self.slots.get_mut(i) {
                slot.take(id, pixels);
            }
        }
    }

    /// Whether entry `i`'s icon has arrived and fits the overlay's atlas: its tile is `i`.
    pub fn ready(&self, i: usize) -> bool {
        i < vxe_render::MAX_TILES as usize && self.slots.get(i).is_some_and(|s| s.made)
    }

    /// The icons arrived and not yet given to the overlay, by entry: each tile's index and
    /// pixels, once.
    pub fn uploads(&mut self) -> impl Iterator<Item = (u32, Vec<u8>)> + '_ {
        self.slots
            .iter_mut()
            .enumerate()
            .take(vxe_render::MAX_TILES as usize)
            .filter_map(|(i, s)| Some((i as u32, s.upload.take()?)))
    }
}

/// A worker thread making the icons it is sent, in order.
fn spawn() -> Worker {
    let (tx, jobs) = channel::<Job>();
    let (done, rx) = channel::<Done>();
    let spawned = std::thread::Builder::new()
        .name("entity icons".into())
        .spawn(move || {
            for (i, id, model, table) in jobs {
                if done.send((i, id, icon(&model, &table))).is_err() {
                    break;
                }
            }
        });
    if spawned.is_err() {
        println!("entity icons: no worker thread; the entries show their words");
    }
    (tx, Arc::new(Mutex::new(rx)))
}

impl super::Walkers {
    /// Takes the icons made since the last call, then asks for the icon of every entry whose
    /// model is built and not yet pictured (the built-in walker's from `table`): an icon
    /// asked for in one call arrives at a later one.
    pub fn refresh_icons(&mut self, table: &MaterialTable) {
        self.icons.poll();
        for i in 0..self.library.entries.len() {
            let model = match &self.library.entries[i].source {
                Ok(super::library::Source::Builtin) => self.model(table),
                Ok(super::library::Source::Rig(v)) => v.loaded.as_ref().map(|l| l.model.clone()),
                Err(_) => None,
            };
            if let Some(m) = model {
                self.icons.want(i, &m, table);
            }
        }
    }
}
