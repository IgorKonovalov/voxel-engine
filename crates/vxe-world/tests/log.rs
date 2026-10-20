//! The op log, input log and log file encodings (ADR-0007): entry sizes, round trips, the
//! decode errors, and replay.

mod support;

use support::*;
use vxe_world::wire::*;
use vxe_world::*;

fn p(x: i64, y: i64, z: i64) -> Fixed3 {
    Fixed3([x, y, z])
}

fn sphere_carve() -> EditOp {
    EditOp::sphere(p(1, -2, 3), 4 * FIXED_ONE, Mode::Carve)
}

fn box_fill() -> EditOp {
    EditOp::box_(p(-5, 6, -7), p(8, -9, 10), Mode::Fill(2))
}

fn capsule_replace() -> EditOp {
    EditOp::capsule(
        p(11, 12, -13),
        p(-14, 15, 16),
        3 * FIXED_ONE + 7,
        Mode::Replace { from: 3, to: 5 },
    )
}

fn entry_bytes(tick: u64, seq: u32, op: EditOp) -> Vec<u8> {
    op_entry_bytes(tick, seq, Op::Edit(op))
}

fn op_entry_bytes(tick: u64, seq: u32, op: Op) -> Vec<u8> {
    let mut out = Vec::new();
    LogEntry::Op { tick, seq, op }.encode(&mut out);
    out
}

/// `n` distinct cells, x fastest over a 16-wide slab.
fn cells(n: usize) -> Vec<CellCoord> {
    (0..n as i32)
        .map(|i| CellCoord([i % 16 - 8, (i / 16) % 16 - 8, i / 256 - 3]))
        .collect()
}

fn stamp_op(prefab: u64) -> Op {
    Op::Stamp {
        prefab,
        origin: VoxelCoord([32, -32, 7]),
        rot: Rot::from_index(5).unwrap(),
    }
}

fn drop_op(n: usize) -> Op {
    Op::Drop {
        cells: cells(n),
        max_drop: 8,
        yield_below: 0,
    }
}

/// A drop through cells under 64 solid voxels: kind 5.
fn through_op(n: usize) -> Op {
    Op::Drop {
        cells: cells(n),
        max_drop: 8,
        yield_below: 64,
    }
}

fn pile_op(n: usize) -> Op {
    Op::Pile {
        cells: cells(n),
        rubble: 4,
        count: 141,
        max_depth: 64,
    }
}

fn remove_op(n: usize) -> Op {
    Op::Remove {
        cells: cells(n),
        rubble: 4,
        fraction: (1, 4),
        max_depth: 64,
    }
}

#[test]
fn entry_sizes_are_the_documented_ones() {
    // tick 8 + seq 4 + kind 1 + mode 1 (+ payload) + shape 1 + payload.
    assert_eq!(entry_bytes(0, 0, sphere_carve()).len(), 47);
    assert_eq!(entry_bytes(0, 0, box_fill()).len(), 65);
    assert_eq!(entry_bytes(0, 0, capsule_replace()).len(), 75);
    // tick 8 + seq 4 + kind 1 + id 8 + origin 12 + rotation 1.
    assert_eq!(op_entry_bytes(0, 0, stamp_op(7)).len(), 34);
    for n in [0usize, 1, 9, 4096] {
        // + kind 1 + max_drop 4 + count 4 + cells.
        assert_eq!(op_entry_bytes(0, 0, drop_op(n)).len(), 21 + 12 * n);
        // + kind 1 + rubble 2 + fraction 8 + max_depth 4 + count 4 + cells.
        assert_eq!(op_entry_bytes(0, 0, remove_op(n)).len(), 31 + 12 * n);
        // + kind 1 + max_drop 4 + yield_below 2 + count 4 + cells.
        assert_eq!(op_entry_bytes(0, 0, through_op(n)).len(), 23 + 12 * n);
        // + kind 1 + rubble 2 + count 4 + max_depth 4 + cell count 4 + cells.
        assert_eq!(op_entry_bytes(0, 0, pile_op(n)).len(), 27 + 12 * n);
    }
    assert_eq!(op_entry_bytes(0, 0, drop_op(9)).len(), 129);
    assert_eq!(op_entry_bytes(0, 0, drop_op(4096)).len(), 49_173);
    assert_eq!(op_entry_bytes(0, 0, through_op(9)).len(), 131);
    assert_eq!(op_entry_bytes(0, 0, through_op(4096)).len(), 49_175);
    assert_eq!(op_entry_bytes(0, 0, pile_op(9)).len(), 135);
    for n in [0usize, 1, 7, 300] {
        let mut out = Vec::new();
        InputEntry {
            tick: 9,
            client: 2,
            bytes: vec![0xAB; n],
        }
        .encode(&mut out);
        assert_eq!(out.len(), 14 + n);
    }
}

