//! The MagicaVoxel `.vox` parser.
//!
//! A file is `"VOX "`, a little-endian `u32` version (150 or 200), then one `MAIN` chunk
//! whose children are the data chunks. Every chunk is a four-byte id, the byte counts of its
//! content (`N`) and of its children (`M`), then `N + M` bytes ("MagicaVoxel .vox File
//! Format", section 2). This parser reads `PACK`, `SIZE` and `XYZI` pairs, `RGBA`, `MATL`
//! (a palette entry's material), `LAYR` (layers) and the scene graph (`nTRN`, `nGRP`, `nSHP`,
//! from the format's extension document), and skips every other chunk by its declared sizes.
//! Every multi-byte integer is little-endian.

use std::collections::{BTreeMap, BTreeSet};

use vxe_world::{Rot, VoxelCoord};

use crate::palette::DEFAULT_PALETTE;

/// The largest model edge accepted, in voxels. `XYZI` coordinates are `u8`, so no voxel
/// sits past index 255 on an axis; a longer axis could only add empty cells.
pub const MAX_MODEL_EDGE: u32 = 256;

/// The most placements a scene graph may make. A subtree is placed once per reference, so
/// without a cap a group that lists the same child twice per level doubles the count per
/// level of a small file.
pub const MAX_PLACEMENTS: usize = 65_536;

/// The largest magnitude of a `_t` component. Composing up to 2047 nested transforms of this
/// size stays inside `i32`; a deeper chain whose sum leaves `i32` is refused.
pub const MAX_TRANSLATION: i32 = 1 << 20;

/// Why a file did not parse.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum VoxError {
    /// The first four bytes are not `"VOX "`.
    Magic([u8; 4]),
    /// A version other than 150 or 200.
    Version(u32),
    /// Chunk `chunk` needed bytes the file (or its parent chunk) does not have; `at` is the
    /// file offset of the first missing byte.
    Truncated { chunk: [u8; 4], at: usize },
    /// A model edge over [`MAX_MODEL_EDGE`].
    TooLarge { size: [u32; 3] },
    /// A transform's `_r` byte that is not a rotation (a repeated axis or a reflection).
    Rotation(u8),
    /// A chunk whose content contradicts the format.
    Malformed { chunk: [u8; 4], what: &'static str },
    /// The scene graph names a missing node or model, has a cycle, or lacks node 0.
    SceneGraph { node: u32, what: &'static str },
}

impl std::fmt::Display for VoxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let id = |c: &[u8; 4]| String::from_utf8_lossy(c).into_owned();
        match self {
            Self::Magic(m) => write!(f, "not a .vox file (magic {m:?})"),
            Self::Version(v) => write!(f, ".vox version {v} is not 150 or 200"),
            Self::Truncated { chunk, at } => {
                write!(f, "chunk {} cut short at byte {at}", id(chunk))
            }
            Self::TooLarge { size } => {
                write!(f, "model size {size:?} exceeds {MAX_MODEL_EDGE} on an axis")
            }
            Self::Rotation(r) => write!(f, "_r byte {r} is not a rotation"),
            Self::Malformed { chunk, what } => write!(f, "chunk {}: {what}", id(chunk)),
            Self::SceneGraph { node, what } => write!(f, "scene graph node {node}: {what}"),
        }
    }
}

impl std::error::Error for VoxError {}

/// One model: its size and its voxels as `(x, y, z, palette index)` in file order.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct VoxModel {
    pub size: [u32; 3],
    pub voxels: Vec<[u8; 4]>,
}

/// A scene-graph node, frame 0 only.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Node {
    Transform {
        child: u32,
        translation: [i32; 3],
        rot: Rot,
    },
    Group {
        children: Vec<u32>,
    },
    Shape {
        models: Vec<u32>,
    },
}

/// Where the scene graph puts one model: the model's centre (see
/// [`Placement::min_corner`]) at `translation`, turned by `rot`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Placement {
    pub model: usize,
    pub translation: [i32; 3],
    pub rot: Rot,
}

