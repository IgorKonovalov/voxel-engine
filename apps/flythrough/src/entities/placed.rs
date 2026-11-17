//! The placed entities: the set the frame draws, each placement's model and playback, the
//! built-in walkers' walk and eye lights, and the HUD row and banner words they give.

use std::fmt::Write as _;
use std::sync::Arc;

use vxe_entity::{EntityId, EntitySet, Model, Pose};

use crate::viewer::{Loaded, Play, Viewer, placed};
use vxe_render::{MAX_ENTITIES, MAX_ENTITY_PARTS, ModelHandle};
use vxe_world::MaterialTable;

use super::library::{Library, Source};
use super::walker::{
    EYE_LIGHT_AT, EYE_LIGHT_COLOUR, EYE_LIGHT_INTENSITY, FLOOR_SEARCH, RING_RADIUS, WALK_RADIUS,
    WALK_SPEED, WALK_STRIDES, WALK_SWING, facing, model, pose, rows,
};

/// One placed entity: its id, where it stands and which way it faces when placed, the walk's
/// clock then (a built-in walker's), its library entry (`None` for the built-in walker) and
/// its own playback of that entry's clips.
#[derive(Clone, Debug)]
pub struct Placed {
    pub id: EntityId,
    pub feet: [f32; 3],
    pub yaw: f32,
    pub since: f32,
    pub entry: Option<usize>,
    pub play: Play,
}

impl PartialEq for Placed {
    fn eq(&self, o: &Self) -> bool {
        (self.id, self.feet, self.yaw, self.since, self.entry)
            == (o.id, o.feet, o.yaw, o.since, o.entry)
            && (self.play.clip, self.play.paused, self.play.ticks)
                == (o.play.clip, o.play.paused, o.play.ticks)
    }
}

/// What a placement takes: the built-in walker, a library entry's rig, or why the selected
/// entry cannot be placed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pick {
    Builtin,
    Rig(usize),
    Refused(String),
}

/// The testbed's entities: the set the frame draws, the placements in order, the library
/// they are placed from, the built-in walker's model and its handle once uploaded, the entity
/// under the crosshair, and the counts the frame drew last. Two are equal when they place the
/// same entities alike (the set follows from the placements).
#[derive(Clone, Debug, Default)]
pub struct Walkers {
    pub set: EntitySet,
    pub placed: Vec<Placed>,
    model: Option<Arc<Model>>,
    pub handle: Option<ModelHandle>,
    /// `--entities N`'s ring, placed on the first frame that has the world.
    pub ring: u32,
    /// What `4` removes, newest last: `true` an entity, `false` a light.
    pub newest: Vec<bool>,
    pub drawn: usize,
    pub parts: usize,
    pub over: usize,
    /// The entity pool's bytes in use and its size, as the last [`super::sync_models`] read
    /// them; 0 and 0 before.
    pub pool_used: u64,
    pub pool_total: u64,
    /// Whether placed built-in walkers carry their eye lights (on by default).
    pub eye_lights_off: bool,
    /// Whether the built-in walkers walk (off by default).
    pub walking: bool,
    /// Seconds walked: the walk's clock, held while the walk is off.
    pub clock: f32,
    /// The entities library: an empty one places the built-in walker.
    pub library: Library,
    /// The placed entity under the crosshair, as the frame last read it: the clip keys act on
    /// it, else on the newest.
    pub aimed: Option<EntityId>,
    /// The entities page's preview: the selected entry standing at the crosshair, in the set
    /// and drawn as any placement but not one of [`Walkers::placed`] (`super::picker`).
    pub preview: Option<Placed>,
    /// The frame's drawn parts, kept across frames ([`Walkers::refresh_displays`]): the first
    /// `shown` are this frame's, the rest storage kept for later frames.
    displays: Vec<(EntityId, ModelHandle, vxe_render::DisplayPose)>,
    shown: usize,
    /// A rig instance's two poses placed, reused instance to instance.
    rig_scratch: Option<(Pose, Pose)>,
}

