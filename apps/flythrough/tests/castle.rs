//! `--castle`: the generated castle goes through the `.vox` writer, the parser, the palette
//! map and `StampedSource` over the hall, and the world it makes holds the castle's parts.

mod support;

use flythrough::castle::{self, GLASS, IRON, ORIGIN, STONE, TORCH, TRIM};
use flythrough::cave::{HallSource, ROCK};
use vxe_world::{
    AIR, BRICK_VOXELS, Chunk, ChunkCoord, MaterialId, StampedSource, VoxelCoord, WorldSource,
};

/// The castle's source the way `--castle` builds it: generated, encoded, parsed, mapped.
fn source(file: &vxe_assets::VoxFile) -> StampedSource<HallSource> {
    let bytes = vxe_assets::encode(file);
    let parsed = vxe_assets::VoxFile::parse(&bytes).expect("the castle parses");
    assert_eq!(&parsed, file, "the castle round-trips through its bytes");
    castle::castle_source(
        1,
        castle::castle_prefabs(&parsed).expect("every index is mapped"),
    )
}

/// The material at world voxel `p`, from the chunk holding it.
fn at(src: &impl WorldSource, p: [i32; 3]) -> MaterialId {
    let p = VoxelCoord(p);
    let mut c = Chunk::default();
    src.chunk(p.chunk(), &mut c);
    c.brick(p.brick_slot()).get(p.index_in_brick())
}

/// World voxel of castle-local `(u, v, w)`.
fn world(l: [i32; 3]) -> [i32; 3] {
    [0, 1, 2].map(|k| ORIGIN[k] + l[k])
}

/// The chunks the castle's box meets: x 2..=11, y -5..=5, z -2..=6.
fn castle_chunks() -> Vec<ChunkCoord> {
    let mut out = Vec::new();
    for z in -2..=6 {
        for y in -5..=5 {
            for x in 2..=11 {
                out.push(ChunkCoord([x, y, z]));
            }
        }
    }
    out
}

/// Voxels of `m` over `chunks`.
fn count(src: &impl WorldSource, chunks: &[ChunkCoord], m: MaterialId) -> u64 {
    let mut n = 0;
    for &coord in chunks {
        let mut c = Chunk::default();
        src.chunk(coord, &mut c);
        for b in c.bricks() {
            n += (0..BRICK_VOXELS).filter(|&i| b.get(i) == m).count() as u64;
        }
    }
    n
}

#[test]
fn the_castle_encodes_to_the_same_bytes_on_any_thread_count() {
    let a = vxe_assets::encode(&castle::castle_vox_on(1));
    let b = vxe_assets::encode(&castle::castle_vox_on(4));
    assert_eq!(a.len(), b.len());
    assert!(a == b, "the castle's bytes depend on the thread count");
    assert_eq!(vxe_assets::encode(&castle::castle_vox_on(4)), b);
}

#[test]
fn the_tiles_cover_the_box_in_27_models_of_at_most_256() {
    let t = castle::tiles();
    assert_eq!(t.len(), 27);
    assert!(t.iter().all(|t| t.size.iter().all(|&s| s <= 256)));
    let total: usize = t.iter().map(castle::Tile::len).sum();
    assert_eq!(total, 600 * 600 * 500);
    assert!(t.iter().all(|t| t.len() % 64 == 0));
    assert_eq!(t.iter().filter(|t| t.dungeon()).count(), 9);
}

/// The gate, the wall beside it, the spawn's air and the rock under the floor, and every
/// stone voxel the generator lists lands in the world as castle stone.
#[test]
fn the_castle_stands_in_the_hall_with_its_gate_facing_the_spawn() {
    let file = castle::castle_vox();
    let src = source(&file);
    let stone = castle::material(STONE);
    // The gate arch's centre: 10 voxels into the wall, 3 m up.
    assert_eq!(at(&src, [132 + 60 + 10, 32, 12 + 30]), AIR);
    assert_eq!(world([70, 300, 30]), [202, 32, 42]);
    // The curtain wall 120 voxels to the side of the gate, clear of its towers.
    assert_eq!(at(&src, [202, 152, 42]), stone);
    assert_eq!(at(&src, [32, 32, 32]), AIR, "the spawn");
    assert_eq!(at(&src, [32, 32, 0]), ROCK, "under the hall floor");
    // A torch on each side of the arch.
    for v in castle::GATE_TORCH_V {
        assert_eq!(at(&src, world([44, v + 1, 41])), castle::material(TORCH));
    }
    let listed = castle::index_counts(&file)[usize::from(STONE)];
    assert!(listed > 0);
    assert_eq!(count(&src, &castle_chunks(), stone), listed);
}

/// `castle.rs` is integer only.
#[test]
fn the_castle_generator_holds_no_floating_point() {
    let src = include_str!("../src/castle.rs");
    for t in ["f32", "f64"] {
        assert!(!src.contains(t), "castle.rs names {t}");
    }
}

