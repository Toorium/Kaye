//! Makes the installer: `kaye-pack <setup program> <kaye.exe> <output>`.

use std::process::ExitCode;

fn run(setup: &str, program: &str, output: &str) -> std::io::Result<()> {
    std::fs::write(output, kaye_setup::pack::pack(&std::fs::read(setup)?, &std::fs::read(program)?)?)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [setup, program, output] = args.as_slice() else {
        eprintln!("usage: kaye-pack <setup program> <kaye.exe> <output>");
        return ExitCode::FAILURE;
    };
    match run(setup, program, output) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
