//! The fly-through's model viewer (ADR-0051): `--model <file.rig>` loads a rig and the `.vox`
//! it names, and `` ` `` (and `--entities N`'s ring) places that model in place of the
//! built-in walker, playing a clip at the rig's rate from the frame's time step. The drawn
//! pose is between two ticks ([`vxe_render::DisplayPose::between`]); the set the queries read
//! holds the integer tick's. A change of clip fades into the new one over [`CLIP_FADE_TICKS`]
//! ([`vxe_entity::Player`]). The two files' modified times are read once a second and a change
//! reloads them, keeping every placed instance; a file that fails keeps the old model and
//! shows the error (`rig line <n>: <message>`) in the banner, and keeps it on screen in red
//! (the HUD's row, or a banner row with the HUD off) until a reload succeeds. The wall clock
//! read here is the testbed's, never a crate's.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use vxe_assets::{Rig, VoxFile};
use vxe_entity::{Clip, Model, Playback, Player, Pose, blend};
use vxe_render::ModelHandle;
use vxe_world::{MaterialId, MaterialTable};

/// Seconds between two reads of the files' modified times.
pub const POLL_SECONDS: f64 = 1.0;

/// The fade from one clip into the next, in the rig's ticks: about a quarter second at the
/// sample walker's 30 a second.
pub const CLIP_FADE_TICKS: u32 = 8;

/// Whether `line` is the viewer's error row ([`Viewer::write_row`] while an error stands),
/// which the HUD and the banner draw in red.
pub fn is_error_row(line: &str) -> bool {
    line.starts_with("model ") && line.contains(" error: ")
}

/// A built rig: its model and its parts' names, its rest pose, its clips by name and its ticks per second.
#[derive(Clone, Debug)]
pub struct Loaded {
    pub model: Arc<Model>,
    /// The parts' names in the model's order.
    pub part_names: Vec<String>,
    pub rest: Pose,
    pub clips: Vec<(String, Arc<Clip>)>,
    pub rate: u32,
}

/// The viewer: the rig's path, the model built from it, the clip playing and its clock, and
/// the files' modified times when last read.
#[derive(Clone, Debug)]
pub struct Viewer {
    pub rig_path: PathBuf,
    /// The rig file's stem, the HUD row's model name.
    pub name: String,
    /// The `.vox` the rig names, beside it.
    vox_path: PathBuf,
    /// The files read at [`Viewer::open`], built against the world's table on first use.
    pending: Option<(String, Vec<u8>)>,
    pub loaded: Option<Loaded>,
    /// The model uploaded to the frame's entity mirror; `None` until uploaded and after a
    /// reload.
    pub handle: Option<ModelHandle>,
    /// The handle a reload replaced, for [`crate::entities::sync_models`] to free once the new
    /// model is uploaded.
    pub stale: Option<ModelHandle>,
    /// The viewer's own playback: the clip the HUD row names, the one a new placement and
    /// the picker's preview start on.
    pub play: Play,
    /// `--clip`'s name, picked at the first build.
    want_clip: Option<String>,
    /// The last build's or reload's error, kept until one succeeds.
    pub error: Option<String>,
    stamps: [Option<SystemTime>; 2],
    polled_at: Option<f64>,
}

