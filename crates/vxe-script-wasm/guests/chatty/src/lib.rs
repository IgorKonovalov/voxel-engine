//! A logic whose `tick` writes 1 KiB to stderr, and at tick 100 panics: far more stderr over a
//! run than the host keeps for one call, and a guest panic whose message ends up in the trap.

use vxe_net::{ClientInput, Host, Logic, TickOutput};

#[derive(Default)]
struct Chatty;

impl Logic for Chatty {
    fn init(&mut self, _host: &mut dyn Host, _seed: u64) {}

    fn tick(&mut self, _host: &mut dyn Host, tick: u64, _inputs: &[ClientInput]) -> TickOutput {
        // 1023 characters and the newline.
        eprintln!("{}", "x".repeat(1023));
        assert!(tick != 100, "chatty stops at tick 100");
        TickOutput::default()
    }
}

vxe_script_wasm::export_logic!(Chatty);
