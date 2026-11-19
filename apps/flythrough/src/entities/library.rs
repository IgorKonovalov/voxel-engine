//! The entities library: every subdirectory `<name>/` of a directory that holds `<name>.rig`
//! is one entry named `<name>`, listed in name order. The default directory is the
//! fly-through's `assets/entities/`; `--library-dir DIR` points elsewhere and `--model x.rig`
//! adds that rig as an entry ahead of the library's, selected. An entry whose files do not
//! read or parse, or whose rig does not build against the world's table, stays in the list
//! with its error and cannot be placed. A missing or empty library gives the built-in walker
//! (`super::walker`) as its one entry, so a run from an unpacked tester archive still places
//! something. `flythrough --library` prints the list ([`library_text`]) and exits.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use vxe_entity::{Model, Pose};
use vxe_world::MaterialTable;

use crate::viewer::Viewer;

/// The library the run scans unless `--library-dir` names another: the fly-through's
/// `assets/entities/` where it was built.
pub const DEFAULT_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/entities");

/// The built-in walker's entry name.
pub const BUILTIN_NAME: &str = "walker";

/// Where an entry's model comes from.
#[derive(Clone, Debug)]
pub enum Source {
    /// The walker made in code ([`super::walker::model`]).
    Builtin,
    /// A rig and its `.vox`, loaded and polled by their own viewer.
    Rig(Box<Viewer>),
}

/// One entry of the library: its name, its model's source or why its files did not load,
/// and whether it is `--model`'s.
#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub source: Result<Source, String>,
    pub flagged: bool,
}

impl Entry {
    /// The entry's viewer, for a rig that loaded.
    pub fn viewer(&self) -> Option<&Viewer> {
        match &self.source {
            Ok(Source::Rig(v)) => Some(v),
            _ => None,
        }
    }

    pub fn viewer_mut(&mut self) -> Option<&mut Viewer> {
        match &mut self.source {
            Ok(Source::Rig(v)) => Some(v),
            _ => None,
        }
    }

    /// Why the entry cannot be placed: its files' error, or a build's that left no model.
    pub fn error(&self) -> Option<&str> {
        match &self.source {
            Err(e) => Some(e),
            Ok(Source::Rig(v)) => v.failed(),
            Ok(Source::Builtin) => None,
        }
    }
}

/// The entries in the picker's order, the scan's notes (a directory with no rig, a missing
/// library) and the selected entry's index.
#[derive(Clone, Debug, Default)]
pub struct Library {
    pub entries: Vec<Entry>,
    pub notes: Vec<String>,
    pub selected: usize,
}

/// The built-in walker's entry.
fn builtin() -> Entry {
    Entry {
        name: BUILTIN_NAME.to_owned(),
        source: Ok(Source::Builtin),
        flagged: false,
    }
}

/// The library at `dir`: one entry per subdirectory `<name>/` holding `<name>.rig`, in name
/// order, each rig read and parsed (built later, against the world's table); a subdirectory
/// with no such rig is a note. A directory that does not read, or holds no entry, gives the
/// built-in walker and a note saying why.
pub fn scan(dir: &Path) -> Library {
    let mut lib = Library::default();
    let mut names: Vec<String> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect(),
        Err(e) => {
            lib.notes.push(format!(
                "library {}: {e}; the built-in walker",
                dir.display()
            ));
            lib.entries.push(builtin());
            return lib;
        }
    };
    names.sort();
    for name in names {
        let rig = dir.join(&name).join(format!("{name}.rig"));
        if !rig.is_file() {
            lib.notes
                .push(format!("library: {name}/ holds no {name}.rig"));
            continue;
        }
        let source = Viewer::load(&rig, None).map(|v| Source::Rig(Box::new(v)));
        lib.entries.push(Entry {
            name,
            source,
            flagged: false,
        });
    }
    if lib.entries.is_empty() {
        lib.notes.push(format!(
            "library {}: no entry; the built-in walker",
            dir.display()
        ));
        lib.entries.push(builtin());
    }
    lib
}

impl Library {
    /// The built-in walker alone: the library of a run that scanned none.
    pub fn builtin() -> Self {
        Self {
            entries: vec![builtin()],
            ..Self::default()
        }
    }

    /// `--model`'s viewer as the library's flagged entry, selected: the scanned entry of the
    /// same rig file (the two paths equal once canonicalised, each the literal path when it
    /// does not canonicalise) takes `--model`'s viewer in place and keeps its name and place;
    /// a rig outside the library is an extra entry ahead of the others.
    pub fn with_model(mut self, viewer: Viewer) -> Self {
        let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        let rig = canon(&viewer.rig_path);
        let scanned = self
            .entries
            .iter()
            .position(|e| e.viewer().is_some_and(|v| canon(&v.rig_path) == rig));
        let name = viewer.name.clone();
        let source = Ok(Source::Rig(Box::new(viewer)));
        self.selected = match scanned {
            Some(i) => {
                let e = &mut self.entries[i];
                (e.source, e.flagged) = (source, true);
                i
            }
            None => {
                let flagged = true;
                self.entries.insert(
                    0,
                    Entry {
                        name,
                        source,
                        flagged,
                    },
                );
                0
            }
        };
        self
    }

