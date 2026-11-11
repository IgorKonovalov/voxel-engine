//! Voxels drawn as text: a `.voxtext` file to a [`VoxFile`] and back.
//!
//! ```text
//! # a two-voxel-high post and a cap
//! color a 9 255 160 32       # character, palette index 1..255, red green blue
//! color b 1 90 90 90
//! model post at 0 0 0        # a named model; `at` is its .vox translation (default 0 0 0)
//! layer                      # z = 0, the bottom
//! .b.
//! bbb
//! layer                      # z = 1
//! .a.
//! ...
//! model cap
//! layer
//! aa
//! ```
//!
//! A model's layers run from the bottom up (`.vox` z, MagicaVoxel's up axis). In a layer the
//! first row is the largest y and the last row y = 0, so a layer reads as the view from above;
//! a row's first character is x = 0. Every row of a model has the same length (x), every layer
//! the same row count (y). `.` is an empty cell; a line that is exactly `layer` starts a
//! layer; `#` starts a comment, so neither is a colour character.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use vxe_world::Rot;

use crate::palette::DEFAULT_PALETTE;
use crate::vox::{MAX_MODEL_EDGE, Node, VoxFile, VoxModel};

/// The characters [`VoxText::from_vox_file`] assigns to palette indices, in order.
const CHARS: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789\
                     !\"$%&'()*+,-/:;<=>?@[\\]^_`{|}~";

/// A `.voxtext` error: the line (1-based) and a message that quotes the token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VoxTextError {
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for VoxTextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "voxtext line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for VoxTextError {}

fn err(line: usize, message: impl Into<String>) -> VoxTextError {
    VoxTextError {
        line,
        message: message.into(),
    }
}

/// A `color` line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextColor {
    pub ch: char,
    pub index: u8,
    pub rgb: [u8; 3],
}

/// One model: its name, its `.vox` translation, its size and one palette index a cell (0 is
/// empty), x fastest, then y, then z.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TextModel {
    pub name: String,
    pub at: [i32; 3],
    pub size: [u32; 3],
    pub cells: Vec<u8>,
}

/// A parsed `.voxtext`. Equality compares palette indices and colours, not the characters
/// chosen for them.
#[derive(Clone, Debug, Eq)]
pub struct VoxText {
    pub colors: Vec<TextColor>,
    pub models: Vec<TextModel>,
}

impl PartialEq for VoxText {
    fn eq(&self, other: &Self) -> bool {
        let key = |t: &Self| {
            t.colors
                .iter()
                .map(|c| (c.index, c.rgb))
                .collect::<BTreeMap<_, _>>()
        };
        self.models == other.models && key(self) == key(other)
    }
}

/// A model being read: its rows per layer.
struct Drawing {
    name: String,
    at: [i32; 3],
    line: usize,
    layers: Vec<Vec<Vec<u8>>>,
}

fn finish(d: Drawing) -> Result<TextModel, VoxTextError> {
    let Some(first) = d.layers.first() else {
        return Err(err(d.line, format!("model `{}` has no `layer`", d.name)));
    };
    let sy = first.len();
    let sx = first.first().map_or(0, Vec::len);
    let sz = d.layers.len();
    if sy == 0 {
        return Err(err(
            d.line,
            format!("model `{}` has an empty layer", d.name),
        ));
    }
    if d.layers.iter().any(|l| l.len() != sy) {
        return Err(err(
            d.line,
            format!("model `{}`'s layers do not all have {sy} rows", d.name),
        ));
    }
    let edge = MAX_MODEL_EDGE as usize;
    if sx > edge || sy > edge || sz > edge {
        return Err(err(
            d.line,
            format!(
                "model `{}` is {sx} x {sy} x {sz}, at most {edge} a side",
                d.name
            ),
        ));
    }
    let mut cells = vec![0u8; sx * sy * sz];
    for (z, layer) in d.layers.iter().enumerate() {
        for (r, row) in layer.iter().enumerate() {
            let y = sy - 1 - r;
            cells[sx * (y + sy * z)..][..sx].copy_from_slice(row);
        }
    }
    Ok(TextModel {
        name: d.name,
        at: d.at,
        size: [sx as u32, sy as u32, sz as u32],
        cells,
    })
}

