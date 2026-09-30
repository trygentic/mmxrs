//! `mmx-signal` binary: see the library docs and `--help`.

use std::process::ExitCode;

use mmx_signal::{parse_args, serve, Command, USAGE};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (bind, verbose) = match parse_args(&args) {
        Ok(Command::Help) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Ok(Command::Run { bind, verbose }) => (bind, verbose),
        Err(e) => {
            eprintln!("mmx-signal: {e}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("mmx-signal: cannot start runtime: {e}");
            return ExitCode::from(1);
        }
    };
    match runtime.block_on(serve(bind, verbose)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mmx-signal: {e}");
            ExitCode::from(1)
        }
    }
}
