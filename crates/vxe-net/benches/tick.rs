//! Tick overhead (`nfr.md` §4): 100 inputs, each a radius-1 m carve, sent by a shared client,
//! drained in one tick through `EchoLogic` and announced to one poll, against the same 100
//! carves applied directly, and against those carves each followed by the coarse grid update
//! the server makes after every applied op. The tick minus the direct carves with their grid
//! updates is the server's overhead per 100 inputs; the direct carves with grid updates minus
//! the direct carves alone is the grid's cost on that path (`nfr.md` §8). Each iteration starts
//! from a fresh world (and grid), built untimed.

#[path = "../tests/support/mod.rs"]
mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use criterion::{Criterion, criterion_group, criterion_main};
use support::*;
use vxe_net::*;
use vxe_world::CoarseGrid;

#[allow(
    clippy::disallowed_methods,
    reason = "the bench times the tick path alone, between untimed world builds"
)]
fn since(t: Instant) -> Duration {
    Instant::now() - t
}

#[allow(
    clippy::disallowed_methods,
    reason = "the bench times the tick path alone, between untimed world builds"
)]
fn now() -> Instant {
    Instant::now()
}

fn tick(c: &mut Criterion) {
    let carves = seeded_carves(0x71C4, 100);
    let inputs: Vec<Vec<u8>> = carves.iter().map(|op| op_bytes(*op)).collect();
    let mut group = c.benchmark_group("tick");
    group.bench_function(
        "100 inputs: send, one EchoLogic tick, one poll (shared)",
        |b| {
            b.iter_custom(|iters| {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    let world = Arc::new(solid3_world());
                    let mut server = Server::new(world.clone(), Box::new(EchoLogic), 0);
                    let (near, far) = MemChannel::pair();
                    server.attach(far);
                    let mut client = Client::shared(world, near);
                    let t = now();
                    for bytes in &inputs {
                        client.send_input(bytes.clone());
                    }
                    let report = server.tick();
                    let events = client.poll();
                    total += since(t);
                    assert_eq!(report.applied, 100);
                    // 100 `Applied` and the logic's one `Event`.
                    assert_eq!(events.len(), 101);
                    std::hint::black_box(events);
                }
                total
            })
        },
    );
    group.bench_function("100 direct apply calls", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                let world = solid3_world();
                let t = now();
                for op in &carves {
                    std::hint::black_box(world.apply(op).unwrap());
                }
                total += since(t);
            }
            total
        })
    });
    group.bench_function(
        "100 direct apply calls, each with its coarse grid update",
        |b| {
            b.iter_custom(|iters| {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    let world = solid3_world();
                    let mut grid = CoarseGrid::build(&world);
                    let t = now();
                    for op in &carves {
                        let r = world.apply(op).unwrap();
                        std::hint::black_box(grid.update_aabb(&world, r.aabb));
                    }
                    total += since(t);
                }
                total
            })
        },
    );
    group.finish();
}

criterion_group!(benches, tick);
criterion_main!(benches);
