//! The material table file (ADR-0065): the fly-through's file against the showcase's rows,
//! the canonical round trip, and errors that name their lines.

use svet_assets::{MaterialFile, srgb_to_linear};
use svet_world::{Material, MaterialFlags};

fn flythrough_file() -> String {
    let p = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../apps/flythrough/assets/materials.txt"
    );
    std::fs::read_to_string(p).expect("the fly-through's materials.txt reads")
}

/// The file's first seven rows as the fly-through writes them in Rust (`scene.rs`): the
/// cave's four rows with the ore emissive, then the showcase's crystal, amethyst and ember.
fn showcase_rows() -> Vec<Material> {
    let row = |name: &str, albedo: [f32; 3], emission: [f32; 3]| Material {
        albedo,
        emission,
        ..Material::named(name)
    };
    vec![
        row("rock", [0.42, 0.38, 0.34], [0.0; 3]),
        row("ore", [0.72, 0.45, 0.20], [1.6, 0.9, 0.35]),
        Material {
            flags: MaterialFlags::INDESTRUCTIBLE,
            ..row("shell", [0.12, 0.12, 0.14], [0.0; 3])
        },
        row("rubble", [0.36, 0.33, 0.30], [0.0; 3]),
        row("crystal", [0.35, 0.75, 0.85], [0.14, 0.64, 0.8]),
        row("amethyst", [0.45, 0.25, 0.60], [0.3, 0.1, 0.55]),
        row("ember", [0.60, 0.22, 0.10], [1.0, 0.12, 0.03]),
    ]
}

/// Plan 0083 Phase 2 done-when 1: the fly-through's `materials.txt` holds the cave's and the
/// showcase's first rows row for row (id, name, every field) as its first seven, then the
/// castle's, the rigs' wood, the outdoor hills', the showcase's base and other glowing rows
/// and the outdoor hills' other leaves; `canonical` of the file parses back equal.
#[test]
fn a_table_file_round_trips() {
    let file = MaterialFile::parse(&flythrough_file()).expect("parses");
    let want = showcase_rows();
    for (i, w) in want.iter().enumerate() {
        assert_eq!(&file.rows[i], w, "row {}", i + 1);
        assert_eq!(file.row(&w.name).map(|(id, _)| id), Some(i as u16 + 1));
    }
    let table = file.table();
    assert_eq!(table.len(), 1 + file.rows.len());
    assert_eq!(table.get(5).unwrap().name, "crystal");
    let names: Vec<&str> = file.rows[7..].iter().map(|m| m.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "castle stone",
            "castle trim",
            "castle slate",
            "castle wood",
            "castle iron",
            "castle glass",
            "castle torch",
            "rig wood",
            "grass",
            "soil",
            "bark",
            "leaf",
            "showcase rock",
            "showcase ore",
            "jade",
            "citrine",
            "rose",
            "sapphire",
            "leaf 2",
            "leaf 3"
        ]
    );
    let glass = file.row("castle glass").unwrap().1;
    assert!(glass.flags.contains(MaterialFlags::TRANSLUCENT));
    assert_eq!(glass.opacity, 0.6);
    let text = file.canonical();
    assert_eq!(MaterialFile::parse(&text).unwrap(), file);
    assert_eq!(MaterialFile::of_table(&table), file);
}

/// Every field and flag, written and read back; `color` decodes sRGB exactly.
#[test]
fn every_field_reads() {
    let text = "\
material a b  # a comment
  color #808080   # mid grey
  roughness 0.25
  emission 1 2 3
  opacity 0.5
  hardness 4
  acoustic 0.1 0.6 0
  payload 18446744073709551615
  flags indestructible translucent crisp
material plain
";
    let file = MaterialFile::parse(text).unwrap();
    let a = &file.rows[0];
    assert_eq!(a.name, "a b");
    assert_eq!(a.albedo, [srgb_to_linear(128); 3]);
    assert_eq!(
        (a.roughness, a.emission, a.opacity, a.hardness),
        (0.25, [1.0, 2.0, 3.0], 0.5, 4.0)
    );
    assert_eq!(a.acoustic, [0.1, 0.6, 0.0]);
    assert_eq!(a.payload, u64::MAX);
    assert_eq!(a.flags, MaterialFlags(7));
    assert_eq!(file.rows[1], Material::named("plain"));
    assert_eq!(MaterialFile::parse(&file.canonical()).unwrap(), file);
}

/// Plan 0083 Phase 2 done-when 2: a bad line names itself.
#[test]
fn a_bad_line_names_itself() {
    let fails = |t: &str| MaterialFile::parse(t).unwrap_err();
    let e = fails("material rock\n  roughness 1\n  color #12\n");
    assert_eq!(e.line, 3);
    assert!(e.message.contains("6 hex digits"), "{e}");
    assert!(e.to_string().starts_with("materials line 3: "), "{e}");
    let e = fails("material rock\n  flags glow\n");
    assert_eq!(e.line, 2);
    assert!(e.message.contains("`glow`"), "{e}");
    let e = fails("material rock\nmaterial ore\nmaterial rock\n");
    assert_eq!(e.line, 3);
    assert!(
        e.message.contains("`rock`") && e.message.contains("twice"),
        "{e}"
    );
    let e = fails("  albedo 1 1 1\n");
    assert_eq!(e.line, 1);
    let e = fails("material rock\n  albedo 1 1\n");
    assert_eq!(e.line, 2);
    let e = fails("material rock\n  shiny 1\n");
    assert!(e.message.contains("`shiny`"), "{e}");
    let e = fails("material rock\n  roughness x\n");
    assert!(e.message.contains("`x`"), "{e}");
    let e = fails("material\n");
    assert_eq!(e.line, 1);
}
