//! Bounded MCP stdio surface for the canonical `latticed` entry.

use std::cell::RefCell;
use std::collections::HashSet;
use std::env;
use std::error::Error;
use std::fmt;
use std::fmt::Write as _;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256};
use lattice_contracts::{
    ContentDigest, SubjectBinding, TASK_INGRESS_CLIENT_REQUEST_ID_MAX_BYTES,
    valid_task_ingress_client_request_id,
};
use lattice_foreman_state::{DependencyBinding, ForemanCheckpointIntent, ForemanState};
use lattice_task_ledger::{
    TASK_LEDGER_PROJECT_SNAPSHOT_ID_MAX_BYTES, task_submission_text_contains_secret,
};
use serde_json::{Map, Value, json};
use sha2::{Digest as _, Sha256};
use time::format_description::well_known::Rfc3339;
use time::{OffsetDateTime, UtcOffset};
use unicode_normalization::is_nfc;

use crate::mcp_budget::{McpAdmission, McpBudget, McpToolClass};

/// Legacy stateful MCP protocol version implemented by this server.
pub const MCP_PROTOCOL_VERSION: &str = "2025-11-25";
/// Stateless MCP protocol version implemented by this server.
pub const MCP_STATELESS_PROTOCOL_VERSION: &str = "2026-07-28";
/// Sole delivery execution tool.
pub const DELIVERY_RUN_TOOL: &str = "lattice_delivery_run";
/// Sole delivery status tool.
pub const DELIVERY_STATUS_TOOL: &str = "lattice_delivery_status";
/// Read-only Runtime component status tool.
pub const RUNTIME_STATUS_TOOL: &str = "lattice_runtime_status";
/// Read-only durable delivery-reconciliation probe.
pub const DELIVERY_RECONCILE_TOOL: &str = "lattice_delivery_reconcile";
/// Bounded high-level task submission tool.
pub const TASK_SUBMIT_TOOL: &str = "lattice_task_submit";
/// Bounded durable task status tool.
pub const TASK_STATUS_TOOL: &str = "lattice_task_status";
/// Sole durable foreman checkpoint tool.
pub const FOREMAN_CHECKPOINT_TOOL: &str = "lattice_foreman_checkpoint";
/// Retained exact canary intent accepted alongside bounded general objectives.
pub const CONTROLLED_CODEX_CANARY_INTENT: &str = "CONTROLLED_CODEX_CANARY";
/// Closed lifecycle intent for adopting an independently verified external result.
pub const ADOPT_VERIFIED_RESULT_INTENT: &str = "ADOPT_VERIFIED_RESULT_V1";

const LEGACY_DELIVERY_RUN_DISABLED: &str = "LATTICE_DELIVERY_RUN_REQUIRES_CANONICAL_LATTICED";

const MAX_CLIENT_REQUEST_ID_BYTES: usize = TASK_INGRESS_CLIENT_REQUEST_ID_MAX_BYTES;
// JSON Schema `maxLength` counts Unicode scalar values while the durable
// ledger bounds UTF-8 bytes.  Keep both limits explicit and choose character
// limits whose worst-case UTF-8 representation still fits the durable bound.
const MAX_TASK_OBJECTIVE_CHARS: usize = 512;
const MAX_TASK_OBJECTIVE_BYTES: usize = 2_048;
const MAX_PROJECT_ID_BYTES: usize = 64;
const MAX_PROJECT_NAME_CHARS: usize = 64;
const MAX_PROJECT_NAME_BYTES: usize = 256;
const MAX_PROJECT_SNAPSHOT_ID_BYTES: usize = TASK_LEDGER_PROJECT_SNAPSHOT_ID_MAX_BYTES;
const FOREMAN_CHECKPOINT_RESULT_SCHEMA: &str = "lattice.foreman-checkpoint-result/1.0";
const TASK_PUBLIC_STATUS_SCHEMA_V2: &str = "lattice.task.status.v2";
const TASK_PUBLIC_STATUS_SCHEMA_V3: &str = "lattice.task.status.v3";
const TASK_PUBLIC_STATUS_SCHEMA_V4: &str = "lattice.task.status.v4";
const TASK_PUBLIC_STATUS_SCHEMA_V5: &str = "lattice.task.status.v5";
const TASK_PUBLIC_STATUS_SCHEMA_V6: &str = "lattice.task.status.v6";
const TASK_PUBLIC_STATUS_VALUES: [&str; 5] = [
    "NOT_SUBMITTED",
    "SUBMITTED",
    "RECONCILIATION_REQUIRED",
    "FAILED",
    "COMPLETED",
];
const TASK_MANAGED_PUBLIC_STATUS_VALUES: [&str; 5] = [
    "SUBMITTED",
    "RUNNING",
    "BLOCKED",
    "FAILED",
    "AWAITING_MERGE_APPROVAL",
];
const TASK_MANAGED_MODEL_VALUES: [&str; 3] = ["gpt-5.6-luna", "gpt-5.6-terra", "gpt-5.6-sol"];
const TASK_MANAGED_REASONING_VALUES: [&str; 6] = ["low", "medium", "high", "xhigh", "max", "ultra"];
const TASK_MANAGED_VERIFICATION_VALUES: [&str; 4] = ["NOT_STARTED", "RUNNING", "PASSED", "FAILED"];
pub(crate) const TASK_PUBLIC_OBJECTIVE_SUMMARY: &str = "Objective retained; digest only.";
const TASK_PUBLIC_STATE_VALUES: [&str; 15] = [
    "NOT_SUBMITTED",
    "DRAFT",
    "AWAITING_EXECUTION_APPROVAL",
    "PREPARING",
    "EXECUTING",
    "VERIFYING",
    "REVIEWING",
    "AWAITING_MERGE_APPROVAL",
    "MERGING",
    "COMPLETED",
    "REJECTED",
    "BLOCKED",
    "FAILED",
    "STOPPING",
    "CANCELLED",
];

pub(crate) fn task_public_objective_digest(objective: &str) -> Option<ContentDigest> {
    let domain = HashDomain::new("lattice.managed-status.objective", "1.0").ok()?;
    let digest = canonical_sha256(&domain, &CanonicalValue::String(objective.to_owned())).ok()?;
    ContentDigest::from_sha256(digest.to_hex()).ok()
}

/// Maximum encoded bytes accepted for one newline-delimited stdio message.
pub const MAX_STDIO_MESSAGE_BYTES: usize = 65_536;
/// Maximum valid tool invocations accepted during one MCP server session.
pub const MAX_TOOL_INVOCATIONS_PER_SESSION: usize = 64;
const MCP_HANDOFF_RESERVE: usize = 8;

const META_PROTOCOL_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const META_CLIENT_INFO: &str = "io.modelcontextprotocol/clientInfo";
const META_CLIENT_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
const META_LOG_LEVEL: &str = "io.modelcontextprotocol/logLevel";
const META_SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";

const ACCEPTANCE_EVIDENCE_PATH_ENV: &str = "LATTICE_MCP_ACCEPTANCE_EVIDENCE_PATH";
const ACCEPTANCE_SESSION_ID_ENV: &str = "LATTICE_MCP_ACCEPTANCE_SESSION_ID";
const ACCEPTANCE_SAFE_CONFIG_SHA256_ENV: &str = "LATTICE_MCP_ACCEPTANCE_SAFE_CONFIG_SHA256";
const OBSERVED_EFFECT_EVIDENCE_PATH_ENV: &str = "LATTICE_MCP_OBSERVED_EFFECT_PATH";
const OBSERVED_EFFECT_NONCE_ENV: &str = "LATTICE_MCP_OBSERVED_EFFECT_NONCE";
const ACCEPTANCE_EVIDENCE_SCHEMA: &str = "lattice.mcp.acceptance-dispatch.v1";
const ACCEPTANCE_EVIDENCE_HASH_DOMAIN: &str = "lattice.mcp.acceptance-dispatch-hash.v1";
const OBSERVED_EFFECT_EVIDENCE_SCHEMA: &str = "lattice.mcp.observed-effect.v1";
const OBSERVED_EFFECT_HASH_DOMAIN: &str = "lattice.mcp.observed-effect-hash.v1";
const OBSERVED_EFFECT_NONCE_DOMAIN: &str = "lattice.mcp.observed-effect-nonce.v1";
const OBSERVED_EFFECT_PROBE_DOMAIN: &str = "lattice.mcp.observed-effect-probe.v1";
const OBSERVED_EFFECT_MAX_AGE_NANOS: u128 = 15 * 60 * 1_000_000_000;

struct AcceptanceEvidence {
    file: File,
    session_id: String,
    safe_config_sha256: String,
    ordinal: u64,
    dispatch_accepted_count: u64,
    previous_event_sha256: String,
}

impl AcceptanceEvidence {
    fn from_process_environment() -> io::Result<Option<Self>> {
        let path = env::var_os(ACCEPTANCE_EVIDENCE_PATH_ENV);
        let session_id = env::var_os(ACCEPTANCE_SESSION_ID_ENV);
        let safe_config_sha256 = env::var_os(ACCEPTANCE_SAFE_CONFIG_SHA256_ENV);
        if path.is_none() && session_id.is_none() && safe_config_sha256.is_none() {
            return Ok(None);
        }
        let path = path
            .and_then(|value| value.into_string().ok())
            .ok_or_else(|| acceptance_evidence_error("incomplete or non-UTF-8 evidence path"))?;
        let session_id = session_id
            .and_then(|value| value.into_string().ok())
            .ok_or_else(|| acceptance_evidence_error("incomplete or non-UTF-8 session id"))?;
        let safe_config_sha256 = safe_config_sha256
            .and_then(|value| value.into_string().ok())
            .ok_or_else(|| acceptance_evidence_error("incomplete or non-UTF-8 safe config"))?;
        Self::open(&PathBuf::from(path), session_id, safe_config_sha256).map(Some)
    }

    fn open(path: &Path, session_id: String, safe_config_sha256: String) -> io::Result<Self> {
        if !path.is_absolute()
            || path
                .to_string_lossy()
                .to_ascii_lowercase()
                .starts_with(r"\\.\pipe\")
            || !valid_lower_hex(&session_id, 32)
            || !valid_lower_hex(&safe_config_sha256, 64)
        {
            return Err(acceptance_evidence_error("invalid evidence configuration"));
        }
        let metadata = std::fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() != 0 {
            return Err(acceptance_evidence_error(
                "evidence sink is not a fresh regular file",
            ));
        }
        let file = OpenOptions::new().append(true).open(path)?;
        if !file.metadata()?.is_file() || file.metadata()?.len() != 0 {
            return Err(acceptance_evidence_error(
                "evidence sink changed before open",
            ));
        }
        let mut evidence = Self {
            file,
            session_id,
            safe_config_sha256,
            ordinal: 0,
            dispatch_accepted_count: 0,
            previous_event_sha256: "0".repeat(64),
        };
        evidence.append("SESSION_OPEN", None, None)?;
        Ok(evidence)
    }

    fn record_dispatch(&mut self, tool_name: &str, request_id: &Value) -> io::Result<()> {
        self.dispatch_accepted_count = self
            .dispatch_accepted_count
            .checked_add(1)
            .ok_or_else(|| acceptance_evidence_error("dispatch counter overflow"))?;
        let request_id_bytes = serde_json::to_vec(request_id)
            .map_err(|error| acceptance_evidence_error(&error.to_string()))?;
        let request_id_sha256 = sha256_hex(&request_id_bytes);
        self.append(
            "DISPATCH_ACCEPTED",
            Some(tool_name),
            Some(&request_id_sha256),
        )
    }

    fn close(&mut self) -> io::Result<()> {
        self.append("SESSION_CLOSED", None, None)
    }

    fn append(
        &mut self,
        record_type: &str,
        tool_name: Option<&str>,
        request_id_sha256: Option<&str>,
    ) -> io::Result<()> {
        self.ordinal = self
            .ordinal
            .checked_add(1)
            .ok_or_else(|| acceptance_evidence_error("event ordinal overflow"))?;
        let observed_at_unix_nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| acceptance_evidence_error(&error.to_string()))?
            .as_nanos()
            .to_string();
        let tool_name_hash_field = tool_name.unwrap_or("null");
        let request_id_hash_field = request_id_sha256.unwrap_or("null");
        let ordinal = self.ordinal.to_string();
        let process_id = std::process::id().to_string();
        let dispatch_accepted_count = self.dispatch_accepted_count.to_string();
        let hash_input = [
            ACCEPTANCE_EVIDENCE_HASH_DOMAIN,
            &self.previous_event_sha256,
            &self.session_id,
            &self.safe_config_sha256,
            record_type,
            &ordinal,
            &process_id,
            tool_name_hash_field,
            request_id_hash_field,
            &dispatch_accepted_count,
            &observed_at_unix_nanos,
        ]
        .join("\n");
        let event_sha256 = sha256_hex(hash_input.as_bytes());
        let record = json!({
            "schema": ACCEPTANCE_EVIDENCE_SCHEMA,
            "record_type": record_type,
            "session_id": self.session_id,
            "safe_config_sha256": self.safe_config_sha256,
            "process_id": std::process::id(),
            "ordinal": self.ordinal,
            "tool_name": tool_name,
            "request_id_sha256": request_id_sha256,
            "dispatch_accepted_count": self.dispatch_accepted_count,
            "observed_at_unix_nanos": observed_at_unix_nanos,
            "previous_event_sha256": self.previous_event_sha256,
            "event_sha256": event_sha256,
        });
        let mut bytes = serde_json::to_vec(&record)
            .map_err(|error| acceptance_evidence_error(&error.to_string()))?;
        bytes.push(b'\n');
        self.file.write_all(&bytes)?;
        self.file.sync_all()?;
        self.previous_event_sha256 = event_sha256;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ObservedEffectCounters {
    dispatch: u64,
    database: u64,
    filesystem: u64,
    process: u64,
    network: u64,
    codex: u64,
}

impl ObservedEffectCounters {
    fn increment(&mut self, kind: ObservedEffectKind) -> io::Result<()> {
        let value = match kind {
            ObservedEffectKind::Dispatch => &mut self.dispatch,
            ObservedEffectKind::Database => &mut self.database,
            ObservedEffectKind::Filesystem => &mut self.filesystem,
            ObservedEffectKind::Process => &mut self.process,
            ObservedEffectKind::Network => &mut self.network,
            ObservedEffectKind::Codex => &mut self.codex,
        };
        *value = value
            .checked_add(1)
            .ok_or_else(|| acceptance_evidence_error("observed effect counter overflow"))?;
        Ok(())
    }

    fn as_value(self) -> Value {
        json!({
            "dispatch": self.dispatch,
            "database": self.database,
            "filesystem": self.filesystem,
            "process": self.process,
            "network": self.network,
            "codex": self.codex,
        })
    }

    fn hash_field(self) -> String {
        [
            self.dispatch.to_string(),
            self.database.to_string(),
            self.filesystem.to_string(),
            self.process.to_string(),
            self.network.to_string(),
            self.codex.to_string(),
        ]
        .join(":")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ObservedEffectKind {
    Dispatch,
    Database,
    Filesystem,
    Process,
    Network,
    Codex,
}

impl ObservedEffectKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Dispatch => "dispatch",
            Self::Database => "database",
            Self::Filesystem => "filesystem",
            Self::Process => "process",
            Self::Network => "network",
            Self::Codex => "codex",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "dispatch" => Some(Self::Dispatch),
            "database" => Some(Self::Database),
            "filesystem" => Some(Self::Filesystem),
            "process" => Some(Self::Process),
            "network" => Some(Self::Network),
            "codex" => Some(Self::Codex),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ObservedProbe {
    probe_id: String,
    tool_name: String,
    request_id_sha256: String,
    counters: ObservedEffectCounters,
}

struct ObservedEffectEvidence {
    file: File,
    path: PathBuf,
    session_id: String,
    safe_config_sha256: String,
    nonce: String,
    nonce_commitment: String,
    ordinal: u64,
    session_counters: ObservedEffectCounters,
    previous_event_sha256: String,
    probe: Option<ObservedProbe>,
    closed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct VerifiedObservedEffectEvidence {
    schema: &'static str,
    rejected_probe_count: u64,
    dispatch_count: u64,
    database_effect_count: u64,
    filesystem_effect_count: u64,
    process_effect_count: u64,
    network_effect_count: u64,
    codex_effect_count: u64,
    normal_close_complete: bool,
}

impl ObservedEffectEvidence {
    fn from_process_environment() -> io::Result<Option<Self>> {
        let path = env::var_os(OBSERVED_EFFECT_EVIDENCE_PATH_ENV);
        let nonce = env::var_os(OBSERVED_EFFECT_NONCE_ENV);
        if path.is_none() && nonce.is_none() {
            return Ok(None);
        }
        let path = path
            .and_then(|value| value.into_string().ok())
            .ok_or_else(|| acceptance_evidence_error("incomplete observed effect path"))?;
        let nonce = nonce
            .and_then(|value| value.into_string().ok())
            .ok_or_else(|| acceptance_evidence_error("incomplete observed effect nonce"))?;
        let session_id = env::var(ACCEPTANCE_SESSION_ID_ENV)
            .map_err(|_| acceptance_evidence_error("missing observed effect session"))?;
        let safe_config_sha256 = env::var(ACCEPTANCE_SAFE_CONFIG_SHA256_ENV)
            .map_err(|_| acceptance_evidence_error("missing observed effect safe config"))?;
        Self::open(&PathBuf::from(path), session_id, safe_config_sha256, nonce).map(Some)
    }

    fn open(
        path: &Path,
        session_id: String,
        safe_config_sha256: String,
        nonce: String,
    ) -> io::Result<Self> {
        if !path.is_absolute()
            || !valid_lower_hex(&session_id, 32)
            || !valid_lower_hex(&safe_config_sha256, 64)
            || !valid_lower_hex(&nonce, 64)
        {
            return Err(acceptance_evidence_error(
                "invalid observed effect configuration",
            ));
        }
        let metadata = std::fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() != 0 {
            return Err(acceptance_evidence_error(
                "observed effect sink is not a fresh regular file",
            ));
        }
        let file = OpenOptions::new().read(true).append(true).open(path)?;
        if !file.metadata()?.is_file() || file.metadata()?.len() != 0 {
            return Err(acceptance_evidence_error(
                "observed effect sink changed before open",
            ));
        }
        let nonce_commitment = sha256_hex(
            [
                OBSERVED_EFFECT_NONCE_DOMAIN,
                &session_id,
                &safe_config_sha256,
                &nonce,
            ]
            .join("\n")
            .as_bytes(),
        );
        let mut evidence = Self {
            file,
            path: path.to_path_buf(),
            session_id,
            safe_config_sha256,
            nonce,
            nonce_commitment,
            ordinal: 0,
            session_counters: ObservedEffectCounters::default(),
            previous_event_sha256: "0".repeat(64),
            probe: None,
            closed: false,
        };
        evidence.append("SESSION_OPEN", None, None)?;
        Ok(evidence)
    }

    fn begin_probe(
        &mut self,
        correlation: &str,
        tool_name: &str,
        request_id: &Value,
    ) -> io::Result<()> {
        if self.closed
            || self.probe.is_some()
            || correlation.is_empty()
            || correlation.len() > 96
            || !correlation
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            || tool_name.is_empty()
            || tool_name.len() > 64
            || !tool_name.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
            })
        {
            return Err(acceptance_evidence_error("invalid observed effect probe"));
        }
        let request_id_bytes = serde_json::to_vec(request_id)
            .map_err(|error| acceptance_evidence_error(&error.to_string()))?;
        let request_id_sha256 = sha256_hex(&request_id_bytes);
        let probe_id = sha256_hex(
            [
                OBSERVED_EFFECT_PROBE_DOMAIN,
                &self.session_id,
                correlation,
                tool_name,
                &request_id_sha256,
            ]
            .join("\n")
            .as_bytes(),
        );
        self.probe = Some(ObservedProbe {
            probe_id,
            tool_name: tool_name.to_owned(),
            request_id_sha256,
            counters: ObservedEffectCounters::default(),
        });
        self.append("PROBE_OPEN", None, None)
    }

    fn accept_dispatch(&mut self) -> io::Result<()> {
        self.increment(ObservedEffectKind::Dispatch)?;
        self.append("DISPATCH_ACCEPTED", Some("MCP_DISPATCH_ACCEPTED"), None)
    }

    fn record_effect(&mut self, kind: ObservedEffectKind) -> io::Result<()> {
        if kind == ObservedEffectKind::Dispatch {
            return Err(acceptance_evidence_error("invalid observed effect kind"));
        }
        self.increment(kind)?;
        self.append("EFFECT_OBSERVED", None, Some(kind))
    }

    fn reject_probe(&mut self, classification: &str) -> io::Result<()> {
        if !matches!(
            classification,
            "MCP_INVALID_PARAMS" | "MCP_UNKNOWN_TOOL" | "MCP_INVOCATION_LIMIT" | "MCP_NOT_READY"
        ) {
            return Err(acceptance_evidence_error(
                "invalid observed effect rejection classification",
            ));
        }
        self.append("PROBE_REJECTED", Some(classification), None)?;
        self.probe = None;
        Ok(())
    }

    fn complete_probe(&mut self, classification: &str) -> io::Result<()> {
        if !matches!(classification, "MCP_RESULT" | "MCP_TOOL_ERROR") {
            return Err(acceptance_evidence_error(
                "invalid observed effect completion classification",
            ));
        }
        self.append("PROBE_COMPLETED", Some(classification), None)?;
        self.probe = None;
        Ok(())
    }

    fn close(&mut self) -> io::Result<()> {
        if self.closed || self.probe.is_some() {
            return Err(acceptance_evidence_error(
                "observed effect session is incomplete",
            ));
        }
        self.append("SESSION_CLOSED", None, None)?;
        self.closed = true;
        self.file.sync_all()?;
        let bytes = std::fs::read(&self.path)?;
        verify_observed_effect_evidence(
            &bytes,
            &self.session_id,
            &self.safe_config_sha256,
            &self.nonce,
            SystemTime::now(),
        )?;
        Ok(())
    }

    fn increment(&mut self, kind: ObservedEffectKind) -> io::Result<()> {
        let probe = self
            .probe
            .as_mut()
            .ok_or_else(|| acceptance_evidence_error("observed effect probe is absent"))?;
        probe.counters.increment(kind)?;
        self.session_counters.increment(kind)
    }

    fn append(
        &mut self,
        record_type: &str,
        classification: Option<&str>,
        effect_kind: Option<ObservedEffectKind>,
    ) -> io::Result<()> {
        self.ordinal = self
            .ordinal
            .checked_add(1)
            .ok_or_else(|| acceptance_evidence_error("observed effect ordinal overflow"))?;
        let observed_at_unix_nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| acceptance_evidence_error(&error.to_string()))?
            .as_nanos()
            .to_string();
        let (probe_id, tool_name, request_id_sha256, probe_counters) = self.probe.as_ref().map_or(
            (None, None, None, ObservedEffectCounters::default()),
            |probe| {
                (
                    Some(probe.probe_id.as_str()),
                    Some(probe.tool_name.as_str()),
                    Some(probe.request_id_sha256.as_str()),
                    probe.counters,
                )
            },
        );
        let event_sha256 = observed_effect_event_sha256(
            &self.nonce,
            &self.previous_event_sha256,
            &self.session_id,
            &self.safe_config_sha256,
            &self.nonce_commitment,
            self.ordinal,
            record_type,
            probe_id,
            tool_name,
            request_id_sha256,
            classification,
            effect_kind,
            probe_counters,
            self.session_counters,
            &observed_at_unix_nanos,
        );
        let record = json!({
            "schema": OBSERVED_EFFECT_EVIDENCE_SCHEMA,
            "record_type": record_type,
            "session_id": self.session_id,
            "probe_id": probe_id,
            "safe_config_sha256": self.safe_config_sha256,
            "nonce_commitment": self.nonce_commitment,
            "process_id": std::process::id(),
            "ordinal": self.ordinal,
            "tool_name": tool_name,
            "request_id_sha256": request_id_sha256,
            "classification": classification,
            "effect_kind": effect_kind.map(ObservedEffectKind::as_str),
            "probe_counters": probe_counters.as_value(),
            "session_counters": self.session_counters.as_value(),
            "observed_at_unix_nanos": observed_at_unix_nanos,
            "previous_event_sha256": self.previous_event_sha256,
            "event_sha256": event_sha256,
        });
        let mut bytes = serde_json::to_vec(&record)
            .map_err(|error| acceptance_evidence_error(&error.to_string()))?;
        bytes.push(b'\n');
        self.file.write_all(&bytes)?;
        self.file.sync_all()?;
        self.previous_event_sha256 = event_sha256;
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
fn observed_effect_event_sha256(
    nonce: &str,
    previous_event_sha256: &str,
    session_id: &str,
    safe_config_sha256: &str,
    nonce_commitment: &str,
    ordinal: u64,
    record_type: &str,
    probe_id: Option<&str>,
    tool_name: Option<&str>,
    request_id_sha256: Option<&str>,
    classification: Option<&str>,
    effect_kind: Option<ObservedEffectKind>,
    probe_counters: ObservedEffectCounters,
    session_counters: ObservedEffectCounters,
    observed_at_unix_nanos: &str,
) -> String {
    hmac_sha256_hex(
        nonce.as_bytes(),
        [
            OBSERVED_EFFECT_HASH_DOMAIN,
            previous_event_sha256,
            session_id,
            safe_config_sha256,
            nonce_commitment,
            &ordinal.to_string(),
            record_type,
            probe_id.unwrap_or("null"),
            tool_name.unwrap_or("null"),
            request_id_sha256.unwrap_or("null"),
            classification.unwrap_or("null"),
            effect_kind.map_or("null", ObservedEffectKind::as_str),
            &probe_counters.hash_field(),
            &session_counters.hash_field(),
            observed_at_unix_nanos,
        ]
        .join("\n")
        .as_bytes(),
    )
}

fn counters_from_value(value: &Value) -> io::Result<ObservedEffectCounters> {
    let object = value
        .as_object()
        .ok_or_else(|| acceptance_evidence_error("invalid observed effect counters"))?;
    if object.len() != 6
        || ![
            "dispatch",
            "database",
            "filesystem",
            "process",
            "network",
            "codex",
        ]
        .iter()
        .all(|key| object.contains_key(*key))
    {
        return Err(acceptance_evidence_error(
            "invalid observed effect counter keys",
        ));
    }
    let read = |key: &str| {
        object
            .get(key)
            .and_then(Value::as_u64)
            .ok_or_else(|| acceptance_evidence_error("invalid observed effect counter"))
    };
    Ok(ObservedEffectCounters {
        dispatch: read("dispatch")?,
        database: read("database")?,
        filesystem: read("filesystem")?,
        process: read("process")?,
        network: read("network")?,
        codex: read("codex")?,
    })
}

fn nullable_string<'a>(record: &'a Map<String, Value>, key: &str) -> io::Result<Option<&'a str>> {
    match record.get(key) {
        Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value)),
        _ => Err(acceptance_evidence_error(
            "invalid observed effect nullable string",
        )),
    }
}

