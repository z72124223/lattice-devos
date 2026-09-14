use std::process::ExitCode;

fn main() -> ExitCode {
    if std::env::args_os().len() != 1 {
        eprintln!("LATTICE_FULL_CHAIN_ARGUMENTS_REJECTED");
        return ExitCode::from(2);
    }
    eprintln!("LATTICE_FULL_CHAIN_RETIRED");
    ExitCode::from(2)
}
