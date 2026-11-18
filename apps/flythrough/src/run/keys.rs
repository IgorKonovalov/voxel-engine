//! The run's key and pad dispatch, and the cycles its keys step through.

use crate::app::ms_since;
use crate::castle;
use crate::cli::walk_allowed;
use crate::entities::Pick;
use crate::entities::belt;
use crate::entities::picker::full_words;
use crate::fly::Walk;
use crate::hud::menu::{Activated, AudioView, RunView};
use crate::hud::{self};
use crate::lights;
use crate::logic::DigInput;
use crate::logic::REACH;
use crate::path::Drive;
use crate::run::Run;
use crate::run::hud_feed::Samples;
use crate::setup::{castle_banner, castle_camera};
use crate::toggles::{ACTIONS, Act, BELT_KEYS, TOGGLES, press};
use vxe_app::{Axis, Input, MouseButton, PadButton};
use vxe_audio::{Command, PlayParams};
use vxe_render::Camera;
use vxe_world::{FIXED_ONE, Face, Fixed3};

/// A trigger presses (`R2` digs, `L2` fills) once pulled past this, and lets go under
/// [`TRIGGER_RELEASE`], so a trigger resting near a threshold does not repeat.
pub(crate) const TRIGGER_PRESS: f32 = 0.5;

pub(crate) const TRIGGER_RELEASE: f32 = 0.3;

/// Whether a trigger at `pull` presses this frame, with `held` its state since the last.
pub(crate) fn trigger_pressed(held: &mut bool, pull: f32) -> bool {
    if *held {
        *held = pull >= TRIGGER_RELEASE;
        false
    } else {
        *held = pull >= TRIGGER_PRESS;
        *held
    }
}

/// The ore's emission scale `ms` milliseconds into the run while `M` pulses it: between 0.25
/// and 1.75, one cycle every 1.5 s, about [`ORE_PULSE_MEAN`].
pub(crate) fn ore_pulse(ms: f64) -> f32 {
    (f64::from(ORE_PULSE_MEAN) + 0.75 * (ms / 1500.0 * std::f64::consts::TAU).sin()) as f32
}

/// How far the crosshair looks for a placed entity with no world surface in the way, in
/// voxels: 50 m.
pub(crate) const AIM_RANGE: f32 = 500.0;

/// The ore pulse's time average, the scale the probe rays see the ore at, pulsing or not.
pub(crate) const ORE_PULSE_MEAN: f32 = 1.0;

impl Run {
    /// `G`: walk mode with the body under the camera, or back to flight.
    pub(crate) fn toggle_walk(&mut self) {
        if self.walk.take().is_some() {
            return self.say("fly mode");
        }
        if let Err(e) = walk_allowed(self.stream.is_some()) {
            return self.say(format!("walk: {e}; still flying"));
        }
        match Walk::at_camera(&self.world, &self.fly.camera) {
            Some(w) => {
                self.walk = Some(w);
                self.say("walk mode");
            }
            None => self.say("walk: no room for the body here; still flying"),
        }
    }

    /// Brings the run up to the modes after a toggle changed them from `before`: the
    /// upscaler and its sharpening at the next frame, the shadow ray, and the live queue's
    /// order, withdrawal, budget, full-first radius, missing light and reach, kept in `args`
    /// for the frame and the readings.
    pub(crate) fn apply_modes(&mut self, before: &hud::Modes) {
        let m = &self.modes;
        self.upscaler_changed |= m.upscaler.mode != before.upscaler.mode;
        if m.upscaler.sharpen != before.upscaler.sharpen {
            self.args.look.sharpen = m.upscaler.sharpen;
            self.sharpen_changed = true;
        }
        self.light.shadows = m.shadow_ray;
        let (q, b) = (m.queue, before.queue);
        let bake = &mut self.args.bake;
        (bake.queue_distance, bake.queue_tier) = (q.distance, q.tier);
        (bake.queue_angle, bake.queue_refine, bake.queue_withdraw) =
            (q.angle, q.refine, q.withdraw);
        bake.budget = q.budget;
        bake.full_first = Some(q.full_first);
        bake.missing_light = q.missing_light;
        bake.reach = q.reach;
        let Some(g) = self.gfx.as_mut() else { return };
        if q.reach != b.reach {
            g.frame.set_bake_reach(q.reach);
        }
        if q.full_first != b.full_first {
            g.frame.set_bake_full_first(q.full_first);
        }
        if q.budget != b.budget {
            g.frame.set_bake_budget(q.budget);
        }
        if (q.order(), q.withdraw) != (b.order(), b.withdraw)
            && let Some(queue) = g.frame.bake_queue_mut()
        {
            queue.order = q.order();
            queue.withdraw = q.withdraw;
        }
    }

