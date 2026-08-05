//! Unique owned Windows -> WSL2 -> bubblewrap Hermes construction chain.

use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use lattice_contracts::{ContentDigest, HermesEvidence, HermesResearchRequest, RequestId};
use lattice_ports::{HermesPort, PortError, PortResult};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::broker::CodexBrokerReceipt;
use crate::containment::{
    HermesContainmentFrameLimits, HermesWslContainmentConfig, OUTER_RUNNER_SOURCE,
    PRIVATE_RUNNER_SOURCE, WSL_DISTRO, minimal_wsl_environment, parse_containment_frame,
};
use crate::runtime::HermesOfflineRuntimeManifest;
use crate::{
    CanonicalReflection, ContainmentOwnerState, HermesAdapterConfig, HermesAdapterError,
    HermesAdapterErrorKind, HermesAdapterResult, HermesContainmentReceipt, HermesReflectionAdapter,
    HermesReflectionEvidence, HermesReflectionJob, cross_binding, encode_sha256, error, malformed,
    map_port_error, sha256_text,
};

const STARTUP_MAGIC: &[u8] = b"LATTICE_HERMES_PRODUCTION_START_V1\n";
const STARTUP_SCHEMA: &str = "lattice.hermes.production-start.v1";
const ATTESTATION_SCHEMA: &str = "lattice.hermes.containment-attestation.v2";
const CONFIG_SCHEMA: &str = "lattice.hermes.production-config.v1";
const BWRAP_SHA256: &str = "8e19e40e7d5f7a7e8b488c7926feb040eab6ed10c58fa360e266d2f70670e92b";
const MAX_STARTUP_BYTES: usize = 128 * 1024;
const MAX_RUNNER_TIMEOUT: Duration = Duration::from_mins(5);
static RUNNER_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
enum RunnerMode {
    Official,
    #[cfg(test)]
    ScriptedFixture(String),
}

impl RunnerMode {
    const fn as_str(&self) -> &'static str {
        match self {
            Self::Official => "official",
            #[cfg(test)]
            Self::ScriptedFixture(_) => "scripted_fixture",
        }
    }

    fn fixture_reflection(&self) -> Option<&str> {
        match self {
            Self::Official => None,
            #[cfg(test)]
            Self::ScriptedFixture(reflection) => Some(reflection),
        }
    }
}

/// Validated inputs for the only production Hermes construction chain.
///
/// This value cannot install a receipt into an arbitrary adapter. [`Self::launch`]
/// owns WSL, bubblewrap, the namespace PID, endpoint, and adapter together.
pub struct HermesProductionRunnerConfig {
    containment: HermesWslContainmentConfig,
    expected_request: HermesResearchRequest,
    runtime_manifest_sha256: String,
    broker_receipt_sha256: String,
    api_key: String,
    model: String,
    startup_timeout: Duration,
    operation_timeout: Duration,
    poll_interval: Duration,
    mode: RunnerMode,
}

