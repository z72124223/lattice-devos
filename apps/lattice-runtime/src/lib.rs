//! LATTICE runtime composition entry.

pub mod composition;
pub mod coordination;
pub mod delivery_ledger;
pub mod git_delivery;
pub mod mcp;
pub mod task_control;

use std::error::Error;
use std::fmt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use lattice_codex_adapter::{
    CodexIdentityErrorKind, CodexIdentityExpectation, preflight_codex_identity,
};
use lattice_contracts::DeliveryRuntime;
use serde_json::{Value, json};

use crate::composition::{LatticedDeliveryConfig, LatticedDeliveryService, LatticedErrorKind};
use crate::delivery_ledger::{
    DeliveryDatabaseBinding, DeliveryLedger, DeliveryLedgerErrorKind, connect_fixed_runtime_client,
};
use crate::git_delivery::GitDeliveryErrorKind;

const USAGE: &str = "usage:\n  lattice-runtime codex-preflight --launcher <absolute-path> --version <exact-version> --sha256 <lowercase-sha256> --schema-dir <absent-path>\n  lattice-runtime delivery-run --launcher <absolute-path> --version <exact-version> --sha256 <lowercase-sha256> --schema-dir <absent-path> --codex-home <absolute-path> --delivery-root <absent-absolute-path> --git-exe <absolute-path> --timeout-seconds <1..3600> --postgres-host 127.0.0.1 --postgres-port <ephemeral-port> --postgres-run-id <32-lowercase-hex>\n  lattice-runtime delivery-status --postgres-host 127.0.0.1 --postgres-port <ephemeral-port> --postgres-run-id <32-lowercase-hex>\n  lattice-runtime runtime-health --postgres-host 127.0.0.1 --postgres-port <ephemeral-port> --postgres-run-id <32-lowercase-hex>\n  lattice-runtime receipt-state --postgres-host 127.0.0.1 --postgres-port <ephemeral-port> --postgres-run-id <32-lowercase-hex>";

const DELIVERY_PROMPT: &str = concat!(
    "Create answer.txt in the current repository with exactly the bytes LATTICE_DELIVERY_OK followed by one newline. ",
    "Use code mode and one standalone exec call whose JavaScript invokes nested tools.shell_command for the write. ",
    "Keep the inherited workspaceWrite sandbox and current working directory: Do not set sandbox_permissions, request escalation, or pass a different workdir. ",
    "The nested PowerShell command must be exactly [System.IO.File]::WriteAllBytes('answer.txt',[byte[]](76,65,84,84,73,67,69,95,68,69,76,73,86,69,82,89,95,79,75,10)), producing 20 bytes ending in LF. ",
    "In every exec assign the awaited nested result to result and, before calling text(result), run this fail-closed check: if (typeof result !== \"string\" || !/^Exit code: 0(?:\\r?\\n|$)/.test(result)) { throw new Error(\"nested shell_command failed\"); }. ",
    "Do not call tools.apply_patch. The write exec must perform no verification or other tool work. ",
    "Confirm that call has completed, then use a separate verification exec call whose JavaScript invokes nested tools.shell_command. ",
    "That verification call's nested PowerShell command must be exactly $bytes=[System.IO.File]::ReadAllBytes('answer.txt'); $expected=[byte[]](76,65,84,84,73,67,69,95,68,69,76,73,86,69,82,89,95,79,75,10); if ($bytes.Length -ne $expected.Length -or (Compare-Object -ReferenceObject $expected -DifferenceObject $bytes -SyncWindow 0)) { exit 1 }. ",
    "The second exec is only the exact-byte verification test. Do not combine file creation and verification in the same exec call. If any exec result says Script running with cell ID, call functions.wait with that exact cell_id until Script completed is received, and require exit code 0 before reporting success. ",
    "Never terminate a yielded cell or claim completion from a running marker. Do not modify any other path. Do not run Git commands, stage, or commit files; LATTICE performs scope inspection, the fixed project test, and Git commit afterward."
);

