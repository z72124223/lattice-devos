//! Strict quota-head and staging-state reconstruction.

use lattice_cjson::CanonicalValue;
use lattice_contracts::{
    ARTIFACT_STORE_PRODUCER_ID, ARTIFACT_STORE_PRODUCER_VERSION, ArtifactRevision, RuntimeKind,
};

use crate::quota_owner::ArtifactQuotaHeadSet;
use crate::snapshot_contract::parse_runtime;
use crate::snapshot_parse::{
    SnapshotParseError, SnapshotParseResult, StrictSnapshotObject, parse_digest, parse_i64,
    parse_object_identity, parse_project_id, parse_task_id, parse_u64,
};
use crate::{
    ArtifactLimitKind, ArtifactQuotaHead, ArtifactQuotaProjection, ArtifactQuotaScope,
    ArtifactStagingState, ArtifactStoreIdentity, ArtifactStoreLimits,
};

pub(crate) fn parse_quota_head_set(
    value: &CanonicalValue,
    limits: ArtifactStoreLimits,
) -> SnapshotParseResult<ArtifactQuotaHeadSet> {
    let raw = StrictSnapshotObject::new(
        value,
        &["limit_snapshot_digest", "checkpoint_digest", "heads"],
    )?;
    let limit_snapshot_digest = parse_digest(raw.string("limit_snapshot_digest")?)?;
    if limit_snapshot_digest
        != limits
            .limit_snapshot_digest()
            .map_err(|_| SnapshotParseError)?
    {
        return Err(SnapshotParseError);
    }
    let mut heads = Vec::with_capacity(raw.array("heads")?.len());
    for value in raw.array("heads")? {
        heads.push(parse_quota_head(value, limits, &limit_snapshot_digest)?);
    }
    let checkpoint_digest = parse_digest(raw.string("checkpoint_digest")?)?;
    ArtifactQuotaHeadSet::restore_exact(limit_snapshot_digest, heads, &checkpoint_digest)
        .map_err(|_| SnapshotParseError)
}

fn parse_quota_head(
    value: &CanonicalValue,
    limits: ArtifactStoreLimits,
    expected_limit_digest: &lattice_contracts::ContentDigest,
) -> SnapshotParseResult<ArtifactQuotaHead> {
    let raw = StrictSnapshotObject::new(
        value,
        &[
            "scope",
            "producer_id",
            "producer_version",
            "runtime",
            "revision",
            "projection",
            "limit_snapshot_digest",
            "predecessor_head_digest",
            "transition_tail_digest",
            "head_digest",
        ],
    )?;
    if raw.string("producer_id")? != ARTIFACT_STORE_PRODUCER_ID
        || raw.string("producer_version")? != ARTIFACT_STORE_PRODUCER_VERSION
        || parse_runtime(raw.string("runtime")?)? != RuntimeKind::Fake
        || parse_digest(raw.string("limit_snapshot_digest")?)? != *expected_limit_digest
    {
        return Err(SnapshotParseError);
    }
    let transition_tail_digest = parse_digest(raw.string("transition_tail_digest")?)?;
    let head_digest = parse_digest(raw.string("head_digest")?)?;
    ArtifactQuotaHead::restore_exact(
        parse_quota_scope(raw.get("scope")?)?,
        ArtifactRevision::new(parse_u64(raw.string("revision")?)?)
            .map_err(|_| SnapshotParseError)?,
        parse_projection(raw.get("projection")?)?,
        expected_limit_digest.clone(),
        parse_digest(raw.string("predecessor_head_digest")?)?,
        &transition_tail_digest,
        &head_digest,
        limits,
    )
    .map_err(|_| SnapshotParseError)
}

fn parse_projection(value: &CanonicalValue) -> SnapshotParseResult<ArtifactQuotaProjection> {
    let CanonicalValue::Object(fields) = value else {
        return Err(SnapshotParseError);
    };
    if fields.len() != ArtifactLimitKind::ALL.len() {
        return Err(SnapshotParseError);
    }
    let mut projection = ArtifactQuotaProjection::zero();
    for ((name, value), kind) in fields.iter().zip(ArtifactLimitKind::ALL) {
        let CanonicalValue::String(value) = value else {
            return Err(SnapshotParseError);
        };
        if name != kind.as_str() {
            return Err(SnapshotParseError);
        }
        projection = projection
            .with_value(kind, parse_i64(value)?)
            .map_err(|_| SnapshotParseError)?;
    }
    Ok(projection)
}

fn parse_quota_scope(value: &CanonicalValue) -> SnapshotParseResult<ArtifactQuotaScope> {
    let CanonicalValue::Object(fields) = value else {
        return Err(SnapshotParseError);
    };
    let Some(CanonicalValue::String(scope_type)) = fields.first().map(|(_, value)| value) else {
        return Err(SnapshotParseError);
    };
    match scope_type.as_str() {
        "OBJECT" => {
            let raw = StrictSnapshotObject::new(value, &["scope_type", "object"])?;
            Ok(ArtifactQuotaScope::Object(parse_object_identity(
                raw.get("object")?,
            )?))
        }
        "TASK" => {
            let raw = StrictSnapshotObject::new(value, &["scope_type", "project_id", "task_id"])?;
            Ok(ArtifactQuotaScope::Task {
                project_id: parse_project_id(raw.string("project_id")?)?,
                task_id: parse_task_id(raw.string("task_id")?)?,
            })
        }
        "PROJECT" => {
            let raw = StrictSnapshotObject::new(value, &["scope_type", "project_id"])?;
            Ok(ArtifactQuotaScope::Project(parse_project_id(
                raw.string("project_id")?,
            )?))
        }
        "STORE" => {
            let raw = StrictSnapshotObject::new(value, &["scope_type", "store_id"])?;
            Ok(ArtifactQuotaScope::Store(
                ArtifactStoreIdentity::new(raw.string("store_id")?.to_owned())
                    .map_err(|_| SnapshotParseError)?,
            ))
        }
        _ => Err(SnapshotParseError),
    }
}

pub(crate) fn parse_staging_state(value: &str) -> SnapshotParseResult<ArtifactStagingState> {
    match value {
        "ACTIVE" => Ok(ArtifactStagingState::Active),
        "SEALED_ORPHAN" => Ok(ArtifactStagingState::SealedOrphan),
        "RECONCILIATION_REQUIRED" => Ok(ArtifactStagingState::ReconciliationRequired),
        "VERIFIED_PUBLISHED" => Ok(ArtifactStagingState::VerifiedPublished),
        "VERIFIED_CLEANED" => Ok(ArtifactStagingState::VerifiedCleaned),
        _ => Err(SnapshotParseError),
    }
}
