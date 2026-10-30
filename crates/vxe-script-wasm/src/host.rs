//! [`WasmLogic`]: a logic component behind [`vxe_net::Logic`].
//!
//! The engine compiles the component once, at [`WasmLogic::load`]. [`Logic::init`] makes the
//! store, with a WASI context built from the seed (`wasi` module), instantiates the component
//! and calls its `init`; [`Logic::tick`] calls its `tick`. After a failed call the next tick
//! starts a fresh instance from the seed, up to [`WasmConfig::max_restarts`] times, and then
//! the logic stops. During each of those calls the server's [`Host`] is lent to the store, and the
//! component's `host` imports answer through it; outside them an import traps.

use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::path::Path;
use std::ptr::NonNull;

use vxe_net::{ClientInput, Host, Logic, LogicError, TickOutput};
use vxe_world::wire::{Reader, decode_op};
use vxe_world::{CellCoord, EditError, Face, Fixed3, MaterialId, Op, OpResult, Region, VoxelCoord};
use wasmtime::StoreContextMut;
use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Engine, ResourceLimiter, Store, Trap};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

use crate::wasi::{WasiHandles, context, ready_timer, stream_state};
use crate::{ANSWER_UNDECODABLE, encode_answer};

mod bindings {
    wasmtime::component::bindgen!({
        path: "wit",
        world: "logic",
        imports: { default: trappable },
    });
}

use bindings::vxe::logic::host as wit;

/// The limits and the clock step a component runs under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WasmConfig {
    /// Fuel for one call of `init` or `tick`, reset before each.
    pub fuel_per_tick: u64,
    /// The cap on all of the component's linear memories together, in bytes.
    pub memory_bytes: u64,
    /// Nanoseconds of the component's clocks per tick.
    pub frame_ns: u64,
    /// Fresh instances started after a failure before the logic stops for good; 0 is
    /// fail-stop.
    pub max_restarts: u32,
    /// The fuel each host import is charged for the work it did.
    pub rates: WorkRates,
    /// The support query's `max_region` the host's `unsupported` floods under, for that
    /// import's fuel bound ([`WorkRates::support_bound`]). A host configured with a larger cap
    /// is still charged exactly; only the trap before the work comes later.
    pub support_max_region: u32,
}

impl Default for WasmConfig {
    fn default() -> Self {
        Self {
            fuel_per_tick: 10_000_000_000,
            memory_bytes: 64 << 20,
            frame_ns: 16_666_667,
            max_restarts: 8,
            rates: WorkRates::DEFAULT,
            support_max_region: vxe_net::DEFAULT_SUPPORT.max_region,
        }
    }
}

/// The fuel a host import is charged, on top of the guest's own instructions.
///
/// Before any host work, the imports that take a list are charged `per_byte` for each byte
/// of it: `apply` its op bytes, `unsupported` 12 per changed cell. The charge for the work
/// itself: every call `per_call`; `apply` also `per_voxel` for each voxel in the op's bounds
/// (an edit's shape bounds, a stamp's or a region op's changed bounds) and `64 x per_voxel`
/// for each chunk it touched; `raycast` `per_step` for each step of its walk; `count_in_box`
/// `per_voxel` for each voxel it counted; `unsupported` `per_step` for each cell the support
/// query looked at. A refused or undecodable call is charged its bytes and `per_call` only.
/// The steps come from the lent host's stats (`Host::raycast_with_steps`,
/// `Host::unsupported_with_steps`); a host that does not report them is charged none.
///
/// The three imports that walk the world (`raycast`, `count-in-box`, `unsupported`) are
/// charged an upper bound of that work **before** the walk ([`WorkRates::ray_bound`],
/// [`WorkRates::count_bound`], [`WorkRates::support_bound`]) and settle after it: the
/// difference is refunded, or, past a bound a host exceeded, charged. A call that completes
/// is so charged exactly what the work cost; a bound past the fuel left ends the call before
/// any host work. `apply` and `material-at` are charged after the host returns. A charge
/// past the fuel left ends the call as [`LogicError::Fuel`], as an instruction overrun does.
/// Every charge comes from integer counts the world produces, so it is the same on every
/// machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkRates {
    pub per_voxel: u64,
    pub per_step: u64,
    pub per_call: u64,
    pub per_byte: u64,
}