impl Placement {
    /// The minimum corner of the model's rotated box. MagicaVoxel's translation places the
    /// model's centre, taken as `size / 2` in integer division per axis of the rotated size,
    /// so a stamp at this corner reproduces the editor's layout.
    /// The subtraction wraps, so a hand-built placement near `i32::MIN` cannot panic; a
    /// parsed file's translations are far inside the range.
    pub fn min_corner(&self, rotated_size: [u32; 3]) -> VoxelCoord {
        VoxelCoord(
            [0, 1, 2].map(|k| self.translation[k].wrapping_sub((rotated_size[k] / 2) as i32)),
        )
    }
}

/// What a palette entry's `MATL` chunk makes it, as MagicaVoxel's renderer reads it. The
/// values are the chunk's own, unscaled.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum VoxMaterialKind {
    /// `_diffuse`, a `_blend` or an unknown `_type`, and every entry with no `MATL`.
    #[default]
    Diffuse,
    /// `_metal`: `_rough`.
    Metal { rough: f32 },
    /// `_glass`: `_trans` (or the older `_alpha`), the share of light let through, and `_rough`.
    Glass { trans: f32, rough: f32 },
    /// `_emit`: `_emit` (or the older `_weight`) and `_flux`, MagicaVoxel's power step 0 to 4.
    Emit { emit: f32, flux: u8 },
    /// `_media` and `_cloud`: `_trans` (or `_alpha`).
    Media { trans: f32 },
}

/// One palette entry's material.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VoxMaterial {
    pub kind: VoxMaterialKind,
}

/// A parsed `.vox` file.
#[derive(Clone, Debug, PartialEq)]
pub struct VoxFile {
    pub version: u32,
    /// Models in file order; a model's id is its index here.
    pub models: Vec<VoxModel>,
    /// The colour of each palette index as `[r, g, b, a]`; index 0 is empty and
    /// `[0, 0, 0, 0]`. `RGBA` entry `k` is index `k + 1`. MagicaVoxel's default palette when
    /// the file has no `RGBA`.
    pub palette: [[u8; 4]; 256],
    /// The material of each palette index, from the file's `MATL` chunks (a chunk's id is the
    /// palette index it describes); [`VoxMaterialKind::Diffuse`] where there is none.
    pub materials: [VoxMaterial; 256],
    /// Scene-graph nodes by id; empty when the file has no scene graph.
    pub nodes: BTreeMap<u32, Node>,
    /// Scene-graph node names by node id, from each node's `_name` attribute; a node without
    /// one has no entry. MagicaVoxel lets a node go unnamed and lets two share a name.
    pub names: BTreeMap<u32, String>,
    /// The file has an `IMAP` chunk, which remaps palette indices in the editor. This parser
    /// ignores it, so a map written against the editor's displayed order may be wrong; a
    /// caller can refuse such files.
    pub imap_present: bool,
    /// Transform nodes whose frame-0 `_r` byte is a reflection (determinant -1), with the
    /// byte. The node's `rot` holds the reflection composed with a mirror of the child's x
    /// axis ([`decode_frame`]). [`VoxFile::try_placements`] refuses such a file.
    pub reflected: BTreeMap<u32, u8>,
    /// Nodes whose `_hidden` attribute is 1.
    pub hidden: BTreeSet<u32>,
    /// Transform nodes' layer ids where they differ from MagicaVoxel's default (-1 for node 0,
    /// 0 for every other transform); [`VoxFile::layer_of`] reads them.
    pub node_layers: BTreeMap<u32, i32>,
    /// The `LAYR` chunks by layer id.
    pub layers: BTreeMap<i32, VoxLayer>,
}

/// A `LAYR` chunk: the layer's `_name` (empty when it has none) and its `_hidden` flag.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct VoxLayer {
    pub name: String,
    pub hidden: bool,
}