/// Closed command surface for the first delivery node.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeCommand {
    CodexPreflight {
        launcher: PathBuf,
        version: String,
        sha256: String,
        schema_dir: PathBuf,
    },
    DeliveryStatus {
        database: DeliveryDatabaseBinding,
    },
    RuntimeHealth {
        database: DeliveryDatabaseBinding,
    },
    ReceiptState {
        database: DeliveryDatabaseBinding,
    },
    DeliveryRun {
        launcher: PathBuf,
        version: String,
        sha256: String,
        schema_dir: PathBuf,
        codex_home: PathBuf,
        delivery_root: PathBuf,
        git_exe: PathBuf,
        timeout_seconds: u64,
        database: DeliveryDatabaseBinding,
    },
}

/// Stable command-line failures without sensitive process output.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeError {
    Usage,
    InvalidDigest,
    InvalidTimeout,
    CodexIdentity(CodexIdentityErrorKind),
    MissingDatabaseSecret,
    DeliveryLedger(DeliveryLedgerErrorKind),
    DeliveryLedgerOperation(DeliveryLedgerStage, DeliveryLedgerErrorKind),
    GitDelivery(GitDeliveryErrorKind),
    Latticed(LatticedErrorKind),
}

/// Exact durable-ledger operation that failed, without SQL or credentials.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryLedgerStage {
    Connect,
    Intent,
    Outcome,
    Receipt,
}

impl RuntimeError {
    /// Returns the stable diagnostic code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Usage => "LATTICE_RUNTIME_USAGE",
            Self::InvalidDigest => "LATTICE_RUNTIME_INVALID_DIGEST",
            Self::InvalidTimeout => "LATTICE_RUNTIME_INVALID_TIMEOUT",
            Self::CodexIdentity(_) => "LATTICE_RUNTIME_CODEX_IDENTITY_REJECTED",
            Self::MissingDatabaseSecret => "LATTICE_RUNTIME_DATABASE_SECRET_MISSING",
            Self::DeliveryLedger(_) => "LATTICE_RUNTIME_DELIVERY_LEDGER_REJECTED",
            Self::DeliveryLedgerOperation(stage, _) => match stage {
                DeliveryLedgerStage::Connect => "LATTICE_RUNTIME_DELIVERY_CONNECT_REJECTED",
                DeliveryLedgerStage::Intent => "LATTICE_RUNTIME_DELIVERY_INTENT_REJECTED",
                DeliveryLedgerStage::Outcome => "LATTICE_RUNTIME_DELIVERY_OUTCOME_REJECTED",
                DeliveryLedgerStage::Receipt => "LATTICE_RUNTIME_DELIVERY_RECEIPT_REJECTED",
            },
            Self::GitDelivery(_) => "LATTICE_RUNTIME_GIT_DELIVERY_REJECTED",
            Self::Latticed(kind) => kind.code(),
        }
    }
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage => formatter.write_str(USAGE),
            Self::InvalidDigest => formatter.write_str("expected one lowercase SHA-256 digest"),
            Self::InvalidTimeout => {
                formatter.write_str("timeout must be between 1 and 3600 seconds")
            }
            Self::CodexIdentity(kind) => write!(formatter, "Codex identity rejected: {kind:?}"),
            Self::MissingDatabaseSecret => {
                formatter.write_str("required PostgreSQL password environment is missing")
            }
            Self::DeliveryLedger(kind) => write!(formatter, "delivery ledger rejected: {kind:?}"),
            Self::DeliveryLedgerOperation(stage, kind) => {
                write!(formatter, "delivery ledger {stage:?} rejected: {kind:?}")
            }
            Self::GitDelivery(kind) => write!(formatter, "Git delivery rejected: {kind:?}"),
            Self::Latticed(kind) => formatter.write_str(kind.code()),
        }
    }
}

impl Error for RuntimeError {}

