//! Closed-schema, context-free lifecycle metadata reconstruction.

use super::{
    ArtifactAvailability, ArtifactCounter, ArtifactDeleteStatus, ArtifactIntegratedHeadEvidence,
    ArtifactLifecycleState, ArtifactLimitKind, ArtifactReadStatus, ArtifactReferenceStatus,
    ArtifactRevision, ArtifactStoreLimits, CanonicalValue, HashMap, HashSet, ObjectRecord,
    StoredRead, build_read_head, build_reference_head, object_key_token, parse_canonical_time,
    read_terminal_token, reference_terminal_token,
};
use crate::snapshot_contract::{
    parse_authority_receipt, parse_availability, parse_delete_status, parse_optional_token,
    parse_read_head, parse_reference_head,
};
use crate::snapshot_parse::{
    SnapshotParseError, SnapshotParseResult, StrictSnapshotObject, parse_digest,
    parse_object_identity, parse_u64,
};

impl ArtifactLifecycleState {
    pub(crate) fn restore_snapshot(
        value: &CanonicalValue,
        limits: ArtifactStoreLimits,
    ) -> SnapshotParseResult<Self> {
        let raw = StrictSnapshotObject::new(
            value,
            &[
                "limits",
                "last_generations",
                "objects",
                "terminal_read_ids",
                "terminal_reference_ids",
            ],
        )?;
        verify_limits(raw.get("limits")?, limits)?;

        let mut objects = HashMap::new();
        for value in raw.array("objects")? {
            let record = parse_object_record(value)?;
            let key = object_key_token(record.identity.key());
            if objects.insert(key, record).is_some() {
                return Err(SnapshotParseError);
            }
        }

        let mut last_generations = HashMap::new();
        for value in raw.array("last_generations")? {
            let row = StrictSnapshotObject::new(value, &["object_key", "last_generation"])?;
            let generation = parse_u64(row.string("last_generation")?)?;
            if generation == 0
                || last_generations
                    .insert(row.string("object_key")?.to_owned(), generation)
                    .is_some()
            {
                return Err(SnapshotParseError);
            }
        }

        let terminal_read_ids = parse_string_set(raw.array("terminal_read_ids")?)?;
        let terminal_reference_ids = parse_string_set(raw.array("terminal_reference_ids")?)?;
        let state = Self {
            limits,
            objects,
            last_generations,
            terminal_reference_ids,
            terminal_read_ids,
        };
        state.validate_restored_invariants()?;
        if state
            .canonical_metadata_state()
            .map_err(|_| SnapshotParseError)?
            != *value
        {
            return Err(SnapshotParseError);
        }
        Ok(state)
    }

    fn validate_restored_invariants(&self) -> SnapshotParseResult<()> {
        if self.objects.len() != self.last_generations.len() {
            return Err(SnapshotParseError);
        }
        for record in self.objects.values() {
            if self
                .last_generations
                .get(&object_key_token(record.identity.key()))
                .copied()
                != Some(record.identity.generation().get())
                || record.revision == 0
            {
                return Err(SnapshotParseError);
            }
            parse_canonical_time(&record.sweep_not_before).map_err(|_| SnapshotParseError)?;
            match (
                record.availability,
                record.delete_status,
                record.delete_claim_token.is_some(),
            ) {
                (ArtifactAvailability::Available, ArtifactDeleteStatus::NotClaimed, false)
                | (ArtifactAvailability::Available, ArtifactDeleteStatus::VerifiedNoEffect, true)
                | (ArtifactAvailability::DeleteClaimed, ArtifactDeleteStatus::Claimed, true)
                | (ArtifactAvailability::Deleted, ArtifactDeleteStatus::VerifiedDeleted, true)
                | (
                    ArtifactAvailability::ReconciliationRequired,
                    ArtifactDeleteStatus::ReconciliationRequired,
                    true,
                ) => {}
                _ => return Err(SnapshotParseError),
            }
            for reference in record.references.values() {
                if reference.manifest().object() != &record.identity {
                    return Err(SnapshotParseError);
                }
                let rebuilt = build_reference_head(
                    reference.manifest().clone(),
                    reference.transition_authority().clone(),
                    reference.revision().get(),
                    reference.status(),
                )
                .map_err(|_| SnapshotParseError)?;
                if &rebuilt != reference
                    || (reference.status() == ArtifactReferenceStatus::Released
                        && !self
                            .terminal_reference_ids
                            .contains(&reference_terminal_token(reference.manifest())))
                {
                    return Err(SnapshotParseError);
                }
            }
            for read in record.reads.values() {
                if read.head.authority().receipt().binding().object() != &record.identity {
                    return Err(SnapshotParseError);
                }
                let rebuilt = build_read_head(
                    read.head.authority().clone(),
                    read.head.revision().get(),
                    read.head.status(),
                    read.head.holder_id(),
                    read.head.acquired_at(),
                    read.head.expires_at(),
                )
                .map_err(|_| SnapshotParseError)?;
                if rebuilt != read.head
                    || (read.head.status() == ArtifactReadStatus::Released
                        && !self.terminal_read_ids.contains(&read_terminal_token(
                            &record.identity,
                            read.head.authority().receipt().binding().read_claim_id(),
                        )))
                {
                    return Err(SnapshotParseError);
                }
            }
            let evidence = record
                .integrated_head_evidence
                .as_ref()
                .ok_or(SnapshotParseError)?;
            if evidence.object() != &record.identity
                || evidence.lifecycle_revision().get() != record.revision
            {
                return Err(SnapshotParseError);
            }
            let receipt = record.last_receipt.as_ref().ok_or(SnapshotParseError)?;
            if receipt.object().object() != &record.identity
                || receipt.object().revision().get() != record.revision
            {
                return Err(SnapshotParseError);
            }
        }
        self.validate_quotas().map_err(|_| SnapshotParseError)
    }
}