impl WorkRates {
    /// 10 per voxel, 10 per step, 100 per call, 1 per argument byte.
    pub const DEFAULT: Self = Self {
        per_voxel: 10,
        per_step: 10,
        per_call: 100,
        per_byte: 1,
    };

    /// No charge: the guest's instructions alone use fuel.
    pub const ZERO: Self = Self {
        per_voxel: 0,
        per_step: 0,
        per_call: 0,
        per_byte: 0,
    };

    /// The charge for an `apply` of `op` that gave `result`.
    pub fn apply_cost(&self, op: &Op, result: &Result<OpResult, EditError>) -> u64 {
        let Ok(r) = result else {
            return self.per_call;
        };
        let bounds = match op {
            Op::Edit(e) => e.shape.voxel_bounds(),
            _ => r
                .aabb()
                .map(|(lo, hi)| (lo.0.map(i64::from), hi.0.map(i64::from))),
        };
        let voxels = bounds.map_or(0, |(lo, hi)| {
            (0..3)
                .map(|k| hi[k].abs_diff(lo[k]).saturating_add(1))
                .fold(1u64, u64::saturating_mul)
        });
        let chunks = r.edit_result().touched.len() as u64;
        self.per_call.saturating_add(
            self.per_voxel
                .saturating_mul(voxels.saturating_add(chunks.saturating_mul(64))),
        )
    }

    /// The charge, before any host work, for an argument list of `bytes` bytes.
    pub fn bytes_cost(&self, bytes: usize) -> u64 {
        self.per_byte
            .saturating_mul(u64::try_from(bytes).unwrap_or(u64::MAX))
    }

    /// The charge for a `raycast` or an `unsupported` whose work took `steps`.
    pub fn steps_cost(&self, steps: u64) -> u64 {
        self.per_call
            .saturating_add(self.per_step.saturating_mul(steps))
    }

    /// The charge for a `count_in_box` that returned `counts`.
    pub fn count_cost(&self, counts: &[(MaterialId, u64)]) -> u64 {
        let voxels = counts.iter().fold(0u64, |n, &(_, c)| n.saturating_add(c));
        self.per_call
            .saturating_add(self.per_voxel.saturating_mul(voxels))
    }

    /// The most a `count_in_box` over the inclusive box `lo..=hi` can be charged: every voxel
    /// of the box counted. A box reversed on an axis counts nothing.
    pub fn count_bound(&self, lo: VoxelCoord, hi: VoxelCoord) -> u64 {
        let voxels = (0..3)
            .map(|k| {
                let (l, h) = (i64::from(lo.0[k]), i64::from(hi.0[k]));
                if h < l {
                    0
                } else {
                    (h - l + 1).cast_unsigned()
                }
            })
            .fold(1u64, u64::saturating_mul);
        self.per_call
            .saturating_add(self.per_voxel.saturating_mul(voxels))
    }

    /// The most a `raycast` along `origin + dir * t / 256`, `0 <= t <= max`, can be charged.
    /// Each step of the world's walk leaves a cell of one or more whole voxels, so it crosses
    /// at least one voxel plane; the ray crosses at most `ceil(|dir[k]| x max / 2^16) + 1`
    /// planes on axis `k`, and the hit's cell is one step more.
    pub fn ray_bound(&self, dir: Fixed3, max: i64) -> u64 {
        let t = u128::from(max.max(0).cast_unsigned());
        let steps = dir.0.iter().fold(1u128, |n, &d| {
            let reach = u128::from(d.unsigned_abs()) * t;
            n.saturating_add(reach.div_ceil(1 << 16)).saturating_add(1)
        });
        self.steps_cost(u64::try_from(steps).unwrap_or(u64::MAX))
    }

    /// The most an `unsupported` over `changed` can be charged by a host flooding under
    /// `max_region` (`vxe_world::find_unsupported_with_stats`): each of the `d` distinct
    /// cells and its six neighbours is a candidate, and each candidate starts at most one
    /// flood of at most `max_region + 1` cells, so `7 d (max_region + 2)` steps.
    pub fn support_bound(&self, changed: &[CellCoord], max_region: u32) -> u64 {
        let mut cells = changed.to_vec();
        cells.sort_unstable();
        cells.dedup();
        let steps = (cells.len() as u64)
            .saturating_mul(7)
            .saturating_mul(u64::from(max_region) + 2);
        self.steps_cost(steps)
    }
}

