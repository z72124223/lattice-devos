use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

#[cfg(not(windows))]
use std::fs::OpenOptions;
#[cfg(not(windows))]
use std::process::{Child, Command, Stdio};

use lattice_contracts::GRAPHIFY_VERSION;

use crate::error::{GraphifyAdapterError, GraphifyAdapterErrorKind, GraphifyAdapterResult};
use crate::graph::{GraphParseLimits, NormalizedGraph, parse_graph};
use crate::identity::{
    GRAPHIFY_PRIVATE_RUNNER_SHA256, GRAPHIFY_WSL_BWRAP_HELP_SHA256, GRAPHIFY_WSL_BWRAP_PATH,
    GRAPHIFY_WSL_BWRAP_SHA256, GRAPHIFY_WSL_BWRAP_VERSION_SHA256, GRAPHIFY_WSL_DISTRO,
    GRAPHIFY_WSL_EXECUTION_IDENTITY_SHA256, GRAPHIFY_WSL_GRAPHIFY_HELP_SHA256,
    GRAPHIFY_WSL_GRAPHIFY_VERSION_SHA256, GRAPHIFY_WSL_INSTALL_REPORT_SHA256,
    GRAPHIFY_WSL_OS_RELEASE_SHA256, GRAPHIFY_WSL_PYTHON_PATH, GRAPHIFY_WSL_PYTHON_SHA256,
    GRAPHIFY_WSL_PYTHON_VERSION_SHA256, GRAPHIFY_WSL_RUNTIME_BYTE_COUNT,
    GRAPHIFY_WSL_RUNTIME_FILE_COUNT, GRAPHIFY_WSL_RUNTIME_MANIFEST_SHA256, verify_reviewed_runtime,
};
use crate::snapshot::{
    MaterializedSnapshot, SnapshotBridge, file_sha256, framed_digest, verify_snapshot_binding,
};

static RUN_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static PROCESS_SEQUENCE: AtomicU64 = AtomicU64::new(1);

const REQUIRED_HELP_FRAGMENTS: &[&str] = &[
    "extract <path>",
    "--code-only",
    "--no-cluster",
    "--max-workers N",
    "--out DIR",
];

const WSL_SHA256SUM_PATH: &str = "/usr/bin/sha256sum";
const PRIVATE_RUNNER_SOURCE: &str = include_str!("sandbox_runner.py");
const PRIVATE_FRAME_MAGIC: &[u8] = b"LATTICE_GRAPHIFY_PRIVATE_V1\n";
const PRIVATE_FRAME_FIELD_COUNT: usize = 7;

/// Strict raw-output and diagnostic limits for one Graphify run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
// The repeated prefix makes each process/output bound explicit to callers.
#[allow(clippy::struct_field_names)]
pub struct GraphOutputLimits {
    max_graph_bytes: u64,
    max_nodes: usize,
    max_edges: usize,
    max_text_bytes: usize,
    max_diagnostic_bytes: u64,
}

impl GraphOutputLimits {
    #[must_use]
    pub const fn new(
        max_graph_bytes: u64,
        max_nodes: usize,
        max_edges: usize,
        max_text_bytes: usize,
        max_diagnostic_bytes: u64,
    ) -> Self {
        Self {
            max_graph_bytes,
            max_nodes,
            max_edges,
            max_text_bytes,
            max_diagnostic_bytes,
        }
    }

    #[must_use]
    pub const fn max_graph_bytes(self) -> u64 {
        self.max_graph_bytes
    }

    #[must_use]
    pub const fn max_nodes(self) -> usize {
        self.max_nodes
    }

    #[must_use]
    pub const fn max_edges(self) -> usize {
        self.max_edges
    }

    #[must_use]
    pub const fn max_text_bytes(self) -> usize {
        self.max_text_bytes
    }

    #[must_use]
    pub const fn max_diagnostic_bytes(self) -> u64 {
        self.max_diagnostic_bytes
    }
}

impl Default for GraphOutputLimits {
    fn default() -> Self {
        Self::new(64 * 1024 * 1024, 100_000, 250_000, 1_024, 1024 * 1024)
    }
}

/// Process-owned pinned runtime configuration; none of these fields come from
/// MCP or a graph-memory run request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphifyRuntimeConfig {
    wsl_executable: PathBuf,
    runtime_root: PathBuf,
    expected_launcher_sha256: String,
    expected_execution_identity_sha256: String,
    expected_help_sha256: String,
    expected_payload_manifest_sha256: Option<String>,
    staging_root: PathBuf,
    timeout: Duration,
    limits: GraphOutputLimits,
}

impl GraphifyRuntimeConfig {
    /// Creates a pinned, bounded Graphify runtime configuration.
    ///
    /// # Errors
    ///
    /// Returns an identity error unless the Windows WSL launcher, reviewed
    /// Ubuntu system boundary, and complete LATTICE-owned Python payload match
    /// the Graphify v0.9.33 pin. Also rejects invalid paths or bounds.
    pub fn new(
        wsl_executable: impl Into<PathBuf>,
        runtime_root: impl Into<PathBuf>,
        staging_root: impl Into<PathBuf>,
        timeout: Duration,
        limits: GraphOutputLimits,
    ) -> GraphifyAdapterResult<Self> {
        let wsl_executable = wsl_executable.into();
        let runtime_root = runtime_root.into();
        let staging_root = staging_root.into();
        if !wsl_executable.is_absolute()
            || !runtime_root.is_absolute()
            || !staging_root.is_absolute()
            || timeout.is_zero()
            || timeout > Duration::from_hours(1)
            || limits.max_graph_bytes == 0
            || limits.max_nodes == 0
            || limits.max_edges == 0
            || limits.max_text_bytes == 0
            || limits.max_diagnostic_bytes == 0
        {
            return Err(error(
                GraphifyAdapterErrorKind::Configuration,
                "GRAPHIFY_RUNTIME_CONFIG_REJECTED",
            ));
        }
        let reviewed = verify_reviewed_runtime(&wsl_executable, &runtime_root)?;
        Ok(Self {
            wsl_executable: reviewed.wsl_executable().to_path_buf(),
            runtime_root: reviewed.runtime_root().to_path_buf(),
            expected_launcher_sha256: reviewed.launcher_sha256().to_owned(),
            expected_execution_identity_sha256: reviewed.execution_identity_sha256().to_owned(),
            expected_help_sha256: GRAPHIFY_WSL_GRAPHIFY_HELP_SHA256.to_owned(),
            expected_payload_manifest_sha256: Some(reviewed.manifest_sha256().to_owned()),
            staging_root,
            timeout,
            limits,
        })
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    fn for_test_unverified(
        wsl_executable: impl Into<PathBuf>,
        runtime_root: impl Into<PathBuf>,
        expected_launcher_sha256: impl Into<String>,
        expected_execution_identity_sha256: impl Into<String>,
        expected_help_sha256: impl Into<String>,
        staging_root: impl Into<PathBuf>,
        timeout: Duration,
        limits: GraphOutputLimits,
    ) -> GraphifyAdapterResult<Self> {
        let wsl_executable = wsl_executable.into();
        let runtime_root = runtime_root.into();
        let expected_launcher_sha256 = expected_launcher_sha256.into();
        let expected_execution_identity_sha256 = expected_execution_identity_sha256.into();
        let expected_help_sha256 = expected_help_sha256.into();
        let staging_root = staging_root.into();
        if !wsl_executable.is_absolute()
            || !runtime_root.is_absolute()
            || !staging_root.is_absolute()
            || !is_lowercase_sha256(&expected_launcher_sha256)
            || !is_lowercase_sha256(&expected_execution_identity_sha256)
            || !is_lowercase_sha256(&expected_help_sha256)
            || timeout.is_zero()
        {
            return Err(error(
                GraphifyAdapterErrorKind::Configuration,
                "GRAPHIFY_TEST_RUNTIME_CONFIG_REJECTED",
            ));
        }
        Ok(Self {
            wsl_executable,
            runtime_root,
            expected_launcher_sha256,
            expected_execution_identity_sha256,
            expected_help_sha256,
            expected_payload_manifest_sha256: None,
            staging_root,
            timeout,
            limits,
        })
    }

    #[must_use]
    pub fn wsl_executable(&self) -> &Path {
        &self.wsl_executable
    }

    #[must_use]
    pub fn runtime_root(&self) -> &Path {
        &self.runtime_root
    }

    #[must_use]
    pub fn expected_launcher_sha256(&self) -> &str {
        &self.expected_launcher_sha256
    }

    #[must_use]
    pub fn expected_execution_identity_sha256(&self) -> &str {
        &self.expected_execution_identity_sha256
    }

    #[must_use]
    pub fn expected_help_sha256(&self) -> &str {
        &self.expected_help_sha256
    }

    #[must_use]
    pub fn expected_payload_manifest_sha256(&self) -> Option<&str> {
        self.expected_payload_manifest_sha256.as_deref()
    }

    #[must_use]
    pub fn staging_root(&self) -> &Path {
        &self.staging_root
    }

    #[must_use]
    pub const fn timeout(&self) -> Duration {
        self.timeout
    }

    #[must_use]
    pub const fn limits(&self) -> GraphOutputLimits {
        self.limits
    }

    #[must_use]
    pub fn capability_sha256(&self) -> String {
        let payload_manifest = self
            .expected_payload_manifest_sha256
            .as_deref()
            .unwrap_or("TEST_UNVERIFIED_GRAPHIFY_PAYLOAD");
        let timeout_millis = self.timeout.as_millis().to_string();
        framed_digest(&[
            b"lattice-graphify-adapter-private-copy-1.0",
            b"Ubuntu",
            b"--exec",
            b"/usr/bin/bwrap",
            b"--die-with-parent",
            b"--unshare-all",
            b"--unshare-user",
            b"--disable-userns",
            b"--assert-userns-disabled",
            b"--new-session",
            b"--cap-drop=ALL",
            b"/usr=ro",
            b"/lib=ro",
            b"/lib64=ro",
            b"/runtime-input=ro-ingress",
            b"/source-input=ro-ingress",
            b"/runtime=private-tmpfs-landlock",
            b"/source=private-tmpfs-landlock",
            b"/output=private-tmpfs-landlock",
            b"landlock-abi-minimum=3",
            b"landlock-truncate-probe=runtime-install-report",
            b"capture=same-exclusive-handle",
            b"LATTICE_GRAPHIFY_PRIVATE_V1",
            GRAPHIFY_PRIVATE_RUNNER_SHA256.as_bytes(),
            b"network=unshared",
            b"environment=cleared",
            b"PYTHONDONTWRITEBYTECODE=1",
            b"PYTHONSAFEPATH=1",
            b"/usr/bin/python3.14",
            b"-I",
            b"-S",
            b"-B",
            b"-c=embedded-private-runner",
            b"graphify=version-help-extract-after-copy-verify",
            b"extract",
            b"/source",
            b"--code-only",
            b"--no-cluster",
            b"--max-workers",
            b"1",
            b"--out=/output",
            b"GRAPHIFY_QUERY_LOG_DISABLE=1",
            b"provider-env-cleared",
            self.expected_execution_identity_sha256.as_bytes(),
            payload_manifest.as_bytes(),
            timeout_millis.as_bytes(),
            &self.limits.max_graph_bytes.to_be_bytes(),
            &(self.limits.max_nodes as u64).to_be_bytes(),
            &(self.limits.max_edges as u64).to_be_bytes(),
            &(self.limits.max_text_bytes as u64).to_be_bytes(),
            &self.limits.max_diagnostic_bytes.to_be_bytes(),
        ])
    }
}

/// Complete non-durable analysis returned before typed contract conversion.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphifyAnalysis {
    graph: NormalizedGraph,
    executable_sha256: String,
    help_sha256: String,
    payload_manifest_sha256: Option<String>,
    capability_sha256: String,
    raw_process_sha256: String,
    evidence_sha256: String,
}

