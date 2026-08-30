//! Same-process validation and bounded launch support for the WSL2 execution
//! environment preflight bridge. PostgreSQL owns durable descriptor truth;
//! this module independently validates the live receipt before any provider
//! process or model check can be reached.

use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::fmt;
use std::io::{Read, Write};
use std::path::Path;
use std::process::Command;
use std::sync::mpsc::{self, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use lattice_artifact_store::{ManagedEvidenceInput, ManagedEvidenceKind, VerifiedManagedEvidence};
use lattice_codex_adapter::SupervisedDuplexChild;
use lattice_contracts::{ContentDigest, ProjectId};
use lattice_postgres_foreman::ExecutionEnvironmentDescriptor;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::managed_file_identity::ManagedEffectBundleGuard;

const WSL2_PREFLIGHT_SCHEMA: &str = "lattice.wsl2-zero-model-preflight/1.0";
const WSL2_PREFLIGHT_REQUEST_SCHEMA: &str = "lattice.wsl2-execution-preflight-request/1.0";
const WSL2_PREFLIGHT_RESULT_SCHEMA: &str = "lattice.wsl2-execution-preflight-result/1.0";
const WSL2_PREFLIGHT_MAX_INPUT_BYTES: usize = 262_144;
const WSL2_PREFLIGHT_MAX_OUTPUT_BYTES: usize = 1_048_576;
const WSL2_PREFLIGHT_BRIDGE_MAX_BYTES: u64 = 4 * 1_024 * 1_024;
const WSL2_PREFLIGHT_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ManagedExecutionEnvironmentError {
    code: &'static str,
}

impl ManagedExecutionEnvironmentError {
    pub(crate) const fn code(self) -> &'static str {
        self.code
    }
}

impl fmt::Display for ManagedExecutionEnvironmentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code)
    }
}

impl Error for ManagedExecutionEnvironmentError {}

fn rejected() -> ManagedExecutionEnvironmentError {
    ManagedExecutionEnvironmentError {
        code: "LATTICE_MANAGED_WSL2_PREFLIGHT_REJECTED",
    }
}

fn timed_out() -> ManagedExecutionEnvironmentError {
    ManagedExecutionEnvironmentError {
        code: "LATTICE_MANAGED_WSL2_PREFLIGHT_TIMEOUT",
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VerifiedWsl2Preflight {
    descriptor: ExecutionEnvironmentDescriptor,
    evidence: VerifiedManagedEvidence,
    receipt_digest: String,
}

impl VerifiedWsl2Preflight {
    pub(crate) const fn descriptor(&self) -> &ExecutionEnvironmentDescriptor {
        &self.descriptor
    }

    pub(crate) const fn evidence(&self) -> &VerifiedManagedEvidence {
        &self.evidence
    }

    pub(crate) fn receipt_digest(&self) -> &str {
        &self.receipt_digest
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExpectedWsl2Preflight {
    pub(crate) task_ref: String,
    pub(crate) attempt: u8,
    pub(crate) worktree_ref: String,
    pub(crate) environment_ref: String,
    pub(crate) immutable_snapshot_ref: String,
    pub(crate) sandbox_policy_ref: String,
    pub(crate) privilege_boundary_ref: String,
    pub(crate) linux_cwd: String,
    pub(crate) repository_head: String,
    pub(crate) process_fence: String,
    pub(crate) retry_of: Option<String>,
    pub(crate) reconnect_of: Option<String>,
}

fn canonicalize_json(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(canonicalize_json).collect()),
        Value::Object(object) => {
            let sorted = object
                .iter()
                .map(|(key, value)| (key.clone(), canonicalize_json(value)))
                .collect::<BTreeMap<_, _>>();
            Value::Object(sorted.into_iter().collect())
        }
        _ => value.clone(),
    }
}

fn canonical_json(value: &Value) -> Result<String, ManagedExecutionEnvironmentError> {
    serde_json::to_string(&canonicalize_json(value)).map_err(|_| rejected())
}

fn typed_json_digest(
    domain: &str,
    value: &Value,
) -> Result<String, ManagedExecutionEnvironmentError> {
    if domain.is_empty()
        || !domain
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(rejected());
    }
    let digest = Sha256::digest(canonical_json(value)?.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(format!("{domain}:sha256:{digest}"))
}

fn exact_object<'a>(
    value: &'a Value,
    keys: &[&str],
) -> Result<&'a Map<String, Value>, ManagedExecutionEnvironmentError> {
    let object = value.as_object().ok_or_else(rejected)?;
    if object.len() != keys.len() || keys.iter().any(|key| !object.contains_key(*key)) {
        return Err(rejected());
    }
    Ok(object)
}