impl Default for WorkRates {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// The server's host, lent to the store for one call.
struct Lent(Option<NonNull<dyn Host>>);

// SAFETY: the pointer is set only for the duration of one `WasmLogic::call`, on the thread
// that holds `&mut WasmLogic` and the `&mut dyn Host` it came from; nothing reads it from
// another thread, and it is cleared before that borrow ends.
unsafe impl Send for Lent {}

/// The cap on memory growth across all of a store's linear memories; records whether the
/// component hit it. A component with several core instances has several memories, and none
/// of them shrinks or is freed while the store lives.
struct Limits {
    memory_bytes: usize,
    /// The bytes of every memory accepted so far.
    total: usize,
    hit: bool,
}

/// Table elements a component may grow a table to.
const TABLE_ELEMENTS: usize = 1 << 16;

impl ResourceLimiter for Limits {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        let total = self.total.saturating_add(desired.saturating_sub(current));
        if total > self.memory_bytes {
            self.hit = true;
            wasmtime::bail!(
                "memory growth to {total} bytes across the component's memories exceeds the \
                 cap of {} bytes",
                self.memory_bytes
            );
        }
        self.total = total;
        Ok(true)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        Ok(desired <= TABLE_ELEMENTS)
    }
}

struct State {
    wasi: WasiCtx,
    table: ResourceTable,
    host: Lent,
    limits: Limits,
}

impl State {
    /// The lent host; an error (and so a trap) when none is lent.
    fn host(&mut self) -> wasmtime::Result<&mut dyn Host> {
        match self.host.0 {
            // SAFETY: a lent pointer comes from a `&mut dyn Host` that outlives the call it is
            // lent for (`WasmLogic::call` clears it before returning), and the store reaches
            // it only through `&mut self`, so this is the one live reference.
            Some(mut p) => Ok(unsafe { p.as_mut() }),
            None => wasmtime::bail!("a host import was called outside init and tick"),
        }
    }
}

