use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256};
use lattice_contracts::{
    AttemptId, ContentDigest, GATEWAY_PROTOCOL_ID, GATEWAY_PROTOCOL_VERSION,
    GATEWAY_STATUS_PAGE_MAX_ITEMS, GATEWAY_TASK_SPEC_SCHEMA_ID, GATEWAY_TASK_SPEC_SCHEMA_VERSION,
    GatewayAction, GatewayApprovalDisposition, GatewayApprovalId, GatewayApprovalRoute,
    GatewayChallengeId, GatewayCommandId, GatewayCorrelationId, GatewayDenialCode,
    GatewayNormalApprovalKind, GatewayProjectStatusTarget, GatewayReply, GatewayReplyBody,
    GatewayRequest, GatewayRequestBody, GatewayStatusObservation, GatewayStatusTarget,
    GatewayStopDisposition, GatewayStopReason, GatewayStopTarget, GatewayTaskProjection,
    GatewayTaskState, GatewayTaskTarget, GatewayUnknownCode, ProjectId, ProjectSnapshotId,
    SubjectBinding, TaskId, TaskSpecSubmission,
};

use crate::{
    CodecError, CodecErrorKind, MAX_ARRAY_ITEMS, encode_canonical_frame, parse_canonical_frame,
    preflight_encode_value, verify_task_spec_document,
};

const REQUEST_HASH_SCHEMA: &str = "lattice.gateway-request";
const REPLY_HASH_SCHEMA: &str = "lattice.gateway-reply";
const HASH_SCHEMA_VERSION: &str = "1.0";

/// Builds one request and computes its domain-separated digest.
///
/// # Errors
///
/// Rejects a mechanically invalid Task Spec carrier, non-NFC subject, or hash
/// representation.
pub fn build_request(
    command_id: GatewayCommandId,
    correlation_id: GatewayCorrelationId,
    body: GatewayRequestBody,
) -> Result<GatewayRequest, CodecError> {
    validate_request_body(&body)?;
    let subject = request_subject(&command_id, &correlation_id, &body)?;
    let digest = digest_value(REQUEST_HASH_SCHEMA, &subject)?;
    GatewayRequest::new(
        GATEWAY_PROTOCOL_VERSION,
        command_id,
        correlation_id,
        body,
        digest,
    )
    .map_err(|_| CodecError::new(CodecErrorKind::InvalidField))
}

/// Encodes a request to its complete canonical protocol frame.
///
/// # Errors
///
/// Rejects a changed digest, invalid Task Spec binding, or oversized frame.
pub fn encode_request(request: &GatewayRequest) -> Result<Vec<u8>, CodecError> {
    validate_request(request)?;
    encode_canonical_frame(&request_frame(request)?)
}

/// Decodes and mechanically verifies one complete canonical request frame.
///
/// # Errors
///
/// Rejects malformed syntax, unknown/missing fields, unsupported protocol
/// values, shape disagreement, invalid bounds, or digest/binding mismatch.
pub fn decode_request(input: &[u8]) -> Result<GatewayRequest, CodecError> {
    let frame = parse_canonical_frame(input)?;
    let fields = exact_object(
        &frame,
        &[
            "action",
            "body",
            "command_id",
            "correlation_id",
            "protocol",
            "request_digest",
            "version",
        ],
    )?;
    if string(field(fields, "protocol")?)? != GATEWAY_PROTOCOL_ID {
        return Err(CodecError::new(CodecErrorKind::UnsupportedProtocol));
    }
    if string(field(fields, "version")?)? != GATEWAY_PROTOCOL_VERSION.to_string() {
        return Err(CodecError::new(CodecErrorKind::UnsupportedVersion));
    }
    let action = parse_action(string(field(fields, "action")?)?)?;
    let body = parse_request_body(action, field(fields, "body")?)?;
    let request = GatewayRequest::new(
        GATEWAY_PROTOCOL_VERSION,
        GatewayCommandId::new(string(field(fields, "command_id")?)?)
            .map_err(|_| CodecError::new(CodecErrorKind::InvalidField))?,
        GatewayCorrelationId::new(string(field(fields, "correlation_id")?)?)
            .map_err(|_| CodecError::new(CodecErrorKind::InvalidField))?,
        body,
        parse_digest(field(fields, "request_digest")?)?,
    )
    .map_err(|_| CodecError::new(CodecErrorKind::InvalidField))?;
    validate_request(&request)?;
    Ok(request)
}

