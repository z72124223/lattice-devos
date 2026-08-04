//! Strict reconstruction of shared Artifact Store contract values.

use lattice_cjson::CanonicalValue;
use lattice_contracts::{
    ArtifactAuthorityOwnerKind, ArtifactAuthorityReceipt, ArtifactAuthorityStatus,
    ArtifactAvailability, ArtifactBundleBounds, ArtifactByteLength, ArtifactCounter,
    ArtifactDeleteStatus, ArtifactObjectHead, ArtifactObjectIdentity, ArtifactObjectKey,
    ArtifactProvenance, ArtifactPurpose, ArtifactReadAuthorityAction, ArtifactReadAuthorityBinding,
    ArtifactReadAuthorityHead, ArtifactReadAuthorityPair, ArtifactReadAuthorityReceipt,
    ArtifactReadHead, ArtifactReadStatus, ArtifactReferenceAuthorityAction,
    ArtifactReferenceAuthorityBinding, ArtifactReferenceAuthorityHead,
    ArtifactReferenceAuthorityPair, ArtifactReferenceAuthorityReceipt, ArtifactReferenceHead,
    ArtifactReferenceManifest, ArtifactReferenceStatus, ArtifactRevision, AttemptId, DaemonEpoch,
    ProjectSnapshotId, RequestId, RuntimeAdmissionMode, RuntimeKind, SubjectBinding,
};

use crate::artifact_manifest_digest;
use crate::snapshot_parse::{
    SnapshotParseError, SnapshotParseResult, StrictSnapshotObject, parse_digest,
    parse_object_identity, parse_project_id, parse_task_id, parse_u64,
};

pub(crate) fn parse_runtime(value: &str) -> SnapshotParseResult<RuntimeKind> {
    match value {
        "FAKE" => Ok(RuntimeKind::Fake),
        "LIVE" => Ok(RuntimeKind::Live),
        _ => Err(SnapshotParseError),
    }
}

fn parse_counter(value: &str) -> SnapshotParseResult<ArtifactCounter> {
    ArtifactCounter::new(parse_u64(value)?).map_err(|_| SnapshotParseError)
}

fn parse_revision(value: &str) -> SnapshotParseResult<ArtifactRevision> {
    ArtifactRevision::new(parse_u64(value)?).map_err(|_| SnapshotParseError)
}

fn parse_byte_length(value: &str) -> SnapshotParseResult<ArtifactByteLength> {
    ArtifactByteLength::new(parse_u64(value)?).map_err(|_| SnapshotParseError)
}

fn parse_daemon_epoch(value: &str) -> SnapshotParseResult<DaemonEpoch> {
    DaemonEpoch::new(parse_u64(value)?).map_err(|_| SnapshotParseError)
}

fn parse_owner_kind(value: &str) -> SnapshotParseResult<ArtifactAuthorityOwnerKind> {
    match value {
        "TASK_LEDGER" => Ok(ArtifactAuthorityOwnerKind::TaskLedger),
        "CODEBASE_MEMORY" => Ok(ArtifactAuthorityOwnerKind::CodebaseMemory),
        "REVIEW_RUNTIME" => Ok(ArtifactAuthorityOwnerKind::ReviewRuntime),
        "GUARDIAN" => Ok(ArtifactAuthorityOwnerKind::Guardian),
        "ARTIFACT_STORE" => Ok(ArtifactAuthorityOwnerKind::ArtifactStore),
        _ => Err(SnapshotParseError),
    }
}

fn parse_authority_status(value: &str) -> SnapshotParseResult<ArtifactAuthorityStatus> {
    match value {
        "AVAILABLE" => Ok(ArtifactAuthorityStatus::Available),
        "CONSUMED" => Ok(ArtifactAuthorityStatus::Consumed),
        "REVOKED" => Ok(ArtifactAuthorityStatus::Revoked),
        _ => Err(SnapshotParseError),
    }
}

