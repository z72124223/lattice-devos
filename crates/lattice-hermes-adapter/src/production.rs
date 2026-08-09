//! Unique owned Windows -> WSL2 -> bubblewrap Hermes construction chain.

use std::collections::HashSet;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use lattice_contracts::{ContentDigest, HermesEvidence, HermesResearchRequest, RequestId};
use lattice_ports::{HermesPort, PortError, PortResult};
use serde::de::{IgnoredAny, MapAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::HERMES_SCHEMA_VERSION;
use crate::broker::{CodexBrokerPreflightReceipt, CodexReflectionBrokerConfig};
use crate::codex_proxy::{
    ProductionCodexProxyControl, ProductionCodexProxyDuplex, ProductionCodexProxyProvider,
};
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
const CONFIG_SCHEMA: &str = "lattice.hermes.production-config.v2";
const OFFICIAL_HERMES_CONFIG: &[u8] = br"_config_version: 33
model:
  provider: openai-api
  default: gpt-5.3-codex-spark
  openai_runtime: codex_app_server
  api_mode: codex_app_server
  base_url: http://127.0.0.1:9/v1
platform_toolsets:
  api_server: []
plugins:
  enabled: []
mcp_servers: {}
";
const BWRAP_SHA256: &str = "0abea81db798ebf6b4742ac0664802d97521547a353c2a0dbdc21d76cbbfd2c0";
const OFFICIAL_RUNTIME_GUEST_ROOT: &str = concat!(
    "/var/tmp/lattice-runtime-targets/",
    "hermes-v2026.8.3-cpython-3.12.13-pbs-20260804-errorfix-v1"
);
const OFFICIAL_RUNTIME_MANIFEST_SHA256: &str =
    "e3a3272b6cead30cd2df1af755df031766475595fdacfb080d0886671b6d1fbb";
const OFFICIAL_RUNTIME_TREE_SHA256: &str =
    "cb0e331bcb2b4fe2fd0977401d246819aadb800b645ca31ec233ad4e25b96929";
const OFFICIAL_RUNTIME_FILE_COUNT: u64 = 14_077;
const OFFICIAL_RUNTIME_BYTE_COUNT: u64 = 722_643_145;
const MAX_STARTUP_BYTES: usize = 128 * 1024;
const MAX_RUNNER_TIMEOUT: Duration = Duration::from_mins(5);
const CODEX_PROXY_MAGIC: &[u8] = b"LATTICE_HERMES_CODEX_PROXY_V1\n";
const CODEX_PROXY_BINDING_DOMAIN: &[u8] = b"LATTICE_HERMES_CODEX_PROXY_V1";
const CODEX_PROXY_STREAM_ID: u32 = 1;
const CODEX_PROXY_HEADER_BYTES: usize = 41;
const MAX_CODEX_PROXY_DATA_BYTES: usize = 65_536;
const MAX_CODEX_PROXY_JSONL_LINE_BYTES: usize = MAX_CODEX_PROXY_DATA_BYTES;
const MAX_CODEX_PROXY_JSONL_BATCH_BYTES: usize = MAX_CODEX_PROXY_DATA_BYTES * 2;
const MAX_CODEX_PROXY_BODY_BYTES: usize = CODEX_PROXY_HEADER_BYTES + MAX_CODEX_PROXY_DATA_BYTES;
const MAX_CODEX_PROXY_WIRE_BYTES: usize = CODEX_PROXY_MAGIC.len() + 4 + MAX_CODEX_PROXY_BODY_BYTES;
const MAX_CODEX_PROXY_BUFFER_BYTES: usize = MAX_CODEX_PROXY_WIRE_BYTES + 8192;
const CODEX_PROXY_TEARDOWN_TIMEOUT: Duration = Duration::from_secs(3);
static RUNNER_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CodexProxyHostEvent {
    Open,
    Data(Vec<u8>),
    Close,
    Error(u16),
    Terminal,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexProxyFailureEvidence {
    byte_count: u64,
    sha256: String,
}

impl CodexProxyFailureEvidence {
    #[must_use]
    pub const fn byte_count(&self) -> u64 {
        self.byte_count
    }

    #[must_use]
    pub fn sha256(&self) -> &str {
        &self.sha256
    }
}

pub(crate) struct CodexProxyHostSession {
    binding: [u8; 32],
    deadline: Instant,
    expected_sequence: u32,
    outbound_sequence: u32,
    inbound: CodexProxyInboundState,
    outbound: CodexProxyOutboundState,
    terminal: bool,
    failure_evidence: Option<CodexProxyFailureEvidence>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CodexProxyInboundState {
    AwaitOpen,
    Open,
    Closed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CodexProxyOutboundState {
    AwaitOpenAck,
    Open,
    Closed,
}

impl CodexProxyHostSession {
    pub(crate) fn new(
        nonce: &str,
        broker_receipt: &ContentDigest,
        deadline: Instant,
    ) -> HermesAdapterResult<Self> {
        if deadline <= Instant::now() {
            return Err(error(
                HermesAdapterErrorKind::Timeout,
                "HERMES_CODEX_PROXY_DEADLINE_EXCEEDED",
            ));
        }
        let nonce = decode_hex(nonce)
            .map_err(|_| malformed("HERMES_CODEX_PROXY_BINDING_INPUT_REJECTED"))?;
        let broker = decode_hex(broker_receipt.as_str())
            .map_err(|_| malformed("HERMES_CODEX_PROXY_BINDING_INPUT_REJECTED"))?;
        let digest = digest_join(&[&nonce, &broker, CODEX_PROXY_BINDING_DOMAIN]);
        let binding: [u8; 32] = decode_hex(&digest)
            .map_err(|_| malformed("HERMES_CODEX_PROXY_BINDING_INPUT_REJECTED"))?
            .try_into()
            .map_err(|_| malformed("HERMES_CODEX_PROXY_BINDING_INPUT_REJECTED"))?;
        Ok(Self {
            binding,
            deadline,
            expected_sequence: 0,
            outbound_sequence: 0,
            inbound: CodexProxyInboundState::AwaitOpen,
            outbound: CodexProxyOutboundState::AwaitOpenAck,
            terminal: false,
            failure_evidence: None,
        })
    }

    #[cfg(test)]
    pub(crate) const fn binding(&self) -> [u8; 32] {
        self.binding
    }

    pub(crate) fn failure_evidence(&self) -> Option<&CodexProxyFailureEvidence> {
        self.failure_evidence.as_ref()
    }

    const fn inbound_closed(&self) -> bool {
        matches!(self.inbound, CodexProxyInboundState::Closed)
    }

    const fn outbound_closed(&self) -> bool {
        matches!(self.outbound, CodexProxyOutboundState::Closed)
    }

    pub(crate) fn encode_open_ack(&mut self) -> HermesAdapterResult<Vec<u8>> {
        if self.inbound != CodexProxyInboundState::Open
            || self.outbound != CodexProxyOutboundState::AwaitOpenAck
        {
            return Err(malformed("HERMES_CODEX_PROXY_STATE_REJECTED"));
        }
        let frame = self.encode_outbound(1, &[])?;
        self.outbound = CodexProxyOutboundState::Open;
        Ok(frame)
    }

    pub(crate) fn encode_data(&mut self, payload: &[u8]) -> HermesAdapterResult<Vec<u8>> {
        if self.outbound != CodexProxyOutboundState::Open
            || payload.is_empty()
            || payload.len() > MAX_CODEX_PROXY_DATA_BYTES
        {
            return Err(malformed("HERMES_CODEX_PROXY_STATE_REJECTED"));
        }
        self.encode_outbound(2, payload)
    }

    pub(crate) fn encode_close(&mut self) -> HermesAdapterResult<Vec<u8>> {
        if self.outbound != CodexProxyOutboundState::Open {
            return Err(malformed("HERMES_CODEX_PROXY_STATE_REJECTED"));
        }
        let frame = self.encode_outbound(3, &[])?;
        self.outbound = CodexProxyOutboundState::Closed;
        Ok(frame)
    }

    pub(crate) fn accept(&mut self, frame: &[u8]) -> HermesAdapterResult<CodexProxyHostEvent> {
        let result = self.accept_inner(frame);
        if result.is_err() {
            self.record_failure(frame);
        }
        result
    }

    fn record_failure(&mut self, bytes: &[u8]) {
        self.failure_evidence = Some(CodexProxyFailureEvidence {
            byte_count: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            sha256: encode_sha256(&Sha256::digest(bytes)),
        });
    }

    fn accept_inner(&mut self, frame: &[u8]) -> HermesAdapterResult<CodexProxyHostEvent> {
        if Instant::now() >= self.deadline {
            return Err(error(
                HermesAdapterErrorKind::Timeout,
                "HERMES_CODEX_PROXY_DEADLINE_EXCEEDED",
            ));
        }
        let Some(encoded) = frame.strip_prefix(CODEX_PROXY_MAGIC) else {
            return Err(malformed("HERMES_CODEX_PROXY_MAGIC_REJECTED"));
        };
        let length_bytes: [u8; 4] = encoded
            .get(..4)
            .and_then(|value| value.try_into().ok())
            .ok_or_else(|| malformed("HERMES_CODEX_PROXY_SIZE_REJECTED"))?;
        let body_length = usize::try_from(u32::from_be_bytes(length_bytes))
            .map_err(|_| malformed("HERMES_CODEX_PROXY_SIZE_REJECTED"))?;
        let body = encoded
            .get(4..)
            .filter(|body| body.len() == body_length)
            .ok_or_else(|| malformed("HERMES_CODEX_PROXY_SIZE_REJECTED"))?;
        if !(CODEX_PROXY_HEADER_BYTES..=MAX_CODEX_PROXY_BODY_BYTES).contains(&body.len()) {
            return Err(malformed("HERMES_CODEX_PROXY_SIZE_REJECTED"));
        }
        let kind = body[0];
        let stream_id = u32::from_be_bytes(
            body[1..5]
                .try_into()
                .map_err(|_| malformed("HERMES_CODEX_PROXY_SIZE_REJECTED"))?,
        );
        let sequence = u32::from_be_bytes(
            body[5..9]
                .try_into()
                .map_err(|_| malformed("HERMES_CODEX_PROXY_SIZE_REJECTED"))?,
        );
        if stream_id != CODEX_PROXY_STREAM_ID {
            return Err(malformed("HERMES_CODEX_PROXY_STREAM_REJECTED"));
        }
        if !constant_time_equal(&body[9..41], &self.binding) {
            return Err(cross_binding("HERMES_CODEX_PROXY_BINDING_REJECTED"));
        }
        if sequence != self.expected_sequence {
            return Err(cross_binding("HERMES_CODEX_PROXY_SEQUENCE_REJECTED"));
        }
        let payload = &body[CODEX_PROXY_HEADER_BYTES..];
        let event = match kind {
            1 if payload.is_empty()
                && self.inbound == CodexProxyInboundState::AwaitOpen
                && self.outbound == CodexProxyOutboundState::AwaitOpenAck =>
            {
                self.inbound = CodexProxyInboundState::Open;
                CodexProxyHostEvent::Open
            }
            2 if !payload.is_empty()
                && payload.len() <= MAX_CODEX_PROXY_DATA_BYTES
                && self.inbound == CodexProxyInboundState::Open =>
            {
                CodexProxyHostEvent::Data(payload.to_vec())
            }
            3 if payload.is_empty() && self.inbound == CodexProxyInboundState::Open => {
                self.inbound = CodexProxyInboundState::Closed;
                CodexProxyHostEvent::Close
            }
            4 if payload.len() == 2 && self.inbound == CodexProxyInboundState::Open => {
                self.inbound = CodexProxyInboundState::Closed;
                CodexProxyHostEvent::Error(u16::from_be_bytes([payload[0], payload[1]]))
            }
            5 if payload.is_empty()
                && self.inbound_closed()
                && self.outbound_closed()
                && !self.terminal =>
            {
                self.terminal = true;
                CodexProxyHostEvent::Terminal
            }
            1..=5 => return Err(malformed("HERMES_CODEX_PROXY_STATE_REJECTED")),
            _ => return Err(malformed("HERMES_CODEX_PROXY_KIND_REJECTED")),
        };
        self.expected_sequence = self
            .expected_sequence
            .checked_add(1)
            .ok_or_else(|| malformed("HERMES_CODEX_PROXY_SEQUENCE_REJECTED"))?;
        Ok(event)
    }

    fn encode_outbound(&mut self, kind: u8, payload: &[u8]) -> HermesAdapterResult<Vec<u8>> {
        if Instant::now() >= self.deadline {
            return Err(error(
                HermesAdapterErrorKind::Timeout,
                "HERMES_CODEX_PROXY_DEADLINE_EXCEEDED",
            ));
        }
        let frame = encode_codex_proxy_frame(kind, self.outbound_sequence, self.binding, payload)?;
        self.outbound_sequence = self
            .outbound_sequence
            .checked_add(1)
            .ok_or_else(|| malformed("HERMES_CODEX_PROXY_SEQUENCE_REJECTED"))?;
        Ok(frame)
    }
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

#[cfg(test)]
pub(crate) fn encode_codex_proxy_test_frame(
    kind: u8,
    sequence: u32,
    binding: [u8; 32],
    payload: &[u8],
) -> Vec<u8> {
    encode_codex_proxy_frame(kind, sequence, binding, payload).unwrap_or_default()
}

fn encode_codex_proxy_frame(
    kind: u8,
    sequence: u32,
    binding: [u8; 32],
    payload: &[u8],
) -> HermesAdapterResult<Vec<u8>> {
    let body_length = CODEX_PROXY_HEADER_BYTES
        .checked_add(payload.len())
        .filter(|length| *length <= MAX_CODEX_PROXY_BODY_BYTES)
        .ok_or_else(|| malformed("HERMES_CODEX_PROXY_SIZE_REJECTED"))?;
    let mut frame = Vec::with_capacity(CODEX_PROXY_MAGIC.len() + 4 + body_length);
    frame.extend_from_slice(CODEX_PROXY_MAGIC);
    frame.extend_from_slice(
        &u32::try_from(body_length)
            .map_err(|_| malformed("HERMES_CODEX_PROXY_SIZE_REJECTED"))?
            .to_be_bytes(),
    );
    frame.push(kind);
    frame.extend_from_slice(&CODEX_PROXY_STREAM_ID.to_be_bytes());
    frame.extend_from_slice(&sequence.to_be_bytes());
    frame.extend_from_slice(&binding);
    frame.extend_from_slice(payload);
    Ok(frame)
}

enum OuterStreamEvent {
    Data(Vec<u8>),
    Eof,
    Failed,
}

enum ProviderStreamEvent {
    Data(Vec<u8>),
    Eof,
    Failed,
}

#[derive(Default)]
struct CodexProxyProviderOutputGate {
    pending: Vec<u8>,
}

enum CodexProxyProviderLineAction {
    ForwardOriginal,
    ForwardNormalized(Vec<u8>),
    Drop,
}

impl CodexProxyProviderOutputGate {
    fn ingest(&mut self, payload: &[u8]) -> HermesAdapterResult<Vec<u8>> {
        if payload.is_empty() || payload.len() > MAX_CODEX_PROXY_DATA_BYTES {
            return Err(malformed("HERMES_CODEX_PROXY_PROVIDER_JSONL_SIZE_REJECTED"));
        }
        let admitted_capacity = self
            .pending
            .len()
            .checked_add(payload.len())
            .filter(|length| *length <= MAX_CODEX_PROXY_JSONL_BATCH_BYTES)
            .ok_or_else(|| malformed("HERMES_CODEX_PROXY_PROVIDER_JSONL_SIZE_REJECTED"))?;
        let mut admitted = Vec::with_capacity(admitted_capacity);
        let mut cursor = 0;
        while cursor < payload.len() {
            let remaining = &payload[cursor..];
            let Some(newline_offset) = remaining.iter().position(|byte| *byte == b'\n') else {
                self.extend_pending(remaining)?;
                break;
            };
            let line_fragment = &remaining[..newline_offset];
            self.extend_pending(line_fragment)?;
            match self.validate_complete_line()? {
                CodexProxyProviderLineAction::ForwardOriginal => {
                    admitted.extend_from_slice(&self.pending);
                    admitted.push(b'\n');
                }
                CodexProxyProviderLineAction::ForwardNormalized(normalized) => {
                    admitted.extend_from_slice(&normalized);
                    admitted.push(b'\n');
                }
                CodexProxyProviderLineAction::Drop => {}
            }
            self.pending.clear();
            cursor = cursor
                .checked_add(newline_offset + 1)
                .ok_or_else(|| malformed("HERMES_CODEX_PROXY_PROVIDER_JSONL_SIZE_REJECTED"))?;
        }
        Ok(admitted)
    }

    fn extend_pending(&mut self, fragment: &[u8]) -> HermesAdapterResult<()> {
        let line_length = self
            .pending
            .len()
            .checked_add(fragment.len())
            .filter(|length| *length <= MAX_CODEX_PROXY_JSONL_LINE_BYTES)
            .ok_or_else(|| malformed("HERMES_CODEX_PROXY_PROVIDER_JSONL_SIZE_REJECTED"))?;
        self.pending.reserve(line_length - self.pending.len());
        self.pending.extend_from_slice(fragment);
        Ok(())
    }

    fn validate_complete_line(&self) -> HermesAdapterResult<CodexProxyProviderLineAction> {
        let line = self.pending.strip_suffix(b"\r").unwrap_or(&self.pending);
        let value: serde_json::Value = serde_json::from_slice(line)
            .map_err(|_| malformed("HERMES_CODEX_PROXY_PROVIDER_JSONL_REJECTED"))?;
        let object = value
            .as_object()
            .ok_or_else(|| malformed("HERMES_CODEX_PROXY_PROVIDER_JSONL_REJECTED"))?;
        let Some(method) = object.get("method").and_then(serde_json::Value::as_str) else {
            return Ok(CodexProxyProviderLineAction::ForwardOriginal);
        };
        if object.contains_key("id") {
            return Err(malformed("HERMES_CODEX_PROXY_PROVIDER_REQUEST_REJECTED"));
        }
        if is_codex_proxy_ignorable_provider_notice(method) {
            validate_codex_proxy_ignorable_provider_notice(method, object)?;
            return Ok(CodexProxyProviderLineAction::Drop);
        }
        if !is_codex_proxy_hermes_notification(method) {
            return Err(malformed("HERMES_CODEX_PROXY_PROVIDER_NOTICE_REJECTED"));
        }
        let keys = object.keys().map(String::as_str).collect::<HashSet<_>>();
        match keys {
            keys if keys == HashSet::from(["method", "params"]) => {
                Ok(CodexProxyProviderLineAction::ForwardOriginal)
            }
            keys if keys == HashSet::from(["emittedAtMs", "method", "params"]) => {
                if !object
                    .get("emittedAtMs")
                    .is_some_and(serde_json::Value::is_number)
                    || !object
                        .get("params")
                        .is_some_and(serde_json::Value::is_object)
                {
                    return Err(malformed("HERMES_CODEX_PROXY_PROVIDER_JSONL_REJECTED"));
                }
                let mut normalized = value;
                normalized
                    .as_object_mut()
                    .ok_or_else(|| malformed("HERMES_CODEX_PROXY_PROVIDER_JSONL_REJECTED"))?
                    .remove("emittedAtMs");
                serde_json::to_vec(&normalized)
                    .map(CodexProxyProviderLineAction::ForwardNormalized)
                    .map_err(|_| malformed("HERMES_CODEX_PROXY_PROVIDER_JSONL_REJECTED"))
            }
            _ => Err(malformed("HERMES_CODEX_PROXY_PROVIDER_JSONL_REJECTED")),
        }
    }

    fn finish_input(&self) -> HermesAdapterResult<()> {
        if self.pending.is_empty() {
            Ok(())
        } else {
            Err(malformed(
                "HERMES_CODEX_PROXY_PROVIDER_JSONL_PARTIAL_REJECTED",
            ))
        }
    }
}

fn is_codex_proxy_ignorable_provider_notice(method: &str) -> bool {
    matches!(method, "remoteControl/status/changed" | "deprecationNotice")
}

fn is_codex_proxy_hermes_notification(method: &str) -> bool {
    matches!(
        method,
        "thread/started"
            | "turn/started"
            | "account/rateLimits/updated"
            | "thread/status/changed"
            | "thread/tokenUsage/updated"
            | "item/agentMessage/delta"
            | "item/reasoning/textDelta"
            | "item/reasoning/summaryPartAdded"
            | "item/reasoning/summaryTextDelta"
            | "item/started"
            | "item/completed"
            | "turn/completed"
    )
}

fn validate_codex_proxy_ignorable_provider_notice(
    method: &str,
    object: &serde_json::Map<String, serde_json::Value>,
) -> HermesAdapterResult<()> {
    let keys = object.keys().map(String::as_str).collect::<HashSet<_>>();
    if keys != HashSet::from(["emittedAtMs", "method", "params"])
        || !object
            .get("emittedAtMs")
            .is_some_and(serde_json::Value::is_number)
    {
        return Err(malformed("HERMES_CODEX_PROXY_PROVIDER_NOTICE_REJECTED"));
    }
    let params = object
        .get("params")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| malformed("HERMES_CODEX_PROXY_PROVIDER_NOTICE_REJECTED"))?;
    let params_keys = params.keys().map(String::as_str).collect::<HashSet<_>>();
    match method {
        "remoteControl/status/changed" => {
            if params_keys
                != HashSet::from(["environmentId", "installationId", "serverName", "status"])
                || params.values().any(|value| !value.is_string())
            {
                return Err(malformed("HERMES_CODEX_PROXY_PROVIDER_NOTICE_REJECTED"));
            }
        }
        "deprecationNotice" => {
            if params_keys != HashSet::from(["details", "summary"])
                || params.values().any(|value| !value.is_string())
            {
                return Err(malformed("HERMES_CODEX_PROXY_PROVIDER_NOTICE_REJECTED"));
            }
        }
        _ => return Err(malformed("HERMES_CODEX_PROXY_PROVIDER_NOTICE_REJECTED")),
    }
    Ok(())
}

#[derive(Default)]
struct CodexProxyOneTurnGate {
    pending: Vec<u8>,
    turn_start_count: u8,
    output_schema: Option<serde_json::Value>,
}

struct CodexProxyJsonLine {
    method: Option<String>,
    params: Option<CodexProxyJsonParams>,
}

struct CodexProxyJsonParams {
    cwd: Option<String>,
}

impl<'de> Deserialize<'de> for CodexProxyJsonLine {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct JsonLineVisitor;

        impl<'de> Visitor<'de> for JsonLineVisitor {
            type Value = CodexProxyJsonLine;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("one complete JSON object")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut keys = HashSet::new();
                let mut method = None;
                let mut params = None;
                while let Some(key) = map.next_key::<String>()? {
                    let is_method = key == "method";
                    let is_params = key == "params";
                    if !keys.insert(key) {
                        return Err(serde::de::Error::custom("duplicate JSON object key"));
                    }
                    if is_method {
                        let value = map.next_value::<serde_json::Value>()?;
                        method = Some(
                            value
                                .as_str()
                                .ok_or_else(|| {
                                    serde::de::Error::custom("JSON-RPC method is not a string")
                                })?
                                .to_owned(),
                        );
                    } else if is_params {
                        params = Some(map.next_value::<CodexProxyJsonParams>()?);
                    } else {
                        map.next_value::<IgnoredAny>()?;
                    }
                }
                Ok(CodexProxyJsonLine { method, params })
            }
        }

        deserializer.deserialize_map(JsonLineVisitor)
    }
}

impl<'de> Deserialize<'de> for CodexProxyJsonParams {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct JsonParamsVisitor;

        impl<'de> Visitor<'de> for JsonParamsVisitor {
            type Value = CodexProxyJsonParams;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("one JSON-RPC params object")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut keys = HashSet::new();
                let mut cwd = None;
                while let Some(key) = map.next_key::<String>()? {
                    let is_cwd = key == "cwd";
                    if !keys.insert(key) {
                        return Err(serde::de::Error::custom("duplicate JSON params key"));
                    }
                    if is_cwd {
                        cwd = Some(map.next_value::<String>()?);
                    } else {
                        map.next_value::<IgnoredAny>()?;
                    }
                }
                Ok(CodexProxyJsonParams { cwd })
            }
        }

        deserializer.deserialize_map(JsonParamsVisitor)
    }
}

