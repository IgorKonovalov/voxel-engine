//! Plan 0083 Phase 4: the structures library with no device. The chapel's files in step, and
//! the chapel placed ahead of the spawn by `--vox`'s path.

use std::collections::BTreeMap;
use std::path::PathBuf;

use flythrough::cave::CaveSource;
use flythrough::structures::{self, CHAPEL_GLASS, CHAPEL_LAMP, VoxArg};
use flythrough::{cave_materials, with_emissive_ore};
use vxe_assets::{VoxFile, encode};
use vxe_world::{ChunkExtent, MaterialId, VoxelCoord, World};

fn chapel_dir() -> PathBuf {
    PathBuf::from(structures::LIBRARY).join("chapel")
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
        let name = vxe_assets::palette_row(&file, i, 0.0).name;
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
    assert_eq!(world.material_at(VoxelCoord(mid)), Some(vxe_world::AIR));
    assert_eq!(world.voxel_hash(), 0x62cc_2a8f_36d6_eaaf, "pinned on Linux");
}

/// `--vox-op`'s path against `--vox`'s: the chapel as ops (the box carve, then a stamp per
/// placement) on the plain cave equals the chapel through the cave's source, voxel for voxel.
#[test]
fn the_chapel_as_ops_equals_the_chapel_from_the_source() {
    let file = structures::read(&chapel_dir().join("chapel.vox")).expect("reads");
    let mut table = with_emissive_ore(&cave_materials());
    structures::add_rows(&file, &mut table);
    let scene = structures::scene(&file, &table, 1).expect("loads");
    let cave = CaveSource::unbounded(1);
    let offset = structures::offset(&scene, cave.spawn(), None);
    let (lo, hi) = structures::clear_box(&scene, offset).expect("a box");
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
    let v = structures::ops(&scene, offset, true);
    assert_eq!(v.ops.len(), 1 + scene.placements.len());
    let mut registry = vxe_world::PrefabRegistry::new();
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
    use vxe_world::{PersistConfig, Residency, StreamConfig, Streamer, World};

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
}
