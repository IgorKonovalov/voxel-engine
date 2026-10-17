//! The fly-through's server logic: digs, fills and the bench as inputs, applied through the
//! host (ADR-0010). It depends on `vxe-net`'s traits and `vxe-world` alone, nothing GPU, so
//! the same crate builds natively (the fly-through links it) and, with the `component`
//! feature for `wasm32-wasip2`, as a logic component that exports [`DigLogic`]
//! (`scripts/build-guests.sh` places it at `target/guests/flythrough-logic.wasm`).
//!
//! An input is a tag `u8` then little-endian fields, `Fixed3` as 3 × `i64`:
//!
//! | input   | tag | fields                                                       | bytes |
//! |---------|-----|--------------------------------------------------------------|-------|
//! | `Dig`   | 1   | origin `Fixed3`, dir 3 × `i64`, radius `i64`                 | 57    |
//! | `Fill`  | 2   | origin `Fixed3`, dir 3 × `i64`, radius `i64`, material `u16` | 59    |
//! | `Bench` | 3   | origin voxel 3 × `i64`, seed `u64`                           | 33    |
//!
//! The broadcast is UTF-8 lines joined by `\n`. A line `op <verb>` stands for the tick's next
//! op, applied or refused, in the order the client receives their announcements (`carved`
//! for a dig, `filled` for a fill, `bench` for each of a bench's [`BENCH_CARVES`] carves);
//! any other line is a message to print as it is.

use vxe_net::{ClientInput, Host, Logic, TickOutput};
use vxe_world::hash::mix64;
use vxe_world::wire::{Reader, WireError, put_fixed3};
use vxe_world::{EditOp, FIXED_ONE, Fixed3, MaterialId, Mode};

/// Reach of the dig and fill rays: 10 m.
pub const REACH: i64 = 100 * FIXED_ONE;
/// The bench: this many radius-1 m carves, centred within 10 m (100 voxels) of its origin.
pub const BENCH_CARVES: usize = 100;
pub const BENCH_RADIUS: i64 = 10 * FIXED_ONE;
pub const BENCH_SPREAD: i64 = 100;

/// The verb of a dig's `op` line.
pub const DIG_VERB: &str = "carved";
/// The verb of a fill's `op` line.
pub const FILL_VERB: &str = "filled";
/// The verb of a bench carve's `op` line.
pub const BENCH_VERB: &str = "bench";

/// Input tags.
pub const INPUT_DIG: u8 = 1;
pub const INPUT_FILL: u8 = 2;
pub const INPUT_BENCH: u8 = 3;

/// One input to [`DigLogic`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DigInput {
    /// Carve a capsule of `radius` from `origin` to where the ray `origin + dir * t / 256`
    /// first hits, or to its end at [`REACH`].
    Dig {
        origin: Fixed3,
        dir: [i64; 3],
        radius: i64,
    },
    /// Fill a sphere of `radius` with `material` where the ray first hits, if it hits within
    /// [`REACH`].
    Fill {
        origin: Fixed3,
        dir: [i64; 3],
        radius: i64,
        material: MaterialId,
    },
    /// [`BENCH_CARVES`] carves at positions drawn from `seed` within [`BENCH_SPREAD`] voxels
    /// of the voxel `origin`.
    Bench { origin: [i64; 3], seed: u64 },
}

impl DigInput {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(59);
        match *self {
            Self::Dig {
                origin,
                dir,
                radius,
            } => {
                out.push(INPUT_DIG);
                put_fixed3(&mut out, origin);
                put_fixed3(&mut out, Fixed3(dir));
                out.extend_from_slice(&radius.to_le_bytes());
            }
            Self::Fill {
                origin,
                dir,
                radius,
                material,
            } => {
                out.push(INPUT_FILL);
                put_fixed3(&mut out, origin);
                put_fixed3(&mut out, Fixed3(dir));
                out.extend_from_slice(&radius.to_le_bytes());
                out.extend_from_slice(&material.to_le_bytes());
            }
            Self::Bench { origin, seed } => {
                out.push(INPUT_BENCH);
                put_fixed3(&mut out, Fixed3(origin));
                out.extend_from_slice(&seed.to_le_bytes());
            }
        }
        out
    }

    /// Decodes one input; bytes after it are [`WireError::Trailing`].
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut r = Reader::new(bytes);
        let input = match r.u8()? {
            INPUT_DIG => Self::Dig {
                origin: r.fixed3()?,
                dir: r.fixed3()?.0,
                radius: r.i64()?,
            },
            INPUT_FILL => Self::Fill {
                origin: r.fixed3()?,
                dir: r.fixed3()?.0,
                radius: r.i64()?,
                material: r.u16()?,
            },
            INPUT_BENCH => Self::Bench {
                origin: r.fixed3()?.0,
                seed: r.u64()?,
            },
            k => return Err(WireError::UnknownKind(k)),
        };
        if !r.is_empty() {
            return Err(WireError::Trailing { at: r.pos() });
        }
        Ok(input)
    }
}

