//! The strip tool's gate tests: the aliasing reads it prints (`support::aliasing`) checked on
//! seeded series and hand-built hit buffers, and two short runs on lavapipe that pin the eye
//! lamp's still frame and the pan's one-pixel shift. The strip itself is a reading, the
//! `strip` example (`cargo run -p flythrough --example strip -- --help`).

mod support;

use support::aliasing::{self, Hits, Mask};
use vxe_render::{Exposure, Frame, GpuWorld, Lighting, MirrorConfig, Window};

/// 72 frames of 10 000 pixels (100 x 100) at 18 phases.
const FOLD_SIDE: usize = 100;
const FOLD_FRAMES: usize = 72;
const FOLD_PHASES: u32 = 18;

fn series(value: impl Fn(usize, usize) -> u8) -> Vec<Vec<u8>> {
    (0..FOLD_FRAMES)
        .map(|f| {
            (0..FOLD_SIDE * FOLD_SIDE)
                .flat_map(|i| {
                    let v = value(f, i);
                    [v, v.wrapping_add(17), v.wrapping_mul(3), 255]
                })
                .collect()
        })
        .collect()
}

#[test]
fn the_phase_fold_splits_a_locked_series_from_noise() {
    // A series of period 18: each pixel takes a seeded value per phase.
    let mut mix = aliasing::Mix(56);
    let table: Vec<u8> = (0..FOLD_PHASES as usize * FOLD_SIDE * FOLD_SIDE)
        .map(|_| mix.byte())
        .collect();
    let locked = aliasing::fold(
        &series(|f, i| table[(f % FOLD_PHASES as usize) * FOLD_SIDE * FOLD_SIDE + i]),
        FOLD_PHASES,
    );
    println!(
        "locked: T {} | L {} | J {:.6}",
        locked.t.cell(),
        locked.l.cell(),
        locked.j
    );
    assert_eq!(locked.cycles, 4);
    assert!(
        locked.t.mean > 0.0 && locked.t.jumps > 0,
        "T sees the cycle"
    );
    assert_eq!(locked.l.max, 0, "L sees nothing of a locked series");
    assert!(
        (locked.j - 1.0).abs() < 1e-9,
        "J of a locked series: {}",
        locked.j
    );

    // Seeded white noise.
    let mut mix = aliasing::Mix(0x0056);
    let noise: Vec<u8> = (0..FOLD_FRAMES * FOLD_SIDE * FOLD_SIDE)
        .map(|_| mix.byte())
        .collect();
    let noisy = aliasing::fold(
        &series(|f, i| noise[f * FOLD_SIDE * FOLD_SIDE + i]),
        FOLD_PHASES,
    );
    println!(
        "noise: T {} | L {} | J {:.6}",
        noisy.t.cell(),
        noisy.l.cell(),
        noisy.j
    );
    assert!(noisy.j.abs() <= 0.1, "J of noise: {}", noisy.j);
    assert!(
        (noisy.l.mean - noisy.t.mean).abs() <= 0.2 * noisy.t.mean,
        "L's mean {} against T's {}",
        noisy.l.mean,
        noisy.t.mean
    );

    // A constant series.
    let still = aliasing::fold(&series(|_, i| (i % 251) as u8), FOLD_PHASES);
    let zero = aliasing::Change {
        samples: still.t.samples,
        ..aliasing::Change::default()
    };
    assert_eq!(still.t, zero);
    assert_eq!(
        still.l,
        aliasing::Change {
            samples: still.l.samples,
            ..zero
        }
    );
    assert_eq!(still.j, 0.0);
    assert!(still.t_jumps.iter().chain(&still.l_jumps).all(|&c| c == 0));
}

/// A hit on the `axis` face (0 x, 1 y, 2 z) at `p`, of material `m`.
fn hit(p: [f32; 3], axis: u16, m: u16) -> ([f32; 4], [u16; 4]) {
    let t = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
    ([p[0], p[1], p[2], t], [0, 0, m, 1 | (axis + 1) << 3])
}

