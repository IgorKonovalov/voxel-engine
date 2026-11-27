// `gpu_sun`'s synthetic shaft (Plan 0105 Phase 7c): a shaft dug by steps under an open sky,
// its last dig's held records against their landed ones, included by `gpu_sun.rs` (kept apart
// for the file-size cap).

/// The shaft's ground: rock below voxel z 56 over 3 x 3 x 2 chunks (the upper layer air, the
/// sky over it).
const SHAFT_GROUND: i32 = 56;
/// The shaft's cross-section, 10 x 10 voxels inside chunk (1, 1, 0).
const SHAFT_LO: [i32; 2] = [91, 91];
const SHAFT_HI: [i32; 2] = [101, 101];
/// Each dig carves [`SHAFT_STEP`] voxels down; [`SHAFT_DIGS`] digs reach 40.
const SHAFT_STEP: i32 = 10;
const SHAFT_DIGS: i32 = 4;
/// The most frames the start or a settle steps before the rig is at fault.
const SHAFT_SETTLE_CAP: usize = 1500;
/// Frames stepped past a settle (the temporal passes' history).
const SHAFT_TAIL: usize = 24;
/// Frames read from the last dig, D+0 on; the last is the settled frame.
const SHAFT_READ: usize = 96;
/// The fewest frames between the landing and the settled frame.
const SHAFT_SETTLE_AFTER: usize = 40;

/// Over the shaft's axis, 6 voxels over the ground, looking straight down.
fn shaft_camera() -> Camera {
    Camera {
        position: Vec3::new(96.0, 96.0, (SHAFT_GROUND + 6) as f32),
        yaw: 0.0,
        pitch: -FRAC_PI_2 + 1e-4,
        vfov: 60f32.to_radians(),
    }
}

/// Dig `k`'s box (voxels, half-open): the cross-section from `SHAFT_STEP * k` to
/// `SHAFT_STEP * (k + 1)` under the ground.
fn shaft_box(k: i32) -> ([i32; 3], [i32; 3]) {
    (
        [SHAFT_LO[0], SHAFT_LO[1], SHAFT_GROUND - SHAFT_STEP * (k + 1)],
        [SHAFT_HI[0], SHAFT_HI[1], SHAFT_GROUND - SHAFT_STEP * k],
    )
}

/// The cell record holding world voxel `v` in `f`'s store (cells of `BakeSettings::default().cell`
/// voxels; the directory arithmetic of temporal.wgsl's `store_cell_record`), `None` where its
/// chunk holds no block or its brick no record.
fn shaft_cell_record(f: &Frame, v: [i32; 3]) -> Option<Vec<u32>> {
    let block = f.store().block_words(ChunkCoord(v.map(|c| c.div_euclid(64))))?;
    let cell = svet_render::BakeSettings::default().cell as i32;
    let l = v.map(|c| c.rem_euclid(64));
    let b = l.map(|c| c / 8);
    let dir = block[(b[0] + 8 * (b[1] + 8 * b[2])) as usize];
    if dir == 0 {
        return None;
    }
    let n = 8 / cell;
    let c = l.map(|k| (k % 8) / cell);
    let sub = (c[0] + n * (c[1] + n * c[2])) as usize;
    let w = svet_render::internals::BRICK_WORDS;
    let at = ((dir >> 11) as usize + w * sub).checked_sub(w * (n * n * n) as usize)?;
    block.get(at..at + w).map(<[u32]>::to_vec)
}