    /// The toggles' keys, through [`TOGGLES`]: each pressed one flips its mode and prints its
    /// HUD row; then the run takes the changes ([`Run::apply_modes`]).
    pub(crate) fn toggle_keys(&mut self, input: &Input) {
        let mut before = None;
        let now = ms_since(self.start);
        let placing = self.modes.walkers.belt.on;
        for t in TOGGLES.iter().filter(|t| t.pressed(input)) {
            // In placement mode `West` turns the ghost; the toggle it presses is the menu's.
            if placing
                && let crate::toggles::Press::Key(k, Some(PadButton::West)) = t.press
                && !input.pressed(k)
            {
                continue;
            }
            before.get_or_insert_with(|| self.modes.clone());
            println!("{}", press(t, &mut self.modes, now));
        }
        if let Some(before) = before {
            self.apply_modes(&before);
        }
    }

    /// What the menu hands the run: a toggle flipped as its key flips it (its row answers; a
    /// flip that leaves the row as it was says its line in the notice), a number stepped, an
    /// action run as its bind runs it, or an unavailable entry's reason said.
    pub(crate) fn menu_act(&mut self, a: Activated) {
        match a {
            Activated::Toggle(t) => {
                let before = self.modes.clone();
                let line = press(t, &mut self.modes, ms_since(self.start));
                if (t.get)(&self.modes) == (t.get)(&before) {
                    self.say(line);
                } else {
                    println!("{line}");
                }
                self.apply_modes(&before);
            }
            Activated::Step(t, d) => {
                let before = self.modes.clone();
                if let Some(n) = t.number {
                    (n.step)(&mut self.modes, d);
                }
                println!("{}", self.modes.row(t.row));
                self.apply_modes(&before);
            }
            Activated::Action(act) => self.act(act),
            Activated::Refused(why) => self.say(why),
            Activated::Choose { entry } => self.choose(entry),
            Activated::Assign { entry, slot } => {
                self.fill_belt();
                let belt = &mut self.modes.walkers.belt;
                belt.assign(slot, entry);
                belt.point(slot, ms_since(self.start));
                self.belt_selected();
                let name = self.entry_name(Some(entry));
                self.say(format!("{name} in slot {}", slot + 1));
            }
        }
    }

    /// Prints `line` and shows it in the menu's notice row from now: what an action answers
    /// in the window.
    pub(crate) fn say(&mut self, line: impl Into<String>) {
        let line = line.into();
        println!("{line}");
        self.notice = Some((line, ms_since(self.start)));
    }

    /// The flags an action's mode reads beside the modes.
    pub(crate) fn run_view(&self) -> RunView {
        RunView {
            walking: self.walk.is_some(),
            audio: self.audio.as_ref().map(|a| {
                AudioView::of(
                    &a.mix,
                    a.acoustics.reflections_on(),
                    a.acoustics.reverb_on(),
                )
            }),
            path: self.player.is_some(),
            castle: self.castle_shot.is_some(),
            model: self.modes.walkers.viewer().is_some(),
            placed: !self.modes.walkers.placed.is_empty(),
            streaming: self.stream.is_some(),
            coarse: self.coarse.is_some(),
        }
    }

