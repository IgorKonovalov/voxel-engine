//! Plan 0083 Phase 4: a `.vox` scene for the world. Hidden layers and nodes, reflected frames,
//! and the whole scale.

use std::collections::BTreeMap;

use vxe_assets::*;
use vxe_world::{AIR, ChunkExtent, Material, MaterialTable, Rot, UniformSource, VoxelCoord, World};

fn trn(child: u32, translation: [i32; 3]) -> Node {
    Node::Transform {
        child,
        translation,
        rot: Rot::IDENTITY,
    }
}

fn shp(model: u32) -> Node {
    Node::Shape {
        models: vec![model],
    }
}

/// One voxel of index 1 per model.
fn dot() -> VoxModel {
    VoxModel {
        size: [1, 1, 1],
        voxels: vec![[0, 0, 0, 1]],
    }
}

/// Every palette index to material 1, index 0 to "leave alone".
fn map() -> VoxMap {
    let mut m = VoxMap::new();
    m.set(0, Slot::Skip);
    for i in 1..=255 {
        m.set(i, Slot::Material(1));
    }
    m
}

/// Done-when 1: a model on a hidden layer and one under a `_hidden 1` transform are not
/// placed; with the flags off both are. The file goes through `encode` and `parse` first.
#[test]
fn hidden_layers_and_nodes_are_not_placed() {
    let mut nodes = BTreeMap::new();
    nodes.insert(0, trn(1, [0; 3]));
    nodes.insert(
        1,
        Node::Group {
            children: vec![2, 4, 6],
        },
    );
    nodes.insert(2, trn(3, [10, 0, 0]));
    nodes.insert(3, shp(0));
    nodes.insert(4, trn(5, [20, 0, 0]));
    nodes.insert(5, shp(1));
    nodes.insert(6, trn(7, [30, 0, 0]));
    nodes.insert(7, shp(2));
    let mut file = VoxFile {
        models: vec![dot(), dot(), dot()],
        nodes,
        ..VoxFile::default()
    };
    file.node_layers.insert(2, 3);
    file.layers.insert(
        3,
        VoxLayer {
            name: "scaffold".into(),
            hidden: true,
        },
    );
    file.layers.insert(0, VoxLayer::default());
    file.hidden.insert(4);
    let file = VoxFile::parse(&encode(&file)).expect("parses");
    assert_eq!(file.layer_of(2), 3);
    assert_eq!(file.layer_of(6), 0);
    assert!(file.hidden.contains(&4));
    assert!(file.layers[&3].hidden);
    assert_eq!(VoxFile::parse(&encode(&file)).unwrap(), file);

    let shown = VoxScene::load(&file, &map(), SceneOptions::default()).unwrap();
    let corners: Vec<[i32; 3]> = shown.placements.iter().map(|p| p.1.0).collect();
    assert_eq!(corners, [[30, 0, 0]], "only the visible model");
    assert_eq!(shown.layers, [("layer 0".to_owned(), vec![0])]);
    let all = VoxScene::load(
        &file,
        &map(),
        SceneOptions {
            honour_hidden: false,
            ..SceneOptions::default()
        },
    )
    .unwrap();
    let corners: Vec<[i32; 3]> = all.placements.iter().map(|p| p.1.0).collect();
    assert_eq!(corners, [[10, 0, 0], [20, 0, 0], [30, 0, 0]]);
    assert_eq!(
        all.layers,
        [
            ("layer 0".to_owned(), vec![1, 2]),
            ("scaffold".to_owned(), vec![0])
        ]
    );
    assert_eq!(all.bounds, ([10, 0, 0], [30, 0, 0]));
}

/// The L: four voxels of index 1 at x 0..3 on y 0, and x 0 on y 1, z 0 and 1 at x 0 (an
/// L with a post, so no reflection maps it onto itself), size 3 x 2 x 2.
fn ell() -> VoxModel {
    VoxModel {
        size: [3, 2, 2],
        voxels: vec![
            [0, 0, 0, 1],
            [1, 0, 0, 1],
            [2, 0, 0, 1],
            [0, 1, 0, 2],
            [0, 0, 1, 3],
        ],
    }
}