/// The faces dig `box_` made in `world` (after the dig): each solid voxel beside a carved one,
/// by its record's cell (voxel coordinates over the cell's edge) and the face's direction
/// (`2 * axis + 1` where the carved voxel lies on the positive side), with one voxel of each.
fn shaft_dug_cells(world: &World, (lo, hi): ([i32; 3], [i32; 3])) -> Vec<([i32; 3], usize, [i32; 3])> {
    let cell = svet_render::BakeSettings::default().cell as i32;
    let mut cells = std::collections::BTreeMap::new();
    for z in lo[2]..hi[2] {
        for y in lo[1]..hi[1] {
            for x in lo[0]..hi[0] {
                for axis in 0..3 {
                    for s in [-1, 1] {
                        let mut n = [x, y, z];
                        n[axis] -= s;
                        let inside = (0..3).all(|k| (lo[k]..hi[k]).contains(&n[k]));
                        if inside || world.material_at(n).is_none_or(|m| m == AIR) {
                            continue;
                        }
                        let face = 2 * axis + usize::from(s > 0);
                        cells.entry((n.map(|c| c.div_euclid(cell)), face)).or_insert(n);
                    }
                }
            }
        }
    }
    cells.into_iter().map(|((c, f), v)| (c, f, v)).collect()
}

/// One step of the shaft rig: the queue's step, the frame, the device's wait; the queue's stats.
fn shaft_step(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    f: &mut Frame,
    gpu: &mut GpuWorld,
    world: &World,
    cam: &Camera,
) -> svet_render::QueueStats {
    dig_queue_step(device, queue, f, gpu, world, cam);
    f.step(device, queue, gpu, cam, &LightParams::headlamp());
    // The device waited each frame: the next step's read-backs are all in, whatever the
    // adapter's pace, so the queue's bake order is the same on every run.
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    f.bake_queue().expect("the queue runs").stats()
}

/// [`a_dig_at_depth_holds_no_light_over_its_landing`]'s run.
struct ShaftRun {
    /// The frames each settle stepped (the start, then the digs before the last).
    steps: Vec<usize>,
    /// The last dig's faces by cell and direction ([`shaft_dug_cells`]), and each one's
    /// `record_dig_visibility` at D+0 and at the settled frame.
    cells: Vec<([i32; 3], usize, [i32; 3])>,
    held: Vec<Option<f32>>,
    landed_vis: Vec<Option<f32>>,
    /// The frames D+0 to D+[`SHAFT_READ`] - 1, and the landing's index among them.
    shots: Vec<DigShot>,
    landed: Option<usize>,
}

