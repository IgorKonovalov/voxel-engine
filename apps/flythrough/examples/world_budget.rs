//! Prints the cave's storage figures (`nfr.md` §3, §5), bounded or streamed.
//!
//! ```text
//! cargo run --release -p flythrough --example world_budget -- --chunks 20x20x8 --seed 1
//! cargo run --release -p flythrough --example world_budget -- --stream 16,24 --seed 1
//! cargo run --release -p flythrough --example world_budget -- --stream 16,24 --surface
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
//!
//! `--surface` (with `--stream`) adds the contour-attribute census of the `Full` bricks of
//! full chunks: their count, their *plane voxels* (solid voxels with at least one air face
//! neighbour, neighbours read across chunk edges from the resident chunk or, past the
//! residency, from the source; an upper bound on the attribute pass's planes, which also drop
//! voxels whose occupancy gradient vanishes), the plane voxels per brick in 32-voxel bins, the
//! 2 x 2 x 2 blocks holding a plane voxel, and the attribute bytes of each footprint layout.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard};

use flythrough::cave::CaveSource;
use flythrough::{cave_materials, chunk_ram, mips_ram, window_side};
use vxe_world::{
    AIR, BRICK_EDGE, BRICK_VOXELS, Brick, CHUNK_BRICKS, Chunk, ChunkCoord, ChunkExtent,
    PersistConfig, Residency, StreamConfig, Streamer, World, WorldSource, brick_of_slot,
    brick_slot, index_in_brick,
};

/// Mirror bytes per `Full` brick or mip-brick, and per mirrored chunk's record.
const SLOT_AND_MASK: usize = 512 + 8;
const RECORD: usize = 2560;

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
    let mut surface = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--surface" {
            surface = true;
            continue;
        }
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
        Some(r) => streamed(seed, r, surface),
        None if surface => usage("--surface"),
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
    println!("cpu bytes:         {} ({:.1} MiB)", cpu, mib(cpu as u64));
    println!(
        "mirror bytes:      {} ({:.1} MiB)",
        mirror,
        mib(mirror as u64)
    );
    println!("generation:        {:.3} s on {threads} threads", secs);
    println!("voxels/s/thread:   {:.3e}", voxels / secs / threads as f64);
}

fn streamed(seed: u64, r: Residency, surface: bool) {
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
        "chunks:            {} full, {} LOD-only",
        st.resident_full, st.resident_lod
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
    if surface {
        surface_census(&world, &source, workers, mirror as u64);
    }
}

/// Bytes of the contour attribute per `Full` brick of a full chunk as built: two 512-byte
/// slots and their two 8-byte mask entries.
const ATTR_PER_BRICK: u64 = 2 * (512 + 8);
/// Plan 0005's live rise of the streamed mirror at r_full 16, r_lod 24 with attribute pages.
const PLAN_0005_RISE: u64 = 1_705_903_808;

/// What the census counts over a set of bricks.
#[derive(Clone, Copy, Default)]
struct Census {
    bricks: u64,
    planes: u64,
    blocks: u64,
    /// Bricks by plane voxels, in bins of 32 (bin 16 holds exactly 512).
    hist: [u64; 17],
}

impl Census {
    fn add(&mut self, o: &Census) {
        self.bricks += o.bricks;
        self.planes += o.planes;
        self.blocks += o.blocks;
        for (a, b) in self.hist.iter_mut().zip(&o.hist) {
            *a += b;
        }
    }
}

/// Chunks generated from the source past the residency, for neighbours across its edge.
type SourceCache = Mutex<HashMap<ChunkCoord, Arc<RwLock<Chunk>>>>;

/// The resident chunk at `c`, or the source's, generated once.
fn chunk_or_source(
    world: &World,
    source: &CaveSource,
    cache: &SourceCache,
    c: ChunkCoord,
) -> Arc<RwLock<Chunk>> {
    if let Some(chunk) = world.store().get(c) {
        return chunk;
    }
    let mut cache = cache.lock().expect("cache lock");
    cache
        .entry(c)
        .or_insert_with(|| {
            let mut chunk = Chunk::default();
            source.chunk(c, &mut chunk);
            Arc::new(RwLock::new(chunk))
        })
        .clone()
}

