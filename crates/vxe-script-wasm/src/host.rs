//! [`WasmLogic`]: a logic component behind [`vxe_net::Logic`].
//!
//! The engine compiles the component once, at [`WasmLogic::load`]. [`Logic::init`] makes the
//! store, with a WASI context built from the seed (`wasi` module), instantiates the component
//! and calls its `init`; [`Logic::tick`] calls its `tick`. During each of those calls the
//! server's [`Host`] is lent to the store, and the component's `host` imports answer through
//! it; outside them an import traps.

use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::path::Path;
use std::ptr::NonNull;

use vxe_net::{ClientInput, Host, Logic, LogicError, TickOutput};
use vxe_world::wire::{Reader, decode_op};
use vxe_world::{Face, Fixed3, Op, VoxelCoord};
use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Engine, ResourceLimiter, Store, Trap};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

use crate::wasi::{WasiHandles, context, stream_state};
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
    /// The cap on each of the component's linear memories, in bytes.
    pub memory_bytes: u64,
    /// Nanoseconds of the component's clocks per tick.
    pub frame_ns: u64,
}

impl Default for WasmConfig {
    fn default() -> Self {
        Self {
            fuel_per_tick: 10_000_000_000,
            memory_bytes: 64 << 20,
            frame_ns: 16_666_667,
        }
    }
}

/// The server's host, lent to the store for one call.
struct Lent(Option<NonNull<dyn Host>>);

// SAFETY: the pointer is set only for the duration of one `WasmLogic::call`, on the thread
// that holds `&mut WasmLogic` and the `&mut dyn Host` it came from; nothing reads it from
// another thread, and it is cleared before that borrow ends.
unsafe impl Send for Lent {}

/// The cap on memory growth; records whether the component hit it.
struct Limits {
    memory_bytes: usize,
    hit: bool,
}

/// Table elements a component may grow a table to.
const TABLE_ELEMENTS: usize = 1 << 16;

impl ResourceLimiter for Limits {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        if desired > self.memory_bytes {
            self.hit = true;
            wasmtime::bail!(
                "memory growth to {desired} bytes exceeds the cap of {} bytes",
                self.memory_bytes
            );
        }
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

impl wit::Host for State {
    fn apply(&mut self, op: Vec<u8>) -> wasmtime::Result<Vec<u8>> {
        let host = self.host()?;
        let mut r = Reader::new(&op);
        let op = match decode_op(&mut r) {
            Ok(op) if r.is_empty() => op,
            _ => return Ok(vec![ANSWER_UNDECODABLE]),
        };
        let Op::Edit(op) = op;
        Ok(encode_answer(&host.apply(op)))
    }

    fn material_at(&mut self, v: wit::Voxel) -> wasmtime::Result<Option<u16>> {
        Ok(self.host()?.material_at(voxel(v)))
    }

    fn raycast(
        &mut self,
        origin: wit::Fixed3,
        dir: wit::Fixed3,
        max: i64,
    ) -> wasmtime::Result<Option<wit::Hit>> {
        let hit = self.host()?.raycast(fixed3(origin), fixed3(dir), max);
        Ok(hit.map(|h| wit::Hit {
            voxel: wit_voxel(h.voxel),
            face: h.face.map(wit_face),
            t: h.t,
            material: h.material,
        }))
    }