    /// `--model`'s viewer, when there is one.
    pub fn flagged(&self) -> Option<&Viewer> {
        self.entries.iter().find(|e| e.flagged)?.viewer()
    }

    pub fn flagged_mut(&mut self) -> Option<&mut Viewer> {
        self.entries.iter_mut().find(|e| e.flagged)?.viewer_mut()
    }

    /// Builds every rig not built yet against `table` ([`Viewer::build_pending`]); gives each
    /// new error as `<name>: <error>`. When the selected entry is not `--model`'s and failed,
    /// the first entry that can be placed is selected instead.
    pub fn build(&mut self, table: &MaterialTable) -> Vec<String> {
        let mut errors = Vec::new();
        for e in &mut self.entries {
            let name = e.name.clone();
            if let Some(v) = e.viewer_mut()
                && let Some(err) = v.build_pending(table)
            {
                errors.push(format!("{name}: {err}"));
            }
        }
        if let Some(sel) = self.entries.get(self.selected)
            && !sel.flagged
            && sel.error().is_some()
            && let Some(i) = self.entries.iter().position(|e| e.error().is_none())
        {
            self.selected = i;
        }
        errors
    }

    /// The entries in the chooser's and the belt's order: `--model`'s first, then those that
    /// can be placed, then those in error, each in the library's order (name order).
    pub fn order(&self) -> Vec<usize> {
        let rank = |e: &Entry| (!e.flagged, e.error().is_some());
        let mut out: Vec<usize> = (0..self.entries.len()).collect();
        out.sort_by_key(|&i| rank(&self.entries[i]));
        out
    }

    /// The `--library` lines once [`Library::build`] has run: one per entry ([`entry_line`]),
    /// `--model`'s first and the others in the library's order, then the notes.
    pub fn lines(&self, table: &MaterialTable) -> Vec<String> {
        let mut entries: Vec<&Entry> = self.entries.iter().collect();
        entries.sort_by_key(|e| !e.flagged);
        let mut out: Vec<String> = entries.iter().map(|e| entry_line(e, table)).collect();
        out.extend(self.notes.iter().cloned());
        out
    }
}

/// An entry's model and rest pose (root at the origin): the built-in walker's from `table`'s
/// walker rows, a built rig's; `None` for an entry in error or a table without the rows.
pub fn model_of(e: &Entry, table: &MaterialTable) -> Option<(std::sync::Arc<Model>, Pose)> {
    match e.source.as_ref().ok()? {
        Source::Builtin => {
            let (body, eye) = super::walker::rows(table)?;
            Some((
                std::sync::Arc::new(super::walker::model(body, eye)),
                super::walker::pose([0.0; 3], 0.0, 0.0),
            ))
        }
        Source::Rig(v) => {
            let l = v.loaded.as_ref()?;
            Some((l.model.clone(), l.rest.clone()))
        }
    }
}

/// The built-in walker's model and rest pose from `table`'s walker rows ([`model_of`]).
pub fn builtin_model(table: &MaterialTable) -> Option<(std::sync::Arc<Model>, Pose)> {
    model_of(&builtin(), table)
}

/// The size of `model` standing in `pose`, in world voxels along x, y and z: the box around
/// every corner of every part's volume.
pub fn extent(model: &Model, pose: &Pose) -> [f32; 3] {
    let Some(c) = pose.compose(model) else {
        return [0.0; 3];
    };
    let mut lo = [i64::MAX; 3];
    let mut hi = [i64::MIN; 3];
    for (i, part) in model.parts().iter().enumerate() {
        let s = part.volume.size().map(|v| v as i32 * 256);
        for k in 0..8 {
            let corner = [0, 1, 2].map(|a| if k >> a & 1 == 1 { s[a] } else { 0 });
            if let Some(p) = c.part_point(i, corner) {
                for a in 0..3 {
                    lo[a] = lo[a].min(p.0[a]);
                    hi[a] = hi[a].max(p.0[a]);
                }
            }
        }
    }
    if lo[0] > hi[0] {
        return [0.0; 3];
    }
    std::array::from_fn(|a| (hi[a] - lo[a]) as f32 / 256.0)
}

/// The mirror's pool bytes `model`'s distinct volumes take
/// ([`vxe_render::EntityMirror::volume_bytes`]).
pub fn pool_bytes(model: &Model) -> u64 {
    let mut seen = Vec::new();
    let mut bytes = 0;
    for p in model.parts() {
        if !seen.contains(&p.volume.id()) {
            seen.push(p.volume.id());
            bytes += vxe_render::EntityMirror::volume_bytes(&p.volume);
        }
    }
    bytes
}