/// `name` resolved against `table`'s rows: a row of that name, or of that name with each `_`
/// read as a space (a rig's tokens hold no space; the walker's eyes are `walker eye`).
pub fn material(table: &MaterialTable, name: &str) -> Option<MaterialId> {
    let spaced = name.replace('_', " ");
    table
        .rows()
        .iter()
        .position(|m| m.name == name || m.name == spaced)
        .and_then(|i| MaterialId::try_from(i).ok())
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// The rig at `rig_path` and the `.vox` it names: the rig's text, the `.vox`'s path and bytes.
fn read_files(rig_path: &Path) -> Result<(String, PathBuf, Vec<u8>), String> {
    let text =
        std::fs::read_to_string(rig_path).map_err(|e| format!("{}: {e}", rig_path.display()))?;
    let rig = Rig::parse(&text).map_err(|e| e.to_string())?;
    let vox_path = rig_path.parent().unwrap_or(Path::new(".")).join(&rig.vox);
    let bytes = std::fs::read(&vox_path).map_err(|e| format!("{}: {e}", vox_path.display()))?;
    Ok((text, vox_path, bytes))
}

impl Viewer {
    /// Reads and parses the rig at `path` and its `.vox`, with `clip` (`--clip`) the clip to
    /// play first. A file that does not read or parse, or a clip the rig lacks, is an error
    /// that names the path. The model is built on first use, against the world's table
    /// ([`Viewer::build_pending`]).
    pub fn open(path: &Path, clip: Option<String>) -> Result<Self, String> {
        Self::load(path, clip).map_err(|e| format!("--model {}: {e}", path.display()))
    }

    /// [`Viewer::open`] with the error as the files give it (`rig line 3: ...`), not
    /// prefixed with the flag: a library entry's (`crate::entities::library`).
    pub fn load(path: &Path, clip: Option<String>) -> Result<Self, String> {
        let (text, vox_path, bytes) = read_files(path)?;
        VoxFile::parse(&bytes).map_err(|e| format!("{}: {e}", vox_path.display()))?;
        if let Some(c) = &clip {
            let rig = Rig::parse(&text).map_err(|e| e.to_string())?;
            if !rig.clips.iter().any(|r| &r.name == c) {
                return Err(format!("no clip `{c}`"));
            }
        }
        let name = path
            .file_stem()
            .map_or_else(|| "model".to_owned(), |s| s.to_string_lossy().into_owned());
        Ok(Self {
            rig_path: path.to_path_buf(),
            name,
            stamps: [modified(path), modified(&vox_path)],
            vox_path,
            pending: Some((text, bytes)),
            loaded: None,
            handle: None,
            stale: None,
            play: Play::default(),
            want_clip: clip,
            error: None,
            polled_at: None,
        })
    }

    /// The reload step: builds the rig `rig_text` over the `.vox` `vox_bytes` against `table`
    /// and, when it builds, swaps the model in (the playing clip kept by name, else the first,
    /// with no fade) and moves the uploaded handle to [`Viewer::stale`]. On an error the old
    /// model stays, the error is kept ([`Viewer::error`]) and the banner's words come back
    /// (`rig line <n>: <message>`); a build clears the kept error.
    pub fn reload(
        &mut self,
        rig_text: &str,
        vox_bytes: &[u8],
        table: &MaterialTable,
    ) -> Result<(), String> {
        let r = self.try_reload(rig_text, vox_bytes, table);
        self.keep(r)
    }

    /// Keeps `r`'s error, or clears the kept one on `Ok`.
    fn keep(&mut self, r: Result<(), String>) -> Result<(), String> {
        self.error = r.as_ref().err().cloned();
        r
    }

    fn try_reload(
        &mut self,
        rig_text: &str,
        vox_bytes: &[u8],
        table: &MaterialTable,
    ) -> Result<(), String> {
        let rig = Rig::parse(rig_text).map_err(|e| e.to_string())?;
        let vox = VoxFile::parse(vox_bytes).map_err(|e| format!("{}: {e}", rig.vox))?;
        let built = rig
            .build(&vox, &|n| material(table, n))
            .map_err(|e| e.to_string())?;
        let playing = self.clip_name().map(str::to_owned);
        // Built: `--clip`'s pick is spent.
        let want = self.want_clip.take().or(playing);
        let clips: Vec<(String, Arc<Clip>)> = built
            .clips
            .into_iter()
            .map(|(n, c)| (n, Arc::new(c)))
            .collect();
        self.vox_path = self
            .rig_path
            .parent()
            .unwrap_or(Path::new("."))
            .join(&rig.vox);
        self.loaded = Some(Loaded {
            model: Arc::new(built.model),
            part_names: built.part_names,
            rest: built.rest,
            clips,
            rate: rig.rate,
        });
        let l = self.loaded.as_ref().expect("just built");
        self.play.rebind(want.as_deref(), l);
        // Between two uploads `stale` holds at most one handle: an upload frees it, and
        // `handle` is `None` until the next upload.
        if let Some(h) = self.handle.take() {
            self.stale = Some(h);
        }
        Ok(())
    }

    /// Builds the files [`Viewer::open`] read, once: the banner's words for an error, which
    /// stands until a reload builds.
    pub fn build_pending(&mut self, table: &MaterialTable) -> Option<String> {
        let (text, bytes) = self.pending.take()?;
        self.reload(&text, &bytes, table).err()
    }

    /// Once [`POLL_SECONDS`] has passed since the last read (`now` in seconds): reads the two
    /// files' modified times and reloads on a change. `Some` with the banner's words
    /// (`reloaded`, or the error) when it tried.
    pub fn poll(&mut self, now: f64, table: &MaterialTable) -> Option<String> {
        if self.polled_at.is_some_and(|t| now - t < POLL_SECONDS) {
            return None;
        }
        self.polled_at = Some(now);
        let stamps = [modified(&self.rig_path), modified(&self.vox_path)];
        if stamps == self.stamps {
            return None;
        }
        self.stamps = stamps;
        let words = match read_files(&self.rig_path) {
            Ok((text, vox_path, bytes)) => {
                self.vox_path = vox_path;
                self.stamps[1] = modified(&self.vox_path);
                self.reload(&text, &bytes, table)
            }
            Err(e) => self.keep(Err(e)),
        };
        Some(words.map_or_else(|e| e, |()| "reloaded".to_owned()))
    }

    /// The playing clip's name, once built and when the rig has a clip.
    pub fn clip_name(&self) -> Option<&str> {
        self.play.clip_name(self.loaded.as_ref()?)
    }

    /// Advances the clip by `dt` seconds at the rig's rate unless paused.
    pub fn advance(&mut self, dt: f32) {
        let rate = self.loaded.as_ref().map_or(0, |l| l.rate);
        self.play.advance(dt, rate);
    }

    /// The pose at the integer tick now and at the next, and the fraction between them, the
    /// root at the origin ([`Play::poses`]). `None` before a build.
    pub fn poses(&self) -> Option<(Pose, Pose, f32)> {
        Some(self.play.poses(self.loaded.as_ref()?))
    }

    /// `step` clips on from the playing one ([`Play::cycle`]): the banner's words.
    pub fn cycle(&mut self, step: i32) -> String {
        match &self.loaded {
            Some(l) if self.play.cycle(l, step) => {
                format!("clip {}", self.clip_name().unwrap_or(""))
            }
            _ => format!("model {}: no clip", self.name),
        }
    }

    /// Pauses or resumes the clip: the banner's words.
    pub fn toggle_pause(&mut self) -> &'static str {
        self.play.toggle_pause()
    }

    /// Whether the last build or reload failed with no model built: an entry that cannot be
    /// placed.
    pub fn failed(&self) -> Option<&str> {
        match (&self.loaded, &self.error) {
            (None, Some(e)) => Some(e),
            _ => None,
        }
    }

    /// The HUD row: `model <name> clip <clip> <tick>/<length>`, then ` paused` while paused;
    /// `model <name> not built` before a build succeeds; `model <name> error: <error>` while
    /// the last build's or reload's error stands ([`is_error_row`]).
    pub fn write_row(&self, out: &mut String) {
        use std::fmt::Write as _;
        let _ = write!(out, "model {}", self.name);
        if let Some(e) = &self.error {
            let _ = write!(out, " error: {e}");
            return;
        }
        let Some(l) = &self.loaded else {
            out.push_str(" not built");
            return;
        };
        self.play.write(l, out);
    }
}

