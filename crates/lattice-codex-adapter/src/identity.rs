use std::error::Error;
use std::fmt::{self, Write as _};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::process::{
    OwnedProcessTree, OwnedSandboxTemp, PinnedCodexResources, cleanup_sandbox_temp,
    configure_pinned_child_environment, stop_owned_child,
    terminate_uncontained_process_tree_bounded, validate_pinned_resources_for_launcher,
};

const SCHEMA_BUNDLE_DOMAIN: &[u8] = b"lattice.codex-app-server.schema-bundle.v1\0";
const DEFAULT_IDENTITY_PREFLIGHT_TIMEOUT: Duration = Duration::from_mins(2);
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(10);
const MAX_VERSION_OUTPUT_BYTES: u64 = 4 * 1024;

/// Pinned identity that a configured Codex launcher must match exactly.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexIdentityExpectation {
    launcher_path: PathBuf,
    version: String,
    launcher_sha256: String,
}

impl CodexIdentityExpectation {
    /// Creates one exact path, version, and launcher-byte expectation.
    #[must_use]
    pub fn new(
        launcher_path: impl Into<PathBuf>,
        version: impl Into<String>,
        launcher_sha256: impl Into<String>,
    ) -> Self {
        Self {
            launcher_path: launcher_path.into(),
            version: version.into(),
            launcher_sha256: launcher_sha256.into(),
        }
    }

    /// Returns the exact launcher path that may be executed.
    #[must_use]
    pub fn launcher_path(&self) -> &Path {
        &self.launcher_path
    }

    /// Returns the exact expected `--version` line without its line ending.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Returns the expected lowercase SHA-256 of the launcher bytes.
    #[must_use]
    pub fn launcher_sha256(&self) -> &str {
        &self.launcher_sha256
    }

    /// Runs one identity preflight under a caller-owned absolute deadline.
    ///
    /// The same deadline bounds both the version command and schema
    /// generation. A timed-out owned launcher process is terminated before
    /// this method returns.
    ///
    /// # Errors
    ///
    /// Returns [`CodexIdentityErrorKind::Timeout`] if either command cannot
    /// finish before `deadline`, or another typed identity rejection.
    pub fn preflight_with_deadline(
        &self,
        configured_launcher: &Path,
        schema_output_dir: &Path,
        deadline: Instant,
    ) -> Result<CodexIdentityEvidence, CodexIdentityError> {
        preflight_codex_identity_until(configured_launcher, self, schema_output_dir, deadline)
    }
}

/// Exact executable and generated-schema identity observed by one preflight.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexIdentityEvidence {
    launcher_path: PathBuf,
    version: String,
    launcher_sha256: String,
    schema_bundle_sha256: String,
    schema_file_count: usize,
}

impl CodexIdentityEvidence {
    /// Returns the launcher path used for both identity commands.
    #[must_use]
    pub fn launcher_path(&self) -> &Path {
        &self.launcher_path
    }

    /// Returns the exact observed version line.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Returns the lowercase SHA-256 of the launcher bytes.
    #[must_use]
    pub fn launcher_sha256(&self) -> &str {
        &self.launcher_sha256
    }

    /// Returns the deterministic digest of every generated JSON schema file.
    #[must_use]
    pub fn schema_bundle_sha256(&self) -> &str {
        &self.schema_bundle_sha256
    }

    /// Returns the number of generated JSON schema files bound by the digest.
    #[must_use]
    pub const fn schema_file_count(&self) -> usize {
        self.schema_file_count
    }
}

/// Stable fail-closed reason for a Codex identity preflight rejection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodexIdentityErrorKind {
    LauncherPathMismatch,
    LauncherNotFile,
    LauncherReadFailed,
    LauncherDigestMismatch,
    LauncherChanged,
    VersionCommandFailed,
    VersionOutputInvalid,
    VersionMismatch,
    SchemaOutputExists,
    SchemaGenerationFailed,
    SchemaBundleInvalid,
    SchemaBundleEmpty,
    SchemaReadFailed,
    Timeout,
    ProcessContainmentFailed,
    PinnedResourcesRejected,
    PinnedResourcesChanged,
}

/// Payload-free Codex identity preflight error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CodexIdentityError {
    kind: CodexIdentityErrorKind,
}

