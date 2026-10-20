//! Test logics and fixtures shared by this crate's tests, the WebAssembly host's tests and
//! its guests, behind the `test-logic` feature: [`EchoLogic`], [`SeededLogic`], the worlds they
//! run on and the inputs that drive them. Nothing here is engine behaviour.

use std::sync::Arc;

use vxe_world::wire::put_i32x3;
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
pub fn op_bytes(op: impl Into<Op>) -> Vec<u8> {
    let mut out = Vec::new();
    wire::encode_op(&mut out, &op.into());
    out
}

/// Outcome bytes [`EchoLogic`] broadcasts, one per input.
pub const ECHO_APPLIED: u8 = 1;
pub const ECHO_REFUSED: u8 = 2;
pub const ECHO_UNDECODABLE: u8 = 3;
/// The outcome of a support query input; the regions follow it ([`EchoLogic`]).
pub const ECHO_REGIONS: u8 = 4;

/// First byte of an [`EchoLogic`] input that asks the support query instead of applying an
/// op; no op kind uses it.
pub const ECHO_QUERY: u8 = 0x40;

/// The input that has [`EchoLogic`] ask [`Host::unsupported`] about `cells`: [`ECHO_QUERY`],
/// a `u32` count, the cells as 3 × `i32`.
pub fn query_bytes(cells: &[CellCoord]) -> Vec<u8> {
    let mut out = vec![ECHO_QUERY];
    out.extend_from_slice(&(cells.len() as u32).to_le_bytes());
    for c in cells {
        put_i32x3(&mut out, c.0);
    }
    out
}

fn read_cells(r: &mut wire::Reader<'_>) -> Result<Vec<CellCoord>, WireError> {
    let n = r.u32()?;
    (0..n).map(|_| Ok(CellCoord(r.i32x3()?))).collect()
}

/// Appends `regions` as [`EchoLogic`] broadcasts them: a `u32` count, then per region its
/// cells (a `u32` count and 3 × `i32` each), its counts (a `u16` count and `(u16, u32)`
/// pairs) and its aabb (6 × `i32`).
pub fn encode_regions(out: &mut Vec<u8>, regions: &[Region]) {
    out.extend_from_slice(&(regions.len() as u32).to_le_bytes());
    for g in regions {
        out.extend_from_slice(&(g.cells.len() as u32).to_le_bytes());
        for c in &g.cells {
            put_i32x3(out, c.0);
        }
        out.extend_from_slice(&(g.counts.len() as u16).to_le_bytes());
        for &(m, n) in &g.counts {
            out.extend_from_slice(&m.to_le_bytes());
            out.extend_from_slice(&n.to_le_bytes());
        }
        put_i32x3(out, g.aabb.0.0);
        put_i32x3(out, g.aabb.1.0);
    }
}

/// Reads regions as [`encode_regions`] writes them.
pub fn decode_regions(r: &mut wire::Reader<'_>) -> Result<Vec<Region>, WireError> {
    let n = r.u32()?;
    (0..n)
        .map(|_| {
            let cells = read_cells(r)?;
            let k = r.u16()?;
            let counts = (0..k)
                .map(|_| Ok((r.u16()?, r.u32()?)))
                .collect::<Result<_, WireError>>()?;
            let aabb = (VoxelCoord(r.i32x3()?), VoxelCoord(r.i32x3()?));
            Ok(Region {
                cells,
                counts,
                aabb,
            })
        })
        .collect()
}

/// Decodes each input as one op of any kind in the wire encoding and applies it through the
/// host, or, for an input starting with [`ECHO_QUERY`], asks the host's support query about
/// its cells. Its broadcast is one outcome byte per input ([`ECHO_APPLIED`],
/// [`ECHO_REFUSED`], [`ECHO_UNDECODABLE`], or [`ECHO_REGIONS`] followed by the regions as
/// [`encode_regions`] writes them), empty for a tick without inputs.
#[derive(Default)]
pub struct EchoLogic;

impl Logic for EchoLogic {
    fn init(&mut self, _host: &mut dyn Host, _seed: u64) {}

