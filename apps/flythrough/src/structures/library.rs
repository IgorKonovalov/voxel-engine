//! The structures library: one entry per `<name>/<name>.vox` under the library's directory, in
//! name order, then the generated castle. Each file's palette rows join the world's table
//! before it is generated ([`add_library_rows`], ADR-0065); an entry holds its scene at scale
//! 1 against that table, the rows it added and its size. The castle is built on a worker the
//! first time it is asked for ([`Library::build_castle`]) and maps through the table's castle
//! rows by name, so under `--castle` its prefabs are the ones the hall stamps. An entry is a
//! belt item (`crate::entities::belt::Item::Structure`); its ghost is a coarse copy of the
//! structure inside the frame of its box ([`ghost_model`]).

use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

use svet_assets::{SceneOptions, Slot, VoxFile, VoxMap, VoxScene};
use svet_entity::{MAX_COARSE, MAX_EDGE, MAX_PARTS, Model, Part};
use svet_world::{AIR, MaterialId, MaterialTable, Prefab, Rot};

/// The castle entry's name.
pub const CASTLE_NAME: &str = "castle";
/// A frame corner's arms, in voxels, for a box over [`MAX_EDGE`] a side.
pub const CORNER_ARM: u32 = 32;

/// A ghost's body: a scene's coarse volume and its factor `k` ([`coarse_volume`]).
pub type Body = (Arc<Prefab>, u32);

/// The box voxels up to which a ghost's body is made on the frame thread; a larger box's is
/// made on a worker, the frame standing alone until it arrives. [`coarse_volume`] reads about
/// 20 ns a box voxel (opt-level 2), so this is about 20 ms.
pub const FRAME_BODY_VOXELS: u64 = 1 << 20;

/// A ghost's body at one scale: made (`None` for a scene with none), or on its worker with
/// the scaled scene.
#[derive(Clone)]
enum BodyState {
    Made(Option<Body>),
    Making(Arc<Mutex<Receiver<MadeBody>>>),
}

/// What a body's worker sends: the scaled scene and its body.
type MadeBody = Result<(Arc<VoxScene>, Option<Body>), String>;

/// A built scene at scale 1, its icon's model ([`icon_model`]) and its ghost's body.
type Built = Result<(Arc<VoxScene>, Option<Arc<Model>>, Option<Body>), String>;

/// The castle entry's state: asked for or not, building on a worker, or built.
#[derive(Clone, Default)]
pub enum Castle {
    #[default]
    NotBuilt,
    Building(Arc<Mutex<Receiver<Built>>>),
    Built(Arc<VoxScene>),
}

/// What an entry places: a `.vox` file and its scene at scale 1, or the castle.
#[derive(Clone)]
pub enum Source {
    File(Arc<VoxFile>, Arc<VoxScene>),
    Castle(Castle),
}

/// One structure: its name, its scene (or why it has none), the palette rows its file added,
/// its scenes at scales over 1 once made, its icon's model once made, and its ghost's body per
/// scale once made.
#[derive(Clone)]
pub struct Entry {
    pub name: String,
    pub source: Result<Source, String>,
    pub rows: usize,
    scaled: Vec<(u32, Arc<VoxScene>)>,
    /// The icon's model ([`icon_model`]) once tried: `Some(None)` for a scene with none.
    icon: Option<Option<Arc<Model>>>,
    /// The ghost's body per scale once asked for.
    bodies: Vec<(u32, BodyState)>,
}

impl Entry {
    fn new(name: String, source: Result<Source, String>, rows: usize) -> Self {
        Self {
            name,
            source,
            rows,
            scaled: Vec::new(),
            icon: None,
            bodies: Vec::new(),
        }
    }

    /// Why the entry cannot be placed: its file's error, or the castle's build error.
    pub fn error(&self) -> Option<&str> {
        self.source.as_ref().err().map(String::as_str)
    }

    /// The scene at scale 1, once there is one.
    pub fn scene(&self) -> Option<&Arc<VoxScene>> {
        match self.source.as_ref().ok()? {
            Source::File(_, s) | Source::Castle(Castle::Built(s)) => Some(s),
            Source::Castle(_) => None,
        }
    }

