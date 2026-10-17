//! A logic whose `tick` first applies each input as one op in wire bytes through the host
//! (as `EchoLogic` does), then allocates and touches 1 GiB: the host's memory cap ends it,
//! after the ops are applied.

use vxe_net::test_logic::EchoLogic;
use vxe_net::{ClientInput, Host, Logic, TickOutput};

#[derive(Default)]
struct Hog;

impl Logic for Hog {
    fn init(&mut self, _host: &mut dyn Host, _seed: u64) {}

    fn tick(&mut self, host: &mut dyn Host, tick: u64, inputs: &[ClientInput]) -> TickOutput {
        let applied = EchoLogic.tick(host, tick, inputs);
        let block = std::hint::black_box(vec![1u8; 1 << 30]);
        TickOutput {
            broadcast: [&applied.broadcast[..], &block[..8]].concat(),
        }
    }
}

vxe_script_wasm::export_logic!(Hog);
