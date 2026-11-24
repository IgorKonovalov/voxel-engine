//! The entities library under `assets/entities/` with no device: every entry
//! (`<name>/<name>.voxtext`, `.vox` and `.rig`) in step and built against the fly-through's
//! tables, so a new entry is checked by being there; and the spider's rows and clips.

use std::path::{Path, PathBuf};

use flythrough::viewer::{Viewer, material};
use flythrough::{RUBBLE, cave, showcase, showcase_materials, walker};
use svet_assets::{Rig, RigBody, VoxFile, VoxText, encode};
use svet_world::MaterialTable;

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
    let mapped: Vec<(u8, Option<svet_world::MaterialId>)> = rig
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

/// The library's rigs built against the showcase's table carry their bodies and strides in
/// world units (detail 2: two model voxels to a world voxel): the walker a ground body, the
/// spider a surface body, each a non-zero `walk` stride.
#[test]
fn the_committed_library_builds() {
    let mut t = showcase_table();
    let mut build = |name: &str| {
        let dir = library().join(name);
        let rig =
            Rig::parse(&std::fs::read_to_string(dir.join(format!("{name}.rig"))).unwrap()).unwrap();
        let vox = VoxFile::parse(&std::fs::read(dir.join(&rig.vox)).unwrap()).unwrap();
        let lookup = t.clone();
        let mut rows = svet_assets::PaletteRows::new(&mut t, svet_assets::DEFAULT_EMISSION_FACTOR);
        rig.build(&vox, &|n| material(&lookup, n), &mut rows)
            .unwrap()
    };
    let walk_stride = |m: &svet_assets::RigModel| {
        m.clips
            .iter()
            .find(|(n, _)| n == "walk")
            .map(|(_, c)| c.stride())
    };
    let walker = build("walker");
    assert_eq!(
        walker.body,
        Some(RigBody::Ground {
            radius: 512,
            height: 3328,
            step: 512
        })
    );
    assert_eq!(walk_stride(&walker), Some(2048));
    let spider = build("spider");
    assert_eq!(
        spider.body,
        Some(RigBody::Surface {
            radius: 1536,
            reach: 384
        })
    );
    assert_eq!(walk_stride(&spider), Some(3072));
}

/// Plan 0079 Phase 4 done-when 3: the walker's `walk` (30 ticks, stride 2048) follows the
/// distance: 2048 units moved is clip tick 0 again, exactly, and 1024 is tick 15; a wandering
/// walker that stands still fades into `idle` over 8 ticks (weight 32 a tick from the last
/// walk pose) and at the 8th tick stands in `blend(last walk pose, idle at tick 0, 256)`.
#[test]
fn the_walk_clip_follows_the_distance() {
    use flythrough::entities::mover::{Mover, walk_tick};
    use flythrough::viewer::Play;
    use svet_entity::{blend, sample};

    let path = library().join("walker/walker.rig");
    let mut v = Viewer::open(&path, Some("walk".into())).expect("opens");
    assert_eq!(v.build_pending(&showcase_table()), None);
    let l = v.loaded.clone().expect("built");
    let clip = |name: &str| {
        l.clips
            .iter()
            .find(|(n, _)| n == name)
            .expect(name)
            .1
            .clone()
    };
    let (walk, idle) = (clip("walk"), clip("idle"));
    let at = |d: i64| walk.clip_tick(walk_tick(d, &walk).expect("a stride"));
    assert_eq!((at(0), at(1024), at(2048), at(3072)), (0, 15, 0, 15));

    let body = v.body.expect("a body");
    let play = Play::start(&l, 0);
    let rest = sample(&l.model, &l.rest, &walk, 0);
    let mut m = Mover::new(
        body,
        [0.0; 3],
        [0.0, 0.0, 1.0],
        [0.0, -1.0, 0.0],
        (1, 0),
        rest,
    );
    // 1024 units in 24 ticks of 43 (and 8 more): the walk at the distance's tick.
    for _ in 0..24 {
        m.animate(43, true, true, (&l, &play));
    }
    m.animate(8, true, true, (&l, &play));
    assert_eq!(m.distance, 1040);
    let last_walk = sample(&l.model, &l.rest, &walk, 1040 * 30 / 2048);
    assert_eq!(*m.parts(), last_walk);
    let idle0 = sample(&l.model, &l.rest, &idle, 0);
    for k in 1..=8u16 {
        m.animate(0, true, true, (&l, &play));
        assert_eq!(
            *m.parts(),
            blend(&last_walk, &idle0, 32 * k),
            "still tick {k}"
        );
    }
    assert_eq!(*m.parts(), blend(&last_walk, &idle0, 256));
    assert_eq!(m.still, 8);
}