impl PartialEq for Walkers {
    fn eq(&self, o: &Self) -> bool {
        (self.placed == o.placed)
            && (self.handle, self.ring, &self.newest) == (o.handle, o.ring, &o.newest)
            && (self.drawn, self.parts, self.over) == (o.drawn, o.parts, o.over)
            && (self.pool_used, self.pool_total) == (o.pool_used, o.pool_total)
            && self.eye_lights_off == o.eye_lights_off
            && (self.walking, self.clock) == (o.walking, o.clock)
            && (self.library.selected, self.aimed) == (o.library.selected, o.aimed)
    }
}

impl Walkers {
    /// No entity yet; `ring` placed on the ring once the world is there.
    pub fn with_ring(ring: u32) -> Self {
        Self {
            ring,
            ..Self::default()
        }
    }

    /// No entity yet; `ring` placed on the ring once the world is there, the built-in
    /// walkers walking from the first frame when `walking` (`--entities-walk`).
    pub fn with_ring_walking(ring: u32, walking: bool) -> Self {
        Self {
            walking,
            ..Self::with_ring(ring)
        }
    }

    /// The built-in walker's model, built on first use from `table`'s walker rows; `None`
    /// without them.
    pub fn model(&mut self, table: &MaterialTable) -> Option<Arc<Model>> {
        if self.model.is_none() {
            let (body, eye) = rows(table)?;
            self.model = Some(Arc::new(model(body, eye)));
        }
        self.model.clone()
    }

    /// These entities placed from `library`.
    pub fn with_library(self, library: Library) -> Self {
        Self { library, ..self }
    }

    /// These entities with `--model`'s viewer as the library's one entry, selected.
    pub fn with_viewer(self, viewer: Option<Viewer>) -> Self {
        match viewer {
            Some(v) => self.with_library(Library::default().with_model(v)),
            None => self,
        }
    }

    /// `--model`'s viewer, when there is one.
    pub fn viewer(&self) -> Option<&Viewer> {
        self.library.flagged()
    }

    pub fn viewer_mut(&mut self) -> Option<&mut Viewer> {
        self.library.flagged_mut()
    }

    /// What a placement of the selected entry takes: the built-in walker with no library.
    pub fn pick(&self) -> Pick {
        let i = self.library.selected;
        let Some(e) = self.library.entries.get(i) else {
            return Pick::Builtin;
        };
        if let Some(err) = e.error() {
            return Pick::Refused(format!("{}: {err}", e.name));
        }
        match e.source {
            Ok(Source::Builtin) => Pick::Builtin,
            _ => Pick::Rig(i),
        }
    }

    /// Places the selected entry with its feet at `feet`, facing `eye`; whether it placed.
    pub fn place(&mut self, table: &MaterialTable, feet: [f32; 3], eye: [f32; 3]) -> bool {
        self.place_selected(table, feet, eye).is_ok()
    }

    /// [`Walkers::place`] giving the new entity's id, or why nothing was placed. The library's
    /// rigs not built yet are built against `table` first.
    pub fn place_selected(
        &mut self,
        table: &MaterialTable,
        feet: [f32; 3],
        eye: [f32; 3],
    ) -> Result<EntityId, String> {
        for e in self.library.build(table) {
            println!("{e}");
        }
        let entry = match self.pick() {
            Pick::Builtin => None,
            Pick::Rig(i) => Some(i),
            Pick::Refused(why) => return Err(why),
        };
        self.place_entry(table, entry, feet, facing(feet, eye))
    }

    /// Places `entry` (`None` the built-in walker) standing at `feet`, turned `yaw`: a rig at
    /// its entry's selected clip's tick 0, on a playback of its own.
    pub fn place_entry(
        &mut self,
        table: &MaterialTable,
        entry: Option<usize>,
        feet: [f32; 3],
        yaw: f32,
    ) -> Result<EntityId, String> {
        let (model, pose, play) = match entry {
            None => {
                let model = self
                    .model(table)
                    .ok_or("walker: no walker rows in this world's table")?;
                (model, pose(feet, yaw, 0.0), Play::default())
            }
            Some(i) => {
                let e = self.library.entries.get(i).ok_or("no such entry")?;
                let v = e.viewer().ok_or_else(|| format!("{}: not a rig", e.name))?;
                let Some(l) = &v.loaded else {
                    return Err(format!("model {}: not built", e.name));
                };
                let play = Play::start(l, v.play.clip);
                let now = play.poses(l).0;
                (l.model.clone(), placed(&now, feet, yaw), play)
            }
        };
        let id = self.set.insert(model, pose).map_err(|e| e.to_string())?;
        self.placed.push(Placed {
            id,
            feet,
            yaw,
            since: self.clock,
            entry,
            play,
        });
        Ok(id)
    }