/// Parses the deliberately narrow first runtime command.
///
/// # Errors
///
/// Returns `RuntimeError::Usage` for missing, duplicated, or unknown options,
/// and `RuntimeError::InvalidDigest` for a non-canonical SHA-256 value.
#[allow(clippy::too_many_lines)]
pub fn parse_command(arguments: &[String]) -> Result<RuntimeCommand, RuntimeError> {
    let Some((command, options)) = arguments.split_first() else {
        return Err(RuntimeError::Usage);
    };
    match command.as_str() {
        "codex-preflight" => {
            let values = parse_options(
                options,
                &["--launcher", "--version", "--sha256", "--schema-dir"],
            )?;
            let sha256 = values[2].clone();
            if !is_lowercase_sha256(&sha256) {
                return Err(RuntimeError::InvalidDigest);
            }
            Ok(RuntimeCommand::CodexPreflight {
                launcher: PathBuf::from(&values[0]),
                version: values[1].clone(),
                sha256,
                schema_dir: PathBuf::from(&values[3]),
            })
        }
        "delivery-run" => {
            let values = parse_options(
                options,
                &[
                    "--launcher",
                    "--version",
                    "--sha256",
                    "--schema-dir",
                    "--codex-home",
                    "--delivery-root",
                    "--git-exe",
                    "--timeout-seconds",
                    "--postgres-host",
                    "--postgres-port",
                    "--postgres-run-id",
                ],
            )?;
            let sha256 = values[2].clone();
            if !is_lowercase_sha256(&sha256) {
                return Err(RuntimeError::InvalidDigest);
            }
            if [0_usize, 3, 4, 5, 6]
                .into_iter()
                .any(|index| !PathBuf::from(&values[index]).is_absolute())
            {
                return Err(RuntimeError::Usage);
            }
            let timeout_seconds = values[7]
                .parse::<u64>()
                .ok()
                .filter(|value| (1..=3600).contains(value))
                .ok_or(RuntimeError::InvalidTimeout)?;
            let database = parse_database_binding(&values[8], &values[9], &values[10])?;
            Ok(RuntimeCommand::DeliveryRun {
                launcher: PathBuf::from(&values[0]),
                version: values[1].clone(),
                sha256,
                schema_dir: PathBuf::from(&values[3]),
                codex_home: PathBuf::from(&values[4]),
                delivery_root: PathBuf::from(&values[5]),
                git_exe: PathBuf::from(&values[6]),
                timeout_seconds,
                database,
            })
        }
        "delivery-status" => {
            let values = parse_options(
                options,
                &["--postgres-host", "--postgres-port", "--postgres-run-id"],
            )?;
            let database = parse_database_binding(&values[0], &values[1], &values[2])?;
            Ok(RuntimeCommand::DeliveryStatus { database })
        }
        "runtime-health" => {
            let values = parse_options(
                options,
                &["--postgres-host", "--postgres-port", "--postgres-run-id"],
            )?;
            let database = parse_database_binding(&values[0], &values[1], &values[2])?;
            Ok(RuntimeCommand::RuntimeHealth { database })
        }
        "receipt-state" => {
            let values = parse_options(
                options,
                &["--postgres-host", "--postgres-port", "--postgres-run-id"],
            )?;
            let database = parse_database_binding(&values[0], &values[1], &values[2])?;
            Ok(RuntimeCommand::ReceiptState { database })
        }
        _ => Err(RuntimeError::Usage),
    }
}

