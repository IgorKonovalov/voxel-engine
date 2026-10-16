//! The collision budget (`nfr.md`, collision and coarse grids): 1000 controller ticks of the
//! F6 staircase course, and 1000 sweeps of the F2 wall case (a body swept 10000 fixed units
//! along `+x` into the wall two chunks away). The fixtures are the integration tests'.
//!
//! The longer F6 runs leave the `[-2, 2)³` extent near tick 150 and fall from there, so most
//! of their ticks are free fall. The 60-tick run is exactly the staircase walk the controller
//! test pins (20 steps, grounded on the top at tick 60): every tick on terrain.

#[path = "../tests/support/mod.rs"]
mod support;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use support::*;
use vxe_sim::{CharacterController, CharacterState, MoveInput, sweep};
use vxe_world::Fixed3;

/// The F6 course's input: walk along `+x`, no jump, no climb.
const WALK: MoveInput = MoveInput {
    wish: WISH,
    jump: false,
    climb: false,
};

fn ticks(c: &mut Criterion) {
    let world = f6();
    let cc = CharacterController::new(params());
    for n in [300u32, 1000] {
        c.bench_function(&format!("{n} ticks of the F6 course"), |b| {
            b.iter_batched(
                || CharacterState::new(Fixed3(F6_START)),
                |mut s| {
                    for _ in 0..n {
                        std::hint::black_box(cc.tick(&world, &mut s, &WALK));
                    }
                    s
                },
                BatchSize::SmallInput,
            )
        });
    }
}

fn sweeps(c: &mut Criterion) {
    let world = f2();
    let body = body_at(S);
    let delta = Fixed3([10000, 0, 0]);
    c.bench_function("1000 sweeps of the F2 wall case", |b| {
        b.iter(|| {
            for _ in 0..1000 {
                std::hint::black_box(sweep(&world, &body, delta));
            }
        })
    });
}

fn terrain_ticks(c: &mut Criterion) {
    let world = f6();
    let cc = CharacterController::new(params());
    c.bench_function("60 ticks of the F6 staircase, all on terrain", |b| {
        b.iter_batched(
            || CharacterState::new(Fixed3(F6_START)),
            |mut s| {
                for _ in 0..60 {
                    std::hint::black_box(cc.tick(&world, &mut s, &WALK));
                }
                assert!(s.grounded && s.foot == Fixed3([20608, 2688, 5120]));
                s
            },
            BatchSize::SmallInput,
        )
    });
}

criterion_group!(benches, ticks, terrain_ticks, sweeps);
criterion_main!(benches);
