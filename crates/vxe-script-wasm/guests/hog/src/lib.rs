//! A logic whose `tick` allocates and touches 1 GiB: the host's memory cap ends it.

use vxe_net::{ClientInput, Host, Logic, TickOutput};

#[derive(Default)]
struct Hog;

impl Logic for Hog {
    fn init(&mut self, _host: &mut dyn Host, _seed: u64) {}

    fn tick(&mut self, _host: &mut dyn Host, _tick: u64, _inputs: &[ClientInput]) -> TickOutput {
        let block = std::hint::black_box(vec![1u8; 1 << 30]);
        TickOutput {
            broadcast: block[..8].to_vec(),
        }
    }
}

vxe_script_wasm::export_logic!(Hog);