/// Builds one reply bound to the exact request and computes its digest.
///
/// # Errors
///
/// Rejects an action/body mismatch, non-NFC subject, or invalid hash
/// representation.
pub fn build_reply(
    request: &GatewayRequest,
    body: GatewayReplyBody,
) -> Result<GatewayReply, CodecError> {
    GatewayReply::validate_body(request, &body)
        .map_err(|_| CodecError::new(CodecErrorKind::ShapeMismatch))?;
    let subject = reply_subject(request, &body);
    let digest = digest_value(REPLY_HASH_SCHEMA, &subject)?;
    GatewayReply::new(request, body, digest)
        .map_err(|_| CodecError::new(CodecErrorKind::ShapeMismatch))
}

/// Encodes a typed reply after recomputing its binding and digest.
///
/// # Errors
///
/// Rejects a changed digest, invalid shape, or oversized frame.
pub fn encode_reply(reply: &GatewayReply) -> Result<Vec<u8>, CodecError> {
    let subject = reply_subject_from_reply(reply);
    let actual = digest_value(REPLY_HASH_SCHEMA, &subject)?;
    if &actual != reply.reply_digest() {
        return Err(CodecError::new(CodecErrorKind::DigestMismatch));
    }
    encode_canonical_frame(&reply_frame(reply))
}

/// Decodes a reply and verifies its exact request binding.
///
/// # Errors
///
/// Rejects malformed syntax, unknown/missing fields, a request mismatch,
/// invalid reply shape, or a changed reply digest.
pub fn decode_reply(request: &GatewayRequest, input: &[u8]) -> Result<GatewayReply, CodecError> {
    let frame = parse_canonical_frame(input)?;
    let fields = exact_object(
        &frame,
        &[
            "action",
            "body",
            "command_id",
            "correlation_id",
            "protocol",
            "reply_digest",
            "request_digest",
            "version",
        ],
    )?;
    if string(field(fields, "protocol")?)? != GATEWAY_PROTOCOL_ID {
        return Err(CodecError::new(CodecErrorKind::UnsupportedProtocol));
    }
    if string(field(fields, "version")?)? != GATEWAY_PROTOCOL_VERSION.to_string() {
        return Err(CodecError::new(CodecErrorKind::UnsupportedVersion));
    }
    let action = parse_action(string(field(fields, "action")?)?)?;
    if action != request.action()
        || string(field(fields, "command_id")?)? != request.command_id().as_str()
        || string(field(fields, "correlation_id")?)? != request.correlation_id().as_str()
        || string(field(fields, "request_digest")?)? != request.request_digest().as_str()
    {
        return Err(CodecError::new(CodecErrorKind::ReplyMismatch));
    }
    let body = parse_reply_body(field(fields, "body")?)?;
    let reply_digest = parse_digest(field(fields, "reply_digest")?)?;
    let reply = GatewayReply::new(request, body, reply_digest)
        .map_err(|_| CodecError::new(CodecErrorKind::ShapeMismatch))?;
    let subject = reply_subject_from_reply(&reply);
    if digest_value(REPLY_HASH_SCHEMA, &subject)? != *reply.reply_digest() {
        return Err(CodecError::new(CodecErrorKind::DigestMismatch));
    }
    Ok(reply)
}

fn validate_request(request: &GatewayRequest) -> Result<(), CodecError> {
    if request.version() != GATEWAY_PROTOCOL_VERSION {
        return Err(CodecError::new(CodecErrorKind::UnsupportedVersion));
    }
    validate_request_body(request.body())?;
    let subject = request_subject(
        request.command_id(),
        request.correlation_id(),
        request.body(),
    )?;
    if digest_value(REQUEST_HASH_SCHEMA, &subject)? != *request.request_digest() {
        return Err(CodecError::new(CodecErrorKind::DigestMismatch));
    }
    Ok(())
}

fn validate_request_body(body: &GatewayRequestBody) -> Result<(), CodecError> {
    if let GatewayRequestBody::Submit(submission) = body {
        verify_task_spec_document(
            submission.canonical_document(),
            submission.claimed_spec_digest(),
            submission.binding(),
        )?;
    }
    Ok(())
}

fn request_subject(
    command_id: &GatewayCommandId,
    correlation_id: &GatewayCorrelationId,
    body: &GatewayRequestBody,
) -> Result<CanonicalValue, CodecError> {
    Ok(object(vec![
        ("action", text(body.action().as_str())),
        ("body", request_body_value(body)?),
        ("command_id", text(command_id.as_str())),
        ("correlation_id", text(correlation_id.as_str())),
        ("protocol", text(GATEWAY_PROTOCOL_ID)),
        ("version", text(GATEWAY_PROTOCOL_VERSION.to_string())),
    ]))
}

fn request_frame(request: &GatewayRequest) -> Result<CanonicalValue, CodecError> {
    Ok(object(vec![
        ("action", text(request.action().as_str())),
        ("body", request_body_value(request.body())?),
        ("command_id", text(request.command_id().as_str())),
        ("correlation_id", text(request.correlation_id().as_str())),
        ("protocol", text(GATEWAY_PROTOCOL_ID)),
        ("request_digest", text(request.request_digest().as_str())),
        ("version", text(GATEWAY_PROTOCOL_VERSION.to_string())),
    ]))
}