    /// Whether the entry is the castle before its scene arrives.
    pub fn building(&self) -> bool {
        matches!(&self.source, Ok(Source::Castle(c)) if !matches!(c, Castle::Built(_)))
    }

    /// The box's size in voxels at `scale`, turned `quarters` quarter turns about z.
    pub fn size(&self, scale: u32, quarters: u8) -> Option<[u32; 3]> {
        let s = self.scene()?.size().map(|v| v * scale);
        Some(if quarters % 2 == 1 {
            [s[1], s[0], s[2]]
        } else {
            s
        })
    }

    /// Why the entry cannot be placed at `scale` in a world whose edits span at most `limit`
    /// voxels an axis: its box with [`super::CLEAR_MARGIN`] each side over the limit
    /// (`castle x2: 1200 x 1200 x 1000 voxels, over the edit limit of 1024`), or a scale over 1
    /// of an entry with no file to scale (`castle: scale 1 only`); `None` when it can be, and
    /// for an entry in error or still building (their own words say why). The reason follows the
    /// words' first `: `.
    pub fn refusal(&self, scale: u32, limit: u32) -> Option<String> {
        let size = self.size(scale, 0)?;
        let m = 2 * super::CLEAR_MARGIN as u32;
        if size.iter().any(|&v| v + m > limit) {
            return Some(format!(
                "{} x{scale}: {} x {} x {} voxels, over the edit limit of {limit}",
                self.name, size[0], size[1], size[2]
            ));
        }
        (scale > 1 && !matches!(&self.source, Ok(Source::File(..))))
            .then(|| format!("{}: scale 1 only", self.name))
    }
}

impl std::fmt::Debug for Entry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (rows +{}", self.name, self.rows)?;
        match (self.error(), self.building()) {
            (Some(e), _) => write!(f, ", {e})"),
            (None, true) => write!(f, ", building)"),
            (None, false) => write!(f, ")"),
        }
    }
}

/// The structures, in the chooser's and the belt's order, and the notes the scan left (a
/// directory without its `.vox`).
#[derive(Clone, Debug, Default)]
pub struct Library {
    pub entries: Vec<Entry>,
    pub notes: Vec<String>,
}

impl PartialEq for Library {
    fn eq(&self, o: &Self) -> bool {
        let key = |e: &Entry| {
            (
                e.name.clone(),
                e.rows,
                e.error().map(str::to_owned),
                e.building(),
            )
        };
        self.entries.iter().map(key).eq(o.entries.iter().map(key)) && self.notes == o.notes
    }
}

/// The library's directory `args` name: `--structures-dir`'s or [`super::LIBRARY`].
pub fn dir_of(args: &crate::cli::Args) -> PathBuf {
    args.structures_dir
        .clone()
        .unwrap_or_else(|| PathBuf::from(super::LIBRARY))
}

/// The library `args` name ([`dir_of`]) against the world's `table`, its start-up line
/// printed (`structures: 2 entries, +255 rows`) and its notes after it.
pub fn of_args(args: &crate::cli::Args, table: &MaterialTable) -> Library {
    let base = crate::setup::base_table(args, &crate::setup::materials(args));
    let lib = Library::load(&dir_of(args), table, &base);
    let rows: usize = lib.entries.iter().map(|e| e.rows).sum();
    println!("structures: {} entries, +{rows} rows", lib.entries.len());
    for n in &lib.notes {
        println!("{n}");
    }
    lib
}

/// A library file's name and its parsed `.vox`, or why it did not read.
pub type Scanned = (String, Result<VoxFile, String>);

/// The library's files at `dir`: each subdirectory `<name>/` holding `<name>.vox`, in name
/// order, read and parsed (or the reason not); the notes for those without one.
pub fn scan_files(dir: &Path) -> (Vec<Scanned>, Vec<String>) {
    let mut notes = Vec::new();
    let mut names: Vec<String> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect(),
        Err(e) => {
            notes.push(format!("structures {}: {e}", dir.display()));
            Vec::new()
        }
    };
    names.sort();
    let mut files = Vec::new();
    for name in names {
        let path = dir.join(&name).join(format!("{name}.vox"));
        if !path.is_file() {
            notes.push(format!("structures: {name}/ holds no {name}.vox"));
            continue;
        }
        files.push((name, super::read(&path)));
    }
    (files, notes)
}

