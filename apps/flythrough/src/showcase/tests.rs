//! The showcase's tests: the hall's volume, its shots, its ores' light and the determinism
//! of its bytes.

use super::*;

use std::collections::BTreeMap;

use vxe_world::{AIR, BRICK_EDGE, Brick};

fn solid(m: MaterialId) -> bool {
    m != AIR && m != SHELL
}

/// The worked points of the hall: air in the middle, rock under the floor's lowest point,
/// rock past the noise's reach in x and z, the shell at the face, and air at `hall`'s eye.
#[test]
fn the_hall_is_the_worked_shape() {
    let s = ShowcaseSource::new(1);
    assert_eq!(s.material([448, 384, 200]), AIR);
    assert_eq!(s.material([448, 384, 100]), ROCK);
    assert!(matches!(s.material([40, 384, 200]), ROCK | AMBER));
    assert_eq!(s.material([448, 384, 400]), ROCK);
    assert_eq!(s.material([0, 384, 200]), SHELL);
    assert_eq!(s.material(SHOTS[0].eye.0), AIR);
    assert_eq!(s.spawn(), SHOTS[0].eye);
    assert_eq!(Shot::named("chasm").map(|s| s.eye), Some(SHOTS[1].eye));
    assert_eq!(Shot::named("keep"), None);
}

/// Compares `brick` with per-voxel evaluation against every feature; true when the brick
/// is `Full`.
fn early_out_agrees(s: &ShowcaseSource, b: [i32; 3]) -> bool {
    let fast = s.brick(b);
    let slow = Brick::from_fn(|l| s.material([0, 1, 2].map(|k| b[k] + l[k])));
    assert_eq!(fast, slow, "brick at {b:?}");
    fast.is_full()
}

/// The early-out never disagrees with evaluating every voxel: over a 12^3 brick sample
/// spanning the west wall, the floor and the shell, a coarser one across the west half,
/// and bricks that touch each kind of feature. Each kind's sample holds `Full` bricks.
#[test]
#[ignore = "soak"]
fn the_brick_early_out_matches_per_voxel_evaluation() {
    let s = ShowcaseSource::new(1);
    let (mut uniform, mut full) = (0, 0);
    // x 0..96 crosses the shell and the west wall; z 96..192 the floor and the air.
    for (origin, step) in [([0, 340, 96], 8), ([40, 300, 100], 24)] {
        for bz in 0..12 {
            for by in 0..12 {
                for bx in 0..12 {
                    let b = [0, 1, 2].map(|k| origin[k] + [bx, by, bz][k] * step);
                    if early_out_agrees(&s, brick_min(b)) {
                        full += 1;
                    } else {
                        uniform += 1;
                    }
                }
            }
        }
    }
    assert!(full > 0, "the sample crosses a surface");
    assert!(uniform > full, "{uniform} uniform, {full} full");
    let mut by_kind: BTreeMap<Kind, Vec<[i32; 3]>> = BTreeMap::new();
    for (b, list) in &s.buckets {
        for &i in list {
            by_kind
                .entry(s.features()[i as usize].kind())
                .or_default()
                .push(*b);
        }
    }
    assert_eq!(by_kind.len(), 6, "every kind is bucketed");
    for (kind, bricks) in by_kind {
        let step = bricks.len() / 48 + 1;
        let full = bricks
            .iter()
            .step_by(step)
            .filter(|&&b| early_out_agrees(&s, b))
            .count();
        println!(
            "{kind:?}: {} bricks bucketed, {full} full in the sample",
            bricks.len()
        );
        assert!(full > 0, "{kind:?}");
    }
}