    /// Each rig's first build and its reload once a second (`now` in seconds;
    /// [`crate::viewer::Viewer::poll`]): on a new model every placed instance of that entry,
    /// and of no other, takes it under its own id ([`vxe_entity::EntitySet::replace`]), its
    /// clip kept by name. The banner's words when anything was tried: `--model`'s as its
    /// viewer gives them, another entry's after its name.
    pub fn poll_viewer(&mut self, now: f64, table: &MaterialTable) -> Option<String> {
        let mut words = None;
        let mut changed = Vec::new();
        for (i, e) in self.library.entries.iter_mut().enumerate() {
            let (name, flagged) = (e.name.clone(), e.flagged);
            let Some(v) = e.viewer_mut() else { continue };
            let had = v.loaded.clone();
            let w = v.build_pending(table).or_else(|| v.poll(now, table));
            if let Some(w) = w {
                let w = if flagged { w } else { format!("{name}: {w}") };
                if words.is_some() {
                    println!("{w}");
                } else {
                    words = Some(w);
                }
            }
            if let (Some(old), Some(l)) = (had, &v.loaded)
                && !Arc::ptr_eq(&old.model, &l.model)
            {
                changed.push((i, old));
            }
        }
        // A failed first build moves the selection off the entry.
        self.library.build(table);
        for (i, old) in changed {
            let Some(l) = self.library.entries[i]
                .viewer()
                .and_then(|v| v.loaded.as_ref())
            else {
                continue;
            };
            for p in (self.placed.iter_mut().chain(self.preview.iter_mut()))
                .filter(|p| p.entry == Some(i))
            {
                let name = p.play.clip_name(&old).map(str::to_owned);
                p.play.rebind(name.as_deref(), l);
                let pose = placed(&p.play.poses(l).0, p.feet, p.yaw);
                // The pose is the new model's, so it fits; the entity keeps its id, and with
                // it the mirror's slots and the pixels' history.
                let _ = self.set.replace(p.id, l.model.clone(), pose);
            }
        }
        words
    }

    /// Removes the newest placed entity.
    pub fn remove_newest(&mut self) {
        if let Some(p) = self.placed.pop() {
            self.set.remove(p.id);
        }
    }

    /// Removes placement `i`; its id, when there was one.
    pub fn remove_at(&mut self, i: usize) -> Option<EntityId> {
        if i >= self.placed.len() {
            return None;
        }
        let p = self.placed.remove(i);
        self.set.remove(p.id);
        Some(p.id)
    }

    /// The placements and then the preview, when there is one.
    pub fn all(&self) -> impl Iterator<Item = &Placed> {
        self.placed.iter().chain(self.preview.iter())
    }

    /// The uploaded handle placement `p` is drawn under: its entry's, or the built-in
    /// walker's.
    fn handle_of(&self, p: &Placed) -> Option<ModelHandle> {
        match p.entry {
            None => self.handle,
            Some(i) => self.library.entries.get(i)?.viewer()?.handle,
        }
    }

    /// The frame's list: every placed entity under its model's uploaded handle, those whose
    /// model is not uploaded left out.
    pub fn list(&self) -> Vec<(EntityId, ModelHandle)> {
        self.all()
            .filter_map(|p| Some((p.id, self.handle_of(p)?)))
            .collect()
    }

