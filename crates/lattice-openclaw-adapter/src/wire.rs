use lattice_cjson::CanonicalValue;
use lattice_contracts::{
    AttemptId, ContentDigest, GatewayAction, GatewayCommandId, GatewayCorrelationId,
    GatewayDenialCode, GatewayProjectStatusTarget, GatewayReply, GatewayReplyBody, GatewayRequest,
    GatewayRequestBody, GatewayStatusTarget, GatewayStopReason, GatewayStopTarget,
    GatewayTaskTarget, GatewayUnknownCode, ProjectId, ProjectSnapshotId, SubjectBinding, TaskId,
};
use lattice_gateway_ipc::{build_request, encode_canonical_frame, inspect_canonical_frame};
use serde::Deserialize;
use std::fmt::Write as _;

use super::{
    GatewayTransportError, GatewayTransportErrorKind, MAX_LAUNCH_RECORD_ID_BYTES,
    OPENCLAW_CLIENT_HELLO_PROTOCOL, OpenClawClientHello, OpenClawProcessStartNonce,
    PROCESS_START_NONCE_BYTES,
};

const OPENCLAW_INBOUND_PROTOCOL: &str = "lattice-openclaw-inbound";
const OPENCLAW_INBOUND_REPLY_PROTOCOL: &str = "lattice-openclaw-inbound-reply";
const OPENCLAW_INBOUND_VERSION: &str = "1";
const SUBMIT_ACTION: &str = "submit";
const STATUS_ACTION: &str = "status";
const STOP_ACTION: &str = "stop";

/// Closed `OpenClaw` Submit selector. Only its Task Spec digest crosses the wire.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenClawSubmitRequest {
    command_id: GatewayCommandId,
    correlation_id: GatewayCorrelationId,
    binding: SubjectBinding,
}

impl OpenClawSubmitRequest {
    /// Creates one typed selector for an already frozen LATTICE task binding.
    #[must_use]
    pub const fn new(
        command_id: GatewayCommandId,
        correlation_id: GatewayCorrelationId,
        binding: SubjectBinding,
    ) -> Self {
        Self {
            command_id,
            correlation_id,
            binding,
        }
    }

    /// Returns the idempotent command identity.
    #[must_use]
    pub const fn command_id(&self) -> &GatewayCommandId {
        &self.command_id
    }

    /// Returns the request correlation identity.
    #[must_use]
    pub const fn correlation_id(&self) -> &GatewayCorrelationId {
        &self.correlation_id
    }

    /// Returns the local expected binding used to validate the typed reply.
    #[must_use]
    pub const fn binding(&self) -> &SubjectBinding {
        &self.binding
    }
}

/// Closed `OpenClaw` status request. Only shared typed status targets are accepted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenClawStatusRequest {
    command_id: GatewayCommandId,
    correlation_id: GatewayCorrelationId,
    target: GatewayStatusTarget,
}

impl OpenClawStatusRequest {
    pub(crate) const fn from_target(
        command_id: GatewayCommandId,
        correlation_id: GatewayCorrelationId,
        target: GatewayStatusTarget,
    ) -> Self {
        Self {
            command_id,
            correlation_id,
            target,
        }
    }

    /// Creates a bounded project-status request.
    #[must_use]
    pub const fn project(
        command_id: GatewayCommandId,
        correlation_id: GatewayCorrelationId,
        target: GatewayProjectStatusTarget,
    ) -> Self {
        Self {
            command_id,
            correlation_id,
            target: GatewayStatusTarget::Project(target),
        }
    }

    /// Creates a status request for one prior command identity.
    #[must_use]
    pub const fn command(
        command_id: GatewayCommandId,
        correlation_id: GatewayCorrelationId,
        project_id: ProjectId,
        target_command_id: GatewayCommandId,
    ) -> Self {
        Self {
            command_id,
            correlation_id,
            target: GatewayStatusTarget::Command {
                project_id,
                original_command_id: target_command_id,
            },
        }
    }

    /// Creates a status request for one frozen task binding and ledger head.
    #[must_use]
    pub const fn task(
        command_id: GatewayCommandId,
        correlation_id: GatewayCorrelationId,
        target: GatewayTaskTarget,
    ) -> Self {
        Self {
            command_id,
            correlation_id,
            target: GatewayStatusTarget::Task(target),
        }
    }

    /// Returns the idempotent command identity.
    #[must_use]
    pub const fn command_id(&self) -> &GatewayCommandId {
        &self.command_id
    }