    /// The keys, buttons and pad buttons of interactive flight; the autopilot takes none of
    /// them, so a stray click cannot change the world it measures. The toggles go through
    /// [`TOGGLES`] ([`Run::toggle_keys`]) and the actions through [`ACTIONS`] ([`Run::act`]);
    /// the wheel, the clicks and the triggers are read here. Out of placement mode the left
    /// click (or `R2`) digs, the right click (or `L2`) fills and the wheel enters the mode at
    /// the belt's selected slot. In the mode the wheel steps the slot (down the next), the
    /// left click or `R2` places the ghost, the right click, `L2` or the pad's `East` leaves,
    /// `West` turns the ghost and every digit selects its slot (`toggles::BELT_KEYS`); `1`,
    /// `4` and `6` drop, remove and fire nothing and `East` knocks nothing. No digit enters
    /// the mode.
    pub(crate) fn handle_input(&mut self, input: &Input) {
        let placing = self.modes.walkers.belt.on;
        self.toggle_keys(input);
        let now = ms_since(self.start);
        let lines = self.scroll.add(input.scroll());
        if lines != 0 {
            self.fill_belt();
            let belt = &mut self.modes.walkers.belt;
            if belt.on {
                belt.step(-lines as i32, now);
            } else {
                belt.select(belt.selected, now);
            }
            self.belt_selected();
        }
        let r2 = trigger_pressed(&mut self.triggers[1], input.axis(Axis::TriggerR));
        let l2 = trigger_pressed(&mut self.triggers[0], input.axis(Axis::TriggerL));
        let (left, right) = (
            input.clicked(MouseButton::Left),
            input.clicked(MouseButton::Right),
        );
        if placing {
            for (s, &k) in BELT_KEYS.iter().enumerate() {
                if input.pressed(k) {
                    self.modes.walkers.belt.select(s, now);
                    self.belt_selected();
                }
            }
            if input.pad_pressed(PadButton::West) {
                self.act(Act::Turn);
            }
            if left || r2 {
                self.place_ghost();
            }
            if right || l2 || input.pad_pressed(PadButton::East) {
                self.end_placement("placement off");
            }
        } else {
            if left || r2 {
                self.dig();
            }
            if right || l2 {
                self.fill();
            }
        }
        for a in ACTIONS.iter().filter(|a| a.pressed(input)) {
            let act = match a.act {
                Act::DropLight | Act::Remove | Act::FireLight | Act::Knock if placing => continue,
                act => act,
            };
            self.act(act);
        }
        self.sync_ghost();
    }

    /// Runs `act`: the run's method an action's bind and its menu entry both call.
    pub(crate) fn act(&mut self, act: Act) {
        match act {
            Act::Walk => self.toggle_walk(),
            Act::ResumePath => self.resume_path(),
            Act::CastleShot => self.next_castle_shot(),
            Act::Picker => {
                self.fill_belt();
                let w = &self.modes.walkers;
                let row = w
                    .belt
                    .entry()
                    .and_then(|i| w.library.order().iter().position(|&o| o == i));
                self.menu.open_entities(row.unwrap_or(0));
                self.sync_ghost();
                let n = self.modes.walkers.library.entries.len().max(1);
                self.say(format!("entities library: {n} entries"));
            }
            Act::BeltStep(d) => {
                self.fill_belt();
                let belt = &mut self.modes.walkers.belt;
                belt.step(i32::from(d), ms_since(self.start));
                self.menu.close();
                self.belt_selected();
            }
            Act::Turn => {
                let belt = &mut self.modes.walkers.belt;
                if belt.on {
                    belt.turn_step();
                    let deg = f32::from(belt.turn) * belt::TURN_STEP_DEG;
                    self.say(format!("ghost turned {deg} degrees"));
                } else {
                    self.say("turn: in placement mode only");
                }
            }
            Act::DropLight
            | Act::Remove
            | Act::FireLight
            | Act::RemoveAimed
            | Act::WalkersWalk
            | Act::EyeLights
            | Act::ClipBack
            | Act::ClipOn
            | Act::ClipPause => {
                if let Some(words) = self.light_act(act) {
                    self.modes.show_lights(words.clone(), ms_since(self.start));
                    self.say(words);
                }
            }
            Act::Knock
            | Act::Pulse
            | Act::Hum
            | Act::Echoes
            | Act::Tail
            | Act::EchoTrim(_)
            | Act::TailTrim(_)
            | Act::PreDelay(_)
            | Act::Glide
            | Act::DirectDelay => self.audio_act(act),
            Act::Readings => {
                self.samples = Some(Samples::default());
                self.say(format!(
                    "readings: sampling {} frames",
                    crate::run::hud_feed::SAMPLES
                ));
            }
            Act::Bench => {
                self.bench();
                self.say("bench: sent to the logic");
            }
            Act::Stamp => {
                self.stamp();
                self.say("stamp: sent to the logic");
            }
            Act::Crumble => match self.coarse {
                Some(_) => {
                    self.send(DigInput::Crumble);
                    self.say("crumble: sent to the logic");
                }
                None => self.say("collapse: not kept while streaming"),
            },
            Act::CoarsePrint => self.say(self.coarse_line()),
            Act::Profile => self.cycle_profile(),
            Act::NextSource => {
                self.switch = true;
                self.say("switch: the next source, after this frame");
            }
            Act::Trees => {
                if self.args.mode.is_outdoor() {
                    self.retree = true;
                    self.say("switch: the next trees, after this frame");
                } else {
                    self.say("trees: needs --source outdoor");
                }
            }
        }
    }