    /// Refreshes each listed entity's drawn parts ([`Walkers::displays`]): a built-in walker
    /// as the set poses it, a rig instance between its own clip's two ticks around now
    /// ([`vxe_render::DisplayPose::between_into`]). The list and each entry's storage are kept
    /// across frames, so built-in walkers allocate nothing per entity; a rig instance samples
    /// its own two poses.
    pub fn refresh_displays(&mut self) {
        let mut n = 0;
        let total = self.placed.len() + usize::from(self.preview.is_some());
        for k in 0..total {
            let p = self
                .placed
                .get(k)
                .or(self.preview.as_ref())
                .expect("k < total");
            let Some((model, pose)) = self.set.get(p.id) else {
                continue;
            };
            let Some(handle) = self.handle_of(p) else {
                continue;
            };
            if n == self.displays.len() {
                self.displays
                    .push((p.id, handle, vxe_render::DisplayPose::default()));
            }
            let slot = &mut self.displays[n];
            (slot.0, slot.1) = (p.id, handle);
            let fits = match p.entry {
                Some(i) => {
                    let Some(l) = self.library.entries[i]
                        .viewer()
                        .and_then(|v| v.loaded.as_ref())
                    else {
                        continue;
                    };
                    let (a, b, alpha) = p.play.poses(l);
                    let (sa, sb) = self
                        .rig_scratch
                        .get_or_insert_with(|| (a.clone(), b.clone()));
                    crate::viewer::place_into(sa, &a, p.feet, p.yaw);
                    crate::viewer::place_into(sb, &b, p.feet, p.yaw);
                    slot.2.between_into(model, sa, sb, alpha)
                }
                None => slot.2.of_into(model, pose),
            };
            n += usize::from(fits);
        }
        self.shown = n;
    }

    /// The drawn parts [`Walkers::refresh_displays`] last refreshed, in placement order.
    pub fn displays(&self) -> &[(EntityId, ModelHandle, vxe_render::DisplayPose)] {
        &self.displays[..self.shown]
    }

    /// Each placed built-in walker's eye light, in placement order, while they are on: the
    /// frame takes those that fit the list's free slots.
    pub fn eye_lights(&self) -> Vec<vxe_render::Light> {
        if self.eye_lights_off {
            return Vec::new();
        }
        self.placed
            .iter()
            .filter(|p| p.entry.is_none())
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

    /// The menu's `eye lights`: flips them; the banner's words.
    pub fn toggle_eye_lights(&mut self) -> &'static str {
        self.eye_lights_off = !self.eye_lights_off;
        if self.eye_lights_off {
            "eye lights off"
        } else {
            "eye lights on"
        }
    }

    /// The menu's `walkers walk`: the walk on or off; the banner's words.
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

    /// Advances by `dt` seconds: each rig's own clip (the viewer's row and the next
    /// placement's) and every rig instance's, posing it at its clip's tick now; and the walk
    /// while it is on, posing every placed built-in walker ([`Walkers::walk_pose`]); while it
    /// is off the walkers hold their poses.
    pub fn step(&mut self, dt: f32) {
        for e in &mut self.library.entries {
            if let Some(v) = e.viewer_mut() {
                v.advance(dt);
            }
        }
        for p in self.placed.iter_mut().chain(self.preview.iter_mut()) {
            let Some(i) = p.entry else { continue };
            let Some(l) = self.library.entries[i]
                .viewer()
                .and_then(|v| v.loaded.as_ref())
            else {
                continue;
            };
            p.play.advance(dt, l.rate);
            let now = p.play.poses(l).0;
            let _ = self.set.set_pose(p.id, placed(&now, p.feet, p.yaw));
        }
        if !self.walking {
            return;
        }
        self.clock += dt;
        for p in self.placed.iter().filter(|p| p.entry.is_none()) {
            let (feet, yaw, swing) = Self::walk_pose(p, self.clock);
            // A walker's pose has its four parts, so it always fits its model.
            let _ = self.set.set_pose(p.id, pose(feet, yaw, swing));
        }
    }

    /// The placement the clip keys act on: the aimed entity's, else the newest.
    pub fn target(&self) -> Option<usize> {
        self.aimed
            .and_then(|id| self.placed.iter().position(|p| p.id == id))
            .or_else(|| self.placed.len().checked_sub(1))
    }

    /// The target's entry name and built rig, when it is a rig's instance.
    fn target_rig(&self) -> Option<(usize, &str, &Loaded)> {
        let k = self.target()?;
        let e = &self.library.entries[self.placed[k].entry?];
        Some((k, e.name.as_str(), e.viewer()?.loaded.as_ref()?))
    }