/// One playback of a built rig's clips: the clip's index in [`Loaded::clips`], the player
/// (the clip and the fade into it, in this playback's ticks; `None` with no clip), the pause,
/// and the rig's ticks played, as a real number: the seconds played times the rate. A clip
/// change snaps the ticks down to a whole tick. The viewer has one; each placed instance of a
/// rig has its own (`crate::entities::Placed`).
#[derive(Clone, Debug, Default)]
pub struct Play {
    pub clip: usize,
    pub player: Option<Player>,
    pub paused: bool,
    pub ticks: f64,
}

impl Play {
    /// `l`'s clip `clip` (the first when it has no such clip) from its tick 0, no fade.
    pub fn start(l: &Loaded, clip: usize) -> Self {
        let clip = if clip < l.clips.len() { clip } else { 0 };
        Self {
            clip,
            player: l.clips.get(clip).map(|(_, c)| {
                Player::new(Playback {
                    clip: c.clone(),
                    start_tick: 0,
                    rate: 256,
                })
            }),
            paused: false,
            ticks: 0.0,
        }
    }

    /// Onto `l` after a reload: the clip named `want` (else the first), going on from the
    /// tick it had reached; a fade from the old model's pose is dropped, since the new model
    /// may have other parts.
    pub fn rebind(&mut self, want: Option<&str>, l: &Loaded) {
        self.clip = want
            .and_then(|w| l.clips.iter().position(|(n, _)| n == w))
            .unwrap_or(0);
        let start_tick = self.player.as_ref().map_or(0, |p| p.playback.start_tick);
        self.player = l.clips.get(self.clip).map(|(_, c)| {
            Player::new(Playback {
                clip: c.clone(),
                start_tick,
                rate: 256,
            })
        });
    }

