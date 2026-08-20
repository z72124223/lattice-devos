//! Fixed WSL bubblewrap/Landlock boundary for one Hermes reflection.

use std::ffi::OsString;

#[cfg(windows)]
use std::collections::BTreeMap;
#[cfg(windows)]
use std::fs;
#[cfg(windows)]
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(windows)]
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(windows)]
use lattice_contracts::ContentDigest;
#[cfg(windows)]
use serde::{Deserialize, Serialize};
#[cfg(windows)]
use sha2::{Digest, Sha256};

use crate::{HermesAdapterError, HermesAdapterErrorKind, HermesAdapterResult};

const BWRAP_PATH: &str = "/usr/bin/bwrap";
pub(crate) const HERMES_BWRAP_PACKAGE_VERSION: &str = "0.11.1-1ubuntu0.1";
pub(crate) const HERMES_BWRAP_PACKAGE_SOURCE: &str =
    "Ubuntu 26.04 LTS resolute-security USN-8288-1 CVE-2026-41163";
pub(crate) const HERMES_BWRAP_PACKAGE_DEB_SHA256: &str =
    "b353088d1003adb3f760deeccfb84c47928a36c8dc102bf680efc94eb19f4408";
pub(crate) const HERMES_BWRAP_SHA256: &str =
    "0abea81db798ebf6b4742ac0664802d97521547a353c2a0dbdc21d76cbbfd2c0";
#[cfg(test)]
pub(crate) const HERMES_HISTORICAL_VULNERABLE_BWRAP_SHA256: &str =
    "8e19e40e7d5f7a7e8b488c7926feb040eab6ed10c58fa360e266d2f70670e92b";
const PRIVATE_RUNNER_SOURCE: &str = include_str!("hermes_sandbox_runner.py");
const PRIVATE_FRAME_MAGIC: &[u8] = b"LATTICE_HERMES_CONTAINED_V1\n";
const PRIVATE_FRAME_FIELD_COUNT: usize = 7;
#[cfg(windows)]
const WSL_DISTRO: &str = "Ubuntu";
#[cfg(windows)]
const WSL_EXE_SHA256: &str = "4e589e3883229b7a74a4acdb878689dcec94e2539fcad1c194f415b149c337a9";
#[cfg(windows)]
const OUTER_RUNNER_SOURCE: &str = include_str!("wsl_outer_runner.py");
#[cfg(windows)]
const SOCKETPAIR_MAGIC: &[u8] = b"LATTICE_HERMES_SOCKETPAIR_V1\n";
#[cfg(windows)]
static CANARY_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// Static mount and namespace contract for the production Hermes sandbox.
///
/// This type deliberately contains no caller-selected mount paths. Host paths
/// are validated separately and may only populate these fixed guest targets.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HermesSandboxProfile {
    _private: (),
}

impl HermesSandboxProfile {
    /// Returns the only production sandbox profile.
    #[must_use]
    pub const fn official() -> Self {
        Self { _private: () }
    }

    /// Empty, non-product working directory visible to Hermes.
    #[must_use]
    pub const fn work_directory(self) -> &'static str {
        "/work"
    }

    /// Exact read-only ingress roots visible inside the sandbox.
    #[must_use]
    pub const fn read_only_ingress(self) -> [&'static str; 4] {
        [
            "/runtime-input",
            "/config-input",
            "/request-input",
            "/broker-input",
        ]
    }

    /// Exact paths which the Landlock ruleset permits to be mutated.
    #[must_use]
    pub const fn writable_paths(self) -> [&'static str; 3] {
        ["/state", "/output", "/tmp"]
    }

    /// Product source is intentionally not representable as a mount.
    #[must_use]
    pub const fn product_source_mount(self) -> Option<&'static str> {
        None
    }

    /// Bubblewrap always creates a private network namespace.
    #[must_use]
    pub const fn network_namespace_isolated(self) -> bool {
        true
    }

    /// `REFER` and `TRUNCATE` mediation require Landlock ABI 3 or newer.
    #[must_use]
    pub const fn minimum_landlock_abi(self) -> u32 {
        3
    }
}

