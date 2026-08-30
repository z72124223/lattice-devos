//! Concrete process adapter for the managed Control/Codex JSONL bridge.
//!
//! The bridge owns the live App Server connection. This adapter keeps its
//! stdin/stdout open for the exact attempt so an interrupt targets that same
//! active turn; no shell is involved and objective text is only JSON data.

use std::collections::BTreeMap;
use std::env;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, TryRecvError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration as StdDuration, Instant};

use lattice_artifact_store::{ManagedEvidenceInput, ManagedEvidenceKind, VerifiedManagedEvidence};
use lattice_codex_adapter::{ManagedCodexSpawnIdentity, SupervisedDuplexChild};
use lattice_contracts::{ContentDigest, ProjectId};
use lattice_foreman_state::{AttemptPacketIdentity, ModelSelection, WorkerTerminal};
use lattice_ports::{
    ManagedCodexWorkerPort, ManagedModelAvailability, ManagedPortError, ManagedPortErrorKind,
    ManagedPortResult, ManagedTerminalCandidate, ManagedWorkerExecutionEvent,
    ManagedWorkerObservation, ManagedWorkerPrestartRecovery, ManagedWorkerReconciliation,
    VerifiedWorkerAttemptRecord,
};
use lattice_postgres_foreman::ExecutionEnvironmentDescriptor;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::managed_file_identity::{
    ManagedEffectBundleGuard, ManagedFileIdentity, ManagedFileIdentityBundle, ManagedFileSeal,
    capture_managed_codex_home_guard, managed_shell_path,
};

const COMMAND_SCHEMA: &str = "lattice.managed-codex-worker-command/1.0";
const CONTROL_SCHEMA: &str = "lattice.managed-codex-worker-control/1.0";
const RESULT_SCHEMA: &str = "lattice.managed-codex-worker-bridge-result/1.0";
const MAX_BRIDGE_LINE_BYTES: usize = 16_384;
const WSL2_PROVIDER_SUBTREE_MARKER_SCHEMA: &str = "lattice.wsl2-provider-subtree-marker/1.0";
const WSL2_PROVIDER_SUBTREE_RECEIPT_SCHEMA: &str = "lattice.wsl2-provider-subtree-receipt/1.0";
const WSL2_PROVIDER_SUBTREE_RECONCILIATION_SCHEMA: &str =
    "lattice.wsl2-provider-subtree-reconciliation/1.0";
const WSL2_PROVIDER_SUBTREE_RECONCILE_REQUEST_SCHEMA: &str =
    "lattice.wsl2-provider-subtree-reconcile-request/1.0";
const WSL2_PROVIDER_SUBTREE_RECONCILE_MAX_INPUT_BYTES: usize = 131_072;
const WSL2_PROVIDER_SUBTREE_RECONCILE_TIMEOUT: StdDuration = StdDuration::from_secs(45);
const BRIDGE_TEARDOWN_GRACE: StdDuration = StdDuration::from_secs(5);
const BRIDGE_RECORD_QUEUE: usize = 32;
const CANCELLATION_POLL: StdDuration = StdDuration::from_millis(100);
const MAX_MANAGED_NODE_BYTES: u64 = 512 * 1_024 * 1_024;
const MAX_MANAGED_CODEX_BYTES: u64 = 512 * 1_024 * 1_024;
const MAX_MANAGED_WORKER_BRIDGE_BYTES: u64 = 4 * 1_024 * 1_024;
const MAX_MANAGED_WORKER_DEPENDENCY_BYTES: u64 = 8 * 1_024 * 1_024;
const MANAGED_GRACEFUL_SHUTDOWN_IDLE: &str = "LATTICE_MANAGED_GRACEFUL_SHUTDOWN_IDLE";
const MANAGED_GRACEFUL_SHUTDOWN_RECEIPT_REQUIRED: &str =
    "LATTICE_MANAGED_GRACEFUL_SHUTDOWN_RECEIPT_REQUIRED";
const MANAGED_MODEL_PROBE_TIMEOUT_RECONCILIATION_REQUIRED: &str =
    "LATTICE_MANAGED_MODEL_PROBE_TIMEOUT_RECONCILIATION_REQUIRED";

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn valid_raw_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn typed_sha256(value: &str, domain: &str) -> bool {
    value
        .strip_prefix(domain)
        .and_then(|suffix| suffix.strip_prefix(":sha256:"))
        .is_some_and(valid_raw_sha256)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Wsl2ProviderSubtreeEvidenceKind {
    Open,
    Closed,
    Reconciled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ValidatedWsl2ProviderSubtreeEvidence {
    kind: Wsl2ProviderSubtreeEvidenceKind,
    schema: &'static str,
    role: &'static str,
    source_preflight_descriptor_digest: String,
    provider_subtree_segment_ref: String,
    source_marker_digest: Option<String>,
    retry_of: Option<String>,
    reconnect_of: Option<String>,
    closure_digest: String,
    provider_effect_count_before: u64,
    provider_effect_count_after: u64,
}

impl ValidatedWsl2ProviderSubtreeEvidence {
    pub(crate) const fn kind(&self) -> Wsl2ProviderSubtreeEvidenceKind {
        self.kind
    }

    pub(crate) const fn schema(&self) -> &'static str {
        self.schema
    }

    pub(crate) const fn role(&self) -> &'static str {
        self.role
    }

    pub(crate) fn source_preflight_descriptor_digest(&self) -> &str {
        &self.source_preflight_descriptor_digest
    }

    pub(crate) fn closure_digest(&self) -> &str {
        &self.closure_digest
    }

    pub(crate) fn provider_subtree_segment_ref(&self) -> &str {
        &self.provider_subtree_segment_ref
    }

    pub(crate) fn source_marker_digest(&self) -> Option<&str> {
        self.source_marker_digest.as_deref()
    }

    pub(crate) fn retry_of(&self) -> Option<&str> {
        self.retry_of.as_deref()
    }

    pub(crate) fn reconnect_of(&self) -> Option<&str> {
        self.reconnect_of.as_deref()
    }

    pub(crate) const fn provider_effect_count_before(&self) -> u64 {
        self.provider_effect_count_before
    }

    pub(crate) const fn provider_effect_count_after(&self) -> u64 {
        self.provider_effect_count_after
    }
}

struct ProviderSubtreeAnchor<'a> {
    packet: &'a AttemptPacketIdentity,
    descriptor_digest: String,
    preflight_descriptor_digest: String,
    preflight_content_digest: String,
    preflight_receipt_digest: String,
    credential_seal_digest: String,
    boot_id_digest: String,
    fence: String,
    unit: String,
    retry_of: Option<String>,
    reconnect_of: Option<String>,
}

fn exact_value_keys(value: &Value, expected: &[&str]) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    object.len() == expected.len() && expected.iter().all(|key| object.contains_key(*key))
}

fn value_str<'a>(value: &'a Value, key: &str) -> ManagedPortResult<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"))
}

fn same_optional_string(value: Option<&Value>, expected: Option<&str>) -> bool {
    match (value, expected) {
        (Some(Value::Null), None) => true,
        (Some(Value::String(actual)), Some(expected)) => actual == expected,
        _ => false,
    }
}

