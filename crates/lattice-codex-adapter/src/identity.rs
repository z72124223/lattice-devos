use std::error::Error;
use std::fmt::{self, Write as _};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

const SCHEMA_BUNDLE_DOMAIN: &[u8] = b"lattice.codex-app-server.schema-bundle.v1\0";

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
}

/// Payload-free Codex identity preflight error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CodexIdentityError {
    kind: CodexIdentityErrorKind,
}

impl CodexIdentityError {
    const fn new(kind: CodexIdentityErrorKind) -> Self {
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

    let version_output = Command::new(configured_launcher)
        .arg("--version")
        .output()
        .map_err(|_| error(CodexIdentityErrorKind::VersionCommandFailed))?;
    if !version_output.status.success() {
        return Err(error(CodexIdentityErrorKind::VersionCommandFailed));
    }
    let version = String::from_utf8(version_output.stdout)
        .map_err(|_| error(CodexIdentityErrorKind::VersionOutputInvalid))?;
    let version = version.trim_end_matches(['\r', '\n']).to_owned();
    if version != expectation.version() {
        return Err(error(CodexIdentityErrorKind::VersionMismatch));
    }

    let schema_output = Command::new(configured_launcher)
        .args(["app-server", "generate-json-schema", "--out"])
        .arg(schema_output_dir)
        .output()
        .map_err(|_| error(CodexIdentityErrorKind::SchemaGenerationFailed))?;
    if !schema_output.status.success() {
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
        hasher.update(metadata.len().to_be_bytes());
        hash_file_contents(path, metadata.len(), &mut hasher)?;
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

fn hash_file_contents(
    path: &Path,
    expected_len: u64,
    hasher: &mut Sha256,
) -> Result<(), CodexIdentityError> {
    let mut file = File::open(path).map_err(|_| error(CodexIdentityErrorKind::SchemaReadFailed))?;
    let mut observed_len = 0_u64;
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| error(CodexIdentityErrorKind::SchemaReadFailed))?;
        if read == 0 {
            break;
        }
        observed_len = observed_len
            .checked_add(
                u64::try_from(read)
                    .map_err(|_| error(CodexIdentityErrorKind::SchemaBundleInvalid))?,
            )
            .ok_or_else(|| error(CodexIdentityErrorKind::SchemaBundleInvalid))?;
        hasher.update(&buffer[..read]);
    }
    if observed_len != expected_len {
        return Err(error(CodexIdentityErrorKind::SchemaReadFailed));
    }
    Ok(())
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