impl WasiView for State {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

fn voxel(v: wit::Voxel) -> VoxelCoord {
    VoxelCoord([v.x, v.y, v.z])
}

fn wit_voxel(v: VoxelCoord) -> wit::Voxel {
    let [x, y, z] = v.0;
    wit::Voxel { x, y, z }
}

fn fixed3(p: wit::Fixed3) -> Fixed3 {
    Fixed3([p.x, p.y, p.z])
}

fn wit_region(g: Region) -> wit::Region {
    wit::Region {
        cells: g
            .cells
            .into_iter()
            .map(|c| {
                let [x, y, z] = c.0;
                wit::Cell { x, y, z }
            })
            .collect(),
        counts: g.counts,
        lo: wit_voxel(g.aabb.0),
        hi: wit_voxel(g.aabb.1),
    }
}

fn wit_face(f: Face) -> wit::Face {
    match f {
        Face::NegX => wit::Face::NegX,
        Face::PosX => wit::Face::PosX,
        Face::NegY => wit::Face::NegY,
        Face::PosY => wit::Face::PosY,
        Face::NegZ => wit::Face::NegZ,
        Face::PosZ => wit::Face::PosZ,
    }
}

/// Runs one host import's call. A panic in it (a bug in the host or the world; parameters
/// out of the world's range are refused, not panicked on) becomes an error, so the call
/// traps and the logic fails instead of the panic unwinding through wasmtime into the
/// server.
fn guarded<R>(call: impl FnOnce() -> wasmtime::Result<R>) -> wasmtime::Result<R> {
    match catch_unwind(AssertUnwindSafe(call)) {
        Ok(r) => r,
        Err(panic) => {
            let message = panic
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("a payload that is not a string");
            wasmtime::bail!("host call panicked: {message}")
        }
    }
}

/// The `host` interface of the `logic` world, as the component imports it.
const HOST_INTERFACE: &str = "vxe:logic/host@0.1.0";

/// Takes `cost` from the store's fuel; past the fuel left, the call ends as an out-of-fuel
/// trap, as an instruction overrun does. A zero cost touches nothing.
fn charge(store: &mut StoreContextMut<'_, State>, cost: u64) -> wasmtime::Result<()> {
    if cost == 0 {
        return Ok(());
    }
    let left = store.get_fuel()?;
    match left.checked_sub(cost) {
        Some(left) => store.set_fuel(left),
        None => {
            store.set_fuel(0)?;
            Err(Trap::OutOfFuel.into())
        }
    }
}

/// The out-of-fuel trap of an import whose pre-charged `bound` exceeds the `left` fuel,
/// naming the import. [`classify`] reads it as [`LogicError::Fuel`].
fn fuel_trap(import: &str, bound: u64, left: u64) -> wasmtime::Error {
    wasmtime::Error::from(Trap::OutOfFuel).context(format!(
        "host import `{import}` needs up to {bound} fuel before its work; {left} left"
    ))
}

/// Takes the upper bound `bound` of an import's charge before its host work; past the fuel
/// left, the call ends as an out-of-fuel trap naming `import` and no host work runs.
fn precharge(
    store: &mut StoreContextMut<'_, State>,
    import: &str,
    bound: u64,
) -> wasmtime::Result<()> {
    let left = store.get_fuel()?;
    match left.checked_sub(bound) {
        Some(left) => store.set_fuel(left),
        None => {
            store.set_fuel(0)?;
            Err(fuel_trap(import, bound, left))
        }
    }
}

/// Settles a [`precharge`] of `bound` once the work is known to cost `cost`: refunds the
/// difference, or charges the excess of a host that exceeded the bound.
fn settle(store: &mut StoreContextMut<'_, State>, bound: u64, cost: u64) -> wasmtime::Result<()> {
    match bound.checked_sub(cost) {
        Some(refund) => {
            let left = store.get_fuel()?;
            store.set_fuel(left.saturating_add(refund))
        }
        None => charge(store, cost - bound),
    }
}

/// Registers the `host` imports. Each runs under [`guarded`] on the lent host; the world
/// walks are pre-charged their bound at `rates` and settled after, the others charged after
/// ([`WorkRates`]). The store's fuel is reachable from a `func_wrap` closure, not from a
/// `bindgen` trait method.
fn add_host(
    linker: &mut Linker<State>,
    rates: WorkRates,
    support_max_region: u32,
) -> wasmtime::Result<()> {
    let mut host = linker.instance(HOST_INTERFACE)?;
    host.func_wrap(
        "apply",
        move |mut store: StoreContextMut<'_, State>, (op,): (Vec<u8>,)| {
            charge(&mut store, rates.bytes_cost(op.len()))?;
            let (answer, cost) = guarded(|| {
                let host = store.data_mut().host()?;
                let mut r = Reader::new(&op);
                let op = match decode_op(&mut r) {
                    Ok(op) if r.is_empty() => op,
                    _ => return Ok((vec![ANSWER_UNDECODABLE], rates.per_call)),
                };
                let result = host.apply(op.clone());
                Ok((encode_answer(&result), rates.apply_cost(&op, &result)))
            })?;
            charge(&mut store, cost)?;
            Ok((answer,))
        },
    )?;
    host.func_wrap(
        "material-at",
        move |mut store: StoreContextMut<'_, State>, (v,): (wit::Voxel,)| {
            let m = guarded(|| Ok(store.data_mut().host()?.material_at(voxel(v))))?;
            charge(&mut store, rates.per_call)?;
            Ok((m,))
        },
    )?;
    host.func_wrap(
        "raycast",
        move |mut store: StoreContextMut<'_, State>,
              (origin, dir, max): (wit::Fixed3, wit::Fixed3, i64)| {
            let bound = rates.ray_bound(fixed3(dir), max);
            precharge(&mut store, "raycast", bound)?;
            let (hit, steps) = guarded(|| {
                Ok(store
                    .data_mut()
                    .host()?
                    .raycast_with_steps(fixed3(origin), fixed3(dir), max))
            })?;
            settle(&mut store, bound, rates.steps_cost(steps))?;
            Ok((hit.map(|h| wit::Hit {
                voxel: wit_voxel(h.voxel),
                face: h.face.map(wit_face),
                t: h.t,
                material: h.material,
            }),))
        },
    )?;
    host.func_wrap(
        "count-in-box",
        move |mut store: StoreContextMut<'_, State>, (lo, hi): (wit::Voxel, wit::Voxel)| {
            let (lo, hi) = (voxel(lo), voxel(hi));
            let bound = rates.count_bound(lo, hi);
            precharge(&mut store, "count-in-box", bound)?;
            let counts = guarded(|| Ok(store.data_mut().host()?.count_in_box(lo, hi)))?;
            settle(&mut store, bound, rates.count_cost(&counts))?;
            Ok((counts,))
        },
    )?;
    host.func_wrap(
        "unsupported",
        move |mut store: StoreContextMut<'_, State>, (changed,): (Vec<wit::Cell>,)| {
            charge(
                &mut store,
                rates.bytes_cost(changed.len().saturating_mul(12)),
            )?;
            let changed: Vec<CellCoord> = changed
                .into_iter()
                .map(|c| CellCoord([c.x, c.y, c.z]))
                .collect();
            let bound = rates.support_bound(&changed, support_max_region);
            precharge(&mut store, "unsupported", bound)?;
            let (regions, steps) =
                guarded(|| Ok(store.data_mut().host()?.unsupported_with_steps(&changed)))?;
            settle(&mut store, bound, rates.steps_cost(steps))?;
            Ok((regions.into_iter().map(wit_region).collect::<Vec<_>>(),))
        },
    )?;
    Ok(())
}

