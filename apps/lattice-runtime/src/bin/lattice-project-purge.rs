//! Dedicated local maintenance binary. Never loaded by the ordinary MCP runtime.
use lattice_postgres_store::{connect_project_purge, execute_project_purge};
use std::io::{self, Read};
use std::process::ExitCode;
fn run() -> Result<serde_json::Value, &'static str> {
    if std::env::args_os().len() != 1 {
        return Err("PROJECT_PURGE_INPUT_REJECTED");
    }
    let mut bytes = Vec::new();
    io::stdin()
        .take(65537)
        .read_to_end(&mut bytes)
        .map_err(|_| "PROJECT_PURGE_INPUT_REJECTED")?;
    if bytes.len() > 65536 {
        return Err("PROJECT_PURGE_INPUT_REJECTED");
    }
    let request = serde_json::from_slice(&bytes).map_err(|_| "PROJECT_PURGE_INPUT_REJECTED")?;
    let port = std::env::var("LATTICE_TASK019_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .ok_or("PROJECT_PURGE_CONFIGURATION_REJECTED")?;
    let run_id = std::env::var("LATTICE_TASK019_RUN_ID")
        .map_err(|_| "PROJECT_PURGE_CONFIGURATION_REJECTED")?;
    let password = std::env::var("LATTICE_TASK019_PASSWORD")
        .map_err(|_| "PROJECT_PURGE_CONFIGURATION_REJECTED")?;
    let (mut client, target) = connect_project_purge(port, &run_id, &password)?;
    execute_project_purge(&mut client, &target, &request)
}
fn main() -> ExitCode {
    match run() {
        Ok(value) => {
            println!("{value}");
            ExitCode::SUCCESS
        }
        Err(code) => {
            eprintln!("{code}");
            ExitCode::from(2)
        }
    }
}
