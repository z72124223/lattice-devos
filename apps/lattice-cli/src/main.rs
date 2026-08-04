use std::process::ExitCode;

fn main() -> ExitCode {
    let arguments: Vec<_> = std::env::args().skip(1).collect();

    match lattice_cli::dispatch(&arguments) {
        Ok(output) => {
            print!("{output}");
            ExitCode::SUCCESS
        }
        Err(usage) => {
            eprintln!("{usage}");
            ExitCode::from(2)
        }
    }
}
