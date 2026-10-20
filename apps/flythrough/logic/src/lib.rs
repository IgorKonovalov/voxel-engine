//! The fly-through's server logic: digs, fills, stamps, the bench and the collapse that follows
//! a dig, as inputs applied through the host (ADR-0010). Every edit it makes is an op the
//! server logs, so a saved session replays all of it. It depends on `vxe-net`'s traits and `vxe-world` alone, nothing GPU, so
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
//! | `Stamp` | 4   | origin `Fixed3`, dir 3 × `i64`, prefab id `u64`              | 57    |
//! | `Crumble` | 5 | none                                                         | 1     |
//!
//! After every applied dig the logic asks the host which regions lost their support (over the
//! cells of the dig's aabb, [`vxe_net::cells_in`]). In fall mode (the default) each region
//! drops [`FALL_STEP`] voxels a tick, one `Drop` op per region per tick, until a drop moves it
//! less than a whole step; in crumble mode (`Crumble` toggles it) the region is removed at
//! once, a [`RUBBLE_FRACTION`] of it piled as [`RUBBLE`]. A tick runs the falls first, then
//! its inputs in order.
//!
//! The broadcast is UTF-8 lines joined by `\n`. A line `op <verb>` stands for the tick's next
//! op, applied or refused, in the order the client receives their announcements (`carved`
//! for a dig, `filled` for a fill, `bench` for each of a bench's [`BENCH_CARVES`] carves,
//! `stamped`, `dropped` for each tick's drop of a falling region, `removed`). A line
//! `region <cells> <material>:<voxels> ...` reports one unsupported region the query found
//! after the dig before it ([`region_line`], [`parse_region_line`]); any other line is a
//! message to print as it is.

use vxe_net::{ClientInput, Host, Logic, TickOutput, cells_in};
use vxe_world::hash::mix64;
use vxe_world::wire::{Reader, WireError, put_fixed3};
use vxe_world::{
    BRICK_EDGE, CellCoord, EditOp, FIXED_ONE, Fixed3, MaterialId, Mode, Op, OpResult, Region, Rot,
    VoxelCoord,
};

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
/// The verb of a stamp's `op` line.
pub const STAMP_VERB: &str = "stamped";
/// The verb of a falling region's drop's `op` line.
pub const DROP_VERB: &str = "dropped";
/// The verb of a crumbling region's removal's `op` line.
pub const REMOVE_VERB: &str = "removed";

/// Voxels a falling region drops per tick: one cell, so it stays on the cell lattice.
pub const FALL_STEP: i32 = BRICK_EDGE;
/// The material a removed region's rubble is made of (the fly-through's table row 4).
pub const RUBBLE: MaterialId = 4;
/// A removed region's rubble: a quarter of each column's volume, on ground within 6.4 m.
pub const RUBBLE_FRACTION: (u32, u32) = (1, 4);
pub const RUBBLE_DEPTH: i32 = 64;

/// Input tags.
pub const INPUT_DIG: u8 = 1;
pub const INPUT_FILL: u8 = 2;
pub const INPUT_BENCH: u8 = 3;
pub const INPUT_STAMP: u8 = 4;
pub const INPUT_CRUMBLE: u8 = 5;

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
    /// Stamp the prefab with content id `prefab` (registered on the server) unrotated, its
    /// minimum corner at the first solid voxel the ray enters within [`REACH`].
    Stamp {
        origin: Fixed3,
        dir: [i64; 3],
        prefab: u64,
    },
    /// Flip between dropping unsupported regions and removing them with rubble.
    Crumble,
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
            Self::Stamp {
                origin,
                dir,
                prefab,
            } => {
                out.push(INPUT_STAMP);
                put_fixed3(&mut out, origin);
                put_fixed3(&mut out, Fixed3(dir));
                out.extend_from_slice(&prefab.to_le_bytes());
            }
            Self::Crumble => out.push(INPUT_CRUMBLE),
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
            INPUT_STAMP => Self::Stamp {
                origin: r.fixed3()?,
                dir: r.fixed3()?.0,
                prefab: r.u64()?,
            },
            INPUT_CRUMBLE => Self::Crumble,
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

/// The line reporting an unsupported region: `region <cells>`, then `<material>:<voxels>` per
/// material, ascending by material.
pub fn region_line(region: &Region) -> String {
    let mut line = format!("region {}", region.cells.len());
    for (m, n) in &region.counts {
        line.push_str(&format!(" {m}:{n}"));
    }
    line
}

/// The cell count and the per-material counts of a [`region_line`]; `None` for any other
/// line.
pub fn parse_region_line(line: &str) -> Option<(usize, Vec<(MaterialId, u32)>)> {
    let mut words = line.strip_prefix("region ")?.split(' ');
    let cells = words.next()?.parse().ok()?;
    let counts = words
        .map(|w| {
            let (m, n) = w.split_once(':')?;
            Some((m.parse().ok()?, n.parse().ok()?))
        })
        .collect::<Option<_>>()?;
    Some((cells, counts))
}

/// The line after a falling region's last drop.
pub fn landed_line(ticks: u32, fell: i32) -> String {
    format!("landed after {ticks} ticks, fell {fell} voxels")
}