fn canonical_embedded_digest(
    value: &Value,
    digest_key: &str,
    domain: &str,
) -> ManagedPortResult<String> {
    let mut subject = value.clone();
    subject
        .as_object_mut()
        .ok_or_else(|| known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"))?
        .remove(digest_key)
        .ok_or_else(|| known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"))?;
    let bytes = serde_json::to_vec(&subject)
        .map_err(|_| known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"))?;
    Ok(format!("{domain}:sha256:{}", sha256_hex(&bytes)))
}

fn provider_subtree_anchor<'a>(
    packet: &'a AttemptPacketIdentity,
    descriptor_json: &str,
    preflight: &VerifiedManagedEvidence,
) -> ManagedPortResult<ProviderSubtreeAnchor<'a>> {
    if preflight.kind() != ManagedEvidenceKind::WorkerLifecycle
        || preflight.payload_schema() != "lattice.wsl2-zero-model-preflight/1.0"
        || preflight.media_type() != "application/json"
        || preflight.task_ref().as_str() != packet.task_ref()
        || preflight.attempt() != packet.attempt()
        || preflight.content_digest().as_str() != sha256_hex(preflight.bytes())
    {
        return Err(known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"));
    }
    let descriptor: Value = serde_json::from_str(descriptor_json)
        .map_err(|_| known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"))?;
    let receipt: Value = serde_json::from_slice(preflight.bytes())
        .map_err(|_| known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"))?;
    let descriptor_digest = sha256_hex(descriptor_json.as_bytes());
    let environment_ref = descriptor
        .get("identity_digest")
        .and_then(Value::as_str)
        .ok_or_else(|| known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"))?;
    let process_fence = receipt
        .get("process_fence")
        .and_then(Value::as_object)
        .ok_or_else(|| known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"))?;
    let continuation = receipt
        .get("continuation")
        .and_then(Value::as_object)
        .ok_or_else(|| known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"))?;
    let retry_of = continuation
        .get("retry_of")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let reconnect_of = continuation
        .get("reconnect_of")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let fence = process_fence
        .get("fence")
        .and_then(Value::as_str)
        .filter(|value| valid_raw_sha256(value))
        .ok_or_else(|| known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"))?
        .to_owned();
    if receipt.get("schema").and_then(Value::as_str)
        != Some("lattice.wsl2-zero-model-preflight/1.0")
        || receipt.get("status").and_then(Value::as_str) != Some("PASS")
        || receipt.get("task_ref").and_then(Value::as_str) != Some(packet.task_ref())
        || receipt.get("attempt").and_then(Value::as_u64) != Some(u64::from(packet.attempt()))
        || receipt.get("worktree_ref").and_then(Value::as_str) != Some(packet.worktree_ref())
        || receipt.get("repository_head").and_then(Value::as_str) != Some(packet.base_commit())
        || receipt
            .get("execution_environment_ref")
            .and_then(Value::as_str)
            != Some(packet.execution_environment_ref())
        || environment_ref != packet.execution_environment_ref()
        || receipt.get("provider_effect_count").and_then(Value::as_u64) != Some(0)
        || retry_of.is_some() && reconnect_of.is_some()
        || packet.task_ref().len() < 16
    {
        return Err(known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"));
    }
    let credential_seal_digest = value_str(&receipt, "credential_seal_digest")?.to_owned();
    let boot_id_digest = process_fence
        .get("boot_id_digest")
        .and_then(Value::as_str)
        .filter(|value| typed_sha256(value, "wsl-boot"))
        .ok_or_else(|| known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"))?
        .to_owned();
    let preflight_receipt_digest = value_str(&receipt, "receipt_digest")?.to_owned();
    if !typed_sha256(&credential_seal_digest, "credential-seal")
        || !typed_sha256(&preflight_receipt_digest, "wsl2-preflight")
    {
        return Err(known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"));
    }
    let unit = format!(
        "lattice-wsl2-{}-provider-{}.service",
        &packet.task_ref()[..16],
        &fence[..12]
    );
    Ok(ProviderSubtreeAnchor {
        packet,
        descriptor_digest,
        preflight_descriptor_digest: preflight.descriptor_digest().as_str().to_owned(),
        preflight_content_digest: preflight.content_digest().as_str().to_owned(),
        preflight_receipt_digest,
        credential_seal_digest,
        boot_id_digest,
        fence,
        unit,
        retry_of,
        reconnect_of,
    })
}

fn validate_provider_common(value: &Value, anchor: &ProviderSubtreeAnchor<'_>) -> bool {
    value.get("task_ref").and_then(Value::as_str) == Some(anchor.packet.task_ref())
        && value.get("attempt").and_then(Value::as_u64) == Some(u64::from(anchor.packet.attempt()))
        && value.get("packet_digest").and_then(Value::as_str) == Some(anchor.packet.digest())
        && value.get("worktree_ref").and_then(Value::as_str) == Some(anchor.packet.worktree_ref())
        && value.get("repository_head").and_then(Value::as_str) == Some(anchor.packet.base_commit())
        && value
            .get("execution_environment_ref")
            .and_then(Value::as_str)
            == Some(anchor.packet.execution_environment_ref())
        && value.get("descriptor_digest").and_then(Value::as_str)
            == Some(anchor.descriptor_digest.as_str())
        && value
            .get("source_preflight_descriptor_digest")
            .and_then(Value::as_str)
            == Some(anchor.preflight_descriptor_digest.as_str())
        && value
            .get("source_preflight_content_digest")
            .and_then(Value::as_str)
            == Some(anchor.preflight_content_digest.as_str())
        && value
            .get("source_preflight_receipt_digest")
            .and_then(Value::as_str)
            == Some(anchor.preflight_receipt_digest.as_str())
        && value.get("role").and_then(Value::as_str) == Some("PROVIDER")
}

fn validate_provider_process_marker(value: &Value, anchor: &ProviderSubtreeAnchor<'_>) -> bool {
    exact_value_keys(
        value,
        &[
            "schema",
            "fence",
            "unit",
            "execution_environment_ref",
            "credential_seal_digest",
            "boot_id_digest",
            "pid",
            "process_start_ticks",
            "process_group_id",
            "cgroup_path",
            "cgroup_version",
            "delegated",
            "attempt",
            "retry_of",
            "reconnect_of",
        ],
    ) && value.get("schema").and_then(Value::as_str) == Some("lattice.wsl2-process-fence/1.1")
        && value.get("fence").and_then(Value::as_str) == Some(anchor.fence.as_str())
        && value.get("unit").and_then(Value::as_str) == Some(anchor.unit.as_str())
        && value
            .get("execution_environment_ref")
            .and_then(Value::as_str)
            == Some(anchor.packet.execution_environment_ref())
        && value.get("credential_seal_digest").and_then(Value::as_str)
            == Some(anchor.credential_seal_digest.as_str())
        && value.get("boot_id_digest").and_then(Value::as_str)
            == Some(anchor.boot_id_digest.as_str())
        && value
            .get("pid")
            .and_then(Value::as_u64)
            .is_some_and(|pid| pid > 0)
        && value
            .get("process_start_ticks")
            .and_then(Value::as_str)
            .is_some_and(|ticks| {
                !ticks.is_empty() && ticks.bytes().all(|byte| byte.is_ascii_digit())
            })
        && value
            .get("process_group_id")
            .and_then(Value::as_u64)
            .is_some_and(|pid| pid > 0)
        && value
            .get("cgroup_path")
            .and_then(Value::as_str)
            .is_some_and(|path| {
                path.starts_with("/user.slice/")
                    && path.ends_with(&format!("/{}", anchor.unit))
                    && !path.contains("..")
                    && !path.contains('\\')
            })
        && value.get("cgroup_version").and_then(Value::as_u64) == Some(2)
        && value.get("delegated").and_then(Value::as_bool) == Some(false)
        && value.get("attempt").and_then(Value::as_u64) == Some(u64::from(anchor.packet.attempt()))
        && same_optional_string(value.get("retry_of"), anchor.retry_of.as_deref())
        && same_optional_string(value.get("reconnect_of"), anchor.reconnect_of.as_deref())
}

fn validate_provider_outer_exit(
    value: &Value,
    marker: &Value,
    anchor: &ProviderSubtreeAnchor<'_>,
) -> bool {
    let cgroup_closed = match (
        value.get("cgroup_exists").and_then(Value::as_bool),
        value.get("populated"),
    ) {
        (Some(false), Some(Value::Null)) => true,
        (Some(true), Some(Value::Number(number))) => number.as_u64() == Some(0),
        _ => false,
    };
    exact_value_keys(
        value,
        &[
            "schema",
            "unit",
            "fence",
            "cgroup_path",
            "boot_id_digest",
            "active_state",
            "sub_state",
            "result",
            "delegate",
            "cgroup_exists",
            "populated",
        ],
    ) && value.get("schema").and_then(Value::as_str)
        == Some("lattice.wsl2-provider-outer-post-exit/1.0")
        && value.get("unit") == marker.get("unit")
        && value.get("fence") == marker.get("fence")
        && value.get("cgroup_path") == marker.get("cgroup_path")
        && value.get("boot_id_digest").and_then(Value::as_str)
            == Some(anchor.boot_id_digest.as_str())
        && value.get("active_state").and_then(Value::as_str) == Some("inactive")
        && value.get("sub_state").and_then(Value::as_str) == Some("dead")
        && value
            .get("result")
            .and_then(Value::as_str)
            .is_some_and(|result| {
                !result.is_empty()
                    && result.len() <= 32
                    && result.bytes().all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'
                    })
            })
        && value.get("delegate").and_then(Value::as_str) == Some("no")
        && cgroup_closed
}

fn validate_provider_file_seal(value: &Value, library: bool) -> bool {
    let expected = if library {
        &[
            "manifest_path",
            "path",
            "resolved_path",
            "sha256",
            "device",
            "inode",
            "owner_uid",
            "mode",
            "size",
        ][..]
    } else {
        &[
            "path",
            "resolved_path",
            "sha256",
            "device",
            "inode",
            "owner_uid",
            "mode",
            "size",
        ][..]
    };
    let digits = |field: &str| {
        value
            .get(field)
            .and_then(Value::as_str)
            .is_some_and(|text| !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()))
    };
    exact_value_keys(value, expected)
        && value
            .get("path")
            .and_then(Value::as_str)
            .is_some_and(|path| path.starts_with('/') && path.len() <= 1_024)
        && value
            .get("resolved_path")
            .and_then(Value::as_str)
            .is_some_and(|path| path.starts_with('/') && path.len() <= 1_024)
        && value
            .get("sha256")
            .and_then(Value::as_str)
            .is_some_and(valid_raw_sha256)
        && digits("device")
        && digits("inode")
        && value.get("owner_uid").and_then(Value::as_u64).is_some()
        && value
            .get("mode")
            .and_then(Value::as_u64)
            .is_some_and(|mode| mode > 0 && mode & 0o022 == 0)
        && value
            .get("size")
            .and_then(Value::as_u64)
            .is_some_and(|size| size > 0)
        && (!library
            || value
                .get("manifest_path")
                .and_then(Value::as_str)
                .is_some_and(|path| {
                    !path.is_empty()
                        && path.len() <= 128
                        && path.bytes().all(|byte| {
                            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')
                        })
                }))
}

fn validate_provider_cleanup(value: &Value) -> bool {
    let Some(actions) = value.get("actions").and_then(Value::as_array) else {
        return false;
    };
    let sequence = ["TERM", "STOP", "KILL", "FORCE_STOP"];
    exact_value_keys(value, &["schema", "actions"])
        && value.get("schema").and_then(Value::as_str)
            == Some("lattice.wsl2-provider-subtree-cleanup/1.0")
        && matches!(actions.len(), 0 | 2 | 4)
        && actions.iter().enumerate().all(|(index, action)| {
            exact_value_keys(
                action,
                &[
                    "sequence",
                    "action",
                    "result",
                    "exit_code",
                    "signal",
                    "stdout_bytes",
                    "stderr_bytes",
                    "stdout_sha256",
                    "stderr_sha256",
                ],
            ) && action.get("sequence").and_then(Value::as_u64) == u64::try_from(index + 1).ok()
                && action.get("action").and_then(Value::as_str) == Some(sequence[index])
                && action
                    .get("result")
                    .and_then(Value::as_str)
                    .is_some_and(|result| {
                        matches!(result, "SUCCESS" | "EXIT_NONZERO" | "TRANSPORT_ERROR")
                    })
                && action.get("exit_code").is_some_and(|code| {
                    code.is_null() || code.as_u64().is_some_and(|code| code <= 255)
                })
                && action.get("signal").is_some_and(|signal| {
                    signal.is_null()
                        || signal.as_str().is_some_and(|signal| {
                            !signal.is_empty()
                                && signal.len() <= 32
                                && signal
                                    .bytes()
                                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
                        })
                })
                && action
                    .get("stdout_bytes")
                    .and_then(Value::as_u64)
                    .is_some_and(|bytes| bytes <= 65_536)
                && action
                    .get("stderr_bytes")
                    .and_then(Value::as_u64)
                    .is_some_and(|bytes| bytes <= 65_536)
                && action
                    .get("stdout_sha256")
                    .and_then(Value::as_str)
                    .is_some_and(valid_raw_sha256)
                && action
                    .get("stderr_sha256")
                    .and_then(Value::as_str)
                    .is_some_and(valid_raw_sha256)
        })
}

fn provider_subtree_segment_ref(anchor: &ProviderSubtreeAnchor<'_>) -> ManagedPortResult<String> {
    let value = json!({
        "task_ref": anchor.packet.task_ref(),
        "attempt": anchor.packet.attempt(),
        "source_preflight_descriptor_digest": anchor.preflight_descriptor_digest,
        "source_preflight_content_digest": anchor.preflight_content_digest,
        "source_preflight_receipt_digest": anchor.preflight_receipt_digest,
        "fence": anchor.fence,
        "role": "PROVIDER",
        "continuation": {
            "retry_of": anchor.retry_of,
            "reconnect_of": anchor.reconnect_of,
        },
    });
    let bytes = serde_json::to_vec(&value)
        .map_err(|_| known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"))?;
    Ok(format!(
        "provider-subtree-segment:sha256:{}",
        sha256_hex(&bytes)
    ))
}

fn valid_continuation(value: &Value, anchor: &ProviderSubtreeAnchor<'_>) -> bool {
    exact_value_keys(value, &["retry_of", "reconnect_of"])
        && same_optional_string(value.get("retry_of"), anchor.retry_of.as_deref())
        && same_optional_string(value.get("reconnect_of"), anchor.reconnect_of.as_deref())
}

fn validate_provider_subtree_exit(
    value: &Value,
    marker: &Value,
    anchor: &ProviderSubtreeAnchor<'_>,
) -> bool {
    let Some(stdout_bytes) = value.get("stdout_bytes").and_then(Value::as_u64) else {
        return false;
    };
    let Some(stderr_bytes) = value.get("stderr_bytes").and_then(Value::as_u64) else {
        return false;
    };
    let Some(stdout_limit) = value.get("stdout_limit_bytes").and_then(Value::as_u64) else {
        return false;
    };
    let Some(stderr_limit) = value.get("stderr_limit_bytes").and_then(Value::as_u64) else {
        return false;
    };
    let Some(tools) = value.get("tool_input_identities") else {
        return false;
    };
    exact_value_keys(
        value,
        &[
            "schema",
            "fence",
            "unit",
            "execution_environment_ref",
            "credential_seal_digest",
            "cgroup_path",
            "zero_descendants",
            "credential_seal_intact",
            "credential_watch_intact",
            "keyring_daemon_sha256",
            "keyring_library_manifest_digest",
            "tool_input_identities",
            "stdout_bytes",
            "stderr_bytes",
            "stdout_limit_bytes",
            "stderr_limit_bytes",
            "output_bound_exceeded",
            "timeout_ms",
            "timed_out",
            "interrupted",
            "stdin_bytes",
            "stdin_sha256",
            "stdin_complete",
            "attempt",
            "retry_of",
            "reconnect_of",
            "exit_code",
            "exit_signal",
        ],
    ) && exact_value_keys(
        tools,
        &[
            "executable",
            "verifier_tool",
            "sandbox_helper",
            "node_runtime",
            "rustc",
            "rustdoc",
            "keyring_daemon",
            "keyring_libraries",
        ],
    ) && value.get("schema").and_then(Value::as_str) == Some("lattice.wsl2-subtree-exit/1.2")
        && value.get("fence") == marker.get("fence")
        && value.get("unit") == marker.get("unit")
        && value
            .get("execution_environment_ref")
            .and_then(Value::as_str)
            == Some(anchor.packet.execution_environment_ref())
        && value.get("credential_seal_digest").and_then(Value::as_str)
            == Some(anchor.credential_seal_digest.as_str())
        && value.get("cgroup_path") == marker.get("cgroup_path")
        && value.get("zero_descendants").and_then(Value::as_bool) == Some(true)
        && value.get("credential_seal_intact").and_then(Value::as_bool) == Some(true)
        && value
            .get("credential_watch_intact")
            .and_then(Value::as_bool)
            == Some(true)
        && value
            .get("keyring_daemon_sha256")
            .and_then(Value::as_str)
            .is_some_and(valid_raw_sha256)
        && value
            .get("keyring_library_manifest_digest")
            .and_then(Value::as_str)
            .is_some_and(|digest| typed_sha256(digest, "keyring-library-manifest"))
        && tools.get("verifier_tool") == Some(&Value::Null)
        && tools.get("node_runtime") == Some(&Value::Null)
        && tools.get("rustc") == Some(&Value::Null)
        && tools.get("rustdoc") == Some(&Value::Null)
        && tools
            .get("executable")
            .is_some_and(|seal| validate_provider_file_seal(seal, false))
        && tools
            .get("sandbox_helper")
            .is_some_and(|seal| validate_provider_file_seal(seal, false))
        && tools
            .get("keyring_daemon")
            .is_some_and(|seal| validate_provider_file_seal(seal, false))
        && tools
            .get("keyring_libraries")
            .and_then(Value::as_array)
            .is_some_and(|entries| {
                entries.len() == 2
                    && entries
                        .iter()
                        .all(|seal| validate_provider_file_seal(seal, true))
            })
        && stdout_limit >= 1_024
        && stderr_limit >= 1_024
        && stdout_bytes <= stdout_limit
        && stderr_bytes <= stderr_limit
        && value.get("output_bound_exceeded").and_then(Value::as_bool) == Some(false)
        && value
            .get("timeout_ms")
            .and_then(Value::as_u64)
            .is_some_and(|timeout| timeout >= 1_000)
        && value.get("timed_out").and_then(Value::as_bool) == Some(false)
        && value.get("interrupted").and_then(Value::as_bool) == Some(false)
        && value.get("stdin_bytes").and_then(Value::as_u64).is_some()
        && value
            .get("stdin_sha256")
            .and_then(Value::as_str)
            .is_some_and(valid_raw_sha256)
        && value.get("stdin_complete").and_then(Value::as_bool) == Some(true)
        && value.get("attempt").and_then(Value::as_u64) == Some(u64::from(anchor.packet.attempt()))
        && same_optional_string(value.get("retry_of"), anchor.retry_of.as_deref())
        && same_optional_string(value.get("reconnect_of"), anchor.reconnect_of.as_deref())
        && value
            .get("exit_code")
            .is_some_and(|exit| exit.is_null() || exit.as_u64().is_some_and(|code| code <= 255))
        && value.get("exit_signal").is_some_and(|signal| {
            signal.is_null()
                || signal.as_str().is_some_and(|name| {
                    name.starts_with("SIG")
                        && name.len() <= 27
                        && name
                            .bytes()
                            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
                })
        })
}

/// Validates a persisted provider-subtree lifecycle artifact against the
/// exact packet, descriptor, source preflight, and optional OPEN segment.
pub(crate) fn validate_wsl2_provider_subtree_evidence(
    packet: &AttemptPacketIdentity,
    descriptor_json: &str,
    preflight: &VerifiedManagedEvidence,
    open_marker: Option<&VerifiedManagedEvidence>,
    evidence: &VerifiedManagedEvidence,
) -> ManagedPortResult<ValidatedWsl2ProviderSubtreeEvidence> {
    let anchor = provider_subtree_anchor(packet, descriptor_json, preflight)?;
    if evidence.kind() != ManagedEvidenceKind::WorkerLifecycle
        || evidence.media_type() != "application/json"
        || evidence.task_ref().as_str() != packet.task_ref()
        || evidence.attempt() != packet.attempt()
        || evidence.bytes().len() > MAX_BRIDGE_LINE_BYTES
    {
        return Err(known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"));
    }
    let value: Value = serde_json::from_slice(evidence.bytes())
        .map_err(|_| known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"))?;
    let schema = value_str(&value, "schema")?;
    let expected_producer = match schema {
        WSL2_PROVIDER_SUBTREE_MARKER_SCHEMA | WSL2_PROVIDER_SUBTREE_RECEIPT_SCHEMA => {
            "lattice-managed-codex-worker"
        }
        WSL2_PROVIDER_SUBTREE_RECONCILIATION_SCHEMA => {
            "lattice-runtime-wsl2-provider-subtree-reconciler"
        }
        _ => {
            return Err(known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"));
        }
    };
    if evidence.payload_schema() != schema
        || evidence.project_id() != preflight.project_id()
        || evidence.producer_id() != expected_producer
        || evidence.producer_version() != env!("CARGO_PKG_VERSION")
        || evidence.content_digest().as_str() != sha256_hex(evidence.bytes())
    {
        return Err(known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"));
    }
    let expected_segment = provider_subtree_segment_ref(&anchor)?;
    if value
        .get("provider_subtree_segment_ref")
        .and_then(Value::as_str)
        != Some(expected_segment.as_str())
    {
        return Err(known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"));
    }
    match schema {
        WSL2_PROVIDER_SUBTREE_MARKER_SCHEMA => {
            if open_marker.is_some()
                || !exact_value_keys(
                    &value,
                    &[
                        "schema",
                        "status",
                        "task_ref",
                        "attempt",
                        "packet_digest",
                        "worktree_ref",
                        "repository_head",
                        "execution_environment_ref",
                        "descriptor_digest",
                        "source_preflight_descriptor_digest",
                        "source_preflight_content_digest",
                        "source_preflight_receipt_digest",
                        "role",
                        "provider_subtree_segment_ref",
                        "process_marker",
                        "boot_id_digest",
                        "credential_seal_digest",
                        "continuation",
                        "provider_effect_count",
                        "marker_digest",
                    ],
                )
                || value.get("status").and_then(Value::as_str) != Some("OPEN")
                || !validate_provider_common(&value, &anchor)
                || value.get("provider_effect_count").and_then(Value::as_u64) != Some(0)
                || value.get("boot_id_digest").and_then(Value::as_str)
                    != Some(anchor.boot_id_digest.as_str())
                || value.get("credential_seal_digest").and_then(Value::as_str)
                    != Some(anchor.credential_seal_digest.as_str())
                || !valid_continuation(value.get("continuation").unwrap_or(&Value::Null), &anchor)
                || !validate_provider_process_marker(
                    value.get("process_marker").unwrap_or(&Value::Null),
                    &anchor,
                )
            {
                return Err(known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"));
            }
            let digest = value_str(&value, "marker_digest")?.to_owned();
            if digest
                != canonical_embedded_digest(&value, "marker_digest", "provider-subtree-marker")?
            {
                return Err(known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"));
            }
            Ok(ValidatedWsl2ProviderSubtreeEvidence {
                kind: Wsl2ProviderSubtreeEvidenceKind::Open,
                schema: WSL2_PROVIDER_SUBTREE_MARKER_SCHEMA,
                role: "PROVIDER",
                source_preflight_descriptor_digest: anchor.preflight_descriptor_digest,
                provider_subtree_segment_ref: expected_segment,
                source_marker_digest: None,
                retry_of: anchor.retry_of,
                reconnect_of: anchor.reconnect_of,
                closure_digest: digest,
                provider_effect_count_before: 0,
                provider_effect_count_after: 0,
            })
        }
        WSL2_PROVIDER_SUBTREE_RECEIPT_SCHEMA => {
            let open = open_marker
                .ok_or_else(|| known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"))?;
            let validated_open = validate_wsl2_provider_subtree_evidence(
                packet,
                descriptor_json,
                preflight,
                None,
                open,
            )?;
            let open_value: Value = serde_json::from_slice(open.bytes())
                .map_err(|_| known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"))?;
            let marker = value.get("process_marker").unwrap_or(&Value::Null);
            let effects = value
                .get("provider_effect_count")
                .and_then(Value::as_u64)
                .filter(|count| *count <= 16)
                .ok_or_else(|| known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"))?;
            if !exact_value_keys(
                &value,
                &[
                    "schema",
                    "status",
                    "task_ref",
                    "attempt",
                    "packet_digest",
                    "worktree_ref",
                    "repository_head",
                    "execution_environment_ref",
                    "descriptor_digest",
                    "source_preflight_descriptor_digest",
                    "source_preflight_content_digest",
                    "source_preflight_receipt_digest",
                    "role",
                    "provider_subtree_segment_ref",
                    "source_marker_digest",
                    "process_marker",
                    "subtree_exit",
                    "outer_post_exit",
                    "boot_id_digest",
                    "credential_seal_digest",
                    "continuation",
                    "provider_effect_count",
                    "receipt_digest",
                ],
            ) || value.get("status").and_then(Value::as_str) != Some("CLOSED")
                || !validate_provider_common(&value, &anchor)
                || value.get("source_marker_digest").and_then(Value::as_str)
                    != Some(validated_open.closure_digest())
                || marker != open_value.get("process_marker").unwrap_or(&Value::Null)
                || !valid_continuation(value.get("continuation").unwrap_or(&Value::Null), &anchor)
                || !validate_provider_process_marker(marker, &anchor)
                || !validate_provider_subtree_exit(
                    value.get("subtree_exit").unwrap_or(&Value::Null),
                    marker,
                    &anchor,
                )
                || !validate_provider_outer_exit(
                    value.get("outer_post_exit").unwrap_or(&Value::Null),
                    marker,
                    &anchor,
                )
            {
                return Err(known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"));
            }
            let digest = value_str(&value, "receipt_digest")?.to_owned();
            if digest
                != canonical_embedded_digest(&value, "receipt_digest", "provider-subtree-receipt")?
            {
                return Err(known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"));
            }
            Ok(ValidatedWsl2ProviderSubtreeEvidence {
                kind: Wsl2ProviderSubtreeEvidenceKind::Closed,
                schema: WSL2_PROVIDER_SUBTREE_RECEIPT_SCHEMA,
                role: "PROVIDER",
                source_preflight_descriptor_digest: anchor.preflight_descriptor_digest,
                provider_subtree_segment_ref: expected_segment,
                source_marker_digest: Some(validated_open.closure_digest().to_owned()),
                retry_of: anchor.retry_of,
                reconnect_of: anchor.reconnect_of,
                closure_digest: digest,
                provider_effect_count_before: 0,
                provider_effect_count_after: effects,
            })
        }
        WSL2_PROVIDER_SUBTREE_RECONCILIATION_SCHEMA => {
            let marker_observation = value_str(&value, "marker_observation")?;
            let validated_open = match (marker_observation, open_marker) {
                ("PRESENT", Some(open)) => Some(validate_wsl2_provider_subtree_evidence(
                    packet,
                    descriptor_json,
                    preflight,
                    None,
                    open,
                )?),
                ("ABSENT_AFTER_TRANSPORT_LOSS", None) => None,
                _ => {
                    return Err(known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"));
                }
            };
            let before = value
                .get("provider_effect_count_before")
                .and_then(Value::as_u64)
                .filter(|count| *count <= 16)
                .ok_or_else(|| known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"))?;
            let after = value
                .get("provider_effect_count_after")
                .and_then(Value::as_u64)
                .ok_or_else(|| known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"))?;
            let marker = value.get("process_marker").unwrap_or(&Value::Null);
            let synthetic_marker = json!({
                "unit": anchor.unit,
                "fence": anchor.fence,
                "cgroup_path": value.get("cgroup_path").cloned().unwrap_or(Value::Null),
            });
            let outer_marker = if marker.is_null() {
                &synthetic_marker
            } else {
                marker
            };
            if !exact_value_keys(
                &value,
                &[
                    "schema",
                    "status",
                    "task_ref",
                    "attempt",
                    "worktree_ref",
                    "repository_head",
                    "execution_environment_ref",
                    "descriptor_digest",
                    "source_preflight_descriptor_digest",
                    "source_preflight_content_digest",
                    "source_preflight_receipt_digest",
                    "role",
                    "provider_subtree_segment_ref",
                    "marker_observation",
                    "source_marker_digest",
                    "packet_digest",
                    "process_marker",
                    "fence",
                    "unit",
                    "cgroup_path",
                    "boot_id_digest",
                    "credential_seal_digest",
                    "continuation",
                    "cleanup",
                    "outer_post_exit",
                    "provider_effect_count_before",
                    "provider_effect_count_after",
                    "reconciliation_digest",
                ],
            ) || value.get("status").and_then(Value::as_str) != Some("RECONCILED")
                || !validate_provider_common(&value, &anchor)
                || value.get("fence").and_then(Value::as_str) != Some(anchor.fence.as_str())
                || value.get("unit").and_then(Value::as_str) != Some(anchor.unit.as_str())
                || value.get("boot_id_digest").and_then(Value::as_str)
                    != Some(anchor.boot_id_digest.as_str())
                || value.get("credential_seal_digest").and_then(Value::as_str)
                    != Some(anchor.credential_seal_digest.as_str())
                || !valid_continuation(value.get("continuation").unwrap_or(&Value::Null), &anchor)
                || before != after
                || !validate_provider_cleanup(value.get("cleanup").unwrap_or(&Value::Null))
                || !validate_provider_outer_exit(
                    value.get("outer_post_exit").unwrap_or(&Value::Null),
                    outer_marker,
                    &anchor,
                )
                || validated_open.as_ref().is_some_and(|open| {
                    value.get("source_marker_digest").and_then(Value::as_str)
                        != Some(open.closure_digest())
                        || marker.is_null()
                        || !validate_provider_process_marker(marker, &anchor)
                })
                || validated_open.is_none()
                    && (value.get("source_marker_digest") != Some(&Value::Null)
                        || !marker.is_null())
            {
                return Err(known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"));
            }
            let digest = value_str(&value, "reconciliation_digest")?.to_owned();
            if digest
                != canonical_embedded_digest(
                    &value,
                    "reconciliation_digest",
                    "provider-subtree-reconciliation",
                )?
            {
                return Err(known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"));
            }
            Ok(ValidatedWsl2ProviderSubtreeEvidence {
                kind: Wsl2ProviderSubtreeEvidenceKind::Reconciled,
                schema: WSL2_PROVIDER_SUBTREE_RECONCILIATION_SCHEMA,
                role: "PROVIDER",
                source_preflight_descriptor_digest: anchor.preflight_descriptor_digest,
                provider_subtree_segment_ref: expected_segment,
                source_marker_digest: validated_open
                    .as_ref()
                    .map(|open| open.closure_digest().to_owned()),
                retry_of: anchor.retry_of,
                reconnect_of: anchor.reconnect_of,
                closure_digest: digest,
                provider_effect_count_before: before,
                provider_effect_count_after: after,
            })
        }
        _ => Err(known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED")),
    }
}

fn read_provider_reconcile_output(mut reader: impl Read, limit: usize) -> std::io::Result<Vec<u8>> {
    let take = u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1);
    let mut bytes = Vec::new();
    reader.by_ref().take(take).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(std::io::Error::other(
            "provider reconcile output bound exceeded",
        ));
    }
    Ok(bytes)
}

fn join_provider_reconcile_reader(
    reader: thread::JoinHandle<std::io::Result<Vec<u8>>>,
) -> ManagedPortResult<Vec<u8>> {
    reader
        .join()
        .map_err(|_| known("LATTICE_MANAGED_PROVIDER_SUBTREE_RECONCILIATION_REJECTED"))?
        .map_err(|_| known("LATTICE_MANAGED_PROVIDER_SUBTREE_RECONCILIATION_REJECTED"))
}

/// Runs the one sealed provider-subtree reconciler sibling with a closed host
/// environment. Role-specific callers must validate the returned payload
/// against their own durable packet and preflight authority before persisting
/// it.
pub(crate) fn execute_wsl2_subtree_reconciliation(
    node_executable: &Path,
    bridge_path: &Path,
    runtime_guard: &ManagedEffectBundleGuard,
    request: &Value,
) -> ManagedPortResult<Value> {
    let rejected = || known("LATTICE_MANAGED_PROVIDER_SUBTREE_RECONCILIATION_REJECTED");
    if !matches!(
        bridge_path.file_name().and_then(|name| name.to_str()),
        Some("managed-codex-worker-bridge.mjs" | "managed-semantic-reviewer.mjs")
    ) {
        return Err(rejected());
    }
    let reconcile_path = bridge_path
        .parent()
        .ok_or_else(rejected)?
        .join("wsl2-provider-subtree-reconcile.mjs");
    runtime_guard
        .verify()
        .and_then(|()| runtime_guard.covers_file(node_executable))
        .and_then(|()| runtime_guard.covers_file(bridge_path))
        .and_then(|()| runtime_guard.covers_file(&reconcile_path))
        .map_err(|_| rejected())?;
    let mut input = serde_json::to_vec(request).map_err(|_| rejected())?;
    input.push(b'\n');
    if input.len() > WSL2_PROVIDER_SUBTREE_RECONCILE_MAX_INPUT_BYTES {
        return Err(rejected());
    }

    let mut command = Command::new(node_executable);
    command.arg(&reconcile_path);
    if let Some(parent) = reconcile_path.parent() {
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
    if runtime_guard.verify().is_err() {
        let _ = child.terminate_and_reap();
        return Err(rejected());
    }
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
    let stdout_reader =
        thread::spawn(move || read_provider_reconcile_output(stdout, MAX_BRIDGE_LINE_BYTES));
    let stderr_reader =
        thread::spawn(move || read_provider_reconcile_output(stderr, MAX_BRIDGE_LINE_BYTES));
    let wrote = stdin.write_all(&input).and_then(|()| stdin.flush()).is_ok();
    drop(stdin);
    if !wrote {
        let _ = child.terminate_and_reap();
        let _ = join_provider_reconcile_reader(stdout_reader);
        let _ = join_provider_reconcile_reader(stderr_reader);
        return Err(rejected());
    }
    let deadline = Instant::now()
        .checked_add(WSL2_PROVIDER_SUBTREE_RECONCILE_TIMEOUT)
        .ok_or_else(rejected)?;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|_| rejected())? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.terminate_and_reap();
            let _ = join_provider_reconcile_reader(stdout_reader);
            let _ = join_provider_reconcile_reader(stderr_reader);
            return Err(known(
                "LATTICE_MANAGED_PROVIDER_SUBTREE_RECONCILIATION_TIMEOUT",
            ));
        }
        thread::sleep(StdDuration::from_millis(5));
    };
    let cleanup = child.terminate_and_reap().map_err(|_| rejected());
    let stdout = join_provider_reconcile_reader(stdout_reader)?;
    let stderr = join_provider_reconcile_reader(stderr_reader)?;
    cleanup?;
    if !status.success()
        || !stderr.is_empty()
        || !stdout.ends_with(b"\n")
        || stdout.contains(&b'\r')
        || stdout[..stdout.len().saturating_sub(1)].contains(&b'\n')
    {
        return Err(rejected());
    }
    let payload: Value =
        serde_json::from_slice(&stdout[..stdout.len() - 1]).map_err(|_| rejected())?;
    if payload.get("schema").and_then(Value::as_str)
        != Some(WSL2_PROVIDER_SUBTREE_RECONCILIATION_SCHEMA)
        || payload.get("status").and_then(Value::as_str) != Some("RECONCILED")
    {
        return Err(rejected());
    }
    Ok(payload)
}

/// Executes the fixed, sealed, zero-model reconciliation sibling and returns
/// a freshly validated lifecycle artifact. This function never opens a
/// repository and never derives a replacement process fence.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub(crate) fn run_wsl2_provider_subtree_reconciliation(
    node_executable: &Path,
    bridge_path: &Path,
    runtime_guard: &ManagedEffectBundleGuard,
    project_id: ProjectId,
    producer_digest: ContentDigest,
    packet: &AttemptPacketIdentity,
    descriptor_json: &str,
    preflight: &VerifiedManagedEvidence,
    open_marker: Option<&VerifiedManagedEvidence>,
    provider_effect_count_before: u64,
    provider_effect_count_after: u64,
) -> ManagedPortResult<VerifiedManagedEvidence> {
    let rejected = || known("LATTICE_MANAGED_PROVIDER_SUBTREE_RECONCILIATION_REJECTED");
    if bridge_path.file_name().and_then(|name| name.to_str())
        != Some("managed-codex-worker-bridge.mjs")
        || provider_effect_count_before > 16
        || provider_effect_count_after != provider_effect_count_before
        || project_id != *preflight.project_id()
    {
        return Err(rejected());
    }
    provider_subtree_anchor(packet, descriptor_json, preflight)?;
    let open_value = open_marker
        .map(|open| {
            validate_wsl2_provider_subtree_evidence(
                packet,
                descriptor_json,
                preflight,
                None,
                open,
            )?;
            serde_json::from_slice::<Value>(open.bytes()).map_err(|_| rejected())
        })
        .transpose()?;
    let preflight_receipt = std::str::from_utf8(preflight.bytes()).map_err(|_| rejected())?;
    if descriptor_json.len() > 65_536 || preflight_receipt.len() > 65_536 {
        return Err(rejected());
    }
    let request = json!({
        "schema": WSL2_PROVIDER_SUBTREE_RECONCILE_REQUEST_SCHEMA,
        "descriptor_json": descriptor_json,
        "descriptor_digest": sha256_hex(descriptor_json.as_bytes()),
        "source_preflight": {
            "descriptor_digest": preflight.descriptor_digest().as_str(),
            "content_digest": preflight.content_digest().as_str(),
            "receipt_json": preflight_receipt,
        },
        "open_marker": open_value,
        "packet_digest": packet.digest(),
        "provider_effect_count_before": provider_effect_count_before,
        "provider_effect_count_after": provider_effect_count_after,
    });
    let payload =
        execute_wsl2_subtree_reconciliation(node_executable, bridge_path, runtime_guard, &request)?;
    let task_ref = ContentDigest::from_sha256(packet.task_ref()).map_err(|_| rejected())?;
    let created_at = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|_| rejected())?;
    let evidence = VerifiedManagedEvidence::new(
        ManagedEvidenceInput::new(
            project_id,
            task_ref,
            packet.attempt(),
            ManagedEvidenceKind::WorkerLifecycle,
            "application/json",
            WSL2_PROVIDER_SUBTREE_RECONCILIATION_SCHEMA,
            "lattice-runtime-wsl2-provider-subtree-reconciler",
            env!("CARGO_PKG_VERSION"),
            producer_digest,
            created_at,
            serde_json::to_vec(&payload).map_err(|_| rejected())?,
        )
        .map_err(|_| rejected())?,
    )
    .map_err(|_| rejected())?;
    validate_wsl2_provider_subtree_evidence(
        packet,
        descriptor_json,
        preflight,
        open_marker,
        &evidence,
    )?;
    Ok(evidence)
}

#[derive(Clone)]
struct ManagedWslAuthIdentity {
    descriptor_sha256: String,
    codex_home_digest: String,
    config_digest: String,
}

fn capture_wsl_auth_identity(
    packet: &AttemptPacketIdentity,
    descriptor: Option<&str>,
    preflight_receipt: Option<&str>,
) -> ManagedPortResult<Option<ManagedWslAuthIdentity>> {
    let Some(descriptor) = descriptor else {
        if preflight_receipt.is_some() || !packet.is_native_windows_execution_environment() {
            return Err(known(
                "LATTICE_MANAGED_EXECUTION_ENVIRONMENT_IDENTITY_REJECTED",
            ));
        }
        return Ok(None);
    };
    let receipt: Value = serde_json::from_str(
        preflight_receipt
            .ok_or_else(|| known("LATTICE_MANAGED_EXECUTION_ENVIRONMENT_IDENTITY_REJECTED"))?,
    )
    .map_err(|_| known("LATTICE_MANAGED_EXECUTION_ENVIRONMENT_IDENTITY_REJECTED"))?;
    let value: Value = serde_json::from_str(&descriptor)
        .map_err(|_| known("LATTICE_MANAGED_EXECUTION_ENVIRONMENT_IDENTITY_REJECTED"))?;
    let reference = value
        .get("identity_digest")
        .and_then(Value::as_str)
        .ok_or_else(|| known("LATTICE_MANAGED_EXECUTION_ENVIRONMENT_IDENTITY_REJECTED"))?;
    let linux = value
        .get("linux")
        .and_then(Value::as_object)
        .ok_or_else(|| known("LATTICE_MANAGED_EXECUTION_ENVIRONMENT_IDENTITY_REJECTED"))?;
    let codex_home = linux
        .get("codex_home")
        .and_then(Value::as_str)
        .ok_or_else(|| known("LATTICE_MANAGED_EXECUTION_ENVIRONMENT_IDENTITY_REJECTED"))?;
    let config_digest = linux
        .get("config_digest")
        .and_then(Value::as_str)
        .ok_or_else(|| known("LATTICE_MANAGED_EXECUTION_ENVIRONMENT_IDENTITY_REJECTED"))?;
    let task_root = value
        .pointer("/verification_toolchain/task_root")
        .and_then(Value::as_str)
        .ok_or_else(|| known("LATTICE_MANAGED_EXECUTION_ENVIRONMENT_IDENTITY_REJECTED"))?;
    let linux_cwd = linux
        .get("cwd")
        .and_then(Value::as_str)
        .ok_or_else(|| known("LATTICE_MANAGED_EXECUTION_ENVIRONMENT_IDENTITY_REJECTED"))?;
    if reference != packet.execution_environment_ref()
        || !task_root.starts_with("/home/")
        || codex_home != format!("{task_root}/codex-home")
        || !linux_cwd.starts_with(&format!("{task_root}/managed-worktrees/"))
        || !config_digest.starts_with("codex-config:sha256:")
        || config_digest.len() != "codex-config:sha256:".len() + 64
    {
        return Err(known(
            "LATTICE_MANAGED_EXECUTION_ENVIRONMENT_IDENTITY_REJECTED",
        ));
    }
    if receipt.get("schema").and_then(Value::as_str)
        != Some("lattice.wsl2-zero-model-preflight/1.0")
        || receipt.get("status").and_then(Value::as_str) != Some("PASS")
        || receipt
            .get("execution_environment_ref")
            .and_then(Value::as_str)
            != Some(reference)
        || receipt.get("task_ref").and_then(Value::as_str) != Some(packet.task_ref())
        || receipt.get("attempt").and_then(Value::as_u64) != Some(u64::from(packet.attempt()))
        || receipt.get("worktree_ref").and_then(Value::as_str) != Some(packet.worktree_ref())
        || receipt.get("repository_head").and_then(Value::as_str) != Some(packet.base_commit())
        || receipt.get("provider_effect_count").and_then(Value::as_u64) != Some(0)
    {
        return Err(known(
            "LATTICE_MANAGED_EXECUTION_ENVIRONMENT_IDENTITY_REJECTED",
        ));
    }
    let codex_home_digest = receipt
        .get("codex_home_digest")
        .and_then(Value::as_str)
        .filter(|value| value.starts_with("codex-home:sha256:") && value.len() == 83)
        .ok_or_else(|| known("LATTICE_MANAGED_EXECUTION_ENVIRONMENT_IDENTITY_REJECTED"))?;
    Ok(Some(ManagedWslAuthIdentity {
        descriptor_sha256: sha256_hex(descriptor.as_bytes()),
        codex_home_digest: codex_home_digest.to_owned(),
        config_digest: config_digest.to_owned(),
    }))
}

fn execution_watchdog_remaining_at(
    last_activity: Instant,
    hard_deadline: Instant,
    heartbeat_timeout: StdDuration,
    now: Instant,
) -> Option<StdDuration> {
    let silence_deadline = last_activity.checked_add(heartbeat_timeout)?;
    let remaining = silence_deadline
        .min(hard_deadline)
        .checked_duration_since(now)?;
    (!remaining.is_zero()).then_some(remaining)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ManagedWorkerShutdownReceipt {
    task_ref: String,
    attempt: u8,
    thread_id: String,
    turn_id: String,
    terminal: WorkerTerminal,
    terminal_evidence_digest: ContentDigest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ManagedPrestartShutdownDisposition {
    NoBridgeSpawned,
    BridgeSubtreeExited,
}

/// Process-owned proof that graceful cancellation stopped before exact
/// `turn/started`. `BridgeSubtreeExited` is constructed only after the
/// supervised root and its private Job subtree were synchronously reaped.
#[derive(Clone, Debug, Eq, PartialEq)]
struct ManagedWorkerPrestartShutdownReceipt {
    task_ref: String,
    attempt: u8,
    packet_digest: String,
    thread_id: Option<String>,
    turn_id: Option<String>,
    disposition: ManagedPrestartShutdownDisposition,
}

/// Review-bridge shutdown is not worker-attempt shutdown.  This closed value
/// lets the service distinguish a reaped prestart reviewer from an exact
/// reviewer turn that reached an interrupted/failed terminal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ManagedReviewerShutdownDisposition {
    Prestart,
    ExactTerminal,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ManagedReviewerShutdownReceipt {
    task_ref: String,
    attempt: u8,
    subject_digest: ContentDigest,
    thread_id: Option<String>,
    turn_id: Option<String>,
    disposition: ManagedReviewerShutdownDisposition,
    terminal: Option<WorkerTerminal>,
    terminal_evidence_digest: Option<ContentDigest>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ManagedProviderEffectAdmissionError {
    Cancelled,
}

/// Linearization guard shared with graceful cancellation.  While it is held,
/// `request()` cannot win the race that would otherwise sit between a stale
/// cancellation check and a provider-effect control write.
pub(crate) struct ManagedProviderEffectAdmission<'a> {
    _guard: MutexGuard<'a, ()>,
}

/// One counted managed bridge. Dropping it without `record_reaped` leaves the
/// count active deliberately, so ambiguous cleanup can never masquerade as a
/// drained process tree.
pub(crate) struct ManagedBridgeRegistration {
    cancellation: ManagedWorkerCancellation,
    reaped: bool,
}

impl ManagedBridgeRegistration {
    pub(crate) fn record_reaped(&mut self) {
        if !self.reaped {
            self.cancellation.record_bridge_reaped();
            self.reaped = true;
        }
    }
}

#[derive(Default)]
struct ManagedWorkerCancellationInner {
    requested: AtomicBool,
    provider_effect_gate: Mutex<()>,
    wake: Mutex<()>,
    changed: Condvar,
    receipts: Mutex<BTreeMap<(String, u8), ManagedWorkerShutdownReceipt>>,
    prestart_receipts: Mutex<BTreeMap<(String, u8), ManagedWorkerPrestartShutdownReceipt>>,
    reviewer_receipts: Mutex<
        BTreeMap<(String, u8, Option<String>, Option<String>), ManagedReviewerShutdownReceipt>,
    >,
    active_bridges: AtomicUsize,
}

/// Process-owned graceful-shutdown signal shared by the scheduler and only
/// the exact adapters it constructed. It carries no provider identity and
/// therefore cannot itself authorize an interrupt.
#[derive(Clone, Default)]
pub(crate) struct ManagedWorkerCancellation {
    inner: Arc<ManagedWorkerCancellationInner>,
}

impl ManagedWorkerCancellation {
    pub(crate) fn request(&self) {
        let _effect_gate = match self.inner.provider_effect_gate.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        self.inner.requested.store(true, Ordering::Release);
        self.inner.changed.notify_all();
    }

    pub(crate) fn is_requested(&self) -> bool {
        self.inner.requested.load(Ordering::Acquire)
    }

    fn lock_provider_effect_admission(&self) -> MutexGuard<'_, ()> {
        match self.inner.provider_effect_gate.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    pub(crate) fn admit_provider_effect(
        &self,
    ) -> Result<ManagedProviderEffectAdmission<'_>, ManagedProviderEffectAdmissionError> {
        let guard = self.lock_provider_effect_admission();
        if self.is_requested() {
            return Err(ManagedProviderEffectAdmissionError::Cancelled);
        }
        Ok(ManagedProviderEffectAdmission { _guard: guard })
    }

    pub(crate) fn wait_timeout(&self, timeout: StdDuration) {
        if self.is_requested() {
            return;
        }
        let guard = match self.inner.wake.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if !self.is_requested() {
            let _ = self.inner.changed.wait_timeout(guard, timeout);
        }
    }

    fn register_bridge(&self) {
        self.inner.active_bridges.fetch_add(1, Ordering::AcqRel);
    }

    fn record_bridge_reaped(&self) {
        let previous = self.inner.active_bridges.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0, "managed bridge accounting underflow");
        self.inner.changed.notify_all();
    }

    #[must_use]
    pub(crate) fn register_managed_bridge(&self) -> ManagedBridgeRegistration {
        self.register_bridge();
        ManagedBridgeRegistration {
            cancellation: self.clone(),
            reaped: false,
        }
    }

    pub(crate) fn active_bridge_count(&self) -> usize {
        self.inner.active_bridges.load(Ordering::Acquire)
    }

    pub(crate) fn wait_for_no_active_bridges(&self, timeout: StdDuration) -> bool {
        let Some(deadline) = Instant::now().checked_add(timeout) else {
            return false;
        };
        let mut guard = match self.inner.wake.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        loop {
            if self.active_bridge_count() == 0 {
                return true;
            }
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return false;
            };
            let waited = self.inner.changed.wait_timeout(guard, remaining);
            match waited {
                Ok((next, result)) => {
                    guard = next;
                    if result.timed_out() && self.active_bridge_count() != 0 {
                        return false;
                    }
                }
                Err(poisoned) => {
                    let (next, _) = poisoned.into_inner();
                    guard = next;
                }
            }
        }
    }

    fn record_exact_receipt(&self, receipt: ManagedWorkerShutdownReceipt) -> ManagedPortResult<()> {
        let key = (receipt.task_ref.clone(), receipt.attempt);
        let mut receipts = self
            .inner
            .receipts
            .lock()
            .map_err(|_| known("LATTICE_MANAGED_SHUTDOWN_RECEIPT_REJECTED"))?;
        match receipts.get(&key) {
            Some(existing) if existing == &receipt => Ok(()),
            Some(_) => Err(known("LATTICE_MANAGED_SHUTDOWN_RECEIPT_REJECTED")),
            None => {
                receipts.insert(key, receipt);
                Ok(())
            }
        }
    }

    pub(crate) fn has_exact_receipt(&self, task_ref: &str, attempt: u8) -> bool {
        self.inner
            .receipts
            .lock()
            .ok()
            .and_then(|receipts| receipts.get(&(task_ref.to_owned(), attempt)).cloned())
            .is_some()
    }

    fn record_prestart_receipt(
        &self,
        receipt: ManagedWorkerPrestartShutdownReceipt,
    ) -> ManagedPortResult<()> {
        let key = (receipt.task_ref.clone(), receipt.attempt);
        let mut receipts = self
            .inner
            .prestart_receipts
            .lock()
            .map_err(|_| known("LATTICE_MANAGED_PRESTART_SHUTDOWN_RECEIPT_REJECTED"))?;
        match receipts.get(&key) {
            Some(existing) if existing == &receipt => Ok(()),
            Some(_) => Err(known("LATTICE_MANAGED_PRESTART_SHUTDOWN_RECEIPT_REJECTED")),
            None => {
                receipts.insert(key, receipt);
                Ok(())
            }
        }
    }

    pub(crate) fn has_exact_prestart_receipt(&self, task_ref: &str, attempt: u8) -> bool {
        self.inner
            .prestart_receipts
            .lock()
            .ok()
            .and_then(|receipts| receipts.get(&(task_ref.to_owned(), attempt)).cloned())
            .is_some()
    }

    fn record_reviewer_shutdown_receipt(
        &self,
        receipt: ManagedReviewerShutdownReceipt,
    ) -> ManagedPortResult<()> {
        if receipt.task_ref.is_empty()
            || receipt.task_ref.len() > 256
            || receipt.attempt == 0
            || receipt
                .thread_id
                .as_deref()
                .is_some_and(|value| value.is_empty() || value.len() > 256 || !value.is_ascii())
            || receipt
                .turn_id
                .as_deref()
                .is_some_and(|value| value.is_empty() || value.len() > 256 || !value.is_ascii())
            || (receipt.turn_id.is_some() && receipt.thread_id.is_none())
            || match receipt.disposition {
                ManagedReviewerShutdownDisposition::Prestart => {
                    receipt.terminal.is_some() || receipt.terminal_evidence_digest.is_some()
                }
                ManagedReviewerShutdownDisposition::ExactTerminal => {
                    receipt.thread_id.is_none()
                        || receipt.turn_id.is_none()
                        || !matches!(
                            receipt.terminal,
                            Some(WorkerTerminal::Interrupted | WorkerTerminal::Failed)
                        )
                        || receipt.terminal_evidence_digest.is_none()
                }
            }
        {
            return Err(known("LATTICE_MANAGED_REVIEWER_SHUTDOWN_RECEIPT_REJECTED"));
        }
        let key = (
            receipt.task_ref.clone(),
            receipt.attempt,
            receipt.thread_id.clone(),
            receipt.turn_id.clone(),
        );
        let mut receipts = self
            .inner
            .reviewer_receipts
            .lock()
            .map_err(|_| known("LATTICE_MANAGED_REVIEWER_SHUTDOWN_RECEIPT_REJECTED"))?;
        if receipts
            .iter()
            .any(|((task_ref, attempt, _, _), existing)| {
                task_ref == &receipt.task_ref && *attempt == receipt.attempt && existing != &receipt
            })
        {
            return Err(known("LATTICE_MANAGED_REVIEWER_SHUTDOWN_RECEIPT_REJECTED"));
        }
        match receipts.get(&key) {
            Some(existing) if existing == &receipt => Ok(()),
            Some(_) => Err(known("LATTICE_MANAGED_REVIEWER_SHUTDOWN_RECEIPT_REJECTED")),
            None => {
                receipts.insert(key, receipt);
                Ok(())
            }
        }
    }

    pub(crate) fn record_reviewer_prestart_receipt(
        &self,
        task_ref: &str,
        attempt: u8,
        subject_digest: ContentDigest,
        thread_id: Option<&str>,
        turn_id: Option<&str>,
    ) -> ManagedPortResult<()> {
        self.record_reviewer_shutdown_receipt(ManagedReviewerShutdownReceipt {
            task_ref: task_ref.to_owned(),
            attempt,
            subject_digest,
            thread_id: thread_id.map(str::to_owned),
            turn_id: turn_id.map(str::to_owned),
            disposition: ManagedReviewerShutdownDisposition::Prestart,
            terminal: None,
            terminal_evidence_digest: None,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record_reviewer_terminal_receipt(
        &self,
        task_ref: &str,
        attempt: u8,
        subject_digest: ContentDigest,
        thread_id: &str,
        turn_id: &str,
        terminal: WorkerTerminal,
        terminal_evidence_digest: ContentDigest,
    ) -> ManagedPortResult<()> {
        self.record_reviewer_shutdown_receipt(ManagedReviewerShutdownReceipt {
            task_ref: task_ref.to_owned(),
            attempt,
            subject_digest,
            thread_id: Some(thread_id.to_owned()),
            turn_id: Some(turn_id.to_owned()),
            disposition: ManagedReviewerShutdownDisposition::ExactTerminal,
            terminal: Some(terminal),
            terminal_evidence_digest: Some(terminal_evidence_digest),
        })
    }

    pub(crate) fn reviewer_shutdown_disposition(
        &self,
        task_ref: &str,
        attempt: u8,
    ) -> Option<ManagedReviewerShutdownDisposition> {
        let receipts = self.inner.reviewer_receipts.lock().ok()?;
        let mut matching = receipts
            .iter()
            .filter(|((candidate, candidate_attempt, _, _), _)| {
                candidate == task_ref && *candidate_attempt == attempt
            })
            .map(|(_, receipt)| receipt.disposition);
        let disposition = matching.next()?;
        if matching.next().is_some() {
            return None;
        }
        Some(disposition)
    }
}

#[derive(Debug, Eq, PartialEq)]
enum CancellableBridgeRecord {
    Record(Value),
    Cancelled,
}

#[derive(Debug, Eq, PartialEq)]
enum BoundedProbeRecord {
    Record(Value),
    Cancelled,
    DeadlineElapsed,
}

fn receive_record_or_cancellation(
    receiver: &Receiver<ManagedPortResult<Value>>,
    cancellation: &ManagedWorkerCancellation,
    poll: StdDuration,
) -> ManagedPortResult<CancellableBridgeRecord> {
    loop {
        match receiver.try_recv() {
            Ok(record) => return record.map(CancellableBridgeRecord::Record),
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                return Err(ambiguous("LATTICE_MANAGED_PROCESS_EXIT_WITHOUT_TERMINAL"));
            }
        }
        if cancellation.is_requested() {
            return Ok(CancellableBridgeRecord::Cancelled);
        }
        match receiver.recv_timeout(poll) {
            Ok(record) => return record.map(CancellableBridgeRecord::Record),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Err(ambiguous("LATTICE_MANAGED_PROCESS_EXIT_WITHOUT_TERMINAL"));
            }
        }
    }
}

fn receive_probe_record_until(
    receiver: &Receiver<ManagedPortResult<Value>>,
    cancellation: &ManagedWorkerCancellation,
    poll: StdDuration,
    deadline: Instant,
) -> ManagedPortResult<BoundedProbeRecord> {
    loop {
        if cancellation.is_requested() {
            return Ok(BoundedProbeRecord::Cancelled);
        }
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return Ok(BoundedProbeRecord::DeadlineElapsed);
        };
        match receiver.try_recv() {
            Ok(record) => return record.map(BoundedProbeRecord::Record),
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                return Err(ambiguous("LATTICE_MANAGED_PROCESS_EXIT_WITHOUT_TERMINAL"));
            }
        }
        match receiver.recv_timeout(poll.min(remaining)) {
            Ok(record) => {
                if Instant::now() >= deadline {
                    return Ok(BoundedProbeRecord::DeadlineElapsed);
                }
                return record.map(BoundedProbeRecord::Record);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Err(ambiguous("LATTICE_MANAGED_PROCESS_EXIT_WITHOUT_TERMINAL"));
            }
        }
    }
}

fn probe_watchdog_budget_at(
    packet_deadline: OffsetDateTime,
    heartbeat_timeout: StdDuration,
    now: OffsetDateTime,
) -> Option<StdDuration> {
    let remaining_ms = (packet_deadline - now).whole_milliseconds();
    let remaining_ms = u64::try_from(remaining_ms).ok()?;
    let packet_budget = StdDuration::from_millis(remaining_ms);
    let budget = packet_budget.min(heartbeat_timeout);
    (!budget.is_zero()).then_some(budget)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ManagedBridgeProcessExitReceipt {
    terminal: WorkerTerminal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TurnStartLifecycle {
    NotAuthorized,
    AuthorizationSent,
    Accepted,
    ExactStarted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CancellationDisposition {
    Prestart,
    AmbiguousAuthorization,
    DrainExactIdentity,
}

struct ActiveBridge {
    child: SupervisedDuplexChild,
    stdin: Option<Box<dyn Write + Send>>,
    records: Option<Receiver<ManagedPortResult<Value>>>,
    reader: Option<thread::JoinHandle<()>>,
    operation: String,
    task_ref: String,
    attempt: u8,
    packet_digest: String,
    thread_id: Option<String>,
    turn_id: Option<String>,
    app_server_generation: Option<u64>,
    app_server_identity_digest: Option<ContentDigest>,
    reconciliation_digest: Option<ContentDigest>,
    provider_open_evidence: Option<VerifiedManagedEvidence>,
    provider_readiness_requested: bool,
    provider_readiness_verified: bool,
    provider_dispatch_authorized: bool,
    turn_start_lifecycle: TurnStartLifecycle,
    exact_active: bool,
    cancellation: ManagedWorkerCancellation,
    bridge_registered: bool,
    effect_identity: ManagedWorkerEffectIdentity,
    // Held until exact terminal/reap (including interrupt/reconnect paths), so
    // Node cannot load an ABA-substituted entry/import after pre-spawn replay.
    _effect_seal: Option<ManagedFileSeal>,
}

#[derive(Clone)]
struct ManagedWorkerEffectIdentity {
    node: Option<ManagedFileIdentity>,
    codex_file: Option<ManagedFileIdentity>,
    codex: Option<ManagedCodexSpawnIdentity>,
    codex_home_guard: Option<ManagedEffectBundleGuard>,
    codex_home: PathBuf,
    worktree: PathBuf,
    bridge_bundle: Option<ManagedFileIdentityBundle>,
    external_bundle: Option<ManagedEffectBundleGuard>,
    runtime_bundle: Option<ManagedEffectBundleGuard>,
    execution_environment_descriptor_sha256: Option<String>,
    execution_environment_descriptor: Option<String>,
    execution_preflight_receipt: Option<String>,
    execution_preflight_descriptor_digest: Option<String>,
    execution_preflight_content_digest: Option<String>,
    execution_preflight_evidence: Option<VerifiedManagedEvidence>,
    auth_codex_home_digest: String,
    auth_config_digest: String,
}

impl ManagedWorkerEffectIdentity {
    fn verify(&self) -> ManagedPortResult<()> {
        if let Some(expected) = &self.execution_environment_descriptor_sha256 {
            let descriptor = self
                .execution_environment_descriptor
                .as_ref()
                .ok_or_else(|| known("LATTICE_MANAGED_EXECUTION_ENVIRONMENT_IDENTITY_REJECTED"))?;
            if sha256_hex(descriptor.as_bytes()) != *expected {
                return Err(known(
                    "LATTICE_MANAGED_EXECUTION_ENVIRONMENT_IDENTITY_REJECTED",
                ));
            }
        }
        if let Some(codex) = &self.codex {
            if let Some(bundle) = &self.external_bundle {
                bundle
                    .covers_exact_file(codex.launcher(), codex.launcher_sha256())
                    .map_err(|_| known("LATTICE_MANAGED_EXTERNAL_BUNDLE_IDENTITY_REJECTED"))?;
                codex
                    .verify_context(&self.codex_home, &self.worktree)
                    .map_err(|_| known("LATTICE_MANAGED_CODEX_IDENTITY_REJECTED"))?;
            } else {
                codex
                    .verify(&self.codex_home, &self.worktree)
                    .map_err(|_| known("LATTICE_MANAGED_CODEX_IDENTITY_REJECTED"))?;
            }
        } else if self.execution_environment_descriptor.is_none()
            || self.execution_preflight_receipt.is_none()
        {
            return Err(known(
                "LATTICE_MANAGED_EXECUTION_ENVIRONMENT_IDENTITY_REJECTED",
            ));
        }
        if let Some(codex_home_guard) = &self.codex_home_guard {
            codex_home_guard
                .verify()
                .map_err(|_| known("LATTICE_MANAGED_CODEX_HOME_SEAL_REJECTED"))?;
            codex_home_guard
                .covers_file(&self.codex_home.join("config.toml"))
                .map_err(|_| known("LATTICE_MANAGED_CODEX_HOME_SEAL_REJECTED"))?;
        }
        if let Some(bundle) = &self.runtime_bundle {
            bundle
                .verify()
                .map_err(|_| known("LATTICE_MANAGED_RUNTIME_BUNDLE_IDENTITY_REJECTED"))?;
        } else {
            self.node
                .as_ref()
                .ok_or_else(|| known("LATTICE_MANAGED_NODE_IDENTITY_REJECTED"))?
                .verify()
                .map_err(|_| known("LATTICE_MANAGED_NODE_IDENTITY_REJECTED"))?;
            self.bridge_bundle
                .as_ref()
                .ok_or_else(|| known("LATTICE_MANAGED_WORKER_BRIDGE_IDENTITY_REJECTED"))?
                .verify()
                .map_err(|_| known("LATTICE_MANAGED_WORKER_BRIDGE_IDENTITY_REJECTED"))?;
        }
        if let Some(codex_file) = &self.codex_file {
            codex_file
                .verify()
                .map_err(|_| known("LATTICE_MANAGED_CODEX_IDENTITY_REJECTED"))?;
        }
        Ok(())
    }

    fn auth_codex_home_digest(&self) -> &str {
        &self.auth_codex_home_digest
    }

    fn auth_config_digest(&self) -> &str {
        &self.auth_config_digest
    }

    fn seal(&self) -> ManagedPortResult<Option<ManagedFileSeal>> {
        if self.runtime_bundle.is_some() {
            self.verify()?;
            return Ok(None);
        }
        let mut seal = self
            .node
            .as_ref()
            .ok_or_else(|| known("LATTICE_MANAGED_NODE_IDENTITY_REJECTED"))?
            .seal()
            .map_err(|_| known("LATTICE_MANAGED_NODE_IDENTITY_REJECTED"))?;
        if let Some(codex_file) = &self.codex_file {
            seal.extend(
                codex_file
                    .seal()
                    .map_err(|_| known("LATTICE_MANAGED_CODEX_IDENTITY_REJECTED"))?,
            );
        }
        seal.extend(
            self.bridge_bundle
                .as_ref()
                .ok_or_else(|| known("LATTICE_MANAGED_WORKER_BRIDGE_IDENTITY_REJECTED"))?
                .seal()
                .map_err(|_| known("LATTICE_MANAGED_WORKER_BRIDGE_IDENTITY_REJECTED"))?,
        );
        self.verify()?;
        Ok(Some(seal))
    }
}

impl ActiveBridge {
    fn teardown_control(&self) -> Option<Value> {
        exact_teardown_control(
            self.exact_active,
            &self.task_ref,
            self.attempt,
            &self.packet_digest,
            self.thread_id.as_deref(),
            self.turn_id.as_deref(),
        )
    }

    fn wait_for_root(&mut self, timeout: StdDuration) -> std::io::Result<Option<ExitStatus>> {
        let Some(deadline) = Instant::now().checked_add(timeout) else {
            return Ok(None);
        };
        loop {
            if let Some(status) = self.child.try_wait()? {
                return Ok(Some(status));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            thread::sleep(StdDuration::from_millis(10));
        }
    }

    fn next_record_timeout(&self, timeout: StdDuration) -> ManagedPortResult<Option<Value>> {
        match self
            .records
            .as_ref()
            .ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_READER_REJECTED"))?
            .recv_timeout(timeout)
        {
            Ok(record) => record.map(Some),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => {
                Err(ambiguous("LATTICE_MANAGED_PROCESS_EXIT_WITHOUT_TERMINAL"))
            }
        }
    }

    fn join_reader(&mut self) -> ManagedPortResult<()> {
        self.reader
            .take()
            .ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_READER_REJECTED"))?
            .join()
            .map_err(|_| ambiguous("LATTICE_MANAGED_BRIDGE_READER_REJECTED"))
    }

    fn prove_subtree_empty_and_join_reader(
        &mut self,
        error_code: &'static str,
    ) -> ManagedPortResult<()> {
        self.child
            .terminate_and_reap()
            .map_err(|_| ambiguous(error_code))?;
        self.record_subtree_reaped();
        // A bounded sender can be waiting even after the process subtree has
        // exited. Drop the receiver before joining so cleanup cannot deadlock
        // on a full evidence queue.
        self.records.take();
        self.join_reader()
    }

    fn record_subtree_reaped(&mut self) {
        if self.bridge_registered {
            self.cancellation.record_bridge_reaped();
            self.bridge_registered = false;
        }
    }

    fn cancellation_disposition(
        &self,
        retained_exact_reconciliation: bool,
    ) -> CancellationDisposition {
        if retained_exact_reconciliation
            || self.exact_active
            || matches!(
                self.turn_start_lifecycle,
                TurnStartLifecycle::Accepted | TurnStartLifecycle::ExactStarted
            )
        {
            CancellationDisposition::DrainExactIdentity
        } else if self.turn_start_lifecycle == TurnStartLifecycle::AuthorizationSent {
            CancellationDisposition::AmbiguousAuthorization
        } else {
            CancellationDisposition::Prestart
        }
    }

    fn prestart_shutdown_receipt(&self) -> ManagedPortResult<ManagedWorkerPrestartShutdownReceipt> {
        if self.turn_start_lifecycle != TurnStartLifecycle::NotAuthorized || self.exact_active {
            return Err(ambiguous("LATTICE_MANAGED_TURN_START_SHUTDOWN_AMBIGUOUS"));
        }
        Ok(ManagedWorkerPrestartShutdownReceipt {
            task_ref: self.task_ref.clone(),
            attempt: self.attempt,
            packet_digest: self.packet_digest.clone(),
            thread_id: self.thread_id.clone(),
            turn_id: self.turn_id.clone(),
            disposition: ManagedPrestartShutdownDisposition::BridgeSubtreeExited,
        })
    }

    fn terminate_prestart_and_reap(
        &mut self,
    ) -> ManagedPortResult<ManagedWorkerPrestartShutdownReceipt> {
        if self.cancellation_disposition(false) != CancellationDisposition::Prestart {
            return Err(known(MANAGED_GRACEFUL_SHUTDOWN_RECEIPT_REQUIRED));
        }
        self.stdin.take();
        self.prove_subtree_empty_and_join_reader(MANAGED_GRACEFUL_SHUTDOWN_RECEIPT_REQUIRED)?;
        self.prestart_shutdown_receipt()
    }
}

fn exact_teardown_control(
    exact_active: bool,
    task_ref: &str,
    attempt: u8,
    packet_digest: &str,
    thread_id: Option<&str>,
    turn_id: Option<&str>,
) -> Option<Value> {
    if !exact_active || task_ref.is_empty() || attempt == 0 || packet_digest.is_empty() {
        return None;
    }
    Some(json!({
        "schema": CONTROL_SCHEMA,
        "operation": "interrupt",
        "task_ref": task_ref,
        "attempt": attempt,
        "packet_digest": packet_digest,
        "thread_id": thread_id?,
        "turn_id": turn_id?,
    }))
}

fn exact_turn_start_authorization_control(
    task_ref: &str,
    attempt: u8,
    packet_digest: &str,
    thread_id: &str,
) -> ManagedPortResult<Value> {
    if task_ref.is_empty()
        || attempt == 0
        || packet_digest.is_empty()
        || thread_id.is_empty()
        || thread_id.len() > 256
    {
        return Err(known("LATTICE_MANAGED_TURN_START_AUTHORIZATION_REJECTED"));
    }
    Ok(json!({
        "schema": CONTROL_SCHEMA,
        "operation": "authorize_turn_start",
        "task_ref": task_ref,
        "attempt": attempt,
        "packet_digest": packet_digest,
        "thread_id": thread_id,
    }))
}

fn exact_provider_dispatch_authorization_control(
    task_ref: &str,
    attempt: u8,
    packet_digest: &str,
    marker_digest: &str,
) -> ManagedPortResult<Value> {
    if task_ref.is_empty()
        || attempt == 0
        || packet_digest.is_empty()
        || !typed_sha256(marker_digest, "provider-subtree-marker")
    {
        return Err(known(
            "LATTICE_MANAGED_PROVIDER_DISPATCH_AUTHORIZATION_REJECTED",
        ));
    }
    Ok(json!({
        "schema": CONTROL_SCHEMA,
        "operation": "authorize_provider_dispatch",
        "task_ref": task_ref,
        "attempt": attempt,
        "packet_digest": packet_digest,
        "marker_digest": marker_digest,
    }))
}

fn exact_provider_readiness_control(
    task_ref: &str,
    attempt: u8,
    packet_digest: &str,
    marker_digest: &str,
) -> ManagedPortResult<Value> {
    if task_ref.is_empty()
        || attempt == 0
        || !typed_sha256(packet_digest, "attempt-packet")
        || !typed_sha256(marker_digest, "provider-subtree-marker")
    {
        return Err(known("LATTICE_MANAGED_PROVIDER_READINESS_REJECTED"));
    }
    Ok(json!({
        "schema": CONTROL_SCHEMA,
        "operation": "probe_provider_readiness",
        "task_ref": task_ref,
        "attempt": attempt,
        "packet_digest": packet_digest,
        "marker_digest": marker_digest,
    }))
}

fn reconcile_app_server_generation(
    current: Option<u64>,
    observed: u64,
    event_type: &str,
    exact_active: bool,
) -> ManagedPortResult<u64> {
    if observed == 0 {
        return Err(known("LATTICE_MANAGED_BRIDGE_GENERATION_REJECTED"));
    }
    match current {
        None => Ok(observed),
        Some(current) if current == observed => Ok(observed),
        Some(current)
            if exact_active && event_type == "RECONCILE_STARTED" && observed > current =>
        {
            Ok(observed)
        }
        Some(_) => Err(known("LATTICE_MANAGED_BRIDGE_GENERATION_REJECTED")),
    }
}

impl Drop for ActiveBridge {
    fn drop(&mut self) {
        if self.effect_identity.verify().is_ok()
            && let (Some(control), Some(stdin)) = (self.teardown_control(), self.stdin.as_mut())
        {
            let _ = serde_json::to_writer(&mut **stdin, &control)
                .and_then(|()| stdin.write_all(b"\n").map_err(serde_json::Error::io))
                .and_then(|()| stdin.flush().map_err(serde_json::Error::io));
            self.stdin.take();
            let _ = self.wait_for_root(BRIDGE_TEARDOWN_GRACE);
        } else {
            self.stdin.take();
        }
        // The supervised child owns a kill-on-close Job on Windows. This call
        // proves the complete Node -> App Server -> worker subtree is empty;
        // its own Drop repeats the same bounded cleanup if this call fails.
        if self.child.terminate_and_reap().is_ok() {
            self.record_subtree_reaped();
        }
        // Failure paths already return a concrete error to the caller. Drop
        // only prevents a bounded reader from outliving that failed bridge.
        self.records.take();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

/// One-attempt adapter. The path, prompt and heartbeat are process-owned
/// transient values and are deliberately absent from durable packet identity.
pub(crate) struct ManagedCodexWorkerAdapter {
    node_executable: PathBuf,
    codex_home: PathBuf,
    bridge_path: PathBuf,
    effect_identity: ManagedWorkerEffectIdentity,
    worktree: PathBuf,
    prompt: String,
    heartbeat_timeout_ms: u64,
    packet: AttemptPacketIdentity,
    preclaim_auth_readiness: Option<ManagedCodexAuthReadiness>,
    retained_thread_id: Option<String>,
    retained_turn_id: Option<String>,
    retained_attempt_started_at: Option<String>,
    retained_attempt_deadline_at: Option<String>,
    retained_last_heartbeat_at: Option<String>,
    retained_last_meaningful_progress_at: Option<String>,
    pending_terminal_reconciliation: Option<ManagedTerminalCandidate>,
    pending_terminal_candidate: Option<ManagedTerminalCandidate>,
    resource_evidence_identity: Option<(ProjectId, ContentDigest)>,
    closed_blocker_code: Option<&'static str>,
    cancellation: ManagedWorkerCancellation,
    shutdown_interrupt_sent: bool,
    shutdown_deadline: Option<Instant>,
    execution_last_activity: Option<Instant>,
    execution_hard_deadline: Option<Instant>,
    active: Option<ActiveBridge>,
}

impl ManagedCodexWorkerAdapter {
    pub(crate) fn new(
        node_executable: PathBuf,
        codex_executable: PathBuf,
        codex_home: PathBuf,
        bridge_path: PathBuf,
        worktree: PathBuf,
        prompt: String,
        heartbeat_timeout_ms: u64,
        packet: AttemptPacketIdentity,
    ) -> ManagedPortResult<Self> {
        Self::new_inner(
            node_executable,
            codex_executable,
            codex_home,
            bridge_path,
            worktree,
            prompt,
            heartbeat_timeout_ms,
            packet,
            None,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_with_effect_bundle_guard(
        node_executable: PathBuf,
        codex_identity: ManagedCodexSpawnIdentity,
        codex_home: PathBuf,
        bridge_path: PathBuf,
        worktree: PathBuf,
        prompt: String,
        heartbeat_timeout_ms: u64,
        packet: AttemptPacketIdentity,
        guard: ManagedEffectBundleGuard,
        runtime_guard: ManagedEffectBundleGuard,
    ) -> ManagedPortResult<Self> {
        Self::new_inner(
            node_executable,
            codex_identity.launcher().to_path_buf(),
            codex_home,
            bridge_path,
            worktree,
            prompt,
            heartbeat_timeout_ms,
            packet,
            Some((codex_identity, guard, runtime_guard)),
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_wsl_with_effect_bundle_guard(
        node_executable: PathBuf,
        codex_identity: ManagedCodexSpawnIdentity,
        codex_home: PathBuf,
        bridge_path: PathBuf,
        worktree: PathBuf,
        prompt: String,
        heartbeat_timeout_ms: u64,
        packet: AttemptPacketIdentity,
        guard: ManagedEffectBundleGuard,
        runtime_guard: ManagedEffectBundleGuard,
        execution_environment: &ExecutionEnvironmentDescriptor,
        execution_preflight: &VerifiedManagedEvidence,
    ) -> ManagedPortResult<Self> {
        let preflight = std::str::from_utf8(execution_preflight.bytes())
            .map_err(|_| known("LATTICE_MANAGED_EXECUTION_ENVIRONMENT_IDENTITY_REJECTED"))?
            .to_owned();
        Self::new_inner(
            node_executable,
            codex_identity.launcher().to_path_buf(),
            codex_home,
            bridge_path,
            worktree,
            prompt,
            heartbeat_timeout_ms,
            packet,
            Some((codex_identity, guard, runtime_guard)),
            Some((
                execution_environment.as_json().to_owned(),
                preflight,
                execution_preflight.descriptor_digest().as_str().to_owned(),
                execution_preflight.content_digest().as_str().to_owned(),
                execution_preflight.clone(),
            )),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn new_inner(
        node_executable: PathBuf,
        codex_executable: PathBuf,
        codex_home: PathBuf,
        bridge_path: PathBuf,
        worktree: PathBuf,
        prompt: String,
        heartbeat_timeout_ms: u64,
        packet: AttemptPacketIdentity,
        sealed_codex: Option<(
            ManagedCodexSpawnIdentity,
            ManagedEffectBundleGuard,
            ManagedEffectBundleGuard,
        )>,
        wsl_execution: Option<(String, String, String, String, VerifiedManagedEvidence)>,
    ) -> ManagedPortResult<Self> {
        if !node_executable.is_absolute()
            || !codex_executable.is_absolute()
            || !codex_home.is_absolute()
            || !bridge_path.is_absolute()
            || !worktree.is_absolute()
            || prompt.is_empty()
            || prompt.len() > 16_384
            || heartbeat_timeout_ms == 0
            || heartbeat_timeout_ms > 86_400_000
        {
            return Err(known("LATTICE_MANAGED_WORKER_CONFIGURATION_REJECTED"));
        }
        let (execution_environment_descriptor, execution_preflight_receipt) = wsl_execution
            .as_ref()
            .map(|(descriptor, receipt, _, _, _)| {
                (Some(descriptor.as_str()), Some(receipt.as_str()))
            })
            .unwrap_or((None, None));
        let wsl_auth_identity = capture_wsl_auth_identity(
            &packet,
            execution_environment_descriptor,
            execution_preflight_receipt,
        )?;
        let (codex_identity, external_bundle, runtime_bundle) = match sealed_codex {
            Some((identity, guard, runtime_guard)) => (
                wsl_auth_identity.is_none().then_some(identity),
                Some(guard),
                Some(runtime_guard),
            ),
            None => (
                Some(
                    ManagedCodexSpawnIdentity::capture(
                        codex_executable.clone(),
                        &codex_home,
                        &worktree,
                    )
                    .map_err(|_| known("LATTICE_MANAGED_CODEX_IDENTITY_REJECTED"))?,
                ),
                None,
                None,
            ),
        };
        let codex_file = match (&external_bundle, &codex_identity) {
            (_, None) => None,
            (Some(guard), Some(codex_identity)) => {
                guard
                    .covers_exact_file(codex_identity.launcher(), codex_identity.launcher_sha256())
                    .map_err(|_| known("LATTICE_MANAGED_EXTERNAL_BUNDLE_IDENTITY_REJECTED"))?;
                None
            }
            (None, Some(codex_identity)) => Some(
                ManagedFileIdentity::capture(codex_identity.launcher(), MAX_MANAGED_CODEX_BYTES)
                    .map_err(|_| known("LATTICE_MANAGED_CODEX_IDENTITY_REJECTED"))?,
            ),
        };
        let bridge_parent = bridge_path
            .parent()
            .ok_or_else(|| known("LATTICE_MANAGED_WORKER_BRIDGE_IDENTITY_REJECTED"))?;
        let worker_dependency = bridge_parent.join("managed-codex-worker.mjs");
        let app_server_dependency = bridge_parent.join("codex-app-server.mjs");
        let execution_domain_dependency = bridge_parent.join("wsl2-execution-domain.mjs");
        let execution_preflight_dependency = bridge_parent.join("wsl2-execution-preflight.mjs");
        let wsl_supervisor_dependency = bridge_parent.join("wsl2-codex-supervisor.mjs");
        let wsl_proc_identity_dependency = bridge_parent.join("wsl2-proc-identity.mjs");
        let provider_subtree_dependency = bridge_parent.join("wsl2-provider-subtree-reconcile.mjs");
        if let Some(guard) = &runtime_bundle {
            for path in [
                node_executable.as_path(),
                bridge_path.as_path(),
                worker_dependency.as_path(),
                app_server_dependency.as_path(),
                provider_subtree_dependency.as_path(),
            ] {
                guard
                    .covers_file(path)
                    .map_err(|_| known("LATTICE_MANAGED_RUNTIME_BUNDLE_IDENTITY_REJECTED"))?;
            }
        }
        let node_identity = runtime_bundle
            .is_none()
            .then(|| {
                ManagedFileIdentity::capture(&node_executable, MAX_MANAGED_NODE_BYTES)
                    .map_err(|_| known("LATTICE_MANAGED_NODE_IDENTITY_REJECTED"))
            })
            .transpose()?;
        let bridge_bundle = runtime_bundle
            .is_none()
            .then(|| {
                ManagedFileIdentityBundle::capture([
                    (bridge_path.clone(), MAX_MANAGED_WORKER_BRIDGE_BYTES),
                    (worker_dependency, MAX_MANAGED_WORKER_DEPENDENCY_BYTES),
                    (app_server_dependency, MAX_MANAGED_WORKER_DEPENDENCY_BYTES),
                    (
                        execution_domain_dependency,
                        MAX_MANAGED_WORKER_DEPENDENCY_BYTES,
                    ),
                    (
                        execution_preflight_dependency,
                        MAX_MANAGED_WORKER_DEPENDENCY_BYTES,
                    ),
                    (
                        wsl_supervisor_dependency,
                        MAX_MANAGED_WORKER_DEPENDENCY_BYTES,
                    ),
                    (
                        wsl_proc_identity_dependency,
                        MAX_MANAGED_WORKER_DEPENDENCY_BYTES,
                    ),
                    (
                        provider_subtree_dependency,
                        MAX_MANAGED_WORKER_DEPENDENCY_BYTES,
                    ),
                ])
                .map_err(|_| known("LATTICE_MANAGED_WORKER_BRIDGE_IDENTITY_REJECTED"))
            })
            .transpose()?;
        let auth_codex_home_digest = wsl_auth_identity.as_ref().map_or_else(
            || {
                codex_identity
                    .as_ref()
                    .expect("native codex identity")
                    .codex_home_digest()
                    .to_owned()
            },
            |identity| identity.codex_home_digest.clone(),
        );
        let auth_config_digest = wsl_auth_identity.as_ref().map_or_else(
            || {
                codex_identity
                    .as_ref()
                    .expect("native codex identity")
                    .config_digest()
                    .to_owned()
            },
            |identity| identity.config_digest.clone(),
        );
        let effect_identity = ManagedWorkerEffectIdentity {
            node: node_identity,
            codex_file,
            codex: codex_identity,
            codex_home_guard: wsl_auth_identity
                .is_none()
                .then(|| {
                    capture_managed_codex_home_guard(&codex_home)
                        .map_err(|_| known("LATTICE_MANAGED_CODEX_HOME_SEAL_REJECTED"))
                })
                .transpose()?,
            codex_home: codex_home.clone(),
            worktree: worktree.clone(),
            bridge_bundle,
            external_bundle,
            runtime_bundle,
            execution_environment_descriptor_sha256: wsl_auth_identity
                .as_ref()
                .map(|identity| identity.descriptor_sha256.clone()),
            execution_environment_descriptor: wsl_execution
                .as_ref()
                .map(|(descriptor, _, _, _, _)| descriptor.clone()),
            execution_preflight_receipt: wsl_execution
                .as_ref()
                .map(|(_, receipt, _, _, _)| receipt.clone()),
            execution_preflight_descriptor_digest: wsl_execution
                .as_ref()
                .map(|(_, _, digest, _, _)| digest.clone()),
            execution_preflight_content_digest: wsl_execution
                .as_ref()
                .map(|(_, _, _, digest, _)| digest.clone()),
            execution_preflight_evidence: wsl_execution
                .as_ref()
                .map(|(_, _, _, _, evidence)| evidence.clone()),
            auth_codex_home_digest,
            auth_config_digest,
        };
        Ok(Self {
            node_executable,
            codex_home,
            bridge_path,
            effect_identity,
            worktree,
            prompt,
            heartbeat_timeout_ms,
            packet,
            preclaim_auth_readiness: None,
            retained_thread_id: None,
            retained_turn_id: None,
            retained_attempt_started_at: None,
            retained_attempt_deadline_at: None,
            retained_last_heartbeat_at: None,
            retained_last_meaningful_progress_at: None,
            pending_terminal_reconciliation: None,
            pending_terminal_candidate: None,
            resource_evidence_identity: None,
            closed_blocker_code: None,
            cancellation: ManagedWorkerCancellation::default(),
            shutdown_interrupt_sent: false,
            shutdown_deadline: None,
            execution_last_activity: None,
            execution_hard_deadline: None,
            active: None,
        })
    }

    pub(crate) fn with_cancellation(mut self, cancellation: ManagedWorkerCancellation) -> Self {
        self.cancellation = cancellation;
        self
    }

    pub(crate) fn with_resource_evidence_identity(
        mut self,
        project_id: ProjectId,
        producer_digest: ContentDigest,
    ) -> Self {
        self.resource_evidence_identity = Some((project_id, producer_digest));
        self
    }

    pub(crate) fn with_retained_turn_id(
        mut self,
        turn_id: impl Into<String>,
    ) -> ManagedPortResult<Self> {
        let turn_id = turn_id.into();
        if turn_id.is_empty()
            || turn_id.len() > 256
            || !turn_id.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-')
            })
        {
            return Err(known("LATTICE_MANAGED_RETAINED_TURN_REJECTED"));
        }
        self.retained_turn_id = Some(turn_id);
        Ok(self)
    }

    pub(crate) fn with_retained_last_meaningful_progress_at(
        mut self,
        observed_at: impl Into<String>,
    ) -> ManagedPortResult<Self> {
        let observed_at = observed_at.into();
        let parsed = OffsetDateTime::parse(&observed_at, &Rfc3339)
            .map_err(|_| known("LATTICE_MANAGED_RETAINED_PROGRESS_REJECTED"))?;
        let canonical = parsed
            .format(&Rfc3339)
            .map_err(|_| known("LATTICE_MANAGED_RETAINED_PROGRESS_REJECTED"))?;
        if let Some(started_at) = self.retained_attempt_started_at.as_deref() {
            let started = OffsetDateTime::parse(started_at, &Rfc3339)
                .map_err(|_| known("LATTICE_MANAGED_RETAINED_EXACT_START_REQUIRED"))?;
            if parsed < started {
                return Err(known("LATTICE_MANAGED_RETAINED_PROGRESS_REJECTED"));
            }
        }
        self.retained_last_meaningful_progress_at = Some(canonical);
        Ok(self)
    }

    pub(crate) fn with_retained_last_heartbeat_at(
        mut self,
        observed_at: impl Into<String>,
    ) -> ManagedPortResult<Self> {
        let observed_at = observed_at.into();
        let parsed = OffsetDateTime::parse(&observed_at, &Rfc3339)
            .map_err(|_| known("LATTICE_MANAGED_RETAINED_HEARTBEAT_REJECTED"))?;
        let canonical = parsed
            .format(&Rfc3339)
            .map_err(|_| known("LATTICE_MANAGED_RETAINED_HEARTBEAT_REJECTED"))?;
        if let Some(started_at) = self.retained_attempt_started_at.as_deref() {
            let started = OffsetDateTime::parse(started_at, &Rfc3339)
                .map_err(|_| known("LATTICE_MANAGED_RETAINED_EXACT_START_REQUIRED"))?;
            if parsed < started {
                return Err(known("LATTICE_MANAGED_RETAINED_HEARTBEAT_REJECTED"));
            }
        }
        self.retained_last_heartbeat_at = Some(canonical);
        Ok(self)
    }

    pub(crate) fn with_retained_execution_window(
        mut self,
        started_at: impl Into<String>,
        deadline_at: impl Into<String>,
    ) -> ManagedPortResult<Self> {
        let started_at = started_at.into();
        let deadline_at = deadline_at.into();
        validate_execution_window(
            &started_at,
            &deadline_at,
            self.packet.max_duration_seconds(),
            self.packet.deadline_at(),
        )?;
        if let Some(progress_at) = self.retained_last_meaningful_progress_at.as_deref() {
            let progress = OffsetDateTime::parse(progress_at, &Rfc3339)
                .map_err(|_| known("LATTICE_MANAGED_RETAINED_PROGRESS_REJECTED"))?;
            let started = OffsetDateTime::parse(&started_at, &Rfc3339)
                .map_err(|_| known("LATTICE_MANAGED_RETAINED_EXECUTION_WINDOW_REJECTED"))?;
            if progress < started {
                return Err(known("LATTICE_MANAGED_RETAINED_PROGRESS_REJECTED"));
            }
        }
        if let Some(heartbeat_at) = self.retained_last_heartbeat_at.as_deref() {
            let heartbeat = OffsetDateTime::parse(heartbeat_at, &Rfc3339)
                .map_err(|_| known("LATTICE_MANAGED_RETAINED_HEARTBEAT_REJECTED"))?;
            let started = OffsetDateTime::parse(&started_at, &Rfc3339)
                .map_err(|_| known("LATTICE_MANAGED_RETAINED_EXECUTION_WINDOW_REJECTED"))?;
            if heartbeat < started {
                return Err(known("LATTICE_MANAGED_RETAINED_HEARTBEAT_REJECTED"));
            }
        }
        self.retained_attempt_started_at = Some(started_at);
        self.retained_attempt_deadline_at = Some(deadline_at);
        Ok(self)
    }

    pub(crate) const fn closed_blocker_code(&self) -> Option<&'static str> {
        self.closed_blocker_code
    }

    fn packet_json(&self) -> ManagedPortResult<Value> {
        let cwd = self
            .worktree
            .to_str()
            .ok_or_else(|| known("LATTICE_MANAGED_WORKTREE_REJECTED"))?;
        Ok(json!({
            "schema": self.packet.schema(),
            "task_ref": self.packet.task_ref(),
            "attempt": self.packet.attempt(),
            "project_ref": self.packet.project_ref(),
            "spec_ref": self.packet.spec_ref(),
            "approval_ref": self.packet.approval_ref(),
            "budget_digest": self.packet.budget_digest(),
            "global_active_limit": self.packet.global_active_limit(),
            "per_task_active_limit": self.packet.per_task_active_limit(),
            "repair_retry_limit": self.packet.repair_retry_limit(),
            "max_duration_seconds": self.packet.max_duration_seconds(),
            "max_total_tokens": self.packet.max_total_tokens(),
            "max_model_calls": self.packet.max_model_calls(),
            "remaining_total_tokens": self.packet.remaining_total_tokens(),
            "remaining_model_calls": self.packet.remaining_model_calls(),
            "external_cost_status": self.packet.external_cost().status(),
            "external_cost_limit_micros": self.packet.external_cost().limit_micros(),
            "non_model_external_spend_allowed": self.packet.non_model_external_spend_allowed(),
            "verification_ref": self.packet.verification_ref(),
            "worktree_ref": self.packet.worktree_ref(),
            "execution_environment_ref": self.packet.execution_environment_ref(),
            "base_commit": self.packet.base_commit(),
            "packet_digest": self.packet.digest(),
            "model_reason_digest": self.packet.model_selection().digest(),
            "model": self.packet.model_selection().model().as_str(),
            "reasoning": self.packet.model_selection().reasoning().as_str(),
            "deadline_at": self.packet.deadline_at(),
            "heartbeat_timeout_ms": self.heartbeat_timeout_ms,
            "writer_fence": self.packet.writer_fence(),
            "prior_terminal_evidence_ref": self.packet.prior_terminal_evidence_ref(),
            "continuation": self.packet.continuation().map(|value| value.text()),
            "continuation_digest": self.packet.continuation().map(|value| value.digest()),
            "cwd": cwd,
            "prompt": self.prompt,
        }))
    }

    fn initial_command(
        &self,
        operation: &str,
        retained: Option<Value>,
        claimed_at: Option<&str>,
    ) -> ManagedPortResult<Value> {
        let mut command = json!({
            "schema": COMMAND_SCHEMA,
            "operation": operation,
            "auth_context": {
                "schema": "lattice.managed-codex-auth-context/1.0",
                "codex_home_digest": self.effect_identity.auth_codex_home_digest(),
                "config_digest": self.effect_identity.auth_config_digest(),
            },
            "packet": self.packet_json()?,
        });
        if let Some(retained) = retained {
            command
                .as_object_mut()
                .ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_COMMAND_REJECTED"))?
                .insert("retained".to_owned(), retained);
        }
        if let Some(claimed_at) = claimed_at {
            command
                .as_object_mut()
                .ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_COMMAND_REJECTED"))?
                .insert(
                    "claimed_at".to_owned(),
                    Value::String(claimed_at.to_owned()),
                );
        }
        Ok(command)
    }

    fn retained(&self, thread_id: &str, turn_id: &str) -> ManagedPortResult<Value> {
        let attempt_started_at = self
            .retained_attempt_started_at
            .as_deref()
            .ok_or_else(|| known("LATTICE_MANAGED_RETAINED_EXACT_START_REQUIRED"))?;
        let attempt_deadline_at = self
            .retained_attempt_deadline_at
            .as_deref()
            .ok_or_else(|| known("LATTICE_MANAGED_RETAINED_EXACT_START_REQUIRED"))?;
        let last_meaningful_progress_at = self
            .retained_last_meaningful_progress_at
            .as_deref()
            .ok_or_else(|| known("LATTICE_MANAGED_RETAINED_PROGRESS_REQUIRED"))?;
        let last_heartbeat_at = self
            .retained_last_heartbeat_at
            .as_deref()
            .ok_or_else(|| known("LATTICE_MANAGED_RETAINED_HEARTBEAT_REQUIRED"))?;
        Ok(json!({
            "task_ref": self.packet.task_ref(),
            "attempt": self.packet.attempt(),
            "packet_digest": self.packet.digest(),
            "thread_id": thread_id,
            "turn_id": turn_id,
            "attempt_started_at": attempt_started_at,
            "attempt_deadline_at": attempt_deadline_at,
            "last_heartbeat_at": last_heartbeat_at,
            "last_meaningful_progress_at": last_meaningful_progress_at,
        }))
    }

    fn retained_empty_thread(&self, thread_id: &str) -> ManagedPortResult<Value> {
        if self.retained_thread_id.as_deref() != Some(thread_id) {
            return Err(known("LATTICE_MANAGED_RETAINED_THREAD_REQUIRED"));
        }
        Ok(json!({
            "task_ref": self.packet.task_ref(),
            "attempt": self.packet.attempt(),
            "packet_digest": self.packet.digest(),
            "thread_id": thread_id,
        }))
    }

    fn retained_prestart(
        &self,
        thread_id: &str,
        turn_id: Option<&str>,
    ) -> ManagedPortResult<Value> {
        let valid = |value: &str| {
            !value.is_empty()
                && value.len() <= 256
                && value.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-')
                })
        };
        if !valid(thread_id) || turn_id.is_some_and(|value| !valid(value)) {
            return Err(known("LATTICE_MANAGED_RETAINED_PRESTART_REJECTED"));
        }
        let mut retained = json!({
            "task_ref": self.packet.task_ref(),
            "attempt": self.packet.attempt(),
            "packet_digest": self.packet.digest(),
            "thread_id": thread_id,
        });
        if let Some(turn_id) = turn_id {
            retained
                .as_object_mut()
                .ok_or_else(|| known("LATTICE_MANAGED_RETAINED_PRESTART_REJECTED"))?
                .insert("turn_id".to_owned(), Value::String(turn_id.to_owned()));
        }
        Ok(retained)
    }

    fn authorize_turn_start_control(&self, thread_id: &str) -> ManagedPortResult<Value> {
        exact_turn_start_authorization_control(
            self.packet.task_ref(),
            self.packet.attempt(),
            self.packet.digest(),
            thread_id,
        )
    }

    fn send_control(&mut self, control: &Value) -> ManagedPortResult<()> {
        self.effect_identity.verify()?;
        let bridge = self
            .active
            .as_mut()
            .ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_NOT_ACTIVE"))?;
        let stdin = bridge
            .stdin
            .as_mut()
            .ok_or_else(|| known("LATTICE_MANAGED_INTERRUPT_CHANNEL_UNAVAILABLE"))?;
        serde_json::to_writer(&mut **stdin, control)
            .and_then(|()| stdin.write_all(b"\n").map_err(serde_json::Error::io))
            .and_then(|()| stdin.flush().map_err(serde_json::Error::io))
            .map_err(|_| ambiguous("LATTICE_MANAGED_INTERRUPT_AMBIGUOUS"))
    }

    fn send_turn_start_authorization(&mut self, control: &Value) -> ManagedPortResult<()> {
        let cancellation = self.cancellation.clone();
        let effect_gate = cancellation.lock_provider_effect_admission();
        if cancellation.is_requested() {
            drop(effect_gate);
            return self.close_prestart_for_cancellation();
        }
        let active = self
            .active
            .as_mut()
            .ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_NOT_ACTIVE"))?;
        if active.turn_start_lifecycle != TurnStartLifecycle::NotAuthorized {
            drop(effect_gate);
            return Err(known("LATTICE_MANAGED_TURN_START_AUTHORIZATION_REJECTED"));
        }
        // Commit the local state before the write: a failed/partial pipe write
        // can still have authorized a provider effect and therefore must never
        // be represented by a pre-start idle receipt.
        active.turn_start_lifecycle = TurnStartLifecycle::AuthorizationSent;
        let result = self.send_control(control);
        drop(effect_gate);
        result
    }

    fn send_provider_dispatch_authorization(&mut self, control: &Value) -> ManagedPortResult<()> {
        let cancellation = self.cancellation.clone();
        let effect_gate = cancellation.lock_provider_effect_admission();
        if cancellation.is_requested() {
            drop(effect_gate);
            return self.close_prestart_for_cancellation();
        }
        let active = self
            .active
            .as_mut()
            .ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_NOT_ACTIVE"))?;
        if active.provider_dispatch_authorized
            || active.provider_open_evidence.is_none()
            || !active.provider_readiness_verified
        {
            drop(effect_gate);
            return Err(known(
                "LATTICE_MANAGED_PROVIDER_DISPATCH_AUTHORIZATION_REJECTED",
            ));
        }
        active.provider_dispatch_authorized = true;
        let result = self.send_control(control);
        drop(effect_gate);
        result
    }

    fn spawn(&self, command: &Value, retain_stdin: bool) -> ManagedPortResult<ActiveBridge> {
        self.spawn_with_post_spawn_hook(command, retain_stdin, || {})
    }

    fn spawn_with_post_spawn_hook(
        &self,
        command: &Value,
        retain_stdin: bool,
        post_spawn_hook: impl FnOnce(),
    ) -> ManagedPortResult<ActiveBridge> {
        let operation = command
            .get("operation")
            .and_then(Value::as_str)
            .ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_COMMAND_REJECTED"))?
            .to_owned();
        let cancellation = self.cancellation.clone();
        let provider_effect_gate =
            (operation == "start").then(|| cancellation.lock_provider_effect_admission());
        if provider_effect_gate.is_some() && cancellation.is_requested() {
            cancellation.record_prestart_receipt(self.no_bridge_prestart_receipt())?;
            return Err(known(MANAGED_GRACEFUL_SHUTDOWN_IDLE));
        }
        let effect_seal = self.effect_identity.seal()?;
        let mut process = Command::new(&self.node_executable);
        process.arg(&self.bridge_path).current_dir(&self.worktree);
        configure_managed_codex_environment(
            &mut process,
            self.effect_identity
                .codex
                .as_ref()
                .map(ManagedCodexSpawnIdentity::launcher),
            &self.codex_home,
            self.packet.execution_environment_ref(),
            self.effect_identity
                .execution_environment_descriptor
                .as_deref(),
            self.effect_identity.execution_preflight_receipt.as_deref(),
            self.effect_identity
                .execution_preflight_descriptor_digest
                .as_deref(),
            self.effect_identity
                .execution_preflight_content_digest
                .as_deref(),
        )?;
        let mut child = SupervisedDuplexChild::spawn_cleared(&mut process)
            .map_err(|_| ambiguous("LATTICE_MANAGED_BRIDGE_START_AMBIGUOUS"))?;
        post_spawn_hook();
        let mut pending_reader = None;
        let mut pending_receiver = None;
        let setup_result = (|| {
            // Node opens the ESM graph after process creation.  Recheck the
            // entry, every local import, Node, and Codex launcher before the
            // first command can create a provider thread or turn.
            self.effect_identity.verify()?;
            let stdout = child
                .take_stdout()
                .ok_or_else(|| ambiguous("LATTICE_MANAGED_BRIDGE_START_AMBIGUOUS"))?;
            let (record_sender, record_receiver) = mpsc::sync_channel(BRIDGE_RECORD_QUEUE);
            let attempt = self.packet.attempt();
            let reader = thread::Builder::new()
                .name(format!("lattice-managed-bridge-reader-{attempt}"))
                .spawn(move || {
                    let _activity = WorkerReaderActivity::new();
                    let mut stdout = BufReader::new(stdout);
                    loop {
                        let record = read_record(&mut stdout);
                        let failed = record.is_err();
                        if record_sender.send(record).is_err() || failed {
                            return;
                        }
                    }
                })
                .map_err(|_| ambiguous("LATTICE_MANAGED_BRIDGE_READER_REJECTED"))?;
            pending_reader = Some(reader);
            pending_receiver = Some(record_receiver);
            let mut stdin = child
                .take_stdin()
                .ok_or_else(|| ambiguous("LATTICE_MANAGED_BRIDGE_START_AMBIGUOUS"))?;
            serde_json::to_writer(&mut stdin, command)
                .and_then(|()| stdin.write_all(b"\n").map_err(serde_json::Error::io))
                .and_then(|()| stdin.flush().map_err(serde_json::Error::io))
                .map_err(|_| ambiguous("LATTICE_MANAGED_BRIDGE_WRITE_AMBIGUOUS"))?;
            Ok::<_, ManagedPortError>(stdin)
        })();
        let stdin = match setup_result {
            Ok(stdin) => stdin,
            Err(failure) => {
                let cleanup = child
                    .terminate_and_reap()
                    .map_err(|_| ambiguous("LATTICE_MANAGED_BRIDGE_CLEANUP_AMBIGUOUS"));
                drop(pending_receiver.take());
                let joined = pending_reader
                    .take()
                    .map(|reader| {
                        reader
                            .join()
                            .map_err(|_| ambiguous("LATTICE_MANAGED_BRIDGE_READER_REJECTED"))
                    })
                    .transpose();
                cleanup?;
                joined?;
                return Err(failure);
            }
        };
        let record_receiver = pending_receiver
            .take()
            .ok_or_else(|| ambiguous("LATTICE_MANAGED_BRIDGE_READER_REJECTED"))?;
        let reader = pending_reader
            .take()
            .ok_or_else(|| ambiguous("LATTICE_MANAGED_BRIDGE_READER_REJECTED"))?;
        let turn_start_lifecycle = if operation == "resume" {
            TurnStartLifecycle::ExactStarted
        } else if operation == "recover-prestart" && self.retained_turn_id.is_some() {
            TurnStartLifecycle::Accepted
        } else {
            TurnStartLifecycle::NotAuthorized
        };
        self.cancellation.register_bridge();
        Ok(ActiveBridge {
            child,
            stdin: retain_stdin.then_some(stdin),
            records: Some(record_receiver),
            reader: Some(reader),
            operation,
            task_ref: self.packet.task_ref().to_owned(),
            attempt: self.packet.attempt(),
            packet_digest: self.packet.digest().to_owned(),
            thread_id: None,
            turn_id: None,
            app_server_generation: None,
            app_server_identity_digest: None,
            reconciliation_digest: None,
            provider_open_evidence: None,
            provider_readiness_requested: false,
            provider_readiness_verified: false,
            provider_dispatch_authorized: false,
            turn_start_lifecycle,
            exact_active: false,
            cancellation: self.cancellation.clone(),
            bridge_registered: true,
            effect_identity: self.effect_identity.clone(),
            _effect_seal: effect_seal,
        })
    }

    fn no_bridge_prestart_receipt(&self) -> ManagedWorkerPrestartShutdownReceipt {
        ManagedWorkerPrestartShutdownReceipt {
            task_ref: self.packet.task_ref().to_owned(),
            attempt: self.packet.attempt(),
            packet_digest: self.packet.digest().to_owned(),
            thread_id: self.retained_thread_id.clone(),
            turn_id: self.retained_turn_id.clone(),
            disposition: ManagedPrestartShutdownDisposition::NoBridgeSpawned,
        }
    }

    fn reject_if_cancelled_before_bridge(&self) -> ManagedPortResult<()> {
        if !self.cancellation.is_requested() {
            return Ok(());
        }
        self.cancellation
            .record_prestart_receipt(self.no_bridge_prestart_receipt())?;
        Err(known(MANAGED_GRACEFUL_SHUTDOWN_IDLE))
    }

    fn close_prestart_for_cancellation(&mut self) -> ManagedPortResult<()> {
        let mut bridge = self
            .active
            .take()
            .ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_NOT_ACTIVE"))?;
        let receipt = bridge.terminate_prestart_and_reap()?;
        self.cancellation.record_prestart_receipt(receipt)?;
        Err(known(MANAGED_GRACEFUL_SHUTDOWN_IDLE))
    }

    fn reject_if_cancelled_prestart(&mut self) -> ManagedPortResult<()> {
        if !self.cancellation.is_requested() {
            return Ok(());
        }
        let Some(disposition) = self
            .active
            .as_ref()
            .map(|bridge| bridge.cancellation_disposition(false))
        else {
            return self.reject_if_cancelled_before_bridge();
        };
        match disposition {
            CancellationDisposition::Prestart => self.close_prestart_for_cancellation(),
            CancellationDisposition::AmbiguousAuthorization => {
                self.terminate_without_shutdown_receipt()?;
                Err(ambiguous("LATTICE_MANAGED_TURN_START_SHUTDOWN_AMBIGUOUS"))
            }
            CancellationDisposition::DrainExactIdentity => Ok(()),
        }
    }

    fn next_bridge_record(&mut self) -> ManagedPortResult<Value> {
        loop {
            let cancelled = self.cancellation.is_requested();
            let retained_exact_reconciliation = self.retained_attempt_started_at.is_some()
                && self
                    .active
                    .as_ref()
                    .is_some_and(|bridge| bridge.operation == "resume");
            let cancellation_disposition = self
                .active
                .as_ref()
                .map(|bridge| bridge.cancellation_disposition(retained_exact_reconciliation));
            if cancelled {
                match cancellation_disposition {
                    Some(CancellationDisposition::Prestart) => {
                        self.close_prestart_for_cancellation()?;
                        unreachable!("graceful prestart cancellation always returns an error");
                    }
                    Some(CancellationDisposition::AmbiguousAuthorization) => {
                        self.terminate_without_shutdown_receipt()?;
                        return Err(ambiguous("LATTICE_MANAGED_TURN_START_SHUTDOWN_AMBIGUOUS"));
                    }
                    Some(CancellationDisposition::DrainExactIdentity) => {}
                    None => {
                        self.reject_if_cancelled_before_bridge()?;
                        unreachable!("graceful pre-bridge cancellation always returns an error");
                    }
                }
            }
            if cancelled {
                let deadline = self.shutdown_deadline.get_or_insert_with(|| {
                    Instant::now()
                        .checked_add(BRIDGE_TEARDOWN_GRACE)
                        .unwrap_or_else(Instant::now)
                });
                if Instant::now() >= *deadline {
                    self.terminate_without_shutdown_receipt()?;
                    return Err(ambiguous(MANAGED_GRACEFUL_SHUTDOWN_RECEIPT_REQUIRED));
                }
            }
            let outcome = {
                let bridge = self
                    .active
                    .as_ref()
                    .ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_NOT_ACTIVE"))?;
                if cancelled {
                    bridge
                        .next_record_timeout(CANCELLATION_POLL)?
                        .map(CancellableBridgeRecord::Record)
                        .unwrap_or(CancellableBridgeRecord::Cancelled)
                } else {
                    receive_record_or_cancellation(
                        bridge
                            .records
                            .as_ref()
                            .ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_READER_REJECTED"))?,
                        &self.cancellation,
                        CANCELLATION_POLL,
                    )?
                }
            };
            match outcome {
                CancellableBridgeRecord::Record(record) => return Ok(record),
                CancellableBridgeRecord::Cancelled => {}
            }
        }
    }

    fn terminate_without_shutdown_receipt(&mut self) -> ManagedPortResult<()> {
        if let Some(mut bridge) = self.active.take() {
            bridge.stdin.take();
            bridge
                .child
                .terminate_and_reap()
                .map_err(|_| ambiguous(MANAGED_GRACEFUL_SHUTDOWN_RECEIPT_REQUIRED))?;
            bridge.record_subtree_reaped();
            bridge.records.take();
            bridge.join_reader()?;
        }
        Ok(())
    }

    fn shutdown_deadline_expired(&self) -> bool {
        self.shutdown_deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
    }

    fn arm_execution_watchdog(&mut self, attempt_deadline_at: &str) -> ManagedPortResult<()> {
        let deadline = canonical_execution_time(attempt_deadline_at)?;
        let remaining_ms = (deadline - OffsetDateTime::now_utc())
            .whole_milliseconds()
            .max(0);
        let remaining_ms = u64::try_from(remaining_ms)
            .map_err(|_| known("LATTICE_MANAGED_EXECUTION_WATCHDOG_REJECTED"))?;
        let now = Instant::now();
        let hard_deadline = now
            .checked_add(StdDuration::from_millis(remaining_ms))
            .and_then(|deadline| deadline.checked_add(BRIDGE_TEARDOWN_GRACE))
            .ok_or_else(|| known("LATTICE_MANAGED_EXECUTION_WATCHDOG_REJECTED"))?;
        self.execution_last_activity = Some(now);
        self.execution_hard_deadline = Some(hard_deadline);
        Ok(())
    }

    fn execution_watchdog_remaining(&self) -> ManagedPortResult<Option<StdDuration>> {
        let last_activity = self
            .execution_last_activity
            .ok_or_else(|| known("LATTICE_MANAGED_EXECUTION_WATCHDOG_REJECTED"))?;
        let hard_deadline = self
            .execution_hard_deadline
            .ok_or_else(|| known("LATTICE_MANAGED_EXECUTION_WATCHDOG_REJECTED"))?;
        let now = Instant::now();
        Ok(execution_watchdog_remaining_at(
            last_activity,
            hard_deadline,
            StdDuration::from_millis(self.heartbeat_timeout_ms),
            now,
        ))
    }

    fn fail_silent_execution_bridge(&mut self) -> ManagedPortResult<ManagedWorkerExecutionEvent> {
        self.closed_blocker_code =
            Some("LATTICE_MANAGED_BRIDGE_HEARTBEAT_TIMEOUT_RECONCILIATION_REQUIRED");
        self.terminate_without_shutdown_receipt()?;
        Err(ambiguous(
            "LATTICE_MANAGED_BRIDGE_HEARTBEAT_TIMEOUT_RECONCILIATION_REQUIRED",
        ))
    }

    fn run_probe(&mut self) -> ManagedPortResult<ManagedModelAvailability> {
        if self
            .effect_identity
            .execution_environment_descriptor
            .is_some()
        {
            return self.run_prepared_provider_probe();
        }
        self.reject_if_cancelled_before_bridge()?;
        self.preclaim_auth_readiness = None;
        let packet_deadline = canonical_execution_time(self.packet.deadline_at())?;
        let probe_budget = probe_watchdog_budget_at(
            packet_deadline,
            StdDuration::from_millis(self.heartbeat_timeout_ms),
            OffsetDateTime::now_utc(),
        )
        .ok_or_else(|| {
            ManagedPortError::new(
                ManagedPortErrorKind::ReconcileRequired,
                MANAGED_MODEL_PROBE_TIMEOUT_RECONCILIATION_REQUIRED,
            )
        })?;
        let probe_deadline = Instant::now()
            .checked_add(probe_budget)
            .ok_or_else(|| known("LATTICE_MANAGED_MODEL_PROBE_WATCHDOG_REJECTED"))?;
        let command = self.initial_command("probe", None, None)?;
        let mut bridge = self.spawn(&command, false)?;
        let record = loop {
            let records = bridge
                .records
                .as_ref()
                .ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_READER_REJECTED"))?;
            match receive_probe_record_until(
                records,
                &self.cancellation,
                CANCELLATION_POLL,
                probe_deadline,
            )? {
                BoundedProbeRecord::Record(record) => break record,
                BoundedProbeRecord::Cancelled => {
                    let receipt = bridge.terminate_prestart_and_reap()?;
                    self.cancellation.record_prestart_receipt(receipt)?;
                    return Err(known(MANAGED_GRACEFUL_SHUTDOWN_IDLE));
                }
                BoundedProbeRecord::DeadlineElapsed => {
                    let receipt = bridge.terminate_prestart_and_reap()?;
                    self.cancellation.record_prestart_receipt(receipt)?;
                    return Err(ManagedPortError::new(
                        ManagedPortErrorKind::ReconcileRequired,
                        MANAGED_MODEL_PROBE_TIMEOUT_RECONCILIATION_REQUIRED,
                    ));
                }
            }
        };
        let status = bridge
            .wait_for_root(BRIDGE_TEARDOWN_GRACE)
            .map_err(|_| ambiguous("LATTICE_MANAGED_MODEL_PROBE_AMBIGUOUS"))?;
        let status = status.ok_or_else(|| ambiguous("LATTICE_MANAGED_MODEL_PROBE_AMBIGUOUS"))?;
        bridge.prove_subtree_empty_and_join_reader("LATTICE_MANAGED_MODEL_PROBE_AMBIGUOUS")?;
        if record.get("kind").and_then(Value::as_str) == Some("result")
            && record.pointer("/result/available").and_then(Value::as_bool) == Some(true)
            && record.pointer("/result/model").and_then(Value::as_str)
                == Some(self.packet.model_selection().model().as_str())
            && status.success()
        {
            let readiness = parse_managed_auth_readiness(
                record
                    .pointer("/result/auth_readiness")
                    .ok_or_else(|| known("CREDENTIAL_READ_ISOLATION_NOT_VERIFIED"))?,
                self.effect_identity.auth_codex_home_digest(),
                self.effect_identity.auth_config_digest(),
            )?;
            self.preclaim_auth_readiness = Some(readiness);
            return Ok(ManagedModelAvailability::Available);
        }
        if status.code() == Some(3) && record.get("kind").and_then(Value::as_str) == Some("error") {
            return Ok(ManagedModelAvailability::Unavailable {
                code: "MANAGED_CODEX_MODEL_UNAVAILABLE",
            });
        }
        if record.get("kind").and_then(Value::as_str) == Some("error") {
            return Err(bridge_reported_failure(&record));
        }
        Err(known("LATTICE_MANAGED_MODEL_PROBE_REJECTED"))
    }

    fn require_preclaim_auth_readiness(&self) -> ManagedPortResult<()> {
        self.effect_identity.verify()?;
        validate_preclaim_auth_readiness(
            self.preclaim_auth_readiness.as_ref(),
            self.effect_identity.auth_codex_home_digest(),
            self.effect_identity.auth_config_digest(),
        )
    }

    fn run_prepared_provider_probe(&mut self) -> ManagedPortResult<ManagedModelAvailability> {
        self.reject_if_cancelled_prestart()?;
        self.effect_identity.verify()?;
        self.preclaim_auth_readiness = None;
        let open = self
            .active
            .as_ref()
            .filter(|bridge| {
                bridge.operation == "start"
                    && !bridge.provider_dispatch_authorized
                    && !bridge.provider_readiness_requested
            })
            .and_then(|bridge| bridge.provider_open_evidence.as_ref())
            .ok_or_else(|| known("LATTICE_MANAGED_PROVIDER_SUBTREE_OPEN_REQUIRED"))?;
        let descriptor = self
            .effect_identity
            .execution_environment_descriptor
            .as_deref()
            .ok_or_else(|| known("LATTICE_MANAGED_PROVIDER_SUBTREE_OPEN_REQUIRED"))?;
        let preflight = self
            .effect_identity
            .execution_preflight_evidence
            .as_ref()
            .ok_or_else(|| known("LATTICE_MANAGED_PROVIDER_SUBTREE_OPEN_REQUIRED"))?;
        let parsed = validate_wsl2_provider_subtree_evidence(
            &self.packet,
            descriptor,
            preflight,
            None,
            open,
        )?;
        let control = exact_provider_readiness_control(
            self.packet.task_ref(),
            self.packet.attempt(),
            self.packet.digest(),
            parsed.closure_digest(),
        )?;
        self.active
            .as_mut()
            .ok_or_else(|| known("LATTICE_MANAGED_PROVIDER_SUBTREE_OPEN_REQUIRED"))?
            .provider_readiness_requested = true;
        self.send_control(&control)?;
        let record = self.next_bridge_record()?;
        if !exact_value_keys(
            &record,
            &[
                "kind",
                "status",
                "task_ref",
                "attempt",
                "packet_digest",
                "marker_digest",
                "model",
                "auth_readiness",
                "code",
            ],
        ) || record.get("kind").and_then(Value::as_str) != Some("provider_model_availability")
            || record.get("task_ref").and_then(Value::as_str) != Some(self.packet.task_ref())
            || record.get("attempt").and_then(Value::as_u64)
                != Some(u64::from(self.packet.attempt()))
            || record.get("packet_digest").and_then(Value::as_str) != Some(self.packet.digest())
            || record.get("marker_digest").and_then(Value::as_str) != Some(parsed.closure_digest())
            || record.get("model").and_then(Value::as_str)
                != Some(self.packet.model_selection().model().as_str())
        {
            return Err(known("LATTICE_MANAGED_PROVIDER_READINESS_REJECTED"));
        }
        match record.get("status").and_then(Value::as_str) {
            Some("AVAILABLE") if record.get("code") == Some(&Value::Null) => {
                let readiness = parse_managed_auth_readiness(
                    record
                        .get("auth_readiness")
                        .ok_or_else(|| known("CREDENTIAL_READ_ISOLATION_NOT_VERIFIED"))?,
                    self.effect_identity.auth_codex_home_digest(),
                    self.effect_identity.auth_config_digest(),
                )?;
                self.preclaim_auth_readiness = Some(readiness);
                self.active
                    .as_mut()
                    .ok_or_else(|| known("LATTICE_MANAGED_PROVIDER_SUBTREE_OPEN_REQUIRED"))?
                    .provider_readiness_verified = true;
                Ok(ManagedModelAvailability::Available)
            }
            Some("UNAVAILABLE")
                if record.get("auth_readiness") == Some(&Value::Null)
                    && record.get("code").and_then(Value::as_str)
                        == Some("MANAGED_CODEX_MODEL_UNAVAILABLE") =>
            {
                Ok(ManagedModelAvailability::Unavailable {
                    code: "MANAGED_CODEX_MODEL_UNAVAILABLE",
                })
            }
            Some("ERROR") if record.get("auth_readiness") == Some(&Value::Null) => {
                Err(known("LATTICE_MANAGED_PROVIDER_READINESS_REJECTED"))
            }
            _ => Err(known("LATTICE_MANAGED_PROVIDER_READINESS_REJECTED")),
        }
    }

    fn ensure_packet(
        &self,
        attempt: &VerifiedWorkerAttemptRecord,
        packet: Option<&AttemptPacketIdentity>,
    ) -> ManagedPortResult<()> {
        if attempt.attempt_number() != u64::from(self.packet.attempt())
            || attempt.task_ref().as_str() != self.packet.task_ref()
            || packet.is_some_and(|value| value != &self.packet)
        {
            return Err(known("LATTICE_MANAGED_WORKER_BINDING_REJECTED"));
        }
        Ok(())
    }

    fn next_matching_event(&mut self, expected: &[&str]) -> ManagedPortResult<Value> {
        loop {
            let record = self.next_bridge_record()?;
            match record.get("kind").and_then(Value::as_str) {
                Some("event") => {
                    let event = record
                        .get("event")
                        .cloned()
                        .ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_OUTPUT_REJECTED"))?;
                    let kind = event
                        .get("event_type")
                        .and_then(Value::as_str)
                        .ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_OUTPUT_REJECTED"))?;
                    if expected.contains(&kind) {
                        return Ok(event);
                    }
                }
                Some("error") => {
                    return Err(bridge_reported_failure(&record));
                }
                Some("result") => {
                    return Err(known("LATTICE_MANAGED_BRIDGE_EARLY_RESULT"));
                }
                _ => return Err(known("LATTICE_MANAGED_BRIDGE_OUTPUT_REJECTED")),
            }
        }
    }

    fn next_matching_event_timeout(
        &mut self,
        expected: &[&str],
        timeout: StdDuration,
    ) -> ManagedPortResult<Option<Value>> {
        let bridge = self
            .active
            .as_ref()
            .ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_NOT_ACTIVE"))?;
        let Some(record) = bridge.next_record_timeout(timeout)? else {
            return Ok(None);
        };
        match record.get("kind").and_then(Value::as_str) {
            Some("event") => {
                let event = record
                    .get("event")
                    .cloned()
                    .ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_OUTPUT_REJECTED"))?;
                let kind = event
                    .get("event_type")
                    .and_then(Value::as_str)
                    .ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_OUTPUT_REJECTED"))?;
                Ok(expected.contains(&kind).then_some(event))
            }
            Some("error") => Err(bridge_reported_failure(&record)),
            Some("result") => Err(known("LATTICE_MANAGED_BRIDGE_EARLY_RESULT")),
            _ => Err(known("LATTICE_MANAGED_BRIDGE_OUTPUT_REJECTED")),
        }
    }

    fn exact_event_identity(
        &mut self,
        event: &Value,
        expected_thread: Option<&str>,
        expected_turn: Option<&str>,
    ) -> ManagedPortResult<(String, Option<String>, u64, ContentDigest, ContentDigest)> {
        if event.get("task_ref").and_then(Value::as_str) != Some(self.packet.task_ref())
            || event.get("attempt").and_then(Value::as_u64)
                != Some(u64::from(self.packet.attempt()))
            || event.get("packet_digest").and_then(Value::as_str) != Some(self.packet.digest())
        {
            return Err(known("LATTICE_MANAGED_BRIDGE_IDENTITY_MISMATCH"));
        }
        let thread = event
            .get("thread_id")
            .and_then(Value::as_str)
            .ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_IDENTITY_MISMATCH"))?
            .to_owned();
        let turn = event
            .get("turn_id")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if expected_thread.is_some_and(|value| value != thread)
            || expected_turn.is_some_and(|value| turn.as_deref() != Some(value))
        {
            return Err(known("LATTICE_MANAGED_BRIDGE_IDENTITY_MISMATCH"));
        }
        let generation = event
            .get("app_server_generation")
            .and_then(Value::as_u64)
            .filter(|value| *value > 0)
            .ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_GENERATION_REJECTED"))?;
        let app_server_identity_digest = managed_app_server_identity_digest(
            event
                .get("app_server_session_id")
                .and_then(Value::as_str)
                .ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_APP_SERVER_IDENTITY_REJECTED"))?,
            event
                .get("codex_home_digest")
                .and_then(Value::as_str)
                .ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_APP_SERVER_IDENTITY_REJECTED"))?,
            event
                .get("config_digest")
                .and_then(Value::as_str)
                .ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_APP_SERVER_IDENTITY_REJECTED"))?,
            self.effect_identity.auth_codex_home_digest(),
            self.effect_identity.auth_config_digest(),
        )?;
        let event_type = event
            .get("event_type")
            .and_then(Value::as_str)
            .ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_OUTPUT_REJECTED"))?;
        let evidence = parse_prefixed_digest(
            event
                .get("evidence_digest")
                .and_then(Value::as_str)
                .ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_EVIDENCE_REJECTED"))?,
            "managed-worker-event:sha256:",
        )?;
        if let Some(active) = self.active.as_mut() {
            if active
                .thread_id
                .as_deref()
                .is_some_and(|value| value != thread)
                || active
                    .turn_id
                    .as_deref()
                    .is_some_and(|value| turn.as_deref() != Some(value))
            {
                return Err(known("LATTICE_MANAGED_BRIDGE_IDENTITY_MISMATCH"));
            }
            let generation = reconcile_app_server_generation(
                active.app_server_generation,
                generation,
                event_type,
                active.exact_active,
            )?;
            if active
                .app_server_identity_digest
                .as_ref()
                .is_some_and(|current| current != &app_server_identity_digest)
                && !(active.exact_active && event_type == "RECONCILE_STARTED")
            {
                return Err(known("LATTICE_MANAGED_BRIDGE_APP_SERVER_IDENTITY_REJECTED"));
            }
            active.thread_id = Some(thread.clone());
            if turn.is_some() {
                active.turn_id.clone_from(&turn);
            }
            active.app_server_generation = Some(generation);
            active.app_server_identity_digest = Some(app_server_identity_digest.clone());
            match event_type {
                "TURN_START_ACCEPTED" => {
                    if active.turn_start_lifecycle != TurnStartLifecycle::AuthorizationSent {
                        return Err(known("LATTICE_MANAGED_TURN_START_AUTHORIZATION_REJECTED"));
                    }
                    active.turn_start_lifecycle = TurnStartLifecycle::Accepted;
                }
                "TURN_STARTED" => {
                    if active.turn_start_lifecycle != TurnStartLifecycle::Accepted {
                        return Err(known("LATTICE_MANAGED_EXACT_START_REJECTED"));
                    }
                    active.turn_start_lifecycle = TurnStartLifecycle::ExactStarted;
                    active.exact_active = true;
                }
                "RECONCILED_ACTIVE" => {
                    active.turn_start_lifecycle = TurnStartLifecycle::ExactStarted;
                    active.exact_active = true;
                }
                _ => {}
            }
        }
        Ok((
            thread,
            turn,
            generation,
            app_server_identity_digest,
            evidence,
        ))
    }

    fn finalize_terminal(
        &mut self,
    ) -> ManagedPortResult<(
        ManagedBridgeProcessExitReceipt,
        Option<VerifiedManagedEvidence>,
    )> {
        let mut bridge = self
            .active
            .take()
            .ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_NOT_ACTIVE"))?;
        bridge.stdin.take();
        let result_deadline = self.shutdown_deadline.unwrap_or_else(|| {
            Instant::now()
                .checked_add(BRIDGE_TEARDOWN_GRACE)
                .unwrap_or_else(Instant::now)
        });
        let mut provider_lifecycle = None;
        let result_record = loop {
            if Instant::now() >= result_deadline {
                return Err(ambiguous("LATTICE_MANAGED_BRIDGE_TERMINAL_AMBIGUOUS"));
            }
            let Some(record) = bridge.next_record_timeout(CANCELLATION_POLL)? else {
                continue;
            };
            match record.get("kind").and_then(Value::as_str) {
                Some("event") => {}
                Some("provider_subtree_receipt") => {
                    if provider_lifecycle.is_some() {
                        return Err(known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"));
                    }
                    let open = bridge
                        .provider_open_evidence
                        .as_ref()
                        .ok_or_else(|| known("LATTICE_MANAGED_PROVIDER_SUBTREE_OPEN_REQUIRED"))?;
                    provider_lifecycle = Some(self.provider_lifecycle_evidence(
                        &record,
                        "provider_subtree_receipt",
                        Some(open),
                    )?);
                }
                Some("result") => {
                    if bridge.provider_open_evidence.is_some() && provider_lifecycle.is_none() {
                        return Err(known("LATTICE_MANAGED_PROVIDER_SUBTREE_RECEIPT_REQUIRED"));
                    }
                    break record;
                }
                _ => return Err(known("LATTICE_MANAGED_BRIDGE_OUTPUT_REJECTED")),
            }
        };
        let terminal = match result_record
            .pointer("/result/status")
            .and_then(Value::as_str)
        {
            Some("completed") => WorkerTerminal::Completed,
            Some("interrupted") => WorkerTerminal::Interrupted,
            Some("failed") => WorkerTerminal::Failed,
            _ => return Err(known("LATTICE_MANAGED_BRIDGE_TERMINAL_REJECTED")),
        };
        if result_record.get("schema").and_then(Value::as_str) != Some(RESULT_SCHEMA)
            || result_record.get("task_ref").and_then(Value::as_str)
                != Some(bridge.task_ref.as_str())
            || result_record.get("attempt").and_then(Value::as_u64)
                != Some(u64::from(bridge.attempt))
            || result_record.get("packet_digest").and_then(Value::as_str)
                != Some(bridge.packet_digest.as_str())
            || !matches!(
                result_record.get("operation").and_then(Value::as_str),
                Some(
                    "start"
                        | "recover-dispatch"
                        | "recover-prestart"
                        | "continue-turn"
                        | "resume"
                        | "recover"
                )
            )
        {
            return Err(known("LATTICE_MANAGED_BRIDGE_TERMINAL_REJECTED"));
        }
        let status = bridge
            .wait_for_root(BRIDGE_TEARDOWN_GRACE)
            .map_err(|_| ambiguous("LATTICE_MANAGED_BRIDGE_TERMINAL_AMBIGUOUS"))?;
        let status =
            status.ok_or_else(|| ambiguous("LATTICE_MANAGED_BRIDGE_TERMINAL_AMBIGUOUS"))?;
        if !status.success() && status.code() != Some(6) {
            return Err(known("LATTICE_MANAGED_BRIDGE_TERMINAL_REJECTED"));
        }
        bridge
            .child
            .terminate_and_reap()
            .map_err(|_| ambiguous("LATTICE_MANAGED_BRIDGE_TERMINAL_AMBIGUOUS"))?;
        bridge.record_subtree_reaped();
        bridge.records.take();
        bridge.join_reader()?;
        Ok((
            ManagedBridgeProcessExitReceipt { terminal },
            provider_lifecycle,
        ))
    }

    fn finalize_consumed_result(&mut self, allow_failed_exit: bool) -> ManagedPortResult<()> {
        let mut bridge = self
            .active
            .take()
            .ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_NOT_ACTIVE"))?;
        bridge.stdin.take();
        let status = bridge
            .wait_for_root(BRIDGE_TEARDOWN_GRACE)
            .map_err(|_| ambiguous("LATTICE_MANAGED_BRIDGE_TERMINAL_AMBIGUOUS"))?
            .ok_or_else(|| ambiguous("LATTICE_MANAGED_BRIDGE_TERMINAL_AMBIGUOUS"))?;
        if !status.success() && !(allow_failed_exit && status.code() == Some(6)) {
            return Err(known("LATTICE_MANAGED_BRIDGE_TERMINAL_REJECTED"));
        }
        bridge.prove_subtree_empty_and_join_reader("LATTICE_MANAGED_BRIDGE_TERMINAL_AMBIGUOUS")?;
        Ok(())
    }

    fn collect_prestart_recovery(
        &mut self,
        expected_thread: Option<&str>,
        expected_turn: Option<&str>,
    ) -> ManagedPortResult<ManagedWorkerPrestartRecovery> {
        let mut thread: Option<ManagedWorkerObservation> = None;
        let mut turn: Option<ManagedWorkerObservation> = None;
        loop {
            let record = self.next_bridge_record()?;
            match record.get("kind").and_then(Value::as_str) {
                Some("error") => {
                    let failure = bridge_reported_failure(&record);
                    // Even a typed reconciliation result may not release the
                    // exact attempt while the old bridge subtree is still
                    // alive: a subsequent recovery spawn could duplicate the
                    // provider effect.  Convert the bridge error only after
                    // synchronous subtree-zero and bounded reader join proof.
                    self.terminate_active_bridge_after_error(
                        "LATTICE_MANAGED_PRESTART_RECOVERY_CLEANUP_AMBIGUOUS",
                    )?;
                    if failure.kind() == ManagedPortErrorKind::ReconcileRequired {
                        return Ok(ManagedWorkerPrestartRecovery::ReconciliationRequired);
                    }
                    return Err(failure);
                }
                Some("result") => {
                    if record.get("task_ref").and_then(Value::as_str)
                        != Some(self.packet.task_ref())
                        || record.get("attempt").and_then(Value::as_u64)
                            != Some(u64::from(self.packet.attempt()))
                        || record.get("packet_digest").and_then(Value::as_str)
                            != Some(self.packet.digest())
                    {
                        return Err(known("LATTICE_MANAGED_PRESTART_RECOVERY_REJECTED"));
                    }
                    if record.pointer("/result/kind").and_then(Value::as_str)
                        == Some("PROVEN_NO_PROVIDER_CANDIDATE")
                    {
                        if thread.is_some() || turn.is_some() {
                            return Err(known("LATTICE_MANAGED_PRESTART_RECOVERY_REJECTED"));
                        }
                        self.finalize_consumed_result(false)?;
                        return Ok(ManagedWorkerPrestartRecovery::ProvenNoProviderCandidate);
                    }
                    if record.pointer("/result/kind").and_then(Value::as_str)
                        != Some("EXACT_EMPTY_THREAD")
                        || turn.is_some()
                    {
                        return Err(known("LATTICE_MANAGED_PRESTART_RECOVERY_REJECTED"));
                    }
                    let recovered_thread = thread
                        .take()
                        .ok_or_else(|| known("LATTICE_MANAGED_PRESTART_RECOVERY_REJECTED"))?;
                    if record.pointer("/result/thread_id").and_then(Value::as_str)
                        != Some(recovered_thread.thread_id())
                        || self
                            .retained_thread_id
                            .as_deref()
                            .is_some_and(|retained| retained != recovered_thread.thread_id())
                    {
                        return Err(known("LATTICE_MANAGED_PRESTART_RECOVERY_REJECTED"));
                    }
                    let retained_thread_id = recovered_thread.thread_id().to_owned();
                    self.finalize_consumed_result(false)?;
                    self.retained_thread_id = Some(retained_thread_id);
                    return Ok(ManagedWorkerPrestartRecovery::ExactEmptyThread {
                        thread: recovered_thread,
                    });
                }
                Some("event") => {
                    let event = record
                        .get("event")
                        .cloned()
                        .ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_OUTPUT_REJECTED"))?;
                    let event_type = event
                        .get("event_type")
                        .and_then(Value::as_str)
                        .ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_OUTPUT_REJECTED"))?;
                    let event_expected_turn = (event_type != "THREAD_START_ACCEPTED")
                        .then_some(expected_turn)
                        .flatten();
                    let (thread_id, turn_id, generation, app_server_identity, evidence) =
                        self.exact_event_identity(&event, expected_thread, event_expected_turn)?;
                    match event_type {
                        "THREAD_START_ACCEPTED" => {
                            if turn_id.is_some() || thread.is_some() {
                                return Err(known("LATTICE_MANAGED_PRESTART_RECOVERY_REJECTED"));
                            }
                            thread = Some(ManagedWorkerObservation::thread_accepted(
                                self.packet.attempt().into(),
                                thread_id,
                                generation,
                                app_server_identity.clone(),
                                evidence,
                            )?);
                        }
                        "TURN_START_ACCEPTED" => {
                            if thread
                                .as_ref()
                                .is_none_or(|value| value.thread_id() != thread_id)
                                || turn.is_some()
                            {
                                return Err(known("LATTICE_MANAGED_PRESTART_RECOVERY_REJECTED"));
                            }
                            turn = Some(ManagedWorkerObservation::turn_accepted(
                                self.packet.attempt().into(),
                                thread_id,
                                turn_id.ok_or_else(|| {
                                    known("LATTICE_MANAGED_PRESTART_RECOVERY_REJECTED")
                                })?,
                                generation,
                                app_server_identity.clone(),
                                evidence,
                            )?);
                        }
                        "INTERRUPT_REQUESTED" => {
                            if turn_id.is_none()
                                || thread
                                    .as_ref()
                                    .is_none_or(|value| value.thread_id() != thread_id)
                                || turn
                                    .as_ref()
                                    .is_none_or(|value| value.turn_id() != turn_id.as_deref())
                            {
                                return Err(known("LATTICE_MANAGED_PRESTART_RECOVERY_REJECTED"));
                            }
                        }
                        "PRESTART_TERMINAL" => {
                            if event.get("status").and_then(Value::as_str) != Some("failed")
                                || !matches!(
                                    event
                                        .get("provider_terminal_status")
                                        .and_then(Value::as_str),
                                    Some("completed" | "interrupted" | "failed")
                                )
                                || event.get("failure_reason").and_then(Value::as_str)
                                    != Some("EXACT_START_NOT_DURABLE")
                            {
                                return Err(known("LATTICE_MANAGED_PRESTART_RECOVERY_REJECTED"));
                            }
                            let thread_observation = thread.take().ok_or_else(|| {
                                known("LATTICE_MANAGED_PRESTART_RECOVERY_REJECTED")
                            })?;
                            let turn_observation = turn.take().ok_or_else(|| {
                                known("LATTICE_MANAGED_PRESTART_RECOVERY_REJECTED")
                            })?;
                            let exact_turn = turn_id.ok_or_else(|| {
                                known("LATTICE_MANAGED_PRESTART_RECOVERY_REJECTED")
                            })?;
                            if thread_observation.thread_id() != thread_id
                                || turn_observation.turn_id() != Some(exact_turn.as_str())
                            {
                                return Err(known("LATTICE_MANAGED_PRESTART_RECOVERY_REJECTED"));
                            }
                            let terminal = ManagedTerminalCandidate::new(
                                ManagedWorkerObservation::prestart_terminal_failed(
                                    self.packet.attempt().into(),
                                    thread_id,
                                    exact_turn,
                                    generation,
                                    app_server_identity,
                                    evidence,
                                )?,
                            )?;
                            let (exit, lifecycle) = self.finalize_terminal()?;
                            if exit.terminal != WorkerTerminal::Failed || lifecycle.is_some() {
                                return Err(known("LATTICE_MANAGED_BRIDGE_TERMINAL_REJECTED"));
                            }
                            return Ok(ManagedWorkerPrestartRecovery::ExactFailedStart {
                                thread: thread_observation,
                                turn: Box::new(turn_observation),
                                terminal: Box::new(terminal),
                            });
                        }
                        _ => {
                            return Err(known("LATTICE_MANAGED_PRESTART_RECOVERY_REJECTED"));
                        }
                    }
                }
                _ => return Err(known("LATTICE_MANAGED_BRIDGE_OUTPUT_REJECTED")),
            }
        }
    }

    fn terminate_active_bridge_after_error(
        &mut self,
        error_code: &'static str,
    ) -> ManagedPortResult<()> {
        let mut bridge = self
            .active
            .take()
            .ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_NOT_ACTIVE"))?;
        bridge.stdin.take();
        bridge.prove_subtree_empty_and_join_reader(error_code)
    }

    fn finish_active_bridge_call<T>(
        &mut self,
        result: ManagedPortResult<T>,
    ) -> ManagedPortResult<T> {
        match result {
            Ok(value) => Ok(value),
            Err(failure) => {
                if self.active.is_some() {
                    self.terminate_active_bridge_after_error(
                        "LATTICE_MANAGED_PRESTART_RECOVERY_CLEANUP_AMBIGUOUS",
                    )?;
                }
                Err(failure)
            }
        }
    }

    fn cached_reconciliation(
        &self,
        thread_id: &str,
        turn_id: &str,
    ) -> ManagedPortResult<ManagedWorkerReconciliation> {
        let bridge = self
            .active
            .as_ref()
            .ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_NOT_ACTIVE"))?;
        if bridge.thread_id.as_deref() != Some(thread_id)
            || bridge.turn_id.as_deref() != Some(turn_id)
        {
            return Err(known("LATTICE_MANAGED_BRIDGE_IDENTITY_MISMATCH"));
        }
        let generation = bridge
            .app_server_generation
            .ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_GENERATION_REJECTED"))?;
        let digest = bridge
            .reconciliation_digest
            .clone()
            .ok_or_else(|| known("LATTICE_MANAGED_RECONCILIATION_REJECTED"))?;
        let app_server_identity = bridge
            .app_server_identity_digest
            .clone()
            .ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_APP_SERVER_IDENTITY_REJECTED"))?;
        ManagedWorkerObservation::reconciled(
            self.packet.attempt().into(),
            thread_id,
            turn_id,
            generation,
            app_server_identity,
            digest,
        )
        .map(ManagedWorkerReconciliation::ExactActive)
    }

    fn provider_lifecycle_evidence(
        &self,
        record: &Value,
        record_kind: &str,
        open_marker: Option<&VerifiedManagedEvidence>,
    ) -> ManagedPortResult<VerifiedManagedEvidence> {
        let payload_key = match record_kind {
            "provider_subtree_marker" => "marker",
            "provider_subtree_receipt" => "receipt",
            _ => {
                return Err(known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"));
            }
        };
        if !exact_value_keys(record, &["kind", payload_key])
            || record.get("kind").and_then(Value::as_str) != Some(record_kind)
        {
            return Err(known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"));
        }
        let payload = record
            .get(payload_key)
            .ok_or_else(|| known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"))?;
        let schema = value_str(payload, "schema")?;
        let bytes = serde_json::to_vec(payload)
            .map_err(|_| known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"))?;
        if bytes.len() > MAX_BRIDGE_LINE_BYTES {
            return Err(known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"));
        }
        let (project_id, producer_digest) = self
            .resource_evidence_identity
            .as_ref()
            .ok_or_else(|| known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"))?;
        let created_at = OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .map_err(|_| known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"))?;
        let task_ref = ContentDigest::from_sha256(self.packet.task_ref())
            .map_err(|_| known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"))?;
        let input = ManagedEvidenceInput::new(
            project_id.clone(),
            task_ref,
            self.packet.attempt(),
            ManagedEvidenceKind::WorkerLifecycle,
            "application/json",
            schema,
            "lattice-managed-codex-worker",
            env!("CARGO_PKG_VERSION"),
            producer_digest.clone(),
            created_at,
            bytes,
        )
        .map_err(|_| known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"))?;
        let evidence = VerifiedManagedEvidence::new(input)
            .map_err(|_| known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"))?;
        let descriptor = self
            .effect_identity
            .execution_environment_descriptor
            .as_deref()
            .ok_or_else(|| known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"))?;
        let preflight = self
            .effect_identity
            .execution_preflight_evidence
            .as_ref()
            .ok_or_else(|| known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"))?;
        validate_wsl2_provider_subtree_evidence(
            &self.packet,
            descriptor,
            preflight,
            open_marker,
            &evidence,
        )?;
        Ok(evidence)
    }

    fn resource_evidence(
        &self,
        event: &Value,
        thread_id: &str,
        turn_id: &str,
    ) -> ManagedPortResult<Option<VerifiedManagedEvidence>> {
        let Some((project_id, producer_digest)) = self.resource_evidence_identity.as_ref() else {
            return Ok(None);
        };
        let observed_at = event
            .get("observed_at")
            .and_then(Value::as_str)
            .ok_or_else(|| known("LATTICE_MANAGED_RESOURCE_OBSERVATION_REJECTED"))?;
        let parsed = OffsetDateTime::parse(observed_at, &Rfc3339)
            .map_err(|_| known("LATTICE_MANAGED_RESOURCE_OBSERVATION_REJECTED"))?;
        let created_at = parsed
            .format(&Rfc3339)
            .map_err(|_| known("LATTICE_MANAGED_RESOURCE_OBSERVATION_REJECTED"))?;
        let bounded = json!({
            "schema": "lattice.codex-resource-observation/1.0",
            "model_call_identity": managed_model_call_identity(
                self.packet.task_ref(),
                self.packet.attempt(),
                "worker",
                thread_id,
                turn_id,
            )?,
            "input_tokens": bounded_counter(event, "input_tokens")?,
            "cached_input_tokens": bounded_counter(event, "cached_input_tokens")?,
            "output_tokens": bounded_counter(event, "output_tokens")?,
            "reasoning_output_tokens": bounded_counter(event, "reasoning_output_tokens")?,
            "total_tokens": bounded_counter(event, "total_tokens")?,
            "model_context_window": bounded_counter(event, "model_context_window")?,
            "usage_scope": match event.get("usage_scope").and_then(Value::as_str) {
                Some("CUMULATIVE_INTERMEDIATE") => "CUMULATIVE_INTERMEDIATE",
                Some("CUMULATIVE_TERMINAL") => "CUMULATIVE_TERMINAL",
                _ => return Err(known("LATTICE_MANAGED_RESOURCE_OBSERVATION_REJECTED")),
            },
            "external_cost_status": "UNAVAILABLE",
            "event_evidence_digest": event
                .get("evidence_digest")
                .and_then(Value::as_str)
                .ok_or_else(|| known("LATTICE_MANAGED_RESOURCE_OBSERVATION_REJECTED"))?,
        });
        let bytes = serde_json::to_vec(&bounded)
            .map_err(|_| known("LATTICE_MANAGED_RESOURCE_OBSERVATION_REJECTED"))?;
        let task_ref = ContentDigest::from_sha256(self.packet.task_ref())
            .map_err(|_| known("LATTICE_MANAGED_RESOURCE_OBSERVATION_REJECTED"))?;
        let input = ManagedEvidenceInput::new(
            project_id.clone(),
            task_ref,
            self.packet.attempt(),
            ManagedEvidenceKind::ResourceObservation,
            "application/json",
            "lattice.codex-resource-observation/1.0",
            "lattice-managed-codex-worker",
            env!("CARGO_PKG_VERSION"),
            producer_digest.clone(),
            created_at,
            bytes,
        )
        .map_err(|_| known("LATTICE_MANAGED_RESOURCE_OBSERVATION_REJECTED"))?;
        VerifiedManagedEvidence::new(input)
            .map(Some)
            .map_err(|_| known("LATTICE_MANAGED_RESOURCE_OBSERVATION_REJECTED"))
    }
}

/// Returns a secret-free, role-separated identity for one exact Codex model
/// call. Worker and reviewer observations can therefore be accumulated even
/// when they belong to the same managed attempt.
pub(crate) fn managed_model_call_identity(
    task_ref: &str,
    attempt: u8,
    role: &str,
    thread_id: &str,
    turn_id: &str,
) -> ManagedPortResult<String> {
    if task_ref.is_empty()
        || attempt == 0
        || !matches!(role, "worker" | "reviewer")
        || thread_id.is_empty()
        || turn_id.is_empty()
    {
        return Err(known("LATTICE_MANAGED_MODEL_CALL_IDENTITY_REJECTED"));
    }
    let attempt = attempt.to_string();
    digest_parts(&[
        "managed-model-call-v1",
        task_ref,
        &attempt,
        role,
        thread_id,
        turn_id,
    ])
    .map(|digest| format!("model-call:sha256:{}", digest.as_str()))
}

fn bounded_counter(event: &Value, field: &str) -> ManagedPortResult<Option<u64>> {
    match event.get(field) {
        Some(Value::Null) | None => Ok(None),
        Some(value) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| known("LATTICE_MANAGED_RESOURCE_OBSERVATION_REJECTED")),
    }
}

fn configure_managed_codex_environment(
    command: &mut Command,
    codex_executable: Option<&Path>,
    codex_home: &Path,
    execution_environment_ref: &str,
    execution_environment_descriptor: Option<&str>,
    execution_preflight_receipt: Option<&str>,
    execution_preflight_descriptor_digest: Option<&str>,
    execution_preflight_content_digest: Option<&str>,
) -> ManagedPortResult<()> {
    command.env_clear();
    for key in ["SystemRoot", "WINDIR"] {
        if let Some(value) = env::var_os(key) {
            command.env(key, value);
        }
    }
    if let Some(descriptor) = execution_environment_descriptor {
        if descriptor.len() > 16_384 {
            return Err(known("LATTICE_MANAGED_EXECUTION_ENVIRONMENT_REJECTED"));
        }
        let value: Value = serde_json::from_str(&descriptor)
            .map_err(|_| known("LATTICE_MANAGED_EXECUTION_ENVIRONMENT_REJECTED"))?;
        let object = value
            .as_object()
            .ok_or_else(|| known("LATTICE_MANAGED_EXECUTION_ENVIRONMENT_REJECTED"))?;
        if object.get("schema").and_then(Value::as_str)
            != Some("lattice.execution-environment.wsl2-linux/1.1")
            || object.get("kind").and_then(Value::as_str) != Some("WSL2_LINUX")
            || object.get("identity_digest").and_then(Value::as_str)
                != Some(execution_environment_ref)
        {
            return Err(known("LATTICE_MANAGED_EXECUTION_ENVIRONMENT_REJECTED"));
        }
        let receipt = execution_preflight_receipt
            .ok_or_else(|| known("LATTICE_MANAGED_EXECUTION_ENVIRONMENT_REJECTED"))?;
        let preflight_descriptor_digest = execution_preflight_descriptor_digest
            .filter(|digest| valid_raw_sha256(digest))
            .ok_or_else(|| known("LATTICE_MANAGED_EXECUTION_ENVIRONMENT_REJECTED"))?;
        let preflight_content_digest = execution_preflight_content_digest
            .filter(|digest| valid_raw_sha256(digest))
            .ok_or_else(|| known("LATTICE_MANAGED_EXECUTION_ENVIRONMENT_REJECTED"))?;
        let descriptor_digest = sha256_hex(descriptor.as_bytes());
        if sha256_hex(receipt.as_bytes()) != preflight_content_digest {
            return Err(known("LATTICE_MANAGED_EXECUTION_ENVIRONMENT_REJECTED"));
        }
        let receipt_value: Value = serde_json::from_str(receipt)
            .map_err(|_| known("LATTICE_MANAGED_EXECUTION_ENVIRONMENT_REJECTED"))?;
        if receipt_value.get("schema").and_then(Value::as_str)
            != Some("lattice.wsl2-zero-model-preflight/1.0")
            || receipt_value
                .get("execution_environment_ref")
                .and_then(Value::as_str)
                != Some(execution_environment_ref)
            || receipt_value
                .get("provider_effect_count")
                .and_then(Value::as_u64)
                != Some(0)
        {
            return Err(known("LATTICE_MANAGED_EXECUTION_ENVIRONMENT_REJECTED"));
        }
        command
            .env("LATTICE_MANAGED_EXECUTION_ENVIRONMENT_JSON", descriptor)
            .env(
                "LATTICE_MANAGED_EXECUTION_ENVIRONMENT_DESCRIPTOR_DIGEST",
                descriptor_digest,
            )
            .env("LATTICE_MANAGED_EXECUTION_PREFLIGHT_JSON", receipt)
            .env(
                "LATTICE_MANAGED_EXECUTION_PREFLIGHT_DESCRIPTOR_DIGEST",
                preflight_descriptor_digest,
            )
            .env(
                "LATTICE_MANAGED_EXECUTION_PREFLIGHT_CONTENT_DIGEST",
                preflight_content_digest,
            );
    } else {
        if execution_preflight_receipt.is_some()
            || execution_preflight_descriptor_digest.is_some()
            || execution_preflight_content_digest.is_some()
            || execution_environment_ref
                != "execution-environment:sha256:0000000000000000000000000000000000000000000000000000000000000001"
        {
            return Err(known("LATTICE_MANAGED_EXECUTION_ENVIRONMENT_REJECTED"));
        }
        let codex_executable = non_verbatim_managed_child_path(
            codex_executable.ok_or_else(|| known("LATTICE_MANAGED_CODEX_IDENTITY_REJECTED"))?,
        )?;
        let codex_home = non_verbatim_managed_child_path(codex_home)?;
        for key in [
            "ComSpec",
            "PATHEXT",
            "PROCESSOR_ARCHITECTURE",
            "NUMBER_OF_PROCESSORS",
            "TEMP",
            "TMP",
            "LANG",
            "LC_ALL",
        ] {
            if let Some(value) = env::var_os(key) {
                command.env(key, value);
            }
        }
        command
            .env(
                "PATH",
                managed_shell_path().map_err(|_| known("LATTICE_MANAGED_SHELL_PATH_REJECTED"))?,
            )
            .env("LATTICE_CODEX_BIN", &codex_executable)
            .env("LATTICE_DELIVERY_CODEX_HOME", &codex_home)
            .env("CODEX_HOME", &codex_home)
            .env("HOME", &codex_home)
            .env("USERPROFILE", &codex_home)
            .env("APPDATA", &codex_home)
            .env("LOCALAPPDATA", &codex_home);
    }
    if execution_environment_descriptor.is_none()
        && env::var("LATTICE_DELIVERY_CODEX_MODE").as_deref() == Ok("SCRIPTED_ACCEPTANCE")
    {
        command.env("LATTICE_DELIVERY_CODEX_MODE", "SCRIPTED_ACCEPTANCE");
    }
    #[cfg(windows)]
    if execution_environment_descriptor.is_none()
        && let Some(system_root) = env::var_os("SystemRoot").or_else(|| env::var_os("WINDIR"))
    {
        command.env(
            "PSModulePath",
            PathBuf::from(system_root).join("System32/WindowsPowerShell/v1.0/Modules"),
        );
    }
    #[cfg(windows)]
    if execution_environment_descriptor.is_none() {
        command.env(
            "PSModuleAnalysisCachePath",
            codex_home.join("powershell-module-analysis-cache"),
        );
    }
    Ok(())
}

fn non_verbatim_managed_child_path(path: &Path) -> ManagedPortResult<PathBuf> {
    #[cfg(windows)]
    {
        let text = path
            .to_str()
            .ok_or_else(|| known("LATTICE_MANAGED_CHILD_PATH_REJECTED"))?;
        if let Some(non_verbatim) = text.strip_prefix(r"\\?\") {
            if non_verbatim.starts_with("UNC\\") {
                return Err(known("LATTICE_MANAGED_CHILD_PATH_REJECTED"));
            }
            return Ok(PathBuf::from(non_verbatim));
        }
    }
    Ok(path.to_path_buf())
}

fn retain_control_channel(operation: &str) -> bool {
    matches!(operation, "start" | "continue-turn" | "resume")
}

impl ManagedCodexWorkerPort for ManagedCodexWorkerAdapter {
    fn model_availability(
        &mut self,
        selection: &ModelSelection,
    ) -> ManagedPortResult<ManagedModelAvailability> {
        if selection != self.packet.model_selection() {
            return Err(known("LATTICE_MANAGED_MODEL_SELECTION_MISMATCH"));
        }
        self.run_probe()
    }

    fn prepare_provider_dispatch(
        &mut self,
        attempt: &VerifiedWorkerAttemptRecord,
        packet: &AttemptPacketIdentity,
    ) -> ManagedPortResult<VerifiedManagedEvidence> {
        self.ensure_packet(attempt, Some(packet))?;
        self.effect_identity.verify()?;
        if self
            .effect_identity
            .execution_environment_descriptor
            .is_none()
        {
            return Err(known("LATTICE_MANAGED_PROVIDER_SUBTREE_WSL2_REQUIRED"));
        }
        if self.active.is_some() {
            return Err(known("LATTICE_MANAGED_DUPLICATE_DISPATCH_REJECTED"));
        }
        self.reject_if_cancelled_prestart()?;
        let command = self.initial_command("start", None, Some(attempt.claimed_at()))?;
        self.active = Some(self.spawn(&command, retain_control_channel("start"))?);
        let record = self.next_bridge_record()?;
        let evidence =
            self.provider_lifecycle_evidence(&record, "provider_subtree_marker", None)?;
        let active = self
            .active
            .as_mut()
            .ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_NOT_ACTIVE"))?;
        if active
            .provider_open_evidence
            .replace(evidence.clone())
            .is_some()
        {
            return Err(known("LATTICE_MANAGED_PROVIDER_SUBTREE_EVIDENCE_REJECTED"));
        }
        Ok(evidence)
    }

    fn start_thread(
        &mut self,
        attempt: &VerifiedWorkerAttemptRecord,
        packet: &AttemptPacketIdentity,
    ) -> ManagedPortResult<ManagedWorkerObservation> {
        self.ensure_packet(attempt, Some(packet))?;
        self.require_preclaim_auth_readiness()?;
        self.reject_if_cancelled_prestart()?;
        let result = (|| {
            if self
                .effect_identity
                .execution_environment_descriptor
                .is_some()
            {
                let open = self
                    .active
                    .as_ref()
                    .and_then(|bridge| bridge.provider_open_evidence.as_ref())
                    .ok_or_else(|| known("LATTICE_MANAGED_PROVIDER_SUBTREE_OPEN_REQUIRED"))?;
                let parsed = validate_wsl2_provider_subtree_evidence(
                    &self.packet,
                    self.effect_identity
                        .execution_environment_descriptor
                        .as_deref()
                        .ok_or_else(|| known("LATTICE_MANAGED_PROVIDER_SUBTREE_OPEN_REQUIRED"))?,
                    self.effect_identity
                        .execution_preflight_evidence
                        .as_ref()
                        .ok_or_else(|| known("LATTICE_MANAGED_PROVIDER_SUBTREE_OPEN_REQUIRED"))?,
                    None,
                    open,
                )?;
                let control = exact_provider_dispatch_authorization_control(
                    self.packet.task_ref(),
                    self.packet.attempt(),
                    self.packet.digest(),
                    parsed.closure_digest(),
                )?;
                self.send_provider_dispatch_authorization(&control)?;
            } else {
                if self.active.is_some() {
                    return Err(known("LATTICE_MANAGED_DUPLICATE_DISPATCH_REJECTED"));
                }
                let command = self.initial_command("start", None, Some(attempt.claimed_at()))?;
                self.active = Some(self.spawn(&command, retain_control_channel("start"))?);
            }
            let event = self.next_matching_event(&["THREAD_START_ACCEPTED"])?;
            let (thread, _, generation, app_server_identity, evidence) =
                self.exact_event_identity(&event, None, None)?;
            ManagedWorkerObservation::thread_accepted(
                attempt.attempt_number(),
                thread,
                generation,
                app_server_identity,
                evidence,
            )
        })();
        self.finish_active_bridge_call(result)
    }

    fn start_turn(
        &mut self,
        attempt: &VerifiedWorkerAttemptRecord,
        thread_id: &str,
    ) -> ManagedPortResult<ManagedWorkerObservation> {
        let result = (|| {
            self.ensure_packet(attempt, None)?;
            self.reject_if_cancelled_prestart()?;
            if self.active.is_none() {
                let retained = self.retained_empty_thread(thread_id)?;
                let command = self.initial_command("continue-turn", Some(retained), None)?;
                self.active = Some(self.spawn(&command, retain_control_channel("continue-turn"))?);
                let event = self.next_matching_event(&["THREAD_RECONCILED_EMPTY"])?;
                let (_thread, turn, _generation, _app_server_identity, _evidence) =
                    self.exact_event_identity(&event, Some(thread_id), None)?;
                if turn.is_some() {
                    return Err(known("LATTICE_MANAGED_BRIDGE_IDENTITY_MISMATCH"));
                }
            }
            if self
                .active
                .as_ref()
                .and_then(|bridge| bridge.thread_id.as_deref())
                != Some(thread_id)
            {
                return Err(known("LATTICE_MANAGED_BRIDGE_IDENTITY_MISMATCH"));
            }
            self.reject_if_cancelled_prestart()?;
            let authorization = self.authorize_turn_start_control(thread_id)?;
            self.send_turn_start_authorization(&authorization)?;
            let event = self.next_matching_event(&["TURN_START_ACCEPTED"])?;
            let (thread, turn, generation, app_server_identity, evidence) =
                self.exact_event_identity(&event, Some(thread_id), None)?;
            ManagedWorkerObservation::turn_accepted(
                attempt.attempt_number(),
                thread,
                turn.ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_IDENTITY_MISMATCH"))?,
                generation,
                app_server_identity,
                evidence,
            )
        })();
        self.finish_active_bridge_call(result)
    }

    fn wait_exact_started(
        &mut self,
        attempt: &VerifiedWorkerAttemptRecord,
        thread_id: &str,
        turn_id: &str,
    ) -> ManagedPortResult<ManagedWorkerObservation> {
        let result = (|| {
            self.ensure_packet(attempt, None)?;
            self.reject_if_cancelled_prestart()?;
            let event = self.next_matching_event(&["TURN_STARTED"])?;
            if event.get("status").and_then(Value::as_str) != Some("inProgress") {
                return Err(known("LATTICE_MANAGED_EXACT_START_REJECTED"));
            }
            let observed_at = event
                .get("observed_at")
                .and_then(Value::as_str)
                .ok_or_else(|| known("LATTICE_MANAGED_EXACT_START_REJECTED"))?;
            let attempt_deadline_at = event
                .get("attempt_deadline_at")
                .and_then(Value::as_str)
                .ok_or_else(|| known("LATTICE_MANAGED_EXACT_START_REJECTED"))?;
            validate_execution_window(
                observed_at,
                attempt_deadline_at,
                self.packet.max_duration_seconds(),
                self.packet.deadline_at(),
            )?;
            let (thread, turn, generation, app_server_identity, evidence) =
                self.exact_event_identity(&event, Some(thread_id), Some(turn_id))?;
            let observation = ManagedWorkerObservation::exact_started(
                attempt.attempt_number(),
                thread,
                turn.ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_IDENTITY_MISMATCH"))?,
                generation,
                app_server_identity,
                observed_at,
                evidence,
            )?;
            self.retained_attempt_started_at = Some(observed_at.to_owned());
            self.retained_attempt_deadline_at = Some(attempt_deadline_at.to_owned());
            self.retained_last_heartbeat_at = Some(observed_at.to_owned());
            self.retained_last_meaningful_progress_at = Some(observed_at.to_owned());
            self.arm_execution_watchdog(attempt_deadline_at)?;
            Ok(observation)
        })();
        self.finish_active_bridge_call(result)
    }

    fn recover_claimed_dispatch(
        &mut self,
        attempt: &VerifiedWorkerAttemptRecord,
        packet: &AttemptPacketIdentity,
    ) -> ManagedPortResult<ManagedWorkerPrestartRecovery> {
        self.ensure_packet(attempt, Some(packet))?;
        if self.active.is_some() {
            return Err(known("LATTICE_MANAGED_DUPLICATE_RECONCILIATION_REJECTED"));
        }
        self.reject_if_cancelled_prestart()?;
        let command = self.initial_command("recover-dispatch", None, Some(attempt.claimed_at()))?;
        self.active = Some(self.spawn(&command, retain_control_channel("recover-dispatch"))?);
        self.collect_prestart_recovery(None, None)
    }

    fn recover_prestart(
        &mut self,
        attempt: &VerifiedWorkerAttemptRecord,
        thread_id: &str,
        turn_id: Option<&str>,
    ) -> ManagedPortResult<ManagedWorkerPrestartRecovery> {
        self.ensure_packet(attempt, None)?;
        if self.active.is_some() {
            return Err(known("LATTICE_MANAGED_DUPLICATE_RECONCILIATION_REJECTED"));
        }
        self.reject_if_cancelled_prestart()?;
        let retained = self.retained_prestart(thread_id, turn_id)?;
        let command = self.initial_command("recover-prestart", Some(retained), None)?;
        self.active = Some(self.spawn(&command, retain_control_channel("recover-prestart"))?);
        self.collect_prestart_recovery(Some(thread_id), turn_id)
    }

    fn next_execution_event(
        &mut self,
        attempt: &VerifiedWorkerAttemptRecord,
        thread_id: &str,
        turn_id: &str,
    ) -> ManagedPortResult<ManagedWorkerExecutionEvent> {
        self.ensure_packet(attempt, None)?;
        if let Some(candidate) = self.pending_terminal_candidate.take() {
            if candidate.observation().thread_id() != thread_id
                || candidate.observation().turn_id() != Some(turn_id)
            {
                return Err(known("LATTICE_MANAGED_BRIDGE_TERMINAL_REJECTED"));
            }
            return Ok(ManagedWorkerExecutionEvent::Terminal(candidate));
        }
        if self.execution_hard_deadline.is_none() {
            let deadline = self
                .retained_attempt_deadline_at
                .clone()
                .ok_or_else(|| known("LATTICE_MANAGED_RETAINED_EXECUTION_WINDOW_REJECTED"))?;
            self.arm_execution_watchdog(&deadline)?;
        }
        loop {
            if self.cancellation.is_requested()
                && self.shutdown_interrupt_sent
                && self.shutdown_deadline.is_none()
            {
                self.shutdown_deadline = Some(
                    Instant::now()
                        .checked_add(BRIDGE_TEARDOWN_GRACE)
                        .unwrap_or_else(Instant::now),
                );
            }
            if self.cancellation.is_requested()
                && self.shutdown_interrupt_sent
                && self.shutdown_deadline_expired()
            {
                self.terminate_without_shutdown_receipt()?;
                return Err(ambiguous(MANAGED_GRACEFUL_SHUTDOWN_RECEIPT_REQUIRED));
            }
            if self.cancellation.is_requested() && !self.shutdown_interrupt_sent {
                let observation = ManagedCodexWorkerPort::interrupt_exact_turn(
                    self, attempt, thread_id, turn_id,
                )?;
                return Ok(ManagedWorkerExecutionEvent::Observation(observation));
            }
            let Some(watchdog_remaining) = self.execution_watchdog_remaining()? else {
                return self.fail_silent_execution_bridge();
            };
            let poll = CANCELLATION_POLL.min(watchdog_remaining);
            let Some(event) = self.next_matching_event_timeout(
                &[
                    "RESOURCE_OBSERVATION",
                    "MEANINGFUL_PROGRESS",
                    "HEARTBEAT",
                    "RECONCILE_STARTED",
                    "RECONCILED_ACTIVE",
                    "STALL_CLASSIFIED",
                    "INTERRUPT_REQUESTED",
                    "TURN_TERMINAL",
                    "RECONCILED_TERMINAL",
                    "INTERRUPT_TERMINAL",
                ],
                poll,
            )?
            else {
                continue;
            };
            self.execution_last_activity = Some(Instant::now());
            let event_type = event
                .get("event_type")
                .and_then(Value::as_str)
                .ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_OUTPUT_REJECTED"))?;
            let (thread, turn, generation, app_server_identity, evidence) =
                self.exact_event_identity(&event, Some(thread_id), Some(turn_id))?;
            let turn = turn.ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_IDENTITY_MISMATCH"))?;
            match event_type {
                "RESOURCE_OBSERVATION" => {
                    let resource = self
                        .resource_evidence(&event, &thread, &turn)?
                        .ok_or_else(|| known("LATTICE_MANAGED_RESOURCE_EVIDENCE_REQUIRED"))?;
                    let observation = ManagedWorkerObservation::meaningful_progress(
                        attempt.attempt_number(),
                        thread,
                        turn,
                        generation,
                        app_server_identity,
                        evidence,
                    )?;
                    return Ok(ManagedWorkerExecutionEvent::ResourceObservation {
                        observation,
                        evidence: Box::new(resource),
                    });
                }
                "MEANINGFUL_PROGRESS" => {
                    if !matches!(
                        event.get("progress_kind").and_then(Value::as_str),
                        Some(
                            "ITEM_STARTED"
                                | "ITEM_COMPLETED"
                                | "COMMAND_EXECUTION_PROGRESS"
                                | "TURN_DIFF_UPDATED"
                                | "TURN_PLAN_UPDATED"
                        )
                    ) {
                        return Err(known("LATTICE_MANAGED_PROGRESS_OBSERVATION_REJECTED"));
                    }
                    return ManagedWorkerObservation::meaningful_progress(
                        attempt.attempt_number(),
                        thread,
                        turn,
                        generation,
                        app_server_identity,
                        evidence,
                    )
                    .map(ManagedWorkerExecutionEvent::Observation);
                }
                "HEARTBEAT" => {
                    return ManagedWorkerObservation::heartbeat(
                        attempt.attempt_number(),
                        thread,
                        turn,
                        generation,
                        app_server_identity,
                        evidence,
                    )
                    .map(ManagedWorkerExecutionEvent::Observation);
                }
                "RECONCILE_STARTED" => {}
                "RECONCILED_ACTIVE" => {
                    return ManagedWorkerObservation::reconciled(
                        attempt.attempt_number(),
                        thread,
                        turn,
                        generation,
                        app_server_identity,
                        evidence,
                    )
                    .map(ManagedWorkerExecutionEvent::Observation);
                }
                "STALL_CLASSIFIED" => {
                    self.closed_blocker_code =
                        match event.get("stall_reason").and_then(Value::as_str) {
                            Some("HEARTBEAT_TIMEOUT_ACTIVE_TURN") => {
                                Some("LATTICE_MANAGED_HEARTBEAT_TIMEOUT_WHILE_IN_PROGRESS")
                            }
                            Some("DEADLINE_EXCEEDED") => Some("LATTICE_MANAGED_DEADLINE_EXCEEDED"),
                            Some("TOKEN_BUDGET_EXCEEDED") => {
                                Some("LATTICE_MANAGED_TOKEN_BUDGET_EXHAUSTED")
                            }
                            _ => {
                                return Err(known("LATTICE_MANAGED_STALL_REASON_REJECTED"));
                            }
                        };
                    return ManagedWorkerObservation::stall_classified(
                        attempt.attempt_number(),
                        thread,
                        turn,
                        generation,
                        app_server_identity,
                        evidence,
                    )
                    .map(ManagedWorkerExecutionEvent::Observation);
                }
                "INTERRUPT_REQUESTED" => {
                    return ManagedWorkerObservation::interrupt_requested(
                        attempt.attempt_number(),
                        thread,
                        turn,
                        generation,
                        app_server_identity,
                        evidence,
                    )
                    .map(ManagedWorkerExecutionEvent::Observation);
                }
                "TURN_TERMINAL" | "RECONCILED_TERMINAL" | "INTERRUPT_TERMINAL" => {
                    let terminal = parse_terminal(&event)?;
                    let shutdown_identity = self
                        .shutdown_interrupt_sent
                        .then(|| (thread.clone(), turn.clone(), evidence.clone()));
                    let observation = ManagedWorkerObservation::terminal(
                        attempt.attempt_number(),
                        thread,
                        turn,
                        terminal,
                        generation,
                        app_server_identity,
                        evidence,
                    )?;
                    let (exit, lifecycle) = self.finalize_terminal()?;
                    if exit.terminal != terminal {
                        return Err(known("LATTICE_MANAGED_BRIDGE_TERMINAL_REJECTED"));
                    }
                    if let Some((thread_id, turn_id, terminal_evidence_digest)) = shutdown_identity
                    {
                        if !matches!(
                            terminal,
                            WorkerTerminal::Interrupted | WorkerTerminal::Failed
                        ) {
                            return Err(known("LATTICE_MANAGED_SHUTDOWN_TERMINAL_REJECTED"));
                        }
                        self.cancellation
                            .record_exact_receipt(ManagedWorkerShutdownReceipt {
                                task_ref: self.packet.task_ref().to_owned(),
                                attempt: self.packet.attempt(),
                                thread_id,
                                turn_id,
                                terminal,
                                terminal_evidence_digest,
                            })?;
                    }
                    let candidate = ManagedTerminalCandidate::new(observation)?;
                    if let Some(lifecycle) = lifecycle {
                        if self.pending_terminal_candidate.replace(candidate).is_some() {
                            return Err(known("LATTICE_MANAGED_BRIDGE_TERMINAL_REJECTED"));
                        }
                        return Ok(ManagedWorkerExecutionEvent::LifecycleEvidence(lifecycle));
                    }
                    return Ok(ManagedWorkerExecutionEvent::Terminal(candidate));
                }
                _ => return Err(known("LATTICE_MANAGED_BRIDGE_OUTPUT_REJECTED")),
            }
        }
    }

    fn read_exact_thread(
        &mut self,
        attempt: &VerifiedWorkerAttemptRecord,
        thread_id: &str,
    ) -> ManagedPortResult<ManagedWorkerReconciliation> {
        self.ensure_packet(attempt, None)?;
        if self.active.is_some() {
            let retained_turn = self
                .active
                .as_ref()
                .and_then(|bridge| bridge.turn_id.as_deref())
                .ok_or_else(|| known("LATTICE_MANAGED_RECONCILIATION_REJECTED"))?;
            return self.cached_reconciliation(thread_id, retained_turn);
        }
        if let Some(turn_id) = self.retained_turn_id.clone() {
            if self.pending_terminal_reconciliation.is_some() {
                return self.read_exact_turn(attempt, thread_id, &turn_id);
            }
            return match self.read_exact_turn(attempt, thread_id, &turn_id)? {
                ManagedWorkerReconciliation::ExactTerminal(candidate) => {
                    // A fresh App Server owns a new session identity.  Make
                    // that exact-terminal reconciliation durable before the
                    // terminal observation is replayed on the next ordered
                    // read; otherwise PostgreSQL correctly rejects identity
                    // drift and the retained attempt can never reach retry.
                    let (boundary, pending) = stage_retained_terminal_reconciliation(
                        attempt.attempt_number(),
                        candidate,
                    )?;
                    self.pending_terminal_reconciliation = Some(pending);
                    Ok(ManagedWorkerReconciliation::ExactActive(boundary))
                }
                reconciliation => Ok(reconciliation),
            };
        }
        match self.recover_prestart(attempt, thread_id, None)? {
            ManagedWorkerPrestartRecovery::ExactFailedStart { terminal, .. } => {
                Ok(ManagedWorkerReconciliation::ExactTerminal(*terminal))
            }
            ManagedWorkerPrestartRecovery::ProvenNoProviderCandidate
            | ManagedWorkerPrestartRecovery::ExactEmptyThread { .. }
            | ManagedWorkerPrestartRecovery::ReconciliationRequired => {
                Ok(ManagedWorkerReconciliation::Unresolved)
            }
        }
    }

    fn read_exact_turn(
        &mut self,
        attempt: &VerifiedWorkerAttemptRecord,
        thread_id: &str,
        turn_id: &str,
    ) -> ManagedPortResult<ManagedWorkerReconciliation> {
        self.ensure_packet(attempt, None)?;
        if let Some(candidate) = self.pending_terminal_reconciliation.take() {
            let observation = candidate.observation();
            if observation.thread_id() != thread_id || observation.turn_id() != Some(turn_id) {
                return Err(known("LATTICE_MANAGED_BRIDGE_IDENTITY_MISMATCH"));
            }
            return Ok(ManagedWorkerReconciliation::ExactTerminal(candidate));
        }
        if self.retained_attempt_started_at.is_none() {
            return match self.recover_prestart(attempt, thread_id, Some(turn_id))? {
                ManagedWorkerPrestartRecovery::ExactFailedStart { terminal, .. } => {
                    Ok(ManagedWorkerReconciliation::ExactTerminal(*terminal))
                }
                ManagedWorkerPrestartRecovery::ProvenNoProviderCandidate
                | ManagedWorkerPrestartRecovery::ExactEmptyThread { .. }
                | ManagedWorkerPrestartRecovery::ReconciliationRequired => {
                    Ok(ManagedWorkerReconciliation::Unresolved)
                }
            };
        }
        if self.active.is_none() {
            let retained = self.retained(thread_id, turn_id)?;
            let command = self.initial_command("resume", Some(retained), None)?;
            self.active = Some(self.spawn(&command, retain_control_channel("resume"))?);
            let mut progress = Vec::new();
            let mut resources = Vec::new();
            loop {
                let event = self.next_matching_event(&[
                    "RESOURCE_OBSERVATION",
                    "RECONCILED_ACTIVE",
                    "RECONCILED_TERMINAL",
                ])?;
                let event_type = event
                    .get("event_type")
                    .and_then(Value::as_str)
                    .ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_OUTPUT_REJECTED"))?;
                let (thread, turn, generation, app_server_identity, evidence) =
                    self.exact_event_identity(&event, Some(thread_id), Some(turn_id))?;
                let exact_turn = turn
                    .clone()
                    .ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_IDENTITY_MISMATCH"))?;
                if event_type == "RESOURCE_OBSERVATION" {
                    let resource = self
                        .resource_evidence(&event, &thread, &exact_turn)?
                        .ok_or_else(|| known("LATTICE_MANAGED_RESOURCE_EVIDENCE_REQUIRED"))?;
                    progress.push(ManagedWorkerObservation::meaningful_progress(
                        attempt.attempt_number(),
                        thread,
                        exact_turn,
                        generation,
                        app_server_identity.clone(),
                        evidence,
                    )?);
                    resources.push(resource);
                    continue;
                }
                if event_type == "RECONCILED_TERMINAL" {
                    let terminal = parse_terminal(&event)?;
                    let observation = ManagedWorkerObservation::terminal(
                        attempt.attempt_number(),
                        thread,
                        exact_turn,
                        terminal,
                        generation,
                        app_server_identity.clone(),
                        evidence,
                    )?;
                    let (exit, lifecycle) = self.finalize_terminal()?;
                    if exit.terminal != terminal || lifecycle.is_some() {
                        return Err(known("LATTICE_MANAGED_BRIDGE_TERMINAL_REJECTED"));
                    }
                    return ManagedTerminalCandidate::new(observation)
                        .map(|candidate| candidate.with_intermediate(progress, resources))
                        .map(ManagedWorkerReconciliation::ExactTerminal);
                }
                if let Some(active) = self.active.as_mut() {
                    active.reconciliation_digest = Some(evidence.clone());
                }
                return ManagedWorkerObservation::reconciled(
                    attempt.attempt_number(),
                    thread,
                    exact_turn,
                    generation,
                    app_server_identity,
                    evidence,
                )
                .map(ManagedWorkerReconciliation::ExactActive);
            }
        }
        self.cached_reconciliation(thread_id, turn_id)
    }

    fn resume_exact_turn(
        &mut self,
        attempt: &VerifiedWorkerAttemptRecord,
        thread_id: &str,
        turn_id: &str,
    ) -> ManagedPortResult<ManagedWorkerReconciliation> {
        self.ensure_packet(attempt, None)?;
        self.cached_reconciliation(thread_id, turn_id)
    }

    fn reconcile_exact_turn(
        &mut self,
        attempt: &VerifiedWorkerAttemptRecord,
        thread_id: &str,
        turn_id: &str,
    ) -> ManagedPortResult<ManagedWorkerReconciliation> {
        self.ensure_packet(attempt, None)?;
        self.cached_reconciliation(thread_id, turn_id)
    }

    fn interrupt_exact_turn(
        &mut self,
        attempt: &VerifiedWorkerAttemptRecord,
        thread_id: &str,
        turn_id: &str,
    ) -> ManagedPortResult<ManagedWorkerObservation> {
        self.ensure_packet(attempt, None)?;
        let control = json!({
            "schema": CONTROL_SCHEMA,
            "operation": "interrupt",
            "task_ref": self.packet.task_ref(),
            "attempt": self.packet.attempt(),
            "packet_digest": self.packet.digest(),
            "thread_id": thread_id,
            "turn_id": turn_id,
        });
        self.send_control(&control)?;
        if self.cancellation.is_requested() {
            self.shutdown_deadline.get_or_insert_with(|| {
                Instant::now()
                    .checked_add(BRIDGE_TEARDOWN_GRACE)
                    .unwrap_or_else(Instant::now)
            });
        }
        let event = self.next_matching_event(&["INTERRUPT_REQUESTED"])?;
        let (thread, turn, generation, app_server_identity, evidence) =
            self.exact_event_identity(&event, Some(thread_id), Some(turn_id))?;
        let observation = ManagedWorkerObservation::interrupt_requested(
            attempt.attempt_number(),
            thread,
            turn.ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_IDENTITY_MISMATCH"))?,
            generation,
            app_server_identity,
            evidence,
        )?;
        if self.cancellation.is_requested() {
            self.shutdown_interrupt_sent = true;
            // The request acknowledgement must be made durable before the
            // terminal receipt is consumed. Give that second, independently
            // bounded wait its own grace window; the scheduler still owns the
            // stricter aggregate shutdown deadline.
            self.shutdown_deadline = None;
        }
        Ok(observation)
    }

    fn reconcile_terminal_usage(
        &mut self,
        attempt: &VerifiedWorkerAttemptRecord,
        thread_id: &str,
        turn_id: &str,
    ) -> ManagedPortResult<Option<VerifiedManagedEvidence>> {
        self.ensure_packet(attempt, None)?;
        if self.active.is_some() {
            return Err(known("LATTICE_MANAGED_DUPLICATE_RECONCILIATION_REJECTED"));
        }
        let retained = self.retained(thread_id, turn_id)?;
        let command = self.initial_command("resume", Some(retained), None)?;
        self.active = Some(self.spawn(&command, retain_control_channel("resume"))?);
        let mut terminal_resource = None;
        loop {
            let event = self.next_matching_event(&[
                "RESOURCE_OBSERVATION",
                "RECONCILED_ACTIVE",
                "RECONCILED_TERMINAL",
            ])?;
            let event_type = event
                .get("event_type")
                .and_then(Value::as_str)
                .ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_OUTPUT_REJECTED"))?;
            let (thread, turn, _generation, _app_server_identity, _evidence) =
                self.exact_event_identity(&event, Some(thread_id), Some(turn_id))?;
            let turn = turn.ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_IDENTITY_MISMATCH"))?;
            match event_type {
                "RESOURCE_OBSERVATION" => {
                    if event.get("usage_scope").and_then(Value::as_str)
                        != Some("CUMULATIVE_TERMINAL")
                    {
                        continue;
                    }
                    let resource = self.resource_evidence(&event, &thread, &turn)?;
                    terminal_resource = bounded_counter(&event, "total_tokens")?
                        .is_some()
                        .then_some(resource)
                        .flatten();
                }
                "RECONCILED_TERMINAL" => {
                    let terminal = parse_terminal(&event)?;
                    let (exit, lifecycle) = self.finalize_terminal()?;
                    if exit.terminal != terminal || lifecycle.is_some() {
                        return Err(known("LATTICE_MANAGED_BRIDGE_TERMINAL_REJECTED"));
                    }
                    return Ok(terminal_resource);
                }
                "RECONCILED_ACTIVE" => {
                    // This path is read-only. Suppress Drop's exact interrupt;
                    // a durable-terminal/provider-active contradiction must be
                    // surfaced for human reconciliation, never mutated here.
                    if let Some(active) = self.active.as_mut() {
                        active.exact_active = false;
                    }
                    return Err(ManagedPortError::new(
                        ManagedPortErrorKind::ReconcileRequired,
                        "LATTICE_MANAGED_MODEL_USAGE_RECONCILIATION_REQUIRED",
                    ));
                }
                _ => return Err(known("LATTICE_MANAGED_BRIDGE_OUTPUT_REJECTED")),
            }
        }
    }
}

fn read_record(reader: &mut BufReader<Box<dyn Read + Send>>) -> ManagedPortResult<Value> {
    let mut line = Vec::with_capacity(MAX_BRIDGE_LINE_BYTES.min(1_024));
    loop {
        let available = reader
            .fill_buf()
            .map_err(|_| ambiguous("LATTICE_MANAGED_BRIDGE_READ_AMBIGUOUS"))?;
        if available.is_empty() {
            if line.is_empty() {
                return Err(ambiguous("LATTICE_MANAGED_PROCESS_EXIT_WITHOUT_TERMINAL"));
            }
            break;
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(available.len(), |index| index.saturating_add(1));
        if line
            .len()
            .checked_add(consumed)
            .is_none_or(|total| total > MAX_BRIDGE_LINE_BYTES)
        {
            return Err(known("LATTICE_MANAGED_BRIDGE_OUTPUT_REJECTED"));
        }
        line.extend_from_slice(&available[..consumed]);
        reader.consume(consumed);
        if newline.is_some() {
            break;
        }
    }
    serde_json::from_slice(&line).map_err(|_| known("LATTICE_MANAGED_BRIDGE_OUTPUT_REJECTED"))
}

struct WorkerReaderActivity;

impl WorkerReaderActivity {
    fn new() -> Self {
        #[cfg(test)]
        ACTIVE_WORKER_READERS.fetch_add(1, Ordering::AcqRel);
        Self
    }
}

impl Drop for WorkerReaderActivity {
    fn drop(&mut self) {
        #[cfg(test)]
        ACTIVE_WORKER_READERS.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
static ACTIVE_WORKER_READERS: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Debug, Eq, PartialEq)]
struct ManagedCodexAuthReadiness {
    app_server_generation: u64,
    app_server_session_id: String,
    app_server_identity_digest: ContentDigest,
    codex_home_digest: String,
    config_digest: String,
}

pub(crate) fn managed_app_server_identity_digest(
    app_server_session_id: &str,
    codex_home_digest: &str,
    config_digest: &str,
    expected_codex_home_digest: &str,
    expected_config_digest: &str,
) -> ManagedPortResult<ContentDigest> {
    if !app_server_session_id
        .strip_prefix("app-server-session:sha256:")
        .is_some_and(|value| {
            value.len() == 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        })
        || codex_home_digest != expected_codex_home_digest
        || config_digest != expected_config_digest
    {
        return Err(known("LATTICE_MANAGED_BRIDGE_APP_SERVER_IDENTITY_REJECTED"));
    }
    let mut hasher = Sha256::new();
    hasher.update(b"LATTICE_MANAGED_APP_SERVER_IDENTITY_V1\0");
    for part in [app_server_session_id, codex_home_digest, config_digest] {
        if part.contains('\0') {
            return Err(known("LATTICE_MANAGED_BRIDGE_APP_SERVER_IDENTITY_REJECTED"));
        }
        hasher.update(part.as_bytes());
        hasher.update(b"\0");
    }
    let mut encoded = String::with_capacity(64);
    for byte in hasher.finalize() {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}")
            .map_err(|_| known("LATTICE_MANAGED_BRIDGE_APP_SERVER_IDENTITY_REJECTED"))?;
    }
    ContentDigest::from_sha256(encoded)
        .map_err(|_| known("LATTICE_MANAGED_BRIDGE_APP_SERVER_IDENTITY_REJECTED"))
}

fn parse_managed_auth_readiness(
    value: &Value,
    expected_codex_home_digest: &str,
    expected_config_digest: &str,
) -> ManagedPortResult<ManagedCodexAuthReadiness> {
    const KEYS: [&str; 7] = [
        "schema",
        "ready",
        "auth_mode",
        "app_server_generation",
        "app_server_session_id",
        "codex_home_digest",
        "config_digest",
    ];
    let object = value
        .as_object()
        .filter(|object| object.len() == KEYS.len())
        .filter(|object| KEYS.iter().all(|key| object.contains_key(*key)))
        .ok_or_else(|| known("CREDENTIAL_READ_ISOLATION_NOT_VERIFIED"))?;
    let app_server_generation = object
        .get("app_server_generation")
        .and_then(Value::as_u64)
        .filter(|generation| *generation > 0)
        .ok_or_else(|| known("CREDENTIAL_READ_ISOLATION_NOT_VERIFIED"))?;
    let app_server_session_id = object
        .get("app_server_session_id")
        .and_then(Value::as_str)
        .ok_or_else(|| known("CREDENTIAL_READ_ISOLATION_NOT_VERIFIED"))?;
    if object.get("schema").and_then(Value::as_str)
        != Some("lattice.managed-codex-auth-readiness/1.0")
        || object.get("ready").and_then(Value::as_bool) != Some(true)
        || object.get("auth_mode").and_then(Value::as_str) != Some("chatgpt")
        || object.get("codex_home_digest").and_then(Value::as_str)
            != Some(expected_codex_home_digest)
        || object.get("config_digest").and_then(Value::as_str) != Some(expected_config_digest)
    {
        return Err(known("CREDENTIAL_READ_ISOLATION_NOT_VERIFIED"));
    }
    let app_server_identity_digest = managed_app_server_identity_digest(
        app_server_session_id,
        expected_codex_home_digest,
        expected_config_digest,
        expected_codex_home_digest,
        expected_config_digest,
    )?;
    Ok(ManagedCodexAuthReadiness {
        app_server_generation,
        app_server_session_id: app_server_session_id.to_owned(),
        app_server_identity_digest,
        codex_home_digest: expected_codex_home_digest.to_owned(),
        config_digest: expected_config_digest.to_owned(),
    })
}

fn validate_preclaim_auth_readiness(
    readiness: Option<&ManagedCodexAuthReadiness>,
    expected_codex_home_digest: &str,
    expected_config_digest: &str,
) -> ManagedPortResult<()> {
    let readiness = readiness.ok_or_else(|| known("CREDENTIAL_READ_ISOLATION_NOT_VERIFIED"))?;
    if readiness.codex_home_digest != expected_codex_home_digest
        || readiness.config_digest != expected_config_digest
        || readiness.app_server_generation == 0
        || readiness.app_server_session_id.is_empty()
    {
        return Err(known("CREDENTIAL_READ_ISOLATION_NOT_VERIFIED"));
    }
    Ok(())
}

fn parse_prefixed_digest(value: &str, prefix: &str) -> ManagedPortResult<ContentDigest> {
    let digest = value
        .strip_prefix(prefix)
        .ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_EVIDENCE_REJECTED"))?;
    ContentDigest::from_sha256(digest)
        .map_err(|_| known("LATTICE_MANAGED_BRIDGE_EVIDENCE_REJECTED"))
}

fn parse_terminal(event: &Value) -> ManagedPortResult<WorkerTerminal> {
    match event.get("status").and_then(Value::as_str) {
        Some("completed") => Ok(WorkerTerminal::Completed),
        Some("interrupted") => Ok(WorkerTerminal::Interrupted),
        Some("failed") => Ok(WorkerTerminal::Failed),
        _ => Err(known("LATTICE_MANAGED_TERMINAL_REJECTED")),
    }
}

fn stage_retained_terminal_reconciliation(
    attempt_number: u64,
    terminal: ManagedTerminalCandidate,
) -> ManagedPortResult<(ManagedWorkerObservation, ManagedTerminalCandidate)> {
    let observation = terminal.observation();
    let turn_id = observation
        .turn_id()
        .ok_or_else(|| known("LATTICE_MANAGED_BRIDGE_IDENTITY_MISMATCH"))?;
    let reconciled = ManagedWorkerObservation::reconciled(
        attempt_number,
        observation.thread_id(),
        turn_id,
        observation.app_server_generation(),
        observation.app_server_identity_digest().clone(),
        observation.evidence_digest().clone(),
    )?;
    Ok((reconciled, terminal))
}

fn canonical_execution_time(value: &str) -> ManagedPortResult<OffsetDateTime> {
    let parsed = OffsetDateTime::parse(value, &Rfc3339)
        .map_err(|_| known("LATTICE_MANAGED_RETAINED_EXECUTION_WINDOW_REJECTED"))?;
    let canonical = parsed
        .format(&Rfc3339)
        .map_err(|_| known("LATTICE_MANAGED_RETAINED_EXECUTION_WINDOW_REJECTED"))?;
    if !value.ends_with('Z') || canonical != value {
        return Err(known("LATTICE_MANAGED_RETAINED_EXECUTION_WINDOW_REJECTED"));
    }
    Ok(parsed)
}

fn validate_execution_window(
    started_at: &str,
    deadline_at: &str,
    max_duration_seconds: u64,
    task_deadline_at: &str,
) -> ManagedPortResult<()> {
    let started = canonical_execution_time(started_at)?;
    let deadline = canonical_execution_time(deadline_at)?;
    let task_deadline = canonical_execution_time(task_deadline_at)?;
    let duration = i64::try_from(max_duration_seconds)
        .map_err(|_| known("LATTICE_MANAGED_RETAINED_EXECUTION_WINDOW_REJECTED"))?;
    let attempt_deadline = started
        .checked_add(time::Duration::seconds(duration))
        .ok_or_else(|| known("LATTICE_MANAGED_RETAINED_EXECUTION_WINDOW_REJECTED"))?;
    let expected = attempt_deadline.min(task_deadline);
    if deadline != expected {
        return Err(known("LATTICE_MANAGED_RETAINED_EXECUTION_WINDOW_REJECTED"));
    }
    Ok(())
}

fn bridge_reported_failure(record: &Value) -> ManagedPortError {
    let provider_method = record.get("provider_method").and_then(Value::as_str);
    match record.get("code").and_then(Value::as_str) {
        Some("CODEX_APP_SERVER_RPC_REJECTED")
            if record.get("provider_method").and_then(Value::as_str) == Some("thread/start")
                && record.get("provider_rpc_code").and_then(Value::as_i64) == Some(-32602) =>
        {
            known("LATTICE_MANAGED_THREAD_START_RPC_INVALID_PARAMS")
        }
        Some("CODEX_APP_SERVER_RPC_REJECTED")
            if record.get("provider_method").and_then(Value::as_str) == Some("thread/start") =>
        {
            known("LATTICE_MANAGED_THREAD_START_RPC_REJECTED")
        }
        Some("CODEX_APP_SERVER_RPC_REJECTED")
            if record.get("provider_method").and_then(Value::as_str) == Some("turn/start")
                && record.get("provider_rpc_code").and_then(Value::as_i64) == Some(-32602) =>
        {
            known("LATTICE_MANAGED_TURN_START_RPC_INVALID_PARAMS")
        }
        Some("CODEX_APP_SERVER_RPC_REJECTED")
            if record.get("provider_method").and_then(Value::as_str) == Some("turn/start") =>
        {
            known("LATTICE_MANAGED_TURN_START_RPC_REJECTED")
        }
        Some("CODEX_APP_SERVER_TIMEOUT")
            if record.get("provider_method").and_then(Value::as_str) == Some("turn/start") =>
        {
            ManagedPortError::new(
                ManagedPortErrorKind::ReconcileRequired,
                "LATTICE_MANAGED_TURN_START_TIMEOUT_RECONCILIATION_REQUIRED",
            )
        }
        Some("CODEX_APP_SERVER_TIMEOUT") if provider_method == Some("turn/started") => {
            ManagedPortError::new(
                ManagedPortErrorKind::ReconcileRequired,
                "LATTICE_MANAGED_EXACT_START_TIMEOUT_RECONCILIATION_REQUIRED",
            )
        }
        Some("CODEX_APP_SERVER_TIMEOUT")
            if matches!(provider_method, Some("thread/start" | "thread/started")) =>
        {
            ManagedPortError::new(
                ManagedPortErrorKind::ReconcileRequired,
                "LATTICE_MANAGED_THREAD_START_TIMEOUT_RECONCILIATION_REQUIRED",
            )
        }
        Some("CODEX_APP_SERVER_PROCESS_EXITED" | "CODEX_APP_SERVER_TRANSPORT_ERROR")
            if matches!(provider_method, Some("turn/start" | "turn/started")) =>
        {
            ManagedPortError::new(
                ManagedPortErrorKind::ReconcileRequired,
                "LATTICE_MANAGED_EXACT_START_TRANSPORT_RECONCILIATION_REQUIRED",
            )
        }
        Some("CODEX_APP_SERVER_PROCESS_EXITED" | "CODEX_APP_SERVER_TRANSPORT_ERROR")
            if matches!(provider_method, Some("thread/start" | "thread/started")) =>
        {
            ManagedPortError::new(
                ManagedPortErrorKind::ReconcileRequired,
                "LATTICE_MANAGED_THREAD_START_TRANSPORT_RECONCILIATION_REQUIRED",
            )
        }
        Some("MANAGED_CODEX_DISPATCH_RECONCILIATION_REQUIRED") => ManagedPortError::new(
            ManagedPortErrorKind::ReconcileRequired,
            "LATTICE_MANAGED_DISPATCH_RECONCILIATION_REQUIRED",
        ),
        Some("MANAGED_CODEX_EXACT_START_EVIDENCE_LOST_AFTER_DISPATCH") => ManagedPortError::new(
            ManagedPortErrorKind::ReconcileRequired,
            "LATTICE_MANAGED_EXACT_START_EVIDENCE_LOST_AFTER_DISPATCH",
        ),
        Some("MANAGED_CODEX_AUTH_READINESS_NOT_VERIFIED") => {
            known("CREDENTIAL_READ_ISOLATION_NOT_VERIFIED")
        }
        Some("MANAGED_CODEX_MODEL_UNAVAILABLE") => known("LATTICE_MANAGED_MODEL_UNAVAILABLE"),
        Some("MANAGED_CODEX_STALL_RECONCILIATION_FAILED")
        | Some("MANAGED_CODEX_RPC_DISCONNECT_RECONCILIATION_EXHAUSTED")
        | Some("CODEX_THREAD_NOT_RECOVERABLE")
        | Some("CODEX_THREAD_LIST_NOT_RECOVERABLE") => ManagedPortError::new(
            ManagedPortErrorKind::ReconcileRequired,
            "LATTICE_MANAGED_RPC_DISCONNECT_RECONCILIATION_EXHAUSTED",
        ),
        Some("MANAGED_CODEX_PROCESS_EXIT_WITHOUT_TERMINAL") => ManagedPortError::new(
            ManagedPortErrorKind::ReconcileRequired,
            "LATTICE_MANAGED_PROCESS_EXIT_WITHOUT_TERMINAL",
        ),
        _ => known("LATTICE_MANAGED_BRIDGE_REPORTED_FAILURE"),
    }
}

fn digest_parts(parts: &[&str]) -> ManagedPortResult<ContentDigest> {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(u64::try_from(part.len()).unwrap_or(u64::MAX).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    let bytes = hasher.finalize();
    let mut encoded = String::with_capacity(64);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}")
            .map_err(|_| known("LATTICE_MANAGED_EVIDENCE_DIGEST_REJECTED"))?;
    }
    ContentDigest::from_sha256(encoded)
        .map_err(|_| known("LATTICE_MANAGED_EVIDENCE_DIGEST_REJECTED"))
}

const fn known(code: &'static str) -> ManagedPortError {
    ManagedPortError::new(ManagedPortErrorKind::Known, code)
}

const fn ambiguous(code: &'static str) -> ManagedPortError {
    ManagedPortError::new(ManagedPortErrorKind::Ambiguous, code)
}

#[allow(dead_code)]
fn _assert_absolute(path: &Path) -> bool {
    path.is_absolute()
}

#[cfg(test)]
mod tests {
    use super::{
        ACTIVE_WORKER_READERS, ManagedPrestartShutdownDisposition,
        ManagedProviderEffectAdmissionError, ManagedReviewerShutdownDisposition,
        ManagedWorkerCancellation, ManagedWorkerPrestartShutdownReceipt,
        ManagedWorkerShutdownReceipt, WorkerReaderActivity, bridge_reported_failure,
        configure_managed_codex_environment, exact_teardown_control,
        exact_turn_start_authorization_control, execution_watchdog_remaining_at,
        managed_app_server_identity_digest, non_verbatim_managed_child_path,
        parse_managed_auth_readiness, probe_watchdog_budget_at, read_record,
        receive_probe_record_until, reconcile_app_server_generation, retain_control_channel,
        stage_retained_terminal_reconciliation, validate_execution_window,
        validate_preclaim_auth_readiness,
    };
    use lattice_contracts::ContentDigest;
    use lattice_foreman_state::WorkerTerminal;
    use lattice_ports::{
        ManagedPortErrorKind, ManagedTerminalCandidate, ManagedWorkerObservation,
        WorkerObservationKind,
    };
    use std::collections::BTreeMap;
    use std::ffi::OsStr;
    use std::io::{BufReader, Cursor, Read};
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    #[test]
    fn extensionless_oversized_bridge_record_fails_before_unbounded_allocation_and_reader_exits() {
        let reader = thread::spawn(|| {
            let _activity = WorkerReaderActivity::new();
            let input: Box<dyn Read + Send> =
                Box::new(Cursor::new(vec![b'x'; super::MAX_BRIDGE_LINE_BYTES + 1]));
            let mut input = BufReader::new(input);
            read_record(&mut input).expect_err("extensionless oversized line must fail closed")
        });
        assert_eq!(
            reader.join().expect("bounded reader join").code(),
            "LATTICE_MANAGED_BRIDGE_OUTPUT_REJECTED"
        );
        assert_eq!(
            ACTIVE_WORKER_READERS.load(std::sync::atomic::Ordering::Acquire),
            0
        );
    }

    #[test]
    fn restart_resume_retains_the_exact_interrupt_control_channel() {
        assert!(retain_control_channel("start"));
        assert!(retain_control_channel("continue-turn"));
        assert!(retain_control_channel("resume"));
        assert!(!retain_control_channel("recover-dispatch"));
        assert!(!retain_control_channel("recover-prestart"));
        assert!(!retain_control_channel("probe"));
    }

    #[test]
    fn credential_readiness_requires_an_exact_sanitized_identity_bound_projection() {
        let home = format!("codex-home:sha256:{}", "a".repeat(64));
        let config = format!("codex-config:sha256:{}", "b".repeat(64));
        let session = format!("app-server-session:sha256:{}", "c".repeat(64));
        let exact = serde_json::json!({
            "schema": "lattice.managed-codex-auth-readiness/1.0",
            "ready": true,
            "auth_mode": "chatgpt",
            "app_server_generation": 3,
            "app_server_session_id": session,
            "codex_home_digest": home,
            "config_digest": config,
        });
        let parsed = parse_managed_auth_readiness(&exact, &home, &config)
            .expect("exact sanitized readiness");
        assert_eq!(parsed.app_server_generation, 3);
        assert_eq!(parsed.app_server_session_id, session);
        assert_eq!(
            parsed.app_server_identity_digest,
            managed_app_server_identity_digest(&session, &home, &config, &home, &config)
                .expect("canonical identity digest")
        );
        validate_preclaim_auth_readiness(Some(&parsed), &home, &config)
            .expect("exact preclaim receipt remains current");
        assert_eq!(
            validate_preclaim_auth_readiness(None, &home, &config)
                .expect_err("missing preclaim readiness must block before a provider effect")
                .code(),
            "CREDENTIAL_READ_ISOLATION_NOT_VERIFIED"
        );

        for rejected in [
            serde_json::json!({
                "schema": "lattice.managed-codex-auth-readiness/1.0",
                "ready": true,
                "auth_mode": "chatgpt",
                "app_server_generation": 3,
                "app_server_session_id": session,
                "codex_home_digest": format!("codex-home:sha256:{}", "c".repeat(64)),
                "config_digest": config,
            }),
            serde_json::json!({
                "schema": "lattice.managed-codex-auth-readiness/1.0",
                "ready": true,
                "auth_mode": "chatgpt",
                "app_server_generation": 3,
                "app_server_session_id": session,
                "codex_home_digest": home,
                "config_digest": config,
                "email": "must-not-be-admitted@example.invalid",
            }),
        ] {
            assert_eq!(
                parse_managed_auth_readiness(&rejected, &home, &config)
                    .expect_err("tampered or identity-bearing readiness must fail")
                    .code(),
                "CREDENTIAL_READ_ISOLATION_NOT_VERIFIED"
            );
        }
    }

    #[test]
    fn bridge_transport_exhaustion_maps_only_to_retained_provider_blockers() {
        for (reported, expected) in [
            (
                "MANAGED_CODEX_RPC_DISCONNECT_RECONCILIATION_EXHAUSTED",
                "LATTICE_MANAGED_RPC_DISCONNECT_RECONCILIATION_EXHAUSTED",
            ),
            (
                "MANAGED_CODEX_PROCESS_EXIT_WITHOUT_TERMINAL",
                "LATTICE_MANAGED_PROCESS_EXIT_WITHOUT_TERMINAL",
            ),
        ] {
            let failure = bridge_reported_failure(&serde_json::json!({
                "kind": "error",
                "code": reported,
            }));
            assert_eq!(failure.kind(), ManagedPortErrorKind::ReconcileRequired);
            assert_eq!(failure.code(), expected);
        }
    }

    #[test]
    fn account_readiness_failure_maps_to_the_closed_credential_blocker() {
        let failure = bridge_reported_failure(&serde_json::json!({
            "kind": "error",
            "code": "MANAGED_CODEX_AUTH_READINESS_NOT_VERIFIED",
            "provider_method": "account/read",
        }));
        assert_eq!(failure.kind(), ManagedPortErrorKind::Known);
        assert_eq!(failure.code(), "CREDENTIAL_READ_ISOLATION_NOT_VERIFIED");
    }

    #[test]
    fn provider_start_failures_preserve_closed_retry_semantics() {
        let invalid_thread = bridge_reported_failure(&serde_json::json!({
            "kind": "error",
            "code": "CODEX_APP_SERVER_RPC_REJECTED",
            "provider_method": "thread/start",
            "provider_rpc_code": -32602,
        }));
        assert_eq!(invalid_thread.kind(), ManagedPortErrorKind::Known);
        assert_eq!(
            invalid_thread.code(),
            "LATTICE_MANAGED_THREAD_START_RPC_INVALID_PARAMS"
        );

        let rejected_thread = bridge_reported_failure(&serde_json::json!({
            "kind": "error",
            "code": "CODEX_APP_SERVER_RPC_REJECTED",
            "provider_method": "thread/start",
            "provider_rpc_code": -32603,
        }));
        assert_eq!(rejected_thread.kind(), ManagedPortErrorKind::Known);
        assert_eq!(
            rejected_thread.code(),
            "LATTICE_MANAGED_THREAD_START_RPC_REJECTED"
        );

        let invalid = bridge_reported_failure(&serde_json::json!({
            "kind": "error",
            "code": "CODEX_APP_SERVER_RPC_REJECTED",
            "provider_method": "turn/start",
            "provider_rpc_code": -32602,
        }));
        assert_eq!(invalid.kind(), ManagedPortErrorKind::Known);
        assert_eq!(
            invalid.code(),
            "LATTICE_MANAGED_TURN_START_RPC_INVALID_PARAMS"
        );

        let rejected = bridge_reported_failure(&serde_json::json!({
            "kind": "error",
            "code": "CODEX_APP_SERVER_RPC_REJECTED",
            "provider_method": "turn/start",
            "provider_rpc_code": -32603,
        }));
        assert_eq!(rejected.kind(), ManagedPortErrorKind::Known);
        assert_eq!(rejected.code(), "LATTICE_MANAGED_TURN_START_RPC_REJECTED");

        let timed_out = bridge_reported_failure(&serde_json::json!({
            "kind": "error",
            "code": "CODEX_APP_SERVER_TIMEOUT",
            "provider_method": "turn/start",
        }));
        assert_eq!(timed_out.kind(), ManagedPortErrorKind::ReconcileRequired);
        assert_eq!(
            timed_out.code(),
            "LATTICE_MANAGED_TURN_START_TIMEOUT_RECONCILIATION_REQUIRED"
        );

        for (record, expected) in [
            (
                serde_json::json!({
                    "kind": "error",
                    "code": "CODEX_APP_SERVER_TIMEOUT",
                    "provider_method": "turn/started",
                }),
                "LATTICE_MANAGED_EXACT_START_TIMEOUT_RECONCILIATION_REQUIRED",
            ),
            (
                serde_json::json!({
                    "kind": "error",
                    "code": "CODEX_APP_SERVER_TIMEOUT",
                    "provider_method": "thread/started",
                }),
                "LATTICE_MANAGED_THREAD_START_TIMEOUT_RECONCILIATION_REQUIRED",
            ),
            (
                serde_json::json!({
                    "kind": "error",
                    "code": "CODEX_APP_SERVER_PROCESS_EXITED",
                    "provider_method": "turn/start",
                }),
                "LATTICE_MANAGED_EXACT_START_TRANSPORT_RECONCILIATION_REQUIRED",
            ),
            (
                serde_json::json!({
                    "kind": "error",
                    "code": "CODEX_APP_SERVER_PROCESS_EXITED",
                    "provider_method": "turn/started",
                }),
                "LATTICE_MANAGED_EXACT_START_TRANSPORT_RECONCILIATION_REQUIRED",
            ),
            (
                serde_json::json!({
                    "kind": "error",
                    "code": "CODEX_APP_SERVER_TRANSPORT_ERROR",
                    "provider_method": "thread/start",
                }),
                "LATTICE_MANAGED_THREAD_START_TRANSPORT_RECONCILIATION_REQUIRED",
            ),
            (
                serde_json::json!({
                    "kind": "error",
                    "code": "CODEX_APP_SERVER_PROCESS_EXITED",
                    "provider_method": "thread/started",
                }),
                "LATTICE_MANAGED_THREAD_START_TRANSPORT_RECONCILIATION_REQUIRED",
            ),
        ] {
            let failure = bridge_reported_failure(&record);
            assert_eq!(failure.kind(), ManagedPortErrorKind::ReconcileRequired);
            assert_eq!(failure.code(), expected);
        }
    }

    #[test]
    fn exact_reconnect_generation_advances_only_at_the_reconcile_boundary() {
        assert_eq!(
            reconcile_app_server_generation(None, 4, "TURN_START_ACCEPTED", false)
                .expect("initial generation"),
            4
        );
        assert_eq!(
            reconcile_app_server_generation(Some(4), 4, "HEARTBEAT", true)
                .expect("same active generation"),
            4
        );
        assert_eq!(
            reconcile_app_server_generation(Some(4), 6, "RECONCILE_STARTED", true)
                .expect("bounded reconnect may skip failed connection generations"),
            6
        );
        for (event_type, exact_active) in [
            ("RECONCILED_ACTIVE", true),
            ("RECONCILE_STARTED", false),
            ("TURN_TERMINAL", true),
        ] {
            assert_eq!(
                reconcile_app_server_generation(Some(4), 5, event_type, exact_active)
                    .expect_err("generation substitution must fail closed")
                    .code(),
                "LATTICE_MANAGED_BRIDGE_GENERATION_REJECTED"
            );
        }
        assert_eq!(
            reconcile_app_server_generation(Some(5), 4, "RECONCILE_STARTED", true)
                .expect_err("generation rollback must fail closed")
                .code(),
            "LATTICE_MANAGED_BRIDGE_GENERATION_REJECTED"
        );
    }

    #[test]
    fn retained_terminal_rotates_identity_at_a_reconciled_boundary_before_terminal() {
        let identity = ContentDigest::from_sha256("a".repeat(64)).expect("identity");
        let evidence = ContentDigest::from_sha256("b".repeat(64)).expect("evidence");
        let terminal = ManagedTerminalCandidate::new(
            ManagedWorkerObservation::terminal(
                1,
                "thread-retained",
                "turn-retained",
                WorkerTerminal::Interrupted,
                2,
                identity.clone(),
                evidence.clone(),
            )
            .expect("exact retained terminal"),
        )
        .expect("terminal candidate");

        let (boundary, pending) = stage_retained_terminal_reconciliation(1, terminal)
            .expect("terminal reconciliation boundary");

        assert_eq!(boundary.kind(), WorkerObservationKind::Reconciled);
        assert_eq!(boundary.thread_id(), "thread-retained");
        assert_eq!(boundary.turn_id(), Some("turn-retained"));
        assert_eq!(boundary.app_server_generation(), 2);
        assert_eq!(boundary.app_server_identity_digest(), &identity);
        assert_eq!(boundary.evidence_digest(), &evidence);
        assert_eq!(
            pending.observation().terminal_kind(),
            Some(WorkerTerminal::Interrupted)
        );
    }

    #[test]
    fn retained_terminal_boundary_is_durable_before_cached_terminal_replay() {
        let source = include_str!("managed_worker_adapter.rs");
        let read_thread = source
            .split("    fn read_exact_thread(")
            .nth(1)
            .expect("read exact thread")
            .split("    fn read_exact_turn(")
            .next()
            .expect("read exact thread body");
        let pending = read_thread
            .find("self.pending_terminal_reconciliation.is_some()")
            .expect("repeat read consumes the cached terminal");
        let staged = read_thread
            .find("stage_retained_terminal_reconciliation")
            .expect("stage reconciled boundary");
        let cached = read_thread
            .find("self.pending_terminal_reconciliation = Some(pending)")
            .expect("cache exact terminal");
        let returned = read_thread
            .find("ManagedWorkerReconciliation::ExactActive(boundary)")
            .expect("return durable boundary first");
        assert!(pending < staged && staged < cached && cached < returned);

        let read_turn = source
            .split("    fn read_exact_turn(")
            .nth(1)
            .expect("read exact turn")
            .split("    fn resume_exact_turn(")
            .next()
            .expect("read exact turn body");
        let cached_terminal = read_turn
            .find("self.pending_terminal_reconciliation.take()")
            .expect("consume cached exact terminal");
        let spawn = read_turn
            .find("self.spawn(&command")
            .expect("fresh resume spawn");
        assert!(
            cached_terminal < spawn,
            "cached terminal must not spawn again"
        );
    }

    #[test]
    fn exact_failed_start_maps_to_the_typed_prestart_terminal_only() {
        let observation = ManagedWorkerObservation::prestart_terminal_failed(
            1,
            "thread-prestart",
            "turn-prestart",
            3,
            ContentDigest::from_sha256("b".repeat(64)).expect("identity digest"),
            ContentDigest::from_sha256("a".repeat(64)).expect("evidence digest"),
        )
        .expect("typed prestart terminal");
        assert_eq!(
            observation.kind(),
            WorkerObservationKind::PrestartTerminalFailed
        );
        assert_eq!(observation.terminal_kind(), Some(WorkerTerminal::Failed));
        ManagedTerminalCandidate::new(observation)
            .expect("prestart failure is terminal without an exact-start observation");

        let source = include_str!("managed_worker_adapter.rs");
        let recovery = source
            .split("\"PRESTART_TERMINAL\" =>")
            .nth(1)
            .expect("prestart terminal transport")
            .split("_ =>")
            .next()
            .expect("prestart terminal mapping");
        assert!(recovery.contains("ManagedWorkerObservation::prestart_terminal_failed"));
        assert!(!recovery.contains("ManagedWorkerObservation::terminal("));
    }

    #[test]
    fn graceful_cancellation_receipt_is_exact_and_substitution_closed() {
        let cancellation = ManagedWorkerCancellation::default();
        assert!(!cancellation.is_requested());
        cancellation.request();
        assert!(cancellation.is_requested());
        cancellation.wait_timeout(std::time::Duration::from_secs(60));

        let exact = ManagedWorkerShutdownReceipt {
            task_ref: "task:sha256:aaaaaaaa".to_owned(),
            attempt: 1,
            thread_id: "thread-managed".to_owned(),
            turn_id: "turn-managed".to_owned(),
            terminal: WorkerTerminal::Interrupted,
            terminal_evidence_digest: ContentDigest::from_sha256("1".repeat(64)).expect("digest"),
        };
        cancellation
            .record_exact_receipt(exact.clone())
            .expect("record exact receipt");
        cancellation
            .record_exact_receipt(exact.clone())
            .expect("exact replay");
        assert!(cancellation.has_exact_receipt(&exact.task_ref, exact.attempt));
        assert!(!cancellation.has_exact_receipt(&exact.task_ref, 2));

        let substituted = ManagedWorkerShutdownReceipt {
            turn_id: "turn-substituted".to_owned(),
            ..exact
        };
        assert_eq!(
            cancellation
                .record_exact_receipt(substituted)
                .expect_err("substitution must fail")
                .code(),
            "LATTICE_MANAGED_SHUTDOWN_RECEIPT_REJECTED"
        );
    }

    #[test]
    fn reviewer_shutdown_receipts_are_separate_and_exact_identity_bound() {
        let subject_digest = ContentDigest::from_sha256("a".repeat(64)).expect("subject digest");
        let cancellation = ManagedWorkerCancellation::default();
        cancellation
            .record_reviewer_prestart_receipt(
                "review-task",
                2,
                subject_digest.clone(),
                Some("review-thread"),
                None,
            )
            .expect("reviewer prestart receipt");
        assert_eq!(
            cancellation.reviewer_shutdown_disposition("review-task", 2),
            Some(ManagedReviewerShutdownDisposition::Prestart)
        );
        assert_eq!(
            cancellation.reviewer_shutdown_disposition("review-task", 1),
            None
        );
        assert!(
            cancellation
                .record_reviewer_prestart_receipt(
                    "review-task",
                    2,
                    subject_digest.clone(),
                    Some("substituted-thread"),
                    None,
                )
                .is_err()
        );

        let exact = ManagedWorkerCancellation::default();
        let terminal_digest = ContentDigest::from_sha256("b".repeat(64)).expect("terminal digest");
        exact
            .record_reviewer_terminal_receipt(
                "review-task",
                2,
                subject_digest,
                "review-thread",
                "review-turn",
                WorkerTerminal::Interrupted,
                terminal_digest.clone(),
            )
            .expect("reviewer exact-terminal receipt");
        exact
            .record_reviewer_terminal_receipt(
                "review-task",
                2,
                ContentDigest::from_sha256("a".repeat(64)).expect("subject digest replay"),
                "review-thread",
                "review-turn",
                WorkerTerminal::Interrupted,
                terminal_digest.clone(),
            )
            .expect("reviewer exact-terminal replay");
        assert_eq!(
            exact.reviewer_shutdown_disposition("review-task", 2),
            Some(ManagedReviewerShutdownDisposition::ExactTerminal)
        );
        assert!(!exact.has_exact_receipt("review-task", 2));
        assert!(!exact.has_exact_prestart_receipt("review-task", 2));
        assert!(
            exact
                .record_reviewer_terminal_receipt(
                    "review-task",
                    2,
                    ContentDigest::from_sha256("a".repeat(64)).expect("subject digest"),
                    "review-thread",
                    "substituted-turn",
                    WorkerTerminal::Interrupted,
                    terminal_digest,
                )
                .is_err()
        );
    }

    #[test]
    fn prestart_shutdown_receipt_is_exact_and_substitution_closed() {
        let cancellation = ManagedWorkerCancellation::default();
        let exact = ManagedWorkerPrestartShutdownReceipt {
            task_ref: "task:sha256:aaaaaaaa".to_owned(),
            attempt: 2,
            packet_digest: format!("attempt-packet:sha256:{}", "b".repeat(64)),
            thread_id: Some("thread-managed".to_owned()),
            turn_id: Some("turn-managed".to_owned()),
            disposition: ManagedPrestartShutdownDisposition::BridgeSubtreeExited,
        };
        cancellation
            .record_prestart_receipt(exact.clone())
            .expect("record exact prestart receipt");
        cancellation
            .record_prestart_receipt(exact.clone())
            .expect("exact prestart replay");
        assert!(cancellation.has_exact_prestart_receipt(&exact.task_ref, exact.attempt));
        assert!(!cancellation.has_exact_prestart_receipt(&exact.task_ref, 1));

        let substituted = ManagedWorkerPrestartShutdownReceipt {
            packet_digest: format!("attempt-packet:sha256:{}", "c".repeat(64)),
            ..exact
        };
        assert_eq!(
            cancellation
                .record_prestart_receipt(substituted)
                .expect_err("prestart substitution must fail")
                .code(),
            "LATTICE_MANAGED_PRESTART_SHUTDOWN_RECEIPT_REJECTED"
        );
    }

    #[test]
    fn prestart_shutdown_proof_requires_job_reap_before_receipt_registration() {
        let source = include_str!("managed_worker_adapter.rs");
        let terminate = source
            .split("fn terminate_prestart_and_reap")
            .nth(1)
            .expect("prestart teardown")
            .split("impl Drop for ActiveBridge")
            .next()
            .expect("prestart teardown body");
        let reap = terminate
            .find("prove_subtree_empty_and_join_reader")
            .expect("owned Job subtree reap helper");
        let proof = terminate
            .find("self.prestart_shutdown_receipt()")
            .expect("typed exit proof");
        assert!(reap < proof);

        let cleanup = source
            .split("fn prove_subtree_empty_and_join_reader")
            .nth(1)
            .expect("subtree cleanup helper")
            .split("fn cancellation_disposition")
            .next()
            .expect("subtree cleanup body");
        let terminate_job = cleanup
            .find(".terminate_and_reap()")
            .expect("owned Job subtree reap");
        let drop_receiver = cleanup
            .find("self.records.take()")
            .expect("bounded receiver release");
        let join = cleanup.find("self.join_reader()").expect("reader join");
        assert!(terminate_job < drop_receiver && drop_receiver < join);

        let close = source
            .split("fn close_prestart_for_cancellation")
            .nth(1)
            .expect("prestart cancellation close")
            .split("fn reject_if_cancelled_prestart")
            .next()
            .expect("prestart cancellation body");
        let exit = close
            .find("terminate_prestart_and_reap")
            .expect("exact subtree exit");
        let register = close
            .find("record_prestart_receipt")
            .expect("typed proof registration");
        assert!(exit < register);
    }

    #[test]
    fn terminal_result_is_not_a_receipt_before_exact_root_exit_and_reader_join() {
        let source = include_str!("managed_worker_adapter.rs");
        let finalize = source
            .split("fn finalize_terminal")
            .nth(1)
            .expect("terminal finalizer")
            .split("fn finalize_consumed_result")
            .next()
            .expect("terminal finalizer body");
        let result = finalize
            .find("let result_record")
            .expect("bounded bridge result");
        let root = finalize
            .find(".wait_for_root(BRIDGE_TEARDOWN_GRACE)")
            .expect("exact root exit observation");
        let success = finalize
            .find("status.success()")
            .expect("closed exit status");
        let reap = finalize
            .find(".terminate_and_reap()")
            .expect("owned Job subtree must be proven empty");
        let receiver = finalize
            .find("bridge.records.take()")
            .expect("bounded receiver must be dropped before reader join");
        let reader = finalize.find("join_reader()?").expect("reader join");
        let receipt = finalize
            .find("ManagedBridgeProcessExitReceipt { terminal }")
            .expect("typed exit receipt");
        assert!(
            result < root
                && root < success
                && success < reap
                && reap < receiver
                && receiver < reader
                && reader < receipt
        );
    }

    #[test]
    fn turn_start_shutdown_distinguishes_unsent_authorized_and_exact_states() {
        let source = include_str!("managed_worker_adapter.rs");
        assert!(source.contains("enum TurnStartLifecycle"));
        assert!(source.contains("TurnStartLifecycle::NotAuthorized"));
        assert!(source.contains("TurnStartLifecycle::AuthorizationSent"));
        assert!(source.contains("TurnStartLifecycle::Accepted"));
        assert!(source.contains("TurnStartLifecycle::ExactStarted"));
        assert!(source.contains("LATTICE_MANAGED_TURN_START_SHUTDOWN_AMBIGUOUS"));
    }

    #[test]
    fn active_execution_polls_typed_cancellation_through_a_bounded_reader() {
        assert_eq!(super::BRIDGE_RECORD_QUEUE, 32);
        assert_eq!(
            super::CANCELLATION_POLL,
            std::time::Duration::from_millis(100)
        );
        let source = include_str!("managed_worker_adapter.rs");
        let execution = source
            .split("fn next_execution_event")
            .nth(1)
            .expect("execution event")
            .split("fn read_exact_thread")
            .next()
            .expect("execution event body");
        let requested = execution
            .find("self.cancellation.is_requested()")
            .expect("typed cancellation check");
        let interrupt = execution
            .find("ManagedCodexWorkerPort::interrupt_exact_turn")
            .expect("exact interrupt");
        let bounded_read = execution
            .find("next_matching_event_timeout")
            .expect("bounded reader poll");
        assert!(requested < interrupt && interrupt < bounded_read);
    }

    #[test]
    fn worker_seals_transitive_bridge_graph_before_spawn_and_rechecks_before_effects() {
        let source = include_str!("managed_worker_adapter.rs");
        assert!(source.contains("managed-codex-worker.mjs"));
        assert!(source.contains("codex-app-server.mjs"));
        let spawn = source
            .split("    fn spawn_with_post_spawn_hook(")
            .nth(1)
            .expect("worker spawn")
            .split("    fn no_bridge_prestart_receipt(")
            .next()
            .expect("worker spawn body");
        let process_spawn = spawn
            .find("SupervisedDuplexChild::spawn")
            .expect("supervised spawn");
        let seal = spawn
            .find("effect_identity.seal()")
            .expect("pre-spawn immutable effect seal");
        let replay = spawn[process_spawn..]
            .find("effect_identity.verify()")
            .expect("post-spawn bundle replay")
            + process_spawn;
        let initial_write = spawn.find("serde_json::to_writer").expect("initial effect");
        assert!(seal < process_spawn && process_spawn < replay && replay < initial_write);
        let control = source
            .split("    fn send_control(")
            .nth(1)
            .expect("control write")
            .split("    fn send_turn_start_authorization(")
            .next()
            .expect("control body");
        assert!(
            control.find("effect_identity.verify()").unwrap()
                < control.find("serde_json::to_writer").unwrap()
        );
    }

    #[test]
    fn prestart_reconcile_result_requires_exact_bridge_cleanup_before_new_spawn() {
        let source = include_str!("managed_worker_adapter.rs");
        let recovery = source
            .split("    fn collect_prestart_recovery(")
            .nth(1)
            .expect("prestart recovery")
            .split("    fn terminate_active_bridge_after_error(")
            .next()
            .expect("recovery body");
        let reported = recovery
            .find("bridge_reported_failure")
            .expect("typed bridge failure");
        let cleanup = recovery
            .find("terminate_active_bridge_after_error")
            .expect("explicit subtree cleanup");
        let reconcile = recovery
            .find("ManagedWorkerPrestartRecovery::ReconciliationRequired")
            .expect("reconciliation result");
        assert!(reported < cleanup && cleanup < reconcile);

        let cleanup_body = source
            .split("    fn terminate_active_bridge_after_error(")
            .nth(1)
            .expect("cleanup helper")
            .split("    fn no_bridge_prestart_receipt(")
            .next()
            .expect("cleanup helper body");
        assert!(cleanup_body.contains("prove_subtree_empty_and_join_reader"));
    }

    #[test]
    fn normal_turn_start_failures_reap_the_active_bridge_before_returning() {
        let source = include_str!("managed_worker_adapter.rs");
        let start_thread = source
            .split("    fn start_thread(")
            .nth(1)
            .expect("thread start")
            .split("    fn start_turn(")
            .next()
            .expect("thread start body");
        let start_turn = source
            .split("    fn start_turn(")
            .nth(1)
            .expect("turn start")
            .split("    fn wait_exact_started(")
            .next()
            .expect("turn start body");
        let exact_start = source
            .split("    fn wait_exact_started(")
            .nth(1)
            .expect("exact start")
            .split("    fn recover_claimed_dispatch(")
            .next()
            .expect("exact start body");

        assert!(start_thread.contains("finish_active_bridge_call"));
        assert!(start_turn.contains("finish_active_bridge_call"));
        assert!(exact_start.contains("finish_active_bridge_call"));

        let cleanup = source
            .split("    fn finish_active_bridge_call")
            .nth(1)
            .expect("normal start cleanup wrapper")
            .split("    fn cached_reconciliation(")
            .next()
            .expect("normal start cleanup body");
        let failure = cleanup.find("Err(failure)").expect("retained failure");
        let reap = cleanup
            .find("terminate_active_bridge_after_error")
            .expect("exact subtree cleanup");
        let returned = cleanup.rfind("Err(failure)").expect("failure return");
        assert!(failure < reap && reap < returned);
        assert!(cleanup.contains("LATTICE_MANAGED_PRESTART_RECOVERY_CLEANUP_AMBIGUOUS"));
    }

    #[test]
    fn blocking_reader_observes_typed_cancellation_within_the_bounded_poll() {
        let cancellation = ManagedWorkerCancellation::default();
        let (sender, receiver) = mpsc::sync_channel(1);
        let signal = cancellation.clone();
        let requester = thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            signal.request();
        });
        let started = Instant::now();
        let outcome = super::receive_record_or_cancellation(
            &receiver,
            &cancellation,
            Duration::from_millis(100),
        )
        .expect("typed cancellation result");
        requester.join().expect("cancellation requester");
        drop(sender);

        assert_eq!(outcome, super::CancellableBridgeRecord::Cancelled);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn silent_model_probe_exits_at_the_bounded_watchdog_and_orders_exact_cleanup() {
        let cancellation = ManagedWorkerCancellation::default();
        let (_sender, receiver) = mpsc::sync_channel(1);
        let started = Instant::now();
        let deadline = started + Duration::from_millis(30);

        let outcome =
            receive_probe_record_until(&receiver, &cancellation, Duration::from_secs(5), deadline)
                .expect("bounded silent probe outcome");

        assert_eq!(outcome, super::BoundedProbeRecord::DeadlineElapsed);
        assert!(started.elapsed() < Duration::from_secs(1));

        let cancelled = ManagedWorkerCancellation::default();
        cancelled.request();
        let (_sender, receiver) = mpsc::sync_channel(1);
        assert_eq!(
            receive_probe_record_until(
                &receiver,
                &cancelled,
                Duration::from_secs(5),
                Instant::now() + Duration::from_secs(5),
            )
            .expect("cancelled bounded probe outcome"),
            super::BoundedProbeRecord::Cancelled,
        );

        let source = include_str!("managed_worker_adapter.rs");
        let run_probe = source
            .split("fn run_probe")
            .nth(1)
            .expect("model probe")
            .split("fn require_preclaim_auth_readiness")
            .next()
            .expect("model probe body");
        let timeout = run_probe
            .find("BoundedProbeRecord::DeadlineElapsed")
            .expect("closed probe deadline");
        let timeout_branch = &run_probe[timeout..];
        let reap = timeout_branch
            .find("bridge.terminate_prestart_and_reap()")
            .expect("owned subtree reap");
        let receipt = timeout_branch
            .find("record_prestart_receipt")
            .expect("exact cleanup receipt");
        let closed = timeout_branch
            .find("ManagedPortErrorKind::ReconcileRequired")
            .expect("closed reconciliation result");
        assert!(reap < receipt && receipt < closed);
        assert!(
            !timeout_branch[closed..].contains("self.spawn("),
            "a timed-out probe must not spawn a replacement",
        );
    }

    #[test]
    fn model_probe_watchdog_uses_the_smaller_packet_or_heartbeat_budget() {
        let now = time::OffsetDateTime::from_unix_timestamp(1_000).expect("fixed time");
        assert_eq!(
            probe_watchdog_budget_at(
                now + time::Duration::seconds(5),
                Duration::from_millis(250),
                now,
            ),
            Some(Duration::from_millis(250)),
        );
        assert_eq!(
            probe_watchdog_budget_at(
                now + time::Duration::milliseconds(40),
                Duration::from_millis(250),
                now,
            ),
            Some(Duration::from_millis(40)),
        );
        assert_eq!(
            probe_watchdog_budget_at(
                now - time::Duration::milliseconds(1),
                Duration::from_millis(250),
                now,
            ),
            None,
        );
    }

    #[test]
    fn monotonic_execution_watchdog_bounds_silent_bridge_and_absolute_deadline() {
        let origin = Instant::now();
        let heartbeat = Duration::from_millis(80);
        let hard = origin + Duration::from_millis(250);
        assert_eq!(
            execution_watchdog_remaining_at(origin, hard, heartbeat, origin),
            Some(heartbeat)
        );
        assert_eq!(
            execution_watchdog_remaining_at(
                origin,
                hard,
                heartbeat,
                origin + Duration::from_millis(80),
            ),
            None
        );
        let refreshed = origin + Duration::from_millis(70);
        assert_eq!(
            execution_watchdog_remaining_at(
                refreshed,
                hard,
                heartbeat,
                origin + Duration::from_millis(100),
            ),
            Some(Duration::from_millis(50))
        );
        assert_eq!(
            execution_watchdog_remaining_at(
                origin + Duration::from_millis(240),
                hard,
                heartbeat,
                origin + Duration::from_millis(250),
            ),
            None,
            "real activity cannot extend the immutable hard deadline",
        );

        let source = include_str!("managed_worker_adapter.rs");
        let failure = source
            .split("fn fail_silent_execution_bridge")
            .nth(1)
            .expect("silent bridge failure")
            .split("fn run_probe")
            .next()
            .expect("silent bridge boundary");
        let blocker = failure
            .find("LATTICE_MANAGED_BRIDGE_HEARTBEAT_TIMEOUT_RECONCILIATION_REQUIRED")
            .expect("retained reconciliation blocker");
        let reap = failure
            .find("terminate_without_shutdown_receipt")
            .expect("owned subtree reap");
        let returned = failure.rfind("Err(ambiguous").expect("retained error");
        assert!(blocker < reap && reap < returned);
    }

    #[test]
    fn cancellation_and_provider_effect_admission_are_strictly_ordered() {
        let cancellation = ManagedWorkerCancellation::default();
        let effect_gate = cancellation
            .admit_provider_effect()
            .expect("provider effect admitted before cancellation");
        let signal = cancellation.clone();
        let requester = thread::spawn(move || signal.request());
        thread::sleep(Duration::from_millis(20));
        assert!(!cancellation.is_requested());
        drop(effect_gate);
        requester.join().expect("ordered cancellation request");
        assert!(cancellation.is_requested());
        assert!(matches!(
            cancellation.admit_provider_effect(),
            Err(ManagedProviderEffectAdmissionError::Cancelled)
        ));

        let source = include_str!("managed_worker_adapter.rs");
        let spawn = source
            .split("fn spawn(&self")
            .nth(1)
            .expect("bridge spawn")
            .split("fn no_bridge_prestart_receipt")
            .next()
            .expect("bridge spawn body");
        let gate = spawn
            .find("lock_provider_effect_admission")
            .expect("thread effect gate");
        let child = spawn
            .find("SupervisedDuplexChild::spawn")
            .expect("owned bridge spawn");
        assert!(gate < child);

        let authorization = source
            .split("fn send_turn_start_authorization")
            .nth(1)
            .expect("turn authorization")
            .split("fn spawn")
            .next()
            .expect("turn authorization body");
        let gate = authorization
            .find("lock_provider_effect_admission")
            .expect("turn effect gate");
        let send = authorization
            .find("send_control")
            .expect("turn control send");
        assert!(gate < send);
    }

    #[test]
    fn durable_turn_authorization_binds_only_the_exact_accepted_thread() {
        let control = exact_turn_start_authorization_control(
            "task:sha256:aaaaaaaa",
            2,
            "attempt-packet:sha256:bbbbbbbb",
            "thread-managed",
        )
        .expect("exact bounded authorization");
        assert_eq!(control["operation"], "authorize_turn_start");
        assert_eq!(control["task_ref"], "task:sha256:aaaaaaaa");
        assert_eq!(control["attempt"], 2);
        assert_eq!(control["packet_digest"], "attempt-packet:sha256:bbbbbbbb");
        assert_eq!(control["thread_id"], "thread-managed");
        assert_eq!(
            exact_turn_start_authorization_control("task", 1, "packet", "")
                .expect_err("missing thread must fail closed")
                .code(),
            "LATTICE_MANAGED_TURN_START_AUTHORIZATION_REJECTED"
        );
    }

    #[test]
    fn exact_started_execution_window_is_derived_and_tamper_closed() {
        assert!(
            validate_execution_window(
                "2026-08-26T14:20:00Z",
                "2026-08-26T14:21:00Z",
                60,
                "2026-08-26T14:30:00Z",
            )
            .is_ok()
        );
        assert!(
            validate_execution_window(
                "2026-08-26T14:20:00.123Z",
                "2026-08-26T14:21:00.123Z",
                60,
                "2026-08-26T14:30:00Z",
            )
            .is_ok()
        );
        assert!(
            validate_execution_window(
                "2026-08-26T14:20:00.12Z",
                "2026-08-26T14:21:00.12Z",
                60,
                "2026-08-26T14:30:00Z",
            )
            .is_ok()
        );
        assert!(
            validate_execution_window(
                "2026-08-26T14:20:00Z",
                "2026-08-26T14:20:30Z",
                60,
                "2026-08-26T14:20:30Z",
            )
            .is_ok()
        );
        assert_eq!(
            validate_execution_window(
                "2026-08-26T14:20:00.120Z",
                "2026-08-26T14:21:00.120Z",
                60,
                "2026-08-26T14:30:00Z",
            )
            .expect_err("trailing-zero timestamp is not cross-runtime canonical")
            .code(),
            "LATTICE_MANAGED_RETAINED_EXECUTION_WINDOW_REJECTED"
        );
        assert_eq!(
            validate_execution_window(
                "2026-08-26T14:20:00Z",
                "2026-08-26T14:22:00Z",
                60,
                "2026-08-26T14:30:00Z",
            )
            .expect_err("tampered deadline")
            .code(),
            "LATTICE_MANAGED_RETAINED_EXECUTION_WINDOW_REJECTED"
        );
        assert_eq!(
            validate_execution_window(
                "2026-08-26T14:20:00.000Z",
                "2026-08-26T14:21:00Z",
                60,
                "2026-08-26T14:30:00Z",
            )
            .expect_err("non-canonical provider start")
            .code(),
            "LATTICE_MANAGED_RETAINED_EXECUTION_WINDOW_REJECTED"
        );
    }

    #[test]
    fn teardown_interrupt_requires_the_retained_exact_active_identity() {
        assert!(
            exact_teardown_control(
                false,
                "task:sha256:aaaaaaaa",
                1,
                "attempt-packet:sha256:bbbbbbbb",
                Some("thread-managed"),
                Some("turn-managed"),
            )
            .is_none()
        );
        assert!(
            exact_teardown_control(
                true,
                "task:sha256:aaaaaaaa",
                1,
                "attempt-packet:sha256:bbbbbbbb",
                Some("thread-managed"),
                None,
            )
            .is_none()
        );
        let control = exact_teardown_control(
            true,
            "task:sha256:aaaaaaaa",
            2,
            "attempt-packet:sha256:bbbbbbbb",
            Some("thread-managed"),
            Some("turn-managed"),
        )
        .expect("exact active teardown");
        assert_eq!(control["operation"], "interrupt");
        assert_eq!(control["attempt"], 2);
        assert_eq!(control["thread_id"], "thread-managed");
        assert_eq!(control["turn_id"], "turn-managed");
    }

    #[test]
    fn graceful_terminal_deadline_starts_after_the_verified_control_write() {
        let source = include_str!("managed_worker_adapter.rs");
        let interrupt = source
            .split("fn interrupt_exact_turn(")
            .nth(1)
            .expect("interrupt implementation")
            .split("fn reconcile_terminal_usage(")
            .next()
            .expect("interrupt implementation body");
        let write = interrupt
            .find("self.send_control(&control)?")
            .expect("control write");
        let deadline = interrupt
            .find("self.shutdown_deadline.get_or_insert_with")
            .expect("terminal deadline");
        let acknowledged = interrupt
            .find("self.shutdown_interrupt_sent = true")
            .expect("durable interrupt acknowledgement handoff");
        let reset = interrupt
            .find("self.shutdown_deadline = None")
            .expect("separate terminal receipt grace");
        assert!(
            write < deadline,
            "control identity verification and write precede terminal grace"
        );
        assert!(
            deadline < acknowledged && acknowledged < reset,
            "terminal receipt grace begins only after request acknowledgement returns for persistence"
        );
    }

    #[test]
    fn bridge_child_uses_the_process_owned_codex_home() {
        let mut command = Command::new("placeholder");
        command.env("LATTICE_TASK019_PASSWORD", "sentinel-secret");
        command.env("OPENAI_API_KEY", "sentinel-secret");
        command.env("PATH", r"C:\hostile\codex-launchers;C:\ambient-fallback");
        command.env(
            "PSModuleAnalysisCachePath",
            r"Microsoft\Windows\PowerShell\ModuleAnalysisCache",
        );
        let executable = Path::new(r"C:\official-codex\codex.exe");
        let home = Path::new(r"C:\lattice\managed-codex-home");
        configure_managed_codex_environment(
            &mut command,
            Some(executable),
            home,
            "execution-environment:sha256:0000000000000000000000000000000000000000000000000000000000000001",
            None,
            None,
            None,
            None,
        )
            .expect("managed child environment");
        let environment = command
            .get_envs()
            .map(|(key, value)| (key.to_owned(), value.map(OsStr::to_owned)))
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(
            environment.get(OsStr::new("LATTICE_CODEX_BIN")),
            Some(&Some(executable.as_os_str().to_owned()))
        );
        for key in ["CODEX_HOME", "LATTICE_DELIVERY_CODEX_HOME"] {
            assert_eq!(
                environment.get(OsStr::new(key)),
                Some(&Some(home.as_os_str().to_owned()))
            );
        }
        for key in ["HOME", "USERPROFILE", "APPDATA", "LOCALAPPDATA"] {
            assert_eq!(
                environment.get(OsStr::new(key)),
                Some(&Some(home.as_os_str().to_owned()))
            );
        }
        assert!(!environment.contains_key(OsStr::new("LATTICE_TASK019_PASSWORD")));
        assert!(!environment.contains_key(OsStr::new("OPENAI_API_KEY")));
        #[cfg(windows)]
        assert_eq!(
            environment.get(OsStr::new("PSModuleAnalysisCachePath")),
            Some(&Some(
                home.join("powershell-module-analysis-cache")
                    .into_os_string()
            ))
        );
        let shell_path = environment
            .get(OsStr::new("PATH"))
            .and_then(Option::as_ref)
            .expect("closed shell PATH");
        assert!(!shell_path.to_string_lossy().contains("hostile"));
        assert!(!shell_path.to_string_lossy().contains("ambient-fallback"));
        assert!(
            std::env::split_paths(shell_path).all(|path| {
                !["codex", "codex.exe", "codex.cmd", "codex.ps1"]
                    .iter()
                    .any(|name| path.join(name).exists())
            }),
            "managed shell PATH must exclude every Codex launcher directory"
        );
    }

    #[test]
    fn wsl_bridge_receives_only_exact_descriptor_and_preflight_not_windows_codex_identity() {
        let environment_ref = format!("execution-environment:sha256:{}", "a".repeat(64));
        let descriptor = serde_json::json!({
            "schema": "lattice.execution-environment.wsl2-linux/1.1",
            "kind": "WSL2_LINUX",
            "identity_digest": environment_ref,
        })
        .to_string();
        let receipt = serde_json::json!({
            "schema": "lattice.wsl2-zero-model-preflight/1.0",
            "status": "PASS",
            "execution_environment_ref": environment_ref,
            "provider_effect_count": 0,
        })
        .to_string();
        let preflight_descriptor_digest = "b".repeat(64);
        let preflight_content_digest = super::sha256_hex(receipt.as_bytes());
        let mut command = Command::new(if cfg!(windows) {
            "cmd.exe"
        } else {
            "/bin/true"
        });
        configure_managed_codex_environment(
            &mut command,
            None,
            Path::new(if cfg!(windows) {
                r"C:\windows-home"
            } else {
                "/tmp/windows-home"
            }),
            &environment_ref,
            Some(&descriptor),
            Some(&receipt),
            Some(&preflight_descriptor_digest),
            Some(&preflight_content_digest),
        )
        .expect("WSL bridge environment");
        let environment = command
            .get_envs()
            .map(|(name, value)| {
                (
                    name.to_string_lossy().into_owned(),
                    value.map(|value| value.to_string_lossy().into_owned()),
                )
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            environment
                .get("LATTICE_MANAGED_EXECUTION_ENVIRONMENT_JSON")
                .and_then(Option::as_deref),
            Some(descriptor.as_str())
        );
        assert_eq!(
            environment
                .get("LATTICE_MANAGED_EXECUTION_PREFLIGHT_JSON")
                .and_then(Option::as_deref),
            Some(receipt.as_str())
        );
        for forbidden in [
            "LATTICE_CODEX_BIN",
            "LATTICE_DELIVERY_CODEX_HOME",
            "CODEX_HOME",
            "HOME",
            "USERPROFILE",
            "APPDATA",
            "LOCALAPPDATA",
            "PSModulePath",
            "PSModuleAnalysisCachePath",
        ] {
            assert!(
                !environment.contains_key(forbidden),
                "forbidden {forbidden}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn managed_child_execution_normalizes_only_local_verbatim_paths() {
        assert_eq!(
            non_verbatim_managed_child_path(Path::new(r"\\?\C:\lattice\codex.exe"))
                .expect("local verbatim path"),
            PathBuf::from(r"C:\lattice\codex.exe")
        );
        assert!(
            non_verbatim_managed_child_path(Path::new(r"\\?\UNC\server\share\codex.exe")).is_err()
        );
    }
}
