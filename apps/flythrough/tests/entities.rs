//! The entities library under `assets/entities/` with no device: every entry
//! (`<name>/<name>.voxtext`, `.vox` and `.rig`) in step and built against the fly-through's
//! tables, so a new entry is checked by being there; and the spider's rows and clips.

use std::path::{Path, PathBuf};

use flythrough::viewer::{Viewer, material};
use flythrough::{RUBBLE, cave, showcase, showcase_materials, walker};
use vxe_assets::{Rig, VoxText, encode};
use vxe_world::MaterialTable;

fn library() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/entities")
}

/// The showcase's table with the walker's rows: the table the clip recorder and
/// `--source showcase --model` draw an entry under.
fn showcase_table() -> MaterialTable {
    walker::with_walker(&showcase_materials())
}

/// The library's entries by directory name, sorted.
fn entries() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(library())
        .expect("the library reads")
        .map(|e| e.expect("an entry"))
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Every entry holds `<name>.voxtext`, `<name>.vox` and `<name>.rig`; the voxtext converts to
/// the committed `.vox`'s bytes; the rig names that `.vox`, builds against the showcase's
/// table with the walker's rows, and has a clip; and the library's README names the entry.
#[test]
fn every_library_entry_is_in_step_and_builds() {
    let names = entries();
    assert!(names.contains(&"walker".to_owned()) && names.contains(&"spider".to_owned()));
    let readme = std::fs::read_to_string(library().join("README.md")).expect("README reads");
    for name in &names {
        let dir = library().join(name);
        let text = std::fs::read_to_string(dir.join(format!("{name}.voxtext")))
            .unwrap_or_else(|e| panic!("{name}.voxtext: {e}"));
        let vt = VoxText::parse(&text).unwrap_or_else(|e| panic!("{name}.voxtext: {e}"));
        let committed = std::fs::read(dir.join(format!("{name}.vox")))
            .unwrap_or_else(|e| panic!("{name}.vox: {e}"));
        assert!(
            encode(&vt.to_vox_file()) == committed,
            "{name}.vox is not {name}.voxtext's: regenerate it with the voxtext example"
        );
        let rig_path = dir.join(format!("{name}.rig"));
        let rig_text =
            std::fs::read_to_string(&rig_path).unwrap_or_else(|e| panic!("{name}.rig: {e}"));
        let rig = Rig::parse(&rig_text).unwrap_or_else(|e| panic!("{name}.rig: {e}"));
        assert_eq!(rig.vox, format!("{name}.vox"), "{name}.rig's vox line");
        let mut v = Viewer::open(&rig_path, None).expect("opens");
        assert_eq!(
            v.build_pending(&showcase_table()),
            None,
            "{name}.rig builds"
        );
        let l = v.loaded.as_ref().expect("built");
        assert!(!l.clips.is_empty(), "{name}.rig has a clip");
        assert!(
            readme.contains(&format!("`{name}`")),
            "the library's README.md lists `{name}`"
        );
    }
}

/// The spider maps its nine colours to the named rows (the showcase's three glowing rows among
/// them) and builds into fourteen parts, the body the root and every other part its child,
/// with the looping clips `walk` (48 ticks), `idle` (120) and `rear` (60) at 30 ticks a
/// second; against the bare cave table, which lacks the showcase's rows, it does not build. A
/// run's own table takes those rows from the material file (`setup::world_table`), so a run
/// builds it on every source.
#[test]
fn the_spider_has_its_rows_parts_and_clips() {
    let path = library().join("spider/spider.rig");
    let rig = Rig::parse(&std::fs::read_to_string(&path).expect("reads")).expect("parses");
    let t = showcase_table();
    let (body_row, eye_row) = walker::rows(&t).expect("the walker's rows");
    let mapped: Vec<(u8, Option<vxe_world::MaterialId>)> = rig
        .materials
        .iter()
        .map(|m| (m.index, material(&t, &m.name)))
        .collect();
    assert_eq!(
        mapped,
        [
            (1, Some(cave::SHELL)),
            (2, Some(RUBBLE)),
            (3, Some(cave::ROCK)),
            (4, Some(cave::ORE)),
            (5, Some(showcase::CRYSTAL)),
            (6, Some(showcase::AMETHYST)),
            (7, Some(showcase::EMBER)),
            (8, Some(body_row)),
            (9, Some(eye_row)),
        ]
    );
    let parents: Vec<Option<&str>> = rig.parts.iter().map(|p| p.parent.as_deref()).collect();
    assert_eq!(parents[0], None);
    assert!(parents[1..].iter().all(|p| *p == Some("body")));
    assert!(rig.clips.iter().all(|c| c.looped));

    let mut v = Viewer::open(&path, Some("walk".into())).expect("opens");
    assert_eq!(v.build_pending(&t), None);
    let l = v.loaded.as_ref().expect("built");
    let legs = (1..=4).flat_map(|i| [format!("leg_l{i}"), format!("leg_r{i}")]);
    let names: Vec<String> = ["body", "abdomen"]
        .map(str::to_owned)
        .into_iter()
        .chain(legs)
        .chain(["palp_l", "palp_r", "fang_l", "fang_r"].map(str::to_owned))
        .collect();
    assert_eq!(l.part_names, names);
    assert_eq!(l.model.parts().len(), 14);
    assert_eq!(l.rate, 30);
    let clips: Vec<(&str, u32)> = l
        .clips
        .iter()
        .map(|(n, c)| (n.as_str(), c.length()))
        .collect();
    assert_eq!(clips, [("walk", 48), ("idle", 120), ("rear", 60)]);
    assert_eq!(v.clip_name(), Some("walk"));

    let mut cave_only = Viewer::open(&path, None).expect("opens");
    let e = cave_only
        .build_pending(&walker::with_walker(&flythrough::cave_materials()))
        .expect("the cave's table has no crystal row");
    assert!(e.contains("unknown material `crystal`"), "{e}");
}
