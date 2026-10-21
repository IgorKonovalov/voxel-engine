//! The attribute work-list builder on one crossing of the fly-through's residency (r_full 16,
//! r_lod 24, hysteresis 1), without a window or an adapter. The view is the worst case: every
//! chunk within 17 chunks of the origin is full with every brick `Full`, every other chunk
//! within 25 is LOD-only. Two frames of a +x focus step are timed: the eviction frame (the
//! 1961 chunks leaving ball(25) as shell chunks) and the promotion frame (the 797 chunks
//! entering ball(16) taken whole).

use std::collections::{HashMap, HashSet};
use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use vxe_render::attr_work::{self, MirrorView};
use vxe_world::{CHUNK_BRICKS, CHUNK_EDGE_BRICKS, ChunkCoord, brick_slot};

struct Synthetic {
    /// Full or LOD-only, and the chunk's first slot.
    chunks: HashMap<ChunkCoord, (bool, u32)>,
}

fn ball(r2: i32, at: [i32; 3]) -> HashSet<ChunkCoord> {
    let r = f64::from(r2).sqrt() as i32 + 1;
    let mut out = HashSet::new();
    for z in -r..=r {
        for y in -r..=r {
            for x in -r..=r {
                if x * x + y * y + z * z <= r2 {
                    out.insert(ChunkCoord([x + at[0], y + at[1], z + at[2]]));
                }
            }
        }
    }
    out
}

impl Synthetic {
    fn new() -> Self {
        let mut coords: Vec<ChunkCoord> = ball(25 * 25, [0; 3]).into_iter().collect();
        coords.sort_by_key(|c| c.0);
        let chunks = coords
            .into_iter()
            .enumerate()
            .map(|(n, c)| {
                let d2: i32 = c.0.iter().map(|v| v * v).sum();
                (c, (d2 <= 17 * 17, n as u32 * CHUNK_BRICKS as u32))
            })
            .collect();
        Self { chunks }
    }
}

impl MirrorView for Synthetic {
    fn is_full_chunk(&self, c: ChunkCoord) -> bool {
        self.chunks.get(&c).is_some_and(|m| m.0)
    }

    fn full_slot(&self, b: [i32; 3]) -> Option<u32> {
        let e = CHUNK_EDGE_BRICKS;
        let c = ChunkCoord(b.map(|v| v.div_euclid(e)));
        let &(_, first) = self.chunks.get(&c).filter(|m| m.0)?;
        Some(first + brick_slot(b.map(|v| v.rem_euclid(e))) as u32)
    }
}

fn bench(c: &mut Criterion) {
    let view = Synthetic::new();
    let shifted25 = ball(25 * 25, [1, 0, 0]);
    let evicted: HashSet<ChunkCoord> = ball(25 * 25, [0; 3])
        .into_iter()
        .filter(|c| !shifted25.contains(c))
        .collect();
    let ball16 = ball(16 * 16, [0; 3]);
    let promoted: HashSet<ChunkCoord> = ball(16 * 16, [1, 0, 0])
        .into_iter()
        .filter(|c| !ball16.contains(c))
        .collect();
    let none = HashSet::new();
    let none_b = HashSet::new();
    let mut items = Vec::new();
    let mut g = c.benchmark_group("attr_work");
    g.sample_size(20);
    g.bench_function("eviction frame (1961 shell chunks)", |b| {
        b.iter(|| {
            items.clear();
            black_box(attr_work::build(
                &view, &none, &none_b, &evicted, &mut items,
            ))
        })
    });
    g.bench_function("promotion frame (797 chunks whole)", |b| {
        b.iter(|| {
            items.clear();
            black_box(attr_work::build(
                &view, &promoted, &none_b, &none, &mut items,
            ))
        })
    });
    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