fn request_body_value(body: &GatewayRequestBody) -> Result<CanonicalValue, CodecError> {
    match body {
        GatewayRequestBody::Submit(submission) => {
            let document = parse_canonical_frame(submission.canonical_document())?;
            Ok(object(vec![
                ("binding", binding_value(submission.binding())),
                ("canonical_document", document),
                (
                    "claimed_spec_digest",
                    text(submission.claimed_spec_digest().as_str()),
                ),
                ("schema_id", text(GATEWAY_TASK_SPEC_SCHEMA_ID)),
                ("schema_version", text(GATEWAY_TASK_SPEC_SCHEMA_VERSION)),
            ]))
        }
        GatewayRequestBody::Plan(target) => Ok(task_target_value(target)),
        GatewayRequestBody::Status(target) => Ok(status_target_value(target)),
        GatewayRequestBody::Approve(route) | GatewayRequestBody::Reject(route) => {
            Ok(approval_route_value(route))
        }
        GatewayRequestBody::Stop(target) => Ok(stop_target_value(target)),
    }
}

fn parse_request_body(
    action: GatewayAction,
    value: &CanonicalValue,
) -> Result<GatewayRequestBody, CodecError> {
    Ok(match action {
        GatewayAction::Submit => {
            let fields = exact_object(
                value,
                &[
                    "binding",
                    "canonical_document",
                    "claimed_spec_digest",
                    "schema_id",
                    "schema_version",
                ],
            )?;
            if string(field(fields, "schema_id")?)? != GATEWAY_TASK_SPEC_SCHEMA_ID {
                return Err(CodecError::new(CodecErrorKind::InvalidField));
            }
            if string(field(fields, "schema_version")?)? != GATEWAY_TASK_SPEC_SCHEMA_VERSION {
                return Err(CodecError::new(CodecErrorKind::UnsupportedTaskSpecVersion));
            }
            let document = lattice_cjson::canonicalize(field(fields, "canonical_document")?)
                .map_err(|_| CodecError::new(CodecErrorKind::Malformed))?
                .into_vec();
            let submission = TaskSpecSubmission::new(
                parse_binding(field(fields, "binding")?)?,
                document,
                parse_digest(field(fields, "claimed_spec_digest")?)?,
            )
            .map_err(|_| CodecError::new(CodecErrorKind::InvalidField))?;
            verify_task_spec_document(
                submission.canonical_document(),
                submission.claimed_spec_digest(),
                submission.binding(),
            )?;
            GatewayRequestBody::Submit(submission)
        }
        GatewayAction::Plan => GatewayRequestBody::Plan(parse_task_target(value)?),
        GatewayAction::Status => GatewayRequestBody::Status(parse_status_target(value)?),
        GatewayAction::Approve => GatewayRequestBody::Approve(parse_approval_route(value)?),
        GatewayAction::Reject => GatewayRequestBody::Reject(parse_approval_route(value)?),
        GatewayAction::Stop => GatewayRequestBody::Stop(parse_stop_target(value)?),
    })
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

fn parse_binding(value: &CanonicalValue) -> Result<SubjectBinding, CodecError> {
    let fields = exact_object(
        value,
        &[
            "project_id",
            "project_snapshot_id",
            "task_id",
            "task_revision",
            "task_spec_digest",
        ],
    )?;
    SubjectBinding::new(
        ProjectId::new(string(field(fields, "project_id")?)?)
            .map_err(|_| CodecError::new(CodecErrorKind::InvalidField))?,
        ProjectSnapshotId::new(string(field(fields, "project_snapshot_id")?)?)
            .map_err(|_| CodecError::new(CodecErrorKind::InvalidField))?,
        TaskId::new(string(field(fields, "task_id")?)?)
            .map_err(|_| CodecError::new(CodecErrorKind::InvalidField))?,
        string(field(fields, "task_revision")?)?,
        parse_digest(field(fields, "task_spec_digest")?)?,
    )
    .map_err(|_| CodecError::new(CodecErrorKind::InvalidField))
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

fn parse_task_target(value: &CanonicalValue) -> Result<GatewayTaskTarget, CodecError> {
    let fields = exact_object(value, &["binding", "expected_ledger_head_digest"])?;
    GatewayTaskTarget::new(
        parse_binding(field(fields, "binding")?)?,
        parse_digest(field(fields, "expected_ledger_head_digest")?)?,
    )
    .map_err(|_| CodecError::new(CodecErrorKind::InvalidField))
}

fn status_target_value(target: &GatewayStatusTarget) -> CanonicalValue {
    match target {
        GatewayStatusTarget::Project(target) => object(vec![
            ("cursor", optional_text(target.cursor())),
            ("kind", text("project")),
            ("page_size", text(target.page_size().to_string())),
            ("project_id", text(target.project_id().as_str())),
        ]),
        GatewayStatusTarget::Task(target) => object(vec![
            ("kind", text("task")),
            ("target", task_target_value(target)),
        ]),
        GatewayStatusTarget::Command {
            project_id,
            original_command_id,
        } => object(vec![
            ("kind", text("command")),
            ("original_command_id", text(original_command_id.as_str())),
            ("project_id", text(project_id.as_str())),
        ]),
    }
}

fn parse_status_target(value: &CanonicalValue) -> Result<GatewayStatusTarget, CodecError> {
    let fields = object_fields(value)?;
    match string(field(fields, "kind")?)? {
        "project" => {
            exact_fields(fields, &["cursor", "kind", "page_size", "project_id"])?;
            let page_size = parse_u16(string(field(fields, "page_size")?)?)?;
            let cursor = optional_string(field(fields, "cursor")?)?;
            Ok(GatewayStatusTarget::Project(
                GatewayProjectStatusTarget::new(
                    ProjectId::new(string(field(fields, "project_id")?)?)
                        .map_err(|_| CodecError::new(CodecErrorKind::InvalidField))?,
                    page_size,
                    cursor,
                )
                .map_err(|_| CodecError::new(CodecErrorKind::InvalidField))?,
            ))
        }
        "task" => {
            exact_fields(fields, &["kind", "target"])?;
            Ok(GatewayStatusTarget::Task(parse_task_target(field(
                fields, "target",
            )?)?))
        }
        "command" => {
            exact_fields(fields, &["kind", "original_command_id", "project_id"])?;
            Ok(GatewayStatusTarget::Command {
                project_id: ProjectId::new(string(field(fields, "project_id")?)?)
                    .map_err(|_| CodecError::new(CodecErrorKind::InvalidField))?,
                original_command_id: GatewayCommandId::new(string(field(
                    fields,
                    "original_command_id",
                )?)?)
                .map_err(|_| CodecError::new(CodecErrorKind::InvalidField))?,
            })
        }
        _ => Err(CodecError::new(CodecErrorKind::ShapeMismatch)),
    }
}

fn approval_route_value(route: &GatewayApprovalRoute) -> CanonicalValue {
    object(vec![
        ("approval_id", text(route.approval_id().as_str())),
        ("binding", binding_value(route.binding())),
        ("challenge_digest", text(route.challenge_digest().as_str())),
        ("challenge_id", text(route.challenge_id().as_str())),
        ("kind", text(route.kind().as_str())),
        (
            "presentation_digest",
            text(route.presentation_digest().as_str()),
        ),
        ("subject_digest", text(route.subject_digest().as_str())),
    ])
}

fn parse_approval_route(value: &CanonicalValue) -> Result<GatewayApprovalRoute, CodecError> {
    let fields = exact_object(
        value,
        &[
            "approval_id",
            "binding",
            "challenge_digest",
            "challenge_id",
            "kind",
            "presentation_digest",
            "subject_digest",
        ],
    )?;
    let kind = match string(field(fields, "kind")?)? {
        "EXECUTION" => GatewayNormalApprovalKind::Execution,
        "MERGE" => GatewayNormalApprovalKind::Merge,
        "PREFERENCE" => GatewayNormalApprovalKind::Preference,
        "PROTECTED_CHANGE" => GatewayNormalApprovalKind::ProtectedChange,
        _ => return Err(CodecError::new(CodecErrorKind::InvalidField)),
    };
    GatewayApprovalRoute::new(
        parse_binding(field(fields, "binding")?)?,
        kind,
        GatewayApprovalId::new(string(field(fields, "approval_id")?)?)
            .map_err(|_| CodecError::new(CodecErrorKind::InvalidField))?,
        GatewayChallengeId::new(string(field(fields, "challenge_id")?)?)
            .map_err(|_| CodecError::new(CodecErrorKind::InvalidField))?,
        parse_digest(field(fields, "subject_digest")?)?,
        parse_digest(field(fields, "challenge_digest")?)?,
        parse_digest(field(fields, "presentation_digest")?)?,
    )
    .map_err(|_| CodecError::new(CodecErrorKind::InvalidField))
}

fn stop_target_value(target: &GatewayStopTarget) -> CanonicalValue {
    object(vec![
        ("attempt_id", text(target.attempt_id().as_str())),
        ("reason", text(target.reason().as_str())),
        ("target", task_target_value(target.target())),
    ])
}

fn parse_stop_target(value: &CanonicalValue) -> Result<GatewayStopTarget, CodecError> {
    let fields = exact_object(value, &["attempt_id", "reason", "target"])?;
    let reason = match string(field(fields, "reason")?)? {
        "USER_REQUESTED" => GatewayStopReason::UserRequested,
        "SUPERSEDED" => GatewayStopReason::Superseded,
        "SAFETY_CONCERN" => GatewayStopReason::SafetyConcern,
        _ => return Err(CodecError::new(CodecErrorKind::InvalidField)),
    };
    GatewayStopTarget::new(
        parse_task_target(field(fields, "target")?)?,
        AttemptId::new(string(field(fields, "attempt_id")?)?)
            .map_err(|_| CodecError::new(CodecErrorKind::InvalidField))?,
        reason,
    )
    .map_err(|_| CodecError::new(CodecErrorKind::InvalidField))
}

fn reply_subject(request: &GatewayRequest, body: &GatewayReplyBody) -> CanonicalValue {
    let body_value = reply_body_value(body);
    object(vec![
        ("action", text(request.action().as_str())),
        ("body", body_value),
        ("command_id", text(request.command_id().as_str())),
        ("correlation_id", text(request.correlation_id().as_str())),
        ("protocol", text(GATEWAY_PROTOCOL_ID)),
        ("request_digest", text(request.request_digest().as_str())),
        ("version", text(GATEWAY_PROTOCOL_VERSION.to_string())),
    ])
}

fn reply_subject_from_reply(reply: &GatewayReply) -> CanonicalValue {
    object(vec![
        ("action", text(reply.action().as_str())),
        ("body", reply_body_value(reply.body())),
        ("command_id", text(reply.command_id().as_str())),
        ("correlation_id", text(reply.correlation_id().as_str())),
        ("protocol", text(GATEWAY_PROTOCOL_ID)),
        ("request_digest", text(reply.request_digest().as_str())),
        ("version", text(GATEWAY_PROTOCOL_VERSION.to_string())),
    ])
}

fn reply_frame(reply: &GatewayReply) -> CanonicalValue {
    object(vec![
        ("action", text(reply.action().as_str())),
        ("body", reply_body_value(reply.body())),
        ("command_id", text(reply.command_id().as_str())),
        ("correlation_id", text(reply.correlation_id().as_str())),
        ("protocol", text(GATEWAY_PROTOCOL_ID)),
        ("reply_digest", text(reply.reply_digest().as_str())),
        ("request_digest", text(reply.request_digest().as_str())),
        ("version", text(GATEWAY_PROTOCOL_VERSION.to_string())),
    ])
}

fn reply_body_value(body: &GatewayReplyBody) -> CanonicalValue {
    match body {
        GatewayReplyBody::SubmitAccepted {
            binding,
            command_receipt_digest,
        } => object(vec![
            ("binding", binding_value(binding)),
            (
                "command_receipt_digest",
                text(command_receipt_digest.as_str()),
            ),
            ("kind", text("submit_accepted")),
        ]),
        GatewayReplyBody::PlanRouted {
            binding,
            command_receipt_digest,
        } => object(vec![
            ("binding", binding_value(binding)),
            (
                "command_receipt_digest",
                text(command_receipt_digest.as_str()),
            ),
            ("kind", text("plan_routed")),
        ]),
        GatewayReplyBody::StatusObserved(observation) => object(vec![
            ("kind", text("status_observed")),
            ("observation", status_observation_value(observation)),
        ]),
        GatewayReplyBody::ApprovalRouted {
            binding,
            approval_id,
            challenge_id,
            challenge_digest,
            disposition,
            routing_receipt_digest,
        } => object(vec![
            ("approval_id", text(approval_id.as_str())),
            ("binding", binding_value(binding)),
            ("challenge_digest", text(challenge_digest.as_str())),
            ("challenge_id", text(challenge_id.as_str())),
            ("disposition", text(disposition.as_str())),
            ("kind", text("approval_routed")),
            (
                "routing_receipt_digest",
                text(routing_receipt_digest.as_str()),
            ),
        ]),
        GatewayReplyBody::StopRouted {
            target,
            disposition,
            routing_receipt_digest,
        } => object(vec![
            ("disposition", text(disposition.as_str())),
            ("kind", text("stop_routed")),
            (
                "routing_receipt_digest",
                text(routing_receipt_digest.as_str()),
            ),
            ("target", stop_target_value(target)),
        ]),
        GatewayReplyBody::Denied(code) => object(vec![
            ("code", text(code.as_str())),
            ("kind", text("denied")),
        ]),
        GatewayReplyBody::UnknownOutcome(code) => object(vec![
            ("code", text(code.as_str())),
            ("kind", text("unknown_outcome")),
        ]),
    }
}

fn parse_reply_body(value: &CanonicalValue) -> Result<GatewayReplyBody, CodecError> {
    let fields = object_fields(value)?;
    match string(field(fields, "kind")?)? {
        "submit_accepted" => {
            exact_fields(fields, &["binding", "command_receipt_digest", "kind"])?;
            Ok(GatewayReplyBody::SubmitAccepted {
                binding: parse_binding(field(fields, "binding")?)?,
                command_receipt_digest: parse_digest(field(fields, "command_receipt_digest")?)?,
            })
        }
        "plan_routed" => {
            exact_fields(fields, &["binding", "command_receipt_digest", "kind"])?;
            Ok(GatewayReplyBody::PlanRouted {
                binding: parse_binding(field(fields, "binding")?)?,
                command_receipt_digest: parse_digest(field(fields, "command_receipt_digest")?)?,
            })
        }
        "status_observed" => {
            exact_fields(fields, &["kind", "observation"])?;
            Ok(GatewayReplyBody::StatusObserved(parse_status_observation(
                field(fields, "observation")?,
            )?))
        }
        "approval_routed" => {
            exact_fields(
                fields,
                &[
                    "approval_id",
                    "binding",
                    "challenge_digest",
                    "challenge_id",
                    "disposition",
                    "kind",
                    "routing_receipt_digest",
                ],
            )?;
            let disposition = match string(field(fields, "disposition")?)? {
                "ROUTED_FOR_VERIFICATION" => GatewayApprovalDisposition::RoutedForVerification,
                "REJECTION_RECORDED" => GatewayApprovalDisposition::RejectionRecorded,
                _ => return Err(CodecError::new(CodecErrorKind::InvalidField)),
            };
            Ok(GatewayReplyBody::ApprovalRouted {
                binding: parse_binding(field(fields, "binding")?)?,
                approval_id: GatewayApprovalId::new(string(field(fields, "approval_id")?)?)
                    .map_err(|_| CodecError::new(CodecErrorKind::InvalidField))?,
                challenge_digest: parse_digest(field(fields, "challenge_digest")?)?,
                challenge_id: GatewayChallengeId::new(string(field(fields, "challenge_id")?)?)
                    .map_err(|_| CodecError::new(CodecErrorKind::InvalidField))?,
                disposition,
                routing_receipt_digest: parse_digest(field(fields, "routing_receipt_digest")?)?,
            })
        }
        "stop_routed" => {
            exact_fields(
                fields,
                &["disposition", "kind", "routing_receipt_digest", "target"],
            )?;
            let disposition = match string(field(fields, "disposition")?)? {
                "REQUESTED" => GatewayStopDisposition::Requested,
                "ALREADY_TERMINAL" => GatewayStopDisposition::AlreadyTerminal,
                "RECONCILIATION_REQUIRED" => GatewayStopDisposition::ReconciliationRequired,
                _ => return Err(CodecError::new(CodecErrorKind::InvalidField)),
            };
            Ok(GatewayReplyBody::StopRouted {
                target: parse_stop_target(field(fields, "target")?)?,
                disposition,
                routing_receipt_digest: parse_digest(field(fields, "routing_receipt_digest")?)?,
            })
        }
        "denied" => {
            exact_fields(fields, &["code", "kind"])?;
            Ok(GatewayReplyBody::Denied(parse_denial(string(field(
                fields, "code",
            )?)?)?))
        }
        "unknown_outcome" => {
            exact_fields(fields, &["code", "kind"])?;
            Ok(GatewayReplyBody::UnknownOutcome(parse_unknown(string(
                field(fields, "code")?,
            )?)?))
        }
        _ => Err(CodecError::new(CodecErrorKind::ShapeMismatch)),
    }
}

fn status_observation_value(observation: &GatewayStatusObservation) -> CanonicalValue {
    match observation {
        GatewayStatusObservation::Project {
            project_id,
            tasks,
            next_cursor,
        } => object(vec![
            ("kind", text("project")),
            ("next_cursor", optional_text(next_cursor.as_deref())),
            ("project_id", text(project_id.as_str())),
            (
                "tasks",
                CanonicalValue::Array(tasks.iter().map(task_projection_value).collect()),
            ),
        ]),
        GatewayStatusObservation::Task(task) => object(vec![
            ("kind", text("task")),
            ("task", task_projection_value(task)),
        ]),
        GatewayStatusObservation::Command {
            project_id,
            original_command_id,
            terminal_reply_digest,
        } => object(vec![
            ("kind", text("command")),
            ("original_command_id", text(original_command_id.as_str())),
            ("project_id", text(project_id.as_str())),
            (
                "terminal_reply_digest",
                text(terminal_reply_digest.as_str()),
            ),
        ]),
    }
}

fn parse_status_observation(
    value: &CanonicalValue,
) -> Result<GatewayStatusObservation, CodecError> {
    let fields = object_fields(value)?;
    match string(field(fields, "kind")?)? {
        "project" => {
            exact_fields(fields, &["kind", "next_cursor", "project_id", "tasks"])?;
            let task_values = array(field(fields, "tasks")?)?;
            if task_values.len() > usize::from(GATEWAY_STATUS_PAGE_MAX_ITEMS) {
                return Err(CodecError::new(CodecErrorKind::ArrayLimit));
            }
            let tasks = task_values
                .iter()
                .map(parse_task_projection)
                .collect::<Result<Vec<_>, _>>()?;
            let next_cursor = optional_string(field(fields, "next_cursor")?)?;
            if next_cursor
                .as_ref()
                .is_some_and(|cursor| cursor.len() > lattice_contracts::GATEWAY_CURSOR_MAX_BYTES)
            {
                return Err(CodecError::new(CodecErrorKind::InvalidField));
            }
            Ok(GatewayStatusObservation::Project {
                project_id: ProjectId::new(string(field(fields, "project_id")?)?)
                    .map_err(|_| CodecError::new(CodecErrorKind::InvalidField))?,
                tasks,
                next_cursor,
            })
        }
        "task" => {
            exact_fields(fields, &["kind", "task"])?;
            Ok(GatewayStatusObservation::Task(parse_task_projection(
                field(fields, "task")?,
            )?))
        }
        "command" => {
            exact_fields(
                fields,
                &[
                    "kind",
                    "original_command_id",
                    "project_id",
                    "terminal_reply_digest",
                ],
            )?;
            Ok(GatewayStatusObservation::Command {
                project_id: ProjectId::new(string(field(fields, "project_id")?)?)
                    .map_err(|_| CodecError::new(CodecErrorKind::InvalidField))?,
                original_command_id: GatewayCommandId::new(string(field(
                    fields,
                    "original_command_id",
                )?)?)
                .map_err(|_| CodecError::new(CodecErrorKind::InvalidField))?,
                terminal_reply_digest: parse_digest(field(fields, "terminal_reply_digest")?)?,
            })
        }
        _ => Err(CodecError::new(CodecErrorKind::ShapeMismatch)),
    }
}

fn task_projection_value(task: &GatewayTaskProjection) -> CanonicalValue {
    object(vec![
        ("binding", binding_value(task.binding())),
        (
            "ledger_head_digest",
            text(task.ledger_head_digest().as_str()),
        ),
        (
            "observation_receipt_digest",
            text(task.observation_receipt_digest().as_str()),
        ),
        ("state", text(task.state().as_str())),
    ])
}

fn parse_task_projection(value: &CanonicalValue) -> Result<GatewayTaskProjection, CodecError> {
    let fields = exact_object(
        value,
        &[
            "binding",
            "ledger_head_digest",
            "observation_receipt_digest",
            "state",
        ],
    )?;
    GatewayTaskProjection::new(
        parse_binding(field(fields, "binding")?)?,
        parse_task_state(string(field(fields, "state")?)?)?,
        parse_digest(field(fields, "ledger_head_digest")?)?,
        parse_digest(field(fields, "observation_receipt_digest")?)?,
    )
    .map_err(|_| CodecError::new(CodecErrorKind::InvalidField))
}

fn parse_task_state(value: &str) -> Result<GatewayTaskState, CodecError> {
    Ok(match value {
        "DRAFT" => GatewayTaskState::Draft,
        "AWAITING_EXECUTION_APPROVAL" => GatewayTaskState::AwaitingExecutionApproval,
        "PREPARING" => GatewayTaskState::Preparing,
        "EXECUTING" => GatewayTaskState::Executing,
        "VERIFYING" => GatewayTaskState::Verifying,
        "REVIEWING" => GatewayTaskState::Reviewing,
        "AWAITING_MERGE_APPROVAL" => GatewayTaskState::AwaitingMergeApproval,
        "MERGING" => GatewayTaskState::Merging,
        "COMPLETED" => GatewayTaskState::Completed,
        "REJECTED" => GatewayTaskState::Rejected,
        "BLOCKED" => GatewayTaskState::Blocked,
        "FAILED" => GatewayTaskState::Failed,
        "STOPPING" => GatewayTaskState::Stopping,
        "CANCELLED" => GatewayTaskState::Cancelled,
        _ => return Err(CodecError::new(CodecErrorKind::InvalidField)),
    })
}

fn parse_denial(value: &str) -> Result<GatewayDenialCode, CodecError> {
    Ok(match value {
        "SCOPE_DENIED" => GatewayDenialCode::ScopeDenied,
        "SESSION_NOT_CURRENT" => GatewayDenialCode::SessionNotCurrent,
        "ROLE_DENIED" => GatewayDenialCode::RoleDenied,
        "PROTECTED_SURFACE_REQUIRED" => GatewayDenialCode::ProtectedSurfaceRequired,
        "COMMAND_SUBSTITUTION" => GatewayDenialCode::CommandSubstitution,
        "MALFORMED_SUBJECT" => GatewayDenialCode::MalformedSubject,
        "DOWNSTREAM_DENIED" => GatewayDenialCode::DownstreamDenied,
        _ => return Err(CodecError::new(CodecErrorKind::InvalidField)),
    })
}

fn parse_unknown(value: &str) -> Result<GatewayUnknownCode, CodecError> {
    Ok(match value {
        "DOWNSTREAM_AMBIGUOUS" => GatewayUnknownCode::DownstreamAmbiguous,
        "RECONCILIATION_REQUIRED" => GatewayUnknownCode::ReconciliationRequired,
        _ => return Err(CodecError::new(CodecErrorKind::InvalidField)),
    })
}

fn parse_action(value: &str) -> Result<GatewayAction, CodecError> {
    match value {
        "submit" => Ok(GatewayAction::Submit),
        "plan" => Ok(GatewayAction::Plan),
        "status" => Ok(GatewayAction::Status),
        "approve" => Ok(GatewayAction::Approve),
        "reject" => Ok(GatewayAction::Reject),
        "stop" => Ok(GatewayAction::Stop),
        _ => Err(CodecError::new(CodecErrorKind::UnknownAction)),
    }
}

fn digest_value(schema: &str, value: &CanonicalValue) -> Result<ContentDigest, CodecError> {
    preflight_encode_value(value)?;
    let domain = HashDomain::new(schema, HASH_SCHEMA_VERSION)
        .map_err(|_| CodecError::new(CodecErrorKind::Malformed))?;
    let digest =
        canonical_sha256(&domain, value).map_err(|_| CodecError::new(CodecErrorKind::Malformed))?;
    ContentDigest::from_sha256(digest.to_hex())
        .map_err(|_| CodecError::new(CodecErrorKind::Malformed))
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

fn optional_text(value: Option<&str>) -> CanonicalValue {
    value.map_or(CanonicalValue::Null, text)
}

fn object_fields(value: &CanonicalValue) -> Result<&[(String, CanonicalValue)], CodecError> {
    match value {
        CanonicalValue::Object(fields) => Ok(fields),
        _ => Err(CodecError::new(CodecErrorKind::ShapeMismatch)),
    }
}

fn exact_object<'a>(
    value: &'a CanonicalValue,
    expected: &[&str],
) -> Result<&'a [(String, CanonicalValue)], CodecError> {
    let fields = object_fields(value)?;
    exact_fields(fields, expected)?;
    Ok(fields)
}

fn exact_fields(fields: &[(String, CanonicalValue)], expected: &[&str]) -> Result<(), CodecError> {
    if fields
        .iter()
        .any(|(key, _)| !expected.contains(&key.as_str()))
    {
        return Err(CodecError::new(CodecErrorKind::UnknownField));
    }
    if expected
        .iter()
        .any(|expected_key| !fields.iter().any(|(key, _)| key == expected_key))
    {
        return Err(CodecError::new(CodecErrorKind::MissingField));
    }
    Ok(())
}

fn field<'a>(
    fields: &'a [(String, CanonicalValue)],
    name: &str,
) -> Result<&'a CanonicalValue, CodecError> {
    fields
        .iter()
        .find_map(|(key, value)| (key == name).then_some(value))
        .ok_or_else(|| CodecError::new(CodecErrorKind::MissingField))
}

