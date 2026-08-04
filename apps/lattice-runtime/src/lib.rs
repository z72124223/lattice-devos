//! LATTICE runtime composition entry.

pub mod delivery_ledger;
pub mod git_delivery;

use std::error::Error;
use std::fmt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use lattice_codex_adapter::{
    AppServerRunConfig, CodexIdentityErrorKind, CodexIdentityExpectation, TurnStatus,
    preflight_codex_identity, run_codex_app_server,
};
use serde_json::{Value, json};

use crate::delivery_ledger::{
    DeliveryDatabaseBinding, DeliveryIntentEvidence, DeliveryLedger, DeliveryLedgerErrorKind,
    DeliveryStatus, DeliverySuccessEvidence,
};
use crate::git_delivery::{GitDeliveryErrorKind, IsolatedGitDelivery};

const USAGE: &str = "usage:\n  lattice-runtime codex-preflight --launcher <absolute-path> --version <exact-version> --sha256 <lowercase-sha256> --schema-dir <absent-path>\n  lattice-runtime codex-turn --launcher <absolute-path> --version <exact-version> --sha256 <lowercase-sha256> --schema-dir <absent-path> --codex-home <absolute-path> --cwd <absolute-path> --prompt <text> --timeout-seconds <1..3600>\n  lattice-runtime delivery-run --launcher <absolute-path> --version <exact-version> --sha256 <lowercase-sha256> --schema-dir <absent-path> --codex-home <absolute-path> --delivery-root <absent-absolute-path> --git-exe <absolute-path> --timeout-seconds <1..3600> --postgres-host 127.0.0.1 --postgres-port <ephemeral-port> --postgres-run-id <32-lowercase-hex>\n  lattice-runtime delivery-status --postgres-host 127.0.0.1 --postgres-port <ephemeral-port> --postgres-run-id <32-lowercase-hex>";

const DELIVERY_PROMPT: &str = "Create answer.txt in the current repository with exactly the bytes LATTICE_DELIVERY_OK followed by one newline. Do not modify any other path. Do not stage or commit files and do not run Git commands.";

