//! A logic that asks the host for as much work as an input names, so the host's charge for
//! that work can be read. Each input is a command byte and its little-endian payload:
//!
//! - `1`: `count_in_box` over the inclusive box `lo..=hi`, 6 × `i32`; broadcasts the voxels
//!   counted as a `u64`.
//! - `2`: `n` (`u32`) rays `origin + dir * t / 256`, `t <= max`, origin and dir 3 × `i64`
//!   each, max `i64`; broadcasts the hits as a `u32`.
//! - `3`: the `apply` import called `n` (`u32`) times with the rest of the input as the op
//!   bytes, as given; broadcasts the last answer's tag byte.
//! - `4`: the `apply` import called `n` (`u32`) times with one buffer of `len` (`u32`) bytes
//!   of `0xFF`, which decodes as no op; broadcasts the calls made as a `u32`.
//! - `5`: `unsupported` over the rest of the input's cells, 3 × `i32` each; broadcasts the
//!   regions found as a `u32`.

use vxe_net::{ClientInput, Host, Logic, TickOutput};
use vxe_script_wasm::guest::bindings::vxe::logic::host as wit;
use vxe_world::{CellCoord, Fixed3, VoxelCoord};

#[derive(Default)]
struct Greedy;

/// Little-endian fields read in order; a short input reads as zeros.
struct Fields<'a>(&'a [u8]);

impl Fields<'_> {
    fn take<const N: usize>(&mut self) -> [u8; N] {
        let mut out = [0; N];
        let n = N.min(self.0.len());
        out[..n].copy_from_slice(&self.0[..n]);
        self.0 = &self.0[n..];
        out
    }
    fn i32(&mut self) -> i32 {
        i32::from_le_bytes(self.take())
    }
    fn i64(&mut self) -> i64 {
        i64::from_le_bytes(self.take())
    }
    fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.take())
    }
    fn voxel(&mut self) -> VoxelCoord {
        VoxelCoord([self.i32(), self.i32(), self.i32()])
    }
    fn fixed3(&mut self) -> Fixed3 {
        Fixed3([self.i64(), self.i64(), self.i64()])
    }
}

impl Logic for Greedy {
    fn init(&mut self, _host: &mut dyn Host, _seed: u64) {}

    fn tick(&mut self, host: &mut dyn Host, _tick: u64, inputs: &[ClientInput]) -> TickOutput {
        let mut broadcast = Vec::new();
        for input in inputs {
            let Some((&command, rest)) = input.bytes.split_first() else {
                continue;
            };
            let mut f = Fields(rest);
            match command {
                1 => {
                    let (lo, hi) = (f.voxel(), f.voxel());
                    let n: u64 = host.count_in_box(lo, hi).iter().map(|&(_, c)| c).sum();
                    broadcast.extend_from_slice(&n.to_le_bytes());
                }
                2 => {
                    let n = f.u32();
                    let (origin, dir, max) = (f.fixed3(), f.fixed3(), f.i64());
                    let hits = (0..n)
                        .filter(|_| host.raycast(origin, dir, max).is_some())
                        .count() as u32;
                    broadcast.extend_from_slice(&hits.to_le_bytes());
                }
                3 => {
                    let n = f.u32();
                    let op = f.0;
                    let mut tag = 0;
                    for _ in 0..n {
                        tag = wit::apply(op).first().copied().unwrap_or(0);
                    }
                    broadcast.push(tag);
                }
                4 => {
                    let (n, len) = (f.u32(), f.u32());
                    let junk = vec![0xFF; len as usize];
                    let mut calls = 0u32;
                    for _ in 0..n {
                        std::hint::black_box(wit::apply(&junk));
                        calls += 1;
                    }
                    broadcast.extend_from_slice(&calls.to_le_bytes());
                }
                5 => {
                    let mut cells = Vec::new();
                    while !f.0.is_empty() {
                        cells.push(CellCoord([f.i32(), f.i32(), f.i32()]));
                    }
                    let n = host.unsupported(&cells).len() as u32;
                    broadcast.extend_from_slice(&n.to_le_bytes());
                }
                _ => {}
            }
        }
        TickOutput { broadcast }
    }
}

vxe_script_wasm::export_logic!(Greedy);