fn verify_observed_effect_evidence(
    bytes: &[u8],
    expected_session_id: &str,
    expected_safe_config_sha256: &str,
    nonce: &str,
    now: SystemTime,
) -> io::Result<VerifiedObservedEffectEvidence> {
    if bytes.is_empty()
        || bytes.len() > 1_048_576
        || bytes.starts_with(&[0xef, 0xbb, 0xbf])
        || bytes.last() != Some(&b'\n')
        || bytes.contains(&b'\r')
        || !valid_lower_hex(expected_session_id, 32)
        || !valid_lower_hex(expected_safe_config_sha256, 64)
        || !valid_lower_hex(nonce, 64)
    {
        return Err(acceptance_evidence_error(
            "invalid observed effect evidence bytes",
        ));
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|error| acceptance_evidence_error(&error.to_string()))?;
    let expected_nonce_commitment = sha256_hex(
        [
            OBSERVED_EFFECT_NONCE_DOMAIN,
            expected_session_id,
            expected_safe_config_sha256,
            nonce,
        ]
        .join("\n")
        .as_bytes(),
    );
    let now_nanos = now
        .duration_since(UNIX_EPOCH)
        .map_err(|error| acceptance_evidence_error(&error.to_string()))?
        .as_nanos();
    let mut previous_event_sha256 = "0".repeat(64);
    let mut previous_observed_nanos = 0_u128;
    let mut expected_session_counters = ObservedEffectCounters::default();
    let mut expected_probe_counters = ObservedEffectCounters::default();
    let mut active_probe = false;
    let mut saw_open = false;
    let mut saw_close = false;
    let mut rejected_probe_count = 0_u64;
    let mut observed_probe_ids = HashSet::new();
    let lines = text.lines().collect::<Vec<_>>();
    if lines.len() < 2 {
        return Err(acceptance_evidence_error(
            "observed effect evidence is incomplete",
        ));
    }
    for (index, line) in lines.iter().enumerate() {
        let value = serde_json::from_str::<Value>(line)
            .map_err(|error| acceptance_evidence_error(&error.to_string()))?;
        let record = value
            .as_object()
            .ok_or_else(|| acceptance_evidence_error("invalid observed effect record"))?;
        let expected_keys = [
            "schema",
            "record_type",
            "session_id",
            "probe_id",
            "safe_config_sha256",
            "nonce_commitment",
            "process_id",
            "ordinal",
            "tool_name",
            "request_id_sha256",
            "classification",
            "effect_kind",
            "probe_counters",
            "session_counters",
            "observed_at_unix_nanos",
            "previous_event_sha256",
            "event_sha256",
        ];
        if record.len() != expected_keys.len()
            || !expected_keys.iter().all(|key| record.contains_key(*key))
        {
            return Err(acceptance_evidence_error(
                "invalid observed effect record keys",
            ));
        }
        let string = |key: &str| {
            record
                .get(key)
                .and_then(Value::as_str)
                .ok_or_else(|| acceptance_evidence_error("invalid observed effect string"))
        };
        let record_type = string("record_type")?;
        let ordinal = record
            .get("ordinal")
            .and_then(Value::as_u64)
            .ok_or_else(|| acceptance_evidence_error("invalid observed effect ordinal"))?;
        let observed_at_unix_nanos = string("observed_at_unix_nanos")?;
        let observed_nanos = observed_at_unix_nanos
            .parse::<u128>()
            .map_err(|error| acceptance_evidence_error(&error.to_string()))?;
        if string("schema")? != OBSERVED_EFFECT_EVIDENCE_SCHEMA
            || string("session_id")? != expected_session_id
            || string("safe_config_sha256")? != expected_safe_config_sha256
            || string("nonce_commitment")? != expected_nonce_commitment
            || record.get("process_id").and_then(Value::as_u64)
                != Some(u64::from(std::process::id()))
            || ordinal != (index as u64 + 1)
            || observed_nanos < previous_observed_nanos
            || observed_nanos > now_nanos.saturating_add(30_000_000_000)
            || now_nanos.saturating_sub(observed_nanos) > OBSERVED_EFFECT_MAX_AGE_NANOS
            || string("previous_event_sha256")? != previous_event_sha256
        {
            return Err(acceptance_evidence_error(
                "observed effect binding rejected",
            ));
        }
        let probe_id = nullable_string(record, "probe_id")?;
        let tool_name = nullable_string(record, "tool_name")?;
        let request_id_sha256 = nullable_string(record, "request_id_sha256")?;
        let classification = nullable_string(record, "classification")?;
        let effect_kind_text = nullable_string(record, "effect_kind")?;
        let effect_kind = effect_kind_text.and_then(ObservedEffectKind::parse);
        if effect_kind_text.is_some() && effect_kind.is_none() {
            return Err(acceptance_evidence_error("unknown observed effect kind"));
        }
        let closes_probe = match record_type {
            "SESSION_OPEN" if index == 0 && !saw_open => {
                if classification.is_some() || effect_kind.is_some() {
                    return Err(acceptance_evidence_error(
                        "invalid observed effect session open fields",
                    ));
                }
                saw_open = true;
                false
            }
            "PROBE_OPEN" if saw_open && !active_probe && !saw_close => {
                if classification.is_some() || effect_kind.is_some() {
                    return Err(acceptance_evidence_error(
                        "invalid observed effect probe open fields",
                    ));
                }
                let probe_id = probe_id
                    .ok_or_else(|| acceptance_evidence_error("missing observed effect probe id"))?;
                if !observed_probe_ids.insert(probe_id.to_owned()) {
                    return Err(acceptance_evidence_error(
                        "duplicate observed effect probe rejected",
                    ));
                }
                active_probe = true;
                expected_probe_counters = ObservedEffectCounters::default();
                false
            }
            "DISPATCH_ACCEPTED" if active_probe => {
                if classification != Some("MCP_DISPATCH_ACCEPTED") || effect_kind.is_some() {
                    return Err(acceptance_evidence_error(
                        "invalid observed effect dispatch fields",
                    ));
                }
                expected_probe_counters.increment(ObservedEffectKind::Dispatch)?;
                expected_session_counters.increment(ObservedEffectKind::Dispatch)?;
                if expected_probe_counters.dispatch != 1 {
                    return Err(acceptance_evidence_error(
                        "duplicate observed effect dispatch rejected",
                    ));
                }
                false
            }
            "EFFECT_OBSERVED" if active_probe => {
                if classification.is_some() {
                    return Err(acceptance_evidence_error(
                        "invalid observed effect classification",
                    ));
                }
                let kind = effect_kind
                    .ok_or_else(|| acceptance_evidence_error("missing observed effect kind"))?;
                if kind == ObservedEffectKind::Dispatch {
                    return Err(acceptance_evidence_error(
                        "dispatch encoded as external effect",
                    ));
                }
                expected_probe_counters.increment(kind)?;
                expected_session_counters.increment(kind)?;
                false
            }
            "PROBE_REJECTED" if active_probe => {
                if !matches!(
                    classification,
                    Some(
                        "MCP_INVALID_PARAMS"
                            | "MCP_UNKNOWN_TOOL"
                            | "MCP_INVOCATION_LIMIT"
                            | "MCP_NOT_READY"
                    )
                ) || effect_kind.is_some()
                    || expected_probe_counters != ObservedEffectCounters::default()
                {
                    return Err(acceptance_evidence_error(
                        "invalid rejected probe classification or counters",
                    ));
                }
                rejected_probe_count = rejected_probe_count
                    .checked_add(1)
                    .ok_or_else(|| acceptance_evidence_error("rejected probe overflow"))?;
                true
            }
            "PROBE_COMPLETED"
                if active_probe
                    && matches!(classification, Some("MCP_RESULT" | "MCP_TOOL_ERROR"))
                    && effect_kind.is_none()
                    && expected_probe_counters.dispatch == 1 =>
            {
                true
            }
            "SESSION_CLOSED" if index == lines.len() - 1 && !active_probe && !saw_close => {
                if classification.is_some() || effect_kind.is_some() {
                    return Err(acceptance_evidence_error(
                        "invalid observed effect session close fields",
                    ));
                }
                saw_close = true;
                false
            }
            _ => {
                return Err(acceptance_evidence_error(
                    "observed effect record order rejected",
                ));
            }
        };
        let expected_probe_binding =
            active_probe || matches!(record_type, "PROBE_REJECTED" | "PROBE_COMPLETED");
        if expected_probe_binding {
            if probe_id.is_none()
                || tool_name.is_none()
                || request_id_sha256.is_none_or(|value| !valid_lower_hex(value, 64))
            {
                return Err(acceptance_evidence_error(
                    "observed effect probe binding rejected",
                ));
            }
        } else if probe_id.is_some() || tool_name.is_some() || request_id_sha256.is_some() {
            return Err(acceptance_evidence_error(
                "unexpected observed effect probe binding",
            ));
        }
        let probe_counters = counters_from_value(
            record
                .get("probe_counters")
                .ok_or_else(|| acceptance_evidence_error("missing probe counters"))?,
        )?;
        let session_counters = counters_from_value(
            record
                .get("session_counters")
                .ok_or_else(|| acceptance_evidence_error("missing session counters"))?,
        )?;
        if probe_counters != expected_probe_counters
            || session_counters != expected_session_counters
        {
            return Err(acceptance_evidence_error(
                "observed effect counters rejected",
            ));
        }
        let event_sha256 = observed_effect_event_sha256(
            nonce,
            &previous_event_sha256,
            expected_session_id,
            expected_safe_config_sha256,
            &expected_nonce_commitment,
            ordinal,
            record_type,
            probe_id,
            tool_name,
            request_id_sha256,
            classification,
            effect_kind,
            probe_counters,
            session_counters,
            observed_at_unix_nanos,
        );
        if string("event_sha256")? != event_sha256 {
            return Err(acceptance_evidence_error(
                "observed effect hash chain rejected",
            ));
        }
        previous_event_sha256 = event_sha256;
        previous_observed_nanos = observed_nanos;
        if closes_probe {
            active_probe = false;
            expected_probe_counters = ObservedEffectCounters::default();
        }
    }
    if !saw_open || !saw_close || active_probe {
        return Err(acceptance_evidence_error(
            "observed effect evidence did not close",
        ));
    }
    Ok(VerifiedObservedEffectEvidence {
        schema: OBSERVED_EFFECT_EVIDENCE_SCHEMA,
        rejected_probe_count,
        dispatch_count: expected_session_counters.dispatch,
        database_effect_count: expected_session_counters.database,
        filesystem_effect_count: expected_session_counters.filesystem,
        process_effect_count: expected_session_counters.process,
        network_effect_count: expected_session_counters.network,
        codex_effect_count: expected_session_counters.codex,
        normal_close_complete: true,
    })
}

