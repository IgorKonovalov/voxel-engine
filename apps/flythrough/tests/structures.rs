//! Plan 0083 Phases 4 and 6: the structures library with no device. The library's scan and
//! rows, the chapel's files in step, and the chapel placed ahead of the spawn by `--vox`'s
//! path and as ops.

use std::collections::BTreeMap;
use std::path::PathBuf;

use flythrough::cave::CaveSource;
use flythrough::structures::{self, CHAPEL_GLASS, CHAPEL_LAMP, VoxArg};
use flythrough::{cave_materials, with_emissive_ore};
use svet_assets::{VoxFile, encode};
use svet_world::{ChunkExtent, MaterialId, VoxelCoord, World};

fn chapel_dir() -> PathBuf {
    PathBuf::from(structures::LIBRARY).join("chapel")
}

/// Phase 6 done-when 1: the scan of the committed library gives the chapel, then the castle;
/// the chapel adds the 255 rows the README states and the castle none (its rows come from the
/// material file when the table has none). In a scratch library a second file of the same
/// palette adds no row, a directory without its `.vox` is a note, and a file that does not
/// parse is an entry in error with its reason.
#[test]
fn the_library_lists_every_structure() {
    let base = flythrough::walker::with_walker(&cave_materials());
    let dir = PathBuf::from(structures::LIBRARY);
    let mut table = base.clone();
    assert_eq!(structures::add_library_rows(&dir, None, &mut table), [255]);
    for n in flythrough::materials::CASTLE_ROWS {
        assert!(table.rows().iter().any(|m| m.name == n), "{n}");
    }
    let readme = std::fs::read_to_string(dir.join("README.md")).expect("reads");
    assert!(
        readme.contains("rows +255"),
        "the README states the chapel's rows"
    );
    let lib = structures::Library::load(&dir, &table, &base);
    let names: Vec<&str> = lib.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["chapel", structures::CASTLE_NAME]);
    let rows: Vec<usize> = lib.entries.iter().map(|e| e.rows).collect();
    assert_eq!(rows, [255, 0]);
    let chapel = lib.entries[0].scene().expect("the chapel loads");
    assert_eq!((chapel.placements.len(), chapel.size()), (4, [48, 32, 40]));
    assert!(lib.entries[1].building() && lib.entries[1].error().is_none());
    assert!(lib.notes.is_empty(), "{:?}", lib.notes);

    let scratch = std::env::temp_dir().join(format!("svet-structures-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    let bytes = std::fs::read(chapel_dir().join("chapel.vox")).expect("reads");
    for (name, content) in [("a", &bytes[..]), ("b", &bytes[..]), ("d", b"not a vox")] {
        std::fs::create_dir_all(scratch.join(name)).unwrap();
        std::fs::write(scratch.join(name).join(format!("{name}.vox")), content).unwrap();
    }
    std::fs::create_dir_all(scratch.join("c")).unwrap();
    let mut table = base.clone();
    assert_eq!(
        structures::add_library_rows(&scratch, None, &mut table),
        [255, 0, 0]
    );
    let lib = structures::Library::load(&scratch, &table, &base);
    let names: Vec<&str> = lib.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["a", "b", "d", "castle"]);
    let rows: Vec<usize> = lib.entries.iter().map(|e| e.rows).collect();
    assert_eq!(rows, [255, 0, 0, 0]);
    let err = lib.entries[2].error().expect("d is in error");
    assert!(err.contains("d.vox"), "{err}");
    assert_eq!(lib.notes, ["structures: c/ holds no c.vox"]);
    let _ = std::fs::remove_dir_all(&scratch);
}

/// `chapel.vox` is `chapel.voxtext` with its materials, layers and mirror added
/// (`structures::chapel_vox`). `VXE_BLESS_CHAPEL=1` writes it.
#[test]
fn the_chapel_assets_are_in_step() {
    let text = std::fs::read_to_string(chapel_dir().join("chapel.voxtext")).expect("reads");
    let file = structures::chapel_vox(&text).expect("builds");
    let bytes = encode(&file);
    let path = chapel_dir().join("chapel.vox");
    if std::env::var_os("VXE_BLESS_CHAPEL").is_some() {
        std::fs::write(&path, &bytes).expect("writes chapel.vox");
    }
    let committed = std::fs::read(&path).expect("chapel.vox reads");
    assert!(
        committed == bytes,
        "chapel.vox is not chapel.voxtext's: VXE_BLESS_CHAPEL=1 writes it"
    );
    let back = VoxFile::parse(&committed).expect("parses");
    assert_eq!(back, file);
    assert_eq!(back.reflected.len(), 1, "one reflected frame");
    assert!(
        back.layers
            .values()
            .any(|l| l.name == "scaffold" && l.hidden)
    );
}