fn string(value: &CanonicalValue) -> Result<&str, CodecError> {
    match value {
        CanonicalValue::String(value) => Ok(value),
        _ => Err(CodecError::new(CodecErrorKind::InvalidField)),
    }
}

fn optional_string(value: &CanonicalValue) -> Result<Option<String>, CodecError> {
    match value {
        CanonicalValue::Null => Ok(None),
        CanonicalValue::String(value) => Ok(Some(value.clone())),
        _ => Err(CodecError::new(CodecErrorKind::InvalidField)),
    }
}

fn array(value: &CanonicalValue) -> Result<&[CanonicalValue], CodecError> {
    match value {
        CanonicalValue::Array(values) if values.len() <= MAX_ARRAY_ITEMS => Ok(values),
        CanonicalValue::Array(_) => Err(CodecError::new(CodecErrorKind::ArrayLimit)),
        _ => Err(CodecError::new(CodecErrorKind::InvalidField)),
    }
}

fn parse_digest(value: &CanonicalValue) -> Result<ContentDigest, CodecError> {
    ContentDigest::from_sha256(string(value)?)
        .map_err(|_| CodecError::new(CodecErrorKind::InvalidField))
}

fn parse_u16(value: &str) -> Result<u16, CodecError> {
    if value.is_empty()
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(CodecError::new(CodecErrorKind::InvalidField));
    }
    value
        .parse()
        .map_err(|_| CodecError::new(CodecErrorKind::InvalidField))
}