    /// A tour in free flight resumes from the path's time nearest the camera, as `T` does in
    /// the path's own step.
    fn resume_path(&mut self) {
        let eye = self.fly.camera.position.to_array();
        let Some(p) = self.player.as_mut() else {
            return self.say("tour: no path in this run");
        };
        if p.drive != Drive::Free {
            return self.say("tour: already on the path");
        }
        let t_s = p.path.nearest_time(eye);
        p.drive = Drive::Path { t_s };
        self.say(format!("tour: resumed at {t_s:.1} s"));
    }

    /// The camera at the next castle shot, its banner shown.
    fn next_castle_shot(&mut self) {
        let Some(shot) = self.castle_shot.map(castle::CastleShot::next) else {
            return self.say("castle: no castle in this run");
        };
        self.castle_shot = Some(shot);
        self.fly.camera = Camera {
            vfov: self.fly.camera.vfov,
            ..castle_camera(shot)
        };
        let line = castle_banner(shot);
        self.say(line.clone());
        self.scene = Some((line, ms_since(self.start)));
    }

    /// The camera ray's first surface within `max` voxels: its distance in voxels and the
    /// entered face's outward normal.
    fn surface_along_view(&self, max: f32) -> Option<(f32, [f32; 3])> {
        let (origin, dir) = self.camera_ray();
        let hit = self
            .world
            .raycast(origin, dir, (max * FIXED_ONE as f32) as i64)
            .ok()
            .flatten()?;
        let normal = match hit.face? {
            Face::NegX => [-1.0, 0.0, 0.0],
            Face::PosX => [1.0, 0.0, 0.0],
            Face::NegY => [0.0, -1.0, 0.0],
            Face::PosY => [0.0, 1.0, 0.0],
            Face::NegZ => [0.0, 0.0, -1.0],
            Face::PosZ => [0.0, 0.0, 1.0],
        };
        Some((hit.t as f32 / FIXED_ONE as f32, normal))
    }

    /// Reads the placed entity under the crosshair into the entities' `aimed`: the first along
    /// the camera's ray before the world's surface ([`vxe_entity::EntitySet::raycast`] with
    /// the world hit's distance as its reach; [`AIM_RANGE`] voxels with no hit).
    pub(crate) fn aim_entities(&mut self) {
        let w = &mut self.modes.walkers;
        if w.placed.is_empty() {
            w.aimed = None;
            return;
        }
        let (origin, dir) = self.camera_ray();
        let reach = (AIM_RANGE * FIXED_ONE as f32) as i64;
        let max_t = self
            .world
            .raycast(origin, dir, reach)
            .ok()
            .flatten()
            .map_or(reach, |h| h.t);
        let w = &mut self.modes.walkers;
        let preview = w.preview.as_ref().map(|p| p.id);
        w.aimed = w
            .set
            .raycast(origin, dir, max_t)
            .map(|h| h.entity)
            .filter(|&id| Some(id) != preview);
    }