/// `--vox`'s value: a path, an optional corner and an optional scale.
#[test]
fn the_vox_flag_parses() {
    let a = VoxArg::parse("a/b.vox@1,-2,3*2").unwrap();
    assert_eq!(
        (a.path, a.at, a.scale),
        (PathBuf::from("a/b.vox"), Some([1, -2, 3]), 2)
    );
    let a = VoxArg::parse("c.vox").unwrap();
    assert_eq!((a.at, a.scale), (None, 1));
    assert!(VoxArg::parse("c.vox*5").unwrap_err().contains("1..=4"));
    assert!(VoxArg::parse("c.vox@1,2").is_err());
}

/// Done-when 4: `--vox` of the chapel generates a world whose voxel hash equals a pinned value
/// (integer only, so on every CI arm), with the glass row translucent and the lamp row
/// emissive; the chapel's voxels in the world, counted per row, match the `.vox`'s per-index
/// counts over its placed models (the nave and the tower once, the transept twice through its
/// mirror, the hidden scaffold not at all).
#[test]
fn the_chapel_stands_at_the_spawn() {
    let file = structures::read(&chapel_dir().join("chapel.vox")).expect("reads");
    let mut table = with_emissive_ore(&cave_materials());
    let base = table.len();
    let added = structures::add_rows(&file, &mut table);
    assert_eq!(added, 255, "a row per palette entry");
    let scene = structures::scene(&file, &table, 1).expect("loads");
    let cave = CaveSource::unbounded(1);
    let offset = structures::offset(&scene, cave.spawn(), None);
    let source = structures::source(cave, &scene, offset, true);
    let (lo, hi) = scene.bounds;
    let lo = [0, 1, 2].map(|k| lo[k] + offset[k]);
    let hi = [0, 1, 2].map(|k| hi[k] + offset[k]);
    let m = structures::CLEAR_MARGIN;
    let extent = ChunkExtent::new(
        [0, 1, 2].map(|k| (lo[k] - m).div_euclid(64)),
        [0, 1, 2].map(|k| (hi[k] + m).div_euclid(64) + 1),
    );
    let world = World::generate(extent, &source, &table);

    let row = |i: u8| {
        let name = svet_assets::palette_row(&file, i, 0.0).name;
        let id = table.rows().iter().position(|m| m.name == name).unwrap();
        id as MaterialId
    };
    assert!(usize::from(row(1)) >= base);
    assert!(table.is_translucent(row(CHAPEL_GLASS)));
    let lamp = table.get(row(CHAPEL_LAMP)).unwrap();
    assert!(lamp.emission.iter().all(|&e| e > 0.0), "{lamp:?}");

    // The .vox's counts per index over the placed models, by name.
    let times: BTreeMap<&str, u64> = [("nave", 1), ("transept", 2), ("tower", 1), ("scaffold", 0)]
        .into_iter()
        .collect();
    let mut want: BTreeMap<MaterialId, u64> = BTreeMap::new();
    for (k, model) in file.models.iter().enumerate() {
        let name = file.names[&(2 + 2 * k as u32)].as_str();
        for v in &model.voxels {
            *want.entry(row(v[3])).or_default() += times[name];
        }
    }
    let mut got: BTreeMap<MaterialId, u64> = BTreeMap::new();
    for z in lo[2]..=hi[2] {
        for y in lo[1]..=hi[1] {
            for x in lo[0]..=hi[0] {
                let m = world.material_at(VoxelCoord([x, y, z])).expect("resident");
                if usize::from(m) >= base {
                    *got.entry(m).or_default() += 1;
                }
            }
        }
    }
    want.retain(|_, n| *n > 0);
    assert_eq!(got, want);
    // The box's margin is air: a voxel 5 beyond the box's low x side at mid height.
    let mid = [lo[0] - 5, (lo[1] + hi[1]) / 2, (lo[2] + hi[2]) / 2];
    assert_eq!(world.material_at(VoxelCoord(mid)), Some(svet_world::AIR));
    assert_eq!(world.voxel_hash(), 0x62cc_2a8f_36d6_eaaf, "pinned on Linux");
}

