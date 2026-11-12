//! The edits the fly-through sends and the lines it prints for them: the dig radius, the
//! simulation split's client and server ([`Net`]), the bench's sum, the session's failure, and
//! the bounded level's coarse grid.

use crate::ScrollSteps;
use crate::app::ms_since;
use crate::cave::vault;
use crate::hud::EditLine;
use crate::logic::{BENCH_VERB, DIG_VERB, FILL_VERB};
use crate::logic::{
    DROP_VERB, PILE_VERB, REMOVE_VERB, STAMP_VERB, parse_region_line, parse_wedged_line,
};
use crate::now;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};
use vxe_net::{Client, DEFAULT_SUPPORT, Logic, LogicError, MemChannel, Server};
use vxe_render::glam::Vec3;
use vxe_world::{
    CellChange, ChunkCoord, CoarseGrid, EditResult, FIXED_ONE, Fixed3, MaterialId, MaterialTable,
    VoxelCoord, WalkableMap, WalkableParams, World,
};

/// Dig radius at start (0.5 m), its scroll step (0.1 m) and its range.
pub(crate) const DIG_RADIUS: i64 = 5 * FIXED_ONE;

pub(crate) const DIG_STEP: i64 = FIXED_ONE;

pub(crate) const DIG_RANGE: (i64, i64) = (FIXED_ONE, 30 * FIXED_ONE);

/// The dig radius after a frame's `delta` scroll lines banked in `scroll`: whole steps of
/// [`DIG_STEP`], clamped to [`DIG_RANGE`]; `None` when it does not change. At either end of
/// the range, scroll pushing past it is dropped rather than banked, so the first line back
/// steps at once.
pub(crate) fn scroll_dig_radius(radius: i64, scroll: &mut ScrollSteps, delta: f32) -> Option<i64> {
    let steps = scroll.add(delta);
    let r = (radius + steps * DIG_STEP).clamp(DIG_RANGE.0, DIG_RANGE.1);
    if (r == DIG_RANGE.1 && scroll.carry() > 0.0) || (r == DIG_RANGE.0 && scroll.carry() < 0.0) {
        scroll.clear();
    }
    (r != radius).then_some(r)
}

/// Radius of the right-click fill: 0.3 m.
pub(crate) const FILL_RADIUS: i64 = 3 * FIXED_ONE;

/// Walkable cells: at most 64 solid voxels, on a floor cell of at least 448, under two open
/// cells (1.6 m).
pub(crate) const WALKABLE: WalkableParams = WalkableParams {
    max_solid: 64,
    min_floor: 448,
    clearance: 2,
};

/// The verb of a `DigLogic` `op` line, as the `'static` the edit lines and the HUD hold;
/// `None` for a line that is not one of its verbs.
pub(crate) fn op_verb(verb: &str) -> Option<&'static str> {
    [
        DIG_VERB,
        FILL_VERB,
        BENCH_VERB,
        STAMP_VERB,
        DROP_VERB,
        PILE_VERB,
        REMOVE_VERB,
    ]
    .into_iter()
    .find(|&v| v == verb)
}

/// One printed line per edit: `carved: rock 1234, ore 12 (3 chunks, 0.21 ms)`.
pub(crate) fn edit_line(
    verb: &'static str,
    r: &EditResult,
    table: &MaterialTable,
    ms: f64,
) -> String {
    edit_record(verb, r, table, ms).to_string()
}

/// The edit as the HUD's last-edit row holds it.
pub(crate) fn edit_record(
    verb: &'static str,
    r: &EditResult,
    table: &MaterialTable,
    ms: f64,
) -> EditLine {
    EditLine {
        verb,
        counts: r
            .changed_from
            .iter()
            .map(|&(m, n)| (material_name(table, m), n))
            .collect(),
        chunks: r.touched.len() as u32,
        ms: ms as f32,
    }
}

pub(crate) fn material_name(table: &MaterialTable, m: MaterialId) -> String {
    table
        .get(m)
        .map_or_else(|| format!("#{m}"), |row| row.name.clone())
}

pub(crate) fn voxel_of(p: Vec3) -> VoxelCoord {
    VoxelCoord(p.to_array().map(|v| v.floor() as i32))
}

/// Chunk offsets within squared distance `r2`.
pub(crate) fn ball(r2: i32) -> Vec<[i32; 3]> {
    let r = (0..).find(|r| r * r > r2).unwrap_or(1);
    let mut v = Vec::new();
    for x in -r..=r {
        for y in -r..=r {
            for z in -r..=r {
                if x * x + y * y + z * z <= r2 {
                    v.push([x, y, z]);
                }
            }
        }
    }
    v
}

