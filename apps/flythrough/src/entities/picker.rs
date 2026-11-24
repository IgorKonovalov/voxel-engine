//! The chooser (the menu's dynamic page, `hud::menu::PageId::Entities`): each library entry's
//! rows as the page draws them; and the ghost, placement mode's selected entry standing at
//! the aim (`super::belt::stand`; a surface body on the aimed face) facing the camera turned
//! by the belt's turn, a real entity in the set that a click leaves placed under its own id.

use std::sync::Arc;

use svet_entity::{EntityId, Model, Pose};
use svet_render::MAX_ENTITIES;
use svet_world::MaterialTable;

use super::library::{BUILTIN_NAME, Source, cut, extent, model_of};
use super::mover::{Mover, Stance};
use super::walker::pose;
use super::{FrameGhost, Placed, Walkers};
use crate::hud::Modes;
use crate::viewer::placed;

/// Why library entry `i` cannot be placed, at the modes `m`.
pub fn item_error(m: &Modes, i: usize) -> Option<&str> {
    m.walkers.library.entries.get(i)?.error()
}

/// Where the reason of entry `i` in error starts in its [`write_item`] words, in bytes: after
/// its name padded to ten cells and a space. `None` for an entry not in error.
pub fn reason_at(m: &Modes, i: usize) -> Option<usize> {
    let e = m.walkers.library.entries.get(i)?;
    e.error().map(|_| e.name.len().max(10) + 1)
}

/// Entry `i`'s row after its selection mark, at most `width` bytes: `<name> <n> parts <x> x
/// <y> x <z> m` (its size standing in its rest pose), `<name> built-in`, or for an entry in
/// error its name and the error from [`reason_at`] (the name drawn grey, the reason red).
pub fn write_item(m: &Modes, i: usize, text: &mut String, width: usize) {
    let lib = &m.walkers.library;
    let row = match lib.entries.get(i) {
        None => format!("{BUILTIN_NAME:<10} built-in"),
        Some(e) => match (e.error(), &e.source) {
            (Some(err), _) => format!("{:<10} {err}", e.name),
            (None, Ok(Source::Rig(v))) => match &v.loaded {
                Some(l) => {
                    let s = extent(&l.model, &l.rest).map(|v| v / 10.0);
                    format!(
                        "{:<10} {:>2} parts  {:.1} x {:.1} x {:.1} m",
                        e.name,
                        l.model.parts().len(),
                        s[0],
                        s[1],
                        s[2]
                    )
                }
                None => format!("{:<10} not built", e.name),
            },
            (None, _) => format!("{:<10} built-in", e.name),
        },
    };
    text.push_str(&cut(row, width));
}

/// Entry `i`'s second row: its clips (the one a placement starts on marked `*`) or `no clip`,
/// then the belt's slot that holds it (`slot 2`) or `no slot`.
pub fn write_item_clips(m: &Modes, i: usize, text: &mut String) {
    use std::fmt::Write as _;
    let lib = &m.walkers.library;
    let rig = lib.entries.get(i).and_then(|e| e.viewer());
    match rig.and_then(|v| Some((v, v.loaded.as_ref()?))) {
        Some((v, l)) if !l.clips.is_empty() => {
            text.push_str("clips");
            for (k, (name, _)) in l.clips.iter().enumerate() {
                let star = if k == v.play.clip { "*" } else { "" };
                let _ = write!(text, " {name}{star}");
            }
        }
        _ => text.push_str("no clip"),
    }
    match m.walkers.belt.slot_of(super::belt::Item::Entity(i)) {
        Some(s) => {
            let _ = write!(text, "   slot {}", s + 1);
        }
        None => text.push_str("   no slot"),
    }
}

/// The words a full set gives in place of a preview or a placement.
pub fn full_words() -> String {
    format!("entities {MAX_ENTITIES}/{MAX_ENTITIES} full")
}

impl Walkers {
    /// Stands the ghost of `entry` (`None` the built-in walker) at `stance`: moved there when
    /// it already shows that entry (a body's mover stood anew, its wander kept), else placed
    /// anew (the old one removed). With [`MAX_ENTITIES`] placed there is no ghost and the
    /// words say the set is full.
    pub fn show_preview(
        &mut self,
        table: &MaterialTable,
        entry: Option<usize>,
        stance: Stance,
    ) -> Result<(), String> {
        if self.placed.len() >= MAX_ENTITIES {
            self.clear_preview();
            return Err(full_words());
        }
        let (feet, yaw) = (stance.point, stance.yaw);
        if let Some(p) = &mut self.preview
            && p.entry == entry
        {
            if (p.feet, p.yaw) != (feet, yaw) {
                (p.feet, p.yaw) = (feet, yaw);
                let rig = entry
                    .and_then(|i| self.library.entries[i].viewer())
                    .and_then(|v| Some((v.loaded.as_ref()?, v.body)));
                let now = match (rig, &mut p.mover) {
                    (Some((l, Some(body))), Some(m)) => {
                        let parts = p.play.poses(l).0;
                        let wander = m.wander;
                        let (up, fwd) = (stance.up, stance.forward);
                        *m = Mover::new(body, feet, up, fwd, (0, 0), parts);
                        m.wander = wander;
                        m.now.clone()
                    }
                    (Some((l, _)), _) => placed(&p.play.poses(l).0, feet, yaw),
                    (None, _) => pose(feet, yaw, 0.0),
                };
                let _ = self.set.set_pose(p.id, now);
            }
            return Ok(());
        }
        self.clear_preview();
        self.place_at(table, entry, stance)?;
        self.preview = self.placed.pop();
        Ok(())
    }