fn parse_reference_action(value: &str) -> SnapshotParseResult<ArtifactReferenceAuthorityAction> {
    match value {
        "PUBLISH_INITIAL_REFERENCE" => {
            Ok(ArtifactReferenceAuthorityAction::PublishInitialReference)
        }
        "ADD_REFERENCE" => Ok(ArtifactReferenceAuthorityAction::AddReference),
        "RELEASE_REFERENCE" => Ok(ArtifactReferenceAuthorityAction::ReleaseReference),
        _ => Err(SnapshotParseError),
    }
}

fn parse_read_action(value: &str) -> SnapshotParseResult<ArtifactReadAuthorityAction> {
    match value {
        "ACQUIRE_READ" => Ok(ArtifactReadAuthorityAction::AcquireRead),
        "RELEASE_READ" => Ok(ArtifactReadAuthorityAction::ReleaseRead),
        _ => Err(SnapshotParseError),
    }
}

fn parse_reference_status(value: &str) -> SnapshotParseResult<ArtifactReferenceStatus> {
    match value {
        "ACTIVE" => Ok(ArtifactReferenceStatus::Active),
        "RELEASED" => Ok(ArtifactReferenceStatus::Released),
        _ => Err(SnapshotParseError),
    }
}

fn parse_read_status(value: &str) -> SnapshotParseResult<ArtifactReadStatus> {
    match value {
        "ACTIVE" => Ok(ArtifactReadStatus::Active),
        "EXPIRED_SUSPECT" => Ok(ArtifactReadStatus::ExpiredSuspect),
        "RELEASED" => Ok(ArtifactReadStatus::Released),
        _ => Err(SnapshotParseError),
    }
}

pub(crate) fn parse_availability(value: &str) -> SnapshotParseResult<ArtifactAvailability> {
    match value {
        "AVAILABLE" => Ok(ArtifactAvailability::Available),
        "DELETE_CLAIMED" => Ok(ArtifactAvailability::DeleteClaimed),
        "DELETED" => Ok(ArtifactAvailability::Deleted),
        "RECONCILIATION_REQUIRED" => Ok(ArtifactAvailability::ReconciliationRequired),
        _ => Err(SnapshotParseError),
    }
}

pub(crate) fn parse_delete_status(value: &str) -> SnapshotParseResult<ArtifactDeleteStatus> {
    match value {
        "NOT_CLAIMED" => Ok(ArtifactDeleteStatus::NotClaimed),
        "CLAIMED" => Ok(ArtifactDeleteStatus::Claimed),
        "VERIFIED_NO_EFFECT" => Ok(ArtifactDeleteStatus::VerifiedNoEffect),
        "VERIFIED_DELETED" => Ok(ArtifactDeleteStatus::VerifiedDeleted),
        "RECONCILIATION_REQUIRED" => Ok(ArtifactDeleteStatus::ReconciliationRequired),
        _ => Err(SnapshotParseError),
    }
}

fn parse_admission(value: &str) -> SnapshotParseResult<RuntimeAdmissionMode> {
    match value {
        "ACTIVE" => Ok(RuntimeAdmissionMode::Active),
        "DRAINING" => Ok(RuntimeAdmissionMode::Draining),
        "CANARY" => Ok(RuntimeAdmissionMode::Canary),
        "STOPPED" => Ok(RuntimeAdmissionMode::Stopped),
        "RECONCILIATION_REQUIRED" => Ok(RuntimeAdmissionMode::ReconciliationRequired),
        _ => Err(SnapshotParseError),
    }
}

fn parse_purpose(value: &str) -> SnapshotParseResult<ArtifactPurpose> {
    match value {
        "GRAPHIFY_GRAPH" => Ok(ArtifactPurpose::GraphifyGraph),
        "HERMES_CANDIDATE" => Ok(ArtifactPurpose::HermesCandidate),
        "CODEX_EVIDENCE" => Ok(ArtifactPurpose::CodexEvidence),
        "REVIEW_BUNDLE" => Ok(ArtifactPurpose::ReviewBundle),
        "CODEBASE_MEMORY_SOURCE" => Ok(ArtifactPurpose::CodebaseMemorySource),
        "UPGRADE_CANDIDATE" => Ok(ArtifactPurpose::UpgradeCandidate),
        "TASK_OUTPUT" => Ok(ArtifactPurpose::TaskOutput),
        _ => Err(SnapshotParseError),
    }
}

