//! Prints the cave's storage figures (`nfr.md` §3, §5), bounded or streamed.
//!
//! ```text
//! cargo run --release -p flythrough --example world_budget -- --chunks 20x20x8 --seed 1
//! cargo run --release -p flythrough --example world_budget -- --stream 16,24 --seed 1
//! ```
//!
//! `--chunks` generates the bounded cave over an extent. `--stream R_FULL,R_LOD` builds the
//! residency the fly-through streams around the unbounded cave's spawn (hysteresis 1), on one
//! worker per core, and waits until it is complete.
//!
//! Mirror bytes use the uploader's formula (ADR-0004): each `Full` brick or mip-brick takes a
//! 512-byte slot of palette indices plus an 8-byte occupancy mask, each mirrored chunk (full or
//! LOD-only) a 2560-byte record, each chunk-table entry 4 bytes (the window's `W^3` when
//! streaming), and each material row 8 `f32`s. CPU bytes count a full chunk's slot array and
//! its `Full` bricks' voxels, and a LOD-only chunk's mip-brick slots and `Full` mip-bricks.

use std::sync::Arc;

use flythrough::cave::CaveSource;
use flythrough::{cave_materials, chunk_ram, mips_ram, window_side};
use vxe_world::{
    BRICK_VOXELS, Brick, CHUNK_BRICKS, ChunkExtent, PersistConfig, Residency, StreamConfig,
    Streamer, World,
};

/// Mirror bytes per `Full` brick or mip-brick, and per mirrored chunk's record.
const SLOT_AND_MASK: usize = 512 + 8;
const RECORD: usize = 2560;

/// Width of the label column the figures are printed after.
const LABEL: usize = 19;

#[allow(
    clippy::disallowed_methods,
    reason = "a budget tool timing generation; the world itself reads no clock"
)]
fn now() -> std::time::Instant {
    std::time::Instant::now()
}

fn main() {
    let mut chunks = [20, 20, 8];
    let mut seed = 1u64;
    let mut stream = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let v = args.next().unwrap_or_else(|| usage(&a));
        match a.as_str() {
            "--chunks" => {
                let parts: Vec<i32> = v.split('x').filter_map(|p| p.parse().ok()).collect();
                chunks = parts.try_into().unwrap_or_else(|_| usage(&v));
            }
            "--seed" => seed = v.parse().unwrap_or_else(|_| usage(&v)),
            "--stream" => {
                let parts: Vec<i32> = v.split(',').filter_map(|p| p.parse().ok()).collect();
                let [r_full, r_lod]: [i32; 2] = parts.try_into().unwrap_or_else(|_| usage(&v));
                stream = Some(Residency {
                    r_full,
                    r_lod,
                    hysteresis: 1,
                });
            }
            _ => usage(&a),
        }
    }
    match stream {
        Some(r) => streamed(seed, r),
        None => bounded(seed, chunks),
    }
}

fn bounded(seed: u64, chunks: [i32; 3]) {
    let extent = ChunkExtent::new([0, 0, 0], chunks);
    let source = CaveSource::new(seed, extent);
    let table = cave_materials();
    let threads = std::thread::available_parallelism().map_or(1, usize::from);

    let start = now();
    let world = World::generate_with_threads(extent, &source, &table, threads);
    let secs = (now() - start).as_secs_f64();

    let s = world.stats();
    let bricks = s.full_bricks + s.uniform_bricks;
    let slot_array = CHUNK_BRICKS * size_of::<Brick>();
    let cpu = s.full_bricks * BRICK_VOXELS * 2 + s.chunks * slot_array;
    let mirror = s.full_bricks * SLOT_AND_MASK + s.chunks * (RECORD + 4) + table.len() * 8 * 4;
    let voxels = s.chunks as f64 * 64.0 * 64.0 * 64.0;
    let air = world.count_in_box(
        extent.min.min_voxel(),
        vxe_world::VoxelCoord(extent.max.min_voxel().0.map(|v| v - 1)),
    );
    let air = air.iter().find(|(m, _)| *m == 0).map_or(0, |&(_, n)| n);

    println!(
        "{:LABEL$}{}x{}x{} chunks, seed {seed}",
        "extent:", chunks[0], chunks[1], chunks[2]
    );
    println!("{:LABEL$}{}", "chunks:", s.chunks);
    println!(
        "{:LABEL$}{} Full, {} Uniform",
        "bricks:", s.full_bricks, s.uniform_bricks
    );
    println!(
        "full fraction:     {:.2} %",
        100.0 * s.full_bricks as f64 / bricks as f64
    );
    println!("air fraction:      {:.2} %", 100.0 * air as f64 / voxels);
    println!("cpu bytes:         {} ({:.1} MiB)", cpu, mib(cpu as u64));
    println!(
        "mirror bytes:      {} ({:.1} MiB)",
        mirror,
        mib(mirror as u64)
    );
    println!("generation:        {:.3} s on {threads} threads", secs);
    println!("voxels/s/thread:   {:.3e}", voxels / secs / threads as f64);
}

