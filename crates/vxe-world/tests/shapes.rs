mod support;

use support::*;
use vxe_world::*;

fn centre(i: i64) -> i64 {
    i * FIXED_ONE + FIXED_HALF
}

/// The exact set `{x in x_lo..=x_hi : contains(centre(x, y, z))}` as a range, or `None` when
/// it is empty. Panics when the set has a gap.
fn contained_range(shape: &Shape, y: i64, z: i64, x_lo: i64, x_hi: i64) -> Option<(i64, i64)> {
    let xs: Vec<i64> = (x_lo..=x_hi)
        .filter(|&x| shape.contains([centre(x), centre(y), centre(z)]))
        .collect();
    let (&first, &last) = (xs.first()?, xs.last()?);
    assert_eq!(
        xs.len() as i64,
        last - first + 1,
        "gap in {shape:?} row {y} {z}"
    );
    Some((first, last))
}

fn seeded_shape(rng: &mut SplitMix64, kind: u64) -> Shape {
    let point = |rng: &mut SplitMix64| Fixed3([0; 3].map(|_| rng.range(-64 * 256, 64 * 256)));
    let radius = |rng: &mut SplitMix64| rng.range(FIXED_ONE, 40 * FIXED_ONE);
    match kind {
        0 => Shape::Sphere {
            c: point(rng),
            r: radius(rng),
        },
        1 => {
            let c = point(rng);
            let lo = [0; 3].map(|_| radius(rng));
            let hi = [0; 3].map(|_| radius(rng));
            Shape::Box_ {
                min: Fixed3([0, 1, 2].map(|k| c.0[k] - lo[k])),
                max: Fixed3([0, 1, 2].map(|k| c.0[k] + hi[k])),
            }
        }
        _ => Shape::Capsule {
            a: point(rng),
            b: point(rng),
            r: radius(rng),
        },
    }
}

/// For 1000 seeded rows over all three shapes, the span equals the exact set of contained
/// centres in the row's window: both `None`, or the same inclusive range.
#[test]
fn row_span_equals_the_set_of_contained_centres() {
    let mut rng = SplitMix64(0x0003_0003);
    let mut spans = 0;
    for i in 0..1000 {
        let shape = seeded_shape(&mut rng, i % 3);
        // Rows within two voxels of the shape's bounds, so most meet the shape and the rest
        // pass just outside it.
        let (lo, hi) = shape.voxel_bounds().unwrap();
        let y = rng.range(lo[1] - 2, hi[1] + 2);
        let z = rng.range(lo[2] - 2, hi[2] + 2);
        // Windows from one voxel wide to wider than the shape, starting anywhere from just
        // before the shape to its far end.
        let x_lo = rng.range(lo[0] - 4, hi[0]);
        let x_hi = x_lo + rng.range(0, hi[0] - lo[0] + 8);
        let expected = contained_range(&shape, y, z, x_lo, x_hi);
        assert_eq!(
            shape.row_span(y, z, x_lo, x_hi),
            expected,
            "row {i}: {shape:?} y {y} z {z} x {x_lo}..={x_hi}"
        );
        spans += usize::from(expected.is_some());
    }
    println!("{spans} of 1000 rows hold a span");
    assert!(spans >= 300, "{spans} rows hold a span");
}

/// A centre exactly on the sphere's surface, `|256 x + 128 - cx| == h`, is inside.
#[test]
fn row_span_includes_the_sphere_boundary() {
    let c = Fixed3([128 + 17, 128, 128]);
    // x = 3: 256 * 3 + 128 - 145 = 751. x = -3: |-768 + 128 - 145| = 785.
    let sphere = |r| Shape::Sphere { c, r };
    assert_eq!(sphere(751).row_span(0, 0, -10, 10), Some((-2, 3)));
    assert_eq!(sphere(750).row_span(0, 0, -10, 10), Some((-2, 2)));
    assert_eq!(sphere(785).row_span(0, 0, -10, 10), Some((-3, 3)));
    // Centred on a voxel, radius 3 voxels: the poles at +-3 are on the surface.
    let s = Shape::Sphere {
        c: centre_of_voxel([0, 0, 0]),
        r: 3 * FIXED_ONE,
    };
    assert_eq!(s.row_span(0, 0, -10, 10), Some((-3, 3)));
    assert_eq!(s.row_span(3, 0, -10, 10), Some((0, 0)));
    assert_eq!(s.row_span(3, 1, -10, 10), None);
    // The window clips.
    assert_eq!(s.row_span(0, 0, 1, 10), Some((1, 3)));
    assert_eq!(s.row_span(0, 0, 4, 10), None);
}

