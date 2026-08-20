use std::process::ExitCode;

fn main() -> ExitCode {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    let result = lattice_runtime::parse_command(&arguments).and_then(lattice_runtime::execute);
    match result {
        Ok(evidence) => {
            println!("{evidence}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{}", error.error_envelope());
            ExitCode::from(2)
        }
    }
}
