//! The pinned course: F1 to F6, 300 ticks each, hashed every tick with FNV-1a 64. The
//! constant is blessed on Linux and asserted on every CI arm; it is never loosened.

mod support;

use support::*;
use vxe_sim::*;
use vxe_world::hash::Fnv1a;

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
