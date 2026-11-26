//! Seeded for check-test-clocks.mjs: rule 3, the bake thread on with no blocking wait.

mod rig;

#[test]
fn thread_on_and_counted() {
    let mut q = rig::Q::new();
    // The bake thread runs at its own pace while the test counts frames.
    q.frame.set_bake_thread(&q.queue, true);
    let mut frames = 0;
    while !q.landed() {
        q.step();
        frames += 1;
    }
    assert!(frames <= 3);
}