/// `--vox-op`'s path against `--vox`'s: the chapel as ops (a stamp of written air over the
/// box, then a stamp per placement) on the plain cave equals the chapel through the cave's
/// source, voxel for voxel.
#[test]
fn the_chapel_as_ops_equals_the_chapel_from_the_source() {
    let file = structures::read(&chapel_dir().join("chapel.vox")).expect("reads");
    let mut table = with_emissive_ore(&cave_materials());
    structures::add_rows(&file, &mut table);
    let scene = structures::scene(&file, &table, 1).expect("loads");
    let cave = CaveSource::unbounded(1);
    let offset = structures::offset(&scene, cave.spawn(), None);
    let (lo, hi) = structures::clear_box(&scene, offset, 0).expect("a box");
    let extent = ChunkExtent::new(
        lo.map(|v| v.div_euclid(64)),
        hi.map(|v| v.div_euclid(64) + 1),
    );
    let sourced = World::generate(
        extent,
        &structures::source(cave, &scene, offset, true),
        &table,
    );
    let opped = World::generate(extent, &cave, &table);
    let v = structures::ops(&scene, offset, 0, true);
    assert_eq!(v.ops.len(), 1 + scene.placements.len());
    let mut registry = svet_world::PrefabRegistry::new();
    for p in &v.prefabs {
        registry.register(p.clone()).expect("registers");
    }
    for op in &v.ops {
        op.apply(&opped, &registry).expect("applies");
    }
    assert_eq!(opped.store().pending_placements(), 0);
    assert_eq!(opped.voxel_hash(), sourced.voxel_hash());
}

mod reading {
    use std::sync::Arc;

    use flythrough::castle;
    use flythrough::cave::{HallSource, UNBOUNDED_SPAWN};
    use svet_world::{PersistConfig, Residency, StreamConfig, Streamer, World};

    /// Plan 0083 Phase 5's reading: the castle's prefabs stamped at runtime, through the op
    /// path, into the hall streamed around the spawn at `r_full` (the fly-through's 16, then 6
    /// so more of the castle lies past residency): the wall time of the stamps, the chunks
    /// written now and the placements left pending. Run by hand under the perf lock:
    /// `cargo nextest run -p flythrough --test structures --run-ignored only --no-capture
    /// -E 'test(=reading::castle_stamped_at_runtime)'`.
    #[test]
    #[ignore = "reading: the runtime castle's stamp time, by hand under the perf lock"]
    fn castle_stamped_at_runtime() {
        let file = castle::castle_vox();
        let placements = castle::castle_prefabs(&file).expect("mapped");
        let table = flythrough::castle_materials(&flythrough::cave_materials());
        for r_full in [16, 6] {
            let world = World::new(&table);
            let mut s = Streamer::new(
                Arc::new(HallSource::new(1)),
                StreamConfig {
                    residency: Residency {
                        r_full,
                        r_lod: r_full,
                        hysteresis: 1,
                    },
                    workers: 4,
                    persist: PersistConfig::default(),
                },
            );
            let t = flythrough::now();
            s.update(&world, &[UNBOUNDED_SPAWN]);
            s.wait_idle(&world);
            let t1 = flythrough::now();
            let gen_ms = t1.duration_since(t).as_secs_f64() * 1e3;
            let t = flythrough::now();
            let mut touched = std::collections::BTreeSet::new();
            let mut changed = 0u64;
            for (p, o, r) in &placements {
                let res = world.stamp(p, *o, *r).expect("a stamp applies");
                touched.extend(res.touched);
                changed += res
                    .changed_from
                    .iter()
                    .map(|&(_, n)| u64::from(n))
                    .sum::<u64>();
            }
            let ms = flythrough::now().duration_since(t).as_secs_f64() * 1e3;
            println!(
                "reading: castle at r_full {r_full}: {} resident chunks (generated in {gen_ms:.0} \
                 ms), {} stamps in {ms:.1} ms, {} chunks written now, {changed} voxels changed, \
                 {} placements pending",
                world.store().len(),
                placements.len(),
                touched.len(),
                world.store().pending_placements()
            );
        }
    }