    /// Returns the correlation identity.
    #[must_use]
    pub const fn correlation_id(&self) -> &GatewayCorrelationId {
        &self.correlation_id
    }

    /// Returns the exact closed status target.
    #[must_use]
    pub const fn target(&self) -> &GatewayStatusTarget {
        &self.target
    }

    pub(crate) fn gateway_request(&self) -> Result<GatewayRequest, GatewayTransportError> {
        build_request(
            self.command_id.clone(),
            self.correlation_id.clone(),
            GatewayRequestBody::Status(self.target.clone()),
        )
        .map_err(|_| codec_error())
    }
}

/// Closed `OpenClaw` stop request for one exact task attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenClawStopRequest {
    command_id: GatewayCommandId,
    correlation_id: GatewayCorrelationId,
    target: GatewayStopTarget,
}

impl OpenClawStopRequest {
    /// Creates one exact stop request without process, path, or shell authority.
    #[must_use]
    pub const fn new(
        command_id: GatewayCommandId,
        correlation_id: GatewayCorrelationId,
        target: GatewayStopTarget,
    ) -> Self {
        Self {
            command_id,
            correlation_id,
            target,
        }
    }

    /// Returns the idempotent command identity.
    #[must_use]
    pub const fn command_id(&self) -> &GatewayCommandId {
        &self.command_id
    }

    /// Returns the correlation identity.
    #[must_use]
    pub const fn correlation_id(&self) -> &GatewayCorrelationId {
        &self.correlation_id
    }

    /// Returns the exact typed stop target.
    #[must_use]
    pub const fn target(&self) -> &GatewayStopTarget {
        &self.target
    }

    pub(crate) fn gateway_request(&self) -> Result<GatewayRequest, GatewayTransportError> {
        build_request(
            self.command_id.clone(),
            self.correlation_id.clone(),
            GatewayRequestBody::Stop(self.target.clone()),
        )
        .map_err(|_| codec_error())
    }
}

/// Closed result family for a binding-only `OpenClaw` Submit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OpenClawSubmitReplyBody {
    /// The frozen submission reached the gateway and was accepted.
    Accepted {
        /// Exact gateway-validated task binding.
        binding: SubjectBinding,
        /// Terminal receipt digest produced below the gateway boundary.
        command_receipt_digest: ContentDigest,
    },
    /// The gateway denied the valid typed command.
    Denied(GatewayDenialCode),
    /// The gateway could not prove a terminal outcome.
    UnknownOutcome(GatewayUnknownCode),
}

/// Authenticated typed reply to one binding-only Submit selector.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenClawSubmitReply {
    command_id: GatewayCommandId,
    correlation_id: GatewayCorrelationId,
    body: OpenClawSubmitReplyBody,
    gateway_reply_digest: ContentDigest,
}

impl OpenClawSubmitReply {
    /// Returns the idempotent command identity echoed by the gateway.
    #[must_use]
    pub const fn command_id(&self) -> &GatewayCommandId {
        &self.command_id
    }

    /// Returns the correlation identity echoed by the gateway.
    #[must_use]
    pub const fn correlation_id(&self) -> &GatewayCorrelationId {
        &self.correlation_id
    }

    /// Returns the closed typed outcome.
    #[must_use]
    pub const fn body(&self) -> &OpenClawSubmitReplyBody {
        &self.body
    }

