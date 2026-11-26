//! Silent for check-test-clocks.mjs: the bake thread on, and a module this file declares names
//! the blocking wait, so the frames it counts wait for each step.

mod support;

#[test]
fn thread_on_and_stepped() {
    let mut q = support::Q::new();
    q.frame.set_bake_thread(&q.queue, true);
    q.step();
    assert!(q.landed());
}