impl HermesProductionRunnerConfig {
    /// Creates an exact official-runtime runner configuration.
    ///
    /// # Errors
    ///
    /// Rejects an invalid broker receipt, runtime manifest serialization,
    /// bearer/model value, or relative timeout before any process is started.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        containment: HermesWslContainmentConfig,
        runtime_manifest: &HermesOfflineRuntimeManifest,
        broker_receipt: &CodexBrokerReceipt,
        expected_request: HermesResearchRequest,
        api_key: impl Into<String>,
        model: impl Into<String>,
        startup_timeout: Duration,
        operation_timeout: Duration,
        poll_interval: Duration,
    ) -> HermesAdapterResult<Self> {
        broker_receipt.validate_for_containment()?;
        Self::validated(
            containment,
            runtime_manifest,
            broker_receipt.receipt_digest().as_str().to_owned(),
            expected_request,
            api_key.into(),
            model.into(),
            startup_timeout,
            operation_timeout,
            poll_interval,
            RunnerMode::Official,
        )
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn scripted_fixture(
        containment: HermesWslContainmentConfig,
        runtime_manifest: &HermesOfflineRuntimeManifest,
        broker_receipt_digest: &ContentDigest,
        expected_request: HermesResearchRequest,
        api_key: impl Into<String>,
        model: impl Into<String>,
        startup_timeout: Duration,
        operation_timeout: Duration,
        poll_interval: Duration,
        reflection: impl Into<String>,
    ) -> HermesAdapterResult<Self> {
        let reflection = reflection.into();
        if reflection.is_empty() || reflection.len() > 64 * 1024 {
            return Err(malformed("HERMES_PRODUCTION_FIXTURE_REJECTED"));
        }
        serde_json::from_str::<serde_json::Value>(&reflection)
            .map_err(|_| malformed("HERMES_PRODUCTION_FIXTURE_REJECTED"))?;
        Self::validated(
            containment,
            runtime_manifest,
            broker_receipt_digest.as_str().to_owned(),
            expected_request,
            api_key.into(),
            model.into(),
            startup_timeout,
            operation_timeout,
            poll_interval,
            RunnerMode::ScriptedFixture(reflection),
        )
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn official_with_broker_digest(
        containment: HermesWslContainmentConfig,
        runtime_manifest: &HermesOfflineRuntimeManifest,
        broker_receipt_digest: &ContentDigest,
        expected_request: HermesResearchRequest,
        api_key: impl Into<String>,
        model: impl Into<String>,
        startup_timeout: Duration,
        operation_timeout: Duration,
        poll_interval: Duration,
    ) -> HermesAdapterResult<Self> {
        Self::validated(
            containment,
            runtime_manifest,
            broker_receipt_digest.as_str().to_owned(),
            expected_request,
            api_key.into(),
            model.into(),
            startup_timeout,
            operation_timeout,
            poll_interval,
            RunnerMode::Official,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn validated(
        containment: HermesWslContainmentConfig,
        runtime_manifest: &HermesOfflineRuntimeManifest,
        broker_receipt_sha256: String,
        expected_request: HermesResearchRequest,
        api_key: String,
        model: String,
        startup_timeout: Duration,
        operation_timeout: Duration,
        poll_interval: Duration,
        mode: RunnerMode,
    ) -> HermesAdapterResult<Self> {
        if startup_timeout.is_zero() || startup_timeout > MAX_RUNNER_TIMEOUT {
            return Err(error(
                HermesAdapterErrorKind::Configuration,
                "HERMES_PRODUCTION_STARTUP_TIMEOUT_REJECTED",
            ));
        }
        HermesAdapterConfig::new(
            "127.0.0.1:1".parse().expect("fixed loopback endpoint"),
            api_key.clone(),
            operation_timeout,
            poll_interval,
        )?;
        let manifest_bytes = serde_json::to_vec(runtime_manifest)
            .map_err(|_| malformed("HERMES_RUNTIME_MANIFEST_CANONICALIZATION_FAILED"))?;
        let runtime_manifest_sha256 = encode_sha256(&Sha256::digest(&manifest_bytes));
        Ok(Self {
            containment,
            expected_request,
            runtime_manifest_sha256,
            broker_receipt_sha256,
            api_key,
            model,
            startup_timeout,
            operation_timeout,
            poll_interval,
            mode,
        })
    }

    /// Starts the pinned WSL/bubblewrap child and privately installs the
    /// resulting receipt into the adapter owned by the returned port.
    ///
    /// # Errors
    ///
    /// Fails closed on deadline, path, launcher, runtime, broker, socketpair,
    /// endpoint, PID, containment-frame, child-liveness, or startup ambiguity.
    pub fn launch(self, absolute_deadline: Instant) -> HermesAdapterResult<ProductionHermesRunner> {
        self.launch_inner(absolute_deadline)
    }

    #[allow(clippy::too_many_lines)]
    fn launch_inner(
        self,
        absolute_deadline: Instant,
    ) -> HermesAdapterResult<ProductionHermesRunner> {
        if absolute_deadline <= Instant::now() {
            return Err(error(
                HermesAdapterErrorKind::Configuration,
                "HERMES_PRODUCTION_LAUNCH_BINDING_REJECTED",
            ));
        }
        if self.containment.isolation_root().exists()
            || self
                .containment
                .isolation_root()
                .starts_with(self.containment.product_root())
            || self
                .containment
                .product_root()
                .starts_with(self.containment.isolation_root())
        {
            return Err(error(
                HermesAdapterErrorKind::Configuration,
                "HERMES_PRODUCTION_ROOT_REJECTED",
            ));
        }
        fs::create_dir(self.containment.isolation_root()).map_err(|_| {
            error(
                HermesAdapterErrorKind::Spawn,
                "HERMES_PRODUCTION_ROOT_CREATE_FAILED",
            )
        })?;
        let capture_root = self.containment.isolation_root().join("capture");
        fs::create_dir(&capture_root).map_err(|_| {
            error(
                HermesAdapterErrorKind::Spawn,
                "HERMES_PRODUCTION_CAPTURE_CREATE_FAILED",
            )
        })?;
        let nonce = production_nonce(self.containment.isolation_root())?;
        let remaining = absolute_deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| {
                error(
                    HermesAdapterErrorKind::Timeout,
                    "HERMES_PRODUCTION_DEADLINE_EXCEEDED",
                )
            })?;
        let deadline_millis = u64::try_from(remaining.as_millis())
            .unwrap_or(u64::MAX)
            .min(300_000);
        if deadline_millis == 0 {
            return Err(error(
                HermesAdapterErrorKind::Timeout,
                "HERMES_PRODUCTION_DEADLINE_EXCEEDED",
            ));
        }
        let secret_path = self.containment.isolation_root().join("launch-secret.json");
        let runner_path = self.containment.isolation_root().join("inner-runner.py");
        let secret = LaunchSecret {
            api_key: &self.api_key,
            broker_receipt_sha256: &self.broker_receipt_sha256,
            config_sha256: "0000000000000000000000000000000000000000000000000000000000000000",
            deadline_millis,
            endpoint: "127.0.0.1:0",
            fixture_reflection: self.mode.fixture_reflection(),
            mode: self.mode.as_str(),
            model: &self.model,
            nonce: &nonce,
            runtime_manifest_sha256: &self.runtime_manifest_sha256,
        };
        let secret_bytes = serde_json::to_vec(&secret)
            .map_err(|_| malformed("HERMES_PRODUCTION_SECRET_REJECTED"))?;
        write_new_secret(&secret_path, &secret_bytes)?;
        if let Err(failure) = write_new_runner(&runner_path, PRIVATE_RUNNER_SOURCE.as_bytes()) {
            remove_ingress(&secret_path);
            return Err(failure);
        }
        let secret_guest_path = windows_path_to_wsl(&secret_path)?;
        let runner_guest_path = windows_path_to_wsl(&runner_path)?;
        let runner_sha256 = sha256_text(PRIVATE_RUNNER_SOURCE);
        let interpreter = format!(
            "{}/python/bin/python3.12",
            self.containment.runtime_guest_root()
        );
        let arguments = [
            OsString::from("-d"),
            OsString::from(WSL_DISTRO),
            OsString::from("--exec"),
            OsString::from(interpreter),
            OsString::from("-I"),
            OsString::from("-S"),
            OsString::from("-B"),
            OsString::from("-c"),
            OsString::from(OUTER_RUNNER_SOURCE),
            OsString::from("production"),
            OsString::from(self.containment.runtime_guest_root()),
            OsString::from(&nonce),
            OsString::from(secret_guest_path),
            OsString::from(runner_guest_path),
            OsString::from(runner_sha256),
        ]
        .into_iter()
        .collect::<Vec<_>>();
        let plan = crate::windows_job::WindowsJobCommandPlan {
            executable: self.containment.wsl_executable().to_path_buf(),
            arguments,
            current_dir: self.containment.isolation_root().to_path_buf(),
            environment: minimal_wsl_environment(self.containment.wsl_executable())?,
            run_root: self.containment.isolation_root().to_path_buf(),
            stdout_path: capture_root.join("production.stdout"),
            stderr_path: capture_root.join("production.stderr"),
            stdout_limit: MAX_STARTUP_BYTES as u64,
            stderr_limit: 4096,
            deadline: absolute_deadline,
            teardown_timeout: Duration::from_secs(3),
        };
        let mut process = match crate::windows_job::spawn(&plan) {
            Ok(process) => process,
            Err(failure) => {
                remove_ingress(&secret_path);
                remove_ingress(&runner_path);
                return Err(failure);
            }
        };
        let startup_deadline = Instant::now()
            .checked_add(self.startup_timeout)
            .map_or(absolute_deadline, |candidate| {
                candidate.min(absolute_deadline)
            });
        let startup = match wait_for_startup(&mut process, startup_deadline) {
            Ok(startup) => startup,
            Err(failure) => {
                let mapped = map_outer_failure(&process, failure);
                let _ = process.terminate();
                remove_ingress(&secret_path);
                remove_ingress(&runner_path);
                return Err(mapped);
            }
        };
        if secret_path.exists() || runner_path.exists() {
            let _ = process.terminate();
            remove_ingress(&secret_path);
            remove_ingress(&runner_path);
            return Err(error(
                HermesAdapterErrorKind::Ambiguous,
                "HERMES_PRODUCTION_INGRESS_NOT_CONSUMED",
            ));
        }
        let attestation = verify_startup(
            &startup,
            &self.runtime_manifest_sha256,
            &self.broker_receipt_sha256,
            &self.api_key,
            &self.model,
            &nonce,
            self.mode.as_str(),
        )?;
        process.ensure_running()?;
        let runner_nonce_sha256 = attestation.runner_nonce_sha256.clone();
        let owner = Arc::new(ContainmentOwnerState::new(runner_nonce_sha256.clone()));
        let receipt = mint_receipt(
            &attestation,
            process.process_id(),
            runner_nonce_sha256,
            Arc::downgrade(&owner),
        )?;
        Ok(ProductionHermesRunner {
            endpoint: attestation.endpoint,
            api_key: self.api_key,
            model: self.model,
            expected_request: self.expected_request,
            receipt,
            process,
            owner,
            absolute_deadline,
            operation_timeout: self.operation_timeout,
            poll_interval: self.poll_interval,
            windows_launcher_pid: startup.windows_launcher_pid,
            outer_pid: startup.wire.outer_pid,
            bwrap_pid: startup.wire.bwrap_pid,
        })
    }
}

/// Live contained Hermes process that exists before any Codex effect and can
/// be bound exactly once to the resulting immutable reflection job.
pub struct ProductionHermesRunner {
    endpoint: SocketAddr,
    api_key: String,
    model: String,
    expected_request: HermesResearchRequest,
    receipt: HermesContainmentReceipt,
    process: crate::windows_job::WindowsJobChild,
    owner: Arc<ContainmentOwnerState>,
    absolute_deadline: Instant,
    operation_timeout: Duration,
    poll_interval: Duration,
    windows_launcher_pid: u32,
    outer_pid: u32,
    bwrap_pid: u32,
}

impl ProductionHermesRunner {
    /// Consumes the sole runner owner and binds it once to the completed job.
    ///
    /// # Errors
    ///
    /// Rejects a mismatched model, expired/dead child, or receipt binding before
    /// adapter construction. Any error drops and reaps the owned process tree.
    pub fn bind(mut self, job: HermesReflectionJob) -> HermesAdapterResult<ProductionHermesPort> {
        if job.model() != self.model || job.request() != &self.expected_request {
            return Err(error(
                HermesAdapterErrorKind::CrossBinding,
                "HERMES_PRODUCTION_JOB_BINDING_REJECTED",
            ));
        }
        self.process.ensure_running()?;
        self.receipt.verify_binding(self.endpoint, &self.api_key)?;
        let remaining = self
            .absolute_deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| {
                error(
                    HermesAdapterErrorKind::Timeout,
                    "HERMES_PRODUCTION_DEADLINE_EXCEEDED",
                )
            })?;
        let timeout = self.operation_timeout.min(remaining);
        let mut adapter_config = HermesAdapterConfig::new(
            self.endpoint,
            self.api_key,
            timeout,
            self.poll_interval.min(timeout),
        )?;
        adapter_config.install_containment_receipt(self.receipt.clone())?;
        let adapter = HermesReflectionAdapter::connect(adapter_config, job)?;
        Ok(ProductionHermesPort {
            adapter,
            receipt: self.receipt,
            process: self.process,
            owner: self.owner,
            absolute_deadline: self.absolute_deadline,
            operation_timeout: self.operation_timeout,
            poll_interval: self.poll_interval,
            windows_launcher_pid: self.windows_launcher_pid,
            outer_pid: self.outer_pid,
            bwrap_pid: self.bwrap_pid,
        })
    }

    /// Returns the sealed attestation needed by the full-chain orchestrator.
    #[must_use]
    pub const fn containment_receipt(&self) -> &HermesContainmentReceipt {
        &self.receipt
    }

    /// Proves that the same owned Job process tree remains alive.
    ///
    /// # Errors
    ///
    /// Fails closed on deadline, process exit, Job ambiguity, or receipt replay.
    pub fn verify_live(&mut self) -> HermesAdapterResult<()> {
        self.process.ensure_running()?;
        self.receipt.verify_binding(self.endpoint, &self.api_key)
    }

    /// Explicitly invalidates the receipt and reaps the owned WSL tree.
    ///
    /// # Errors
    ///
    /// Reports teardown ambiguity if the Job cannot prove all descendants exit.
    pub fn terminate(mut self) -> HermesAdapterResult<()> {
        self.owner.invalidate();
        self.process.terminate()
    }

    #[must_use]
    pub const fn windows_launcher_pid(&self) -> u32 {
        self.windows_launcher_pid
    }

    #[must_use]
    pub const fn outer_pid(&self) -> u32 {
        self.outer_pid
    }

    #[must_use]
    pub const fn bwrap_pid(&self) -> u32 {
        self.bwrap_pid
    }
}

/// Production-only Hermes port whose adapter and contained child share one
/// unforgeable owner capability.
pub struct ProductionHermesPort {
    adapter: HermesReflectionAdapter,
    receipt: HermesContainmentReceipt,
    process: crate::windows_job::WindowsJobChild,
    owner: Arc<ContainmentOwnerState>,
    absolute_deadline: Instant,
    operation_timeout: Duration,
    poll_interval: Duration,
    windows_launcher_pid: u32,
    outer_pid: u32,
    bwrap_pid: u32,
}

impl ProductionHermesPort {
    /// Runs one reflection while the exact contained namespace process remains
    /// alive and returns canonical payload plus normalized evidence.
    ///
    /// # Errors
    ///
    /// Fails before endpoint I/O on child death, deadline, binding, or replay,
    /// and discards any result if the owned child dies before return.
    pub fn run_reflection_evidence(
        &mut self,
        request: &HermesResearchRequest,
    ) -> PortResult<HermesReflectionEvidence> {
        self.prepare_operation()?;
        let result = self.adapter.run_reflection_evidence(request);
        self.ensure_live()?;
        result
    }

    /// Reconciles one already-submitted run without changing the owner or
    /// recomputing normalized evidence.
    ///
    /// # Errors
    ///
    /// Preserves owner, deadline, recovery, and adapter failures.
    pub fn reconcile_reflection(
        &mut self,
        request: &HermesResearchRequest,
        receipt: &crate::HermesRunRecoveryReceipt,
    ) -> PortResult<CanonicalReflection> {
        self.prepare_operation()?;
        let result = self
            .adapter
            .reconcile_reflection(request, receipt)
            .map_err(|failure| map_port_error(&failure));
        self.ensure_live()?;
        result
    }

    #[must_use]
    pub fn containment_receipt(&self) -> &HermesContainmentReceipt {
        &self.receipt
    }

    #[must_use]
    pub const fn windows_launcher_pid(&self) -> u32 {
        self.windows_launcher_pid
    }

    #[must_use]
    pub const fn outer_pid(&self) -> u32 {
        self.outer_pid
    }

    #[must_use]
    pub const fn bwrap_pid(&self) -> u32 {
        self.bwrap_pid
    }

    /// Explicitly invalidates the receipt and reaps the owned WSL tree.
    ///
    /// # Errors
    ///
    /// Reports teardown ambiguity if the Job cannot prove all descendants
    /// exited. Drop retains kill-on-close as a final backstop.
    pub fn terminate(mut self) -> HermesAdapterResult<()> {
        self.owner.invalidate();
        self.process.terminate()
    }

    fn prepare_operation(&mut self) -> PortResult<()> {
        self.ensure_live()?;
        let remaining = self
            .absolute_deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| {
                PortError::new(
                    lattice_contracts::Component::Hermes,
                    lattice_ports::PortErrorKind::Timeout,
                    "HERMES_PRODUCTION_DEADLINE_EXCEEDED",
                )
            })?;
        self.adapter.config.timeout = self.operation_timeout.min(remaining);
        self.adapter.config.poll_interval = self.poll_interval.min(self.adapter.config.timeout);
        Ok(())
    }

    fn ensure_live(&mut self) -> PortResult<()> {
        if let Err(failure) = self.process.ensure_running() {
            self.owner.invalidate();
            return Err(map_port_error(&failure));
        }
        self.adapter
            .require_containment_receipt()
            .map(|_| ())
            .map_err(|failure| map_port_error(&failure))
    }

    #[cfg(test)]
    pub(crate) fn terminate_child_for_test(&mut self) -> HermesAdapterResult<()> {
        self.process.terminate()
    }
}

