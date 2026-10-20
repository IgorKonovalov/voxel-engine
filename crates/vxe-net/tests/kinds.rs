//! Every op kind through the server: a stamp, a carve, four drops and a removal, applied,
//! logged and announced with their full results, kept in step on an owned client, replayed
//! from the log, and the server's coarse grid kept current.

mod support;

use std::sync::Arc;

use support::*;
use vxe_net::*;
use vxe_world::*;

/// A box op over the half-open voxel bounds `lo..hi`.
fn box_voxels(lo: [i32; 3], hi: [i32; 3], mode: Mode) -> EditOp {
    let f = |v: [i32; 3]| Fixed3(v.map(|c| i64::from(c) * FIXED_ONE));
    EditOp::box_(f(lo), f(hi), mode)
}

const FLOOR: MaterialId = 3;
const ROCK: MaterialId = 1;
const SHELL: MaterialId = 2;
const RUBBLE: MaterialId = 5;

const PILLAR_A: ([i32; 3], [i32; 3]) = ([72, 24, 8], [80, 32, 40]);
const PILLAR_B: ([i32; 3], [i32; 3]) = ([72, 88, 8], [80, 96, 40]);

/// Air over chunks `[0, 2)³` (voxels `[0, 128)`), a floor 8 voxels deep, and two 3 x 3-cell
/// slabs of rock 32 voxels above it, each on a one-cell pillar.
fn slab_world() -> World {
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [2, 2, 2]),
        &UniformSource(AIR),
        &test_table(),
    );
    for (lo, hi, m) in [
        ([0, 0, 0], [128, 128, 8], FLOOR),
        (PILLAR_A.0, PILLAR_A.1, ROCK),
        ([64, 16, 40], [88, 40, 48], ROCK),
        (PILLAR_B.0, PILLAR_B.1, ROCK),
        ([64, 80, 40], [88, 104, 48], ROCK),
    ] {
        world.apply(&box_voxels(lo, hi, Mode::Fill(m))).unwrap();
    }
    world
}

/// The 9 cells of a slab whose cells span `x` 8..=10, `y` from `y0` and `z` at `z`.
fn slab_cells(y0: i32, z: i32) -> Vec<CellCoord> {
    let mut cells = Vec::new();
    for y in y0..y0 + 3 {
        for x in 8..11 {
            cells.push(CellCoord([x, y, z]));
        }
    }
    cells.sort_unstable();
    cells
}

fn vault() -> Arc<Prefab> {
    Arc::new(Prefab::vault(16, SHELL))
}

/// The session: one op a tick, submitted directly.
fn session(vault_id: u64) -> Vec<Op> {
    let mut ops = vec![
        Op::Stamp {
            prefab: vault_id,
            origin: VoxelCoord([32, 32, 32]),
            rot: Rot::IDENTITY,
        },
        Op::Edit(box_voxels(PILLAR_A.0, PILLAR_A.1, Mode::Carve)),
    ];
    // Each drop of a whole cell moves the slab's cells one cell down.
    for k in 0..4 {
        ops.push(Op::Drop {
            cells: slab_cells(2, 5 - k),
            max_drop: 8,
        });
    }
    ops.push(Op::Edit(box_voxels(PILLAR_B.0, PILLAR_B.1, Mode::Carve)));
    ops.push(Op::Remove {
        cells: slab_cells(10, 5),
        rubble: RUBBLE,
        fraction: (1, 4),
        max_depth: 64,
    });
    ops
}

/// The world hash after the session, from `slab_world()`. Blessed on Linux x86_64, asserted
/// on every arm.
const KINDS_HASH: u64 = 0x2077_5c41_d63d_17c5;

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
        world.count_in_box([64, 16, 8], [87, 39, 15]),
        [(ROCK, 4608)]
    );
    assert_eq!(
        world.count_in_box([64, 80, 8], [87, 103, 9]),
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
    assert_eq!(world.voxel_hash(), KINDS_HASH);
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
