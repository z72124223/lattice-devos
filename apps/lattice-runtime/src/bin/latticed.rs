use std::process::ExitCode;

fn main() -> ExitCode {
    if std::env::args_os().len() != 1 {
        eprintln!("LATTICED_ARGUMENTS_REJECTED");
        return ExitCode::from(2);
    }
    match lattice_runtime::composition::serve_stdio_from_environment() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{}", error.code());
            ExitCode::from(2)
        }
    }
}