/// The census of chunk `c`'s `Full` bricks.
fn census_chunk(world: &World, source: &CaveSource, cache: &SourceCache, c: ChunkCoord) -> Census {
    const FACES: [[i32; 3]; 6] = [
        [-1, 0, 0],
        [1, 0, 0],
        [0, -1, 0],
        [0, 1, 0],
        [0, 0, -1],
        [0, 0, 1],
    ];
    let here = world.store().get(c).expect("resident");
    let here = here.read().expect("chunk lock");
    if !here.bricks().iter().any(Brick::is_full) {
        return Census::default();
    }
    // The chunk and its six face neighbours, indexed 0 (here) and 1 + face.
    let arcs: Vec<Arc<RwLock<Chunk>>> = FACES
        .iter()
        .map(|d| {
            chunk_or_source(
                world,
                source,
                cache,
                ChunkCoord([0, 1, 2].map(|k| c.0[k] + d[k])),
            )
        })
        .collect();
    let guards: Vec<RwLockReadGuard<'_, Chunk>> =
        arcs.iter().map(|a| a.read().expect("chunk lock")).collect();
    let e = BRICK_EDGE;
    let chunk_bricks = 64 / e;
    // The brick at chunk-local brick coordinate `b`, which may lie one brick past a face.
    let brick_at = |b: [i32; 3]| -> &Brick {
        let d = b.map(|v| v.div_euclid(chunk_bricks));
        let local = b.map(|v| v.rem_euclid(chunk_bricks));
        let slot = brick_slot(local);
        match FACES.iter().position(|f| *f == d) {
            Some(k) => guards[k].brick(slot),
            None => here.brick(slot),
        }
    };
    let mut out = Census::default();
    for slot in 0..CHUNK_BRICKS {
        let brick = here.brick(slot);
        if !brick.is_full() {
            continue;
        }
        let b = brick_of_slot(slot);
        // Occupancy of the brick and its six face layers, 10 voxels to an edge.
        let mut occ = [false; 1000];
        let at = |x: i32, y: i32, z: i32| ((x + 1) + 10 * ((y + 1) + 10 * (z + 1))) as usize;
        for z in 0..e {
            for y in 0..e {
                for x in 0..e {
                    occ[at(x, y, z)] = brick.get(index_in_brick([x, y, z])) != AIR;
                }
            }
        }
        for d in FACES {
            let nb = brick_at([0, 1, 2].map(|k| b[k] + d[k]));
            for u in 0..e {
                for w in 0..e {
                    // The neighbour's layer touching this brick, and where it sits in `occ`.
                    let mut l = [0; 3];
                    let mut o = [0; 3];
                    let axis = d.iter().position(|&v| v != 0).expect("a face");
                    let (a1, a2) = ((axis + 1) % 3, (axis + 2) % 3);
                    l[a1] = u;
                    l[a2] = w;
                    o[a1] = u;
                    o[a2] = w;
                    l[axis] = if d[axis] > 0 { 0 } else { e - 1 };
                    o[axis] = if d[axis] > 0 { e } else { -1 };
                    occ[at(o[0], o[1], o[2])] = nb.get(index_in_brick(l)) != AIR;
                }
            }
        }
        let mut planes = 0u64;
        let mut blocks = 0u64;
        for i in 0..64 {
            let k = [i % 4, (i / 4) % 4, i / 16].map(|v| v * 2);
            let mut any = false;
            for j in 0..8 {
                let v = [k[0] + (j & 1), k[1] + ((j >> 1) & 1), k[2] + (j >> 2)];
                if !occ[at(v[0], v[1], v[2])] {
                    continue;
                }
                let exposed = FACES
                    .iter()
                    .any(|f| !occ[at(v[0] + f[0], v[1] + f[1], v[2] + f[2])]);
                if exposed {
                    planes += 1;
                    any = true;
                }
            }
            blocks += u64::from(any);
        }
        out.bricks += 1;
        out.planes += planes;
        out.blocks += blocks;
        out.hist[(planes / 32) as usize] += 1;
    }
    out
}

/// The plane-voxel census of the resident full chunks and the attribute bytes of each
/// footprint layout; `face_mirror` is the face-mode mirror estimate printed above.
fn surface_census(world: &World, source: &CaveSource, threads: usize, face_mirror: u64) {
    let start = now();
    let coords = world.store().coords();
    let cache = SourceCache::default();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let total = Mutex::new(Census::default());
    std::thread::scope(|s| {
        for _ in 0..threads.max(1) {
            s.spawn(|| {
                let mut mine = Census::default();
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(&c) = coords.get(i) else {
                        break;
                    };
                    mine.add(&census_chunk(world, source, &cache, c));
                }
                total.lock().expect("total lock").add(&mine);
            });
        }
    });
    let t = *total.lock().expect("total lock");
    let secs = (now() - start).as_secs_f64();
    let n = t.bricks;
    println!(
        "surface census:    {} Full bricks of full chunks, {} source chunks past the residency, {secs:.1} s",
        n,
        cache.lock().expect("cache lock").len()
    );
    println!(
        "attribute bytes:   {} at {ATTR_PER_BRICK} B per brick; Plan 0005's live rise {PLAN_0005_RISE} ({:+.3} %)",
        n * ATTR_PER_BRICK,
        100.0 * ((n * ATTR_PER_BRICK) as f64 / PLAN_0005_RISE as f64 - 1.0)
    );
    println!(
        "plane voxels:      {} ({:.1} per brick, {:.2} % of the bricks' voxels)",
        t.planes,
        t.planes as f64 / n.max(1) as f64,
        100.0 * t.planes as f64 / (n.max(1) * BRICK_VOXELS as u64) as f64
    );
    println!(
        "plane blocks:      {} of {} 2x2x2 blocks ({:.2} %)",
        t.blocks,
        n * 64,
        100.0 * t.blocks as f64 / (n.max(1) * 64) as f64
    );
    println!("plane voxels per brick (bricks per 32-voxel bin):");
    for (k, &count) in t.hist.iter().enumerate() {
        if count > 0 {
            println!(
                "  {:>3}..{:<3}  {count:>9}  {:6.2} %",
                k * 32,
                k * 32 + 31,
                100.0 * count as f64 / n.max(1) as f64
            );
        }
    }
    let rows = [
        ("current (2 x 512 B + 2 x 8 B)", n * ATTR_PER_BRICK),
        ("A (64 B mask + 2 B per plane voxel)", n * 64 + 2 * t.planes),
        ("B (512 B code + 8 B mask)", n * 520),
        ("C (64 x u16 per brick)", n * 128),
    ];
    println!("| layout | attribute bytes live | per brick | face mirror + attributes |");
    println!("|---|---|---|---|");
    for (name, bytes) in rows {
        println!(
            "| {name} | {bytes} | {:.1} | {} |",
            bytes as f64 / n.max(1) as f64,
            face_mirror + bytes
        );
    }
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
