//! Test logics and fixtures shared by this crate's tests, the WebAssembly host's tests and
//! its guests, behind the `test-logic` feature: [`EchoLogic`], [`SeededLogic`], the world they
//! run on and the seeded inputs that drive them. Nothing here is engine behaviour.

use vxe_world::*;

use crate::{ClientInput, Host, Logic, TickOutput};

/// Rows 0 (air), 1..=5 (plain), with 4 flagged indestructible.
pub fn test_table() -> MaterialTable {
    let mut t = MaterialTable::new();
    for i in 1..=5u16 {
        let mut m = Material::named(format!("m{i}"));
        if i == 4 {
            m.flags = MaterialFlags::INDESTRUCTIBLE;
        }
        assert_eq!(t.push(m), i);
    }
    t
}

/// `UniformSource(3)` over chunks `[-2, 2)³`: voxels `[-128, 128)` on every axis.
pub fn solid3_world() -> World {
    World::generate(
        ChunkExtent::new([-2, -2, -2], [2, 2, 2]),
        &UniformSource(3),
        &test_table(),
    )
}

/// SplitMix64: a seeded, platform-independent stream for test inputs.
pub struct SplitMix64(pub u64);

impl SplitMix64 {
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in `lo..=hi`.
    pub fn range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + (self.next_u64() % (hi - lo + 1) as u64) as i64
    }
}

/// One metre at 10 cm voxels, in fixed point.
pub const METRE: i64 = 10 * FIXED_ONE;

/// `n` distinct voxel centres drawn from `seed`, each at least 1 m plus one voxel inside
/// [`solid3_world`]'s faces, so a 1 m sphere around any of them stays resident.
pub fn seeded_centres(seed: u64, n: usize) -> Vec<Fixed3> {
    let mut rng = SplitMix64(seed);
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::with_capacity(n);
    while out.len() < n {
        let v = [0; 3].map(|_| rng.range(-117, 116) as i32);
        if seen.insert(v) {
            out.push(centre_of_voxel(v));
        }
    }
    out
}

/// A 1 m carve at each of `seeded_centres(seed, n)`.
pub fn seeded_carves(seed: u64, n: usize) -> Vec<EditOp> {
    seeded_centres(seed, n)
        .into_iter()
        .map(|c| EditOp::sphere(c, METRE, Mode::Carve))
        .collect()
}

/// The op's wire bytes, as [`EchoLogic`] reads an input.
pub fn op_bytes(op: EditOp) -> Vec<u8> {
    let mut out = Vec::new();
    wire::encode_op(&mut out, &Op::Edit(op));
    out
}

/// Outcome bytes [`EchoLogic`] broadcasts, one per input.
pub const ECHO_APPLIED: u8 = 1;
pub const ECHO_REFUSED: u8 = 2;
pub const ECHO_UNDECODABLE: u8 = 3;

/// Decodes each input as one op in the wire encoding and applies it through the host. Its
/// broadcast is one outcome byte per input ([`ECHO_APPLIED`], [`ECHO_REFUSED`] or
/// [`ECHO_UNDECODABLE`]), empty for a tick without inputs.
#[derive(Default)]
pub struct EchoLogic;

impl Logic for EchoLogic {
    fn init(&mut self, _host: &mut dyn Host, _seed: u64) {}

    fn tick(&mut self, host: &mut dyn Host, _tick: u64, inputs: &[ClientInput]) -> TickOutput {
        let mut broadcast = Vec::with_capacity(inputs.len());
        for input in inputs {
            let mut r = wire::Reader::new(&input.bytes);
            let outcome = match wire::decode_op(&mut r) {
                Ok(Op::Edit(op)) => match host.apply(op) {
                    Ok(_) => ECHO_APPLIED,
                    Err(_) => ECHO_REFUSED,
                },
                Err(_) => ECHO_UNDECODABLE,
            };
            broadcast.push(outcome);
        }
        TickOutput { broadcast }
    }
}

/// One xorshift64 step (13, 7, 17); the state must be non-zero.
pub fn xorshift64(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

/// The state [`SeededLogic`] starts from for `seed`: never zero, so xorshift never sticks.
pub fn seeded_state(seed: u64) -> u64 {
    seed ^ 0x9e37_79b9_7f4a_7c15
}

/// A voxel coordinate inside [`solid3_world`] with 1 m plus one voxel of margin, from one
/// xorshift draw.
pub fn interior_voxel(state: &mut u64) -> i32 {
    (xorshift64(state) % 234) as i32 - 117
}

/// Carves one 1 m sphere per input at a centre drawn from its own xorshift state, ignoring
/// the payload. Its broadcast is the 8-byte little-endian state after the tick.
pub struct SeededLogic {
    state: u64,
}

impl Default for SeededLogic {
    fn default() -> Self {
        Self { state: 1 }
    }
}

impl Logic for SeededLogic {
    fn init(&mut self, _host: &mut dyn Host, seed: u64) {
        self.state = seeded_state(seed);
    }

    fn tick(&mut self, host: &mut dyn Host, _tick: u64, inputs: &[ClientInput]) -> TickOutput {
        for _ in inputs {
            let v = [0; 3].map(|_| interior_voxel(&mut self.state));
            let _ = host.apply(EditOp::sphere(centre_of_voxel(v), METRE, Mode::Carve));
        }
        TickOutput {
            broadcast: self.state.to_le_bytes().to_vec(),
        }
    }
}