/// Executes one parsed local runtime command.
///
/// # Errors
///
/// Returns a typed identity rejection when the pinned Codex launcher or its
/// generated schema does not match the supplied expectation.
#[allow(clippy::too_many_lines)]
pub fn execute(command: RuntimeCommand) -> Result<Value, RuntimeError> {
    match command {
        RuntimeCommand::CodexPreflight {
            launcher,
            version,
            sha256,
            schema_dir,
        } => {
            let expectation = CodexIdentityExpectation::new(launcher.clone(), version, sha256);
            let evidence = preflight_codex_identity(&launcher, &expectation, &schema_dir)
                .map_err(|error| RuntimeError::CodexIdentity(error.kind()))?;
            Ok(json!({
                "status": "READY",
                "component": "codex",
                "launcher_path": evidence.launcher_path().to_string_lossy(),
                "version": evidence.version(),
                "launcher_sha256": evidence.launcher_sha256(),
                "schema_bundle_sha256": evidence.schema_bundle_sha256(),
                "schema_file_count": evidence.schema_file_count()
            }))
        }
        RuntimeCommand::DeliveryRun {
            launcher,
            version,
            sha256,
            schema_dir,
            codex_home,
            delivery_root,
            git_exe,
            timeout_seconds,
            database,
        } => execute_delivery(DeliveryRunInput {
            launcher,
            version,
            sha256,
            schema_dir,
            codex_home,
            delivery_root,
            git_exe,
            timeout_seconds,
            database,
        }),
        RuntimeCommand::DeliveryStatus { database } => execute_delivery_status(database),
        RuntimeCommand::RuntimeHealth { database } => execute_runtime_health(&database),
        RuntimeCommand::ReceiptState { database } => execute_receipt_state(&database),
    }
}

struct DeliveryRunInput {
    launcher: PathBuf,
    version: String,
    sha256: String,
    schema_dir: PathBuf,
    codex_home: PathBuf,
    delivery_root: PathBuf,
    git_exe: PathBuf,
    timeout_seconds: u64,
    database: DeliveryDatabaseBinding,
}

#[allow(clippy::too_many_lines)]
fn execute_delivery(input: DeliveryRunInput) -> Result<Value, RuntimeError> {
    let timeout = Duration::from_secs(input.timeout_seconds);
    let runtime = delivery_runtime_environment()?;
    validate_delivery_command_runtime(runtime)?;
    let config = LatticedDeliveryConfig::new(
        input.launcher,
        input.version,
        input.sha256,
        input.schema_dir,
        input.codex_home,
        input.delivery_root,
        input.git_exe,
        timeout,
        runtime,
    )
    .map_err(|error| RuntimeError::Latticed(error.kind()))?;
    let password = delivery_database_password()?;
    let mut service = LatticedDeliveryService::for_delivery(config, input.database, password)
        .map_err(|error| RuntimeError::Latticed(error.kind()))?;
    service
        .run_scripted_acceptance_json()
        .map_err(|error| RuntimeError::Latticed(error.kind()))
}

fn execute_delivery_status(database: DeliveryDatabaseBinding) -> Result<Value, RuntimeError> {
    let password = delivery_database_password()?;
    let mut service =
        LatticedDeliveryService::status_only(database, password, Duration::from_secs(30))
            .map_err(|error| RuntimeError::Latticed(error.kind()))?;
    service
        .status_json()
        .map_err(|error| RuntimeError::Latticed(error.kind()))
}

fn execute_runtime_health(database: &DeliveryDatabaseBinding) -> Result<Value, RuntimeError> {
    let password = delivery_database_password()?;
    let _client =
        connect_fixed_runtime_client(database, &password, Instant::now() + Duration::from_secs(5))
            .map_err(|_| RuntimeError::Latticed(LatticedErrorKind::DatabaseConnect))?;

    Ok(runtime_health_projection(
        std::env::var("LATTICE_RUNTIME_INTEGRATION").ok().as_deref(),
    )?)
}

fn runtime_health_projection(integration_mode: Option<&str>) -> Result<Value, RuntimeError> {
    let (mode, graphify_status, hermes_status) = match integration_mode {
        None | Some("CORE_ONLY") => ("CORE_ONLY", "DEFERRED", "DEFERRED"),
        Some("GRAPHIFY") => ("GRAPHIFY", "NOT_INSPECTED", "DEFERRED"),
        Some("GRAPHIFY_HERMES") | Some("FULL_CHAIN") => {
            ("GRAPHIFY_HERMES", "NOT_INSPECTED", "NOT_INSPECTED")
        }
        Some(_) => return Err(RuntimeError::Latticed(LatticedErrorKind::Configuration)),
    };

    Ok(json!({
        "runtime": "LATTICE",
        "mode": mode,
        "components": {
            "control": {"status": "READY", "role": "coordination"},
            "postgresql": {"status": "CONNECTABLE", "role": "durable-truth"},
            "delivery_receipt": {"status": "NOT_INSPECTED", "role": "read-separately"},
            "graphify": {"status": graphify_status, "role": "derived-memory"},
            "hermes": {"status": hermes_status, "role": "reflection"}
        }
    }))
}