fn acceptance_evidence_error(message: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("LATTICE_MCP_ACCEPTANCE_EVIDENCE_REJECTED:{message}"),
    )
}

fn valid_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}

fn hmac_sha256_hex(key: &[u8], message: &[u8]) -> String {
    const BLOCK_BYTES: usize = 64;
    let mut normalized = [0_u8; BLOCK_BYTES];
    if key.len() > BLOCK_BYTES {
        normalized[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        normalized[..key.len()].copy_from_slice(key);
    }
    let mut inner_pad = [0x36_u8; BLOCK_BYTES];
    let mut outer_pad = [0x5c_u8; BLOCK_BYTES];
    for index in 0..BLOCK_BYTES {
        inner_pad[index] ^= normalized[index];
        outer_pad[index] ^= normalized[index];
    }
    let mut inner = Sha256::new();
    inner.update(inner_pad);
    inner.update(message);
    let inner_digest = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner_digest);
    let digest = outer.finalize();
    let mut output = String::with_capacity(64);
    for byte in digest {
        write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}

thread_local! {
    static OBSERVED_EFFECT_EVIDENCE: RefCell<Option<ObservedEffectEvidence>> = const {
        RefCell::new(None)
    };
}

fn install_observed_effect_evidence() -> io::Result<bool> {
    let evidence = ObservedEffectEvidence::from_process_environment()?;
    let enabled = evidence.is_some();
    OBSERVED_EFFECT_EVIDENCE.with(|slot| {
        let mut slot = slot
            .try_borrow_mut()
            .map_err(|_| acceptance_evidence_error("observed effect recorder is busy"))?;
        if slot.is_some() {
            return Err(acceptance_evidence_error(
                "observed effect recorder already installed",
            ));
        }
        *slot = evidence;
        Ok(enabled)
    })
}

fn with_observed_effect_evidence(
    operation: impl FnOnce(&mut ObservedEffectEvidence) -> io::Result<()>,
) -> io::Result<()> {
    OBSERVED_EFFECT_EVIDENCE.with(|slot| {
        let mut slot = slot
            .try_borrow_mut()
            .map_err(|_| acceptance_evidence_error("observed effect recorder is busy"))?;
        match slot.as_mut() {
            Some(evidence) => operation(evidence),
            None => Ok(()),
        }
    })
}

fn close_observed_effect_evidence(enabled: bool) -> io::Result<()> {
    if !enabled {
        return Ok(());
    }
    OBSERVED_EFFECT_EVIDENCE.with(|slot| {
        let mut slot = slot
            .try_borrow_mut()
            .map_err(|_| acceptance_evidence_error("observed effect recorder is busy"))?;
        let mut evidence = slot
            .take()
            .ok_or_else(|| acceptance_evidence_error("observed effect recorder is absent"))?;
        evidence.close()
    })
}

pub(crate) fn record_observed_effect(kind: ObservedEffectKind) -> io::Result<()> {
    with_observed_effect_evidence(|evidence| evidence.record_effect(kind))
}

/// Returns the process-owned commitment to the authorization-relevant MCP
/// protocol and closed tool schemas. Descriptions are intentionally excluded;
/// the adapter binary digest independently commits the complete executable.
pub(crate) fn task_ingress_schema_digest() -> Option<ContentDigest> {
    let value = CanonicalValue::Object(vec![
        (
            "legacy_protocol".to_owned(),
            CanonicalValue::String(MCP_PROTOCOL_VERSION.to_owned()),
        ),
        (
            "stateless_protocol".to_owned(),
            CanonicalValue::String(MCP_STATELESS_PROTOCOL_VERSION.to_owned()),
        ),
        (
            "delivery_tools".to_owned(),
            CanonicalValue::Array(vec![
                CanonicalValue::String(DELIVERY_RUN_TOOL.to_owned()),
                CanonicalValue::String(DELIVERY_STATUS_TOOL.to_owned()),
                CanonicalValue::String(RUNTIME_STATUS_TOOL.to_owned()),
                CanonicalValue::String(DELIVERY_RECONCILE_TOOL.to_owned()),
            ]),
        ),
        (
            "delivery_schema".to_owned(),
            CanonicalValue::String("closed-empty-object".to_owned()),
        ),
        (
            "task_submit_tool".to_owned(),
            CanonicalValue::String(TASK_SUBMIT_TOOL.to_owned()),
        ),
        (
            "task_submit_schema".to_owned(),
            CanonicalValue::String(format!(
                "closed:v3:client_request_id:ascii-control-id:no-secret:1..={MAX_CLIENT_REQUEST_ID_BYTES};legacy-intent:{CONTROLLED_CODEX_CANARY_INTENT}|general-objective-or-intent:nfc-no-control-no-secret:chars:1..={MAX_TASK_OBJECTIVE_CHARS}:utf8-bytes:1..={MAX_TASK_OBJECTIVE_BYTES};optional-selector:zero-or-one:project_id:canonical:bytes:2..={MAX_PROJECT_ID_BYTES}|project_name:chars:1..={MAX_PROJECT_NAME_CHARS}:utf8-bytes:1..={MAX_PROJECT_NAME_BYTES}|external-verified-adoption:{ADOPT_VERIFIED_RESULT_INTENT}:task_ref+expected_head+source_sha+target_sha+four-evidence-refs+approval-refs:1..=8"
            )),
        ),
        (
            "task_status_tool".to_owned(),
            CanonicalValue::String(TASK_STATUS_TOOL.to_owned()),
        ),
        (
            "task_status_schema".to_owned(),
            CanonicalValue::String(format!(
                "closed:v2:optional-client_request_id:ascii-control-id:no-secret:1..={MAX_CLIENT_REQUEST_ID_BYTES};task_ref:lower-sha256"
            )),
        ),
        (
            "task_output_schema".to_owned(),
            CanonicalValue::String(
                "closed:v2-legacy|v5-general-create-only-redacted|v6-external-verified-adoption|v4-managed-general;v2-status:NOT_SUBMITTED|RECONCILIATION_REQUIRED|FAILED|COMPLETED;v2-task_state:NOT_SUBMITTED|DRAFT|AWAITING_EXECUTION_APPROVAL|PREPARING|EXECUTING|VERIFYING|REVIEWING|AWAITING_MERGE_APPROVAL|MERGING|COMPLETED|REJECTED|BLOCKED|FAILED|STOPPING|CANCELLED;v5-status:SUBMITTED;v5-task_state:DRAFT;v6-status:COMPLETED;v6-task_state:COMPLETED;v6-result_digest:lower-sha256;v4-status:SUBMITTED|RUNNING|BLOCKED|FAILED|AWAITING_MERGE_APPROVAL;v4-task_state:existing-closed-enum;task_ref:lower-sha256;ledger_head_digest:lower-sha256;v2-result_digest:lower-sha256|null;v2-failure_stage:upper-underscore|null;v2-failure_code:upper-underscore|null;v5-result_digest:null;v5-failure_stage:null;v5-failure_code:null;v4-v5-v6:objective_summary:fixed-redacted|objective_digest:lower-sha256|project_id|project_name|project_snapshot_id;v4:worker_running:bool|attempt:null-or-1..3|retry_count:0..2|model:null-or-gpt-5.6-luna-terra-sol|reasoning:null-or-low-medium-high-xhigh-max-ultra|thread_id:null-or-safe-1..256|turn_id:null-or-safe-1..256|last_progress_at:null-or-canonical-utc|blocker:null-or-upper-underscore-1..128|verification_status:NOT_STARTED-RUNNING-PASSED-FAILED|verification_digest:null-or-lower-sha256|evidence_digest:null-or-lower-sha256|resource_observation:null-or-closed-task-cumulative-11-field-nonnegative-token-budget-object-unavailable-cost|next_action:safe-text-1..256|foreman_generation:>=1|foreman_checkpoint_digest:lower-sha256"
                    .to_owned(),
            ),
        ),
    ]);
    let domain = HashDomain::new("lattice.mcp.task-ingress-schema", "1.0").ok()?;
    let digest = canonical_sha256(&domain, &value).ok()?;
    ContentDigest::from_sha256(digest.to_hex()).ok()
}

/// Bounded execution failure safe for an MCP tool result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolExecutionError {
    code: &'static str,
}

impl ToolExecutionError {
    /// Constructs one static, secret-free failure.
    #[must_use]
    pub const fn new(code: &'static str) -> Self {
        Self { code }
    }

    #[must_use]
    pub const fn code(self) -> &'static str {
        self.code
    }
}

impl fmt::Display for ToolExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code)
    }
}

impl Error for ToolExecutionError {}

/// Exact fixed-task selector injected by composition for both MCP tools.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliveryToolArguments {
    binding: SubjectBinding,
}

impl DeliveryToolArguments {
    pub(crate) const fn new(binding: SubjectBinding) -> Self {
        Self { binding }
    }

    /// Returns the fully typed immutable task binding selected by composition.
    #[must_use]
    pub const fn binding(&self) -> &SubjectBinding {
        &self.binding
    }
}

/// Validated high-level task request accepted by the MCP transport boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskSubmitArguments {
    client_request_id: String,
    intent: String,
    objective: Option<String>,
    project_id: Option<String>,
    project_name: Option<String>,
    verified_result_adoption: Option<VerifiedResultAdoptionArguments>,
}

/// Closed, bounded evidence pointers for one externally verified result adoption.
///
/// Every reference is an opaque descriptor digest. No evidence payload, path,
/// command, credential, or caller-selected lifecycle action crosses the MCP
/// transport boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedResultAdoptionArguments {
    task_ref: String,
    expected_ledger_head_digest: String,
    source_sha: String,
    target_sha: String,
    push_merge_receipt_ref: String,
    deployment_receipt_ref: String,
    deployment_artifact_ref: String,
    independent_acceptance_ref: String,
    protected_action_approval_refs: Vec<String>,
}

impl VerifiedResultAdoptionArguments {
    #[must_use]
    pub fn task_ref(&self) -> &str {
        &self.task_ref
    }

    #[must_use]
    pub fn expected_ledger_head_digest(&self) -> &str {
        &self.expected_ledger_head_digest
    }

    #[must_use]
    pub fn source_sha(&self) -> &str {
        &self.source_sha
    }

    #[must_use]
    pub fn target_sha(&self) -> &str {
        &self.target_sha
    }

    #[must_use]
    pub fn push_merge_receipt_ref(&self) -> &str {
        &self.push_merge_receipt_ref
    }

    #[must_use]
    pub fn deployment_receipt_ref(&self) -> &str {
        &self.deployment_receipt_ref
    }

    #[must_use]
    pub fn deployment_artifact_ref(&self) -> &str {
        &self.deployment_artifact_ref
    }

    #[must_use]
    pub fn independent_acceptance_ref(&self) -> &str {
        &self.independent_acceptance_ref
    }

    #[must_use]
    pub fn protected_action_approval_refs(&self) -> &[String] {
        &self.protected_action_approval_refs
    }
}

impl TaskSubmitArguments {
    fn from_value(value: Option<&Value>) -> Option<Self> {
        let arguments = value?.as_object()?;
        let client_request_id = arguments.get("client_request_id")?.as_str()?;
        if !valid_client_request_id(client_request_id) {
            return None;
        }
        let intent_value = arguments.get("intent").and_then(Value::as_str);
        let objective_value = arguments.get("objective").and_then(Value::as_str);
        if arguments.contains_key("intent") != intent_value.is_some()
            || arguments.contains_key("objective") != objective_value.is_some()
            || intent_value.is_some() == objective_value.is_some()
        {
            return None;
        }
        if intent_value == Some(CONTROLLED_CODEX_CANARY_INTENT) {
            if arguments.len() != 2
                || arguments
                    .keys()
                    .any(|key| !matches!(key.as_str(), "client_request_id" | "intent"))
            {
                return None;
            }
            return Some(Self {
                client_request_id: client_request_id.to_owned(),
                intent: CONTROLLED_CODEX_CANARY_INTENT.to_owned(),
                objective: None,
                project_id: None,
                project_name: None,
                verified_result_adoption: None,
            });
        }

        if intent_value == Some(ADOPT_VERIFIED_RESULT_INTENT) {
            let exact_fields = [
                "client_request_id",
                "intent",
                "task_ref",
                "expected_ledger_head_digest",
                "source_sha",
                "target_sha",
                "push_merge_receipt_ref",
                "deployment_receipt_ref",
                "deployment_artifact_ref",
                "independent_acceptance_ref",
                "protected_action_approval_refs",
            ];
            if arguments.len() != exact_fields.len()
                || exact_fields
                    .iter()
                    .any(|field| !arguments.contains_key(*field))
            {
                return None;
            }
            let text = |field: &str| arguments.get(field).and_then(Value::as_str);
            let task_ref = text("task_ref")?;
            let expected_ledger_head_digest = text("expected_ledger_head_digest")?;
            let source_sha = text("source_sha")?;
            let target_sha = text("target_sha")?;
            let push_merge_receipt_ref = text("push_merge_receipt_ref")?;
            let deployment_receipt_ref = text("deployment_receipt_ref")?;
            let deployment_artifact_ref = text("deployment_artifact_ref")?;
            let independent_acceptance_ref = text("independent_acceptance_ref")?;
            let approvals = arguments
                .get("protected_action_approval_refs")?
                .as_array()?;
            if !valid_task_ref(task_ref)
                || !valid_task_ref(expected_ledger_head_digest)
                || !valid_git_sha(source_sha)
                || !valid_git_sha(target_sha)
                || !valid_evidence_ref(push_merge_receipt_ref)
                || !valid_evidence_ref(deployment_receipt_ref)
                || !valid_evidence_ref(deployment_artifact_ref)
                || !valid_evidence_ref(independent_acceptance_ref)
                || !(1..=8).contains(&approvals.len())
            {
                return None;
            }
            let protected_action_approval_refs = approvals
                .iter()
                .map(Value::as_str)
                .collect::<Option<Vec<_>>>()?;
            if protected_action_approval_refs
                .iter()
                .any(|value| !valid_evidence_ref(value))
                || protected_action_approval_refs
                    .iter()
                    .collect::<HashSet<_>>()
                    .len()
                    != protected_action_approval_refs.len()
            {
                return None;
            }
            return Some(Self {
                client_request_id: client_request_id.to_owned(),
                intent: ADOPT_VERIFIED_RESULT_INTENT.to_owned(),
                objective: None,
                project_id: None,
                project_name: None,
                verified_result_adoption: Some(VerifiedResultAdoptionArguments {
                    task_ref: task_ref.to_owned(),
                    expected_ledger_head_digest: expected_ledger_head_digest.to_owned(),
                    source_sha: source_sha.to_owned(),
                    target_sha: target_sha.to_owned(),
                    push_merge_receipt_ref: push_merge_receipt_ref.to_owned(),
                    deployment_receipt_ref: deployment_receipt_ref.to_owned(),
                    deployment_artifact_ref: deployment_artifact_ref.to_owned(),
                    independent_acceptance_ref: independent_acceptance_ref.to_owned(),
                    protected_action_approval_refs: protected_action_approval_refs
                        .into_iter()
                        .map(ToOwned::to_owned)
                        .collect(),
                }),
            });
        }

        let objective = objective_value.or(intent_value)?;
        if !valid_task_objective(objective)
            || arguments.len() > 3
            || arguments.keys().any(|key| {
                !matches!(
                    key.as_str(),
                    "client_request_id" | "intent" | "objective" | "project_id" | "project_name"
                )
            })
        {
            return None;
        }
        let project_id = arguments.get("project_id").and_then(Value::as_str);
        let project_name = arguments.get("project_name").and_then(Value::as_str);
        if arguments.contains_key("project_id") != project_id.is_some()
            || arguments.contains_key("project_name") != project_name.is_some()
            || (project_id.is_some() && project_name.is_some())
            || project_id.is_some_and(|value| !valid_project_id(value))
            || project_name.is_some_and(|value| !valid_project_name(value))
        {
            return None;
        }
        Some(Self {
            client_request_id: client_request_id.to_owned(),
            intent: objective.to_owned(),
            objective: Some(objective.to_owned()),
            project_id: project_id.map(ToOwned::to_owned),
            project_name: project_name.map(ToOwned::to_owned),
            verified_result_adoption: None,
        })
    }

    /// Returns the bounded idempotency key supplied by the MCP client.
    #[must_use]
    pub fn client_request_id(&self) -> &str {
        &self.client_request_id
    }

    /// Returns the one high-level task intent admitted by this transport slice.
    #[must_use]
    pub fn intent(&self) -> &str {
        &self.intent
    }

    /// Distinguishes the retained execution canary from a create-only objective.
    #[must_use]
    pub fn is_controlled_canary(&self) -> bool {
        self.objective.is_none() && self.verified_result_adoption.is_none()
    }

    /// Returns the validated natural-language objective for create-only intake.
    #[must_use]
    pub fn objective(&self) -> Option<&str> {
        self.objective.as_deref()
    }

    /// Returns an optional exact Control catalog identifier. It is a locator,
    /// never a caller-supplied path or Registry authority claim.
    #[must_use]
    pub fn project_id(&self) -> Option<&str> {
        self.project_id.as_deref()
    }

    /// Returns an optional exact Control catalog display-name selector.
    #[must_use]
    pub fn project_name(&self) -> Option<&str> {
        self.project_name.as_deref()
    }

    /// Returns the typed externally verified-result adoption payload, if selected.
    #[must_use]
    pub const fn verified_result_adoption(&self) -> Option<&VerifiedResultAdoptionArguments> {
        self.verified_result_adoption.as_ref()
    }
}

/// Validated durable task reference accepted by the MCP transport boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskStatusArguments {
    client_request_id: Option<String>,
    task_ref: String,
}

/// Validated closed checkpoint request accepted by the MCP boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForemanCheckpointArguments {
    intent: ForemanCheckpointIntent,
}