    /// The world's first hit along the unit `dir` from `origin` within `max` voxels: its
    /// distance and the entered face's outward normal (no normal, distance 0, when `origin`
    /// is inside a solid voxel). [`belt::stand`]'s cast.
    fn cast_world(&self, origin: [f32; 3], dir: [f32; 3], max: f32) -> Option<(f32, [f32; 3])> {
        let one = FIXED_ONE as f32;
        let o = Fixed3(origin.map(|v| (v * one).floor() as i64));
        let d = dir.map(|v| (v * one).round() as i64);
        let hit = self
            .world
            .raycast(o, d, (max * one) as i64)
            .ok()
            .flatten()?;
        let normal = match hit.face {
            Some(Face::NegX) => [-1.0, 0.0, 0.0],
            Some(Face::PosX) => [1.0, 0.0, 0.0],
            Some(Face::NegY) => [0.0, -1.0, 0.0],
            Some(Face::PosY) => [0.0, 1.0, 0.0],
            Some(Face::NegZ) => [0.0, 0.0, -1.0],
            Some(Face::PosZ) => [0.0, 0.0, 1.0],
            None => [0.0; 3],
        };
        Some((hit.t as f32 / one, normal))
    }

    /// Where a model `extent` voxels across stands for the camera now ([`belt::stand`]), and
    /// the eye it faces. The one place the ghost, and with it every placement of the belt,
    /// stands an entity.
    pub(crate) fn aim_feet(&self, extent: [f32; 3]) -> ([f32; 3], [f32; 3]) {
        let c = &self.fly.camera;
        let (eye, fwd) = (c.position.to_array(), c.basis().0.to_array());
        let feet = belt::stand(eye, fwd, extent, |o, d, max| self.cast_world(o, d, max));
        (feet, eye)
    }

    /// Fills the belt from the library's order, once (`belt::Belt::fill`).
    pub(crate) fn fill_belt(&mut self) {
        let w = &mut self.modes.walkers;
        if w.belt.filled() {
            return;
        }
        let order = w.library.order();
        w.belt.fill(&order);
    }

    /// Entry `entry`'s name (`None` the built-in walker).
    fn entry_name(&self, entry: Option<usize>) -> String {
        let lib = &self.modes.walkers.library;
        entry
            .and_then(|i| lib.entries.get(i))
            .map_or(crate::entities::library::BUILTIN_NAME, |e| &e.name)
            .to_owned()
    }

    /// After the belt's selection changed: the library's selection follows the slot's entry,
    /// and the ghost stands anew.
    fn belt_selected(&mut self) {
        let w = &mut self.modes.walkers;
        if let Some(i) = w.belt.entry().filter(|&i| i < w.library.entries.len()) {
            w.library.selected = i;
        }
        self.sync_ghost();
    }

    /// The chooser's `Enter` on `entry`: the slot that holds it selected, else it goes into
    /// the selected slot; the menu closes and placement mode starts there.
    fn choose(&mut self, entry: usize) {
        self.fill_belt();
        let now = ms_since(self.start);
        let belt = &mut self.modes.walkers.belt;
        let s = match belt.slot_of(entry) {
            Some(s) => s,
            None => {
                belt.assign(belt.selected, entry);
                belt.selected
            }
        };
        belt.select(s, now);
        self.menu.close();
        self.belt_selected();
    }

    /// Ends placement mode: the ghost removed, the turn reset; says `words`.
    pub(crate) fn end_placement(&mut self, words: &str) {
        self.modes.walkers.belt.end();
        self.modes.walkers.clear_preview();
        self.say(words);
    }