/// Done-when 2: an `_r` byte with determinant -1 over the L yields a placement whose `Rot` has
/// determinant +1 and whose voxels, stamped, land where the reflection puts them, voxel by
/// voxel against a hand-mirrored copy; `VoxFile::try_placements` refuses the file, `VoxScene`
/// does not.
#[test]
fn a_reflected_frame_loads_as_a_mirrored_model() {
    let mut table = MaterialTable::new();
    for n in ["a", "b", "c"] {
        table.push(Material::named(n));
    }
    let mut m = VoxMap::new();
    m.set(0, Slot::Skip);
    for i in 1..=3 {
        m.set(i, Slot::Material(u16::from(i)));
    }
    // x mirrored; x and y swapped; y and z swapped with x and the new z negated.
    for byte in [4 | 16, 1, (1 << 6) | (1 << 4) | 8] {
        assert!(decode_rotation(byte).is_err(), "{byte} is a reflection");
        let t = [40, 50, 60];
        let mut nodes = BTreeMap::new();
        nodes.insert(0, trn(1, [0; 3]));
        nodes.insert(1, Node::Group { children: vec![2] });
        nodes.insert(2, trn(3, t));
        nodes.insert(3, shp(0));
        let mut file = VoxFile {
            models: vec![ell()],
            nodes,
            ..VoxFile::default()
        };
        let (proper, mirrored) = decode_frame(byte).unwrap();
        assert!(mirrored);
        if let Some(Node::Transform { rot, .. }) = file.nodes.get_mut(&2) {
            *rot = proper;
        }
        file.reflected.insert(2, byte);
        let file = VoxFile::parse(&encode(&file)).expect("a reflected frame parses");
        assert_eq!(file.reflected.get(&2), Some(&byte));
        assert_eq!(file.try_placements(), Err(VoxError::Rotation(byte)));

        let scene = VoxScene::load(&file, &m, SceneOptions::default()).unwrap();
        assert_eq!(scene.placements.len(), 1);
        let (_, origin, rot) = scene.placements[0];
        assert_eq!(rot.determinant(), 1);

        // The hand-mirrored copy: row k of the byte reads source axis a, from the far side
        // when negative; the box's minimum corner is the translation less half the turned size.
        let axis = [
            byte & 3,
            (byte >> 2) & 3,
            3 - (byte & 3) - ((byte >> 2) & 3),
        ]
        .map(usize::from);
        let neg = [byte >> 4 & 1 != 0, byte >> 5 & 1 != 0, byte >> 6 & 1 != 0];
        let size = ell().size;
        let turned = axis.map(|a| size[a]);
        let corner = [0, 1, 2].map(|k| t[k] - (turned[k] / 2) as i32);
        assert_eq!(origin.0, corner, "byte {byte}");
        let world = || {
            World::generate(
                ChunkExtent::new([0, 0, 0], [2, 2, 2]),
                &UniformSource(AIR),
                &table,
            )
        };
        let want = world();
        for &[x, y, z, i] in &ell().voxels {
            let l = [x, y, z].map(u32::from);
            let d = [0, 1, 2].map(|k| {
                let a = axis[k];
                let c = if neg[k] { size[a] - 1 - l[a] } else { l[a] };
                corner[k] + c as i32
            });
            let p = vxe_world::Prefab::new([1, 1, 1], vec![u16::from(i)], None);
            want.stamp(&p, VoxelCoord(d), Rot::IDENTITY).unwrap();
        }
        let got = world();
        for (p, o, r) in scene.placed_at([0; 3]) {
            got.stamp(&p, o, r).unwrap();
        }
        assert_eq!(got.voxel_hash(), want.voxel_hash(), "byte {byte}");
        for z in 0..6 {
            for y in 0..6 {
                for x in 0..6 {
                    let v = VoxelCoord([corner[0] - 1 + x, corner[1] - 1 + y, corner[2] - 1 + z]);
                    assert_eq!(got.material_at(v), want.material_at(v), "{v:?}");
                }
            }
        }
    }
}

/// Done-when 3: a 2 x 1 x 1 model at scale 3 is a 6 x 3 x 3 prefab of 54 written voxels; scale
/// 5 is refused naming the range.
#[test]
fn scale_repeats_voxels() {
    let file = VoxFile {
        models: vec![VoxModel {
            size: [2, 1, 1],
            voxels: vec![[0, 0, 0, 1], [1, 0, 0, 2]],
        }],
        ..VoxFile::default()
    };
    let scaled = |k| {
        VoxScene::load(
            &file,
            &map(),
            SceneOptions {
                scale: k,
                ..SceneOptions::default()
            },
        )
    };
    let s = scaled(3).unwrap();
    let p = &s.prefabs[0];
    assert_eq!(p.size(), [6, 3, 3]);
    assert_eq!(p.written_count(), 54);
    assert_eq!(s.size(), [6, 3, 3]);
    let e = scaled(5).unwrap_err();
    assert_eq!(e, SceneError::Scale(5));
    assert!(e.to_string().contains("1..=4"), "{e}");
    assert_eq!(scaled(0).unwrap_err(), SceneError::Scale(0));
}
