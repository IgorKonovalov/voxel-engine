//! The planar edit kernel of `packed.rs` against today's row write, on the same brick-edits
//! (ADR-0012's spike).
//!
//! The brick-edits are those of the edit bench's `worst_warm` case: 100 carves of radius 10
//! voxels at seeded positions on the solid world of chunks `[-2, 2)^3`, its brick-array pool
//! reserved, captured once by `World::apply_capturing` (each brick reaching the row write: its
//! state before the op and its 64 row spans, in op order). Before any timing, every captured
//! brick-edit is replayed through both writers and must give equal per-material tallies,
//! equal changed bounds and equal voxels, and the planar result must equal the canonical
//! encoding of today's voxels.
//!
//! Timed, each as the median of `REPS` runs with the bricks and pool built outside the timed
//! region:
//!
//! - the 100 carves' brick-edits through each writer: the summed write time and ns per
//!   brick-edit. Today's writer expands a `Uniform` brick into a pooled, touched array; the
//!   planar writer allocates its 72-byte mask-only box;
//! - the same spans as `Fill(rock)` on the inverse bricks (air and rock swapped), and as
//!   `Replace { from: rock, to: ore }` on the original bricks;
//! - `decode` of a one-, two- and four-plane brick into a scratch array, and 1000 seeded
//!   `get` calls on each, beside a copy and 1000 reads of a raw 512-id array;
//! - the expansion of a `Uniform` brick into the mask-only class, allocation included, and a
//!   class change from one plane to two, beside a raw array's allocation.
//!
//! Every figure prints with the classes it ran on. `cargo bench -p vxe-world --bench
//! brick_kernel`.

use std::collections::BTreeMap;
use std::hint::black_box;

use vxe_world::hash::mix64;
use vxe_world::packed::{Packed, PackedClass, RowSpans};
use vxe_world::*;

/// Timed runs per figure, after `WARMUP` untimed ones.
const REPS: usize = 60;
const WARMUP: usize = 5;
/// Calls per run of the per-brick figures.
const CALLS: usize = 10_000;

const ROCK: MaterialId = 1;
const ORE: MaterialId = 2;

#[allow(
    clippy::disallowed_methods,
    reason = "a bench timing the kernels; the kernels read no clock"
)]
fn now() -> std::time::Instant {
    std::time::Instant::now()
}

fn table() -> MaterialTable {
    let mut t = MaterialTable::new();
    assert_eq!(t.push(Material::named("rock")), ROCK);
    assert_eq!(t.push(Material::named("ore")), ORE);
    t
}

/// The edit bench's 100 distinct voxels within 100 voxels of the origin.
fn seeded_voxels(seed: u64) -> Vec<[i32; 3]> {
    let mut out: Vec<[i32; 3]> = Vec::new();
    let mut state = seed;
    while out.len() < 100 {
        let v = [0; 3].map(|_| {
            state = mix64(state);
            (state % 201) as i32 - 100
        });
        if !out.contains(&v) {
            out.push(v);
        }
    }
    out
}

/// `worst_warm`'s brick-edits, in op order.
fn capture(table: &MaterialTable) -> Vec<CapturedBrickEdit> {
    let world = World::generate(
        ChunkExtent::new([-2, -2, -2], [2, 2, 2]),
        &UniformSource(ROCK),
        table,
    );
    let mut out = Vec::new();
    for v in seeded_voxels(0x5EED_0004) {
        let op = EditOp::sphere(centre_of_voxel(v), 10 * FIXED_ONE, Mode::Carve);
        world.apply_capturing(&op, &mut out).expect("resident");
    }
    out
}

fn voxels(b: &Brick) -> [MaterialId; BRICK_VOXELS] {
    std::array::from_fn(|i| b.get(i))
}

fn raw_voxels(b: &RawBrick) -> [MaterialId; BRICK_VOXELS] {
    std::array::from_fn(|i| b.get(i))
}

fn class_name(b: &Brick) -> &'static str {
    match b.class() {
        None => "Uniform",
        Some(PackedClass::Mask) => "Mask",
        Some(PackedClass::Plane1) => "Plane1",
        Some(PackedClass::Plane2) => "Plane2",
        Some(PackedClass::Plane4) => "Plane4",
        Some(PackedClass::Wide) => "Wide",
    }
}

