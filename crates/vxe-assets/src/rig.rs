//! The `.rig` text file: which `.vox` node is which part, where each part pivots and hangs,
//! which palette entries are which material, and the clips (ADR-0051).
//!
//! One statement a line; `#` starts a comment; tokens are separated by whitespace.
//!
//! ```text
//! vox walker.vox                       # the .vox file, relative to the rig
//! rate 30                              # ticks per second, for viewers (default 30)
//! material 9 glow_amber                # palette index -> a material name
//! part torso node torso pivot 4 0 3 detail 2
//! part head  node head  parent torso at 0 12 0  pivot 3 0 3 detail 2
//! part leg_l model 2    parent torso at -2 0 0  pivot 1.5 8 1.5 detail 2
//! clip walk ticks 40 loop
//! key 0  leg_l rot x 30
//! key 20 leg_l rot x -30 move 0 0.5 0
//! key 40 leg_l rot x 30
//! ```
//!
//! - **`part <name>`** takes, in any order: `node <name>` (the `.vox` transform node of that
//!   name, whose shape gives the voxels) or `model <index>`; `pivot x y z` (required, in the
//!   part's model voxels); `parent <part>` and `at x y z` (the pivot's rest place from the
//!   parent's pivot, in the parent's model voxels, or from the root in the part's own for a part
//!   with no parent); `detail 1|2|4` (default 1). A parent may be declared later in the file.
//! - **`key <tick> <part>`** then any of `rot <x|y|z> <whole degrees>` and `move x y z` (an
//!   offset from the rest place, in the parent's model voxels as `at`). Several `rot` compose in
//!   the order written: the product `r1 * r2 * ...`, each later turn about the axes the earlier
//!   ones turned. A key belongs to the clip above it; one part's keys rise in tick.
//! - **Lengths** are decimals that are whole multiples of 1/256 (`1.5`, `-0.25`, `0.00390625`).
//!   At build a length in model voxels becomes 1/256 world voxel by dividing by the detail,
//!   rounded half away from zero.
//! - **Degrees** become Q14 through a committed table of sines (never the platform's `sin`).
//!
//! The rest layout goes into a rest [`Pose`] (the part has no rest offset of its own); a key's
//! translation is an offset from it (see [`vxe_entity::sample`]).

use std::collections::BTreeMap;
use std::fmt::Write as _;

use vxe_entity::{Clip, Key, Model, Part, Pose, Quat, Track};
use vxe_world::MaterialId;

use crate::map::{Slot, VoxMap};
use crate::sines::sin_cos_half_degrees;
use crate::vox::{Node, VoxFile};

/// A source line number, 1-based. It does not count toward equality, so a rig written back
/// and parsed again equals the first parse.
#[derive(Clone, Copy, Debug, Default)]
pub struct Line(pub usize);

impl std::hash::Hash for Line {
    /// Nothing: equal rigs hash alike whatever their lines.
    fn hash<H: std::hash::Hasher>(&self, _: &mut H) {}
}

impl PartialEq for Line {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for Line {}

/// A rig error: the line it was found on (`None` for one found against the `.vox` that has no
/// single line) and a message that quotes the token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RigError {
    pub line: Option<usize>,
    pub message: String,
}

impl std::fmt::Display for RigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.line {
            Some(n) => write!(f, "rig line {n}: {}", self.message),
            None => write!(f, "rig: {}", self.message),
        }
    }
}

impl std::error::Error for RigError {}

fn err(line: usize, message: impl Into<String>) -> RigError {
    RigError {
        line: Some(line),
        message: message.into(),
    }
}

/// Where a part's voxels come from in the `.vox` file.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum VoxSource {
    /// The transform node of this name; its child shape's first model.
    Node(String),
    /// The model of this index.
    Model(u32),
}

/// An axis of a `rot`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    fn name(self) -> &'static str {
        match self {
            Self::X => "x",
            Self::Y => "y",
            Self::Z => "z",
        }
    }
}

/// A `material` line: a palette index and a material name.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RigMaterial {
    pub index: u8,
    pub name: String,
    pub line: Line,
}