impl HermesPort for ProductionHermesPort {
    fn research(&mut self, request: HermesResearchRequest) -> PortResult<HermesEvidence> {
        self.run_reflection_evidence(&request)
            .map(HermesReflectionEvidence::into_evidence)
    }

    fn interrupt(&mut self, request_id: &RequestId) -> PortResult<()> {
        self.prepare_operation()?;
        let result = HermesPort::interrupt(&mut self.adapter, request_id);
        self.ensure_live()?;
        result
    }
}

impl Drop for ProductionHermesPort {
    fn drop(&mut self) {
        self.owner.invalidate();
        let _ = self.process.terminate();
    }
}

#[derive(Serialize)]
struct LaunchSecret<'a> {
    api_key: &'a str,
    broker_receipt_sha256: &'a str,
    config_sha256: &'a str,
    deadline_millis: u64,
    endpoint: &'a str,
    fixture_reflection: Option<&'a str>,
    mode: &'a str,
    model: &'a str,
    nonce: &'a str,
    runtime_manifest_sha256: &'a str,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StartupWire {
    bwrap_pid: u32,
    containment_frame_hex: String,
    containment_frame_sha256: String,
    outer_pid: u32,
    schema: String,
}

struct StartupObservation {
    wire: StartupWire,
    frame: Vec<u8>,
    windows_launcher_pid: u32,
}

