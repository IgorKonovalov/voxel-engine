//! The testbed's walker (ADR-0050): a voxel model of four rigid parts made in code, a torso, a
//! head with two glowing eyes and two legs, all at detail 2. `` ` `` places one standing on the
//! looked-at surface, facing the camera; `--entities N` places `N` on a ring around the
//! spawn; `F11` (or `--entities-walk on`) sets them walking, each on a circle from where it
//! stands with its legs swinging. The engine knows no walker: this is the fly-through's
//! entity.

use std::fmt::Write as _;
use std::sync::Arc;

use vxe_entity::{EntityId, EntitySet, Model, Part, PartPose, Pose, Q14_ONE, Quat};
use vxe_render::{MAX_ENTITIES, MAX_ENTITY_PARTS, ModelHandle};
use vxe_world::{AIR, Fixed3, Material, MaterialId, MaterialTable, Prefab};

/// The walker's body row's name in the table ([`with_walker`]).
pub const BODY_ROW: &str = "walker";
/// The eyes' row's name: it emits.
pub const EYE_ROW: &str = "walker eye";
/// The eyes' emission, radiance per channel: a warm glow.
pub const EYE_EMISSION: [f32; 3] = [3.0, 1.6, 0.5];
/// Model voxels per world voxel.
pub const DETAIL: u8 = 2;
/// The torso's size in model voxels: 8 wide (x), 6 deep (y), 12 tall (z).
pub const TORSO: [u32; 3] = [8, 6, 12];
/// The head's size: a 6-voxel cube; its eyes are on its front face (y 0).
pub const HEAD: [u32; 3] = [6, 6, 6];
/// A leg's size: 3 x 3, 8 tall.
pub const LEG: [u32; 3] = [3, 3, 8];
/// A walker's eye light: warm, intensity 150, casting shadows, not bouncing.
pub const EYE_LIGHT_COLOUR: [f32; 3] = [1.0, 0.7, 0.4];
pub const EYE_LIGHT_INTENSITY: f32 = 150.0;
/// Where the eye light sits in the head's space, in 1/256 model voxel: between the eyes'
/// centres in x and z, one model voxel in front of the face (a light inside the head would be
/// shadowed by it).
pub const EYE_LIGHT_AT: [i32; 3] = [3 * 256, -256, 896];
/// The ring `--entities` places walkers on: 3 m around the spawn.
pub const RING_RADIUS: f32 = 30.0;
/// How far down from the ring's points a walker looks for the floor, in voxels.
const FLOOR_SEARCH: i64 = 200;
/// The radius of the circle a walking walker walks from where it stands: 2 m.
pub const WALK_RADIUS: f32 = 20.0;
/// Its pace along the circle: 1 m/s, in voxels per second.
pub const WALK_SPEED: f32 = 10.0;
/// Its legs' swing either way: 30 degrees.
pub const WALK_SWING: f32 = std::f32::consts::FRAC_PI_6;
/// Its strides per second (one swing forward and back of each leg).
pub const WALK_STRIDES: f32 = 1.0;

/// `table` with the walker's two rows appended: its body and its emissive eyes.
pub fn with_walker(table: &MaterialTable) -> MaterialTable {
    let mut t = table.clone();
    t.push(Material {
        albedo: [0.55, 0.52, 0.48],
        ..Material::named(BODY_ROW)
    });
    t.push(Material {
        albedo: [0.9, 0.7, 0.4],
        emission: EYE_EMISSION,
        ..Material::named(EYE_ROW)
    });
    t
}

/// The ids of the walker's rows in `table`, when it has them.
pub fn rows(table: &MaterialTable) -> Option<(MaterialId, MaterialId)> {
    let find = |name: &str| {
        table
            .rows()
            .iter()
            .position(|m| m.name == name)
            .and_then(|i| MaterialId::try_from(i).ok())
    };
    Some((find(BODY_ROW)?, find(EYE_ROW)?))
}

fn solid(size: [u32; 3], m: MaterialId) -> Vec<MaterialId> {
    vec![m; (size[0] * size[1] * size[2]) as usize]
}