    fn tick(&mut self, host: &mut dyn Host, _tick: u64, inputs: &[ClientInput]) -> TickOutput {
        let mut broadcast = Vec::with_capacity(inputs.len());
        for input in inputs {
            let mut r = wire::Reader::new(&input.bytes);
            if input.bytes.first() == Some(&ECHO_QUERY) {
                r.u8().expect("one byte is there");
                match read_cells(&mut r) {
                    Ok(cells) if r.is_empty() => {
                        broadcast.push(ECHO_REGIONS);
                        encode_regions(&mut broadcast, &host.unsupported(&cells));
                    }
                    _ => broadcast.push(ECHO_UNDECODABLE),
                }
                continue;
            }
            let outcome = match wire::decode_op(&mut r) {
                Ok(op) => match host.apply(op) {
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

/// A box op over the half-open voxel bounds `lo..hi`.
pub fn box_voxels(lo: [i32; 3], hi: [i32; 3], mode: Mode) -> EditOp {
    let f = |v: [i32; 3]| Fixed3(v.map(|c| i64::from(c) * FIXED_ONE));
    EditOp::box_(f(lo), f(hi), mode)
}

/// [`slab_world`]'s materials: the floor, the pillars and slabs, the vault's shell, the
/// rubble a removal piles.
pub const SLAB_FLOOR: MaterialId = 3;
pub const SLAB_ROCK: MaterialId = 1;
pub const SLAB_SHELL: MaterialId = 2;
pub const SLAB_RUBBLE: MaterialId = 5;

/// The pillars under [`slab_world`]'s two slabs, as half-open voxel bounds.
pub const PILLAR_A: ([i32; 3], [i32; 3]) = ([72, 24, 8], [80, 32, 40]);
pub const PILLAR_B: ([i32; 3], [i32; 3]) = ([72, 88, 8], [80, 96, 40]);

/// Air over chunks `[0, 2)³` (voxels `[0, 128)`), a floor 8 voxels deep, and two 3 x 3-cell
/// slabs of rock 32 voxels above it, each on a one-cell pillar ([`PILLAR_A`], [`PILLAR_B`]).
pub fn slab_world() -> World {
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [2, 2, 2]),
        &UniformSource(AIR),
        &test_table(),
    );
    for (lo, hi, m) in [
        ([0, 0, 0], [128, 128, 8], SLAB_FLOOR),
        (PILLAR_A.0, PILLAR_A.1, SLAB_ROCK),
        ([64, 16, 40], [88, 40, 48], SLAB_ROCK),
        (PILLAR_B.0, PILLAR_B.1, SLAB_ROCK),
        ([64, 80, 40], [88, 104, 48], SLAB_ROCK),
    ] {
        world
            .apply(&box_voxels(lo, hi, Mode::Fill(m)))
            .expect("the slab world is resident");
    }
    world
}

/// The 9 cells, ascending, of a slab whose cells span `x` 8..=10, `y` from `y0` and `z` at
/// `z`: slab A at `y0 = 2`, slab B at `y0 = 10`, both at `z = 5` before they move.
pub fn slab_cells(y0: i32, z: i32) -> Vec<CellCoord> {
    let mut cells = Vec::new();
    for y in y0..y0 + 3 {
        for x in 8..11 {
            cells.push(CellCoord([x, y, z]));
        }
    }
    cells.sort_unstable();
    cells
}

/// The 16³ vault of [`SLAB_SHELL`] the slab session stamps.
pub fn slab_vault() -> Arc<Prefab> {
    Arc::new(Prefab::vault(16, SLAB_SHELL))
}

/// The slab session on [`slab_world`], one op a tick: the vault (`vault_id`) stamped at
/// `(32, 32, 32)` unrotated, pillar A carved, slab A dropped 8 voxels four times (each drop
/// names the cells one cell lower), pillar B carved, and slab B removed with a quarter left
/// as rubble.
pub fn slab_session(vault_id: u64) -> Vec<Op> {
    let mut ops = vec![
        Op::Stamp {
            prefab: vault_id,
            origin: VoxelCoord([32, 32, 32]),
            rot: Rot::IDENTITY,
        },
        Op::Edit(box_voxels(PILLAR_A.0, PILLAR_A.1, Mode::Carve)),
    ];
    for k in 0..4 {
        ops.push(Op::Drop {
            cells: slab_cells(2, 5 - k),
            max_drop: 8,
        });
    }
    ops.push(Op::Edit(box_voxels(PILLAR_B.0, PILLAR_B.1, Mode::Carve)));
    ops.push(Op::Remove {
        cells: slab_cells(10, 5),
        rubble: SLAB_RUBBLE,
        fraction: (1, 4),
        max_depth: 64,
    });
    ops
}

/// The world hash after [`slab_session`] on [`slab_world`]. Blessed on Linux x86_64,
/// asserted on every arm.
pub const SLAB_SESSION_HASH: u64 = 0x2077_5c41_d63d_17c5;

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
            let _ = host.apply(Op::Edit(EditOp::sphere(
                centre_of_voxel(v),
                METRE,
                Mode::Carve,
            )));
        }
        TickOutput {
            broadcast: self.state.to_le_bytes().to_vec(),
        }
    }
}