impl ForemanCheckpointArguments {
    fn from_value(value: Option<&Value>) -> Option<Self> {
        let arguments = value?.as_object()?;
        if arguments.len() != 7
            || ![
                "checkpoint_id",
                "generation",
                "occurred_at",
                "state",
                "blocker_ref",
                "heartbeat_ref",
                "evidence_ref",
            ]
            .iter()
            .all(|field| arguments.contains_key(*field))
        {
            return None;
        }
        let (blocker_ref, dependency_evidence_ref) = match arguments.get("blocker_ref")? {
            Value::Null => (None, None),
            Value::String(value) => (Some(value.clone()), None),
            Value::Object(dependency) => {
                if dependency.len() != 7
                    || ![
                        "schema",
                        "parent_task_id",
                        "dependency_task_id",
                        "dependency_worktree_id",
                        "dependency_branch",
                        "base_sha",
                        "next_action",
                    ]
                    .iter()
                    .all(|field| dependency.contains_key(*field))
                    || dependency.get("schema")?.as_str()? != "lattice.dependency-blocker/1.0"
                {
                    return None;
                }
                let binding = DependencyBinding::new(
                    dependency.get("parent_task_id")?.as_str()?,
                    dependency.get("dependency_task_id")?.as_str()?,
                    dependency.get("dependency_worktree_id")?.as_str()?,
                    dependency.get("dependency_branch")?.as_str()?,
                    dependency.get("base_sha")?.as_str()?,
                    dependency.get("next_action")?.as_str()?,
                )
                .ok()?;
                (
                    Some(binding.as_blocker_ref().to_owned()),
                    Some(binding.evidence_ref().to_owned()),
                )
            }
            _ => return None,
        };
        let supplied_evidence_ref = arguments.get("evidence_ref")?.as_str()?;
        let supplied_evidence_digest = supplied_evidence_ref.strip_prefix("evidence:sha256:")?;
        if supplied_evidence_digest.len() != 64
            || !supplied_evidence_digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return None;
        }
        let evidence_ref = dependency_evidence_ref
            .as_deref()
            .unwrap_or(supplied_evidence_ref);
        let intent = ForemanCheckpointIntent::new(
            arguments.get("checkpoint_id")?.as_str()?,
            arguments.get("generation")?.as_u64()?,
            arguments.get("occurred_at")?.as_str()?,
            ForemanState::from_persisted(arguments.get("state")?.as_str()?).ok()?,
            blocker_ref,
            arguments.get("heartbeat_ref")?.as_str()?,
            evidence_ref,
        )
        .ok()?;
        Some(Self { intent })
    }

    #[must_use]
    pub const fn intent(&self) -> &ForemanCheckpointIntent {
        &self.intent
    }
}

impl TaskStatusArguments {
    fn from_value(value: Option<&Value>) -> Option<Self> {
        let arguments = value?.as_object()?;
        if !(arguments.len() == 1 || arguments.len() == 2)
            || !arguments.contains_key("task_ref")
            || arguments
                .keys()
                .any(|key| !matches!(key.as_str(), "client_request_id" | "task_ref"))
        {
            return None;
        }
        let client_request_id = arguments.get("client_request_id").and_then(Value::as_str);
        let task_ref = arguments.get("task_ref")?.as_str()?;
        if arguments.contains_key("client_request_id") != client_request_id.is_some()
            || client_request_id.is_some_and(|value| !valid_client_request_id(value))
            || !valid_task_ref(task_ref)
        {
            return None;
        }
        Some(Self {
            client_request_id: client_request_id.map(ToOwned::to_owned),
            task_ref: task_ref.to_owned(),
        })
    }

    /// Returns the optional legacy idempotency key used to reconstruct a canary.
    #[must_use]
    pub fn client_request_id(&self) -> Option<&str> {
        self.client_request_id.as_deref()
    }