impl Default for HermesSandboxProfile {
    fn default() -> Self {
        Self::official()
    }
}

/// Strict upper bounds for one private containment frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HermesContainmentFrameLimits {
    max_reflection_bytes: u64,
}

impl HermesContainmentFrameLimits {
    /// Creates strict limits for a private containment frame.
    #[must_use]
    pub const fn new(max_reflection_bytes: u64) -> Self {
        Self {
            max_reflection_bytes,
        }
    }
}

impl Default for HermesContainmentFrameLimits {
    fn default() -> Self {
        Self::new(2 * 1024 * 1024)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HermesContainmentFrame<'a> {
    runtime_manifest_sha256: &'a [u8],
    config_sha256: &'a [u8],
    request_sha256: &'a [u8],
    broker_receipt_sha256: &'a [u8],
    canary_sha256: &'a [u8],
    transcript_sha256: &'a [u8],
    reflection: &'a [u8],
}

impl<'a> HermesContainmentFrame<'a> {
    /// Pinned runtime manifest digest.
    #[must_use]
    pub const fn runtime_manifest_sha256(self) -> &'a [u8] {
        self.runtime_manifest_sha256
    }

    /// Canonical redacted request digest.
    #[must_use]
    pub const fn request_sha256(self) -> &'a [u8] {
        self.request_sha256
    }

    /// Strict canonical reflection payload.
    #[must_use]
    pub const fn reflection(self) -> &'a [u8] {
        self.reflection
    }

    /// Exact contained config digest.
    #[must_use]
    pub const fn config_sha256(self) -> &'a [u8] {
        self.config_sha256
    }

    /// Sealed host broker receipt digest.
    #[must_use]
    pub const fn broker_receipt_sha256(self) -> &'a [u8] {
        self.broker_receipt_sha256
    }

    /// No-marker canary receipt digest.
    #[must_use]
    pub const fn canary_sha256(self) -> &'a [u8] {
        self.canary_sha256
    }

    /// Digest-only contained transcript.
    #[must_use]
    pub const fn transcript_sha256(self) -> &'a [u8] {
        self.transcript_sha256
    }
}