/// Voxel probes of every part: the nave, its vault's rib, the rose window's glass, a cell's
/// bars, the stair, the crypt, the rock around the dungeon and the gate's torches; and the
/// kept prefabs' sizes and bytes.
#[test]
fn the_whole_castle_holds_its_parts() {
    let file = castle::castle_vox();
    let placements = castle::castle_prefabs(&file).expect("every index is mapped");
    // Each kept prefab has its tile's size.
    let sizes: Vec<usize> = file
        .models
        .iter()
        .map(|m| m.size.iter().map(|&s| s as usize).product())
        .collect();
    let lens: Vec<usize> = placements.iter().map(|(p, _, _)| p.len()).collect();
    assert_eq!(lens, sizes);
    let bytes = castle::prefab_bytes(placements.iter().map(|(p, _, _)| &**p));
    let want: u64 = sizes
        .iter()
        .map(|&n| 2 * n as u64 + 8 * (n as u64).div_ceil(64))
        .sum();
    assert_eq!(bytes, want);
    println!(
        "castle: {} of 27 tiles kept, {bytes} prefab bytes",
        file.models.len()
    );
    if file.models.len() == 27 {
        assert_eq!(bytes, 382_500_000);
    }
    let src = castle::castle_source(1, placements);
    let m = castle::material;
    let probe = |l: [i32; 3]| at(&src, world(l));
    assert_eq!(probe([380, 300, 100]), AIR, "the nave under the vault");
    assert_eq!(
        probe([340, 233, 140]),
        m(TRIM),
        "a transverse rib at the springing line"
    );
    assert_eq!(probe([266, 310, 172]), m(GLASS), "the rose window's glass");
    assert_eq!(probe([246, 437, -90]), m(IRON), "a cell's bar");
    assert_eq!(probe([247, 437, -90]), AIR, "between two bars");
    assert_eq!(probe([200, 450, -50]), AIR, "the stair's axis");
    assert_eq!(probe([185, 448, -50]), m(STONE), "a step beside it");
    assert_eq!(probe([345, 285, -80]), AIR, "the crypt's central bays");
    assert_eq!(
        probe([360, 300, -80]),
        m(STONE),
        "the crypt's middle pillar"
    );
    assert_eq!(
        probe([161, 450, -50]),
        ROCK,
        "10 voxels outside the stair's lining"
    );
    for v in castle::GATE_TORCH_V {
        assert_eq!(probe([44, v + 1, 41]), m(TORCH));
    }
}

/// Every shot's eye is air with at least 10 voxels of air along each axis both ways.
#[test]
fn every_shot_has_room_around_its_eye() {
    let file = castle::castle_vox();
    let src = castle::castle_source(1, castle::castle_prefabs(&file).unwrap());
    for shot in castle::CastleShot::ALL {
        let (eye, _, _) = shot.pose();
        assert_eq!(at(&src, eye), AIR, "{shot:?}");
        for k in 0..3 {
            for d in (-10..=10).filter(|&d| d != 0) {
                let mut p = eye;
                p[k] += d;
                assert_eq!(at(&src, p), AIR, "{shot:?} at {p:?}");
            }
        }
    }
    assert_eq!(
        castle::CastleShot::Gate.pose().0,
        [32, 32, 32],
        "the gate is the spawn"
    );
}

/// The gate golden's world: chunks x -1..4, y -4..5, z -1..4 around the spawn's view, built
/// from only the castle tiles that meet them.
fn gate_world() -> (vxe_world::World, usize) {
    let extent = vxe_world::ChunkExtent::new([-1, -4, -1], [4, 5, 4]);
    let (lo, hi) = (extent.min.0.map(|c| c * 64), extent.max.0.map(|c| c * 64));
    let file = castle::castle_vox_where(4, castle::tiles_meeting(lo, hi));
    let tiles = castle::tiles()
        .iter()
        .filter(|t| castle::tiles_meeting(lo, hi)(t))
        .count();
    let src = source(&file);
    drop(file);
    let table =
        flythrough::castle_materials(&flythrough::with_emissive_ore(&flythrough::cave_materials()));
    (vxe_world::World::generate(extent, &src, &table), tiles)
}

/// The process's peak resident set in bytes from `/proc/self/status`, where there is one.
fn peak_rss() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    let kb: u64 = s
        .lines()
        .find_map(|l| l.strip_prefix("VmHWM:"))?
        .trim()
        .trim_end_matches("kB")
        .trim()
        .parse()
        .ok()?;
    Some(kb * 1024)
}

/// `castle_gate.png`: the spawn pose facing the gate, the headlamp on and the torches lit,
/// after 16 frames of history, on lavapipe. The world holds only the 6 tiles that meet its
/// extent, and the process peaks under 1 GB.
#[test]
fn castle_gate_golden() {
    use support::*;
    let (world, tiles) = gate_world();
    assert_eq!(tiles, 6);
    let Some(mut rig) = LookRig::new(&world, None) else {
        return;
    };
    let (eye, yaw, pitch) = castle::CastleShot::Gate.pose();
    let camera = vxe_render::Camera {
        position: eye.map(|v| v as f32 + 0.5).into(),
        yaw: (yaw as f32).to_radians(),
        pitch: (pitch as f32).to_radians(),
        vfov: 70f32.to_radians(),
    };
    rig.gpu.set_focus(VoxelCoord(eye));
    for _ in 0..4 {
        rig.gpu
            .sync_now(&world, &rig.device, &rig.queue)
            .expect("mirror fits the adapter");
    }
    for _ in 0..16 {
        rig.frame.step(
            &rig.device,
            &rig.queue,
            &rig.gpu,
            &camera,
            &vxe_render::LightParams::headlamp(),
        );
    }
    let img = vxe_render::readback::read_rgba8(&rig.device, &rig.queue, rig.frame.target());
    println!(
        "castle_gate.png: {} light samples, bins covering 1 %: {}",
        rig.gpu.light_samples(),
        bins_covering(&img, 0.01)
    );
    assert_golden("castle_gate.png", &img);
    if let Some(peak) = peak_rss() {
        println!("castle_gate.png: peak RSS {peak} B");
        assert!(peak < 1 << 30, "peak RSS {peak} B");
    }
}