fn parse_object_record(value: &CanonicalValue) -> SnapshotParseResult<ObjectRecord> {
    let raw = StrictSnapshotObject::new(
        value,
        &[
            "identity",
            "byte_length",
            "bundle_total_declared_bytes",
            "revision",
            "availability",
            "delete_status",
            "delete_claim_token",
            "references",
            "reads",
            "sweep_not_before",
            "integrated_head_evidence",
            "last_receipt",
        ],
    )?;
    let identity = parse_object_identity(raw.get("identity")?)?;
    let mut references = HashMap::new();
    for value in raw.array("references")? {
        let row = StrictSnapshotObject::new(value, &["map_reference_id", "head"])?;
        let head = parse_reference_head(row.get("head")?)?;
        let reference_id = row.string("map_reference_id")?.to_owned();
        if head.manifest().reference_id() != reference_id
            || references.insert(reference_id, head).is_some()
        {
            return Err(SnapshotParseError);
        }
    }
    let mut reads = HashMap::new();
    for value in raw.array("reads")? {
        let row = StrictSnapshotObject::new(value, &["map_read_claim_id", "head"])?;
        let head = parse_read_head(row.get("head")?)?;
        let read_claim_id = row.string("map_read_claim_id")?.to_owned();
        if head.authority().receipt().binding().read_claim_id() != read_claim_id
            || reads.insert(read_claim_id, StoredRead { head }).is_some()
        {
            return Err(SnapshotParseError);
        }
    }
    let integrated_head_evidence = match raw.get("integrated_head_evidence")? {
        CanonicalValue::Null => None,
        value => Some(parse_integrated_evidence(value)?),
    };
    let last_receipt = match raw.get("last_receipt")? {
        CanonicalValue::Null => None,
        value => Some(parse_authority_receipt(value)?),
    };
    Ok(ObjectRecord {
        identity,
        byte_length: parse_u64(raw.string("byte_length")?)?,
        bundle_total_declared_bytes: parse_u64(raw.string("bundle_total_declared_bytes")?)?,
        revision: parse_u64(raw.string("revision")?)?,
        availability: parse_availability(raw.string("availability")?)?,
        delete_status: parse_delete_status(raw.string("delete_status")?)?,
        delete_claim_token: parse_optional_token(raw.string("delete_claim_token")?),
        references,
        reads,
        sweep_not_before: raw.string("sweep_not_before")?.to_owned(),
        integrated_head_evidence,
        last_receipt,
    })
}

fn parse_integrated_evidence(
    value: &CanonicalValue,
) -> SnapshotParseResult<ArtifactIntegratedHeadEvidence> {
    let raw = StrictSnapshotObject::new(
        value,
        &[
            "object",
            "lifecycle_revision",
            "task_quota_head_digest",
            "project_quota_head_digest",
            "store_quota_head_digest",
            "staging_quota_head_digest",
            "command_high_water",
            "command_tail_digest",
        ],
    )?;
    ArtifactIntegratedHeadEvidence::new(
        parse_object_identity(raw.get("object")?)?,
        ArtifactRevision::new(parse_u64(raw.string("lifecycle_revision")?)?)
            .map_err(|_| SnapshotParseError)?,
        parse_digest(raw.string("task_quota_head_digest")?)?,
        parse_digest(raw.string("project_quota_head_digest")?)?,
        parse_digest(raw.string("store_quota_head_digest")?)?,
        parse_digest(raw.string("staging_quota_head_digest")?)?,
        ArtifactCounter::new(parse_u64(raw.string("command_high_water")?)?)
            .map_err(|_| SnapshotParseError)?,
        parse_digest(raw.string("command_tail_digest")?)?,
    )
    .map_err(|_| SnapshotParseError)
}

fn parse_string_set(values: &[CanonicalValue]) -> SnapshotParseResult<HashSet<String>> {
    let mut output = HashSet::with_capacity(values.len());
    for value in values {
        let CanonicalValue::String(value) = value else {
            return Err(SnapshotParseError);
        };
        if value.is_empty() || !output.insert(value.clone()) {
            return Err(SnapshotParseError);
        }
    }
    Ok(output)
}

fn verify_limits(value: &CanonicalValue, expected: ArtifactStoreLimits) -> SnapshotParseResult<()> {
    let CanonicalValue::Object(fields) = value else {
        return Err(SnapshotParseError);
    };
    if fields.len() != ArtifactLimitKind::ALL.len() {
        return Err(SnapshotParseError);
    }
    for ((name, value), kind) in fields.iter().zip(ArtifactLimitKind::ALL) {
        let CanonicalValue::String(value) = value else {
            return Err(SnapshotParseError);
        };
        if name != kind.as_str() || parse_u64(value)? != expected.get(kind) {
            return Err(SnapshotParseError);
        }
    }
    Ok(())
}