struct VerifiedAttestation {
    endpoint: SocketAddr,
    namespace_pid: u32,
    runtime_manifest_sha256: String,
    broker_receipt_sha256: String,
    api_key_sha256: String,
    runner_nonce_sha256: String,
    bwrap_sha256: String,
    socketpair_binding_sha256: String,
    containment_frame_sha256: String,
    outer_pid: u32,
    bwrap_pid: u32,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ContainmentAttestationWire {
    api_key_sha256: String,
    endpoint: String,
    mode: String,
    namespace_pid: u32,
    net_namespace: String,
    nonce_sha256: String,
    schema: String,
}

fn wait_for_startup(
    process: &mut crate::windows_job::WindowsJobChild,
    deadline: Instant,
) -> HermesAdapterResult<StartupObservation> {
    loop {
        let bytes = process.read_stdout(MAX_STARTUP_BYTES as u64)?;
        if let Some((wire, frame)) = parse_startup(&bytes)? {
            return Ok(StartupObservation {
                wire,
                frame,
                windows_launcher_pid: process.process_id(),
            });
        }
        process.ensure_running()?;
        if Instant::now() >= deadline {
            return Err(error(
                HermesAdapterErrorKind::Timeout,
                "HERMES_PRODUCTION_STARTUP_TIMEOUT",
            ));
        }
        thread::sleep(Duration::from_millis(2));
    }
}

fn parse_startup(bytes: &[u8]) -> HermesAdapterResult<Option<(StartupWire, Vec<u8>)>> {
    if bytes.len() < STARTUP_MAGIC.len() {
        if STARTUP_MAGIC.starts_with(bytes) {
            return Ok(None);
        }
        return Err(malformed("HERMES_PRODUCTION_STARTUP_MAGIC_REJECTED"));
    }
    if !bytes.starts_with(STARTUP_MAGIC) {
        return Err(malformed("HERMES_PRODUCTION_STARTUP_MAGIC_REJECTED"));
    }
    if bytes.len() < STARTUP_MAGIC.len() + 8 {
        return Ok(None);
    }
    let offset = STARTUP_MAGIC.len();
    let length = u64::from_be_bytes(
        bytes[offset..offset + 8]
            .try_into()
            .expect("fixed eight-byte slice"),
    );
    let length = usize::try_from(length)
        .map_err(|_| malformed("HERMES_PRODUCTION_STARTUP_LENGTH_REJECTED"))?;
    if length == 0 || length > MAX_STARTUP_BYTES {
        return Err(malformed("HERMES_PRODUCTION_STARTUP_LENGTH_REJECTED"));
    }
    let total = STARTUP_MAGIC
        .len()
        .checked_add(8)
        .and_then(|value| value.checked_add(length))
        .ok_or_else(|| malformed("HERMES_PRODUCTION_STARTUP_LENGTH_REJECTED"))?;
    if bytes.len() < total {
        return Ok(None);
    }
    if bytes.len() != total {
        return Err(malformed("HERMES_PRODUCTION_STARTUP_TRAILING_BYTES"));
    }
    let encoded = &bytes[offset + 8..];
    let wire: StartupWire = serde_json::from_slice(encoded)
        .map_err(|_| malformed("HERMES_PRODUCTION_STARTUP_MALFORMED"))?;
    if serde_json::to_vec(&wire).map_err(|_| malformed("HERMES_PRODUCTION_STARTUP_MALFORMED"))?
        != encoded
        || wire.schema != STARTUP_SCHEMA
        || wire.outer_pid == 0
        || wire.bwrap_pid == 0
    {
        return Err(malformed("HERMES_PRODUCTION_STARTUP_MALFORMED"));
    }
    let frame = decode_hex(&wire.containment_frame_hex)?;
    if encode_sha256(&Sha256::digest(&frame)) != wire.containment_frame_sha256 {
        return Err(cross_binding(
            "HERMES_PRODUCTION_CONTAINMENT_FRAME_DIGEST_REJECTED",
        ));
    }
    Ok(Some((wire, frame)))
}

#[allow(clippy::too_many_arguments)]
fn verify_startup(
    startup: &StartupObservation,
    runtime_manifest_sha256: &str,
    broker_receipt_sha256: &str,
    api_key: &str,
    model: &str,
    nonce: &str,
    mode: &str,
) -> HermesAdapterResult<VerifiedAttestation> {
    let frame =
        parse_containment_frame(&startup.frame, HermesContainmentFrameLimits::new(16 * 1024))?;
    let api_key_sha256 = sha256_text(api_key);
    let nonce_bytes = decode_hex(nonce)?;
    let runner_nonce_sha256 = encode_sha256(&Sha256::digest(&nonce_bytes));
    let socketpair_binding_sha256 =
        digest_join(&[&nonce_bytes, b"LATTICE_HERMES_PRODUCTION_SOCKETPAIR_V1"]);
    let request_sha256 = digest_join(&[&nonce_bytes, b"LATTICE_HERMES_PRODUCTION_REQUEST_V1"]);
    let transcript_sha256 = digest_join(&[&nonce_bytes, b"LATTICE_HERMES_PRODUCTION_READY_V1"]);
    let config_sha256 = production_config_sha256(frame.endpoint(), &api_key_sha256, model, nonce)?;
    if frame.runtime_manifest_sha256() != runtime_manifest_sha256.as_bytes()
        || frame.config_sha256() != config_sha256.as_bytes()
        || frame.request_sha256() != request_sha256.as_bytes()
        || frame.broker_receipt_sha256() != broker_receipt_sha256.as_bytes()
        || frame.bwrap_sha256() != BWRAP_SHA256.as_bytes()
        || frame.socketpair_binding_sha256() != socketpair_binding_sha256.as_bytes()
        || frame.api_key_sha256() != api_key_sha256.as_bytes()
        || frame.nonce_sha256() != runner_nonce_sha256.as_bytes()
        || frame.transcript_sha256() != transcript_sha256.as_bytes()
        || frame.mode() != mode
    {
        return Err(cross_binding("HERMES_PRODUCTION_FRAME_BINDING_REJECTED"));
    }
    let metadata: ContainmentAttestationWire = serde_json::from_slice(frame.reflection())
        .map_err(|_| malformed("HERMES_PRODUCTION_ATTESTATION_REJECTED"))?;
    if serde_json::to_vec(&metadata)
        .map_err(|_| malformed("HERMES_PRODUCTION_ATTESTATION_REJECTED"))?
        != frame.reflection()
        || metadata.schema != ATTESTATION_SCHEMA
        || metadata.endpoint != frame.endpoint().to_string()
        || metadata.mode != mode
        || metadata.namespace_pid != frame.namespace_pid()
        || metadata.api_key_sha256 != api_key_sha256
        || metadata.nonce_sha256 != runner_nonce_sha256
        || !metadata.net_namespace.starts_with("net:[")
        || !metadata.net_namespace.ends_with(']')
    {
        return Err(cross_binding("HERMES_PRODUCTION_ATTESTATION_REJECTED"));
    }
    Ok(VerifiedAttestation {
        endpoint: frame.endpoint(),
        namespace_pid: frame.namespace_pid(),
        runtime_manifest_sha256: runtime_manifest_sha256.to_owned(),
        broker_receipt_sha256: broker_receipt_sha256.to_owned(),
        api_key_sha256,
        runner_nonce_sha256,
        bwrap_sha256: BWRAP_SHA256.to_owned(),
        socketpair_binding_sha256,
        containment_frame_sha256: startup.wire.containment_frame_sha256.clone(),
        outer_pid: startup.wire.outer_pid,
        bwrap_pid: startup.wire.bwrap_pid,
    })
}

fn mint_receipt(
    attestation: &VerifiedAttestation,
    windows_launcher_pid: u32,
    runner_nonce_sha256: String,
    owner: std::sync::Weak<ContainmentOwnerState>,
) -> HermesAdapterResult<HermesContainmentReceipt> {
    if windows_launcher_pid == 0
        || attestation.outer_pid == 0
        || attestation.bwrap_pid == 0
        || attestation.namespace_pid == 0
        || runner_nonce_sha256 != attestation.runner_nonce_sha256
    {
        return Err(cross_binding("HERMES_PRODUCTION_PID_BINDING_REJECTED"));
    }
    let mut digest = Sha256::new();
    digest.update(b"lattice.hermes.production-containment-receipt.v1\0");
    for field in [
        windows_launcher_pid.to_string(),
        attestation.outer_pid.to_string(),
        attestation.bwrap_pid.to_string(),
        attestation.namespace_pid.to_string(),
        attestation.endpoint.to_string(),
        attestation.api_key_sha256.clone(),
        runner_nonce_sha256.clone(),
        attestation.runtime_manifest_sha256.clone(),
        attestation.bwrap_sha256.clone(),
        attestation.socketpair_binding_sha256.clone(),
        attestation.broker_receipt_sha256.clone(),
        attestation.containment_frame_sha256.clone(),
    ] {
        digest.update((field.len() as u64).to_be_bytes());
        digest.update(field.as_bytes());
    }
    let receipt_digest = ContentDigest::from_sha256(encode_sha256(&digest.finalize()))
        .map_err(|_| malformed("HERMES_PRODUCTION_RECEIPT_REJECTED"))?;
    Ok(HermesContainmentReceipt {
        endpoint: attestation.endpoint,
        api_key_sha256: attestation.api_key_sha256.clone(),
        runner_nonce_sha256,
        contained_pid: attestation.namespace_pid,
        runtime_manifest_sha256: attestation.runtime_manifest_sha256.clone(),
        bwrap_sha256: attestation.bwrap_sha256.clone(),
        socketpair_binding_sha256: attestation.socketpair_binding_sha256.clone(),
        broker_receipt_sha256: attestation.broker_receipt_sha256.clone(),
        containment_frame_sha256: attestation.containment_frame_sha256.clone(),
        receipt_digest,
        owner: Some(owner),
    })
}

fn production_config_sha256(
    endpoint: SocketAddr,
    api_key_sha256: &str,
    model: &str,
    nonce: &str,
) -> HermesAdapterResult<String> {
    #[derive(Serialize)]
    struct ConfigWire<'a> {
        api_key_sha256: &'a str,
        endpoint: String,
        model: &'a str,
        nonce: &'a str,
        schema: &'a str,
    }
    let bytes = serde_json::to_vec(&ConfigWire {
        api_key_sha256,
        endpoint: endpoint.to_string(),
        model,
        nonce,
        schema: CONFIG_SCHEMA,
    })
    .map_err(|_| malformed("HERMES_PRODUCTION_CONFIG_DIGEST_REJECTED"))?;
    Ok(encode_sha256(&Sha256::digest(bytes)))
}