/// Builds the fixed inner bubblewrap command. Sources are constrained to
/// LATTICE-owned WSL runtime/broker roots or single-run Windows ingress paths;
/// no product path parameter exists.
///
/// # Errors
///
/// Rejects non-canonical guest paths and any source outside the fixed
/// LATTICE-owned ingress roots.
pub fn build_hermes_bwrap_arguments(
    runtime_root: &str,
    config_root: &str,
    request_file: &str,
) -> HermesAdapterResult<Vec<OsString>> {
    validate_guest_source(
        runtime_root,
        "/var/tmp/lattice-runtime-targets/",
        "HERMES_RUNTIME_GUEST_PATH_REJECTED",
    )?;
    validate_guest_source(config_root, "/mnt/", "HERMES_CONFIG_GUEST_PATH_REJECTED")?;
    validate_guest_source(request_file, "/mnt/", "HERMES_REQUEST_GUEST_PATH_REJECTED")?;
    let mut arguments = Vec::new();
    for argument in [
        BWRAP_PATH,
        "--die-with-parent",
        "--unshare-all",
        "--unshare-user",
        "--disable-userns",
        "--assert-userns-disabled",
        "--new-session",
        "--cap-drop",
        "ALL",
        "--ro-bind",
        "/usr",
        "/usr",
        "--ro-bind",
        "/lib",
        "/lib",
        "--ro-bind",
        "/lib64",
        "/lib64",
        "--proc",
        "/proc",
        "--dev",
        "/dev",
        "--dir",
        "/work",
        "--tmpfs",
        "/state",
        "--tmpfs",
        "/output",
        "--tmpfs",
        "/tmp",
    ] {
        arguments.push(OsString::from(argument));
    }
    for (source, destination) in [
        (runtime_root, "/runtime-input"),
        (config_root, "/config-input"),
        (request_file, "/request-input/request.json"),
    ] {
        arguments.push(OsString::from("--ro-bind"));
        arguments.push(OsString::from(source));
        arguments.push(OsString::from(destination));
    }
    arguments.push(OsString::from("--clearenv"));
    for (name, value) in [
        (
            "PATH",
            "/config-input/bin:/runtime-input/python/bin:/usr/bin:/bin",
        ),
        ("HOME", "/state/hermes"),
        ("HERMES_HOME", "/state/hermes"),
        ("CODEX_HOME", "/state/codex-unavailable"),
        ("LATTICE_CODEX_BROKER_READ_FD", "0"),
        ("LATTICE_CODEX_BROKER_WRITE_FD", "1"),
        ("TMPDIR", "/tmp"),
        ("PYTHONDONTWRITEBYTECODE", "1"),
        ("PYTHONHASHSEED", "0"),
        ("PYTHONNOUSERSITE", "1"),
        ("PYTHONSAFEPATH", "1"),
        ("PYTHONUTF8", "1"),
        ("NO_COLOR", "1"),
        ("CI", "1"),
        ("TZ", "UTC"),
        ("LANG", "C.UTF-8"),
        ("LC_ALL", "C.UTF-8"),
    ] {
        arguments.extend([
            OsString::from("--setenv"),
            OsString::from(name),
            OsString::from(value),
        ]);
    }
    arguments.extend([
        OsString::from("--chdir"),
        OsString::from("/work"),
        OsString::from("/runtime-input/python/bin/python3.12"),
        OsString::from("-I"),
        OsString::from("-S"),
        OsString::from("-B"),
        OsString::from("-c"),
        OsString::from(PRIVATE_RUNNER_SOURCE),
        OsString::from("contained-reflection"),
    ]);
    Ok(arguments)
}

/// Parses one bounded private frame and rejects unknown/trailing bytes.
///
/// # Errors
///
/// Rejects the wrong magic, length overflow, non-digest bindings, an empty
/// reflection, or trailing bytes.
pub fn parse_containment_frame(
    bytes: &[u8],
    limits: HermesContainmentFrameLimits,
) -> HermesAdapterResult<HermesContainmentFrame<'_>> {
    let Some(mut remaining) = bytes.strip_prefix(PRIVATE_FRAME_MAGIC) else {
        return Err(malformed("HERMES_CONTAINMENT_FRAME_MAGIC_REJECTED"));
    };
    let mut fields = Vec::with_capacity(PRIVATE_FRAME_FIELD_COUNT);
    for index in 0..PRIVATE_FRAME_FIELD_COUNT {
        let length_bytes: [u8; 8] = remaining
            .get(..8)
            .and_then(|prefix| prefix.try_into().ok())
            .ok_or_else(|| malformed("HERMES_CONTAINMENT_FRAME_TRUNCATED"))?;
        remaining = &remaining[8..];
        let length = usize::try_from(u64::from_be_bytes(length_bytes))
            .map_err(|_| malformed("HERMES_CONTAINMENT_FRAME_LENGTH_OVERFLOW"))?;
        let bound = if index + 1 == PRIVATE_FRAME_FIELD_COUNT {
            limits.max_reflection_bytes
        } else {
            64
        };
        if u64::try_from(length).map_or(true, |value| value > bound) {
            return Err(malformed("HERMES_CONTAINMENT_FRAME_FIELD_LIMIT"));
        }
        let (field, tail) = remaining
            .split_at_checked(length)
            .ok_or_else(|| malformed("HERMES_CONTAINMENT_FRAME_TRUNCATED"))?;
        fields.push(field);
        remaining = tail;
    }
    if !remaining.is_empty() {
        return Err(malformed("HERMES_CONTAINMENT_FRAME_TRAILING_BYTES"));
    }
    if fields[..6].iter().any(|field| !is_lowercase_sha256(field)) || fields[6].is_empty() {
        return Err(malformed("HERMES_CONTAINMENT_FRAME_FIELD_REJECTED"));
    }
    Ok(HermesContainmentFrame {
        runtime_manifest_sha256: fields[0],
        config_sha256: fields[1],
        request_sha256: fields[2],
        broker_receipt_sha256: fields[3],
        canary_sha256: fields[4],
        transcript_sha256: fields[5],
        reflection: fields[6],
    })
}

