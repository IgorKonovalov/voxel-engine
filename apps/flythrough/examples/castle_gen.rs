//! Times chunk generation over the 990 chunks the castle's box meets (x 2..=11, y -5..=5,
//! z -2..=6): `StampedSource::chunk` with the castle's placements, and the hall alone over the
//! same chunks. Prints the mean and p99 per chunk of each, single-threaded, over `--runs N`
//! passes (default 3), each over the same chunks in the same order.
//!
//! ```text
//! cargo run --release -p flythrough --example castle_gen -- --runs 3
//! ```

use flythrough::castle;
use vxe_world::{Chunk, ChunkCoord, WorldSource};

#[allow(
    clippy::disallowed_methods,
    reason = "the example's purpose is a wall-clock reading of chunk generation"
)]
fn now() -> std::time::Instant {
    std::time::Instant::now()
}

/// Per-chunk generation times of `src` over `chunks`, in microseconds.
fn time(src: &impl WorldSource, chunks: &[ChunkCoord]) -> Vec<f64> {
    chunks
        .iter()
        .map(|&c| {
            let mut out = Chunk::default();
            let t = now();
            src.chunk(c, &mut out);
            let us = (now() - t).as_secs_f64() * 1e6;
            std::hint::black_box(&out);
            us
        })
        .collect()
}

fn line(name: &str, mut us: Vec<f64>) {
    us.sort_by(f64::total_cmp);
    let mean = us.iter().sum::<f64>() / us.len() as f64;
    let p99 = us[(us.len() * 99).div_ceil(100) - 1];
    let total: f64 = us.iter().sum();
    println!(
        "{name}: {} chunks, mean {mean:.1} us, p99 {p99:.1} us, max {:.1} us, total {:.1} ms",
        us.len(),
        us[us.len() - 1],
        total / 1e3
    );
}

fn main() {
    let mut runs = 3;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--runs" => runs = args.next().and_then(|v| v.parse().ok()).unwrap_or(3),
            other => {
                eprintln!("castle_gen: unknown argument {other}");
                std::process::exit(2);
            }
        }
    }
    let mut chunks = Vec::new();
    for z in -2..=6 {
        for y in -5..=5 {
            for x in 2..=11 {
                chunks.push(ChunkCoord([x, y, z]));
            }
        }
    }
    let file = castle::castle_vox();
    let placed = castle::castle_source(1, castle::castle_prefabs(&file).expect("mapped"));
    drop(file);
    let base = *placed.base();
    for run in 1..=runs {
        line(&format!("run {run} castle"), time(&placed, &chunks));
        line(&format!("run {run} hall alone"), time(&base, &chunks));
    }
}