/// The line after a [`DigInput::Crumble`].
pub fn crumble_line(crumble: bool) -> &'static str {
    if crumble {
        "collapse: remove unsupported regions with rubble"
    } else {
        "collapse: drop unsupported regions"
    }
}

/// A region on its way down: its cells (one cell lower after each whole step), the ticks it
/// moved on and the voxels it fell.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Falling {
    cells: Vec<CellCoord>,
    ticks: u32,
    fell: i32,
}

/// The fly-through's logic: each input is one [`DigInput`]; undecodable inputs are ignored.
/// Its state is the collapse mode and the regions falling, a function of its inputs and the
/// world's answers, so re-simulation needs only the inputs and the submitted ops.
#[derive(Clone, Debug, Default)]
pub struct DigLogic {
    /// Remove unsupported regions with rubble instead of dropping them.
    crumble: bool,
    /// Regions falling, in the order they were found.
    falling: Vec<Falling>,
}

impl DigLogic {
    /// One tick of falling: every falling region drops by up to [`FALL_STEP`]. A drop of a
    /// whole step moves the region's cells one cell down and it falls on; a shorter drop (or
    /// none, or a refusal) means it has landed.
    fn fall(&mut self, host: &mut dyn Host, lines: &mut Vec<String>) {
        for mut f in std::mem::take(&mut self.falling) {
            let r = host.apply(Op::Drop {
                cells: f.cells.clone(),
                max_drop: FALL_STEP,
                yield_below: 0,
            });
            lines.push(format!("op {DROP_VERB}"));
            let dropped = match r {
                Ok(OpResult::Drop(d)) => d.dropped,
                _ => 0,
            };
            if dropped > 0 {
                f.ticks += 1;
                f.fell += dropped;
            }
            if dropped == FALL_STEP {
                for c in &mut f.cells {
                    c.0[2] -= 1;
                }
                self.falling.push(f);
            } else {
                lines.push(landed_line(f.ticks, f.fell));
            }
        }
    }

    /// After a dig that changed the voxels in `aabb`: asks the host which regions lost their
    /// support, reports each, and queues it to fall or removes it. A region overlapping one
    /// already falling is that one, and is skipped.
    fn collapse(
        &mut self,
        host: &mut dyn Host,
        aabb: Option<(VoxelCoord, VoxelCoord)>,
        lines: &mut Vec<String>,
    ) {
        for region in host.unsupported(&cells_in(aabb)) {
            if self.falling.iter().any(|f| {
                region
                    .cells
                    .iter()
                    .any(|c| f.cells.binary_search(c).is_ok())
            }) {
                continue;
            }
            lines.push(region_line(&region));
            if self.crumble {
                let _ = host.apply(Op::Remove {
                    cells: region.cells,
                    rubble: RUBBLE,
                    fraction: RUBBLE_FRACTION,
                    max_depth: RUBBLE_DEPTH,
                });
                lines.push(format!("op {REMOVE_VERB}"));
            } else {
                self.falling.push(Falling {
                    cells: region.cells,
                    ticks: 0,
                    fell: 0,
                });
            }
        }
    }
}

impl Logic for DigLogic {
    fn init(&mut self, _host: &mut dyn Host, _seed: u64) {
        *self = Self::default();
    }

    fn tick(&mut self, host: &mut dyn Host, _tick: u64, inputs: &[ClientInput]) -> TickOutput {
        let mut lines: Vec<String> = Vec::new();
        self.fall(host, &mut lines);
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
                    let r = host.apply(Op::Edit(EditOp::capsule(origin, end, radius, Mode::Carve)));
                    lines.push(format!("op {DIG_VERB}"));
                    if let Ok(r) = r {
                        self.collapse(host, r.aabb(), &mut lines);
                    }
                }
                Ok(DigInput::Fill {
                    origin,
                    dir,
                    radius,
                    material,
                }) => match host.raycast(origin, Fixed3(dir), REACH) {
                    Some(hit) => {
                        let at = along(origin, dir, hit.t);
                        let _ =
                            host.apply(Op::Edit(EditOp::sphere(at, radius, Mode::Fill(material))));
                        lines.push(format!("op {FILL_VERB}"));
                    }
                    None => lines.push("filled: no surface within 10 m".to_owned()),
                },
                Ok(DigInput::Bench { origin, seed }) => {
                    for op in bench_ops(origin, seed) {
                        let _ = host.apply(Op::Edit(op));
                        lines.push(BENCH_OP_LINE.to_owned());
                    }
                }
                Ok(DigInput::Stamp {
                    origin,
                    dir,
                    prefab,
                }) => match host.raycast(origin, Fixed3(dir), REACH) {
                    Some(hit) => {
                        let _ = host.apply(Op::Stamp {
                            prefab,
                            origin: hit.voxel,
                            rot: Rot::IDENTITY,
                        });
                        lines.push(format!("op {STAMP_VERB}"));
                    }
                    None => lines.push("stamped: no surface within 10 m".to_owned()),
                },
                Ok(DigInput::Crumble) => {
                    self.crumble = !self.crumble;
                    lines.push(crumble_line(self.crumble).to_owned());
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
