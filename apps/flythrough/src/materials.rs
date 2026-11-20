//! The fly-through's material table as a file (ADR-0065): `assets/materials.txt`, or
//! `--materials PATH`, read at the start and again once a second while the window is open.
//!
//! Each scene takes the file's rows by name in its own order ([`Selection`]), so the ids the
//! scenes have always used hold: the cave's four rows (the showcase's own rock and ore in
//! place of rock and ore), then the showcase's, the castle's, the outdoor hills' or the rigs'
//! wood. A reload rewrites the GPU rows' fields and re-bakes the chunks holding a
//! row whose emission moved; a reload that adds, removes or renames a row, or changes a row's
//! flags or whether it emits, takes effect at the next start. The wall clock read here is the
//! testbed's, never a crate's.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use vxe_assets::MaterialFile;
use vxe_world::{MaterialId, MaterialTable};

use crate::setup::CrispRows;

/// The file the fly-through reads unless `--materials` names another.
pub const DEFAULT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/materials.txt");

/// The default file as built in, for a binary run away from its source tree: read when
/// [`DEFAULT_PATH`] is missing, and then not watched.
pub const BUILT_IN: &str = include_str!("../assets/materials.txt");

/// The cave's rows, ids 1 to 4.
pub const CAVE_ROWS: [&str; 4] = ["rock", "ore", "shell", "rubble"];
/// The showcase's rows at the cave's ids 1 to 4: a pale rock and a dimmer ore.
pub const SHOWCASE_BASE_ROWS: [&str; 4] = ["showcase rock", "showcase ore", "shell", "rubble"];
/// The showcase's glowing rows after its base rows (`showcase::CRYSTAL` on).
pub const SHOWCASE_ROWS: [&str; 7] = [
    "crystal", "amethyst", "ember", "jade", "citrine", "rose", "sapphire",
];
/// The castle's rows after the cave's, in its palette's order.
pub const CASTLE_ROWS: [&str; 7] = [
    "castle stone",
    "castle trim",
    "castle slate",
    "castle wood",
    "castle iron",
    "castle glass",
    "castle torch",
];
/// The outdoor hills' rows after the cave's (`crate::outdoor`'s ids).
pub const OUTDOOR_ROWS: [&str; 6] = ["grass", "soil", "bark", "leaf", "leaf 2", "leaf 3"];
/// The thin and seam rigs' row after the cave's.
pub const RIG_ROWS: [&str; 1] = ["rig wood"];

/// Seconds between two reads of the file's modified time.
pub const POLL_SECONDS: f64 = 1.0;

/// The banner's words for a reload that waits for a restart.
pub const RESTART_WORDS: &str = "materials: shape changed, restart";

/// Which rows of the file a scene's table takes, in order, and what it changes on them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub names: Vec<&'static str>,
    /// Rows after `names` that `crisp` leaves alone (the showcase's).
    pub after_crisp: Vec<&'static str>,
    /// `--emissive-ore`: off clears the ore row's emission.
    pub emissive_ore: bool,
    /// `--crisp`'s rows.
    pub crisp: CrispRows,
}