impl CodexProxyOneTurnGate {
    fn with_output_schema(output_schema: Option<serde_json::Value>) -> Self {
        Self {
            output_schema,
            ..Self::default()
        }
    }

    fn ingest(&mut self, payload: &[u8]) -> HermesAdapterResult<Vec<u8>> {
        if payload.is_empty() || payload.len() > MAX_CODEX_PROXY_DATA_BYTES {
            return Err(malformed("HERMES_CODEX_PROXY_JSONL_SIZE_REJECTED"));
        }
        let admitted_capacity = self
            .pending
            .len()
            .checked_add(payload.len())
            .filter(|length| *length <= MAX_CODEX_PROXY_JSONL_BATCH_BYTES)
            .ok_or_else(|| malformed("HERMES_CODEX_PROXY_JSONL_SIZE_REJECTED"))?;
        let mut admitted = Vec::with_capacity(admitted_capacity);
        let mut cursor = 0;
        while cursor < payload.len() {
            let remaining = &payload[cursor..];
            let Some(newline_offset) = remaining.iter().position(|byte| *byte == b'\n') else {
                self.extend_pending(remaining)?;
                break;
            };
            let line_fragment = &remaining[..newline_offset];
            self.extend_pending(line_fragment)?;
            if let Some(normalized) = self.validate_complete_line()? {
                admitted.extend_from_slice(&normalized);
            } else {
                admitted.extend_from_slice(&self.pending);
            }
            admitted.push(b'\n');
            self.pending.clear();
            cursor = cursor
                .checked_add(newline_offset + 1)
                .ok_or_else(|| malformed("HERMES_CODEX_PROXY_JSONL_SIZE_REJECTED"))?;
        }
        Ok(admitted)
    }

    fn extend_pending(&mut self, fragment: &[u8]) -> HermesAdapterResult<()> {
        let line_length = self
            .pending
            .len()
            .checked_add(fragment.len())
            .filter(|length| *length <= MAX_CODEX_PROXY_JSONL_LINE_BYTES)
            .ok_or_else(|| malformed("HERMES_CODEX_PROXY_JSONL_SIZE_REJECTED"))?;
        self.pending.reserve(line_length - self.pending.len());
        self.pending.extend_from_slice(fragment);
        Ok(())
    }

    fn validate_complete_line(&mut self) -> HermesAdapterResult<Option<Vec<u8>>> {
        let line = self.pending.strip_suffix(b"\r").unwrap_or(&self.pending);
        let value: CodexProxyJsonLine = serde_json::from_slice(line)
            .map_err(|_| malformed("HERMES_CODEX_PROXY_JSONL_REJECTED"))?;
        if value.method.as_deref() == Some("turn/start") {
            if self.turn_start_count != 0 {
                return Err(malformed("HERMES_CODEX_PROXY_TURN_REPLAY_REJECTED"));
            }
            self.turn_start_count = 1;
            let contained_cwd = match value
                .params
                .as_ref()
                .and_then(|params| params.cwd.as_deref())
            {
                Some("/work") => true,
                Some(_) => return Err(cross_binding("HERMES_CODEX_PROXY_CWD_REJECTED")),
                None => false,
            };
            if self.output_schema.is_none() && !contained_cwd {
                return Ok(None);
            };
            let mut normalized: serde_json::Value = serde_json::from_slice(line)
                .map_err(|_| malformed("HERMES_CODEX_PROXY_JSONL_REJECTED"))?;
            let params = normalized
                .get_mut("params")
                .and_then(serde_json::Value::as_object_mut)
                .ok_or_else(|| malformed("HERMES_CODEX_PROXY_JSONL_REJECTED"))?;
            if contained_cwd {
                params.remove("cwd");
            }
            if let Some(output_schema) = &self.output_schema {
                params.insert("outputSchema".to_owned(), output_schema.clone());
            }
            return serde_json::to_vec(&normalized)
                .map(Some)
                .map_err(|_| malformed("HERMES_CODEX_PROXY_JSONL_REJECTED"));
        }
        if value.method.as_deref() != Some("thread/start") {
            return Ok(None);
        }
        let Some(cwd) = value.params.and_then(|params| params.cwd) else {
            return Ok(None);
        };
        if cwd != "/work" {
            return Err(cross_binding("HERMES_CODEX_PROXY_CWD_REJECTED"));
        }
        let mut normalized: serde_json::Value = serde_json::from_slice(line)
            .map_err(|_| malformed("HERMES_CODEX_PROXY_JSONL_REJECTED"))?;
        normalized
            .get_mut("params")
            .and_then(serde_json::Value::as_object_mut)
            .ok_or_else(|| malformed("HERMES_CODEX_PROXY_JSONL_REJECTED"))?
            .remove("cwd");
        serde_json::to_vec(&normalized)
            .map(Some)
            .map_err(|_| malformed("HERMES_CODEX_PROXY_JSONL_REJECTED"))
    }

    fn finish_input(&self) -> HermesAdapterResult<()> {
        if self.pending.is_empty() {
            Ok(())
        } else {
            Err(malformed("HERMES_CODEX_PROXY_JSONL_PARTIAL_REJECTED"))
        }
    }

    fn ensure_single_turn(&self) -> HermesAdapterResult<()> {
        if self.turn_start_count == 1 {
            Ok(())
        } else {
            Err(malformed("HERMES_CODEX_PROXY_TURN_COUNT_REJECTED"))
        }
    }

    const fn turn_start_count(&self) -> u8 {
        self.turn_start_count
    }

    fn pending(&self) -> &[u8] {
        &self.pending
    }
}

fn codex_reflection_output_schema(job: &HermesReflectionJob) -> serde_json::Value {
    let invocation = job.request().invocation();
    let evidence_digests = job
        .evidence()
        .iter()
        .map(|evidence| evidence.digest().as_str())
        .collect::<Vec<_>>();
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["schema_version", "binding", "summary", "findings", "next_actions"],
        "properties": {
            "schema_version": {"type": "string", "enum": [HERMES_SCHEMA_VERSION]},
            "binding": {
                "type": "object",
                "additionalProperties": false,
                "required": [
                    "request_id", "task_id", "attempt_id", "project_snapshot_id",
                    "subject_digest", "session_id", "input_digest", "model"
                ],
                "properties": {
                    "request_id": {"type": "string", "enum": [invocation.request_id().as_str()]},
                    "task_id": {"type": "string", "enum": [invocation.task_id().as_str()]},
                    "attempt_id": {"type": "string", "enum": [invocation.attempt_id().as_str()]},
                    "project_snapshot_id": {"type": "string", "enum": [invocation.project_snapshot_id().as_str()]},
                    "subject_digest": {"type": "string", "enum": [invocation.subject_digest().as_str()]},
                    "session_id": {"type": "string", "enum": [job.session_id()]},
                    "input_digest": {"type": "string", "enum": [job.input_digest().as_str()]},
                    "model": {"type": "string", "enum": [job.model()]}
                }
            },
            "summary": {"type": "string"},
            "findings": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["classification", "statement", "evidence_digests"],
                    "properties": {
                        "classification": {"type": "string", "enum": ["inference"]},
                        "statement": {"type": "string"},
                        "evidence_digests": {
                            "type": "array",
                            "items": {"type": "string", "enum": evidence_digests}
                        }
                    }
                }
            },
            "next_actions": {
                "type": "array",
                "items": {"type": "string"}
            }
        }
    })
}

