//! Golden images of the cave on the software adapter (lavapipe). `VXE_BLESS=1` rewrites them.

mod support;

use support::*;
use vxe_render::{GpuWorld, Marcher, readback};
use vxe_world::World;

fn render(world: &World) -> Option<Vec<u8>> {
    let full = world.stats().full_bricks as u64;
    let (device, queue) = software_device((full + 4096) * 1024)?;
    let mut gpu = GpuWorld::new(&device, world.extent());
    gpu.sync(world, &queue).expect("mirror fits the adapter");
    let marcher = Marcher::new(&device, WIDTH, HEIGHT);
    marcher.render(&device, &queue, &gpu, &scene_camera());
    Some(readback::read_rgba8(&device, &queue, marcher.target()))
}

#[test]
fn primary_golden_shows_the_carve() {
    let world = scene_world();
    let r = world.apply(&scene_carve(&world)).unwrap();
    assert!(!r.changed_from.is_empty(), "the carve removes rock");
    let Some(carved) = render(&world) else { return };
    assert_golden("primary.png", &carved);

    // The golden holds structure: at least two colour bins each cover 5 % of the pixels.
    let (golden, _, _) = read_png(&golden_path("primary.png"));
    let bins = bins_covering(&golden, 0.05);
    println!("bins covering 5 %: {bins}");
    assert!(bins >= 2, "{bins} bins");

    // The carve is visible: the uncarved scene differs from the golden.
    let plain = render(&scene_world()).unwrap();
    let f = fraction_differing(&plain, &golden, 8);
    println!(
        "uncarved differs from golden at {:.2} % of pixels",
        f * 100.0
    );
    assert!(f >= 0.02, "{:.2} %", f * 100.0);
}