    /// Returns the exact lowercase SHA-256 task reference.
    #[must_use]
    pub fn task_ref(&self) -> &str {
        &self.task_ref
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TaskPublicStatus {
    schema_version: String,
    status: String,
    task_state: String,
    task_ref: String,
    ledger_head_digest: String,
    result_digest: Option<String>,
    failure_stage: Option<String>,
    failure_code: Option<String>,
    objective: Option<String>,
    project_id: Option<String>,
    project_name: Option<String>,
    project_snapshot_id: Option<String>,
    redacted_objective: Option<RedactedTaskObjective>,
    managed: Option<ManagedTaskPublicStatus>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RedactedTaskObjective {
    summary: String,
    digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ManagedTaskPublicStatus {
    objective_summary: String,
    objective_digest: String,
    worker_running: bool,
    attempt: Option<u64>,
    retry_count: u64,
    model: Option<String>,
    reasoning: Option<String>,
    thread_id: Option<String>,
    turn_id: Option<String>,
    last_progress_at: Option<String>,
    blocker: Option<String>,
    verification_status: String,
    verification_digest: Option<String>,
    evidence_digest: Option<String>,
    resource_observation: Option<ManagedResourceObservation>,
    next_action: String,
    foreman_generation: u64,
    foreman_checkpoint_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ManagedResourceObservation {
    attempts_observed: u64,
    model_calls: u64,
    remaining_model_calls: u64,
    remaining_total_tokens: Option<u64>,
    input_tokens: Option<u64>,
    cached_input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    reasoning_output_tokens: Option<u64>,
    total_tokens: Option<u64>,
}

impl TaskPublicStatus {
    fn from_value(value: &Value) -> Option<Self> {
        let object = value.as_object()?;
        let schema_version = object.get("schema_version")?.as_str()?;
        let required = [
            "schema_version",
            "status",
            "task_state",
            "task_ref",
            "ledger_head_digest",
            "result_digest",
            "failure_stage",
            "failure_code",
        ];
        if !required.iter().all(|field| object.contains_key(*field)) {
            return None;
        }
        let general_fields = ["project_id", "project_name", "project_snapshot_id"];
        let redacted_objective_fields = ["objective_summary", "objective_digest"];
        let managed_fields = [
            "worker_running",
            "attempt",
            "retry_count",
            "model",
            "reasoning",
            "thread_id",
            "turn_id",
            "last_progress_at",
            "blocker",
            "verification_status",
            "verification_digest",
            "evidence_digest",
            "resource_observation",
            "next_action",
            "foreman_generation",
            "foreman_checkpoint_digest",
        ];
        let is_legacy_general_create_only = schema_version == TASK_PUBLIC_STATUS_SCHEMA_V3;
        let is_redacted_general_create_only = schema_version == TASK_PUBLIC_STATUS_SCHEMA_V5;
        let is_external_verified_adoption = schema_version == TASK_PUBLIC_STATUS_SCHEMA_V6;
        let is_general_create_only =
            is_legacy_general_create_only || is_redacted_general_create_only;
        let is_managed = schema_version == TASK_PUBLIC_STATUS_SCHEMA_V4;
        let is_general = is_general_create_only || is_external_verified_adoption || is_managed;
        let expected_fields = if is_managed {
            29
        } else if is_redacted_general_create_only || is_external_verified_adoption {
            13
        } else if is_legacy_general_create_only {
            12
        } else {
            8
        };
        if (schema_version != TASK_PUBLIC_STATUS_SCHEMA_V2 && !is_general)
            || object.len() != expected_fields
            || general_fields
                .iter()
                .any(|field| object.contains_key(*field) != is_general)
            || object.contains_key("objective") != is_legacy_general_create_only
            || redacted_objective_fields.iter().any(|field| {
                object.contains_key(*field)
                    != (is_redacted_general_create_only
                        || is_external_verified_adoption
                        || is_managed)
            })
            || managed_fields
                .iter()
                .any(|field| object.contains_key(*field) != is_managed)
        {
            return None;
        }
        let status = object.get("status")?.as_str()?;
        if (is_managed && !TASK_MANAGED_PUBLIC_STATUS_VALUES.contains(&status))
            || (!is_managed
                && (!TASK_PUBLIC_STATUS_VALUES.contains(&status)
                    || (!is_general_create_only && status == "SUBMITTED")))
        {
            return None;
        }
        let task_state = object.get("task_state")?.as_str()?;
        if !TASK_PUBLIC_STATE_VALUES.contains(&task_state) {
            return None;
        }
        let task_ref = object.get("task_ref")?.as_str()?;
        let ledger_head_digest = object.get("ledger_head_digest")?.as_str()?;
        if !valid_task_ref(task_ref) || !valid_task_ref(ledger_head_digest) {
            return None;
        }
        let result_digest = match object.get("result_digest")? {
            Value::Null => None,
            Value::String(value) if valid_task_ref(value) => Some(value.clone()),
            _ => return None,
        };
        let failure_stage = optional_public_failure_atom(object.get("failure_stage")?)?;
        let failure_code = optional_public_failure_atom(object.get("failure_code")?)?;
        if failure_stage.is_some() != failure_code.is_some() {
            return None;
        }
        if is_general_create_only
            && (status != "SUBMITTED"
                || task_state != "DRAFT"
                || result_digest.is_some()
                || failure_stage.is_some()
                || failure_code.is_some())
        {
            return None;
        }
        if is_external_verified_adoption
            && (status != "COMPLETED"
                || task_state != "COMPLETED"
                || result_digest.is_none()
                || failure_stage.is_some()
                || failure_code.is_some())
        {
            return None;
        }
        let (objective, project_id, project_name, project_snapshot_id) = if is_general {
            let project_id = object.get("project_id")?.as_str()?;
            let project_name = object.get("project_name")?.as_str()?;
            let project_snapshot_id = object.get("project_snapshot_id")?.as_str()?;
            let objective = if is_legacy_general_create_only {
                let objective = object.get("objective")?.as_str()?;
                if !valid_task_objective(objective) {
                    return None;
                }
                Some(objective.to_owned())
            } else {
                None
            };
            if !valid_project_id(project_id)
                || !valid_project_name(project_name)
                || !valid_project_snapshot_id(project_snapshot_id)
            {
                return None;
            }
            (
                objective,
                Some(project_id.to_owned()),
                Some(project_name.to_owned()),
                Some(project_snapshot_id.to_owned()),
            )
        } else {
            (None, None, None, None)
        };
        let redacted_objective = if is_redacted_general_create_only || is_external_verified_adoption
        {
            let summary = object.get("objective_summary")?.as_str()?;
            let digest = object.get("objective_digest")?.as_str()?;
            if summary != TASK_PUBLIC_OBJECTIVE_SUMMARY || !valid_task_ref(digest) {
                return None;
            }
            Some(RedactedTaskObjective {
                summary: summary.to_owned(),
                digest: digest.to_owned(),
            })
        } else {
            None
        };
        let managed = if is_managed {
            Some(ManagedTaskPublicStatus::from_object(object)?)
        } else {
            None
        };

        Some(Self {
            schema_version: schema_version.to_owned(),
            status: status.to_owned(),
            task_state: task_state.to_owned(),
            task_ref: task_ref.to_owned(),
            ledger_head_digest: ledger_head_digest.to_owned(),
            result_digest,
            failure_stage,
            failure_code,
            objective,
            project_id,
            project_name,
            project_snapshot_id,
            redacted_objective,
            managed,
        })
    }

    fn into_value(self) -> Value {
        let mut value = json!({
            "schema_version": self.schema_version,
            "status": self.status,
            "task_state": self.task_state,
            "task_ref": self.task_ref,
            "ledger_head_digest": self.ledger_head_digest,
            "result_digest": self.result_digest,
            "failure_stage": self.failure_stage,
            "failure_code": self.failure_code,
        });
        if let (Some(project_id), Some(project_name), Some(project_snapshot_id)) =
            (self.project_id, self.project_name, self.project_snapshot_id)
        {
            let object = value.as_object_mut().expect("task status object");
            object.insert("project_id".to_owned(), Value::String(project_id));
            object.insert("project_name".to_owned(), Value::String(project_name));
            object.insert(
                "project_snapshot_id".to_owned(),
                Value::String(project_snapshot_id),
            );
            if let Some(objective) = self.objective {
                object.insert("objective".to_owned(), Value::String(objective));
            }
        }
        if let Some(redacted) = self.redacted_objective {
            let object = value.as_object_mut().expect("redacted task status object");
            object.insert(
                "objective_summary".to_owned(),
                Value::String(redacted.summary),
            );
            object.insert(
                "objective_digest".to_owned(),
                Value::String(redacted.digest),
            );
        }
        if let Some(managed) = self.managed {
            managed.insert_into(value.as_object_mut().expect("managed task status object"));
        }
        value
    }
}

impl ManagedTaskPublicStatus {
    fn from_object(object: &Map<String, Value>) -> Option<Self> {
        let objective_summary = object.get("objective_summary")?.as_str()?;
        if objective_summary != TASK_PUBLIC_OBJECTIVE_SUMMARY {
            return None;
        }
        let objective_digest = object.get("objective_digest")?.as_str()?;
        if !valid_task_ref(objective_digest) {
            return None;
        }
        let worker_running = object.get("worker_running")?.as_bool()?;
        let attempt = optional_bounded_u64(object.get("attempt")?, 1, 3)?;
        let retry_count = object.get("retry_count")?.as_u64()?;
        if retry_count > 2 {
            return None;
        }
        let model = optional_allowlisted_string(object.get("model")?, &TASK_MANAGED_MODEL_VALUES)?;
        let reasoning =
            optional_allowlisted_string(object.get("reasoning")?, &TASK_MANAGED_REASONING_VALUES)?;
        let thread_id = optional_safe_identifier(object.get("thread_id")?)?;
        let turn_id = optional_safe_identifier(object.get("turn_id")?)?;
        let last_progress_at = optional_canonical_utc(object.get("last_progress_at")?)?;
        let blocker = optional_public_failure_atom(object.get("blocker")?)?;
        let verification_status = object.get("verification_status")?.as_str()?;
        if !TASK_MANAGED_VERIFICATION_VALUES.contains(&verification_status) {
            return None;
        }
        let verification_digest = optional_lower_sha256(object.get("verification_digest")?)?;
        let evidence_digest = optional_lower_sha256(object.get("evidence_digest")?)?;
        let resource_observation =
            ManagedResourceObservation::optional_from_value(object.get("resource_observation")?)?;
        let next_action = object.get("next_action")?.as_str()?;
        if !valid_public_plain_text(next_action, 256) {
            return None;
        }
        let foreman_generation = object.get("foreman_generation")?.as_u64()?;
        if foreman_generation == 0 {
            return None;
        }
        let foreman_checkpoint_digest = object.get("foreman_checkpoint_digest")?.as_str()?;
        if !valid_task_ref(foreman_checkpoint_digest) {
            return None;
        }
        Some(Self {
            objective_summary: objective_summary.to_owned(),
            objective_digest: objective_digest.to_owned(),
            worker_running,
            attempt,
            retry_count,
            model,
            reasoning,
            thread_id,
            turn_id,
            last_progress_at,
            blocker,
            verification_status: verification_status.to_owned(),
            verification_digest,
            evidence_digest,
            resource_observation,
            next_action: next_action.to_owned(),
            foreman_generation,
            foreman_checkpoint_digest: foreman_checkpoint_digest.to_owned(),
        })
    }

    fn insert_into(self, object: &mut Map<String, Value>) {
        object.insert(
            "objective_summary".to_owned(),
            Value::String(self.objective_summary),
        );
        object.insert(
            "objective_digest".to_owned(),
            Value::String(self.objective_digest),
        );
        object.insert("worker_running".to_owned(), json!(self.worker_running));
        object.insert("attempt".to_owned(), json!(self.attempt));
        object.insert("retry_count".to_owned(), json!(self.retry_count));
        object.insert("model".to_owned(), json!(self.model));
        object.insert("reasoning".to_owned(), json!(self.reasoning));
        object.insert("thread_id".to_owned(), json!(self.thread_id));
        object.insert("turn_id".to_owned(), json!(self.turn_id));
        object.insert("last_progress_at".to_owned(), json!(self.last_progress_at));
        object.insert("blocker".to_owned(), json!(self.blocker));
        object.insert(
            "verification_status".to_owned(),
            Value::String(self.verification_status),
        );
        object.insert(
            "verification_digest".to_owned(),
            json!(self.verification_digest),
        );
        object.insert("evidence_digest".to_owned(), json!(self.evidence_digest));
        object.insert(
            "resource_observation".to_owned(),
            self.resource_observation
                .map_or(Value::Null, ManagedResourceObservation::into_value),
        );
        object.insert("next_action".to_owned(), Value::String(self.next_action));
        object.insert(
            "foreman_generation".to_owned(),
            json!(self.foreman_generation),
        );
        object.insert(
            "foreman_checkpoint_digest".to_owned(),
            Value::String(self.foreman_checkpoint_digest),
        );
    }
}

impl ManagedResourceObservation {
    fn optional_from_value(value: &Value) -> Option<Option<Self>> {
        if value.is_null() {
            return Some(None);
        }
        let object = value.as_object()?;
        let fields = [
            "scope",
            "attempts_observed",
            "model_calls",
            "remaining_model_calls",
            "remaining_total_tokens",
            "input_tokens",
            "cached_input_tokens",
            "output_tokens",
            "reasoning_output_tokens",
            "total_tokens",
            "external_cost_status",
        ];
        if object.len() != fields.len() || !fields.iter().all(|field| object.contains_key(*field)) {
            return None;
        }
        if object.get("scope")?.as_str()? != "TASK_CUMULATIVE"
            || object.get("external_cost_status")?.as_str()? != "UNAVAILABLE"
        {
            return None;
        }
        Some(Some(Self {
            attempts_observed: object.get("attempts_observed")?.as_u64()?,
            model_calls: object.get("model_calls")?.as_u64()?,
            remaining_model_calls: object.get("remaining_model_calls")?.as_u64()?,
            remaining_total_tokens: optional_non_negative_integer(
                object.get("remaining_total_tokens")?,
            )?,
            input_tokens: optional_non_negative_integer(object.get("input_tokens")?)?,
            cached_input_tokens: optional_non_negative_integer(object.get("cached_input_tokens")?)?,
            output_tokens: optional_non_negative_integer(object.get("output_tokens")?)?,
            reasoning_output_tokens: optional_non_negative_integer(
                object.get("reasoning_output_tokens")?,
            )?,
            total_tokens: optional_non_negative_integer(object.get("total_tokens")?)?,
        }))
    }

    fn into_value(self) -> Value {
        json!({
            "scope": "TASK_CUMULATIVE",
            "attempts_observed": self.attempts_observed,
            "model_calls": self.model_calls,
            "remaining_model_calls": self.remaining_model_calls,
            "remaining_total_tokens": self.remaining_total_tokens,
            "input_tokens": self.input_tokens,
            "cached_input_tokens": self.cached_input_tokens,
            "output_tokens": self.output_tokens,
            "reasoning_output_tokens": self.reasoning_output_tokens,
            "total_tokens": self.total_tokens,
            "external_cost_status": "UNAVAILABLE"
        })
    }
}

fn optional_public_failure_atom(value: &Value) -> Option<Option<String>> {
    match value {
        Value::Null => Some(None),
        Value::String(value)
            if !value.is_empty()
                && value.len() <= 128
                && value.bytes().all(|byte| {
                    byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'
                }) =>
        {
            Some(Some(value.clone()))
        }
        _ => None,
    }
}

fn optional_bounded_u64(value: &Value, minimum: u64, maximum: u64) -> Option<Option<u64>> {
    match value {
        Value::Null => Some(None),
        Value::Number(_) => value
            .as_u64()
            .filter(|number| (minimum..=maximum).contains(number))
            .map(Some),
        _ => None,
    }
}

fn optional_non_negative_integer(value: &Value) -> Option<Option<u64>> {
    match value {
        Value::Null => Some(None),
        Value::Number(_) => value.as_u64().map(Some),
        _ => None,
    }
}

fn optional_allowlisted_string(value: &Value, allowlist: &[&str]) -> Option<Option<String>> {
    match value {
        Value::Null => Some(None),
        Value::String(value) if allowlist.contains(&value.as_str()) => Some(Some(value.clone())),
        _ => None,
    }
}

fn optional_safe_identifier(value: &Value) -> Option<Option<String>> {
    match value {
        Value::Null => Some(None),
        Value::String(value) if valid_safe_identifier(value) => Some(Some(value.clone())),
        _ => None,
    }
}

fn valid_safe_identifier(value: &str) -> bool {
    let bytes = value.as_bytes();
    (1..=256).contains(&bytes.len())
        && bytes.first().is_some_and(u8::is_ascii_alphanumeric)
        && bytes
            .iter()
            .skip(1)
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'-' | b'_' | b'.' | b':'))
        && !task_submission_text_contains_secret(value)
}

fn optional_canonical_utc(value: &Value) -> Option<Option<String>> {
    match value {
        Value::Null => Some(None),
        Value::String(value) if valid_canonical_utc(value) => Some(Some(value.clone())),
        _ => None,
    }
}

fn valid_canonical_utc(value: &str) -> bool {
    let Ok(parsed) = OffsetDateTime::parse(value, &Rfc3339) else {
        return false;
    };
    parsed.offset() == UtcOffset::UTC
        && value.ends_with('Z')
        && parsed
            .format(&Rfc3339)
            .is_ok_and(|formatted| formatted == value)
}

fn optional_lower_sha256(value: &Value) -> Option<Option<String>> {
    match value {
        Value::Null => Some(None),
        Value::String(value) if valid_task_ref(value) => Some(Some(value.clone())),
        _ => None,
    }
}

fn valid_public_plain_text(value: &str, maximum_chars: usize) -> bool {
    !value.is_empty()
        && value.chars().count() <= maximum_chars
        && value.len() <= maximum_chars.saturating_mul(4)
        && value.trim() == value
        && is_nfc(value)
        && !value.chars().any(char::is_control)
        && !task_submission_text_contains_secret(value)
}

/// Composition-owned typed operations exposed by MCP.
pub trait DeliveryToolService {
    /// Executes the fixed delivery profile.
    ///
    /// # Errors
    ///
    /// Returns only a stable, secret-free failure code.
    fn run(&mut self, arguments: &DeliveryToolArguments) -> Result<Value, ToolExecutionError>;

    /// Reads the fixed delivery profile's durable status.
    ///
    /// # Errors
    ///
    /// Returns only a stable, secret-free failure code.
    fn status(&mut self, arguments: &DeliveryToolArguments) -> Result<Value, ToolExecutionError>;

    /// Reads secret-free state for the Runtime's independently degradable components.
    ///
    /// # Errors
    ///
    /// Returns only a stable, secret-free failure code.
    fn runtime_status(
        &mut self,
        _arguments: &DeliveryToolArguments,
    ) -> Result<Value, ToolExecutionError> {
        Err(ToolExecutionError::new(
            "LATTICE_RUNTIME_STATUS_UNAVAILABLE",
        ))
    }

    /// Replays the fixed delivery receipt to determine whether reconciliation is needed.
    ///
    /// This probe is read-only. It must never dispatch Codex, append a receipt,
    /// or reinterpret uncertain evidence as success.
    fn reconcile(
        &mut self,
        _arguments: &DeliveryToolArguments,
    ) -> Result<Value, ToolExecutionError> {
        Err(ToolExecutionError::new(
            "LATTICE_DELIVERY_RECONCILIATION_UNAVAILABLE",
        ))
    }

    /// Submits one validated high-level task intent to the existing service.
    ///
    /// # Errors
    ///
    /// Returns only a stable, secret-free failure code.
    fn task_submit(&mut self, arguments: &TaskSubmitArguments)
    -> Result<Value, ToolExecutionError>;

    /// Reads one validated durable task reference from the existing service.
    ///
    /// # Errors
    ///
    /// Returns only a stable, secret-free failure code.
    fn task_status(&mut self, arguments: &TaskStatusArguments)
    -> Result<Value, ToolExecutionError>;

    /// Records or exactly replays the sole foreman's durable checkpoint.
    ///
    /// # Errors
    ///
    /// Returns only a stable, secret-free failure code.
    fn foreman_checkpoint(
        &mut self,
        _arguments: &ForemanCheckpointArguments,
    ) -> Result<Value, ToolExecutionError> {
        Err(ToolExecutionError::new("FOREMAN_CHECKPOINT_UNAVAILABLE"))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Lifecycle {
    AwaitingInitialize,
    AwaitingInitialized,
    Ready,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RequestProtocol {
    Legacy,
    Stateless,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ToolSurface {
    CanonicalTaskControl,
    LegacyDeliveryObserver,
}

impl ToolSurface {
    const fn allows_task_control(self) -> bool {
        matches!(self, Self::CanonicalTaskControl)
    }

    const fn allows_delivery_run(self) -> bool {
        matches!(self, Self::CanonicalTaskControl)
    }

    const fn instructions(self) -> &'static str {
        match self {
            Self::CanonicalTaskControl => {
                "Five bounded LATTICE tools. Authority, task binding, orchestration, and execution configuration remain server-owned."
            }
            Self::LegacyDeliveryObserver => {
                "Legacy LATTICE delivery observer. Delivery mutation and task control are available only through the canonical latticed entrypoint."
            }
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
enum RequestProtocolError {
    InvalidMetadata,
    Unsupported(String),
}

/// Stateful MCP lifecycle and request dispatcher.
pub struct McpServer<S> {
    service: S,
    arguments: DeliveryToolArguments,
    lifecycle: Lifecycle,
    tool_surface: ToolSurface,
    tool_budget: McpBudget,
    acceptance_evidence: Option<AcceptanceEvidence>,
    acceptance_evidence_error: Option<io::Error>,
    observed_effect_enabled: bool,
}

impl<S: DeliveryToolService> McpServer<S> {
    /// Constructs an uninitialized server.
    #[must_use]
    pub const fn new(service: S, binding: SubjectBinding) -> Self {
        Self {
            service,
            arguments: DeliveryToolArguments::new(binding),
            lifecycle: Lifecycle::AwaitingInitialize,
            tool_surface: ToolSurface::CanonicalTaskControl,
            tool_budget: McpBudget::new(
                MAX_TOOL_INVOCATIONS_PER_SESSION as u16,
                MCP_HANDOFF_RESERVE as u16,
            )
            .expect("constant budget is valid"),
            acceptance_evidence: None,
            acceptance_evidence_error: None,
            observed_effect_enabled: false,
        }
    }

    /// Constructs an uninitialized legacy observer with no mutation capability.
    #[must_use]
    pub const fn new_legacy_delivery_observer(service: S, binding: SubjectBinding) -> Self {
        Self {
            service,
            arguments: DeliveryToolArguments::new(binding),
            lifecycle: Lifecycle::AwaitingInitialize,
            tool_surface: ToolSurface::LegacyDeliveryObserver,
            tool_budget: McpBudget::new(
                MAX_TOOL_INVOCATIONS_PER_SESSION as u16,
                MCP_HANDOFF_RESERVE as u16,
            )
            .expect("constant budget is valid"),
            acceptance_evidence: None,
            acceptance_evidence_error: None,
            observed_effect_enabled: false,
        }
    }

    fn enable_acceptance_evidence(&mut self) -> io::Result<()> {
        self.acceptance_evidence = AcceptanceEvidence::from_process_environment()?;
        self.observed_effect_enabled = install_observed_effect_evidence()?;
        Ok(())
    }

    fn take_acceptance_evidence_error(&mut self) -> Option<io::Error> {
        self.acceptance_evidence_error.take()
    }

    fn close_acceptance_evidence(&mut self) -> io::Result<()> {
        close_observed_effect_evidence(self.observed_effect_enabled)?;
        self.observed_effect_enabled = false;
        if let Some(evidence) = self.acceptance_evidence.as_mut() {
            evidence.close()?;
        }
        Ok(())
    }

    fn reject_observed_probe(
        &mut self,
        id: Value,
        classification: &str,
        code: i32,
        message: &'static str,
    ) -> Value {
        if let Err(error) =
            with_observed_effect_evidence(|evidence| evidence.reject_probe(classification))
        {
            self.acceptance_evidence_error = Some(error);
            return protocol_error(id, -32603, "Acceptance evidence rejected");
        }
        protocol_error(id, code, message)
    }

    fn reject_foreman_checkpoint_params(&mut self, id: Value) -> Value {
        if let Err(error) =
            with_observed_effect_evidence(|evidence| evidence.reject_probe("MCP_INVALID_PARAMS"))
        {
            self.acceptance_evidence_error = Some(error);
            return protocol_error(id, -32603, "Acceptance evidence rejected");
        }
        protocol_error_with_machine_code(
            id,
            -32602,
            "Invalid foreman checkpoint arguments",
            "FOREMAN_CHECKPOINT_INVALID",
        )
    }

    fn reject_budget_probe(
        &mut self,
        id: Value,
        receipt: crate::mcp_budget::McpRejectionReceipt,
    ) -> Value {
        if let Err(error) =
            with_observed_effect_evidence(|evidence| evidence.reject_probe("MCP_INVOCATION_LIMIT"))
        {
            self.acceptance_evidence_error = Some(error);
            return protocol_error(id, -32603, "Acceptance evidence rejected");
        }
        success(
            id,
            budget_rejection_result(
                receipt,
                self.tool_budget.remaining(),
                self.tool_budget.read_only_reserve(),
            ),
        )
    }

    /// Handles one decoded JSON-RPC message. Notifications return no value.
    #[must_use]
    pub fn handle(&mut self, message: Value) -> Option<Value> {
        let Value::Object(mut object) = message else {
            return Some(protocol_error(Value::Null, -32600, "Invalid Request"));
        };
        let id = object.remove("id");
        if id.as_ref().is_some_and(|id| !valid_request_id(id)) {
            return Some(protocol_error(Value::Null, -32600, "Invalid Request"));
        }
        if object
            .remove("jsonrpc")
            .and_then(|value| value.as_str().map(str::to_owned))
            .as_deref()
            != Some("2.0")
        {
            return id.map(|id| protocol_error(id, -32600, "Invalid Request"));
        }
        let Some(method) = object
            .remove("method")
            .and_then(|value| value.as_str().map(str::to_owned))
        else {
            return id.map(|id| protocol_error(id, -32600, "Invalid Request"));
        };
        let params = object.remove("params");

        if id.is_none() {
            if method == "notifications/initialized"
                && self.lifecycle == Lifecycle::AwaitingInitialized
            {
                self.lifecycle = Lifecycle::Ready;
            }
            return None;
        }
        let id = id?;
        let protocol = match request_protocol(params.as_ref()) {
            Ok(protocol) => protocol,
            Err(RequestProtocolError::InvalidMetadata) => {
                return Some(protocol_error(id, -32602, "Invalid request metadata"));
            }
            Err(RequestProtocolError::Unsupported(requested)) => {
                return Some(unsupported_protocol_error(id, &requested));
            }
        };
        match method.as_str() {
            "server/discover" => Some(self.discover(id, params.as_ref(), protocol)),
            "initialize" if protocol == RequestProtocol::Stateless => {
                Some(protocol_error(id, -32601, "Method not found"))
            }
            "initialize" => Some(self.initialize(id, params.as_ref())),
            "ping" if protocol == RequestProtocol::Legacy => Some(success(id, json!({}))),
            "tools/list" => Some(self.list_tools(id, params.as_ref(), protocol)),
            "tools/call" => Some(self.call_tool(id, params.as_ref(), protocol)),
            _ => Some(protocol_error(id, -32601, "Method not found")),
        }
    }

    fn discover(&self, id: Value, params: Option<&Value>, protocol: RequestProtocol) -> Value {
        if protocol != RequestProtocol::Stateless || !metadata_only_object_or_absent(params) {
            return protocol_error(id, -32602, "Invalid server/discover params");
        }
        success(
            id,
            json!({
                "resultType": "complete",
                "supportedVersions": [MCP_STATELESS_PROTOCOL_VERSION],
                "capabilities": {"tools": {}},
                "instructions": self.tool_surface.instructions(),
                "ttlMs": 0,
                "cacheScope": "private",
                "_meta": server_result_meta()
            }),
        )
    }

    fn initialize(&mut self, id: Value, params: Option<&Value>) -> Value {
        if self.lifecycle != Lifecycle::AwaitingInitialize {
            return protocol_error(id, -32600, "Already initialized");
        }
        let Some(params) = params.and_then(Value::as_object) else {
            return protocol_error(id, -32602, "Invalid initialize params");
        };
        if params
            .get("protocolVersion")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
            || !params.get("capabilities").is_some_and(Value::is_object)
            || !params.get("clientInfo").is_some_and(Value::is_object)
        {
            return protocol_error(id, -32602, "Invalid initialize params");
        }
        self.lifecycle = Lifecycle::AwaitingInitialized;
        success(
            id,
            json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {"tools": {}},
                "serverInfo": {
                    "name": "latticed",
                    "title": "LATTICE DevOS",
                    "version": "1.0.0"
                },
                "instructions": self.tool_surface.instructions()
            }),
        )
    }

    fn list_tools(&self, id: Value, params: Option<&Value>, protocol: RequestProtocol) -> Value {
        if protocol == RequestProtocol::Legacy && self.lifecycle != Lifecycle::Ready {
            return protocol_error(id, -32002, "Server not initialized");
        }
        if !metadata_only_object_or_absent(params) {
            return protocol_error(id, -32602, "Invalid tools/list params");
        }
        let mut result = json!({"tools": tool_catalog(protocol, self.tool_surface)});
        if protocol == RequestProtocol::Stateless {
            let result = result
                .as_object_mut()
                .expect("tool list result is an object");
            result.insert(
                "resultType".to_owned(),
                Value::String("complete".to_owned()),
            );
            result.insert("ttlMs".to_owned(), Value::from(0));
            result.insert("cacheScope".to_owned(), Value::String("private".to_owned()));
            result.insert("_meta".to_owned(), server_result_meta());
        }
        success(id, result)
    }

    fn call_tool(&mut self, id: Value, params: Option<&Value>, protocol: RequestProtocol) -> Value {
        let observed_tool_name = params
            .and_then(Value::as_object)
            .and_then(|params| params.get("name"))
            .and_then(Value::as_str)
            .filter(|name| {
                matches!(
                    *name,
                    DELIVERY_RUN_TOOL
                        | DELIVERY_STATUS_TOOL
                        | RUNTIME_STATUS_TOOL
                        | DELIVERY_RECONCILE_TOOL
                        | TASK_SUBMIT_TOOL
                        | TASK_STATUS_TOOL
                        | FOREMAN_CHECKPOINT_TOOL
                )
            })
            .unwrap_or("unknown");
        if let Err(error) = with_observed_effect_evidence(|evidence| {
            evidence.begin_probe("mcp-tools-call", observed_tool_name, &id)
        }) {
            self.acceptance_evidence_error = Some(error);
            return protocol_error(id, -32603, "Acceptance evidence rejected");
        }
        if protocol == RequestProtocol::Legacy && self.lifecycle != Lifecycle::Ready {
            return self.reject_observed_probe(
                id,
                "MCP_NOT_READY",
                -32002,
                "Server not initialized",
            );
        }
        let Some(params) = params.and_then(Value::as_object) else {
            return self.reject_observed_probe(
                id,
                "MCP_INVALID_PARAMS",
                -32602,
                "Invalid tools/call params",
            );
        };
        if params
            .keys()
            .any(|key| key != "name" && key != "arguments" && key != "_meta")
            || !metadata_object_or_absent(params.get("_meta"))
        {
            return self.reject_observed_probe(
                id,
                "MCP_INVALID_PARAMS",
                -32602,
                "Invalid tools/call params",
            );
        }
        let Some(name) = params.get("name").and_then(Value::as_str) else {
            return self.reject_observed_probe(
                id,
                "MCP_INVALID_PARAMS",
                -32602,
                "Invalid tools/call params",
            );
        };
        if !self.tool_surface.allows_task_control()
            && matches!(
                name,
                RUNTIME_STATUS_TOOL
                    | DELIVERY_RECONCILE_TOOL
                    | TASK_SUBMIT_TOOL
                    | TASK_STATUS_TOOL
                    | FOREMAN_CHECKPOINT_TOOL
            )
        {
            return self.reject_observed_probe(id, "MCP_UNKNOWN_TOOL", -32602, "Unknown tool");
        }
        let operation = match name {
            DELIVERY_RUN_TOOL if empty_object_or_absent(params.get("arguments")) => {
                ToolOperation::DeliveryRun
            }
            DELIVERY_STATUS_TOOL if empty_object_or_absent(params.get("arguments")) => {
                ToolOperation::DeliveryStatus
            }
            RUNTIME_STATUS_TOOL if empty_object_or_absent(params.get("arguments")) => {
                ToolOperation::RuntimeStatus
            }
            DELIVERY_RECONCILE_TOOL if empty_object_or_absent(params.get("arguments")) => {
                ToolOperation::DeliveryReconcile
            }
            DELIVERY_RUN_TOOL
            | DELIVERY_STATUS_TOOL
            | RUNTIME_STATUS_TOOL
            | DELIVERY_RECONCILE_TOOL => {
                return self.reject_observed_probe(
                    id,
                    "MCP_INVALID_PARAMS",
                    -32602,
                    "Tool accepts no arguments",
                );
            }
            TASK_SUBMIT_TOOL => {
                let Some(arguments) = TaskSubmitArguments::from_value(params.get("arguments"))
                else {
                    return self.reject_observed_probe(
                        id,
                        "MCP_INVALID_PARAMS",
                        -32602,
                        "Invalid task submit arguments",
                    );
                };
                ToolOperation::TaskSubmit(arguments)
            }
            TASK_STATUS_TOOL => {
                let Some(arguments) = TaskStatusArguments::from_value(params.get("arguments"))
                else {
                    return self.reject_observed_probe(
                        id,
                        "MCP_INVALID_PARAMS",
                        -32602,
                        "Invalid task status arguments",
                    );
                };
                ToolOperation::TaskStatus(arguments)
            }
            FOREMAN_CHECKPOINT_TOOL => {
                let Some(arguments) =
                    ForemanCheckpointArguments::from_value(params.get("arguments"))
                else {
                    return self.reject_foreman_checkpoint_params(id);
                };
                ToolOperation::ForemanCheckpoint(arguments)
            }
            _ => {
                return self.reject_observed_probe(id, "MCP_UNKNOWN_TOOL", -32602, "Unknown tool");
            }
        };
        let class = if matches!(
            &operation,
            ToolOperation::DeliveryRun
                | ToolOperation::TaskSubmit(_)
                | ToolOperation::ForemanCheckpoint(_)
        ) {
            McpToolClass::Execution
        } else {
            McpToolClass::Observation
        };
        if let McpAdmission::Rejected(receipt) = self.tool_budget.admit(class) {
            return self.reject_budget_probe(id, receipt);
        }
        if let Some(evidence) = self.acceptance_evidence.as_mut()
            && let Err(error) = evidence.record_dispatch(name, &id)
        {
            self.acceptance_evidence_error = Some(error);
            return protocol_error(id, -32603, "Acceptance evidence rejected");
        }
        if let Err(error) = with_observed_effect_evidence(ObservedEffectEvidence::accept_dispatch) {
            self.acceptance_evidence_error = Some(error);
            return protocol_error(id, -32603, "Acceptance evidence rejected");
        }
        let result = match operation {
            ToolOperation::DeliveryRun if !self.tool_surface.allows_delivery_run() => {
                Err(ToolExecutionError::new(LEGACY_DELIVERY_RUN_DISABLED))
            }
            ToolOperation::DeliveryRun => self.service.run(&self.arguments),
            ToolOperation::DeliveryStatus => self.service.status(&self.arguments),
            ToolOperation::RuntimeStatus => self.service.runtime_status(&self.arguments),
            ToolOperation::DeliveryReconcile => self.service.reconcile(&self.arguments),
            ToolOperation::TaskSubmit(arguments) => {
                closed_task_public_status(self.service.task_submit(&arguments))
            }
            ToolOperation::TaskStatus(arguments) => {
                closed_task_public_status(self.service.task_status(&arguments))
            }
            ToolOperation::ForemanCheckpoint(arguments) => {
                closed_foreman_checkpoint_result(self.service.foreman_checkpoint(&arguments))
            }
        };
        let observed_classification = if result.is_ok() {
            "MCP_RESULT"
        } else {
            "MCP_TOOL_ERROR"
        };
        if let Err(error) = with_observed_effect_evidence(|evidence| {
            evidence.complete_probe(observed_classification)
        }) {
            self.acceptance_evidence_error = Some(error);
            return protocol_error(id, -32603, "Acceptance evidence rejected");
        }
        let mut result = tool_result(result);
        if protocol == RequestProtocol::Stateless {
            let result = result.as_object_mut().expect("tool result is an object");
            result.insert(
                "resultType".to_owned(),
                Value::String("complete".to_owned()),
            );
            result.insert("_meta".to_owned(), server_result_meta());
        }
        success(id, result)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ToolOperation {
    DeliveryRun,
    DeliveryStatus,
    RuntimeStatus,
    DeliveryReconcile,
    TaskSubmit(TaskSubmitArguments),
    TaskStatus(TaskStatusArguments),
    ForemanCheckpoint(ForemanCheckpointArguments),
}

#[derive(Debug, Eq, PartialEq)]
enum StdioFrame {
    EndOfStream,
    Complete(Vec<u8>),
    Oversized,
    Unterminated,
}

/// Fixed MCP lifecycle milestones safe for process-local startup diagnostics.
///
/// These events intentionally carry no request, configuration, or service
/// values. The composition root may mirror them to stderr without changing the
/// MCP stdout protocol.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StdioLifecycleEvent {
    /// The server is blocked waiting for the next newline-delimited frame.
    WaitingForInput,
    /// A syntactically decoded `initialize` request was received.
    InitializeReceived,
    /// A syntactically decoded initialized notification was received.
    InitializedNotificationReceived,
    /// A syntactically decoded `tools/list` request was received.
    ToolsListReceived,
    /// The stdin stream reached EOF before another frame.
    EndOfStream,
}

/// Serves newline-delimited MCP JSON-RPC over the supplied streams.
///
/// # Errors
///
/// Returns only transport read/write errors. Protocol and parse errors are
/// written as JSON-RPC responses.
pub fn serve<S: DeliveryToolService, R: BufRead, W: Write>(
    service: S,
    binding: SubjectBinding,
    reader: R,
    writer: W,
) -> io::Result<()> {
    serve_with_lifecycle_observer(service, binding, reader, writer, |_| {})
}

/// Serves MCP while reporting only fixed lifecycle milestones to `observer`.
///
/// The observer is process-local and never writes to the supplied MCP stdout
/// stream. It receives no caller payload, arguments, or error text.
///
/// # Errors
///
/// Returns only the existing MCP transport or acceptance-evidence read/write
/// failures; observer delivery has no fallible path.
pub fn serve_with_lifecycle_observer<
    S: DeliveryToolService,
    R: BufRead,
    W: Write,
    F: FnMut(StdioLifecycleEvent),
>(
    service: S,
    binding: SubjectBinding,
    reader: R,
    writer: W,
    observer: F,
) -> io::Result<()> {
    let mut server = McpServer::new(service, binding);
    server.enable_acceptance_evidence()?;
    serve_server(server, reader, writer, observer)
}

/// Serves the legacy read-only delivery observer over the supplied streams.
///
/// # Errors
///
/// Returns only transport read/write errors. Protocol and parse errors are
/// written as JSON-RPC responses.
pub fn serve_legacy_delivery_observer<S: DeliveryToolService, R: BufRead, W: Write>(
    service: S,
    binding: SubjectBinding,
    reader: R,
    writer: W,
) -> io::Result<()> {
    let mut server = McpServer::new_legacy_delivery_observer(service, binding);
    server.enable_acceptance_evidence()?;
    serve_server(server, reader, writer, |_| {})
}

fn serve_server<S: DeliveryToolService, R: BufRead, W: Write, F: FnMut(StdioLifecycleEvent)>(
    mut server: McpServer<S>,
    mut reader: R,
    mut writer: W,
    mut observer: F,
) -> io::Result<()> {
    let mut initial_input_wait_reported = false;
    loop {
        if !initial_input_wait_reported {
            observer(StdioLifecycleEvent::WaitingForInput);
            initial_input_wait_reported = true;
        }
        let response = match read_bounded_frame(&mut reader)? {
            StdioFrame::EndOfStream => {
                observer(StdioLifecycleEvent::EndOfStream);
                server.close_acceptance_evidence()?;
                return Ok(());
            }
            StdioFrame::Oversized => Some(protocol_error(Value::Null, -32600, "Message too large")),
            StdioFrame::Unterminated => {
                Some(protocol_error(Value::Null, -32600, "Unterminated message"))
            }
            StdioFrame::Complete(buffer) => match serde_json::from_slice::<Value>(&buffer) {
                Ok(message) => {
                    observe_lifecycle_message(&message, &mut observer);
                    server.handle(message)
                }
                Err(_) => Some(protocol_error(Value::Null, -32700, "Parse error")),
            },
        };
        if let Some(error) = server.take_acceptance_evidence_error() {
            return Err(error);
        }
        if let Some(response) = response {
            serde_json::to_writer(&mut writer, &response)?;
            writer.write_all(b"\n")?;
            writer.flush()?;
        }
    }
}

fn observe_lifecycle_message<F: FnMut(StdioLifecycleEvent)>(message: &Value, observer: &mut F) {
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        return;
    };
    match method {
        "initialize" => observer(StdioLifecycleEvent::InitializeReceived),
        "notifications/initialized" => {
            observer(StdioLifecycleEvent::InitializedNotificationReceived);
        }
        "tools/list" => observer(StdioLifecycleEvent::ToolsListReceived),
        _ => {}
    }
}

fn read_bounded_frame<R: BufRead>(reader: &mut R) -> io::Result<StdioFrame> {
    let mut buffer = Vec::new();
    let mut oversized = false;
    let mut saw_bytes = false;

    loop {
        let (consumed, terminated) = {
            let available = reader.fill_buf()?;
            if available.is_empty() {
                return Ok(if !saw_bytes {
                    StdioFrame::EndOfStream
                } else if oversized {
                    StdioFrame::Oversized
                } else {
                    StdioFrame::Unterminated
                });
            }

            saw_bytes = true;
            let newline = available.iter().position(|byte| *byte == b'\n');
            let consumed = newline.map_or(available.len(), |position| position + 1);
            if !oversized {
                let remaining = MAX_STDIO_MESSAGE_BYTES.saturating_sub(buffer.len());
                if consumed <= remaining {
                    buffer.extend_from_slice(&available[..consumed]);
                } else {
                    buffer.extend_from_slice(&available[..remaining]);
                    oversized = true;
                }
            }
            (consumed, newline.is_some())
        };

        reader.consume(consumed);
        if terminated {
            return Ok(if oversized {
                StdioFrame::Oversized
            } else {
                StdioFrame::Complete(buffer)
            });
        }
    }
}

fn delivery_arguments_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false
    })
}

fn task_submit_arguments_schema() -> Value {
    json!({
        "oneOf": [
            {
                "type": "object",
                "properties": {
                    "client_request_id": client_request_id_schema(),
                    "intent": {
                        "type": "string",
                        "enum": [CONTROLLED_CODEX_CANARY_INTENT]
                    }
                },
                "required": ["client_request_id", "intent"],
                "additionalProperties": false
            },
            general_task_submit_schema("objective", false),
            general_task_submit_schema("intent", true),
            verified_result_adoption_schema()
        ]
    })
}

fn verified_result_adoption_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "client_request_id": client_request_id_schema(),
            "intent": {"type": "string", "enum": [ADOPT_VERIFIED_RESULT_INTENT]},
            "task_ref": lower_sha256_schema(),
            "expected_ledger_head_digest": lower_sha256_schema(),
            "source_sha": git_sha_schema(),
            "target_sha": git_sha_schema(),
            "push_merge_receipt_ref": evidence_ref_schema(),
            "deployment_receipt_ref": evidence_ref_schema(),
            "deployment_artifact_ref": evidence_ref_schema(),
            "independent_acceptance_ref": evidence_ref_schema(),
            "protected_action_approval_refs": {
                "type": "array",
                "minItems": 1,
                "maxItems": 8,
                "uniqueItems": true,
                "items": evidence_ref_schema()
            }
        },
        "required": [
            "client_request_id", "intent", "task_ref", "expected_ledger_head_digest",
            "source_sha", "target_sha", "push_merge_receipt_ref",
            "deployment_receipt_ref", "deployment_artifact_ref",
            "independent_acceptance_ref", "protected_action_approval_refs"
        ],
        "additionalProperties": false
    })
}

