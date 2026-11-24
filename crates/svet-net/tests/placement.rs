//! A stamp over a half-resident world through the server (ADR-0066): one logged op, and the
//! same world on a replay with another residency once every chunk has arrived.

use std::sync::Arc;

use svet_net::*;
use svet_world::*;

fn table() -> MaterialTable {
    ["rock", "stamp", "other"]
        .map(Material::named)
        .into_iter()
        .collect()
}

fn streamed(foci: &[VoxelCoord]) -> (World, Streamer) {
    let world = World::new(&table());
    let mut s = Streamer::new(
        Arc::new(UniformSource(1)),
        StreamConfig {
            residency: Residency {
                r_full: 2,
                r_lod: 2,
                hysteresis: 1,
            },
            workers: 2,
            persist: PersistConfig::default(),
        },
    );
    s.update(&world, foci);
    s.wait_idle(&world);
    (world, s)
}

/// Plan 0083 Phase 5 done-when 4: a stamp over a half-resident world (its middle chunk left
/// out between two foci) appears once in the server's log, at kind 2; once the middle chunk
/// arrives the server's world equals a replay of the log on a world that had every chunk
/// resident from the start.
#[test]
fn a_replayed_placement_is_one_op() {
    let ends = [VoxelCoord([0, 0, 0]), VoxelCoord([6 * 64, 0, 0])];
    let all = [ends[0], VoxelCoord([3 * 64, 0, 0]), ends[1]];
    let prefab = Arc::new(Prefab::from_fn([192, 12, 9], |[x, y, z]| {
        ((x * 7 + y * 3 + z) % 5 != 0).then_some(2)
    }));

    let (world, mut s) = streamed(&ends);
    assert!(!world.store().contains(ChunkCoord([3, 0, 0])));
    let world = Arc::new(world);
    let mut server = Server::new(world.clone(), Box::new(NoLogic), 0);
    let id = server.register_prefab(prefab.clone()).unwrap();
    server.submit(Op::Stamp {
        prefab: id,
        origin: VoxelCoord([128, 5, 6]),
        rot: Rot::IDENTITY,
    });
    let report = server.tick();
    assert_eq!((report.applied, report.refused), (1, 0));
    let entries = server.ops().entries();
    let stamps: Vec<u8> = entries
        .iter()
        .filter(|e| e.op().is_some())
        .map(|e| {
            let mut out = Vec::new();
            e.encode(&mut out);
            out[12]
        })
        .collect();
    assert_eq!(stamps, [2], "one op, kind 2");
    assert_eq!(world.store().pending_placements(), 1);

    s.update(&world, &all);
    s.wait_idle(&world);
    assert_eq!(world.store().pending_placements(), 0);

    let (replayed, _t) = streamed(&all);
    let results = replayed.replay(server.ops(), server.prefabs());
    assert!(results.iter().all(Result::is_ok), "{results:?}");
    assert_eq!(replayed.store().pending_placements(), 0);
    assert_eq!(replayed.store().coords(), world.store().coords());
    assert_eq!(replayed.voxel_hash(), world.voxel_hash());
}