    /// `step` clips on from the target's playing clip, faded into ([`Play::cycle`]): the
    /// banner's words.
    pub fn cycle_clip(&mut self, step: i32) -> String {
        let Some((k, name, l)) = self.target_rig() else {
            return "no clip: no rig placed".to_owned();
        };
        let (name, l) = (name.to_owned(), l.clone());
        let p = &mut self.placed[k];
        if p.play.cycle(&l, step) {
            format!(
                "{name} #{} clip {}",
                k + 1,
                p.play.clip_name(&l).unwrap_or("")
            )
        } else {
            format!("{name}: no clip")
        }
    }

    /// Pauses or plays the target's clip: the banner's words.
    pub fn toggle_pause(&mut self) -> String {
        let Some((k, name, _)) = self.target_rig() else {
            return "no clip: no rig placed".to_owned();
        };
        let name = name.to_owned();
        format!("{name} #{} {}", k + 1, self.placed[k].play.toggle_pause())
    }

    /// The HUD's target row, `<entry> #<n> <clip> <tick>/<length>` (then ` paused`), for the
    /// aimed (else the newest) placement when it is a rig's instance; nothing otherwise.
    pub fn write_target(&self, out: &mut String) -> bool {
        let Some((k, name, l)) = self.target_rig() else {
            return false;
        };
        let p = &self.placed[k];
        let _ = write!(out, "{name} #{}", k + 1);
        match (p.play.clip_name(l), p.play.clip_tick(l)) {
            (Some(c), Some((t, len))) => {
                let _ = write!(out, " {c} {t}/{len}");
            }
            _ => out.push_str(" no clip"),
        }
        if p.play.paused {
            out.push_str(" paused");
        }
        true
    }

