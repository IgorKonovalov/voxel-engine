//! Plan 0079 Phase 4: the wander with no device. Eight walkers and eight spiders of the
//! committed library wander a hand-built closed room for 1500 ticks (25 s at 60 Hz): every
//! tick's poses hash to a constant blessed on Linux, nothing embeds, the walkers stay inside,
//! the spiders stay attached and cover several faces; a dig under a body drops it, wander on
//! or off.

use std::path::Path;

use flythrough::entities::Walkers;
use flythrough::entities::library;
use flythrough::entities::mover::{Mover, Stance};
use flythrough::viewer::material;
use flythrough::{showcase_materials, walker};
use vxe_entity::{EntityId, Pose};
use vxe_sim::SurfaceEvent;
use vxe_world::hash::Fnv1a;
use vxe_world::{
    AIR, ChunkExtent, EditOp, FIXED_ONE, Fixed3, FnSource, MaterialTable, Mode, World,
};

/// The room's ticks: 25 s at 60 Hz.
const TICKS: u32 = 1500;

/// The showcase's table with the walker's rows, the table the library builds under.
fn table() -> MaterialTable {
    walker::with_walker(&showcase_materials())
}

/// Whether voxel `p` is solid in the room: a `64^3` box of rock `[0, 64)^3` whose interior
/// `[1, 63)^3` is air, standing on and under more rock (`z < 0` and `z >= 64`) so a dig through
/// its floor or ceiling finds rock; two pillars floor to ceiling (`[18, 24) x [18, 24)` and
/// `[40, 46) x [34, 40)`) and a step 4 voxels high over `y >= 50`.
fn solid(p: [i32; 3]) -> bool {
    let inside = |lo: [i32; 3], hi: [i32; 3]| (0..3).all(|k| p[k] >= lo[k] && p[k] < hi[k]);
    if !inside([1, 1, 1], [63, 63, 63]) {
        return true;
    }
    inside([18, 18, 1], [24, 24, 63])
        || inside([40, 34, 1], [46, 40, 63])
        || inside([1, 50, 1], [63, 63, 5])
}

/// The room under `table`.
fn room(table: &MaterialTable) -> World {
    let rock = material(table, "rock").expect("a rock row");
    World::generate(
        ChunkExtent::new([0, 0, -1], [1, 1, 2]),
        &FnSource(|p: [i32; 3]| if solid(p) { rock } else { AIR }),
        table,
    )
}

/// The committed library built under `table`, the wander's seed 1.
fn walkers(table: &MaterialTable) -> Walkers {
    let lib = library::scan(Path::new(library::DEFAULT_DIR));
    let mut w = Walkers::default().with_library(lib).with_seed(1);
    assert!(w.library.build(table).is_empty(), "the library builds");
    w
}

fn entry(w: &Walkers, name: &str) -> usize {
    w.library
        .entries
        .iter()
        .position(|e| e.name == name)
        .expect("the entry")
}

/// SplitMix64, for the placements' points.
struct Points(u64);

