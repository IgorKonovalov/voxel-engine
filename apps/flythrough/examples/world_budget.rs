//! Generates the cave at a given size and prints its storage figures (`nfr.md` §3, §5).
//!
//! ```text
//! cargo run --release -p flythrough --example world_budget -- --chunks 20x20x8 --seed 1
//! ```
//!
//! Mirror bytes use the uploader's formula: each `Full` brick takes 1024 bytes of voxels plus
//! 8 of occupancy mask, each chunk a 512-entry `u32` record, and each material row 8 `f32`s.

use flythrough::cave::CaveSource;
use flythrough::cave_materials;
use vxe_world::{BRICK_VOXELS, Brick, CHUNK_BRICKS, ChunkExtent, World};

fn main() {
    let mut chunks = [20, 20, 8];
    let mut seed = 1u64;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let v = args.next().unwrap_or_else(|| usage(&a));
        match a.as_str() {
            "--chunks" => {
                let parts: Vec<i32> = v.split('x').filter_map(|p| p.parse().ok()).collect();
                chunks = parts.try_into().unwrap_or_else(|_| usage(&v));
            }
            "--seed" => seed = v.parse().unwrap_or_else(|_| usage(&v)),
            _ => usage(&a),
        }
    }

    let extent = ChunkExtent::new([0, 0, 0], chunks);
    let source = CaveSource::new(seed, extent);
    let table = cave_materials();
    let threads = std::thread::available_parallelism().map_or(1, usize::from);

    #[allow(
        clippy::disallowed_methods,
        reason = "a budget tool timing generation; the world itself reads no clock"
    )]
    let start = std::time::Instant::now();
    let world = World::generate_with_threads(extent, &source, &table, threads);
    #[allow(
        clippy::disallowed_methods,
        reason = "a budget tool timing generation; the world itself reads no clock"
    )]
    let secs = start.elapsed().as_secs_f64();

    let s = world.stats();
    let bricks = s.full_bricks + s.uniform_bricks;
    let slot_array = CHUNK_BRICKS * size_of::<Brick>();
    let cpu = s.full_bricks * BRICK_VOXELS * 2 + s.chunks * slot_array;
    let mirror =
        s.full_bricks * (BRICK_VOXELS * 2 + 8) + s.chunks * CHUNK_BRICKS * 4 + table.len() * 8 * 4;
    let voxels = s.chunks as f64 * 64.0 * 64.0 * 64.0;
    let air = world.count_in_box(
        extent.min.min_voxel(),
        vxe_world::VoxelCoord(extent.max.min_voxel().0.map(|v| v - 1)),
    );
    let air = air.iter().find(|(m, _)| *m == 0).map_or(0, |&(_, n)| n);

    println!(
        "extent:            {}x{}x{} chunks, seed {seed}",
        chunks[0], chunks[1], chunks[2]
    );
    println!("chunks:            {}", s.chunks);
    println!(
        "bricks:            {} Full, {} Uniform",
        s.full_bricks, s.uniform_bricks
    );
    println!(
        "full fraction:     {:.2} %",
        100.0 * s.full_bricks as f64 / bricks as f64
    );
    println!("air fraction:      {:.2} %", 100.0 * air as f64 / voxels);
    println!("cpu bytes:         {} ({:.1} MiB)", cpu, mib(cpu));
    println!("mirror bytes:      {} ({:.1} MiB)", mirror, mib(mirror));
    println!("generation:        {:.3} s on {threads} threads", secs);
    println!("voxels/s/thread:   {:.3e}", voxels / secs / threads as f64);
}

fn mib(b: usize) -> f64 {
    b as f64 / (1024.0 * 1024.0)
}

fn usage(bad: &str) -> ! {
    eprintln!("bad argument {bad:?}; usage: world_budget --chunks AxBxC --seed N");
    std::process::exit(2)
}