const MISS: ([f32; 4], [u16; 4]) = ([0.0, 0.0, 0.0, 3e38], [0, 0, 0, 0]);

/// The class of the centre of a 3 x 3 grid whose centre is a hit on the z = 10 plane at
/// (5.5, 7.5) and whose right neighbour is `right`; the other neighbours share the centre's
/// face.
fn centre_class(right: ([f32; 4], [u16; 4])) -> (aliasing::Class, Mask) {
    let centre = hit([5.5, 7.5, 10.0], 2, 3);
    let near = hit([5.2, 7.2, 10.0], 2, 3);
    let mut grid = vec![near; 9];
    grid[4] = centre;
    grid[5] = right;
    let pos: Vec<[f32; 4]> = grid.iter().map(|g| g.0).collect();
    let info: Vec<[u16; 4]> = grid.iter().map(|g| g.1).collect();
    let hits = Hits {
        size: [3, 3],
        pos: &pos,
        info: &info,
    };
    let emits = |m: u16| m == 9;
    let mask = Mask::of(&hits, [3, 3], 2.0, &emits);
    (hits.class(1, 1, &emits), mask)
}

#[test]
fn the_class_mask_names_each_class() {
    use aliasing::Class;
    let cases = [
        ("interior", hit([5.9, 7.1, 10.0], 2, 3), Class::Interior),
        ("seam", hit([6.2, 7.5, 10.0], 2, 3), Class::Seam),
        ("crease", hit([6.0, 7.5, 10.6], 0, 3), Class::Crease),
        ("silhouette", hit([6.4, 7.5, 18.0], 2, 3), Class::Silhouette),
        ("silhouette by a miss", MISS, Class::Silhouette),
        ("emitter", hit([6.2, 7.5, 10.0], 2, 9), Class::Emitter),
    ];
    for (name, right, want) in cases {
        let (got, mask) = centre_class(right);
        assert_eq!(got, want, "{name}");
        assert_eq!(
            mask.class[4], want,
            "{name}: the display pixel over the centre"
        );
    }
    let pos = vec![MISS.0; 9];
    let info = vec![MISS.1; 9];
    let all_miss = Hits {
        size: [3, 3],
        pos: &pos,
        info: &info,
    };
    assert_eq!(all_miss.class(1, 1, &|_| false), Class::Miss);
    // Inside a cycle the pixel was interior, then a silhouette: it takes the silhouette.
    let (_, mut cycle) = centre_class(cases[0].1);
    let (_, edge) = centre_class(cases[3].1);
    let (_, back) = centre_class(cases[0].1);
    cycle.merge(&edge);
    cycle.merge(&back);
    assert_eq!(cycle.class[4], Class::Silhouette);
}

/// A two-step staircase rising along +x seen from `STAIR_EYE` (above and behind it), one internal
/// pixel a column, every row alike: treads (+z faces) of 4 columns at z 1, 2, 3 and risers (-x
/// faces) of one column between them. The hit of column `x` and its store direction.
const STAIR_EYE: [f32; 3] = [-20.0, 0.5, 30.0];

/// A hit's position with `t`, and its record.
type HitCell = ([f32; 4], [u16; 4]);

fn stair_hit(x: u32) -> (HitCell, u8) {
    // Columns 0..4 tread at z 1, 4 riser at x 4, 5..9 tread at z 2, 9 riser at x 9, 10..14
    // tread at z 3.
    let rel = |p: [f32; 3]| {
        [
            p[0] - STAIR_EYE[0],
            p[1] - STAIR_EYE[1],
            p[2] - STAIR_EYE[2],
        ]
    };
    let at = |p: [f32; 3], axis: u16| {
        let r = rel(p);
        let t = (r[0] * r[0] + r[1] * r[1] + r[2] * r[2]).sqrt();
        ([p[0], p[1], p[2], t], [0, 0, 3, 1 | (axis + 1) << 3])
    };
    match x {
        4 => (at([4.0, 0.5, 1.5], 0), 0),
        9 => (at([9.0, 0.5, 2.5], 0), 0),
        _ => {
            let z = 1.0 + (x / 5) as f32;
            (at([x as f32 + 0.5, 0.5, z], 2), 5)
        }
    }
}

