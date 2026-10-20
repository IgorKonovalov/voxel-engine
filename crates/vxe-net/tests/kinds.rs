//! Every op kind through the server: a stamp, a carve, four drops and a removal, applied,
//! logged and announced with their full results, kept in step on an owned client, replayed
//! from the log, and the server's coarse grid kept current.

mod support;

use std::sync::Arc;

use support::*;
use vxe_net::*;
use vxe_world::*;

const ROCK: MaterialId = SLAB_ROCK;
const RUBBLE: MaterialId = SLAB_RUBBLE;

fn vault() -> Arc<Prefab> {
    slab_vault()
}

fn session(vault_id: u64) -> Vec<Op> {
    slab_session(vault_id)
}

#[test]
fn every_kind_applies_through_the_server_and_replays() {
    let world = Arc::new(slab_world());
    let mut server = Server::new(world.clone(), Box::new(NoLogic), 0);
    let id = server.register_prefab(vault()).unwrap();
    let (near, far) = MemChannel::pair();
    server.attach(far);
    let mut shared = Client::shared(world.clone(), near);
    let (near, far) = MemChannel::pair();
    server.attach(far);
    let mut owned = Client::owned(slab_world(), near);
    assert_eq!(owned.register_prefab(vault()), Ok(id));

    // The same ops as direct `World` calls, for the expected results.
    let direct = slab_world();
    let ops = session(id);
    let expected: Vec<OpResult> = ops
        .iter()
        .map(|op| match op {
            Op::Edit(e) => OpResult::Edit(direct.apply(e).unwrap()),
            Op::Stamp { origin, rot, .. } => {
                OpResult::Edit(direct.stamp(&vault(), *origin, *rot).unwrap())
            }
            Op::Drop {
                cells,
                max_drop,
                yield_below,
            } => OpResult::Drop(
                direct
                    .drop_region_through(cells, *max_drop, *yield_below)
                    .unwrap(),
            ),
            Op::Remove {
                cells,
                rubble,
                fraction,
                max_depth,
            } => OpResult::Remove(
                direct
                    .remove_region(cells, *rubble, *fraction, *max_depth)
                    .unwrap(),
            ),
            Op::Pile {
                cells,
                rubble,
                count,
                max_depth,
            } => OpResult::Pile(
                direct
                    .pile_rubble(cells, *rubble, *count, *max_depth)
                    .unwrap(),
            ),
        })
        .collect();

    let mut announced = Vec::new();
    for (tick, op) in ops.iter().enumerate() {
        let seq = server.submit(op.clone());
        let report = server.tick();
        assert_eq!((report.applied, report.refused), (1, 0), "tick {tick}");
        let events = shared.poll();
        let [Event::Applied { seq: s, result, .. }] = &events[..] else {
            panic!("tick {tick}: {events:?}");
        };
        assert_eq!(*s, seq);
        announced.push(result.clone());
        let events = owned.poll();
        assert!(
            matches!(&events[..], [Event::Applied { .. }]),
            "tick {tick}: {events:?}"
        );
        assert_eq!(
            owned.world().voxel_hash(),
            server.world().voxel_hash(),
            "tick {tick}"
        );
    }
    assert_eq!(announced, expected);
    assert_eq!(world.voxel_hash(), direct.voxel_hash());

    // The four drops each moved the slab 8 voxels, and the removal piled a quarter of it.
    let drops: Vec<(i32, Vec<(MaterialId, u32)>)> = announced
        .iter()
        .filter_map(|r| match r {
            OpResult::Drop(d) => Some((d.dropped, d.moved.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(drops, vec![(8, vec![(ROCK, 4608)]); 4]);
    let OpResult::Remove(removed) = &announced[7] else {
        panic!("{:?}", announced[7]);
    };
    assert_eq!(
        (removed.removed.as_slice(), removed.placed),
        (&[(ROCK, 4608)][..], 1152)
    );
    assert_eq!(
        world.count_in_box([64, 16, 8], [87, 39, 15]).unwrap(),
        [(ROCK, 4608)]
    );
    assert_eq!(
        world.count_in_box([64, 80, 8], [87, 103, 9]).unwrap(),
        [(RUBBLE, 1152)]
    );
    let OpResult::Edit(stamped) = &announced[0] else {
        panic!("{:?}", announced[0]);
    };
    assert_eq!(stamped.changed_from, [(AIR, 16 * 16 * 16 - 14 * 14 * 14)]);

    // A stamp of an id nobody registered: refused, not logged, no desync.
    let unknown = id ^ 1;
    let logged = server.ops().len();
    let seq = server.submit(Op::Stamp {
        prefab: unknown,
        origin: VoxelCoord([0, 0, 64]),
        rot: Rot::IDENTITY,
    });
    let report = server.tick();
    assert_eq!((report.applied, report.refused), (0, 1));
    assert_eq!(server.ops().len(), logged);
    for client in [&mut shared, &mut owned] {
        let events = client.poll();
        assert!(
            matches!(
                &events[..],
                [Event::Refused { seq: s, error: EditError::UnknownPrefab(u), .. }]
                    if *s == seq && *u == unknown
            ),
            "{events:?}"
        );
    }
    assert!(!owned.desynced());
    assert_eq!(owned.world().voxel_hash(), server.world().voxel_hash());

    // The server's grid is the grid of its world.
    assert_eq!(*server.coarse(), CoarseGrid::build(server.world()));

    // The drops, with no yield threshold, are logged as kind 3 at 21 + 12n bytes.
    let drops: Vec<(u8, usize)> = server
        .ops()
        .entries()
        .iter()
        .filter(|e| matches!(e.op(), Some(Op::Drop { .. })))
        .map(|e| {
            let mut out = Vec::new();
            e.encode(&mut out);
            (out[12], out.len())
        })
        .collect();
    assert_eq!(drops, [(3, 21 + 12 * 9); 4]);

    // The log, through a save's bytes with its prefab table, replays to the same world.
    let file = LogFile {
        seed: 0,
        source_version: 1,
        extent: world.extent(),
        ops: server.ops().clone(),
        inputs: server.inputs().clone(),
        prefabs: server.prefabs().subset(server.ops().prefab_ids()),
    };
    let file = LogFile::decode(&file.encode()).unwrap();
    assert_eq!(file.prefabs.len(), 1);
    let replayed = slab_world();
    let results = replayed.replay(&file.ops, &file.prefabs);
    assert_eq!(
        results.into_iter().collect::<Result<Vec<_>, _>>(),
        Ok(expected)
    );
    assert_eq!(replayed.voxel_hash(), world.voxel_hash());
    // And re-simulated from the seed with the save's table.
    let fresh = Arc::new(slab_world());
    let resim = Server::resimulate(
        fresh.clone(),
        Box::new(NoLogic),
        0,
        &file.prefabs,
        &file.ops,
        &file.inputs,
    );
    assert_eq!(resim.encode(), server.ops().encode());
    assert_eq!(fresh.voxel_hash(), world.voxel_hash());

    println!("kinds hash {:#018x}", world.voxel_hash());
    assert_eq!(world.voxel_hash(), SLAB_SESSION_HASH);
}

/// One chunk of air holding a floor `[0, 0, 0]..[64, 64, 8]`, a 32-voxel sliver
/// `[32, 32, 39]..[36, 40, 40]` and a 3 x 3-cell slab `[24, 24, 40]..[48, 48, 48]`, all of
/// [`SLAB_FLOOR`]: the slab rests on a cell holding half of 64 solid voxels.
fn sliver_world() -> World {
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [1, 1, 1]),
        &UniformSource(AIR),
        &test_table(),
    );
    for (lo, hi) in [
        ([0, 0, 0], [64, 64, 8]),
        ([32, 32, 39], [36, 40, 40]),
        ([24, 24, 40], [48, 48, 48]),
    ] {
        world
            .apply(&box_voxels(lo, hi, Mode::Fill(SLAB_FLOOR)))
            .unwrap();
    }
    world
}

/// The 3 x 3 slab cells at cell height `z`.
fn sliver_slab_cells(z: i32) -> Vec<CellCoord> {
    let mut v = Vec::new();
    for x in 3..=5 {
        for y in 3..=5 {
            v.push(CellCoord([x, y, z]));
        }
    }
    v
}

/// A kind-5 drop through the sliver and a kind-6 pile on the landed slab, through the server:
/// both announced with their results, logged under their kinds, kept in step on an owned
/// client, and replayed from the log to the live world; the server's grid follows.
#[test]
fn a_drop_through_a_sliver_and_its_pile_go_through_the_server() {
    let world = Arc::new(sliver_world());
    let mut server = Server::new(world.clone(), Box::new(NoLogic), 0);
    let (near, far) = MemChannel::pair();
    server.attach(far);
    let mut shared = Client::shared(world.clone(), near);
    let (near, far) = MemChannel::pair();
    server.attach(far);
    let mut owned = Client::owned(sliver_world(), near);

    server.submit(Op::Drop {
        cells: sliver_slab_cells(5),
        max_drop: 32,
        yield_below: 64,
    });
    assert_eq!(server.tick().applied, 1);
    let events = shared.poll();
    let [
        Event::Applied {
            result: OpResult::Drop(d),
            ..
        },
    ] = &events[..]
    else {
        panic!("{events:?}");
    };
    assert_eq!(
        (d.dropped, d.moved.as_slice(), d.crushed.as_slice()),
        (32, &[(SLAB_FLOOR, 4608)][..], &[(SLAB_FLOOR, 32)][..])
    );

    server.submit(Op::Pile {
        cells: sliver_slab_cells(1),
        rubble: SLAB_RUBBLE,
        count: 32,
        max_depth: 64,
    });
    assert_eq!(server.tick().applied, 1);
    let events = shared.poll();
    let [
        Event::Applied {
            result: OpResult::Pile(p),
            ..
        },
    ] = &events[..]
    else {
        panic!("{events:?}");
    };
    assert_eq!(p.placed, 32);
    assert_eq!(
        p.aabb,
        Some((VoxelCoord([25, 24, 16]), VoxelCoord([47, 47, 16])))
    );
    assert_eq!(
        world.count_in_box([24, 24, 16], [47, 47, 16]).unwrap(),
        [(AIR, 544), (SLAB_RUBBLE, 32)]
    );

    let events = owned.poll();
    assert_eq!(events.len(), 2, "{events:?}");
    assert!(!owned.desynced(), "{events:?}");
    assert_eq!(owned.world().voxel_hash(), world.voxel_hash());

    // Logged under kinds 5 and 6, at 23 + 12n and 27 + 12n bytes for the 9 cells.
    let sizes: Vec<(u8, usize)> = server
        .ops()
        .entries()
        .iter()
        .filter(|e| e.op().is_some())
        .map(|e| {
            let mut out = Vec::new();
            e.encode(&mut out);
            (out[12], out.len())
        })
        .collect();
    assert_eq!(sizes, [(5, 131), (6, 135)]);
    let bytes = server.ops().encode();
    let replayed = sliver_world();
    let results = replayed.replay(&OpLog::decode(&bytes).unwrap(), &PrefabRegistry::new());
    assert!(results.iter().all(Result::is_ok), "{results:?}");
    assert_eq!(replayed.voxel_hash(), world.voxel_hash());
    assert_eq!(*server.coarse(), CoarseGrid::build(&world));
}

/// A client registry that lacks the stamp's prefab refuses the op on its side: a desync.
#[test]
fn an_owned_client_without_the_prefab_desyncs() {
    let mut server = Server::new(Arc::new(slab_world()), Box::new(NoLogic), 0);
    let id = server.register_prefab(vault()).unwrap();
    let (near, far) = MemChannel::pair();
    server.attach(far);
    let mut owned = Client::owned(slab_world(), near);
    server.submit(session(id).remove(0));
    server.tick();
    let events = owned.poll();
    assert!(
        matches!(
            &events[..],
            [Event::Desync {
                seq: 0,
                ours: None,
                ..
            }]
        ),
        "{events:?}"
    );
}

/// Registering an equal prefab again is a no-op under the same id.
#[test]
fn registering_the_same_prefab_twice_keeps_one() {
    let mut server = Server::new(Arc::new(slab_world()), Box::new(NoLogic), 0);
    let id = server.register_prefab(vault()).unwrap();
    assert_eq!(server.register_prefab(vault()), Ok(id));
    assert_eq!(server.prefabs().len(), 1);
    assert_eq!(server.prefabs().get(id).map(|p| p.id()), Some(id));
}