pub(crate) fn parse_reference_authority_pair(
    value: &CanonicalValue,
) -> SnapshotParseResult<ArtifactReferenceAuthorityPair> {
    let raw = StrictSnapshotObject::new(
        value,
        &[
            "receipt_version",
            "owner_kind",
            "producer_id",
            "producer_version",
            "runtime",
            "owner_record_id",
            "owner_revision",
            "status",
            "action",
            "project_id",
            "task_id",
            "object",
            "reference_id",
            "observation_digest",
            "authority_receipt_digest",
            "current_head_version",
            "authority_current_head_receipt_digest",
        ],
    )?;
    let owner_kind = parse_owner_kind(raw.string("owner_kind")?)?;
    if raw.string("producer_id")? != owner_kind.producer_id()
        || raw.string("producer_version")? != owner_kind.producer_version()
    {
        return Err(SnapshotParseError);
    }
    let binding = ArtifactReferenceAuthorityBinding::new(
        owner_kind,
        parse_runtime(raw.string("runtime")?)?,
        raw.string("owner_record_id")?.to_owned(),
        parse_revision(raw.string("owner_revision")?)?,
        parse_authority_status(raw.string("status")?)?,
        parse_reference_action(raw.string("action")?)?,
        parse_project_id(raw.string("project_id")?)?,
        parse_task_id(raw.string("task_id")?)?,
        parse_object_identity(raw.get("object")?)?,
        raw.string("reference_id")?.to_owned(),
        parse_digest(raw.string("observation_digest")?)?,
    )
    .map_err(|_| SnapshotParseError)?;
    let receipt_digest = parse_digest(raw.string("authority_receipt_digest")?)?;
    let receipt = ArtifactReferenceAuthorityReceipt::new(
        parse_u16(raw.string("receipt_version")?)?,
        binding.clone(),
        receipt_digest,
    )
    .map_err(|_| SnapshotParseError)?;
    let current_head = ArtifactReferenceAuthorityHead::new(
        parse_u16(raw.string("current_head_version")?)?,
        binding,
        parse_digest(raw.string("authority_current_head_receipt_digest")?)?,
    )
    .map_err(|_| SnapshotParseError)?;
    ArtifactReferenceAuthorityPair::new(receipt, current_head).map_err(|_| SnapshotParseError)
}

pub(crate) fn parse_read_authority_pair(
    value: &CanonicalValue,
) -> SnapshotParseResult<ArtifactReadAuthorityPair> {
    let raw = StrictSnapshotObject::new(
        value,
        &[
            "receipt_version",
            "owner_kind",
            "producer_id",
            "producer_version",
            "runtime",
            "owner_record_id",
            "owner_revision",
            "status",
            "action",
            "project_id",
            "task_id",
            "object",
            "read_claim_id",
            "observation_digest",
            "authority_receipt_digest",
            "current_head_version",
            "authority_current_head_receipt_digest",
        ],
    )?;
    let owner_kind = parse_owner_kind(raw.string("owner_kind")?)?;
    if raw.string("producer_id")? != owner_kind.producer_id()
        || raw.string("producer_version")? != owner_kind.producer_version()
    {
        return Err(SnapshotParseError);
    }
    let binding = ArtifactReadAuthorityBinding::new(
        owner_kind,
        parse_runtime(raw.string("runtime")?)?,
        raw.string("owner_record_id")?.to_owned(),
        parse_revision(raw.string("owner_revision")?)?,
        parse_authority_status(raw.string("status")?)?,
        parse_read_action(raw.string("action")?)?,
        parse_project_id(raw.string("project_id")?)?,
        parse_task_id(raw.string("task_id")?)?,
        parse_object_identity(raw.get("object")?)?,
        raw.string("read_claim_id")?.to_owned(),
        parse_digest(raw.string("observation_digest")?)?,
    )
    .map_err(|_| SnapshotParseError)?;
    let receipt = ArtifactReadAuthorityReceipt::new(
        parse_u16(raw.string("receipt_version")?)?,
        binding.clone(),
        parse_digest(raw.string("authority_receipt_digest")?)?,
    )
    .map_err(|_| SnapshotParseError)?;
    let current_head = ArtifactReadAuthorityHead::new(
        parse_u16(raw.string("current_head_version")?)?,
        binding,
        parse_digest(raw.string("authority_current_head_receipt_digest")?)?,
    )
    .map_err(|_| SnapshotParseError)?;
    ArtifactReadAuthorityPair::new(receipt, current_head).map_err(|_| SnapshotParseError)
}