#[derive(Default)]
struct CodexProxyHostStatus {
    failure: Option<HermesAdapterError>,
    failure_evidence: Option<CodexProxyFailureEvidence>,
    adapter_success_accepted: bool,
    authenticated_open: bool,
    clean_terminal: bool,
    turn_start_count: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CodexProxyHostCommand {
    AdapterSucceeded,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CodexProxyProviderInputState {
    Open,
    ClosedByPeer,
    ClosedAfterAdapterSuccess,
}

struct ProductionCodexProxyHost {
    status: Arc<Mutex<CodexProxyHostStatus>>,
    stop: Arc<AtomicBool>,
    adapter_success_requested: AtomicBool,
    commands: mpsc::SyncSender<CodexProxyHostCommand>,
    control: Arc<dyn ProductionCodexProxyControl>,
    worker: Option<thread::JoinHandle<()>>,
}

impl ProductionCodexProxyHost {
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    fn start(
        provider: Box<dyn ProductionCodexProxyProvider>,
        nonce: &str,
        broker_receipt: &ContentDigest,
        absolute_deadline: Instant,
        outer_input: std::fs::File,
        outer_stream: Receiver<OuterStreamEvent>,
        initial_bytes: Vec<u8>,
        owner: Arc<ContainmentOwnerState>,
    ) -> HermesAdapterResult<Self> {
        Self::start_with_output_schema(
            provider,
            nonce,
            broker_receipt,
            absolute_deadline,
            outer_input,
            outer_stream,
            initial_bytes,
            None,
            owner,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn start_with_output_schema(
        provider: Box<dyn ProductionCodexProxyProvider>,
        nonce: &str,
        broker_receipt: &ContentDigest,
        absolute_deadline: Instant,
        outer_input: std::fs::File,
        outer_stream: Receiver<OuterStreamEvent>,
        initial_bytes: Vec<u8>,
        output_schema: Option<serde_json::Value>,
        owner: Arc<ContainmentOwnerState>,
    ) -> HermesAdapterResult<Self> {
        let mut session = CodexProxyHostSession::new(nonce, broker_receipt, absolute_deadline)?;
        let control = provider.control();
        let status = Arc::new(Mutex::new(CodexProxyHostStatus::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (commands, worker_commands) = mpsc::sync_channel(1);
        let worker_status = Arc::clone(&status);
        let worker_stop = Arc::clone(&stop);
        let worker_control = Arc::clone(&control);
        let worker = thread::Builder::new()
            .name("lattice-hermes-codex-proxy".to_owned())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run_codex_proxy_host(
                        provider,
                        absolute_deadline,
                        outer_input,
                        &outer_stream,
                        initial_bytes,
                        &worker_stop,
                        &mut session,
                        &worker_control,
                        &worker_status,
                        &worker_commands,
                        output_schema,
                    )
                }))
                .unwrap_or_else(|_| {
                    Err(error(
                        HermesAdapterErrorKind::Ambiguous,
                        "HERMES_CODEX_PROXY_HOST_PANICKED",
                    ))
                });
                if let Err(failure) = result {
                    owner.invalidate();
                    if let Err(teardown) = worker_control.terminate() {
                        eprintln!(
                            "{}",
                            json!({
                                "component": "Hermes",
                                "error_code": teardown.code(),
                                "event": "teardown_rejected",
                                "owner_invalidated": true,
                                "target": "codex_proxy_worker"
                            })
                        );
                    }
                    if let Ok(mut observed) = worker_status.lock() {
                        observed.failure = Some(failure);
                        observed.failure_evidence = session.failure_evidence().cloned();
                    }
                }
            })
            .map_err(|_| {
                error(
                    HermesAdapterErrorKind::Spawn,
                    "HERMES_CODEX_PROXY_HOST_THREAD_FAILED",
                )
            })?;
        Ok(Self {
            status,
            stop,
            adapter_success_requested: AtomicBool::new(false),
            commands,
            control,
            worker: Some(worker),
        })
    }

    fn ensure_live(&self) -> HermesAdapterResult<()> {
        let observed = self.status.lock().map_err(|_| {
            error(
                HermesAdapterErrorKind::Ambiguous,
                "HERMES_CODEX_PROXY_HOST_STATE_UNKNOWN",
            )
        })?;
        if let Some(failure) = observed.failure.clone() {
            return Err(failure);
        }
        if self
            .worker
            .as_ref()
            .is_some_and(thread::JoinHandle::is_finished)
        {
            return Err(error(
                HermesAdapterErrorKind::Ambiguous,
                "HERMES_CODEX_PROXY_HOST_EXITED",
            ));
        }
        Ok(())
    }

    fn failure_evidence(&self) -> Option<CodexProxyFailureEvidence> {
        self.status
            .lock()
            .ok()
            .and_then(|observed| observed.failure_evidence.clone())
    }

    #[cfg(test)]
    fn wait_for_clean_terminal(&self, deadline: Instant) -> HermesAdapterResult<()> {
        self.wait_for_terminal(deadline, false)
    }

    fn complete_after_adapter_success(&self, deadline: Instant) -> HermesAdapterResult<()> {
        if self
            .adapter_success_requested
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(malformed(
                "HERMES_CODEX_PROXY_ADAPTER_SUCCESS_REPLAY_REJECTED",
            ));
        }
        if self
            .commands
            .try_send(CodexProxyHostCommand::AdapterSucceeded)
            .is_err()
        {
            return Err(error(
                HermesAdapterErrorKind::Ambiguous,
                "HERMES_CODEX_PROXY_HOST_COMMAND_FAILED",
            ));
        }
        self.wait_for_terminal(deadline, true)
    }

    fn wait_for_terminal(
        &self,
        deadline: Instant,
        require_adapter_success: bool,
    ) -> HermesAdapterResult<()> {
        loop {
            {
                let observed = self.status.lock().map_err(|_| {
                    error(
                        HermesAdapterErrorKind::Ambiguous,
                        "HERMES_CODEX_PROXY_HOST_STATE_UNKNOWN",
                    )
                })?;
                if let Some(failure) = observed.failure.clone() {
                    return Err(failure);
                }
                if observed.authenticated_open
                    && observed.clean_terminal
                    && observed.turn_start_count == 1
                    && (!require_adapter_success || observed.adapter_success_accepted)
                {
                    return Ok(());
                }
            }
            if self
                .worker
                .as_ref()
                .is_some_and(thread::JoinHandle::is_finished)
            {
                return Err(error(
                    HermesAdapterErrorKind::Ambiguous,
                    "HERMES_CODEX_PROXY_HOST_EXITED",
                ));
            }
            if Instant::now() >= deadline {
                return Err(error(
                    HermesAdapterErrorKind::Timeout,
                    "HERMES_CODEX_PROXY_COMPLETION_TIMEOUT",
                ));
            }
            thread::sleep(Duration::from_millis(1));
        }
    }

    fn stop_and_reap(&mut self) -> HermesAdapterResult<()> {
        self.stop.store(true, Ordering::Release);
        let control_result = self.control.terminate();
        let join_result = self.join_worker_bounded();
        control_result.and(join_result)
    }

    fn terminate(&mut self) -> HermesAdapterResult<()> {
        self.stop_and_reap()?;
        let observed = self.status.lock().map_err(|_| {
            error(
                HermesAdapterErrorKind::Ambiguous,
                "HERMES_CODEX_PROXY_HOST_STATE_UNKNOWN",
            )
        })?;
        observed.failure.clone().map_or(Ok(()), Err)
    }

    fn join_worker_bounded(&mut self) -> HermesAdapterResult<()> {
        let Some(worker) = self.worker.take() else {
            return Ok(());
        };
        let deadline = Instant::now()
            .checked_add(CODEX_PROXY_TEARDOWN_TIMEOUT)
            .ok_or_else(|| {
                error(
                    HermesAdapterErrorKind::Ambiguous,
                    "HERMES_CODEX_PROXY_HOST_TEARDOWN_AMBIGUOUS",
                )
            })?;
        while !worker.is_finished() {
            if Instant::now() >= deadline {
                drop(worker);
                return Err(error(
                    HermesAdapterErrorKind::Ambiguous,
                    "HERMES_CODEX_PROXY_HOST_TEARDOWN_AMBIGUOUS",
                ));
            }
            thread::sleep(Duration::from_millis(1));
        }
        worker.join().map_err(|_| {
            error(
                HermesAdapterErrorKind::Ambiguous,
                "HERMES_CODEX_PROXY_HOST_JOIN_FAILED",
            )
        })
    }
}

impl Drop for ProductionCodexProxyHost {
    fn drop(&mut self) {
        let _ = self.terminate();
    }
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn run_codex_proxy_host(
    provider: Box<dyn ProductionCodexProxyProvider>,
    absolute_deadline: Instant,
    mut outer_input: std::fs::File,
    outer_stream: &Receiver<OuterStreamEvent>,
    initial_bytes: Vec<u8>,
    stop: &AtomicBool,
    session: &mut CodexProxyHostSession,
    control: &Arc<dyn ProductionCodexProxyControl>,
    status: &Arc<Mutex<CodexProxyHostStatus>>,
    commands: &Receiver<CodexProxyHostCommand>,
    output_schema: Option<serde_json::Value>,
) -> HermesAdapterResult<()> {
    let mut provider = Some(provider);
    let mut duplex: Option<ProductionCodexProxyDuplex> = None;
    let mut provider_stream: Option<Receiver<ProviderStreamEvent>> = None;
    let mut buffer = initial_bytes;
    let mut one_turn_gate = CodexProxyOneTurnGate::with_output_schema(output_schema);
    let mut provider_output_gate = CodexProxyProviderOutputGate::default();
    let mut provider_input_state = CodexProxyProviderInputState::Open;

    loop {
        if stop.load(Ordering::Acquire) {
            return Ok(());
        }
        if Instant::now() >= absolute_deadline {
            return Err(error(
                HermesAdapterErrorKind::Timeout,
                "HERMES_CODEX_PROXY_DEADLINE_EXCEEDED",
            ));
        }

        while let Some(frame) = take_codex_proxy_frame(&mut buffer, session)? {
            match session.accept(&frame)? {
                CodexProxyHostEvent::Open => {
                    let sealed = provider
                        .take()
                        .ok_or_else(|| malformed("HERMES_CODEX_PROXY_PROVIDER_REPLAY_REJECTED"))?;
                    let mut opened = sealed.open(absolute_deadline)?;
                    control.ensure_running()?;
                    let reader = opened.take_reader()?;
                    provider_stream = Some(start_provider_reader(reader)?);
                    write_proxy_frame(&mut outer_input, &session.encode_open_ack()?)?;
                    status
                        .lock()
                        .map_err(|_| {
                            error(
                                HermesAdapterErrorKind::Ambiguous,
                                "HERMES_CODEX_PROXY_HOST_STATE_UNKNOWN",
                            )
                        })?
                        .authenticated_open = true;
                    duplex = Some(opened);
                }
                CodexProxyHostEvent::Data(payload) => {
                    let opened = duplex
                        .as_mut()
                        .ok_or_else(|| malformed("HERMES_CODEX_PROXY_PROVIDER_STATE_REJECTED"))?;
                    let admitted = one_turn_gate.ingest(&payload).inspect_err(|_| {
                        session.record_failure(&payload);
                    })?;
                    if provider_input_state != CodexProxyProviderInputState::Open {
                        session.record_failure(&payload);
                        return Err(malformed("HERMES_CODEX_PROXY_STATE_REJECTED"));
                    }
                    control.ensure_running()?;
                    if !admitted.is_empty() {
                        opened.write_all(&admitted)?;
                    }
                }
                CodexProxyHostEvent::Close => {
                    let opened = duplex
                        .as_mut()
                        .ok_or_else(|| malformed("HERMES_CODEX_PROXY_PROVIDER_STATE_REJECTED"))?;
                    one_turn_gate.finish_input().inspect_err(|_| {
                        session.record_failure(one_turn_gate.pending());
                    })?;
                    if provider_input_state == CodexProxyProviderInputState::Open {
                        opened.close_input();
                        provider_input_state = CodexProxyProviderInputState::ClosedByPeer;
                    }
                }
                CodexProxyHostEvent::Error(_) => {
                    return Err(error(
                        HermesAdapterErrorKind::Failed,
                        "HERMES_CODEX_PROXY_CHILD_ERROR",
                    ));
                }
                CodexProxyHostEvent::Terminal => {
                    one_turn_gate.ensure_single_turn()?;
                    let mut observed = status.lock().map_err(|_| {
                        error(
                            HermesAdapterErrorKind::Ambiguous,
                            "HERMES_CODEX_PROXY_HOST_STATE_UNKNOWN",
                        )
                    })?;
                    observed.turn_start_count = one_turn_gate.turn_start_count();
                    observed.clean_terminal = true;
                }
            }
        }

        let provider_event = provider_stream
            .as_ref()
            .and_then(|events| match events.try_recv() {
                Ok(event) => Some(Ok(event)),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Err(error(
                    HermesAdapterErrorKind::Ambiguous,
                    "HERMES_CODEX_PROXY_PROVIDER_STREAM_LOST",
                ))),
            });
        if let Some(provider_event) = provider_event {
            match provider_event? {
                ProviderStreamEvent::Data(payload) => {
                    let admitted = provider_output_gate.ingest(&payload).inspect_err(|_| {
                        session.record_failure(&payload);
                    })?;
                    if !admitted.is_empty() {
                        write_proxy_frame(&mut outer_input, &session.encode_data(&admitted)?)?;
                    }
                }
                ProviderStreamEvent::Eof => {
                    provider_output_gate.finish_input().inspect_err(|_| {
                        session.record_failure(provider_output_gate.pending.as_slice());
                    })?;
                    if provider_input_state == CodexProxyProviderInputState::Open {
                        control.ensure_running()?;
                        return Err(error(
                            HermesAdapterErrorKind::Failed,
                            "HERMES_CODEX_PROXY_PROVIDER_EOF_BEFORE_CLOSE",
                        ));
                    }
                    write_proxy_frame(&mut outer_input, &session.encode_close()?)?;
                    provider_stream = None;
                    debug_assert!(session.outbound_closed());
                }
                ProviderStreamEvent::Failed => {
                    return Err(error(
                        HermesAdapterErrorKind::Transport,
                        "HERMES_CODEX_PROXY_PROVIDER_READ_FAILED",
                    ));
                }
            }
        }

        match commands.try_recv() {
            Ok(CodexProxyHostCommand::AdapterSucceeded) => {
                let opened = duplex
                    .as_mut()
                    .ok_or_else(|| malformed("HERMES_CODEX_PROXY_PROVIDER_STATE_REJECTED"))?;
                one_turn_gate.finish_input().inspect_err(|_| {
                    session.record_failure(one_turn_gate.pending());
                })?;
                one_turn_gate.ensure_single_turn()?;
                if provider_input_state == CodexProxyProviderInputState::Open {
                    opened.close_input();
                    provider_input_state = CodexProxyProviderInputState::ClosedAfterAdapterSuccess;
                }
                let mut observed = status.lock().map_err(|_| {
                    error(
                        HermesAdapterErrorKind::Ambiguous,
                        "HERMES_CODEX_PROXY_HOST_STATE_UNKNOWN",
                    )
                })?;
                observed.adapter_success_accepted = true;
                observed.turn_start_count = one_turn_gate.turn_start_count();
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                return Err(error(
                    HermesAdapterErrorKind::Ambiguous,
                    "HERMES_CODEX_PROXY_HOST_COMMAND_LOST",
                ));
            }
        }

        match outer_stream.recv_timeout(Duration::from_millis(2)) {
            Ok(OuterStreamEvent::Data(payload)) => {
                buffer.extend_from_slice(&payload);
                if buffer.len() > MAX_CODEX_PROXY_BUFFER_BYTES {
                    session.record_failure(&buffer);
                    return Err(malformed("HERMES_CODEX_PROXY_SIZE_REJECTED"));
                }
            }
            Ok(OuterStreamEvent::Eof) => {
                one_turn_gate.finish_input().inspect_err(|_| {
                    session.record_failure(one_turn_gate.pending());
                })?;
                return Err(error(
                    HermesAdapterErrorKind::Failed,
                    "HERMES_CODEX_PROXY_OUTER_EOF",
                ));
            }
            Ok(OuterStreamEvent::Failed) => {
                return Err(error(
                    HermesAdapterErrorKind::Transport,
                    "HERMES_CODEX_PROXY_OUTER_READ_FAILED",
                ));
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Err(error(
                    HermesAdapterErrorKind::Ambiguous,
                    "HERMES_CODEX_PROXY_OUTER_STREAM_LOST",
                ));
            }
        }
    }
}

