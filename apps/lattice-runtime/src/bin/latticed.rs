use std::process::ExitCode;

fn main() -> ExitCode {
    let mut arguments = std::env::args_os();
    let _program = arguments.next();
    if let Some(argument) = arguments.next() {
        if argument == "--hermes-preflight" && arguments.next().is_none() {
            let preflight =
                lattice_runtime::composition::hermes_production_preflight_from_environment();
            eprintln!("{}", preflight.render());
            return ExitCode::from(2);
        }
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