/// The shaft rig's run ([`a_dig_at_depth_holds_no_light_over_its_landing`]).
fn shaft_run() -> Option<ShaftRun> {
    let (device, queue) = device()?;
    let world = floor_world([3, 3, 2], |p| p[2] < SHAFT_GROUND);
    let mut gpu = mirror(&device, &queue, &world, None);
    let mut f = dig_frame(&device);
    f.set_sky(Some(Sky {
        altitude_zero_z: SHAFT_GROUND as f32,
        ..Sky::uniform(1.0)
    }));
    assert_eq!(
        f.lighting().dig_estimate,
        svet_render::DigEstimate::Depth,
        "the default estimate"
    );
    f.start_bake_queue(
        &queue,
        &gpu,
        svet_render::queue_settings(),
        svet_render::DEFAULT_BAKE_SLICE_MS,
    );
    f.set_bake_coarse(8, 2);
    let cam = shaft_camera();
    let settle = |f: &mut Frame, gpu: &mut GpuWorld, what: &str| {
        let mut n = 0;
        loop {
            let st = shaft_step(&device, &queue, f, gpu, &world, &cam);
            n += 1;
            let baked = (0..9).all(|k| {
                f.store()
                    .block_words(ChunkCoord([k % 3, k / 3, 0]))
                    .is_some()
            });
            if baked && st.digs == 0 && st.fading == 0 && st.jobs == 0 && st.coarse == 0 && n > 2 {
                break;
            }
            assert!(n < SHAFT_SETTLE_CAP, "rig fault: {what} did not settle in {n} frames");
        }
        for _ in 0..SHAFT_TAIL {
            shaft_step(&device, &queue, f, gpu, &world, &cam);
        }
        n + SHAFT_TAIL
    };
    let mut steps = vec![settle(&mut f, &mut gpu, "the start")];
    let carve = |f: &mut Frame, gpu: &mut GpuWorld, k: i32| {
        let (lo, hi) = shaft_box(k);
        let r = world
            .apply(&EditOp::box_(
                Fixed3(lo.map(|v| i64::from(v) * FIXED_ONE)),
                Fixed3(hi.map(|v| i64::from(v) * FIXED_ONE)),
                Mode::Carve,
            ))
            .expect("the dig is resident");
        let (a, b) = r.aabb.expect("the dig changed voxels");
        let mut enc = device.create_command_encoder(&Default::default());
        gpu.sync(&world, &queue, &mut enc).expect("the mirror fits");
        queue.submit([enc.finish()]);
        f.invalidate(a.0, b.0);
    };
    for k in 0..SHAFT_DIGS - 1 {
        carve(&mut f, &mut gpu, k);
        steps.push(settle(&mut f, &mut gpu, "a dig"));
    }
    let last = shaft_box(SHAFT_DIGS - 1);
    let cells = shaft_dug_cells(&world, last);
    let st0 = f.bake_queue().expect("the queue runs").stats();
    carve(&mut f, &mut gpu, SHAFT_DIGS - 1);
    let mut held = Vec::new();
    let mut shots = Vec::new();
    let mut landed = None;
    for k in 0..SHAFT_READ {
        let st = shaft_step(&device, &queue, &mut f, &mut gpu, &world, &cam);
        if k == 0 {
            println!(
                "the last dig's step: whole digs {}, digs {}",
                st.digs_whole - st0.digs_whole,
                st.digs
            );
            held = cells
                .iter()
                .map(|(_, face, v)| {
                    shaft_cell_record(&f, *v)
                        .and_then(|r| svet_render::internals::record_dig_visibility(&r, *face))
                })
                .collect();
        }
        if landed.is_none() && st.digs_baked > st0.digs_baked {
            landed = Some(k);
        }
        shots.push(dig_shot(&device, &queue, &f));
    }
    let landed_vis = cells
        .iter()
        .map(|(_, face, v)| {
            shaft_cell_record(&f, *v)
                .and_then(|r| svet_render::internals::record_dig_visibility(&r, *face))
        })
        .collect();
    Some(ShaftRun {
        steps,
        cells,
        held,
        landed_vis,
        shots,
        landed,
    })
}