fn take_codex_proxy_frame(
    buffer: &mut Vec<u8>,
    session: &mut CodexProxyHostSession,
) -> HermesAdapterResult<Option<Vec<u8>>> {
    let comparable = buffer.len().min(CODEX_PROXY_MAGIC.len());
    if buffer[..comparable] != CODEX_PROXY_MAGIC[..comparable] {
        session.record_failure(buffer);
        return Err(malformed("HERMES_CODEX_PROXY_MAGIC_REJECTED"));
    }
    let prefix_bytes = CODEX_PROXY_MAGIC.len() + 4;
    if buffer.len() < prefix_bytes {
        return Ok(None);
    }
    let body_length = usize::try_from(u32::from_be_bytes(
        buffer[CODEX_PROXY_MAGIC.len()..prefix_bytes]
            .try_into()
            .expect("fixed four-byte proxy length"),
    ))
    .map_err(|_| malformed("HERMES_CODEX_PROXY_SIZE_REJECTED"))?;
    if !(CODEX_PROXY_HEADER_BYTES..=MAX_CODEX_PROXY_BODY_BYTES).contains(&body_length) {
        session.record_failure(&buffer[..prefix_bytes]);
        return Err(malformed("HERMES_CODEX_PROXY_SIZE_REJECTED"));
    }
    let total = prefix_bytes
        .checked_add(body_length)
        .ok_or_else(|| malformed("HERMES_CODEX_PROXY_SIZE_REJECTED"))?;
    if buffer.len() < total {
        return Ok(None);
    }
    Ok(Some(buffer.drain(..total).collect()))
}

fn write_proxy_frame(writer: &mut std::fs::File, frame: &[u8]) -> HermesAdapterResult<()> {
    writer
        .write_all(frame)
        .and_then(|()| writer.flush())
        .map_err(|_| {
            error(
                HermesAdapterErrorKind::Transport,
                "HERMES_CODEX_PROXY_OUTER_WRITE_FAILED",
            )
        })
}

fn start_outer_reader(mut reader: std::fs::File) -> Receiver<OuterStreamEvent> {
    let (sender, receiver) = mpsc::sync_channel(8);
    thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => {
                    let _ = sender.send(OuterStreamEvent::Eof);
                    return;
                }
                Ok(read) => {
                    if sender
                        .send(OuterStreamEvent::Data(buffer[..read].to_vec()))
                        .is_err()
                    {
                        return;
                    }
                }
                Err(_) => {
                    let _ = sender.send(OuterStreamEvent::Failed);
                    return;
                }
            }
        }
    });
    receiver
}

fn start_provider_reader(
    mut reader: Box<dyn Read + Send>,
) -> HermesAdapterResult<Receiver<ProviderStreamEvent>> {
    let (sender, receiver) = mpsc::sync_channel(8);
    thread::Builder::new()
        .name("lattice-hermes-codex-output".to_owned())
        .spawn(move || {
            let mut buffer = vec![0_u8; MAX_CODEX_PROXY_DATA_BYTES].into_boxed_slice();
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => {
                        let _ = sender.send(ProviderStreamEvent::Eof);
                        return;
                    }
                    Ok(read) => {
                        if sender
                            .send(ProviderStreamEvent::Data(buffer[..read].to_vec()))
                            .is_err()
                        {
                            return;
                        }
                    }
                    Err(_) => {
                        let _ = sender.send(ProviderStreamEvent::Failed);
                        return;
                    }
                }
            }
        })
        .map_err(|_| {
            error(
                HermesAdapterErrorKind::Spawn,
                "HERMES_CODEX_PROXY_READER_THREAD_FAILED",
            )
        })?;
    Ok(receiver)
}

#[cfg(test)]
#[derive(Default)]
struct ScriptedCodexProxyProvider {
    control: Arc<ScriptedCodexProxyControl>,
}

#[cfg(test)]
impl ProductionCodexProxyProvider for ScriptedCodexProxyProvider {
    fn control(&self) -> Arc<dyn ProductionCodexProxyControl> {
        self.control.clone()
    }

    fn open(
        self: Box<Self>,
        absolute_deadline: Instant,
    ) -> HermesAdapterResult<ProductionCodexProxyDuplex> {
        if absolute_deadline <= Instant::now() {
            return Err(error(
                HermesAdapterErrorKind::Timeout,
                "HERMES_CODEX_PROXY_DEADLINE_EXCEEDED",
            ));
        }
        let response = concat!(
            "{\"id\":0,\"result\":{\"ok\":true}}\n",
            "{\"id\":1,\"result\":{\"ok\":true}}\n",
            "{\"id\":2,\"result\":{\"ok\":true}}\n"
        )
        .as_bytes()
        .to_vec();
        Ok(ProductionCodexProxyDuplex::new(
            Box::new(std::io::Cursor::new(response)),
            Box::new(Vec::<u8>::new()),
        ))
    }
}

#[cfg(test)]
#[derive(Default)]
struct ScriptedCodexProxyControl;

#[cfg(test)]
impl ProductionCodexProxyControl for ScriptedCodexProxyControl {
    fn ensure_running(&self) -> HermesAdapterResult<()> {
        Ok(())
    }

