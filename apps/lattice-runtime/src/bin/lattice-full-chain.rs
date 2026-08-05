use std::process::ExitCode;

fn main() -> ExitCode {
    if std::env::args_os().len() != 1 {
        eprintln!("LATTICE_FULL_CHAIN_ARGUMENTS_REJECTED");
        return ExitCode::from(2);
    }
    match lattice_runtime::composition::serve_full_chain_from_environment() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{}", error.code());
            ExitCode::from(2)
        }
    }
}