/// Plan 0105 Phase 7c (1): a dig at depth holds no light over its landing. A shaft 10 x 10
/// voxels dug from the ground ([`SHAFT_GROUND`], rock 56 voxels deep over 3 x 3 x 2 chunks under
/// `Sky::uniform(1.0)`, the layer above air) by four digs of 10 voxels to 40 under it, each
/// settled (the queue holding no dig, no fade, no job and no coarse block, then
/// [`SHAFT_TAIL`] frames) before the next, under the default `Lighting::dig_estimate`
/// (`depth`) and dig hold, lockstep (the live queue stepped in the test, the device waited each
/// frame), [`dig_frame`]'s lighting, seen from over the axis looking down. The last dig is read
/// from D+0 to D+[`SHAFT_READ`] - 1, the last frame the settled one. (a) Every cell record
/// holding a face the last dig made (a solid voxel beside a voxel it carved, by cell and
/// direction), read as a dig's face reads it (`record_dig_visibility`): at D+0 (after the
/// dig's step, the held record) its byte is over the settled frame's (the landed record) by at
/// most one; the cells with no held record are counted and printed, their faces read by (b).
/// (b) Over the display pixels whose hit lies in the last dig's box dilated by one voxel in a
/// frame and in the settled frame, no frame from D+0 to the landing (the first frame whose
/// `digs_baked` rose) shows the mean luma over the settled frame's by more than 4 display
/// levels. The landing lies at least [`SHAFT_SETTLE_AFTER`] frames before the settled frame,
/// and the rig's start bakes every ground chunk, else the rig is at fault.
/// Soak: over the line on lavapipe (Plan 0105 Phase 7c's readings log).
#[test]
#[ignore = "soak"]
fn a_dig_at_depth_holds_no_light_over_its_landing() {
    let Some(ShaftRun {
        steps,
        cells,
        held,
        landed_vis,
        shots,
        landed,
    }) = shaft_run()
    else {
        return;
    };
    let last = shaft_box(SHAFT_DIGS - 1);
    // (a) the held bytes against the landed ones.
    let byte = |v: f32| (v * 255.0).round() as i32;
    let (mut no_held, mut no_landed, mut over) = (Vec::new(), Vec::new(), Vec::new());
    let mut worst = i32::MIN;
    for (((c, face, _), h), l) in cells.iter().zip(&held).zip(&landed_vis) {
        match (h, *l) {
            (None, _) => no_held.push((*c, *face)),
            (Some(_), None) => no_landed.push((*c, *face)),
            (Some(h), Some(l)) => {
                let d = byte(*h) - byte(l);
                worst = worst.max(d);
                if d > 1 {
                    over.push((*c, *face, byte(*h), byte(l)));
                }
            }
        }
    }
    // (b) the box's mean against the settled frame's.
    let settled = &shots[SHAFT_READ - 1];
    let in_box = |h: [f32; 4]| {
        h[3] < 1e30 && (0..3).all(|k| h[k] >= last.0[k] as f32 - 1.0 && h[k] <= last.1[k] as f32 + 1.0)
    };
    let excess: Vec<(usize, usize, f64)> = shots
        .iter()
        .enumerate()
        .map(|(k, s)| {
            let px: Vec<usize> = (0..s.luma.len())
                .filter(|&i| in_box(s.hit[i]) && in_box(settled.hit[i]))
                .collect();
            let mean = |l: &[f32]| floor_mean(l, &px);
            (k, px.len(), mean(&s.luma) - mean(&settled.luma))
        })
        .collect();
    let end = landed.unwrap_or(SHAFT_READ - 1);
    let window = &excess[..=end];
    let (peak_k, _, peak) = window
        .iter()
        .copied()
        .max_by(|a, b| a.2.total_cmp(&b.2))
        .expect("a frame");
    let cast = |s: &DigShot| {
        let n = (0..s.cast.len()).filter(|&i| in_box(s.hit[i])).count().max(1);
        100.0 * (0..s.cast.len()).filter(|&i| in_box(s.hit[i]) && s.cast[i]).count() as f64 / n as f64
    };
    println!(
        "the shaft: frames to settle (start, digs 1 to 3) {steps:?}; the last dig landed at D+{landed:?}; settled frame D+{}",
        SHAFT_READ - 1
    );
    println!(
        "(a) {} dug cells by direction: {} with no held record {no_held:?}; {} held with no landed value {no_landed:?}; held over landed by more than one byte {over:?}; the largest held over landed {worst} bytes",
        cells.len(),
        no_held.len(),
        no_landed.len()
    );
    let rows: Vec<String> = window
        .iter()
        .map(|(k, n, e)| format!("D+{k} {e:+.3} ({n} px, cast {:.0} %)", cast(&shots[*k])))
        .collect();
    println!("(b) the box's mean over the settled frame's, D+0 to the landing: {}", rows.join("; "));
    println!(
        "(b) the peak {peak:+.3} levels at D+{peak_k}; the settled frame's mean {:.2} over {} px",
        floor_mean(&settled.luma, &(0..settled.luma.len()).filter(|&i| in_box(settled.hit[i])).collect::<Vec<_>>()),
        excess[SHAFT_READ - 1].1
    );
    let at = landed.expect("rig fault: the last dig's job did not land in the read frames");
    assert!(
        at + SHAFT_SETTLE_AFTER < SHAFT_READ,
        "rig fault: the landing at D+{at} leaves the settled frame under {SHAFT_SETTLE_AFTER} frames after it"
    );
    assert!(!cells.is_empty(), "rig fault: the last dig made no face");
    assert!(excess[SHAFT_READ - 1].1 > 0, "rig fault: the last dig's box is not in view");
    assert!(
        over.is_empty(),
        "dug cells held over their landed byte by more than one: {over:?}"
    );
    assert!(
        peak <= 4.0,
        "the box's mean over the settled frame's by {peak:+.3} levels at D+{peak_k}"
    );
}