fn parse_provenance(value: &CanonicalValue) -> SnapshotParseResult<ArtifactProvenance> {
    let raw = StrictSnapshotObject::new(
        value,
        &[
            "source_producer_id",
            "source_producer_version",
            "source_runtime",
            "producer_binary_digest",
            "adapter_id",
            "adapter_version",
            "adapter_binary_digest",
            "invocation_id",
            "correlation_id",
            "run_id",
            "sequence",
            "produced_at",
            "payload_digest",
            "capability_id",
            "input_set_digest",
            "configuration_digest",
            "evidence_digest",
            "registry_authority_receipt_digest",
            "registry_current_head_digest",
            "effect_claim_id",
            "effect_claim_digest",
            "daemon_instance_id",
            "daemon_epoch",
            "runtime_admission",
            "capability_owner_receipt_digest",
            "capability_owner_current_head_digest",
            "limit_snapshot_digest",
        ],
    )?;
    ArtifactProvenance::new(
        raw.string("source_producer_id")?.to_owned(),
        raw.string("source_producer_version")?.to_owned(),
        parse_runtime(raw.string("source_runtime")?)?,
        parse_digest(raw.string("producer_binary_digest")?)?,
        raw.string("adapter_id")?.to_owned(),
        raw.string("adapter_version")?.to_owned(),
        parse_digest(raw.string("adapter_binary_digest")?)?,
        raw.string("invocation_id")?.to_owned(),
        raw.string("correlation_id")?.to_owned(),
        raw.string("run_id")?.to_owned(),
        parse_counter(raw.string("sequence")?)?,
        raw.string("produced_at")?.to_owned(),
        parse_digest(raw.string("payload_digest")?)?,
        raw.string("capability_id")?.to_owned(),
        parse_digest(raw.string("input_set_digest")?)?,
        parse_digest(raw.string("configuration_digest")?)?,
        parse_digest(raw.string("evidence_digest")?)?,
        parse_digest(raw.string("registry_authority_receipt_digest")?)?,
        parse_digest(raw.string("registry_current_head_digest")?)?,
        raw.string("effect_claim_id")?.to_owned(),
        parse_digest(raw.string("effect_claim_digest")?)?,
        raw.string("daemon_instance_id")?.to_owned(),
        parse_daemon_epoch(raw.string("daemon_epoch")?)?,
        parse_admission(raw.string("runtime_admission")?)?,
        parse_digest(raw.string("capability_owner_receipt_digest")?)?,
        parse_digest(raw.string("capability_owner_current_head_digest")?)?,
        parse_digest(raw.string("limit_snapshot_digest")?)?,
    )
    .map_err(|_| SnapshotParseError)
}

fn parse_bundle(value: &CanonicalValue) -> SnapshotParseResult<Option<ArtifactBundleBounds>> {
    if matches!(value, CanonicalValue::Null) {
        return Ok(None);
    }
    let raw =
        StrictSnapshotObject::new(value, &["entry_count", "max_depth", "total_declared_bytes"])?;
    ArtifactBundleBounds::new(
        parse_counter(raw.string("entry_count")?)?,
        parse_counter(raw.string("max_depth")?)?,
        parse_byte_length(raw.string("total_declared_bytes")?)?,
    )
    .map(Some)
    .map_err(|_| SnapshotParseError)
}