impl GraphifyAnalysis {
    #[must_use]
    pub const fn graph(&self) -> &NormalizedGraph {
        &self.graph
    }

    #[must_use]
    pub fn executable_sha256(&self) -> &str {
        &self.executable_sha256
    }

    #[must_use]
    pub fn help_sha256(&self) -> &str {
        &self.help_sha256
    }

    #[must_use]
    pub fn payload_manifest_sha256(&self) -> Option<&str> {
        self.payload_manifest_sha256.as_deref()
    }

    #[must_use]
    pub fn capability_sha256(&self) -> &str {
        &self.capability_sha256
    }

    #[must_use]
    pub fn raw_process_sha256(&self) -> &str {
        &self.raw_process_sha256
    }

    #[must_use]
    pub fn evidence_sha256(&self) -> &str {
        &self.evidence_sha256
    }
}

/// Production Graphify port implementation using one owned child at a time.
pub struct PinnedGraphifyAdapter {
    config: GraphifyRuntimeConfig,
    bridge: SnapshotBridge,
    executor: Box<dyn GraphifyExecutor + Send>,
}

impl std::fmt::Debug for PinnedGraphifyAdapter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PinnedGraphifyAdapter")
            .field("config", &self.config)
            .field("bridge", &self.bridge)
            .finish_non_exhaustive()
    }
}

impl PinnedGraphifyAdapter {
    #[must_use]
    pub fn new(config: GraphifyRuntimeConfig, bridge: SnapshotBridge) -> Self {
        Self {
            config,
            bridge,
            executor: Box::new(OwnedChildExecutor),
        }
    }

    #[must_use]
    pub const fn config(&self) -> &GraphifyRuntimeConfig {
        &self.config
    }

    #[must_use]
    pub const fn bridge(&self) -> &SnapshotBridge {
        &self.bridge
    }

    pub(crate) fn snapshot_for_key(
        &self,
        key: &str,
    ) -> GraphifyAdapterResult<MaterializedSnapshot> {
        self.bridge.get(key)
    }

