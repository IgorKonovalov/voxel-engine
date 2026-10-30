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
//! drops [`FALL_STEP`] voxels a tick, one `Drop` op per region per tick, through the cells
//! under [`YIELD_BELOW`] solid voxels (ADR-0016), crushing what it passes, until a drop moves
//! it less than a whole step; then, when the fall crushed anything, one `Pile` op puts the
//! whole crushed volume back as [`RUBBLE`] on top of it. In crumble mode (`Crumble` toggles
//! it) the region is removed at once, a [`RUBBLE_FRACTION`] of it piled as [`RUBBLE`]. A
//! region that cannot move on its first tick is wedged, and a region whose drop is refused
//! stops; either is remembered (at most [`STUCK_MAX`] of them, the oldest forgotten first) and
//! not reported again until a dig changes its voxels. A tick runs the falls first, then its
//! inputs in order.
//!
//! The broadcast is UTF-8 lines joined by `\n`. A line `op <verb>` stands for the tick's next
//! op, applied or refused, in the order the client receives their announcements (`carved`
//! for a dig, `filled` for a fill, `bench` for each of a bench's [`BENCH_CARVES`] carves,
//! `stamped`, `dropped` for each tick's drop of a falling region, `piled` for the rubble pile
//! at the end of a fall that crushed voxels, `removed`). A line `region <cells>
//! <material>:<voxels> ...` reports one unsupported region the query found after the dig
//! before it ([`region_line`], [`parse_region_line`]); a line `wedged <cells>
//! <material>:<voxels> ...` reports a region that cannot fall ([`wedged_line`],
//! [`parse_wedged_line`]). A fall ends with `landed after N ticks, fell M voxels` or `drop
//! refused: <error>, after N ticks, fell M voxels`, followed, when it crushed `K` voxels, by
//! `, crushed K voxels, P rubble` (`P` the pile's placed voxels) or `, crushed K voxels, pile
//! refused: <error>`. Any other line is a message to print as it is.

use std::collections::VecDeque;

use vxe_net::{ClientInput, DEFAULT_SUPPORT, Host, Logic, TickOutput, cells_in};
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
/// The verb of the rubble pile's `op` line at the end of a fall that crushed voxels.
pub const PILE_VERB: &str = "piled";

/// Voxels a falling region drops per tick: one cell, so it stays on the cell lattice.
pub const FALL_STEP: i32 = BRICK_EDGE;
/// A falling region passes through, and crushes, the voxels of cells holding fewer solid
/// voxels than this: the support query's own threshold, so a cell the query calls load-free
/// does not stop the fall. The fly-through sets [`DEFAULT_SUPPORT`] on its server.
pub const YIELD_BELOW: u16 = DEFAULT_SUPPORT.solid_min;
/// The wedged or refused regions the logic remembers, so as not to report them again.
pub const STUCK_MAX: usize = 64;
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

/// `<verb> <cells>`, then `<material>:<voxels>` per material, ascending by material.
fn cells_line(verb: &str, cells: usize, counts: &[(MaterialId, u32)]) -> String {
    let mut line = format!("{verb} {cells}");
    for (m, n) in counts {
        line.push_str(&format!(" {m}:{n}"));
    }
    line
}

/// The line reporting an unsupported region: `region <cells>`, then `<material>:<voxels>` per
/// material, ascending by material.
pub fn region_line(region: &Region) -> String {
    cells_line("region", region.cells.len(), &region.counts)
}

/// The cell count and the per-material counts of a [`region_line`]; `None` for any other
/// line.
pub fn parse_region_line(line: &str) -> Option<(usize, Vec<(MaterialId, u32)>)> {
    parse_cells_line("region ", line)
}

/// The line reporting a region that cannot fall: [`region_line`]'s grammar with the verb
/// `wedged`.
pub fn wedged_line(cells: usize, counts: &[(MaterialId, u32)]) -> String {
    cells_line("wedged", cells, counts)
}

/// The cell count and the per-material counts of a [`wedged_line`]; `None` for any other
/// line.
pub fn parse_wedged_line(line: &str) -> Option<(usize, Vec<(MaterialId, u32)>)> {
    parse_cells_line("wedged ", line)
}

