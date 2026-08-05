//! LATTICE runtime composition entry.

pub mod composition;
pub mod delivery_ledger;
pub mod git_delivery;
pub mod mcp;

use std::error::Error;
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use lattice_codex_adapter::{
    CodexIdentityErrorKind, CodexIdentityExpectation, preflight_codex_identity,
};
use lattice_contracts::DeliveryRuntime;
use serde_json::{Value, json};

use crate::composition::{LatticedDeliveryConfig, LatticedDeliveryService, LatticedErrorKind};
use crate::delivery_ledger::{DeliveryDatabaseBinding, DeliveryLedgerErrorKind};
use crate::git_delivery::GitDeliveryErrorKind;

const USAGE: &str = "usage:\n  lattice-runtime codex-preflight --launcher <absolute-path> --version <exact-version> --sha256 <lowercase-sha256> --schema-dir <absent-path>\n  lattice-runtime delivery-run --launcher <absolute-path> --version <exact-version> --sha256 <lowercase-sha256> --schema-dir <absent-path> --codex-home <absolute-path> --delivery-root <absent-absolute-path> --git-exe <absolute-path> --timeout-seconds <1..3600> --postgres-host 127.0.0.1 --postgres-port <ephemeral-port> --postgres-run-id <32-lowercase-hex>\n  lattice-runtime delivery-status --postgres-host 127.0.0.1 --postgres-port <ephemeral-port> --postgres-run-id <32-lowercase-hex>";

const DELIVERY_PROMPT: &str = "Create answer.txt in the current repository with exactly the bytes LATTICE_DELIVERY_OK followed by one newline. Use one standalone apply_patch operation in an exec call that performs no verification or other tool work. Confirm that call has completed, then use a separate verification call to read and validate the exact bytes. Do not combine file creation and verification in the same exec call. If any exec result says Script running with cell ID, call functions.wait with that exact cell_id until Script completed is received, and require exit code 0 before reporting success. Never terminate a yielded cell or claim completion from a running marker. Do not modify any other path. Do not stage or commit files and do not run Git commands.";

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
        .run_json()
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
    use super::DELIVERY_PROMPT;

    #[test]
    fn fixed_delivery_prompt_requires_completed_separate_tool_evidence() {
        assert!(DELIVERY_PROMPT.contains("standalone apply_patch"));
        assert!(DELIVERY_PROMPT.contains("separate verification"));
        assert!(DELIVERY_PROMPT.contains("functions.wait"));
        assert!(DELIVERY_PROMPT.contains("Script completed"));
        assert!(DELIVERY_PROMPT.contains("Do not combine"));
    }
}
