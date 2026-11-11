//! Voxels drawn as text: to `.vox` bytes and a parsed `VoxFile`, and back to identical bytes.

use vxe_assets::{Node, VoxFile, VoxText, encode};

const TWO_MODELS: &str = "\
# a post and a cap
color a 9 255 160 32
color b 1 90 90 90     # grey
model post at 4 -2 0
layer
.b.
bbb
layer
.a.
...
model cap
layer
aa
";

#[test]
fn voxtext_round_trips_through_vox() {
    let vt = VoxText::parse(TWO_MODELS).unwrap();
    let bytes = encode(&vt.to_vox_file());
    let vox = VoxFile::parse(&bytes).unwrap();
    assert_eq!(vox.models.len(), 2);
    // post: 3 x 2 x 2. The first row of a layer is the larger y.
    assert_eq!(vox.models[0].size, [3, 2, 2]);
    assert_eq!(
        vox.models[0].voxels,
        vec![
            [0, 0, 0, 1],
            [1, 0, 0, 1],
            [2, 0, 0, 1],
            [1, 1, 0, 1],
            [1, 1, 1, 9]
        ]
    );
    assert_eq!(vox.models[1].size, [2, 1, 1]);
    assert_eq!(vox.models[1].voxels, vec![[0, 0, 0, 9], [1, 0, 0, 9]]);
    assert_eq!(vox.palette[9], [255, 160, 32, 255]);
    assert_eq!(vox.palette[1], [90, 90, 90, 255]);
    // The names sit on the transforms over each model's shape.
    let named: Vec<(&str, [i32; 3], u32)> = vox
        .names
        .iter()
        .map(|(id, name)| match &vox.nodes[id] {
            Node::Transform {
                child, translation, ..
            } => match &vox.nodes[child] {
                Node::Shape { models } => (name.as_str(), *translation, models[0]),
                n => panic!("{n:?}"),
            },
            n => panic!("{n:?}"),
        })
        .collect();
    assert_eq!(named, vec![("post", [4, -2, 0], 0), ("cap", [0, 0, 0], 1)]);

    // from-vox then to-vox: identical bytes.
    let back = VoxText::from_vox_file(&vox).unwrap();
    assert_eq!(back, vt);
    assert_eq!(encode(&back.to_vox_file()), bytes);
    // The written text parses back to an equal value.
    let text = back.write();
    assert_eq!(VoxText::parse(&text).unwrap(), vt);
    println!("{text}");
}

#[test]
fn voxtext_errors_name_their_line() {
    let cases = [
        ("color a 9 1 2 3\nmodel m\nlayer\nax\n", 4, "`x`"),
        ("color a 9 1 2 3\nmodel m\nlayer\naa\na\n", 5, "`a`"),
        ("color a 9 1 2 3\nmodel m\naa\n", 3, "`aa`"),
        ("color . 9 1 2 3\n", 1, "`.`"),
        ("color a 0 1 2 3\n", 1, "`0`"),
        ("model m\n", 1, "`m`"),
    ];
    for (text, line, token) in cases {
        let e = VoxText::parse(text).unwrap_err();
        assert_eq!(e.line, line, "{text:?}: {e}");
        assert!(e.message.contains(token), "{text:?}: {e}");
    }
}