    /// Removes the ghost from the set.
    pub fn clear_preview(&mut self) {
        if let Some(p) = self.preview.take() {
            self.set.remove(p.id);
        }
    }

    /// Stands a structure's ghost, its coarse copy in its box's frame, with its minimum corner
    /// at `corner`: moved there when the ghost shows `key` (the structure, scale, quarter
    /// turns and whether the body is in), else made anew by `make` (the old one removed, its
    /// handle freed by the next sync). With [`MAX_ENTITIES`] placed there is no ghost and the
    /// words say the set is full.
    pub fn show_frame(
        &mut self,
        key: (usize, u32, u8, bool),
        make: impl FnOnce() -> Model,
        corner: [i32; 3],
    ) -> Result<(), String> {
        if self.placed.len() >= MAX_ENTITIES {
            self.clear_frame();
            return Err(full_words());
        }
        let root = svet_world::Fixed3(corner.map(|v| i64::from(v) * 256));
        if let Some(f) = &mut self.frame
            && f.key == key
        {
            if f.corner != corner {
                f.corner = corner;
                let pose = Pose {
                    root,
                    ..Pose::rest(&f.model)
                };
                let _ = self.set.set_pose(f.id, pose);
            }
            return Ok(());
        }
        self.clear_frame();
        let model = Arc::new(make());
        let pose = Pose {
            root,
            ..Pose::rest(&model)
        };
        let id = self
            .set
            .insert(model.clone(), pose)
            .map_err(|e| e.to_string())?;
        self.frame = Some(FrameGhost {
            id,
            key,
            model,
            handle: None,
            corner,
        });
        Ok(())
    }

    /// Removes a structure's ghost from the set; its handle is freed by the next sync.
    pub fn clear_frame(&mut self) {
        if let Some(f) = self.frame.take() {
            self.set.remove(f.id);
            self.frame_stale.extend(f.handle);
        }
    }

    /// The ghost placed where it stands: it joins [`Walkers::placed`] under its own id.
    pub fn place_preview(&mut self) -> Option<EntityId> {
        let p: Placed = self.preview.take()?;
        let id = p.id;
        self.placed.push(p);
        Some(id)
    }

    /// The ghost's clip `step` clips on (faded into), and its entry's with it, so the next
    /// ghost starts on it: the banner's words.
    pub fn ghost_clip(&mut self, step: i32) -> String {
        let Some(i) = self.preview.as_ref().and_then(|p| p.entry) else {
            return "no clip: no rig in placement".to_owned();
        };
        let e = &mut self.library.entries[i];
        let name = e.name.clone();
        let Some(v) = e.viewer_mut() else {
            return format!("{name}: no clip");
        };
        let Some(l) = v.loaded.clone() else {
            return format!("{name}: no clip");
        };
        let p = self.preview.as_mut().expect("a ghost");
        if !p.play.cycle(&l, step) {
            return format!("{name}: no clip");
        }
        while v.play.clip != p.play.clip && v.play.cycle(&l, 1) {}
        let clip = p.play.clip_name(&l).unwrap_or("");
        format!("{name} ghost clip {clip}")
    }

    /// Pauses or plays the ghost's clip: the banner's words.
    pub fn ghost_pause(&mut self) -> String {
        match self.preview.as_mut() {
            Some(p) if p.entry.is_some() => format!("ghost {}", p.play.toggle_pause()),
            _ => "no clip: no rig in placement".to_owned(),
        }
    }

    /// The size of entry `entry`'s model (`None` the built-in walker) standing in its rest
    /// pose, in voxels along x, y and z; zero for one with no model.
    pub fn extent_of(&self, entry: Option<usize>, table: &MaterialTable) -> [f32; 3] {
        let model = match entry {
            Some(i) => self.library.entries.get(i).and_then(|e| model_of(e, table)),
            None => super::library::builtin_model(table),
        };
        model.map_or([0.0; 3], |(m, rest)| extent(&m, &rest))
    }

    /// Removes the placed entity under the crosshair ([`Walkers::aimed`]; the ghost is not
    /// one) and its mark in `4`'s list: the banner's words.
    pub fn remove_aimed(&mut self) -> String {
        let Some(k) = self
            .aimed
            .and_then(|id| self.placed.iter().position(|p| p.id == id))
        else {
            return "remove: no entity at the crosshair".to_owned();
        };
        self.remove_at(k);
        // `4`'s list marks entities `true` in placement order: the k-th mark is this one's.
        if let Some(j) = self
            .newest
            .iter()
            .enumerate()
            .filter(|(_, e)| **e)
            .nth(k)
            .map(|(j, _)| j)
        {
            self.newest.remove(j);
        }
        self.count_words()
    }
}
