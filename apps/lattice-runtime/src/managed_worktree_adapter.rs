//! Process-isolated owner for one task-owned managed Git worktree.
//!
//! The JavaScript side reuses the existing `GitWorkspace` containment,
//! common-Git-directory, empty-hooks, and ownership-marker contract. This
//! adapter accepts only its closed baseline projection and turns those exact
//! bytes into the existing Artifact Store `GIT_SNAPSHOT` kind.

use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use lattice_artifact_store::{ManagedEvidenceInput, ManagedEvidenceKind, VerifiedManagedEvidence};
use lattice_codex_adapter::SupervisedDuplexChild;
use lattice_contracts::{ContentDigest, ProjectId, TaskId};
use lattice_ports::{
    ManagedPortError, ManagedPortErrorKind, ManagedPortResult, VerificationOutcome,
};
use lattice_postgres_foreman::ExecutionEnvironmentDescriptor;
use lattice_task_ledger::VerifiedTaskVerificationRecord;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::managed_file_identity::{
    ManagedEffectBundleGuard, ManagedFileIdentity, ManagedFileIdentityBundle, ManagedFileSeal,
};
use crate::managed_worker_adapter::ManagedWorkerCancellation;

pub(crate) const MANAGED_WORKTREE_BASELINE_SCHEMA: &str = "lattice.managed-worktree-baseline/1.0";
const COMMAND_SCHEMA: &str = "lattice.managed-worktree-command/1.1";
const RESULT_SCHEMA: &str = "lattice.managed-worktree-bridge-result/1.0";
const PROTECTED_RESULT_REF_DIGEST_DOMAIN: &str = "LATTICE_MANAGED_PROTECTED_RESULT_REF_V1";
const PRODUCER_ID: &str = "lattice-control-managed-worktree";
const PRODUCER_VERSION: &str = "1.0";
const MAX_BRIDGE_OUTPUT_BYTES: u64 = 32_768;
const MAX_BASELINE_BYTES: usize = 16_384;
const MAX_MANAGED_EXECUTABLE_BYTES: u64 = 512 * 1_024 * 1_024;
const MAX_MANAGED_WORKTREE_BRIDGE_BYTES: u64 = 4 * 1_024 * 1_024;
const MAX_MANAGED_WORKTREE_DEPENDENCY_BYTES: u64 = 8 * 1_024 * 1_024;
const MANAGED_GRACEFUL_SHUTDOWN_IDLE: &str = "LATTICE_MANAGED_GRACEFUL_SHUTDOWN_IDLE";

#[derive(Clone)]
pub(crate) struct ManagedWorktreeAdapterConfig {
    node_executable: PathBuf,
    node_identity: Option<ManagedFileIdentity>,
    bridge_path: PathBuf,
    bridge_bundle: Option<ManagedFileIdentityBundle>,
    git_executable: PathBuf,
    git_identity: Option<ManagedFileIdentity>,
    repository_root: PathBuf,
    worktree_root: PathBuf,
    timeout: Duration,
    cancellation: ManagedWorkerCancellation,
    runtime_bundle: Option<ManagedEffectBundleGuard>,
    execution_environment_descriptor: Option<ExecutionEnvironmentDescriptor>,
}

