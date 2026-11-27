//! The start-up's `threads:` line: the workers chosen, the listers they leave, the sum.

use crate::cli::parse_args_from;
use crate::test_support::argv;
use svet_render::ThreadPlan;

/// The line for 16 logical CPUs at the default and with `--workers 15`, from fixed counts, not
/// from the machine's: 15 workers leave the listers their floor of one, two over the CPUs.
#[test]
fn the_threads_line_names_the_sum_and_the_overrun() {
    let default = parse_args_from(argv(&[])).expect("no flags parse").workers;
    assert_eq!(
        ThreadPlan::for_logical(16, default).to_string(),
        "threads: 8 workers + 6 listers + render + bake = 16 of 16 logical CPUs"
    );
    let chosen = parse_args_from(argv(&["--workers", "15"]))
        .expect("--workers 15 parses")
        .workers;
    assert_eq!(
        ThreadPlan::for_logical(16, chosen).to_string(),
        "threads: 15 workers + 1 listers + render + bake = 18 of 16 logical CPUs (over by 2)"
    );
}