impl Selection {
    /// The cave's rows followed by `extra`, then `after_crisp`.
    pub fn cave_and(
        extra: &[&'static str],
        after_crisp: &[&'static str],
        emissive_ore: bool,
        crisp: CrispRows,
    ) -> Self {
        Self::base_and(&CAVE_ROWS, extra, after_crisp, emissive_ore, crisp)
    }

    /// `base` (the four rows at the cave's ids, the ore's second, each named in the table by
    /// the cave's name, so a rig's `rock` is the scene's rock) followed by `extra`, then
    /// `after_crisp`.
    pub fn base_and(
        base: &[&'static str; 4],
        extra: &[&'static str],
        after_crisp: &[&'static str],
        emissive_ore: bool,
        crisp: CrispRows,
    ) -> Self {
        let mut names = base.to_vec();
        names.extend_from_slice(extra);
        Self {
            names,
            after_crisp: after_crisp.to_vec(),
            emissive_ore,
            crisp,
        }
    }

    /// The selected rows as a table: air, then each name's row in the selection's order. A
    /// name the file lacks is an error naming it. The first four rows take [`CAVE_ROWS`]'
    /// names; `emissive_ore` off clears the emission of the row at the ore's id (the second).
    pub fn compose(&self, file: &MaterialFile) -> Result<MaterialTable, String> {
        let row = |n: &str, slot: Option<usize>| {
            let (_, row) = file
                .row(n)
                .ok_or_else(|| format!("materials: no row `{n}`, which the scene takes"))?;
            let mut row = row.clone();
            if let Some(i) = slot {
                row.name = CAVE_ROWS[i].to_owned();
            }
            if slot == Some(1) && !self.emissive_ore {
                row.emission = [0.0; 3];
            }
            Ok::<_, String>(row)
        };
        let mut t = MaterialTable::new();
        for (i, &n) in self.names.iter().enumerate() {
            t.push(row(n, (i < CAVE_ROWS.len()).then_some(i))?);
        }
        let mut t = self.crisp.apply(&t);
        for &n in &self.after_crisp {
            t.push(row(n, None)?);
        }
        Ok(t)
    }
}

/// Reads the material file: `path` when given (an error if it does not read or parse), else
/// [`DEFAULT_PATH`], or [`BUILT_IN`] when that is missing. Returns the file and the path to
/// watch (`None` for the built-in copy).
pub fn load(path: Option<&Path>) -> Result<(MaterialFile, Option<PathBuf>), String> {
    let (text, watched) = match path {
        Some(p) => (
            std::fs::read_to_string(p).map_err(|e| format!("--materials {}: {e}", p.display()))?,
            Some(p.to_path_buf()),
        ),
        None => match std::fs::read_to_string(DEFAULT_PATH) {
            Ok(t) => (t, Some(PathBuf::from(DEFAULT_PATH))),
            Err(_) => (BUILT_IN.to_owned(), None),
        },
    };
    let shown = watched.as_ref().map_or_else(
        || "the built-in materials".to_owned(),
        |p| p.display().to_string(),
    );
    let file = MaterialFile::parse(&text).map_err(|e| format!("{shown}: {e}"))?;
    Ok((file, watched))
}

/// What one reload of the file does.
#[derive(Clone, Debug, PartialEq)]
pub enum Reload {
    /// The running table with the scene's rows replaced: the same row count, the same flags
    /// and the same emitting rows. `changed` lists the rows whose fields moved and `emission`
    /// those whose emission moved, ascending.
    Rewrite {
        table: MaterialTable,
        changed: Vec<MaterialId>,
        emission: Vec<MaterialId>,
    },
    /// Rows added, removed, renamed, or a row's flags or emitting changed: nothing is
    /// rewritten until a restart.
    Restart,
    /// The file does not parse, or lacks a row the scene takes: the error with its line.
    Error(String),
}

impl Reload {
    /// The banner's words.
    pub fn words(&self) -> String {
        match self {
            Self::Rewrite { .. } => "materials reloaded".to_owned(),
            Self::Restart => RESTART_WORDS.to_owned(),
            Self::Error(e) => e.clone(),
        }
    }
}

/// Whether a row emits: the bake's emitter lists are built from it at the start.
fn emits(m: &vxe_world::Material) -> bool {
    m.emission.iter().any(|&e| e != 0.0)
}

/// The reload step, without the clock: `text` read against the file the world started with
/// (`start`, for its row names), the scene's `selection` and the `running` table (the scene's
/// rows first, then the rows the start added after them: the walker's and the palettes').
pub fn reload(
    start: &MaterialFile,
    selection: &Selection,
    running: &MaterialTable,
    text: &str,
) -> Reload {
    let file = match MaterialFile::parse(text) {
        Ok(f) => f,
        Err(e) => return Reload::Error(e.to_string()),
    };
    let names = |f: &MaterialFile| f.rows.iter().map(|m| m.name.clone()).collect::<Vec<_>>();
    if names(&file) != names(start) {
        return Reload::Restart;
    }
    let scene = match selection.compose(&file) {
        Ok(t) => t,
        Err(e) => return Reload::Error(e),
    };
    let mut table = scene.clone();
    for row in running.rows().iter().skip(scene.len()) {
        table.push(row.clone());
    }
    if table.len() != running.len() {
        return Reload::Restart;
    }
    let mut changed = Vec::new();
    let mut emission = Vec::new();
    for (i, (new, old)) in table.rows().iter().zip(running.rows()).enumerate() {
        if new.flags != old.flags || emits(new) != emits(old) {
            return Reload::Restart;
        }
        let id = i as MaterialId;
        if new != old {
            changed.push(id);
        }
        if new.emission != old.emission {
            emission.push(id);
        }
    }
    Reload::Rewrite {
        table,
        changed,
        emission,
    }
}

/// The file watched while the window is open: its path, the file and selection the world
/// started with, its modified time when last read and when it was last polled.
#[derive(Clone, Debug)]
pub struct Watch {
    pub path: PathBuf,
    pub start: MaterialFile,
    pub selection: Selection,
    /// The table the last rewrite wrote, which the next reload compares against; `None`
    /// before one, when the world's own table stands.
    pub current: Option<MaterialTable>,
    stamp: Option<SystemTime>,
    polled_at: Option<f64>,
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

impl Watch {
    /// Watches `path`, whose contents the world started from as `start`.
    pub fn new(path: PathBuf, start: MaterialFile, selection: Selection) -> Self {
        Self {
            stamp: modified(&path),
            path,
            start,
            selection,
            current: None,
            polled_at: None,
        }
    }

    /// Once [`POLL_SECONDS`] has passed since the last read (`now` in seconds): reads the
    /// file's modified time and, on a change, the file, through [`reload`] against the last
    /// rewrite's table, or `running` (the world's) before one.
    pub fn poll(&mut self, now: f64, running: &MaterialTable) -> Option<Reload> {
        if self.polled_at.is_some_and(|t| now - t < POLL_SECONDS) {
            return None;
        }
        self.polled_at = Some(now);
        let stamp = modified(&self.path);
        if stamp == self.stamp {
            return None;
        }
        self.stamp = stamp;
        let r = match std::fs::read_to_string(&self.path) {
            Ok(text) => {
                let against = self.current.as_ref().unwrap_or(running);
                reload(&self.start, &self.selection, against, &text)
            }
            Err(e) => Reload::Error(format!("materials: {}: {e}", self.path.display())),
        };
        if let Reload::Rewrite { table, .. } = &r {
            self.current = Some(table.clone());
        }
        Some(r)
    }
}

/// Appends to `table` the row of `file` named by each of `names` that `table` has no row of,
/// in `names`' order, each once; a name `file` lacks is passed over. The rows appended.
pub fn append_named<'a>(
    table: &mut MaterialTable,
    file: &MaterialFile,
    names: impl IntoIterator<Item = &'a str>,
) -> usize {
    let mut added = 0;
    for n in names {
        let held =
            (1..table.len()).any(|i| table.get(i as MaterialId).is_some_and(|m| m.name == n));
        if !held && let Some((_, row)) = file.row(n) {
            table.push(row.clone());
            added += 1;
        }
    }
    added
}

/// The chunks of `world` resident now that hold a voxel of a row in `rows`, ascending.
pub fn chunks_holding(world: &vxe_world::World, rows: &[MaterialId]) -> Vec<vxe_world::ChunkCoord> {
    use vxe_world::Brick;
    let mut out = Vec::new();
    for c in world.store().coords() {
        let Some(chunk) = world.store().get(c) else {
            continue;
        };
        let chunk = chunk
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let holds = chunk.bricks().iter().any(|b| match b {
            Brick::Uniform(m) => rows.contains(m),
            Brick::Full(p) => match p.wide_ids() {
                Some(ids) => ids.iter().any(|m| rows.contains(m)),
                None => p.palette().iter().any(|m| rows.contains(m)),
            },
        });
        if holds {
            out.push(c);
        }
    }
    out
}

/// The selection `args` names: the cave's rows, then the castle's under `--castle`, the rigs'
/// wood under the thin and seam rigs; under the showcase its own base rows and then its
/// glowing rows (after `--crisp`).
pub fn selection(args: &crate::cli::Args) -> Selection {
    if args.mode.is_showcase() {
        return Selection::base_and(
            &SHOWCASE_BASE_ROWS,
            &[],
            &SHOWCASE_ROWS,
            args.light.emissive_ore,
            args.look.crisp,
        );
    }
    let (extra, after): (&[&'static str], &[&'static str]) = if args.castle {
        (&CASTLE_ROWS, &[])
    } else if args.mode.is_outdoor() {
        (&OUTDOOR_ROWS, &[])
    } else if args.mode.is_thin_rig() || args.mode.is_seam_rig() {
        (&RIG_ROWS, &[])
    } else {
        (&[], &[])
    };
    Selection::cave_and(extra, after, args.light.emissive_ore, args.look.crisp)
}

/// The watch `args` ask for: the file the world started from, when it is a file on disk.
pub fn watch(args: &crate::cli::Args) -> Option<Watch> {
    let (file, path) = load(args.materials.as_deref()).ok()?;
    Some(Watch::new(path?, file, selection(args)))
}