/// The bounded level's coarse grid and walkable map, updated from every edit the client is
/// told of, and what the last dig's support query found.
pub(crate) struct Coarse {
    pub(crate) grid: CoarseGrid,
    pub(crate) map: WalkableMap,
    /// Cells the last edit changed, and the grid plus walkable update time in microseconds.
    pub(crate) last: Option<(usize, f64)>,
    /// Regions the logic reported after the last applied dig.
    pub(crate) query: Option<usize>,
}

impl Coarse {
    pub(crate) fn build(world: &World) -> Self {
        let t = now();
        let grid = CoarseGrid::build(world);
        let map = WalkableMap::build(&grid, WALKABLE);
        println!(
            "coarse: {} chunks, {} bytes, {} walkable cells, built in {:.0} ms",
            grid.len(),
            grid.bytes(),
            map.count(),
            ms_since(t)
        );
        Self {
            grid,
            map,
            last: None,
            query: None,
        }
    }

    /// Updates the grid and the walkable map from `r`; returns the grid's changed cells.
    pub(crate) fn update(&mut self, world: &World, r: &EditResult) -> Vec<CellChange> {
        let t = now();
        let changes = self.grid.update(world, r);
        self.map.update(&self.grid, &changes);
        self.last = Some((changes.len(), ms_since(t) * 1e3));
        changes
    }

    /// The `K` line.
    pub(crate) fn line(&self) -> String {
        match self.last {
            Some((m, us)) => format!(
                "coarse: {} walkable cells, last edit changed {m} cells in {us:.0} us",
                self.map.count()
            ),
            None => format!("coarse: {} walkable cells, no edit yet", self.map.count()),
        }
    }

    /// The `F1` collapse line.
    pub(crate) fn query_line(&self) -> String {
        match self.query {
            Some(n) => format!("collapse: last dig's query found {n} regions"),
            None => "collapse: no query yet".to_owned(),
        }
    }
}

/// The line for a logic's `region` line: `unsupported: 9 cells, {rock: 4608}`; `None` for any
/// other line.
pub(crate) fn unsupported_line(table: &MaterialTable, line: &str) -> Option<String> {
    let (cells, counts) = parse_region_line(line)?;
    Some(format!(
        "unsupported: {cells} cells, {{{}}}",
        counts_line(table, &counts)
    ))
}

/// The line for a logic's `wedged` line: `wedged: 9 cells, {rock: 4608}, cannot fall`; `None`
/// for any other line. A `drop refused` line is already prose and prints as it is.
pub(crate) fn wedged_terminal_line(table: &MaterialTable, line: &str) -> Option<String> {
    let (cells, counts) = parse_wedged_line(line)?;
    Some(format!(
        "wedged: {cells} cells, {{{}}}, cannot fall",
        counts_line(table, &counts)
    ))
}

/// The line for an applied removal: `removed: {rock: 4608}, 1152 rubble`.
pub(crate) fn removed_line(table: &MaterialTable, r: &vxe_world::RemoveResult) -> String {
    format!(
        "removed: {{{}}}, {} rubble",
        counts_line(table, &r.removed),
        r.placed
    )
}

/// `rock: 4608, ore: 12`: per-material counts by name.
pub(crate) fn counts_line(table: &MaterialTable, counts: &[(MaterialId, u32)]) -> String {
    counts
        .iter()
        .map(|&(m, n)| format!("{}: {n}", material_name(table, m)))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The simulation split around the world: the server running the fly-through's logic
/// (`--logic`) and the shared client every edit goes through.
pub(crate) struct Net {
    pub(crate) server: Server,
    pub(crate) client: Client,
    /// Whether the logic's stop has been printed: a hosted logic whose restarts are spent
    /// reports [`LogicError::Stopped`] on every later tick, printed only the first time.
    pub(crate) stop_printed: bool,
}

impl Net {
    /// The server on `world` with `logic`, the vault registered on it and on the client (the
    /// prefab `P` stamps, ADR-0014) and [`DEFAULT_SUPPORT`] set (the support query whose
    /// threshold the logic's drops yield below), and the shared client.
    pub(crate) fn new(world: &Arc<World>, logic: Box<dyn Logic>, seed: u64) -> Self {
        let vault = Arc::new(vault());
        let mut prefabs = vxe_world::PrefabRegistry::new();
        prefabs
            .register(vault.clone())
            .expect("an empty registry takes the vault");
        let mut server = Server::with_prefabs(world.clone(), prefabs, logic, seed);
        server.set_support(DEFAULT_SUPPORT, &[]);
        let (near, far) = MemChannel::pair();
        server.attach(far);
        let mut client = Client::shared(world.clone(), near);
        client
            .register_prefab(vault)
            .expect("an empty registry takes the vault");
        let mut net = Self {
            server,
            client,
            stop_printed: false,
        };
        if let Some(e) = net.server.init_error().cloned()
            && let Some(line) = net.logic_failure(&e)
        {
            println!("{line}");
        }
        net
    }

    /// The line to print for the logic's error `e`: every failure and every restart, and
    /// [`LogicError::Stopped`] the first time only.
    pub(crate) fn logic_failure(&mut self, e: &LogicError) -> Option<String> {
        match e {
            LogicError::Stopped { .. } => (!std::mem::replace(&mut self.stop_printed, true))
                .then(|| format!("logic: {e}; the session continues without it")),
            _ => Some(format!("logic: {e}")),
        }
    }
}

/// Bench carves the client has seen this tick, summed until a bench's [`BENCH_CARVES`] are in.
#[derive(Default)]
pub(crate) struct BenchSum {
    pub(crate) carves: usize,
    pub(crate) refused: usize,
    pub(crate) counts: BTreeMap<MaterialId, u32>,
    pub(crate) touched: BTreeSet<ChunkCoord>,
}

/// The error that ended the session, shared by the app, its run and `main`: the first one
/// set is kept, and `main` prints it after the teardown and exits with status 1.
#[derive(Clone, Default)]
pub(crate) struct Failure(Arc<Mutex<Option<String>>>);

impl Failure {
    /// Records `message` unless a failure is already recorded.
    pub(crate) fn set(&self, message: String) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_or_insert(message);
    }

    pub(crate) fn is_some(&self) -> bool {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
    }

    /// The recorded failure, cleared.
    pub(crate) fn take(&self) -> Option<String> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).take()
    }
}