    fn terminate(&self) -> HermesAdapterResult<()> {
        Ok(())
    }
}

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
    runtime_manifest_sha256: String,
    broker_receipt_sha256: String,
    api_key: String,
    model: String,
    startup_timeout: Duration,
    operation_timeout: Duration,
    poll_interval: Duration,
    mode: RunnerMode,
    codex_provider: Option<Box<dyn ProductionCodexProxyProvider>>,
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
        broker: CodexReflectionBrokerConfig,
        broker_receipt: &CodexBrokerPreflightReceipt,
        api_key: impl Into<String>,
        model: impl Into<String>,
        startup_timeout: Duration,
        operation_timeout: Duration,
        poll_interval: Duration,
    ) -> HermesAdapterResult<Self> {
        validate_official_runtime_identity(&containment, runtime_manifest)?;
        broker_receipt.validate_for_containment()?;
        let model = model.into();
        let codex_provider = broker
            .into_production_proxy_provider_from_preflight(broker_receipt, "gpt-5.3-codex-spark")?;
        Self::validated(
            containment,
            runtime_manifest,
            broker_receipt.receipt_digest().as_str().to_owned(),
            api_key.into(),
            model,
            startup_timeout,
            operation_timeout,
            poll_interval,
            RunnerMode::Official,
            Some(codex_provider),
        )
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn scripted_fixture(
        containment: HermesWslContainmentConfig,
        runtime_manifest: &HermesOfflineRuntimeManifest,
        broker_receipt_digest: &ContentDigest,
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
            api_key.into(),
            model.into(),
            startup_timeout,
            operation_timeout,
            poll_interval,
            RunnerMode::ScriptedFixture(reflection),
            Some(Box::new(ScriptedCodexProxyProvider::default())),
        )
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn official_with_broker_digest(
        containment: HermesWslContainmentConfig,
        runtime_manifest: &HermesOfflineRuntimeManifest,
        broker_receipt_digest: &ContentDigest,
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
            api_key.into(),
            model.into(),
            startup_timeout,
            operation_timeout,
            poll_interval,
            RunnerMode::Official,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn validated(
        containment: HermesWslContainmentConfig,
        runtime_manifest: &HermesOfflineRuntimeManifest,
        broker_receipt_sha256: String,
        api_key: String,
        model: String,
        startup_timeout: Duration,
        operation_timeout: Duration,
        poll_interval: Duration,
        mode: RunnerMode,
        codex_provider: Option<Box<dyn ProductionCodexProxyProvider>>,
    ) -> HermesAdapterResult<Self> {
        if matches!(mode, RunnerMode::Official)
            && (model != "hermes-agent"
                || api_key.len() < 16
                || !api_key.is_ascii()
                || (codex_provider.is_none() && !cfg!(test)))
        {
            return Err(error(
                HermesAdapterErrorKind::Configuration,
                "HERMES_PRODUCTION_OFFICIAL_CONFIG_REJECTED",
            ));
        }
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
            runtime_manifest_sha256,
            broker_receipt_sha256,
            api_key,
            model,
            startup_timeout,
            operation_timeout,
            poll_interval,
            mode,
            codex_provider,
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
        let mut process = match crate::windows_job::spawn_with_parent_stdio(&plan) {
            Ok(process) => process,
            Err(failure) => {
                remove_ingress(&secret_path);
                remove_ingress(&runner_path);
                return Err(failure);
            }
        };
        let outer_input = match process.take_stdin_writer() {
            Ok(writer) => writer,
            Err(failure) => {
                let _ = process.terminate();
                remove_ingress(&secret_path);
                remove_ingress(&runner_path);
                return Err(failure);
            }
        };
        let outer_output = match process.take_stdout_reader() {
            Ok(reader) => reader,
            Err(failure) => {
                let _ = process.terminate();
                remove_ingress(&secret_path);
                remove_ingress(&runner_path);
                return Err(failure);
            }
        };
        let outer_stream = start_outer_reader(outer_output);
        let startup_deadline = Instant::now()
            .checked_add(self.startup_timeout)
            .map_or(absolute_deadline, |candidate| {
                candidate.min(absolute_deadline)
            });
        let mut startup = match wait_for_startup(&mut process, &outer_stream, startup_deadline) {
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
        let outer_initial_bytes = std::mem::take(&mut startup.trailing);
        Ok(ProductionHermesRunner {
            endpoint: attestation.endpoint,
            api_key: self.api_key,
            model: self.model,
            nonce,
            broker_receipt_sha256: self.broker_receipt_sha256,
            codex_provider: self.codex_provider,
            outer_input: Some(outer_input),
            outer_stream: Some(outer_stream),
            outer_initial_bytes,
            receipt,
            process,
            owner,
            absolute_deadline,
            deadline_window: Duration::from_millis(deadline_millis),
            operation_timeout: self.operation_timeout,
            poll_interval: self.poll_interval,
            windows_launcher_pid: startup.windows_launcher_pid,
            outer_pid: startup.wire.outer_pid,
            bwrap_pid: startup.wire.bwrap_pid,
        })
    }
}

fn validate_official_runtime_identity(
    containment: &HermesWslContainmentConfig,
    runtime_manifest: &HermesOfflineRuntimeManifest,
) -> HermesAdapterResult<()> {
    let manifest_bytes = serde_json::to_vec(runtime_manifest)
        .map_err(|_| malformed("HERMES_RUNTIME_MANIFEST_CANONICALIZATION_FAILED"))?;
    if containment.runtime_guest_root() != OFFICIAL_RUNTIME_GUEST_ROOT
        || encode_sha256(&Sha256::digest(&manifest_bytes)) != OFFICIAL_RUNTIME_MANIFEST_SHA256
        || runtime_manifest.payload_file_count() != OFFICIAL_RUNTIME_FILE_COUNT
        || runtime_manifest.payload_byte_count() != OFFICIAL_RUNTIME_BYTE_COUNT
        || runtime_manifest.payload_manifest_sha256() != OFFICIAL_RUNTIME_TREE_SHA256
    {
        return Err(error(
            HermesAdapterErrorKind::Identity,
            "HERMES_PRODUCTION_RUNTIME_IDENTITY_REJECTED",
        ));
    }
    Ok(())
}

/// Live contained Hermes process that exists before any Codex effect and can
/// be bound exactly once to the resulting immutable reflection job.
pub struct ProductionHermesRunner {
    endpoint: SocketAddr,
    api_key: String,
    model: String,
    nonce: String,
    broker_receipt_sha256: String,
    codex_provider: Option<Box<dyn ProductionCodexProxyProvider>>,
    outer_input: Option<std::fs::File>,
    outer_stream: Option<Receiver<OuterStreamEvent>>,
    outer_initial_bytes: Vec<u8>,
    receipt: HermesContainmentReceipt,
    process: crate::windows_job::WindowsJobChild,
    owner: Arc<ContainmentOwnerState>,
    absolute_deadline: Instant,
    deadline_window: Duration,
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
        if job.model() != self.model {
            return Err(error(
                HermesAdapterErrorKind::CrossBinding,
                "HERMES_PRODUCTION_JOB_BINDING_REJECTED",
            ));
        }
        self.process.ensure_running()?;
        self.absolute_deadline = Instant::now()
            .checked_add(self.deadline_window)
            .ok_or_else(|| {
                error(
                    HermesAdapterErrorKind::Timeout,
                    "HERMES_PRODUCTION_DEADLINE_EXCEEDED",
                )
            })?;
        self.receipt.verify_binding(self.endpoint, &self.api_key)?;
        let broker_receipt = ContentDigest::from_sha256(self.broker_receipt_sha256.clone())
            .map_err(|_| malformed("HERMES_CODEX_PROXY_BINDING_INPUT_REJECTED"))?;
        let provider = self.codex_provider.take().ok_or_else(|| {
            error(
                HermesAdapterErrorKind::CapabilityMismatch,
                "HERMES_CODEX_PROXY_PROVIDER_NOT_STAGED",
            )
        })?;
        let outer_input = self.outer_input.take().ok_or_else(|| {
            error(
                HermesAdapterErrorKind::Ambiguous,
                "HERMES_CODEX_PROXY_OUTER_INPUT_UNAVAILABLE",
            )
        })?;
        let outer_stream = self.outer_stream.take().ok_or_else(|| {
            error(
                HermesAdapterErrorKind::Ambiguous,
                "HERMES_CODEX_PROXY_OUTER_STREAM_UNAVAILABLE",
            )
        })?;
        let codex_proxy = ProductionCodexProxyHost::start_with_output_schema(
            provider,
            &self.nonce,
            &broker_receipt,
            self.absolute_deadline,
            outer_input,
            outer_stream,
            std::mem::take(&mut self.outer_initial_bytes),
            Some(codex_reflection_output_schema(&job)),
            Arc::clone(&self.owner),
        )?;
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
            codex_proxy,
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
    codex_proxy: ProductionCodexProxyHost,
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
        if result.is_ok() {
            self.complete_proxy_after_adapter_success()?;
        }
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
        if result.is_ok() {
            self.complete_proxy_after_adapter_success()?;
        }
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
        let process_result = self.process.terminate();
        let proxy_result = self.codex_proxy.terminate();
        match (process_result, proxy_result) {
            (Err(process_failure), _) => Err(process_failure),
            (Ok(()), result) => result,
        }
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
            return Err(self.fail_closed(&failure));
        }
        if let Err(failure) = self.codex_proxy.ensure_live() {
            return Err(self.fail_closed(&failure));
        }
        self.adapter
            .require_containment_receipt()
            .map(|_| ())
            .map_err(|failure| map_port_error(&failure))
    }

    fn complete_proxy_after_adapter_success(&mut self) -> PortResult<()> {
        match self
            .codex_proxy
            .complete_after_adapter_success(self.absolute_deadline)
        {
            Ok(()) => Ok(()),
            Err(failure) => Err(self.fail_closed(&failure)),
        }
    }

    fn fail_closed(&mut self, failure: &HermesAdapterError) -> PortError {
        self.owner.invalidate();
        let process_result = self.process.terminate();
        let proxy_result = self.codex_proxy.stop_and_reap();
        if let Err(teardown) = process_result {
            eprintln!(
                "{}",
                json!({
                    "component": "Hermes",
                    "error_code": teardown.code(),
                    "event": "teardown_rejected",
                    "owner_invalidated": true,
                    "target": "production_child"
                })
            );
        }
        if let Err(teardown) = proxy_result {
            eprintln!(
                "{}",
                json!({
                    "component": "Hermes",
                    "error_code": teardown.code(),
                    "event": "teardown_rejected",
                    "owner_invalidated": true,
                    "target": "codex_proxy"
                })
            );
        }
        map_port_error(failure)
    }

    #[cfg(test)]
    pub(crate) fn terminate_child_for_test(&mut self) -> HermesAdapterResult<()> {
        self.process.terminate()
    }

    /// Returns bounded count-and-digest evidence for rejected proxy bytes.
    #[must_use]
    pub fn codex_proxy_failure_evidence(&self) -> Option<CodexProxyFailureEvidence> {
        self.codex_proxy.failure_evidence()
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
        let _ = self.codex_proxy.stop_and_reap();
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
    trailing: Vec<u8>,
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
    stream: &Receiver<OuterStreamEvent>,
    deadline: Instant,
) -> HermesAdapterResult<StartupObservation> {
    let mut bytes = Vec::new();
    loop {
        if let Some((wire, frame, consumed)) = parse_startup(&bytes)? {
            let trailing = bytes.split_off(consumed);
            return Ok(StartupObservation {
                wire,
                frame,
                windows_launcher_pid: process.process_id(),
                trailing,
            });
        }
        process.ensure_running()?;
        if Instant::now() >= deadline {
            return Err(error(
                HermesAdapterErrorKind::Timeout,
                "HERMES_PRODUCTION_STARTUP_TIMEOUT",
            ));
        }
        let wait = deadline
            .checked_duration_since(Instant::now())
            .unwrap_or_default()
            .min(Duration::from_millis(2));
        match stream.recv_timeout(wait) {
            Ok(OuterStreamEvent::Data(payload)) => {
                bytes.extend_from_slice(&payload);
                if bytes.len() > MAX_STARTUP_BYTES + MAX_CODEX_PROXY_WIRE_BYTES {
                    return Err(malformed("HERMES_PRODUCTION_STARTUP_LENGTH_REJECTED"));
                }
            }
            Ok(OuterStreamEvent::Eof) => {
                return Err(error(
                    HermesAdapterErrorKind::Failed,
                    "HERMES_PRODUCTION_CHILD_EXITED",
                ));
            }
            Ok(OuterStreamEvent::Failed) => {
                return Err(error(
                    HermesAdapterErrorKind::Transport,
                    "HERMES_PRODUCTION_STARTUP_READ_FAILED",
                ));
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Err(error(
                    HermesAdapterErrorKind::Ambiguous,
                    "HERMES_PRODUCTION_STARTUP_STREAM_LOST",
                ));
            }
        }
    }
}

fn parse_startup(bytes: &[u8]) -> HermesAdapterResult<Option<(StartupWire, Vec<u8>, usize)>> {
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
    let encoded = &bytes[offset + 8..total];
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
    Ok(Some((wire, frame, total)))
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
    let config_sha256 =
        production_config_sha256(frame.endpoint(), &api_key_sha256, model, nonce, mode)?;
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
    mode: &str,
) -> HermesAdapterResult<String> {
    #[derive(Serialize)]
    struct ConfigWire<'a> {
        api_key_sha256: &'a str,
        endpoint: String,
        hermes_config_sha256: Option<&'a str>,
        model: &'a str,
        nonce: &'a str,
        schema: &'a str,
    }
    let hermes_config_sha256 =
        (mode == "official").then(|| encode_sha256(&Sha256::digest(OFFICIAL_HERMES_CONFIG)));
    let bytes = serde_json::to_vec(&ConfigWire {
        api_key_sha256,
        endpoint: endpoint.to_string(),
        hermes_config_sha256: hermes_config_sha256.as_deref(),
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

#[cfg(test)]
mod proxy_host_tests {
    use std::collections::VecDeque;
    use std::io;
    use std::sync::Condvar;

    use lattice_contracts::{AttemptId, CONTRACT_VERSION, Invocation, ProjectSnapshotId, TaskId};

    use super::*;
    use crate::{
        HERMES_CPYTHON_ARCHIVE_BYTES, HERMES_CPYTHON_ARCHIVE_SHA256, HERMES_CPYTHON_BUILD_RELEASE,
        HERMES_CPYTHON_PROVENANCE, HERMES_CPYTHON_SHA256SUMS_SHA256, HERMES_PYPROJECT_SHA256,
        HERMES_RUNTIME_ARCHIVE_SHA256, HERMES_SCHEMA_VERSION, HERMES_UPSTREAM_COMMIT,
        HERMES_UV_LOCK_SHA256, ReflectionEvidence, ReflectionEvidenceKind,
    };

    #[test]
    fn one_turn_gate_forwards_only_complete_valid_jsonl() {
        let mut gate = CodexProxyOneTurnGate::default();
        assert_eq!(
            gate.ingest(
                b"{\"id\":0,\"method\":\"initialize\",\"params\":{}}\n{\"id\":1,\"method\":\"turn/"
            )
            .expect("valid complete line and partial request"),
            b"{\"id\":0,\"method\":\"initialize\",\"params\":{}}\n"
        );
        assert_eq!(gate.turn_start_count(), 0);
        assert_eq!(
            gate.ingest(b"start\",\"params\":{}}\n")
                .expect("completed split turn request"),
            b"{\"id\":1,\"method\":\"turn/start\",\"params\":{}}\n"
        );
        assert_eq!(gate.turn_start_count(), 1);
        gate.finish_input().expect("no partial JSON line");
        gate.ensure_single_turn()
            .expect("exactly one turn reaches the barrier");
    }

    #[test]
    fn one_turn_gate_installs_the_owned_reflection_output_schema() {
        let job = zero_model_job(zero_model_request());
        let schema = codex_reflection_output_schema(&job);
        let mut gate = CodexProxyOneTurnGate::with_output_schema(Some(schema.clone()));
        let admitted = gate
            .ingest(
                b"{\"id\":1,\"method\":\"turn/start\",\"params\":{\"outputSchema\":{\"type\":\"string\"}}}\n",
            )
            .expect("owned output schema replaces the contained request value");
        let line: serde_json::Value = serde_json::from_slice(
            admitted
                .strip_suffix(b"\n")
                .expect("normalized JSONL keeps one newline"),
        )
        .expect("normalized turn is JSON");

        assert_eq!(line["params"]["outputSchema"], schema);
        assert_eq!(gate.turn_start_count(), 1);
    }

    #[test]
    fn one_turn_gate_maps_only_the_contained_work_directory() {
        let mut gate = CodexProxyOneTurnGate::default();
        assert_eq!(
            gate.ingest(b"{\"id\":1,\"method\":\"thread/start\",\"params\":{\"cwd\":\"/wo")
                .expect("partial contained cwd request"),
            b""
        );
        assert_eq!(
            gate.ingest(b"rk\"}}\n")
                .expect("contained cwd is mapped to the broker-owned directory"),
            b"{\"id\":1,\"method\":\"thread/start\",\"params\":{}}\n"
        );

        let mut absent = CodexProxyOneTurnGate::default();
        let without_cwd = b"{\"id\":1,\"method\":\"thread/start\",\"params\":{}}\n";
        assert_eq!(
            absent
                .ingest(without_cwd)
                .expect("absent cwd remains absent"),
            without_cwd
        );

        let mut foreign = CodexProxyOneTurnGate::default();
        assert_eq!(
            foreign
                .ingest(
                    b"{\"id\":1,\"method\":\"thread/start\",\"params\":{\"cwd\":\"C:\\\\foreign\"}}\n"
                )
                .expect_err("foreign cwd must fail closed")
                .code(),
            "HERMES_CODEX_PROXY_CWD_REJECTED"
        );

        let mut duplicate = CodexProxyOneTurnGate::default();
        assert_eq!(
            duplicate
                .ingest(
                    b"{\"id\":1,\"method\":\"thread/start\",\"params\":{\"cwd\":\"/work\",\"cwd\":\"/work\"}}\n"
                )
                .expect_err("duplicate cwd must fail closed")
                .code(),
            "HERMES_CODEX_PROXY_JSONL_REJECTED"
        );

        let mut turn = CodexProxyOneTurnGate::default();
        assert_eq!(
            turn.ingest(b"{\"id\":2,\"method\":\"turn/start\",\"params\":{\"cwd\":\"/work\"}}\n")
                .expect("contained turn cwd is mapped to the broker-owned directory"),
            b"{\"id\":2,\"method\":\"turn/start\",\"params\":{}}\n"
        );
        assert_eq!(turn.turn_start_count(), 1);

        let mut foreign_turn = CodexProxyOneTurnGate::default();
        assert_eq!(
            foreign_turn
                .ingest(b"{\"id\":2,\"method\":\"turn/start\",\"params\":{\"cwd\":\"C:\\\\foreign\"}}\n")
                .expect_err("foreign turn cwd must fail closed")
                .code(),
            "HERMES_CODEX_PROXY_CWD_REJECTED"
        );
    }

    #[test]
    fn one_turn_gate_rejects_second_turn_and_malformed_jsonl() {
        let mut second = CodexProxyOneTurnGate::default();
        second
            .ingest(b"{\"id\":1,\"method\":\"turn/start\",\"params\":{}}\n")
            .expect("first turn admitted");
        assert_eq!(
            second
                .ingest(b"{\"id\":2,\"method\":\"turn/start\",\"params\":{}}\n")
                .expect_err("second turn is rejected before forwarding")
                .code(),
            "HERMES_CODEX_PROXY_TURN_REPLAY_REJECTED"
        );

        let mut malformed = CodexProxyOneTurnGate::default();
        assert_eq!(
            malformed
                .ingest(b"{\"id\":0,\"method\":\"initialize\",\"params\":{}}\nnot-json\n")
                .expect_err("malformed JSON line is rejected before forwarding")
                .code(),
            "HERMES_CODEX_PROXY_JSONL_REJECTED"
        );

        let mut duplicate = CodexProxyOneTurnGate::default();
        assert_eq!(
            duplicate
                .ingest(b"{\"method\":\"turn/start\",\"method\":\"turn/start\"}\n")
                .expect_err("duplicate JSON-RPC keys are rejected")
                .code(),
            "HERMES_CODEX_PROXY_JSONL_REJECTED"
        );
    }

    #[test]
    fn one_turn_gate_rejects_oversize_and_partial_eof() {
        let mut oversize = CodexProxyOneTurnGate::default();
        assert!(
            oversize
                .ingest(&vec![b' '; MAX_CODEX_PROXY_JSONL_LINE_BYTES])
                .expect("bounded partial line is held")
                .is_empty()
        );
        assert_eq!(
            oversize
                .ingest(b"x\n")
                .expect_err("oversize line is rejected before forwarding")
                .code(),
            "HERMES_CODEX_PROXY_JSONL_SIZE_REJECTED"
        );

        let mut partial = CodexProxyOneTurnGate::default();
        assert!(
            partial
                .ingest(br#"{"id":1,"method":"turn/start""#)
                .expect("bounded partial JSON is held")
                .is_empty()
        );
        assert_eq!(
            partial
                .finish_input()
                .expect_err("EOF with a partial JSON line is rejected")
                .code(),
            "HERMES_CODEX_PROXY_JSONL_PARTIAL_REJECTED"
        );

        let empty = CodexProxyOneTurnGate::default();
        assert_eq!(
            empty
                .ensure_single_turn()
                .expect_err("zero turns cannot cross the terminal barrier")
                .code(),
            "HERMES_CODEX_PROXY_TURN_COUNT_REJECTED"
        );
    }

    #[test]
    fn official_hermes_config_bytes_are_exact_and_cross_bound() {
        let config_text = std::str::from_utf8(OFFICIAL_HERMES_CONFIG).expect("ASCII YAML");
        assert_eq!(OFFICIAL_HERMES_CONFIG.len(), 254);
        assert_eq!(
            encode_sha256(&Sha256::digest(OFFICIAL_HERMES_CONFIG)),
            "1bc83178fc5fbbbb12fb7c9ff512b88c13bda8a55bfadcd572970f9bc06d1a45"
        );
        assert!(PRIVATE_RUNNER_SOURCE.contains(config_text));
        assert!(OUTER_RUNNER_SOURCE.contains(config_text));
        let endpoint = "127.0.0.1:8642".parse().expect("fixed loopback endpoint");
        let api_key_sha256 = "a".repeat(64);
        let nonce = "b".repeat(64);
        let official = production_config_sha256(
            endpoint,
            &api_key_sha256,
            "hermes-agent",
            &nonce,
            "official",
        )
        .expect("official config binding");
        let fixture = production_config_sha256(
            endpoint,
            &api_key_sha256,
            "hermes-agent",
            &nonce,
            "scripted_fixture",
        )
        .expect("fixture config binding");
        assert_ne!(official, fixture);
    }

    #[derive(Default)]
    struct BlockingProxyState {
        cancelled: Mutex<bool>,
        wake: Condvar,
        open_entered: AtomicBool,
        read_exited: AtomicBool,
        write_entered: AtomicBool,
    }

    struct BlockingProxyControl(Arc<BlockingProxyState>);

    impl ProductionCodexProxyControl for BlockingProxyControl {
        fn ensure_running(&self) -> HermesAdapterResult<()> {
            if *self.0.cancelled.lock().expect("blocking control lock") {
                Err(error(
                    HermesAdapterErrorKind::Failed,
                    "HERMES_CODEX_PROXY_TEST_CHILD_EXITED",
                ))
            } else {
                Ok(())
            }
        }

        fn terminate(&self) -> HermesAdapterResult<()> {
            *self.0.cancelled.lock().expect("blocking control lock") = true;
            self.0.wake.notify_all();
            Ok(())
        }
    }

    struct BlockingOpenProvider {
        state: Arc<BlockingProxyState>,
        control: Arc<BlockingProxyControl>,
    }

    impl BlockingOpenProvider {
        fn new(state: Arc<BlockingProxyState>) -> Self {
            Self {
                control: Arc::new(BlockingProxyControl(Arc::clone(&state))),
                state,
            }
        }
    }

    impl ProductionCodexProxyProvider for BlockingOpenProvider {
        fn control(&self) -> Arc<dyn ProductionCodexProxyControl> {
            self.control.clone()
        }

        fn open(
            self: Box<Self>,
            _absolute_deadline: Instant,
        ) -> HermesAdapterResult<ProductionCodexProxyDuplex> {
            self.state.open_entered.store(true, Ordering::Release);
            let mut cancelled = self.state.cancelled.lock().expect("blocking open lock");
            while !*cancelled {
                cancelled = self.state.wake.wait(cancelled).expect("blocking open wait");
            }
            Err(error(
                HermesAdapterErrorKind::Cancelled,
                "HERMES_CODEX_PROXY_OPEN_CANCELLED",
            ))
        }
    }

    struct BlockingReader(Arc<BlockingProxyState>);

    impl Read for BlockingReader {
        fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
            let mut cancelled = self.0.cancelled.lock().expect("blocking read lock");
            while !*cancelled {
                cancelled = self.0.wake.wait(cancelled).expect("blocking read wait");
            }
            self.0.read_exited.store(true, Ordering::Release);
            Ok(0)
        }
    }

    struct BlockingWriter(Arc<BlockingProxyState>);

    impl Write for BlockingWriter {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            self.0.write_entered.store(true, Ordering::Release);
            let mut cancelled = self.0.cancelled.lock().expect("blocking write lock");
            while !*cancelled {
                cancelled = self.0.wake.wait(cancelled).expect("blocking write wait");
            }
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "cancelled test provider",
            ))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct BlockingIoProvider {
        state: Arc<BlockingProxyState>,
        control: Arc<BlockingProxyControl>,
    }

    struct RecordingWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for RecordingWriter {
        fn write(&mut self, payload: &[u8]) -> io::Result<usize> {
            self.0
                .lock()
                .expect("recording writer lock")
                .extend_from_slice(payload);
            Ok(payload.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct RecordingIoProvider {
        state: Arc<BlockingProxyState>,
        control: Arc<BlockingProxyControl>,
        written: Arc<Mutex<Vec<u8>>>,
    }

    struct HalfCloseReader(mpsc::Receiver<()>);

    impl Read for HalfCloseReader {
        fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
            self.0
                .recv()
                .map_err(|_| io::Error::other("half-close signal lost"))?;
            Ok(0)
        }
    }

    struct HalfCloseWriter {
        close: Option<mpsc::Sender<()>>,
        close_count: Arc<AtomicU64>,
        written: Arc<Mutex<Vec<u8>>>,
    }

    impl Write for HalfCloseWriter {
        fn write(&mut self, payload: &[u8]) -> io::Result<usize> {
            self.written
                .lock()
                .map_err(|_| io::Error::other("half-close writer poisoned"))?
                .extend_from_slice(payload);
            Ok(payload.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Drop for HalfCloseWriter {
        fn drop(&mut self) {
            self.close_count.fetch_add(1, Ordering::AcqRel);
            if let Some(close) = self.close.take() {
                let _ = close.send(());
            }
        }
    }

    struct HalfCloseControl(mpsc::Sender<()>);

    impl ProductionCodexProxyControl for HalfCloseControl {
        fn ensure_running(&self) -> HermesAdapterResult<()> {
            Ok(())
        }

        fn terminate(&self) -> HermesAdapterResult<()> {
            let _ = self.0.send(());
            Ok(())
        }
    }

    struct HalfCloseProvider {
        close: mpsc::Sender<()>,
        eof: mpsc::Receiver<()>,
        close_count: Arc<AtomicU64>,
        control: Arc<HalfCloseControl>,
        written: Arc<Mutex<Vec<u8>>>,
    }

    impl HalfCloseProvider {
        fn new() -> (Self, Arc<AtomicU64>, Arc<Mutex<Vec<u8>>>) {
            let (close, eof) = mpsc::channel();
            let close_count = Arc::new(AtomicU64::new(0));
            let written = Arc::new(Mutex::new(Vec::new()));
            (
                Self {
                    close: close.clone(),
                    eof,
                    close_count: Arc::clone(&close_count),
                    control: Arc::new(HalfCloseControl(close)),
                    written: Arc::clone(&written),
                },
                close_count,
                written,
            )
        }
    }

    impl ProductionCodexProxyProvider for HalfCloseProvider {
        fn control(&self) -> Arc<dyn ProductionCodexProxyControl> {
            self.control.clone()
        }

        fn open(
            self: Box<Self>,
            _absolute_deadline: Instant,
        ) -> HermesAdapterResult<ProductionCodexProxyDuplex> {
            Ok(ProductionCodexProxyDuplex::new(
                Box::new(HalfCloseReader(self.eof)),
                Box::new(HalfCloseWriter {
                    close: Some(self.close),
                    close_count: self.close_count,
                    written: self.written,
                }),
            ))
        }
    }

    impl RecordingIoProvider {
        fn new(state: Arc<BlockingProxyState>, written: Arc<Mutex<Vec<u8>>>) -> Self {
            Self {
                control: Arc::new(BlockingProxyControl(Arc::clone(&state))),
                state,
                written,
            }
        }
    }

    impl ProductionCodexProxyProvider for RecordingIoProvider {
        fn control(&self) -> Arc<dyn ProductionCodexProxyControl> {
            self.control.clone()
        }

        fn open(
            self: Box<Self>,
            _absolute_deadline: Instant,
        ) -> HermesAdapterResult<ProductionCodexProxyDuplex> {
            Ok(ProductionCodexProxyDuplex::new(
                Box::new(BlockingReader(Arc::clone(&self.state))),
                Box::new(RecordingWriter(Arc::clone(&self.written))),
            ))
        }
    }

    impl BlockingIoProvider {
        fn new(state: Arc<BlockingProxyState>) -> Self {
            Self {
                control: Arc::new(BlockingProxyControl(Arc::clone(&state))),
                state,
            }
        }
    }

    impl ProductionCodexProxyProvider for BlockingIoProvider {
        fn control(&self) -> Arc<dyn ProductionCodexProxyControl> {
            self.control.clone()
        }

        fn open(
            self: Box<Self>,
            _absolute_deadline: Instant,
        ) -> HermesAdapterResult<ProductionCodexProxyDuplex> {
            Ok(ProductionCodexProxyDuplex::new(
                Box::new(BlockingReader(Arc::clone(&self.state))),
                Box::new(BlockingWriter(Arc::clone(&self.state))),
            ))
        }
    }

    fn test_binding(nonce: &str, receipt: &ContentDigest) -> [u8; 32] {
        CodexProxyHostSession::new(nonce, receipt, Instant::now() + Duration::from_secs(2))
            .expect("test session")
            .binding()
    }

    fn test_sink(name: &str) -> (std::fs::File, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "lattice-hermes-{name}-{}-{}",
            std::process::id(),
            RUNNER_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let sink = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .expect("create exact test sink");
        (sink, path)
    }

    fn wait_until(predicate: impl Fn() -> bool, message: &str) {
        let deadline = Instant::now() + Duration::from_secs(1);
        while !predicate() {
            assert!(Instant::now() < deadline, "{message}");
            thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn pre_magic_bytes_fail_closed_with_only_bounded_digest_evidence() {
        let path = std::env::temp_dir().join(format!(
            "lattice-hermes-proxy-evidence-{}-{}",
            std::process::id(),
            RUNNER_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let outer_input = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .expect("create exact test sink");
        let (sender, receiver) = mpsc::sync_channel(1);
        let owner = Arc::new(ContainmentOwnerState::new("11".repeat(32)));
        let mut host = ProductionCodexProxyHost::start(
            Box::new(ScriptedCodexProxyProvider::default()),
            &"22".repeat(32),
            &ContentDigest::from_sha256("33".repeat(32)).expect("digest"),
            Instant::now() + Duration::from_secs(2),
            outer_input,
            receiver,
            Vec::new(),
            Arc::clone(&owner),
        )
        .expect("start proxy host");
        let diagnostic = b"bwrap: pre-exec diagnostic\n";
        sender
            .send(OuterStreamEvent::Data(diagnostic.to_vec()))
            .expect("send untrusted diagnostic");
        let deadline = Instant::now() + Duration::from_secs(1);
        let failure = loop {
            if let Err(failure) = host.ensure_live() {
                break failure;
            }
            assert!(
                Instant::now() < deadline,
                "proxy rejection reached deadline"
            );
            thread::sleep(Duration::from_millis(1));
        };
        assert_eq!(failure.code(), "HERMES_CODEX_PROXY_MAGIC_REJECTED");
        let evidence = host.failure_evidence().expect("bounded failure evidence");
        assert_eq!(evidence.byte_count(), diagnostic.len() as u64);
        assert_eq!(
            evidence.sha256(),
            encode_sha256(&Sha256::digest(diagnostic))
        );
        assert!(!owner.active.load(Ordering::Acquire));
        let terminal = host.terminate().expect_err("terminal failure is retained");
        assert_eq!(terminal.code(), "HERMES_CODEX_PROXY_MAGIC_REJECTED");
        drop(host);
        fs::remove_file(path).expect("remove exact test sink");
    }

    #[test]
    fn reverse_half_close_keeps_inbound_data_and_close_valid() {
        let receipt = ContentDigest::from_sha256("33".repeat(32)).expect("digest");
        let nonce = "22".repeat(32);
        let mut session =
            CodexProxyHostSession::new(&nonce, &receipt, Instant::now() + Duration::from_secs(2))
                .expect("test session");
        let binding = session.binding();
        let open = encode_codex_proxy_test_frame(1, 0, binding, &[]);
        assert_eq!(
            session.accept(&open).expect("open"),
            CodexProxyHostEvent::Open
        );
        session.encode_open_ack().expect("open ack");
        session.encode_close().expect("provider output close");
        let data = encode_codex_proxy_test_frame(2, 1, binding, b"late input");
        assert_eq!(
            session.accept(&data).expect("independent inbound data"),
            CodexProxyHostEvent::Data(b"late input".to_vec())
        );
        let close = encode_codex_proxy_test_frame(3, 2, binding, &[]);
        assert_eq!(
            session.accept(&close).expect("independent inbound close"),
            CodexProxyHostEvent::Close
        );
        let terminal = encode_codex_proxy_test_frame(5, 3, binding, &[]);
        assert_eq!(
            session.accept(&terminal).expect("bound terminal"),
            CodexProxyHostEvent::Terminal
        );
        let replay = encode_codex_proxy_test_frame(5, 4, binding, &[]);
        assert_eq!(
            session
                .accept(&replay)
                .expect_err("terminal replay rejected")
                .code(),
            "HERMES_CODEX_PROXY_STATE_REJECTED"
        );
    }

    #[test]
    fn adapter_success_half_closes_provider_once_and_waits_for_bound_terminal() {
        let (outer_input, path) = test_sink("proxy-adapter-success-half-close");
        let (sender, receiver) = mpsc::sync_channel(8);
        let (provider, close_count, written) = HalfCloseProvider::new();
        let nonce = "22".repeat(32);
        let receipt = ContentDigest::from_sha256("33".repeat(32)).expect("digest");
        let binding = test_binding(&nonce, &receipt);
        let mut host = ProductionCodexProxyHost::start(
            Box::new(provider),
            &nonce,
            &receipt,
            Instant::now() + Duration::from_secs(2),
            outer_input,
            receiver,
            Vec::new(),
            Arc::new(ContainmentOwnerState::new("11".repeat(32))),
        )
        .expect("start proxy host");

        let contained = concat!(
            "{\"id\":0,\"method\":\"initialize\",\"params\":{}}\n",
            "{\"id\":1,\"method\":\"thread/start\",\"params\":{\"cwd\":\"/work\"}}\n",
            "{\"id\":2,\"method\":\"turn/start\",\"params\":{}}\n"
        )
        .as_bytes();
        let expected = concat!(
            "{\"id\":0,\"method\":\"initialize\",\"params\":{}}\n",
            "{\"id\":1,\"method\":\"thread/start\",\"params\":{}}\n",
            "{\"id\":2,\"method\":\"turn/start\",\"params\":{}}\n"
        )
        .as_bytes();
        let mut frames = encode_codex_proxy_test_frame(1, 0, binding, &[]);
        frames.extend_from_slice(&encode_codex_proxy_test_frame(2, 1, binding, contained));
        sender
            .send(OuterStreamEvent::Data(frames))
            .expect("send authenticated one-turn input");
        wait_until(
            || written.lock().expect("half-close written bytes").as_slice() == expected,
            "one-turn input was not forwarded",
        );

        let peer_path = path.clone();
        let peer_sender = sender.clone();
        let peer = thread::spawn(move || {
            wait_until(
                || close_count.load(Ordering::Acquire) == 1,
                "adapter success did not close provider input",
            );
            wait_until(
                || {
                    fs::read(&peer_path).is_ok_and(|bytes| {
                        bytes
                            .windows(CODEX_PROXY_MAGIC.len())
                            .filter(|window| *window == CODEX_PROXY_MAGIC)
                            .count()
                            >= 2
                    })
                },
                "provider EOF did not produce outbound close",
            );
            let mut terminal = encode_codex_proxy_test_frame(3, 2, binding, &[]);
            terminal.extend_from_slice(&encode_codex_proxy_test_frame(5, 3, binding, &[]));
            peer_sender
                .send(OuterStreamEvent::Data(terminal))
                .expect("send peer close and terminal");
        });

        host.complete_after_adapter_success(Instant::now() + Duration::from_secs(1))
            .expect("adapter success crosses the bound terminal barrier");
        peer.join().expect("bound terminal peer");
        assert_eq!(
            host.complete_after_adapter_success(Instant::now() + Duration::from_millis(10))
                .expect_err("adapter success is one-shot")
                .code(),
            "HERMES_CODEX_PROXY_ADAPTER_SUCCESS_REPLAY_REJECTED"
        );
        assert_eq!(host.status.lock().expect("host status").turn_start_count, 1);
        host.terminate().expect("completed host terminates cleanly");
        drop(host);
        fs::remove_file(path).expect("remove exact test sink");
    }

    #[test]
    fn provider_eof_before_authenticated_remote_close_fails_closed() {
        let (outer_input, path) = test_sink("proxy-early-eof");
        let (sender, receiver) = mpsc::sync_channel(8);
        let owner = Arc::new(ContainmentOwnerState::new("11".repeat(32)));
        let nonce = "22".repeat(32);
        let receipt = ContentDigest::from_sha256("33".repeat(32)).expect("digest");
        let binding = test_binding(&nonce, &receipt);
        let mut host = ProductionCodexProxyHost::start(
            Box::new(ScriptedCodexProxyProvider::default()),
            &nonce,
            &receipt,
            Instant::now() + Duration::from_secs(2),
            outer_input,
            receiver,
            Vec::new(),
            Arc::clone(&owner),
        )
        .expect("start proxy host");
        sender
            .send(OuterStreamEvent::Data(encode_codex_proxy_test_frame(
                1,
                0,
                binding,
                &[],
            )))
            .expect("send open");
        wait_until(|| host.ensure_live().is_err(), "early EOF was not rejected");
        let failure = host.ensure_live().expect_err("early EOF stays failed");
        assert_eq!(
            failure.code(),
            "HERMES_CODEX_PROXY_PROVIDER_EOF_BEFORE_CLOSE"
        );
        assert!(!owner.active.load(Ordering::Acquire));
        let _ = host.terminate();
        drop(host);
        fs::remove_file(path).expect("remove exact test sink");
    }

    #[test]
    fn blocked_provider_open_is_cancelled_before_bounded_join() {
        let (outer_input, path) = test_sink("proxy-blocked-open");
        let (sender, receiver) = mpsc::sync_channel(8);
        let state = Arc::new(BlockingProxyState::default());
        let nonce = "22".repeat(32);
        let receipt = ContentDigest::from_sha256("33".repeat(32)).expect("digest");
        let binding = test_binding(&nonce, &receipt);
        let host = ProductionCodexProxyHost::start(
            Box::new(BlockingOpenProvider::new(Arc::clone(&state))),
            &nonce,
            &receipt,
            Instant::now() + Duration::from_secs(2),
            outer_input,
            receiver,
            Vec::new(),
            Arc::new(ContainmentOwnerState::new("11".repeat(32))),
        )
        .expect("start proxy host");
        sender
            .send(OuterStreamEvent::Data(encode_codex_proxy_test_frame(
                1,
                0,
                binding,
                &[],
            )))
            .expect("send open");
        wait_until(
            || state.open_entered.load(Ordering::Acquire),
            "provider open did not block",
        );
        let (result_sender, result_receiver) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let mut host = host;
            let _ = result_sender.send(host.terminate());
        });
        let result = result_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("terminate must cancel open before bounded join");
        assert_eq!(
            result.expect_err("cancelled open is retained").code(),
            "HERMES_CODEX_PROXY_OPEN_CANCELLED"
        );
        fs::remove_file(path).expect("remove exact test sink");
    }

    #[test]
    fn blocked_provider_read_and_write_are_cancelled_before_join() {
        let (outer_input, path) = test_sink("proxy-blocked-io");
        let (sender, receiver) = mpsc::sync_channel(8);
        let state = Arc::new(BlockingProxyState::default());
        let nonce = "22".repeat(32);
        let receipt = ContentDigest::from_sha256("33".repeat(32)).expect("digest");
        let binding = test_binding(&nonce, &receipt);
        let mut host = ProductionCodexProxyHost::start(
            Box::new(BlockingIoProvider::new(Arc::clone(&state))),
            &nonce,
            &receipt,
            Instant::now() + Duration::from_secs(2),
            outer_input,
            receiver,
            Vec::new(),
            Arc::new(ContainmentOwnerState::new("11".repeat(32))),
        )
        .expect("start proxy host");
        sender
            .send(OuterStreamEvent::Data(encode_codex_proxy_test_frame(
                1,
                0,
                binding,
                &[],
            )))
            .expect("send open");
        wait_until(
            || host.status.lock().expect("host status").authenticated_open,
            "provider did not authenticate open",
        );
        sender
            .send(OuterStreamEvent::Data(encode_codex_proxy_test_frame(
                2,
                1,
                binding,
                b"{\"id\":0,\"method\":\"initialize\",\"params\":{}}\n",
            )))
            .expect("send provider input");
        wait_until(
            || state.write_entered.load(Ordering::Acquire),
            "provider write did not block",
        );
        let started = Instant::now();
        let failure = host
            .terminate()
            .expect_err("cancelled blocked write remains a failure");
        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(failure.code(), "HERMES_CODEX_PROXY_PROVIDER_WRITE_FAILED");
        wait_until(
            || state.read_exited.load(Ordering::Acquire),
            "provider read did not exit after cancellation",
        );
        drop(host);
        fs::remove_file(path).expect("remove exact test sink");
    }

    #[test]
    fn second_turn_is_not_forwarded_and_cancels_the_owned_provider() {
        let (outer_input, path) = test_sink("proxy-second-turn");
        let (sender, receiver) = mpsc::sync_channel(8);
        let state = Arc::new(BlockingProxyState::default());
        let written = Arc::new(Mutex::new(Vec::new()));
        let nonce = "22".repeat(32);
        let receipt = ContentDigest::from_sha256("33".repeat(32)).expect("digest");
        let binding = test_binding(&nonce, &receipt);
        let mut host = ProductionCodexProxyHost::start(
            Box::new(RecordingIoProvider::new(
                Arc::clone(&state),
                Arc::clone(&written),
            )),
            &nonce,
            &receipt,
            Instant::now() + Duration::from_secs(2),
            outer_input,
            receiver,
            Vec::new(),
            Arc::new(ContainmentOwnerState::new("11".repeat(32))),
        )
        .expect("start proxy host");
        sender
            .send(OuterStreamEvent::Data(encode_codex_proxy_test_frame(
                1,
                0,
                binding,
                &[],
            )))
            .expect("send open");
        wait_until(
            || host.status.lock().expect("host status").authenticated_open,
            "provider did not authenticate open",
        );
        let contained_thread =
            b"{\"id\":0,\"method\":\"thread/start\",\"params\":{\"cwd\":\"/work\"}}\n";
        let broker_thread = b"{\"id\":0,\"method\":\"thread/start\",\"params\":{}}\n";
        sender
            .send(OuterStreamEvent::Data(encode_codex_proxy_test_frame(
                2,
                1,
                binding,
                contained_thread,
            )))
            .expect("send contained thread start");
        wait_until(
            || written.lock().expect("written input").as_slice() == broker_thread,
            "contained cwd was not mapped before provider forwarding",
        );
        let first = b"{\"id\":1,\"method\":\"turn/start\",\"params\":{}}\n";
        sender
            .send(OuterStreamEvent::Data(encode_codex_proxy_test_frame(
                2, 2, binding, first,
            )))
            .expect("send first turn");
        let expected = [broker_thread.as_slice(), first.as_slice()].concat();
        wait_until(
            || written.lock().expect("written input").as_slice() == expected,
            "first turn was not forwarded",
        );
        sender
            .send(OuterStreamEvent::Data(encode_codex_proxy_test_frame(
                2,
                3,
                binding,
                b"{\"id\":2,\"method\":\"turn/start\",\"params\":{}}\n",
            )))
            .expect("send rejected second turn");
        wait_until(
            || host.ensure_live().is_err(),
            "second turn was not rejected",
        );
        let failure = host.ensure_live().expect_err("second turn remains failed");
        assert_eq!(failure.code(), "HERMES_CODEX_PROXY_TURN_REPLAY_REJECTED");
        assert_eq!(written.lock().expect("written input").as_slice(), expected);
        assert!(*state.cancelled.lock().expect("cancelled provider"));
        wait_until(
            || state.read_exited.load(Ordering::Acquire),
            "provider reader was not reaped after cancellation",
        );
        let _ = host.terminate();
        drop(host);
        fs::remove_file(path).expect("remove exact test sink");
    }

    #[test]
    fn partial_jsonl_eof_is_not_forwarded_and_cancels_the_owned_provider() {
        let (outer_input, path) = test_sink("proxy-partial-jsonl");
        let (sender, receiver) = mpsc::sync_channel(8);
        let state = Arc::new(BlockingProxyState::default());
        let written = Arc::new(Mutex::new(Vec::new()));
        let nonce = "22".repeat(32);
        let receipt = ContentDigest::from_sha256("33".repeat(32)).expect("digest");
        let binding = test_binding(&nonce, &receipt);
        let mut host = ProductionCodexProxyHost::start(
            Box::new(RecordingIoProvider::new(
                Arc::clone(&state),
                Arc::clone(&written),
            )),
            &nonce,
            &receipt,
            Instant::now() + Duration::from_secs(2),
            outer_input,
            receiver,
            Vec::new(),
            Arc::new(ContainmentOwnerState::new("11".repeat(32))),
        )
        .expect("start proxy host");
        let mut frames = encode_codex_proxy_test_frame(1, 0, binding, &[]);
        frames.extend_from_slice(&encode_codex_proxy_test_frame(
            2,
            1,
            binding,
            br#"{"id":1,"method":"turn/start""#,
        ));
        sender
            .send(OuterStreamEvent::Data(frames))
            .expect("send partial JSONL");
        sender.send(OuterStreamEvent::Eof).expect("send outer EOF");
        wait_until(
            || host.ensure_live().is_err(),
            "partial JSONL was not rejected",
        );
        let failure = host
            .ensure_live()
            .expect_err("partial JSONL remains failed");
        assert_eq!(failure.code(), "HERMES_CODEX_PROXY_JSONL_PARTIAL_REJECTED");
        assert!(written.lock().expect("written input").is_empty());
        assert!(*state.cancelled.lock().expect("cancelled provider"));
        wait_until(
            || state.read_exited.load(Ordering::Acquire),
            "provider reader was not reaped after cancellation",
        );
        let _ = host.terminate();
        drop(host);
        fs::remove_file(path).expect("remove exact test sink");
    }

    #[test]
    fn clean_terminal_is_required_before_evidence_barrier() {
        let (outer_input, path) = test_sink("proxy-no-open");
        let (_sender, receiver) = mpsc::sync_channel(1);
        let mut host = ProductionCodexProxyHost::start(
            Box::new(ScriptedCodexProxyProvider::default()),
            &"22".repeat(32),
            &ContentDigest::from_sha256("33".repeat(32)).expect("digest"),
            Instant::now() + Duration::from_secs(2),
            outer_input,
            receiver,
            Vec::new(),
            Arc::new(ContainmentOwnerState::new("11".repeat(32))),
        )
        .expect("start proxy host");
        let failure = host
            .wait_for_clean_terminal(Instant::now() + Duration::from_millis(25))
            .expect_err("no-open host cannot cross completion barrier");
        assert_eq!(failure.code(), "HERMES_CODEX_PROXY_COMPLETION_TIMEOUT");
        host.terminate().expect("unused provider terminates");
        drop(host);
        fs::remove_file(path).expect("remove exact test sink");
    }

    #[test]
    fn provider_output_gate_filters_codex_0146_compatibility_notices() {
        let mut gate = CodexProxyProviderOutputGate::default();
        let admitted = gate
            .ingest(
                br#"{"method":"remoteControl/status/changed","params":{"environmentId":"environment-zero","installationId":"installation-zero","serverName":"lattice","status":"running"},"emittedAtMs":1}
{"method":"thread/started","params":{"thread":{"id":"thread-zero-model"}},"emittedAtMs":2}
{"id":1,"result":{"thread":{"id":"thread-zero-model"}}}
{"method":"deprecationNotice","params":{"details":"legacy client notice","summary":"deprecated field"},"emittedAtMs":3}
"#,
            )
            .expect("0.146 compatibility notices are normalized for frozen Hermes");
        gate.finish_input().expect("no partial provider JSONL");

        let lines = String::from_utf8(admitted)
            .expect("normalized provider output remains UTF-8")
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        assert_eq!(
            lines.len(),
            2,
            "remote-control and deprecation notices are not forwarded"
        );
        let lifecycle: serde_json::Value =
            serde_json::from_str(&lines[0]).expect("normalized lifecycle line");
        assert_eq!(lifecycle["method"], "thread/started");
        assert!(lifecycle.get("emittedAtMs").is_none());
        assert_eq!(lifecycle["params"]["thread"]["id"], "thread-zero-model");

        let response: serde_json::Value =
            serde_json::from_str(&lines[1]).expect("response line is preserved");
        assert_eq!(response["id"], 1);
        assert_eq!(response["result"]["thread"]["id"], "thread-zero-model");
    }

    #[test]
    fn provider_output_gate_fails_closed_on_unknown_notifications() {
        let mut gate = CodexProxyProviderOutputGate::default();
        assert_eq!(
            gate.ingest(
                br#"{"method":"future/tool/request","params":{},"emittedAtMs":1}
"#
            )
            .expect_err("unknown app-server notifications cannot be hidden from Hermes")
            .code(),
            "HERMES_CODEX_PROXY_PROVIDER_NOTICE_REJECTED"
        );
    }

    #[test]
    fn delayed_invalid_frame_before_terminal_prevents_completion() {
        let (outer_input, path) = test_sink("proxy-delayed-invalid");
        let (sender, receiver) = mpsc::sync_channel(8);
        let nonce = "22".repeat(32);
        let receipt = ContentDigest::from_sha256("33".repeat(32)).expect("digest");
        let binding = test_binding(&nonce, &receipt);
        let mut host = ProductionCodexProxyHost::start(
            Box::new(ScriptedCodexProxyProvider::default()),
            &nonce,
            &receipt,
            Instant::now() + Duration::from_secs(2),
            outer_input,
            receiver,
            Vec::new(),
            Arc::new(ContainmentOwnerState::new("11".repeat(32))),
        )
        .expect("start proxy host");
        let mut open_then_close = encode_codex_proxy_test_frame(1, 0, binding, &[]);
        open_then_close.extend_from_slice(&encode_codex_proxy_test_frame(3, 1, binding, &[]));
        sender
            .send(OuterStreamEvent::Data(open_then_close))
            .expect("send open and remote close");
        wait_until(
            || host.status.lock().expect("host status").authenticated_open,
            "provider did not authenticate open",
        );
        thread::sleep(Duration::from_millis(25));
        assert!(
            !host.status.lock().expect("host status").clean_terminal,
            "bilateral close without the inner terminal frame is not complete"
        );
        sender
            .send(OuterStreamEvent::Data(encode_codex_proxy_test_frame(
                2,
                2,
                binding,
                b"replayed after close",
            )))
            .expect("send delayed invalid frame");
        let failure = host
            .wait_for_clean_terminal(Instant::now() + Duration::from_secs(1))
            .expect_err("invalid pre-terminal frame fails the barrier");
        assert_eq!(failure.code(), "HERMES_CODEX_PROXY_STATE_REJECTED");
        let _ = host.terminate();
        drop(host);
        fs::remove_file(path).expect("remove exact test sink");
    }

    #[derive(Default)]
    struct InteractiveFakeCodexObservation {
        calls: Vec<String>,
        reflection_emitted: bool,
        thread_start_had_cwd: Option<bool>,
        turn_input: Option<String>,
        turn_start_had_cwd: Option<bool>,
        turn_output_schema: Option<serde_json::Value>,
    }

    #[derive(Default)]
    struct InteractiveFakeCodexQueue {
        bytes: VecDeque<u8>,
        cancelled: bool,
    }

    struct InteractiveFakeCodexState {
        observation: Arc<Mutex<InteractiveFakeCodexObservation>>,
        queue: Mutex<InteractiveFakeCodexQueue>,
        reflection: String,
        fail_turn: bool,
        wake: Condvar,
    }

    impl InteractiveFakeCodexState {
        fn enqueue(&self, messages: &[serde_json::Value]) -> io::Result<()> {
            let mut queue = self
                .queue
                .lock()
                .map_err(|_| io::Error::other("fake Codex queue poisoned"))?;
            for message in messages {
                let encoded = serde_json::to_vec(message)
                    .map_err(|failure| io::Error::other(failure.to_string()))?;
                queue.bytes.extend(encoded);
                queue.bytes.push_back(b'\n');
            }
            self.wake.notify_all();
            Ok(())
        }

        fn turn_responses(&self, request_id: &serde_json::Value) -> Vec<serde_json::Value> {
            let mut responses = vec![
                serde_json::json!({
                    "id": request_id,
                    "result": {"turn": {"id": "turn-zero-model"}}
                }),
                serde_json::json!({
                    "method": "turn/started",
                    "params": {
                        "threadId": "thread-zero-model",
                        "turn": {"id": "turn-zero-model", "status": "inProgress"}
                    }
                }),
            ];
            if self.fail_turn {
                responses.push(serde_json::json!({
                    "method": "turn/completed",
                    "params": {
                        "threadId": "thread-zero-model",
                        "turn": {
                            "id": "turn-zero-model",
                            "status": "failed",
                            "error": {"message": "zero-model injected failure"}
                        }
                    }
                }));
            } else {
                responses.push(serde_json::json!({
                    "method": "item/completed",
                    "params": {
                        "threadId": "thread-zero-model",
                        "turnId": "turn-zero-model",
                        "item": {
                            "id": "item-zero-model",
                            "type": "agentMessage",
                            "text": self.reflection
                        }
                    }
                }));
                responses.push(serde_json::json!({
                    "method": "turn/completed",
                    "params": {
                        "threadId": "thread-zero-model",
                        "turn": {"id": "turn-zero-model", "status": "completed"}
                    }
                }));
            }
            responses
        }
    }

    struct InteractiveFakeCodexReader(Arc<InteractiveFakeCodexState>);

    impl Read for InteractiveFakeCodexReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if buffer.is_empty() {
                return Ok(0);
            }
            let mut queue = self
                .0
                .queue
                .lock()
                .map_err(|_| io::Error::other("fake Codex queue poisoned"))?;
            loop {
                if !queue.bytes.is_empty() {
                    let read = buffer.len().min(queue.bytes.len());
                    for byte in &mut buffer[..read] {
                        *byte = queue.bytes.pop_front().expect("bounded fake response byte");
                    }
                    return Ok(read);
                }
                if queue.cancelled {
                    return Ok(0);
                }
                queue = self
                    .0
                    .wake
                    .wait(queue)
                    .map_err(|_| io::Error::other("fake Codex queue wait poisoned"))?;
            }
        }
    }

    struct InteractiveFakeCodexWriter {
        pending: Vec<u8>,
        state: Arc<InteractiveFakeCodexState>,
    }

    impl InteractiveFakeCodexWriter {
        fn accept_line(&self, line: &[u8]) -> io::Result<()> {
            let request: serde_json::Value = serde_json::from_slice(line)
                .map_err(|failure| io::Error::other(failure.to_string()))?;
            let method = request
                .get("method")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| io::Error::other("fake Codex request lacks method"))?;
            self.state
                .observation
                .lock()
                .map_err(|_| io::Error::other("fake Codex observation poisoned"))?
                .calls
                .push(method.to_owned());

            match method {
                "initialize" => self.state.enqueue(&[serde_json::json!({
                    "id": request.get("id").cloned().unwrap_or(serde_json::Value::Null),
                    "result": {
                        "codexHome": "fake",
                        "platformFamily": "windows",
                        "platformOs": "windows",
                        "userAgent": "lattice-zero-model-fake"
                    }
                })]),
                "initialized" => Ok(()),
                "thread/start" => {
                    let had_cwd = request
                        .get("params")
                        .and_then(|params| params.get("cwd"))
                        .is_some();
                    self.state
                        .observation
                        .lock()
                        .map_err(|_| io::Error::other("fake Codex observation poisoned"))?
                        .thread_start_had_cwd = Some(had_cwd);
                    self.state.enqueue(&[serde_json::json!({
                        "id": request.get("id").cloned().unwrap_or(serde_json::Value::Null),
                        "result": {"thread": {"id": "thread-zero-model"}}
                    })])
                }
                "turn/start" => {
                    let turn_input = request
                        .get("params")
                        .and_then(|params| params.get("input"))
                        .and_then(serde_json::Value::as_array)
                        .and_then(|items| items.first())
                        .and_then(|item| item.get("text"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    let turn_output_schema = request
                        .get("params")
                        .and_then(|params| params.get("outputSchema"))
                        .cloned();
                    let turn_start_had_cwd = request
                        .get("params")
                        .and_then(|params| params.get("cwd"))
                        .is_some();
                    {
                        let mut observation = self
                            .state
                            .observation
                            .lock()
                            .map_err(|_| io::Error::other("fake Codex observation poisoned"))?;
                        observation.turn_input = Some(turn_input);
                        observation.turn_start_had_cwd = Some(turn_start_had_cwd);
                        observation.turn_output_schema = turn_output_schema;
                        observation.reflection_emitted = !self.state.fail_turn;
                    }
                    let responses = self
                        .state
                        .turn_responses(request.get("id").unwrap_or(&serde_json::Value::Null));
                    self.state.enqueue(&responses)
                }
                _ => Err(io::Error::other(format!(
                    "unexpected fake Codex method {method}"
                ))),
            }
        }
    }

    impl Write for InteractiveFakeCodexWriter {
        fn write(&mut self, payload: &[u8]) -> io::Result<usize> {
            self.pending.extend_from_slice(payload);
            while let Some(newline) = self.pending.iter().position(|byte| *byte == b'\n') {
                let mut line = self.pending.drain(..=newline).collect::<Vec<_>>();
                line.pop();
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                self.accept_line(&line)?;
            }
            Ok(payload.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Drop for InteractiveFakeCodexWriter {
        fn drop(&mut self) {
            if let Ok(mut queue) = self.state.queue.lock() {
                queue.cancelled = true;
                self.state.wake.notify_all();
            }
        }
    }

    struct InteractiveFakeCodexControl(Arc<InteractiveFakeCodexState>);

    impl ProductionCodexProxyControl for InteractiveFakeCodexControl {
        fn ensure_running(&self) -> HermesAdapterResult<()> {
            Ok(())
        }

        fn terminate(&self) -> HermesAdapterResult<()> {
            let mut queue = self.0.queue.lock().map_err(|_| {
                error(
                    HermesAdapterErrorKind::Ambiguous,
                    "HERMES_CODEX_PROXY_TEST_STATE_UNKNOWN",
                )
            })?;
            queue.cancelled = true;
            self.0.wake.notify_all();
            Ok(())
        }
    }

    struct InteractiveFakeCodexProvider {
        control: Arc<InteractiveFakeCodexControl>,
        state: Arc<InteractiveFakeCodexState>,
    }

    impl InteractiveFakeCodexProvider {
        fn new(reflection: String) -> (Self, Arc<Mutex<InteractiveFakeCodexObservation>>) {
            Self::with_outcome(reflection, false)
        }

        fn failing() -> (Self, Arc<Mutex<InteractiveFakeCodexObservation>>) {
            Self::with_outcome(String::new(), true)
        }

        fn with_outcome(
            reflection: String,
            fail_turn: bool,
        ) -> (Self, Arc<Mutex<InteractiveFakeCodexObservation>>) {
            let observation = Arc::new(Mutex::new(InteractiveFakeCodexObservation::default()));
            let state = Arc::new(InteractiveFakeCodexState {
                observation: Arc::clone(&observation),
                queue: Mutex::new(InteractiveFakeCodexQueue::default()),
                reflection,
                fail_turn,
                wake: Condvar::new(),
            });
            (
                Self {
                    control: Arc::new(InteractiveFakeCodexControl(Arc::clone(&state))),
                    state,
                },
                observation,
            )
        }
    }

    impl ProductionCodexProxyProvider for InteractiveFakeCodexProvider {
        fn control(&self) -> Arc<dyn ProductionCodexProxyControl> {
            self.control.clone()
        }

        fn open(
            self: Box<Self>,
            absolute_deadline: Instant,
        ) -> HermesAdapterResult<ProductionCodexProxyDuplex> {
            if absolute_deadline <= Instant::now() {
                return Err(error(
                    HermesAdapterErrorKind::Timeout,
                    "HERMES_CODEX_PROXY_DEADLINE_EXCEEDED",
                ));
            }
            Ok(ProductionCodexProxyDuplex::new(
                Box::new(InteractiveFakeCodexReader(Arc::clone(&self.state))),
                Box::new(InteractiveFakeCodexWriter {
                    pending: Vec::new(),
                    state: Arc::clone(&self.state),
                }),
            ))
        }
    }

    fn zero_model_runtime_manifest() -> HermesOfflineRuntimeManifest {
        let bytes = format!(
            concat!(
                "{{\"cpython_archive_bytes\":{},",
                "\"cpython_archive_sha256\":\"{}\",",
                "\"cpython_build_release\":\"{}\",",
                "\"cpython_provenance\":\"{}\",",
                "\"cpython_sha256sums_sha256\":\"{}\",",
                "\"cpython_version\":\"3.12.13\",",
                "\"hermes_archive_sha256\":\"{}\",",
                "\"hermes_commit\":\"{}\",",
                "\"hermes_release\":\"v2026.8.3\",",
                "\"payload_byte_count\":722643145,\"payload_file_count\":14077,",
                "\"payload_manifest_sha256\":\"{}\",",
                "\"platform\":\"x86_64-unknown-linux-gnu\",",
                "\"pyproject_sha256\":\"{}\",",
                "\"schema\":\"lattice.hermes.offline-runtime.v1\",",
                "\"uv_lock_sha256\":\"{}\"}}"
            ),
            HERMES_CPYTHON_ARCHIVE_BYTES,
            HERMES_CPYTHON_ARCHIVE_SHA256,
            HERMES_CPYTHON_BUILD_RELEASE,
            HERMES_CPYTHON_PROVENANCE,
            HERMES_CPYTHON_SHA256SUMS_SHA256,
            HERMES_RUNTIME_ARCHIVE_SHA256,
            HERMES_UPSTREAM_COMMIT,
            OFFICIAL_RUNTIME_TREE_SHA256,
            HERMES_PYPROJECT_SHA256,
            HERMES_UV_LOCK_SHA256,
        );
        HermesOfflineRuntimeManifest::from_canonical_json(bytes.as_bytes())
            .expect("exact zero-model runtime manifest")
    }

    fn zero_model_request() -> HermesResearchRequest {
        HermesResearchRequest::new(
            Invocation::new(
                CONTRACT_VERSION,
                RequestId::new("request-zero-model").expect("request id"),
                TaskId::new("task-zero-model").expect("task id"),
                AttemptId::new("attempt-zero-model").expect("attempt id"),
                ProjectSnapshotId::new("snapshot-zero-model").expect("snapshot id"),
                ContentDigest::from_sha256("aa".repeat(32)).expect("subject digest"),
            )
            .expect("zero-model invocation"),
        )
    }

    fn zero_model_job(request: HermesResearchRequest) -> HermesReflectionJob {
        HermesReflectionJob::new(
            request,
            "session-zero-model",
            "hermes-agent",
            vec![
                ReflectionEvidence::new(
                    ReflectionEvidenceKind::Graphify,
                    ContentDigest::from_sha256("bb".repeat(32)).expect("graph digest"),
                )
                .expect("graph evidence"),
            ],
        )
        .expect("zero-model reflection job")
    }

    fn zero_model_reflection(job: &HermesReflectionJob) -> String {
        format!(
            concat!(
                "{{\"schema_version\":\"{}\",\"binding\":{{",
                "\"request_id\":\"request-zero-model\",",
                "\"task_id\":\"task-zero-model\",",
                "\"attempt_id\":\"attempt-zero-model\",",
                "\"project_snapshot_id\":\"snapshot-zero-model\",",
                "\"subject_digest\":\"{}\",",
                "\"session_id\":\"session-zero-model\",",
                "\"input_digest\":\"{}\",",
                "\"model\":\"hermes-agent\"}},",
                "\"summary\":\"The official Hermes gateway reached the fake Codex proxy.\",",
                "\"findings\":[{{\"classification\":\"inference\",",
                "\"statement\":\"The zero-model shim path completed one turn.\",",
                "\"evidence_digests\":[\"{}\"]}}],",
                "\"next_actions\":[\"Close the successful Codex session before the terminal barrier.\"]}}"
            ),
            HERMES_SCHEMA_VERSION,
            "aa".repeat(32),
            job.input_digest().as_str(),
            "bb".repeat(32),
        )
    }

    #[test]
    #[ignore = "requires WSL2, bubblewrap, and the exact frozen Hermes runtime"]
    fn official_hermes_gateway_reaches_interactive_fake_codex_without_model() {
        let request = zero_model_request();
        let job = zero_model_job(request.clone());
        let reflection = zero_model_reflection(&job);
        serde_json::from_str::<serde_json::Value>(&reflection).expect("canonical reflection JSON");
        let (provider, observation) = InteractiveFakeCodexProvider::new(reflection.clone());
        let isolation_root = std::env::temp_dir().join(format!(
            "lattice-hermes-official-zero-model-{}-{}",
            std::process::id(),
            RUNNER_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let containment = HermesWslContainmentConfig::new(
            r"C:\Windows\System32\wsl.exe",
            OFFICIAL_RUNTIME_GUEST_ROOT,
            isolation_root.clone(),
            fs::canonicalize(std::env::current_dir().expect("cwd"))
                .expect("canonical product root"),
        )
        .expect("official zero-model containment");
        let mut config = HermesProductionRunnerConfig::official_with_broker_digest(
            containment,
            &zero_model_runtime_manifest(),
            &ContentDigest::from_sha256("ff".repeat(32)).expect("broker digest"),
            "production-zero-model-key",
            "hermes-agent",
            Duration::from_secs(10),
            Duration::from_secs(4),
            Duration::from_millis(1),
        )
        .expect("official zero-model config");
        config.codex_provider = Some(Box::new(provider));
        let runner = config
            .launch(Instant::now() + Duration::from_secs(12))
            .expect("official Hermes gateway starts");
        let mut port = runner.bind(job.clone()).expect("bind zero-model job");
        let result = port
            .run_reflection_evidence(&request)
            .expect("successful reflection reaches a clean proxy terminal");
        assert_eq!(
            result.reflection().summary(),
            "The official Hermes gateway reached the fake Codex proxy."
        );
        assert_eq!(
            result.reflection().binding().input_digest(),
            job.input_digest().as_str()
        );
        assert_eq!(
            result.reflection().output_digest(),
            result.evidence().output_digest()
        );
        drop(port);

        let observed = observation.lock().expect("fake Codex observation");
        assert_eq!(
            observed.calls,
            ["initialize", "initialized", "thread/start", "turn/start"],
            "unexpected calls in the complete fake Codex lifecycle"
        );
        assert_eq!(observed.thread_start_had_cwd, Some(false));
        assert_eq!(observed.turn_start_had_cwd, Some(false));
        assert!(observed.reflection_emitted);
        assert!(
            observed
                .turn_input
                .as_deref()
                .is_some_and(|input| input.contains(job.input_digest().as_str()))
        );
        let output_schema = observed
            .turn_output_schema
            .as_ref()
            .expect("production Hermes turns carry the owned reflection schema");
        assert_eq!(
            output_schema["properties"]["schema_version"]["enum"][0],
            HERMES_SCHEMA_VERSION
        );
        assert_eq!(
            output_schema["properties"]["binding"]["properties"]["input_digest"]["enum"][0],
            job.input_digest().as_str()
        );
        drop(observed);
        fs::remove_dir_all(&isolation_root).expect("remove zero-model isolation root");
    }

    #[test]
    #[ignore = "requires WSL2, bubblewrap, and the exact frozen Hermes runtime"]
    fn official_hermes_gateway_reports_failed_fake_codex_turn_without_model() {
        let request = zero_model_request();
        let job = zero_model_job(request.clone());
        let (provider, observation) = InteractiveFakeCodexProvider::failing();
        let isolation_root = std::env::temp_dir().join(format!(
            "lattice-hermes-official-zero-model-failure-{}-{}",
            std::process::id(),
            RUNNER_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let containment = HermesWslContainmentConfig::new(
            r"C:\Windows\System32\wsl.exe",
            OFFICIAL_RUNTIME_GUEST_ROOT,
            isolation_root.clone(),
            fs::canonicalize(std::env::current_dir().expect("cwd"))
                .expect("canonical product root"),
        )
        .expect("official zero-model containment");
        let mut config = HermesProductionRunnerConfig::official_with_broker_digest(
            containment,
            &zero_model_runtime_manifest(),
            &ContentDigest::from_sha256("ff".repeat(32)).expect("broker digest"),
            "production-zero-model-failure-key",
            "hermes-agent",
            Duration::from_secs(10),
            Duration::from_secs(4),
            Duration::from_millis(1),
        )
        .expect("official zero-model failure config");
        config.codex_provider = Some(Box::new(provider));
        let runner = config
            .launch(Instant::now() + Duration::from_secs(12))
            .expect("official Hermes gateway starts");
        let mut port = runner.bind(job).expect("bind zero-model failure job");
        let failure = port
            .run_reflection_evidence(&request)
            .expect_err("failed Codex turn must produce run.failed");
        assert_eq!(failure.code(), "HERMES_RUN_FAILED");
        drop(port);

        let observed = observation.lock().expect("fake Codex observation");
        assert_eq!(
            observed.calls,
            ["initialize", "initialized", "thread/start", "turn/start"]
        );
        assert!(!observed.reflection_emitted);
        drop(observed);
        fs::remove_dir_all(&isolation_root).expect("remove zero-model failure isolation root");
    }
}