impl Default for VoxFile {
    /// A version-200 file with no models, MagicaVoxel's default palette, no materials and no
    /// scene graph.
    fn default() -> Self {
        Self {
            version: 200,
            models: Vec::new(),
            palette: DEFAULT_PALETTE,
            materials: [VoxMaterial::default(); 256],
            nodes: BTreeMap::new(),
            names: BTreeMap::new(),
            imap_present: false,
            reflected: BTreeMap::new(),
            hidden: BTreeSet::new(),
            node_layers: BTreeMap::new(),
            layers: BTreeMap::new(),
        }
    }
}

const MAIN: [u8; 4] = *b"MAIN";

/// One DICT entry: key and value bytes.
type Pair<'a> = (&'a [u8], &'a [u8]);

/// A bounded cursor over the file. `end` is the end of the chunk (or file) being read, and
/// a read past it is [`VoxError::Truncated`] for `chunk`.
struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
    end: usize,
    chunk: [u8; 4],
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], VoxError> {
        if self.end - self.pos < n {
            return Err(VoxError::Truncated {
                chunk: self.chunk,
                at: self.end,
            });
        }
        let s = &self.b[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn id(&mut self) -> Result<[u8; 4], VoxError> {
        let s = self.take(4)?;
        Ok([s[0], s[1], s[2], s[3]])
    }

    fn u32(&mut self) -> Result<u32, VoxError> {
        Ok(u32::from_le_bytes(self.id()?))
    }

    fn i32(&mut self) -> Result<i32, VoxError> {
        Ok(i32::from_le_bytes(self.id()?))
    }

    /// An `i32` that must not be negative (a count or an id).
    fn count(&mut self, what: &'static str) -> Result<u32, VoxError> {
        let v = self.i32()?;
        u32::try_from(v).map_err(|_| self.malformed(what))
    }

    fn malformed(&self, what: &'static str) -> VoxError {
        VoxError::Malformed {
            chunk: self.chunk,
            what,
        }
    }

    /// A STRING: an `i32` byte count, then the bytes.
    fn string(&mut self) -> Result<&'a [u8], VoxError> {
        let n = self.count("negative string length")?;
        self.take(n as usize)
    }

    /// A DICT: an `i32` pair count, then key and value STRINGs.
    fn dict(&mut self) -> Result<Vec<Pair<'a>>, VoxError> {
        let n = self.count("negative dictionary size")?;
        let mut out = Vec::new();
        for _ in 0..n {
            let k = self.string()?;
            let v = self.string()?;
            out.push((k, v));
        }
        Ok(out)
    }
}

/// A transform frame's `_r` byte as a rotation. Extension document, "(c) ROTATION type":
/// bits 0 to 1 hold the index of the non-zero entry of row 1, bits 2 to 3 that of row 2 (row
/// 3 takes the remaining index), and bits 4, 5 and 6 the signs of rows 1, 2 and 3 (set is
/// negative). Byte 4 is the identity.
pub fn decode_rotation(r: u8) -> Result<Rot, VoxError> {
    let a0 = r & 3;
    let a1 = (r >> 2) & 3;
    if a0 > 2 || a1 > 2 || a0 == a1 {
        return Err(VoxError::Rotation(r));
    }
    let axis = [a0, a1, 3 - a0 - a1];
    let neg = [r >> 4 & 1 != 0, r >> 5 & 1 != 0, r >> 6 & 1 != 0];
    Rot::from_rows(axis, neg).ok_or(VoxError::Rotation(r))
}

fn parse_int(s: &[u8]) -> Option<i64> {
    std::str::from_utf8(s).ok()?.trim().parse().ok()
}

/// A transform frame's `_r` byte as a rotation, and whether the byte is a reflection: for a
/// reflection (determinant -1) the rotation is the byte's matrix times a mirror of x
/// (`R = rot * diag(-1, 1, 1)`, so `R v = rot (-v.x, v.y, v.z)`), a proper rotation. A byte
/// whose rows repeat an axis is [`VoxError::Rotation`].
pub fn decode_frame(r: u8) -> Result<(Rot, bool), VoxError> {
    let a0 = r & 3;
    let a1 = (r >> 2) & 3;
    if a0 > 2 || a1 > 2 || a0 == a1 {
        return Err(VoxError::Rotation(r));
    }
    let axis = [a0, a1, 3 - a0 - a1];
    let mut neg = [r >> 4 & 1 != 0, r >> 5 & 1 != 0, r >> 6 & 1 != 0];
    if let Some(rot) = Rot::from_rows(axis, neg) {
        return Ok((rot, false));
    }
    // The row reading x changes sign: the matrix times diag(-1, 1, 1).
    let k = axis
        .iter()
        .position(|&a| a == 0)
        .expect("a permutation holds 0");
    neg[k] = !neg[k];
    Rot::from_rows(axis, neg)
        .map(|rot| (rot, true))
        .ok_or(VoxError::Rotation(r))
}

