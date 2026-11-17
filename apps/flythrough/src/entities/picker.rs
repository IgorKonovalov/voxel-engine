//! The entities page's picker (the menu's dynamic page, `hud::menu::PageId::Entities`): the
//! library's rows and the selected entry's clip row as the page draws them, and the preview,
//! the selected entry standing at the crosshair's surface point facing the camera, a real
//! entity in the set that `Enter` (or `` ` ``) leaves placed under its own id.

use vxe_entity::EntityId;
use vxe_render::MAX_ENTITIES;
use vxe_world::MaterialTable;

use super::library::{BUILTIN_NAME, Source, cut, extent};
use super::walker::{facing, pose};
use super::{Placed, Walkers};
use crate::hud::Modes;
use crate::viewer::placed;

/// Why library entry `i` cannot be placed, at the modes `m`.
pub fn item_error(m: &Modes, i: usize) -> Option<&str> {
    m.walkers.library.entries.get(i)?.error()
}

/// Entry `i`'s row after its selection mark, at most `width` bytes: `<name> <n> parts <x> x
/// <y> x <z> m` (its size standing in its rest pose), `<name> built-in`, or for an entry in
/// error its name and the error (the row drawn grey).
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

/// The clip row: `clip  <name>  (<k>/<n>)` for the selected entry's clip, which `Enter`
/// cycles; `clip  none` for an entry with no clip.
pub fn write_clip(m: &Modes, text: &mut String) {
    use std::fmt::Write as _;
    let lib = &m.walkers.library;
    let clips = lib
        .entries
        .get(lib.selected)
        .and_then(|e| e.viewer())
        .and_then(|v| Some((v, v.loaded.as_ref()?)))
        .filter(|(_, l)| !l.clips.is_empty());
    match clips {
        Some((v, l)) => {
            let name = v.clip_name().unwrap_or("");
            let _ = write!(
                text,
                "clip  {name}  ({}/{})",
                v.play.clip + 1,
                l.clips.len()
            );
        }
        None => text.push_str("clip  none"),
    }
}

/// The words a full set gives in place of a preview or a placement.
pub fn full_words() -> String {
    format!("entities {MAX_ENTITIES}/{MAX_ENTITIES} full")
}

impl Walkers {
    /// Stands the preview of `entry` (`None` the built-in walker) at `feet`, facing `eye`:
    /// moved there when it already shows that entry, else placed anew (the old one removed).
    /// With [`MAX_ENTITIES`] placed there is no preview and the words say the set is full.
    pub fn show_preview(
        &mut self,
        table: &MaterialTable,
        entry: Option<usize>,
        feet: [f32; 3],
        eye: [f32; 3],
    ) -> Result<(), String> {
        if self.placed.len() >= MAX_ENTITIES {
            self.clear_preview();
            return Err(full_words());
        }
        let yaw = facing(feet, eye);
        if let Some(p) = &mut self.preview
            && p.entry == entry
        {
            if (p.feet, p.yaw) != (feet, yaw) {
                (p.feet, p.yaw) = (feet, yaw);
                let rig = entry
                    .and_then(|i| self.library.entries[i].viewer())
                    .and_then(|v| v.loaded.as_ref());
                let now = match rig {
                    Some(l) => placed(&p.play.poses(l).0, feet, yaw),
                    None => pose(feet, yaw, 0.0),
                };
                let _ = self.set.set_pose(p.id, now);
            }
            return Ok(());
        }
        self.clear_preview();
        self.place_entry(table, entry, feet, yaw)?;
        self.preview = self.placed.pop();
        Ok(())
    }

    /// Removes the preview from the set.
    pub fn clear_preview(&mut self) {
        if let Some(p) = self.preview.take() {
            self.set.remove(p.id);
        }
    }

    /// The preview placed where it stands: it joins [`Walkers::placed`] under its own id.
    pub fn place_preview(&mut self) -> Option<EntityId> {
        let p: Placed = self.preview.take()?;
        let id = p.id;
        self.placed.push(p);
        Some(id)
    }

    /// The selected entry's next clip, the preview's with it: the notice's words.
    pub fn next_clip(&mut self) -> String {
        let i = self.library.selected;
        let Some(e) = self.library.entries.get_mut(i) else {
            return "clip: none".to_owned();
        };
        let name = e.name.clone();
        let Some(v) = e.viewer_mut() else {
            return format!("{name}: no clip");
        };
        let words = v.cycle(1);
        let (clip, l) = (v.play.clip, v.loaded.clone());
        if let (Some(p), Some(l)) = (&mut self.preview, l)
            && p.entry == Some(i)
        {
            while p.play.clip != clip && p.play.cycle(&l, 1) {}
        }
        format!("{name} {words}")
    }

    /// Removes the placed entity under the crosshair ([`Walkers::aimed`]; the preview is not
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
