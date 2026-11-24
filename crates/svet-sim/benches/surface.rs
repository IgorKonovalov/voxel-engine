//! The surface mover's budget (`nfr.md`, collision and coarse grids), the smoothed normal
//! on: one tick attached on
//! the F1 floor, one tick rounding the cube's top edge, and one `probe_ahead` on F2. The
//! fixtures are the integration tests'.

#[path = "../tests/support/mod.rs"]
mod support;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use support::*;
use svet_sim::{SurfaceInput, SurfaceMover, SurfaceParams, SurfaceState, probe_ahead};
use svet_world::Fixed3;

const CRAWL: SurfaceInput = SurfaceInput {
    heading: [256, 0],
    speed: 64,
};

fn tick_on(c: &mut Criterion, name: &str, world: &svet_world::World, start: SurfaceState) {
    let m = SurfaceMover::new(SurfaceParams {
        smooth: true,
        ..surface_params()
    });
    c.bench_function(name, |b| {
        b.iter_batched(
            || start,
            |mut s| std::hint::black_box(m.tick(world, &mut s, &CRAWL)),
            BatchSize::SmallInput,
        )
    });
}

fn ticks(c: &mut Criterion) {
    let floor = SurfaceState::new(Fixed3([2688, 2688, 256]), [0, 0, 256], [256, 0, 0]);
    tick_on(c, "one surface tick attached on the F1 floor", &f1(), floor);
    // Tick 33 of the cube course: the first tick past the top edge at x = 32.
    let edge = SurfaceState::new(Fixed3([8192, 6144, 8448]), [0, 0, 256], [256, 0, 0]);
    tick_on(
        c,
        "one surface tick rounding the cube's edge",
        &cube(),
        edge,
    );
}

fn probes(c: &mut Criterion) {
    let world = f2();
    c.bench_function("one probe_ahead on F2", |b| {
        b.iter(|| {
            std::hint::black_box(probe_ahead(
                &world,
                Fixed3([2688, 2688, 128]),
                [256, 0, 0],
                [0, 0, 256],
                3840,
                2560,
            ))
        })
    });
}

criterion_group!(benches, ticks, probes);
criterion_main!(benches);