fn parse_cells_line(prefix: &str, line: &str) -> Option<(usize, Vec<(MaterialId, u32)>)> {
    let mut words = line.strip_prefix(prefix)?.split(' ');
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

/// The line that ends a refused fall: `drop refused: <error>, after N ticks, fell M voxels`.
pub fn refused_line(error: &impl std::fmt::Display, ticks: u32, fell: i32) -> String {
    format!("drop refused: {error}, after {ticks} ticks, fell {fell} voxels")
}

/// The refusal text for a host that answered an op of kind `want` with `got`, another
/// kind's result: `the host answered a drop with a pile result`.
fn not_the_result(want: &str, got: &OpResult) -> String {
    let kind = match got {
        OpResult::Edit(_) => "an edit",
        OpResult::Drop(_) => "a drop",
        OpResult::Remove(_) => "a remove",
        OpResult::Pile(_) => "a pile",
    };
    format!("the host answered a {want} with {kind} result")
}

/// A region on its way down: its cells (one cell lower after each whole step), its voxels by
/// material as the query reported them, the ticks it moved on, the voxels it fell and the
/// voxels its drops crushed.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Falling {
    cells: Vec<CellCoord>,
    counts: Vec<(MaterialId, u32)>,
    ticks: u32,
    fell: i32,
    crushed: u32,
}

/// The key a stuck region is remembered by: `mix64` folded over its cells and counts. A dig
/// that changes the region's voxels changes its counts, and so its key.
fn stuck_key(cells: &[CellCoord], counts: &[(MaterialId, u32)]) -> u64 {
    let mut k = 0u64;
    let mut add = |v: u64| k = mix64(k ^ v);
    add(cells.len() as u64);
    for c in cells {
        for v in c.0 {
            add(u64::from(v as u32));
        }
    }
    add(counts.len() as u64);
    for &(m, n) in counts {
        add(u64::from(m));
        add(u64::from(n));
    }
    k
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
    /// Keys ([`stuck_key`]) of the regions that were wedged or refused, oldest first, at most
    /// [`STUCK_MAX`].
    stuck: VecDeque<u64>,
}

impl DigLogic {
    /// One tick of falling: every falling region drops by up to [`FALL_STEP`]. A drop of a
    /// whole step moves the region's cells one cell down and it falls on. A shorter drop after
    /// it has moved means it has landed; no drop at all on its first tick means it is wedged;
    /// a refusal stops it. A landing, or a refusal after it moved, that crushed voxels piles
    /// them as rubble on its cells.
    fn fall(&mut self, host: &mut dyn Host, lines: &mut Vec<String>) {
        for mut f in std::mem::take(&mut self.falling) {
            let r = host.apply(Op::Drop {
                cells: f.cells.clone(),
                max_drop: FALL_STEP,
                yield_below: YIELD_BELOW,
            });
            lines.push(format!("op {DROP_VERB}"));
            // A host that answers a drop with another kind of result is refused like an error:
            // the same line natively and as a component, and no panic.
            let refused = match r {
                Ok(OpResult::Drop(d)) => Ok(d),
                Ok(other) => Err(not_the_result("drop", &other)),
                Err(e) => Err(e.to_string()),
            };
            let d = match refused {
                Ok(d) => d,
                Err(e) => {
                    let line = refused_line(&e, f.ticks, f.fell);
                    let line = self.end_fall(host, &f, line, lines);
                    lines.push(line);
                    self.remember(&f);
                    continue;
                }
            };
            if d.dropped == 0 && f.ticks == 0 {
                lines.push(wedged_line(f.cells.len(), &f.counts));
                self.remember(&f);
                continue;
            }
            f.crushed += d.crushed.iter().map(|c| c.1).sum::<u32>();
            if d.dropped > 0 {
                f.ticks += 1;
                f.fell += d.dropped;
            }
            if d.dropped == FALL_STEP {
                for c in &mut f.cells {
                    c.0[2] -= 1;
                }
                self.falling.push(f);
            } else {
                let line = landed_line(f.ticks, f.fell);
                let line = self.end_fall(host, &f, line, lines);
                lines.push(line);
            }
        }
    }

    /// The end of fall `f`, whose last line is `line`: when it moved and crushed voxels, piles
    /// them on its cells, pushes `op piled`, and returns `line` with the crush and the pile
    /// appended; otherwise `line` as it is.
    fn end_fall(
        &self,
        host: &mut dyn Host,
        f: &Falling,
        line: String,
        lines: &mut Vec<String>,
    ) -> String {
        if f.ticks == 0 || f.crushed == 0 {
            return line;
        }
        let r = host.apply(Op::Pile {
            cells: f.cells.clone(),
            rubble: RUBBLE,
            count: f.crushed,
            max_depth: RUBBLE_DEPTH,
        });
        lines.push(format!("op {PILE_VERB}"));
        match r {
            Ok(OpResult::Pile(p)) => {
                format!("{line}, crushed {} voxels, {} rubble", f.crushed, p.placed)
            }
            Ok(other) => format!(
                "{line}, crushed {} voxels, pile refused: {}",
                f.crushed,
                not_the_result("pile", &other)
            ),
            Err(e) => format!("{line}, crushed {} voxels, pile refused: {e}", f.crushed),
        }
    }