/// The walker's model: torso (part 0, hung from the root), head (1) on the torso, left and
/// right legs (2, 3) from the torso's hips, the legs sharing one prefab. Each part's pivot is
/// where it joins its parent: the torso's and head's bottom centres, the legs' top centres.
pub fn model(body: MaterialId, eye: MaterialId) -> Model {
    let mut head = solid(HEAD, body);
    for x in [1usize, 4] {
        // Voxel (x, 0, 3): the front face, a little above the middle.
        head[x + 6 * 6 * 3] = eye;
    }
    let leg = Arc::new(Prefab::new(LEG, solid(LEG, body), None));
    let parts = vec![
        Part {
            volume: Arc::new(Prefab::new(TORSO, solid(TORSO, body), None)),
            parent: None,
            pivot: [4 * 256, 3 * 256, 0],
            detail: DETAIL,
        },
        Part {
            volume: Arc::new(Prefab::new(HEAD, head, None)),
            parent: Some(0),
            pivot: [3 * 256, 3 * 256, 0],
            detail: DETAIL,
        },
        Part {
            volume: leg.clone(),
            parent: Some(0),
            pivot: [384, 384, 8 * 256],
            detail: DETAIL,
        },
        Part {
            volume: leg,
            parent: Some(0),
            pivot: [384, 384, 8 * 256],
            detail: DETAIL,
        },
    ];
    Model::new(parts).expect("the walker's parts are valid")
}

/// A turn of `angle` radians about axis `axis` (0 x, 1 y, 2 z) in Q14.
pub fn turn(axis: usize, angle: f32) -> Quat {
    let (s, c) = (0.5 * angle).sin_cos();
    let mut q = [(c * Q14_ONE as f32).round() as i16, 0, 0, 0];
    q[1 + axis] = (s * Q14_ONE as f32).round() as i16;
    Quat(q)
}

/// The walker standing with its feet's centre at `feet` (world voxels), turned `yaw` radians
/// about z from facing -y, its legs swung `swing` radians about x, opposite ways.
pub fn pose(feet: [f32; 3], yaw: f32, swing: f32) -> Pose {
    let fixed = |v: f32| (v * 256.0).round() as i64;
    // Each offset in 1/256 world voxel: a model voxel is half a world voxel.
    let leg_top = (LEG[2] as i32) * 128;
    Pose {
        root: Fixed3(feet.map(fixed)),
        root_q: turn(2, yaw),
        parts: vec![
            PartPose {
                t: [0, 0, leg_top],
                q: Quat::IDENTITY,
            },
            PartPose {
                t: [0, 0, TORSO[2] as i32 * 128],
                q: Quat::IDENTITY,
            },
            PartPose {
                t: [-256, 0, 0],
                q: turn(0, swing),
            },
            PartPose {
                t: [256, 0, 0],
                q: turn(0, -swing),
            },
        ],
    }
}

/// The yaw that turns a walker at `at` to face `eye` (its front is -y at yaw 0).
pub fn facing(at: [f32; 3], eye: [f32; 3]) -> f32 {
    let (dx, dy) = (eye[0] - at[0], eye[1] - at[1]);
    // -y turned by yaw about z is (sin yaw, -cos yaw).
    dx.atan2(-dy)
}

/// One placed walker: its entity, where it stands and which way it faces when placed, the
/// walk's clock then, and whether it is the viewer's model (`--model`, `crate::viewer`), which
/// plays its clip where it stands, in place of the built-in walker.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub id: EntityId,
    pub feet: [f32; 3],
    pub yaw: f32,
    pub since: f32,
    pub rig: bool,
}

/// The testbed's walkers: the set the frame draws, the walkers in the order placed, the
/// model and its handle once uploaded, and the counts the frame drew last. Two are equal when
/// they place the same walkers alike (the set follows from the placements).
#[derive(Clone, Debug, Default)]
pub struct Walkers {
    pub set: EntitySet,
    pub placed: Vec<Placed>,
    model: Option<Arc<Model>>,
    pub handle: Option<ModelHandle>,
    /// `--entities N`'s ring, placed on the first frame that has the world.
    pub ring: u32,
    /// What `4` removes, newest last: `true` a walker, `false` a light.
    pub newest: Vec<bool>,
    pub drawn: usize,
    pub parts: usize,
    pub over: usize,
    /// Whether placed walkers carry their eye lights (`F12` flips it; on by default).
    pub eye_lights_off: bool,
    /// Whether the walkers walk (`F11` flips it; off by default).
    pub walking: bool,
    /// Seconds walked: the walk's clock, held while the walk is off.
    pub clock: f32,
    /// `--model`'s viewer: placements take its model in place of the built-in walker.
    pub viewer: Option<crate::viewer::Viewer>,
}