/// Frame 0's `_t` and `_r`, defaulting to no translation and the identity, and the `_r`
/// byte when it is a reflection ([`decode_frame`]).
fn frame(r: &Reader<'_>, attrs: &[Pair<'_>]) -> Result<([i32; 3], Rot, Option<u8>), VoxError> {
    let mut t = [0; 3];
    let mut rot = Rot::IDENTITY;
    let mut reflected = None;
    for &(k, v) in attrs {
        match k {
            b"_t" => {
                let s = std::str::from_utf8(v).map_err(|_| r.malformed("_t is not text"))?;
                let mut it = s.split_ascii_whitespace().map(str::parse::<i32>);
                for c in &mut t {
                    *c = it
                        .next()
                        .and_then(Result::ok)
                        .ok_or_else(|| r.malformed("_t is not three integers"))?;
                }
                if it.next().is_some() {
                    return Err(r.malformed("_t is not three integers"));
                }
                if t.iter()
                    .any(|c| c.unsigned_abs() > MAX_TRANSLATION.unsigned_abs())
                {
                    return Err(r.malformed("_t out of range"));
                }
            }
            b"_r" => {
                let b = parse_int(v)
                    .and_then(|b| u8::try_from(b).ok())
                    .ok_or_else(|| r.malformed("_r is not a byte"))?;
                let (proper, mirror) = decode_frame(b)?;
                rot = proper;
                reflected = mirror.then_some(b);
            }
            _ => {}
        }
    }
    Ok((t, rot, reflected))
}

/// What one scene-graph chunk says beside its node: its `_name`, `_hidden`, a transform's
/// layer id and the `_r` byte of a reflected frame 0.
struct NodeExtra {
    name: Option<String>,
    hidden: bool,
    layer: Option<i32>,
    reflected: Option<u8>,
}

/// Parses one scene-graph chunk's content into `(id, node, extra)`, the name from the node's
/// `_name` attribute (MagicaVoxel writes it on transforms), its bytes read as UTF-8 with
/// replacement characters.
fn node(r: &mut Reader<'_>) -> Result<(u32, Node, NodeExtra), VoxError> {
    let id = r.count("negative node id")?;
    let attrs = r.dict()?;
    let name = attrs
        .iter()
        .find(|(k, _)| *k == b"_name")
        .map(|(_, v)| String::from_utf8_lossy(v).into_owned());
    let hidden = attrs
        .iter()
        .any(|(k, v)| *k == b"_hidden" && parse_int(v) == Some(1));
    let mut extra = NodeExtra {
        name,
        hidden,
        layer: None,
        reflected: None,
    };
    let node = match &r.chunk {
        b"nTRN" => {
            let child = r.count("negative child id")?;
            let _reserved = r.i32()?;
            let layer = r.i32()?;
            extra.layer = Some(layer);
            let frames = r.count("negative frame count")?;
            let mut first = ([0; 3], Rot::IDENTITY, None);
            for f in 0..frames {
                let attrs = r.dict()?;
                if f == 0 {
                    first = frame(r, &attrs)?;
                }
            }
            extra.reflected = first.2;
            Node::Transform {
                child,
                translation: first.0,
                rot: first.1,
            }
        }
        b"nGRP" => {
            let n = r.count("negative child count")?;
            let children = (0..n)
                .map(|_| r.count("negative child id"))
                .collect::<Result<_, _>>()?;
            Node::Group { children }
        }
        _ => {
            let n = r.count("negative model count")?;
            let mut models = Vec::new();
            for _ in 0..n {
                models.push(r.count("negative model id")?);
                let _attrs = r.dict()?;
            }
            Node::Shape { models }
        }
    };
    Ok((id, node, extra))
}