    /// Plan 0083 Phase 8's settle reading needs the castle as a `.vox` the fly-through's
    /// `--vox PATH --vox-op` places at runtime: writes `castle::castle_vox()` encoded to
    /// `$VXE_CASTLE_VOX` (default `live/castle.vox` under the workspace). Its palette has no
    /// `MATL`, so its torches are plain rows there. Run by hand:
    /// `cargo nextest run -p flythrough --test structures --run-ignored only
    /// -E 'test(=reading::castle_as_a_vox_file)'`.
    #[test]
    #[ignore = "reading: writes the castle as a .vox for the settle reading, by hand"]
    fn castle_as_a_vox_file() {
        let path = std::env::var("VXE_CASTLE_VOX").map_or_else(
            |_| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../live/castle.vox"),
            std::path::PathBuf::from,
        );
        let bytes = svet_assets::encode(&castle::castle_vox());
        std::fs::write(&path, &bytes).expect("writes");
        println!(
            "reading: castle.vox {} B at {}",
            bytes.len(),
            path.display()
        );
    }

    /// Plan 0083 Phase 6's readings: the structures library's scan and rows at the start, the
    /// frame ghosts' pool bytes (the chapel's edges, the castle's corners), the chapel's icon
    /// time, and the castle's cleared box as a uniform air prefab (its construction, its id
    /// and its stamp into a streamed hall). Run by hand under the perf lock:
    /// `cargo nextest run -p flythrough --test structures --run-ignored only --no-capture
    /// -E 'test(=reading::structure_readings)'`.
    #[test]
    #[ignore = "reading: the structures' start, frame bytes, icon and air box, by hand"]
    fn structure_readings() {
        let ms = |t: std::time::Instant| flythrough::now().duration_since(t).as_secs_f64() * 1e3;
        let dir = std::path::Path::new(flythrough::structures::LIBRARY);
        let base = flythrough::walker::with_walker(&flythrough::cave_materials());
        let t = flythrough::now();
        let mut table = base.clone();
        let rows = flythrough::structures::add_library_rows(dir, None, &mut table);
        let rows_ms = ms(t);
        let t = flythrough::now();
        let lib = flythrough::structures::Library::load(dir, &table, &base);
        println!(
            "reading: library rows {rows:?} in {rows_ms:.1} ms, {} entries loaded in {:.1} ms, \
             table {} -> {} rows",
            lib.entries.len(),
            ms(t),
            base.len(),
            table.len()
        );
        let chapel = lib.entries[0].scene().expect("the chapel").clone();
        let row = flythrough::structures::frame_row(&chapel, &table);
        for (name, size) in [("chapel", chapel.size()), ("castle", [600, 600, 500])] {
            let t = flythrough::now();
            let m = flythrough::structures::frame_model(size, row);
            println!(
                "reading: {name} frame {size:?}: {} parts, {} pool bytes, made in {:.2} ms",
                m.parts().len(),
                flythrough::entities::library::pool_bytes(&m),
                ms(t)
            );
        }
        let model = Arc::new(flythrough::structures::scene_model(&chapel).expect("fits"));
        let t = flythrough::now();
        let icon = flythrough::entities::icon::icon(&model, &table);
        println!(
            "reading: chapel icon {} in {:.1} ms",
            if icon.is_some() { "made" } else { "none" },
            ms(t)
        );

        let size = [620, 620, 510];
        let t = flythrough::now();
        let air = svet_world::Prefab::uniform(size, svet_world::AIR);
        let build_ms = ms(t);
        let t = flythrough::now();
        let id = air.id();
        println!(
            "reading: air box {size:?}: built in {build_ms:.2} ms, {} B, id {id:016x} in {:.1} ms",
            air.bytes(),
            ms(t)
        );
        let world = World::new(&flythrough::castle_materials(&flythrough::cave_materials()));
        let mut s = Streamer::new(
            Arc::new(HallSource::new(1)),
            StreamConfig {
                residency: Residency {
                    r_full: 6,
                    r_lod: 6,
                    hysteresis: 1,
                },
                workers: 4,
                persist: PersistConfig::default(),
            },
        );
        s.update(&world, &[UNBOUNDED_SPAWN]);
        s.wait_idle(&world);
        let at = [
            castle::ORIGIN[0] - 10,
            castle::ORIGIN[1] - 10,
            castle::ORIGIN[2] - 100,
        ];
        let t = flythrough::now();
        let r = world
            .stamp(&air, at, svet_world::Rot::IDENTITY)
            .expect("applies");
        println!(
            "reading: air box stamped in {:.1} ms at r_full 6: {} chunks written now, {} \
             placements pending",
            ms(t),
            r.touched.len(),
            world.store().pending_placements()
        );
    }
}
