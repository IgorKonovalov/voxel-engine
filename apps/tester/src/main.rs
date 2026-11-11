//! `vxe-tester`: the tester build's launcher (ADR-0054).
//!
//! `vxe-tester --system-json PATH` writes the machine's `system.json` (`vxe_tester::system`)
//! to `PATH` without opening a window, prints it, and exits 0; a file that cannot be written
//! exits 1.

use vxe_tester::system::{SystemInfo, adapters};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [flag, path] if flag == "--system-json" => {
            let json = SystemInfo::collect(adapters(), None).to_json();
            print!("{json}");
            if let Err(e) = std::fs::write(path, &json) {
                eprintln!("vxe-tester: --system-json {path}: {e}");
                std::process::exit(1);
            }
        }
        _ => {
            eprintln!("vxe-tester: usage: vxe-tester --system-json PATH");
            std::process::exit(2);
        }
    }
}