/// A `LAYR` chunk: its id and layer.
fn layr(c: &mut Reader<'_>) -> Result<(i32, VoxLayer), VoxError> {
    let id = c.i32()?;
    let attrs = c.dict()?;
    let mut layer = VoxLayer::default();
    for (k, v) in attrs {
        match k {
            b"_name" => layer.name = String::from_utf8_lossy(v).into_owned(),
            b"_hidden" => layer.hidden = parse_int(v) == Some(1),
            _ => {}
        }
    }
    Ok((id, layer))
}

/// MagicaVoxel's layer id for transform `id` when the file gives none other: -1 for the root,
/// 0 for every other transform.
pub(crate) fn default_layer(id: u32) -> i32 {
    if id == 0 { -1 } else { 0 }
}

/// Reads the header and `MAIN`'s header: the version, and a reader over `MAIN`'s children.
fn main_chunk(bytes: &[u8]) -> Result<(u32, Reader<'_>), VoxError> {
    let mut r = Reader {
        b: bytes,
        pos: 0,
        end: bytes.len(),
        chunk: *b"VOX ",
    };
    let magic = r.id()?;
    if &magic != b"VOX " {
        return Err(VoxError::Magic(magic));
    }
    let version = r.u32()?;
    if version != 150 && version != 200 {
        return Err(VoxError::Version(version));
    }
    r.chunk = MAIN;
    let id = r.id()?;
    if id != MAIN {
        return Err(VoxError::Malformed {
            chunk: id,
            what: "the first chunk is not MAIN",
        });
    }
    let n = r.u32()? as usize;
    let m = r.u32()? as usize;
    r.take(n)?;
    // Data after MAIN's children is ignored.
    let end = r.pos.checked_add(m).filter(|&e| e <= bytes.len());
    let Some(end) = end else {
        return Err(VoxError::Truncated {
            chunk: MAIN,
            at: bytes.len(),
        });
    };
    r.end = end;
    Ok((version, r))
}

/// The next chunk under `r`: a reader over its content, and where the chunk (content and
/// children) ends.
fn next_chunk<'a>(r: &mut Reader<'a>) -> Result<(Reader<'a>, usize), VoxError> {
    r.chunk = MAIN;
    let id = r.id()?;
    r.chunk = id;
    let n = r.u32()? as usize;
    let m = r.u32()? as usize;
    let start = r.pos;
    let content_end = start.checked_add(n).filter(|&e| e <= r.end);
    let chunk_end = content_end
        .and_then(|e| e.checked_add(m))
        .filter(|&e| e <= r.end);
    let (Some(content_end), Some(chunk_end)) = (content_end, chunk_end) else {
        return Err(VoxError::Truncated {
            chunk: id,
            at: r.end,
        });
    };
    let c = Reader {
        b: r.b,
        pos: start,
        end: content_end,
        chunk: id,
    };
    Ok((c, chunk_end))
}

/// A `SIZE` chunk's model size.
fn size(c: &mut Reader<'_>) -> Result<[u32; 3], VoxError> {
    let s = [c.i32()?, c.i32()?, c.i32()?];
    if s.iter().any(|&v| v < 0) {
        return Err(c.malformed("negative model size"));
    }
    let s = s.map(|v| v as u32);
    if s.iter().any(|&v| v > MAX_MODEL_EDGE) {
        return Err(VoxError::TooLarge { size: s });
    }
    Ok(s)
}

/// An `XYZI` chunk's voxels, for a model of `size`.
fn xyzi(c: &mut Reader<'_>, size: [u32; 3]) -> Result<VoxModel, VoxError> {
    let count = c.count("negative voxel count")? as usize;
    let raw = c.take(count.saturating_mul(4))?;
    let mut voxels = Vec::with_capacity(count);
    for v in raw.chunks_exact(4) {
        let v = [v[0], v[1], v[2], v[3]];
        if (0..3).any(|k| u32::from(v[k]) >= size[k]) {
            return Err(c.malformed("voxel outside the model size"));
        }
        voxels.push(v);
    }
    Ok(VoxModel { size, voxels })
}

