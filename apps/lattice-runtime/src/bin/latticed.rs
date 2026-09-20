use std::process::ExitCode;

fn main() -> ExitCode {
    let mut arguments = std::env::args_os();
    let _program = arguments.next();
    if let Some(argument) = arguments.next() {
        if matches!(
            argument.to_str(),
            Some(
                "project-registry-inspect"
                    | "project-registry-reconcile"
                    | "project-registry-restore"
                    | "project-registry-restore-observe"
            )
        ) {
            let Ok(all) = std::iter::once(argument)
                .chain(arguments)
                .map(|a| a.into_string())
                .collect::<Result<Vec<_>, _>>()
            else {
                eprintln!("LATTICED_ARGUMENTS_REJECTED");
                return ExitCode::from(2);
            };
            return match lattice_runtime::parse_command(&all).and_then(lattice_runtime::execute) {
                Ok(value) => {
                    println!("{value}");
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("{}", error.code());
                    ExitCode::from(2)
                }
            };
        }
        if argument == "--graphify-configuration" && arguments.next().is_none() {
            return match lattice_runtime::composition::graphify_configuration_from_environment() {
                Ok(value) => {
                    println!("{value}");
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("{}", error.code());
                    ExitCode::from(2)
                }
            };
        }
        if argument == "--external-result-import" || argument == "--local-result-import" {
            let Some(path) = arguments.next() else {
                eprintln!("LATTICED_ARGUMENTS_REJECTED");
                return ExitCode::from(2);
            };
            if arguments.next().is_some() {
                eprintln!("LATTICED_ARGUMENTS_REJECTED");
                return ExitCode::from(2);
            }
            let result = if argument == "--local-result-import" {
                lattice_runtime::composition::import_local_result_from_environment(
                    std::path::Path::new(&path),
                )
            } else {
                lattice_runtime::composition::import_external_result_from_environment(
                    std::path::Path::new(&path),
                )
            };
            return match result {
                Ok(receipt) => {
                    println!("{receipt}");
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("{error}");
                    ExitCode::from(2)
                }
            };
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
        if argument == "--graphify-refresh" || argument == "--graphify-refresh-project" {
            let project_id = if argument == "--graphify-refresh-project" {
                match arguments.next().and_then(|value| value.into_string().ok()) {
                    Some(value) => Some(value),
                    None => {
                        eprintln!("LATTICED_ARGUMENTS_REJECTED");
                        return ExitCode::from(2);
                    }
                }
            } else {
                None
            };
            if arguments.next().is_some() {
                eprintln!("LATTICED_ARGUMENTS_REJECTED");
                return ExitCode::from(2);
            }
            let result = if let Some(project_id) = &project_id {
                lattice_runtime::composition::refresh_registered_project_graphify_from_environment(
                    project_id,
                )
            } else {
                lattice_runtime::composition::refresh_runtime_graphify_from_environment()
                    .map_err(|error| error.code())
            };
            return match result {
                Ok(receipt) => {
                    println!(
                        "{}",
                        serde_json::json!({
                            "component": "graphify", "status": "PERSISTED",
                            "commit": receipt.persistence().request().commit_id().as_str(),
                            "record_count": receipt.persistence().record_count(),
                            "retrieved_count": receipt.retrieval().results().len(),
                            "receipt_digest": receipt.receipt_digest().as_str(),
                            "registered_project_id": project_id,
                        })
                    );
                    eprintln!("LATTICE_GRAPHIFY_REFRESH_READY");
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("{error}");
                    ExitCode::from(2)
                }
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
        if argument == "--postgres-initialize" && arguments.next().is_none() {
            return match lattice_runtime::composition::initialize_runtime_postgres_from_environment(
            ) {
                Ok(()) => {
                    eprintln!("LATTICE_POSTGRES_INITIALIZE_READY");
                    ExitCode::SUCCESS
                }
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