/// A `part` line. Lengths are in 1/256 model voxel.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RigPart {
    pub name: String,
    pub source: VoxSource,
    pub parent: Option<String>,
    /// The pivot's rest place from the parent's pivot, in 1/256 of the parent's model voxel (of
    /// the part's own for a part with no parent).
    pub at: [i32; 3],
    /// The pivot in the part's space, 1/256 model voxel.
    pub pivot: [i32; 3],
    pub detail: u8,
    pub line: Line,
}

/// A `key` line.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RigKey {
    pub tick: u32,
    pub part: String,
    /// In the order written, whole degrees.
    pub rots: Vec<(Axis, i32)>,
    /// The offset from the rest place, in 1/256 of the parent's model voxel as [`RigPart::at`].
    pub moved: [i32; 3],
    pub line: Line,
}

/// A `clip` line and the keys under it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RigClip {
    pub name: String,
    pub ticks: u32,
    pub looped: bool,
    pub keys: Vec<RigKey>,
    pub line: Line,
}

/// A parsed rig: every statement as written, checked for names and order but not yet against
/// the `.vox` file.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Rig {
    /// The `.vox` path as written, relative to the rig file.
    pub vox: String,
    /// Ticks per second, for viewers.
    pub rate: u32,
    pub materials: Vec<RigMaterial>,
    /// In file order.
    pub parts: Vec<RigPart>,
    pub clips: Vec<RigClip>,
}

/// A rig built against its `.vox` file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RigModel {
    /// The parts, parents before children (file order otherwise).
    pub model: Model,
    /// The part names in the model's order.
    pub part_names: Vec<String>,
    /// The rest layout: each part's rest place from its parent in [`vxe_entity::PartPose::t`],
    /// rotations the identity, the root at the origin. The `rest` argument of
    /// [`vxe_entity::sample`].
    pub rest: Pose,
    /// The clips in file order.
    pub clips: Vec<(String, Clip)>,
}

/// Ticks per second when the rig names no `rate`.
pub const DEFAULT_RATE: u32 = 30;
/// The largest whole-degree value a `rot` takes, either sign.
pub const MAX_DEGREES: i32 = 3600;

/// A length token: a decimal that is a whole multiple of 1/256, as 1/256 units.
fn length(line: usize, tok: &str) -> Result<i32, RigError> {
    let bad = || err(line, format!("`{tok}` is not a length"));
    let (neg, body) = match tok.strip_prefix('-') {
        Some(b) => (true, b),
        None => (false, tok.strip_prefix('+').unwrap_or(tok)),
    };
    let (int, frac) = body.split_once('.').unwrap_or((body, ""));
    if int.is_empty() && frac.is_empty()
        || !int.bytes().all(|b| b.is_ascii_digit())
        || !frac.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(bad());
    }
    let int: i64 = if int.is_empty() {
        0
    } else {
        int.parse()
            .ok()
            .filter(|&v: &i64| v <= 1 << 20)
            .ok_or_else(|| err(line, format!("`{tok}` is out of range")))?
    };
    let frac = frac.trim_end_matches('0');
    // 1/256 has eight decimals; a ninth significant one is never a multiple of it.
    if frac.len() > 8 {
        return Err(err(line, format!("`{tok}` is not a multiple of 1/256")));
    }
    let digits: i64 = if frac.is_empty() {
        0
    } else {
        frac.parse().map_err(|_| bad())?
    };
    let scale = 10i64.pow(frac.len() as u32);
    if digits * 256 % scale != 0 {
        return Err(err(line, format!("`{tok}` is not a multiple of 1/256")));
    }
    let v = int * 256 + digits * 256 / scale;
    Ok((if neg { -v } else { v }) as i32)
}

/// A 1/256 value printed as the shortest exact decimal.
fn write_length(out: &mut String, v: i32) {
    let a = i64::from(v).abs();
    let sign = if v < 0 { "-" } else { "" };
    let (int, rem) = (a / 256, a % 256);
    if rem == 0 {
        let _ = write!(out, "{sign}{int}");
    } else {
        // rem / 256 = rem * 390625 / 10^8, exactly.
        let frac = format!("{:08}", rem * 390_625);
        let _ = write!(out, "{sign}{int}.{}", frac.trim_end_matches('0'));
    }
}

