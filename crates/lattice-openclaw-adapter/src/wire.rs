use lattice_cjson::CanonicalValue;
use lattice_contracts::{
    ContentDigest, GatewayAction, GatewayCommandId, GatewayCorrelationId, GatewayDenialCode,
    GatewayReply, GatewayReplyBody, GatewayUnknownCode, ProjectId, ProjectSnapshotId,
    SubjectBinding, TaskId,
};
use lattice_gateway_ipc::{encode_canonical_frame, inspect_canonical_frame};
use serde::Deserialize;

use super::{GatewayTransportError, GatewayTransportErrorKind};

const OPENCLAW_INBOUND_PROTOCOL: &str = "lattice-openclaw-inbound";
const OPENCLAW_INBOUND_REPLY_PROTOCOL: &str = "lattice-openclaw-inbound-reply";
const OPENCLAW_INBOUND_VERSION: &str = "1";
const SUBMIT_ACTION: &str = "submit";

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

fn codec_error() -> GatewayTransportError {
    GatewayTransportError::new(GatewayTransportErrorKind::Codec)
}

fn reply_error() -> GatewayTransportError {
    GatewayTransportError::new(GatewayTransportErrorKind::Reply)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