#[test]
fn entry_bytes_are_little_endian_fields_in_order() {
    let b = entry_bytes(0x0102_0304_0506_0708, 0x0A0B_0C0D, sphere_carve());
    assert_eq!(&b[0..8], &0x0102_0304_0506_0708u64.to_le_bytes());
    assert_eq!(&b[8..12], &0x0A0B_0C0Du32.to_le_bytes());
    assert_eq!(b[12], OP_EDIT);
    assert_eq!(b[13], MODE_CARVE);
    assert_eq!(b[14], SHAPE_SPHERE);
    assert_eq!(&b[15..23], &1i64.to_le_bytes());
    assert_eq!(&b[23..31], &(-2i64).to_le_bytes());
    assert_eq!(&b[31..39], &3i64.to_le_bytes());
    assert_eq!(&b[39..47], &(4 * FIXED_ONE).to_le_bytes());

    let b = entry_bytes(1, 2, box_fill());
    assert_eq!((b[13], b[14], b[15], b[16]), (MODE_FILL, 2, 0, SHAPE_BOX));
    let b = entry_bytes(1, 2, capsule_replace());
    assert_eq!(&b[13..19], &[MODE_REPLACE, 3, 0, 5, 0, SHAPE_CAPSULE]);

    let b = op_entry_bytes(1, 2, stamp_op(0x0102_0304_0506_0708));
    assert_eq!(b[12], OP_STAMP);
    assert_eq!(&b[13..21], &0x0102_0304_0506_0708u64.to_le_bytes());
    assert_eq!(&b[21..25], &32i32.to_le_bytes());
    assert_eq!(&b[25..29], &(-32i32).to_le_bytes());
    assert_eq!(&b[29..33], &7i32.to_le_bytes());
    assert_eq!(b[33], 5);
    let b = op_entry_bytes(1, 2, drop_op(2));
    assert_eq!(b[12], OP_DROP);
    assert_eq!(&b[13..17], &8i32.to_le_bytes());
    assert_eq!(&b[17..21], &2u32.to_le_bytes());
    assert_eq!(&b[21..25], &(-8i32).to_le_bytes());
    let b = op_entry_bytes(1, 2, remove_op(1));
    assert_eq!(b[12], OP_REMOVE);
    assert_eq!(&b[13..15], &4u16.to_le_bytes());
    assert_eq!(&b[15..23], &[1, 0, 0, 0, 4, 0, 0, 0]);
    assert_eq!(&b[23..27], &64i32.to_le_bytes());
    assert_eq!(&b[27..31], &1u32.to_le_bytes());
    let b = op_entry_bytes(1, 2, through_op(2));
    assert_eq!(b[12], OP_DROP_THROUGH);
    assert_eq!(OP_DROP_THROUGH, 5);
    assert_eq!(&b[13..17], &8i32.to_le_bytes());
    assert_eq!(&b[17..19], &64u16.to_le_bytes());
    assert_eq!(&b[19..23], &2u32.to_le_bytes());
    assert_eq!(&b[23..27], &(-8i32).to_le_bytes());
    let b = op_entry_bytes(1, 2, pile_op(1));
    assert_eq!(b[12], OP_PILE);
    assert_eq!(OP_PILE, 6);
    assert_eq!(&b[13..15], &4u16.to_le_bytes());
    assert_eq!(&b[15..19], &141u32.to_le_bytes());
    assert_eq!(&b[19..23], &64i32.to_le_bytes());
    assert_eq!(&b[23..27], &1u32.to_le_bytes());

    let mut b = Vec::new();
    InputEntry {
        tick: 7,
        client: 0x0102_0304,
        bytes: vec![9, 8, 7],
    }
    .encode(&mut b);
    assert_eq!(
        b,
        [7, 0, 0, 0, 0, 0, 0, 0, 4, 3, 2, 1, 3, 0, 9, 8, 7].to_vec()
    );
}