impl CodexIdentityError {
    pub(crate) const fn new(kind: CodexIdentityErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the stable failure class without exposing command output.
    #[must_use]
    pub const fn kind(self) -> CodexIdentityErrorKind {
        self.kind
    }
}

impl fmt::Display for CodexIdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            CodexIdentityErrorKind::LauncherPathMismatch => "CODEX_LAUNCHER_PATH_MISMATCH",
            CodexIdentityErrorKind::LauncherNotFile => "CODEX_LAUNCHER_NOT_FILE",
            CodexIdentityErrorKind::LauncherReadFailed => "CODEX_LAUNCHER_READ_FAILED",
            CodexIdentityErrorKind::LauncherDigestMismatch => "CODEX_LAUNCHER_DIGEST_MISMATCH",
            CodexIdentityErrorKind::LauncherChanged => "CODEX_LAUNCHER_CHANGED",
            CodexIdentityErrorKind::VersionCommandFailed => "CODEX_VERSION_COMMAND_FAILED",
            CodexIdentityErrorKind::VersionOutputInvalid => "CODEX_VERSION_OUTPUT_INVALID",
            CodexIdentityErrorKind::VersionMismatch => "CODEX_VERSION_MISMATCH",
            CodexIdentityErrorKind::SchemaOutputExists => "CODEX_SCHEMA_OUTPUT_EXISTS",
            CodexIdentityErrorKind::SchemaGenerationFailed => "CODEX_SCHEMA_GENERATION_FAILED",
            CodexIdentityErrorKind::SchemaBundleInvalid => "CODEX_SCHEMA_BUNDLE_INVALID",
            CodexIdentityErrorKind::SchemaBundleEmpty => "CODEX_SCHEMA_BUNDLE_EMPTY",
            CodexIdentityErrorKind::SchemaReadFailed => "CODEX_SCHEMA_READ_FAILED",
            CodexIdentityErrorKind::Timeout => "CODEX_IDENTITY_TIMEOUT",
            CodexIdentityErrorKind::ProcessContainmentFailed => {
                "CODEX_IDENTITY_PROCESS_CONTAINMENT_FAILED"
            }
            CodexIdentityErrorKind::PinnedResourcesRejected => {
                "CODEX_IDENTITY_PINNED_RESOURCES_REJECTED"
            }
            CodexIdentityErrorKind::PinnedResourcesChanged => {
                "CODEX_IDENTITY_PINNED_RESOURCES_CHANGED"
            }
        })
    }
}

impl Error for CodexIdentityError {}

/// Verifies one pinned Codex launcher and generates its version-specific schema bundle.
///
/// `schema_output_dir` is selected by the caller and must not exist. The exact
/// configured launcher is invoked first with `--version`, then with
/// `app-server generate-json-schema --out`. No model or app-server turn is run.
///
/// # Errors
///
/// Fails closed when the launcher path, file, bytes, version, command result,
/// generated directory, or generated JSON schema bundle is not exact.
pub fn preflight_codex_identity(
    configured_launcher: &Path,
    expectation: &CodexIdentityExpectation,
    schema_output_dir: &Path,
) -> Result<CodexIdentityEvidence, CodexIdentityError> {
    expectation.preflight_with_deadline(
        configured_launcher,
        schema_output_dir,
        Instant::now() + DEFAULT_IDENTITY_PREFLIGHT_TIMEOUT,
    )
}

fn preflight_codex_identity_until(
    configured_launcher: &Path,
    expectation: &CodexIdentityExpectation,
    schema_output_dir: &Path,
    deadline: Instant,
) -> Result<CodexIdentityEvidence, CodexIdentityError> {
    preflight_codex_identity_until_with_home(
        configured_launcher,
        expectation,
        schema_output_dir,
        None,
        None,
        deadline,
    )
}

pub(crate) fn preflight_codex_identity_in_home_until(
    configured_launcher: &Path,
    expectation: &CodexIdentityExpectation,
    schema_output_dir: &Path,
    codex_home: &Path,
    pinned_resources: Option<&PinnedCodexResources>,
    deadline: Instant,
) -> Result<CodexIdentityEvidence, CodexIdentityError> {
    preflight_codex_identity_until_with_home(
        configured_launcher,
        expectation,
        schema_output_dir,
        Some(codex_home),
        pinned_resources,
        deadline,
    )
}

