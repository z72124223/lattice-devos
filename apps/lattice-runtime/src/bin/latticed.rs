use std::process::ExitCode;

fn run_hermes_prepare() -> ExitCode {
    let Some(preparation_root) = std::env::var_os("LATTICE_HERMES_PREPARATION_ROOT") else {
        eprintln!("LATTICE_HERMES_PREPARE_MISSING_CONFIGURATION:LATTICE_HERMES_PREPARATION_ROOT");
        return ExitCode::from(2);
    };
    let Some(product_root) = std::env::var_os("LATTICE_HERMES_PRODUCT_ROOT") else {
        eprintln!("LATTICE_HERMES_PREPARE_MISSING_CONFIGURATION:LATTICE_HERMES_PRODUCT_ROOT");
        return ExitCode::from(2);
    };
    match lattice_hermes_adapter::preparation::materialize_official_preparation_bundle(
        std::path::Path::new(&preparation_root),
        std::path::Path::new(&product_root),
    ) {
        Ok(outcome) => {
            eprintln!("{}", outcome.render());
            ExitCode::SUCCESS
        }
        Err(error) => {
            let classification = match error.code() {
                "HERMES_PREPARATION_TARGET_REJECTED" => "TARGET_REJECTED",
                "HERMES_PREPARATION_ASSET_CONFLICT" => "ASSET_CONFLICT",
                "HERMES_PREPARATION_WRITE_REJECTED" => "WRITE_REJECTED",
                "HERMES_PREPARATION_RECONCILIATION_REQUIRED" => "RECONCILIATION_REQUIRED",
                _ => "PREPARATION_REJECTED",
            };
            eprintln!("LATTICE_HERMES_PREPARE_{classification}");
            ExitCode::from(2)
        }
    }
}

fn main() -> ExitCode {
    let mut arguments = std::env::args_os();
    let _program = arguments.next();
    if let Some(argument) = arguments.next() {
        if argument == "--hermes-prepare" && arguments.next().is_none() {
            return run_hermes_prepare();
        }
        if argument == "--hermes-preflight" && arguments.next().is_none() {
            let preflight =
                lattice_runtime::composition::hermes_production_preflight_from_environment();
            eprintln!("{}", preflight.render());
            return ExitCode::from(2);
        }
        if argument == "--hermes-runtime-preflight" && arguments.next().is_none() {
            let preflight =
                lattice_runtime::composition::hermes_runtime_preflight_from_environment();
            eprintln!("{}", preflight.render());
            return ExitCode::from(2);
        }
        if argument == "--graphify-runtime-preflight" && arguments.next().is_none() {
            let preflight =
                lattice_runtime::composition::graphify_runtime_preflight_from_environment();
            eprintln!("{}", preflight.render());
            return if preflight.is_identity_verified() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(2)
            };
        }
        if argument == "--postgres-bootstrap" && arguments.next().is_none() {
            return match lattice_runtime::composition::bootstrap_postgres_extensions_from_environment() {
                Ok(()) => {
                    eprintln!("LATTICE_POSTGRES_BOOTSTRAP_READY");
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("{}", error.code());
                    ExitCode::from(2)
                }
            };
        }
        if argument == "--hermes-launch" && arguments.next().is_none() {
            return match lattice_runtime::composition::launch_hermes_from_environment() {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("{}", error.code());
                    ExitCode::from(2)
                }
            };
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