/// The clips' names, the first (the one a placement starts on) marked `*`; `no clip` without.
fn clip_words(e: &Entry) -> String {
    let Some(l) = e.viewer().and_then(|v| v.loaded.as_ref()) else {
        return "no clip".to_owned();
    };
    if l.clips.is_empty() {
        return "no clip".to_owned();
    }
    let names: Vec<String> = l
        .clips
        .iter()
        .enumerate()
        .map(|(i, (n, _))| if i == 0 { format!("{n}*") } else { n.clone() })
        .collect();
    format!("clips {}", names.join(" "))
}

/// The widest `--library` line, in bytes.
pub const LINE_BYTES: usize = 80;

/// One `--library` line: `<name> <n> parts <x>x<y>x<z> vox <bytes> B clips <first>* ...`, or
/// `<name> error: <error>`, cut to [`LINE_BYTES`] with a `~` in the last byte when cut.
pub fn entry_line(e: &Entry, table: &MaterialTable) -> String {
    let mut out = format!("{:<10}", e.name);
    match (e.error(), model_of(e, table)) {
        (Some(err), _) => {
            let _ = write!(out, " error: {err}");
        }
        (None, None) => out.push_str(" error: not built"),
        (None, Some((model, rest))) => {
            let s = extent(&model, &rest);
            let _ = write!(
                out,
                " {:>2} parts  {}x{}x{} vox  {:>6} B  {}",
                model.parts().len(),
                s[0],
                s[1],
                s[2],
                pool_bytes(&model),
                clip_words(e)
            );
        }
    }
    cut(out, LINE_BYTES)
}

/// `s` cut to `width` bytes, a `~` in the last byte when cut.
pub fn cut(mut s: String, width: usize) -> String {
    if s.len() > width {
        let mut end = width.saturating_sub(1);
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
        s.push('~');
    }
    s
}

/// The library's directory `args` name: `--library-dir`'s or [`DEFAULT_DIR`].
pub fn dir_of(args: &crate::cli::Args) -> PathBuf {
    args.library_dir
        .clone()
        .unwrap_or_else(|| PathBuf::from(DEFAULT_DIR))
}

/// The material names the rigs of the library `args` name carry in their `material` lines,
/// `--model`'s first, then each `<name>/<name>.rig` of the library's directory in name order,
/// each name once in the order first met; `auto`, `air` and `skip` left out. Read and parsed
/// only, not built: a rig that does not read or parse names nothing here (its entry reports
/// it).
pub fn rig_material_names(args: &crate::cli::Args) -> Vec<String> {
    let dir = dir_of(args);
    let mut rigs: Vec<PathBuf> = args.model.iter().cloned().collect();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        let mut names: Vec<String> = rd
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        rigs.extend(names.iter().map(|n| dir.join(n).join(format!("{n}.rig"))));
    }
    let mut out: Vec<String> = Vec::new();
    for path in rigs {
        let Some(rig) = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| vxe_assets::Rig::parse(&t).ok())
        else {
            continue;
        };
        for m in rig.materials {
            if !matches!(m.name.as_str(), "auto" | "air" | "skip") && !out.contains(&m.name) {
                out.push(m.name);
            }
        }
    }
    out
}

/// The start-up line for `lib` against the world's `table`: its entries and the rows the
/// rigs' `material` lines added to the table from the material file
/// (`crate::setup::world_table`), `entities: 2 entries, +3 rows`.
pub fn start_line(lib: &Library, args: &crate::cli::Args, table: &MaterialTable) -> String {
    let before = crate::setup::table_before_entities(args, &crate::setup::materials(args));
    let rows = table.len().saturating_sub(before.len());
    format!("entities: {} entries, +{rows} rows", lib.entries.len())
}

/// The library `args` name: `--library-dir`'s or [`DEFAULT_DIR`], with `--model`'s rig first
/// when it opens.
pub fn of_args(args: &crate::cli::Args) -> Library {
    let lib = scan(&dir_of(args));
    match args
        .model
        .as_ref()
        .map(|p| Viewer::open(p, args.clip.clone()))
    {
        Some(Ok(mut v)) => {
            v.rows_added = crate::setup::model_rows_added(args);
            lib.with_model(v)
        }
        Some(Err(e)) => {
            println!("{e}");
            lib
        }
        None => lib,
    }
}

/// The table the run's world draws entities against (`crate::setup::world_table`): the
/// scene's rows from the material file (the showcase's for the showcase), the walker's, the
/// palette rows of `--model`'s and `--vox`'s `.vox`, the structures library's rows, then the
/// rows the rigs name that the table lacks.
pub fn table_of_args(args: &crate::cli::Args) -> MaterialTable {
    crate::setup::world_table(args, &crate::setup::materials(args))
}

/// `flythrough --library`'s text: the library `args` name, built against the run's table,
/// one line per entry and then the notes.
pub fn library_text(args: &crate::cli::Args) -> String {
    let table = table_of_args(args);
    let mut lib = of_args(args);
    lib.build(&table);
    let mut out = String::new();
    for l in lib.lines(&table) {
        out.push_str(&l);
        out.push('\n');
    }
    out
}