/// An `RGBA` chunk into `palette`.
fn rgba(c: &mut Reader<'_>, palette: &mut [[u8; 4]; 256]) -> Result<(), VoxError> {
    let raw = c.take(256 * 4)?;
    // Section 7: entry k is the colour of index k + 1; the last entry has no index.
    for (k, e) in raw.chunks_exact(4).take(255).enumerate() {
        palette[k + 1] = [e[0], e[1], e[2], e[3]];
    }
    palette[0] = [0; 4];
    Ok(())
}

/// A `MATL` chunk: its id (the palette index) and the material its properties describe. An
/// id past 255 is returned as is and ignored by the caller; a property that is not a number
/// is [`VoxError::Malformed`].
fn matl(c: &mut Reader<'_>) -> Result<(usize, VoxMaterial), VoxError> {
    let id = c.count("negative material id")? as usize;
    let attrs = c.dict()?;
    let mut ty: &[u8] = b"_diffuse";
    let mut num: BTreeMap<&[u8], f32> = BTreeMap::new();
    for &(k, v) in &attrs {
        if k == b"_type" {
            ty = v;
        } else {
            let parsed = std::str::from_utf8(v)
                .ok()
                .and_then(|s| s.trim().parse::<f32>().ok());
            match parsed {
                Some(f) if f.is_finite() => {
                    num.insert(k, f);
                }
                // MagicaVoxel writes only numbers past `_type`; a key the mapping reads must
                // be one, any other is skipped.
                _ if matches!(
                    k,
                    b"_rough" | b"_trans" | b"_alpha" | b"_emit" | b"_weight" | b"_flux"
                ) =>
                {
                    return Err(c.malformed("a material property is not a number"));
                }
                _ => {}
            }
        }
    }
    let get = |k: &[u8]| num.get(k).copied();
    let trans = get(b"_trans").or_else(|| get(b"_alpha"));
    let kind = match ty {
        b"_metal" => VoxMaterialKind::Metal {
            rough: get(b"_rough").unwrap_or(DEFAULT_ROUGH),
        },
        b"_glass" => VoxMaterialKind::Glass {
            trans: trans.unwrap_or(0.0),
            rough: get(b"_rough").unwrap_or(DEFAULT_ROUGH),
        },
        b"_emit" => VoxMaterialKind::Emit {
            emit: get(b"_emit").or_else(|| get(b"_weight")).unwrap_or(0.0),
            flux: get(b"_flux").map_or(0, |f| f.round().clamp(0.0, 255.0) as u8),
        },
        b"_media" | b"_cloud" => VoxMaterialKind::Media {
            trans: trans.unwrap_or(0.0),
        },
        _ => VoxMaterialKind::Diffuse,
    };
    Ok((id, VoxMaterial { kind }))
}

/// `_rough` when a `_metal` or `_glass` entry leaves it out: MagicaVoxel's slider default.
pub const DEFAULT_ROUGH: f32 = 0.1;

impl VoxFile {
    pub fn parse(bytes: &[u8]) -> Result<Self, VoxError> {
        let (version, mut r) = main_chunk(bytes)?;
        let mut file = Self {
            version,
            ..Self::default()
        };
        let mut pending: Option<[u32; 3]> = None;
        while r.pos < r.end {
            let (mut c, chunk_end) = next_chunk(&mut r)?;
            file.read_chunk(&mut c, &mut pending)?;
            r.pos = chunk_end;
        }
        if pending.is_some() {
            return Err(VoxError::Malformed {
                chunk: *b"SIZE",
                what: "SIZE without its XYZI",
            });
        }
        file.walk(true, &mut |_| {})?;
        Ok(file)
    }