fn mix(bricks: impl Iterator<Item = Brick>) -> String {
    let mut m: BTreeMap<&str, usize> = BTreeMap::new();
    for b in bricks {
        *m.entry(class_name(&b)).or_default() += 1;
    }
    m.iter()
        .map(|(k, v)| format!("{k} {v}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Every brick-edit through both writers alone: equal tallies, bounds and voxels, and the
/// planar brick canonical. Returns the planar bricks after the edits.
fn check(name: &str, mode: Mode, table: &MaterialTable, edits: &[(Brick, RowSpans)]) -> Vec<Brick> {
    let mut after = Vec::with_capacity(edits.len());
    for (i, (before, spans)) in edits.iter().enumerate() {
        let mut today = RawBrick::of(before);
        let mut w = RowWriter::new(mode, table);
        w.write(&mut today, spans);
        let (t_counts, t_bounds) = w.finish();

        let mut planar = before.clone();
        let mut w = PlanarWriter::new(mode, table);
        w.write(&mut planar, spans);
        let (p_counts, p_bounds) = w.finish();

        let mut p_voxels = [AIR; BRICK_VOXELS];
        planar.decode(&mut p_voxels);
        let t_voxels = raw_voxels(&today);
        assert_eq!(p_counts, t_counts, "{name} brick-edit {i}: tallies");
        assert_eq!(p_bounds, t_bounds, "{name} brick-edit {i}: bounds");
        assert_eq!(p_voxels, t_voxels, "{name} brick-edit {i}: voxels");
        assert_eq!(
            planar,
            Brick::from_voxels(Box::new(t_voxels)),
            "{name} brick-edit {i}: canonical"
        );
        after.push(planar);
    }
    after
}

/// Median and minimum of `REPS` timed runs of `run` over a fresh `setup()` each, in ns.
fn time<S, R>(mut setup: impl FnMut() -> S, mut run: impl FnMut(&mut S) -> R) -> (f64, f64) {
    let mut ns = Vec::with_capacity(REPS);
    for rep in 0..WARMUP + REPS {
        let mut s = setup();
        let t = now();
        black_box(run(&mut s));
        let dt = (now() - t).as_nanos() as f64;
        drop(s);
        if rep >= WARMUP {
            ns.push(dt);
        }
    }
    ns.sort_by(f64::total_cmp);
    (ns[ns.len() / 2], ns[0])
}

/// Times both writers over `edits` and prints the sums and ns per brick-edit; returns the
/// planar sum in ns and the count of `Uniform` bricks the edits expand.
fn writers(
    name: &str,
    mode: Mode,
    table: &MaterialTable,
    edits: &[(Brick, RowSpans)],
) -> (f64, usize) {
    let after = check(name, mode, table, edits);
    let n = edits.len();
    let uniform_before = edits.iter().filter(|(b, _)| !b.is_full()).count();
    println!(
        "{name}: {n} brick-edits; before {}; planar after {}",
        mix(edits.iter().map(|(b, _)| b.clone())),
        mix(after.into_iter())
    );
    let (today, today_min) = time(
        || {
            let mut w = RowWriter::new(mode, table);
            w.reserve(uniform_before);
            let bricks: Vec<RawBrick> = edits.iter().map(|(b, _)| RawBrick::of(b)).collect();
            (w, bricks)
        },
        |(w, bricks)| {
            for (b, (_, spans)) in bricks.iter_mut().zip(edits) {
                w.write(b, spans);
            }
        },
    );
    let (planar, planar_min) = time(
        || {
            let w = PlanarWriter::new(mode, table);
            let bricks: Vec<Brick> = edits.iter().map(|(b, _)| b.clone()).collect();
            (w, bricks)
        },
        |(w, bricks)| {
            for (b, (_, spans)) in bricks.iter_mut().zip(edits) {
                w.write(b, spans);
            }
        },
    );
    for (who, med, min) in [("today ", today, today_min), ("planar", planar, planar_min)] {
        println!(
            "  {name} {who}: sum {:.3} ms (min {:.3}), {:.1} ns per brick-edit",
            med / 1e6,
            min / 1e6,
            med / n as f64
        );
    }
    println!("  {name} planar/today: {:.3}", planar / today);
    (planar, uniform_before)
}

/// A seeded brick of `n` materials (1 to `n`) with about two thirds of its voxels solid.
fn brick_of(n: u64, seed: u64) -> [MaterialId; BRICK_VOXELS] {
    let mut s = seed;
    let mut v: [MaterialId; BRICK_VOXELS] = std::array::from_fn(|_| {
        s = mix64(s);
        if s.is_multiple_of(3) {
            AIR
        } else {
            (1 + (s >> 8) % n) as MaterialId
        }
    });
    // Every material present, so the class is the one asked for.
    for m in 1..=n as usize {
        v[m * 7] = m as MaterialId;
    }
    v
}

/// The per-brick figures; returns the ns of one `Uniform` expansion into the mask-only class.
fn per_brick() -> f64 {
    let mut idx = [0usize; 1000];
    let mut s = 0x6E7u64;
    for i in &mut idx {
        s = mix64(s);
        *i = (s % BRICK_VOXELS as u64) as usize;
    }
    let raw = Box::new(brick_of(16, 9));
    let (copy, _) = time(
        || Box::new([AIR; BRICK_VOXELS]),
        |out| {
            for _ in 0..CALLS {
                out.copy_from_slice(black_box(&raw[..]));
                black_box(&mut **out);
            }
        },
    );
    let (gets, _) = time(
        || (),
        |_| {
            let mut sum = 0u32;
            for &i in &idx {
                sum += u32::from(black_box(&raw)[i]);
            }
            sum
        },
    );
    println!(
        "raw [u16; 512] (today's Full): copy {:.3} us per brick; 1000 reads {:.2} us",
        copy / CALLS as f64 / 1e3,
        gets / 1e3
    );
    for n in [2, 4, 16] {
        let b = Brick::from_voxels(Box::new(brick_of(n, n)));
        let class = class_name(&b);
        let (dec, _) = time(
            || Box::new([AIR; BRICK_VOXELS]),
            |out| {
                for _ in 0..CALLS {
                    black_box(&b).decode(out);
                    black_box(&mut **out);
                }
            },
        );
        let (gets, _) = time(
            || (),
            |_| {
                let mut sum = 0u32;
                for &i in &idx {
                    sum += u32::from(black_box(&b).get(i));
                }
                sum
            },
        );
        println!(
            "{class} ({n} materials): decode {:.3} us per brick; 1000 get {:.2} us",
            dec / CALLS as f64 / 1e3,
            gets / 1e3
        );
    }

    let (expand, _) = time(
        || Vec::with_capacity(CALLS),
        |v| {
            for _ in 0..CALLS {
                v.push(Brick::Full(Packed::expand_uniform(black_box(ROCK))));
            }
        },
    );
    let (raw_alloc, _) = time(
        || Vec::with_capacity(CALLS),
        |v| {
            for _ in 0..CALLS {
                v.push(Box::new([black_box(ROCK); BRICK_VOXELS]));
            }
        },
    );
    println!(
        "expand Uniform -> Mask (72 B, allocation included): {:.1} ns; raw 1 KiB array \
         allocated and filled: {:.1} ns",
        expand / CALLS as f64,
        raw_alloc / CALLS as f64
    );
    let Brick::Full(one) = Brick::from_voxels(Box::new(brick_of(2, 5))) else {
        unreachable!("two materials")
    };
    assert_eq!(one.class(), PackedClass::Plane1);
    let (grow, _) = time(
        || Vec::with_capacity(CALLS),
        |v| {
            for _ in 0..CALLS {
                v.push(black_box(&one).reclass(PackedClass::Plane2));
            }
        },
    );
    println!(
        "class change Plane1 -> Plane2 (136 B to 200 B, allocation included): {:.1} ns",
        grow / CALLS as f64
    );
    expand / CALLS as f64
}

fn main() {
    // `cargo bench` passes `--bench`; a test run of the bench target does not, and skips.
    if !std::env::args().any(|a| a == "--bench") {
        return;
    }
    let table = table();
    let captured = capture(&table);
    let carve: Vec<(Brick, RowSpans)> = captured
        .iter()
        .map(|e| (e.before.clone(), e.spans))
        .collect();
    let swap = |m: MaterialId| match m {
        AIR => ROCK,
        ROCK => AIR,
        m => m,
    };
    let inverse: Vec<(Brick, RowSpans)> = carve
        .iter()
        .map(|(b, s)| {
            let v = voxels(b).map(swap);
            (Brick::from_voxels(Box::new(v)), *s)
        })
        .collect();
    println!(
        "brick_kernel: worst_warm's 100 carves of radius 10 on the solid world, {} brick-edits \
         captured; {REPS} runs per figure, median (min)",
        carve.len()
    );
    let (carve_ns, expansions) = writers("carve", Mode::Carve, &table, &carve);
    writers(
        "fill(rock) on the inverse",
        Mode::Fill(ROCK),
        &table,
        &inverse,
    );
    writers(
        "replace(rock->ore)",
        Mode::Replace {
            from: ROCK,
            to: ORE,
        },
        &table,
        &carve,
    );
    let expand_ns = per_brick();
    let alloc = expansions as f64 * expand_ns;
    println!(
        "allocation share of the carves: {expansions} expansions x {expand_ns:.1} ns = {:.1} us, \
         {:.1} % of the planar carve sum, {:.1} % of worst_warm's recorded 3.03 ms",
        alloc / 1e3,
        100.0 * alloc / carve_ns,
        100.0 * alloc / 3.03e6
    );
}