/// One value of every tag: each mode on each shape, a stamp at each rotation index, a drop
/// of each kind, a removal and a pile (one of 4 096 cells), each op result kind, and each
/// error.
#[test]
fn every_tag_round_trips() {
    let modes = [
        Mode::Carve,
        Mode::Fill(7),
        Mode::Replace { from: 1, to: 65535 },
    ];
    let shapes = [
        Shape::Sphere {
            c: p(-1, 2, -3),
            r: 99,
        },
        Shape::Box_ {
            min: p(i64::MIN, 0, 5),
            max: p(i64::MAX, 1, 6),
        },
        Shape::Capsule {
            a: p(1, 2, 3),
            b: p(4, 5, 6),
            r: -1,
        },
    ];
    for mode in modes {
        for shape in shapes {
            let op = Op::Edit(EditOp { shape, mode });
            let mut out = Vec::new();
            encode_op(&mut out, &op);
            let mut r = Reader::new(&out);
            assert_eq!(decode_op(&mut r), Ok(op));
            assert!(r.is_empty());
        }
    }

    let results = [
        EditResult::default(),
        EditResult {
            changed_from: vec![(0, 1), (3, 4169), (65535, u32::MAX)],
            touched: vec![ChunkCoord([-1, 0, 2]), ChunkCoord([i32::MIN, i32::MAX, 0])],
            aabb: Some((VoxelCoord([-10, -9, -8]), VoxelCoord([10, 9, 8]))),
        },
    ];
    for res in results {
        let mut out = Vec::new();
        encode_result(&mut out, &res);
        assert_eq!(
            out.len(),
            2 + 6 * res.changed_from.len()
                + 2
                + 12 * res.touched.len()
                + 1
                + if res.aabb.is_some() { 24 } else { 0 }
        );
        let mut r = Reader::new(&out);
        assert_eq!(decode_result(&mut r), Ok(res));
        assert!(r.is_empty());
    }

    let mut ops: Vec<Op> = (0..24)
        .map(|i| Op::Stamp {
            prefab: u64::MAX - i,
            origin: VoxelCoord([i32::MIN, i32::MAX, -(i as i32)]),
            rot: Rot::from_index(i as u8).unwrap(),
        })
        .collect();
    ops.extend([drop_op(0), drop_op(4096), remove_op(0), remove_op(4096)]);
    ops.extend([through_op(0), through_op(4096), pile_op(0), pile_op(4096)]);
    ops.push(Op::Drop {
        cells: cells(3),
        max_drop: i32::MIN,
        yield_below: 0,
    });
    ops.push(Op::Drop {
        cells: cells(3),
        max_drop: i32::MAX,
        yield_below: u16::MAX,
    });
    ops.push(Op::Pile {
        cells: cells(2),
        rubble: u16::MAX,
        count: u32::MAX,
        max_depth: i32::MIN,
    });
    ops.push(Op::Remove {
        cells: cells(5),
        rubble: u16::MAX,
        fraction: (u32::MAX, 0),
        max_depth: -1,
    });
    for op in ops {
        let mut out = Vec::new();
        encode_op(&mut out, &op);
        let kind = match &op {
            Op::Drop { yield_below: 0, .. } => OP_DROP,
            Op::Drop { .. } => OP_DROP_THROUGH,
            Op::Pile { .. } => OP_PILE,
            _ => out[0],
        };
        assert_eq!(out[0], kind, "{op:?}");
        let mut r = Reader::new(&out);
        assert_eq!(decode_op(&mut r), Ok(op));
        assert!(r.is_empty());
    }
    assert_eq!(Rot::from_index(23).map(Rot::index), Some(23));
    assert_eq!(Rot::from_index(24), None);
    // A rotation index past 23 is an unknown tag.
    let mut out = Vec::new();
    encode_op(&mut out, &stamp_op(1));
    *out.last_mut().unwrap() = 24;
    assert_eq!(
        decode_op(&mut Reader::new(&out)),
        Err(WireError::UnknownKind(24))
    );
    // A cell count past the bytes is truncated, not allocated.
    let mut out = Vec::new();
    encode_op(&mut out, &drop_op(1));
    out[5..9].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(
        decode_op(&mut Reader::new(&out)),
        Err(WireError::Truncated { at: 0 })
    );

    let extent = (
        vec![ChunkCoord([-1, 0, 2])],
        Some((VoxelCoord([-10, -9, -8]), VoxelCoord([10, 9, 8]))),
    );
    let op_results = [
        OpResult::Edit(EditResult::default()),
        OpResult::Edit(EditResult {
            changed_from: vec![(3, 4169)],
            touched: extent.0.clone(),
            aabb: extent.1,
        }),
        OpResult::Drop(DropResult::default()),
        OpResult::Drop(DropResult {
            moved: vec![(1, 4608), (65535, 1)],
            dropped: 8,
            crushed: Vec::new(),
            touched: extent.0.clone(),
            aabb: extent.1,
        }),
        OpResult::Drop(DropResult {
            moved: vec![(1, 4608)],
            dropped: 32,
            crushed: vec![(1, 32), (65535, u32::MAX)],
            touched: extent.0.clone(),
            aabb: extent.1,
        }),
        OpResult::Remove(RemoveResult::default()),
        OpResult::Remove(RemoveResult {
            removed: vec![(1, 4608)],
            placed: 1152,
            touched: extent.0.clone(),
            aabb: extent.1,
        }),
        OpResult::Pile(PileResult::default()),
        OpResult::Pile(PileResult {
            placed: 32,
            touched: extent.0.clone(),
            aabb: extent.1,
        }),
    ];
    let kinds = [
        OP_EDIT, OP_EDIT, OP_DROP, OP_DROP, OP_DROP, OP_REMOVE, OP_REMOVE, OP_PILE, OP_PILE,
    ];
    for (res, kind) in op_results.into_iter().zip(kinds) {
        let mut out = Vec::new();
        encode_op_result(&mut out, &res);
        assert_eq!(out[0], kind);
        let mut r = Reader::new(&out);
        assert_eq!(decode_op_result(&mut r), Ok(res));
        assert!(r.is_empty());
    }
    assert_eq!(
        decode_op_result(&mut Reader::new(&[2])),
        Err(WireError::UnknownKind(2))
    );

    let err = EditError::NotResident(ChunkCoord([4, -1, 1]));
    let mut out = Vec::new();
    encode_error(&mut out, &err);
    assert_eq!(out[0], ERROR_NOT_RESIDENT);
    assert_eq!(out.len(), 13);
    assert_eq!(decode_error(&mut Reader::new(&out)), Ok(err));

    let err = EditError::UnknownPrefab(0xDEAD_BEEF_0000_0001);
    let mut out = Vec::new();
    encode_error(&mut out, &err);
    assert_eq!(out[0], ERROR_UNKNOWN_PREFAB);
    assert_eq!(ERROR_UNKNOWN_PREFAB, 3);
    assert_eq!(out.len(), 9);
    assert_eq!(decode_error(&mut Reader::new(&out)), Ok(err));
    for (i, what) in Field::ALL.into_iter().enumerate() {
        let err = EditError::OutOfRange { what };
        let mut out = Vec::new();
        encode_error(&mut out, &err);
        assert_eq!(out, [ERROR_OUT_OF_RANGE, i as u8 + 1]);
        assert_eq!(decode_error(&mut Reader::new(&out)), Ok(err));
    }
    assert_eq!(ERROR_OUT_OF_RANGE, 2);
    assert_eq!(
        Field::ALL,
        [
            Field::Coord,
            Field::Radius,
            Field::CapsuleLength,
            Field::EditExtent,
            Field::RayLength,
            Field::QueryAxis,
            Field::QueryVolume,
            Field::RegionCells,
        ]
    );
    assert_eq!(
        decode_error(&mut Reader::new(&[2, 0])),
        Err(WireError::UnknownKind(0))
    );
    assert_eq!(
        decode_error(&mut Reader::new(&[2, 9])),
        Err(WireError::UnknownKind(9))
    );
    assert_eq!(
        decode_error(&mut Reader::new(&[4, 0, 0, 0, 0])),
        Err(WireError::UnknownKind(4))
    );
}