fn validate_guest_source(
    value: &str,
    required_prefix: &str,
    code: &'static str,
) -> HermesAdapterResult<()> {
    if !value.starts_with(required_prefix)
        || value.len() <= required_prefix.len()
        || !value.is_ascii()
        || value.bytes().any(|byte| byte.is_ascii_control())
        || value.split('/').any(|component| component == "..")
        || value.ends_with('/')
    {
        return Err(HermesAdapterError::new(
            HermesAdapterErrorKind::Configuration,
            code,
        ));
    }
    Ok(())
}

fn is_lowercase_sha256(value: &[u8]) -> bool {
    value.len() == 64
        && value
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
}

fn malformed(code: &'static str) -> HermesAdapterError {
    HermesAdapterError::new(HermesAdapterErrorKind::Malformed, code)
}

/// Exact WSL launcher and LATTICE-owned run root for containment preflights.
#[cfg(windows)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HermesWslContainmentConfig {
    wsl_executable: PathBuf,
    runtime_guest_root: String,
    isolation_root: PathBuf,
    product_root: PathBuf,
}

#[cfg(windows)]
impl HermesWslContainmentConfig {
    /// Creates an exact WSL/bubblewrap containment preflight configuration.
    ///
    /// # Errors
    ///
    /// Rejects launcher identity, guest runtime path, product overlap,
    /// reparse points, and non-LATTICE isolation roots.
    pub fn new(
        wsl_executable: impl Into<PathBuf>,
        runtime_guest_root: impl Into<String>,
        isolation_root: impl Into<PathBuf>,
        product_root: impl Into<PathBuf>,
    ) -> HermesAdapterResult<Self> {
        let wsl_executable = wsl_executable.into();
        let runtime_guest_root = runtime_guest_root.into();
        let isolation_root = isolation_root.into();
        let product_root = product_root.into();
        validate_guest_source(
            &runtime_guest_root,
            "/var/tmp/lattice-runtime-targets/",
            "HERMES_RUNTIME_GUEST_PATH_REJECTED",
        )?;
        let wsl_executable = fs::canonicalize(&wsl_executable).map_err(|_| {
            HermesAdapterError::new(
                HermesAdapterErrorKind::Identity,
                "HERMES_WSL_LAUNCHER_IDENTITY_REJECTED",
            )
        })?;
        if crate::sha256_file(&wsl_executable)? != WSL_EXE_SHA256 {
            return Err(HermesAdapterError::new(
                HermesAdapterErrorKind::Identity,
                "HERMES_WSL_LAUNCHER_IDENTITY_REJECTED",
            ));
        }
        let (isolation_root, product_root) =
            crate::validate_isolation_boundary(&isolation_root, &product_root)?;
        Ok(Self {
            wsl_executable,
            runtime_guest_root,
            isolation_root,
            product_root,
        })
    }