fn preflight_codex_identity_until_with_home(
    configured_launcher: &Path,
    expectation: &CodexIdentityExpectation,
    schema_output_dir: &Path,
    codex_home: Option<&Path>,
    pinned_resources: Option<&PinnedCodexResources>,
    deadline: Instant,
) -> Result<CodexIdentityEvidence, CodexIdentityError> {
    if configured_launcher != expectation.launcher_path() {
        return Err(error(CodexIdentityErrorKind::LauncherPathMismatch));
    }

    let launcher_metadata = fs::symlink_metadata(configured_launcher)
        .map_err(|_| error(CodexIdentityErrorKind::LauncherNotFile))?;
    if !launcher_metadata.file_type().is_file() {
        return Err(error(CodexIdentityErrorKind::LauncherNotFile));
    }

    match fs::symlink_metadata(schema_output_dir) {
        Ok(_) => return Err(error(CodexIdentityErrorKind::SchemaOutputExists)),
        Err(read_error) if read_error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(error(CodexIdentityErrorKind::SchemaBundleInvalid)),
    }

    let launcher_sha256 = file_sha256(
        configured_launcher,
        CodexIdentityErrorKind::LauncherReadFailed,
    )?;
    if launcher_sha256 != expectation.launcher_sha256() {
        return Err(error(CodexIdentityErrorKind::LauncherDigestMismatch));
    }

    ensure_before_deadline(deadline)?;
    let (version_status, version_stdout) =
        run_version_command(configured_launcher, codex_home, pinned_resources, deadline)?;
    if !version_status.success() {
        return Err(error(CodexIdentityErrorKind::VersionCommandFailed));
    }
    let version = String::from_utf8(version_stdout)
        .map_err(|_| error(CodexIdentityErrorKind::VersionOutputInvalid))?;
    let version = version.trim_end_matches(['\r', '\n']).to_owned();
    if version != expectation.version() {
        return Err(error(CodexIdentityErrorKind::VersionMismatch));
    }

    ensure_before_deadline(deadline)?;
    let schema_status = run_schema_command(
        configured_launcher,
        schema_output_dir,
        codex_home,
        pinned_resources,
        deadline,
    )?;
    if !schema_status.success() {
        return Err(error(CodexIdentityErrorKind::SchemaGenerationFailed));
    }

    let final_launcher_sha256 = file_sha256(
        configured_launcher,
        CodexIdentityErrorKind::LauncherReadFailed,
    )?;
    if final_launcher_sha256 != launcher_sha256 {
        return Err(error(CodexIdentityErrorKind::LauncherChanged));
    }

    let (schema_bundle_sha256, schema_file_count) = schema_bundle_digest(schema_output_dir)?;
    Ok(CodexIdentityEvidence {
        launcher_path: configured_launcher.to_path_buf(),
        version,
        launcher_sha256,
        schema_bundle_sha256,
        schema_file_count,
    })
}

