//! A logic whose `tick` sleeps 30 s, then broadcasts one byte: 1 when a fresh 1 s timer is
//! already ready, 0 otherwise. Real timers would block the host for the sleep and make the
//! byte depend on elapsed wall time; the host's timers are always ready.

use std::time::Duration;

use vxe_net::{ClientInput, Host, Logic, TickOutput};
use wasip2::clocks::monotonic_clock::subscribe_duration;

#[derive(Default)]
struct Timer;

impl Logic for Timer {
    fn init(&mut self, _host: &mut dyn Host, _seed: u64) {}

    fn tick(&mut self, _host: &mut dyn Host, _tick: u64, _inputs: &[ClientInput]) -> TickOutput {
        std::thread::sleep(Duration::from_secs(30));
        TickOutput {
            broadcast: vec![u8::from(subscribe_duration(1_000_000_000).ready())],
        }
    }
}

vxe_script_wasm::export_logic!(Timer);