impl Points {
    fn draw(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A floor point (world voxels, on the floor's top at z = 1) at least 8 voxels from the
    /// walls, the pillars and the step, so a body placed there stands clear.
    fn floor_point(&mut self) -> [f32; 3] {
        loop {
            let x = 9 + (self.draw() % 46) as i32;
            let y = 9 + (self.draw() % 33) as i32;
            let clear = (-8..=8).all(|dx| (-8..=8).all(|dy| !solid([x + dx, y + dy, 2])));
            if clear {
                return [x as f32 + 0.5, y as f32 + 0.5, 1.0];
            }
        }
    }
}

/// The room with 8 walkers and 8 spiders placed on seeded floor points, each facing a seeded
/// way, the wander on.
fn course() -> (World, MaterialTable, Walkers) {
    let t = table();
    let world = room(&t);
    let mut w = walkers(&t);
    let (wi, si) = (entry(&w, "walker"), entry(&w, "spider"));
    let mut pts = Points(79);
    for k in 0..16 {
        let i = if k < 8 { wi } else { si };
        let yaw = (pts.draw() % 360) as f32;
        let stance = Stance::floor(pts.floor_point(), yaw.to_radians());
        w.place_at(&t, Some(i), stance).expect("placed");
    }
    assert!(w.placed.iter().all(|p| p.mover.is_some()));
    w.walking = true;
    (world, t, w)
}

fn feed_pose(h: &mut Fnv1a, p: &Pose) {
    for v in p.root.0 {
        h.write(&v.to_le_bytes());
    }
    for v in p.root_q.0 {
        h.write(&v.to_le_bytes());
    }
    for pp in &p.parts {
        for v in pp.t {
            h.write(&v.to_le_bytes());
        }
        for v in pp.q.0 {
            h.write(&v.to_le_bytes());
        }
    }
}

fn mover(w: &Walkers, id: EntityId) -> &Mover {
    let p = w.placed.iter().find(|p| p.id == id).expect("placed");
    p.mover.as_ref().expect("a mover")
}

/// Done-when 1: every tick's poses over the room's course hash to the pinned constant.
#[test]
fn the_room_course_hashes_to_the_pinned_constant() {
    let (world, _, mut w) = course();
    let mut h = Fnv1a::default();
    for _ in 0..TICKS {
        w.tick(&world);
        for p in &w.placed {
            feed_pose(&mut h, &p.mover.as_ref().expect("a mover").now);
        }
    }
    assert_eq!(h.finish(), 0x93d7_6d32_b893_7835, "the room course's hash");
}

/// Done-when 2: over the same course no tick embeds a body, every walker's foot stays inside
/// the room's interior, every spider is attached on at least 95 % of ticks, and the spiders
/// between them hold at least three distinct axis normals.
#[test]
fn the_wander_stays_in_the_room_and_never_embeds() {
    let (world, _, mut w) = course();
    let mut attached = [0u32; 8];
    let mut normals = std::collections::BTreeSet::new();
    for tick in 0..TICKS {
        for (k, (id, s)) in w.tick(&world).into_iter().enumerate() {
            assert!(!s.embedded, "tick {tick}: body {k} embedded");
            let m = mover(&w, id);
            if let Some(g) = m.ground() {
                let f = g.foot.0;
                let lo = FIXED_ONE;
                let hi = 63 * FIXED_ONE;
                assert!(
                    (0..3).all(|c| f[c] >= lo && f[c] < hi),
                    "tick {tick}: walker {k} at {f:?}"
                );
            }
            if let Some(st) = m.surface() {
                if st.attached {
                    attached[k - 8] += 1;
                }
                let n = st.normal;
                if n.iter().filter(|&&c| c == 0).count() == 2 {
                    normals.insert(n);
                }
            }
        }
    }
    for (k, a) in attached.iter().enumerate() {
        assert!(a * 100 >= TICKS * 95, "spider {k} attached {a} of {TICKS}");
    }
    assert!(normals.len() >= 3, "the spiders' axis normals: {normals:?}");
}

/// The carve of a sphere of radius 1 m (10 voxels) at `at` (fixed units).
fn dig(world: &World, at: [i64; 3]) {
    world
        .apply(&EditOp::sphere(Fixed3(at), 10 * FIXED_ONE, Mode::Carve))
        .expect("resident");
}

/// Done-when 5: in the room a carve of radius 1 m under a walker's foot is followed by a tick
/// that reads it not grounded and within 60 ticks a landing; a carve of the ceiling patch a
/// spider stands on is followed by `Detached`, then `Landed`, then the spider attached with
/// normal `+z`. The same with the wander on and off (the movers tick either way).
#[test]
fn a_dig_under_a_wandering_walker_drops_it() {
    for wandering in [true, false] {
        let (world, t, mut w) = course();
        w.walking = wandering;
        for _ in 0..30 {
            w.tick(&world);
        }
        // The first walker, standing on the room's floor.
        let id = w.placed[0].id;
        let foot = mover(&w, id).ground().expect("a walker").foot.0;
        assert_eq!(foot[2], FIXED_ONE, "on the floor");
        dig(&world, foot);
        let steps = w.tick(&world);
        let s = steps.iter().find(|(i, _)| *i == id).expect("ticked").1;
        assert!(!s.on_surface, "wander {wandering}: grounded after the dig");
        let landed = (0..60).any(|_| {
            let steps = w.tick(&world);
            steps.iter().any(|(i, s)| *i == id && s.landed.is_some())
        });
        assert!(landed, "wander {wandering}: no landing within 60 ticks");

        // A spider on the ceiling over a clear patch of floor (20 voxels or more from the
        // walker's crater), its legs to the ceiling.
        let si = entry(&w, "spider");
        let far = |p: &&[f32; 2]| {
            let d = [0, 1].map(|k| p[k] * 256.0 - foot[k] as f32);
            (d[0] * d[0] + d[1] * d[1]).sqrt() >= 20.0 * 256.0
        };
        let [x, y] = *[[12.5, 12.5], [52.5, 12.5], [30.5, 30.5]]
            .iter()
            .find(far)
            .expect("a clear patch");
        let up = Stance {
            point: [x, y, 63.0],
            up: [0.0, 0.0, -1.0],
            forward: [0.0, -1.0, 0.0],
            yaw: 0.0,
        };
        let sid = w.place_at(&t, Some(si), up).expect("placed");
        let s = w.tick(&world);
        let s = s.iter().find(|(i, _)| *i == sid).expect("ticked").1;
        assert!(
            s.on_surface,
            "wander {wandering}: the spider holds the ceiling"
        );
        let c = mover(&w, sid).surface().expect("a spider").centre.0;
        dig(&world, [c[0], c[1], 63 * FIXED_ONE]);
        let mut events = Vec::new();
        for _ in 0..240 {
            let steps = w.tick(&world);
            let s = steps.iter().find(|(i, _)| *i == sid).expect("ticked").1;
            if let Some(e) = s.event {
                events.push(e);
            }
            if matches!(s.event, Some(SurfaceEvent::Landed(_))) {
                break;
            }
        }
        assert_eq!(events.first(), Some(&SurfaceEvent::Detached), "{events:?}");
        assert!(
            matches!(events.last(), Some(SurfaceEvent::Landed(_))),
            "wander {wandering}: {events:?}"
        );
        let st = mover(&w, sid).surface().expect("a spider");
        assert!(st.attached);
        assert_eq!(st.normal, [0, 0, 256]);
    }
}

mod reading {
    use super::*;

    /// The movers' CPU a tick (`Walkers::tick`: every mover, its probe and its pose) with 64
    /// walkers, then 64 spiders, wandering the room from seeded floor points: 300 ticks to
    /// spread out, then the median, p99 and max of 600 ticks timed one by one. Run by hand,
    /// release: `cargo nextest run --release -p flythrough --run-ignored only -E
    /// 'test(=reading::the_tick_cost_at_64)' --no-capture`.
    #[test]
    #[ignore = "reading"]
    fn the_tick_cost_at_64() {
        for name in ["walker", "spider"] {
            let t = table();
            let world = room(&t);
            let mut w = walkers(&t);
            let i = entry(&w, name);
            let mut pts = Points(64);
            for _ in 0..64 {
                let yaw = (pts.draw() % 360) as f32;
                let stance = Stance::floor(pts.floor_point(), yaw.to_radians());
                w.place_at(&t, Some(i), stance).expect("placed");
            }
            w.walking = true;
            for _ in 0..300 {
                w.tick(&world);
            }
            let mut us: Vec<f64> = (0..600)
                .map(|_| {
                    let at = flythrough::now();
                    w.tick(&world);
                    (flythrough::now() - at).as_secs_f64() * 1e6
                })
                .collect();
            us.sort_by(f64::total_cmp);
            println!(
                "{name} x64 tick: p50 {:.1} us, p99 {:.1} us, max {:.1} us over 600 ticks",
                us[299], us[593], us[599]
            );
        }
    }
}