fn count<T: std::str::FromStr>(line: usize, tok: &str, what: &str) -> Result<T, RigError> {
    if !tok.bytes().all(|b| b.is_ascii_digit()) {
        return Err(err(line, format!("`{tok}` is not {what}")));
    }
    tok.parse()
        .map_err(|_| err(line, format!("`{tok}` is not {what}")))
}

fn name(line: usize, tok: &str) -> Result<String, RigError> {
    if tok.is_empty()
        || !tok
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
    {
        return Err(err(
            line,
            format!("`{tok}` is not a name (letters, digits, `_`, `-`)"),
        ));
    }
    Ok(tok.to_owned())
}

/// The tokens of one statement, with the line to blame.
struct Toks<'a> {
    line: usize,
    it: std::str::SplitAsciiWhitespace<'a>,
    last: &'a str,
}

impl<'a> Toks<'a> {
    fn next(&mut self, what: &str) -> Result<&'a str, RigError> {
        match self.it.next() {
            Some(t) => {
                self.last = t;
                Ok(t)
            }
            None => Err(err(
                self.line,
                format!("expected {what} after `{}`", self.last),
            )),
        }
    }

    fn opt(&mut self) -> Option<&'a str> {
        let t = self.it.next()?;
        self.last = t;
        Some(t)
    }

    fn lengths(&mut self, what: &str) -> Result<[i32; 3], RigError> {
        let mut v = [0; 3];
        for c in &mut v {
            *c = length(self.line, self.next(what)?)?;
        }
        Ok(v)
    }

    fn end(&mut self) -> Result<(), RigError> {
        match self.it.next() {
            None => Ok(()),
            Some(t) => Err(err(self.line, format!("unexpected `{t}`"))),
        }
    }
}

fn parse_part(t: &mut Toks<'_>) -> Result<RigPart, RigError> {
    let line = t.line;
    let part_name = name(line, t.next("a part name")?)?;
    let (mut source, mut parent, mut at, mut pivot, mut detail) = (None, None, None, None, None);
    while let Some(kw) = t.opt() {
        let dup = |seen: bool| {
            if seen {
                Err(err(line, format!("`{kw}` given twice")))
            } else {
                Ok(())
            }
        };
        match kw {
            "node" => {
                dup(source.is_some())?;
                source = Some(VoxSource::Node(name(line, t.next("a node name")?)?));
            }
            "model" => {
                dup(source.is_some())?;
                let tok = t.next("a model index")?;
                source = Some(VoxSource::Model(count(line, tok, "a model index")?));
            }
            "parent" => {
                dup(parent.is_some())?;
                parent = Some(name(line, t.next("a parent part")?)?);
            }
            "at" => {
                dup(at.is_some())?;
                at = Some(t.lengths("three lengths")?);
            }
            "pivot" => {
                dup(pivot.is_some())?;
                pivot = Some(t.lengths("three lengths")?);
            }
            "detail" => {
                dup(detail.is_some())?;
                let tok = t.next("a detail")?;
                detail = Some(match tok {
                    "1" => 1,
                    "2" => 2,
                    "4" => 4,
                    _ => return Err(err(line, format!("detail `{tok}` is not 1, 2 or 4"))),
                });
            }
            _ => return Err(err(line, format!("unknown part keyword `{kw}`"))),
        }
    }
    let source = source.ok_or_else(|| {
        err(
            line,
            format!("part `{part_name}` names neither a `node` nor a `model`"),
        )
    })?;
    let pivot = pivot.ok_or_else(|| err(line, format!("part `{part_name}` has no `pivot`")))?;
    Ok(RigPart {
        name: part_name,
        source,
        parent,
        at: at.unwrap_or_default(),
        pivot,
        detail: detail.unwrap_or(1),
        line: Line(line),
    })
}