fn run_version_command(
    launcher: &Path,
    codex_home: Option<&Path>,
    pinned_resources: Option<&PinnedCodexResources>,
    deadline: Instant,
) -> Result<(ExitStatus, Vec<u8>), CodexIdentityError> {
    let (mut command, sandbox_temp) = identity_command(launcher, codex_home, pinned_resources)?;
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    validate_identity_resources_before_spawn(launcher, pinned_resources)?;
    let Ok(mut child) = command.spawn() else {
        cleanup_identity_sandbox_temp(sandbox_temp)?;
        return Err(error(CodexIdentityErrorKind::VersionCommandFailed));
    };
    let Ok(process_tree) = OwnedProcessTree::attach(&child) else {
        let _ = terminate_uncontained_process_tree_bounded(&mut child);
        cleanup_identity_sandbox_temp(sandbox_temp)?;
        return Err(error(CodexIdentityErrorKind::ProcessContainmentFailed));
    };
    let process_tree = match validate_identity_resources_after_attach(
        &mut child,
        process_tree,
        launcher,
        pinned_resources,
    ) {
        Ok(process_tree) => process_tree,
        Err(error) => {
            cleanup_identity_sandbox_temp(sandbox_temp)?;
            return Err(error);
        }
    };
    let Some(stdout) = child.stdout.take() else {
        let _ = stop_owned_child(&mut child, process_tree);
        cleanup_identity_sandbox_temp(sandbox_temp)?;
        return Err(error(CodexIdentityErrorKind::VersionCommandFailed));
    };
    let reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .take(MAX_VERSION_OUTPUT_BYTES + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let status = wait_for_owned_child(
        &mut child,
        process_tree,
        deadline,
        CodexIdentityErrorKind::VersionCommandFailed,
    );
    cleanup_identity_sandbox_temp(sandbox_temp)?;
    let status = status?;
    let stdout = reader
        .join()
        .map_err(|_| error(CodexIdentityErrorKind::VersionOutputInvalid))?
        .map_err(|_| error(CodexIdentityErrorKind::VersionOutputInvalid))?;
    if u64::try_from(stdout.len()).unwrap_or(u64::MAX) > MAX_VERSION_OUTPUT_BYTES {
        return Err(error(CodexIdentityErrorKind::VersionOutputInvalid));
    }
    Ok((status, stdout))
}

fn run_schema_command(
    launcher: &Path,
    schema_output_dir: &Path,
    codex_home: Option<&Path>,
    pinned_resources: Option<&PinnedCodexResources>,
    deadline: Instant,
) -> Result<ExitStatus, CodexIdentityError> {
    let (mut command, sandbox_temp) = identity_command(launcher, codex_home, pinned_resources)?;
    command
        .args(["app-server", "generate-json-schema", "--out"])
        .arg(schema_output_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    validate_identity_resources_before_spawn(launcher, pinned_resources)?;
    let Ok(mut child) = command.spawn() else {
        cleanup_identity_sandbox_temp(sandbox_temp)?;
        return Err(error(CodexIdentityErrorKind::SchemaGenerationFailed));
    };
    let Ok(process_tree) = OwnedProcessTree::attach(&child) else {
        let _ = terminate_uncontained_process_tree_bounded(&mut child);
        cleanup_identity_sandbox_temp(sandbox_temp)?;
        return Err(error(CodexIdentityErrorKind::ProcessContainmentFailed));
    };
    let process_tree = match validate_identity_resources_after_attach(
        &mut child,
        process_tree,
        launcher,
        pinned_resources,
    ) {
        Ok(process_tree) => process_tree,
        Err(error) => {
            cleanup_identity_sandbox_temp(sandbox_temp)?;
            return Err(error);
        }
    };
    let status = wait_for_owned_child(
        &mut child,
        process_tree,
        deadline,
        CodexIdentityErrorKind::SchemaGenerationFailed,
    );
    cleanup_identity_sandbox_temp(sandbox_temp)?;
    status
}

fn identity_command(
    launcher: &Path,
    codex_home: Option<&Path>,
    pinned_resources: Option<&PinnedCodexResources>,
) -> Result<(Command, Option<OwnedSandboxTemp>), CodexIdentityError> {
    let mut command = Command::new(launcher);
    crate::scrub_protected_environment(&mut command);
    if let Some(codex_home) = codex_home {
        command.env("CODEX_HOME", codex_home);
    }
    configure_pinned_child_environment(&mut command, pinned_resources)
        .map_err(|_| error(CodexIdentityErrorKind::PinnedResourcesRejected))?;
    let sandbox_temp = match (codex_home, pinned_resources) {
        (Some(codex_home), Some(_)) => Some(
            OwnedSandboxTemp::prepare(codex_home)
                .map_err(|_| error(CodexIdentityErrorKind::PinnedResourcesRejected))?,
        ),
        (None, Some(_)) => return Err(error(CodexIdentityErrorKind::PinnedResourcesRejected)),
        (_, None) => None,
    };
    if let Some(sandbox_temp) = &sandbox_temp {
        sandbox_temp.configure(&mut command);
    }
    Ok((command, sandbox_temp))
}

fn cleanup_identity_sandbox_temp(
    sandbox_temp: Option<OwnedSandboxTemp>,
) -> Result<(), CodexIdentityError> {
    cleanup_sandbox_temp(sandbox_temp)
        .map_err(|_| error(CodexIdentityErrorKind::ProcessContainmentFailed))
}

fn validate_identity_resources_before_spawn(
    launcher: &Path,
    pinned_resources: Option<&PinnedCodexResources>,
) -> Result<(), CodexIdentityError> {
    validate_pinned_resources_for_launcher(launcher, pinned_resources)
        .map_err(|_| error(CodexIdentityErrorKind::PinnedResourcesRejected))
}

fn validate_identity_resources_after_attach(
    child: &mut Child,
    process_tree: OwnedProcessTree,
    launcher: &Path,
    pinned_resources: Option<&PinnedCodexResources>,
) -> Result<OwnedProcessTree, CodexIdentityError> {
    if validate_pinned_resources_for_launcher(launcher, pinned_resources).is_err() {
        stop_owned_child(child, process_tree)
            .map_err(|_| error(CodexIdentityErrorKind::ProcessContainmentFailed))?;
        return Err(error(CodexIdentityErrorKind::PinnedResourcesChanged));
    }
    Ok(process_tree)
}

fn ensure_before_deadline(deadline: Instant) -> Result<(), CodexIdentityError> {
    if Instant::now() >= deadline {
        Err(error(CodexIdentityErrorKind::Timeout))
    } else {
        Ok(())
    }
}

fn wait_for_owned_child(
    child: &mut Child,
    process_tree: OwnedProcessTree,
    deadline: Instant,
    command_failure: CodexIdentityErrorKind,
) -> Result<ExitStatus, CodexIdentityError> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                drop(process_tree);
                return Ok(status);
            }
            Ok(None) => {}
            Err(_) => {
                let _ = stop_owned_child(child, process_tree);
                return Err(error(command_failure));
            }
        }

        let now = Instant::now();
        if now >= deadline {
            let _ = stop_owned_child(child, process_tree);
            return Err(error(CodexIdentityErrorKind::Timeout));
        }
        thread::sleep(PROCESS_POLL_INTERVAL.min(deadline.duration_since(now)));
    }
}

