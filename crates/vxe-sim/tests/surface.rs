//! The surface mover's contracts: rounding convex and concave edges, falling when the
//! surface is dug away, landing, depenetration, and the probe ahead. `run_surface` asserts the
//! sphere never overlaps the world after any tick.

mod support;

use support::*;
use vxe_sim::*;
use vxe_world::*;

fn mover() -> SurfaceMover {
    SurfaceMover::new(surface_params())
}

/// Ticks (counting from 1) whose event is `e`.
fn ticks_of(steps: &[SurfaceStep], e: SurfaceEvent) -> Vec<u32> {
    (1..)
        .zip(steps)
        .filter(|(_, s)| s.r.event == Some(e))
        .map(|(t, _)| t)
        .collect()
}

/// The runs of equal normal, in order: `(normal, first tick, length)`.
fn normal_runs(steps: &[SurfaceStep]) -> Vec<([i64; 3], u32, u32)> {
    let mut runs: Vec<([i64; 3], u32, u32)> = Vec::new();
    for (t, s) in (1..).zip(steps) {
        match runs.last_mut() {
            Some((n, _, len)) if *n == s.s.normal => *len += 1,
            _ => runs.push((s.s.normal, t, 1)),
        }
    }
    runs
}

/// The runs of an axis normal, in order: `(normal, length)`.
fn axis_runs(steps: &[SurfaceStep]) -> Vec<([i64; 3], u32)> {
    normal_runs(steps)
        .into_iter()
        .filter(|(n, _, _)| n.iter().filter(|c| **c != 0).count() == 1)
        .map(|(n, _, len)| (n, len))
        .collect()
}

/// Carves the voxel box `[lo, hi)`.
fn carve(world: &World, lo: [i64; 3], hi: [i64; 3]) {
    world
        .apply(&EditOp::box_(
            Fixed3(lo.map(|v| v * 256)),
            Fixed3(hi.map(|v| v * 256)),
            Mode::Carve,
        ))
        .expect("the carve applies");
}

/// The walking skeleton: a crawler on the cube rounds its four edges in the xz plane, one
/// lap and a half in 400 ticks, never leaving the plane `y = 24` and never letting go.
#[test]
fn a_crawler_rounds_the_cube() {
    let world = cube();
    let mut st = SurfaceState::new(Fixed3([6144, 6144, 8448]), [0, 0, 256], [256, 0, 0]);
    let steps = run_surface(&world, &mover(), &mut st, 400, crawl(64));
    // Between the axis runs the normal is diagonal: the edges being rounded.
    let runs = axis_runs(&steps);
    let normals: Vec<_> = runs.iter().map(|(n, _)| *n).collect();
    assert_eq!(
        normals[..5],
        [
            [0, 0, 256],
            [256, 0, 0],
            [0, 0, -256],
            [-256, 0, 0],
            [0, 0, 256]
        ]
    );
    assert!(runs[1..5].iter().all(|(_, len)| *len >= 40), "{runs:?}");
    assert!(
        steps
            .iter()
            .all(|s| s.s.centre.0[1] == 6144 && s.s.attached)
    );
    eprintln!("convex ticks {:?}", ticks_of(&steps, SurfaceEvent::Convex));
}

/// A crawler walking into F2's wall takes the wall's normal, climbs it, rounds its top and
/// comes down the far side onto the floor.
#[test]
fn a_crawler_climbs_the_wall_it_runs_into() {
    let world = f2();
    let mut st = SurfaceState::new(Fixed3([2560, 2560, 256]), [0, 0, 256], [256, 0, 0]);
    let steps = run_surface(&world, &mover(), &mut st, 600, crawl(64));
    let concave = ticks_of(&steps, SurfaceEvent::Concave);
    // Tick 36's sweep ends touching the wall having spent its delta, which reports no contact;
    // tick 37's sweep is stopped at once.
    assert_eq!(concave[0], 37);
    let at = |t: u32| &steps[t as usize - 1].s;
    assert_eq!(at(37).normal, [-256, 0, 0]);
    assert_eq!(at(37).forward, [0, 0, 256]);
    // The event tick slides the rest of its move up the wall: 100 moving ticks, 37 to 136.
    assert_eq!(at(37 + 99).centre, Fixed3([4864, 2560, 6656]));
    let runs = normal_runs(&steps);
    let top = runs
        .iter()
        .position(|(n, t, len)| *n == [0, 0, 256] && *t > 37 && *len >= 4)
        .expect("the wall's top");
    assert!(runs[top + 1..].iter().any(|(n, _, _)| *n == [256, 0, 0]));
    let end = at(600);
    assert_eq!(end.normal, [0, 0, 256]);
    assert_eq!(end.centre.0[2], 256);
    assert!(end.centre.0[0] > 22 * 256 + 256 && end.attached);
}

/// A crawler under the cube follows its ceiling up one voxel when the bottom layer is dug,
/// then falls to the floor when the whole cube goes, and lands.
#[test]
fn a_crawler_falls_when_its_ceiling_is_dug_away() {
    let world = cube_over_floor();
    let m = mover();
    let mut st = SurfaceState::new(Fixed3([6144, 6144, 3840]), [0, 0, -256], [256, 0, 0]);
    carve(&world, [16, 16, 16], [32, 32, 17]);
    let one = run_surface(&world, &m, &mut st, 1, crawl(0));
    assert_eq!(st.centre, Fixed3([6144, 6144, 4096]));
    assert!(st.attached && st.normal == [0, 0, -256]);
    assert_eq!(one[0].r.event, None);
    carve(&world, [16, 16, 17], [32, 32, 32]);
    let steps = run_surface(&world, &m, &mut st, 30, crawl(0));
    assert_eq!(steps[0].r.event, Some(SurfaceEvent::Detached));
    // Falling ticks count from the detach, which falls too: after `k` the body has dropped
    // `8 k (k + 1)`; 21 drop 3696 and 22 reach the floor's 3840.
    assert_eq!(steps[20].s.centre.0[2], 4096 - 3696);
    assert_eq!(steps[21].r.event, Some(SurfaceEvent::Landed(352)));
    let landed = &steps[21].s;
    assert!(landed.attached && landed.normal == [0, 0, 256]);
    assert_eq!(landed.centre, Fixed3([6144, 6144, 256]));
    assert!(steps[22..].iter().all(|s| s.s.centre == landed.centre));
}