fn execute_receipt_state(database: &DeliveryDatabaseBinding) -> Result<Value, RuntimeError> {
    let password = delivery_database_password()?;
    let mut ledger =
        DeliveryLedger::connect(database, &password, Instant::now() + Duration::from_secs(5))
            .map_err(|error| RuntimeError::DeliveryLedger(error.kind()))?;
    let status = ledger
        .status()
        .map_err(|error| RuntimeError::DeliveryLedger(error.kind()))?;

    Ok(json!({
        "component": "delivery-receipt",
        "status": status.as_str(),
        "scope": "receipt-only"
    }))
}

fn delivery_database_password() -> Result<String, RuntimeError> {
    std::env::var("LATTICE_TASK019_PASSWORD")
        .ok()
        .filter(|value| !value.is_empty())
        .ok_or(RuntimeError::MissingDatabaseSecret)
}

fn delivery_runtime_environment() -> Result<DeliveryRuntime, RuntimeError> {
    match std::env::var("LATTICE_DELIVERY_CODEX_MODE").as_deref() {
        Ok("SCRIPTED_ACCEPTANCE") => Ok(DeliveryRuntime::ScriptedAcceptance),
        Ok("OFFICIAL_CODEX_APP_SERVER") => Ok(DeliveryRuntime::OfficialCodexAppServer),
        _ => Err(RuntimeError::Latticed(LatticedErrorKind::Configuration)),
    }
}

fn validate_delivery_command_runtime(runtime: DeliveryRuntime) -> Result<(), RuntimeError> {
    if runtime == DeliveryRuntime::ScriptedAcceptance {
        Ok(())
    } else {
        Err(RuntimeError::Latticed(
            LatticedErrorKind::OfficialLiveBlocked,
        ))
    }
}

fn parse_options(options: &[String], names: &[&str]) -> Result<Vec<String>, RuntimeError> {
    if options.len() != names.len() * 2 {
        return Err(RuntimeError::Usage);
    }
    let mut values = vec![None; names.len()];
    for pair in options.chunks_exact(2) {
        let Some(index) = names.iter().position(|name| *name == pair[0]) else {
            return Err(RuntimeError::Usage);
        };
        if pair[1].is_empty() || values[index].replace(pair[1].clone()).is_some() {
            return Err(RuntimeError::Usage);
        }
    }
    values
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .ok_or(RuntimeError::Usage)
}

fn parse_database_binding(
    host: &str,
    port: &str,
    run_id: &str,
) -> Result<DeliveryDatabaseBinding, RuntimeError> {
    let port = port.parse::<u16>().map_err(|_| RuntimeError::Usage)?;
    DeliveryDatabaseBinding::new(host, port, run_id)
        .map_err(|error| RuntimeError::DeliveryLedger(error.kind()))
}