fn git_sha_schema() -> Value {
    json!({
        "type": "string",
        "minLength": 40,
        "maxLength": 40,
        "pattern": "^[0-9a-f]{40}$"
    })
}

fn evidence_ref_schema() -> Value {
    json!({
        "type": "string",
        "minLength": 80,
        "maxLength": 80,
        "pattern": "^evidence:sha256:[0-9a-f]{64}$"
    })
}

fn client_request_id_schema() -> Value {
    json!({
        "type": "string",
        "minLength": 1,
        "maxLength": MAX_CLIENT_REQUEST_ID_BYTES,
        "pattern": "^[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$",
        "description": "Bounded ASCII idempotency key without recognized secret material."
    })
}

fn general_task_submit_schema(objective_field: &str, excludes_canary: bool) -> Value {
    let mut schema = json!({
        "type": "object",
        "properties": {
            "client_request_id": client_request_id_schema(),
            (objective_field): {
                "type": "string",
                "minLength": 1,
                "maxLength": MAX_TASK_OBJECTIVE_CHARS,
                "description": "NFC text without leading/trailing whitespace, control characters, or secret material."
            },
            "project_id": {
                "type": "string",
                "minLength": 2,
                "maxLength": MAX_PROJECT_ID_BYTES,
                "pattern": "^[a-z0-9][a-z0-9._-]{1,63}$"
            },
            "project_name": {
                "type": "string",
                "minLength": 1,
                "maxLength": MAX_PROJECT_NAME_CHARS,
                "description": "Exact NFC Control catalog display name."
            }
        },
        "required": ["client_request_id", objective_field],
        "not": {"required": ["project_id", "project_name"]},
        "additionalProperties": false
    });
    if excludes_canary {
        schema["properties"][objective_field]["not"] =
            json!({"enum": [CONTROLLED_CODEX_CANARY_INTENT]});
    }
    schema
}

fn task_status_arguments_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "client_request_id": client_request_id_schema(),
            "task_ref": {
                "type": "string",
                "minLength": 64,
                "maxLength": 64,
                "pattern": "^[0-9a-f]{64}$"
            }
        },
        "required": ["task_ref"],
        "additionalProperties": false
    })
}

fn foreman_checkpoint_arguments_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "checkpoint_id": {
                "type": "string",
                "minLength": 1,
                "maxLength": 64,
                "pattern": "^[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$"
            },
            "generation": {"type": "integer", "minimum": 1},
            "occurred_at": {
                "type": "string",
                "pattern": "^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$"
            },
            "state": {"type": "string", "enum": ["ACTIVE", "BLOCKED", "COMPLETED"]},
            "blocker_ref": {
                "anyOf": [
                    {"type": "string", "minLength": 1, "maxLength": 256},
                    {
                        "type": "object",
                        "properties": {
                            "schema": {
                                "type": "string",
                                "enum": ["lattice.dependency-blocker/1.0"]
                            },
                            "parent_task_id": {
                                "type": "string",
                                "minLength": 8,
                                "maxLength": 64,
                                "pattern": "^TASK-[A-Z0-9][A-Z0-9_-]{2,58}$"
                            },
                            "dependency_task_id": {
                                "type": "string",
                                "minLength": 8,
                                "maxLength": 64,
                                "pattern": "^TASK-[A-Z0-9][A-Z0-9_-]{2,58}$"
                            },
                            "dependency_worktree_id": {
                                "type": "string",
                                "minLength": 3,
                                "maxLength": 64,
                                "pattern": "^[A-Z0-9][A-Z0-9_-]{2,63}$"
                            },
                            "dependency_branch": {
                                "type": "string",
                                "minLength": 9,
                                "maxLength": 72,
                                "pattern": "^lattice/task-[a-z0-9][a-z0-9_-]{2,58}$"
                            },
                            "base_sha": {
                                "type": "string",
                                "minLength": 40,
                                "maxLength": 40,
                                "pattern": "^[0-9a-f]{40}$"
                            },
                            "next_action": {
                                "type": "string",
                                "enum": ["COMPLETE_DEPENDENCY"]
                            }
                        },
                        "required": [
                            "schema", "parent_task_id", "dependency_task_id",
                            "dependency_worktree_id", "dependency_branch",
                            "base_sha", "next_action"
                        ],
                        "additionalProperties": false
                    },
                    {"type": "null"}
                ]
            },
            "heartbeat_ref": {
                "type": "string",
                "pattern": "^heartbeat:sha256:[0-9a-f]{64}$"
            },
            "evidence_ref": {
                "type": "string",
                "pattern": "^evidence:sha256:[0-9a-f]{64}$"
            }
        },
        "required": [
            "checkpoint_id", "generation", "occurred_at", "state",
            "blocker_ref", "heartbeat_ref", "evidence_ref"
        ],
        "additionalProperties": false
    })
}

fn foreman_checkpoint_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "schema": {"type": "string", "enum": [FOREMAN_CHECKPOINT_RESULT_SCHEMA]},
            "checkpoint_id": {"type": "string"},
            "generation": {"type": "integer", "minimum": 1},
            "status": {"type": "string", "enum": ["RECORDED", "REPLAYED"]},
            "exact_retry": {"type": "boolean"},
            "ledger_digest": {"type": "string", "pattern": "^[0-9a-f]{64}$"},
            "checkpoint_digest": {"type": "string", "pattern": "^[0-9a-f]{64}$"}
        },
        "required": [
            "schema", "checkpoint_id", "generation", "status", "exact_retry",
            "ledger_digest", "checkpoint_digest"
        ],
        "additionalProperties": false
    })
}

fn task_public_status_schema() -> Value {
    json!({
        "oneOf": [
            task_public_status_variant_schema(false),
            redacted_general_task_public_status_variant_schema(),
            adopted_external_result_task_public_status_variant_schema(),
            managed_task_public_status_variant_schema()
        ]
    })
}

fn redacted_general_task_public_status_variant_schema() -> Value {
    let mut schema = task_public_status_variant_schema(true);
    let properties = schema["properties"]
        .as_object_mut()
        .expect("redacted task status properties");
    properties.remove("objective");
    properties.insert(
        "objective_summary".to_owned(),
        json!({"type": "string", "enum": [TASK_PUBLIC_OBJECTIVE_SUMMARY]}),
    );
    properties.insert("objective_digest".to_owned(), lower_sha256_schema());
    properties["schema_version"] =
        json!({"type": "string", "enum": [TASK_PUBLIC_STATUS_SCHEMA_V5]});
    let required = schema["required"]
        .as_array_mut()
        .expect("redacted task status required fields");
    required.retain(|field| field.as_str() != Some("objective"));
    required.extend(
        ["objective_summary", "objective_digest"]
            .into_iter()
            .map(|field| json!(field)),
    );
    schema
}

fn adopted_external_result_task_public_status_variant_schema() -> Value {
    let mut schema = redacted_general_task_public_status_variant_schema();
    let properties = schema["properties"]
        .as_object_mut()
        .expect("adopted task status properties");
    properties["schema_version"] =
        json!({"type": "string", "enum": [TASK_PUBLIC_STATUS_SCHEMA_V6]});
    properties["status"] = json!({"type": "string", "enum": ["COMPLETED"]});
    properties["task_state"] = json!({"type": "string", "enum": ["COMPLETED"]});
    properties["result_digest"] = lower_sha256_schema();
    schema
}

fn task_public_status_variant_schema(general: bool) -> Value {
    let mut properties = json!({
            "schema_version": {
                "type": "string",
                "enum": [if general { TASK_PUBLIC_STATUS_SCHEMA_V3 } else { TASK_PUBLIC_STATUS_SCHEMA_V2 }]
            },
            "status": {
                "type": "string",
                "enum": if general {
                    Value::Array(TASK_PUBLIC_STATUS_VALUES.iter().map(|value| json!(value)).collect())
                } else {
                    json!(["NOT_SUBMITTED", "RECONCILIATION_REQUIRED", "FAILED", "COMPLETED"])
                }
            },
            "task_state": {
                "type": "string",
                "enum": TASK_PUBLIC_STATE_VALUES
            },
            "task_ref": lower_sha256_schema(),
            "ledger_head_digest": lower_sha256_schema(),
            "result_digest": {
                "anyOf": [lower_sha256_schema(), {"type": "null"}]
            },
            "failure_stage": {
                "anyOf": [
                    {"type": "string", "minLength": 1, "maxLength": 128, "pattern": "^[A-Z0-9_]+$"},
                    {"type": "null"}
                ]
            },
            "failure_code": {
                "anyOf": [
                    {"type": "string", "minLength": 1, "maxLength": 128, "pattern": "^[A-Z0-9_]+$"},
                    {"type": "null"}
                ]
            }
    });
    if general {
        properties["status"] = json!({"type": "string", "enum": ["SUBMITTED"]});
        properties["task_state"] = json!({"type": "string", "enum": ["DRAFT"]});
        properties["result_digest"] = json!({"type": "null"});
        properties["failure_stage"] = json!({"type": "null"});
        properties["failure_code"] = json!({"type": "null"});
    }
    let mut required = vec![
        "schema_version",
        "status",
        "task_state",
        "task_ref",
        "ledger_head_digest",
        "result_digest",
        "failure_stage",
        "failure_code",
    ];
    if general {
        let object = properties
            .as_object_mut()
            .expect("status properties object");
        object.insert(
            "objective".to_owned(),
            json!({"type": "string", "minLength": 1, "maxLength": MAX_TASK_OBJECTIVE_CHARS}),
        );
        object.insert(
            "project_id".to_owned(),
            json!({"type": "string", "minLength": 2, "maxLength": MAX_PROJECT_ID_BYTES, "pattern": "^[a-z0-9][a-z0-9._-]{1,63}$"}),
        );
        object.insert(
            "project_name".to_owned(),
            json!({"type": "string", "minLength": 1, "maxLength": MAX_PROJECT_NAME_CHARS}),
        );
        object.insert(
            "project_snapshot_id".to_owned(),
            json!({"type": "string", "minLength": 1, "maxLength": MAX_PROJECT_SNAPSHOT_ID_BYTES, "pattern": "^[A-Za-z0-9][A-Za-z0-9._:-]{0,158}$"}),
        );
        required.extend([
            "objective",
            "project_id",
            "project_name",
            "project_snapshot_id",
        ]);
    }
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false
    })
}