/// For 1000 seeded capsule rows (radius 1 to 40 voxels, ends within ±64 voxels, including
/// segments parallel to `x` and degenerate segments of length 0, and the extremes of those
/// ranges), the span equals the exact set of contained centres in the row's window.
#[test]
fn capsule_row_span_equals_the_set_of_contained_centres() {
    let mut rng = SplitMix64(0x0014_0004);
    let point = |rng: &mut SplitMix64| [0; 3].map(|_| rng.range(-64 * 256, 64 * 256));
    let (mut spans, mut parallel, mut points) = (0, 0, 0);
    for i in 0..1000 {
        let a = match i % 50 {
            // The corners of the range.
            0 => [-64 * 256; 3],
            1 => [64 * 256; 3],
            _ => point(&mut rng),
        };
        let b = match i % 6 {
            0 => a,
            1 | 2 => [rng.range(-64 * 256, 64 * 256), a[1], a[2]],
            _ => match i % 50 {
                0 => [64 * 256; 3],
                1 => [-64 * 256; 3],
                _ => point(&mut rng),
            },
        };
        let r = match i % 25 {
            0 => FIXED_ONE,
            1 => 40 * FIXED_ONE,
            _ => rng.range(FIXED_ONE, 40 * FIXED_ONE),
        };
        let shape = Shape::Capsule {
            a: Fixed3(a),
            b: Fixed3(b),
            r,
        };
        parallel += usize::from(a[1] == b[1] && a[2] == b[2] && a != b);
        points += usize::from(a == b);
        let (lo, hi) = shape.voxel_bounds().unwrap();
        let y = rng.range(lo[1] - 2, hi[1] + 2);
        let z = rng.range(lo[2] - 2, hi[2] + 2);
        let x_lo = rng.range(lo[0] - 4, hi[0]);
        let x_hi = x_lo + rng.range(0, hi[0] - lo[0] + 8);
        let expected = contained_range(&shape, y, z, x_lo, x_hi);
        assert_eq!(
            shape.row_span(y, z, x_lo, x_hi),
            expected,
            "row {i}: {shape:?} y {y} z {z} x {x_lo}..={x_hi}"
        );
        spans += usize::from(expected.is_some());
    }
    println!("{spans} of 1000 rows hold a span; {parallel} parallel to x, {points} points");
    assert!(spans >= 300, "{spans} rows hold a span");
    assert!(
        parallel >= 300 && points >= 150,
        "{parallel} parallel, {points} points"
    );
}

const LIMIT: i64 = 1 << 30;

/// The documented range: inclusive at `±2^30` fixed units on every coordinate and radius,
/// and `2^30` per axis of a capsule's `b - a`. The edit extent is lifted so the other checks
/// show on their own.
#[test]
fn shapes_are_checked_against_the_documented_range() {
    assert_eq!(WorldLimits::default(), WorldLimits::DEFAULT);
    let limits = WorldLimits {
        edit_axis_max: u32::MAX,
        ..WorldLimits::DEFAULT
    };
    let o = Fixed3([0; 3]);
    let refused = |what| Err(EditError::OutOfRange { what });

    assert_eq!(Shape::Sphere { c: o, r: LIMIT }.check(&limits), Ok(()));
    assert_eq!(
        Shape::Sphere { c: o, r: LIMIT + 1 }.check(&limits),
        refused(Field::Radius)
    );
    assert_eq!(
        Shape::Sphere { c: o, r: i64::MAX }.check(&limits),
        refused(Field::Radius)
    );
    assert_eq!(
        Shape::Sphere {
            c: Fixed3([0, LIMIT + 1, 0]),
            r: 1
        }
        .check(&limits),
        refused(Field::Coord)
    );

    let box_ = |min: [i64; 3], max: [i64; 3]| Shape::Box_ {
        min: Fixed3(min),
        max: Fixed3(max),
    };
    assert_eq!(box_([-LIMIT; 3], [LIMIT; 3]).check(&limits), Ok(()));
    assert_eq!(
        box_([0; 3], [i64::MIN, 0, 0]).check(&limits),
        refused(Field::Coord)
    );
    // A reversed box is in range; it is empty, not refused.
    assert_eq!(box_([LIMIT; 3], [-LIMIT; 3]).check(&limits), Ok(()));
    assert_eq!(box_([LIMIT; 3], [-LIMIT; 3]).voxel_bounds(), None);

    let capsule = |a: [i64; 3], b: [i64; 3], r| Shape::Capsule {
        a: Fixed3(a),
        b: Fixed3(b),
        r,
    };
    let half = LIMIT / 2;
    assert_eq!(capsule([-half; 3], [half; 3], LIMIT).check(&limits), Ok(()));
    assert_eq!(
        capsule([-LIMIT, 0, 0], [LIMIT, 0, 0], 256).check(&limits),
        refused(Field::CapsuleLength)
    );
    assert_eq!(
        capsule([0; 3], [0, 0, LIMIT + 1], 256).check(&limits),
        refused(Field::Coord)
    );
    assert_eq!(
        capsule([0; 3], [256; 3], LIMIT + 1).check(&limits),
        refused(Field::Radius)
    );

    // Under the default limits the same in-range sphere is refused for its extent.
    assert_eq!(
        Shape::Sphere { c: o, r: LIMIT }.check(&WorldLimits::DEFAULT),
        refused(Field::EditExtent)
    );
}

/// The extreme accepted capsule (ends `[-2^29; 3]` and `[2^29; 3]`, radius `2^30`): the
/// inside test and the row solver on the eight corner centres of its voxel bounds and on its
/// midpoint run without `i128` overflow (this test panics on overflow in a debug build);
/// the midpoint is inside and the corners are outside.
#[test]
fn the_extreme_accepted_capsule_does_not_overflow() {
    let half = LIMIT / 2;
    let shape = Shape::Capsule {
        a: Fixed3([-half; 3]),
        b: Fixed3([half; 3]),
        r: LIMIT,
    };
    let limits = WorldLimits {
        edit_axis_max: u32::MAX,
        ..WorldLimits::DEFAULT
    };
    assert_eq!(shape.check(&limits), Ok(()));
    let (lo, hi) = shape.voxel_bounds().unwrap();
    let check = |v: [i64; 3], inside: bool| {
        assert_eq!(shape.contains(v.map(centre)), inside, "{v:?}");
        let span = shape.row_span(v[1], v[2], v[0], v[0]);
        assert_eq!(span, inside.then_some((v[0], v[0])), "{v:?}");
    };
    for i in 0..8 {
        let v = [0, 1, 2].map(|k| if i >> k & 1 == 0 { lo[k] } else { hi[k] });
        check(v, false);
    }
    check([0, 1, 2].map(|k| (lo[k] + hi[k]).div_euclid(2)), true);
}