    /// The ghost kept in step with the mode: in placement mode with the menu closed the
    /// selected slot's entry stands at [`Run::aim_feet`] facing the camera, turned by the
    /// belt's turn; anywhere else, and for an empty slot, there is none (opening the menu ends
    /// the mode). A refusal (an entry in error, a full set) says its words once.
    pub(crate) fn sync_ghost(&mut self) {
        let w = &mut self.modes.walkers;
        if self.menu.is_open() && w.belt.on {
            w.belt.end();
        }
        let Some(i) = w.belt.entry().filter(|_| w.belt.on) else {
            return w.clear_preview();
        };
        let entry = match w.pick_at(i) {
            Pick::Builtin => None,
            Pick::Rig(i) => Some(i),
            Pick::Refused(why) => {
                w.clear_preview();
                return self.say_once(why);
            }
        };
        let table = self.world.materials();
        let extent = self.modes.walkers.extent_of(entry, table);
        let (feet, eye) = self.aim_feet(extent);
        let yaw = crate::walker::facing(feet, eye) + self.modes.walkers.belt.turn_rad();
        let table = self.world.materials();
        if let Err(words) = self.modes.walkers.show_preview(table, entry, feet, yaw) {
            self.say_once(words);
        }
    }

    /// [`Run::say`] unless the notice already reads `words`.
    fn say_once(&mut self, words: String) {
        if self.notice.as_ref().is_none_or(|(l, _)| *l != words) {
            self.modes.show_lights(words.clone(), ms_since(self.start));
            self.say(words);
        }
    }

    /// Placement mode's click: the ghost placed where it stands under its own id, on its clip
    /// and turn, and the notice names it (`placed spider #3 (rear)`); the mode stays and a new
    /// ghost stands at the aim.
    fn place_ghost(&mut self) {
        self.sync_ghost();
        let w = &mut self.modes.walkers;
        let Some(i) = w.belt.entry() else {
            let words = format!("slot {} empty", w.belt.selected + 1);
            return self.say(words);
        };
        let words = match w.pick_at(i) {
            Pick::Refused(why) => why,
            _ if w.placed.len() >= vxe_render::MAX_ENTITIES => full_words(),
            _ => match w.place_preview() {
                Some(_) => {
                    w.newest.push(true);
                    let p = w.placed.last().expect("just placed");
                    let e = p.entry.and_then(|i| w.library.entries.get(i));
                    let name = e.map_or(crate::entities::library::BUILTIN_NAME, |e| &e.name);
                    let clip = e
                        .and_then(|e| e.viewer())
                        .and_then(|v| p.play.clip_name(v.loaded.as_ref()?));
                    let n = w.placed.len();
                    match clip {
                        Some(c) => format!("placed {name} #{n} ({c})"),
                        None => format!("placed {name} #{n}"),
                    }
                }
                None => "nothing placed".to_owned(),
            },
        };
        self.modes.show_lights(words.clone(), ms_since(self.start));
        self.say(words);
        self.sync_ghost();
    }