/// A kind-3 entry in the layout logs held before kind 5 existed, as literal bytes (tick 3,
/// seq 1, max drop 8, one cell `[2, -1, 5]`): it decodes to a drop with no yield threshold
/// and encodes back to the same bytes. The same drop as kind 5 with a threshold of 0 is
/// refused: only kind 3 encodes it.
#[test]
fn a_kind_3_drop_keeps_its_bytes_and_a_kind_5_drop_of_threshold_0_is_refused() {
    let bytes: Vec<u8> = [
        &3u64.to_le_bytes()[..],
        &1u32.to_le_bytes(),
        &[3],
        &8i32.to_le_bytes(),
        &1u32.to_le_bytes(),
        &2i32.to_le_bytes(),
        &(-1i32).to_le_bytes(),
        &5i32.to_le_bytes(),
    ]
    .concat();
    assert_eq!(bytes.len(), 21 + 12);
    let log = OpLog::decode(&bytes).unwrap();
    let drop = Op::Drop {
        cells: vec![CellCoord([2, -1, 5])],
        max_drop: 8,
        yield_below: 0,
    };
    assert_eq!(
        log.entries(),
        [LogEntry::Op {
            tick: 3,
            seq: 1,
            op: drop
        }]
    );
    assert_eq!(log.encode(), bytes);

    let mut kind5: Vec<u8> = bytes[..12].to_vec();
    kind5.push(OP_DROP_THROUGH);
    kind5.extend_from_slice(&8i32.to_le_bytes());
    kind5.extend_from_slice(&0u16.to_le_bytes());
    kind5.extend_from_slice(&bytes[17..]);
    assert_eq!(
        OpLog::decode(&kind5),
        Err(WireError::NonCanonical { at: 0 })
    );
    assert_eq!(
        decode_op(&mut Reader::new(&kind5[12..])),
        Err(WireError::NonCanonical { at: 0 })
    );
}

