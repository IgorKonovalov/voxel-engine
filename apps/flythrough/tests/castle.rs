//! `--castle`: the generated castle goes through the `.vox` writer, the parser, the palette
//! map and `StampedSource` over the hall, and the world it makes holds the castle's parts.

use flythrough::castle::{self, ORIGIN, STONE, TORCH};
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
    for v in [275, 323] {
        assert_eq!(at(&src, world([44, v + 1, 41])), castle::material(TORCH));
    }
    let listed = castle::index_counts(&file)[usize::from(STONE)];
    assert!(listed > 0);
    assert_eq!(count(&src, &castle_chunks(), stone), listed);
}
