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
            Op::Drop { cells, max_drop } => {
                OpResult::Drop(direct.drop_region(cells, *max_drop).unwrap())
            }
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