    /// Returns the gateway's non-zero typed reply digest as evidence.
    #[must_use]
    pub const fn gateway_reply_digest(&self) -> &ContentDigest {
        &self.gateway_reply_digest
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireSubmitRequest {
    action: String,
    body: WireSubmitSelector,
    command_id: String,
    correlation_id: String,
    protocol: String,
    version: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireSubmitSelector {
    task_spec_digest: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireClientHello {
    launch_record_id: String,
    process_start_nonce: String,
    protocol: String,
    version: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireControlRequest {
    action: String,
    body: serde_json::Value,
    command_id: String,
    correlation_id: String,
    protocol: String,
    version: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case", tag = "kind")]
enum WireStatusTarget {
    Project {
        cursor: serde_json::Value,
        page_size: String,
        project_id: String,
    },
    Command {
        project_id: String,
        target_command_id: String,
    },
    Task {
        target: WireTaskTarget,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireTaskTarget {
    binding: WireBinding,
    expected_ledger_head_digest: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireStopTarget {
    attempt_id: String,
    reason: String,
    target: WireTaskTarget,
}

#[derive(Debug)]
pub(crate) struct DecodedOpenClawSubmitRequest {
    pub(crate) command_id: GatewayCommandId,
    pub(crate) correlation_id: GatewayCorrelationId,
    pub(crate) task_spec_digest: ContentDigest,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireSubmitReply {
    action: String,
    body: WireSubmitReplyBody,
    command_id: String,
    correlation_id: String,
    gateway_reply_digest: String,
    protocol: String,
    version: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case", tag = "outcome")]
enum WireSubmitReplyBody {
    Accepted {
        binding: WireBinding,
        command_receipt_digest: String,
    },
    Denied {
        code: String,
    },
    UnknownOutcome {
        code: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireBinding {
    project_id: String,
    project_snapshot_id: String,
    task_id: String,
    task_revision: String,
    task_spec_digest: String,
}

/// Encodes the exact binding-only Submit wire schema.
///
/// The frame deliberately omits the canonical Task Spec document, task text,
/// memory, SQL, paths, credentials, providers, and caller-supplied identity.
///
/// # Errors
///
/// Returns a codec error if canonical encoding exceeds the shared frame bound.
pub fn encode_openclaw_submit_request(
    request: &OpenClawSubmitRequest,
) -> Result<Vec<u8>, GatewayTransportError> {
    encode_canonical_frame(&object(vec![
        ("action", text(SUBMIT_ACTION)),
        (
            "body",
            object(vec![(
                "task_spec_digest",
                text(request.binding.task_spec_digest().as_str()),
            )]),
        ),
        ("command_id", text(request.command_id.as_str())),
        ("correlation_id", text(request.correlation_id.as_str())),
        ("protocol", text(OPENCLAW_INBOUND_PROTOCOL)),
        ("version", text(OPENCLAW_INBOUND_VERSION)),
    ]))
    .map_err(|_| codec_error())
}

/// Encodes the first authenticated official-mode frame.
///
/// The hello contains only the LATTICE-issued launch record identity and
/// process-start nonce. Package, path, credential, and runtime claims are not
/// accepted from the plugin.
///
/// # Errors
///
/// Returns a codec error if canonical encoding exceeds the shared frame bound.
pub fn encode_openclaw_client_hello(
    hello: &OpenClawClientHello,
) -> Result<Vec<u8>, GatewayTransportError> {
    encode_canonical_frame(&object(vec![
        ("launch_record_id", text(hello.launch_record_id())),
        (
            "process_start_nonce",
            text(lowercase_hex(&hello.process_start_nonce().bytes())),
        ),
        ("protocol", text(OPENCLAW_CLIENT_HELLO_PROTOCOL)),
        ("version", text(OPENCLAW_INBOUND_VERSION)),
    ]))
    .map_err(|_| codec_error())
}

pub(crate) fn decode_openclaw_client_hello(
    frame: &[u8],
) -> Result<OpenClawClientHello, GatewayTransportError> {
    inspect_canonical_frame(frame).map_err(|_| codec_error())?;
    let wire: WireClientHello = serde_json::from_slice(frame).map_err(|_| codec_error())?;
    if wire.protocol != OPENCLAW_CLIENT_HELLO_PROTOCOL
        || wire.version != OPENCLAW_INBOUND_VERSION
        || wire.launch_record_id.is_empty()
        || wire.launch_record_id.len() > MAX_LAUNCH_RECORD_ID_BYTES
        || !wire
            .launch_record_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        || matches!(wire.launch_record_id.as_str(), "." | "..")
    {
        return Err(codec_error());
    }
    Ok(OpenClawClientHello {
        launch_record_id: wire.launch_record_id,
        process_start_nonce: OpenClawProcessStartNonce::new(parse_lowercase_hex_16(
            &wire.process_start_nonce,
        )?)
        .map_err(|_| codec_error())?,
    })
}

/// Encodes one exact project, command, or task status request.
///
/// # Errors
///
/// Returns a codec error if canonical encoding exceeds the shared frame bound.
pub fn encode_openclaw_status_request(
    request: &OpenClawStatusRequest,
) -> Result<Vec<u8>, GatewayTransportError> {
    encode_control_request(
        STATUS_ACTION,
        status_target_value(request.target()),
        request.command_id(),
        request.correlation_id(),
    )
}

/// Encodes one exact typed task-attempt stop request.
///
/// # Errors
///
/// Returns a codec error if canonical encoding exceeds the shared frame bound.
pub fn encode_openclaw_stop_request(
    request: &OpenClawStopRequest,
) -> Result<Vec<u8>, GatewayTransportError> {
    encode_control_request(
        STOP_ACTION,
        stop_target_value(request.target()),
        request.command_id(),
        request.correlation_id(),
    )
}

fn encode_control_request(
    action: &str,
    body: CanonicalValue,
    command_id: &GatewayCommandId,
    correlation_id: &GatewayCorrelationId,
) -> Result<Vec<u8>, GatewayTransportError> {
    encode_canonical_frame(&object(vec![
        ("action", text(action)),
        ("body", body),
        ("command_id", text(command_id.as_str())),
        ("correlation_id", text(correlation_id.as_str())),
        ("protocol", text(OPENCLAW_INBOUND_PROTOCOL)),
        ("version", text(OPENCLAW_INBOUND_VERSION)),
    ]))
    .map_err(|_| codec_error())
}

pub(crate) fn decode_openclaw_control_request(
    frame: &[u8],
) -> Result<GatewayRequest, GatewayTransportError> {
    inspect_canonical_frame(frame).map_err(|_| codec_error())?;
    let wire: WireControlRequest = serde_json::from_slice(frame).map_err(|_| codec_error())?;
    if wire.protocol != OPENCLAW_INBOUND_PROTOCOL || wire.version != OPENCLAW_INBOUND_VERSION {
        return Err(codec_error());
    }
    let body = match wire.action.as_str() {
        STATUS_ACTION => GatewayRequestBody::Status(parse_status_target(wire.body)?),
        STOP_ACTION => GatewayRequestBody::Stop(parse_stop_target(wire.body)?),
        _ => return Err(codec_error()),
    };
    build_request(
        GatewayCommandId::new(wire.command_id).map_err(|_| codec_error())?,
        GatewayCorrelationId::new(wire.correlation_id).map_err(|_| codec_error())?,
        body,
    )
    .map_err(|_| codec_error())
}

pub(crate) fn decode_openclaw_submit_request(
    frame: &[u8],
) -> Result<DecodedOpenClawSubmitRequest, GatewayTransportError> {
    inspect_canonical_frame(frame).map_err(|_| codec_error())?;
    let wire: WireSubmitRequest = serde_json::from_slice(frame).map_err(|_| codec_error())?;
    if wire.protocol != OPENCLAW_INBOUND_PROTOCOL
        || wire.version != OPENCLAW_INBOUND_VERSION
        || wire.action != SUBMIT_ACTION
    {
        return Err(codec_error());
    }
    Ok(DecodedOpenClawSubmitRequest {
        command_id: GatewayCommandId::new(wire.command_id).map_err(|_| codec_error())?,
        correlation_id: GatewayCorrelationId::new(wire.correlation_id)
            .map_err(|_| codec_error())?,
        task_spec_digest: ContentDigest::from_sha256(wire.body.task_spec_digest)
            .map_err(|_| codec_error())?,
    })
}

pub(crate) fn encode_openclaw_submit_reply(
    reply: &GatewayReply,
) -> Result<Vec<u8>, GatewayTransportError> {
    if reply.action() != GatewayAction::Submit {
        return Err(reply_error());
    }
    let body = match reply.body() {
        GatewayReplyBody::SubmitAccepted {
            binding,
            command_receipt_digest,
        } => object(vec![
            ("binding", binding_value(binding)),
            (
                "command_receipt_digest",
                text(command_receipt_digest.as_str()),
            ),
            ("outcome", text("accepted")),
        ]),
        GatewayReplyBody::Denied(code) => object(vec![
            ("code", text(code.as_str())),
            ("outcome", text("denied")),
        ]),
        GatewayReplyBody::UnknownOutcome(code) => object(vec![
            ("code", text(code.as_str())),
            ("outcome", text("unknown_outcome")),
        ]),
        _ => return Err(reply_error()),
    };
    encode_canonical_frame(&object(vec![
        ("action", text(SUBMIT_ACTION)),
        ("body", body),
        ("command_id", text(reply.command_id().as_str())),
        ("correlation_id", text(reply.correlation_id().as_str())),
        ("gateway_reply_digest", text(reply.reply_digest().as_str())),
        ("protocol", text(OPENCLAW_INBOUND_REPLY_PROTOCOL)),
        ("version", text(OPENCLAW_INBOUND_VERSION)),
    ]))
    .map_err(|_| reply_error())
}

pub(crate) fn decode_openclaw_submit_reply(
    request: &OpenClawSubmitRequest,
    frame: &[u8],
) -> Result<OpenClawSubmitReply, GatewayTransportError> {
    inspect_canonical_frame(frame).map_err(|_| reply_error())?;
    let wire: WireSubmitReply = serde_json::from_slice(frame).map_err(|_| reply_error())?;
    if wire.protocol != OPENCLAW_INBOUND_REPLY_PROTOCOL
        || wire.version != OPENCLAW_INBOUND_VERSION
        || wire.action != SUBMIT_ACTION
    {
        return Err(reply_error());
    }
    let command_id = GatewayCommandId::new(wire.command_id).map_err(|_| reply_error())?;
    let correlation_id =
        GatewayCorrelationId::new(wire.correlation_id).map_err(|_| reply_error())?;
    if &command_id != request.command_id() || &correlation_id != request.correlation_id() {
        return Err(reply_error());
    }
    let body = match wire.body {
        WireSubmitReplyBody::Accepted {
            binding,
            command_receipt_digest,
        } => {
            let binding = parse_binding(binding)?;
            if &binding != request.binding() {
                return Err(reply_error());
            }
            OpenClawSubmitReplyBody::Accepted {
                binding,
                command_receipt_digest: ContentDigest::from_sha256(command_receipt_digest)
                    .map_err(|_| reply_error())?,
            }
        }
        WireSubmitReplyBody::Denied { code } => {
            OpenClawSubmitReplyBody::Denied(parse_denial_code(&code)?)
        }
        WireSubmitReplyBody::UnknownOutcome { code } => {
            OpenClawSubmitReplyBody::UnknownOutcome(parse_unknown_code(&code)?)
        }
    };
    Ok(OpenClawSubmitReply {
        command_id,
        correlation_id,
        body,
        gateway_reply_digest: ContentDigest::from_sha256(wire.gateway_reply_digest)
            .map_err(|_| reply_error())?,
    })
}

fn parse_binding(value: WireBinding) -> Result<SubjectBinding, GatewayTransportError> {
    SubjectBinding::new(
        ProjectId::new(value.project_id).map_err(|_| reply_error())?,
        ProjectSnapshotId::new(value.project_snapshot_id).map_err(|_| reply_error())?,
        TaskId::new(value.task_id).map_err(|_| reply_error())?,
        value.task_revision,
        ContentDigest::from_sha256(value.task_spec_digest).map_err(|_| reply_error())?,
    )
    .map_err(|_| reply_error())
}

fn parse_denial_code(value: &str) -> Result<GatewayDenialCode, GatewayTransportError> {
    match value {
        "SCOPE_DENIED" => Ok(GatewayDenialCode::ScopeDenied),
        "SESSION_NOT_CURRENT" => Ok(GatewayDenialCode::SessionNotCurrent),
        "ROLE_DENIED" => Ok(GatewayDenialCode::RoleDenied),
        "PROTECTED_SURFACE_REQUIRED" => Ok(GatewayDenialCode::ProtectedSurfaceRequired),
        "COMMAND_SUBSTITUTION" => Ok(GatewayDenialCode::CommandSubstitution),
        "MALFORMED_SUBJECT" => Ok(GatewayDenialCode::MalformedSubject),
        "DOWNSTREAM_DENIED" => Ok(GatewayDenialCode::DownstreamDenied),
        _ => Err(reply_error()),
    }
}

fn parse_unknown_code(value: &str) -> Result<GatewayUnknownCode, GatewayTransportError> {
    match value {
        "DOWNSTREAM_AMBIGUOUS" => Ok(GatewayUnknownCode::DownstreamAmbiguous),
        "RECONCILIATION_REQUIRED" => Ok(GatewayUnknownCode::ReconciliationRequired),
        _ => Err(reply_error()),
    }
}

fn binding_value(binding: &SubjectBinding) -> CanonicalValue {
    object(vec![
        ("project_id", text(binding.project_id().as_str())),
        (
            "project_snapshot_id",
            text(binding.project_snapshot_id().as_str()),
        ),
        ("task_id", text(binding.task_id().as_str())),
        ("task_revision", text(binding.task_revision())),
        (
            "task_spec_digest",
            text(binding.task_spec_digest().as_str()),
        ),
    ])
}

fn task_target_value(target: &GatewayTaskTarget) -> CanonicalValue {
    object(vec![
        ("binding", binding_value(target.binding())),
        (
            "expected_ledger_head_digest",
            text(target.expected_ledger_head_digest().as_str()),
        ),
    ])
}

fn status_target_value(target: &GatewayStatusTarget) -> CanonicalValue {
    match target {
        GatewayStatusTarget::Project(target) => object(vec![
            (
                "cursor",
                target
                    .cursor()
                    .map_or(CanonicalValue::Null, |value| text(value.to_owned())),
            ),
            ("kind", text("project")),
            ("page_size", text(target.page_size().to_string())),
            ("project_id", text(target.project_id().as_str())),
        ]),
        GatewayStatusTarget::Command {
            project_id,
            original_command_id,
        } => object(vec![
            ("kind", text("command")),
            ("project_id", text(project_id.as_str())),
            ("target_command_id", text(original_command_id.as_str())),
        ]),
        GatewayStatusTarget::Task(target) => object(vec![
            ("kind", text("task")),
            ("target", task_target_value(target)),
        ]),
    }
}

fn stop_target_value(target: &GatewayStopTarget) -> CanonicalValue {
    object(vec![
        ("attempt_id", text(target.attempt_id().as_str())),
        ("reason", text(target.reason().as_str())),
        ("target", task_target_value(target.target())),
    ])
}

fn parse_status_target(
    value: serde_json::Value,
) -> Result<GatewayStatusTarget, GatewayTransportError> {
    let target: WireStatusTarget = serde_json::from_value(value).map_err(|_| codec_error())?;
    match target {
        WireStatusTarget::Project {
            cursor,
            page_size,
            project_id,
        } => {
            let cursor = match cursor {
                serde_json::Value::Null => None,
                serde_json::Value::String(value) => Some(value),
                _ => return Err(codec_error()),
            };
            let page_size = page_size.parse::<u16>().map_err(|_| codec_error())?;
            GatewayProjectStatusTarget::new(
                ProjectId::new(project_id).map_err(|_| codec_error())?,
                page_size,
                cursor,
            )
            .map(GatewayStatusTarget::Project)
            .map_err(|_| codec_error())
        }
        WireStatusTarget::Command {
            project_id,
            target_command_id,
        } => Ok(GatewayStatusTarget::Command {
            project_id: ProjectId::new(project_id).map_err(|_| codec_error())?,
            original_command_id: GatewayCommandId::new(target_command_id)
                .map_err(|_| codec_error())?,
        }),
        WireStatusTarget::Task { target } => {
            parse_request_task_target(target).map(GatewayStatusTarget::Task)
        }
    }
}

fn parse_stop_target(value: serde_json::Value) -> Result<GatewayStopTarget, GatewayTransportError> {
    let target: WireStopTarget = serde_json::from_value(value).map_err(|_| codec_error())?;
    let reason = match target.reason.as_str() {
        "USER_REQUESTED" => GatewayStopReason::UserRequested,
        "SUPERSEDED" => GatewayStopReason::Superseded,
        "SAFETY_CONCERN" => GatewayStopReason::SafetyConcern,
        _ => return Err(codec_error()),
    };
    GatewayStopTarget::new(
        parse_request_task_target(target.target)?,
        AttemptId::new(target.attempt_id).map_err(|_| codec_error())?,
        reason,
    )
    .map_err(|_| codec_error())
}

fn parse_request_task_target(
    value: WireTaskTarget,
) -> Result<GatewayTaskTarget, GatewayTransportError> {
    GatewayTaskTarget::new(
        parse_request_binding(value.binding)?,
        ContentDigest::from_sha256(value.expected_ledger_head_digest).map_err(|_| codec_error())?,
    )
    .map_err(|_| codec_error())
}

fn parse_request_binding(value: WireBinding) -> Result<SubjectBinding, GatewayTransportError> {
    SubjectBinding::new(
        ProjectId::new(value.project_id).map_err(|_| codec_error())?,
        ProjectSnapshotId::new(value.project_snapshot_id).map_err(|_| codec_error())?,
        TaskId::new(value.task_id).map_err(|_| codec_error())?,
        value.task_revision,
        ContentDigest::from_sha256(value.task_spec_digest).map_err(|_| codec_error())?,
    )
    .map_err(|_| codec_error())
}

fn object(entries: Vec<(&str, CanonicalValue)>) -> CanonicalValue {
    CanonicalValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

fn text(value: impl Into<String>) -> CanonicalValue {
    CanonicalValue::String(value.into())
}

fn lowercase_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}

fn parse_lowercase_hex_16(
    value: &str,
) -> Result<[u8; PROCESS_START_NONCE_BYTES], GatewayTransportError> {
    if value.len() != PROCESS_START_NONCE_BYTES * 2
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(codec_error());
    }
    let mut bytes = [0_u8; PROCESS_START_NONCE_BYTES];
    for (index, output) in bytes.iter_mut().enumerate() {
        let start = index * 2;
        *output = u8::from_str_radix(&value[start..start + 2], 16).map_err(|_| codec_error())?;
    }
    Ok(bytes)
}

fn codec_error() -> GatewayTransportError {
    GatewayTransportError::new(GatewayTransportErrorKind::Codec)
}

fn reply_error() -> GatewayTransportError {
    GatewayTransportError::new(GatewayTransportErrorKind::Reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lattice_contracts::{
        AttemptId, GatewayProjectStatusTarget, GatewayStatusTarget, GatewayStopReason,
        GatewayStopTarget, GatewayTaskTarget,
    };

    fn digest(fill: char) -> ContentDigest {
        ContentDigest::from_sha256(fill.to_string().repeat(64)).expect("digest")
    }

    fn request() -> OpenClawSubmitRequest {
        OpenClawSubmitRequest::new(
            GatewayCommandId::new("command-wire-submit").expect("command"),
            GatewayCorrelationId::new("correlation-wire-submit").expect("correlation"),
            SubjectBinding::new(
                ProjectId::new("project-a").expect("project"),
                ProjectSnapshotId::new("snapshot-a").expect("snapshot"),
                TaskId::new("task-a").expect("task"),
                "1",
                digest('a'),
            )
            .expect("binding"),
        )
    }

    #[test]
    fn submit_wire_contains_only_command_correlation_and_frozen_digest() {
        let frame = encode_openclaw_submit_request(&request()).expect("encode");
        for forbidden in [
            b"canonical_document".as_slice(),
            b"task_text".as_slice(),
            b"memory".as_slice(),
            b"sql".as_slice(),
            b"path".as_slice(),
            b"credential".as_slice(),
            b"provider".as_slice(),
            b"shell".as_slice(),
        ] {
            assert!(
                !frame
                    .windows(forbidden.len())
                    .any(|window| window == forbidden)
            );
        }
        let decoded = decode_openclaw_submit_request(&frame).expect("decode");
        assert_eq!(decoded.command_id, *request().command_id());
        assert_eq!(
            decoded.task_spec_digest,
            request().binding().task_spec_digest().clone()
        );
    }

    #[test]
    fn submit_wire_rejects_forbidden_or_unknown_fields_and_versions() {
        for forbidden in [
            "canonical_document",
            "task_text",
            "memory",
            "sql",
            "path",
            "credential",
            "provider",
            "shell",
        ] {
            let frame = encode_canonical_frame(&object(vec![
                ("action", text(SUBMIT_ACTION)),
                (
                    "body",
                    object(vec![
                        (forbidden, text("blocked")),
                        ("task_spec_digest", text(digest('a').as_str())),
                    ]),
                ),
                ("command_id", text("command-wire-submit")),
                ("correlation_id", text("correlation-wire-submit")),
                ("protocol", text(OPENCLAW_INBOUND_PROTOCOL)),
                ("version", text(OPENCLAW_INBOUND_VERSION)),
            ]))
            .expect("canonical malicious frame");
            assert_eq!(
                decode_openclaw_submit_request(&frame)
                    .expect_err("unknown field must fail")
                    .kind(),
                GatewayTransportErrorKind::Codec
            );
        }

        let wrong_version = encode_canonical_frame(&object(vec![
            ("action", text(SUBMIT_ACTION)),
            (
                "body",
                object(vec![("task_spec_digest", text(digest('a').as_str()))]),
            ),
            ("command_id", text("command-wire-submit")),
            ("correlation_id", text("correlation-wire-submit")),
            ("protocol", text(OPENCLAW_INBOUND_PROTOCOL)),
            ("version", text("2")),
        ]))
        .expect("wrong version frame");
        assert_eq!(
            decode_openclaw_submit_request(&wrong_version)
                .expect_err("unknown version must fail")
                .kind(),
            GatewayTransportErrorKind::Codec
        );
    }

    #[test]
    fn closed_status_and_stop_wire_round_trip_to_shared_typed_targets() {
        let project = OpenClawStatusRequest::project(
            GatewayCommandId::new("command-status-project").expect("command"),
            GatewayCorrelationId::new("correlation-status-project").expect("correlation"),
            GatewayProjectStatusTarget::new(
                ProjectId::new("project-a").expect("project"),
                10,
                Some("cursor-a".to_owned()),
            )
            .expect("project target"),
        );
        let project_frame = encode_openclaw_status_request(&project).expect("encode project");
        assert_eq!(
            decode_openclaw_control_request(&project_frame)
                .expect("decode project")
                .body(),
            &GatewayRequestBody::Status(GatewayStatusTarget::Project(
                GatewayProjectStatusTarget::new(
                    ProjectId::new("project-a").expect("project"),
                    10,
                    Some("cursor-a".to_owned()),
                )
                .expect("project target"),
            ))
        );

        let command = OpenClawStatusRequest::command(
            GatewayCommandId::new("command-status-command").expect("command"),
            GatewayCorrelationId::new("correlation-status-command").expect("correlation"),
            ProjectId::new("project-a").expect("project"),
            GatewayCommandId::new("target-command-a").expect("target command"),
        );
        let command_frame = encode_openclaw_status_request(&command).expect("encode command");
        assert!(
            command_frame
                .windows(b"target_command_id".len())
                .any(|window| { window == b"target_command_id" })
        );
        assert!(matches!(
            decode_openclaw_control_request(&command_frame)
                .expect("decode command")
                .body(),
            GatewayRequestBody::Status(GatewayStatusTarget::Command {
                project_id,
                original_command_id,
            }) if project_id.as_str() == "project-a"
                && original_command_id.as_str() == "target-command-a"
        ));

        let task_target =
            GatewayTaskTarget::new(request().binding().clone(), digest('b')).expect("task target");
        let task = OpenClawStatusRequest::task(
            GatewayCommandId::new("command-status-task").expect("command"),
            GatewayCorrelationId::new("correlation-status-task").expect("correlation"),
            task_target.clone(),
        );
        assert_eq!(
            decode_openclaw_control_request(
                &encode_openclaw_status_request(&task).expect("encode task")
            )
            .expect("decode task")
            .body(),
            &GatewayRequestBody::Status(GatewayStatusTarget::Task(task_target.clone()))
        );

        let stop_target = GatewayStopTarget::new(
            task_target,
            AttemptId::new("attempt-a").expect("attempt"),
            GatewayStopReason::UserRequested,
        )
        .expect("stop target");
        let stop = OpenClawStopRequest::new(
            GatewayCommandId::new("command-stop-task").expect("command"),
            GatewayCorrelationId::new("correlation-stop-task").expect("correlation"),
            stop_target.clone(),
        );
        assert_eq!(
            decode_openclaw_control_request(
                &encode_openclaw_stop_request(&stop).expect("encode stop")
            )
            .expect("decode stop")
            .body(),
            &GatewayRequestBody::Stop(stop_target)
        );
    }

    #[test]
    fn closed_control_wire_rejects_forbidden_fields_and_unlisted_actions() {
        for forbidden in [
            "credential",
            "memory",
            "path",
            "provider",
            "shell",
            "sql",
            "task_text",
        ] {
            let frame = encode_canonical_frame(&object(vec![
                ("action", text(STATUS_ACTION)),
                (
                    "body",
                    object(vec![
                        (forbidden, text("blocked")),
                        ("kind", text("command")),
                        ("project_id", text("project-a")),
                        ("target_command_id", text("target-command-a")),
                    ]),
                ),
                ("command_id", text("command-status-a")),
                ("correlation_id", text("correlation-status-a")),
                ("protocol", text(OPENCLAW_INBOUND_PROTOCOL)),
                ("version", text(OPENCLAW_INBOUND_VERSION)),
            ]))
            .expect("canonical malicious frame");
            assert_eq!(
                decode_openclaw_control_request(&frame)
                    .expect_err("forbidden field must fail")
                    .kind(),
                GatewayTransportErrorKind::Codec
            );
        }

        let unlisted_action = encode_canonical_frame(&object(vec![
            ("action", text("plan")),
            (
                "body",
                object(vec![
                    ("kind", text("command")),
                    ("project_id", text("project-a")),
                    ("target_command_id", text("target-command-a")),
                ]),
            ),
            ("command_id", text("command-plan-a")),
            ("correlation_id", text("correlation-plan-a")),
            ("protocol", text(OPENCLAW_INBOUND_PROTOCOL)),
            ("version", text(OPENCLAW_INBOUND_VERSION)),
        ]))
        .expect("canonical unlisted action");
        assert_eq!(
            decode_openclaw_control_request(&unlisted_action)
                .expect_err("unlisted action must fail")
                .kind(),
            GatewayTransportErrorKind::Codec
        );
    }
}