/// Adds to `table` a row for each palette entry of every file of the library at `dir`, in
/// its order ([`super::add_rows`]), then the castle's rows from the material file when the
/// table has none of them: each file's rows added, in the scan's order.
pub fn add_library_rows(
    dir: &Path,
    materials: Option<&Path>,
    table: &mut MaterialTable,
) -> Vec<usize> {
    let (files, _) = scan_files(dir);
    let added = files
        .iter()
        .map(|(_, f)| f.as_ref().map_or(0, |f| super::add_rows(f, table)))
        .collect();
    if crate::materials::CASTLE_ROWS
        .iter()
        .all(|n| row_named(table, n).is_none())
        && let Ok((file, _)) = crate::materials::load(materials)
    {
        for n in crate::materials::CASTLE_ROWS {
            if let Some((_, row)) = file.row(n) {
                table.push(row.clone());
            }
        }
    }
    added
}

/// The id of `table`'s row named `name`.
fn row_named(table: &MaterialTable, name: &str) -> Option<MaterialId> {
    (1..table.len())
        .map(|i| i as MaterialId)
        .find(|&i| table.get(i).is_some_and(|m| m.name == name))
}

/// The castle's palette against `table`: index `i` in `1..=7` to the row named
/// `crate::materials::CASTLE_ROWS[i - 1]`, index 8 (the dungeon's written air) to air, index 0
/// left alone. Under `--castle` those rows are `castle::material(i)`, so this is
/// `castle::dungeon_map`.
pub fn castle_map(table: &MaterialTable) -> Result<VoxMap, String> {
    let mut m = VoxMap::new();
    m.set(0, Slot::Skip);
    for (k, n) in crate::materials::CASTLE_ROWS.iter().enumerate() {
        let id = row_named(table, n).ok_or_else(|| format!("no `{n}` row"))?;
        m.set(k as u8 + 1, Slot::Material(id));
    }
    m.set(crate::castle::EXPLICIT_AIR, Slot::Air);
    Ok(m)
}

impl Library {
    /// The library `args` name: [`of_args`].
    pub fn of_args(args: &crate::cli::Args, table: &MaterialTable) -> Self {
        of_args(args, table)
    }

    /// The library at `dir` against the world's `table` (which holds every file's rows): each
    /// file's scene at scale 1 and the rows it adds to `base` (the table before the
    /// library's rows), then the castle, not built.
    pub fn load(dir: &Path, table: &MaterialTable, base: &MaterialTable) -> Self {
        let (files, notes) = scan_files(dir);
        let mut base = base.clone();
        let mut entries = Vec::new();
        for (name, file) in files {
            let rows = file.as_ref().map_or(0, |f| super::add_rows(f, &mut base));
            let source = file.and_then(|f| {
                let s = super::scene(&f, table, 1)?;
                if s.placements.is_empty() {
                    return Err("no placement".to_owned());
                }
                Ok(Source::File(Arc::new(f), Arc::new(s)))
            });
            entries.push(Entry::new(name, source, rows));
        }
        entries.push(Entry::new(
            CASTLE_NAME.to_owned(),
            Ok(Source::Castle(Castle::NotBuilt)),
            0,
        ));
        Self { entries, notes }
    }

    /// The entries in the chooser's and the belt's order: the library's.
    pub fn order(&self) -> Vec<usize> {
        (0..self.entries.len()).collect()
    }