/// The `wasi:clocks/monotonic-clock` instance `add_to_linker_sync` registers. A component
/// importing another 0.2 minor version resolves to it by semver; a wasmtime-wasi bump that
/// moves it must move this too, or the shadowing below defines a second instance and the
/// `timer` guest's test fails.
const MONOTONIC_CLOCK: &str = "wasi:clocks/monotonic-clock@0.2.12";

/// Replaces wasmtime-wasi's `subscribe-duration` and `subscribe-instant`, which wait on a
/// real-time deadline, with timers that are always ready (`wasi` module).
fn shadow_timers(linker: &mut Linker<State>) -> wasmtime::Result<()> {
    linker.allow_shadowing(true);
    let mut clock = linker.instance(MONOTONIC_CLOCK)?;
    clock.func_wrap(
        "subscribe-duration",
        |mut store: StoreContextMut<'_, State>, (_,): (u64,)| {
            Ok((ready_timer(&mut store.data_mut().table)?,))
        },
    )?;
    clock.func_wrap(
        "subscribe-instant",
        |mut store: StoreContextMut<'_, State>, (_,): (u64,)| {
            Ok((ready_timer(&mut store.data_mut().table)?,))
        },
    )?;
    linker.allow_shadowing(false);
    Ok(())
}

/// An instantiated component.
struct Live {
    store: Store<State>,
    logic: bindings::Logic,
    wasi: WasiHandles,
}

/// A logic component loaded from a file, implementing [`Logic`].
///
/// A failed call (`init` or `tick`: fuel, the memory cap, any other trap) leaves its tick
/// without a broadcast, and its error is kept for [`Logic::take_error`]. A trap leaves a
/// component instance's state undefined and wasmtime does not enter it again, so the
/// instance is dropped. The next `tick` instantiates the component afresh, runs its `init`
/// with the session's seed (at tick 0's clock, as at the start of the run) and then the
/// requested `tick`, and reports [`LogicError::Restarted`] through [`Logic::take_restart`];
/// a failure of that `init` is the tick's error, labelled with the tick. The restarted logic
/// believes the run has just begun while the world holds every edit since, and `init`'s ops
/// apply again on the world as it is. After [`WasmConfig::max_restarts`] restarts the logic
/// stops: every later `tick` returns [`LogicError::Stopped`] with the last failing tick
/// without entering wasmtime. Ops the logic applied through the host before a failure stay
/// applied. A new [`WasmLogic::try_init`] starts afresh with the restart count at 0.
pub struct WasmLogic {
    pre: bindings::LogicPre<State>,
    config: WasmConfig,
    seed: u64,
    live: Option<Live>,
    error: Option<LogicError>,
    /// The tick whose call last failed (0 for the first `init`); set, the next tick restarts
    /// the component or, once `restarts` reaches the cap, the logic is stopped.
    failed_at: Option<u64>,
    /// Fresh instances started since the last [`WasmLogic::try_init`].
    restarts: u32,
    /// The restart the last `tick` made, for [`Logic::take_restart`].
    restart: Option<LogicError>,
}