    /// The playing clip's name in `l`.
    pub fn clip_name<'a>(&self, l: &'a Loaded) -> Option<&'a str> {
        l.clips.get(self.clip).map(|(n, _)| n.as_str())
    }

    /// Advances by `dt` seconds at `rate` ticks a second unless paused.
    pub fn advance(&mut self, dt: f32, rate: u32) {
        if !self.paused {
            self.ticks += f64::from(dt) * f64::from(rate);
        }
    }

    /// The whole tick now and the fraction past it.
    fn tick_now(&self) -> (u64, f32) {
        let t = self.ticks.max(0.0);
        (t.floor() as u64, (t - t.floor()) as f32)
    }

    /// The pose at the integer tick now and at the next, and the fraction between them, the
    /// root at the origin, each faded from the last clip's while a fade is under way: the rest
    /// pose with no clip.
    pub fn poses(&self, l: &Loaded) -> (Pose, Pose, f32) {
        let Some(p) = &self.player else {
            return (l.rest.clone(), l.rest.clone(), 0.0);
        };
        let (tick, alpha) = self.tick_now();
        (
            p.pose(&l.model, &l.rest, tick),
            p.pose(&l.model, &l.rest, tick + 1),
            alpha,
        )
    }

    /// `step` clips on from the playing one (wrapping), from its tick 0, faded into over
    /// [`CLIP_FADE_TICKS`] from the pose drawn now; `false` when `l` has no clip. The clock
    /// snaps down to the whole tick, so the fade starts from exactly the drawn pose (its blend
    /// between the two ticks, in fixed point) at the cost of under a tick of time.
    pub fn cycle(&mut self, l: &Loaded, step: i32) -> bool {
        let n = l.clips.len();
        if n == 0 {
            return false;
        }
        let drawn = self.poses(l);
        self.clip = (self.clip as i64 + i64::from(step)).rem_euclid(n as i64) as usize;
        let (tick, _) = self.tick_now();
        let clip = l.clips[self.clip].1.clone();
        match &mut self.player {
            Some(p) => {
                let (a, b, alpha) = drawn;
                let from = blend(&a, &b, (alpha * 256.0).round() as u16);
                p.switch_from(from, tick, clip, CLIP_FADE_TICKS);
            }
            p => {
                *p = Some(Player::new(Playback {
                    clip,
                    start_tick: tick,
                    rate: 256,
                }));
            }
        }
        self.ticks = tick as f64;
        true
    }

    /// Pauses or resumes: the banner's words.
    pub fn toggle_pause(&mut self) -> &'static str {
        self.paused = !self.paused;
        if self.paused { "paused" } else { "playing" }
    }

    /// The tick within the playing clip and its length.
    pub fn clip_tick(&self, l: &Loaded) -> Option<(u32, u32)> {
        let ((_, clip), p) = (l.clips.get(self.clip)?, self.player.as_ref()?);
        Some((
            clip.clip_tick(p.playback.tick_at(self.tick_now().0)),
            clip.length(),
        ))
    }

    /// ` clip <clip> <tick>/<length>`, then ` paused` while paused; ` no clip` without one.
    pub fn write(&self, l: &Loaded, out: &mut String) {
        use std::fmt::Write as _;
        match (self.clip_name(l), self.clip_tick(l)) {
            (Some(name), Some((tick, len))) => {
                let _ = write!(out, " clip {name} {tick}/{len}");
            }
            _ => out.push_str(" no clip"),
        }
        if self.paused {
            out.push_str(" paused");
        }
    }
}

/// `pose` (root at the origin) stood with its root at `feet` (world voxels), turned `yaw`
/// radians about z.
pub fn placed(pose: &Pose, feet: [f32; 3], yaw: f32) -> Pose {
    let mut p = pose.clone();
    place_into(&mut p, pose, feet, yaw);
    p
}