    /// Starts entry `i`'s castle building on a worker against `table`, when it is the castle
    /// and not asked for yet: `castle::castle_vox` and its scene through [`castle_map`].
    pub fn build_castle(&mut self, i: usize, table: &MaterialTable) {
        let Some(Entry {
            source: Ok(Source::Castle(c @ Castle::NotBuilt)),
            ..
        }) = self.entries.get_mut(i)
        else {
            return;
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let map = castle_map(table);
        let spawned = std::thread::Builder::new()
            .name("castle structure".into())
            .spawn(move || {
                let built = map.and_then(|map| {
                    let file = crate::castle::castle_vox();
                    let scene = VoxScene::load(&file, &map, SceneOptions::default())
                        .map_err(|e| e.to_string())?;
                    // One coarse volume serves the icon (when the scene fits no model of a
                    // part per placement) and the ghost's body.
                    let body = coarse_volume(&scene).map(|(v, k)| (Arc::new(v), k));
                    let icon = scene_model(&scene)
                        .or_else(|| body.as_ref().and_then(|(v, _)| one_part(v)))
                        .map(Arc::new);
                    Ok((Arc::new(scene), icon, body))
                });
                let _ = tx.send(built);
            });
        *c = match spawned {
            Ok(_) => Castle::Building(Arc::new(Mutex::new(rx))),
            Err(e) => {
                self.entries[i].source = Err(format!("no worker: {e}"));
                return;
            }
        };
    }

    /// Takes the castle's scene, its icon's model and its ghost's body when its worker has
    /// made them: the words to say (`castle built: 21 placements, 600 x 600 x 500 voxels`, or
    /// the error).
    pub fn poll(&mut self) -> Option<String> {
        for e in &mut self.entries {
            let Ok(Source::Castle(Castle::Building(rx))) = &e.source else {
                continue;
            };
            let got = rx.lock().ok()?.try_recv();
            let built = match got {
                Ok(b) => b,
                Err(std::sync::mpsc::TryRecvError::Empty) => continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    Err("the build stopped".to_owned())
                }
            };
            let words = match &built {
                Ok((s, _, _)) => {
                    let z = s.size();
                    format!(
                        "{} built: {} placements, {} x {} x {} voxels",
                        e.name,
                        s.placements.len(),
                        z[0],
                        z[1],
                        z[2]
                    )
                }
                Err(err) => format!("{}: {err}", e.name),
            };
            e.source = match built {
                Ok((s, icon, body)) => {
                    e.icon = Some(icon);
                    e.bodies = vec![(1, BodyState::Made(body))];
                    Ok(Source::Castle(Castle::Built(s)))
                }
                Err(err) => Err(err),
            };
            return Some(words);
        }
        None
    }