    /// Executes the real WSL/bubblewrap socketpair canary under one caller-
    /// owned absolute deadline and a kill-on-close Windows Job.
    ///
    /// # Errors
    ///
    /// Fails closed on identity, containment, path, protocol, deadline, or
    /// descendant-reap ambiguity.
    pub fn run_socketpair_canary(
        &self,
        deadline: Instant,
    ) -> HermesAdapterResult<HermesSocketpairReceipt> {
        if deadline <= Instant::now() || self.isolation_root.exists() {
            return Err(HermesAdapterError::new(
                HermesAdapterErrorKind::Configuration,
                "HERMES_SOCKETPAIR_CANARY_ROOT_REJECTED",
            ));
        }
        if self.isolation_root.starts_with(&self.product_root)
            || self.product_root.starts_with(&self.isolation_root)
        {
            return Err(HermesAdapterError::new(
                HermesAdapterErrorKind::Configuration,
                "HERMES_PRODUCT_ROOT_OVERLAP_REJECTED",
            ));
        }
        fs::create_dir(&self.isolation_root).map_err(|_| {
            HermesAdapterError::new(
                HermesAdapterErrorKind::Spawn,
                "HERMES_SOCKETPAIR_CANARY_ROOT_CREATE_FAILED",
            )
        })?;
        let capture_root = self.isolation_root.join("capture");
        fs::create_dir(&capture_root).map_err(|_| {
            HermesAdapterError::new(
                HermesAdapterErrorKind::Spawn,
                "HERMES_SOCKETPAIR_CANARY_CAPTURE_CREATE_FAILED",
            )
        })?;
        let nonce = canary_nonce(&self.isolation_root)?;
        let interpreter = format!("{}/python/bin/python3.12", self.runtime_guest_root);
        let arguments = [
            "-d",
            WSL_DISTRO,
            "--exec",
            interpreter.as_str(),
            "-I",
            "-S",
            "-B",
            "-c",
            OUTER_RUNNER_SOURCE,
            "socketpair-canary",
            self.runtime_guest_root.as_str(),
            nonce.as_str(),
        ]
        .into_iter()
        .map(OsString::from)
        .collect::<Vec<_>>();
        let plan = crate::windows_job::WindowsJobCommandPlan {
            executable: self.wsl_executable.clone(),
            arguments,
            current_dir: self.isolation_root.clone(),
            environment: minimal_wsl_environment(&self.wsl_executable)?,
            run_root: self.isolation_root.clone(),
            stdout_path: capture_root.join("socketpair.stdout"),
            stderr_path: capture_root.join("socketpair.stderr"),
            stdout_limit: 4096,
            stderr_limit: 4096,
            deadline,
            teardown_timeout: Duration::from_secs(2),
        };
        let outcome = crate::windows_job::run(&plan)?;
        if outcome.exit_code != 0 || !outcome.stderr.is_empty() {
            let code = match outcome.exit_code {
                64 => "HERMES_SOCKETPAIR_CANARY_ARGUMENT_REJECTED",
                65 => "HERMES_SOCKETPAIR_CANARY_PYTHON_REJECTED",
                66 => "HERMES_SOCKETPAIR_CANARY_BWRAP_REJECTED",
                67 => "HERMES_SOCKETPAIR_CANARY_CHANNEL_REJECTED",
                68 => "HERMES_SOCKETPAIR_CANARY_CHILD_REJECTED",
                _ => "HERMES_SOCKETPAIR_CANARY_FAILED",
            };
            return Err(HermesAdapterError::new(
                HermesAdapterErrorKind::Failed,
                code,
            ));
        }
        parse_socketpair_receipt(&outcome.stdout, &nonce)
    }
}

#[cfg(windows)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HermesSocketpairReceipt {
    broker_read_fd: u32,
    broker_write_fd: u32,
    bwrap_sha256: String,
    descendants_reaped: bool,
    python_version: String,
    receipt_digest: ContentDigest,
}

#[cfg(windows)]
impl HermesSocketpairReceipt {
    /// In-sandbox broker read descriptor.
    #[must_use]
    pub const fn broker_read_fd(&self) -> u32 {
        self.broker_read_fd
    }

    /// In-sandbox broker write descriptor.
    #[must_use]
    pub const fn broker_write_fd(&self) -> u32 {
        self.broker_write_fd
    }