impl PartialEq for Walkers {
    fn eq(&self, o: &Self) -> bool {
        (self.placed == o.placed)
            && (self.handle, self.ring, &self.newest) == (o.handle, o.ring, &o.newest)
            && (self.drawn, self.parts, self.over) == (o.drawn, o.parts, o.over)
            && self.eye_lights_off == o.eye_lights_off
            && (self.walking, self.clock) == (o.walking, o.clock)
    }
}

impl Walkers {
    /// No walker yet; `ring` placed on the ring once the world is there.
    pub fn with_ring(ring: u32) -> Self {
        Self {
            ring,
            ..Self::default()
        }
    }

    /// No walker yet; `ring` placed on the ring once the world is there, walking from the
    /// first frame when `walking` (`--entities-walk`).
    pub fn with_ring_walking(ring: u32, walking: bool) -> Self {
        Self {
            walking,
            ..Self::with_ring(ring)
        }
    }

    /// The model, built on first use from `table`'s walker rows; `None` without them.
    pub fn model(&mut self, table: &MaterialTable) -> Option<Arc<Model>> {
        if self.model.is_none() {
            let (body, eye) = rows(table)?;
            self.model = Some(Arc::new(model(body, eye)));
        }
        self.model.clone()
    }

    /// These walkers with `--model`'s viewer.
    pub fn with_viewer(self, viewer: Option<crate::viewer::Viewer>) -> Self {
        Self { viewer, ..self }
    }

    /// Places a walker with its feet at `feet`, facing `eye`: the viewer's model at its clip's
    /// pose now when there is a viewer, else the built-in walker.
    pub fn place(&mut self, table: &MaterialTable, feet: [f32; 3], eye: [f32; 3]) -> bool {
        let yaw = facing(feet, eye);
        let (model, pose, rig) = match &mut self.viewer {
            Some(v) => {
                if let Some(e) = v.build_pending(table) {
                    println!("{e}");
                }
                let (Some(l), Some((now, _, _))) = (&v.loaded, v.poses()) else {
                    return false;
                };
                (
                    l.model.clone(),
                    crate::viewer::placed(&now, feet, yaw),
                    true,
                )
            }
            None => {
                let Some(model) = self.model(table) else {
                    return false;
                };
                (model, pose(feet, yaw, 0.0), false)
            }
        };
        let Ok(id) = self.set.insert(model, pose) else {
            return false;
        };
        self.placed.push(Placed {
            id,
            feet,
            yaw,
            since: self.clock,
            rig,
        });
        true
    }

    /// The viewer's reload once a second (`now` in seconds; [`crate::viewer::Viewer::poll`])
    /// and its first build: on a new model every placed instance of the old is placed again
    /// with it. The banner's words when anything was tried.
    pub fn poll_viewer(&mut self, now: f64, table: &MaterialTable) -> Option<String> {
        let v = self.viewer.as_mut()?;
        let built = v.build_pending(table);
        let had = v.loaded.as_ref().map(|l| l.model.clone());
        let words = built.or_else(|| v.poll(now, table));
        let (Some(l), Some((now_pose, _, _))) = (&v.loaded, v.poses()) else {
            return words;
        };
        if had.is_some_and(|m| Arc::ptr_eq(&m, &l.model)) {
            return words;
        }
        for p in self.placed.iter_mut().filter(|p| p.rig) {
            self.set.remove(p.id);
            let pose = crate::viewer::placed(&now_pose, p.feet, p.yaw);
            if let Ok(id) = self.set.insert(l.model.clone(), pose) {
                p.id = id;
            }
        }
        words
    }

    /// Removes the newest walker.
    pub fn remove_newest(&mut self) {
        if let Some(p) = self.placed.pop() {
            self.set.remove(p.id);
        }
    }

    /// The frame's list: every placed walker under its model's uploaded handle (the viewer's
    /// for its instances), those whose model is not uploaded left out.
    pub fn list(&self) -> Vec<(EntityId, ModelHandle)> {
        let rig = self.viewer.as_ref().and_then(|v| v.handle);
        self.placed
            .iter()
            .filter_map(|p| Some((p.id, if p.rig { rig? } else { self.handle? })))
            .collect()
    }