fn text<'a>(object: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    object.get(key).and_then(Value::as_str)
}

fn number(object: &Map<String, Value>, key: &str) -> Option<u64> {
    object.get(key).and_then(Value::as_u64)
}

fn valid_typed_sha256(value: &str, domain: &str) -> bool {
    value
        .strip_prefix(&format!("{domain}:sha256:"))
        .is_some_and(valid_sha256)
}

fn valid_sha256(value: &str) -> bool {
    valid_lower_hex(value, 64)
}

fn valid_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn validate_wsl2_preflight_receipt(
    receipt: &Value,
    expected: &ExpectedWsl2Preflight,
) -> Result<(), ManagedExecutionEnvironmentError> {
    let object = exact_object(
        receipt,
        &[
            "schema",
            "status",
            "task_ref",
            "attempt",
            "worktree_ref",
            "execution_environment_ref",
            "descriptor_digest",
            "distribution_identity_ref",
            "linux_cwd",
            "repository_head",
            "repository_identity",
            "codex_home_digest",
            "credential_authority_ref",
            "credential_seal_digest",
            "verification_toolchain_ref",
            "immutable_snapshot_ref",
            "sandbox_policy_ref",
            "privilege_boundary_ref",
            "process_fence",
            "isolation",
            "probes",
            "effect_counters",
            "provider_effect_count",
            "bounds",
            "timeout",
            "continuation",
            "connector_auth_ready",
            "receipt_digest",
        ],
    )?;
    if text(object, "schema") != Some(WSL2_PREFLIGHT_SCHEMA)
        || text(object, "status") != Some("PASS")
        || text(object, "task_ref") != Some(expected.task_ref.as_str())
        || number(object, "attempt") != Some(u64::from(expected.attempt))
        || text(object, "worktree_ref") != Some(expected.worktree_ref.as_str())
        || text(object, "execution_environment_ref") != Some(expected.environment_ref.as_str())
        || text(object, "descriptor_digest") != Some(expected.environment_ref.as_str())
        || text(object, "linux_cwd") != Some(expected.linux_cwd.as_str())
        || text(object, "repository_head") != Some(expected.repository_head.as_str())
        || number(object, "provider_effect_count") != Some(0)
        || object.get("connector_auth_ready").and_then(Value::as_bool) != Some(false)
        || !expected.linux_cwd.starts_with("/home/")
        || expected.linux_cwd.starts_with("/mnt/")
        || expected.linux_cwd.contains('\\')
        || !valid_typed_sha256(
            text(object, "distribution_identity_ref").unwrap_or_default(),
            "wsl2-distribution",
        )
        || !valid_typed_sha256(
            text(object, "repository_identity").unwrap_or_default(),
            "repository",
        )
        || !valid_typed_sha256(
            text(object, "codex_home_digest").unwrap_or_default(),
            "codex-home",
        )
        || !valid_typed_sha256(
            text(object, "credential_authority_ref").unwrap_or_default(),
            "wsl2-credential-authority",
        )
        || !valid_typed_sha256(
            text(object, "credential_seal_digest").unwrap_or_default(),
            "credential-seal",
        )
        || !valid_typed_sha256(
            text(object, "verification_toolchain_ref").unwrap_or_default(),
            "wsl2-verification-toolchain",
        )
        || text(object, "immutable_snapshot_ref") != Some(expected.immutable_snapshot_ref.as_str())
        || !valid_typed_sha256(
            text(object, "immutable_snapshot_ref").unwrap_or_default(),
            "wsl2-immutable-snapshot",
        )
        || text(object, "sandbox_policy_ref") != Some(expected.sandbox_policy_ref.as_str())
        || !valid_typed_sha256(
            text(object, "sandbox_policy_ref").unwrap_or_default(),
            "wsl2-sandbox-policy",
        )
        || text(object, "privilege_boundary_ref") != Some(expected.privilege_boundary_ref.as_str())
        || !valid_typed_sha256(
            text(object, "privilege_boundary_ref").unwrap_or_default(),
            "wsl2-privilege-boundary",
        )
    {
        return Err(rejected());
    }

    let fence = exact_object(
        object.get("process_fence").ok_or_else(rejected)?,
        &[
            "fence",
            "authority_ref",
            "service_unit",
            "cgroup_path",
            "cgroup_version",
            "delegated",
            "boot_id_digest",
            "supervisor_zero_descendants",
            "outer_post_exit",
        ],
    )?;
    let outer = exact_object(
        fence.get("outer_post_exit").ok_or_else(rejected)?,
        &[
            "unit",
            "active_state",
            "sub_state",
            "result",
            "cgroup_path",
            "delegate",
            "cgroup_exists",
            "populated",
        ],
    )?;
    let cgroup_exists = outer.get("cgroup_exists").and_then(Value::as_bool);
    let populated = outer.get("populated");
    let cgroup_empty = match (cgroup_exists, populated) {
        (Some(false), Some(Value::Null)) => true,
        (Some(true), Some(Value::Number(value))) => value.as_u64() == Some(0),
        _ => false,
    };
    if text(fence, "fence") != Some(expected.process_fence.as_str())
        || !valid_sha256(&expected.process_fence)
        || !valid_typed_sha256(
            text(fence, "authority_ref").unwrap_or_default(),
            "wsl2-process-fence-authority",
        )
        || number(fence, "cgroup_version") != Some(2)
        || fence.get("delegated").and_then(Value::as_bool) != Some(false)
        || fence
            .get("supervisor_zero_descendants")
            .and_then(Value::as_bool)
            != Some(true)
        || text(outer, "active_state") != Some("inactive")
        || text(outer, "sub_state") != Some("dead")
        || text(outer, "result") != Some("success")
        || text(outer, "delegate") != Some("no")
        || !cgroup_empty
    {
        return Err(rejected());
    }

    let counters = exact_object(
        object.get("effect_counters").ok_or_else(rejected)?,
        &[
            "account_read",
            "thread_start",
            "turn_start",
            "provider_effect_count",
        ],
    )?;
    if [
        "account_read",
        "thread_start",
        "turn_start",
        "provider_effect_count",
    ]
    .into_iter()
    .any(|key| number(counters, key) != Some(0))
    {
        return Err(rejected());
    }

    let bounds = exact_object(
        object.get("bounds").ok_or_else(rejected)?,
        &[
            "stdout_limit_bytes",
            "stderr_limit_bytes",
            "stdout_observed_bytes",
            "stderr_observed_bytes",
        ],
    )?;
    if number(bounds, "stdout_limit_bytes") != Some(WSL2_PREFLIGHT_MAX_OUTPUT_BYTES as u64)
        || number(bounds, "stderr_limit_bytes") != Some(WSL2_PREFLIGHT_MAX_OUTPUT_BYTES as u64)
        || number(bounds, "stdout_observed_bytes")
            .is_none_or(|value| value > WSL2_PREFLIGHT_MAX_OUTPUT_BYTES as u64)
        || number(bounds, "stderr_observed_bytes")
            .is_none_or(|value| value > WSL2_PREFLIGHT_MAX_OUTPUT_BYTES as u64)
    {
        return Err(rejected());
    }

    let timeout = exact_object(
        object.get("timeout").ok_or_else(rejected)?,
        &["timeout_ms", "timed_out", "interrupted"],
    )?;
    if number(timeout, "timeout_ms") != Some(170_000)
        || timeout.get("timed_out").and_then(Value::as_bool) != Some(false)
        || timeout.get("interrupted").and_then(Value::as_bool) != Some(false)
    {
        return Err(rejected());
    }

    let continuation = exact_object(
        object.get("continuation").ok_or_else(rejected)?,
        &["attempt", "retry_of", "reconnect_of"],
    )?;
    let optional_text = |key: &str| match continuation.get(key) {
        Some(Value::String(value)) => Some(Some(value.as_str())),
        Some(Value::Null) => Some(None),
        _ => None,
    };
    if number(continuation, "attempt") != Some(u64::from(expected.attempt))
        || optional_text("retry_of") != Some(expected.retry_of.as_deref())
        || optional_text("reconnect_of") != Some(expected.reconnect_of.as_deref())
    {
        return Err(rejected());
    }

    if !object.get("isolation").is_some_and(Value::is_object)
        || !object.get("probes").is_some_and(Value::is_object)
    {
        return Err(rejected());
    }
    let receipt_digest = text(object, "receipt_digest").ok_or_else(rejected)?;
    let mut subject = receipt.clone();
    subject
        .as_object_mut()
        .ok_or_else(rejected)?
        .remove("receipt_digest");
    if receipt_digest != typed_json_digest("wsl2-preflight", &subject)? {
        return Err(rejected());
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OutputChannel {
    Stdout,
    Stderr,
}

#[allow(clippy::too_many_arguments)]
fn deterministic_process_fence(
    environment_ref: &str,
    task_ref: &ContentDigest,
    attempt: u8,
    worktree_ref: &str,
    repository_head: &str,
    retry_of: Option<&str>,
    reconnect_of: Option<&str>,
) -> Result<String, ManagedExecutionEnvironmentError> {
    let subject = serde_json::json!({
        "schema": "lattice.wsl2-process-fence-binding/1.0",
        "execution_environment_ref": environment_ref,
        "task_ref": task_ref.as_str(),
        "attempt": attempt,
        "worktree_ref": worktree_ref,
        "repository_head": repository_head,
        "retry_of": retry_of,
        "reconnect_of": reconnect_of,
    });
    Ok(Sha256::digest(canonical_json(&subject)?.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn read_bounded(stream: Box<dyn Read + Send>, maximum_bytes: usize) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(maximum_bytes.saturating_add(1));
    stream
        .take(u64::try_from(maximum_bytes.saturating_add(1)).unwrap_or(u64::MAX))
        .read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn join_output_readers(
    stdout: thread::JoinHandle<()>,
    stderr: thread::JoinHandle<()>,
) -> Result<(), ManagedExecutionEnvironmentError> {
    stdout.join().map_err(|_| rejected())?;
    stderr.join().map_err(|_| rejected())?;
    Ok(())
}

/// Runs the task-owned WSL2 technical preflight in one bounded, sealed Node
/// bridge process. This call does not perform account, thread, turn, or model
/// operations; a successful result is still explicitly `connector_auth_ready=false`.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub(crate) fn run_wsl2_execution_preflight(
    node_executable: &Path,
    bridge_path: &Path,
    runtime_guard: &ManagedEffectBundleGuard,
    template_descriptor: &str,
    windows_worktree_path: &Path,
    project_id: &ProjectId,
    task_ref: &ContentDigest,
    attempt: u8,
    worktree_ref: &str,
    expected_repository_head: &str,
    retry_of: Option<&str>,
    reconnect_of: Option<&str>,
    timeout: Duration,
    created_at: &str,
) -> Result<VerifiedWsl2Preflight, ManagedExecutionEnvironmentError> {
    if !node_executable.is_absolute()
        || !bridge_path.is_absolute()
        || !windows_worktree_path.is_absolute()
        || !(1..=3).contains(&attempt)
        || !worktree_ref
            .strip_prefix("worktree:sha256:")
            .is_some_and(valid_sha256)
        || !valid_lower_hex(expected_repository_head, 40)
        || template_descriptor.is_empty()
        || template_descriptor.len() > 16_384
        || timeout.is_zero()
    {
        return Err(rejected());
    }
    let windows_worktree_path = windows_worktree_path.to_str().ok_or_else(rejected)?;
    if !windows_worktree_path.starts_with(r"\\wsl.localhost\") {
        return Err(rejected());
    }
    let template =
        ExecutionEnvironmentDescriptor::from_json(template_descriptor).map_err(|_| rejected())?;
    if template.verification_task_ref() != task_ref {
        return Err(rejected());
    }

    runtime_guard
        .covers_file(node_executable)
        .and_then(|()| runtime_guard.covers_file(bridge_path))
        .map_err(|_| rejected())?;
    let bridge_snapshot = runtime_guard
        .sealed_file_snapshot(bridge_path)
        .map_err(|_| rejected())?
        .filter(|snapshot| {
            snapshot.length() > 0 && snapshot.length() <= WSL2_PREFLIGHT_BRIDGE_MAX_BYTES
        })
        .ok_or_else(rejected)?;

    let process_fence = deterministic_process_fence(
        template.environment_ref().as_str(),
        task_ref,
        attempt,
        worktree_ref,
        expected_repository_head,
        retry_of,
        reconnect_of,
    )?;
    let request = serde_json::json!({
        "schema": WSL2_PREFLIGHT_REQUEST_SCHEMA,
        "template_descriptor": serde_json::from_str::<Value>(template.as_json()).map_err(|_| rejected())?,
        "windows_worktree_path": windows_worktree_path,
        "task_ref": task_ref.as_str(),
        "attempt": attempt,
        "worktree_ref": worktree_ref,
        "expected_repository_head": expected_repository_head,
        "process_fence": process_fence,
        "retry_of": retry_of,
        "reconnect_of": reconnect_of,
    });
    let mut input = canonical_json(&request)?.into_bytes();
    input.push(b'\n');
    if input.len() > WSL2_PREFLIGHT_MAX_INPUT_BYTES {
        return Err(rejected());
    }

    let mut command = Command::new(node_executable);
    command.arg(bridge_path);
    if let Some(parent) = bridge_path.parent() {
        command.current_dir(parent);
    }
    command.env_clear();
    for name in ["SystemRoot", "WINDIR"] {
        if let Some(value) = env::var_os(name) {
            command.env(name, value);
        }
    }
    let mut child =
        SupervisedDuplexChild::spawn_with_stderr_cleared(&mut command).map_err(|_| rejected())?;
    runtime_guard.verify().map_err(|_| {
        let _ = child.terminate_and_reap();
        rejected()
    })?;
    let mut stdin = child.take_stdin().ok_or_else(|| {
        let _ = child.terminate_and_reap();
        rejected()
    })?;
    let stdout = child.take_stdout().ok_or_else(|| {
        let _ = child.terminate_and_reap();
        rejected()
    })?;
    let stderr = child.take_stderr().ok_or_else(|| {
        let _ = child.terminate_and_reap();
        rejected()
    })?;
    let (sender, receiver) = mpsc::sync_channel(2);
    let stdout_sender = sender.clone();
    let stdout_reader = thread::spawn(move || {
        let result = read_bounded(stdout, WSL2_PREFLIGHT_MAX_OUTPUT_BYTES);
        let _ = stdout_sender.send((OutputChannel::Stdout, result));
    });
    let stderr_reader = thread::spawn(move || {
        let result = read_bounded(stderr, WSL2_PREFLIGHT_MAX_OUTPUT_BYTES);
        let _ = sender.send((OutputChannel::Stderr, result));
    });
    if stdin
        .write_all(&input)
        .and_then(|()| stdin.flush())
        .is_err()
    {
        drop(stdin);
        let _ = child.terminate_and_reap();
        join_output_readers(stdout_reader, stderr_reader)?;
        return Err(rejected());
    }
    drop(stdin);

    let deadline = Instant::now()
        .checked_add(timeout.min(WSL2_PREFLIGHT_TIMEOUT))
        .ok_or_else(rejected)?;
    let mut status = None;
    let mut stdout = None;
    let mut stderr = None;
    let mut reaped = false;
    loop {
        loop {
            match receiver.try_recv() {
                Ok((OutputChannel::Stdout, Ok(bytes))) if stdout.is_none() => stdout = Some(bytes),
                Ok((OutputChannel::Stderr, Ok(bytes))) if stderr.is_none() => stderr = Some(bytes),
                Ok(_) | Err(TryRecvError::Disconnected) => {
                    let _ = child.terminate_and_reap();
                    join_output_readers(stdout_reader, stderr_reader)?;
                    return Err(rejected());
                }
                Err(TryRecvError::Empty) => break,
            }
        }
        if stdout
            .as_ref()
            .is_some_and(|bytes| bytes.len() > WSL2_PREFLIGHT_MAX_OUTPUT_BYTES)
            || stderr
                .as_ref()
                .is_some_and(|bytes| bytes.len() > WSL2_PREFLIGHT_MAX_OUTPUT_BYTES)
        {
            let _ = child.terminate_and_reap();
            join_output_readers(stdout_reader, stderr_reader)?;
            return Err(rejected());
        }
        if status.is_none() {
            status = child.try_wait().map_err(|_| rejected())?;
        }
        if status.is_some() && !reaped {
            child.terminate_and_reap().map_err(|_| rejected())?;
            reaped = true;
        }
        if let (Some(status), Some(stdout), Some(stderr)) =
            (status.as_ref(), stdout.as_ref(), stderr.as_ref())
        {
            let success = status.success();
            let stdout = stdout.clone();
            let stderr = stderr.clone();
            join_output_readers(stdout_reader, stderr_reader)?;
            if !success || !stderr.is_empty() {
                return Err(rejected());
            }
            return verified_preflight_from_output(
                &stdout,
                project_id,
                task_ref,
                attempt,
                worktree_ref,
                expected_repository_head,
                windows_worktree_path,
                &process_fence,
                retry_of,
                reconnect_of,
                created_at,
                bridge_snapshot.content_digest().clone(),
            );
        }
        if Instant::now() >= deadline {
            let cleanup = child.terminate_and_reap().map_err(|_| rejected());
            join_output_readers(stdout_reader, stderr_reader)?;
            cleanup?;
            return Err(timed_out());
        }
        thread::sleep(Duration::from_millis(5));
    }
}

#[allow(clippy::too_many_arguments)]
fn verified_preflight_from_output(
    stdout: &[u8],
    project_id: &ProjectId,
    task_ref: &ContentDigest,
    attempt: u8,
    worktree_ref: &str,
    expected_repository_head: &str,
    windows_worktree_path: &str,
    process_fence: &str,
    retry_of: Option<&str>,
    reconnect_of: Option<&str>,
    created_at: &str,
    producer_digest: ContentDigest,
) -> Result<VerifiedWsl2Preflight, ManagedExecutionEnvironmentError> {
    let output_text = std::str::from_utf8(stdout).map_err(|_| rejected())?;
    if !output_text.ends_with('\n')
        || output_text.contains('\r')
        || output_text[..output_text.len() - 1].contains('\n')
    {
        return Err(rejected());
    }
    let result: Value =
        serde_json::from_str(&output_text[..output_text.len() - 1]).map_err(|_| rejected())?;
    let object = exact_object(
        &result,
        &[
            "schema",
            "status",
            "task_ref",
            "attempt",
            "worktree_ref",
            "environment",
            "receipt",
            "result_digest",
        ],
    )?;
    if text(object, "schema") != Some(WSL2_PREFLIGHT_RESULT_SCHEMA)
        || text(object, "status") != Some("PASS")
        || text(object, "task_ref") != Some(task_ref.as_str())
        || number(object, "attempt") != Some(u64::from(attempt))
        || text(object, "worktree_ref") != Some(worktree_ref)
    {
        return Err(rejected());
    }
    let result_digest = text(object, "result_digest").ok_or_else(rejected)?;
    let mut result_subject = result.clone();
    result_subject
        .as_object_mut()
        .ok_or_else(rejected)?
        .remove("result_digest");
    if result_digest != typed_json_digest("wsl2-preflight-result", &result_subject)? {
        return Err(rejected());
    }

    let environment = object.get("environment").ok_or_else(rejected)?;
    let environment_json = canonical_json(environment)?;
    let descriptor =
        ExecutionEnvironmentDescriptor::from_json(&environment_json).map_err(|_| rejected())?;
    if descriptor.verification_task_ref() != task_ref
        || descriptor.repository_head() != expected_repository_head
        || descriptor.path_mapping_windows_path() != windows_worktree_path
        || descriptor.path_mapping_linux_path() != descriptor.linux_repository_path()
    {
        return Err(rejected());
    }
    let expected = ExpectedWsl2Preflight {
        task_ref: task_ref.as_str().to_owned(),
        attempt,
        worktree_ref: worktree_ref.to_owned(),
        environment_ref: descriptor.environment_ref().as_str().to_owned(),
        immutable_snapshot_ref: descriptor.immutable_snapshot_ref().to_owned(),
        sandbox_policy_ref: descriptor.sandbox_policy_ref().to_owned(),
        privilege_boundary_ref: descriptor.privilege_boundary_ref().to_owned(),
        linux_cwd: descriptor.linux_repository_path().to_owned(),
        repository_head: expected_repository_head.to_owned(),
        process_fence: process_fence.to_owned(),
        retry_of: retry_of.map(str::to_owned),
        reconnect_of: reconnect_of.map(str::to_owned),
    };
    let receipt = object.get("receipt").ok_or_else(rejected)?;
    validate_wsl2_preflight_receipt(receipt, &expected)?;
    let receipt_object = receipt.as_object().ok_or_else(rejected)?;
    let environment_text = |pointer: &str| environment.pointer(pointer).and_then(Value::as_str);
    let fence_object = receipt_object
        .get("process_fence")
        .and_then(Value::as_object)
        .ok_or_else(rejected)?;
    let codex_home_subject = serde_json::json!({
        "distribution_identity_ref": environment_text("/distribution_identity/identity_digest").ok_or_else(rejected)?,
        "linux_codex_home": environment_text("/linux/codex_home").ok_or_else(rejected)?,
        "config_digest": environment_text("/linux/config_digest").ok_or_else(rejected)?,
        "credential_authority_ref": environment_text("/credential_authority/authority_digest").ok_or_else(rejected)?,
    });
    let codex_home_digest = typed_json_digest("codex-home", &codex_home_subject)?;
    if receipt_object
        .get("distribution_identity_ref")
        .and_then(Value::as_str)
        != environment_text("/distribution_identity/identity_digest")
        || receipt_object
            .get("repository_identity")
            .and_then(Value::as_str)
            != environment_text("/linux/repository_identity")
        || receipt_object
            .get("credential_authority_ref")
            .and_then(Value::as_str)
            != environment_text("/credential_authority/authority_digest")
        || receipt_object
            .get("verification_toolchain_ref")
            .and_then(Value::as_str)
            != environment_text("/verification_toolchain/identity_digest")
        || fence_object.get("authority_ref").and_then(Value::as_str)
            != environment_text("/process_fence/identity_digest")
        || receipt_object
            .get("codex_home_digest")
            .and_then(Value::as_str)
            != Some(codex_home_digest.as_str())
    {
        return Err(rejected());
    }
    let receipt_digest = receipt
        .get("receipt_digest")
        .and_then(Value::as_str)
        .ok_or_else(rejected)?
        .to_owned();
    let bytes = canonical_json(receipt)?.into_bytes();
    let evidence = VerifiedManagedEvidence::new(
        ManagedEvidenceInput::new(
            project_id.clone(),
            task_ref.clone(),
            attempt,
            ManagedEvidenceKind::WorkerLifecycle,
            "application/json",
            WSL2_PREFLIGHT_SCHEMA,
            "lattice-runtime-wsl2-preflight-bridge",
            "1.0",
            producer_digest,
            created_at,
            bytes,
        )
        .map_err(|_| rejected())?,
    )
    .map_err(|_| rejected())?;
    Ok(VerifiedWsl2Preflight {
        descriptor,
        evidence,
        receipt_digest,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use lattice_contracts::ContentDigest;

    use super::{
        ExpectedWsl2Preflight, deterministic_process_fence, typed_json_digest,
        validate_wsl2_preflight_receipt,
    };

    fn expected() -> ExpectedWsl2Preflight {
        ExpectedWsl2Preflight {
            task_ref: "a".repeat(64),
            attempt: 1,
            worktree_ref: format!("worktree:sha256:{}", "b".repeat(64)),
            environment_ref: format!("execution-environment:sha256:{}", "c".repeat(64)),
            immutable_snapshot_ref: format!("wsl2-immutable-snapshot:sha256:{}", "8".repeat(64)),
            sandbox_policy_ref: format!("wsl2-sandbox-policy:sha256:{}", "9".repeat(64)),
            privilege_boundary_ref: format!("wsl2-privilege-boundary:sha256:{}", "a".repeat(64)),
            linux_cwd: "/home/zk/lattice/task/managed-worktrees/work-a".to_owned(),
            repository_head: "d".repeat(40),
            process_fence: "e".repeat(64),
            retry_of: None,
            reconnect_of: None,
        }
    }

    fn receipt(expected: &ExpectedWsl2Preflight) -> Value {
        let mut value = json!({
            "schema": "lattice.wsl2-zero-model-preflight/1.0",
            "status": "PASS",
            "task_ref": expected.task_ref,
            "attempt": expected.attempt,
            "worktree_ref": expected.worktree_ref,
            "execution_environment_ref": expected.environment_ref,
            "descriptor_digest": expected.environment_ref,
            "distribution_identity_ref": format!("wsl2-distribution:sha256:{}", "f".repeat(64)),
            "linux_cwd": expected.linux_cwd,
            "repository_head": expected.repository_head,
            "repository_identity": format!("repository:sha256:{}", "1".repeat(64)),
            "codex_home_digest": format!("codex-home:sha256:{}", "2".repeat(64)),
            "credential_authority_ref": format!("wsl2-credential-authority:sha256:{}", "3".repeat(64)),
            "credential_seal_digest": format!("credential-seal:sha256:{}", "4".repeat(64)),
            "verification_toolchain_ref": format!("wsl2-verification-toolchain:sha256:{}", "5".repeat(64)),
            "immutable_snapshot_ref": expected.immutable_snapshot_ref,
            "sandbox_policy_ref": expected.sandbox_policy_ref,
            "privilege_boundary_ref": expected.privilege_boundary_ref,
            "process_fence": {
                "fence": expected.process_fence,
                "authority_ref": format!("wsl2-process-fence-authority:sha256:{}", "6".repeat(64)),
                "service_unit": "lattice-wsl2-aaaaaaaaaaaaaaaa-preflight-eeeeeeeeeeee.service",
                "cgroup_path": "/user.slice/user-1000.slice/app.slice/lattice.service",
                "cgroup_version": 2,
                "delegated": false,
                "boot_id_digest": format!("wsl-boot:sha256:{}", "7".repeat(64)),
                "supervisor_zero_descendants": true,
                "outer_post_exit": {
                    "unit": "lattice-wsl2-aaaaaaaaaaaaaaaa-preflight-eeeeeeeeeeee.service",
                    "active_state": "inactive", "sub_state": "dead", "result": "success",
                    "cgroup_path": "/user.slice/user-1000.slice/app.slice/lattice.service",
                    "delegate": "no", "cgroup_exists": true, "populated": 0
                }
            },
            "isolation": {"root": "/home/zk/lattice/task/isolation", "owner_uid": 1000},
            "probes": {"technical": {"status": "PASS"}},
            "effect_counters": {"account_read": 0, "thread_start": 0, "turn_start": 0, "provider_effect_count": 0},
            "provider_effect_count": 0,
            "bounds": {"stdout_limit_bytes": 1048576, "stderr_limit_bytes": 1048576, "stdout_observed_bytes": 4096, "stderr_observed_bytes": 4096},
            "timeout": {"timeout_ms": 170000, "timed_out": false, "interrupted": false},
            "continuation": {"attempt": expected.attempt, "retry_of": expected.retry_of, "reconnect_of": expected.reconnect_of},
            "connector_auth_ready": false,
            "receipt_digest": null
        });
        let subject = value
            .as_object_mut()
            .expect("receipt")
            .remove("receipt_digest")
            .expect("digest slot");
        drop(subject);
        let digest = typed_json_digest("wsl2-preflight", &value).expect("digest");
        value
            .as_object_mut()
            .expect("receipt")
            .insert("receipt_digest".to_owned(), Value::String(digest));
        value
    }

    #[test]
    fn technical_receipt_is_exact_and_nonzero_effect_or_live_cgroup_fails_closed() {
        let expected = expected();
        let exact = receipt(&expected);
        validate_wsl2_preflight_receipt(&exact, &expected).expect("exact receipt");

        let mut nonzero = exact.clone();
        nonzero["provider_effect_count"] = json!(1);
        assert!(validate_wsl2_preflight_receipt(&nonzero, &expected).is_err());

        let mut populated = exact;
        populated["process_fence"]["outer_post_exit"]["populated"] = json!(1);
        assert!(validate_wsl2_preflight_receipt(&populated, &expected).is_err());

        for key in [
            "immutable_snapshot_ref",
            "sandbox_policy_ref",
            "privilege_boundary_ref",
        ] {
            let mut substituted = receipt(&expected);
            substituted[key] = json!(format!("wsl2-immutable-snapshot:sha256:{}", "0".repeat(64)));
            assert!(
                validate_wsl2_preflight_receipt(&substituted, &expected).is_err(),
                "{key}"
            );
        }
    }

    #[test]
    fn process_fence_is_exact_replay_bound_and_rotates_only_with_attempt_authority() {
        let task_ref = ContentDigest::from_sha256("a".repeat(64)).expect("task ref");
        let environment_ref = format!("execution-environment:sha256:{}", "b".repeat(64));
        let worktree_ref = format!("worktree:sha256:{}", "c".repeat(64));
        let head = "d".repeat(40);
        let first = deterministic_process_fence(
            &environment_ref,
            &task_ref,
            1,
            &worktree_ref,
            &head,
            None,
            None,
        )
        .expect("deterministic fence");
        assert_eq!(first.len(), 64);
        assert!(
            first
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        );
        assert_eq!(
            first,
            deterministic_process_fence(
                &environment_ref,
                &task_ref,
                1,
                &worktree_ref,
                &head,
                None,
                None,
            )
            .expect("exact replay fence")
        );
        let retry_ref = format!("attempt-receipt:sha256:{}", "e".repeat(64));
        for changed in [
            deterministic_process_fence(
                &environment_ref,
                &task_ref,
                2,
                &worktree_ref,
                &head,
                None,
                None,
            ),
            deterministic_process_fence(
                &environment_ref,
                &task_ref,
                1,
                &worktree_ref,
                &head,
                Some(retry_ref.as_str()),
                None,
            ),
        ] {
            assert_ne!(first, changed.expect("rotated fence"));
        }
    }
}