/// Closed command surface for the first delivery node.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeCommand {
    CodexPreflight {
        launcher: PathBuf,
        version: String,
        sha256: String,
        schema_dir: PathBuf,
    },
    CodexTurn {
        launcher: PathBuf,
        version: String,
        sha256: String,
        schema_dir: PathBuf,
        codex_home: PathBuf,
        working_directory: PathBuf,
        prompt: String,
        timeout_seconds: u64,
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
    DeadlineExpired,
    CodexIdentity(CodexIdentityErrorKind),
    CodexRun(lattice_codex_adapter::AppServerRunErrorKind),
    CodexTerminalFailed,
    CodexTerminalInterrupted,
    MissingDatabaseSecret,
    DeliveryLedger(DeliveryLedgerErrorKind),
    DeliveryLedgerOperation(DeliveryLedgerStage, DeliveryLedgerErrorKind),
    GitDelivery(GitDeliveryErrorKind),
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
            Self::DeadlineExpired => "LATTICE_RUNTIME_TIMEOUT",
            Self::CodexIdentity(_) => "LATTICE_RUNTIME_CODEX_IDENTITY_REJECTED",
            Self::CodexRun(_) => "LATTICE_RUNTIME_CODEX_RUN_FAILED",
            Self::CodexTerminalFailed => "LATTICE_RUNTIME_CODEX_TERMINAL_FAILED",
            Self::CodexTerminalInterrupted => "LATTICE_RUNTIME_CODEX_TERMINAL_INTERRUPTED",
            Self::MissingDatabaseSecret => "LATTICE_RUNTIME_DATABASE_SECRET_MISSING",
            Self::DeliveryLedger(_) => "LATTICE_RUNTIME_DELIVERY_LEDGER_REJECTED",
            Self::DeliveryLedgerOperation(stage, _) => match stage {
                DeliveryLedgerStage::Connect => "LATTICE_RUNTIME_DELIVERY_CONNECT_REJECTED",
                DeliveryLedgerStage::Intent => "LATTICE_RUNTIME_DELIVERY_INTENT_REJECTED",
                DeliveryLedgerStage::Outcome => "LATTICE_RUNTIME_DELIVERY_OUTCOME_REJECTED",
                DeliveryLedgerStage::Receipt => "LATTICE_RUNTIME_DELIVERY_RECEIPT_REJECTED",
            },
            Self::GitDelivery(_) => "LATTICE_RUNTIME_GIT_DELIVERY_REJECTED",
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
            Self::DeadlineExpired => formatter.write_str("Codex operation deadline expired"),
            Self::CodexIdentity(kind) => write!(formatter, "Codex identity rejected: {kind:?}"),
            Self::CodexRun(kind) => write!(formatter, "Codex run failed: {kind:?}"),
            Self::CodexTerminalFailed => formatter.write_str("Codex turn reported failed"),
            Self::CodexTerminalInterrupted => {
                formatter.write_str("Codex turn reported interrupted")
            }
            Self::MissingDatabaseSecret => {
                formatter.write_str("required PostgreSQL password environment is missing")
            }
            Self::DeliveryLedger(kind) => write!(formatter, "delivery ledger rejected: {kind:?}"),
            Self::DeliveryLedgerOperation(stage, kind) => {
                write!(formatter, "delivery ledger {stage:?} rejected: {kind:?}")
            }
            Self::GitDelivery(kind) => write!(formatter, "Git delivery rejected: {kind:?}"),
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
        "codex-turn" => {
            let values = parse_options(
                options,
                &[
                    "--launcher",
                    "--version",
                    "--sha256",
                    "--schema-dir",
                    "--codex-home",
                    "--cwd",
                    "--prompt",
                    "--timeout-seconds",
                ],
            )?;
            let sha256 = values[2].clone();
            if !is_lowercase_sha256(&sha256) {
                return Err(RuntimeError::InvalidDigest);
            }
            Ok(RuntimeCommand::CodexTurn {
                launcher: PathBuf::from(&values[0]),
                version: values[1].clone(),
                sha256,
                schema_dir: PathBuf::from(&values[3]),
                codex_home: PathBuf::from(&values[4]),
                working_directory: PathBuf::from(&values[5]),
                prompt: values[6].clone(),
                timeout_seconds: values[7]
                    .parse::<u64>()
                    .ok()
                    .filter(|value| (1..=3600).contains(value))
                    .ok_or(RuntimeError::InvalidTimeout)?,
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
        RuntimeCommand::CodexTurn {
            launcher,
            version,
            sha256,
            schema_dir,
            codex_home,
            working_directory,
            prompt,
            timeout_seconds,
        } => {
            let timeout = Duration::from_secs(timeout_seconds);
            let deadline = Instant::now()
                .checked_add(timeout)
                .ok_or(RuntimeError::InvalidTimeout)?;
            let expectation =
                CodexIdentityExpectation::new(launcher.clone(), version, sha256.clone());

            // Reject malformed task bindings before executing either identity
            // command. The second construction binds the turn to the time
            // left after identity verification, so one deadline covers both.
            AppServerRunConfig::new(
                launcher.clone(),
                sha256.clone(),
                codex_home.clone(),
                working_directory.clone(),
                prompt.clone(),
                timeout,
            )
            .map_err(|error| RuntimeError::CodexRun(error.kind()))?;
            let identity = expectation
                .preflight_with_deadline(&launcher, &schema_dir, deadline)
                .map_err(|error| RuntimeError::CodexIdentity(error.kind()))?;
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .filter(|remaining| !remaining.is_zero())
                .ok_or(RuntimeError::DeadlineExpired)?;
            let config = AppServerRunConfig::new(
                launcher,
                identity.launcher_sha256(),
                codex_home,
                working_directory,
                prompt,
                remaining,
            )
            .map_err(|error| RuntimeError::CodexRun(error.kind()))?;
            let run = run_codex_app_server(&config)
                .map_err(|error| RuntimeError::CodexRun(error.kind()))?;
            let status = require_completed(run.outcome().status)?;
            Ok(json!({
                "status": status,
                "component": "codex",
                "launcher_path": identity.launcher_path().to_string_lossy(),
                "version": identity.version(),
                "launcher_sha256": identity.launcher_sha256(),
                "schema_bundle_sha256": identity.schema_bundle_sha256(),
                "schema_file_count": identity.schema_file_count(),
                "codex_home": run.initialize().codex_home.to_string_lossy(),
                "thread_id": run.thread_id(),
                "turn_id": run.turn_id(),
                "error_message": run.outcome().error_message
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
        RuntimeCommand::DeliveryStatus { database } => {
            let password = std::env::var("LATTICE_TASK019_PASSWORD")
                .ok()
                .filter(|value| !value.is_empty())
                .ok_or(RuntimeError::MissingDatabaseSecret)?;
            let deadline = Instant::now()
                .checked_add(Duration::from_secs(30))
                .ok_or(RuntimeError::InvalidTimeout)?;
            let mut ledger =
                DeliveryLedger::connect(&database, &password, deadline).map_err(|error| {
                    RuntimeError::DeliveryLedgerOperation(
                        DeliveryLedgerStage::Connect,
                        error.kind(),
                    )
                })?;
            let receipt = ledger.receipt().map_err(|error| {
                RuntimeError::DeliveryLedgerOperation(DeliveryLedgerStage::Receipt, error.kind())
            })?;
            Ok(json!({
                "status": "COMPLETED",
                "component": "delivery-ledger",
                "launcher_path": receipt.launcher_path(),
                "version": receipt.version(),
                "launcher_sha256": receipt.launcher_sha256(),
                "schema_bundle_sha256": receipt.schema_bundle_sha256(),
                "schema_file_count": receipt.schema_file_count(),
                "repository_path": receipt.repository_path(),
                "changed_paths": ["answer.txt"],
                "test": "FIXED_TEST_PASSED",
                "test_command_id": "git-diff-no-index-exact-answer-v1",
                "baseline_commit": receipt.parent_sha(),
                "parent_sha": receipt.parent_sha(),
                "commit_sha": receipt.commit_sha(),
                "thread_id": receipt.thread_id(),
                "turn_id": receipt.turn_id(),
                "intent_digest": receipt.intent_digest(),
                "outcome_digest": receipt.outcome_digest()
            }))
        }
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
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(input.timeout_seconds))
        .ok_or(RuntimeError::InvalidTimeout)?;
    let password = std::env::var("LATTICE_TASK019_PASSWORD")
        .ok()
        .filter(|value| !value.is_empty())
        .ok_or(RuntimeError::MissingDatabaseSecret)?;
    let expected_repo = input.delivery_root.join("repo");
    let intent_evidence = DeliveryIntentEvidence::new(
        input.launcher.to_string_lossy(),
        input.version.clone(),
        input.sha256.clone(),
        input.schema_dir.to_string_lossy(),
        input.codex_home.to_string_lossy(),
        expected_repo.to_string_lossy(),
    )
    .map_err(|error| {
        RuntimeError::DeliveryLedgerOperation(DeliveryLedgerStage::Intent, error.kind())
    })?;
    let mut ledger =
        DeliveryLedger::connect(&input.database, &password, deadline).map_err(|error| {
            RuntimeError::DeliveryLedgerOperation(DeliveryLedgerStage::Connect, error.kind())
        })?;
    let intent_digest = ledger.record_intent(&intent_evidence).map_err(|error| {
        RuntimeError::DeliveryLedgerOperation(DeliveryLedgerStage::Intent, error.kind())
    })?;
    let intent_status = ledger.status().map_err(|error| {
        RuntimeError::DeliveryLedgerOperation(DeliveryLedgerStage::Intent, error.kind())
    })?;
    if intent_status != DeliveryStatus::ReconciliationRequired {
        return Err(RuntimeError::DeliveryLedgerOperation(
            DeliveryLedgerStage::Intent,
            DeliveryLedgerErrorKind::ReconciliationRequired,
        ));
    }

    // The durable intent above is the effect boundary: only now may Codex run
    // its version/schema preflight or later mutate the isolated repository.
    let expectation =
        CodexIdentityExpectation::new(input.launcher.clone(), input.version, input.sha256.clone());
    let identity = expectation
        .preflight_with_deadline(&input.launcher, &input.schema_dir, deadline)
        .map_err(|error| RuntimeError::CodexIdentity(error.kind()))?;

    let git = IsolatedGitDelivery::provision_until(&input.delivery_root, &input.git_exe, deadline)
        .map_err(|error| RuntimeError::GitDelivery(error.kind()))?;
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or(RuntimeError::DeadlineExpired)?;
    let app_server = AppServerRunConfig::new(
        input.launcher,
        identity.launcher_sha256(),
        input.codex_home,
        git.repo_path().to_path_buf(),
        DELIVERY_PROMPT,
        remaining,
    )
    .map_err(|error| RuntimeError::CodexRun(error.kind()))?;
    let run =
        run_codex_app_server(&app_server).map_err(|error| RuntimeError::CodexRun(error.kind()))?;
    require_completed(run.outcome().status)?;

    let git_evidence = git
        .verify_and_commit()
        .map_err(|error| RuntimeError::GitDelivery(error.kind()))?;
    let success = DeliverySuccessEvidence::new(
        intent_digest,
        identity.launcher_path().to_string_lossy(),
        identity.version(),
        identity.launcher_sha256(),
        identity.schema_bundle_sha256(),
        identity.schema_file_count(),
        run.thread_id(),
        run.turn_id(),
        git_evidence.repository_path.to_string_lossy(),
        &git_evidence.commit_sha,
        &git_evidence.baseline_commit,
    )
    .map_err(|error| {
        RuntimeError::DeliveryLedgerOperation(DeliveryLedgerStage::Outcome, error.kind())
    })?;
    let outcome_digest = ledger.record_success(&success).map_err(|error| {
        RuntimeError::DeliveryLedgerOperation(DeliveryLedgerStage::Outcome, error.kind())
    })?;
    let receipt = ledger.receipt().map_err(|error| {
        RuntimeError::DeliveryLedgerOperation(DeliveryLedgerStage::Receipt, error.kind())
    })?;
    if receipt.outcome_digest() != outcome_digest.as_str() {
        return Err(RuntimeError::DeliveryLedgerOperation(
            DeliveryLedgerStage::Receipt,
            DeliveryLedgerErrorKind::ReconciliationRequired,
        ));
    }

    Ok(json!({
        "status": "COMPLETED",
        "component": "lattice-delivery",
        "launcher_path": receipt.launcher_path(),
        "version": receipt.version(),
        "launcher_sha256": receipt.launcher_sha256(),
        "schema_bundle_sha256": receipt.schema_bundle_sha256(),
        "schema_file_count": receipt.schema_file_count(),
        "repository_path": receipt.repository_path(),
        "changed_paths": ["answer.txt"],
        "test": "FIXED_TEST_PASSED",
        "test_command_id": "git-diff-no-index-exact-answer-v1",
        "baseline_commit": receipt.parent_sha(),
        "parent_sha": receipt.parent_sha(),
        "commit_sha": receipt.commit_sha(),
        "thread_id": receipt.thread_id(),
        "turn_id": receipt.turn_id(),
        "intent_digest": receipt.intent_digest(),
        "outcome_digest": receipt.outcome_digest()
    }))
}

fn require_completed(status: TurnStatus) -> Result<&'static str, RuntimeError> {
    match status {
        TurnStatus::Completed => Ok("COMPLETED"),
        TurnStatus::Failed => Err(RuntimeError::CodexTerminalFailed),
        TurnStatus::Interrupted => Err(RuntimeError::CodexTerminalInterrupted),
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
    use super::*;

    #[test]
    fn only_a_completed_codex_terminal_is_cli_success() {
        assert_eq!(require_completed(TurnStatus::Completed), Ok("COMPLETED"));
        assert_eq!(
            require_completed(TurnStatus::Failed),
            Err(RuntimeError::CodexTerminalFailed)
        );
        assert_eq!(
            require_completed(TurnStatus::Interrupted),
            Err(RuntimeError::CodexTerminalInterrupted)
        );
    }
}
