//! The frame and the arc: `Quat::from_frame` turns a forward and an up into a pose's
//! rotation, `Quat::shortest_arc` turns one direction into another, both in integers.

use vxe_entity::Quat;
use vxe_world::hash::Fnv1a;

fn near(q: Quat, want: [i16; 4], tol: i16) -> bool {
    (0..4).all(|k| (q.0[k] - want[k]).abs() <= tol)
}

#[test]
fn the_rest_frame_is_the_identity() {
    assert_eq!(
        Quat::from_frame([0, -256, 0], [0, 0, 256]),
        Some(Quat([16384, 0, 0, 0]))
    );
    // A quarter turn about z: facing +x.
    let q = Quat::from_frame([256, 0, 0], [0, 0, 256]).unwrap();
    assert!(near(q, [11585, 0, 0, 11585], 1), "{q:?}");
    // Lying on the +x wall: a quarter turn about y.
    let q = Quat::from_frame([0, -256, 0], [256, 0, 0]).unwrap();
    assert!(near(q, [11585, 0, 11585, 0], 1), "{q:?}");
    // The forward is made orthogonal to the up.
    assert_eq!(
        Quat::from_frame([256, 0, 256], [0, 0, 256]),
        Quat::from_frame([256, 0, 0], [0, 0, 256])
    );
    assert_eq!(Quat::from_frame([0, 0, 256], [0, 0, 256]), None);
    assert_eq!(Quat::from_frame([0, 0, 0], [0, 0, 256]), None);
    assert_eq!(Quat::from_frame([256, 0, 0], [0, 0, 0]), None);
}

fn close(a: [i64; 3], b: [i64; 3], tol: i64) -> bool {
    (0..3).all(|k| (a[k] - b[k]).abs() <= tol)
}

#[test]
fn the_shortest_arc_turns_z_to_x() {
    let (z, x) = ([0, 0, 256], [256, 0, 0]);
    let q = Quat::shortest_arc(z, x);
    assert!(near(q, [11585, 0, 11585, 0], 1), "{q:?}");
    assert!(close(q.rotate_vector(z), x, 2));
    let a = [100, -200, 50];
    assert_eq!(Quat::shortest_arc(a, a), Quat::IDENTITY);
    // Antiparallel: a half turn about e_x made perpendicular to `a` (e_x is not parallel to a).
    let h = Quat::shortest_arc(a, a.map(|c| -c));
    assert_eq!(h.0[0], 0);
    let axis = [h.0[1], h.0[2], h.0[3]].map(i64::from);
    let along: i64 = (0..3).map(|k| axis[k] * a[k]).sum();
    assert!(
        along.abs() <= 3 * 16384,
        "the axis is perpendicular to a: {h:?}"
    );
    assert!(close(h.rotate_vector(a), a.map(|c| -c), 2));
    for (a, b) in [
        ([0, 0, 256], [181, 0, 181]),
        ([256, 0, 0], [0, -256, 0]),
        ([30, 40, -200], [-100, 150, 60]),
    ] {
        let q = Quat::shortest_arc(a, b);
        // `b` scaled to `a`'s length.
        let (la, lb) = (len(a), len(b));
        let want = b.map(|c| c * la / lb);
        assert!(close(q.rotate_vector(a), want, 2), "{a:?} {b:?}");
    }
}

fn len(v: [i64; 3]) -> i64 {
    let n: i64 = v.iter().map(|c| c * c).sum();
    (0..).take_while(|r: &i64| r * r <= n).last().unwrap()
}

/// The committed vector of direction pairs `vxe-sim`'s copy of the arc shares: 56 seeded
/// pairs and eight special ones (equal, antiparallel, axis, zero).
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
    let mut h = Fnv1a::default();
    for (a, b) in arc_pairs() {
        for c in Quat::shortest_arc(a, b).0 {
            h.write(&c.to_le_bytes());
        }
    }
    // The same constant is pinned in `vxe-sim`'s `tests/surface.rs`.
    assert_eq!(
        h.finish(),
        0x08ab_b932_e90a_e5ce,
        "arc hash {:#018x}",
        h.finish()
    );
}