fn map_outer_failure(
    process: &crate::windows_job::WindowsJobChild,
    fallback: HermesAdapterError,
) -> HermesAdapterError {
    let Ok(stderr) = process.read_stderr(4096) else {
        return fallback;
    };
    let Ok(stderr) = std::str::from_utf8(&stderr) else {
        return fallback;
    };
    let Some(code) = stderr
        .lines()
        .rev()
        .find_map(|line| line.strip_prefix("HERMES_OUTER_FAIL:"))
        .and_then(|value| value.parse::<u32>().ok())
    else {
        return fallback;
    };
    match code {
        64 => error(
            HermesAdapterErrorKind::Configuration,
            "HERMES_PRODUCTION_ARGUMENT_REJECTED",
        ),
        65 | 66 => error(
            HermesAdapterErrorKind::Identity,
            "HERMES_PRODUCTION_RUNTIME_IDENTITY_REJECTED",
        ),
        67..=73 | 76..=78 => error(
            HermesAdapterErrorKind::CrossBinding,
            "HERMES_PRODUCTION_CONTAINMENT_PROTOCOL_REJECTED",
        ),
        74 => error(
            HermesAdapterErrorKind::CapabilityMismatch,
            "HERMES_OFFICIAL_SERVER_NOT_STAGED",
        ),
        75 => error(
            HermesAdapterErrorKind::CapabilityMismatch,
            "HERMES_OFFICIAL_SERVER_STARTUP_BLOCKED",
        ),
        79 => error(
            HermesAdapterErrorKind::Timeout,
            "HERMES_PRODUCTION_DEADLINE_EXCEEDED",
        ),
        _ => fallback,
    }
}