fn schema_bundle_digest(root: &Path) -> Result<(String, usize), CodexIdentityError> {
    let root_metadata =
        fs::symlink_metadata(root).map_err(|_| error(CodexIdentityErrorKind::SchemaBundleEmpty))?;
    if !root_metadata.file_type().is_dir() {
        return Err(error(CodexIdentityErrorKind::SchemaBundleInvalid));
    }

    let mut files = Vec::new();
    collect_schema_files(root, root, &mut files)?;
    if files.is_empty() {
        return Err(error(CodexIdentityErrorKind::SchemaBundleEmpty));
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));

    let mut hasher = Sha256::new();
    hasher.update(SCHEMA_BUNDLE_DOMAIN);
    for (relative, path) in &files {
        let relative_bytes = relative.as_bytes();
        let relative_len = u64::try_from(relative_bytes.len())
            .map_err(|_| error(CodexIdentityErrorKind::SchemaBundleInvalid))?;
        hasher.update(relative_len.to_be_bytes());
        hasher.update(relative_bytes);

        let metadata =
            fs::metadata(path).map_err(|_| error(CodexIdentityErrorKind::SchemaReadFailed))?;
        let canonical = canonical_json_file(path, metadata.len())?;
        let canonical_len = u64::try_from(canonical.len())
            .map_err(|_| error(CodexIdentityErrorKind::SchemaBundleInvalid))?;
        hasher.update(canonical_len.to_be_bytes());
        hasher.update(&canonical);
    }

    Ok((hex_digest(hasher.finalize().as_ref()), files.len()))
}

fn collect_schema_files(
    root: &Path,
    directory: &Path,
    files: &mut Vec<(String, PathBuf)>,
) -> Result<(), CodexIdentityError> {
    let entries =
        fs::read_dir(directory).map_err(|_| error(CodexIdentityErrorKind::SchemaReadFailed))?;
    for entry in entries {
        let entry = entry.map_err(|_| error(CodexIdentityErrorKind::SchemaReadFailed))?;
        let file_type = entry
            .file_type()
            .map_err(|_| error(CodexIdentityErrorKind::SchemaReadFailed))?;
        let path = entry.path();
        if file_type.is_dir() {
            collect_schema_files(root, &path, files)?;
        } else if file_type.is_file() {
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                return Err(error(CodexIdentityErrorKind::SchemaBundleInvalid));
            }
            let relative = path
                .strip_prefix(root)
                .map_err(|_| error(CodexIdentityErrorKind::SchemaBundleInvalid))?;
            files.push((normalized_relative_path(relative)?, path));
        } else {
            return Err(error(CodexIdentityErrorKind::SchemaBundleInvalid));
        }
    }
    Ok(())
}