    /// The HUD row, `entities <n>/64 parts <m>/512 over <k>`, then, once the pool is read,
    /// ` pool <used>/<total> KB` (rounded up to whole KB), then ` walk` while they walk.
    pub fn write_row(&self, out: &mut String) {
        let _ = write!(
            out,
            "entities {}/{MAX_ENTITIES} parts {}/{MAX_ENTITY_PARTS} over {}",
            self.drawn, self.parts, self.over
        );
        if self.pool_total > 0 {
            let _ = write!(
                out,
                " pool {}/{} KB",
                self.pool_used.div_ceil(1024),
                self.pool_total.div_ceil(1024)
            );
        }
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

    /// Places the pending ring of the selected entry around `centre`, each on the first floor
    /// below its point (`floor_below` gives the drop in voxels), facing the centre.
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
            if let Err(e) = self.place_selected(table, feet, centre) {
                println!("{e}");
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::sync_models;
    use super::super::walker::{WALK_RADIUS, WALK_SWING, model, pose, rows, with_walker};
    use super::*;
    use vxe_entity::Q14_ONE;

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

    /// Plan 0078 Phase 3 done-when 4: the instance table's write over 64 walking walkers
    /// allocates nothing after the first of 101 writes (the walkers' display poses refreshed in
    /// place between them; the mirror's `set_entity_displays` is this write and two
    /// `write_buffer` calls).
    #[test]
    fn write_allocates_nothing_after_warm_up() {
        use crate::test_support::counting;
        use vxe_render::internals::{EntityInstances, VolumePool};

        let table = with_walker(&crate::cave_materials());
        let mut w = Walkers::default();
        for i in 0..64 {
            let feet = [(i % 8) as f32 * 20.0, (i / 8) as f32 * 20.0, 2.0];
            assert!(w.place(&table, feet, [0.0, -9.0, 2.0]));
        }
        let mut pool = VolumePool::new(vxe_render::DEFAULT_ENTITY_POOL_BYTES);
        assert_eq!(sync_models(&mut w, &mut pool, &table), None);
        w.toggle_walk();
        let mut inst = EntityInstances::new();
        let mut bytes = Vec::with_capacity(101);
        for _ in 0..101 {
            w.step(1.0 / 60.0);
            w.refresh_displays();
            let before = counting::bytes();
            inst.write(
                &pool,
                w.displays().iter().map(|(id, h, d)| (*id, *h, Some(d))),
            );
            bytes.push(counting::bytes() - before);
        }
        assert!(bytes[0] > 0, "the first write grows the scratch");
        assert_eq!(bytes[1..], [0; 100]);
        assert_eq!((inst.entities_drawn(), inst.parts_drawn()), (64, 256));
        assert!(inst.entities_moved());
    }

    /// Plan 0078 Phase 2 done-when 2: with the walker uploaded the row reads `pool 5/8192 KB`
    /// (4156 B rounded up), and `pool 0/8192 KB` once the pool is cleared and read again.
    #[test]
    fn the_hud_row_shows_the_pool() {
        use vxe_render::internals::VolumePool;

        let table = with_walker(&MaterialTable::new());
        let mut w = Walkers::default();
        assert!(w.place(&table, [0.0; 3], [0.0, -9.0, 0.0]));
        (w.drawn, w.parts, w.over) = (1, 4, 0);
        let mut pool = VolumePool::new(vxe_render::DEFAULT_ENTITY_POOL_BYTES);
        assert_eq!(sync_models(&mut w, &mut pool, &table), None);
        assert_eq!(pool.used_bytes(), 4156);
        let row = |w: &Walkers| {
            let mut row = String::new();
            w.write_row(&mut row);
            row
        };
        assert_eq!(row(&w), "entities 1/64 parts 4/512 over 0 pool 5/8192 KB");
        pool.clear();
        assert_eq!(sync_models(&mut w, &mut pool, &table), None);
        assert_eq!(row(&w), "entities 1/64 parts 4/512 over 0 pool 0/8192 KB");
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
            let p = m.walkers.placed[0].clone();
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
        let new = w.placed.last().cloned().unwrap();
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

    /// Refreshing the drawn parts allocates nothing per entity once its storage has grown:
    /// eight built-in walkers allocate nothing from the second refresh on, and each holds its
    /// set pose's parts; rig instances, each on its own playback, allocate only the two poses
    /// each samples: four instances allocate four times one's bytes, the drawn parts' storage
    /// kept.
    #[test]
    fn refreshing_the_displays_allocates_nothing_per_entity() {
        use crate::test_support::counting;
        use vxe_render::DisplayPose;

        let table = with_walker(&crate::cave_materials());
        let mut w = Walkers::default();
        for i in 0..8 {
            assert!(w.place(&table, [i as f32 * 20.0, 0.0, 2.0], [0.0, -9.0, 2.0]));
        }
        // Two handles of a pool of the walker's model and one of the viewer's.
        let mut pool = vxe_render::internals::VolumePool::new(1 << 20);
        let (body, eye) = rows(&table).unwrap();
        let mut handle = || pool.upload(&model(body, eye), &table).unwrap().handle;
        let (h0, h1) = (handle(), handle());
        w.handle = Some(h0);
        w.toggle_walk();
        let mut bytes = Vec::new();
        for _ in 0..4 {
            w.step(1.0 / 60.0);
            let before = counting::bytes();
            w.refresh_displays();
            bytes.push(counting::bytes() - before);
        }
        assert!(bytes[0] > 0, "the first refresh grows the storage");
        assert_eq!(bytes[1..], [0; 3]);
        assert_eq!(w.displays().len(), 8);
        for (p, (id, h, d)) in w.placed.iter().zip(w.displays()) {
            let (model, pose) = w.set.get(p.id).unwrap();
            assert_eq!((*id, *h), (p.id, h0));
            assert_eq!(Some(d), DisplayPose::of(model, pose).as_ref());
        }

        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets/entities/walker/walker.rig");
        let mut viewer = crate::viewer::Viewer::open(&path, Some("walk".into())).expect("opens");
        assert_eq!(viewer.build_pending(&table), None);
        viewer.handle = Some(h1);
        let mut w = Walkers::default().with_viewer(Some(viewer));
        let refresh = |w: &mut Walkers| {
            w.step(1.0 / 45.0);
            let before = counting::bytes();
            w.refresh_displays();
            counting::bytes() - before
        };
        assert!(w.place(&table, [0.0, 0.0, 2.0], [0.0, -9.0, 2.0]));
        refresh(&mut w);
        let one = refresh(&mut w);
        for i in 1..4 {
            assert!(w.place(&table, [i as f32 * 20.0, 0.0, 2.0], [0.0, -9.0, 2.0]));
        }
        refresh(&mut w);
        let four = refresh(&mut w);
        assert_eq!(w.displays().len(), 4);
        assert!(one > 0, "an instance samples its two poses");
        assert_eq!(
            four,
            4 * one,
            "each instance samples its own, the storage kept"
        );
    }
}