fn production_nonce(isolation_root: &Path) -> HermesAdapterResult<String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| malformed("HERMES_PRODUCTION_NONCE_CLOCK_REJECTED"))?;
    let sequence = RUNNER_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let mut digest = Sha256::new();
    digest.update(b"lattice.hermes.production.nonce.v1\0");
    digest.update(std::process::id().to_be_bytes());
    digest.update(sequence.to_be_bytes());
    digest.update(now.as_nanos().to_be_bytes());
    digest.update(isolation_root.as_os_str().to_string_lossy().as_bytes());
    Ok(encode_sha256(&digest.finalize()))
}

fn write_new_secret(path: &Path, bytes: &[u8]) -> HermesAdapterResult<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| {
            error(
                HermesAdapterErrorKind::Spawn,
                "HERMES_PRODUCTION_SECRET_CREATE_FAILED",
            )
        })?;
    file.write_all(bytes).map_err(|_| {
        error(
            HermesAdapterErrorKind::Spawn,
            "HERMES_PRODUCTION_SECRET_WRITE_FAILED",
        )
    })?;
    file.sync_all().map_err(|_| {
        error(
            HermesAdapterErrorKind::Spawn,
            "HERMES_PRODUCTION_SECRET_WRITE_FAILED",
        )
    })
}