    /// Each listed entity's drawn parts: a built-in walker as the set poses it, a viewer
    /// instance between its clip's two ticks around now ([`vxe_render::DisplayPose::between`]).
    pub fn displays(&self) -> Vec<(EntityId, ModelHandle, vxe_render::DisplayPose)> {
        let rig = self
            .viewer
            .as_ref()
            .and_then(|v| Some((v.loaded.as_ref()?, v.poses()?)));
        let rig_handle = self.viewer.as_ref().and_then(|v| v.handle);
        self.placed
            .iter()
            .filter_map(|p| {
                let (model, pose) = self.set.get(p.id)?;
                if p.rig {
                    let (_, (a, b, alpha)) = rig.as_ref()?;
                    let a = crate::viewer::placed(a, p.feet, p.yaw);
                    let b = crate::viewer::placed(b, p.feet, p.yaw);
                    let d = vxe_render::DisplayPose::between(model, &a, &b, *alpha)?;
                    Some((p.id, rig_handle?, d))
                } else {
                    Some((
                        p.id,
                        self.handle?,
                        vxe_render::DisplayPose::of(model, pose)?,
                    ))
                }
            })
            .collect()
    }

    /// Each placed walker's eye light, in placement order, while they are on: the frame takes
    /// those that fit the list's free slots.
    pub fn eye_lights(&self) -> Vec<vxe_render::Light> {
        if self.eye_lights_off {
            return Vec::new();
        }
        self.placed
            .iter()
            .filter(|p| !p.rig)
            .filter_map(|p| {
                let (model, pose) = self.set.get(p.id)?;
                let at = pose.part_point(model, 1, EYE_LIGHT_AT)?;
                let mut l =
                    vxe_render::Light::point(at.0.map(|v| v as f32 / 256.0), EYE_LIGHT_INTENSITY);
                l.colour = EYE_LIGHT_COLOUR;
                l.flags = vxe_render::Light::ON | vxe_render::Light::SHADOWS;
                Some(l)
            })
            .collect()
    }