    /// The lights' and walkers' actions: a drop at the looked-at surface, a walker placed
    /// there (the library's selected entry), the newest of the two removed, a light fired
    /// along the view ray, the walkers' walking and eye lights, and the aimed (else the
    /// newest) entity's clip cycled or paused. Gives the words the banner shows: the count, the clip or the
    /// pause. A drop or a shot into a full list gives [`lights::FULL_LINE`] and adds nothing.
    fn light_act(&mut self, act: Act) -> Option<String> {
        let lamp = self.modes.lighting.lamp;
        let c = &self.fly.camera;
        let eye = c.position.to_array();
        let fwd = c.basis().0.to_array();
        Some(match act {
            Act::DropLight => {
                let hit = self.surface_along_view(REACH as f32 / FIXED_ONE as f32);
                match self.modes.lights.drop_at(lamp, eye, fwd, hit) {
                    Ok(()) => {
                        self.modes.walkers.newest.push(false);
                        self.modes.lights.count_words(lamp)
                    }
                    Err(line) => line.to_owned(),
                }
            }
            Act::RemoveAimed => self.modes.walkers.remove_aimed(),
            Act::ClipBack | Act::ClipOn | Act::ClipPause
                if self.modes.walkers.belt.on && self.modes.walkers.preview.is_some() =>
            {
                let w = &mut self.modes.walkers;
                match act {
                    Act::ClipBack => w.ghost_clip(-1),
                    Act::ClipOn => w.ghost_clip(1),
                    _ => w.ghost_pause(),
                }
            }
            Act::ClipBack => self.modes.walkers.cycle_clip(-1),
            Act::ClipOn => self.modes.walkers.cycle_clip(1),
            Act::ClipPause => self.modes.walkers.toggle_pause(),
            Act::EyeLights => self.modes.walkers.toggle_eye_lights().to_owned(),
            Act::WalkersWalk => self.modes.walkers.toggle_walk().to_owned(),
            Act::Remove => {
                if self.modes.walkers.newest.pop() == Some(true) {
                    self.modes.walkers.remove_newest();
                    self.modes.walkers.count_words()
                } else {
                    self.modes.lights.remove_newest();
                    self.modes.lights.count_words(lamp)
                }
            }
            Act::FireLight => {
                let hit = self.surface_along_view(lights::SHOT_RANGE).map(|(t, _)| t);
                match self.modes.lights.fire(lamp, eye, fwd, hit) {
                    Ok(()) => {
                        self.modes.walkers.newest.push(false);
                        format!("fire, {}", self.modes.lights.count_words(lamp))
                    }
                    Err(line) => line.to_owned(),
                }
            }
            _ => return None,
        })
    }

    /// The audio's actions, when it runs: a knock, a looping pulse, a looping hum, the echoes
    /// and the tail on or off, and the mix's trims, pre-delay, glide and direct delay, each
    /// change sent to the output and printed.
    fn audio_act(&mut self, act: Act) {
        let Some(audio) = self.audio.as_mut() else {
            return self.say("needs --audio");
        };
        let mut mix = audio.mix;
        match act {
            Act::Knock => {
                let line = audio.play("knock", PlayParams::at);
                return self.say(line);
            }
            Act::Pulse | Act::Hum => {
                let name = if act == Act::Hum { "hum" } else { "pulse" };
                let line = audio.play(name, |p| PlayParams {
                    looping: true,
                    ..PlayParams::at(p)
                });
                return self.say(line);
            }
            Act::Echoes => {
                let on = !audio.acoustics.reflections_on();
                audio.acoustics.set_reflections(on);
                return self.say(format!("echoes {}", if on { "on" } else { "off" }));
            }
            Act::Tail => {
                let on = !audio.acoustics.reverb_on();
                audio.acoustics.set_reverb(on);
                return self.say(format!("tail {}", if on { "on" } else { "off" }));
            }
            Act::EchoTrim(d) => mix = mix.step(3.0 * f32::from(d), 0.0, 0.0),
            Act::TailTrim(d) => mix = mix.step(0.0, 3.0 * f32::from(d), 0.0),
            Act::PreDelay(d) => mix = mix.step(0.0, 0.0, 10.0 * f32::from(d)),
            Act::Glide => mix.glide = !mix.glide,
            Act::DirectDelay => mix.direct_delay = !mix.direct_delay,
            _ => return,
        }
        audio.mix = mix;
        // A full channel drops it; the next key sends the whole balance again.
        if let Err(e) = audio.output.send(Command::SetMix(mix.settings())) {
            println!("mix: {e}");
        }
        self.say(mix.line());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trigger_presses_past_half_and_lets_go_under_three_tenths() {
        let mut held = false;
        let pulls = [0.4, 0.5, 0.9, 0.35, 0.6, 0.25, 0.55];
        let presses = pulls.map(|p| trigger_pressed(&mut held, p));
        assert_eq!(presses, [false, true, false, false, false, false, true]);
    }
}