fn is_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::{
        DELIVERY_PROMPT, RuntimeError, runtime_health_projection, validate_delivery_command_runtime,
    };
    use lattice_codex_adapter::{AppServerSession, SessionRequest, TurnStatus};
    use lattice_contracts::DeliveryRuntime;
    use sha2::{Digest, Sha256};

    fn sha256_hex(bytes: &[u8]) -> String {
        use std::fmt::Write as _;

        let mut output = String::with_capacity(64);
        for byte in Sha256::digest(bytes) {
            write!(&mut output, "{byte:02x}").expect("write digest");
        }
        output
    }

    #[test]
    fn compatibility_command_is_scripted_only_and_rejects_official_writer_use() {
        assert!(validate_delivery_command_runtime(DeliveryRuntime::ScriptedAcceptance).is_ok());
        assert_eq!(
            validate_delivery_command_runtime(DeliveryRuntime::OfficialCodexAppServer),
            Err(RuntimeError::Latticed(
                crate::composition::LatticedErrorKind::OfficialLiveBlocked
            ))
        );
    }

    #[test]
    fn core_only_health_reports_optional_modules_as_deferred_without_activating_them() {
        let health = runtime_health_projection(Some("CORE_ONLY")).expect("core-only health");

        assert_eq!(health["runtime"], "LATTICE");
        assert_eq!(health["mode"], "CORE_ONLY");
        assert_eq!(health["components"]["control"]["status"], "READY");
        assert_eq!(health["components"]["postgresql"]["status"], "CONNECTABLE");
        assert_eq!(
            health["components"]["delivery_receipt"]["status"],
            "NOT_INSPECTED"
        );
        assert_eq!(health["components"]["graphify"]["status"], "DEFERRED");
        assert_eq!(health["components"]["hermes"]["status"], "DEFERRED");
    }

    #[test]
    fn runtime_health_rejects_an_unknown_integration_mode() {
        assert_eq!(
            runtime_health_projection(Some("ALL_AT_ONCE")),
            Err(RuntimeError::Latticed(
                crate::composition::LatticedErrorKind::Configuration
            ))
        );
    }

    #[test]
    fn graphify_health_does_not_claim_hermes_is_active() {
        let health = runtime_health_projection(Some("GRAPHIFY")).expect("graphify health");
        assert_eq!(health["mode"], "GRAPHIFY");
        assert_eq!(health["components"]["graphify"]["status"], "NOT_INSPECTED");
        assert_eq!(health["components"]["hermes"]["status"], "DEFERRED");
    }

    #[test]
    fn fixed_delivery_prompt_uses_sandboxed_nested_shell_edit_protocol() {
        assert!(DELIVERY_PROMPT.contains("nested tools.shell_command"));
        assert!(DELIVERY_PROMPT.contains("[System.IO.File]::WriteAllBytes"));
        assert!(DELIVERY_PROMPT.contains(
            "[System.IO.File]::WriteAllBytes('answer.txt',[byte[]](76,65,84,84,73,67,69,95,68,69,76,73,86,69,82,89,95,79,75,10))"
        ));
        assert!(!DELIVERY_PROMPT.contains("[byte[]]@(0x4C"));
        assert!(!DELIVERY_PROMPT.contains("FromBase64String"));
        assert!(!DELIVERY_PROMPT.contains("Base64-encoded"));
        assert!(!DELIVERY_PROMPT.contains("git apply"));
        assert!(DELIVERY_PROMPT.contains("Do not call tools.apply_patch"));
        assert!(DELIVERY_PROMPT.contains("Do not set sandbox_permissions"));
        assert!(DELIVERY_PROMPT.contains("20 bytes ending in LF"));
        assert!(DELIVERY_PROMPT.contains("typeof result !== \"string\""));
        assert!(DELIVERY_PROMPT.contains("/^Exit code: 0(?:\\r?\\n|$)/"));
        assert!(DELIVERY_PROMPT.contains("throw new Error"));
        assert!(DELIVERY_PROMPT.contains("separate verification"));
        assert!(DELIVERY_PROMPT.contains(
            "$bytes=[System.IO.File]::ReadAllBytes('answer.txt'); $expected=[byte[]](76,65,84,84,73,67,69,95,68,69,76,73,86,69,82,89,95,79,75,10); if ($bytes.Length -ne $expected.Length -or (Compare-Object -ReferenceObject $expected -DifferenceObject $bytes -SyncWindow 0)) { exit 1 }"
        ));
        assert!(DELIVERY_PROMPT.contains("functions.wait"));
        assert!(DELIVERY_PROMPT.contains("Script completed"));
        assert!(DELIVERY_PROMPT.contains("Do not combine"));
        assert!(DELIVERY_PROMPT.contains("LATTICE performs scope inspection"));
    }

    #[test]
    fn scripted_fixture_tracks_prompt_and_completed_tool_evidence() {
        let fixture = include_str!("fixtures/task032-scripted-codex.ps1");
        let prompt_sha256 = sha256_hex(DELIVERY_PROMPT.as_bytes());
        assert!(
            fixture.contains(&format!("$expectedPromptSha256 = '{prompt_sha256}'")),
            "scripted fixture must pin prompt digest {prompt_sha256}"
        );
        let notifications = fixture
            .lines()
            .filter_map(|line| line.strip_prefix("[Console]::Out.WriteLine('"))
            .filter_map(|line| line.strip_suffix("')"))
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .collect::<Vec<_>>();
        let completed_items = notifications
            .iter()
            .filter(|notification| {
                notification
                    .get("method")
                    .and_then(serde_json::Value::as_str)
                    == Some("item/completed")
            })
            .map(|notification| notification["params"]["item"].clone())
            .collect::<Vec<_>>();
        assert_eq!(completed_items.len(), 2);
        assert_eq!(
            completed_items[0]["arguments"]["command"],
            "code-mode nested tools.shell_command write fixture"
        );
        assert_eq!(
            completed_items[1]["arguments"]["command"],
            "code-mode nested tools.shell_command verify fixture"
        );
        assert!(completed_items.iter().all(|item| {
            item.get("success").and_then(serde_json::Value::as_bool) == Some(true)
        }));
        let terminal = notifications
            .iter()
            .find(|notification| {
                notification
                    .get("method")
                    .and_then(serde_json::Value::as_str)
                    == Some("turn/completed")
            })
            .expect("scripted terminal JSON line");
        assert_eq!(terminal["params"]["turn"]["itemsView"], "summary");

        let mut session = AppServerSession::new();
        for request in [
            SessionRequest::Initialize,
            SessionRequest::ThreadStart,
            SessionRequest::TurnStart,
        ] {
            session
                .mark_request_sent(request)
                .expect("fixture lifecycle request is sent once");
        }
        session
            .ingest(serde_json::json!({
                "id": 0,
                "result": {
                    "userAgent": "codex_cli_rs/0.144.6",
                    "platformFamily": "windows",
                    "platformOs": "windows",
                    "codexHome": r"C:\lattice\codex-home"
                }
            }))
            .expect("fixture initialize response is valid");
        session
            .ingest(serde_json::json!({
                "id": 1,
                "result": {"thread": {"id": "thread-task032-scripted"}}
            }))
            .expect("fixture thread response is valid");
        session
            .ingest(serde_json::json!({
                "id": 2,
                "result": {"turn": {"id": "turn-task032-scripted"}}
            }))
            .expect("fixture turn response is valid");
        let mut outcome = None;
        for notification in notifications.iter().filter(|notification| {
            matches!(
                notification
                    .get("method")
                    .and_then(serde_json::Value::as_str),
                Some("item/completed" | "turn/completed")
            )
        }) {
            if let Some(completed) = session
                .ingest(notification.clone())
                .expect("fixture notification satisfies the production session")
            {
                outcome = Some(completed);
            }
        }
        assert_eq!(
            outcome
                .expect("scripted terminal completes the session")
                .status,
            TurnStatus::Completed
        );
    }

    #[test]
    fn official_delivery_script_keeps_the_windows_sandbox_unelevated() {
        let script = include_str!("../../../scripts/run-lattice-delivery.ps1");
        assert!(script.contains("'sandbox = \"unelevated\"'"));
        assert!(script.contains("windows_sandbox = 'unelevated'"));
        assert!(script.contains("safety = 'workspace-write;unelevated;stdio-only'"));
        assert!(!script.contains("sandbox = \"elevated\""));
        assert!(!script.contains("workspace-write;elevated;stdio-only"));
    }
}