    /// [`Library::poll`] until no castle is building, for the tests.
    pub fn wait(&mut self) {
        while self
            .entries
            .iter()
            .any(|e| matches!(&e.source, Ok(Source::Castle(Castle::Building(_)))))
        {
            if self.poll().is_none() {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
    }

    /// Entry `i`'s scene at `scale`, made from its file at that scale on first use against
    /// `table` (the castle's at scale 1 only).
    pub fn scene_at(
        &mut self,
        i: usize,
        scale: u32,
        table: &MaterialTable,
    ) -> Result<Arc<VoxScene>, String> {
        let e = self.entries.get_mut(i).ok_or("no such structure")?;
        if let Some(err) = e.error() {
            return Err(format!("{}: {err}", e.name));
        }
        let one = e
            .scene()
            .cloned()
            .ok_or_else(|| format!("{}: building", e.name))?;
        if scale == 1 {
            return Ok(one);
        }
        if let Some((_, s)) = e.scaled.iter().find(|(k, _)| *k == scale) {
            return Ok(s.clone());
        }
        let Ok(Source::File(f, _)) = &e.source else {
            return Err(format!("{}: scale 1 only", e.name));
        };
        let s = Arc::new(super::scene(f, table, scale)?);
        e.scaled.push((scale, s.clone()));
        Ok(s)
    }

    /// Entry `i`'s ghost body at `scale` ([`coarse_volume`] of its scene at that scale), kept
    /// once made; `Ok(None)` for a scene with none and while it is being made. A box of at
    /// most [`FRAME_BODY_VOXELS`] voxels is made here, on first use; a larger one, with its
    /// scaled scene, on a worker that a later call takes it from. The castle's arrives with its
    /// build.
    pub fn body_at(
        &mut self,
        i: usize,
        scale: u32,
        table: &MaterialTable,
    ) -> Result<Option<Body>, String> {
        let e = self.entries.get_mut(i).ok_or("no such structure")?;
        if let Some(at) = e.bodies.iter().position(|(k, _)| *k == scale) {
            let rx = match &e.bodies[at].1 {
                BodyState::Made(b) => return Ok(b.clone()),
                BodyState::Making(rx) => rx.clone(),
            };
            let got = rx.lock().map_err(|_| "a body's worker failed")?.try_recv();
            let made = match got {
                Err(std::sync::mpsc::TryRecvError::Empty) => return Ok(None),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    Err("the body's worker stopped".to_owned())
                }
                Ok(m) => m,
            };
            let body = match made {
                Ok((scene, body)) => {
                    if scale > 1 && !e.scaled.iter().any(|(k, _)| *k == scale) {
                        e.scaled.push((scale, scene));
                    }
                    body
                }
                Err(_) => None,
            };
            e.bodies[at].1 = BodyState::Made(body.clone());
            return Ok(body);
        }
        let size = e
            .size(scale, 0)
            .ok_or_else(|| format!("{}: building", e.name))?;
        let voxels: u64 = size.iter().map(|&v| u64::from(v)).product();
        let file = match &e.source {
            Ok(Source::File(f, one)) if voxels > FRAME_BODY_VOXELS => {
                let have = if scale == 1 {
                    Some(one.clone())
                } else {
                    e.scaled
                        .iter()
                        .find(|(k, _)| *k == scale)
                        .map(|(_, s)| s.clone())
                };
                Some((f.clone(), have))
            }
            _ => None,
        };
        let Some((file, have)) = file else {
            let scene = self.scene_at(i, scale, table)?;
            let body = coarse_volume(&scene).map(|(v, k)| (Arc::new(v), k));
            self.entries[i]
                .bodies
                .push((scale, BodyState::Made(body.clone())));
            return Ok(body);
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let table = table.clone();
        let spawned = std::thread::Builder::new()
            .name("structure ghost".into())
            .spawn(move || {
                let scene = match have {
                    Some(s) => Ok(s),
                    None => super::scene(&file, &table, scale).map(Arc::new),
                };
                let made = scene.map(|scene| {
                    let body = coarse_volume(&scene).map(|(v, k)| (Arc::new(v), k));
                    (scene, body)
                });
                let _ = tx.send(made);
            });
        let state = match spawned {
            Ok(_) => BodyState::Making(Arc::new(Mutex::new(rx))),
            Err(_) => BodyState::Made(None),
        };
        self.entries[i].bodies.push((scale, state));
        Ok(None)
    }

    /// [`Library::body_at`] until entry `i`'s body at `scale` is made, for the tests.
    pub fn wait_body(
        &mut self,
        i: usize,
        scale: u32,
        table: &MaterialTable,
    ) -> Result<Option<Body>, String> {
        loop {
            let b = self.body_at(i, scale, table)?;
            let making = self.entries[i]
                .bodies
                .iter()
                .any(|(k, s)| *k == scale && matches!(s, BodyState::Making(_)));
            if !making {
                return Ok(b);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}

/// The opaque row the frame of `scene` is drawn in: the material of the first written,
/// opaque voxel of its first placed prefab, else row 1.
pub fn frame_row(scene: &VoxScene, table: &MaterialTable) -> MaterialId {
    let found = scene.prefabs.first().and_then(|p| {
        let s = p.size().map(|v| v as i32);
        (0..s[2]).find_map(|z| {
            (0..s[1]).find_map(|y| {
                (0..s[0]).find_map(|x| {
                    p.get([x, y, z])
                        .filter(|&m| m != AIR && !table.is_translucent(m))
                })
            })
        })
    });
    found.unwrap_or(1)
}

/// A part of `volume`, placed with its minimum corner at `at` in the model's rest pose (the
/// pivot carries the offset).
fn part(volume: Prefab, at: [u32; 3]) -> Part {
    Part {
        volume: Arc::new(volume),
        parent: None,
        pivot: at.map(|v| -(v as i32) * 256),
        detail: 1,
        coarse: 1,
    }
}

/// The frame of a box `size` voxels across: its twelve edges, one voxel thick, a part each,
/// when every side is at most [`MAX_EDGE`]; else its eight corners, each one part of three
/// arms of up to [`CORNER_ARM`] voxels meeting at the corner. Every voxel is `material`; the
/// rest pose's root is the box's minimum corner.
pub fn frame_model(size: [u32; 3], material: MaterialId) -> Model {
    Model::new(frame_parts(size, material)).expect("a frame's parts fit a model")
}

/// A structure's ghost: part 0 its `body` (a coarse volume at `k` world voxels a voxel,
/// [`coarse_volume`]) turned `quarters` quarter turns about z (`Prefab::rotated`, exact) with
/// its minimum corner at the box's, then [`frame_model`]'s parts for the box `size` voxels
/// across (`size` already turned): 13 parts for a box at most [`MAX_EDGE`] a side, else 9.
/// With no body, or one over [`MAX_COARSE`], the frame alone. The rest pose's root is the
/// box's minimum corner; the body overhangs the box by under `k` voxels on a side `k` does not
/// divide.
pub fn ghost_model(
    body: Option<&Body>,
    quarters: u8,
    size: [u32; 3],
    material: MaterialId,
) -> Model {
    let mut parts = Vec::new();
    if let Some((volume, k)) = body
        && let Ok(coarse) = u8::try_from(*k)
        && (1..=MAX_COARSE).contains(&coarse)
    {
        let mut turn = Rot::IDENTITY;
        for _ in 0..quarters % 4 {
            turn = turn.then(Rot::Z90);
        }
        let turned = if turn == Rot::IDENTITY {
            volume.clone()
        } else {
            Arc::new(volume.rotated(turn))
        };
        parts.push(Part {
            volume: turned,
            parent: None,
            pivot: [0; 3],
            detail: 1,
            coarse,
        });
    }
    parts.extend(frame_parts(size, material));
    Model::new(parts).expect("a ghost's parts fit a model")
}

/// [`frame_model`]'s parts.
fn frame_parts(size: [u32; 3], material: MaterialId) -> Vec<Part> {
    let s = size.map(|v| v.max(1));
    let mut parts = Vec::new();
    if s.iter().all(|&v| v <= MAX_EDGE) {
        for axis in 0..3 {
            let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
            for (ea, eb) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let mut dims = [1; 3];
                dims[axis] = s[axis];
                let mut at = [0; 3];
                at[a] = ea * (s[a] - 1);
                at[b] = eb * (s[b] - 1);
                parts.push(part(Prefab::uniform(dims, material), at));
            }
        }
    } else {
        let arm = s.map(|v| v.min(CORNER_ARM));
        for c in 0..8u32 {
            let side = [0, 1, 2].map(|k| c >> k & 1);
            // The corner's voxel inside the part's box, on each axis 0 or the arm's end.
            let corner = [0, 1, 2].map(|k| side[k] * (arm[k] - 1));
            let volume = Prefab::from_fn(arm, |l| {
                let on = (0..3).filter(|&k| l[k] == corner[k]).count();
                (on >= 2).then_some(material)
            });
            let at = [0, 1, 2].map(|k| side[k] * (s[k] - arm[k]));
            parts.push(part(volume, at));
        }
    }
    debug_assert!(parts.len() < MAX_PARTS, "a body's part beside the frame's");
    parts
}

/// A model of `scene` for its icon: one part per placement, its prefab turned as placed, at
/// its place in the box; `None` when a turned prefab is over [`MAX_EDGE`] a side or the
/// placements over [`MAX_PARTS`].
pub fn scene_model(scene: &VoxScene) -> Option<Model> {
    if scene.placements.len() > MAX_PARTS {
        return None;
    }
    let (lo, _) = scene.bounds;
    let mut parts = Vec::new();
    for (p, o, r) in scene.placed_at([0; 3], 0) {
        let turned = p.rotated(r);
        if turned.size().iter().any(|&v| v > MAX_EDGE) {
            return None;
        }
        let at = [0, 1, 2].map(|k| (o.0[k] - lo[k]) as u32);
        parts.push(part(turned, at));
    }
    Model::new(parts).ok()
}

/// A model of `scene` for its icon: [`scene_model`] when the scene fits one, else
/// [`coarse_model`].
pub fn icon_model(scene: &VoxScene) -> Option<Model> {
    scene_model(scene).or_else(|| coarse_model(scene))
}

/// A one-part model of `scene` at most [`MAX_EDGE`] a side, for the icon of a scene over the
/// model's limits: [`coarse_volume`] at detail 1; `None` for an empty box.
pub fn coarse_model(scene: &VoxScene) -> Option<Model> {
    one_part(&coarse_volume(scene)?.0)
}

/// A one-part model of `volume` at detail 1, its minimum corner at the root.
fn one_part(volume: &Prefab) -> Option<Model> {
    Model::new(vec![part(volume.clone(), [0; 3])]).ok()
}

/// `scene`'s box at most [`MAX_EDGE`] a side, and `k`: each voxel stands for a cube of `k`
/// voxels a side of the box (`k` the least that fits), and holds the material of the first
/// sample point of that cube a placement writes, the samples `k / 2` apart (at least 1) from
/// the cube's minimum corner, the later placement first where two overlap. At `k` 1 it is the
/// scene voxel for voxel. `None` for an empty box.
pub fn coarse_volume(scene: &VoxScene) -> Option<(Prefab, u32)> {
    let size = scene.size();
    let k = size.iter().map(|&v| v.div_ceil(MAX_EDGE)).max()?;
    if k == 0 {
        return None;
    }
    let (lo, _) = scene.bounds;
    // Each placement's prefab, its turned box's minimum corner in the box and size, its turn.
    let placed: Vec<_> = scene
        .placed_at([0; 3], 0)
        .into_iter()
        .map(|(p, o, r)| {
            let at = [0, 1, 2].map(|i| o.0[i] - lo[i]);
            let turned = r.apply_size(p.size());
            (p, at, turned, r)
        })
        .collect();
    // The material placed at box voxel `v`: the turned voxel read back in the prefab as
    // `Prefab::rotated` reads it.
    let at_voxel = |v: [i32; 3]| {
        placed.iter().rev().find_map(|(p, at, turned, r)| {
            let d = [0, 1, 2].map(|i| v[i] - at[i]);
            if (0..3).any(|i| d[i] < 0 || d[i] >= turned[i] as i32) {
                return None;
            }
            let mut src = [0i32; 3];
            for (i, &di) in d.iter().enumerate() {
                let (a, neg) = r.row(i);
                let a = usize::from(a);
                src[a] = if neg { p.size()[a] as i32 - 1 - di } else { di };
            }
            p.get(src).filter(|&m| m != AIR)
        })
    };
    let step = (k / 2).max(1) as usize;
    let coarse = size.map(|v| v.div_ceil(k));
    let volume = Prefab::from_fn(coarse, |c| {
        let base = c.map(|v| v * k);
        (0..k).step_by(step).find_map(|z| {
            (0..k).step_by(step).find_map(|y| {
                (0..k).step_by(step).find_map(|x| {
                    let v = [base[0] + x, base[1] + y, base[2] + z];
                    (0..3)
                        .all(|i| v[i] < size[i])
                        .then(|| at_voxel(v.map(|n| n as i32)))
                        .flatten()
                })
            })
        })
    });
    Some((volume, k))
}

/// The icons of the entities and of the structures ([`icon_model`], made once per entry; the
/// castle's on its build's worker), each structure's at its tile after the entities'
/// (`crate::entities::belt::tile_of`), asked of the entities' icon worker.
pub fn refresh_icons(m: &mut crate::hud::Modes, table: &MaterialTable) {
    m.walkers.refresh_icons(table);
    let base = m.walkers.library.entries.len().max(1);
    for (j, e) in m.structures.entries.iter_mut().enumerate() {
        if e.icon.is_none()
            && let Some(s) = e.scene()
        {
            e.icon = Some(icon_model(s).map(Arc::new));
        }
        if let Some(Some(model)) = &e.icon {
            m.walkers.icons.want(base + j, model, table);
        }
    }
}