fn managed_task_public_status_variant_schema() -> Value {
    let mut schema = task_public_status_variant_schema(true);
    let properties = schema["properties"]
        .as_object_mut()
        .expect("managed task status properties");
    properties.remove("objective");
    properties.insert(
        "objective_summary".to_owned(),
        json!({"type": "string", "enum": [TASK_PUBLIC_OBJECTIVE_SUMMARY]}),
    );
    properties.insert("objective_digest".to_owned(), lower_sha256_schema());
    properties["schema_version"] =
        json!({"type": "string", "enum": [TASK_PUBLIC_STATUS_SCHEMA_V4]});
    properties["status"] = json!({
        "type": "string",
        "enum": TASK_MANAGED_PUBLIC_STATUS_VALUES
    });
    properties["task_state"] = json!({
        "type": "string",
        "enum": TASK_PUBLIC_STATE_VALUES
    });
    properties["result_digest"] = json!({
        "anyOf": [lower_sha256_schema(), {"type": "null"}]
    });
    for field in ["failure_stage", "failure_code"] {
        properties[field] = json!({
            "anyOf": [
                {"type": "string", "minLength": 1, "maxLength": 128, "pattern": "^[A-Z0-9_]+$"},
                {"type": "null"}
            ]
        });
    }
    properties.insert("worker_running".to_owned(), json!({"type": "boolean"}));
    properties.insert(
        "attempt".to_owned(),
        json!({"anyOf": [{"type": "integer", "minimum": 1, "maximum": 3}, {"type": "null"}]}),
    );
    properties.insert(
        "retry_count".to_owned(),
        json!({"type": "integer", "minimum": 0, "maximum": 2}),
    );
    properties.insert(
        "model".to_owned(),
        json!({"anyOf": [{"type": "string", "enum": TASK_MANAGED_MODEL_VALUES}, {"type": "null"}]}),
    );
    properties.insert(
        "reasoning".to_owned(),
        json!({"anyOf": [{"type": "string", "enum": TASK_MANAGED_REASONING_VALUES}, {"type": "null"}]}),
    );
    let identifier_schema = json!({
        "anyOf": [
            {"type": "string", "minLength": 1, "maxLength": 256, "pattern": "^[A-Za-z0-9][A-Za-z0-9._:-]{0,255}$"},
            {"type": "null"}
        ]
    });
    properties.insert("thread_id".to_owned(), identifier_schema.clone());
    properties.insert("turn_id".to_owned(), identifier_schema);
    properties.insert(
        "last_progress_at".to_owned(),
        json!({"anyOf": [{"type": "string", "format": "date-time", "pattern": "Z$"}, {"type": "null"}]}),
    );
    properties.insert(
        "blocker".to_owned(),
        json!({"anyOf": [{"type": "string", "minLength": 1, "maxLength": 128, "pattern": "^[A-Z0-9_]+$"}, {"type": "null"}]}),
    );
    properties.insert(
        "verification_status".to_owned(),
        json!({"type": "string", "enum": TASK_MANAGED_VERIFICATION_VALUES}),
    );
    let nullable_digest_schema = json!({
        "anyOf": [lower_sha256_schema(), {"type": "null"}]
    });
    properties.insert(
        "verification_digest".to_owned(),
        nullable_digest_schema.clone(),
    );
    properties.insert("evidence_digest".to_owned(), nullable_digest_schema);
    properties.insert(
        "resource_observation".to_owned(),
        json!({
            "anyOf": [managed_resource_observation_schema(), {"type": "null"}]
        }),
    );
    properties.insert(
        "next_action".to_owned(),
        json!({"type": "string", "minLength": 1, "maxLength": 256}),
    );
    properties.insert(
        "foreman_generation".to_owned(),
        json!({"type": "integer", "minimum": 1}),
    );
    properties.insert(
        "foreman_checkpoint_digest".to_owned(),
        lower_sha256_schema(),
    );

    let required = schema["required"]
        .as_array_mut()
        .expect("managed task status required fields");
    required.retain(|field| field.as_str() != Some("objective"));
    required.extend(
        [
            "objective_summary",
            "objective_digest",
            "worker_running",
            "attempt",
            "retry_count",
            "model",
            "reasoning",
            "thread_id",
            "turn_id",
            "last_progress_at",
            "blocker",
            "verification_status",
            "verification_digest",
            "evidence_digest",
            "resource_observation",
            "next_action",
            "foreman_generation",
            "foreman_checkpoint_digest",
        ]
        .into_iter()
        .map(|field| json!(field)),
    );
    schema
}

fn managed_resource_observation_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "scope": {"type": "string", "enum": ["TASK_CUMULATIVE"]},
            "attempts_observed": {"type": "integer", "minimum": 0},
            "model_calls": {"type": "integer", "minimum": 0},
            "remaining_model_calls": {"type": "integer", "minimum": 0},
            "remaining_total_tokens": nullable_non_negative_integer_schema(),
            "input_tokens": nullable_non_negative_integer_schema(),
            "cached_input_tokens": nullable_non_negative_integer_schema(),
            "output_tokens": nullable_non_negative_integer_schema(),
            "reasoning_output_tokens": nullable_non_negative_integer_schema(),
            "total_tokens": nullable_non_negative_integer_schema(),
            "external_cost_status": {"type": "string", "enum": ["UNAVAILABLE"]}
        },
        "required": [
            "scope", "attempts_observed", "model_calls",
            "remaining_model_calls", "remaining_total_tokens",
            "input_tokens", "cached_input_tokens", "output_tokens",
            "reasoning_output_tokens", "total_tokens", "external_cost_status"
        ],
        "additionalProperties": false
    })
}

fn nullable_non_negative_integer_schema() -> Value {
    json!({
        "anyOf": [
            {"type": "integer", "minimum": 0},
            {"type": "null"}
        ]
    })
}

fn lower_sha256_schema() -> Value {
    json!({
        "type": "string",
        "minLength": 64,
        "maxLength": 64,
        "pattern": "^[0-9a-f]{64}$"
    })
}

fn tool_catalog(protocol: RequestProtocol, surface: ToolSurface) -> Value {
    let delivery_run_description = if surface.allows_delivery_run() {
        "Runs the one LATTICE-owned delivery profile using server configuration."
    } else {
        "Legacy name retained for compatibility; mutation requires the canonical latticed entrypoint."
    };
    let mut tools = vec![
        json!({
            "name": DELIVERY_RUN_TOOL,
            "title": "Run LATTICE delivery",
            "description": delivery_run_description,
            "inputSchema": delivery_arguments_schema()
        }),
        json!({
            "name": DELIVERY_STATUS_TOOL,
            "title": "Read LATTICE delivery status",
            "description": "Reads the durable status for the one LATTICE-owned delivery profile.",
            "inputSchema": delivery_arguments_schema()
        }),
    ];
    if surface.allows_task_control() {
        tools.extend([
            json!({
                "name": TASK_SUBMIT_TOOL,
                "title": "Submit a bounded LATTICE task",
                "description": "Creates a durable general LATTICE task for one registered project, or runs the retained controlled canary. Submitting an objective grants no authority. In managed ACTIVE mode, the foreman may dispatch asynchronously only after an independently verified, task/spec/budget-bound local execution gate; in DISABLED mode general intake remains create-only. Merge, deployment, payment, and external-action authority stay separate.",
                "inputSchema": task_submit_arguments_schema(),
                "outputSchema": task_public_status_schema()
            }),
            json!({
                "name": TASK_STATUS_TOOL,
                "title": "Read bounded LATTICE task status",
                "description": "Reads durable status for one validated task reference. General tasks need only task_ref; client_request_id remains optional for legacy canary compatibility.",
                "inputSchema": task_status_arguments_schema(),
                "outputSchema": task_public_status_schema()
            }),
        ]);
        tools.extend([
            json!({
                "name": RUNTIME_STATUS_TOOL,
                "title": "Read LATTICE Runtime component status",
                "description": "Reads PostgreSQL, Graphify, and Hermes activation or degradation state without starting optional components.",
                "inputSchema": delivery_arguments_schema()
            }),
            json!({
                "name": DELIVERY_RECONCILE_TOOL,
                "title": "Reconcile LATTICE delivery evidence",
                "description": "Replays the durable delivery receipt to determine whether reconciliation is required. It never starts work or changes durable evidence.",
                "inputSchema": delivery_arguments_schema()
            }),
            json!({
                "name": FOREMAN_CHECKPOINT_TOOL,
                "title": "Checkpoint the sole LATTICE foreman",
                "description": "Records or exactly replays one closed durable foreman checkpoint through the existing orchestrator.",
                "inputSchema": foreman_checkpoint_arguments_schema(),
                "outputSchema": foreman_checkpoint_output_schema()
            }),
        ]);
    }
    if protocol == RequestProtocol::Stateless {
        tools[0]["annotations"] = json!({
            "readOnlyHint": false,
            "destructiveHint": true,
            "idempotentHint": false,
            "openWorldHint": false
        });
        tools[1]["annotations"] = json!({
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true,
            "openWorldHint": false
        });
        if surface.allows_task_control() {
            tools[2]["annotations"] = json!({
                "readOnlyHint": false,
                "destructiveHint": true,
                "idempotentHint": true,
                "openWorldHint": false
            });
            tools[3]["annotations"] = json!({
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true,
                "openWorldHint": false
            });
            tools[4]["annotations"] = json!({
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true,
                "openWorldHint": false
            });
            tools[5]["annotations"] = json!({
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true,
                "openWorldHint": false
            });
            tools[6]["annotations"] = json!({
                "readOnlyHint": false,
                "destructiveHint": false,
                "idempotentHint": true,
                "openWorldHint": false
            });
        }
    }
    Value::Array(tools)
}

fn server_result_meta() -> Value {
    json!({
        META_SERVER_INFO: {
            "name": "latticed",
            "title": "LATTICE DevOS",
            "version": "1.0.0"
        }
    })
}

fn request_protocol(params: Option<&Value>) -> Result<RequestProtocol, RequestProtocolError> {
    let Some(params) = params.and_then(Value::as_object) else {
        return Ok(RequestProtocol::Legacy);
    };
    let Some(metadata) = params.get("_meta") else {
        return Ok(RequestProtocol::Legacy);
    };
    let Some(metadata) = metadata.as_object() else {
        return Err(RequestProtocolError::InvalidMetadata);
    };
    let has_modern_metadata = [
        META_PROTOCOL_VERSION,
        META_CLIENT_INFO,
        META_CLIENT_CAPABILITIES,
        META_LOG_LEVEL,
    ]
    .iter()
    .any(|key| metadata.contains_key(*key));
    if !has_modern_metadata {
        return Ok(RequestProtocol::Legacy);
    }
    let Some(version) = metadata.get(META_PROTOCOL_VERSION) else {
        return Err(RequestProtocolError::InvalidMetadata);
    };
    let Some(version) = version
        .as_str()
        .filter(|version| valid_protocol_version(version))
    else {
        return Err(RequestProtocolError::InvalidMetadata);
    };
    if version != MCP_STATELESS_PROTOCOL_VERSION {
        return Err(RequestProtocolError::Unsupported(version.to_owned()));
    }
    if !metadata
        .get(META_CLIENT_CAPABILITIES)
        .is_some_and(valid_client_capabilities)
        || metadata
            .get(META_CLIENT_INFO)
            .is_some_and(|value| !valid_implementation(value))
        || metadata
            .get(META_LOG_LEVEL)
            .is_some_and(|value| !valid_logging_level(value))
        || metadata
            .get("progressToken")
            .is_some_and(|value| !value.is_string() && !value.is_number())
    {
        return Err(RequestProtocolError::InvalidMetadata);
    }
    Ok(RequestProtocol::Stateless)
}

fn valid_protocol_version(version: &str) -> bool {
    version.len() == 10
        && version
            .bytes()
            .enumerate()
            .all(|(index, byte)| match index {
                4 | 7 => byte == b'-',
                _ => byte.is_ascii_digit(),
            })
}

fn valid_implementation(value: &Value) -> bool {
    value.as_object().is_some_and(|implementation| {
        ["name", "version"]
            .iter()
            .all(|field| implementation.get(*field).is_some_and(Value::is_string))
            && ["title", "description", "websiteUrl"]
                .iter()
                .all(|field| implementation.get(*field).is_none_or(Value::is_string))
            && implementation.get("icons").is_none_or(|icons| {
                icons
                    .as_array()
                    .is_some_and(|icons| icons.iter().all(valid_icon))
            })
    })
}

fn valid_icon(value: &Value) -> bool {
    value.as_object().is_some_and(|icon| {
        icon.get("src").is_some_and(Value::is_string)
            && icon.get("mimeType").is_none_or(Value::is_string)
            && icon.get("sizes").is_none_or(|sizes| {
                sizes
                    .as_array()
                    .is_some_and(|sizes| sizes.iter().all(Value::is_string))
            })
            && icon.get("theme").is_none_or(|theme| {
                theme
                    .as_str()
                    .is_some_and(|theme| matches!(theme, "light" | "dark"))
            })
    })
}

fn valid_client_capabilities(value: &Value) -> bool {
    value.as_object().is_some_and(|capabilities| {
        capabilities
            .get("experimental")
            .is_none_or(object_values_are_objects)
            && capabilities.get("roots").is_none_or(Value::is_object)
            && capabilities
                .get("sampling")
                .is_none_or(|sampling| object_with_object_fields(sampling, &["context", "tools"]))
            && capabilities
                .get("elicitation")
                .is_none_or(|elicitation| object_with_object_fields(elicitation, &["form", "url"]))
            && capabilities.get("extensions").is_none_or(valid_extensions)
    })
}

fn object_values_are_objects(value: &Value) -> bool {
    value
        .as_object()
        .is_some_and(|object| object.values().all(Value::is_object))
}

fn valid_extensions(value: &Value) -> bool {
    value.as_object().is_some_and(|extensions| {
        extensions
            .iter()
            .all(|(key, value)| valid_prefixed_meta_key(key) && value.is_object())
    })
}

fn valid_prefixed_meta_key(key: &str) -> bool {
    let Some((prefix, name)) = key.split_once('/') else {
        return false;
    };
    !prefix.is_empty()
        && !name.contains('/')
        && prefix.split('.').all(valid_meta_prefix_label)
        && valid_meta_name(name)
}

fn valid_meta_prefix_label(label: &str) -> bool {
    let bytes = label.as_bytes();
    bytes.first().is_some_and(u8::is_ascii_alphabetic)
        && bytes.last().is_some_and(u8::is_ascii_alphanumeric)
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
}

fn valid_meta_name(name: &str) -> bool {
    if name.is_empty() {
        return true;
    }
    let bytes = name.as_bytes();
    bytes.first().is_some_and(u8::is_ascii_alphanumeric)
        && bytes.last().is_some_and(u8::is_ascii_alphanumeric)
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'-' | b'_' | b'.'))
}

fn object_with_object_fields(value: &Value, fields: &[&str]) -> bool {
    value.as_object().is_some_and(|object| {
        fields
            .iter()
            .all(|field| object.get(*field).is_none_or(Value::is_object))
    })
}

fn valid_logging_level(value: &Value) -> bool {
    value.as_str().is_some_and(|level| {
        matches!(
            level,
            "debug" | "info" | "notice" | "warning" | "error" | "critical" | "alert" | "emergency"
        )
    })
}

fn valid_client_request_id(value: &str) -> bool {
    valid_task_ingress_client_request_id(value)
}

fn valid_task_objective(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= MAX_TASK_OBJECTIVE_BYTES
        && value.chars().count() <= MAX_TASK_OBJECTIVE_CHARS
        && value.trim() == value
        && is_nfc(value)
        && !value.chars().any(char::is_control)
        && !task_submission_text_contains_secret(value)
}

fn valid_project_name(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= MAX_PROJECT_NAME_BYTES
        && value.chars().count() <= MAX_PROJECT_NAME_CHARS
        && value.trim() == value
        && is_nfc(value)
        && !value.chars().any(char::is_control)
        && !task_submission_text_contains_secret(value)
}

fn valid_project_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    (2..=MAX_PROJECT_ID_BYTES).contains(&bytes.len())
        && bytes
            .first()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && bytes.iter().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(*byte, b'.' | b'_' | b'-')
        })
        && !task_submission_text_contains_secret(value)
}

fn valid_project_snapshot_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= MAX_PROJECT_SNAPSHOT_ID_BYTES
        && bytes.first().is_some_and(u8::is_ascii_alphanumeric)
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'.' | b'_' | b':' | b'-'))
        && !task_submission_text_contains_secret(value)
}