/// Seed 1's features follow their rules, and each shows at its worked voxels.
#[test]
fn seed_1_places_the_features_by_their_rules() {
    let s = ShowcaseSource::new(1);
    let of = |k: Kind| -> Vec<Feature> {
        s.features()
            .iter()
            .filter(|f| f.kind() == k)
            .copied()
            .collect()
    };
    let kinds: Vec<Kind> = s.features().iter().map(Feature::kind).collect();
    assert!(kinds.is_sorted(), "sorted by kind");
    let pillars = of(Kind::Pillar);
    assert_eq!(pillars.len(), 5);
    for (i, a) in pillars.iter().enumerate() {
        let Feature::Pillar {
            axis, r_mid, mid, ..
        } = *a
        else {
            unreachable!()
        };
        for b in &pillars[i + 1..] {
            let Feature::Pillar { axis: other, .. } = *b else {
                unreachable!()
            };
            assert!(d2_xy(axis, other) >= 80 * 80, "{axis:?} {other:?}");
        }
        let at = |dx: i64| [axis[0] + dx, axis[1], mid].map(|v| v as i32);
        assert!(solid(s.material(at(0))), "pillar {axis:?} at mid-height");
        assert_eq!(s.material(at(r_mid + 1)), AIR, "beside pillar {axis:?}");
    }
    let (hung, standing) = (of(Kind::Stalactite), of(Kind::Stalagmite));
    println!(
        "{} pillars, {} stalactites, {} stalagmites, {} ledges",
        pillars.len(),
        hung.len(),
        standing.len(),
        of(Kind::Ledge).len()
    );
    assert!(hung.len() >= 16, "{} stalactites", hung.len());
    assert!(standing.len() >= 8, "{} stalagmites", standing.len());
    for c in hung.iter().chain(&standing) {
        let Feature::Cone {
            axis, tip, base, ..
        } = *c
        else {
            unreachable!()
        };
        let past = tip + (tip - base).signum();
        let at = |z: i64| [axis[0], axis[1], z].map(|v| v as i32);
        assert!(solid(s.material(at(tip))), "cone tip {:?}", at(tip));
        assert_eq!(s.material(at(past)), AIR, "past the tip {:?}", at(past));
    }
    let x = (CHASM_X + s.chasm_offset(384)) as i32;
    assert_eq!(s.material([x, 384, 32]), AIR, "the chasm's bottom");
    assert!(solid(s.material([x, 384, 31])), "under the chasm");
    assert!(solid(s.material([598, 384, 127])), "the bridge's deck");
    assert_eq!(s.material([598, 384, 129]), AIR, "over the deck");
    assert_eq!(s.material([598, 384, 90]), AIR, "under the arch");
    for i in 0..LEDGES.len() {
        let p = s.ledge_probe(i);
        assert!(solid(s.material(p)), "ledge {i} at {p:?}");
        assert_eq!(s.material([p[0], p[1], p[2] + 1]), AIR, "over ledge {i}");
    }
}

/// No pillar, cone or ledge voxel lies within 4 m of a shot's eye. The bridge is the
/// `chasm` shot's subject, 16 voxels under its eye.
#[test]
fn no_feature_lies_within_four_metres_of_a_shot() {
    let s = ShowcaseSource::new(1);
    let r = EYE_CLEAR as i32;
    for shot in SHOTS {
        let e = shot.eye.0;
        let (lo, hi) = (brick_min(e.map(|v| v - r)), brick_min(e.map(|v| v + r)));
        let mut near = 0;
        for bz in (lo[2]..=hi[2]).step_by(BRICK_EDGE as usize) {
            for by in (lo[1]..=hi[1]).step_by(BRICK_EDGE as usize) {
                for bx in (lo[0]..=hi[0]).step_by(BRICK_EDGE as usize) {
                    let f = Fields::of_brick(s.noises, [bx, by, bz]);
                    for l in 0..BRICK_EDGE.pow(3) {
                        let p = [
                            bx + l % BRICK_EDGE,
                            by + l / BRICK_EDGE % BRICK_EDGE,
                            bz + l / (BRICK_EDGE * BRICK_EDGE),
                        ];
                        let d2: i32 = (0..3).map(|k| (p[k] - e[k]).pow(2)).sum();
                        if d2 >= r * r {
                            continue;
                        }
                        near += usize::from(
                            s.features()
                                .iter()
                                .any(|ft| ft.kind() != Kind::Bridge && s.feature_rock(ft, &f, p)),
                        );
                    }
                }
            }
        }
        assert_eq!(
            near, 0,
            "{} feature voxels within 4 m of {}",
            near, shot.name
        );
    }
}