fn three_entry_log() -> OpLog {
    let mut log = OpLog::new();
    assert_eq!(log.push(0, sphere_carve()), 0);
    assert_eq!(log.push(0, box_fill()), 1);
    assert_eq!(log.push(5, capsule_replace()), 2);
    log
}

#[test]
fn logs_round_trip_through_bytes() {
    let log = three_entry_log();
    let seqs: Vec<u32> = log.entries().iter().map(|e| e.seq()).collect();
    assert_eq!(seqs, [0, 1, 2]);
    let bytes = log.encode();
    assert_eq!(bytes.len(), 47 + 65 + 75);
    assert_eq!(OpLog::decode(&bytes), Ok(log));
    assert_eq!(OpLog::decode(&[]), Ok(OpLog::new()));

    let mut inputs = InputLog::new();
    inputs.push(0, 0, vec![]);
    inputs.push(0, 1, vec![1, 2, 3]);
    inputs.push(4, 0, vec![0xFF; 1000]);
    let bytes = inputs.encode();
    assert_eq!(bytes.len(), 14 + 17 + 1014);
    assert_eq!(InputLog::decode(&bytes), Ok(inputs));
}

#[test]
fn record_keeps_caller_seqs_with_gaps() {
    let mut log = OpLog::new();
    log.record(0, 3, sphere_carve());
    log.record(1, 7, box_fill());
    assert_eq!(log.push(1, capsule_replace()), 8);
    assert_eq!(OpLog::decode(&log.encode()), Ok(log));
}

/// A log as a server writes it after a tick with submitted ops: the ops that applied, a
/// boundary, then the logic's ops.
fn log_with_boundary() -> OpLog {
    let mut log = OpLog::new();
    log.record(3, 0, sphere_carve());
    log.record(3, 2, box_fill());
    log.record_boundary(3, 3);
    log.record(3, 4, capsule_replace());
    log
}

#[test]
fn a_boundary_entry_is_13_bytes_and_round_trips() {
    let mut b = Vec::new();
    LogEntry::Boundary {
        tick: 0x0102_0304_0506_0708,
        seq: 0x0A0B_0C0D,
    }
    .encode(&mut b);
    assert_eq!(b.len(), 13);
    assert_eq!(&b[0..8], &0x0102_0304_0506_0708u64.to_le_bytes());
    assert_eq!(&b[8..12], &0x0A0B_0C0Du32.to_le_bytes());
    assert_eq!(b[12], KIND_BOUNDARY);
    assert_eq!(KIND_BOUNDARY, 0x80);

    let log = log_with_boundary();
    assert_eq!(log.len(), 3);
    assert_eq!(log.entries().len(), 4);
    assert_eq!(log.entries()[2], LogEntry::Boundary { tick: 3, seq: 3 });
    assert_eq!(log.entries()[2].op(), None);
    // The next pushed seq follows the boundary's like any entry's.
    let mut pushed = log.clone();
    assert_eq!(pushed.push(4, sphere_carve()), 5);

    let bytes = log.encode();
    assert_eq!(bytes.len(), 47 + 65 + 13 + 75);
    assert_eq!(bytes[47 + 65 + 12], KIND_BOUNDARY);
    assert_eq!(OpLog::decode(&bytes), Ok(log.clone()));
    // A cut after the boundary's seq is truncated at the boundary's start.
    assert_eq!(
        OpLog::decode(&bytes[..47 + 65 + 12]),
        Err(WireError::Truncated { at: 47 + 65 })
    );

    // The file header counts every entry, the boundary included.
    let f = LogFile {
        seed: 1,
        source_version: 1,
        extent: ChunkExtent::new([0, 0, 0], [1, 1, 1]),
        ops: log,
        inputs: InputLog::new(),
        prefabs: PrefabRegistry::new(),
    };
    let mut bytes = Vec::new();
    f.write_to(&mut bytes).unwrap();
    assert_eq!(bytes.len(), 64 + 47 + 65 + 13 + 75);
    assert_eq!(&bytes[44..52], &4u64.to_le_bytes());
    assert_eq!(LogFile::read_from(&mut bytes.as_slice()), Ok(f));
}