/// A body started inside solid is lifted by the first free candidate, or stays embedded.
#[test]
fn a_crawler_never_overlaps_solid() {
    let world = cube();
    let m = mover();
    // Half a voxel into the top face: the first candidate, one voxel up, is free.
    let mut st = SurfaceState::new(Fixed3([6144, 6144, 8320]), [0, 0, 256], [256, 0, 0]);
    let r = m.tick(&world, &mut st, &SurfaceInput::default());
    assert_eq!(r.lifted, Fixed3([0, 0, 256]));
    assert!(!r.embedded && !sphere_overlaps(&world, &m.sphere(st.centre.0)));
    // At the cube's centre no candidate within two radii is free.
    let mut st = SurfaceState::new(Fixed3([6144, 6144, 6144]), [0, 0, 256], [256, 0, 0]);
    let r = m.tick(&world, &mut st, &crawl(64)(1));
    assert!(r.embedded);
    assert_eq!(r.moved, Fixed3::default());
    assert_eq!(st.centre, Fixed3([6144, 6144, 6144]));
}

#[test]
fn the_probe_reads_the_wall_and_the_drop() {
    let (fwd, up) = ([256, 0, 0], [0, 0, 256]);
    let a = probe_ahead(&f2(), Fixed3([2688, 2688, 128]), fwd, up, 3840, 2560);
    assert_eq!(
        a,
        Ahead {
            wall: Some(2432),
            floor: Some(128)
        }
    );
    let pit = f5();
    let from = Fixed3([11648, 13952, 128]);
    let a = probe_ahead(&pit, from, fwd, up, 2048, 2560);
    assert_eq!(
        a,
        Ahead {
            wall: None,
            floor: Some(2176)
        }
    );
    let a = probe_ahead(&pit, from, fwd, up, 2048, 2048);
    assert_eq!(
        a,
        Ahead {
            wall: None,
            floor: None
        }
    );
}

/// The committed vector of direction pairs `vxe-entity`'s `Quat::shortest_arc` shares: 56
/// seeded pairs and eight special ones (equal, antiparallel, axis, zero).
fn arc_pairs() -> Vec<([i64; 3], [i64; 3])> {
    let mut s = 0x0079_A5C0u64;
    let mut next = || {
        s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((s >> 33) % 513) as i64 - 256
    };
    let mut out: Vec<_> = (0..56)
        .map(|_| ([next(), next(), next()], [next(), next(), next()]))
        .collect();
    out.extend([
        ([0, 0, 256], [0, 0, 256]),
        ([0, 0, 256], [0, 0, -256]),
        ([181, 0, 181], [-181, 0, -181]),
        ([0, 0, 256], [256, 0, 0]),
        ([-256, 0, 0], [0, 256, 0]),
        ([3, -7, 11], [-3, 7, -11]),
        ([0, 0, 0], [0, 0, 256]),
        ([1 << 30, 5, 0], [0, 1 << 28, 3]),
    ]);
    out
}

#[test]
fn the_sim_and_entity_arcs_agree() {
    let mut h = vxe_world::hash::Fnv1a::default();
    for (a, b) in arc_pairs() {
        for c in shortest_arc(a, b) {
            h.write(&c.to_le_bytes());
        }
    }
    // The same constant is pinned in `vxe-entity`'s `tests/quat.rs`.
    assert_eq!(
        h.finish(),
        0x08ab_b932_e90a_e5ce,
        "arc hash {:#018x}",
        h.finish()
    );
}

/// A one-voxel crawler on F6's one-voxel staircase leans with the slope: resting on the tread
/// at x = 70 its contact normal is the tread's `(0, 0, 256)`, while its smoothed normal (the
/// gradient `(-6, 0, 6)` at `half` 1) is the staircase's 45 degrees. With `smooth` off the
/// smoothed normal is the contact normal.
#[test]
fn a_crawler_on_the_staircase_leans_with_it() {
    let world = f6();
    let params = SurfaceParams {
        radius: 128,
        attach_reach: 512,
        smooth: true,
        ..surface_params()
    };
    // Over the tread at x = 70 (its top at z = 10 voxels), a little clear of it; one tick
    // attaches it.
    let start = SurfaceState::new(Fixed3([18048, 2688, 2760]), [0, 0, 256], [256, 0, 0]);
    let mut on = start;
    SurfaceMover::new(params).tick(&world, &mut on, &SurfaceInput::default());
    assert!(on.attached, "{on:?}");
    assert_eq!(on.normal, [0, 0, 256], "{on:?}");
    let s = on.smooth_normal;
    assert!(
        (s[0] + 181).abs() <= 3 && s[1] == 0 && (s[2] - 181).abs() <= 3,
        "{s:?}"
    );
    let mut off = start;
    SurfaceMover::new(SurfaceParams {
        smooth: false,
        ..params
    })
    .tick(&world, &mut off, &SurfaceInput::default());
    assert!(off.attached, "{off:?}");
    assert_eq!(off.smooth_normal, off.normal);
}