impl VoxText {
    /// Parses a `.voxtext`. Every error carries its line.
    pub fn parse(text: &str) -> Result<Self, VoxTextError> {
        let mut colors: Vec<TextColor> = Vec::new();
        let mut models = Vec::new();
        let mut cur: Option<Drawing> = None;
        for (i, raw) in text.lines().enumerate() {
            let line = i + 1;
            let body = raw.split_once('#').map_or(raw, |(b, _)| b).trim();
            if body.is_empty() {
                continue;
            }
            let mut toks = body.split_ascii_whitespace();
            let first = toks.next().unwrap_or_default();
            let rest: Vec<&str> = toks.collect();
            match (first, rest.is_empty()) {
                ("layer", true) => {
                    let Some(d) = cur.as_mut() else {
                        return Err(err(line, "`layer` before any `model`"));
                    };
                    d.layers.push(Vec::new());
                }
                ("color", false) => {
                    if cur.is_some() || !models.is_empty() {
                        return Err(err(line, "`color` after a `model`; colours come first"));
                    }
                    colors.push(parse_color(line, &rest, &colors)?);
                }
                ("model", false) => {
                    if let Some(d) = cur.take() {
                        models.push(finish(d)?);
                    }
                    cur = Some(parse_model(line, &rest)?);
                }
                (_, false) => return Err(err(line, format!("unknown statement `{first}`"))),
                (row, true) => {
                    let Some(d) = cur.as_mut().filter(|d| !d.layers.is_empty()) else {
                        return Err(err(line, format!("row `{row}` outside a `layer`")));
                    };
                    let cells = row
                        .chars()
                        .map(|ch| match ch {
                            '.' => Ok(0),
                            _ => colors
                                .iter()
                                .find(|c| c.ch == ch)
                                .map(|c| c.index)
                                .ok_or_else(|| err(line, format!("`{ch}` has no `color` line"))),
                        })
                        .collect::<Result<Vec<u8>, _>>()?;
                    let width = d.layers.iter().find_map(|l| l.first()).map(Vec::len);
                    if let Some(w) = width
                        && w != cells.len()
                    {
                        return Err(err(
                            line,
                            format!("row `{row}` is {} cells, the model's rows {w}", cells.len()),
                        ));
                    }
                    d.layers.last_mut().expect("checked above").push(cells);
                }
            }
        }
        if let Some(d) = cur.take() {
            models.push(finish(d)?);
        }
        Ok(Self { colors, models })
    }

    /// The text in a canonical form: the colours, then each model with `at` when it is not
    /// zero, then its layers. [`VoxText::parse`] reads it back to an equal value.
    pub fn write(&self) -> String {
        let mut o = String::new();
        for c in &self.colors {
            let [r, g, b] = c.rgb;
            let _ = writeln!(o, "color {} {} {r} {g} {b}", c.ch, c.index);
        }
        let ch: BTreeMap<u8, char> = self.colors.iter().map(|c| (c.index, c.ch)).collect();
        for m in &self.models {
            let _ = write!(o, "model {}", m.name);
            if m.at != [0; 3] {
                let [x, y, z] = m.at;
                let _ = write!(o, " at {x} {y} {z}");
            }
            o.push('\n');
            let [sx, sy, sz] = m.size.map(|s| s as usize);
            for z in 0..sz {
                o.push_str("layer\n");
                for y in (0..sy).rev() {
                    for &c in &m.cells[sx * (y + sy * z)..][..sx] {
                        o.push(if c == 0 {
                            '.'
                        } else {
                            ch.get(&c).copied().unwrap_or('?')
                        });
                    }
                    o.push('\n');
                }
            }
        }
        o
    }

    /// The `.vox` file: version 200; the models in order, each one's voxels x fastest, then y,
    /// then z; MagicaVoxel's default palette with each `color` written over its index (alpha
    /// 255); and a scene graph of a root transform (node 0), a group (node 1) and per model a
    /// transform named after it at its `at` (node `2 + 2k`) over a shape (node `3 + 2k`).
    pub fn to_vox_file(&self) -> VoxFile {
        let mut palette = DEFAULT_PALETTE;
        for c in &self.colors {
            let [r, g, b] = c.rgb;
            palette[usize::from(c.index)] = [r, g, b, 255];
        }
        palette[0] = [0; 4];
        let mut nodes = BTreeMap::new();
        let mut names = BTreeMap::new();
        nodes.insert(
            0,
            Node::Transform {
                child: 1,
                translation: [0; 3],
                rot: Rot::IDENTITY,
            },
        );
        let mut children = Vec::new();
        let mut models = Vec::new();
        for (k, m) in self.models.iter().enumerate() {
            let id = 2 + 2 * k as u32;
            children.push(id);
            nodes.insert(
                id,
                Node::Transform {
                    child: id + 1,
                    translation: m.at,
                    rot: Rot::IDENTITY,
                },
            );
            nodes.insert(
                id + 1,
                Node::Shape {
                    models: vec![k as u32],
                },
            );
            names.insert(id, m.name.clone());
            let [sx, sy, _] = m.size.map(|s| s as usize);
            let voxels = m
                .cells
                .iter()
                .enumerate()
                .filter(|&(_, &c)| c != 0)
                .map(|(at, &c)| {
                    let (x, y, z) = (at % sx, at / sx % sy, at / (sx * sy));
                    [x as u8, y as u8, z as u8, c]
                })
                .collect();
            models.push(VoxModel {
                size: m.size,
                voxels,
            });
        }
        nodes.insert(1, Node::Group { children });
        VoxFile {
            version: 200,
            models,
            palette,
            nodes,
            names,
            imap_present: false,
        }
    }

