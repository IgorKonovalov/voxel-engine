//! The engine's testbed (milestone 1 in `docs/roadmap.md`): a window showing the ray-marched
//! cave lit by a headlamp. Fly, dig, and read the per-material counts each edit returns.
//!
//! The flags and keys are documented on `flythrough::cli`; the run lives in the library.

fn main() {
    // The fly-through's own lines and the shell's teardown trace show by default; RUST_LOG
    // overrides.
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("error,vxe_app=info,flythrough=info"),
    )
    .init();
    let start = flythrough::app::start_clock();
    let args = match flythrough::cli::parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("flythrough: {e}");
            std::process::exit(2);
        }
    };
    if args.help {
        print!("{}", flythrough::cli::help());
        return;
    }
    if args.check_args {
        println!("flythrough: arguments ok");
        return;
    }
    let app = flythrough::setup::flythrough(start, args);
    let result = flythrough::app::run(app);
    log::info!("teardown: done, exiting the process");
    if let Err(e) = result {
        eprintln!("flythrough: {e}");
        std::process::exit(1);
    }
}