fn normalized_relative_path(relative: &Path) -> Result<String, CodexIdentityError> {
    let mut normalized = String::new();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(error(CodexIdentityErrorKind::SchemaBundleInvalid));
        };
        let name = name
            .to_str()
            .ok_or_else(|| error(CodexIdentityErrorKind::SchemaBundleInvalid))?;
        if name.contains(['/', '\\']) {
            return Err(error(CodexIdentityErrorKind::SchemaBundleInvalid));
        }
        if !normalized.is_empty() {
            normalized.push('/');
        }
        normalized.push_str(name);
    }
    if normalized.is_empty() {
        return Err(error(CodexIdentityErrorKind::SchemaBundleInvalid));
    }
    Ok(normalized)
}

fn file_sha256(path: &Path, failure: CodexIdentityErrorKind) -> Result<String, CodexIdentityError> {
    let mut file = File::open(path).map_err(|_| error(failure))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|_| error(failure))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex_digest(hasher.finalize().as_ref()))
}

fn canonical_json_file(path: &Path, expected_len: u64) -> Result<Vec<u8>, CodexIdentityError> {
    let bytes = fs::read(path).map_err(|_| error(CodexIdentityErrorKind::SchemaReadFailed))?;
    if u64::try_from(bytes.len()).map_err(|_| error(CodexIdentityErrorKind::SchemaBundleInvalid))?
        != expected_len
    {
        return Err(error(CodexIdentityErrorKind::SchemaReadFailed));
    }
    canonical_json_bytes(&bytes)
}

fn canonical_json_bytes(bytes: &[u8]) -> Result<Vec<u8>, CodexIdentityError> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|_| error(CodexIdentityErrorKind::SchemaBundleInvalid))?;
    serde_json::to_vec(&normalize_json(value))
        .map_err(|_| error(CodexIdentityErrorKind::SchemaBundleInvalid))
}

fn normalize_json(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(normalize_json).collect()),
        Value::Object(values) => {
            let mut entries: Vec<_> = values.into_iter().collect();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            let mut normalized = Map::new();
            for (key, value) in entries {
                normalized.insert(key, normalize_json(value));
            }
            Value::Object(normalized)
        }
        scalar => scalar,
    }
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}

const fn error(kind: CodexIdentityErrorKind) -> CodexIdentityError {
    CodexIdentityError::new(kind)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::path::Path;

    use super::{CodexIdentityErrorKind, canonical_json_bytes, error, identity_command};

    #[test]
    fn schema_identity_ignores_json_object_member_order() {
        let first =
            br#"{"definitions":{"z":{"type":"string"},"a":{"type":"number"}},"title":"schema"}"#;
        let second =
            br#"{"title":"schema","definitions":{"a":{"type":"number"},"z":{"type":"string"}}}"#;

        assert_eq!(
            canonical_json_bytes(first).expect("first schema is valid"),
            canonical_json_bytes(second).expect("second schema is valid")
        );
    }

    #[test]
    fn identity_commands_bind_the_isolated_codex_home() {
        let (command, sandbox_temp) = identity_command(
            Path::new("codex"),
            Some(Path::new(r"C:\lattice\isolated-codex-home")),
            None,
        )
        .expect("unmanaged identity command");
        assert!(sandbox_temp.is_none());
        let codex_home = command
            .get_envs()
            .find(|(name, _)| *name == OsStr::new("CODEX_HOME"))
            .expect("CODEX_HOME must be explicit");

        assert_eq!(
            codex_home.1,
            Some(OsStr::new(r"C:\lattice\isolated-codex-home"))
        );
    }

    #[test]
    fn pinned_resource_phase_errors_have_distinct_stable_codes() {
        assert_eq!(
            error(CodexIdentityErrorKind::PinnedResourcesRejected).to_string(),
            "CODEX_IDENTITY_PINNED_RESOURCES_REJECTED"
        );
        assert_eq!(
            error(CodexIdentityErrorKind::PinnedResourcesChanged).to_string(),
            "CODEX_IDENTITY_PINNED_RESOURCES_CHANGED"
        );
    }
}
