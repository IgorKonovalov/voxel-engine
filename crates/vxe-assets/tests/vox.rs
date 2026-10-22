mod support;

use support::*;
use vxe_assets::*;
use vxe_world::{AIR, Prefab, Rot, VoxelCoord};

fn two_voxel_model() -> VoxModel {
    VoxModel {
        size: [3, 2, 1],
        voxels: vec![[0, 0, 0, 1], [2, 0, 0, 2]],
    }
}

/// A palette whose entry `k` is `[k, 255 - k, k / 2, 255]`, so every entry is distinct.
fn test_palette() -> [[u8; 4]; 256] {
    std::array::from_fn(|k| [k as u8, 255 - k as u8, (k / 2) as u8, 255])
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

#[test]
fn a_small_model_parses_to_its_size_voxels_and_palette() {
    let p = test_palette();
    let f = VoxFile::parse(&vox_bytes(&[two_voxel_model()], Some(&p), &[])).unwrap();
    assert_eq!(f.version, 150);
    assert_eq!(f.models.len(), 1);
    assert_eq!(f.models[0].size, [3, 2, 1]);
    assert_eq!(f.models[0].voxels, vec![[0, 0, 0, 1], [2, 0, 0, 2]]);
    // RGBA entry k is the colour of index k + 1.
    assert_eq!(f.palette[1], p[0]);
    assert_eq!(f.palette[2], p[1]);
    assert_eq!(f.palette[255], p[254]);
    assert_eq!(f.palette[0], [0, 0, 0, 0]);
    assert!(!f.imap_present);
}

#[test]
fn without_rgba_the_palette_is_magicavoxels_default() {
    let f = VoxFile::parse(&vox_bytes(&[two_voxel_model()], None, &[])).unwrap();
    assert_eq!(f.palette, DEFAULT_PALETTE);
    // FNV-1a 64 over the spec's 256 `0xAABBGGRR` values as little-endian bytes (that is,
    // `[r, g, b, a]` per entry), computed from the spec's listing.
    assert_eq!(fnv1a(DEFAULT_PALETTE.as_flattened()), 0xfd04_e720_d946_8c0c);
    assert_eq!(DEFAULT_PALETTE[0], [0, 0, 0, 0]);
    assert_eq!(DEFAULT_PALETTE[1], [0xff, 0xff, 0xff, 0xff]);
    assert_eq!(DEFAULT_PALETTE[2], [0xff, 0xff, 0xcc, 0xff]); // 0xffccffff
    assert_eq!(DEFAULT_PALETTE[37], [0xcc, 0xff, 0xff, 0xff]); // 0xffffffcc
    assert_eq!(DEFAULT_PALETTE[215], [0x00, 0x00, 0x33, 0xff]); // 0xff330000
    assert_eq!(DEFAULT_PALETTE[216], [0xee, 0x00, 0x00, 0xff]); // 0xff0000ee
    assert_eq!(DEFAULT_PALETTE[255], [0x11, 0x11, 0x11, 0xff]); // 0xff111111
}

#[test]
fn version_200_parses_and_other_versions_and_magics_fail() {
    let m = [two_voxel_model()];
    assert_eq!(
        VoxFile::parse(&vox_bytes_with(200, &m, None, &[], &[]))
            .unwrap()
            .models,
        m
    );
    assert_eq!(
        VoxFile::parse(&vox_bytes_with(151, &m, None, &[], &[])),
        Err(VoxError::Version(151))
    );
    let mut b = vox_bytes(&m, None, &[]);
    b[0] = b'W';
    assert_eq!(VoxFile::parse(&b), Err(VoxError::Magic(*b"WOX ")));
}

#[test]
fn unread_chunks_are_skipped_by_their_declared_size() {
    let models = [two_voxel_model(), vault_model()];
    let plain = VoxFile::parse(&vox_bytes(&models, None, &[])).unwrap();

    let mut matl = 7i32.to_le_bytes().to_vec();
    dict(
        &mut matl,
        &[("_type", "_metal".into()), ("_rough", "0.3".into())],
    );
    let mut layr = 0i32.to_le_bytes().to_vec();
    dict(&mut layr, &[("_name", "walls".into())]);
    layr.extend_from_slice(&(-1i32).to_le_bytes());
    let mut robj = Vec::new();
    dict(&mut robj, &[("_type", "_bloom".into())]);
    let mut rcam = 0i32.to_le_bytes().to_vec();
    dict(
        &mut rcam,
        &[("_mode", "pers".into()), ("_fov", "45".into())],
    );
    let mut note = 2i32.to_le_bytes().to_vec();
    string(&mut note, "stone");
    string(&mut note, "");
    let imap: Vec<u8> = (0..256i32).flat_map(|i| i.to_le_bytes()).collect();
    let extra = vec![
        chunk(b"MATL", &matl, &[]),
        chunk(b"LAYR", &layr, &[]),
        chunk(b"rOBJ", &robj, &[]),
        chunk(b"rCAM", &rcam, &[]),
        chunk(b"NOTE", &note, &[]),
        chunk(b"IMAP", &imap, &[]),
        // An unknown chunk with children of its own.
        chunk(b"ZZZZ", &[1, 2, 3], &chunk(b"SIZE", &[9; 12], &[])),
    ];
    let with = VoxFile::parse(&vox_bytes_with(150, &models, None, &[], &extra)).unwrap();
    assert_eq!(with.models, plain.models);
    assert_eq!(with.palette, plain.palette);
    assert!(with.imap_present);
}

#[test]
fn a_chunk_cut_short_is_truncated_with_its_id_and_offset() {
    // The whole file cut: MAIN's declared children run past the end.
    let full = vox_bytes(&[two_voxel_model()], None, &[]);
    let cut = &full[..full.len() - 3];
    assert_eq!(
        VoxFile::parse(cut),
        Err(VoxError::Truncated {
            chunk: *b"MAIN",
            at: cut.len()
        })
    );

    // Layout: header 0..8, MAIN header 8..20, children from 20.
    // XYZI claims 5 voxels and holds 2: SIZE 20..44, XYZI header 44..56, content 56..68.
    let mut xyzi = 5i32.to_le_bytes().to_vec();
    xyzi.extend_from_slice(&[0, 0, 0, 1, 2, 0, 0, 2]);
    let mut children = size_chunk([3, 2, 1]);
    children.extend(chunk(b"XYZI", &xyzi, &[]));
    assert_eq!(
        VoxFile::parse(&file(150, &children)),
        Err(VoxError::Truncated {
            chunk: *b"XYZI",
            at: 68
        })
    );

    // SIZE with two of its three integers: header 20..32, content 32..40.
    let children = chunk(b"SIZE", &[3, 0, 0, 0, 2, 0, 0, 0], &[]);
    assert_eq!(
        VoxFile::parse(&file(150, &children)),
        Err(VoxError::Truncated {
            chunk: *b"SIZE",
            at: 40
        })
    );

    // A chunk header declaring more content than MAIN holds: MAIN's children are 20..36.
    let mut children = b"XYZI".to_vec();
    children.extend_from_slice(&100u32.to_le_bytes());
    children.extend_from_slice(&0u32.to_le_bytes());
    children.extend_from_slice(&[0; 4]);
    assert_eq!(
        VoxFile::parse(&file(150, &children)),
        Err(VoxError::Truncated {
            chunk: *b"XYZI",
            at: 36
        })
    );

    // Fewer than the eight header bytes.
    assert_eq!(
        VoxFile::parse(b"VOX "),
        Err(VoxError::Truncated {
            chunk: *b"VOX ",
            at: 4
        })
    );
}

#[test]
fn a_model_axis_over_256_is_too_large() {
    let big = VoxModel {
        size: [257, 1, 1],
        voxels: vec![],
    };
    assert_eq!(
        VoxFile::parse(&vox_bytes(&[big], None, &[])),
        Err(VoxError::TooLarge { size: [257, 1, 1] })
    );
    assert_eq!(MAX_MODEL_EDGE, 256);
    let empty = VoxModel {
        size: [256, 256, 256],
        voxels: vec![],
    };
    assert_eq!(
        VoxFile::parse(&vox_bytes(std::slice::from_ref(&empty), None, &[]))
            .unwrap()
            .models,
        [empty]
    );
    let edge = VoxModel {
        size: [256, 1, 256],
        voxels: vec![[255, 0, 255, 1]],
    };
    assert_eq!(
        VoxFile::parse(&vox_bytes(std::slice::from_ref(&edge), None, &[]))
            .unwrap()
            .models,
        [edge]
    );
}

#[test]
fn rotation_bytes_decode_by_the_spec_rule() {
    assert_eq!(decode_rotation(4), Ok(Rot::IDENTITY));
    // 17: row 1 index 1, row 2 index 0, row 1 negative: rows (0, -1, 0), (1, 0, 0), (0, 0, 1).
    let r = decode_rotation(17).unwrap();
    assert_eq!(r, Rot::Z90);
    assert_eq!(r.row(0), (1, true));
    assert_eq!(r.row(1), (0, false));
    assert_eq!(r.row(2), (2, false));
    // The spec's own example: rows (0, 1, 0), (0, 0, -1), (-1, 0, 0).
    let ex = (1 << 0) | (2 << 2) | (1 << 5) | (1 << 6);
    assert_eq!(decode_rotation(ex).unwrap().apply([1, 2, 3]), [2, -3, -1]);
    // A reflection (identity rows, row 1 negated) and a repeated axis are refused.
    assert_eq!(decode_rotation(4 | 16), Err(VoxError::Rotation(20)));
    assert_eq!(decode_rotation(0), Err(VoxError::Rotation(0)));
    assert_eq!(decode_rotation(3 | 4), Err(VoxError::Rotation(7)));
    // Every one of the 24 rotations has a byte.
    let decoded: std::collections::HashSet<Rot> =
        (0..128u8).filter_map(|b| decode_rotation(b).ok()).collect();
    assert_eq!(decoded.len(), 24);
}

#[test]
fn without_a_scene_graph_each_model_is_placed_at_the_origin() {
    let f = VoxFile::parse(&vox_bytes(&[two_voxel_model(), vault_model()], None, &[])).unwrap();
    assert_eq!(
        f.placements(),
        [0, 1].map(|model| Placement {
            model,
            translation: [0; 3],
            rot: Rot::IDENTITY
        })
    );
}

/// Root transform 0 -> group 1 -> (transform 2 -> shape 3 with model 0,
/// transform 4 -> shape 5 with model 1).
fn two_model_graph(
    t0: Option<[i32; 3]>,
    r0: Option<u8>,
    t4: [i32; 3],
    r4: Option<u8>,
) -> Vec<TestNode> {
    vec![
        TestNode::Trn {
            id: 0,
            child: 1,
            t: t0,
            r: r0,
        },
        TestNode::Grp {
            id: 1,
            children: vec![2, 4],
        },
        TestNode::Trn {
            id: 2,
            child: 3,
            t: None,
            r: None,
        },
        TestNode::Shp {
            id: 3,
            models: vec![0],
        },
        TestNode::Trn {
            id: 4,
            child: 5,
            t: Some(t4),
            r: r4,
        },
        TestNode::Shp {
            id: 5,
            models: vec![1],
        },
    ]
}

#[test]
fn the_scene_graph_composes_translations_and_rotations() {
    let models = [two_voxel_model(), vault_model()];
    let f = VoxFile::parse(&vox_bytes(
        &models,
        None,
        &two_model_graph(None, None, [0, 0, 16], None),
    ))
    .unwrap();
    let p = f.placements();
    assert_eq!(p.len(), 2);
    assert_eq!(p[0].model, 0);
    assert_eq!(p[0].translation, [0, 0, 0]);
    assert_eq!(p[1].model, 1);
    assert_eq!(p[1].translation, [0, 0, 16]);
    assert_eq!(p[1].rot, Rot::IDENTITY);

    // The root turns by Z90 and moves by (10, 0, 0); the child's (1, 2, 3) is turned to
    // (-2, 1, 3) first, and its own Z90 follows the root's.
    let f = VoxFile::parse(&vox_bytes(
        &models,
        None,
        &two_model_graph(Some([10, 0, 0]), Some(17), [1, 2, 3], Some(17)),
    ))
    .unwrap();
    let p = f.placements();
    assert_eq!(p[0].translation, [10, 0, 0]);
    assert_eq!(p[0].rot, Rot::Z90);
    assert_eq!(p[1].translation, [8, 1, 3]);
    assert_eq!(p[1].rot, Rot::Z90.then(Rot::Z90));
    assert_eq!(p[1].rot.apply([1, 0, 0]), [-1, 0, 0]);
}

#[test]
fn min_corner_moves_from_the_models_centre() {
    let p = Placement {
        model: 0,
        translation: [8, 8, 8],
        rot: Rot::IDENTITY,
    };
    assert_eq!(p.min_corner([16, 16, 16]), VoxelCoord([0, 0, 0]));
    // Odd sizes: the centre is size / 2 rounded down.
    assert_eq!(p.min_corner([3, 2, 5]), VoxelCoord([7, 7, 6]));
}

#[test]
fn a_broken_scene_graph_is_refused() {
    let models = [two_voxel_model()];
    let missing = [TestNode::Trn {
        id: 0,
        child: 9,
        t: None,
        r: None,
    }];
    assert_eq!(
        VoxFile::parse(&vox_bytes(&models, None, &missing)),
        Err(VoxError::SceneGraph {
            node: 0,
            what: "names a child node the file does not have"
        })
    );
    let cycle = [
        TestNode::Trn {
            id: 0,
            child: 1,
            t: None,
            r: None,
        },
        TestNode::Grp {
            id: 1,
            children: vec![0],
        },
    ];
    assert!(matches!(
        VoxFile::parse(&vox_bytes(&models, None, &cycle)),
        Err(VoxError::SceneGraph { node: 0, .. })
    ));
    let bad_model = [TestNode::Shp {
        id: 0,
        models: vec![1],
    }];
    assert!(matches!(
        VoxFile::parse(&vox_bytes(&models, None, &bad_model)),
        Err(VoxError::SceneGraph { node: 0, .. })
    ));
}

#[test]
fn the_map_lists_every_unset_index_and_honours_air_and_skip() {
    let model = two_voxel_model();
    let mut map = VoxMap::new();
    map.set(1, Slot::Material(4));
    assert_eq!(map.to_prefab(&model), Err(Unmapped(vec![2])));
    // Index 0 counts once it is unset and some cell is empty.
    let mut bare = VoxMap::new();
    bare.set(0, Slot::Unset);
    assert_eq!(bare.to_prefab(&model), Err(Unmapped(vec![0, 1, 2])));

    map.set(2, Slot::Skip);
    let p = map.to_prefab(&model).unwrap();
    assert_eq!(p.size(), [3, 2, 1]);
    assert_eq!(p.get([0, 0, 0]), Some(4));
    assert_eq!(p.get([2, 0, 0]), None, "Skip clears the mask bit");
    assert_eq!(p.get([1, 0, 0]), Some(AIR), "index 0 is Air");
    assert_eq!(p.written_count(), 5);

    map.set(0, Slot::Skip).set(2, Slot::Air);
    let p = map.to_prefab(&model).unwrap();
    assert_eq!(p.get([2, 0, 0]), Some(AIR));
    assert_eq!(p.get([1, 1, 0]), None);
    assert_eq!(p.written_count(), 2);
}

#[test]
fn the_vault_round_trips_through_a_vox_file() {
    let f = VoxFile::parse(&vox_bytes(&[vault_model()], None, &[])).unwrap();
    let mut map = VoxMap::new();
    map.set(1, Slot::Material(4));
    assert_eq!(map.get(0), Slot::Air);
    let p = map.to_prefab(&f.models[0]).unwrap();
    assert_eq!(p.id(), Prefab::vault(16, 4).id());
    assert_eq!(p, Prefab::vault(16, 4));
}

/// Groups 0..depth, each listing the next node twice, then a shape of model 0: 2^depth
/// placements from a file linear in `depth`.
fn doubling_graph(depth: i32) -> Vec<TestNode> {
    let mut nodes: Vec<TestNode> = (0..depth)
        .map(|id| TestNode::Grp {
            id,
            children: vec![id + 1, id + 1],
        })
        .collect();
    nodes.push(TestNode::Shp {
        id: depth,
        models: vec![0],
    });
    nodes
}

#[test]
fn a_shared_subtree_is_capped_at_max_placements() {
    let models = [two_voxel_model()];
    assert_eq!(MAX_PLACEMENTS, 65_536);
    let f = VoxFile::parse(&vox_bytes(&models, None, &doubling_graph(16))).unwrap();
    let p = f.placements();
    assert_eq!(p.len(), 65_536);
    assert!(p.iter().all(|p| p.model == 0 && p.translation == [0; 3]));
    assert_eq!(
        VoxFile::parse(&vox_bytes(&models, None, &doubling_graph(17))),
        Err(VoxError::Malformed {
            chunk: *b"nGRP",
            what: "places more models than MAX_PLACEMENTS"
        })
    );
    // Depth 40 asks for 2^40 placements; it is refused as quickly as depth 17.
    assert!(matches!(
        VoxFile::parse(&vox_bytes(&models, None, &doubling_graph(40))),
        Err(VoxError::Malformed { .. })
    ));
}

#[test]
fn a_subtree_that_places_nothing_is_not_walked() {
    // 2^40 paths ending in a shape with no models: zero placements, and no exponential walk.
    let models = [two_voxel_model()];
    let mut nodes = doubling_graph(40);
    nodes.pop();
    nodes.push(TestNode::Shp {
        id: 40,
        models: vec![],
    });
    let f = VoxFile::parse(&vox_bytes(&models, None, &nodes)).unwrap();
    assert_eq!(f.placements(), []);
}

/// Transform 0 (`_r` = `r0`) -> transform 1 (`_t` = `t1`) -> shape 2 with model 0.
fn turned_translation(r0: u8, t1: [i32; 3]) -> Vec<TestNode> {
    vec![
        TestNode::Trn {
            id: 0,
            child: 1,
            t: None,
            r: Some(r0),
        },
        TestNode::Trn {
            id: 1,
            child: 2,
            t: Some(t1),
            r: None,
        },
        TestNode::Shp {
            id: 2,
            models: vec![0],
        },
    ]
}

#[test]
fn a_translation_past_max_translation_is_refused() {
    let models = [two_voxel_model()];
    // 52: rows (-1, 0, 0), (0, -1, 0), (0, 0, 1), a 180-degree turn about z that negates x.
    let z180 = (1 << 2) | (1 << 4) | (1 << 5);
    assert_eq!(z180, 52);
    assert_eq!(decode_rotation(z180).unwrap().apply([1, 2, 3]), [-1, -2, 3]);
    let out_of_range = Err(VoxError::Malformed {
        chunk: *b"nTRN",
        what: "_t out of range",
    });
    assert_eq!(
        VoxFile::parse(&vox_bytes(
            &models,
            None,
            &turned_translation(z180, [i32::MIN, 0, 0])
        )),
        out_of_range
    );
    assert_eq!(MAX_TRANSLATION, 1 << 20);
    let f = VoxFile::parse(&vox_bytes(
        &models,
        None,
        &turned_translation(z180, [1_048_576, 0, -1_048_576]),
    ))
    .unwrap();
    assert_eq!(f.placements()[0].translation, [-1_048_576, 0, -1_048_576]);
    for t in [[-1_048_577, 0, 0], [0, 1_048_577, 0], [0, 0, i32::MAX]] {
        assert_eq!(
            VoxFile::parse(&vox_bytes(&models, None, &turned_translation(z180, t))),
            out_of_range,
            "{t:?}"
        );
    }
}

/// A chain of `n` transforms, each `_t` = (2^20, 0, 0), ending in a shape of model 0.
fn translation_chain(n: i32) -> Vec<TestNode> {
    let mut nodes: Vec<TestNode> = (0..n)
        .map(|id| TestNode::Trn {
            id,
            child: id + 1,
            t: Some([1 << 20, 0, 0]),
            r: None,
        })
        .collect();
    nodes.push(TestNode::Shp {
        id: n,
        models: vec![0],
    });
    nodes
}

#[test]
fn a_composed_translation_that_leaves_i32_is_refused() {
    let models = [two_voxel_model()];
    // 2047 * 2^20 = 2^31 - 2^20 fits in i32; 2048 * 2^20 = 2^31 does not.
    let f = VoxFile::parse(&vox_bytes(&models, None, &translation_chain(2047))).unwrap();
    assert_eq!(f.placements()[0].translation, [2047 << 20, 0, 0]);
    assert_eq!(
        VoxFile::parse(&vox_bytes(&models, None, &translation_chain(2048))),
        Err(VoxError::Malformed {
            chunk: *b"nTRN",
            what: "composed _t overflows i32"
        })
    );
}

#[test]
fn min_corner_is_total_at_the_i32_extremes() {
    let p = Placement {
        model: 0,
        translation: [i32::MIN, i32::MAX, 0],
        rot: Rot::IDENTITY,
    };
    assert_eq!(
        p.min_corner([4, 4, 4]),
        VoxelCoord([i32::MIN.wrapping_sub(2), i32::MAX - 2, -2])
    );
}

/// A palette index mapped to a `TRANSLUCENT` row maps like any other: the prefab carries the
/// id, and a stamp writes it into a world generated with that table, where every reader but
/// the renderer's opaque walk sees it as solid.
#[test]
fn a_translucent_row_maps_and_stamps_like_any_other() {
    use vxe_world::{ChunkExtent, Material, MaterialFlags, MaterialTable, UniformSource, World};
    let mut table = MaterialTable::new();
    let rock = table.push(Material::named("rock"));
    let glass = table.push(Material {
        flags: MaterialFlags::TRANSLUCENT,
        opacity: 0.5,
        ..Material::named("glass")
    });
    assert!(table.is_translucent(glass) && !table.is_translucent(rock));
    let mut map = VoxMap::new();
    map.set(1, Slot::Material(glass))
        .set(2, Slot::Material(rock));
    let p = map.to_prefab(&two_voxel_model()).unwrap();
    assert_eq!(p.get([0, 0, 0]), Some(glass));
    assert_eq!(p.get([2, 0, 0]), Some(rock));

    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [1, 1, 1]),
        &UniformSource(AIR),
        &table,
    );
    let r = world
        .stamp(&p, VoxelCoord([10, 10, 10]), Rot::IDENTITY)
        .unwrap();
    assert_eq!(r.changed_from, vec![(AIR, 2)], "two cells leave air");
    assert_eq!(world.material_at(VoxelCoord([10, 10, 10])), Some(glass));
    assert_eq!(world.material_at(VoxelCoord([12, 10, 10])), Some(rock));
}

