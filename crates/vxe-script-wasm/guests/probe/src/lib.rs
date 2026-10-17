//! A logic that reports what it can observe of its environment. Its `tick` broadcasts, in
//! order:
//!
//! | field                                  | bytes                         |
//! |----------------------------------------|-------------------------------|
//! | bits of `0.0f32 / 0.0f32`              | `u32` LE                      |
//! | monotonic clock, ns                    | `u64` LE                      |
//! | wall clock, ns since the epoch         | `u64` LE                      |
//! | `getrandom` output                     | 8                             |
//! | iteration order of a `HashMap` of 1..=8 | 8, one key each               |
//! | opening `/` succeeded                  | 1 (`0` failed, `1` succeeded) |
//! | reading `HOME` succeeded               | 1 (`0` failed, `1` succeeded) |

use std::collections::HashMap;
use std::hint::black_box;
use std::time::{SystemTime, UNIX_EPOCH};

use vxe_net::{ClientInput, Host, Logic, TickOutput};

// The monotonic clock's `now`, which the standard library hides behind `Instant`. Its type
// information reaches the component through the standard library's own use of the interface.
#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "wasi:clocks/monotonic-clock@0.2.0")]
unsafe extern "C" {
    #[link_name = "now"]
    safe fn monotonic_now() -> u64;
}

// The workspace build also compiles this crate for the host, where the import has no
// definition and a DLL refuses to link with an unresolved symbol. The probe only runs as a
// component, so the host build reads zero.
#[cfg(not(target_arch = "wasm32"))]
fn monotonic_now() -> u64 {
    0
}

#[derive(Default)]
struct Probe;

impl Logic for Probe {
    fn init(&mut self, _host: &mut dyn Host, _seed: u64) {}

    #[allow(
        clippy::disallowed_methods,
        reason = "the probe reads the wall clock to show that the host fixes it"
    )]
    fn tick(&mut self, _host: &mut dyn Host, _tick: u64, _inputs: &[ClientInput]) -> TickOutput {
        let mut out = Vec::new();
        let nan = black_box(0.0f32) / black_box(0.0f32);
        out.extend_from_slice(&nan.to_bits().to_le_bytes());
        out.extend_from_slice(&monotonic_now().to_le_bytes());
        let wall = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(u64::MAX, |d| {
                u64::try_from(d.as_nanos()).unwrap_or(u64::MAX)
            });
        out.extend_from_slice(&wall.to_le_bytes());
        let mut random = [0u8; 8];
        getrandom::fill(&mut random).expect("getrandom serves eight bytes");
        out.extend_from_slice(&random);
        let map: HashMap<u8, ()> = (1..=8).map(|k| (k, ())).collect();
        out.extend(map.keys().copied());
        out.push(u8::from(std::fs::File::open("/").is_ok()));
        out.push(u8::from(std::env::var("HOME").is_ok()));
        TickOutput { broadcast: out }
    }
}

vxe_script_wasm::export_logic!(Probe);