fn parse_key(t: &mut Toks<'_>) -> Result<RigKey, RigError> {
    let line = t.line;
    let tick = count(line, t.next("a tick")?, "a tick")?;
    let part = name(line, t.next("a part name")?)?;
    let mut rots = Vec::new();
    let mut moved = None;
    while let Some(kw) = t.opt() {
        match kw {
            "rot" => {
                let ax = t.next("an axis")?;
                let axis = match ax {
                    "x" => Axis::X,
                    "y" => Axis::Y,
                    "z" => Axis::Z,
                    _ => return Err(err(line, format!("`{ax}` is not an axis (x, y or z)"))),
                };
                let tok = t.next("whole degrees")?;
                let digits = tok.strip_prefix('-').unwrap_or(tok);
                if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(err(
                        line,
                        format!("`{tok}` is not a whole number of degrees"),
                    ));
                }
                let deg: i32 = tok
                    .parse()
                    .ok()
                    .filter(|d: &i32| d.abs() <= MAX_DEGREES)
                    .ok_or_else(|| err(line, format!("`{tok}` degrees is past +-{MAX_DEGREES}")))?;
                rots.push((axis, deg));
            }
            "move" => {
                if moved.is_some() {
                    return Err(err(line, "`move` given twice"));
                }
                moved = Some(t.lengths("three lengths")?);
            }
            _ => return Err(err(line, format!("unknown key keyword `{kw}`"))),
        }
    }
    Ok(RigKey {
        tick,
        part,
        rots,
        moved: moved.unwrap_or_default(),
        line: Line(line),
    })
}

impl Rig {
    /// Parses a rig. Every error carries its line and quotes the token. Names are checked:
    /// a `key`'s part and a `part`'s parent must be parts of the file, the parents must not
    /// form a cycle, names and material indices are unique, a key lies inside its clip and one
    /// part's keys in a clip rise in tick.
    pub fn parse(text: &str) -> Result<Self, RigError> {
        let mut vox = None;
        let mut rate = None;
        let mut materials: Vec<RigMaterial> = Vec::new();
        let mut parts: Vec<RigPart> = Vec::new();
        let mut clips: Vec<RigClip> = Vec::new();
        for (i, raw) in text.lines().enumerate() {
            let line = i + 1;
            let body = raw.split_once('#').map_or(raw, |(b, _)| b);
            let mut t = Toks {
                line,
                it: body.split_ascii_whitespace(),
                last: "",
            };
            let Some(stmt) = t.opt() else { continue };
            match stmt {
                "vox" => {
                    if vox.is_some() {
                        return Err(err(line, "`vox` given twice"));
                    }
                    vox = Some(t.next("a path")?.to_owned());
                }
                "rate" => {
                    if rate.is_some() {
                        return Err(err(line, "`rate` given twice"));
                    }
                    let tok = t.next("ticks per second")?;
                    let r: u32 = count(line, tok, "ticks per second")?;
                    if r == 0 {
                        return Err(err(line, "`rate` 0: a rate is at least 1"));
                    }
                    rate = Some(r);
                }
                "material" => {
                    let tok = t.next("a palette index")?;
                    let index: u8 = count(line, tok, "a palette index from 1 to 255")?;
                    if index == 0 {
                        return Err(err(line, "palette index `0` is empty; use 1 to 255"));
                    }
                    if materials.iter().any(|m| m.index == index) {
                        return Err(err(line, format!("palette index `{index}` mapped twice")));
                    }
                    let name = name(line, t.next("a material name")?)?;
                    materials.push(RigMaterial {
                        index,
                        name,
                        line: Line(line),
                    });
                }
                "part" => {
                    let p = parse_part(&mut t)?;
                    if parts.iter().any(|q| q.name == p.name) {
                        return Err(err(line, format!("part `{}` declared twice", p.name)));
                    }
                    parts.push(p);
                    continue;
                }
                "clip" => {
                    let clip_name = name(line, t.next("a clip name")?)?;
                    if clips.iter().any(|c| c.name == clip_name) {
                        return Err(err(line, format!("clip `{clip_name}` declared twice")));
                    }
                    let kw = t.next("`ticks`")?;
                    if kw != "ticks" {
                        return Err(err(line, format!("expected `ticks`, found `{kw}`")));
                    }
                    let tok = t.next("a length in ticks")?;
                    let ticks: u32 = count(line, tok, "a length in ticks")?;
                    if ticks == 0 {
                        return Err(err(line, "`ticks 0`: a clip is at least one tick"));
                    }
                    let looped = match t.opt() {
                        None => false,
                        Some("loop") => true,
                        Some(tok) => return Err(err(line, format!("unexpected `{tok}`"))),
                    };
                    clips.push(RigClip {
                        name: clip_name,
                        ticks,
                        looped,
                        keys: Vec::new(),
                        line: Line(line),
                    });
                }
                "key" => {
                    let k = parse_key(&mut t)?;
                    let Some(clip) = clips.last_mut() else {
                        return Err(err(line, "a `key` before any `clip`"));
                    };
                    if k.tick > clip.ticks {
                        return Err(err(
                            line,
                            format!(
                                "key tick `{}` is past clip `{}`'s {} ticks",
                                k.tick, clip.name, clip.ticks
                            ),
                        ));
                    }
                    if let Some(prev) = clip.keys.iter().rev().find(|p| p.part == k.part)
                        && prev.tick >= k.tick
                    {
                        return Err(err(
                            line,
                            format!(
                                "key tick `{}` for part `{}` is not after its key at tick {}",
                                k.tick, k.part, prev.tick
                            ),
                        ));
                    }
                    clip.keys.push(k);
                    continue;
                }
                _ => return Err(err(line, format!("unknown statement `{stmt}`"))),
            }
            t.end()?;
        }
        let vox = vox.ok_or_else(|| RigError {
            line: None,
            message: "no `vox` line".into(),
        })?;
        let rig = Self {
            vox,
            rate: rate.unwrap_or(DEFAULT_RATE),
            materials,
            parts,
            clips,
        };
        rig.check_names()?;
        Ok(rig)
    }