#[test]
fn replay_skips_boundaries() {
    let direct = solid3_world();
    let expected: Vec<_> = [sphere_carve(), box_fill(), capsule_replace()]
        .iter()
        .map(|op| direct.apply(op))
        .collect();
    let world = solid3_world();
    let results = world.replay(&log_with_boundary(), &PrefabRegistry::new());
    let results: Vec<_> = results
        .into_iter()
        .map(|r| r.map(|r| r.edit_result()))
        .collect();
    assert_eq!(results, expected);
    assert_eq!(world_hash(&world), world_hash(&direct));
}

#[test]
#[should_panic(expected = "op log seqs ascend")]
fn a_boundary_out_of_order_is_refused() {
    let mut log = OpLog::new();
    log.record(0, 3, sphere_carve());
    log.record_boundary(0, 3);
}

#[test]
#[should_panic(expected = "op log seqs ascend")]
fn record_refuses_a_seq_out_of_order() {
    let mut log = OpLog::new();
    log.record(0, 3, sphere_carve());
    log.record(0, 3, box_fill());
}

/// Bytes whose seqs do not strictly ascend decode to `OutOfOrder` at the offending entry's
/// offset, through an op log and through a log file (offsets there count the 64-byte header
/// with its empty prefab table),
/// instead of reaching `record`'s panic.
#[test]
fn seqs_that_do_not_ascend_are_out_of_order() {
    for second in [3, 5] {
        let mut ops = entry_bytes(0, 5, sphere_carve());
        assert_eq!(ops.len(), 47);
        ops.extend(entry_bytes(0, second, sphere_carve()));
        assert_eq!(
            OpLog::decode(&ops),
            Err(WireError::OutOfOrder { at: 47 }),
            "seqs 5, {second}"
        );

        let mut bytes = LogFile {
            seed: 1,
            source_version: 1,
            extent: ChunkExtent::new([0, 0, 0], [1, 1, 1]),
            ops: OpLog::new(),
            inputs: InputLog::new(),
            prefabs: PrefabRegistry::new(),
        }
        .encode();
        assert_eq!(bytes.len(), 64);
        bytes[44..52].copy_from_slice(&2u64.to_le_bytes());
        bytes.extend(&ops);
        assert_eq!(
            LogFile::decode(&bytes),
            Err(WireError::OutOfOrder { at: 64 + 47 }),
            "file, seqs 5, {second}"
        );
    }
    assert_eq!(
        WireError::OutOfOrder { at: 47 }.to_string(),
        "op log seq out of order at offset 47"
    );
}

#[test]
fn a_cut_inside_an_entry_is_truncated_at_that_entry() {
    let bytes = three_entry_log().encode();
    let starts = [0usize, 47, 112];
    for cut in 1..bytes.len() {
        if starts.contains(&cut) {
            assert!(OpLog::decode(&bytes[..cut]).is_ok(), "cut {cut}");
            continue;
        }
        let at = *starts.iter().rev().find(|&&s| s < cut).unwrap();
        assert_eq!(
            OpLog::decode(&bytes[..cut]),
            Err(WireError::Truncated { at }),
            "cut {cut}"
        );
    }
}

#[test]
fn an_unknown_op_kind_is_reported() {
    let mut bytes = three_entry_log().encode();
    for k in [0u8, 7, KIND_BOUNDARY - 1, 200] {
        bytes[47 + 12] = k;
        assert_eq!(OpLog::decode(&bytes), Err(WireError::UnknownKind(k)));
    }
    let mut bytes = three_entry_log().encode();
    bytes[13] = 9;
    assert_eq!(OpLog::decode(&bytes), Err(WireError::UnknownKind(9)));
    let mut bytes = three_entry_log().encode();
    bytes[14] = 0;
    assert_eq!(OpLog::decode(&bytes), Err(WireError::UnknownKind(0)));
}

#[test]
fn an_input_len_past_the_end_is_truncated() {
    let mut inputs = InputLog::new();
    inputs.push(0, 0, vec![1, 2]);
    inputs.push(1, 0, vec![3, 4, 5]);
    let mut bytes = inputs.encode();
    // The second entry starts at 16; claim 4 bytes where 3 remain.
    bytes[16 + 12] = 4;
    assert_eq!(
        InputLog::decode(&bytes),
        Err(WireError::Truncated { at: 16 })
    );
    // A len the bytes cannot hold at all.
    bytes[16 + 12..16 + 14].copy_from_slice(&u16::MAX.to_le_bytes());
    assert_eq!(
        InputLog::decode(&bytes),
        Err(WireError::Truncated { at: 16 })
    );
}