    /// Exact bubblewrap digest.
    #[must_use]
    pub fn bwrap_sha256(&self) -> &str {
        &self.bwrap_sha256
    }

    /// Whether the owning Job proved all descendants exited.
    #[must_use]
    pub const fn descendants_reaped(&self) -> bool {
        self.descendants_reaped
    }

    /// Pinned Linux Python version observed inside WSL.
    #[must_use]
    pub fn python_version(&self) -> &str {
        &self.python_version
    }

    /// Digest of the strict socketpair receipt.
    #[must_use]
    pub const fn receipt_digest(&self) -> &ContentDigest {
        &self.receipt_digest
    }

    pub(crate) fn validate_for_containment(&self) -> HermesAdapterResult<()> {
        if self.broker_read_fd != 0
            || self.broker_write_fd != 1
            || !self.descendants_reaped
            || self.python_version != "3.12.13"
        {
            return Err(HermesAdapterError::new(
                HermesAdapterErrorKind::CrossBinding,
                "HERMES_SOCKETPAIR_RECEIPT_BINDING_REJECTED",
            ));
        }
        validate_approved_bwrap_digest(&self.bwrap_sha256)
    }
}

#[cfg(windows)]
pub(crate) fn validate_approved_bwrap_digest(digest: &str) -> HermesAdapterResult<()> {
    if digest == HERMES_BWRAP_SHA256 {
        Ok(())
    } else {
        Err(HermesAdapterError::new(
            HermesAdapterErrorKind::Identity,
            "HERMES_BWRAP_SECURITY_IDENTITY_REJECTED",
        ))
    }
}

#[cfg(windows)]
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SocketpairReceiptWire {
    broker_read_fd: u32,
    broker_write_fd: u32,
    bwrap_package_deb_sha256: String,
    bwrap_package_source: String,
    bwrap_package_version: String,
    bwrap_sha256: String,
    descendants_reaped: bool,
    nonce_binding_sha256: String,
    python_version: String,
    schema: String,
}

#[cfg(windows)]
fn parse_socketpair_receipt(
    bytes: &[u8],
    nonce: &str,
) -> HermesAdapterResult<HermesSocketpairReceipt> {
    let payload = bytes
        .strip_prefix(SOCKETPAIR_MAGIC)
        .ok_or_else(|| malformed("HERMES_SOCKETPAIR_RECEIPT_MAGIC_REJECTED"))?;
    let length_bytes: [u8; 8] = payload
        .get(..8)
        .and_then(|value| value.try_into().ok())
        .ok_or_else(|| malformed("HERMES_SOCKETPAIR_RECEIPT_TRUNCATED"))?;
    let length = usize::try_from(u64::from_be_bytes(length_bytes))
        .map_err(|_| malformed("HERMES_SOCKETPAIR_RECEIPT_LENGTH_REJECTED"))?;
    if length == 0 || length > 2048 || payload.len() != 8 + length {
        return Err(malformed("HERMES_SOCKETPAIR_RECEIPT_LENGTH_REJECTED"));
    }
    let encoded = &payload[8..];
    let wire: SocketpairReceiptWire = serde_json::from_slice(encoded)
        .map_err(|_| malformed("HERMES_SOCKETPAIR_RECEIPT_MALFORMED"))?;
    if serde_json::to_vec(&wire).map_err(|_| malformed("HERMES_SOCKETPAIR_RECEIPT_MALFORMED"))?
        != encoded
    {
        return Err(malformed("HERMES_SOCKETPAIR_RECEIPT_NON_CANONICAL"));
    }
    let mut binding = Sha256::new();
    let nonce_bytes = decode_sha256(nonce)?;
    binding.update(nonce_bytes);
    binding.update(b"LATTICE_SOCKETPAIR_CANARY");
    let expected_binding = encode_digest(&binding.finalize());
    if wire.schema != "lattice.hermes.socketpair-receipt.v2"
        || wire.broker_read_fd != 0
        || wire.broker_write_fd != 1
        || wire.bwrap_package_version != HERMES_BWRAP_PACKAGE_VERSION
        || wire.bwrap_package_source != HERMES_BWRAP_PACKAGE_SOURCE
        || wire.bwrap_package_deb_sha256 != HERMES_BWRAP_PACKAGE_DEB_SHA256
        || !wire.descendants_reaped
        || wire.python_version != "3.12.13"
        || wire.nonce_binding_sha256 != expected_binding
    {
        return Err(HermesAdapterError::new(
            HermesAdapterErrorKind::CrossBinding,
            "HERMES_SOCKETPAIR_RECEIPT_BINDING_REJECTED",
        ));
    }
    validate_approved_bwrap_digest(&wire.bwrap_sha256)?;
    let receipt_digest = ContentDigest::from_sha256(encode_digest(&Sha256::digest(bytes)))
        .map_err(|_| malformed("HERMES_SOCKETPAIR_RECEIPT_DIGEST_REJECTED"))?;
    Ok(HermesSocketpairReceipt {
        broker_read_fd: wire.broker_read_fd,
        broker_write_fd: wire.broker_write_fd,
        bwrap_sha256: wire.bwrap_sha256,
        descendants_reaped: wire.descendants_reaped,
        python_version: wire.python_version,
        receipt_digest,
    })
}