    /// Remembers `f` as stuck, forgetting the oldest past [`STUCK_MAX`].
    fn remember(&mut self, f: &Falling) {
        self.stuck.push_back(stuck_key(&f.cells, &f.counts));
        if self.stuck.len() > STUCK_MAX {
            self.stuck.pop_front();
        }
    }

    /// After a dig that changed the voxels in `aabb`: asks the host which regions lost their
    /// support, reports each, and queues it to fall or removes it. A region overlapping one
    /// already falling is that one, and is skipped; so is a remembered stuck one.
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
            if self
                .stuck
                .contains(&stuck_key(&region.cells, &region.counts))
            {
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
                    counts: region.counts,
                    ticks: 0,
                    fell: 0,
                    crushed: 0,
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

#[cfg(test)]
mod tests {
    use super::*;
    use vxe_world::{DropResult, EditError, EditResult, Hit, PileResult};

    /// A host that carves for any edit, reports one region after it, and answers drops and
    /// piles with the results it holds, of whatever kind.
    struct Answers {
        drop: OpResult,
        pile: OpResult,
    }

    impl Host for Answers {
        fn apply(&mut self, op: Op) -> Result<OpResult, EditError> {
            Ok(match op {
                Op::Drop { .. } => self.drop.clone(),
                Op::Pile { .. } => self.pile.clone(),
                _ => OpResult::Edit(EditResult {
                    changed_from: vec![(3, 1)],
                    touched: Vec::new(),
                    aabb: Some((VoxelCoord([0; 3]), VoxelCoord([7; 3]))),
                }),
            })
        }

        fn material_at(&self, _v: VoxelCoord) -> Option<MaterialId> {
            None
        }

        fn raycast(&self, _origin: Fixed3, _dir: Fixed3, _max: i64) -> Option<Hit> {
            None
        }

        fn count_in_box(&self, _lo: VoxelCoord, _hi: VoxelCoord) -> Vec<(MaterialId, u64)> {
            Vec::new()
        }

        fn unsupported(&self, _changed: &[CellCoord]) -> Vec<Region> {
            vec![Region {
                cells: vec![CellCoord([0, 0, 1])],
                counts: vec![(3, 512)],
                aabb: (VoxelCoord([0, 0, 8]), VoxelCoord([7, 7, 15])),
            }]
        }
    }

    /// The lines of a dig tick, then of the tick its region first falls in.
    fn fall_lines(host: &mut Answers) -> Vec<String> {
        let mut logic = DigLogic::default();
        logic.init(host, 0);
        let dig = DigInput::Dig {
            origin: Fixed3([0; 3]),
            dir: [256, 0, 0],
            radius: FIXED_ONE,
        };
        let input = ClientInput {
            client: 0,
            bytes: dig.encode(),
        };
        let out = logic.tick(host, 0, &[input]);
        let lines = String::from_utf8(out.broadcast).unwrap();
        assert!(lines.contains("region "), "{lines}");
        let out = logic.tick(host, 1, &[]);
        String::from_utf8(out.broadcast)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// A drop answered with a pile result is refused like an error, not a panic.
    #[test]
    fn a_drop_answered_with_a_pile_result_is_refused() {
        let mut host = Answers {
            drop: OpResult::Pile(PileResult::default()),
            pile: OpResult::Pile(PileResult::default()),
        };
        assert_eq!(
            fall_lines(&mut host),
            [
                "op dropped",
                "drop refused: the host answered a drop with a pile result, after 0 ticks, \
                 fell 0 voxels"
            ]
        );
    }

    /// A landing's pile answered with a drop result is refused in the landing's line.
    #[test]
    fn a_pile_answered_with_a_drop_result_is_refused() {
        let mut host = Answers {
            drop: OpResult::Drop(DropResult {
                dropped: 1,
                crushed: vec![(3, 5)],
                ..DropResult::default()
            }),
            pile: OpResult::Drop(DropResult::default()),
        };
        assert_eq!(
            fall_lines(&mut host),
            [
                "op dropped",
                "op piled",
                "landed after 1 ticks, fell 1 voxels, crushed 5 voxels, pile refused: the host \
                 answered a pile with a drop result"
            ]
        );
    }
}

#[cfg(feature = "component")]
vxe_script_wasm::export_logic!(DigLogic);