fn file() -> LogFile {
    let mut inputs = InputLog::new();
    inputs.push(0, 0, vec![1, 2, 3]);
    LogFile {
        seed: 0x5EED_0001,
        source_version: 1,
        extent: ChunkExtent::new([-2, -1, 0], [3, 4, 5]),
        ops: three_entry_log(),
        inputs,
        prefabs: PrefabRegistry::new(),
    }
}

#[test]
fn a_log_file_round_trips_with_a_64_byte_header() {
    let f = file();
    let mut bytes = Vec::new();
    f.write_to(&mut bytes).unwrap();
    assert_eq!(f.header_len(), 64);
    assert_eq!(bytes.len(), 64 + 47 + 65 + 75 + 17);
    assert_eq!(&bytes[0..4], b"VXEL");
    assert_eq!(&bytes[4..8], &2u32.to_le_bytes());
    assert_eq!(&bytes[8..16], &0x5EED_0001u64.to_le_bytes());
    assert_eq!(&bytes[16..20], &1u32.to_le_bytes());
    assert_eq!(&bytes[20..24], &(-2i32).to_le_bytes());
    assert_eq!(&bytes[40..44], &5i32.to_le_bytes());
    assert_eq!(&bytes[44..52], &3u64.to_le_bytes());
    assert_eq!(&bytes[52..60], &1u64.to_le_bytes());
    assert_eq!(&bytes[60..64], &0u32.to_le_bytes());
    assert_eq!(LogFile::read_from(&mut bytes.as_slice()), Ok(f));
}

/// A save whose log stamps the 16³ vault: a 34-byte stamp entry, and a header of 60 fixed
/// bytes, the table's count, the vault's id and its 8 716 bytes of content.
fn vault_file() -> (LogFile, u64) {
    let vault = std::sync::Arc::new(Prefab::vault(16, 4));
    let mut all = PrefabRegistry::new();
    let id = all.register(vault.clone()).unwrap();
    // A second registered prefab the log does not name stays out of the table.
    all.register(std::sync::Arc::new(Prefab::vault(4, 2)))
        .unwrap();
    let mut ops = OpLog::new();
    ops.push(0, sphere_carve());
    ops.push(1, stamp_op(id));
    let prefabs = all.subset(ops.prefab_ids());
    assert_eq!(prefabs.len(), 1);
    let f = LogFile {
        seed: 9,
        source_version: 1,
        extent: ChunkExtent::new([0, 0, 0], [1, 1, 1]),
        ops,
        inputs: InputLog::new(),
        prefabs,
    };
    (f, id)
}

#[test]
fn a_save_with_a_stamp_carries_its_prefab_in_the_header() {
    let (f, id) = vault_file();
    assert_eq!(Prefab::vault(16, 4).encoded_len(), 8_716);
    let bytes = f.encode();
    assert_eq!(f.header_len(), 60 + 4 + 8 + 8_716);
    assert_eq!(bytes.len(), 60 + 4 + 8 + 8_716 + 47 + 34);
    assert_eq!(&bytes[60..64], &1u32.to_le_bytes());
    assert_eq!(&bytes[64..72], &id.to_le_bytes());
    assert_eq!(&bytes[72..76], &16u32.to_le_bytes());
    let back = LogFile::decode(&bytes).unwrap();
    assert_eq!(back, f);
    assert_eq!(back.prefabs.get(id).map(|p| p.id()), Some(id));

    // One byte of the vault's content flipped (a voxel of its surface): refused, naming the
    // id the table gave it.
    let mut bad = bytes.clone();
    bad[72 + 12 + 2 * 100] ^= 1;
    assert_eq!(LogFile::decode(&bad), Err(WireError::PrefabMismatch { id }));
    // A mask bit flipped: the voxel it clears changes the content too.
    let mut bad = bytes.clone();
    bad[72 + 12 + 2 * 4096] ^= 1;
    assert_eq!(LogFile::decode(&bad), Err(WireError::PrefabMismatch { id }));
}

/// A format-1 file (the fixed header, no table) still decodes, with an empty table.
#[test]
fn a_format_1_file_still_decodes() {
    let f = file();
    let mut bytes = f.encode();
    bytes[4..8].copy_from_slice(&1u32.to_le_bytes());
    bytes.drain(60..64);
    assert_eq!(bytes.len(), 60 + 47 + 65 + 75 + 17);
    assert_eq!(LogFile::decode(&bytes), Ok(f));
}