/// The bench's carves: centres drawn from `mix64(seed)` within [`BENCH_SPREAD`] voxels of
/// the voxel `origin` (a ball, by rejection), each at its voxel's centre.
pub fn bench_ops(origin: [i64; 3], seed: u64) -> Vec<EditOp> {
    let mut state = mix64(seed);
    let mut next = || {
        state = mix64(state.wrapping_add(1));
        (state % (2 * BENCH_SPREAD as u64 + 1)) as i64 - BENCH_SPREAD
    };
    let mut ops = Vec::with_capacity(BENCH_CARVES);
    while ops.len() < BENCH_CARVES {
        let d = [next(), next(), next()];
        if d.iter().map(|v| v * v).sum::<i64>() <= BENCH_SPREAD * BENCH_SPREAD {
            let c = [0, 1, 2].map(|k| (origin[k] + d[k]) * FIXED_ONE + FIXED_ONE / 2);
            ops.push(EditOp::sphere(Fixed3(c), BENCH_RADIUS, Mode::Carve));
        }
    }
    ops
}

/// `op ` then [`BENCH_VERB`].
const BENCH_OP_LINE: &str = "op bench";

/// The point `origin + dir * t / 256`.
fn along(origin: Fixed3, dir: [i64; 3], t: i64) -> Fixed3 {
    Fixed3([0, 1, 2].map(|k| origin.0[k] + dir[k] * t / FIXED_ONE))
}

/// The fly-through's logic: each input is one [`DigInput`]; undecodable inputs are ignored.
/// It holds no state, so re-simulation needs only the inputs and the submitted ops.
#[derive(Clone, Copy, Debug, Default)]
pub struct DigLogic;

impl Logic for DigLogic {
    fn init(&mut self, _host: &mut dyn Host, _seed: u64) {}

    fn tick(&mut self, host: &mut dyn Host, _tick: u64, inputs: &[ClientInput]) -> TickOutput {
        let mut lines: Vec<&str> = Vec::new();
        for input in inputs {
            match DigInput::decode(&input.bytes) {
                Ok(DigInput::Dig {
                    origin,
                    dir,
                    radius,
                }) => {
                    let t = host
                        .raycast(origin, Fixed3(dir), REACH)
                        .map_or(REACH, |h| h.t);
                    let end = along(origin, dir, t);
                    let _ = host.apply(EditOp::capsule(origin, end, radius, Mode::Carve));
                    lines.push("op carved");
                }
                Ok(DigInput::Fill {
                    origin,
                    dir,
                    radius,
                    material,
                }) => match host.raycast(origin, Fixed3(dir), REACH) {
                    Some(hit) => {
                        let at = along(origin, dir, hit.t);
                        let _ = host.apply(EditOp::sphere(at, radius, Mode::Fill(material)));
                        lines.push("op filled");
                    }
                    None => lines.push("filled: no surface within 10 m"),
                },
                Ok(DigInput::Bench { origin, seed }) => {
                    for op in bench_ops(origin, seed) {
                        let _ = host.apply(op);
                        lines.push(BENCH_OP_LINE);
                    }
                }
                Err(_) => {}
            }
        }
        TickOutput {
            broadcast: lines.join("\n").into_bytes(),
        }
    }
}

#[cfg(feature = "component")]
vxe_script_wasm::export_logic!(DigLogic);