    // This intentionally remains a linear ownership protocol: preflight,
    // execute, validate, re-bind. Splitting it would obscure teardown order.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn analyze_materialized(
        &mut self,
        snapshot: &MaterializedSnapshot,
    ) -> GraphifyAdapterResult<GraphifyAnalysis> {
        verify_snapshot_binding(snapshot)?;
        fs::create_dir_all(&self.config.staging_root).map_err(|_| {
            error(
                GraphifyAdapterErrorKind::Spawn,
                "GRAPHIFY_STAGING_ROOT_CREATE_FAILED",
            )
        })?;
        let staging_parent = fs::canonicalize(&self.config.staging_root).map_err(|_| {
            error(
                GraphifyAdapterErrorKind::Configuration,
                "GRAPHIFY_STAGING_ROOT_RESOLVE_FAILED",
            )
        })?;
        let snapshot_root = fs::canonicalize(snapshot.root()).map_err(|_| {
            error(
                GraphifyAdapterErrorKind::SnapshotChanged,
                "GRAPHIFY_SNAPSHOT_ROOT_RESOLVE_FAILED",
            )
        })?;
        let sequence = RUN_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let run_root = staging_parent.join(format!(
            "graphify-{}-{}-{sequence}",
            &snapshot.commit_id()[..12],
            std::process::id()
        ));
        fs::create_dir(&run_root).map_err(|_| {
            error(
                GraphifyAdapterErrorKind::Spawn,
                "GRAPHIFY_RUN_STAGING_CREATE_FAILED",
            )
        })?;
        let run_root = fs::canonicalize(&run_root).map_err(|_| {
            error(
                GraphifyAdapterErrorKind::Configuration,
                "GRAPHIFY_RUN_STAGING_RESOLVE_FAILED",
            )
        })?;
        if paths_overlap(&run_root, &snapshot_root)
            || paths_overlap(&run_root, &self.config.runtime_root)
        {
            return Err(error(
                GraphifyAdapterErrorKind::Configuration,
                "GRAPHIFY_SANDBOX_BIND_OVERLAP",
            ));
        }
        let process_root = run_root.join(".lattice-process");
        let sandbox_output = run_root.join("output");
        fs::create_dir(&process_root).map_err(|_| {
            error(
                GraphifyAdapterErrorKind::Spawn,
                "GRAPHIFY_PROCESS_ROOT_CREATE_FAILED",
            )
        })?;
        fs::create_dir(&sandbox_output).map_err(|_| {
            error(
                GraphifyAdapterErrorKind::Spawn,
                "GRAPHIFY_SANDBOX_OUTPUT_CREATE_FAILED",
            )
        })?;
        let deadline = Instant::now()
            .checked_add(self.config.timeout)
            .ok_or_else(|| {
                error(
                    GraphifyAdapterErrorKind::Configuration,
                    "GRAPHIFY_DEADLINE_OVERFLOW",
                )
            })?;

        verify_runtime(&self.config)?;
        if self.config.expected_payload_manifest_sha256.is_some() {
            execute_system_preflight(
                self.executor.as_mut(),
                &self.config,
                snapshot.root(),
                &sandbox_output,
                &run_root,
                &process_root,
                deadline,
            )?;
        }
        let production = self.config.expected_payload_manifest_sha256.is_some();
        let graph_path = sandbox_output.join("graphify-out").join("graph.json");
        let (help_sha256, extract_stdout, extract_stderr, extract_exit_code, graph_bytes) =
            if production {
                let extract_plan =
                    build_private_extract_plan(&self.config, snapshot, &run_root, &process_root)?;
                let extract = self.executor.execute(&extract_plan, deadline)?;
                require_private_success(&extract)?;
                let frame = parse_private_frame(&extract.stdout, self.config.limits)?;
                validate_graphify_version(frame.version_stdout, frame.version_stderr)?;
                let help_sha256 =
                    validate_graphify_help(&self.config, frame.help_stdout, frame.help_stderr)?;
                if !frame.extract_stderr.is_empty() {
                    return Err(error(
                        GraphifyAdapterErrorKind::PartialOutput,
                        "GRAPHIFY_PRIVATE_EXTRACT_STDERR_REJECTED",
                    ));
                }
                (
                    help_sha256,
                    frame.extract_stdout.to_vec(),
                    frame.extract_stderr.to_vec(),
                    extract.exit_code.unwrap_or(-1),
                    frame.graph.to_vec(),
                )
            } else {
                let version_plan = build_plan(
                    &self.config,
                    CommandKind::GraphifyVersion,
                    snapshot.root(),
                    &sandbox_output,
                    &run_root,
                    &process_root,
                )?;
                let version = self.executor.execute(&version_plan, deadline)?;
                require_clean_success(&version, "GRAPHIFY_VERSION_PROCESS_REJECTED")?;
                validate_graphify_version(&version.stdout, &version.stderr)?;

                let help_plan = build_plan(
                    &self.config,
                    CommandKind::GraphifyHelp,
                    snapshot.root(),
                    &sandbox_output,
                    &run_root,
                    &process_root,
                )?;
                let help = self.executor.execute(&help_plan, deadline)?;
                require_clean_success(&help, "GRAPHIFY_HELP_PROCESS_REJECTED")?;
                let help_sha256 = validate_graphify_help(&self.config, &help.stdout, &help.stderr)?;
                let extract_plan = build_plan(
                    &self.config,
                    CommandKind::Extract,
                    snapshot.root(),
                    &sandbox_output,
                    &run_root,
                    &process_root,
                )?;
                let extract = self.executor.execute(&extract_plan, deadline)?;
                require_clean_success(&extract, "GRAPHIFY_EXTRACT_PROCESS_REJECTED")?;
                let metadata = fs::symlink_metadata(&graph_path).map_err(|_| {
                    error(
                        GraphifyAdapterErrorKind::MissingOutput,
                        "GRAPHIFY_GRAPH_OUTPUT_MISSING",
                    )
                })?;
                if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
                    return Err(error(
                        GraphifyAdapterErrorKind::MissingOutput,
                        "GRAPHIFY_GRAPH_OUTPUT_NOT_REGULAR",
                    ));
                }
                if metadata.len() == 0 || metadata.len() > self.config.limits.max_graph_bytes {
                    return Err(error(
                        GraphifyAdapterErrorKind::OutputLimit,
                        "GRAPHIFY_GRAPH_OUTPUT_SIZE_REJECTED",
                    ));
                }
                let graph_bytes = read_bounded(&graph_path, self.config.limits.max_graph_bytes)?;
                (
                    help_sha256,
                    extract.stdout,
                    extract.stderr,
                    extract.exit_code.unwrap_or(-1),
                    graph_bytes,
                )
            };
        if self.config.expected_payload_manifest_sha256.is_some() {
            execute_system_hash_check(
                self.executor.as_mut(),
                &self.config,
                snapshot.root(),
                &sandbox_output,
                &run_root,
                &process_root,
                deadline,
            )?;
        }
        verify_runtime(&self.config)?;

        let graph = parse_graph(
            &graph_bytes,
            snapshot,
            GraphParseLimits {
                max_nodes: self.config.limits.max_nodes,
                max_edges: self.config.limits.max_edges,
                max_text_bytes: self.config.limits.max_text_bytes,
            },
        )?;
        verify_snapshot_binding(snapshot)?;
        let capability_sha256 = self.config.capability_sha256();
        let normalized_stdout =
            normalized_process_stdout(&extract_stdout, snapshot.root(), &graph_path)?;
        let raw_process_sha256 = framed_digest(&[
            &extract_exit_code.to_be_bytes(),
            normalized_stdout.as_bytes(),
            &extract_stderr,
        ]);
        let evidence_sha256 = framed_digest(&[
            snapshot.commit_id().as_bytes(),
            snapshot.tree_id().as_bytes(),
            snapshot.manifest_sha256().as_bytes(),
            snapshot.exclusion_sha256().as_bytes(),
            self.config.expected_execution_identity_sha256.as_bytes(),
            help_sha256.as_bytes(),
            capability_sha256.as_bytes(),
            graph.raw_graph_sha256().as_bytes(),
            graph.record_set_sha256().as_bytes(),
            raw_process_sha256.as_bytes(),
        ]);
        Ok(GraphifyAnalysis {
            graph,
            executable_sha256: self.config.expected_execution_identity_sha256.clone(),
            help_sha256,
            payload_manifest_sha256: self.config.expected_payload_manifest_sha256.clone(),
            capability_sha256,
            raw_process_sha256,
            evidence_sha256,
        })
    }

    #[cfg(test)]
    fn with_executor(
        config: GraphifyRuntimeConfig,
        bridge: SnapshotBridge,
        executor: impl GraphifyExecutor + Send + 'static,
    ) -> Self {
        Self {
            config,
            bridge,
            executor: Box::new(executor),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CommandKind {
    SystemHashes,
    BwrapVersion,
    BwrapHelp,
    PythonVersion,
    GraphifyVersion,
    GraphifyHelp,
    Extract,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CommandPlan {
    kind: CommandKind,
    executable: PathBuf,
    arguments: Vec<OsString>,
    current_dir: PathBuf,
    environment: BTreeMap<OsString, OsString>,
    capture_dir: PathBuf,
    stdout_limit: u64,
    diagnostic_limit: u64,
    output_root: PathBuf,
    artifact_root: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProcessOutcome {
    exit_code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

trait GraphifyExecutor {
    fn execute(
        &mut self,
        plan: &CommandPlan,
        deadline: Instant,
    ) -> GraphifyAdapterResult<ProcessOutcome>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OwnedChildExecutor;

impl GraphifyExecutor for OwnedChildExecutor {
    fn execute(
        &mut self,
        plan: &CommandPlan,
        deadline: Instant,
    ) -> GraphifyAdapterResult<ProcessOutcome> {
        if Instant::now() >= deadline {
            return Err(error(
                GraphifyAdapterErrorKind::Timeout,
                "GRAPHIFY_DEADLINE_EXPIRED_BEFORE_SPAWN",
            ));
        }
        fs::create_dir_all(&plan.capture_dir).map_err(|_| {
            error(
                GraphifyAdapterErrorKind::Spawn,
                "GRAPHIFY_CAPTURE_DIRECTORY_CREATE_FAILED",
            )
        })?;
        let sequence = PROCESS_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let label = match plan.kind {
            CommandKind::SystemHashes => "system-hashes",
            CommandKind::BwrapVersion => "bwrap-version",
            CommandKind::BwrapHelp => "bwrap-help",
            CommandKind::PythonVersion => "python-version",
            CommandKind::GraphifyVersion => "graphify-version",
            CommandKind::GraphifyHelp => "graphify-help",
            CommandKind::Extract => "extract",
        };
        let stdout_path = plan.capture_dir.join(format!("{label}-{sequence}.stdout"));
        let stderr_path = plan.capture_dir.join(format!("{label}-{sequence}.stderr"));

        #[cfg(windows)]
        let outcome = execute_owned_windows(plan, deadline, &stdout_path, &stderr_path);

        #[cfg(not(windows))]
        let outcome = {
            let exit_code = execute_owned_portable(plan, deadline, &stdout_path, &stderr_path)?;
            let stdout = read_bounded(&stdout_path, plan.stdout_limit)?;
            let stderr = read_bounded(&stderr_path, plan.diagnostic_limit)?;
            Ok(ProcessOutcome {
                exit_code,
                stdout,
                stderr,
            })
        };
        outcome
    }
}

#[cfg(windows)]
fn execute_owned_windows(
    plan: &CommandPlan,
    deadline: Instant,
    stdout_path: &Path,
    stderr_path: &Path,
) -> GraphifyAdapterResult<ProcessOutcome> {
    use crate::windows_job::{WindowsJobCommandPlan, run};

    let outcome = run(&WindowsJobCommandPlan {
        executable: plan.executable.clone(),
        arguments: plan.arguments.clone(),
        current_dir: plan.current_dir.clone(),
        environment: plan.environment.clone(),
        run_root: plan.output_root.clone(),
        stdout_path: stdout_path.to_path_buf(),
        stderr_path: stderr_path.to_path_buf(),
        stdout_limit: plan.stdout_limit,
        stderr_limit: plan.diagnostic_limit,
        deadline,
        teardown_timeout: Duration::from_secs(2),
    })?;
    Ok(ProcessOutcome {
        exit_code: Some(outcome.exit_code.cast_signed()),
        stdout: outcome.stdout,
        stderr: outcome.stderr,
    })
}

#[cfg(not(windows))]
fn execute_owned_portable(
    plan: &CommandPlan,
    deadline: Instant,
    stdout_path: &Path,
    stderr_path: &Path,
) -> GraphifyAdapterResult<Option<i32>> {
    let stdout_file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(stdout_path)
        .map_err(|_| {
            error(
                GraphifyAdapterErrorKind::Spawn,
                "GRAPHIFY_STDOUT_CAPTURE_CREATE_FAILED",
            )
        })?;
    let stderr_file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(stderr_path)
        .map_err(|_| {
            error(
                GraphifyAdapterErrorKind::Spawn,
                "GRAPHIFY_STDERR_CAPTURE_CREATE_FAILED",
            )
        })?;
    let mut command = Command::new(&plan.executable);
    command.args(&plan.arguments);
    command.current_dir(&plan.current_dir);
    command.env_clear();
    command.envs(&plan.environment);
    command.stdin(Stdio::null());
    command.stdout(Stdio::from(stdout_file));
    command.stderr(Stdio::from(stderr_file));
    let mut child = command.spawn().map_err(|_| {
        error(
            GraphifyAdapterErrorKind::Spawn,
            "GRAPHIFY_CHILD_SPAWN_FAILED",
        )
    })?;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                terminate_portable_child(&mut child)?;
                return Err(error(
                    GraphifyAdapterErrorKind::Timeout,
                    "GRAPHIFY_TIMEOUT_REAP_CONFIRMED",
                ));
            }
            Err(_) => {
                terminate_portable_child(&mut child)?;
                return Err(error(
                    GraphifyAdapterErrorKind::TeardownAmbiguous,
                    "GRAPHIFY_CHILD_STATUS_UNKNOWN",
                ));
            }
        }
    };
    Ok(status.code())
}

#[cfg(not(windows))]
fn terminate_portable_child(child: &mut Child) -> GraphifyAdapterResult<()> {
    let _kill_attempt = child.kill();
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(2))
        .ok_or_else(|| {
            error(
                GraphifyAdapterErrorKind::TeardownAmbiguous,
                "GRAPHIFY_TEARDOWN_DEADLINE_OVERFLOW",
            )
        })?;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return Ok(()),
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) | Err(_) => {
                return Err(error(
                    GraphifyAdapterErrorKind::TeardownAmbiguous,
                    "GRAPHIFY_TIMEOUT_REAP_UNKNOWN",
                ));
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn execute_system_preflight(
    executor: &mut dyn GraphifyExecutor,
    config: &GraphifyRuntimeConfig,
    snapshot_root: &Path,
    artifact_root: &Path,
    output_root: &Path,
    capture_dir: &Path,
    deadline: Instant,
) -> GraphifyAdapterResult<()> {
    execute_system_hash_check(
        executor,
        config,
        snapshot_root,
        artifact_root,
        output_root,
        capture_dir,
        deadline,
    )?;
    for (kind, expected_digest, process_code, digest_code) in [
        (
            CommandKind::BwrapVersion,
            GRAPHIFY_WSL_BWRAP_VERSION_SHA256,
            "GRAPHIFY_BWRAP_VERSION_PROCESS_REJECTED",
            "GRAPHIFY_BWRAP_VERSION_DIGEST_MISMATCH",
        ),
        (
            CommandKind::BwrapHelp,
            GRAPHIFY_WSL_BWRAP_HELP_SHA256,
            "GRAPHIFY_BWRAP_HELP_PROCESS_REJECTED",
            "GRAPHIFY_BWRAP_HELP_DIGEST_MISMATCH",
        ),
        (
            CommandKind::PythonVersion,
            GRAPHIFY_WSL_PYTHON_VERSION_SHA256,
            "GRAPHIFY_PYTHON_VERSION_PROCESS_REJECTED",
            "GRAPHIFY_PYTHON_VERSION_DIGEST_MISMATCH",
        ),
    ] {
        let plan = build_plan(
            config,
            kind,
            snapshot_root,
            artifact_root,
            output_root,
            capture_dir,
        )?;
        let outcome = executor.execute(&plan, deadline)?;
        require_clean_success(&outcome, process_code)?;
        if crate::snapshot::sha256_bytes(&outcome.stdout) != expected_digest {
            return Err(error(
                GraphifyAdapterErrorKind::GraphifyIdentity,
                digest_code,
            ));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn execute_system_hash_check(
    executor: &mut dyn GraphifyExecutor,
    config: &GraphifyRuntimeConfig,
    snapshot_root: &Path,
    artifact_root: &Path,
    output_root: &Path,
    capture_dir: &Path,
    deadline: Instant,
) -> GraphifyAdapterResult<()> {
    let plan = build_plan(
        config,
        CommandKind::SystemHashes,
        snapshot_root,
        artifact_root,
        output_root,
        capture_dir,
    )?;
    let outcome = executor.execute(&plan, deadline)?;
    require_clean_success(&outcome, "GRAPHIFY_SYSTEM_HASH_PROCESS_REJECTED")?;
    let expected = format!(
        "{GRAPHIFY_WSL_BWRAP_SHA256}  {GRAPHIFY_WSL_BWRAP_PATH}\n\
         {GRAPHIFY_WSL_PYTHON_SHA256}  {GRAPHIFY_WSL_PYTHON_PATH}\n\
         {GRAPHIFY_WSL_OS_RELEASE_SHA256}  /usr/lib/os-release\n"
    );
    if outcome.stdout != expected.as_bytes() {
        return Err(error(
            GraphifyAdapterErrorKind::GraphifyIdentity,
            "GRAPHIFY_SYSTEM_HASH_OUTPUT_MISMATCH",
        ));
    }
    Ok(())
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

fn build_plan(
    config: &GraphifyRuntimeConfig,
    kind: CommandKind,
    snapshot_root: &Path,
    artifact_root: &Path,
    output_root: &Path,
    capture_dir: &Path,
) -> GraphifyAdapterResult<CommandPlan> {
    let arguments = match kind {
        CommandKind::SystemHashes => fixed_wsl_exec(
            WSL_SHA256SUM_PATH,
            [
                GRAPHIFY_WSL_BWRAP_PATH,
                GRAPHIFY_WSL_PYTHON_PATH,
                "/usr/lib/os-release",
            ],
        ),
        CommandKind::BwrapVersion => fixed_wsl_exec(GRAPHIFY_WSL_BWRAP_PATH, ["--version"]),
        CommandKind::BwrapHelp => fixed_wsl_exec(GRAPHIFY_WSL_BWRAP_PATH, ["--help"]),
        CommandKind::PythonVersion => fixed_wsl_exec(GRAPHIFY_WSL_PYTHON_PATH, ["--version"]),
        CommandKind::GraphifyVersion => {
            sandboxed_graphify_arguments(config, snapshot_root, artifact_root, ["--version"])?
        }
        CommandKind::GraphifyHelp => {
            sandboxed_graphify_arguments(config, snapshot_root, artifact_root, ["--help"])?
        }
        CommandKind::Extract => sandboxed_graphify_arguments(
            config,
            snapshot_root,
            artifact_root,
            [
                "extract",
                "/source",
                "--code-only",
                "--no-cluster",
                "--max-workers",
                "1",
                "--out",
                "/output",
            ],
        )?,
    };
    Ok(CommandPlan {
        kind,
        executable: config.wsl_executable.clone(),
        arguments,
        current_dir: output_root.to_path_buf(),
        environment: minimal_launcher_environment(&config.wsl_executable)?,
        capture_dir: capture_dir.to_path_buf(),
        stdout_limit: config.limits.max_diagnostic_bytes,
        diagnostic_limit: config.limits.max_diagnostic_bytes,
        output_root: output_root.to_path_buf(),
        artifact_root: artifact_root.to_path_buf(),
    })
}

fn build_private_extract_plan(
    config: &GraphifyRuntimeConfig,
    snapshot: &MaterializedSnapshot,
    run_root: &Path,
    capture_dir: &Path,
) -> GraphifyAdapterResult<CommandPlan> {
    let source_bytes = snapshot.sources().iter().try_fold(0_u64, |total, source| {
        total.checked_add(source.byte_length()).ok_or_else(|| {
            error(
                GraphifyAdapterErrorKind::Configuration,
                "GRAPHIFY_PRIVATE_SOURCE_SIZE_OVERFLOW",
            )
        })
    })?;
    let stdout_limit = private_frame_limit(config.limits)?;
    Ok(CommandPlan {
        kind: CommandKind::Extract,
        executable: config.wsl_executable.clone(),
        arguments: private_graphify_arguments(config, snapshot, source_bytes)?,
        current_dir: run_root.to_path_buf(),
        environment: minimal_launcher_environment(&config.wsl_executable)?,
        capture_dir: capture_dir.to_path_buf(),
        stdout_limit,
        diagnostic_limit: config.limits.max_diagnostic_bytes,
        output_root: run_root.to_path_buf(),
        artifact_root: run_root.to_path_buf(),
    })
}

fn private_frame_limit(limits: GraphOutputLimits) -> GraphifyAdapterResult<u64> {
    let framing = u64::try_from(PRIVATE_FRAME_MAGIC.len() + PRIVATE_FRAME_FIELD_COUNT * 8)
        .map_err(|_| {
            error(
                GraphifyAdapterErrorKind::Configuration,
                "GRAPHIFY_PRIVATE_FRAME_LIMIT_OVERFLOW",
            )
        })?;
    limits
        .max_diagnostic_bytes
        .checked_mul(6)
        .and_then(|diagnostics| diagnostics.checked_add(limits.max_graph_bytes))
        .and_then(|bytes| bytes.checked_add(framing))
        .ok_or_else(|| {
            error(
                GraphifyAdapterErrorKind::Configuration,
                "GRAPHIFY_PRIVATE_FRAME_LIMIT_OVERFLOW",
            )
        })
}

fn private_graphify_arguments(
    config: &GraphifyRuntimeConfig,
    snapshot: &MaterializedSnapshot,
    source_bytes: u64,
) -> GraphifyAdapterResult<Vec<OsString>> {
    let runtime_site_packages = windows_path_to_wsl(&config.runtime_root.join("site-packages"))?;
    let install_report = windows_path_to_wsl(&config.runtime_root.join("install-report.json"))?;
    let snapshot_root = windows_path_to_wsl(snapshot.root())?;
    let expected_runtime_manifest = config
        .expected_payload_manifest_sha256
        .as_deref()
        .unwrap_or("TEST_UNVERIFIED_GRAPHIFY_PAYLOAD");
    let mut command = fixed_wsl_exec(GRAPHIFY_WSL_BWRAP_PATH, std::iter::empty());
    for argument in [
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
        "--tmpfs",
        "/tmp",
        "--tmpfs",
        "/runtime",
        "--tmpfs",
        "/source",
        "--tmpfs",
        "/output",
        "--dir",
        "/home",
        "--dir",
        "/home/lattice",
        "--dir",
        "/runtime-input",
    ] {
        command.push(OsString::from(argument));
    }
    for (source, destination) in [
        (runtime_site_packages, "/runtime-input/site-packages"),
        (install_report, "/runtime-input/install-report.json"),
        (snapshot_root, "/source-input"),
    ] {
        command.push(OsString::from("--ro-bind"));
        command.push(source);
        command.push(OsString::from(destination));
    }
    command.push(OsString::from("--clearenv"));
    for (name, value) in [
        ("PATH", "/usr/bin:/bin"),
        ("HOME", "/home/lattice"),
        ("TMPDIR", "/tmp"),
        ("NO_COLOR", "1"),
        ("CI", "1"),
        ("TZ", "UTC"),
        ("LANG", "C.UTF-8"),
        ("LC_ALL", "C.UTF-8"),
    ] {
        command.push(OsString::from("--setenv"));
        command.push(OsString::from(name));
        command.push(OsString::from(value));
    }
    command.extend([
        OsString::from("--chdir"),
        OsString::from("/output"),
        OsString::from(GRAPHIFY_WSL_PYTHON_PATH),
        OsString::from("-I"),
        OsString::from("-S"),
        OsString::from("-B"),
        OsString::from("-c"),
        OsString::from(PRIVATE_RUNNER_SOURCE),
        OsString::from("extract"),
        OsString::from(expected_runtime_manifest),
        OsString::from(GRAPHIFY_WSL_RUNTIME_FILE_COUNT.to_string()),
        OsString::from(GRAPHIFY_WSL_RUNTIME_BYTE_COUNT.to_string()),
        OsString::from(GRAPHIFY_WSL_INSTALL_REPORT_SHA256),
        OsString::from(snapshot.manifest_sha256()),
        OsString::from(snapshot.sources().len().to_string()),
        OsString::from(source_bytes.to_string()),
        OsString::from(config.limits.max_graph_bytes.to_string()),
        OsString::from(config.limits.max_diagnostic_bytes.to_string()),
    ]);
    Ok(command)
}

fn fixed_wsl_exec(
    executable: &str,
    arguments: impl IntoIterator<Item = &'static str>,
) -> Vec<OsString> {
    let mut command = vec![
        OsString::from("-d"),
        OsString::from(GRAPHIFY_WSL_DISTRO),
        OsString::from("--exec"),
        OsString::from(executable),
    ];
    command.extend(arguments.into_iter().map(OsString::from));
    command
}

fn sandboxed_graphify_arguments(
    config: &GraphifyRuntimeConfig,
    snapshot_root: &Path,
    artifact_root: &Path,
    graphify_arguments: impl IntoIterator<Item = &'static str>,
) -> GraphifyAdapterResult<Vec<OsString>> {
    let runtime_site_packages = windows_path_to_wsl(&config.runtime_root.join("site-packages"))?;
    let install_report = windows_path_to_wsl(&config.runtime_root.join("install-report.json"))?;
    let snapshot = windows_path_to_wsl(snapshot_root)?;
    let output = windows_path_to_wsl(artifact_root)?;
    let mut command = fixed_wsl_exec(GRAPHIFY_WSL_BWRAP_PATH, std::iter::empty());
    for argument in [
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
        "--tmpfs",
        "/tmp",
        "--dir",
        "/home",
        "--dir",
        "/home/lattice",
        "--dir",
        "/runtime",
    ] {
        command.push(OsString::from(argument));
    }
    for (source, destination, mode) in [
        (runtime_site_packages, "/runtime/site-packages", "--ro-bind"),
        (install_report, "/runtime/install-report.json", "--ro-bind"),
        (snapshot, "/source", "--ro-bind"),
        (output, "/output", "--bind"),
    ] {
        command.push(OsString::from(mode));
        command.push(source);
        command.push(OsString::from(destination));
    }
    command.push(OsString::from("--clearenv"));
    for (name, value) in [
        ("PATH", "/usr/bin:/bin"),
        ("HOME", "/home/lattice"),
        ("TMPDIR", "/tmp"),
        ("XDG_CACHE_HOME", "/tmp/cache"),
        ("XDG_CONFIG_HOME", "/tmp/config"),
        ("PYTHONPATH", "/runtime/site-packages"),
        ("PYTHONPYCACHEPREFIX", "/tmp/pycache"),
        ("PYTHONHASHSEED", "0"),
        ("PYTHONDONTWRITEBYTECODE", "1"),
        ("PYTHONNOUSERSITE", "1"),
        ("PYTHONSAFEPATH", "1"),
        ("PYTHONUTF8", "1"),
        ("GRAPHIFY_QUERY_LOG_DISABLE", "1"),
        ("GRAPHIFY_MAX_WORKERS", "1"),
        ("NO_COLOR", "1"),
        ("CI", "1"),
        ("TZ", "UTC"),
        ("LANG", "C.UTF-8"),
        ("LC_ALL", "C.UTF-8"),
    ] {
        command.push(OsString::from("--setenv"));
        command.push(OsString::from(name));
        command.push(OsString::from(value));
    }
    command.extend([
        OsString::from("--chdir"),
        OsString::from("/output"),
        OsString::from(GRAPHIFY_WSL_PYTHON_PATH),
        OsString::from("-P"),
        OsString::from("-B"),
        OsString::from("-m"),
        OsString::from("graphify"),
    ]);
    command.extend(graphify_arguments.into_iter().map(OsString::from));
    Ok(command)
}

pub(crate) fn minimal_launcher_environment(
    executable: &Path,
) -> GraphifyAdapterResult<BTreeMap<OsString, OsString>> {
    let mut environment = BTreeMap::new();
    for name in ["SystemRoot", "WINDIR", "ComSpec", "PATHEXT"] {
        if let Some(value) = std::env::var_os(name) {
            environment.insert(OsString::from(name), value);
        }
    }
    let executable_parent = executable.parent().ok_or_else(|| {
        error(
            GraphifyAdapterErrorKind::Configuration,
            "GRAPHIFY_EXECUTABLE_PARENT_MISSING",
        )
    })?;
    let mut path_entries = vec![executable_parent.to_path_buf()];
    if let Some(system_root) = std::env::var_os("SystemRoot") {
        let root = PathBuf::from(system_root);
        path_entries.push(root.join("System32"));
        path_entries.push(root.join("System32"));
    } else {
        path_entries.push(PathBuf::from("/usr/bin"));
        path_entries.push(PathBuf::from("/bin"));
    }
    environment.insert(
        OsString::from("PATH"),
        std::env::join_paths(path_entries).map_err(|_| {
            error(
                GraphifyAdapterErrorKind::Configuration,
                "GRAPHIFY_MINIMAL_PATH_REJECTED",
            )
        })?,
    );
    Ok(environment)
}

fn windows_path_to_wsl(path: &Path) -> GraphifyAdapterResult<OsString> {
    let canonical = fs::canonicalize(path).map_err(|_| {
        error(
            GraphifyAdapterErrorKind::Configuration,
            "GRAPHIFY_WSL_BIND_SOURCE_RESOLVE_FAILED",
        )
    })?;
    let text = canonical.to_str().ok_or_else(|| {
        error(
            GraphifyAdapterErrorKind::Configuration,
            "GRAPHIFY_WSL_BIND_SOURCE_NON_UNICODE",
        )
    })?;
    let text = text.strip_prefix(r"\\?\").unwrap_or(text);
    let bytes = text.as_bytes();
    if bytes.len() < 3
        || !bytes[0].is_ascii_alphabetic()
        || bytes[1] != b':'
        || !matches!(bytes[2], b'\\' | b'/')
        || text.contains(['\0', '\r', '\n'])
    {
        return Err(error(
            GraphifyAdapterErrorKind::Configuration,
            "GRAPHIFY_WSL_BIND_SOURCE_NOT_LOCAL_DRIVE",
        ));
    }
    let drive = char::from(bytes[0].to_ascii_lowercase());
    let tail = text[3..].replace('\\', "/");
    Ok(OsString::from(format!("/mnt/{drive}/{tail}")))
}

fn command_path(path: &Path) -> OsString {
    #[cfg(windows)]
    {
        let value = path.as_os_str().to_string_lossy();
        if value.starts_with(r"\\?\") {
            path.as_os_str().to_owned()
        } else if value.starts_with(r"\\") {
            OsString::from(format!(r"\\?\UNC\{}", value.trim_start_matches(r"\\")))
        } else {
            OsString::from(format!(r"\\?\{value}"))
        }
    }
    #[cfg(not(windows))]
    {
        path.as_os_str().to_owned()
    }
}

fn normalized_process_stdout(
    stdout: &[u8],
    snapshot_root: &Path,
    graph_path: &Path,
) -> GraphifyAdapterResult<String> {
    let stdout = std::str::from_utf8(stdout).map_err(|_| {
        error(
            GraphifyAdapterErrorKind::MalformedOutput,
            "GRAPHIFY_PROCESS_STDOUT_NON_UTF8",
        )
    })?;
    let mut normalized = stdout.to_owned();
    normalized = normalized.replace("/output/graphify-out/graph.json", "<LATTICE_GRAPH_OUTPUT>");
    normalized = normalized.replace("/source", "<LATTICE_CODE_SNAPSHOT>");
    for (path, replacement) in [
        (graph_path, "<LATTICE_GRAPH_OUTPUT>"),
        (snapshot_root, "<LATTICE_CODE_SNAPSHOT>"),
    ] {
        let process_path = command_path(path);
        let process_path = process_path.to_string_lossy();
        let filesystem_path = path.as_os_str().to_string_lossy();
        normalized = normalized.replace(process_path.as_ref(), replacement);
        normalized = normalized.replace(filesystem_path.as_ref(), replacement);
    }
    Ok(normalized)
}

fn verify_runtime(config: &GraphifyRuntimeConfig) -> GraphifyAdapterResult<()> {
    if config.expected_payload_manifest_sha256.is_none() {
        let observed = file_sha256(&config.wsl_executable).map_err(|_| {
            error(
                GraphifyAdapterErrorKind::GraphifyIdentity,
                "GRAPHIFY_TEST_LAUNCHER_HASH_UNAVAILABLE",
            )
        })?;
        if observed == config.expected_launcher_sha256 {
            return Ok(());
        }
        return Err(error(
            GraphifyAdapterErrorKind::GraphifyIdentity,
            "GRAPHIFY_TEST_LAUNCHER_DIGEST_MISMATCH",
        ));
    }
    let reviewed = verify_reviewed_runtime(&config.wsl_executable, &config.runtime_root)?;
    if reviewed.wsl_executable() != config.wsl_executable
        || reviewed.runtime_root() != config.runtime_root
        || reviewed.launcher_sha256() != config.expected_launcher_sha256
        || reviewed.execution_identity_sha256() != config.expected_execution_identity_sha256
        || config.expected_execution_identity_sha256 != GRAPHIFY_WSL_EXECUTION_IDENTITY_SHA256
    {
        return Err(error(
            GraphifyAdapterErrorKind::GraphifyIdentity,
            "GRAPHIFY_WSL_EXECUTION_IDENTITY_CHANGED",
        ));
    }
    if let Some(expected_manifest) = &config.expected_payload_manifest_sha256
        && (reviewed.manifest_sha256() != expected_manifest
            || expected_manifest != GRAPHIFY_WSL_RUNTIME_MANIFEST_SHA256)
    {
        return Err(error(
            GraphifyAdapterErrorKind::GraphifyIdentity,
            "GRAPHIFY_PAYLOAD_IDENTITY_CHANGED",
        ));
    }
    Ok(())
}

fn require_clean_success(
    outcome: &ProcessOutcome,
    code: &'static str,
) -> GraphifyAdapterResult<()> {
    if outcome.exit_code != Some(0) {
        return Err(error(GraphifyAdapterErrorKind::NonZeroExit, code));
    }
    if !outcome.stderr.is_empty() {
        return Err(error(
            GraphifyAdapterErrorKind::PartialOutput,
            "GRAPHIFY_SUCCESS_WITH_STDERR_REJECTED",
        ));
    }
    Ok(())
}

fn require_private_success(outcome: &ProcessOutcome) -> GraphifyAdapterResult<()> {
    if outcome.exit_code != Some(0) {
        let code = match outcome.exit_code {
            Some(64) => "GRAPHIFY_PRIVATE_ARGUMENT_REJECTED",
            Some(65) => "GRAPHIFY_PRIVATE_COPY_REJECTED",
            Some(66) => "GRAPHIFY_PRIVATE_MANIFEST_REJECTED",
            Some(67) => "GRAPHIFY_PRIVATE_LANDLOCK_REJECTED",
            Some(68) => "GRAPHIFY_PRIVATE_COMMAND_REJECTED",
            Some(69) => "GRAPHIFY_PRIVATE_DIAGNOSTIC_LIMIT",
            Some(70) => "GRAPHIFY_PRIVATE_GRAPH_REJECTED",
            Some(71) => "GRAPHIFY_PRIVATE_INSTALL_REPORT_REJECTED",
            Some(72) => "GRAPHIFY_PRIVATE_RUNTIME_MANIFEST_REJECTED",
            Some(73) => "GRAPHIFY_PRIVATE_SOURCE_MANIFEST_REJECTED",
            _ => "GRAPHIFY_PRIVATE_SESSION_PROCESS_REJECTED",
        };
        return Err(error(GraphifyAdapterErrorKind::NonZeroExit, code));
    }
    if !outcome.stderr.is_empty() {
        return Err(error(
            GraphifyAdapterErrorKind::PartialOutput,
            "GRAPHIFY_PRIVATE_BOOTSTRAP_STDERR_REJECTED",
        ));
    }
    Ok(())
}

fn validate_graphify_version(stdout: &[u8], stderr: &[u8]) -> GraphifyAdapterResult<()> {
    if !stderr.is_empty() {
        return Err(error(
            GraphifyAdapterErrorKind::PartialOutput,
            "GRAPHIFY_VERSION_STDERR_REJECTED",
        ));
    }
    let version_text = std::str::from_utf8(stdout).map_err(|_| {
        error(
            GraphifyAdapterErrorKind::GraphifyIdentity,
            "GRAPHIFY_VERSION_OUTPUT_NON_UTF8",
        )
    })?;
    if version_text.trim() != format!("graphify {GRAPHIFY_VERSION}") {
        return Err(error(
            GraphifyAdapterErrorKind::GraphifyIdentity,
            "GRAPHIFY_VERSION_MISMATCH",
        ));
    }
    if crate::snapshot::sha256_bytes(stdout) != GRAPHIFY_WSL_GRAPHIFY_VERSION_SHA256 {
        return Err(error(
            GraphifyAdapterErrorKind::GraphifyIdentity,
            "GRAPHIFY_VERSION_DIGEST_MISMATCH",
        ));
    }
    Ok(())
}

fn validate_graphify_help(
    config: &GraphifyRuntimeConfig,
    stdout: &[u8],
    stderr: &[u8],
) -> GraphifyAdapterResult<String> {
    if !stderr.is_empty() {
        return Err(error(
            GraphifyAdapterErrorKind::PartialOutput,
            "GRAPHIFY_HELP_STDERR_REJECTED",
        ));
    }
    let help_text = std::str::from_utf8(stdout).map_err(|_| {
        error(
            GraphifyAdapterErrorKind::GraphifyIdentity,
            "GRAPHIFY_HELP_OUTPUT_NON_UTF8",
        )
    })?;
    if REQUIRED_HELP_FRAGMENTS
        .iter()
        .any(|fragment| !help_text.contains(fragment))
    {
        return Err(error(
            GraphifyAdapterErrorKind::GraphifyIdentity,
            "GRAPHIFY_HELP_CAPABILITY_MISMATCH",
        ));
    }
    let digest = crate::snapshot::sha256_bytes(stdout);
    if digest != config.expected_help_sha256 {
        return Err(error(
            GraphifyAdapterErrorKind::GraphifyIdentity,
            "GRAPHIFY_HELP_DIGEST_MISMATCH",
        ));
    }
    Ok(digest)
}

#[derive(Debug)]
struct PrivateGraphifyFrame<'a> {
    version_stdout: &'a [u8],
    version_stderr: &'a [u8],
    help_stdout: &'a [u8],
    help_stderr: &'a [u8],
    extract_stdout: &'a [u8],
    extract_stderr: &'a [u8],
    graph: &'a [u8],
}

fn parse_private_frame(
    bytes: &[u8],
    limits: GraphOutputLimits,
) -> GraphifyAdapterResult<PrivateGraphifyFrame<'_>> {
    let Some(mut remaining) = bytes.strip_prefix(PRIVATE_FRAME_MAGIC) else {
        return Err(error(
            GraphifyAdapterErrorKind::MalformedOutput,
            "GRAPHIFY_PRIVATE_FRAME_MAGIC_REJECTED",
        ));
    };
    let mut fields = Vec::with_capacity(PRIVATE_FRAME_FIELD_COUNT);
    for index in 0..PRIVATE_FRAME_FIELD_COUNT {
        let length_bytes: [u8; 8] = remaining
            .get(..8)
            .and_then(|prefix| prefix.try_into().ok())
            .ok_or_else(|| {
                error(
                    GraphifyAdapterErrorKind::MalformedOutput,
                    "GRAPHIFY_PRIVATE_FRAME_TRUNCATED",
                )
            })?;
        remaining = &remaining[8..];
        let length = usize::try_from(u64::from_be_bytes(length_bytes)).map_err(|_| {
            error(
                GraphifyAdapterErrorKind::OutputLimit,
                "GRAPHIFY_PRIVATE_FRAME_LENGTH_OVERFLOW",
            )
        })?;
        let bound = if index + 1 == PRIVATE_FRAME_FIELD_COUNT {
            limits.max_graph_bytes
        } else {
            limits.max_diagnostic_bytes
        };
        if u64::try_from(length).map_or(true, |length| length > bound) {
            return Err(error(
                GraphifyAdapterErrorKind::OutputLimit,
                "GRAPHIFY_PRIVATE_FRAME_FIELD_LIMIT",
            ));
        }
        let (field, tail) = remaining.split_at_checked(length).ok_or_else(|| {
            error(
                GraphifyAdapterErrorKind::MalformedOutput,
                "GRAPHIFY_PRIVATE_FRAME_TRUNCATED",
            )
        })?;
        fields.push(field);
        remaining = tail;
    }
    if !remaining.is_empty() {
        return Err(error(
            GraphifyAdapterErrorKind::MalformedOutput,
            "GRAPHIFY_PRIVATE_FRAME_TRAILING_BYTES",
        ));
    }
    Ok(PrivateGraphifyFrame {
        version_stdout: fields[0],
        version_stderr: fields[1],
        help_stdout: fields[2],
        help_stderr: fields[3],
        extract_stdout: fields[4],
        extract_stderr: fields[5],
        graph: fields[6],
    })
}

fn read_bounded(path: &Path, limit: u64) -> GraphifyAdapterResult<Vec<u8>> {
    let metadata = fs::metadata(path).map_err(|_| {
        error(
            GraphifyAdapterErrorKind::MissingOutput,
            "GRAPHIFY_BOUNDED_READ_METADATA_FAILED",
        )
    })?;
    if metadata.len() > limit {
        return Err(error(
            GraphifyAdapterErrorKind::OutputLimit,
            "GRAPHIFY_BOUNDED_READ_LIMIT",
        ));
    }
    let capacity = usize::try_from(metadata.len()).map_err(|_| {
        error(
            GraphifyAdapterErrorKind::OutputLimit,
            "GRAPHIFY_BOUNDED_READ_SIZE_OVERFLOW",
        )
    })?;
    let mut bytes = Vec::with_capacity(capacity);
    File::open(path)
        .and_then(|mut file| file.read_to_end(&mut bytes))
        .map_err(|_| {
            error(
                GraphifyAdapterErrorKind::MissingOutput,
                "GRAPHIFY_BOUNDED_READ_FAILED",
            )
        })?;
    if bytes.len() as u64 != metadata.len() {
        return Err(error(
            GraphifyAdapterErrorKind::PartialOutput,
            "GRAPHIFY_BOUNDED_READ_CHANGED",
        ));
    }
    Ok(bytes)
}

#[cfg(test)]
fn is_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

const fn error(kind: GraphifyAdapterErrorKind, code: &'static str) -> GraphifyAdapterError {
    GraphifyAdapterError::new(kind, code)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::fs;
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::snapshot::MaterializedSnapshot;

    const HELP: &[u8] = b"Usage: graphify <command>\nextract <path> headless\n--code-only\n--no-cluster\n--max-workers N\n--out DIR\n";

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum FakeMode {
        Valid,
        NonZero,
        Missing,
        Malformed,
        ZeroNodes,
        Timeout,
    }

    struct FakeExecutor {
        mode: FakeMode,
        plans: Arc<Mutex<Vec<CommandPlan>>>,
    }

    impl GraphifyExecutor for FakeExecutor {
        fn execute(
            &mut self,
            plan: &CommandPlan,
            _deadline: Instant,
        ) -> GraphifyAdapterResult<ProcessOutcome> {
            self.plans.lock().expect("plans").push(plan.clone());
            match plan.kind {
                CommandKind::GraphifyVersion => Ok(ProcessOutcome {
                    exit_code: Some(0),
                    stdout: format!("graphify {GRAPHIFY_VERSION}\n").into_bytes(),
                    stderr: Vec::new(),
                }),
                CommandKind::GraphifyHelp => Ok(ProcessOutcome {
                    exit_code: Some(0),
                    stdout: HELP.to_vec(),
                    stderr: Vec::new(),
                }),
                CommandKind::SystemHashes
                | CommandKind::BwrapVersion
                | CommandKind::BwrapHelp
                | CommandKind::PythonVersion => {
                    panic!("unverified unit fixture must not execute production WSL preflight")
                }
                CommandKind::Extract if self.mode == FakeMode::Timeout => Err(error(
                    GraphifyAdapterErrorKind::Timeout,
                    "GRAPHIFY_TIMEOUT_REAP_CONFIRMED",
                )),
                CommandKind::Extract if self.mode == FakeMode::NonZero => Ok(ProcessOutcome {
                    exit_code: Some(7),
                    stdout: Vec::new(),
                    stderr: Vec::new(),
                }),
                CommandKind::Extract => {
                    if self.mode != FakeMode::Missing {
                        let directory = plan.artifact_root.join("graphify-out");
                        fs::create_dir_all(&directory).expect("graph output directory");
                        let graph = match self.mode {
                            FakeMode::Malformed => b"{".as_slice(),
                            FakeMode::ZeroNodes => br#"{"nodes":[],"edges":[],"hyperedges":[],"input_tokens":0,"output_tokens":0}"#,
                            _ => valid_graph(),
                        };
                        fs::write(directory.join("graph.json"), graph).expect("graph output");
                    }
                    Ok(ProcessOutcome {
                        exit_code: Some(0),
                        stdout: b"complete\n".to_vec(),
                        stderr: Vec::new(),
                    })
                }
            }
        }
    }

    fn valid_graph() -> &'static [u8] {
        br#"{"nodes":[{"id":"src_lib","label":"lib.rs","file_type":"code","source_file":"src/lib.rs","source_location":"L1","_origin":"ast"},{"id":"src_main","label":"main()","file_type":"code","source_file":"src/lib.rs","source_location":"L1","_origin":"ast"}],"edges":[{"source":"src_lib","target":"src_main","relation":"contains","confidence":"EXTRACTED","source_file":"src/lib.rs","source_location":"L1","weight":1.0,"_origin":"ast"}],"hyperedges":[],"input_tokens":0,"output_tokens":0}"#
    }

    fn fixture(
        mode: FakeMode,
    ) -> (
        PinnedGraphifyAdapter,
        MaterializedSnapshot,
        Arc<Mutex<Vec<CommandPlan>>>,
    ) {
        let root = std::env::temp_dir().join(format!(
            "lattice-graphify-process-{}-{}",
            std::process::id(),
            RUN_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let snapshot_root = root.join("snapshot");
        fs::create_dir_all(snapshot_root.join(".git")).expect("vcs boundary");
        fs::create_dir_all(snapshot_root.join("src")).expect("src");
        fs::write(snapshot_root.join("src/lib.rs"), b"fn main() {}\n").expect("source");
        let executable = root.join(if cfg!(windows) { "wsl.exe" } else { "wsl" });
        fs::write(&executable, b"fake executable").expect("executable");
        let executable_digest = crate::snapshot::sha256_bytes(b"fake executable");
        let runtime_root = root.join("runtime");
        fs::create_dir_all(runtime_root.join("site-packages")).expect("runtime site-packages");
        fs::write(runtime_root.join("install-report.json"), b"{}\n")
            .expect("runtime install report");
        let config = GraphifyRuntimeConfig::for_test_unverified(
            &executable,
            &runtime_root,
            &executable_digest,
            &executable_digest,
            crate::snapshot::sha256_bytes(HELP),
            root.join("staging"),
            Duration::from_secs(1),
            GraphOutputLimits::default(),
        )
        .expect("config");
        let plans = Arc::new(Mutex::new(Vec::new()));
        let adapter = PinnedGraphifyAdapter::with_executor(
            config,
            SnapshotBridge::new(),
            FakeExecutor {
                mode,
                plans: Arc::clone(&plans),
            },
        );
        let snapshot =
            MaterializedSnapshot::for_test(snapshot_root, vec![("src/lib.rs", b"fn main() {}\n")]);
        (adapter, snapshot, plans)
    }

    #[test]
    fn fixed_plan_has_only_bwrap_headless_args_and_minimal_launcher_environment() {
        let (mut adapter, snapshot, _) = fixture(FakeMode::Valid);
        adapter
            .analyze_materialized(&snapshot)
            .expect("valid fake analysis");
        let shape_root = adapter.config.staging_root().join("private-shape");
        let capture = shape_root.join("capture");
        fs::create_dir_all(&capture).expect("private plan roots");
        let extract = build_private_extract_plan(&adapter.config, &snapshot, &shape_root, &capture)
            .expect("private extract plan");
        assert_eq!(
            &extract.arguments[..4],
            [
                OsString::from("-d"),
                OsString::from("Ubuntu"),
                OsString::from("--exec"),
                OsString::from("/usr/bin/bwrap"),
            ]
        );
        for fixed in [
            "--die-with-parent",
            "--unshare-all",
            "--unshare-user",
            "--disable-userns",
            "--assert-userns-disabled",
            "--new-session",
            "--clearenv",
        ] {
            assert!(extract.arguments.iter().any(|argument| argument == fixed));
        }
        for private_mount in ["/runtime", "/source", "/output"] {
            assert!(
                extract
                    .arguments
                    .windows(2)
                    .any(|window| { window[0] == "--tmpfs" && window[1] == private_mount })
            );
        }
        for verified_input in [
            "/runtime-input/site-packages",
            "/runtime-input/install-report.json",
            "/source-input",
        ] {
            assert!(
                extract
                    .arguments
                    .iter()
                    .any(|argument| argument == verified_input)
            );
        }
        assert!(
            !extract
                .arguments
                .iter()
                .any(|argument| argument == "--bind")
        );
        let python = extract
            .arguments
            .iter()
            .position(|argument| argument == GRAPHIFY_WSL_PYTHON_PATH)
            .expect("fixed Python entry point");
        assert_eq!(extract.arguments[python + 1], "-I");
        assert_eq!(extract.arguments[python + 2], "-S");
        assert_eq!(extract.arguments[python + 3], "-B");
        assert_eq!(extract.arguments[python + 4], "-c");
        assert_eq!(extract.arguments[python + 6], "extract");
        assert!(!extract.arguments.iter().any(|argument| argument == "-m"));
        for pair in [
            ("GRAPHIFY_QUERY_LOG_DISABLE", "1"),
            ("PYTHONDONTWRITEBYTECODE", "1"),
            ("PYTHONSAFEPATH", "1"),
        ] {
            assert!(PRIVATE_RUNNER_SOURCE.contains(&format!("\"{}\": \"{}\"", pair.0, pair.1)));
        }
        assert!(PRIVATE_RUNNER_SOURCE.contains("if version < 3:"));
        assert!(PRIVATE_RUNNER_SOURCE.contains("os.truncate(\"/runtime/install-report.json\", 0)"));
        for forbidden in [
            "OPENAI_API_KEY",
            "ANTHROPIC_API_KEY",
            "GOOGLE_API_KEY",
            "GEMINI_API_KEY",
            "AZURE_OPENAI_API_KEY",
            "WSLENV",
        ] {
            assert!(!extract.environment.contains_key(OsStr::new(forbidden)));
        }
    }

    #[test]
    fn fake_failures_close_on_timeout_nonzero_missing_malformed_and_zero_nodes() {
        let cases = [
            (FakeMode::Timeout, GraphifyAdapterErrorKind::Timeout),
            (FakeMode::NonZero, GraphifyAdapterErrorKind::NonZeroExit),
            (FakeMode::Missing, GraphifyAdapterErrorKind::MissingOutput),
            (
                FakeMode::Malformed,
                GraphifyAdapterErrorKind::MalformedOutput,
            ),
            (FakeMode::ZeroNodes, GraphifyAdapterErrorKind::EmptyAnalysis),
        ];
        for (mode, expected) in cases {
            let (mut adapter, snapshot, _) = fixture(mode);
            assert_eq!(
                adapter
                    .analyze_materialized(&snapshot)
                    .expect_err("failure must close")
                    .kind(),
                expected,
                "{mode:?}"
            );
        }
    }

    #[test]
    fn process_digest_normalization_removes_owned_snapshot_and_run_paths() {
        let snapshot = std::env::temp_dir().join("lattice-snapshot-identity");
        let first_graph = std::env::temp_dir()
            .join("lattice-run-one")
            .join("graphify-out/graph.json");
        let second_graph = std::env::temp_dir()
            .join("lattice-run-two")
            .join("graphify-out/graph.json");
        let first = format!(
            "scanning {}\nwrote {}\n",
            command_path(&snapshot).to_string_lossy(),
            command_path(&first_graph).to_string_lossy()
        );
        let second = format!(
            "scanning {}\nwrote {}\n",
            command_path(&snapshot).to_string_lossy(),
            command_path(&second_graph).to_string_lossy()
        );
        let first = normalized_process_stdout(first.as_bytes(), &snapshot, &first_graph)
            .expect("normalize first process output");
        let second = normalized_process_stdout(second.as_bytes(), &snapshot, &second_graph)
            .expect("normalize second process output");
        assert_eq!(first, second);
        assert_eq!(
            first,
            "scanning <LATTICE_CODE_SNAPSHOT>\nwrote <LATTICE_GRAPH_OUTPUT>\n"
        );
    }

    #[test]
    fn private_runner_and_frame_are_identity_bound_and_strict() {
        assert_eq!(
            crate::snapshot::sha256_bytes(PRIVATE_RUNNER_SOURCE.as_bytes()),
            GRAPHIFY_PRIVATE_RUNNER_SHA256
        );
        let fields: [&[u8]; PRIVATE_FRAME_FIELD_COUNT] = [
            b"version\n",
            b"",
            HELP,
            b"",
            b"extract\n",
            b"",
            valid_graph(),
        ];
        let mut frame = PRIVATE_FRAME_MAGIC.to_vec();
        for field in fields {
            frame.extend_from_slice(&(field.len() as u64).to_be_bytes());
            frame.extend_from_slice(field);
        }
        let parsed = parse_private_frame(&frame, GraphOutputLimits::default())
            .expect("one exact private frame");
        assert_eq!(parsed.graph, valid_graph());

        let mut trailing = frame.clone();
        trailing.push(0);
        assert_eq!(
            parse_private_frame(&trailing, GraphOutputLimits::default())
                .expect_err("trailing bytes must fail")
                .kind(),
            GraphifyAdapterErrorKind::MalformedOutput
        );
        assert_eq!(
            parse_private_frame(&frame[..frame.len() - 1], GraphOutputLimits::default())
                .expect_err("truncated frame must fail")
                .kind(),
            GraphifyAdapterErrorKind::MalformedOutput
        );
    }
}
