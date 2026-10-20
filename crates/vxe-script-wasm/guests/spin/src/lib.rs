//! A logic whose `tick` loops forever: the host's fuel budget ends it.

use vxe_net::{ClientInput, Host, Logic, TickOutput};

#[derive(Default)]
struct Spin;

impl Logic for Spin {
    fn init(&mut self, _host: &mut dyn Host, _seed: u64) {}

    fn tick(&mut self, _host: &mut dyn Host, _tick: u64, _inputs: &[ClientInput]) -> TickOutput {
        let mut n = 0u64;
        loop {
            n = std::hint::black_box(n.wrapping_add(1));
        }
    }
}

vxe_script_wasm::export_logic!(Spin);