#[test]
fn the_direction_map_names_each_face_direction() {
    use aliasing::{DirEdges, changes, directions, z_levels};
    let (w, h) = (14u32, 4u32);
    let cells: Vec<(HitCell, u8)> = (0..w * h).map(|i| stair_hit(i % w)).collect();
    let pos: Vec<[f32; 4]> = cells.iter().map(|c| c.0.0).collect();
    let info: Vec<[u16; 4]> = cells.iter().map(|c| c.0.1).collect();
    let hits = Hits {
        size: [w, h],
        pos: &pos,
        info: &info,
    };
    let dirs = directions(&hits, [w, h], STAIR_EYE);
    for (i, (d, (_, want))) in dirs.iter().zip(&cells).enumerate() {
        assert_eq!(*d, Some(*want), "pixel {i}'s store direction");
    }
    // A miss and a hit with no entry face hold none.
    let none_pos = [MISS.0, ([1.0, 1.0, 1.0, 5.0])];
    let none_info = [MISS.1, [0, 0, 3, 1]];
    let none = Hits {
        size: [2, 1],
        pos: &none_pos,
        info: &none_info,
    };
    assert_eq!(directions(&none, [2, 1], STAIR_EYE), vec![None, None]);

    let at = changes(&dirs, [w, h]);
    let want_at = |x: u32| matches!(x, 3..=5 | 8..=10);
    for (i, &a) in at.iter().enumerate() {
        assert_eq!(a, want_at(i as u32 % w), "pixel {i} at a direction change");
    }
    // Luma: tread A 40, the risers 120, tread B 40, tread C 40 then 100 from column 12. The
    // steps at columns 3|4, 4|5, 8|9, 9|10 lie at direction changes; the one at 11|12 inside
    // tread C.
    let rgba: Vec<u8> = (0..w * h)
        .flat_map(|i| {
            let v = match i % w {
                4 | 9 => 120,
                12.. => 100,
                _ => 40,
            };
            [v, v, v, 255]
        })
        .collect();
    let z = z_levels(&hits, [w, h]);
    let r = DirEdges::read(&rgba, &dirs, &at, &z, w, [0, 0, w, h], 0.0);
    // Read over columns 0..13 and rows 0..3: edges are pixels whose right neighbour differs
    // (rows alike, so no vertical edge): columns 3, 4, 8, 9 at a change, 11 inside a face.
    let rows = u64::from(h - 1);
    let at_px = rows * (0..w - 1).filter(|&x| want_at(x)).count() as u64;
    assert_eq!(
        r.beyond,
        [5 * rows, 4 * rows, rows * u64::from(w - 1), at_px],
        "edges, edges at a change, pixels, pixels at a change"
    );
    let ratio = aliasing::edge_ratio(r.beyond);
    let want = (4.0 / 6.0) / (1.0 / 7.0);
    assert!(
        (ratio - want).abs() < 1e-9,
        "the edge ratio {ratio} against {want}"
    );
    assert_eq!(r.by_t.iter().map(|b| b[0]).sum::<u64>(), 5 * rows);
}

/// The first frame of the eye lamp's native run from which every frame to frame 128 is
/// bit-identical, read on lavapipe (`cavity_world`, 192 x 108, `off`): frame 4.
const EYE_STILL_FROM: usize = 4;