fn streamed(seed: u64, r: Residency) {
    let source = CaveSource::unbounded(seed);
    let table = cave_materials();
    let workers = std::thread::available_parallelism().map_or(1, usize::from);
    let world = World::new(&table);
    let mut streamer = Streamer::new(
        Arc::new(source),
        StreamConfig {
            residency: r,
            workers,
            persist: PersistConfig::default(),
        },
    );
    let start = now();
    streamer.update(&world, &[source.spawn()]);
    streamer.wait_idle(&world);
    let secs = (now() - start).as_secs_f64();

    let st = streamer.stats();
    let (mut full_bricks, mut uniform_bricks, mut cpu_full) = (0usize, 0usize, 0u64);
    for c in world.store().coords() {
        let chunk = world.store().get(c).expect("resident");
        let chunk = chunk.read().expect("chunk lock");
        let full = chunk.bricks().iter().filter(|b| b.is_full()).count();
        full_bricks += full;
        uniform_bricks += CHUNK_BRICKS - full;
        cpu_full += chunk_ram(&chunk);
    }
    let (mut mip_full, mut cpu_lod) = (0usize, 0u64);
    for c in world.lod_coords() {
        let mips = world.lod(c).expect("LOD-only");
        mip_full += mips.full_bricks();
        cpu_lod += mips_ram(&mips);
    }
    let w = window_side(r) as usize;
    let mirror_full = full_bricks * SLOT_AND_MASK + st.resident_full * RECORD;
    let mirror_lod = mip_full * SLOT_AND_MASK + st.resident_lod * RECORD;
    let mirror = mirror_full + mirror_lod + w * w * w * 4 + table.len() * 8 * 4;
    let generated = st.generated_chunks as f64 * 64.0 * 64.0 * 64.0;

    println!(
        "residency:         r_full {}, r_lod {}, h {}, seed {seed}, window {w}",
        r.r_full, r.r_lod, r.hysteresis
    );
    println!(
        "{:LABEL$}{} full, {} LOD-only",
        "chunks:", st.resident_full, st.resident_lod
    );
    println!(
        "full fraction:     {:.2} % of full chunks' bricks",
        100.0 * full_bricks as f64 / (full_bricks + uniform_bricks) as f64
    );
    println!(
        "mip-bricks:        {mip_full} Full of {} ({:.2} %)",
        st.resident_lod * 73,
        100.0 * mip_full as f64 / (st.resident_lod * 73).max(1) as f64
    );
    println!(
        "cpu bytes:         {} ({:.1} MiB): full {}, LOD {}",
        cpu_full + cpu_lod,
        mib(cpu_full + cpu_lod),
        cpu_full,
        cpu_lod
    );
    println!(
        "mirror bytes:      {} ({:.1} MiB): full {}, LOD {}",
        mirror,
        mib(mirror as u64),
        mirror_full,
        mirror_lod
    );
    println!("generation:        {secs:.3} s on {workers} workers");
    println!(
        "voxels/s/worker:   {:.3e}",
        generated / secs / workers as f64
    );
    println!(
        "| {} | {} | {} | {} | {:.1} | {:.1} | {:.1} | {:.1} |",
        r.r_full,
        r.r_lod,
        st.resident_full,
        st.resident_lod,
        mib(cpu_full + cpu_lod),
        mib(mirror_full as u64),
        mib(mirror_lod as u64),
        mib(mirror as u64)
    );
}

fn mib(b: u64) -> f64 {
    b as f64 / (1024.0 * 1024.0)
}

fn usage(bad: &str) -> ! {
    eprintln!(
        "bad argument {bad:?}; usage: world_budget [--chunks AxBxC | --stream R_FULL,R_LOD] [--seed N]"
    );
    std::process::exit(2)
}
