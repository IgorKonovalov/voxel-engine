//! The MagicaVoxel `.vox` writer: the inverse of [`VoxFile::parse`].
//!
//! [`encode`] writes `"VOX "`, the file's version and one `MAIN` chunk holding, in order:
//! `PACK` when there is more than one model, each model's `SIZE` and `XYZI` pair in model
//! order with its voxels in their listed order, every scene-graph node in ascending id order
//! (`nTRN` with one frame, `nGRP`, `nSHP`), then `RGBA`. The output is a pure function of the
//! file, so the same file always encodes to the same bytes.

use vxe_world::Rot;

use crate::vox::{Node, VoxFile};

/// A rotation as a transform frame's `_r` byte, the inverse of
/// [`crate::decode_rotation`]: bits 0 to 1 hold the index of row 1's non-zero entry, bits 2
/// to 3 that of row 2, and bits 4, 5 and 6 the signs of rows 1, 2 and 3 (set is negative).
/// [`Rot::IDENTITY`] is 4.
pub fn encode_rotation(rot: Rot) -> u8 {
    let (a0, n0) = rot.row(0);
    let (a1, n1) = rot.row(1);
    let (_, n2) = rot.row(2);
    a0 | a1 << 2 | u8::from(n0) << 4 | u8::from(n1) << 5 | u8::from(n2) << 6
}

/// One chunk: its id, its content's and children's byte counts, then both.
fn chunk(out: &mut Vec<u8>, id: &[u8; 4], content: &[u8]) {
    out.extend_from_slice(id);
    out.extend_from_slice(&len32(content.len()).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(content);
}

/// A byte count or a list length as the format's 32-bit field.
///
/// # Panics
///
/// When `n` does not fit in an `i32`: such a file cannot be written in the format.
fn len32(n: usize) -> u32 {
    i32::try_from(n).expect("a .vox field holds at most i32::MAX") as u32
}

fn int(out: &mut Vec<u8>, v: i32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn string(out: &mut Vec<u8>, s: &str) {
    int(out, len32(s.len()) as i32);
    out.extend_from_slice(s.as_bytes());
}

fn dict(out: &mut Vec<u8>, pairs: &[(&str, String)]) {
    int(out, len32(pairs.len()) as i32);
    for (k, v) in pairs {
        string(out, k);
        string(out, v);
    }
}

/// The file as `.vox` bytes, which [`VoxFile::parse`] reads back to an equal file when the
/// file holds what the parser can produce: `palette[0]` is `[0, 0, 0, 0]`, every model edge
/// is at most [`crate::MAX_MODEL_EDGE`], node ids and model ids fit in an `i32`, and
/// `imap_present` is false (no `IMAP` is written). A transform writes `_t` only when its
/// translation is not zero and `_r` only when its rotation is not the identity, which the
/// parser reads back as those defaults. The root transform's layer is -1 and every other
/// transform's 0, as MagicaVoxel writes them.
///
/// # Panics
///
/// When a count or size does not fit the format's 32-bit fields.
pub fn encode(file: &VoxFile) -> Vec<u8> {
    let mut body = Vec::new();
    if file.models.len() > 1 {
        chunk(&mut body, b"PACK", &len32(file.models.len()).to_le_bytes());
    }
    let mut c = Vec::new();
    for m in &file.models {
        c.clear();
        for s in m.size {
            int(&mut c, len32(s as usize) as i32);
        }
        chunk(&mut body, b"SIZE", &c);
        c.clear();
        c.reserve(4 + 4 * m.voxels.len());
        int(&mut c, len32(m.voxels.len()) as i32);
        c.extend_from_slice(m.voxels.as_flattened());
        chunk(&mut body, b"XYZI", &c);
    }
    for (&id, node) in &file.nodes {
        c.clear();
        int(&mut c, len32(id as usize) as i32);
        dict(&mut c, &[]);
        let tag = match node {
            Node::Transform {
                child,
                translation,
                rot,
            } => {
                int(&mut c, len32(*child as usize) as i32);
                int(&mut c, -1);
                int(&mut c, if id == 0 { -1 } else { 0 });
                int(&mut c, 1);
                let mut frame = Vec::new();
                if *rot != Rot::IDENTITY {
                    frame.push(("_r", encode_rotation(*rot).to_string()));
                }
                if *translation != [0; 3] {
                    let [x, y, z] = *translation;
                    frame.push(("_t", format!("{x} {y} {z}")));
                }
                dict(&mut c, &frame);
                b"nTRN"
            }
            Node::Group { children } => {
                int(&mut c, len32(children.len()) as i32);
                for &k in children {
                    int(&mut c, len32(k as usize) as i32);
                }
                b"nGRP"
            }
            Node::Shape { models } => {
                int(&mut c, len32(models.len()) as i32);
                for &m in models {
                    int(&mut c, len32(m as usize) as i32);
                    dict(&mut c, &[]);
                }
                b"nSHP"
            }
        };
        chunk(&mut body, tag, &c);
    }
    // Entry k is the colour of index k + 1; the last entry has no index.
    let mut rgba = [[0u8; 4]; 256];
    rgba[..255].copy_from_slice(&file.palette[1..]);
    chunk(&mut body, b"RGBA", rgba.as_flattened());

    let mut out = Vec::with_capacity(20 + body.len());
    out.extend_from_slice(b"VOX ");
    out.extend_from_slice(&file.version.to_le_bytes());
    out.extend_from_slice(b"MAIN");
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&len32(body.len()).to_le_bytes());
    out.extend_from_slice(&body);
    out
}
