//! The listing pool (Plan 0099 Phase 4, lever (b)) on the device-less rig: the openings hand
//! their batches to the pool and push what it finished in a later step.

use super::*;

/// Steps `rig` until its queue is empty, sleeping a little while the listing pool holds
/// candidates (its threads run beside the rig's steps); the frames it took.
fn drain(rig: &mut Rig) -> u32 {
    for frame in 0..4000 {
        if rig.read().empty() && rig.queue.listing.is_empty() {
            return frame;
        }
        if !rig.queue.listing.is_empty() {
            std::thread::sleep(std::time::Duration::from_micros(200));
        }
        rig.step();
    }
    panic!("the queue drains within 4000 frames");
}

/// Every chunk's block words, by chunk.
fn blocks(rig: &Rig) -> Vec<([i32; 3], Vec<u32>)> {
    let mut out: Vec<_> = rig
        .world
        .store()
        .coords()
        .into_iter()
        .filter_map(|c| rig.store.block_words(c).map(|w| (c.0, w.to_vec())))
        .collect();
    out.sort();
    out
}

/// A sliced rig drained with its listings handed to the pool holds every chunk's block as the
/// rig that lists in its own step does, and pushes no chunk twice: the pool changes when a job
/// opens, not what it bakes.
#[test]
fn the_listing_pool_drains_to_the_blocks_the_step_lists() {
    let mut step = Rig::new(None);
    let mut pool = Rig::new(None);
    pool.queue
        .start_lister(Arc::new(world_of(CHUNKS[0])))
        .expect("the pool's threads start");
    assert!(pool.queue.lists_off_the_loop());
    let (a, b) = (drain(&mut step), drain(&mut pool));
    let (sa, sb) = (blocks(&step), blocks(&pool));
    assert!(!sa.is_empty(), "the rig bakes something");
    assert_eq!(
        sa.iter().map(|(c, w)| (*c, w.len())).collect::<Vec<_>>(),
        sb.iter().map(|(c, w)| (*c, w.len())).collect::<Vec<_>>(),
        "the same chunks hold blocks of the same size (drained in {a} and {b} frames)"
    );
    assert_eq!(sa, sb, "every block's words are the same");
    pool.queue.stop_lister();
    assert!(!pool.queue.lists_off_the_loop());
}
