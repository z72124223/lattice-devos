//! LATTICE runtime composition entry.

use std::error::Error;
use std::fmt;
use std::path::PathBuf;

use lattice_codex_adapter::{
    CodexIdentityErrorKind, CodexIdentityExpectation, preflight_codex_identity,
};
use serde_json::{Value, json};

const USAGE: &str = "usage: lattice-runtime codex-preflight --launcher <absolute-path> --version <exact-version> --sha256 <lowercase-sha256> --schema-dir <absent-path>";

/// Closed command surface for the first delivery node.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeCommand {
    CodexPreflight {
        launcher: PathBuf,
        version: String,
        sha256: String,
        schema_dir: PathBuf,
    },
}

/// Stable command-line failures without sensitive process output.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeError {
    Usage,
    InvalidDigest,
    CodexIdentity(CodexIdentityErrorKind),
}

impl RuntimeError {
    /// Returns the stable diagnostic code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Usage => "LATTICE_RUNTIME_USAGE",
            Self::InvalidDigest => "LATTICE_RUNTIME_INVALID_DIGEST",
            Self::CodexIdentity(_) => "LATTICE_RUNTIME_CODEX_IDENTITY_REJECTED",
        }
    }
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage => formatter.write_str(USAGE),
            Self::InvalidDigest => formatter.write_str("expected one lowercase SHA-256 digest"),
            Self::CodexIdentity(kind) => write!(formatter, "Codex identity rejected: {kind:?}"),
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
    if command != "codex-preflight" || options.len() != 8 {
        return Err(RuntimeError::Usage);
    }

    let mut launcher = None;
    let mut version = None;
    let mut sha256 = None;
    let mut schema_dir = None;
    for pair in options.chunks_exact(2) {
        let target = match pair[0].as_str() {
            "--launcher" => &mut launcher,
            "--version" => &mut version,
            "--sha256" => &mut sha256,
            "--schema-dir" => &mut schema_dir,
            _ => return Err(RuntimeError::Usage),
        };
        if target.replace(pair[1].clone()).is_some() || pair[1].is_empty() {
            return Err(RuntimeError::Usage);
        }
    }

    let (Some(launcher), Some(version), Some(sha256), Some(schema_dir)) =
        (launcher, version, sha256, schema_dir)
    else {
        return Err(RuntimeError::Usage);
    };
    if !is_lowercase_sha256(&sha256) {
        return Err(RuntimeError::InvalidDigest);
    }

    Ok(RuntimeCommand::CodexPreflight {
        launcher: PathBuf::from(launcher),
        version,
        sha256,
        schema_dir: PathBuf::from(schema_dir),
    })
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
    }
}

fn is_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
