//! LATTICE runtime composition entry.

use std::error::Error;
use std::fmt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use lattice_codex_adapter::{
    AppServerRunConfig, CodexIdentityErrorKind, CodexIdentityExpectation, TurnStatus,
    preflight_codex_identity, run_codex_app_server,
};
use serde_json::{Value, json};

const USAGE: &str = "usage:\n  lattice-runtime codex-preflight --launcher <absolute-path> --version <exact-version> --sha256 <lowercase-sha256> --schema-dir <absent-path>\n  lattice-runtime codex-turn --launcher <absolute-path> --version <exact-version> --sha256 <lowercase-sha256> --schema-dir <absent-path> --codex-home <absolute-path> --cwd <absolute-path> --prompt <text> --timeout-seconds <1..3600>";

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
        _ => Err(RuntimeError::Usage),
    }
}

/// Executes one parsed local runtime command.
///
/// # Errors
///
/// Returns a typed identity rejection when the pinned Codex launcher or its
/// generated schema does not match the supplied expectation.
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
    }
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