    /// `F12`: flips the eye lights; the banner's words.
    pub fn toggle_eye_lights(&mut self) -> &'static str {
        self.eye_lights_off = !self.eye_lights_off;
        if self.eye_lights_off {
            "eye lights off"
        } else {
            "eye lights on"
        }
    }

    /// `F11`: the walk on or off; the banner's words.
    pub fn toggle_walk(&mut self) -> &'static str {
        self.walking = !self.walking;
        if self.walking { "walk on" } else { "walk off" }
    }

    /// Where walker `p` stands at `clock` seconds of walk, its yaw and its legs' swing: on a
    /// circle of [`WALK_RADIUS`] through where it was placed, walking anticlockwise (seen from
    /// above) at [`WALK_SPEED`] and facing along the circle, its legs swinging [`WALK_SWING`]
    /// either way at [`WALK_STRIDES`]. The circle's centre lies [`WALK_RADIUS`] to the
    /// walker's left as placed, so at the clock it was placed (`since`) it stands at its feet,
    /// faces its yaw and has its legs together: the walk starts where it stands. Walkers placed
    /// facing different ways start at different angles on their circles, so a ring does not
    /// walk in step.
    pub fn walk_pose(p: &Placed, clock: f32) -> ([f32; 3], f32, f32) {
        let t = clock - p.since;
        // The front at yaw y is (sin y, -cos y); anticlockwise about a centre at angle a from
        // it the motion is (-sin a, cos a), the front when a = y + pi.
        let a0 = p.yaw + std::f32::consts::PI;
        let centre = [
            p.feet[0] - WALK_RADIUS * a0.cos(),
            p.feet[1] - WALK_RADIUS * a0.sin(),
        ];
        let turned = t * WALK_SPEED / WALK_RADIUS;
        let a = a0 + turned;
        let feet = [
            centre[0] + WALK_RADIUS * a.cos(),
            centre[1] + WALK_RADIUS * a.sin(),
            p.feet[2],
        ];
        let yaw = p.yaw + turned;
        let swing = WALK_SWING * (std::f32::consts::TAU * WALK_STRIDES * t).sin();
        (feet, yaw, swing)
    }

    /// Advances the viewer's clip by `dt` seconds, posing its instances at the clip's tick
    /// now, and the walk while it is on, posing every placed built-in walker
    /// ([`Walkers::walk_pose`]); while it is off the walkers hold their poses.
    pub fn step(&mut self, dt: f32) {
        if let Some(v) = &mut self.viewer {
            v.advance(dt);
            if let Some((now, _, _)) = v.poses() {
                for p in self.placed.iter().filter(|p| p.rig) {
                    let _ = self
                        .set
                        .set_pose(p.id, crate::viewer::placed(&now, p.feet, p.yaw));
                }
            }
        }
        if !self.walking {
            return;
        }
        self.clock += dt;
        for p in self.placed.iter().filter(|p| !p.rig) {
            let (feet, yaw, swing) = Self::walk_pose(p, self.clock);
            // A walker's pose has its four parts, so it always fits its model.
            let _ = self.set.set_pose(p.id, pose(feet, yaw, swing));
        }
    }

    /// The HUD row, `entities <n>/64 parts <m>/512 over <k>`, then ` walk` while they walk.
    pub fn write_row(&self, out: &mut String) {
        let _ = write!(
            out,
            "entities {}/{MAX_ENTITIES} parts {}/{MAX_ENTITY_PARTS} over {}",
            self.drawn, self.parts, self.over
        );
        if self.walking {
            out.push_str(" walk");
        }
    }

    /// The banner's count, `entities <n>/64`.
    pub fn count_words(&self) -> String {
        format!(
            "entities {}/{MAX_ENTITIES}",
            self.placed.len().min(MAX_ENTITIES)
        )
    }

    /// The ring's points: `n` on a horizontal circle of [`RING_RADIUS`] around `centre`.
    pub fn ring_points(n: u32, centre: [f32; 3]) -> Vec<[f32; 3]> {
        (0..n)
            .map(|i| {
                let a = std::f32::consts::TAU * i as f32 / n as f32;
                [
                    centre[0] + RING_RADIUS * a.cos(),
                    centre[1] + RING_RADIUS * a.sin(),
                    centre[2],
                ]
            })
            .collect()
    }

    /// Places the pending ring around `centre`, each walker on the first floor below its point
    /// (`floor_below` gives the drop in voxels), facing the centre.
    pub fn place_ring(
        &mut self,
        table: &MaterialTable,
        centre: [f32; 3],
        floor_below: impl Fn([f32; 3], i64) -> Option<f32>,
    ) {
        let n = std::mem::take(&mut self.ring);
        for p in Self::ring_points(n, centre) {
            let drop = floor_below(p, FLOOR_SEARCH).unwrap_or(0.0);
            let feet = [p[0], p[1], p[2] - drop];
            self.place(table, feet, centre);
        }
    }
}

/// One frame's walkers into `frame`: the pending ring placed around `eye` on `world`'s floor,
/// the model uploaded on first need, and the frame's entities set while any walker is placed
/// or was drawn the frame before; the drawn counts copied back for the HUD.
pub fn feed(
    walkers: &mut Walkers,
    world: &vxe_world::World,
    eye: [f32; 3],
    frame: &mut vxe_render::Frame,
    queue: &vxe_render::wgpu::Queue,
) {
    if walkers.ring > 0 {
        let floor_below = |p: [f32; 3], search: i64| {
            let o = Fixed3(p.map(|v| (v * 256.0).round() as i64));
            let hit = world
                .raycast(o, [0, 0, -256], search * 256)
                .ok()
                .flatten()?;
            Some(hit.t as f32 / 256.0)
        };
        walkers.place_ring(world.materials(), eye, floor_below);
    }
    if walkers.placed.is_empty() && walkers.drawn == 0 && walkers.over == 0 {
        return;
    }
    if walkers.handle.is_none()
        && walkers.placed.iter().any(|p| !p.rig)
        && let Some(model) = walkers.model(world.materials())
    {
        match frame
            .entity_mirror_mut()
            .upload_model(queue, &model, world.materials())
        {
            Ok(h) => walkers.handle = Some(h),
            Err(e) => println!("walker: {e}"),
        }
    }
    if let Some(v) = &mut walkers.viewer
        && v.handle.is_none()
        && let Some(l) = &v.loaded
    {
        match frame
            .entity_mirror_mut()
            .upload_model(queue, &l.model, world.materials())
        {
            Ok(h) => v.handle = Some(h),
            Err(e) => {
                // Retried next frame only after a reload: mark it tried with no model.
                println!("model {}: {e}", v.name);
                v.loaded = None;
            }
        }
    }
    let displays = walkers.displays();
    let list: Vec<_> = displays.iter().map(|(id, h, d)| (*id, *h, d)).collect();
    frame.set_entity_displays(queue, &list);
    let m = frame.entity_mirror();
    (walkers.drawn, walkers.parts, walkers.over) =
        (m.entities_drawn(), m.parts_drawn(), m.entities_over_cap());
}

