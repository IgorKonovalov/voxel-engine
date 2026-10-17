//! A test-side `.vox` writer, so every parser test states its input bytes in the test.
#![allow(dead_code, reason = "shared across test binaries; each uses a subset")]

use vxe_assets::VoxModel;

/// A scene-graph node as written: ids, and frame 0's `_t` and `_r` when given.
pub enum TestNode {
    Trn {
        id: i32,
        child: i32,
        t: Option<[i32; 3]>,
        r: Option<u8>,
    },
    Grp {
        id: i32,
        children: Vec<i32>,
    },
    Shp {
        id: i32,
        models: Vec<i32>,
    },
}

/// One chunk: id, content and children byte counts, content, children.
pub fn chunk(id: &[u8; 4], content: &[u8], children: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(id);
    out.extend_from_slice(&(content.len() as u32).to_le_bytes());
    out.extend_from_slice(&(children.len() as u32).to_le_bytes());
    out.extend_from_slice(content);
    out.extend_from_slice(children);
    out
}

pub fn string(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as i32).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

pub fn dict(out: &mut Vec<u8>, pairs: &[(&str, String)]) {
    out.extend_from_slice(&(pairs.len() as i32).to_le_bytes());
    for (k, v) in pairs {
        string(out, k);
        string(out, v);
    }
}

fn i32s(out: &mut Vec<u8>, v: &[i32]) {
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
}

pub fn size_chunk(size: [u32; 3]) -> Vec<u8> {
    let mut c = Vec::new();
    i32s(&mut c, &size.map(|v| v as i32));
    chunk(b"SIZE", &c, &[])
}

pub fn xyzi_chunk(voxels: &[[u8; 4]]) -> Vec<u8> {
    let mut c = Vec::new();
    i32s(&mut c, &[voxels.len() as i32]);
    for v in voxels {
        c.extend_from_slice(v);
    }
    chunk(b"XYZI", &c, &[])
}

pub fn node_chunk(n: &TestNode) -> Vec<u8> {
    let mut c = Vec::new();
    match n {
        TestNode::Trn { id, child, t, r } => {
            i32s(&mut c, &[*id]);
            dict(&mut c, &[("_name", "n".into())]);
            i32s(&mut c, &[*child, -1, 0, 1]);
            let mut frame = Vec::new();
            if let Some(r) = r {
                frame.push(("_r", r.to_string()));
            }
            if let Some(t) = t {
                frame.push(("_t", format!("{} {} {}", t[0], t[1], t[2])));
            }
            dict(&mut c, &frame);
            chunk(b"nTRN", &c, &[])
        }
        TestNode::Grp { id, children } => {
            i32s(&mut c, &[*id]);
            dict(&mut c, &[]);
            i32s(&mut c, &[children.len() as i32]);
            i32s(&mut c, children);
            chunk(b"nGRP", &c, &[])
        }
        TestNode::Shp { id, models } => {
            i32s(&mut c, &[*id]);
            dict(&mut c, &[]);
            i32s(&mut c, &[models.len() as i32]);
            for m in models {
                i32s(&mut c, &[*m]);
                dict(&mut c, &[]);
            }
            chunk(b"nSHP", &c, &[])
        }
    }
}

/// A version-150 file: `PACK` when there is more than one model, the `SIZE` and `XYZI`
/// pairs, `nodes`, then `RGBA` with `palette` (256 entries as written) when given.
pub fn vox_bytes(
    models: &[VoxModel],
    palette: Option<&[[u8; 4]; 256]>,
    nodes: &[TestNode],
) -> Vec<u8> {
    vox_bytes_with(150, models, palette, nodes, &[])
}

/// [`vox_bytes`] with a version and extra chunks (already framed) appended inside `MAIN`.
pub fn vox_bytes_with(
    version: u32,
    models: &[VoxModel],
    palette: Option<&[[u8; 4]; 256]>,
    nodes: &[TestNode],
    extra: &[Vec<u8>],
) -> Vec<u8> {
    let mut children = Vec::new();
    if models.len() > 1 {
        children.extend(chunk(b"PACK", &(models.len() as i32).to_le_bytes(), &[]));
    }
    for m in models {
        children.extend(size_chunk(m.size));
        children.extend(xyzi_chunk(&m.voxels));
    }
    for n in nodes {
        children.extend(node_chunk(n));
    }
    if let Some(p) = palette {
        children.extend(chunk(b"RGBA", p.as_flattened(), &[]));
    }
    for e in extra {
        children.extend_from_slice(e);
    }
    file(version, &children)
}

/// `"VOX "`, the version, and a `MAIN` chunk holding `children` (already framed).
pub fn file(version: u32, children: &[u8]) -> Vec<u8> {
    let mut out = b"VOX ".to_vec();
    out.extend_from_slice(&version.to_le_bytes());
    out.extend(chunk(b"MAIN", &[], children));
    out
}

/// The 16³ vault as a model: palette index 1 on the surface, nothing listed inside.
pub fn vault_model() -> VoxModel {
    let mut voxels = Vec::new();
    for z in 0..16u8 {
        for y in 0..16u8 {
            for x in 0..16u8 {
                if [x, y, z].iter().any(|&c| c == 0 || c == 15) {
                    voxels.push([x, y, z, 1]);
                }
            }
        }
    }
    VoxModel {
        size: [16; 3],
        voxels,
    }
}