impl ManagedWorktreeAdapterConfig {
    pub(crate) fn new(
        node_executable: PathBuf,
        bridge_path: PathBuf,
        git_executable: PathBuf,
        repository_root: PathBuf,
        worktree_root: PathBuf,
        timeout: Duration,
    ) -> ManagedPortResult<Self> {
        Self::new_inner(
            node_executable,
            bridge_path,
            git_executable,
            repository_root,
            worktree_root,
            timeout,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_with_effect_bundle_guard(
        node_executable: PathBuf,
        bridge_path: PathBuf,
        git_executable: PathBuf,
        repository_root: PathBuf,
        worktree_root: PathBuf,
        timeout: Duration,
        runtime_bundle: ManagedEffectBundleGuard,
    ) -> ManagedPortResult<Self> {
        Self::new_inner(
            node_executable,
            bridge_path,
            git_executable,
            repository_root,
            worktree_root,
            timeout,
            Some(runtime_bundle),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn new_inner(
        node_executable: PathBuf,
        bridge_path: PathBuf,
        git_executable: PathBuf,
        repository_root: PathBuf,
        worktree_root: PathBuf,
        timeout: Duration,
        runtime_bundle: Option<ManagedEffectBundleGuard>,
    ) -> ManagedPortResult<Self> {
        if timeout.is_zero()
            || timeout > Duration::from_secs(300)
            || [
                &node_executable,
                &bridge_path,
                &git_executable,
                &repository_root,
                &worktree_root,
            ]
            .into_iter()
            .any(|path| !path.is_absolute())
            || paths_overlap(&repository_root, &worktree_root)
        {
            return Err(known("LATTICE_MANAGED_WORKTREE_CONFIGURATION_REJECTED"));
        }
        let bridge_directory = bridge_path
            .parent()
            .filter(|path| path.file_name().and_then(|name| name.to_str()) == Some("src"))
            .ok_or_else(|| known("LATTICE_MANAGED_WORKTREE_BRIDGE_IDENTITY_REJECTED"))?;
        let lattice_control = bridge_directory
            .parent()
            .filter(|path| {
                path.file_name().and_then(|name| name.to_str()) == Some("lattice-control")
            })
            .ok_or_else(|| known("LATTICE_MANAGED_WORKTREE_BRIDGE_IDENTITY_REJECTED"))?;
        let lattice_apps = lattice_control
            .parent()
            .filter(|path| path.file_name().and_then(|name| name.to_str()) == Some("apps"))
            .ok_or_else(|| known("LATTICE_MANAGED_WORKTREE_BRIDGE_IDENTITY_REJECTED"))?;
        let lattice_root = lattice_apps
            .parent()
            .ok_or_else(|| known("LATTICE_MANAGED_WORKTREE_BRIDGE_IDENTITY_REJECTED"))?;
        let dependency_paths = [
            bridge_path.clone(),
            bridge_directory.join("managed-worktree.mjs"),
            bridge_directory.join("wsl2-execution-domain.mjs"),
            bridge_directory.join("wsl2-execution-preflight.mjs"),
            lattice_root.join("src/domain/canonical-json.js"),
            lattice_root.join("src/workspace/errors.js"),
            lattice_root.join("src/workspace/git-workspace.js"),
        ];
        if let Some(guard) = &runtime_bundle {
            for path in std::iter::once(node_executable.as_path())
                .chain(std::iter::once(git_executable.as_path()))
                .chain(dependency_paths.iter().map(PathBuf::as_path))
            {
                guard
                    .covers_file(path)
                    .map_err(|_| known("LATTICE_MANAGED_WORKTREE_BUNDLE_IDENTITY_REJECTED"))?;
            }
        }
        let node_identity = runtime_bundle
            .is_none()
            .then(|| {
                ManagedFileIdentity::capture(&node_executable, MAX_MANAGED_EXECUTABLE_BYTES)
                    .map_err(|_| known("LATTICE_MANAGED_WORKTREE_NODE_IDENTITY_REJECTED"))
            })
            .transpose()?;
        let bridge_bundle = runtime_bundle
            .is_none()
            .then(|| {
                ManagedFileIdentityBundle::capture(
                    dependency_paths
                        .iter()
                        .cloned()
                        .enumerate()
                        .map(|(index, path)| {
                            (
                                path,
                                if index == 0 {
                                    MAX_MANAGED_WORKTREE_BRIDGE_BYTES
                                } else {
                                    MAX_MANAGED_WORKTREE_DEPENDENCY_BYTES
                                },
                            )
                        }),
                )
                .map_err(|_| known("LATTICE_MANAGED_WORKTREE_BRIDGE_IDENTITY_REJECTED"))
            })
            .transpose()?;
        let git_identity = runtime_bundle
            .is_none()
            .then(|| {
                ManagedFileIdentity::capture(&git_executable, MAX_MANAGED_EXECUTABLE_BYTES)
                    .map_err(|_| known("LATTICE_MANAGED_WORKTREE_GIT_IDENTITY_REJECTED"))
            })
            .transpose()?;
        Ok(Self {
            node_executable,
            node_identity,
            bridge_path,
            bridge_bundle,
            git_executable,
            git_identity,
            repository_root,
            worktree_root,
            timeout,
            cancellation: ManagedWorkerCancellation::default(),
            runtime_bundle,
            execution_environment_descriptor: None,
        })
    }

    pub(crate) fn with_execution_environment(
        mut self,
        descriptor: &ExecutionEnvironmentDescriptor,
    ) -> Self {
        self.execution_environment_descriptor = Some(descriptor.clone());
        self
    }

    pub(crate) fn with_cancellation(mut self, cancellation: ManagedWorkerCancellation) -> Self {
        self.cancellation = cancellation;
        self
    }

    fn command_context(&self) -> ManagedPortResult<ManagedWorktreeCommandContext<'_>> {
        Ok(ManagedWorktreeCommandContext {
            repository_root: path_text(&self.repository_root)?,
            worktree_root: path_text(&self.worktree_root)?,
            git_executable: path_text(&self.git_executable)?,
            expected_execution_environment_ref: self
                .execution_environment_descriptor
                .as_ref()
                .map(|descriptor| descriptor.environment_ref().as_str()),
        })
    }

    fn verify_effect_identity(&self) -> ManagedPortResult<()> {
        if let Some(bundle) = &self.runtime_bundle {
            return bundle
                .verify()
                .map_err(|_| known("LATTICE_MANAGED_WORKTREE_BUNDLE_IDENTITY_REJECTED"));
        }
        self.node_identity
            .as_ref()
            .ok_or_else(|| known("LATTICE_MANAGED_WORKTREE_NODE_IDENTITY_REJECTED"))?
            .verify()
            .map_err(|_| known("LATTICE_MANAGED_WORKTREE_NODE_IDENTITY_REJECTED"))?;
        self.bridge_bundle
            .as_ref()
            .ok_or_else(|| known("LATTICE_MANAGED_WORKTREE_BRIDGE_IDENTITY_REJECTED"))?
            .verify()
            .map_err(|_| known("LATTICE_MANAGED_WORKTREE_BRIDGE_IDENTITY_REJECTED"))?;
        self.git_identity
            .as_ref()
            .ok_or_else(|| known("LATTICE_MANAGED_WORKTREE_GIT_IDENTITY_REJECTED"))?
            .verify()
            .map_err(|_| known("LATTICE_MANAGED_WORKTREE_GIT_IDENTITY_REJECTED"))
    }

    fn seal_effect_identity(&self) -> ManagedPortResult<Option<ManagedFileSeal>> {
        if self.runtime_bundle.is_some() {
            self.verify_effect_identity()?;
            return Ok(None);
        }
        let mut seal = self
            .node_identity
            .as_ref()
            .ok_or_else(|| known("LATTICE_MANAGED_WORKTREE_NODE_IDENTITY_REJECTED"))?
            .seal()
            .map_err(|_| known("LATTICE_MANAGED_WORKTREE_NODE_IDENTITY_REJECTED"))?;
        seal.extend(
            self.bridge_bundle
                .as_ref()
                .ok_or_else(|| known("LATTICE_MANAGED_WORKTREE_BRIDGE_IDENTITY_REJECTED"))?
                .seal()
                .map_err(|_| known("LATTICE_MANAGED_WORKTREE_BRIDGE_IDENTITY_REJECTED"))?,
        );
        seal.extend(
            self.git_identity
                .as_ref()
                .ok_or_else(|| known("LATTICE_MANAGED_WORKTREE_GIT_IDENTITY_REJECTED"))?
                .seal()
                .map_err(|_| known("LATTICE_MANAGED_WORKTREE_GIT_IDENTITY_REJECTED"))?,
        );
        self.verify_effect_identity()?;
        Ok(Some(seal))
    }
}

#[derive(Clone, Copy)]
struct ManagedWorktreeCommandContext<'a> {
    repository_root: &'a str,
    worktree_root: &'a str,
    git_executable: &'a str,
    expected_execution_environment_ref: Option<&'a str>,
}

fn build_baseline_command(
    context: &ManagedWorktreeCommandContext<'_>,
    task_ref: &ContentDigest,
    task_id: &TaskId,
    base_commit: &str,
    expected_baseline: Option<&ContentDigest>,
) -> Value {
    json!({
        "schema": COMMAND_SCHEMA,
        "operation": if expected_baseline.is_some() { "verify" } else { "prepare" },
        "repository_root": context.repository_root,
        "worktree_root": context.worktree_root,
        "git_executable": context.git_executable,
        "task_ref": task_ref.as_str(),
        "task_id": task_id.as_str(),
        "base_commit": base_commit,
        "expected_baseline_sha256": expected_baseline.map(ContentDigest::as_str),
        "expected_execution_environment_ref": context.expected_execution_environment_ref,
    })
}

#[allow(clippy::too_many_arguments)]
fn build_protect_command(
    context: &ManagedWorktreeCommandContext<'_>,
    task_ref: &ContentDigest,
    task_id: &TaskId,
    attempt: u8,
    writer_fence: u64,
    base_commit: &str,
    result_commit: &str,
    baseline_digest: &ContentDigest,
    require_existing: bool,
) -> Value {
    json!({
        "schema": COMMAND_SCHEMA,
        "operation": "protect",
        "repository_root": context.repository_root,
        "worktree_root": context.worktree_root,
        "git_executable": context.git_executable,
        "task_ref": task_ref.as_str(),
        "task_id": task_id.as_str(),
        "attempt": attempt,
        "writer_fence": writer_fence,
        "base_commit": base_commit,
        "result_commit": result_commit,
        "expected_baseline_sha256": baseline_digest.as_str(),
        "expected_execution_environment_ref": context.expected_execution_environment_ref,
        "require_existing": require_existing,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ManagedWorktreeBaseline {
    worktree_path: PathBuf,
    worktree_id: String,
    branch: String,
    replayed: bool,
    evidence: VerifiedManagedEvidence,
}

impl ManagedWorktreeBaseline {
    pub(crate) fn worktree_path(&self) -> &Path {
        &self.worktree_path
    }

    pub(crate) fn worktree_id(&self) -> &str {
        &self.worktree_id
    }

    pub(crate) fn branch(&self) -> &str {
        &self.branch
    }

    pub(crate) const fn replayed(&self) -> bool {
        self.replayed
    }

    pub(crate) const fn evidence(&self) -> &VerifiedManagedEvidence {
        &self.evidence
    }

    pub(crate) const fn content_digest(&self) -> &ContentDigest {
        self.evidence.content_digest()
    }
}

pub(crate) struct ManagedWorktreeAdapter {
    config: ManagedWorktreeAdapterConfig,
}

impl ManagedWorktreeAdapter {
    pub(crate) const fn new(config: ManagedWorktreeAdapterConfig) -> Self {
        Self { config }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare(
        &self,
        project_id: ProjectId,
        task_ref: ContentDigest,
        task_id: &TaskId,
        attempt: u8,
        base_commit: &str,
        created_at: &str,
        expected_baseline: Option<&ContentDigest>,
    ) -> ManagedPortResult<ManagedWorktreeBaseline> {
        if !(1..=3).contains(&attempt) || !is_oid(base_commit) {
            return Err(known("LATTICE_MANAGED_WORKTREE_REQUEST_REJECTED"));
        }
        let command = build_baseline_command(
            &self.config.command_context()?,
            &task_ref,
            task_id,
            base_commit,
            expected_baseline,
        );
        let response = self.run_bridge(&command)?;
        baseline_from_response(
            &self.config,
            response,
            project_id,
            task_ref,
            task_id,
            attempt,
            base_commit,
            created_at,
            expected_baseline,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn protect_verified_result(
        &self,
        project_id: &ProjectId,
        task_ref: &ContentDigest,
        task_id: &TaskId,
        attempt: u8,
        writer_fence: u64,
        base_commit: &str,
        baseline_digest: &ContentDigest,
        verification: &VerifiedTaskVerificationRecord,
        snapshot: &VerifiedManagedEvidence,
        require_existing: bool,
    ) -> ManagedPortResult<ProtectedManagedResult> {
        if !(1..=3).contains(&attempt)
            || writer_fence == 0
            || !is_oid(base_commit)
            || verification.task_ref() != task_ref
            || verification.attempt_number() != u64::from(attempt)
            || verification.outcome() != VerificationOutcome::Passed
            || verification.review_digest().is_none()
            || snapshot.project_id() != project_id
            || snapshot.task_ref() != task_ref
            || snapshot.attempt() != attempt
            || snapshot.kind() != ManagedEvidenceKind::GitSnapshot
            || snapshot.payload_schema() != "lattice.managed-git-snapshot/1.0"
            || snapshot.descriptor_digest() != verification.evidence_artifact_digest()
        {
            return Err(known("LATTICE_MANAGED_PROTECTED_REF_REJECTED"));
        }
        let result_commit = verified_result_commit(
            snapshot.bytes(),
            base_commit,
            verification.base_commit_digest(),
            verification.result_commit_digest(),
            verification.tree_digest(),
            verification.diff_digest(),
        )?;
        let command = build_protect_command(
            &self.config.command_context()?,
            task_ref,
            task_id,
            attempt,
            writer_fence,
            base_commit,
            &result_commit,
            baseline_digest,
            require_existing,
        );
        protected_result_from_response(
            self.run_bridge(&command)?,
            task_ref,
            task_id,
            attempt,
            writer_fence,
            base_commit,
            &result_commit,
            baseline_digest,
        )
    }

    fn run_bridge(&self, command: &Value) -> ManagedPortResult<Value> {
        self.run_bridge_with_post_spawn_hook(command, || {})
    }

    fn run_bridge_with_post_spawn_hook(
        &self,
        command: &Value,
        post_spawn_hook: impl FnOnce(),
    ) -> ManagedPortResult<Value> {
        if self.config.cancellation.is_requested() {
            return Err(known(MANAGED_GRACEFUL_SHUTDOWN_IDLE));
        }
        // The seal is acquired before process creation and remains live until
        // the supervised subtree has been reaped and its reader joined.  A
        // same-user writer therefore cannot ABA-swap an ESM import between
        // replay and Node's deferred module load.
        let _effect_seal = self.config.seal_effect_identity()?;
        if self.config.cancellation.is_requested() {
            return Err(known(MANAGED_GRACEFUL_SHUTDOWN_IDLE));
        }
        let mut process = Command::new(&self.config.node_executable);
        process
            .arg(&self.config.bridge_path)
            .current_dir(&self.config.repository_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        configure_environment(
            &mut process,
            self.config
                .execution_environment_descriptor
                .as_ref()
                .map(ExecutionEnvironmentDescriptor::as_json),
        );
        let mut child = SupervisedDuplexChild::spawn_cleared(&mut process)
            .map_err(|_| known("LATTICE_MANAGED_WORKTREE_BRIDGE_REJECTED"))?;
        post_spawn_hook();
        let mut reader = None;
        let mut reader_failed = None;
        let operation_result = (|| -> ManagedPortResult<_> {
            // Node resolves the entrypoint and ESM graph after process creation.
            // Replay the full identity set after spawn and before the first
            // effect-bearing command is written.
            self.config.verify_effect_identity()?;
            let bridge_stdout = child
                .take_stdout()
                .ok_or_else(|| known("LATTICE_MANAGED_WORKTREE_BRIDGE_REJECTED"))?;
            let (failed, handle) = spawn_bounded_bridge_reader(bridge_stdout);
            reader_failed = Some(failed);
            reader = Some(handle);
            let mut stdin = child
                .take_stdin()
                .ok_or_else(|| known("LATTICE_MANAGED_WORKTREE_BRIDGE_REJECTED"))?;
            serde_json::to_writer(&mut stdin, command)
                .and_then(|()| stdin.write_all(b"\n").map_err(serde_json::Error::io))
                .and_then(|()| stdin.flush().map_err(serde_json::Error::io))
                .map_err(|_| known("LATTICE_MANAGED_WORKTREE_BRIDGE_REJECTED"))?;
            drop(stdin);
            let deadline = Instant::now()
                .checked_add(self.config.timeout)
                .ok_or_else(|| known("LATTICE_MANAGED_WORKTREE_BRIDGE_REJECTED"))?;
            loop {
                if self.config.cancellation.is_requested() {
                    return Err(known(MANAGED_GRACEFUL_SHUTDOWN_IDLE));
                }
                if reader_failed
                    .as_ref()
                    .is_some_and(|failed| failed.load(Ordering::Acquire))
                {
                    return Err(known("LATTICE_MANAGED_WORKTREE_BRIDGE_REJECTED"));
                }
                if let Some(status) = child
                    .try_wait()
                    .map_err(|_| known("LATTICE_MANAGED_WORKTREE_BRIDGE_REJECTED"))?
                {
                    return Ok(status);
                }
                if Instant::now() >= deadline {
                    return Err(known("LATTICE_MANAGED_WORKTREE_BRIDGE_TIMEOUT"));
                }
                thread::sleep(Duration::from_millis(10));
            }
        })();
        let bytes = cleanup_supervised_bridge(child, reader)?;
        let status = operation_result?;
        let bytes = bytes.ok_or_else(|| known("LATTICE_MANAGED_WORKTREE_BRIDGE_REJECTED"))?;
        if bytes.is_empty() {
            return Err(known("LATTICE_MANAGED_WORKTREE_BRIDGE_REJECTED"));
        }
        let response: Value = serde_json::from_slice(&bytes)
            .map_err(|_| known("LATTICE_MANAGED_WORKTREE_BRIDGE_REJECTED"))?;
        if !status.success() || response.get("kind").and_then(Value::as_str) != Some("result") {
            return Err(match response.get("code").and_then(Value::as_str) {
                Some("MANAGED_WORKTREE_BASELINE_SUBSTITUTION") => {
                    known("LATTICE_MANAGED_WORKTREE_BASELINE_DRIFT")
                }
                Some("MANAGED_WORKTREE_BASELINE_DRIFT")
                | Some("MANAGED_WORKTREE_GIT_POINTER_UNSAFE")
                | Some("MANAGED_WORKTREE_INDEX_UNSAFE")
                | Some("MANAGED_WORKTREE_CONTROL_UNSAFE") => {
                    known("LATTICE_MANAGED_WORKTREE_CONTROL_DRIFT")
                }
                _ => known("LATTICE_MANAGED_WORKTREE_BRIDGE_REJECTED"),
            });
        }
        Ok(response)
    }
}

fn spawn_bounded_bridge_reader(
    stdout: Box<dyn Read + Send>,
) -> (Arc<AtomicBool>, thread::JoinHandle<io::Result<Vec<u8>>>) {
    let failed = Arc::new(AtomicBool::new(false));
    let reader_failed = Arc::clone(&failed);
    let reader = thread::spawn(move || {
        let _activity = WorktreeReaderActivity::new();
        let result = read_bounded_bridge_output(stdout);
        if result.is_err() {
            reader_failed.store(true, Ordering::Release);
        }
        result
    });
    (failed, reader)
}

fn read_bounded_bridge_output(mut stdout: impl Read) -> io::Result<Vec<u8>> {
    let limit = usize::try_from(MAX_BRIDGE_OUTPUT_BYTES)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid bridge output bound"))?;
    let mut bytes = Vec::with_capacity(limit.min(8 * 1_024));
    let mut buffer = [0_u8; 4 * 1_024];
    loop {
        let count = stdout.read(&mut buffer)?;
        if count == 0 {
            return Ok(bytes);
        }
        if bytes
            .len()
            .checked_add(count)
            .is_none_or(|total| total > limit)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "managed worktree bridge output limit exceeded",
            ));
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
}

fn cleanup_supervised_bridge(
    mut child: SupervisedDuplexChild,
    reader: Option<thread::JoinHandle<io::Result<Vec<u8>>>>,
) -> ManagedPortResult<Option<Vec<u8>>> {
    let cleanup = child
        .terminate_and_reap()
        .map_err(|_| known("LATTICE_MANAGED_WORKTREE_BRIDGE_REJECTED"));
    // Closing the supervised process owner after the explicit proof also
    // closes every inherited pipe handle if the proof itself failed.
    drop(child);
    let output = reader
        .map(|reader| {
            reader
                .join()
                .map_err(|_| known("LATTICE_MANAGED_WORKTREE_BRIDGE_REJECTED"))?
                .map_err(|_| known("LATTICE_MANAGED_WORKTREE_BRIDGE_REJECTED"))
        })
        .transpose()?;
    cleanup?;
    Ok(output)
}

struct WorktreeReaderActivity;

impl WorktreeReaderActivity {
    fn new() -> Self {
        #[cfg(test)]
        ACTIVE_WORKTREE_READERS.fetch_add(1, Ordering::AcqRel);
        Self
    }
}

impl Drop for WorktreeReaderActivity {
    fn drop(&mut self) {
        #[cfg(test)]
        ACTIVE_WORKTREE_READERS.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
static ACTIVE_WORKTREE_READERS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProtectedManagedResult {
    protected_ref: String,
    result_commit: String,
    writer_fence: u64,
    evidence_digest: ContentDigest,
    replayed: bool,
}

impl ProtectedManagedResult {
    #[cfg(test)]
    pub(crate) fn test_value(
        protected_ref: String,
        result_commit: String,
        writer_fence: u64,
        evidence_digest: ContentDigest,
        replayed: bool,
    ) -> Self {
        Self {
            protected_ref,
            result_commit,
            writer_fence,
            evidence_digest,
            replayed,
        }
    }

    pub(crate) fn protected_ref(&self) -> &str {
        &self.protected_ref
    }

    pub(crate) fn result_commit(&self) -> &str {
        &self.result_commit
    }

    pub(crate) const fn writer_fence(&self) -> u64 {
        self.writer_fence
    }

    pub(crate) const fn evidence_digest(&self) -> &ContentDigest {
        &self.evidence_digest
    }

    pub(crate) const fn replayed(&self) -> bool {
        self.replayed
    }
}

fn verified_result_commit(
    bytes: &[u8],
    base_commit: &str,
    base_commit_digest: &ContentDigest,
    result_commit_digest: &ContentDigest,
    tree_digest: &ContentDigest,
    expected_diff_digest: &ContentDigest,
) -> ManagedPortResult<String> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|_| known("LATTICE_MANAGED_PROTECTED_REF_REJECTED"))?;
    let object = value
        .as_object()
        .ok_or_else(|| known("LATTICE_MANAGED_PROTECTED_REF_REJECTED"))?;
    let keys = [
        "schema",
        "base_commit",
        "result_commit",
        "tree",
        "diff_digest",
        "command_identity",
        "changed_paths",
        "checks",
    ];
    let result_commit = value
        .get("result_commit")
        .and_then(Value::as_str)
        .filter(|value| is_oid(value))
        .ok_or_else(|| known("LATTICE_MANAGED_PROTECTED_REF_REJECTED"))?;
    let tree = value
        .get("tree")
        .and_then(Value::as_str)
        .filter(|value| is_oid(value))
        .ok_or_else(|| known("LATTICE_MANAGED_PROTECTED_REF_REJECTED"))?;
    let changed_paths = value
        .get("changed_paths")
        .and_then(Value::as_array)
        .filter(|values| {
            !values.is_empty()
                && values.len() <= 256
                && values.iter().all(|path| {
                    path.as_str()
                        .is_some_and(|path| valid_repository_path(path))
                })
        })
        .ok_or_else(|| known("LATTICE_MANAGED_PROTECTED_REF_REJECTED"))?;
    let checks = value
        .get("checks")
        .and_then(Value::as_array)
        .filter(|values| {
            !values.is_empty()
                && values.len() <= 32
                && values.iter().all(|check| {
                    check.as_object().is_some_and(|check| {
                        check.len() == 2
                            && check.get("id").and_then(Value::as_str).is_some_and(|id| {
                                !id.is_empty()
                                    && id.len() <= 128
                                    && id.bytes().all(|byte| {
                                        byte.is_ascii_alphanumeric()
                                            || matches!(byte, b'-' | b'_' | b'.')
                                    })
                            })
                            && check.get("passed").and_then(Value::as_bool) == Some(true)
                    })
                })
        })
        .ok_or_else(|| known("LATTICE_MANAGED_PROTECTED_REF_REJECTED"))?;
    let _ = (changed_paths, checks);
    if object.len() != keys.len()
        || keys.iter().any(|key| !object.contains_key(*key))
        || value.get("schema").and_then(Value::as_str) != Some("lattice.managed-git-snapshot/1.0")
        || value.get("base_commit").and_then(Value::as_str) != Some(base_commit)
        || value.get("diff_digest").and_then(Value::as_str) != Some(expected_diff_digest.as_str())
        || value
            .get("command_identity")
            .and_then(Value::as_str)
            .and_then(|value| ContentDigest::from_sha256(value.to_owned()).ok())
            .is_none_or(|value| value.as_str().bytes().all(|byte| byte == b'0'))
        || &digest_bytes(base_commit.as_bytes())? != base_commit_digest
        || &digest_bytes(result_commit.as_bytes())? != result_commit_digest
        || &digest_bytes(tree.as_bytes())? != tree_digest
    {
        return Err(known("LATTICE_MANAGED_PROTECTED_REF_REJECTED"));
    }
    Ok(result_commit.to_owned())
}

fn valid_repository_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 1_024
        && !value.starts_with('/')
        && !value.ends_with('/')
        && !value.contains(['\\', '\0'])
        && value
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

#[allow(clippy::too_many_arguments)]
fn protected_result_from_response(
    response: Value,
    task_ref: &ContentDigest,
    task_id: &TaskId,
    attempt: u8,
    writer_fence: u64,
    base_commit: &str,
    result_commit: &str,
    baseline_digest: &ContentDigest,
) -> ManagedPortResult<ProtectedManagedResult> {
    let object = response
        .as_object()
        .ok_or_else(|| known("LATTICE_MANAGED_PROTECTED_REF_REJECTED"))?;
    let keys = [
        "schema",
        "kind",
        "operation",
        "task_ref",
        "task_id",
        "attempt",
        "writer_fence",
        "base_commit",
        "result_commit",
        "worktree_path",
        "protected_ref",
        "baseline_sha256",
        "replayed",
        "protected_ref_digest",
    ];
    let expected_ref = format!(
        "refs/lattice/managed/{}/attempt-{attempt}",
        task_ref.as_str()
    );
    if object.len() != keys.len()
        || keys.iter().any(|key| !object.contains_key(*key))
        || response.get("schema").and_then(Value::as_str) != Some(RESULT_SCHEMA)
        || response.get("kind").and_then(Value::as_str) != Some("result")
        || response.get("operation").and_then(Value::as_str) != Some("protect")
        || response.get("task_ref").and_then(Value::as_str) != Some(task_ref.as_str())
        || response.get("task_id").and_then(Value::as_str) != Some(task_id.as_str())
        || response.get("attempt").and_then(Value::as_u64) != Some(u64::from(attempt))
        || response.get("writer_fence").and_then(Value::as_u64) != Some(writer_fence)
        || response.get("base_commit").and_then(Value::as_str) != Some(base_commit)
        || response.get("result_commit").and_then(Value::as_str) != Some(result_commit)
        || response.get("protected_ref").and_then(Value::as_str) != Some(&expected_ref)
        || response.get("baseline_sha256").and_then(Value::as_str) != Some(baseline_digest.as_str())
        || response.get("replayed").and_then(Value::as_bool).is_none()
    {
        return Err(known("LATTICE_MANAGED_PROTECTED_REF_REJECTED"));
    }
    let evidence_digest = response
        .get("protected_ref_digest")
        .and_then(Value::as_str)
        .filter(|value| is_digest(value))
        .ok_or_else(|| known("LATTICE_MANAGED_PROTECTED_REF_REJECTED"))?;
    let expected_evidence_digest = framed_digest(
        PROTECTED_RESULT_REF_DIGEST_DOMAIN,
        &[
            task_ref.as_str(),
            &attempt.to_string(),
            &writer_fence.to_string(),
            &expected_ref,
            result_commit,
            baseline_digest.as_str(),
        ],
    )?;
    if evidence_digest != expected_evidence_digest.as_str() {
        return Err(known("LATTICE_MANAGED_PROTECTED_REF_REJECTED"));
    }
    Ok(ProtectedManagedResult {
        protected_ref: expected_ref,
        result_commit: result_commit.to_owned(),
        writer_fence,
        evidence_digest: ContentDigest::from_sha256(evidence_digest.to_owned())
            .map_err(|_| known("LATTICE_MANAGED_PROTECTED_REF_REJECTED"))?,
        replayed: response
            .get("replayed")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn baseline_from_response(
    config: &ManagedWorktreeAdapterConfig,
    response: Value,
    project_id: ProjectId,
    task_ref: ContentDigest,
    task_id: &TaskId,
    attempt: u8,
    base_commit: &str,
    created_at: &str,
    expected_baseline: Option<&ContentDigest>,
) -> ManagedPortResult<ManagedWorktreeBaseline> {
    let object = response
        .as_object()
        .ok_or_else(|| known("LATTICE_MANAGED_WORKTREE_RESPONSE_REJECTED"))?;
    let expected_keys = [
        "schema",
        "kind",
        "operation",
        "task_ref",
        "task_id",
        "base_commit",
        "worktree_id",
        "worktree_path",
        "branch",
        "replayed",
        "baseline_json",
        "baseline_sha256",
    ];
    if object.len() != expected_keys.len()
        || expected_keys.iter().any(|key| !object.contains_key(*key))
        || response.get("schema").and_then(Value::as_str) != Some(RESULT_SCHEMA)
        || response.get("kind").and_then(Value::as_str) != Some("result")
        || response.get("task_ref").and_then(Value::as_str) != Some(task_ref.as_str())
        || response.get("task_id").and_then(Value::as_str) != Some(task_id.as_str())
        || response.get("base_commit").and_then(Value::as_str) != Some(base_commit)
        || response.get("replayed").and_then(Value::as_bool).is_none()
    {
        return Err(known("LATTICE_MANAGED_WORKTREE_RESPONSE_REJECTED"));
    }
    let baseline_json = response
        .get("baseline_json")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= MAX_BASELINE_BYTES)
        .ok_or_else(|| known("LATTICE_MANAGED_WORKTREE_RESPONSE_REJECTED"))?;
    let baseline_sha256 = response
        .get("baseline_sha256")
        .and_then(Value::as_str)
        .filter(|value| is_digest(value))
        .ok_or_else(|| known("LATTICE_MANAGED_WORKTREE_RESPONSE_REJECTED"))?;
    let computed = digest_bytes(baseline_json.as_bytes())?;
    if computed.as_str() != baseline_sha256
        || expected_baseline.is_some_and(|expected| expected != &computed)
    {
        return Err(known("LATTICE_MANAGED_WORKTREE_BASELINE_DRIFT"));
    }
    validate_baseline_json(baseline_json, task_ref.as_str(), base_commit)?;
    let worktree_path = PathBuf::from(
        response
            .get("worktree_path")
            .and_then(Value::as_str)
            .ok_or_else(|| known("LATTICE_MANAGED_WORKTREE_RESPONSE_REJECTED"))?,
    );
    let worktree_path = fs::canonicalize(&worktree_path)
        .map_err(|_| known("LATTICE_MANAGED_WORKTREE_RESPONSE_REJECTED"))?;
    let worktree_root = fs::canonicalize(&config.worktree_root)
        .map_err(|_| known("LATTICE_MANAGED_WORKTREE_RESPONSE_REJECTED"))?;
    if !contained_by(&worktree_root, &worktree_path) || worktree_path == worktree_root {
        return Err(known("LATTICE_MANAGED_WORKTREE_RESPONSE_REJECTED"));
    }
    let worktree_id = response
        .get("worktree_id")
        .and_then(Value::as_str)
        .filter(|value| valid_identifier(value))
        .ok_or_else(|| known("LATTICE_MANAGED_WORKTREE_RESPONSE_REJECTED"))?
        .to_owned();
    if worktree_path.file_name().and_then(|value| value.to_str())
        != Some(worktree_id.to_ascii_lowercase().as_str())
    {
        return Err(known("LATTICE_MANAGED_WORKTREE_RESPONSE_REJECTED"));
    }
    let branch = response
        .get("branch")
        .and_then(Value::as_str)
        .filter(|value| valid_branch(value))
        .ok_or_else(|| known("LATTICE_MANAGED_WORKTREE_RESPONSE_REJECTED"))?
        .to_owned();
    let producer_digest = digest_bytes(b"lattice-control-managed-worktree/1.0")?;
    let input = ManagedEvidenceInput::new(
        project_id,
        task_ref,
        attempt,
        ManagedEvidenceKind::GitSnapshot,
        "application/json",
        MANAGED_WORKTREE_BASELINE_SCHEMA,
        PRODUCER_ID,
        PRODUCER_VERSION,
        producer_digest,
        created_at,
        baseline_json.as_bytes().to_vec(),
    )
    .map_err(|_| known("LATTICE_MANAGED_WORKTREE_EVIDENCE_REJECTED"))?;
    let evidence = VerifiedManagedEvidence::new(input)
        .map_err(|_| known("LATTICE_MANAGED_WORKTREE_EVIDENCE_REJECTED"))?;
    if evidence.content_digest() != &computed {
        return Err(known("LATTICE_MANAGED_WORKTREE_EVIDENCE_REJECTED"));
    }
    Ok(ManagedWorktreeBaseline {
        worktree_path,
        worktree_id,
        branch,
        replayed: response
            .get("replayed")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        evidence,
    })
}

fn validate_baseline_json(bytes: &str, task_ref: &str, base_commit: &str) -> ManagedPortResult<()> {
    let value: Value = serde_json::from_str(bytes)
        .map_err(|_| known("LATTICE_MANAGED_WORKTREE_EVIDENCE_REJECTED"))?;
    let object = value
        .as_object()
        .ok_or_else(|| known("LATTICE_MANAGED_WORKTREE_EVIDENCE_REJECTED"))?;
    let keys = [
        "schema",
        "task_ref",
        "ownership_digest",
        "repository_locator_digest",
        "worktree_locator_digest",
        "base_commit",
        "base_tree",
        "task_branch",
        "head_commit",
        "head_tree",
        "git_pointer_digest",
        "git_directory_locator_digest",
        "common_git_directory_locator_digest",
        "index_digest",
        "git_control_digest",
        "initial_worktree_state",
    ];
    if object.len() != keys.len()
        || keys.iter().any(|key| !object.contains_key(*key))
        || value.get("schema").and_then(Value::as_str) != Some(MANAGED_WORKTREE_BASELINE_SCHEMA)
        || value.get("task_ref").and_then(Value::as_str) != Some(task_ref)
        || value.get("base_commit").and_then(Value::as_str) != Some(base_commit)
        || value.get("head_commit").and_then(Value::as_str) != Some(base_commit)
        || value.get("base_tree") != value.get("head_tree")
        || value.get("initial_worktree_state").and_then(Value::as_str) != Some("CLEAN")
        || !value
            .get("task_branch")
            .and_then(Value::as_str)
            .is_some_and(valid_branch)
        || [
            "ownership_digest",
            "repository_locator_digest",
            "worktree_locator_digest",
            "git_pointer_digest",
            "git_directory_locator_digest",
            "common_git_directory_locator_digest",
            "index_digest",
            "git_control_digest",
        ]
        .into_iter()
        .any(|key| {
            !value
                .get(key)
                .and_then(Value::as_str)
                .is_some_and(is_digest)
        })
        || !value
            .get("base_tree")
            .and_then(Value::as_str)
            .is_some_and(is_oid)
    {
        return Err(known("LATTICE_MANAGED_WORKTREE_EVIDENCE_REJECTED"));
    }
    Ok(())
}

fn configure_environment(command: &mut Command, execution_environment: Option<&str>) {
    command.env_clear();
    for name in ["SystemRoot", "WINDIR"] {
        if let Some(value) = env::var_os(name) {
            command.env(name, value);
        }
    }
    command.env("GIT_CONFIG_NOSYSTEM", "1");
    command.env(
        "GIT_CONFIG_GLOBAL",
        if cfg!(windows) { "NUL" } else { "/dev/null" },
    );
    command.env("GIT_TERMINAL_PROMPT", "0");
    if let Some(descriptor) = execution_environment {
        if descriptor.len() <= 16_384
            && serde_json::from_str::<Value>(&descriptor).is_ok_and(|value| {
                value.get("schema").and_then(Value::as_str)
                    == Some("lattice.execution-environment.wsl2-linux/1.1")
                    && value.get("kind").and_then(Value::as_str) == Some("WSL2_LINUX")
            })
        {
            command.env("LATTICE_MANAGED_EXECUTION_ENVIRONMENT_JSON", descriptor);
        }
    }
}

fn path_text(path: &Path) -> ManagedPortResult<&str> {
    let text = path
        .to_str()
        .filter(|value| !value.is_empty() && !value.contains('\0'))
        .ok_or_else(|| known("LATTICE_MANAGED_WORKTREE_CONFIGURATION_REJECTED"))?;
    #[cfg(windows)]
    {
        if let Some(non_verbatim) = text.strip_prefix(r"\\?\") {
            if non_verbatim.starts_with("UNC\\") {
                return Err(known("LATTICE_MANAGED_WORKTREE_CONFIGURATION_REJECTED"));
            }
            return Ok(non_verbatim);
        }
    }
    Ok(text)
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    contained_by(left, right) || contained_by(right, left)
}

fn contained_by(parent: &Path, child: &Path) -> bool {
    child.strip_prefix(parent).is_ok_and(|relative| {
        relative
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
            || relative.as_os_str().is_empty()
    })
}

fn valid_identifier(value: &str) -> bool {
    (3..=64).contains(&value.len())
        && value.bytes().all(|byte| {
            byte.is_ascii_uppercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
}

fn valid_branch(value: &str) -> bool {
    value.strip_prefix("lattice/task-").is_some_and(|suffix| {
        (3..=64).contains(&suffix.len())
            && suffix.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
            })
    })
}

fn is_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_oid(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn digest_bytes(bytes: &[u8]) -> ManagedPortResult<ContentDigest> {
    let encoded = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    ContentDigest::from_sha256(encoded)
        .map_err(|_| known("LATTICE_MANAGED_WORKTREE_EVIDENCE_REJECTED"))
}

fn framed_digest(domain: &str, parts: &[&str]) -> ManagedPortResult<ContentDigest> {
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    hasher.update([0]);
    for part in parts {
        let bytes = part.as_bytes();
        hasher.update(
            u64::try_from(bytes.len())
                .map_err(|_| known("LATTICE_MANAGED_WORKTREE_EVIDENCE_REJECTED"))?
                .to_be_bytes(),
        );
        hasher.update(bytes);
    }
    let encoded = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    ContentDigest::from_sha256(encoded)
        .map_err(|_| known("LATTICE_MANAGED_WORKTREE_EVIDENCE_REJECTED"))
}

const fn known(code: &'static str) -> ManagedPortError {
    ManagedPortError::new(ManagedPortErrorKind::Known, code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    static TRANSPORT_FIXTURE_SEQUENCE: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(1);
    #[cfg(windows)]
    static TRANSPORT_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[cfg(windows)]
    fn test_node_executable() -> PathBuf {
        let output = Command::new("where.exe")
            .arg("node.exe")
            .output()
            .expect("locate Node for supervised transport test");
        assert!(output.status.success());
        String::from_utf8(output.stdout)
            .expect("where output")
            .lines()
            .find(|line| !line.trim().is_empty())
            .map(|line| PathBuf::from(line.trim()))
            .expect("absolute Node path")
    }

    #[cfg(windows)]
    fn test_git_executable() -> PathBuf {
        let output = Command::new("where.exe")
            .arg("git.exe")
            .output()
            .expect("locate Git for supervised transport test");
        assert!(output.status.success());
        String::from_utf8(output.stdout)
            .expect("where output")
            .lines()
            .find(|line| !line.trim().is_empty())
            .map(|line| PathBuf::from(line.trim()))
            .expect("absolute Git path")
    }

    #[cfg(windows)]
    fn transport_fixture(
        bridge_source: &str,
        timeout: Duration,
    ) -> (PathBuf, ManagedWorktreeAdapter, PathBuf) {
        let root = env::temp_dir().join(format!(
            "lattice-managed-worktree-transport-{}-{}",
            std::process::id(),
            TRANSPORT_FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let lattice_root = root.join("lattice");
        let bridge_directory = lattice_root.join("apps/lattice-control/src");
        let dependency = bridge_directory.join("managed-worktree.mjs");
        let target_repository = root.join("target-repository");
        let worktree_root = root.join("managed-worktrees");
        for directory in [
            &bridge_directory,
            &lattice_root.join("src/domain"),
            &lattice_root.join("src/workspace"),
            &target_repository,
            &worktree_root,
        ] {
            fs::create_dir_all(directory).expect("transport fixture directory");
        }
        let bridge_path = bridge_directory.join("managed-worktree-bridge.mjs");
        fs::write(&bridge_path, bridge_source).expect("transport bridge");
        fs::write(&dependency, b"export const pinned = true;\n").expect("worktree dependency");
        fs::write(
            bridge_directory.join("wsl2-execution-domain.mjs"),
            b"export const validateWsl2ExecutionEnvironment = value => value;\nexport const windowsWslPathToLinux = value => value;\n",
        )
        .expect("execution-domain dependency");
        fs::write(
            bridge_directory.join("wsl2-execution-preflight.mjs"),
            b"export const pinnedPreflight = true;\n",
        )
        .expect("execution-preflight dependency");
        fs::write(
            lattice_root.join("src/domain/canonical-json.js"),
            b"export const canonicalize = value => value;\n",
        )
        .expect("canonical dependency");
        fs::write(
            lattice_root.join("src/workspace/errors.js"),
            b"export class WorkspaceError extends Error {}\n",
        )
        .expect("error dependency");
        fs::write(
            lattice_root.join("src/workspace/git-workspace.js"),
            b"export class GitWorkspace {}\n",
        )
        .expect("git workspace dependency");
        let git = root.join("pinned-git.exe");
        fs::write(&git, b"test-only-git-identity\n").expect("git identity");
        let config = ManagedWorktreeAdapterConfig::new(
            test_node_executable(),
            bridge_path,
            git,
            target_repository,
            worktree_root,
            timeout,
        )
        .expect("worktree transport config");
        (root, ManagedWorktreeAdapter::new(config), dependency)
    }

    #[test]
    fn bridge_transport_has_one_cleanup_exit_and_a_streaming_hard_cap() {
        let source = include_str!("managed_worktree_adapter.rs");
        let transport = source
            .split("    fn run_bridge(")
            .nth(1)
            .expect("managed worktree transport")
            .split("\n}\n\n#[derive(Clone, Debug, Eq, PartialEq)]")
            .next()
            .expect("transport body");
        assert!(transport.contains("read_bounded_bridge_output"));
        assert!(transport.contains("operation_result"));
        assert!(transport.contains("cleanup_supervised_bridge"));
        assert!(transport.contains("cancellation.is_requested()"));
        let cancellation = transport
            .find("cancellation.is_requested()")
            .expect("typed scheduler cancellation");
        let cleanup = transport
            .rfind("cleanup_supervised_bridge")
            .expect("common cleanup after cancellation");
        assert!(cancellation < cleanup);
        assert!(!transport.contains("std::io::copy"));
        assert!(!transport.contains("fs::read(&output_path)"));
    }

    #[test]
    fn worktree_bridge_seals_transitive_local_imports_before_spawn_and_replays_before_effect() {
        let source = include_str!("managed_worktree_adapter.rs");
        assert!(source.contains("managed-worktree.mjs"));
        assert!(source.contains("wsl2-execution-domain.mjs"));
        assert!(source.contains("wsl2-execution-preflight.mjs"));
        assert!(source.contains("src/domain/canonical-json.js"));
        assert!(source.contains("src/workspace/errors.js"));
        assert!(source.contains("src/workspace/git-workspace.js"));
        let transport = source
            .split("    fn run_bridge(")
            .nth(1)
            .expect("managed worktree transport")
            .split("\n}\n\n#[derive(Clone, Debug, Eq, PartialEq)]")
            .next()
            .expect("transport body");
        let spawn = transport
            .find("SupervisedDuplexChild::spawn")
            .expect("supervised spawn");
        let seal = transport
            .find("seal_effect_identity")
            .expect("pre-spawn immutable bundle seal");
        let post_spawn = transport[spawn..]
            .find("verify_effect_identity")
            .expect("post-spawn identity replay")
            + spawn;
        let write = transport
            .find("serde_json::to_writer")
            .expect("effect write");
        assert!(seal < spawn && spawn < post_spawn && post_spawn < write);
    }

    #[test]
    fn worktree_commands_bind_the_typed_execution_environment_ref() {
        let native = ManagedWorktreeCommandContext {
            repository_root: r"C:\source",
            worktree_root: r"C:\managed",
            git_executable: r"C:\Git\bin\git.exe",
            expected_execution_environment_ref: None,
        };
        let task_ref = ContentDigest::from_sha256("1".repeat(64)).expect("task ref");
        let task_id = TaskId::new("TASK-MANAGED-ENVIRONMENT").expect("task id");
        let base_commit = "2".repeat(40);
        let prepare = build_baseline_command(&native, &task_ref, &task_id, &base_commit, None);
        assert_eq!(prepare["schema"], COMMAND_SCHEMA);
        assert_eq!(prepare["operation"], "prepare");
        assert_eq!(prepare["expected_baseline_sha256"], Value::Null);
        assert_eq!(prepare["expected_execution_environment_ref"], Value::Null);
        assert_eq!(prepare.as_object().expect("prepare object").len(), 10);

        let exact_wsl_ref = format!("execution-environment:sha256:{}", "a".repeat(64));
        let wsl = ManagedWorktreeCommandContext {
            expected_execution_environment_ref: Some(exact_wsl_ref.as_str()),
            ..native
        };
        let baseline = ContentDigest::from_sha256("3".repeat(64)).expect("baseline");
        let verify =
            build_baseline_command(&wsl, &task_ref, &task_id, &base_commit, Some(&baseline));
        assert_eq!(verify["schema"], COMMAND_SCHEMA);
        assert_eq!(verify["operation"], "verify");
        assert_eq!(verify["expected_baseline_sha256"], baseline.as_str());
        assert_eq!(
            verify["expected_execution_environment_ref"],
            exact_wsl_ref.as_str()
        );
        assert_eq!(verify.as_object().expect("verify object").len(), 10);

        let protect = build_protect_command(
            &wsl,
            &task_ref,
            &task_id,
            1,
            7,
            &base_commit,
            &"4".repeat(40),
            &baseline,
            true,
        );
        assert_eq!(protect["schema"], COMMAND_SCHEMA);
        assert_eq!(protect["operation"], "protect");
        assert_eq!(protect["expected_baseline_sha256"], baseline.as_str());
        assert_eq!(
            protect["expected_execution_environment_ref"],
            exact_wsl_ref.as_str()
        );
        assert_eq!(protect.as_object().expect("protect object").len(), 14);
        assert_eq!(COMMAND_SCHEMA, "lattice.managed-worktree-command/1.1");
    }

    #[cfg(windows)]
    #[test]
    fn production_worktree_bridge_creates_a_cold_disposable_worktree_with_cleared_environment() {
        let _serial = TRANSPORT_TEST_LOCK.lock().expect("transport test lock");
        let root = env::temp_dir().join(format!(
            "lattice-managed-worktree-cold-{}-{}",
            std::process::id(),
            TRANSPORT_FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let repository = root.join("repository");
        let worktree_root = root.join("managed-worktrees");
        fs::create_dir_all(&repository).expect("repository root");
        fs::create_dir_all(&worktree_root).expect("worktree root");
        let git = test_git_executable();
        let run_git = |arguments: &[&str]| {
            let output = Command::new(&git)
                .args(arguments)
                .current_dir(&repository)
                .output()
                .expect("run setup Git");
            assert!(
                output.status.success(),
                "setup Git failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            output
        };
        run_git(&["init", "-b", "main"]);
        run_git(&["config", "user.name", "LATTICE test"]);
        run_git(&["config", "user.email", "lattice-test@invalid.example"]);
        fs::write(repository.join("proof.txt"), b"baseline\n").expect("baseline file");
        run_git(&["add", "proof.txt"]);
        run_git(&["commit", "-m", "baseline"]);
        let base_commit = String::from_utf8(run_git(&["rev-parse", "HEAD"]).stdout)
            .expect("base commit")
            .trim()
            .to_owned();

        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let lattice_root = manifest
            .parent()
            .and_then(Path::parent)
            .expect("workspace root");
        let bridge = lattice_root.join("apps/lattice-control/src/managed-worktree-bridge.mjs");
        let canonical_repository = fs::canonicalize(&repository).expect("canonical repository");
        let canonical_worktree_root =
            fs::canonicalize(&worktree_root).expect("canonical worktree root");
        assert!(
            canonical_repository
                .as_os_str()
                .to_string_lossy()
                .starts_with(r"\\?\"),
            "Windows canonical path exercises the verbatim boundary"
        );
        let adapter = ManagedWorktreeAdapter::new(
            ManagedWorktreeAdapterConfig::new(
                test_node_executable(),
                bridge,
                git,
                canonical_repository,
                canonical_worktree_root,
                Duration::from_secs(30),
            )
            .expect("production bridge config"),
        );
        let task_ref = ContentDigest::from_sha256("a".repeat(64)).expect("task ref");
        let baseline = adapter
            .prepare(
                ProjectId::new("managed-worktree-test").expect("project id"),
                task_ref,
                &TaskId::new("TASK-MANAGED-WORKTREE").expect("task id"),
                1,
                &base_commit,
                "2026-08-27T00:00:00Z",
                None,
            )
            .expect("cold worktree through supervised cleared environment");
        assert!(!baseline.replayed());
        assert!(baseline.worktree_path().is_dir());
        fs::remove_dir_all(root).expect("remove cold worktree fixture");
    }

    #[test]
    fn oversized_bridge_output_stops_at_the_hard_cap_and_reader_exits() {
        #[cfg(windows)]
        let _serial = TRANSPORT_TEST_LOCK.lock().expect("transport test lock");
        let oversized = vec![b'x'; usize::try_from(MAX_BRIDGE_OUTPUT_BYTES).unwrap() + 1];
        let (failed, reader) =
            spawn_bounded_bridge_reader(Box::new(std::io::Cursor::new(oversized)));
        assert!(reader.join().expect("reader join").is_err());
        assert!(failed.load(Ordering::Acquire));
        assert_eq!(ACTIVE_WORKTREE_READERS.load(Ordering::Acquire), 0);
    }

    #[cfg(windows)]
    #[test]
    fn post_spawn_transitive_substitution_is_reaped_before_command_effect() {
        let _serial = TRANSPORT_TEST_LOCK.lock().expect("transport test lock");
        let marker_name = "provider-effect.txt";
        let bridge = format!(
            "import './managed-worktree.mjs';\nprocess.stdin.once('data', () => {{ process.stdout.write(JSON.stringify({{ kind: 'result' }}) + '\\n'); process.exit(0); }});\n"
        );
        let (root, adapter, dependency) = transport_fixture(&bridge, Duration::from_secs(2));
        let marker = dependency.parent().unwrap().join(marker_name);
        let swap_rejected = std::cell::Cell::new(false);
        let response = adapter
            .run_bridge_with_post_spawn_hook(&json!({"request": "bounded"}), || {
                let malicious = format!(
                    "import {{ writeFileSync }} from 'node:fs'; writeFileSync(new URL('./{marker_name}', import.meta.url), 'effect');\n"
                );
                swap_rejected.set(fs::write(&dependency, malicious).is_err());
            })
            .expect("sealed trusted bundle may execute");
        assert_eq!(response.get("kind").and_then(Value::as_str), Some("result"));
        assert!(swap_rejected.get(), "post-verify ABA write must be denied");
        thread::sleep(Duration::from_millis(250));
        assert!(
            !marker.exists(),
            "malicious import-time effect must remain absent"
        );
        assert_eq!(ACTIVE_WORKTREE_READERS.load(Ordering::Acquire), 0);
        fs::remove_dir_all(root).expect("remove transport fixture");
    }

    #[cfg(windows)]
    #[test]
    fn timeout_reaps_supervised_descendant_and_joins_reader() {
        let _serial = TRANSPORT_TEST_LOCK.lock().expect("transport test lock");
        let root_seed = env::temp_dir().join(format!(
            "lattice-managed-worktree-marker-{}-{}",
            std::process::id(),
            TRANSPORT_FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let marker = root_seed.join("late-descendant-effect.txt");
        fs::create_dir_all(&root_seed).expect("marker root");
        let marker_json =
            serde_json::to_string(marker.to_str().expect("marker text")).expect("marker JSON");
        let child_source = format!(
            "setTimeout(() => require('node:fs').writeFileSync({marker_json}, 'late'), 500); setInterval(() => {{}}, 1000);"
        );
        let child_json = serde_json::to_string(&child_source).expect("child source JSON");
        let bridge = format!(
            "import {{ spawn }} from 'node:child_process';\nprocess.stdin.once('data', () => {{ spawn(process.execPath, ['-e', {child_json}], {{ stdio: 'ignore' }}); }});\nsetInterval(() => {{}}, 1000);\n"
        );
        let (fixture_root, adapter, _) = transport_fixture(&bridge, Duration::from_millis(100));
        let failure = adapter
            .run_bridge(&json!({"request": "timeout"}))
            .expect_err("nonterminal bridge must time out");
        assert_eq!(failure.code(), "LATTICE_MANAGED_WORKTREE_BRIDGE_TIMEOUT");
        thread::sleep(Duration::from_millis(750));
        assert!(
            !marker.exists(),
            "Job descendant must be reaped before return"
        );
        assert_eq!(ACTIVE_WORKTREE_READERS.load(Ordering::Acquire), 0);
        fs::remove_dir_all(fixture_root).expect("remove transport fixture");
        fs::remove_dir_all(root_seed).expect("remove marker fixture");
    }

    #[cfg(windows)]
    #[test]
    fn graceful_scheduler_cancellation_reaps_worktree_job_and_joins_reader() {
        let _serial = TRANSPORT_TEST_LOCK.lock().expect("transport test lock");
        let root_seed = env::temp_dir().join(format!(
            "lattice-managed-worktree-cancel-{}-{}",
            std::process::id(),
            TRANSPORT_FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root_seed).expect("cancellation marker root");
        let marker = root_seed.join("late-descendant-effect.txt");
        let marker_json = serde_json::to_string(marker.to_str().expect("marker text")).unwrap();
        let child_source = format!(
            "setTimeout(() => require('node:fs').writeFileSync({marker_json}, 'late'), 500); setInterval(() => {{}}, 1000);"
        );
        let child_json = serde_json::to_string(&child_source).unwrap();
        let bridge = format!(
            "import {{ spawn }} from 'node:child_process';\nprocess.stdin.once('data', () => {{ spawn(process.execPath, ['-e', {child_json}], {{ stdio: 'ignore' }}); }});\nsetInterval(() => {{}}, 1000);\n"
        );
        let (fixture_root, mut adapter, _) = transport_fixture(&bridge, Duration::from_secs(10));
        let cancellation = ManagedWorkerCancellation::default();
        adapter.config.cancellation = cancellation.clone();
        let failure = adapter
            .run_bridge_with_post_spawn_hook(&json!({"request": "cancel"}), || {
                // The hook runs only after the child is assigned to its Job.
                // Cancellation then follows the same write/loop/common-cleanup
                // path as a scheduler shutdown racing active setup.
                cancellation.request();
            })
            .expect_err("cancellation must stop the supervised bridge");
        assert_eq!(failure.code(), MANAGED_GRACEFUL_SHUTDOWN_IDLE);
        thread::sleep(Duration::from_millis(750));
        assert!(!marker.exists(), "cancelled Job descendant must be reaped");
        assert_eq!(ACTIVE_WORKTREE_READERS.load(Ordering::Acquire), 0);
        fs::remove_dir_all(fixture_root).expect("remove transport fixture");
        fs::remove_dir_all(root_seed).expect("remove cancellation marker fixture");
    }

    #[test]
    fn baseline_json_is_closed_and_path_free() {
        let digest = "a".repeat(64);
        let oid = "b".repeat(40);
        let value = json!({
            "base_commit": oid,
            "base_tree": "c".repeat(40),
            "common_git_directory_locator_digest": digest,
            "git_control_digest": digest,
            "git_directory_locator_digest": digest,
            "git_pointer_digest": digest,
            "head_commit": "b".repeat(40),
            "head_tree": "c".repeat(40),
            "index_digest": digest,
            "initial_worktree_state": "CLEAN",
            "ownership_digest": digest,
            "repository_locator_digest": digest,
            "schema": MANAGED_WORKTREE_BASELINE_SCHEMA,
            "task_branch": "lattice/task-managed",
            "task_ref": "d".repeat(64),
            "worktree_locator_digest": digest,
        });
        let encoded = serde_json::to_string(&value).expect("json");
        validate_baseline_json(&encoded, &"d".repeat(64), &"b".repeat(40))
            .expect("closed baseline");
        assert!(!encoded.contains("C:\\"));
        assert!(!encoded.contains("https://"));
    }

    #[test]
    fn worktree_root_must_be_absolute_and_separate() {
        let absolute = if cfg!(windows) {
            PathBuf::from(r"C:\lattice\repo")
        } else {
            PathBuf::from("/lattice/repo")
        };
        assert!(
            ManagedWorktreeAdapterConfig::new(
                absolute.join("node"),
                absolute.join("bridge.mjs"),
                absolute.join("git"),
                absolute.clone(),
                absolute.join("children"),
                Duration::from_secs(30),
            )
            .is_err()
        );
    }

    #[test]
    fn protected_result_requires_exact_passing_durable_snapshot() {
        let base = "b".repeat(40);
        let result = "c".repeat(40);
        let tree = "d".repeat(40);
        let diff = ContentDigest::from_sha256("e".repeat(64)).expect("diff");
        let snapshot = json!({
            "schema": "lattice.managed-git-snapshot/1.0",
            "base_commit": base,
            "result_commit": result,
            "tree": tree,
            "diff_digest": diff.as_str(),
            "command_identity": "f".repeat(64),
            "changed_paths": ["phase4-proof.txt"],
            "checks": [{ "id": "git-diff-check-v1", "passed": true }],
        });
        let bytes = serde_json::to_vec(&snapshot).expect("snapshot");
        assert_eq!(
            verified_result_commit(
                &bytes,
                &base,
                &digest_bytes(base.as_bytes()).expect("base digest"),
                &digest_bytes(result.as_bytes()).expect("result digest"),
                &digest_bytes(tree.as_bytes()).expect("tree digest"),
                &diff,
            )
            .expect("verified result"),
            result,
        );

        let mut invalid_command = snapshot.clone();
        invalid_command["command_identity"] = Value::String("0".repeat(64));
        assert!(
            verified_result_commit(
                &serde_json::to_vec(&invalid_command).expect("invalid command snapshot"),
                &base,
                &digest_bytes(base.as_bytes()).expect("base digest"),
                &digest_bytes(result.as_bytes()).expect("result digest"),
                &digest_bytes(tree.as_bytes()).expect("tree digest"),
                &diff,
            )
            .is_err()
        );

        let mut failed = snapshot;
        failed["checks"][0]["passed"] = Value::Bool(false);
        assert!(
            verified_result_commit(
                &serde_json::to_vec(&failed).expect("failed snapshot"),
                &base,
                &digest_bytes(base.as_bytes()).expect("base digest"),
                &digest_bytes(result.as_bytes()).expect("result digest"),
                &digest_bytes(tree.as_bytes()).expect("tree digest"),
                &diff,
            )
            .is_err()
        );
    }

    #[test]
    fn protected_result_recomputes_bridge_digest_from_public_preimage() {
        let task_ref = ContentDigest::from_sha256("1".repeat(64)).expect("task ref");
        let task_id = TaskId::new("TASK-PROTECTED-DIGEST").expect("task id");
        let baseline = ContentDigest::from_sha256("2".repeat(64)).expect("baseline");
        let result_commit = "3".repeat(40);
        let protected_ref = format!("refs/lattice/managed/{}/attempt-1", task_ref.as_str());
        let expected = framed_digest(
            PROTECTED_RESULT_REF_DIGEST_DOMAIN,
            &[
                task_ref.as_str(),
                "1",
                "7",
                &protected_ref,
                &result_commit,
                baseline.as_str(),
            ],
        )
        .expect("expected bridge digest");
        let response = json!({
            "schema": RESULT_SCHEMA,
            "kind": "result",
            "operation": "protect",
            "task_ref": task_ref.as_str(),
            "task_id": task_id.as_str(),
            "attempt": 1,
            "writer_fence": 7,
            "base_commit": "4".repeat(40),
            "result_commit": result_commit,
            "worktree_path": "bounded-worktree",
            "protected_ref": protected_ref,
            "baseline_sha256": baseline.as_str(),
            "replayed": false,
            "protected_ref_digest": expected.as_str(),
        });
        let protected = protected_result_from_response(
            response.clone(),
            &task_ref,
            &task_id,
            1,
            7,
            &"4".repeat(40),
            &"3".repeat(40),
            &baseline,
        )
        .expect("exact public preimage");
        assert_eq!(protected.evidence_digest(), &expected);

        let mut substituted = response;
        substituted["protected_ref_digest"] = Value::String("f".repeat(64));
        assert!(
            protected_result_from_response(
                substituted,
                &task_ref,
                &task_id,
                1,
                7,
                &"4".repeat(40),
                &"3".repeat(40),
                &baseline,
            )
            .is_err(),
            "a syntactically valid substituted bridge digest must fail closed",
        );
    }
}
