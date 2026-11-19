//! The pinned courses: the controller and the surface mover over F1 to F6, hashed every tick
//! with FNV-1a 64. The constants are blessed on Linux and asserted on every CI arm; they are
//! never loosened.

mod support;

use support::*;
use vxe_sim::*;
use vxe_world::hash::Fnv1a;
use vxe_world::{EditOp, Fixed3, Mode};

fn feed(h: &mut Fnv1a, s: &CharacterState, r: &MoveResult) {
    for v in s.foot.0 {
        h.write(&v.to_le_bytes());
    }
    h.write(&s.vz.to_le_bytes());
    h.write(&[
        u8::from(s.grounded),
        u8::from(r.blocked),
        u8::from(r.stepped),
        u8::from(r.climbed),
        u8::from(s.is_climbing()),
    ]);
    match r.climb_available {
        Some(f) => {
            h.write(&[1]);
            for v in f.0 {
                h.write(&v.to_le_bytes());
            }
        }
        None => h.write(&[0]),
    }
    match r.landed {
        Some(v) => {
            h.write(&[1]);
            h.write(&v.to_le_bytes());
        }
        None => h.write(&[0]),
    }
}

/// A fixture's name, world, start and per-tick input.
type Course = (
    &'static str,
    vxe_world::World,
    [i64; 3],
    Box<dyn FnMut(u32) -> MoveInput>,
);

#[test]
fn the_course_hashes_to_the_pinned_constant() {
    let walk_in = |climb: bool| {
        move |_t: u32| MoveInput {
            wish: WISH,
            jump: false,
            climb,
        }
    };
    let mut h = Fnv1a::default();
    let mut per_fixture = Vec::new();
    let courses: [Course; 6] = [
        (
            "F1",
            f1(),
            S,
            Box::new(|t| MoveInput {
                wish: WISH,
                jump: t % 60 == 1,
                climb: false,
            }),
        ),
        ("F2", f2(), S, Box::new(walk_in(false))),
        ("F3", f3(), S, Box::new(walk_in(false))),
        ("F4", f4(), S, Box::new(walk_in(true))),
        ("F5", f5(), F5_START, Box::new(walk_in(true))),
        ("F6", f6(), F6_START, Box::new(walk_in(false))),
    ];
    for (name, world, start, input) in courses {
        let steps = run(&world, start, 300, input);
        let mut one = Fnv1a::default();
        for st in &steps {
            feed(&mut h, &st.s, &st.r);
            feed(&mut one, &st.s, &st.r);
        }
        per_fixture.push((name, one.finish(), steps.last().unwrap().s.foot.0));
    }
    for p in &per_fixture {
        eprintln!("{p:?}");
    }
    assert_eq!(
        h.finish(),
        0x0c80_8031_d14f_27eb,
        "course hash {:#018x}",
        h.finish()
    );
}

fn feed_surface(h: &mut Fnv1a, s: &SurfaceState, r: &SurfaceResult) {
    for v in s
        .centre
        .0
        .iter()
        .chain(&s.normal)
        .chain(&s.smooth_normal)
        .chain(&s.forward)
    {
        h.write(&v.to_le_bytes());
    }
    h.write(&s.vz.to_le_bytes());
    h.write(&[u8::from(s.attached), u8::from(r.embedded)]);
    for v in r.moved.0.iter().chain(&r.lifted.0) {
        h.write(&v.to_le_bytes());
    }
    let (tag, speed) = match r.event {
        None => (0u8, 0),
        Some(SurfaceEvent::Concave) => (1, 0),
        Some(SurfaceEvent::Convex) => (2, 0),
        Some(SurfaceEvent::Detached) => (3, 0),
        Some(SurfaceEvent::Landed(v)) => (4, v),
    };
    h.write(&[tag]);
    h.write(&speed.to_le_bytes());
    h.write(&(r.contacts.len() as u64).to_le_bytes());
    for c in &r.contacts {
        for v in c.voxel.0 {
            h.write(&v.to_le_bytes());
        }
        for v in c.normal {
            h.write(&v.to_le_bytes());
        }
    }
}

/// A fixture's name, world and the crawler's start centre.
type SurfaceStart = (&'static str, fn() -> vxe_world::World, [i64; 3]);

/// The surface course: a crawler on each of F1 to F6, 600 ticks each, at a quarter voxel a
/// tick, turning left and right in 30-tick halves; at tick 300 a sphere of 3 voxels is carved
/// under it; the smoothed normal is on. Every tick's state and result is hashed with FNV-1a
/// 64; the constant is blessed on Linux and asserted on every CI arm.
#[test]
fn the_surface_course_hashes_to_the_pinned_constant() {
    let starts: [SurfaceStart; 6] = [
        ("F1", f1, [2688, 2688, 256]),
        ("F2", f2, [2688, 2688, 256]),
        ("F3", f3, [6528, 2688, 256]),
        ("F4", f4, [6528, 2688, 256]),
        ("F5", f5, [13952, 13952, -1792]),
        ("F6", f6, [12928, 2688, 256]),
    ];
    let m = SurfaceMover::new(SurfaceParams {
        smooth: true,
        ..surface_params()
    });
    let mut h = Fnv1a::default();
    let mut per_fixture = Vec::new();
    for (name, make, start) in starts {
        let world = make();
        let mut st = SurfaceState::new(Fixed3(start), [0, 0, 256], [256, 0, 0]);
        let mut one = Fnv1a::default();
        for t in 1..=600u32 {
            if t == 300 {
                let under = [0, 1, 2].map(|k| st.centre.0[k] - st.normal[k]);
                world
                    .apply(&EditOp::sphere(Fixed3(under), 768, Mode::Carve))
                    .expect("the carve applies");
            }
            let sin = if t % 60 < 30 { 16 } else { -16 };
            let input = SurfaceInput {
                heading: [255, sin],
                speed: 64,
            };
            let r = m.tick(&world, &mut st, &input);
            feed_surface(&mut h, &st, &r);
            feed_surface(&mut one, &st, &r);
        }
        per_fixture.push((name, one.finish(), st.centre.0, st.attached));
    }
    for p in &per_fixture {
        eprintln!("{p:?}");
    }
    assert_eq!(
        h.finish(),
        0x0a07_c86c_51f9_8d3c,
        "surface course hash {:#018x}",
        h.finish()
    );
}
