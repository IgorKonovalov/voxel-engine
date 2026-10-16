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
    let mut out = Vec::new();
    LogEntry {
        tick,
        seq,
        op: Op::Edit(op),
    }
    .encode(&mut out);
    out
}

#[test]
fn entry_sizes_are_the_documented_ones() {
    // tick 8 + seq 4 + kind 1 + mode 1 (+ payload) + shape 1 + payload.
    assert_eq!(entry_bytes(0, 0, sphere_carve()).len(), 47);
    assert_eq!(entry_bytes(0, 0, box_fill()).len(), 65);
    assert_eq!(entry_bytes(0, 0, capsule_replace()).len(), 75);
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

/// One value of every tag: each mode on each shape, a result with and without an aabb, and
/// the error.
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

    let err = EditError::NotResident(ChunkCoord([4, -1, 1]));
    let mut out = Vec::new();
    encode_error(&mut out, &err);
    assert_eq!(out[0], ERROR_NOT_RESIDENT);
    assert_eq!(out.len(), 13);
    assert_eq!(decode_error(&mut Reader::new(&out)), Ok(err));
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
    let seqs: Vec<u32> = log.entries().iter().map(|e| e.seq).collect();
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

#[test]
#[should_panic(expected = "op log seqs ascend")]
fn record_refuses_a_seq_out_of_order() {
    let mut log = OpLog::new();
    log.record(0, 3, sphere_carve());
    log.record(0, 3, box_fill());
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
    for k in [
        0u8,
        OP_RESERVED_STAMP,
        OP_RESERVED_DROP,
        OP_RESERVED_REMOVE,
        200,
    ] {
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
    }
}

#[test]
fn a_log_file_round_trips_with_a_60_byte_header() {
    let f = file();
    let mut bytes = Vec::new();
    f.write_to(&mut bytes).unwrap();
    assert_eq!(bytes.len(), 60 + 47 + 65 + 75 + 17);
    assert_eq!(&bytes[0..4], b"VXEL");
    assert_eq!(&bytes[4..8], &1u32.to_le_bytes());
    assert_eq!(&bytes[8..16], &0x5EED_0001u64.to_le_bytes());
    assert_eq!(&bytes[16..20], &1u32.to_le_bytes());
    assert_eq!(&bytes[20..24], &(-2i32).to_le_bytes());
    assert_eq!(&bytes[40..44], &5i32.to_le_bytes());
    assert_eq!(&bytes[44..52], &3u64.to_le_bytes());
    assert_eq!(&bytes[52..60], &1u64.to_le_bytes());
    assert_eq!(LogFile::read_from(&mut bytes.as_slice()), Ok(f));
}

#[test]
fn a_log_file_with_a_wrong_header_is_refused() {
    let mut bytes = file().encode();
    bytes[0] = b'X';
    assert_eq!(LogFile::decode(&bytes), Err(WireError::BadMagic));

    let mut bytes = file().encode();
    bytes[4..8].copy_from_slice(&2u32.to_le_bytes());
    assert_eq!(
        LogFile::decode(&bytes),
        Err(WireError::UnsupportedFormat(2))
    );

    let bytes = file().encode();
    assert_eq!(
        LogFile::decode(&bytes[..59]),
        Err(WireError::Truncated { at: 0 })
    );
    // The input entry starts after the header and three op entries.
    assert_eq!(
        LogFile::decode(&bytes[..bytes.len() - 1]),
        Err(WireError::Truncated { at: 60 + 187 })
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
    let results = world.replay(&decoded);
    assert_eq!(results, expected);
    assert_eq!(results[0].as_ref().unwrap().changed_from, [(3, 4169)]);
    assert_eq!(
        results[3],
        Err(EditError::NotResident(ChunkCoord([156, -1, 0])))
    );
    assert_eq!(world_hash(&world), world_hash(&direct));
}
