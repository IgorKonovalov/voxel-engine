//! The `.rig` file: the sample rig built against a three-node `.vox`, the degree table, errors
//! with their lines, the canonical round trip and seeded garbage.

use vxe_assets::{Rig, RigError, VoxFile, VoxText, encode};
use vxe_entity::{Quat, sample};
use vxe_world::MaterialId;

const SAMPLE: &str = "\
# illustrative
vox walker.vox
rate 30                              # ticks per second, for viewers
material 9 glow_amber                # palette index -> a material name
part torso node torso pivot 4 0 3 detail 2
part head  node head  parent torso at 0 12 0  pivot 3 0 3 detail 2
part leg_l node leg   parent torso at -2 0 0  pivot 1.5 8 1.5 detail 2
clip walk ticks 40 loop
key 0  leg_l rot x 30
key 20 leg_l rot x -30
key 40 leg_l rot x 30
";

const GLOW: MaterialId = 7;

fn lookup(name: &str) -> Option<MaterialId> {
    (name == "glow_amber").then_some(GLOW)
}

/// A solid block of palette index 9, as voxtext rows.
fn block(name: &str, size: [usize; 3]) -> String {
    let mut s = format!("model {name}\n");
    for _ in 0..size[2] {
        s.push_str("layer\n");
        for _ in 0..size[1] {
            s.push_str(&"g".repeat(size[0]));
            s.push('\n');
        }
    }
    s
}

/// Three named nodes: torso 8 x 12 x 6, head 6 x 6 x 6, leg 3 x 8 x 3, all palette index 9.
fn three_node_vox() -> VoxFile {
    let text = format!(
        "color g 9 255 160 32\n{}{}{}",
        block("torso", [8, 12, 6]),
        block("head", [6, 6, 6]),
        block("leg", [3, 8, 3])
    );
    let vt = VoxText::parse(&text).unwrap();
    VoxFile::parse(&encode(&vt.to_vox_file())).unwrap()
}

fn within_one(got: Quat, want: [i16; 4]) {
    for k in 0..4 {
        assert!(
            (i32::from(got.0[k]) - i32::from(want[k])).abs() <= 1,
            "{got:?}, want {want:?} within 1"
        );
    }
}

#[test]
fn the_sample_rig_builds_its_model_and_clips() {
    let rig = Rig::parse(SAMPLE).unwrap();
    assert_eq!((rig.vox.as_str(), rig.rate), ("walker.vox", 30));
    let built = rig.build(&three_node_vox(), &lookup).unwrap();
    let parts = built.model.parts();
    assert_eq!(parts.len(), 3);
    assert_eq!(built.part_names, ["torso", "head", "leg_l"]);
    assert_eq!(parts[0].parent, None);
    assert_eq!(parts[1].parent, Some(0), "head hangs from torso");
    assert_eq!(parts[2].parent, Some(0));
    assert_eq!(parts[2].pivot, [384, 2048, 384]);
    assert_eq!(parts[0].pivot, [1024, 0, 768]);
    assert!(parts.iter().all(|p| p.detail == 2));
    assert_eq!(parts[0].volume.size(), [8, 12, 6]);
    assert_eq!(parts[2].volume.size(), [3, 8, 3]);
    // Palette index 9 is the material the lookup gives for glow_amber.
    assert_eq!(parts[1].volume.get([0, 0, 0]), Some(GLOW));
    assert_eq!(parts[2].volume.get([2, 7, 2]), Some(GLOW));
    // The rest layout in 1/256 world voxel: model voxels at the parent's detail 2.
    assert_eq!(built.rest.parts[0].t, [0; 3]);
    assert_eq!(built.rest.parts[1].t, [0, 12 * 128, 0]);
    assert_eq!(built.rest.parts[2].t, [-2 * 128, 0, 0]);
    assert_eq!(built.clips.len(), 1);
    let (name, clip) = &built.clips[0];
    assert_eq!(name, "walk");
    assert_eq!((clip.length(), clip.looped()), (40, true));
    assert_eq!(clip.tracks().len(), 1);
    let track = &clip.tracks()[0];
    assert_eq!(track.part, 2);
    assert_eq!(
        track.keys.iter().map(|k| k.tick).collect::<Vec<_>>(),
        [0, 20, 40]
    );
    within_one(track.keys[1].q, [15826, -4240, 0, 0]);
    // Sampled with the rest pose: the leg keeps its rest place and swings through zero.
    let mid = sample(&built.model, &built.rest, clip, 10);
    assert_eq!(mid.parts[2].t, [-256, 0, 0]);
    within_one(mid.parts[2].q, [16384, 0, 0, 0]);
    assert_eq!(mid.parts[1], built.rest.parts[1]);
    assert!(rig.warnings().is_empty(), "{:?}", rig.warnings());
}