    /// Reads one data chunk by its id; `pending` is the `SIZE` waiting for its `XYZI`. Other
    /// ids are skipped.
    fn read_chunk(
        &mut self,
        c: &mut Reader<'_>,
        pending: &mut Option<[u32; 3]>,
    ) -> Result<(), VoxError> {
        match &c.chunk {
            b"PACK" => {
                c.u32()?;
            }
            b"SIZE" => {
                if pending.is_some() {
                    return Err(c.malformed("SIZE without its XYZI"));
                }
                *pending = Some(size(c)?);
            }
            b"XYZI" => {
                let Some(size) = pending.take() else {
                    return Err(c.malformed("XYZI without a SIZE"));
                };
                self.models.push(xyzi(c, size)?);
            }
            b"RGBA" => rgba(c, &mut self.palette)?,
            b"nTRN" | b"nGRP" | b"nSHP" => {
                let (nid, node, extra) = node(c)?;
                if self.nodes.insert(nid, node).is_some() {
                    return Err(c.malformed("duplicate node id"));
                }
                if let Some(name) = extra.name {
                    self.names.insert(nid, name);
                }
                if extra.hidden {
                    self.hidden.insert(nid);
                }
                if let Some(l) = extra.layer.filter(|&l| l != default_layer(nid)) {
                    self.node_layers.insert(nid, l);
                }
                if let Some(b) = extra.reflected {
                    self.reflected.insert(nid, b);
                }
            }
            b"LAYR" => {
                let (id, layer) = layr(c)?;
                self.layers.insert(id, layer);
            }
            b"IMAP" => self.imap_present = true,
            b"MATL" => {
                let (index, m) = matl(c)?;
                if let Some(slot) = self.materials.get_mut(index) {
                    *slot = m;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Node 0 exists, every child and model a node names exists, no node is its own
    /// descendant, and no node places more than [`MAX_PLACEMENTS`] models. Returns each
    /// reachable node's placement count. An iterative depth-first walk, so a deep graph cannot
    /// exhaust the stack; each node is finished once, in post-order, when its children's
    /// counts are known, so a shared subtree costs one visit however often it is placed.
    pub(crate) fn check_graph(&self) -> Result<BTreeMap<u32, u64>, VoxError> {
        if !self.nodes.contains_key(&0) {
            return Err(VoxError::SceneGraph {
                node: 0,
                what: "the root node 0 is missing",
            });
        }
        // 1: on the current path; 2: finished.
        let mut state: BTreeMap<u32, u8> = BTreeMap::new();
        let mut counts: BTreeMap<u32, u64> = BTreeMap::new();
        // (node, index of the next child to visit)
        let mut stack: Vec<(u32, usize)> = vec![(0, 0)];
        state.insert(0, 1);
        while let Some(&mut (id, ref mut next)) = stack.last_mut() {
            let children: &[u32] = match &self.nodes[&id] {
                Node::Transform { child, .. } => std::slice::from_ref(child),
                Node::Group { children } => children,
                Node::Shape { models } => {
                    if models.iter().any(|&m| m as usize >= self.models.len()) {
                        return Err(VoxError::SceneGraph {
                            node: id,
                            what: "a shape names a model the file does not have",
                        });
                    }
                    &[]
                }
            };
            if let Some(&c) = children.get(*next) {
                *next += 1;
                if !self.nodes.contains_key(&c) {
                    return Err(VoxError::SceneGraph {
                        node: id,
                        what: "names a child node the file does not have",
                    });
                }
                match state.get(&c) {
                    Some(1) => {
                        return Err(VoxError::SceneGraph {
                            node: c,
                            what: "is its own descendant",
                        });
                    }
                    Some(_) => {}
                    None => {
                        state.insert(c, 1);
                        stack.push((c, 0));
                    }
                }
            } else {
                // Every child is finished, so its count is present.
                let count = |c: &u32| counts.get(c).copied().unwrap_or(0);
                let (n, chunk) = match &self.nodes[&id] {
                    Node::Transform { child, .. } => (count(child), *b"nTRN"),
                    Node::Group { children } => (
                        children
                            .iter()
                            .fold(0u64, |a, c| a.saturating_add(count(c))),
                        *b"nGRP",
                    ),
                    Node::Shape { models } => (models.len() as u64, *b"nSHP"),
                };
                if n > MAX_PLACEMENTS as u64 {
                    return Err(VoxError::Malformed {
                        chunk,
                        what: "places more models than MAX_PLACEMENTS",
                    });
                }
                counts.insert(id, n);
                state.insert(id, 2);
                stack.pop();
            }
        }
        Ok(counts)
    }

    /// Checks the graph, then hands `emit` every placement in [`VoxFile::placements`]'s
    /// order. Subtrees that place nothing are not entered, so the walk's cost is bounded by
    /// the placement count times the graph's depth. Translations compose with checked
    /// addition: a sum that leaves `i32` is [`VoxError::Malformed`], since each `_t`
    /// component is within [`MAX_TRANSLATION`] and turning it by a rotation cannot overflow.
    fn walk(&self, reflections: bool, emit: &mut impl FnMut(Placement)) -> Result<(), VoxError> {
        if self.nodes.is_empty() {
            for model in 0..self.models.len() {
                emit(Placement {
                    model,
                    translation: [0; 3],
                    rot: Rot::IDENTITY,
                });
            }
            return Ok(());
        }
        let counts = self.check_graph()?;
        let places = |c: &u32| counts.get(c).is_some_and(|&n| n > 0);
        let mut stack = vec![(0u32, [0i32; 3], Rot::IDENTITY)];
        while let Some((id, t, rot)) = stack.pop() {
            match &self.nodes[&id] {
                Node::Transform {
                    child,
                    translation,
                    rot: r,
                } => {
                    if !places(child) {
                        continue;
                    }
                    if let Some(&b) = self.reflected.get(&id).filter(|_| !reflections) {
                        return Err(VoxError::Rotation(b));
                    }
                    let d = rot.apply(*translation);
                    let mut sum = [0; 3];
                    for k in 0..3 {
                        sum[k] = t[k].checked_add(d[k]).ok_or(VoxError::Malformed {
                            chunk: *b"nTRN",
                            what: "composed _t overflows i32",
                        })?;
                    }
                    stack.push((*child, sum, r.then(rot)));
                }
                Node::Group { children } => {
                    stack.extend(
                        children
                            .iter()
                            .rev()
                            .filter(|c| places(c))
                            .map(|&c| (c, t, rot)),
                    );
                }
                Node::Shape { models } => {
                    for &m in models {
                        emit(Placement {
                            model: m as usize,
                            translation: t,
                            rot,
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// Every model the scene graph places, walking from node 0 depth first in child order and
    /// composing translations and rotations down the path (a child's frame is turned by its
    /// parents' rotation, then offset by their translation). Without a scene graph, one
    /// placement per model at translation 0 and [`Rot::IDENTITY`]. At most
    /// [`MAX_PLACEMENTS`] entries. A path through a reflected frame
    /// ([`VoxFile::reflected`]) stops the walk: the placements found before it are returned
    /// ([`VoxFile::try_placements`] says why).
    pub fn placements(&self) -> Vec<Placement> {
        let mut out = Vec::new();
        // A file `parse` returned passes the walk's other checks; a reflected frame, or a graph
        // edited after parsing into one that fails them, yields the placements found before
        // the fault.
        let _ = self.walk(false, &mut |p| out.push(p));
        out
    }

    /// [`VoxFile::placements`], refusing a file whose placed path passes a reflected frame
    /// with [`VoxError::Rotation`] and its `_r` byte ([`crate::VoxScene`] loads such a file).
    pub fn try_placements(&self) -> Result<Vec<Placement>, VoxError> {
        let mut out = Vec::new();
        self.walk(false, &mut |p| out.push(p))?;
        Ok(out)
    }

    /// Transform `id`'s layer id: [`VoxFile::node_layers`]'s, else MagicaVoxel's default.
    pub fn layer_of(&self, id: u32) -> i32 {
        self.node_layers
            .get(&id)
            .copied()
            .unwrap_or_else(|| default_layer(id))
    }
}