/// The fixed-point centre of the voxel holding `p`.
pub(crate) fn centre_fixed(p: Vec3) -> Fixed3 {
    vxe_world::centre_of_voxel(voxel_of(p))
}

/// The hit record's flag bit (word 3) for a hit: `hitrec.wgsl`'s `HIT_FOUND`.
pub(crate) const HIT_FOUND_BIT: u16 = 1;

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    use crate::cave::{CaveSource, LEVEL_CHUNKS, ROCK, vault};

    use crate::logic::DigInput;
    use crate::logic::STAMP_VERB;
    use crate::now;

    use crate::{LogicChoice, RESIDENCY, ScrollSteps, cave_materials};

    use vxe_net::{Event, LogicError};

    use crate::app::ms_since;
    use crate::cave::SHELL;
    use vxe_world::{
        ChunkCoord, ChunkExtent, CoarseGrid, EditOp, EditResult, FIXED_ONE, Fixed3, MaterialId,
        Mode, OpResult, PersistConfig, StreamConfig, Streamer, WalkableMap, World,
    };

    use crate::run::hud_feed::median;

    use crate::test_support::{applied, native_net, net_tick};

    /// 32 frames of an eighth of a line raise the dig radius from 0.5 m by four steps to
    /// 0.9 m, each step reported once. At 3.0 m further upward scroll reports nothing and
    /// banks nothing, so one line down steps to 2.9 m at once.
    #[test]
    fn smooth_scrolling_steps_the_dig_radius() {
        let mut scroll = ScrollSteps::default();
        let mut r = DIG_RADIUS;
        let mut changes = Vec::new();
        for _ in 0..32 {
            if let Some(n) = scroll_dig_radius(r, &mut scroll, 0.125) {
                r = n;
                changes.push(r);
            }
        }
        assert_eq!(changes, [6, 7, 8, 9].map(|v| v * FIXED_ONE));
        assert_eq!(r, 9 * FIXED_ONE);

        let mut r = 30 * FIXED_ONE;
        for delta in [0.125, 0.5, 1.0, 0.875, 0.125] {
            assert_eq!(scroll_dig_radius(r, &mut scroll, delta), None);
        }
        assert_eq!(scroll.carry(), 0.0);
        r = scroll_dig_radius(r, &mut scroll, -1.0).expect("one line down steps");
        assert_eq!(r, 29 * FIXED_ONE);
    }

    /// Every failure and every restart of the logic prints a line; the stop prints once,
    /// naming that the session goes on, and the `Stopped` reports after it print nothing.
    #[test]
    fn each_failure_and_restart_prints_and_the_stop_prints_once() {
        let world = Arc::new(World::new(&cave_materials()));
        let mut net = Net::new(&world, LogicChoice::Native.load().expect("native"), 0);
        assert_eq!(
            net.logic_failure(&LogicError::Fuel { tick: 3 }).as_deref(),
            Some("logic: logic ran out of fuel at tick 3")
        );
        assert_eq!(
            net.logic_failure(&LogicError::Restarted {
                tick: 4,
                after: 3,
                restarts: 1
            })
            .as_deref(),
            Some(
                "logic: logic restarted from its seed at tick 4 after it failed at tick 3 (restart 1)"
            )
        );
        assert_eq!(
            net.logic_failure(&LogicError::Stopped {
                tick: 12,
                since: 11
            })
            .as_deref(),
            Some(
                "logic: logic not run at tick 12: stopped since it failed at tick 11; the session \
                 continues without it"
            )
        );
        assert_eq!(
            net.logic_failure(&LogicError::Stopped {
                tick: 13,
                since: 11
            }),
            None
        );
    }

    #[test]
    fn an_edit_prints_one_line_with_material_names() {
        let r = EditResult {
            changed_from: vec![(1, 1234), (2, 12)],
            touched: vec![
                ChunkCoord([0, 0, 0]),
                ChunkCoord([1, 0, 0]),
                ChunkCoord([1, 1, 0]),
            ],
            aabb: None,
        };
        assert_eq!(
            edit_line("carved", &r, &cave_materials(), 0.214),
            "carved: rock 1234, ore 12 (3 chunks, 0.21 ms)"
        );
        assert_eq!(
            edit_line("filled", &EditResult::default(), &cave_materials(), 0.0),
            "filled: nothing (0 chunks, 0.00 ms)"
        );
    }

    /// The `K` bookkeeping, scripted: an edit updates the kept grid and walkable map to what
    /// a rebuild from the edited world gives, and records the changed-cell count.
    #[test]
    fn an_edit_keeps_the_coarse_grid_and_walkable_map_current() {
        let world = World::generate(
            ChunkExtent::new([-1, -1, -1], [1, 1, 1]),
            &vxe_world::UniformSource(ROCK),
            &cave_materials(),
        );
        let mut c = Coarse::build(&world);
        assert_eq!(c.map.count(), 0, "a solid world has no walkable cell");
        // A 2 m box carved out of the solid: 27 bricks touched, a floor under open cells.
        let r = world
            .apply(&EditOp::box_(
                Fixed3([-8 * FIXED_ONE, -8 * FIXED_ONE, -8 * FIXED_ONE]),
                Fixed3([16 * FIXED_ONE, 16 * FIXED_ONE, 16 * FIXED_ONE]),
                Mode::Carve,
            ))
            .unwrap();
        c.update(&world, &r);
        let rebuilt = CoarseGrid::build(&world);
        let expected = WalkableMap::build(&rebuilt, WALKABLE);
        assert_eq!(c.last.map(|(m, _)| m), Some(27));
        assert!(expected.count() > 0);
        assert_eq!(c.map.cells(), expected.cells());
        assert!(c.line().starts_with(&format!(
            "coarse: {} walkable cells, last edit changed 27 cells in ",
            expected.count()
        )));
    }

    /// `P` headless: from inside a carved 16-voxel box in solid rock, the ray along +x enters
    /// rock at x = 8 and the vault lands there, stamped by the logic through the server; aimed
    /// so the vault crosses the extent's edge it is refused; aimed out of the world it misses.
    #[test]
    fn p_stamps_the_vault_at_the_aim_point() {
        let table = cave_materials();
        let world = Arc::new(World::generate(
            ChunkExtent::new([-1, -1, -1], [1, 1, 1]),
            &vxe_world::UniformSource(ROCK),
            &table,
        ));
        world
            .apply(&EditOp::box_(
                Fixed3([-8 * FIXED_ONE; 3]),
                Fixed3([8 * FIXED_ONE; 3]),
                Mode::Carve,
            ))
            .unwrap();
        let mut net = native_net(&world);
        let stamp = |origin| DigInput::Stamp {
            origin,
            dir: [FIXED_ONE, 0, 0],
            prefab: vault().id(),
        };
        let centre = Fixed3([FIXED_ONE / 2; 3]);
        let (lines, events) = net_tick(&mut net, &[stamp(centre)]);
        assert_eq!(lines, ["op stamped"]);
        let [OpResult::Edit(r)] = &applied(&events)[..] else {
            panic!("{events:?}");
        };
        assert_eq!(r.changed_from, vec![(ROCK, 4096)]);
        assert_eq!(r.touched, vec![ChunkCoord([0, 0, 0])]);
        assert_eq!(
            world.count_in_box([8, 0, 0], [23, 15, 15]).unwrap(),
            vec![(0, 2744), (SHELL, 1352)]
        );
        assert_eq!(
            world.count_in_box([8, 0, 0], [24, 16, 16]).unwrap(),
            vec![(0, 2744), (ROCK, 817), (SHELL, 1352)]
        );
        assert_eq!(
            edit_line(STAMP_VERB, r, &table, 0.5),
            "stamped: rock 4096 (1 chunks, 0.50 ms)"
        );
        assert!(
            matches!(&net.server.ops().entries()[0], vxe_world::LogEntry::Op { op: vxe_world::Op::Stamp { prefab, .. }, .. } if *prefab == vault().id())
        );

        // From inside rock at x = 51 the ray hits that voxel, and a vault there would reach
        // x = 66, past the resident x <= 63: refused, nothing written.
        let past = Fixed3([51 * FIXED_ONE + FIXED_ONE / 2, FIXED_ONE / 2, FIXED_ONE / 2]);
        let before = world.count_in_box([-64; 3], [63; 3]).unwrap();
        let (lines, events) = net_tick(&mut net, &[stamp(past)]);
        assert_eq!(lines, ["op stamped"]);
        assert!(
            events.iter().any(|e| matches!(
                e,
                Event::Refused { error, .. }
                    if error.to_string() == "chunk [1, 0, 0] is not resident"
            )),
            "{events:?}"
        );
        assert_eq!(world.count_in_box([-64; 3], [63; 3]).unwrap(), before);

        let outside = Fixed3([200 * FIXED_ONE, 0, 0]);
        let (lines, _) = net_tick(&mut net, &[stamp(outside)]);
        assert_eq!(lines, ["stamped: no surface within 10 m"]);
    }

    const PILLAR: ([i32; 3], [i32; 3]) = ([32, 32, 8], [40, 40, 40]);

    /// One chunk of air holding a rock floor, a pillar and a 3 x 3-cell ledge on it.
    fn ledge_world() -> World {
        let world = World::generate(
            ChunkExtent::new([0, 0, 0], [1, 1, 1]),
            &vxe_world::UniformSource(vxe_world::AIR),
            &cave_materials(),
        );
        for (lo, hi) in [
            ([0, 0, 0], [64, 64, 8]),
            PILLAR,
            ([24, 24, 40], [48, 48, 48]),
        ] {
            let f = |v: [i32; 3]| Fixed3(v.map(|c| i64::from(c) * FIXED_ONE));
            world
                .apply(&EditOp::box_(f(lo), f(hi), Mode::Fill(ROCK)))
                .unwrap();
        }
        world
    }

    /// [`ledge_world`] with the pillar's top cell cut to a 32-voxel sliver
    /// `[32, 32, 39]..[36, 40, 40]` of `sliver` (the pillar stays full below z = 32), and an
    /// 8-voxel rock lump `[18, 36, 42]..[20, 38, 44]` in cell `(2, 4, 5)`, beside the ledge
    /// without touching it.
    fn sliver_ledge_world(sliver: MaterialId) -> World {
        let world = ledge_world();
        let f = |v: [i32; 3]| Fixed3(v.map(|c| i64::from(c) * FIXED_ONE));
        for (lo, hi, mode) in [
            ([32, 32, 32], [40, 40, 40], Mode::Carve),
            ([32, 32, 39], [36, 40, 40], Mode::Fill(sliver)),
            ([18, 36, 42], [20, 38, 44], Mode::Fill(ROCK)),
        ] {
            world.apply(&EditOp::box_(f(lo), f(hi), mode)).unwrap();
        }
        world
    }

    /// A dig of radius 2 voxels from the centre of [`sliver_ledge_world`]'s lump: the ray
    /// starts in rock, so the capsule is a sphere carving the lump's 8 voxels.
    fn lump_dig() -> DigInput {
        DigInput::Dig {
            origin: Fixed3([19 * FIXED_ONE, 37 * FIXED_ONE, 43 * FIXED_ONE]),
            dir: [FIXED_ONE, 0, 0],
            radius: 2 * FIXED_ONE,
        }
    }

    /// The query beside the ledge reports it (it rests on a cell of 32 voxels, half the
    /// threshold), and it falls through the sliver: the slab's bottom is z 40, the sliver at
    /// z 39 yields, z 38 to 32 is air and the pillar's full top cell holds z 31. Tick 1 drops 8
    /// and crushes the 32 sliver voxels; tick 2 drops 0 onto the pillar, and the logic piles
    /// the 32 as rubble on the landed slab: `C` = 576 columns, every 18th takes one voxel on
    /// the slab's top at z 39, so all 32 are at z 40.
    #[test]
    fn a_ledge_on_a_sliver_falls_through_it_and_piles_what_it_crushed() {
        let world = Arc::new(sliver_ledge_world(ROCK));
        let mut net = native_net(&world);
        let (lines, events) = net_tick(&mut net, &[lump_dig()]);
        assert_eq!(lines, ["op carved", "region 9 1:4608"]);
        let [OpResult::Edit(r)] = &applied(&events)[..] else {
            panic!("{events:?}");
        };
        assert_eq!(r.changed_from, [(ROCK, 8)]);
        let (lines, events) = net_tick(&mut net, &[]);
        assert_eq!(lines, ["op dropped"]);
        let [OpResult::Drop(d)] = &applied(&events)[..] else {
            panic!("{events:?}");
        };
        assert_eq!((d.dropped, d.crushed.as_slice()), (8, &[(ROCK, 32)][..]));
        let (lines, events) = net_tick(&mut net, &[]);
        assert_eq!(
            lines,
            [
                "op dropped",
                "op piled",
                "landed after 1 ticks, fell 8 voxels, crushed 32 voxels, 32 rubble"
            ]
        );
        let [OpResult::Drop(d), OpResult::Pile(p)] = &applied(&events)[..] else {
            panic!("{events:?}");
        };
        assert_eq!(d.dropped, 0);
        assert_eq!(p.placed, 32);
        assert_eq!(world.material_at([25, 26, 40]), Some(crate::RUBBLE));
        assert_eq!(
            world.count_in_box([24, 24, 32], [47, 47, 39]).unwrap(),
            [(ROCK, 4608)]
        );
        // `count_in_box` counts air too: 576 voxels in the layer, 32 of them rubble.
        assert_eq!(
            world.count_in_box([24, 24, 40], [47, 47, 40]).unwrap(),
            [(vxe_world::AIR, 544), (crate::RUBBLE, 32)]
        );
        let kinds: Vec<u8> = net
            .server
            .ops()
            .entries()
            .iter()
            .filter(|e| e.op().is_some())
            .map(|e| {
                let mut out = Vec::new();
                e.encode(&mut out);
                out[12]
            })
            .collect();
        assert_eq!(kinds, [1, 5, 5, 6]);
        assert_eq!(*net.server.coarse(), CoarseGrid::build(&world));
        // Nothing falls on.
        let (lines, events) = net_tick(&mut net, &[]);
        assert!(
            lines.is_empty() && events.is_empty(),
            "{lines:?} {events:?}"
        );
    }

    /// With the sliver made of the indestructible shell the ledge cannot move: its first tick
    /// prints `wedged`, and the logic remembers it. A second lump dig beside it reports
    /// nothing; a dig that takes voxels from the ledge reports it again with its new counts,
    /// and it is wedged again.
    #[test]
    fn a_ledge_on_a_shell_sliver_is_wedged_and_reported_again_only_once_changed() {
        let world = Arc::new(sliver_ledge_world(SHELL));
        let table = world.materials().clone();
        let mut net = native_net(&world);
        let (lines, _) = net_tick(&mut net, &[lump_dig()]);
        assert_eq!(lines, ["op carved", "region 9 1:4608"]);
        let (lines, events) = net_tick(&mut net, &[]);
        assert_eq!(lines, ["op dropped", "wedged 9 1:4608"]);
        assert_eq!(
            wedged_terminal_line(&table, &lines[1]).as_deref(),
            Some("wedged: 9 cells, {rock: 4608}, cannot fall")
        );
        let [OpResult::Drop(d)] = &applied(&events)[..] else {
            panic!("{events:?}");
        };
        assert_eq!(d.dropped, 0);
        let (lines, events) = net_tick(&mut net, &[]);
        assert!(
            lines.is_empty() && events.is_empty(),
            "{lines:?} {events:?}"
        );

        // A second lump in cell (6, 4, 5), beside the ledge, written past the server (its
        // grid takes the cell back from the dig's aabb): dug, and the ledge the query reports
        // is the remembered one.
        let f = |v: [i32; 3]| Fixed3(v.map(|c| i64::from(c) * FIXED_ONE));
        world
            .apply(&EditOp::box_(
                f([50, 36, 42]),
                f([52, 38, 44]),
                Mode::Fill(ROCK),
            ))
            .unwrap();
        let second = DigInput::Dig {
            origin: Fixed3([51 * FIXED_ONE, 37 * FIXED_ONE, 43 * FIXED_ONE]),
            dir: [FIXED_ONE, 0, 0],
            radius: 2 * FIXED_ONE,
        };
        let (lines, events) = net_tick(&mut net, &[second]);
        assert_eq!(lines, ["op carved"]);
        let [OpResult::Edit(r)] = &applied(&events)[..] else {
            panic!("{events:?}");
        };
        assert_eq!(r.changed_from, [(ROCK, 8)]);
        let (lines, events) = net_tick(&mut net, &[]);
        assert!(
            lines.is_empty() && events.is_empty(),
            "{lines:?} {events:?}"
        );

        // A dig into the ledge's corner changes its counts: reported, and wedged, again.
        let corner = DigInput::Dig {
            origin: Fixed3([25 * FIXED_ONE, 25 * FIXED_ONE, 46 * FIXED_ONE]),
            dir: [FIXED_ONE, 0, 0],
            radius: 2 * FIXED_ONE,
        };
        let (lines, events) = net_tick(&mut net, &[corner]);
        let [OpResult::Edit(r)] = &applied(&events)[..] else {
            panic!("{events:?}");
        };
        let left = 4608 - r.changed_from[0].1;
        assert_eq!(
            lines,
            ["op carved".to_owned(), format!("region 9 1:{left}")]
        );
        let (lines, _) = net_tick(&mut net, &[]);
        assert_eq!(
            lines,
            ["op dropped".to_owned(), format!("wedged 9 1:{left}")]
        );
    }

    /// A ledge with no floor under it in the one-chunk world falls five ticks of 8, z 40 to 0;
    /// on tick 6 the scan reaches chunk `[0, 0, -1]`, outside the extent, and the drop is
    /// refused. It crushed nothing, so no pile follows, and no later tick drops it again.
    #[test]
    fn a_ledge_falling_out_of_the_world_is_refused_once() {
        let world = Arc::new(World::generate(
            ChunkExtent::new([0, 0, 0], [1, 1, 1]),
            &vxe_world::UniformSource(vxe_world::AIR),
            &cave_materials(),
        ));
        let f = |v: [i32; 3]| Fixed3(v.map(|c| i64::from(c) * FIXED_ONE));
        for (lo, hi) in [([24, 24, 40], [48, 48, 48]), ([18, 36, 42], [20, 38, 44])] {
            world
                .apply(&EditOp::box_(f(lo), f(hi), Mode::Fill(ROCK)))
                .unwrap();
        }
        let mut net = native_net(&world);
        let (lines, _) = net_tick(&mut net, &[lump_dig()]);
        assert_eq!(lines, ["op carved", "region 9 1:4608"]);
        for tick in 1..=5 {
            let (lines, _) = net_tick(&mut net, &[]);
            assert_eq!(lines, ["op dropped"], "tick {tick}");
        }
        let (lines, events) = net_tick(&mut net, &[]);
        assert_eq!(
            lines,
            [
                "op dropped",
                "drop refused: chunk [0, 0, -1] is not resident, after 5 ticks, fell 40 voxels"
            ]
        );
        assert!(applied(&events).is_empty(), "{events:?}");
        assert!(
            events.iter().any(|e| matches!(
                e,
                Event::Refused {
                    op: vxe_world::Op::Drop { .. },
                    ..
                }
            )),
            "{events:?}"
        );
        let (lines, events) = net_tick(&mut net, &[]);
        assert!(
            lines.is_empty() && events.is_empty(),
            "{lines:?} {events:?}"
        );
        assert_eq!(
            world.count_in_box([24, 24, 0], [47, 47, 7]).unwrap(),
            [(ROCK, 4608)]
        );
    }

    /// A 1 m dig whose origin is inside the pillar, at its axis 16 voxels up: the ray starts
    /// in rock, so the capsule is a sphere there, cutting the pillar through from z = 15 to
    /// z = 32 and leaving its top cell on the ledge.
    fn pillar_cut() -> DigInput {
        DigInput::Dig {
            origin: Fixed3([36 * FIXED_ONE, 36 * FIXED_ONE, 24 * FIXED_ONE]),
            dir: [FIXED_ONE, 0, 0],
            radius: 10 * FIXED_ONE,
        }
    }

    /// The dig's collapse, scripted through the logic: cutting the pillar reports the ledge
    /// with the pillar's top, which falls a cell a tick and lands on the pillar's stump; every
    /// drop is an op in the log, and the server's grid follows the world.
    #[test]
    fn cutting_the_pillar_drops_the_ledge_a_cell_a_tick() {
        let world = Arc::new(ledge_world());
        let table = world.materials().clone();
        let mut net = native_net(&world);
        let (lines, events) = net_tick(&mut net, &[pillar_cut()]);
        assert_eq!(lines, ["op carved", "region 10 1:5024"]);
        assert_eq!(
            unsupported_line(&table, &lines[1]).as_deref(),
            Some("unsupported: 10 cells, {rock: 5024}")
        );
        assert_eq!(applied(&events).len(), 1);
        let mut dropped = Vec::new();
        for tick in 1..=3 {
            let (lines, events) = net_tick(&mut net, &[]);
            let [OpResult::Drop(d)] = &applied(&events)[..] else {
                panic!("tick {tick}: {events:?}");
            };
            dropped.push(d.dropped);
            if tick < 3 {
                assert_eq!(lines, ["op dropped"], "tick {tick}");
            } else {
                assert_eq!(
                    lines,
                    ["op dropped", "landed after 3 ticks, fell 18 voxels"]
                );
            }
        }
        assert_eq!(dropped, [8, 8, 2]);
        // Nothing falls on.
        let (lines, events) = net_tick(&mut net, &[]);
        assert!(
            lines.is_empty() && events.is_empty(),
            "{lines:?} {events:?}"
        );
        assert_eq!(net.server.ops().len(), 4);
        assert_eq!(
            world.count_in_box([24, 24, 22], [47, 47, 29]).unwrap(),
            [(ROCK, 4608)]
        );
        assert_eq!(*net.server.coarse(), CoarseGrid::build(&world));
    }

    /// `R`: the ledge is removed instead, a quarter of it piled as rubble.
    #[test]
    fn with_r_the_ledge_is_removed_leaving_a_quarter_as_rubble() {
        let world = Arc::new(ledge_world());
        let table = world.materials().clone();
        let mut net = native_net(&world);
        let (lines, _) = net_tick(&mut net, &[DigInput::Crumble]);
        assert_eq!(lines, ["collapse: remove unsupported regions with rubble"]);
        let (lines, events) = net_tick(&mut net, &[pillar_cut()]);
        assert_eq!(lines, ["op carved", "region 10 1:5024", "op removed"]);
        let [_, OpResult::Remove(r)] = &applied(&events)[..] else {
            panic!("{events:?}");
        };
        assert_eq!(r.removed, [(ROCK, 5024)]);
        assert_eq!(r.placed, 5024 / 4);
        assert_eq!(
            removed_line(&table, r),
            "removed: {rock: 5024}, 1256 rubble"
        );
        assert_eq!(
            world
                .count_in_box([0, 0, 0], [63, 63, 63])
                .unwrap()
                .into_iter()
                .find(|&(m, _)| m == crate::RUBBLE),
            Some((crate::RUBBLE, 1256))
        );
        // Nothing falls after a removal.
        let (lines, _) = net_tick(&mut net, &[]);
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(*net.server.coarse(), CoarseGrid::build(&world));
        let (lines, _) = net_tick(&mut net, &[DigInput::Crumble]);
        assert_eq!(lines, ["collapse: drop unsupported regions"]);
    }

    /// Readings: measurement tests that print numbers and assert nothing a tier runs. Each is
    /// `#[ignore]`d and run by hand, `cargo nextest run -p flythrough --lib --run-ignored only
    /// -E 'test(=edits::tests::reading::<name>)'`; the soak and full tiers leave this module out
    /// by its name.
    mod reading {
        use super::*;

        /// The support query after digs from the spawn of the seed-1 level, one along each axis
        /// at the default radius, through the logic, printed. Run with `cargo nextest run --release
        /// -p flythrough --lib --run-ignored only --no-capture -E
        /// 'test(=edits::tests::reading::a_dig_in_the_level_prints_the_query)'`.
        #[test]
        #[ignore = "a reading on the generated level, run in release"]
        fn a_dig_in_the_level_prints_the_query() {
            let source = CaveSource::new(1, ChunkExtent::new([0, 0, 0], LEVEL_CHUNKS));
            let world = Arc::new(World::generate(source.extent, &source, &cave_materials()));
            let mut net = native_net(&world);
            let origin = vxe_world::centre_of_voxel(source.spawn());
            let f = FIXED_ONE;
            for dir in [
                [f, 0, 0],
                [-f, 0, 0],
                [0, f, 0],
                [0, -f, 0],
                [0, 0, f],
                [0, 0, -f],
            ] {
                let t = now();
                let (lines, _) = net_tick(
                    &mut net,
                    &[DigInput::Dig {
                        origin,
                        dir,
                        radius: DIG_RADIUS,
                    }],
                );
                println!("dig {dir:?}: tick {:.3} ms; {lines:?}", ms_since(t));
            }
        }

        /// The cost of one `World::stats` call on the streamed cave at its spawn after full
        /// residency, median of 10, which sets [`HUD_STATS_EVERY`]. Run by hand in release:
        /// `cargo nextest run --release -p flythrough --lib --run-ignored only -E
        /// 'test(=edits::tests::reading::world_stats_call_on_the_streamed_world_at_full_residency)'`.
        #[test]
        #[ignore = "a timing reading, run by hand in release"]
        fn world_stats_call_on_the_streamed_world_at_full_residency() {
            let source = CaveSource::unbounded(1);
            let spawn = source.spawn();
            let world = World::new(&cave_materials());
            let workers = std::thread::available_parallelism()
                .map_or(1, usize::from)
                .saturating_sub(1)
                .max(1);
            let mut streamer = Streamer::new(
                Arc::new(source),
                StreamConfig {
                    residency: RESIDENCY,
                    workers,
                    persist: PersistConfig::default(),
                },
            );
            loop {
                streamer.update(&world, &[spawn]);
                let _ = streamer.poll(&world);
                let st = streamer.stats();
                if st.ticks > 1 && st.in_flight == 0 && st.queued == 0 {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            let mut ms: Vec<f64> = (0..10)
                .map(|_| {
                    let t = now();
                    std::hint::black_box(world.stats());
                    ms_since(t)
                })
                .collect();
            let s = world.stats();
            let st = streamer.stats();
            println!(
                "world stats call: median {:.3} ms of 10 (min {:.3}, max {:.3}); {} chunks, {} full bricks; streamer {} full, {} lod",
                median(&mut ms),
                ms[0],
                ms[9],
                s.chunks,
                s.full_bricks,
                st.resident_full,
                st.resident_lod
            );
        }
    }
}