#[test]
fn thirty_degrees_about_x_is_the_table_value() {
    let rig =
        Rig::parse("vox a.vox\npart p model 0 pivot 0 0 0\nclip c ticks 1\nkey 0 p rot x 30\n")
            .unwrap();
    let q = vxe_assets::key_rotation(&rig.clips[0].keys[0].rots);
    within_one(q, [15826, 4240, 0, 0]);
    assert_eq!(q, Quat([15826, 4240, 0, 0]), "the table value exactly");
    // Two turns compose in the order written: 90 about x then 90 about y is r_x * r_y.
    let two = vxe_assets::key_rotation(&[(vxe_assets::Axis::X, 90), (vxe_assets::Axis::Y, 90)]);
    within_one(two, [8192, 8192, 8192, 8192]);
    let other = vxe_assets::key_rotation(&[(vxe_assets::Axis::Y, 90), (vxe_assets::Axis::X, 90)]);
    within_one(other, [8192, 8192, 8192, -8192]);
}

fn fails(text: &str) -> RigError {
    let e = Rig::parse(text).unwrap_err();
    println!("{e}");
    e
}

#[test]
fn errors_name_their_line() {
    let head = "vox a.vox\npart torso model 0 pivot 0 0 0\nclip c ticks 10\n";
    // An unknown part in a key, line 5.
    let e = fails(&format!("{head}key 0 torso\nkey 0 leg_r rot x 3\n"));
    assert_eq!(e.line, Some(5));
    assert!(e.message.contains("`leg_r`"), "{e}");
    // A length of 0.3, line 2.
    let e = fails("vox a.vox\npart torso model 0 pivot 0 0.3 0\n");
    assert_eq!(e.line, Some(2));
    assert!(e.message.contains("`0.3`"), "{e}");
    // A fractional degree, line 4.
    let e = fails(&format!("{head}key 0 torso rot x 30.5\n"));
    assert_eq!(e.line, Some(4));
    assert!(e.message.contains("`30.5`"), "{e}");
    // A parent cycle, blamed on line 4, the part that closes it.
    let e = fails(
        "vox a.vox\npart a model 0 parent c pivot 0 0 0\npart b model 0 parent a pivot 0 0 0\n\
         part c model 0 parent b pivot 0 0 0\n",
    );
    assert_eq!(e.line, Some(4));
    assert!(
        e.message.contains("`b`") && e.message.contains("cycle"),
        "{e}"
    );
    // A part that is its own parent.
    let e = fails("vox a.vox\n\npart a model 0 parent a pivot 0 0 0\n");
    assert_eq!(e.line, Some(3));
    assert!(e.message.contains("`a`"), "{e}");
    // A Display that leads with the line, as a viewer's banner shows it.
    assert!(e.to_string().starts_with("rig line 3: "), "{e}");
}