#[cfg(windows)]
fn canary_nonce(isolation_root: &Path) -> HermesAdapterResult<String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| malformed("HERMES_SOCKETPAIR_NONCE_CLOCK_REJECTED"))?;
    let sequence = CANARY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let mut digest = Sha256::new();
    digest.update(b"lattice.hermes.socketpair.nonce.v1\0");
    digest.update(std::process::id().to_be_bytes());
    digest.update(sequence.to_be_bytes());
    digest.update(now.as_nanos().to_be_bytes());
    digest.update(isolation_root.as_os_str().to_string_lossy().as_bytes());
    Ok(encode_digest(&digest.finalize()))
}

#[cfg(windows)]
fn minimal_wsl_environment(executable: &Path) -> HermesAdapterResult<BTreeMap<OsString, OsString>> {
    let mut environment = BTreeMap::new();
    for name in ["SystemRoot", "WINDIR", "ComSpec", "PATHEXT"] {
        if let Some(value) = std::env::var_os(name) {
            environment.insert(OsString::from(name), value);
        }
    }
    let parent = executable.parent().ok_or_else(|| {
        HermesAdapterError::new(
            HermesAdapterErrorKind::Configuration,
            "HERMES_WSL_LAUNCHER_PARENT_REJECTED",
        )
    })?;
    let mut path_entries = vec![parent.to_path_buf()];
    if let Some(root) = std::env::var_os("SystemRoot") {
        path_entries.push(PathBuf::from(root).join("System32"));
    }
    environment.insert(
        OsString::from("PATH"),
        std::env::join_paths(path_entries).map_err(|_| {
            HermesAdapterError::new(
                HermesAdapterErrorKind::Configuration,
                "HERMES_WSL_MINIMAL_PATH_REJECTED",
            )
        })?,
    );
    Ok(environment)
}

#[cfg(windows)]
fn decode_sha256(value: &str) -> HermesAdapterResult<[u8; 32]> {
    if value.len() != 64 {
        return Err(malformed("HERMES_SOCKETPAIR_NONCE_REJECTED"));
    }
    let mut output = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let text =
            std::str::from_utf8(pair).map_err(|_| malformed("HERMES_SOCKETPAIR_NONCE_REJECTED"))?;
        output[index] = u8::from_str_radix(text, 16)
            .map_err(|_| malformed("HERMES_SOCKETPAIR_NONCE_REJECTED"))?;
    }
    Ok(output)
}

#[cfg(windows)]
fn encode_digest(digest: &[u8]) -> String {
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}