    fn part_index(&self, name: &str) -> Option<usize> {
        self.parts.iter().position(|p| p.name == name)
    }

    fn check_names(&self) -> Result<(), RigError> {
        for p in &self.parts {
            if let Some(parent) = &p.parent
                && self.part_index(parent).is_none()
            {
                return Err(err(p.line.0, format!("unknown parent part `{parent}`")));
            }
        }
        // Walk up from each part; coming back to it is a cycle, blamed on the member declared
        // last (the line that closed it).
        for p in &self.parts {
            let mut members = vec![p];
            let mut at = p;
            while let Some(parent) = &at.parent {
                if *parent == p.name {
                    let last = members
                        .iter()
                        .max_by_key(|m| m.line.0)
                        .expect("a member at least");
                    let parent = last.parent.as_deref().unwrap_or_default();
                    return Err(err(
                        last.line.0,
                        format!(
                            "parent `{parent}` of part `{}` closes a parent cycle",
                            last.name
                        ),
                    ));
                }
                if members.len() > self.parts.len() {
                    break;
                }
                at = &self.parts[self.part_index(parent).expect("checked above")];
                members.push(at);
            }
        }
        for c in &self.clips {
            for k in &c.keys {
                if self.part_index(&k.part).is_none() {
                    return Err(err(k.line.0, format!("unknown part `{}`", k.part)));
                }
            }
        }
        Ok(())
    }

    /// The rig in a canonical form: `vox`, `rate`, the materials, the parts, then each clip
    /// and its keys, single spaces, lengths as the shortest exact decimal. Comments and
    /// alignment are not kept; [`Rig::parse`] reads it back to an equal rig.
    pub fn write(&self) -> String {
        let mut o = String::new();
        let _ = writeln!(o, "vox {}", self.vox);
        let _ = writeln!(o, "rate {}", self.rate);
        for m in &self.materials {
            let _ = writeln!(o, "material {} {}", m.index, m.name);
        }
        let triple = |o: &mut String, kw: &str, v: [i32; 3]| {
            let _ = write!(o, " {kw}");
            for c in v {
                o.push(' ');
                write_length(o, c);
            }
        };
        for p in &self.parts {
            let _ = write!(o, "part {}", p.name);
            match &p.source {
                VoxSource::Node(n) => {
                    let _ = write!(o, " node {n}");
                }
                VoxSource::Model(i) => {
                    let _ = write!(o, " model {i}");
                }
            }
            if let Some(parent) = &p.parent {
                let _ = write!(o, " parent {parent}");
            }
            if p.at != [0; 3] {
                triple(&mut o, "at", p.at);
            }
            triple(&mut o, "pivot", p.pivot);
            let _ = writeln!(o, " detail {}", p.detail);
        }
        for c in &self.clips {
            let _ = write!(o, "clip {} ticks {}", c.name, c.ticks);
            o.push_str(if c.looped { " loop\n" } else { "\n" });
            for k in &c.keys {
                let _ = write!(o, "key {} {}", k.tick, k.part);
                for (axis, deg) in &k.rots {
                    let _ = write!(o, " rot {} {deg}", axis.name());
                }
                if k.moved != [0; 3] {
                    triple(&mut o, "move", k.moved);
                }
                o.push('\n');
            }
        }
        o
    }