    fn count_in_box(
        &mut self,
        lo: wit::Voxel,
        hi: wit::Voxel,
    ) -> wasmtime::Result<Vec<(u16, u64)>> {
        Ok(self.host()?.count_in_box(voxel(lo), voxel(hi)))
    }
}

/// An instantiated component.
struct Live {
    store: Store<State>,
    logic: bindings::Logic,
    wasi: WasiHandles,
}

/// A logic component loaded from a file, implementing [`Logic`].
///
/// A failed call leaves the tick without a broadcast; the error is kept for
/// [`Logic::take_error`]. wasmtime does not enter a component instance again after it
/// trapped, so every call after a failed one fails with [`LogicError::Trap`].
pub struct WasmLogic {
    pre: bindings::LogicPre<State>,
    config: WasmConfig,
    seed: u64,
    live: Option<Live>,
    error: Option<LogicError>,
}

impl std::fmt::Debug for WasmLogic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmLogic")
            .field("config", &self.config)
            .field("seed", &self.seed)
            .field("live", &self.live.is_some())
            .field("error", &self.error)
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
        bindings::Logic::add_to_linker::<State, wasmtime::component::HasSelf<State>>(
            &mut linker,
            |s| s,
        )
        .map_err(load)?;
        let pre = linker.instantiate_pre(&component).map_err(load)?;
        let pre = bindings::LogicPre::new(pre).map_err(load)?;
        Ok(Self {
            pre,
            config: *config,
            seed: 0,
            live: None,
            error: None,
        })
    }

    /// Instantiates the component for `seed` and calls its `init` with `host` lent.
    pub fn try_init(&mut self, host: &mut dyn Host, seed: u64) -> Result<(), LogicError> {
        self.seed = seed;
        self.live = None;
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
            Err(e) => return Err(classify(&store, &handles, 0, 0, e)),
        };
        self.live = Some(Live {
            store,
            logic,
            wasi: handles,
        });
        self.call(host, 0, |store, logic| logic.call_init(store, seed))
    }

    /// Calls the component's `tick` with `host` lent.
    pub fn try_tick(
        &mut self,
        host: &mut dyn Host,
        tick: u64,
        inputs: &[ClientInput],
    ) -> Result<TickOutput, LogicError> {
        let inputs: Vec<bindings::ClientInput> = inputs
            .iter()
            .map(|i| bindings::ClientInput {
                client: i.client,
                bytes: i.bytes.clone(),
            })
            .collect();
        let broadcast = self.call(host, tick, |store, logic| {
            logic.call_tick(store, tick, &inputs)
        })?;
        Ok(TickOutput { broadcast })
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
                message: "the component is not instantiated (init failed or never ran)".into(),
            });
        };
        live.wasi.clock.set(tick.wrapping_mul(config.frame_ns));
        live.wasi.random.restart(stream_state(seed, tick));
        live.store.data_mut().limits.hit = false;
        live.store
            .set_fuel(config.fuel_per_tick)
            .map_err(|e| LogicError::Load(format!("{e:#}")))?;
        let stderr_before = live.wasi.stderr.contents().len();
        // SAFETY: only the trait object's lifetime bound is erased. The pointer is cleared
        // below, before `host`'s borrow ends, on both the normal and the unwinding path, and
        // `State::host` is the only reader.
        let ptr = unsafe {
            std::mem::transmute::<NonNull<dyn Host + '_>, NonNull<dyn Host + 'static>>(
                NonNull::from(host),
            )
        };
        live.store.data_mut().host = Lent(Some(ptr));
        let result = catch_unwind(AssertUnwindSafe(|| f(&mut live.store, &live.logic)));
        live.store.data_mut().host = Lent(None);
        match result {
            Ok(Ok(r)) => Ok(r),
            Ok(Err(e)) => Err(classify(&live.store, &live.wasi, tick, stderr_before, e)),
            Err(panic) => resume_unwind(panic),
        }
    }
}

/// The [`LogicError`] for a failed call at `tick`: the memory cap when the limiter refused a
/// growth during it, fuel when it ran out, otherwise a trap with its message and what the
/// component wrote to stderr during the call (a panic's message).
fn classify(
    store: &Store<State>,
    wasi: &WasiHandles,
    tick: u64,
    stderr_before: usize,
    e: wasmtime::Error,
) -> LogicError {
    if store.data().limits.hit {
        return LogicError::Memory { tick };
    }
    if e.downcast_ref::<Trap>() == Some(&Trap::OutOfFuel) {
        return LogicError::Fuel { tick };
    }
    let stderr = wasi.stderr.contents();
    let written = String::from_utf8_lossy(stderr.get(stderr_before..).unwrap_or_default());
    let written = written.trim();
    let message = if written.is_empty() {
        format!("{e:#}")
    } else {
        format!("{written}\n{e:#}")
    };
    LogicError::Trap { tick, message }
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
}
