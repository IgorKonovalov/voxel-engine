//! The fly-through's model viewer (ADR-0051): `--model <file.rig>` loads a rig and the `.vox`
//! it names, and `` ` `` (and `--entities N`'s ring) places that model in place of the
//! built-in walker, playing a clip at the rig's rate from the frame's time step. The drawn
//! pose is between two ticks ([`vxe_render::DisplayPose::between`]); the set the queries read
//! holds the integer tick's. The two files' modified times are read once a second and a change
//! reloads them, keeping every placed instance; a file that fails keeps the old model and
//! shows the error (`rig line <n>: <message>`) in the banner. The wall clock read here is the
//! testbed's, never a crate's.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use vxe_assets::{Rig, VoxFile};
use vxe_entity::{Clip, Model, Pose, sample};
use vxe_render::ModelHandle;
use vxe_world::{MaterialId, MaterialTable};

/// Seconds between two reads of the files' modified times.
pub const POLL_SECONDS: f64 = 1.0;

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
    /// The playing clip's index in [`Loaded::clips`].
    pub clip: usize,
    /// `--clip`'s name, picked at the first build.
    want_clip: Option<String>,
    pub paused: bool,
    /// Seconds the clip has played.
    pub clock: f64,
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
        let fail = |e: String| format!("--model {}: {e}", path.display());
        let (text, vox_path, bytes) = read_files(path).map_err(fail)?;
        VoxFile::parse(&bytes).map_err(|e| fail(format!("{}: {e}", vox_path.display())))?;
        if let Some(c) = &clip {
            let rig = Rig::parse(&text).map_err(|e| fail(e.to_string()))?;
            if !rig.clips.iter().any(|r| &r.name == c) {
                return Err(fail(format!("no clip `{c}`")));
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
            clip: 0,
            want_clip: clip,
            paused: false,
            clock: 0.0,
            polled_at: None,
        })
    }

    /// The reload step: builds the rig `rig_text` over the `.vox` `vox_bytes` against `table`
    /// and, when it builds, swaps the model in (the playing clip kept by name, else the first)
    /// and drops the uploaded handle. On an error the old model stays and the banner's words
    /// come back (`rig line <n>: <message>`).
    pub fn reload(
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
        self.clip = want
            .and_then(|w| clips.iter().position(|(n, _)| *n == w))
            .unwrap_or(0);
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
        self.handle = None;
        Ok(())
    }

    /// Builds the files [`Viewer::open`] read, once: the banner's words for an error.
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
            Err(e) => Err(e),
        };
        Some(words.map_or_else(|e| e, |()| "reloaded".to_owned()))
    }

    /// The playing clip's name, once built and when the rig has a clip.
    pub fn clip_name(&self) -> Option<&str> {
        let l = self.loaded.as_ref()?;
        l.clips.get(self.clip).map(|(n, _)| n.as_str())
    }

    /// Advances the clip's clock by `dt` seconds unless paused.
    pub fn advance(&mut self, dt: f32) {
        if !self.paused {
            self.clock += f64::from(dt);
        }
    }

    /// The clip's tick now, unwrapped, as a real number: the clock times the rate.
    fn tick_f(&self) -> f64 {
        let rate = self.loaded.as_ref().map_or(0, |l| l.rate);
        self.clock * f64::from(rate)
    }

    /// The pose at the integer tick now and at the next, and the fraction between them, the
    /// root at the origin: the rest pose with no clip. `None` before a build.
    pub fn poses(&self) -> Option<(Pose, Pose, f32)> {
        let l = self.loaded.as_ref()?;
        let Some((_, clip)) = l.clips.get(self.clip) else {
            return Some((l.rest.clone(), l.rest.clone(), 0.0));
        };
        let t = self.tick_f();
        let tick = t.floor() as u64;
        let alpha = (t - t.floor()) as f32;
        Some((
            sample(&l.model, &l.rest, clip, tick),
            sample(&l.model, &l.rest, clip, tick + 1),
            alpha,
        ))
    }

    /// `step` clips on from the playing one (wrapping), from its tick 0: the banner's words.
    pub fn cycle(&mut self, step: i32) -> String {
        let n = self.loaded.as_ref().map_or(0, |l| l.clips.len());
        if n == 0 {
            return format!("model {}: no clip", self.name);
        }
        self.clip = (self.clip as i64 + i64::from(step)).rem_euclid(n as i64) as usize;
        self.clock = 0.0;
        format!("clip {}", self.clip_name().unwrap_or(""))
    }

    /// Pauses or resumes the clip: the banner's words.
    pub fn toggle_pause(&mut self) -> &'static str {
        self.paused = !self.paused;
        if self.paused { "paused" } else { "playing" }
    }

    /// The HUD row: `model <name> clip <clip> <tick>/<length>`, then ` paused` while paused;
    /// `model <name> not built` before a build succeeds.
    pub fn write_row(&self, out: &mut String) {
        use std::fmt::Write as _;
        let _ = write!(out, "model {}", self.name);
        let Some(l) = &self.loaded else {
            out.push_str(" not built");
            return;
        };
        match l.clips.get(self.clip) {
            Some((name, clip)) => {
                let tick = clip.clip_tick(self.tick_f().floor() as u64);
                let _ = write!(out, " clip {name} {tick}/{}", clip.length());
            }
            None => out.push_str(" no clip"),
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
    p.root = vxe_world::Fixed3(feet.map(|v| (v * 256.0).round() as i64));
    p.root_q = crate::walker::turn(2, yaw);
    p
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
}