/// A three-model scene, two models translated and one turned by a rotation from `Rot::all`,
/// under a custom palette: the encoder's bytes parse back to the same file, and the scene's
/// placements come back with the translations and the rotation written.
#[test]
fn an_encoded_three_model_scene_parses_back_to_the_same_file() {
    use std::collections::BTreeMap;
    let rot = Rot::all().nth(13).unwrap();
    assert_ne!(rot, Rot::IDENTITY);
    let small = VoxModel {
        size: [4, 3, 2],
        voxels: vec![[0, 0, 0, 1], [3, 2, 1, 200], [1, 1, 0, 9]],
    };
    let models = vec![two_voxel_model(), vault_model(), small];
    let mut nodes = BTreeMap::new();
    nodes.insert(
        0,
        Node::Transform {
            child: 1,
            translation: [0; 3],
            rot: Rot::IDENTITY,
        },
    );
    nodes.insert(
        1,
        Node::Group {
            children: vec![2, 4, 6],
        },
    );
    let placed = [
        ([10, -20, 30], Rot::IDENTITY),
        ([-300, 7, 0], Rot::IDENTITY),
        ([0; 3], rot),
    ];
    for (k, (t, r)) in placed.iter().enumerate() {
        let id = 2 + 2 * k as u32;
        nodes.insert(
            id,
            Node::Transform {
                child: id + 1,
                translation: *t,
                rot: *r,
            },
        );
        nodes.insert(
            id + 1,
            Node::Shape {
                models: vec![k as u32],
            },
        );
    }
    let mut palette = test_palette();
    palette[0] = [0; 4];
    let file = VoxFile {
        version: 200,
        models,
        palette,
        nodes,
        imap_present: false,
    };
    let bytes = encode(&file);
    let back = VoxFile::parse(&bytes).unwrap();
    assert_eq!(back, file);
    assert_eq!(encode(&back), bytes, "a re-encode is byte-identical");
    let p = back.placements();
    assert_eq!(
        p.iter()
            .map(|p| (p.model, p.translation, p.rot))
            .collect::<Vec<_>>(),
        vec![
            (0, [10, -20, 30], Rot::IDENTITY),
            (1, [-300, 7, 0], Rot::IDENTITY),
            (2, [0; 3], rot)
        ]
    );
    // PACK leads MAIN's children when there is more than one model.
    assert_eq!(&bytes[20..24], b"PACK");
    assert_eq!(u32::from_le_bytes(bytes[32..36].try_into().unwrap()), 3);
}

/// Every one of the 24 rotations writes the `_r` byte that `decode_rotation` reads back to
/// it, and the identity is byte 4.
#[test]
fn every_rotation_encodes_to_the_byte_that_decodes_to_it() {
    assert_eq!(encode_rotation(Rot::IDENTITY), 4);
    for r in Rot::all() {
        assert_eq!(decode_rotation(encode_rotation(r)), Ok(r), "{r:?}");
    }
}

/// One model with no scene graph writes no PACK and parses back unchanged, with the default
/// palette written out as RGBA.
#[test]
fn a_single_model_without_a_scene_graph_round_trips() {
    let file = VoxFile {
        version: 150,
        models: vec![vault_model()],
        palette: DEFAULT_PALETTE,
        nodes: Default::default(),
        imap_present: false,
    };
    let bytes = encode(&file);
    assert_eq!(&bytes[20..24], b"SIZE");
    assert_eq!(VoxFile::parse(&bytes).unwrap(), file);
}