pub(crate) fn parse_reference_head(
    value: &CanonicalValue,
) -> SnapshotParseResult<ArtifactReferenceHead> {
    let raw = StrictSnapshotObject::new(
        value,
        &[
            "manifest",
            "transition_authority",
            "revision",
            "status",
            "transition_digest",
        ],
    )?;
    let manifest_raw = StrictSnapshotObject::new(
        raw.get("manifest")?,
        &["payload", "manifest_digest", "creation_authority"],
    )?;
    let creation_authority =
        parse_reference_authority_pair(manifest_raw.get("creation_authority")?)?;
    let payload = StrictSnapshotObject::new(
        manifest_raw.get("payload")?,
        &[
            "project_id",
            "project_snapshot_id",
            "task_id",
            "task_revision",
            "task_spec_digest",
            "attempt_id",
            "request_id",
            "reference_id",
            "algorithm",
            "content_digest",
            "generation",
            "byte_length",
            "media_type",
            "payload_schema_id",
            "payload_schema_version",
            "bundle",
            "provenance",
            "creation_authority",
            "purpose",
            "retention_until",
        ],
    )?;
    if payload.string("algorithm")? != "sha256" {
        return Err(SnapshotParseError);
    }
    let project_id = parse_project_id(payload.string("project_id")?)?;
    let object = ArtifactObjectIdentity::new(
        ArtifactObjectKey::new(
            project_id.clone(),
            parse_digest(payload.string("content_digest")?)?,
        ),
        lattice_contracts::ArtifactGeneration::new(parse_u64(payload.string("generation")?)?)
            .map_err(|_| SnapshotParseError)?,
    );
    let binding = SubjectBinding::new(
        project_id,
        ProjectSnapshotId::new(payload.string("project_snapshot_id")?.to_owned())
            .map_err(|_| SnapshotParseError)?,
        parse_task_id(payload.string("task_id")?)?,
        payload.string("task_revision")?.to_owned(),
        parse_digest(payload.string("task_spec_digest")?)?,
    )
    .map_err(|_| SnapshotParseError)?;
    let stored_manifest_digest = parse_digest(manifest_raw.string("manifest_digest")?)?;
    let manifest = ArtifactReferenceManifest::new(
        binding,
        AttemptId::new(payload.string("attempt_id")?.to_owned()).map_err(|_| SnapshotParseError)?,
        RequestId::new(payload.string("request_id")?.to_owned()).map_err(|_| SnapshotParseError)?,
        payload.string("reference_id")?.to_owned(),
        object,
        parse_byte_length(payload.string("byte_length")?)?,
        payload.string("media_type")?.to_owned(),
        payload.string("payload_schema_id")?.to_owned(),
        payload.string("payload_schema_version")?.to_owned(),
        parse_bundle(payload.get("bundle")?)?,
        parse_provenance(payload.get("provenance")?)?,
        creation_authority,
        parse_purpose(payload.string("purpose")?)?,
        payload.string("retention_until")?.to_owned(),
        stored_manifest_digest.clone(),
    )
    .map_err(|_| SnapshotParseError)?;
    if artifact_manifest_digest(&manifest).map_err(|_| SnapshotParseError)?
        != stored_manifest_digest
    {
        return Err(SnapshotParseError);
    }
    ArtifactReferenceHead::new(
        manifest,
        parse_reference_authority_pair(raw.get("transition_authority")?)?,
        parse_revision(raw.string("revision")?)?,
        parse_reference_status(raw.string("status")?)?,
        parse_digest(raw.string("transition_digest")?)?,
    )
    .map_err(|_| SnapshotParseError)
}