/// Voxels of the walker's model that are not air, per part, for the readings.
pub fn solid_voxels(model: &Model) -> Vec<usize> {
    model
        .parts()
        .iter()
        .map(|p| {
            let s = p.volume.size();
            let mut n = 0;
            for z in 0..s[2] as i32 {
                for y in 0..s[1] as i32 {
                    for x in 0..s[0] as i32 {
                        n += usize::from(p.volume.get([x, y, z]).is_some_and(|m| m != AIR));
                    }
                }
            }
            n
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_walker_has_four_parts_and_two_glowing_eyes() {
        let table = with_walker(&MaterialTable::new());
        let (body, eye) = rows(&table).unwrap();
        let m = model(body, eye);
        assert_eq!(m.parts().len(), 4);
        let head = &m.parts()[1].volume;
        assert_eq!(head.get([1, 0, 3]), Some(eye));
        assert_eq!(head.get([4, 0, 3]), Some(eye));
        assert_eq!(head.get([2, 0, 3]), Some(body));
        assert_eq!(table.get(eye).unwrap().emission, EYE_EMISSION);
        assert_eq!(m.parts()[2].volume.id(), m.parts()[3].volume.id());
    }

    #[test]
    fn a_walker_faces_the_eye() {
        // At yaw 0 the front is -y: an eye at -y gives yaw 0, one at +x a quarter turn.
        assert!(facing([0.0; 3], [0.0, -5.0, 0.0]).abs() < 1e-6);
        let q = facing([0.0; 3], [5.0, 0.0, 0.0]);
        assert!((q - std::f32::consts::FRAC_PI_2).abs() < 1e-6, "{q}");
    }

    #[test]
    fn the_hud_row_and_the_banner_count_the_walkers() {
        let table = with_walker(&MaterialTable::new());
        let mut w = Walkers::default();
        assert!(w.place(&table, [0.0; 3], [0.0, -9.0, 0.0]));
        assert!(w.place(&table, [9.0, 0.0, 0.0], [0.0, -9.0, 0.0]));
        (w.drawn, w.parts, w.over) = (2, 8, 0);
        let mut row = String::new();
        w.write_row(&mut row);
        assert_eq!(row, "entities 2/64 parts 8/512 over 0");
        assert_eq!(w.count_words(), "entities 2/64");
        w.remove_newest();
        assert_eq!(w.placed.len(), 1);
        assert_eq!(w.set.len(), 1);
    }

    /// `` ` `` shows the count in the banner (`run::keys` passes it to `show_lights`), and the HUD's
    /// mode block gains the walkers' row once one is placed.
    #[test]
    fn the_banner_names_the_walkers_after_the_key() {
        let table = with_walker(&MaterialTable::new());
        let mut m = crate::hud::Modes::new(
            vxe_render::Lighting::default(),
            vxe_render::LightingResolution::Half,
            0.0,
        );
        let mut rows = String::new();
        m.write_rows(&mut rows);
        assert!(!rows.contains("entities"), "no walker, no row");
        assert_eq!(crate::toggles::KEY_WALKER, vxe_app::KeyCode::Backquote);
        for x in [0.0, 9.0] {
            assert!(m.walkers.place(&table, [x, 0.0, 0.0], [0.0, -9.0, 0.0]));
        }
        m.show_lights(m.walkers.count_words(), 10.0);
        assert!(
            m.banner(10.0)
                .unwrap()
                .lines()
                .any(|l| l == "entities 2/64")
        );
        (m.walkers.drawn, m.walkers.parts) = (2, 8);
        let mut rows = String::new();
        m.write_rows(&mut rows);
        assert!(
            rows.lines()
                .any(|l| l == "entities 2/64 parts 8/512 over 0"),
            "{rows}"
        );
    }

    /// `F11` sets the walkers walking and names the mode in the banner; the HUD row gains
    /// `walk`. A walker walks a 2 m circle from where it stands, facing along it, its legs
    /// swinging 30 degrees either way; off, it holds its pose.
    #[test]
    fn the_walk_key_names_its_mode_and_walks_the_circle() {
        let table = with_walker(&MaterialTable::new());
        let mut m = crate::hud::Modes::new(
            vxe_render::Lighting::default(),
            vxe_render::LightingResolution::Half,
            0.0,
        );
        assert_eq!(crate::toggles::KEY_WALK, vxe_app::KeyCode::F11);
        assert!(m.walkers.place(&table, [50.0, 40.0, 2.0], [50.0, 0.0, 8.0]));
        let words = m.walkers.toggle_walk();
        assert_eq!(words, "walk on");
        m.show_lights(words.to_owned(), 5.0);
        assert!(m.banner(5.0).unwrap().lines().any(|l| l == "walk on"));
        (m.walkers.drawn, m.walkers.parts) = (1, 4);
        let mut rows = String::new();
        m.write_rows(&mut rows);
        assert!(
            rows.lines()
                .any(|l| l == "entities 1/64 parts 4/512 over 0 walk"),
            "{rows}"
        );
        // Facing -y (yaw 0) the walker's left is +x: the circle's centre is 2 m that way.
        let centre = [50.0 + WALK_RADIUS, 40.0];
        let mut swings = Vec::new();
        for _ in 0..240 {
            m.walkers.step(1.0 / 60.0);
            let p = m.walkers.placed[0];
            let (feet, yaw, swing) = Walkers::walk_pose(&p, m.walkers.clock);
            let r = ((feet[0] - centre[0]).powi(2) + (feet[1] - centre[1]).powi(2)).sqrt();
            assert!((r - WALK_RADIUS).abs() < 1e-3, "radius {r}");
            assert_eq!(feet[2], 2.0);
            // Facing along the circle: the front is perpendicular to the radius.
            let front = [yaw.sin(), -yaw.cos()];
            let along = (front[0] * (feet[0] - centre[0]) + front[1] * (feet[1] - centre[1])) / r;
            assert!(along.abs() < 1e-3, "facing {along}");
            swings.push(swing);
            let (_, pose_now) = m.walkers.set.get(p.id).unwrap();
            assert_eq!(*pose_now, pose(feet, yaw, swing));
        }
        let most = swings.iter().fold(0f32, |a, s| a.max(s.abs()));
        assert!((most - WALK_SWING).abs() < 0.01, "swing {most}");
        assert!((m.walkers.clock - 4.0).abs() < 1e-4);
        assert_eq!(m.walkers.toggle_walk(), "walk off");
        let held = m.walkers.set.get(m.walkers.placed[0].id).unwrap().1.clone();
        m.walkers.step(1.0 / 60.0);
        assert_eq!(m.walkers.set.get(m.walkers.placed[0].id).unwrap().1, &held);
        let mut rows = String::new();
        m.write_rows(&mut rows);
        assert!(
            rows.lines()
                .any(|l| l == "entities 1/64 parts 4/512 over 0")
        );
    }

    /// The distance between two poses' roots, in world voxels, and the angle between their
    /// root turns, in radians.
    fn apart(a: &Pose, b: &Pose) -> (f32, f32) {
        let d = (0..3)
            .map(|i| ((a.root.0[i] - b.root.0[i]) as f32 / 256.0).powi(2))
            .sum::<f32>()
            .sqrt();
        let q = |p: &Pose| p.root_q.0.map(|v| f32::from(v) / Q14_ONE as f32);
        let (qa, qb) = (q(a), q(b));
        let dot = (0..4).map(|i| qa[i] * qb[i]).sum::<f32>().abs().min(1.0);
        (d, 2.0 * dot.acos())
    }

    /// The walk starts where a walker stands and stops where it is: its first step moves it
    /// one step's length (1/60 s at 1 m/s, a sixth of a voxel), not across the circle; a
    /// stop holds it there, and walking again goes on from there; a walker placed mid-walk
    /// starts from where it was placed.
    #[test]
    fn the_walk_starts_where_the_walker_stands_and_stops_where_it_is() {
        let table = with_walker(&MaterialTable::new());
        let mut w = Walkers::default();
        let step = WALK_SPEED / 60.0;
        // A step turns a walker 1/120 rad; the angle read back from two Q14 turns is good to
        // a few hundredths (acos near 1), against the half turn a misplaced start makes.
        let near = |d: f32, turn: f32| d <= step * 1.01 + 1.0 / 256.0 && turn < 0.05;
        // Four ways of facing, so the centre's side is checked for each.
        for (i, eye) in [
            [50.0, 0.0, 8.0],
            [90.0, 40.0, 8.0],
            [50.0, 80.0, 8.0],
            [0.0, 40.0, 8.0],
        ]
        .into_iter()
        .enumerate()
        {
            assert!(w.place(&table, [50.0 + 100.0 * i as f32, 40.0, 2.0], eye));
        }
        let placed: Vec<Pose> = w
            .placed
            .iter()
            .map(|p| w.set.get(p.id).unwrap().1.clone())
            .collect();
        w.toggle_walk();
        let mut last = placed.clone();
        for frame in 0..120 {
            w.step(1.0 / 60.0);
            for (k, p) in w.placed.iter().enumerate() {
                let now = w.set.get(p.id).unwrap().1;
                let (d, turn) = apart(now, &last[k]);
                assert!(
                    near(d, turn),
                    "walker {k} frame {frame}: moved {d} turned {turn}"
                );
                last[k] = now.clone();
            }
        }
        w.toggle_walk();
        let stopped: Vec<Pose> = w
            .placed
            .iter()
            .map(|p| w.set.get(p.id).unwrap().1.clone())
            .collect();
        for _ in 0..30 {
            w.step(1.0 / 60.0);
        }
        for (k, p) in w.placed.iter().enumerate() {
            assert_eq!(w.set.get(p.id).unwrap().1, &stopped[k], "walker {k} held");
        }
        w.toggle_walk();
        w.step(1.0 / 60.0);
        for (k, p) in w.placed.iter().enumerate() {
            let (d, turn) = apart(w.set.get(p.id).unwrap().1, &stopped[k]);
            assert!(
                near(d, turn),
                "walker {k} on again: moved {d} turned {turn}"
            );
        }
        // A walker placed while the clock reads 2 s starts from where it stands.
        assert!(w.place(&table, [10.0, 300.0, 2.0], [10.0, 0.0, 8.0]));
        let new = w.placed.last().copied().unwrap();
        let at = w.set.get(new.id).unwrap().1.clone();
        w.step(1.0 / 60.0);
        let (d, turn) = apart(w.set.get(new.id).unwrap().1, &at);
        assert!(near(d, turn), "placed mid-walk: moved {d} turned {turn}");
        assert!(d > 0.5 * step, "placed mid-walk: it walks ({d})");
    }

    /// `F12` flips the eye lights and names the mode in the banner; on, each walker carries one
    /// warm shadowing light in front of its eyes, none bouncing.
    #[test]
    fn the_eye_light_key_names_its_mode() {
        let table = with_walker(&MaterialTable::new());
        let mut m = crate::hud::Modes::new(
            vxe_render::Lighting::default(),
            vxe_render::LightingResolution::Half,
            0.0,
        );
        assert_eq!(crate::toggles::KEY_EYE_LIGHTS, vxe_app::KeyCode::F12);
        // Facing -y: the eye at -y of the feet.
        assert!(m.walkers.place(&table, [10.0, 20.0, 2.0], [10.0, 0.0, 8.0]));
        let lights = m.walkers.eye_lights();
        assert_eq!(lights.len(), 1);
        let l = lights[0];
        assert_eq!(
            (l.colour, l.intensity),
            (EYE_LIGHT_COLOUR, EYE_LIGHT_INTENSITY)
        );
        assert_eq!(l.flags, vxe_render::Light::ON | vxe_render::Light::SHADOWS);
        // Legs 4, torso 6 and 1.75 up the head in world voxels over the feet; the face is 1.5
        // voxels in front of the feet's centre, the light half a voxel past it.
        assert_eq!(l.pos, [10.0, 18.0, 13.75]);
        let words = m.walkers.toggle_eye_lights();
        m.show_lights(words.to_owned(), 5.0);
        assert!(
            m.banner(5.0)
                .unwrap()
                .lines()
                .any(|l| l == "eye lights off")
        );
        assert!(m.walkers.eye_lights().is_empty());
        assert_eq!(m.walkers.toggle_eye_lights(), "eye lights on");
    }
}
