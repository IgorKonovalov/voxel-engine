//! The component boundary's overhead (`nfr.md` §4), two figures:
//!
//! - **Per host call:** one tick of 100 inputs, each a wire-encoded radius-1 m carve, through
//!   native `EchoLogic` and through the echo component, both on a host straight on the world.
//!   Each input is one `Host::apply`; the component's is the `apply` import (the op's wire
//!   bytes out, the tagged answer back). The difference over 100 is the cost per host call,
//!   the tick's own entry (fuel, clocks, the inputs lowered) included; the empty component
//!   tick reads that entry alone.
//! - **The bench tick:** one `Bench` input (100 radius-1 m carves) through native `DigLogic`
//!   and through the `flythrough-logic` component, as a ratio.
//!
//! Each iteration starts from a fresh `solid3_world()`, built untimed. Needs the components
//! `scripts/build-guests.sh` builds.

#[path = "../tests/support/mod.rs"]
mod support;

use std::time::{Duration, Instant};

use criterion::{Criterion, criterion_group, criterion_main};
use flythrough_logic::{BENCH_CARVES, DigInput, DigLogic};
use support::guest;
use vxe_net::test_logic::*;
use vxe_net::*;
use vxe_script_wasm::{WasmConfig, WasmLogic};
use vxe_world::{
    CellCoord, EditError, Fixed3, Hit, MaterialId, Op, OpResult, PrefabRegistry, Region,
    VoxelCoord, World,
};

#[allow(
    clippy::disallowed_methods,
    reason = "the bench times the tick alone, between untimed world builds"
)]
fn now() -> Instant {
    Instant::now()
}

/// A host straight on a world: no logs, no announcements, so both sides pay only the edit
/// and their own path to it.
struct WorldHost<'a>(&'a World);

impl Host for WorldHost<'_> {
    fn apply(&mut self, op: Op) -> Result<OpResult, EditError> {
        op.apply(self.0, &PrefabRegistry::new())
    }

    fn material_at(&self, v: VoxelCoord) -> Option<MaterialId> {
        self.0.material_at(v)
    }

    fn raycast(&self, origin: Fixed3, dir: Fixed3, max: i64) -> Option<Hit> {
        self.0.raycast(origin, dir.0, max).ok().flatten()
    }

    fn raycast_with_steps(&self, origin: Fixed3, dir: Fixed3, max: i64) -> (Option<Hit>, u64) {
        self.0
            .raycast_with_stats(origin, dir.0, max)
            .unwrap_or((None, 0))
    }

    fn count_in_box(&self, lo: VoxelCoord, hi: VoxelCoord) -> Vec<(MaterialId, u64)> {
        self.0.count_in_box(lo, hi).unwrap_or_default()
    }

    /// This host keeps no coarse grid; nothing here asks the support query.
    fn unsupported(&self, _changed: &[CellCoord]) -> Vec<Region> {
        Vec::new()
    }
}

/// The component `name`, loaded under the default configuration and initialised.
fn component(name: &str) -> WasmLogic {
    let mut logic =
        WasmLogic::load(&guest(name), &WasmConfig::default()).expect("the component loads");
    let world = solid3_world();
    logic
        .try_init(&mut WorldHost(&world), 0)
        .expect("the component's init returns");
    logic
}

/// `iters` ticks of `logic` on `inputs`, each on a fresh `solid3_world()`, timing the tick
/// alone; `check` sees each tick's world and broadcast.
fn time_ticks(
    iters: u64,
    logic: &mut dyn Logic,
    tick: &mut u64,
    inputs: &[ClientInput],
    check: impl Fn(&World, &TickOutput),
) -> Duration {
    let mut total = Duration::ZERO;
    for _ in 0..iters {
        let world = solid3_world();
        let mut host = WorldHost(&world);
        let t = now();
        let out = logic.tick(&mut host, *tick, inputs);
        total += now() - t;
        assert_eq!(logic.take_error(), None);
        check(&world, &out);
        *tick += 1;
    }
    total
}

fn boundary(c: &mut Criterion) {
    let carves: Vec<ClientInput> = seeded_carves(0xB0DA, 100)
        .into_iter()
        .map(|op| ClientInput {
            client: 0,
            bytes: op_bytes(op),
        })
        .collect();
    let all_applied = |_: &World, out: &TickOutput| {
        assert_eq!(out.broadcast, [ECHO_APPLIED; 100]);
    };

    let mut group = c.benchmark_group("boundary");
    group.bench_function("100 apply: native EchoLogic tick", |b| {
        let mut tick = 0;
        b.iter_custom(|iters| time_ticks(iters, &mut EchoLogic, &mut tick, &carves, all_applied));
    });
    let mut echo = component("echo");
    let mut echo_tick = 0;
    group.bench_function("100 apply: echo component tick", |b| {
        b.iter_custom(|iters| time_ticks(iters, &mut echo, &mut echo_tick, &carves, all_applied));
    });
    group.bench_function("empty tick: echo component", |b| {
        b.iter_custom(|iters| {
            time_ticks(iters, &mut echo, &mut echo_tick, &[], |_, out| {
                assert!(out.broadcast.is_empty());
            })
        });
    });

    let bench = [ClientInput {
        client: 0,
        bytes: DigInput::Bench {
            origin: [0, 0, 0],
            seed: 9,
        }
        .encode(),
    }];
    let bench_ran = |_: &World, out: &TickOutput| {
        let text = std::str::from_utf8(&out.broadcast).expect("UTF-8 lines");
        assert_eq!(
            text.lines().filter(|l| *l == "op bench").count(),
            BENCH_CARVES
        );
    };
    group.bench_function("bench tick: native DigLogic", |b| {
        let mut tick = 0;
        b.iter_custom(|iters| {
            time_ticks(
                iters,
                &mut DigLogic::default(),
                &mut tick,
                &bench,
                bench_ran,
            )
        });
    });
    let mut dig = component("flythrough-logic");
    let mut dig_tick = 0;
    group.bench_function("bench tick: flythrough-logic component", |b| {
        b.iter_custom(|iters| time_ticks(iters, &mut dig, &mut dig_tick, &bench, bench_ran));
    });
    group.finish();
}

criterion_group!(benches, boundary);
criterion_main!(benches);