#[test]
fn build_errors_name_what_is_missing() {
    let vox = three_node_vox();
    let e = Rig::parse("vox a.vox\nmaterial 9 lava\npart t node torso pivot 0 0 0\n")
        .unwrap()
        .build(&vox, &lookup)
        .unwrap_err();
    assert_eq!(e.line, Some(2));
    assert!(e.message.contains("`lava`"), "{e}");
    let e = Rig::parse("vox a.vox\nmaterial 9 glow_amber\npart t node tail pivot 0 0 0\n")
        .unwrap()
        .build(&vox, &lookup)
        .unwrap_err();
    assert!(e.message.contains("`tail`"), "{e}");
    let e = Rig::parse("vox a.vox\npart t node torso pivot 0 0 0\n")
        .unwrap()
        .build(&vox, &lookup)
        .unwrap_err();
    assert!(e.message.contains("[9]"), "an unmapped index is named: {e}");
    let e = Rig::parse("vox a.vox\nmaterial 9 glow_amber\npart t model 3 pivot 0 0 0\n")
        .unwrap()
        .build(&vox, &lookup)
        .unwrap_err();
    assert!(e.message.contains("`3`"), "{e}");
}

#[test]
fn a_far_key_pair_warns() {
    let rig = Rig::parse(
        "vox a.vox\npart p model 0 pivot 0 0 0\nclip c ticks 9\nkey 0 p\nkey 9 p rot y 120\n",
    )
    .unwrap();
    let w = rig.warnings();
    assert_eq!(w.len(), 1);
    assert!(w[0].contains("`p`") && w[0].contains("90 degrees"), "{w:?}");
}

#[test]
fn a_rig_round_trips() {
    let first = Rig::parse(SAMPLE).unwrap();
    let text = first.write();
    println!("{text}");
    assert_eq!(Rig::parse(&text).unwrap(), first);
    // Lengths, moves and several turns survive too.
    let other = "vox m.vox\nrate 24\npart a model 1 at 0.5 -0.00390625 3 pivot 1 2 3.75 detail 4\n\
                 part b node x parent a pivot 0 0 0\nclip one ticks 7\n\
                 key 0 b rot z -45 rot x 3600 move 1.25 0 -2\nkey 7 a\n";
    let first = Rig::parse(other).unwrap();
    assert_eq!(first.parts[0].at, [128, -1, 768]);
    assert_eq!(Rig::parse(&first.write()).unwrap(), first);
    assert_eq!(first.write(), Rig::parse(&first.write()).unwrap().write());
}

/// SplitMix64.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const WORDS: &[&str] = &[
    "vox",
    "rate",
    "material",
    "part",
    "node",
    "model",
    "parent",
    "at",
    "pivot",
    "detail",
    "clip",
    "ticks",
    "loop",
    "key",
    "rot",
    "move",
    "x",
    "y",
    "z",
    "torso",
    "head",
    "leg_l",
    "0",
    "1",
    "2",
    "4",
    "9",
    "30",
    "-30",
    "1.5",
    "0.3",
    "-0.5",
    "99999999999",
    "#",
    "\n",
    " ",
    "\t",
    "glow_amber",
    "walk",
    "40",
    "\u{e9}",
    "4294967296",
];

#[test]
fn the_parser_never_panics() {
    let vox = three_node_vox();
    let mut rng = Rng(0x0063_0002);
    let (mut ok, mut built) = (0, 0);
    let mut check = |text: &str| {
        if let Ok(rig) = Rig::parse(text) {
            ok += 1;
            let _ = rig.write();
            let _ = rig.warnings();
            if rig.build(&vox, &lookup).is_ok() {
                built += 1;
            }
        }
    };
    for _ in 0..10_000 {
        let n = rng.below(64);
        let mut bytes = Vec::new();
        for _ in 0..n {
            if rng.below(4) == 0 {
                bytes.push(rng.next() as u8);
            } else {
                bytes.extend_from_slice(WORDS[rng.below(WORDS.len())].as_bytes());
                bytes.push(if rng.below(3) == 0 { b'\n' } else { b' ' });
            }
        }
        check(&String::from_utf8_lossy(&bytes));
    }
    for end in 0..=SAMPLE.len() {
        check(&SAMPLE[..end]);
    }
    println!("ok {ok}, built {built}");
}