/// Replay resolves a stamp in the table it is given; without the id it is refused, naming the
/// id, and changes nothing.
#[test]
fn replay_resolves_stamps_in_the_table() {
    let (f, id) = vault_file();
    let direct = solid3_world();
    let vault = Prefab::vault(16, 4);
    let expected = [
        direct.apply(&sphere_carve()),
        direct.stamp(
            &vault,
            VoxelCoord([32, -32, 7]),
            Rot::from_index(5).unwrap(),
        ),
    ];
    let world = solid3_world();
    let results = world.replay(&f.ops, &f.prefabs);
    assert_eq!(results.len(), 2);
    for (r, e) in results.iter().zip(&expected) {
        assert_eq!(&r.clone().map(|r| r.edit_result()), e);
    }
    assert_eq!(world_hash(&world), world_hash(&direct));

    let bare = solid3_world();
    let results = bare.replay(&f.ops, &PrefabRegistry::new());
    assert_eq!(results[1], Err(EditError::UnknownPrefab(id)));
    let only_carve = solid3_world();
    only_carve.apply(&sphere_carve()).unwrap();
    assert_eq!(world_hash(&bare), world_hash(&only_carve));
}

/// The registry keeps the first prefab under an id; the same content again is a no-op.
#[test]
fn a_registry_refuses_a_second_content_under_one_id_only() {
    let mut reg = PrefabRegistry::new();
    let a = std::sync::Arc::new(Prefab::vault(16, 4));
    let id = reg.register(a.clone()).unwrap();
    assert_eq!(id, a.id());
    assert_eq!(
        reg.register(std::sync::Arc::new(Prefab::vault(16, 4))),
        Ok(id)
    );
    assert_eq!(reg.len(), 1);
    assert!(reg.contains(id));
    assert_eq!(
        PrefabCollision { id }.to_string(),
        format!("another prefab is registered under id {id:#018x}")
    );
}

#[test]
fn a_log_file_with_a_wrong_header_is_refused() {
    let mut bytes = file().encode();
    bytes[0] = b'X';
    assert_eq!(LogFile::decode(&bytes), Err(WireError::BadMagic));

    for format in [0u32, 3] {
        let mut bytes = file().encode();
        bytes[4..8].copy_from_slice(&format.to_le_bytes());
        assert_eq!(
            LogFile::decode(&bytes),
            Err(WireError::UnsupportedFormat(format))
        );
    }

    let bytes = file().encode();
    assert_eq!(
        LogFile::decode(&bytes[..59]),
        Err(WireError::Truncated { at: 0 })
    );
    // The input entry starts after the header and three op entries.
    assert_eq!(
        LogFile::decode(&bytes[..bytes.len() - 1]),
        Err(WireError::Truncated { at: 64 + 187 })
    );
    let mut long = bytes.clone();
    long.push(0);
    assert_eq!(
        LogFile::decode(&long),
        Err(WireError::Trailing { at: bytes.len() })
    );
}

#[test]
fn replay_applies_in_seq_order_and_matches_direct_apply() {
    let ops = [
        EditOp::sphere(centre_of_voxel([0, 0, 0]), 10 * FIXED_ONE, Mode::Carve),
        EditOp::sphere(centre_of_voxel([0, 0, 0]), 10 * FIXED_ONE, Mode::Fill(2)),
        EditOp::sphere(
            centre_of_voxel([0, 0, 0]),
            10 * FIXED_ONE,
            Mode::Replace { from: 2, to: 5 },
        ),
        EditOp::sphere(centre_of_voxel([10000, 0, 0]), FIXED_ONE, Mode::Carve),
    ];
    let direct = solid3_world();
    let expected: Vec<_> = ops.iter().map(|op| direct.apply(op)).collect();

    let mut log = OpLog::new();
    for op in ops {
        log.push(0, op);
    }
    let decoded = OpLog::decode(&log.encode()).unwrap();
    let world = solid3_world();
    let results: Vec<_> = world
        .replay(&decoded, &PrefabRegistry::new())
        .into_iter()
        .map(|r| r.map(|r| r.edit_result()))
        .collect();
    assert_eq!(results, expected);
    assert_eq!(results[0].as_ref().unwrap().changed_from, [(3, 4169)]);
    assert_eq!(
        results[3],
        Err(EditError::NotResident(ChunkCoord([156, -1, 0])))
    );
    assert_eq!(world_hash(&world), world_hash(&direct));
}