fn valid_task_ref(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_git_sha(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_evidence_ref(value: &str) -> bool {
    value
        .strip_prefix("evidence:sha256:")
        .is_some_and(valid_task_ref)
}

fn closed_task_public_status(
    result: Result<Value, ToolExecutionError>,
) -> Result<Value, ToolExecutionError> {
    result.and_then(|value| {
        TaskPublicStatus::from_value(&value)
            .filter(|status| status.schema_version != TASK_PUBLIC_STATUS_SCHEMA_V3)
            .map(TaskPublicStatus::into_value)
            .ok_or_else(|| ToolExecutionError::new("LATTICE_TASK_PUBLIC_STATUS_REJECTED"))
    })
}

fn closed_foreman_checkpoint_result(
    result: Result<Value, ToolExecutionError>,
) -> Result<Value, ToolExecutionError> {
    result.and_then(|value| {
        let valid = value.as_object().is_some_and(|object| {
            object.len() == 7
                && object.get("schema").and_then(Value::as_str)
                    == Some(FOREMAN_CHECKPOINT_RESULT_SCHEMA)
                && object
                    .get("checkpoint_id")
                    .and_then(Value::as_str)
                    .is_some_and(valid_client_request_id)
                && object
                    .get("generation")
                    .and_then(Value::as_u64)
                    .is_some_and(|generation| generation > 0)
                && matches!(
                    (
                        object.get("status").and_then(Value::as_str),
                        object.get("exact_retry").and_then(Value::as_bool)
                    ),
                    (Some("RECORDED"), Some(false)) | (Some("REPLAYED"), Some(true))
                )
                && object
                    .get("ledger_digest")
                    .and_then(Value::as_str)
                    .is_some_and(valid_task_ref)
                && object
                    .get("checkpoint_digest")
                    .and_then(Value::as_str)
                    .is_some_and(valid_task_ref)
        });
        if valid {
            Ok(value)
        } else {
            Err(ToolExecutionError::new(
                "FOREMAN_CHECKPOINT_RESULT_REJECTED",
            ))
        }
    })
}

fn empty_object_or_absent(value: Option<&Value>) -> bool {
    value.is_none_or(|value| value.as_object().is_some_and(Map::is_empty))
}

fn metadata_object_or_absent(value: Option<&Value>) -> bool {
    value.is_none_or(Value::is_object)
}

fn metadata_only_object_or_absent(value: Option<&Value>) -> bool {
    value.is_none_or(|value| {
        value.as_object().is_some_and(|object| {
            object.keys().all(|key| key == "_meta")
                && metadata_object_or_absent(object.get("_meta"))
        })
    })
}

fn valid_request_id(value: &Value) -> bool {
    match value {
        Value::String(_) => true,
        Value::Number(number) => number.is_i64() || number.is_u64(),
        _ => false,
    }
}

fn tool_result(result: Result<Value, ToolExecutionError>) -> Value {
    match result {
        Ok(value) => json!({
            "content": [{"type": "text", "text": value.to_string()}],
            "structuredContent": value,
            "isError": false
        }),
        Err(error) => {
            let value = json!({"status": "ERROR", "code": error.code()});
            json!({
                "content": [{"type": "text", "text": value.to_string()}],
                "structuredContent": value,
                "isError": true
            })
        }
    }
}

fn budget_rejection_result(
    receipt: crate::mcp_budget::McpRejectionReceipt,
    remaining_calls: u16,
    read_only_reserve: u16,
) -> Value {
    let can_continue_read_only = remaining_calls > 0;
    let value = json!({
        "schema_version": "lattice.mcp.handoff.v1",
        "status": "REJECTED",
        "code": receipt.code,
        "reason": receipt.reason,
        "effect_started": receipt.effect_was_started,
        "retry_allowed": receipt.retry_allowed,
        "handoff_required": receipt.handoff_required,
        "remaining_read_only_calls": remaining_calls,
        "reserved_read_only_calls": read_only_reserve,
        "can_do": if can_continue_read_only {
            json!([RUNTIME_STATUS_TOOL, DELIVERY_RECONCILE_TOOL, DELIVERY_STATUS_TOOL, TASK_STATUS_TOOL])
        } else {
            json!(["start a fresh MCP session, then use read-only status or reconciliation tools"])
        },
        "cannot_do": [DELIVERY_RUN_TOOL, TASK_SUBMIT_TOOL],
        "resume_instruction": if can_continue_read_only {
            "Use a read-only status or reconciliation tool now; do not retry execution in this session."
        } else {
            "Start a fresh MCP session, inspect durable status, then decide whether execution may be resumed."
        }
    });
    json!({
        "content": [{"type": "text", "text": value.to_string()}],
        "structuredContent": value,
        "isError": true
    })
}

fn success(id: Value, result: Value) -> Value {
    let mut response = Map::new();
    response.insert("jsonrpc".to_owned(), Value::String("2.0".to_owned()));
    response.insert("id".to_owned(), id);
    response.insert("result".to_owned(), result);
    Value::Object(response)
}

fn protocol_error(id: Value, code: i32, message: &'static str) -> Value {
    let mut error = Map::new();
    error.insert("code".to_owned(), Value::from(code));
    error.insert("message".to_owned(), Value::String(message.to_owned()));
    let mut response = Map::new();
    response.insert("jsonrpc".to_owned(), Value::String("2.0".to_owned()));
    response.insert("id".to_owned(), id);
    response.insert("error".to_owned(), Value::Object(error));
    Value::Object(response)
}

fn protocol_error_with_machine_code(
    id: Value,
    code: i32,
    message: &'static str,
    machine_code: &'static str,
) -> Value {
    let mut response = protocol_error(id, code, message);
    response["error"]["data"] = json!({"code": machine_code});
    response
}

fn unsupported_protocol_error(id: Value, requested: &str) -> Value {
    let mut error = Map::new();
    error.insert("code".to_owned(), Value::from(-32022));
    error.insert(
        "message".to_owned(),
        Value::String("Unsupported protocol version".to_owned()),
    );
    error.insert(
        "data".to_owned(),
        json!({
            "supported": [MCP_STATELESS_PROTOCOL_VERSION, MCP_PROTOCOL_VERSION],
            "requested": requested
        }),
    );
    let mut response = Map::new();
    response.insert("jsonrpc".to_owned(), Value::String("2.0".to_owned()));
    response.insert("id".to_owned(), id);
    response.insert("error".to_owned(), Value::Object(error));
    Value::Object(response)
}

#[cfg(test)]
mod acceptance_evidence_tests {
    use super::{
        ACCEPTANCE_EVIDENCE_SCHEMA, AcceptanceEvidence, DELIVERY_RECONCILE_TOOL,
        OBSERVED_EFFECT_EVIDENCE_SCHEMA, ObservedEffectEvidence, ObservedEffectKind,
        RUNTIME_STATUS_TOOL, RequestProtocol, TASK_SUBMIT_TOOL, TaskPublicStatus, ToolSurface,
        closed_task_public_status, tool_catalog, verify_observed_effect_evidence,
    };
    use serde_json::{Value, json};
    use std::fs::File;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fresh_sink(label: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "lattice-mcp-acceptance-{label}-{}-{unique}.jsonl",
            std::process::id()
        ));
        File::create(&path).expect("create fresh acceptance sink");
        path
    }

    #[test]
    fn canonical_catalog_exposes_read_only_runtime_status() {
        let tools = tool_catalog(
            RequestProtocol::Stateless,
            ToolSurface::CanonicalTaskControl,
        );
        let runtime = tools
            .as_array()
            .expect("tool array")
            .iter()
            .find(|tool| tool["name"] == RUNTIME_STATUS_TOOL)
            .expect("runtime status tool");
        assert_eq!(runtime["inputSchema"]["additionalProperties"], false);
        assert_eq!(runtime["annotations"]["readOnlyHint"], true);
        assert_eq!(runtime["annotations"]["destructiveHint"], false);
    }

    #[test]
    fn task_submit_catalog_is_mode_neutral_and_keeps_execution_authority_separate() {
        let tools = tool_catalog(
            RequestProtocol::Stateless,
            ToolSurface::CanonicalTaskControl,
        );
        let submit = tools
            .as_array()
            .expect("tool array")
            .iter()
            .find(|tool| tool["name"] == TASK_SUBMIT_TOOL)
            .expect("task submit tool");
        let description = submit["description"].as_str().expect("description");
        assert!(description.contains("Submitting an objective grants no authority"));
        assert!(description.contains("managed ACTIVE mode"));
        assert!(description.contains("independently verified, task/spec/budget-bound"));
        assert!(description.contains("DISABLED mode general intake remains create-only"));
        assert!(
            description.contains(
                "Merge, deployment, payment, and external-action authority stay separate"
            )
        );
        assert!(!description.contains("General creation does not start an Agent"));
    }

    #[test]
    fn canonical_catalog_exposes_a_zero_argument_read_only_reconciliation_probe() {
        let tools = tool_catalog(
            RequestProtocol::Stateless,
            ToolSurface::CanonicalTaskControl,
        );
        let reconciliation = tools
            .as_array()
            .expect("tool array")
            .iter()
            .find(|tool| tool["name"] == DELIVERY_RECONCILE_TOOL)
            .expect("delivery reconciliation tool");
        assert_eq!(reconciliation["inputSchema"]["additionalProperties"], false);
        assert_eq!(reconciliation["annotations"]["readOnlyHint"], true);
        assert_eq!(reconciliation["annotations"]["destructiveHint"], false);
        assert_eq!(reconciliation["annotations"]["idempotentHint"], true);
    }

    #[test]
    fn task_public_status_exposes_only_closed_failure_atoms() {
        let digest = "ab".repeat(32);
        let valid = json!({
            "schema_version": "lattice.task.status.v2",
            "status": "FAILED",
            "task_state": "FAILED",
            "task_ref": digest,
            "ledger_head_digest": "cd".repeat(32),
            "result_digest": null,
            "failure_stage": "CODEX",
            "failure_code": "LATTICE_DELIVERY_FAILED"
        });
        assert!(TaskPublicStatus::from_value(&valid).is_some());
        assert_eq!(
            closed_task_public_status(Ok(valid)).expect("closed public status")["failure_code"],
            "LATTICE_DELIVERY_FAILED"
        );

        let invalid = json!({
            "schema_version": "lattice.task.status.v2",
            "status": "FAILED",
            "task_state": "FAILED",
            "task_ref": "ab".repeat(32),
            "ledger_head_digest": "cd".repeat(32),
            "result_digest": null,
            "failure_stage": "CODEX",
            "failure_code": "path=C:\\\\secret"
        });
        assert!(TaskPublicStatus::from_value(&invalid).is_none());
    }

    #[test]
    fn legacy_v3_decoder_is_retained_but_closed_service_output_rejects_objective_disclosure() {
        let legacy = json!({
            "schema_version": "lattice.task.status.v3",
            "status": "SUBMITTED",
            "task_state": "DRAFT",
            "task_ref": "ab".repeat(32),
            "ledger_head_digest": "cd".repeat(32),
            "result_digest": null,
            "failure_stage": null,
            "failure_code": null,
            "objective": "Internal acquisition codename Quiet Orchard",
            "project_id": "private-project",
            "project_name": "Confidential Planning",
            "project_snapshot_id": "private-project:snapshot:1"
        });
        assert!(TaskPublicStatus::from_value(&legacy).is_some());
        assert_eq!(
            closed_task_public_status(Ok(legacy))
                .expect_err("new service output must reject legacy objective disclosure")
                .code(),
            "LATTICE_TASK_PUBLIC_STATUS_REJECTED"
        );
    }

    #[test]
    fn dispatch_evidence_is_a_durable_open_dispatch_close_hash_chain() {
        let path = fresh_sink("chain");
        let session_id = "0123456789abcdef0123456789abcdef".to_owned();
        let safe_config_sha256 = "ab".repeat(32);
        let mut evidence =
            AcceptanceEvidence::open(&path, session_id.clone(), safe_config_sha256.clone())
                .expect("open acceptance evidence");
        evidence
            .record_dispatch("lattice_task_status", &json!(17))
            .expect("record accepted dispatch");
        evidence.close().expect("close acceptance evidence");
        drop(evidence);

        let text = std::fs::read_to_string(&path).expect("read acceptance evidence");
        assert!(text.ends_with('\n'));
        let records = text
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).expect("valid JSONL record"))
            .collect::<Vec<_>>();
        assert_eq!(records.len(), 3);
        assert_eq!(records[0]["schema"], ACCEPTANCE_EVIDENCE_SCHEMA);
        assert_eq!(records[0]["record_type"], "SESSION_OPEN");
        assert_eq!(records[1]["record_type"], "DISPATCH_ACCEPTED");
        assert_eq!(records[1]["tool_name"], "lattice_task_status");
        assert_eq!(records[2]["record_type"], "SESSION_CLOSED");
        assert_eq!(records[2]["dispatch_accepted_count"], 1);
        assert_eq!(records[0]["previous_event_sha256"], "0".repeat(64));
        assert_eq!(
            records[1]["previous_event_sha256"],
            records[0]["event_sha256"]
        );
        assert_eq!(
            records[2]["previous_event_sha256"],
            records[1]["event_sha256"]
        );
        assert_eq!(records[2]["session_id"], session_id);
        assert_eq!(records[2]["safe_config_sha256"], safe_config_sha256);
        std::fs::remove_file(path).expect("remove test sink");
    }

    #[test]
    fn dispatch_evidence_rejects_nonfresh_or_noncanonical_configuration() {
        let nonfresh_path = fresh_sink("nonfresh");
        std::fs::write(&nonfresh_path, b"existing\n").expect("seed nonfresh sink");
        let nonfresh = AcceptanceEvidence::open(
            &nonfresh_path,
            "0123456789abcdef0123456789abcdef".to_owned(),
            "cd".repeat(32),
        );
        assert!(nonfresh.is_err());
        std::fs::remove_file(nonfresh_path).expect("remove nonfresh sink");

        let uppercase_path = fresh_sink("uppercase");
        let uppercase = AcceptanceEvidence::open(
            &uppercase_path,
            "0123456789abcdef0123456789abcdeF".to_owned(),
            "ef".repeat(32),
        );
        assert!(uppercase.is_err());
        std::fs::remove_file(uppercase_path).expect("remove uppercase sink");
    }

    #[test]
    fn observed_effect_evidence_distinguishes_rejected_zero_from_transient_effects() {
        let path = fresh_sink("observed-effects");
        let session_id = "1123456789abcdef0123456789abcdef".to_owned();
        let safe_config_sha256 = "12".repeat(32);
        let nonce = "34".repeat(32);
        let mut evidence = ObservedEffectEvidence::open(
            &path,
            session_id.clone(),
            safe_config_sha256.clone(),
            nonce.clone(),
        )
        .expect("open observed-effect evidence");

        evidence
            .begin_probe(
                "02n01-invalid-task-submit",
                "lattice_task_submit",
                &json!(2),
            )
            .expect("begin rejected probe");
        evidence
            .reject_probe("MCP_INVALID_PARAMS")
            .expect("record rejected probe");

        evidence
            .begin_probe("03-transient-effect", "lattice_task_submit", &json!(3))
            .expect("begin effect probe");
        evidence.accept_dispatch().expect("record dispatch");
        evidence
            .record_effect(ObservedEffectKind::Database)
            .expect("record database connect attempt");
        evidence
            .record_effect(ObservedEffectKind::Filesystem)
            .expect("record filesystem write attempt");
        evidence
            .record_effect(ObservedEffectKind::Process)
            .expect("record transient process start");
        evidence
            .record_effect(ObservedEffectKind::Network)
            .expect("record transient network connect");
        evidence
            .record_effect(ObservedEffectKind::Codex)
            .expect("record transient Codex request");
        evidence
            .complete_probe("MCP_TOOL_ERROR")
            .expect("complete effect probe");
        evidence.close().expect("close observed-effect evidence");
        drop(evidence);

        let bytes = std::fs::read(&path).expect("read observed-effect evidence");
        let verified = verify_observed_effect_evidence(
            &bytes,
            &session_id,
            &safe_config_sha256,
            &nonce,
            SystemTime::now(),
        )
        .expect("verify observed-effect evidence");
        assert_eq!(verified.schema, OBSERVED_EFFECT_EVIDENCE_SCHEMA);
        assert_eq!(verified.rejected_probe_count, 1);
        assert_eq!(verified.dispatch_count, 1);
        assert_eq!(verified.database_effect_count, 1);
        assert_eq!(verified.filesystem_effect_count, 1);
        assert_eq!(verified.process_effect_count, 1);
        assert_eq!(verified.network_effect_count, 1);
        assert_eq!(verified.codex_effect_count, 1);
        assert!(verified.normal_close_complete);

        let records = String::from_utf8(bytes.clone())
            .expect("strict UTF-8")
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).expect("valid effect JSONL"))
            .collect::<Vec<_>>();
        let rejected = records
            .iter()
            .find(|record| record["record_type"] == "PROBE_REJECTED")
            .expect("rejected probe receipt");
        assert_eq!(rejected["classification"], "MCP_INVALID_PARAMS");
        assert_eq!(rejected["probe_counters"]["dispatch"], 0);
        assert_eq!(rejected["probe_counters"]["database"], 0);
        assert_eq!(rejected["probe_counters"]["filesystem"], 0);
        assert_eq!(rejected["probe_counters"]["process"], 0);
        assert_eq!(rejected["probe_counters"]["network"], 0);
        assert_eq!(rejected["probe_counters"]["codex"], 0);

        let mut reordered = records.clone();
        reordered.swap(1, 2);
        let reordered_bytes = reordered
            .iter()
            .flat_map(|record| {
                let mut bytes = serde_json::to_vec(record).expect("serialize mutation");
                bytes.push(b'\n');
                bytes
            })
            .collect::<Vec<_>>();
        assert!(
            verify_observed_effect_evidence(
                &reordered_bytes,
                &session_id,
                &safe_config_sha256,
                &nonce,
                SystemTime::now(),
            )
            .is_err()
        );
        assert!(
            verify_observed_effect_evidence(
                &bytes,
                "2123456789abcdef0123456789abcdef",
                &safe_config_sha256,
                &nonce,
                SystemTime::now(),
            )
            .is_err()
        );
        assert!(
            verify_observed_effect_evidence(
                &bytes,
                &session_id,
                &safe_config_sha256,
                &"56".repeat(32),
                SystemTime::now(),
            )
            .is_err()
        );

        let mut missing = records.clone();
        missing.remove(2);
        let missing_bytes = missing
            .iter()
            .flat_map(|record| {
                let mut bytes = serde_json::to_vec(record).expect("serialize missing mutation");
                bytes.push(b'\n');
                bytes
            })
            .collect::<Vec<_>>();
        assert!(
            verify_observed_effect_evidence(
                &missing_bytes,
                &session_id,
                &safe_config_sha256,
                &nonce,
                SystemTime::now(),
            )
            .is_err()
        );

        let mut duplicated = records.clone();
        duplicated.insert(2, records[2].clone());
        let duplicated_bytes = duplicated
            .iter()
            .flat_map(|record| {
                let mut bytes = serde_json::to_vec(record).expect("serialize duplicate mutation");
                bytes.push(b'\n');
                bytes
            })
            .collect::<Vec<_>>();
        assert!(
            verify_observed_effect_evidence(
                &duplicated_bytes,
                &session_id,
                &safe_config_sha256,
                &nonce,
                SystemTime::now(),
            )
            .is_err()
        );

        assert!(
            verify_observed_effect_evidence(
                &bytes,
                &session_id,
                &safe_config_sha256,
                &nonce,
                UNIX_EPOCH + std::time::Duration::from_secs(1),
            )
            .is_err()
        );

        std::fs::remove_file(path).expect("remove observed-effect sink");
    }
}