    /// Notes on a parsed rig that are not errors: a key pair on one part more than 90
    /// degrees apart, where the blend turns noticeably unevenly between them.
    pub fn warnings(&self) -> Vec<String> {
        let mut out = Vec::new();
        for c in &self.clips {
            let mut last: BTreeMap<&str, (u32, Quat)> = BTreeMap::new();
            for k in &c.keys {
                let q = key_rotation(&k.rots);
                if let Some(&(tick, prev)) = last.get(k.part.as_str()) {
                    // The turn between two rotations exceeds 90 degrees when
                    // |dot| < cos 45 degrees, that is 2 dot^2 < 1 in Q14 squared.
                    let dot: i64 = (0..4)
                        .map(|i| i64::from(prev.0[i]) * i64::from(q.0[i]))
                        .sum();
                    let one = 1i128 << 28;
                    if 2 * i128::from(dot) * i128::from(dot) < one * one {
                        out.push(format!(
                            "clip `{}`, part `{}`: the keys at ticks {tick} and {} are more \
                             than 90 degrees apart; the blend turns unevenly between them",
                            c.name, k.part, k.tick
                        ));
                    }
                }
                last.insert(&k.part, (k.tick, q));
            }
        }
        out
    }

    /// Builds the rig against its `.vox` file: each part's voxels through the palette map,
    /// each material name through `materials`. An unknown material name, a missing or
    /// ambiguous node name, a model index past the file's end, a palette index a part uses with
    /// no `material` line, or a part the model refuses is an error that names it.
    pub fn build(
        &self,
        vox: &VoxFile,
        materials: &dyn Fn(&str) -> Option<MaterialId>,
    ) -> Result<RigModel, RigError> {
        // A rig built by hand skipped `parse`'s checks.
        self.check_names()?;
        let mut map = VoxMap::new();
        for m in &self.materials {
            let id = materials(&m.name)
                .ok_or_else(|| err(m.line.0, format!("unknown material `{}`", m.name)))?;
            map.set(m.index, Slot::Material(id));
        }
        // Parents before children, file order otherwise.
        let mut order: Vec<usize> = Vec::with_capacity(self.parts.len());
        while order.len() < self.parts.len() {
            for (i, p) in self.parts.iter().enumerate() {
                let ready = p
                    .parent
                    .as_ref()
                    .is_none_or(|n| order.contains(&self.part_index(n).expect("checked")));
                if !order.contains(&i) && ready {
                    order.push(i);
                }
            }
        }
        let new_index = |file_index: usize| order.iter().position(|&j| j == file_index);
        let mut parts = Vec::with_capacity(order.len());
        let mut rest = Vec::with_capacity(order.len());
        for &i in &order {
            let p = &self.parts[i];
            let line = p.line.0;
            let model = self.model_of(vox, p)?;
            let prefab = map.to_prefab(&vox.models[model]).map_err(|u| {
                err(
                    line,
                    format!(
                        "part `{}` uses palette indices {:?} with no `material` line",
                        p.name, u.0
                    ),
                )
            })?;
            let parent = p.parent.as_ref().map(|n| {
                let fi = self.part_index(n).expect("checked");
                (fi, new_index(fi).expect("ordered first") as u8)
            });
            let parent_detail = parent.map_or(p.detail, |(fi, _)| self.parts[fi].detail);
            parts.push(Part {
                volume: std::sync::Arc::new(prefab),
                parent: parent.map(|(_, ni)| ni),
                pivot: p.pivot,
                detail: p.detail,
            });
            rest.push(to_world(p.at, parent_detail));
        }
        let model = Model::new(parts).map_err(|e| RigError {
            line: None,
            message: format!("the model refuses its parts: {e}"),
        })?;
        let mut pose = Pose::rest(&model);
        for (pp, t) in pose.parts.iter_mut().zip(rest) {
            pp.t = t;
        }
        let mut clips = Vec::with_capacity(self.clips.len());
        for c in &self.clips {
            let mut tracks: BTreeMap<usize, Vec<Key>> = BTreeMap::new();
            for k in &c.keys {
                let fi = self.part_index(&k.part).expect("checked");
                let p = &self.parts[fi];
                let parent_detail = p.parent.as_ref().map_or(p.detail, |n| {
                    self.parts[self.part_index(n).expect("checked")].detail
                });
                tracks
                    .entry(new_index(fi).expect("ordered"))
                    .or_default()
                    .push(Key {
                        tick: k.tick,
                        t: to_world(k.moved, parent_detail),
                        q: key_rotation(&k.rots),
                    });
            }
            let tracks = tracks
                .into_iter()
                .map(|(part, keys)| Track {
                    part: part as u8,
                    keys,
                })
                .collect();
            let clip = Clip::new(&model, c.ticks, c.looped, tracks)
                .map_err(|e| err(c.line.0, format!("clip `{}`: {e}", c.name)))?;
            clips.push((c.name.clone(), clip));
        }
        Ok(RigModel {
            model,
            part_names: order.iter().map(|&i| self.parts[i].name.clone()).collect(),
            rest: pose,
            clips,
        })
    }

