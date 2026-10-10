//! Dedicated local maintenance binary. Never loaded by the ordinary MCP runtime.
use lattice_postgres_store::{
    connect_project_purge, execute_bot_project_purge, execute_project_purge_with_graph_source,
    inspect_project_purge_bot_lifecycle, inspect_project_purge_graph,
    install_bot_project_ownership,
};
use std::io::{self, Read};
use std::process::ExitCode;
fn run() -> Result<serde_json::Value, &'static str> {
    lattice_runtime::initialize_registry_anchor_file_audit()?;
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
    let mut request: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| "PROJECT_PURGE_INPUT_REJECTED")?;
    let bot_service = request
        .as_object_mut()
        .ok_or("PROJECT_PURGE_INPUT_REJECTED")?
        .remove("botService");
    let port = std::env::var("LATTICE_TASK019_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .ok_or("PROJECT_PURGE_CONFIGURATION_REJECTED")?;
    let run_id = std::env::var("LATTICE_TASK019_RUN_ID")
        .map_err(|_| "PROJECT_PURGE_CONFIGURATION_REJECTED")?;
    let password = std::env::var("LATTICE_TASK019_PASSWORD")
        .map_err(|_| "PROJECT_PURGE_CONFIGURATION_REJECTED")?;
    if request["action"] == "reconcile-archived-claim" {
        if bot_service.is_some() {
            return Err("CLAIM_RECONCILIATION_INPUT_REJECTED");
        }
        return lattice_runtime::claim_reconciliation::execute(&request, port, &run_id, &password);
    }
    let inspect_bot = |project_id: &str| -> Result<serde_json::Value, &'static str> {
        let service = bot_service
            .as_ref()
            .and_then(serde_json::Value::as_object)
            .ok_or("BOT_LIFECYCLE_SERVICE_NOT_CONFIGURED")?;
        if service.len() != 3
            || service
                .keys()
                .any(|key| !["port", "runId", "systemIdentifier"].contains(&key.as_str()))
        {
            return Err("BOT_LIFECYCLE_CONFIGURATION_REJECTED");
        }
        let bot_port = service["port"]
            .as_u64()
            .and_then(|v| u16::try_from(v).ok())
            .ok_or("BOT_LIFECYCLE_CONFIGURATION_REJECTED")?;
        let bot_run = service["runId"]
            .as_str()
            .ok_or("BOT_LIFECYCLE_CONFIGURATION_REJECTED")?;
        let system = service["systemIdentifier"]
            .as_str()
            .ok_or("BOT_LIFECYCLE_CONFIGURATION_REJECTED")?;
        inspect_project_purge_bot_lifecycle(bot_port, bot_run, &password, system, project_id)
    };
    if request["action"] == "inspect-bot" {
        if request.as_object().unwrap().len() != 3
            || request["schema"] != "lattice.project-purge.request.v1"
        {
            return Err("PROJECT_PURGE_INPUT_REJECTED");
        }
        return inspect_bot(
            request["projectId"]
                .as_str()
                .ok_or("PROJECT_PURGE_INPUT_REJECTED")?,
        );
    }
    if matches!(
        request["action"].as_str(),
        Some("install-bot-ownership" | "preview-bot" | "apply-bot" | "status-bot")
    ) {
        let service = bot_service
            .as_ref()
            .and_then(serde_json::Value::as_object)
            .ok_or("BOT_LIFECYCLE_SERVICE_NOT_CONFIGURED")?;
        if service.len() != 3
            || service
                .keys()
                .any(|key| !["port", "runId", "systemIdentifier"].contains(&key.as_str()))
        {
            return Err("BOT_LIFECYCLE_CONFIGURATION_REJECTED");
        }
        let bot_port = service["port"]
            .as_u64()
            .and_then(|v| u16::try_from(v).ok())
            .ok_or("BOT_LIFECYCLE_CONFIGURATION_REJECTED")?;
        let bot_run = service["runId"]
            .as_str()
            .ok_or("BOT_LIFECYCLE_CONFIGURATION_REJECTED")?;
        let system = service["systemIdentifier"]
            .as_str()
            .ok_or("BOT_LIFECYCLE_CONFIGURATION_REJECTED")?;
        if request["action"] == "install-bot-ownership" {
            if request.as_object().unwrap().len() != 3
                || request["schema"] != "lattice.project-purge.request.v1"
                || request["authorization"] != "INSTALL_BOT_PROJECT_OWNERSHIP"
            {
                return Err("BOT_LIFECYCLE_INPUT_REJECTED");
            }
            return install_bot_project_ownership(bot_port, bot_run, &password, system);
        }
        let (mut client, target) = connect_project_purge(port, &run_id, &password)?;
        return execute_bot_project_purge(
            &mut client,
            &target,
            bot_port,
            bot_run,
            &password,
            system,
            &request,
        );
    }
    let (mut client, target) = connect_project_purge(port, &run_id, &password)?;
    let mut result = execute_project_purge_with_graph_source(
        &mut client,
        &target,
        &request,
        Some(&lattice_runtime::composition::project_purge_graph_source),
    )?;
    if let Some(project_id) = request.get("projectId").and_then(serde_json::Value::as_str) {
        let observed = inspect_bot(project_id).unwrap_or_else(|code| {
            serde_json::json!({
                "schema":"lattice.project-purge.bot-inventory.v1", "ownership":"LATTICE",
                "discovery":"NOT_VERIFIED", "reason":code, "erasureImplemented":false,
            })
        });
        // Preview already committed the identical graph-only row stream inside
        // its read-only snapshot. Reuse it instead of sorting every record twice.
        let graph=result.as_object_mut().and_then(|value| value.remove("runtimeGraphInventory")).unwrap_or_else(||inspect_project_purge_graph(&mut client).unwrap_or_else(|code|
            serde_json::json!({"schema":"lattice.project-purge.graph-inventory.v1","ownership":"LATTICE","discovery":"NOT_VERIFIED","reason":code})));
        result["relatedStores"] = serde_json::json!({"botLifecycle":observed,"runtimeGraph":graph});
        if let Some(canonical) = result["project"]["canonicalPath"].as_str() {
            result["runtimeGraphSource"] =
                match lattice_runtime::composition::project_purge_graph_source_key(
                    std::path::Path::new(canonical),
                ) {
                    Ok(key) => {
                        serde_json::json!({"sourceKey":key,"binding":"REGISTRY_CANONICAL_PATH"})
                    }
                    Err(_) => {
                        serde_json::json!({"binding":"NOT_VERIFIED","reason":"REGISTERED_SOURCE_DIRECTORY_UNAVAILABLE"})
                    }
                };
        }
    }
    Ok(result)
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