impl std::fmt::Debug for WasmLogic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmLogic")
            .field("config", &self.config)
            .field("seed", &self.seed)
            .field("live", &self.live.is_some())
            .field("error", &self.error)
            .field("failed_at", &self.failed_at)
            .field("restarts", &self.restarts)
            .finish_non_exhaustive()
    }
}

impl WasmLogic {
    /// Compiles and links the component at `path`. Fails with [`LogicError::Load`] when the
    /// file cannot be read, is not a component, or does not match the `logic` world.
    pub fn load(path: &Path, config: &WasmConfig) -> Result<Self, LogicError> {
        let load = |e: wasmtime::Error| LogicError::Load(format!("{}: {e:#}", path.display()));
        let mut cfg = wasmtime::Config::new();
        cfg.wasm_component_model(true)
            .consume_fuel(true)
            .cranelift_nan_canonicalization(true)
            .relaxed_simd_deterministic(true);
        let engine = Engine::new(&cfg).map_err(load)?;
        let component = Component::from_file(&engine, path).map_err(load)?;
        let mut linker = Linker::<State>::new(&engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(load)?;
        shadow_timers(&mut linker).map_err(load)?;
        add_host(&mut linker, config.rates, config.support_max_region).map_err(load)?;
        let pre = linker.instantiate_pre(&component).map_err(load)?;
        let pre = bindings::LogicPre::new(pre).map_err(load)?;
        Ok(Self {
            pre,
            config: *config,
            seed: 0,
            live: None,
            error: None,
            failed_at: None,
            restarts: 0,
            restart: None,
        })
    }

    /// Instantiates the component for `seed` and calls its `init` with `host` lent, with the
    /// restart count at 0. After a failure the next tick restarts the component (with
    /// `max_restarts` at 0, every later tick returns [`LogicError::Stopped`] with `since: 0`).
    pub fn try_init(&mut self, host: &mut dyn Host, seed: u64) -> Result<(), LogicError> {
        self.seed = seed;
        self.live = None;
        self.failed_at = None;
        self.restarts = 0;
        self.restart = None;
        let result = self.instantiate_and_init(host, seed);
        if result.is_err() {
            self.live = None;
            self.failed_at = Some(0);
        }
        result
    }

    fn instantiate_and_init(&mut self, host: &mut dyn Host, seed: u64) -> Result<(), LogicError> {
        let (wasi, handles) = context(seed, self.config.frame_ns);
        let memory_bytes = usize::try_from(self.config.memory_bytes).unwrap_or(usize::MAX);
        let mut store = Store::new(
            self.pre.engine(),
            State {
                wasi,
                table: ResourceTable::new(),
                host: Lent(None),
                limits: Limits {
                    memory_bytes,
                    total: 0,
                    hit: false,
                },
            },
        );
        store.limiter(|s| &mut s.limits);
        store
            .set_fuel(self.config.fuel_per_tick)
            .map_err(|e| LogicError::Load(format!("{e:#}")))?;
        let logic = match self.pre.instantiate(&mut store) {
            Ok(l) => l,
            Err(e) => return Err(classify(&store, &handles, 0, e)),
        };
        self.live = Some(Live {
            store,
            logic,
            wasi: handles,
        });
        self.call(host, 0, |store, logic| logic.call_init(store, seed))
    }

    /// Calls the component's `tick` with `host` lent. After a failed call it first restarts
    /// the component (a fresh instance and `init`, see [`WasmLogic`]), recorded for
    /// [`WasmLogic::take_restart`]; once `max_restarts` restarts are spent it returns
    /// [`LogicError::Stopped`] without entering the component.
    pub fn try_tick(
        &mut self,
        host: &mut dyn Host,
        tick: u64,
        inputs: &[ClientInput],
    ) -> Result<TickOutput, LogicError> {
        self.restart = None;
        if let Some(since) = self.failed_at {
            if self.restarts >= self.config.max_restarts {
                return Err(LogicError::Stopped { tick, since });
            }
            self.restarts += 1;
            self.restart = Some(LogicError::Restarted {
                tick,
                after: since,
                restarts: self.restarts,
            });
            self.failed_at = None;
            if let Err(e) = self.instantiate_and_init(host, self.seed) {
                self.live = None;
                self.failed_at = Some(tick);
                return Err(relabel(e, tick));
            }
        }
        let inputs: Vec<bindings::ClientInput> = inputs
            .iter()
            .map(|i| bindings::ClientInput {
                client: i.client,
                bytes: i.bytes.clone(),
            })
            .collect();
        let result = self.call(host, tick, |store, logic| {
            logic.call_tick(store, tick, &inputs)
        });
        match result {
            Ok(broadcast) => Ok(TickOutput { broadcast }),
            Err(e) => {
                self.live = None;
                self.failed_at = Some(tick);
                Err(e)
            }
        }
    }

    /// The fuel left in the live instance after its last call (`init` or `tick`), or `None`
    /// when no instance is live (never initialised, or dropped after a failure). Fuel is
    /// reset to `fuel_per_tick` before each call, so `fuel_per_tick` minus this is what the
    /// last call used: the guest's instructions and the host imports' charges.
    pub fn fuel_left(&self) -> Option<u64> {
        self.live.as_ref().and_then(|l| l.store.get_fuel().ok())
    }

    /// The restart the last [`WasmLogic::try_tick`] made, if it made one, cleared by reading
    /// it.
    pub fn take_restart(&mut self) -> Option<LogicError> {
        self.restart.take()
    }

    /// Runs `f` on the live instance at `tick`'s clock and random stream, with fresh fuel and
    /// `host` lent for its duration.
    fn call<R>(
        &mut self,
        host: &mut dyn Host,
        tick: u64,
        f: impl FnOnce(&mut Store<State>, &bindings::Logic) -> wasmtime::Result<R>,
    ) -> Result<R, LogicError> {
        let config = self.config;
        let seed = self.seed;
        let Some(live) = self.live.as_mut() else {
            return Err(LogicError::Trap {
                tick,
                message: "the component is not instantiated (init never ran)".into(),
            });
        };
        live.wasi.clock.set(tick.wrapping_mul(config.frame_ns));
        live.wasi.random.restart(stream_state(seed, tick));
        live.store.data_mut().limits.hit = false;
        live.store
            .set_fuel(config.fuel_per_tick)
            .map_err(|e| LogicError::Load(format!("{e:#}")))?;
        live.wasi.stderr.clear();
        // SAFETY: only the trait object's lifetime bound is erased. The pointer is cleared
        // below, before `host`'s borrow ends, on both the normal and the unwinding path, and
        // `State::host` is the only reader.
        let ptr = unsafe {
            std::mem::transmute::<NonNull<dyn Host + '_>, NonNull<dyn Host + 'static>>(
                NonNull::from(host),
            )
        };
        live.store.data_mut().host = Lent(Some(ptr));
        // A panic in a host import is already a trap (`guarded`); this catches any other
        // panic only to clear the lent pointer before resuming it.
        let result = catch_unwind(AssertUnwindSafe(|| f(&mut live.store, &live.logic)));
        live.store.data_mut().host = Lent(None);
        match result {
            Ok(Ok(r)) => Ok(r),
            Ok(Err(e)) => Err(classify(&live.store, &live.wasi, tick, e)),
            Err(panic) => resume_unwind(panic),
        }
    }
}

/// The [`LogicError`] for a failed call at `tick`: the memory cap when the limiter refused a
/// growth during it, fuel when it ran out, otherwise a trap with its message and what the
/// component wrote to stderr during the call (a panic's message; the stderr the host keeps
/// holds only the current call's writes).
fn classify(store: &Store<State>, wasi: &WasiHandles, tick: u64, e: wasmtime::Error) -> LogicError {
    if store.data().limits.hit {
        return LogicError::Memory { tick };
    }
    if e.downcast_ref::<Trap>() == Some(&Trap::OutOfFuel) {
        return LogicError::Fuel { tick };
    }
    let stderr = wasi.stderr.contents();
    let written = String::from_utf8_lossy(&stderr);
    let written = written.trim();
    let message = if written.is_empty() {
        format!("{e:#}")
    } else {
        format!("{written}\n{e:#}")
    };
    LogicError::Trap { tick, message }
}

/// `e` with its tick replaced by `tick`: an `init` run on a restart runs at tick 0's clock
/// and is labelled with the tick that restarted it.
fn relabel(e: LogicError, tick: u64) -> LogicError {
    match e {
        LogicError::Trap { message, .. } => LogicError::Trap { tick, message },
        LogicError::Fuel { .. } => LogicError::Fuel { tick },
        LogicError::Memory { .. } => LogicError::Memory { tick },
        other => other,
    }
}

impl Logic for WasmLogic {
    fn init(&mut self, host: &mut dyn Host, seed: u64) {
        if let Err(e) = self.try_init(host, seed) {
            self.error = Some(e);
        }
    }

