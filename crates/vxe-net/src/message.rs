//! Messages between server and clients, as bytes (ADR-0007).
//!
//! Each message is a tag `u8`, then its fields, little-endian, with ops, results and errors
//! in `vxe_world::wire`'s encoding:
//!
//! | tag | message   | fields                                                    |
//! |-----|-----------|-----------------------------------------------------------|
//! | 1   | `Applied` | tick `u64`, seq `u32`, op, result                         |
//! | 2   | `Refused` | tick `u64`, seq `u32`, op, error                          |
//! | 3   | `Event`   | tick `u64`, len `u32`, `len` bytes                        |
//! | 4   | `Input`   | len `u16`, `len` bytes                                    |
//!
//! An event's length is a `u32` because a logic's broadcast for one tick has no bound the
//! engine sets; an input's is a `u16`, the input log's entry length.

use vxe_world::wire::{
    Reader, WireError, decode_error, decode_op, decode_result, encode_error, encode_op,
    encode_result,
};
use vxe_world::{EditError, EditResult, Op};

/// Tag of [`Message::Applied`].
pub const MSG_APPLIED: u8 = 1;
/// Tag of [`Message::Refused`].
pub const MSG_REFUSED: u8 = 2;
/// Tag of [`Message::Event`].
pub const MSG_EVENT: u8 = 3;
/// Tag of [`Message::Input`].
pub const MSG_INPUT: u8 = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    /// Server to client: the op with this seq was applied in this tick and changed this.
    Applied {
        tick: u64,
        seq: u32,
        op: Op,
        result: EditResult,
    },
    /// Server to client: the op with this seq was refused in this tick and changed nothing.
    Refused {
        tick: u64,
        seq: u32,
        op: Op,
        error: EditError,
    },
    /// Server to client: the logic's broadcast for this tick, opaque to the engine.
    Event { tick: u64, bytes: Vec<u8> },
    /// Client to server: one input for the logic, opaque to the engine.
    Input { bytes: Vec<u8> },
}

impl Message {
    /// The message's bytes. Panics when an `Input` payload exceeds `u16::MAX` bytes or an
    /// `Event` payload exceeds `u32::MAX` bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        match self {
            Self::Applied {
                tick,
                seq,
                op,
                result,
            } => {
                out.push(MSG_APPLIED);
                out.extend_from_slice(&tick.to_le_bytes());
                out.extend_from_slice(&seq.to_le_bytes());
                encode_op(&mut out, op);
                encode_result(&mut out, result);
            }
            Self::Refused {
                tick,
                seq,
                op,
                error,
            } => {
                out.push(MSG_REFUSED);
                out.extend_from_slice(&tick.to_le_bytes());
                out.extend_from_slice(&seq.to_le_bytes());
                encode_op(&mut out, op);
                encode_error(&mut out, error);
            }
            Self::Event { tick, bytes } => {
                let len = u32::try_from(bytes.len()).expect("event payloads fit u32");
                out.push(MSG_EVENT);
                out.extend_from_slice(&tick.to_le_bytes());
                out.extend_from_slice(&len.to_le_bytes());
                out.extend_from_slice(bytes);
            }
            Self::Input { bytes } => {
                let len = u16::try_from(bytes.len()).expect("input payloads fit u16");
                out.push(MSG_INPUT);
                out.extend_from_slice(&len.to_le_bytes());
                out.extend_from_slice(bytes);
            }
        }
        out
    }

    /// Reads one whole message. Bytes after its end are [`WireError::Trailing`].
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut r = Reader::new(bytes);
        let m = match r.u8()? {
            MSG_APPLIED => Self::Applied {
                tick: r.u64()?,
                seq: r.u32()?,
                op: decode_op(&mut r)?,
                result: decode_result(&mut r)?,
            },
            MSG_REFUSED => Self::Refused {
                tick: r.u64()?,
                seq: r.u32()?,
                op: decode_op(&mut r)?,
                error: decode_error(&mut r)?,
            },
            MSG_EVENT => {
                let tick = r.u64()?;
                let len = r.u32()?;
                let len = usize::try_from(len).map_err(|_| WireError::Truncated { at: 0 })?;
                Self::Event {
                    tick,
                    bytes: r.take(len)?.to_vec(),
                }
            }
            MSG_INPUT => {
                let len = r.u16()?;
                Self::Input {
                    bytes: r.take(usize::from(len))?.to_vec(),
                }
            }
            k => return Err(WireError::UnknownKind(k)),
        };
        if !r.is_empty() {
            return Err(WireError::Trailing { at: r.pos() });
        }
        Ok(m)
    }
}