/// [`placed`] into `out`, reusing its parts' storage.
pub fn place_into(out: &mut Pose, pose: &Pose, feet: [f32; 3], yaw: f32) {
    out.parts.clone_from(&pose.parts);
    out.root = vxe_world::Fixed3(feet.map(|v| (v * 256.0).round() as i64));
    out.root_q = crate::walker::turn(2, yaw);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_material_name_reads_underscores_as_spaces() {
        let t = crate::walker::with_walker(&crate::cave_materials());
        let (body, eye) = crate::walker::rows(&t).expect("the walker's rows");
        assert_eq!(material(&t, "walker"), Some(body));
        assert_eq!(material(&t, "walker_eye"), Some(eye));
        assert_eq!(material(&t, "walker eye"), Some(eye));
        assert_eq!(material(&t, "nothing"), None);
    }

    /// A refused reload stays on screen until a reload builds. The sample walker saved with line 9
    /// broken: the viewer keeps the error; with the HUD hidden the banner's first row is
    /// `model walker error: rig line 9: ...` in red, and still is once the banner's time is up;
    /// with the HUD shown the viewer's mode row reads the same, in red, and the banner holds no
    /// copy. A fixed save clears it: the banner says `reloaded`, the row names the clip again
    /// and no row is red.
    #[test]
    fn a_refused_reload_stays_on_screen_until_one_builds() {
        use crate::cave::ROCK;
        use crate::now;
        use crate::run::hud_feed::banner_colours;
        use crate::test_support::bare_run;
        use std::time::{Duration, SystemTime};
        use vxe_render::HudColour;

        let assets =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/entities/walker");
        let dir = std::env::temp_dir().join(format!("vxe-viewer-error-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let rig = std::fs::read_to_string(assets.join("walker.rig")).expect("rig reads");
        std::fs::copy(assets.join("walker.vox"), dir.join("walker.vox")).expect("vox copied");
        // Each save stamped `secs` after the epoch, so each is a change the poll sees.
        let save = |text: &str, secs: u64| {
            std::fs::write(dir.join("walker.rig"), text).expect("saved");
            std::fs::File::options()
                .write(true)
                .open(dir.join("walker.rig"))
                .expect("opens")
                .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
                .expect("stamped");
        };
        save(&rig, 5);
        let table = crate::walker::with_walker(&crate::cave_materials());
        let world = vxe_world::World::generate(
            vxe_world::ChunkExtent::new([-1, -1, -1], [1, 1, 1]),
            &vxe_world::UniformSource(ROCK),
            &table,
        );
        let mut run = bare_run(world);
        let viewer = Viewer::open(&dir.join("walker.rig"), None).expect("opens");
        run.modes.walkers.viewer = Some(viewer);
        run.poll_viewer(0.0);
        let v = run.modes.walkers.viewer.as_ref().expect("viewer");
        assert!(v.loaded.is_some() && v.error.is_none(), "{:?}", v.error);

        let broken = {
            let mut lines: Vec<&str> = rig.lines().collect();
            assert!(lines[8].starts_with("material 2"), "{}", lines[8]);
            lines[8] = "material 2";
            lines.join("\n") + "\n"
        };
        save(&broken, 10);
        run.start = now();
        run.poll_viewer(1000.0);
        let e = run.modes.walkers.viewer.as_ref().unwrap().error.clone();
        let e = e.expect("the refusal is kept");
        assert!(e.starts_with("rig line 9: "), "{e}");
        let error_row = format!("model walker error: {e}");

        // The HUD hidden: the banner's first row, red; the other rows yellow.
        run.hud.visible = false;
        let mut colours = Vec::new();
        let banner = run.banner().expect("the banner");
        assert_eq!(banner.lines().next(), Some(error_row.as_str()), "{banner}");
        banner_colours(&banner, &mut colours);
        assert_eq!(colours[0], HudColour::Red);
        assert!(colours[1..].iter().all(|&c| c == HudColour::Yellow));
        // Long after the banner's time the error row stands, and the transient words are gone.
        run.start = now() - Duration::from_secs(60);
        let banner = run.banner().expect("the banner");
        assert_eq!(banner.lines().next(), Some(error_row.as_str()), "{banner}");
        assert_eq!(banner.matches("rig line 9").count(), 1, "{banner}");

        // The HUD shown: the viewer's mode row in red, no copy in the banner.
        run.hud.visible = true;
        assert!(!run.banner().unwrap().contains("rig line 9"));
        run.fill_hud(None);
        let red: Vec<&str> = run
            .hud
            .text
            .lines()
            .zip(&run.hud.colours)
            .filter(|&(_, &c)| c == HudColour::Red)
            .map(|(l, _)| l)
            .collect();
        assert_eq!(red, [error_row.as_str()], "{}", run.hud.text);

        // A fixed save builds: `reloaded`, the clip's row again, nothing red.
        save(&rig, 20);
        run.start = now();
        run.poll_viewer(2000.0);
        let v = run.modes.walkers.viewer.as_ref().unwrap();
        assert_eq!(v.error, None);
        let banner = run.banner().expect("the banner");
        assert!(banner.lines().any(|l| l == "reloaded"), "{banner}");
        assert!(!banner.contains("error"), "{banner}");
        run.fill_hud(None);
        assert!(
            run.hud
                .text
                .lines()
                .any(|l| l == "model walker clip idle 0/60"),
            "{}",
            run.hud.text
        );
        assert!(!run.hud.colours.contains(&HudColour::Red));
        run.hud.visible = false;
        let banner = run.banner().expect("the banner");
        banner_colours(&banner, &mut colours);
        assert!(colours.iter().all(|&c| c == HudColour::Yellow), "{banner}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