#[test]
fn the_eye_lamp_holds_the_native_frame_bit_identical() {
    let world = support::cavity_world();
    let full = world.stats().full_bricks as u64;
    let Some((device, queue)) = support::software_device(((full + 4096) * 512).max(64 << 20))
    else {
        return;
    };
    let cfg = MirrorConfig {
        light_radius: 0,
        ..MirrorConfig::new(Window::Fixed(world.extent()))
    };
    let mut gpu = GpuWorld::with_config(&device, cfg);
    support::SyncNow::sync_now(&mut gpu, &world, &device, &queue).expect("the mirror fits");
    let mut frame = Frame::new(&device, 192, 108);
    frame.set_lighting(aliasing::eye_lighting(Lighting {
        exposure: Exposure::Manual(1.0),
        ..Lighting::default()
    }));
    frame.set_dt(1.0 / 60.0);
    let light = aliasing::eye_lamp(1200.0);
    let camera = support::cavity_camera();
    let mut frames = Vec::new();
    for _ in 0..=128 {
        frame.step(&device, &queue, &gpu, &camera, &light);
        frames.push(vxe_render::debug::read_rgba8(
            &device,
            &queue,
            frame.target(),
        ));
    }
    let last = frames.last().expect("frames");
    let from = (0..frames.len())
        .rev()
        .take_while(|&f| frames[f] == *last)
        .last()
        .expect("the last frame equals itself");
    let lit = last.chunks_exact(4).filter(|p| p[0] > 8).count();
    println!(
        "eye lamp, native: every frame from {from} to 128 is bit-identical; {lit} of {} pixels over 8",
        last.len() / 4
    );
    assert!(lit > last.len() / 8, "the lamp lights the cavity");
    assert!(
        from <= EYE_STILL_FROM,
        "the frame changes until {from}, past {EYE_STILL_FROM}"
    );
}

#[test]
fn the_pan_moves_the_centre_band_one_pixel() {
    let world = support::pillar_world();
    let Some((device, queue)) = support::software_device(64 << 20) else {
        return;
    };
    let cfg = MirrorConfig {
        light_radius: 0,
        ..MirrorConfig::new(Window::Fixed(world.extent()))
    };
    let mut gpu = GpuWorld::with_config(&device, cfg);
    support::SyncNow::sync_now(&mut gpu, &world, &device, &queue).expect("the mirror fits");
    let (w, h) = (support::WIDTH, support::HEIGHT);
    let mut frame = Frame::new(&device, w, h);
    frame.set_lighting(aliasing::eye_lighting(Lighting {
        exposure: Exposure::Manual(1.0),
        ..Lighting::default()
    }));
    frame.set_dt(1.0 / 60.0);
    let light = aliasing::eye_lamp(1200.0);
    let mut camera = support::pillar_camera();
    let step = aliasing::yaw_per_pixel(h, camera.vfov, camera.pitch);
    let (lag, shift) = aliasing::lag_shift(1.0);
    assert_eq!((lag, shift), (1, 1));
    let cols = aliasing::band(w);
    let mut frames = Vec::new();
    for _ in 0..8 {
        frame.step(&device, &queue, &gpu, &camera, &light);
        frames.push(vxe_render::debug::read_rgba8(
            &device,
            &queue,
            frame.target(),
        ));
        camera.yaw += step;
    }
    let residual: Vec<(i32, f64)> = (-2..=2)
        .map(|s| {
            let mut hist = aliasing::Hist::default();
            for p in frames.windows(2) {
                aliasing::crawl(&p[1], &p[0], w, s, cols.clone(), &mut hist, None);
            }
            (s, hist.change().mean)
        })
        .collect();
    println!("the band's mean residual by shift: {residual:?}");
    let best = residual
        .iter()
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .expect("five shifts");
    assert_eq!(best.0, shift, "the residual is least at the rate's shift");
    let unshifted = residual.iter().find(|r| r.0 == 0).expect("shift 0").1;
    assert!(
        best.1 < unshifted,
        "the shifted residual {} is under the unshifted {unshifted}",
        best.1
    );
}