    fn tick(&mut self, host: &mut dyn Host, tick: u64, inputs: &[ClientInput]) -> TickOutput {
        self.try_tick(host, tick, inputs).unwrap_or_else(|e| {
            self.error = Some(e);
            TickOutput::default()
        })
    }

    fn take_error(&mut self) -> Option<LogicError> {
        self.error.take()
    }

    fn take_restart(&mut self) -> Option<LogicError> {
        WasmLogic::take_restart(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pre-charge past the fuel left is an out-of-fuel trap, so [`classify`] reads it as
    /// `LogicError::Fuel`, and its message names the import.
    #[test]
    fn a_bound_past_the_fuel_left_is_a_fuel_trap_naming_the_import() {
        let e = fuel_trap("count-in-box", 10_737_418_340, 9_000);
        assert_eq!(e.downcast_ref::<Trap>(), Some(&Trap::OutOfFuel));
        let message = format!("{e:#}");
        assert!(message.contains("`count-in-box`"), "{message}");
        assert!(message.contains("10737418340"), "{message}");
    }

    /// The bounds: a box's volume, a ray's plane crossings, the support query's floods.
    #[test]
    fn the_bounds_cover_the_work_they_stand_for() {
        let r = WorkRates::DEFAULT;
        let v = |c: [i32; 3]| VoxelCoord(c);
        assert_eq!(r.count_bound(v([0; 3]), v([63; 3])), 100 + 10 * (1 << 18));
        assert_eq!(r.count_bound(v([0; 3]), v([1023; 3])), 100 + 10 * (1 << 30));
        assert_eq!(r.count_bound(v([5, 0, 0]), v([0, 9, 9])), 100);
        // 2^22 fixed units along x are 16 384 voxels: 16 385 planes, 1 on y and z, 1 more.
        assert_eq!(
            r.ray_bound(Fixed3([256, 0, 0]), 1 << 22),
            100 + 10 * (16_385 + 1 + 1 + 1)
        );
        assert_eq!(r.ray_bound(Fixed3([256, 0, 0]), -5), 100 + 10 * 4);
        assert_eq!(r.ray_bound(Fixed3([i64::MIN; 3]), i64::MAX), u64::MAX);
        let c = |x| CellCoord([x, 0, 0]);
        assert_eq!(
            r.support_bound(&[c(1), c(2), c(1)], 4096),
            100 + 10 * 2 * 7 * 4098
        );
        assert_eq!(r.support_bound(&[], 4096), 100);
    }

    /// The cap holds across memories: a second memory cannot take what the first left.
    #[test]
    fn the_memory_cap_is_on_all_memories_together() {
        const MIB: usize = 1 << 20;
        let mut limits = Limits {
            memory_bytes: 64 * MIB,
            total: 0,
            hit: false,
        };
        assert!(limits.memory_growing(0, 40 * MIB, None).unwrap());
        assert!(!limits.hit);
        assert!(limits.memory_growing(0, 40 * MIB, None).is_err());
        assert!(limits.hit);
        assert_eq!(limits.total, 40 * MIB);
    }
}