    /// The index of the model a part takes its voxels from.
    fn model_of(&self, vox: &VoxFile, p: &RigPart) -> Result<usize, RigError> {
        let line = p.line.0;
        match &p.source {
            VoxSource::Model(i) => {
                let i = *i as usize;
                if i < vox.models.len() {
                    Ok(i)
                } else {
                    Err(err(
                        line,
                        format!("model `{i}` is past the file's {} models", vox.models.len()),
                    ))
                }
            }
            VoxSource::Node(name) => {
                let ids: Vec<u32> = vox
                    .names
                    .iter()
                    .filter(|(_, n)| *n == name)
                    .map(|(&id, _)| id)
                    .collect();
                let id = match ids[..] {
                    [id] => id,
                    [] => return Err(err(line, format!("no `.vox` node is named `{name}`"))),
                    _ => {
                        return Err(err(
                            line,
                            format!(
                                "{} `.vox` nodes are named `{name}`; use `model <index>`",
                                ids.len()
                            ),
                        ));
                    }
                };
                let shape = match vox.nodes.get(&id) {
                    Some(Node::Transform { child, .. }) => vox.nodes.get(child),
                    other => other,
                };
                match shape {
                    Some(Node::Shape { models }) if !models.is_empty() => {
                        let m = models[0] as usize;
                        if m < vox.models.len() {
                            Ok(m)
                        } else {
                            Err(err(
                                line,
                                format!("node `{name}` names a missing model {m}"),
                            ))
                        }
                    }
                    _ => Err(err(
                        line,
                        format!("node `{name}` does not hold a shape; use `model <index>`"),
                    )),
                }
            }
        }
    }
}

/// A length in 1/256 model voxel at `detail` as 1/256 world voxel, rounded half away from zero.
fn to_world(v: [i32; 3], detail: u8) -> [i32; 3] {
    let d = i32::from(detail);
    v.map(|c| {
        let q = (2 * c.abs() + d) / (2 * d);
        if c < 0 { -q } else { q }
    })
}

/// One whole-degree turn about an axis as a Q14 quaternion: `(cos h, sin h * axis)` with `h`
/// half the angle, from the committed table.
pub fn axis_rotation(axis: Axis, degrees: i32) -> Quat {
    // Half the angle in degrees is `degrees` half-degree steps.
    let (s, c) = sin_cos_half_degrees(i64::from(degrees));
    match axis {
        Axis::X => Quat([c, s, 0, 0]),
        Axis::Y => Quat([c, 0, s, 0]),
        Axis::Z => Quat([c, 0, 0, s]),
    }
}

/// A key's rotations composed in the order written: `r1 * r2 * ...`. No rotation is the
/// identity, and a single one is its table value exactly.
pub fn key_rotation(rots: &[(Axis, i32)]) -> Quat {
    let mut it = rots.iter().map(|&(a, d)| axis_rotation(a, d));
    let Some(first) = it.next() else {
        return Quat::IDENTITY;
    };
    it.fold(first, |acc, q| acc * q)
}