fn write_new_runner(path: &Path, bytes: &[u8]) -> HermesAdapterResult<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| {
            error(
                HermesAdapterErrorKind::Spawn,
                "HERMES_PRODUCTION_RUNNER_CREATE_FAILED",
            )
        })?;
    file.write_all(bytes).map_err(|_| {
        error(
            HermesAdapterErrorKind::Spawn,
            "HERMES_PRODUCTION_RUNNER_WRITE_FAILED",
        )
    })?;
    file.sync_all().map_err(|_| {
        error(
            HermesAdapterErrorKind::Spawn,
            "HERMES_PRODUCTION_RUNNER_WRITE_FAILED",
        )
    })
}

fn remove_ingress(path: &Path) {
    drop(fs::remove_file(path));
}

fn windows_path_to_wsl(path: &Path) -> HermesAdapterResult<String> {
    let text = path.as_os_str().to_string_lossy();
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    let bytes = text.as_bytes();
    if bytes.len() < 3
        || !bytes[0].is_ascii_alphabetic()
        || bytes[1] != b':'
        || !matches!(bytes[2], b'\\' | b'/')
        || text.starts_with(r"\\")
    {
        return Err(error(
            HermesAdapterErrorKind::Configuration,
            "HERMES_PRODUCTION_WSL_PATH_REJECTED",
        ));
    }
    let drive = char::from(bytes[0].to_ascii_lowercase());
    let suffix = text[3..].replace('\\', "/");
    if suffix.split('/').any(|part| part == "..") {
        return Err(error(
            HermesAdapterErrorKind::Configuration,
            "HERMES_PRODUCTION_WSL_PATH_REJECTED",
        ));
    }
    Ok(format!("/mnt/{drive}/{suffix}"))
}

fn decode_hex(value: &str) -> HermesAdapterResult<Vec<u8>> {
    if value.is_empty() || !value.len().is_multiple_of(2) {
        return Err(malformed("HERMES_PRODUCTION_HEX_REJECTED"));
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair)
                .map_err(|_| malformed("HERMES_PRODUCTION_HEX_REJECTED"))?;
            u8::from_str_radix(pair, 16).map_err(|_| malformed("HERMES_PRODUCTION_HEX_REJECTED"))
        })
        .collect()
}

fn digest_join(parts: &[&[u8]]) -> String {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update(part);
    }
    encode_sha256(&digest.finalize())
}