    /// The text of a `.vox` file. A model's name is the `_name` of the transform over the
    /// shape that holds it (`model<k>` when there is none, whitespace and `#` replaced by
    /// `_`), and its `at` that transform's translation; a rotation in the scene graph is not
    /// kept. Colours are listed for every palette index a model uses or whose colour differs
    /// from MagicaVoxel's default, with characters assigned in index order. Fails when more
    /// indices need a character than there are.
    pub fn from_vox_file(vox: &VoxFile) -> Result<Self, VoxTextError> {
        // Per model, the transform over the first shape that holds it.
        let mut owner: BTreeMap<u32, u32> = BTreeMap::new();
        for (&id, node) in &vox.nodes {
            if let Node::Transform { child, .. } = node
                && let Some(Node::Shape { models }) = vox.nodes.get(child)
            {
                for &m in models {
                    owner.entry(m).or_insert(id);
                }
            }
        }
        let mut used = [false; 256];
        let mut models = Vec::with_capacity(vox.models.len());
        for (k, m) in vox.models.iter().enumerate() {
            let [sx, sy, sz] = m.size.map(|s| s as usize);
            let mut cells = vec![0u8; sx * sy * sz];
            for &[x, y, z, c] in &m.voxels {
                let (x, y, z) = (usize::from(x), usize::from(y), usize::from(z));
                if x < sx && y < sy && z < sz {
                    cells[x + sx * (y + sy * z)] = c;
                }
            }
            for &c in &cells {
                used[usize::from(c)] = true;
            }
            let tr = owner.get(&(k as u32));
            let name = tr
                .and_then(|id| vox.names.get(id))
                .map(|n| {
                    n.chars()
                        .map(|c| {
                            if c.is_whitespace() || c == '#' {
                                '_'
                            } else {
                                c
                            }
                        })
                        .collect::<String>()
                })
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| format!("model{k}"));
            let at = match tr.and_then(|id| vox.nodes.get(id)) {
                Some(Node::Transform { translation, .. }) => *translation,
                _ => [0; 3],
            };
            models.push(TextModel {
                name,
                at,
                size: m.size,
                cells,
            });
        }
        let mut chars = CHARS.chars();
        let mut colors = Vec::new();
        for index in 1..=255u8 {
            let i = usize::from(index);
            let [r, g, b, a] = vox.palette[i];
            let default = DEFAULT_PALETTE[i];
            if !used[i] && [r, g, b, a] == default {
                continue;
            }
            let ch = chars.next().ok_or_else(|| {
                err(
                    0,
                    format!("more palette indices than the {} characters", CHARS.len()),
                )
            })?;
            colors.push(TextColor {
                ch,
                index,
                rgb: [r, g, b],
            });
        }
        Ok(Self { colors, models })
    }
}

fn parse_color(line: usize, rest: &[&str], prior: &[TextColor]) -> Result<TextColor, VoxTextError> {
    let [ch, index, r, g, b] = rest else {
        return Err(err(
            line,
            "`color` takes a character, an index and three colour bytes",
        ));
    };
    let mut cs = ch.chars();
    let (Some(c), None) = (cs.next(), cs.next()) else {
        return Err(err(line, format!("`{ch}` is not one character")));
    };
    if c == '.' || c == '#' {
        return Err(err(line, format!("`{c}` is reserved")));
    }
    let index: u8 = index.parse().ok().filter(|&i| i != 0).ok_or_else(|| {
        err(
            line,
            format!("`{index}` is not a palette index from 1 to 255"),
        )
    })?;
    let byte = |t: &str| {
        t.parse::<u8>()
            .map_err(|_| err(line, format!("`{t}` is not a colour byte")))
    };
    let rgb = [byte(r)?, byte(g)?, byte(b)?];
    if prior.iter().any(|p| p.ch == c) {
        return Err(err(line, format!("`{c}` given twice")));
    }
    if prior.iter().any(|p| p.index == index) {
        return Err(err(line, format!("palette index `{index}` given twice")));
    }
    Ok(TextColor { ch: c, index, rgb })
}

fn parse_model(line: usize, rest: &[&str]) -> Result<Drawing, VoxTextError> {
    let (name, at) = match rest {
        [name] => (*name, [0; 3]),
        [name, "at", x, y, z] => {
            let int = |t: &str| {
                t.parse::<i32>()
                    .ok()
                    .filter(|v| v.unsigned_abs() <= crate::vox::MAX_TRANSLATION.unsigned_abs())
                    .ok_or_else(|| err(line, format!("`{t}` is not a translation")))
            };
            (*name, [int(x)?, int(y)?, int(z)?])
        }
        _ => return Err(err(line, "`model` takes a name and an optional `at x y z`")),
    };
    Ok(Drawing {
        name: name.to_owned(),
        at,
        line,
        layers: Vec::new(),
    })
}
