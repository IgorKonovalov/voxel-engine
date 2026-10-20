//! A WebAssembly component host for the game's logic (ADR-0010).
//!
//! A logic crate implements [`vxe_net::Logic`] once. Built natively it is linked behind the
//! trait; built for `wasm32-wasip2` with this crate's `guest` feature it exports the WIT world
//! in `wit/logic.wit` through [`export_logic!`], and [`WasmLogic`] (the `host` feature, on by
//! default) loads that component and implements [`vxe_net::Logic`] for it. The two builds of
//! one logic, driven by the same inputs, request the same ops.
//!
//! Compound values cross the boundary as [`vxe_world::wire`] bytes: an op of any kind goes
//! out to the host's `apply`, and the answer comes back as a tag byte and the wire bytes of
//! the op result or the error ([`encode_answer`], [`decode_answer`]). Scalars, and the support
//! query's cells and regions, cross typed.
//!
//! The component observes nothing but its seed and its inputs (`wasi` module): its clocks
//! read `tick × frame_ns`, its random streams restart from the seed and the tick, it has no
//! filesystem, environment or network, its NaNs are canonical, and each call runs on a fuel
//! budget under a memory cap ([`WasmConfig`]).

#[cfg(feature = "guest")]
pub mod guest;
#[cfg(feature = "host")]
mod host;
#[cfg(feature = "host")]
mod wasi;

#[cfg(feature = "host")]
pub use host::{WasmConfig, WasmLogic};
pub use vxe_net::LogicError;

use vxe_world::wire::{
    Reader, WireError, decode_error, decode_op_result, encode_error, encode_op_result,
};
use vxe_world::{EditError, OpResult};

/// Tag of an `apply` answer that carries an [`OpResult`].
pub const ANSWER_APPLIED: u8 = 0;
/// Tag of an `apply` answer that carries an [`EditError`].
pub const ANSWER_REFUSED: u8 = 1;
/// Tag of an `apply` answer to op bytes that did not decode; nothing follows it.
pub const ANSWER_UNDECODABLE: u8 = 2;

/// The bytes the host's `apply` returns for `outcome`.
pub fn encode_answer(outcome: &Result<OpResult, EditError>) -> Vec<u8> {
    let mut out = Vec::new();
    match outcome {
        Ok(r) => {
            out.push(ANSWER_APPLIED);
            encode_op_result(&mut out, r);
        }
        Err(e) => {
            out.push(ANSWER_REFUSED);
            encode_error(&mut out, e);
        }
    }
    out
}

/// Reads an `apply` answer. `Ok(None)` is [`ANSWER_UNDECODABLE`]; bytes after the answer are
/// [`WireError::Trailing`].
pub fn decode_answer(bytes: &[u8]) -> Result<Option<Result<OpResult, EditError>>, WireError> {
    let mut r = Reader::new(bytes);
    let answer = match r.u8()? {
        ANSWER_APPLIED => Some(Ok(decode_op_result(&mut r)?)),
        ANSWER_REFUSED => Some(Err(decode_error(&mut r)?)),
        ANSWER_UNDECODABLE => None,
        k => return Err(WireError::UnknownKind(k)),
    };
    if !r.is_empty() {
        return Err(WireError::Trailing { at: r.pos() });
    }
    Ok(answer)
}
