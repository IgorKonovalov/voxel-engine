//! Seeded for check-test-clocks.mjs: rule 4, timestamps on a device a test steps a queue on.

fn device() -> wgpu::Device {
    open(wgpu::Features::empty())
}

/// Rule 4: the step's sample cap would adapt to measured time.
fn timed_device() -> wgpu::Device {
    open(wgpu::Features::TIMESTAMP_QUERY)
}

#[test]
fn steps_counted_on_a_timed_device() {
    let mut q = queue(timed_device());
    q.step();
    let _ = device();
}

mod reading {
    /// Silent: a reading may time the step.
    #[test]
    #[ignore = "reading"]
    fn timed_steps() {
        let mut q = super::queue(super::open(wgpu::Features::TIMESTAMP_QUERY));
        q.step();
    }
}