pub(crate) fn parse_read_head(value: &CanonicalValue) -> SnapshotParseResult<ArtifactReadHead> {
    let raw = StrictSnapshotObject::new(
        value,
        &[
            "authority",
            "revision",
            "status",
            "holder_id",
            "acquired_at",
            "expires_at",
            "transition_digest",
        ],
    )?;
    ArtifactReadHead::new(
        parse_read_authority_pair(raw.get("authority")?)?,
        parse_revision(raw.string("revision")?)?,
        parse_read_status(raw.string("status")?)?,
        raw.string("holder_id")?.to_owned(),
        raw.string("acquired_at")?.to_owned(),
        raw.string("expires_at")?.to_owned(),
        parse_digest(raw.string("transition_digest")?)?,
    )
    .map_err(|_| SnapshotParseError)
}

pub(crate) fn parse_object_head(value: &CanonicalValue) -> SnapshotParseResult<ArtifactObjectHead> {
    let raw = StrictSnapshotObject::new(
        value,
        &[
            "object",
            "revision",
            "availability",
            "byte_length",
            "active_reference_count",
            "active_reference_set_digest",
            "sweep_not_before",
            "active_read_count",
            "active_read_set_digest",
            "delete_status",
            "delete_claim_token",
            "task_quota_projection_digest",
            "project_quota_projection_digest",
            "store_quota_projection_digest",
            "staging_quota_projection_digest",
            "command_high_water",
            "command_tail_digest",
            "transition_digest",
        ],
    )?;
    ArtifactObjectHead::new(
        parse_object_identity(raw.get("object")?)?,
        parse_revision(raw.string("revision")?)?,
        parse_availability(raw.string("availability")?)?,
        parse_byte_length(raw.string("byte_length")?)?,
        parse_counter(raw.string("active_reference_count")?)?,
        parse_digest(raw.string("active_reference_set_digest")?)?,
        raw.string("sweep_not_before")?.to_owned(),
        parse_counter(raw.string("active_read_count")?)?,
        parse_digest(raw.string("active_read_set_digest")?)?,
        parse_delete_status(raw.string("delete_status")?)?,
        parse_optional_token(raw.string("delete_claim_token")?),
        parse_digest(raw.string("task_quota_projection_digest")?)?,
        parse_digest(raw.string("project_quota_projection_digest")?)?,
        parse_digest(raw.string("store_quota_projection_digest")?)?,
        parse_digest(raw.string("staging_quota_projection_digest")?)?,
        parse_counter(raw.string("command_high_water")?)?,
        parse_digest(raw.string("command_tail_digest")?)?,
        parse_digest(raw.string("transition_digest")?)?,
    )
    .map_err(|_| SnapshotParseError)
}

pub(crate) fn parse_authority_receipt(
    value: &CanonicalValue,
) -> SnapshotParseResult<ArtifactAuthorityReceipt> {
    let raw = StrictSnapshotObject::new(
        value,
        &[
            "version",
            "producer_id",
            "producer_version",
            "runtime",
            "object",
            "reference",
            "read",
            "observation_digest",
            "receipt_digest",
        ],
    )?;
    let reference = match raw.get("reference")? {
        CanonicalValue::Null => None,
        value => Some(parse_reference_head(value)?),
    };
    let read = match raw.get("read")? {
        CanonicalValue::Null => None,
        value => Some(parse_read_head(value)?),
    };
    ArtifactAuthorityReceipt::new(
        parse_u16(raw.string("version")?)?,
        raw.string("producer_id")?.to_owned(),
        raw.string("producer_version")?.to_owned(),
        parse_runtime(raw.string("runtime")?)?,
        parse_object_head(raw.get("object")?)?,
        reference,
        read,
        parse_digest(raw.string("observation_digest")?)?,
        parse_digest(raw.string("receipt_digest")?)?,
    )
    .map_err(|_| SnapshotParseError)
}

fn parse_u16(value: &str) -> SnapshotParseResult<u16> {
    let value = parse_u64(value)?;
    u16::try_from(value).map_err(|_| SnapshotParseError)
}

pub(crate) fn parse_optional_token(value: &str) -> Option<String> {
    (value != "NONE").then(|| value.to_owned())
}
