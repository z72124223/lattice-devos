//! Closed, duplicate-rejecting wire codec; decoding does not attest a baseline.
use super::{RegistryEpochBaseline, RegistryHistoricalCommand};
use crate::{
    CONTRACT_VERSION, CanonicalValue, CommandId, ContentDigest, GitRefIdentity, IdentityDimension,
    IdentityDrift, MAX_CANONICAL_ROOT_BYTES, MAX_REGISTRY_COMMANDS, MAX_REGISTRY_PROJECTS,
    MAX_REGISTRY_RETAINED_BYTES, ProjectAuthorityHead, ProjectAuthorityReceipt, ProjectClass,
    ProjectId, ProjectLifecycle, ProjectSnapshotId, ReconciliationDecision, RegistryCheckpoint,
    RegistryCommand, RegistryCommandOutcome, RegistryCommandReceipt, RegistryCommandRecord,
    RegistryDenial, RegistryError, RegistryIdentityReservation, RegistryProjectProjection,
    RegistryRecordSet, RegistryReservationStatus, RepositoryObservation, RuntimeKind,
    command_result_digest, normalize_nfc, registry_checkpoint_projection_value,
    registry_command_core_value, registry_digest, registry_observation_value,
    registry_project_value, registry_record_set_value, registry_reservation_value, text_entry,
};
use std::collections::{BTreeMap, BTreeSet};

const SCHEMA: &str = "lattice.project-registry.epoch-baseline.v1";
const MAX_RESERVATIONS: usize = MAX_REGISTRY_PROJECTS * 6;
type Decoded<T> = Result<T, RegistryError>;
type Observations = BTreeMap<String, RepositoryObservation>;

pub(super) fn encode(baseline: &RegistryEpochBaseline) -> CanonicalValue {
    CanonicalValue::Object(vec![
        text_entry("schema", SCHEMA),
        text_entry("epoch", &baseline.epoch.to_string()),
        (
            "source_checkpoint".to_owned(),
            registry_checkpoint_projection_value(&baseline.source_checkpoint),
        ),
        (
            "observations".to_owned(),
            CanonicalValue::Array(
                baseline
                    .observations
                    .values()
                    .map(registry_observation_value)
                    .collect(),
            ),
        ),
        (
            "projects".to_owned(),
            CanonicalValue::Array(
                baseline
                    .projects
                    .iter()
                    .map(|(id, value)| registry_project_value(id, value))
                    .collect(),
            ),
        ),
        (
            "reservations".to_owned(),
            CanonicalValue::Array(
                baseline
                    .reservations
                    .iter()
                    .map(registry_reservation_value)
                    .collect(),
            ),
        ),
        (
            "history".to_owned(),
            CanonicalValue::Array(
                baseline
                    .history
                    .values()
                    .map(|historical| {
                        CanonicalValue::Object(vec![
                            text_entry("origin_epoch", &historical.origin_epoch.to_string()),
                            ("record".to_owned(), record_value(&historical.record)),
                            (
                                "record_set".to_owned(),
                                record_set_value(&historical.record_set),
                            ),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "redacted".to_owned(),
            CanonicalValue::Array(
                baseline
                    .redacted
                    .iter()
                    .cloned()
                    .map(CanonicalValue::String)
                    .collect(),
            ),
        ),
    ])
}

fn record_value(record: &RegistryCommandRecord) -> CanonicalValue {
    CanonicalValue::Object(vec![
        (
            "core".to_owned(),
            registry_command_core_value(record.ordinal, &record.command, &record.receipt),
        ),
        (
            "base_checkpoint".to_owned(),
            registry_checkpoint_projection_value(&record.base_checkpoint),
        ),
        (
            "result_checkpoint".to_owned(),
            registry_checkpoint_projection_value(&record.result_checkpoint),
        ),
        text_entry("record_set_digest", record.record_set_digest.as_str()),
    ])
}

fn record_set_payload(value: &RegistryRecordSet) -> CanonicalValue {
    registry_record_set_value(
        value.ordinal,
        &value.command,
        &value.receipt,
        &value.base_checkpoint,
        &value.result_checkpoint,
        value.new_observation.as_ref(),
        value.project_replacement.as_ref(),
        &value.reservation_deletes,
        &value.reservation_inserts,
    )
}

fn record_set_value(value: &RegistryRecordSet) -> CanonicalValue {
    let CanonicalValue::Object(mut fields) = record_set_payload(value) else {
        unreachable!("fixed Registry record-set builder returns an object")
    };
    fields.push(text_entry(
        "record_set_digest",
        value.record_set_digest.as_str(),
    ));
    CanonicalValue::Object(fields)
}

pub(super) fn decode(value: &CanonicalValue) -> Decoded<RegistryEpochBaseline> {
    let fields = Fields::new(
        value,
        &[
            "schema",
            "epoch",
            "source_checkpoint",
            "observations",
            "projects",
            "reservations",
            "history",
            "redacted",
        ],
    )?;
    if text(fields.get("schema")?)? != SCHEMA {
        return Err(invalid());
    }
    let epoch = positive(fields.get("epoch")?)?;
    let source_checkpoint = checkpoint(fields.get("source_checkpoint")?)?;
    let mut observations = BTreeMap::new();
    for value in array(fields.get("observations")?, MAX_REGISTRY_COMMANDS)? {
        let observed = observation(value)?;
        insert_ordered(
            &mut observations,
            observed.digest().as_str().to_owned(),
            observed,
        )?;
    }
    let mut projects = BTreeMap::new();
    for value in array(fields.get("projects")?, MAX_REGISTRY_PROJECTS)? {
        let (id, projection) = project(value, &observations)?;
        insert_ordered(&mut projects, id, projection)?;
    }
    let reservations = reservations(fields.get("reservations")?, MAX_RESERVATIONS)?;
    let mut history = BTreeMap::new();
    let mut ordinals = BTreeSet::new();
    for value in array(fields.get("history")?, MAX_REGISTRY_COMMANDS)? {
        let fields = Fields::new(value, &["origin_epoch", "record", "record_set"])?;
        let origin_epoch = number(fields.get("origin_epoch")?)?;
        if origin_epoch >= epoch {
            return Err(invalid());
        }
        let record = command_record(fields.get("record")?, &observations, None)?;
        if !ordinals.insert((origin_epoch, record.ordinal)) {
            return Err(invalid());
        }
        let record_set = record_set(fields.get("record_set")?, &observations)?;
        if record.ordinal != record_set.ordinal
            || record.command != record_set.command
            || record.receipt != record_set.receipt
            || record.base_checkpoint != record_set.base_checkpoint
            || record.result_checkpoint != record_set.result_checkpoint
            || record.record_set_digest != record_set.record_set_digest
        {
            return Err(invalid());
        }
        let id = record.command.command_id().clone();
        insert_ordered(
            &mut history,
            id,
            RegistryHistoricalCommand {
                origin_epoch,
                record,
                record_set,
            },
        )?;
    }
    let mut redacted = BTreeSet::new();
    for value in array(fields.get("redacted")?, MAX_REGISTRY_COMMANDS)? {
        let value = digest(value)?.as_str().to_owned();
        if redacted.last().is_some_and(|last| last >= &value) || !redacted.insert(value) {
            return Err(invalid());
        }
    }
    if history
        .len()
        .checked_add(redacted.len())
        .is_none_or(|count| count > MAX_REGISTRY_COMMANDS)
    {
        return Err(RegistryError::CapacityExceeded);
    }
    Ok(RegistryEpochBaseline {
        epoch,
        source_checkpoint,
        observations,
        projects,
        reservations,
        history,
        redacted,
    })
}

fn invalid() -> RegistryError {
    RegistryError::EpochBaselineInvalid
}

/// Objects preserve duplicate keys in `CanonicalValue`. Never collect before checking.
struct Fields<'a>(BTreeMap<&'a str, &'a CanonicalValue>);

impl<'a> Fields<'a> {
    fn new(value: &'a CanonicalValue, names: &[&str]) -> Decoded<Self> {
        let CanonicalValue::Object(values) = value else {
            return Err(invalid());
        };
        if values.len() != names.len() {
            return Err(invalid());
        }
        let mut result = BTreeMap::new();
        for (name, value) in values {
            if normalize_nfc(name) != *name
                || !names.contains(&name.as_str())
                || result.insert(name.as_str(), value).is_some()
            {
                return Err(invalid());
            }
        }
        Ok(Self(result))
    }

    fn get(&self, name: &str) -> Decoded<&'a CanonicalValue> {
        self.0.get(name).copied().ok_or_else(invalid)
    }
}

/// Only selects a closed-union discriminator; the selected arm checks all fields.
fn discriminant<'a>(value: &'a CanonicalValue, name: &str) -> Decoded<&'a str> {
    let CanonicalValue::Object(fields) = value else {
        return Err(invalid());
    };
    if fields.len() > 16 {
        return Err(invalid());
    }
    let mut matches = fields.iter().filter(|(key, _)| key == name);
    let value = matches.next().ok_or_else(invalid)?;
    if matches.next().is_some() {
        return Err(invalid());
    }
    text(&value.1)
}

fn text(value: &CanonicalValue) -> Decoded<&str> {
    let CanonicalValue::String(value) = value else {
        return Err(invalid());
    };
    if value.len() > MAX_CANONICAL_ROOT_BYTES
        || value.contains('\0')
        || normalize_nfc(value) != *value
    {
        return Err(invalid());
    }
    Ok(value)
}

fn number(value: &CanonicalValue) -> Decoded<u64> {
    let value = text(value)?;
    if value.is_empty()
        || value.len() > 20
        || !value.bytes().all(|byte| byte.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return Err(invalid());
    }
    value.parse().map_err(|_| invalid())
}

fn positive(value: &CanonicalValue) -> Decoded<u64> {
    let value = number(value)?;
    if value == 0 {
        return Err(invalid());
    }
    Ok(value)
}

fn array(value: &CanonicalValue, maximum: usize) -> Decoded<&[CanonicalValue]> {
    let CanonicalValue::Array(values) = value else {
        return Err(invalid());
    };
    if values.len() > maximum {
        return Err(RegistryError::CapacityExceeded);
    }
    Ok(values)
}

fn optional<T>(
    value: &CanonicalValue,
    decode: impl FnOnce(&CanonicalValue) -> Decoded<T>,
) -> Decoded<Option<T>> {
    if value == &CanonicalValue::Null {
        Ok(None)
    } else {
        decode(value).map(Some)
    }
}

fn insert_ordered<K: Ord, V>(map: &mut BTreeMap<K, V>, key: K, value: V) -> Decoded<()> {
    if map.last_key_value().is_some_and(|(last, _)| last >= &key)
        || map.insert(key, value).is_some()
    {
        return Err(invalid());
    }
    Ok(())
}

fn digest(value: &CanonicalValue) -> Decoded<ContentDigest> {
    ContentDigest::from_sha256(text(value)?).map_err(RegistryError::from)
}

fn project_id(value: &CanonicalValue) -> Decoded<ProjectId> {
    ProjectId::new(text(value)?).map_err(RegistryError::from)
}

fn runtime(value: &CanonicalValue) -> Decoded<RuntimeKind> {
    match text(value)? {
        "FAKE" => Ok(RuntimeKind::Fake),
        "LIVE" => Ok(RuntimeKind::Live),
        _ => Err(invalid()),
    }
}

fn project_class(value: &CanonicalValue) -> Decoded<ProjectClass> {
    match text(value)? {
        "USER_PROJECT" => Ok(ProjectClass::UserProject),
        "LATTICE_SYSTEM" => Ok(ProjectClass::LatticeSystem),
        _ => Err(invalid()),
    }
}

fn lifecycle(value: &CanonicalValue) -> Decoded<ProjectLifecycle> {
    match text(value)? {
        "ACTIVE" => Ok(ProjectLifecycle::Active),
        "SUSPENDED" => Ok(ProjectLifecycle::Suspended),
        "RECONCILIATION_REQUIRED" => Ok(ProjectLifecycle::ReconciliationRequired),
        _ => Err(invalid()),
    }
}

fn decision(value: &CanonicalValue) -> Decoded<ReconciliationDecision> {
    match text(value)? {
        "ACCEPT_MOVE" => Ok(ReconciliationDecision::AcceptMove),
        "ACCEPT_IDENTITY_CHANGE" => Ok(ReconciliationDecision::AcceptIdentityChange),
        "REACTIVATE" => Ok(ReconciliationDecision::Reactivate),
        _ => Err(invalid()),
    }
}

fn dimension(value: &CanonicalValue) -> Decoded<IdentityDimension> {
    match text(value)? {
        "PROJECT_ID" => Ok(IdentityDimension::ProjectId),
        "CANONICAL_ROOT" => Ok(IdentityDimension::CanonicalRoot),
        "REPOSITORY" => Ok(IdentityDimension::Repository),
        "FILE" => Ok(IdentityDimension::File),
        _ => Err(invalid()),
    }
}

fn drift(value: &CanonicalValue) -> Decoded<Vec<IdentityDrift>> {
    let mut result = Vec::new();
    let mut last = None;
    for value in array(value, 5)? {
        let (rank, value) = match text(value)? {
            "CANONICAL_ROOT" => (0, IdentityDrift::CanonicalRoot),
            "REPOSITORY" => (1, IdentityDrift::Repository),
            "FILE" => (2, IdentityDrift::File),
            "PRIMARY_REF_NAME" => (3, IdentityDrift::PrimaryRefName),
            "PRIMARY_REF_STORAGE" => (4, IdentityDrift::PrimaryRefStorage),
            _ => return Err(invalid()),
        };
        if last.is_some_and(|previous| previous >= rank) {
            return Err(invalid());
        }
        last = Some(rank);
        result.push(value);
    }
    Ok(result)
}

fn checkpoint(value: &CanonicalValue) -> Decoded<RegistryCheckpoint> {
    let fields = Fields::new(
        value,
        &[
            "runtime",
            "command_ordinal",
            "observation_count",
            "project_count",
            "command_count",
            "reservation_count",
            "retained_bytes",
            "checkpoint_digest",
        ],
    )?;
    let ordinal = number(fields.get("command_ordinal")?)?;
    let observations = number(fields.get("observation_count")?)?;
    let projects = number(fields.get("project_count")?)?;
    let commands = number(fields.get("command_count")?)?;
    let reservations = number(fields.get("reservation_count")?)?;
    let bytes = number(fields.get("retained_bytes")?)?;
    if ordinal != commands
        || commands > MAX_REGISTRY_COMMANDS as u64
        || observations > MAX_REGISTRY_COMMANDS as u64
        || projects > MAX_REGISTRY_PROJECTS as u64
        || reservations > MAX_RESERVATIONS as u64
        || !(103..=MAX_REGISTRY_RETAINED_BYTES).contains(&bytes)
    {
        return Err(invalid());
    }
    Ok(RegistryCheckpoint::from_retained(
        runtime(fields.get("runtime")?)?,
        ordinal,
        observations,
        projects,
        commands,
        reservations,
        bytes,
        digest(fields.get("checkpoint_digest")?)?,
    ))
}

fn branch(fields: &Fields<'_>) -> Decoded<GitRefIdentity> {
    GitRefIdentity::new(
        text(fields.get("primary_ref")?)?,
        digest(fields.get("primary_ref_storage_identity_digest")?)?,
    )
    .map_err(RegistryError::from)
}

fn observation(value: &CanonicalValue) -> Decoded<RepositoryObservation> {
    let fields = Fields::new(
        value,
        &[
            "digest",
            "canonical_root",
            "canonical_root_identity_digest",
            "repository_identity_digest",
            "file_identity_digest",
            "primary_ref",
            "primary_ref_storage_identity_digest",
        ],
    )?;
    let observation = RepositoryObservation::new(
        text(fields.get("canonical_root")?)?,
        digest(fields.get("canonical_root_identity_digest")?)?,
        digest(fields.get("repository_identity_digest")?)?,
        digest(fields.get("file_identity_digest")?)?,
        branch(&fields)?,
    )?;
    if observation.digest() != &digest(fields.get("digest")?)? {
        return Err(invalid());
    }
    Ok(observation)
}

fn observed(value: &CanonicalValue, observations: &Observations) -> Decoded<RepositoryObservation> {
    let key = digest(value)?;
    observations.get(key.as_str()).cloned().ok_or_else(invalid)
}

fn authority_fields(fields: &Fields<'_>, version: u16) -> Decoded<ProjectAuthorityReceipt> {
    ProjectAuthorityReceipt::new(
        version,
        text(fields.get("producer_id")?)?,
        text(fields.get("producer_version")?)?,
        runtime(fields.get("runtime")?)?,
        project_id(fields.get("project_id")?)?,
        ProjectSnapshotId::new(text(fields.get("project_snapshot_id")?)?)?,
        positive(fields.get("registry_revision")?)?,
        lifecycle(fields.get("lifecycle")?)?,
        project_class(fields.get("project_class")?)?,
        branch(fields)?,
        digest(fields.get("observation_digest")?)?,
        digest(fields.get("receipt_digest")?)?,
    )
    .map_err(RegistryError::from)
}

fn authority(value: &CanonicalValue) -> Decoded<ProjectAuthorityReceipt> {
    let fields = Fields::new(
        value,
        &[
            "contract_version",
            "producer_id",
            "producer_version",
            "runtime",
            "project_id",
            "project_snapshot_id",
            "registry_revision",
            "lifecycle",
            "project_class",
            "primary_ref",
            "primary_ref_storage_identity_digest",
            "observation_digest",
            "receipt_digest",
        ],
    )?;
    let version = u16::try_from(number(fields.get("contract_version")?)?).map_err(|_| invalid())?;
    authority_fields(&fields, version)
}

fn head(value: &CanonicalValue) -> Decoded<ProjectAuthorityHead> {
    let fields = Fields::new(
        value,
        &[
            "producer_id",
            "producer_version",
            "runtime",
            "project_id",
            "project_snapshot_id",
            "registry_revision",
            "lifecycle",
            "project_class",
            "primary_ref",
            "primary_ref_storage_identity_digest",
            "observation_digest",
            "receipt_digest",
        ],
    )?;
    Ok(authority_fields(&fields, CONTRACT_VERSION)?.head())
}

fn project(
    value: &CanonicalValue,
    observations: &Observations,
) -> Decoded<(ProjectId, RegistryProjectProjection)> {
    let fields = Fields::new(
        value,
        &[
            "project_id",
            "project_class",
            "accepted_observation_digest",
            "pending_observation_digest",
            "drift",
            "authority",
        ],
    )?;
    let id = project_id(fields.get("project_id")?)?;
    let class = project_class(fields.get("project_class")?)?;
    let receipt = authority(fields.get("authority")?)?;
    if receipt.project_id() != &id || receipt.project_class() != class {
        return Err(invalid());
    }
    let projection = RegistryProjectProjection::from_retained(
        class,
        observed(fields.get("accepted_observation_digest")?, observations)?,
        optional(fields.get("pending_observation_digest")?, |value| {
            observed(value, observations)
        })?,
        drift(fields.get("drift")?)?,
        receipt,
    );
    Ok((id, projection))
}

fn reservations(
    value: &CanonicalValue,
    maximum: usize,
) -> Decoded<Vec<RegistryIdentityReservation>> {
    let mut result = Vec::new();
    let mut keys = BTreeSet::new();
    for value in array(value, maximum)? {
        let fields = Fields::new(
            value,
            &["dimension", "identity_digest", "status", "project_id"],
        )?;
        let dimension = dimension(fields.get("dimension")?)?;
        if dimension == IdentityDimension::ProjectId {
            return Err(invalid());
        }
        let status = match text(fields.get("status")?)? {
            "ACCEPTED" => RegistryReservationStatus::Accepted,
            "PENDING" => RegistryReservationStatus::Pending,
            _ => return Err(invalid()),
        };
        let identity = digest(fields.get("identity_digest")?)?;
        if !keys.insert((dimension, identity.as_str().to_owned(), status)) {
            return Err(invalid());
        }
        let value = RegistryIdentityReservation::from_retained(
            dimension,
            identity,
            status,
            project_id(fields.get("project_id")?)?,
        );
        if result.last().is_some_and(|last| last >= &value) {
            return Err(invalid());
        }
        result.push(value);
    }
    Ok(result)
}

fn command(value: &CanonicalValue, observations: &Observations) -> Decoded<RegistryCommand> {
    let action = discriminant(value, "action")?;
    let names: &[&str] = match action {
        "REGISTER" => &[
            "command_id",
            "action",
            "project_id",
            "project_class",
            "observation_digest",
        ],
        "OBSERVE" => &[
            "command_id",
            "action",
            "project_id",
            "expected_head",
            "observation_digest",
        ],
        "SUSPEND" => &[
            "command_id",
            "action",
            "project_id",
            "expected_head",
            "evidence_digest",
        ],
        "RECONCILE" => &[
            "command_id",
            "action",
            "project_id",
            "expected_head",
            "observation_digest",
            "decision",
            "evidence_digest",
        ],
        _ => return Err(invalid()),
    };
    let fields = Fields::new(value, names)?;
    let id = CommandId::new(text(fields.get("command_id")?)?)?;
    let project = project_id(fields.get("project_id")?)?;
    Ok(match action {
        "REGISTER" => RegistryCommand::register(
            id,
            project,
            project_class(fields.get("project_class")?)?,
            observed(fields.get("observation_digest")?, observations)?,
        ),
        "OBSERVE" => RegistryCommand::observe(
            id,
            project,
            head(fields.get("expected_head")?)?,
            observed(fields.get("observation_digest")?, observations)?,
        ),
        "SUSPEND" => RegistryCommand::suspend(
            id,
            project,
            head(fields.get("expected_head")?)?,
            digest(fields.get("evidence_digest")?)?,
        ),
        "RECONCILE" => RegistryCommand::reconcile(
            id,
            project,
            head(fields.get("expected_head")?)?,
            observed(fields.get("observation_digest")?, observations)?,
            decision(fields.get("decision")?)?,
            digest(fields.get("evidence_digest")?)?,
        ),
        _ => return Err(invalid()),
    })
}

fn outcome(value: &CanonicalValue) -> Decoded<RegistryCommandOutcome> {
    let status = discriminant(value, "status")?;
    if status == "APPLIED" {
        Fields::new(value, &["status"])?;
        return Ok(RegistryCommandOutcome::Applied);
    }
    if !matches!(status, "DENIED" | "BLOCKED") {
        return Err(invalid());
    }
    let reason = discriminant(value, "reason")?;
    let names: &[&str] = match reason {
        "DUPLICATE_IDENTITY" => &["status", "reason", "dimension", "existing_project_id"],
        "LIFECYCLE_BLOCKED" => &["status", "reason", "lifecycle"],
        "RECONCILIATION_DECISION_MISMATCH" => &["status", "reason", "expected", "found"],
        "UNKNOWN_PROJECT" | "STALE_HEAD" | "PENDING_OBSERVATION_MISMATCH" | "REVISION_OVERFLOW" => {
            &["status", "reason"]
        }
        _ => return Err(invalid()),
    };
    let fields = Fields::new(value, names)?;
    let denial = match reason {
        "DUPLICATE_IDENTITY" => RegistryDenial::DuplicateIdentity {
            dimension: dimension(fields.get("dimension")?)?,
            existing_project_id: project_id(fields.get("existing_project_id")?)?,
        },
        "UNKNOWN_PROJECT" => RegistryDenial::UnknownProject,
        "STALE_HEAD" => RegistryDenial::StaleHead,
        "LIFECYCLE_BLOCKED" => RegistryDenial::LifecycleBlocked {
            lifecycle: lifecycle(fields.get("lifecycle")?)?,
        },
        "RECONCILIATION_DECISION_MISMATCH" => RegistryDenial::ReconciliationDecisionMismatch {
            expected: decision(fields.get("expected")?)?,
            found: decision(fields.get("found")?)?,
        },
        "PENDING_OBSERVATION_MISMATCH" => RegistryDenial::PendingObservationMismatch,
        "REVISION_OVERFLOW" => RegistryDenial::RevisionOverflow,
        _ => return Err(invalid()),
    };
    Ok(if status == "DENIED" {
        RegistryCommandOutcome::Denied(denial)
    } else {
        RegistryCommandOutcome::Blocked(denial)
    })
}

fn receipt(value: &CanonicalValue) -> Decoded<RegistryCommandReceipt> {
    let fields = Fields::new(
        value,
        &[
            "command_id",
            "request_digest",
            "before",
            "after",
            "outcome",
            "drift",
            "authority",
            "result_digest",
        ],
    )?;
    let receipt = RegistryCommandReceipt::from_retained(
        CommandId::new(text(fields.get("command_id")?)?)?,
        digest(fields.get("request_digest")?)?,
        optional(fields.get("before")?, head)?,
        optional(fields.get("after")?, head)?,
        outcome(fields.get("outcome")?)?,
        drift(fields.get("drift")?)?,
        optional(fields.get("authority")?, authority)?,
        digest(fields.get("result_digest")?)?,
    );
    let expected = command_result_digest(
        &receipt.command_id,
        &receipt.request_digest,
        receipt.before.as_ref(),
        receipt.after.as_ref(),
        &receipt.outcome,
        &receipt.drift,
        receipt.authority.as_ref(),
    )?;
    if expected != receipt.result_digest {
        return Err(invalid());
    }
    Ok(receipt)
}

fn command_record(
    value: &CanonicalValue,
    observations: &Observations,
    external_digest: Option<&ContentDigest>,
) -> Decoded<RegistryCommandRecord> {
    let names: &[&str] = if external_digest.is_some() {
        &["core", "base_checkpoint", "result_checkpoint"]
    } else {
        &[
            "core",
            "base_checkpoint",
            "result_checkpoint",
            "record_set_digest",
        ]
    };
    let fields = Fields::new(value, names)?;
    let core = Fields::new(fields.get("core")?, &["ordinal", "request", "receipt"])?;
    let ordinal = positive(core.get("ordinal")?)?;
    let command = command(core.get("request")?, observations)?;
    let receipt = receipt(core.get("receipt")?)?;
    let base = checkpoint(fields.get("base_checkpoint")?)?;
    let result = checkpoint(fields.get("result_checkpoint")?)?;
    if command.command_id() != &receipt.command_id
        || command.request_digest()? != receipt.request_digest
        || base.command_ordinal.checked_add(1) != Some(ordinal)
        || result.command_ordinal != ordinal
        || base.runtime != result.runtime
    {
        return Err(invalid());
    }
    let record_set_digest = match external_digest {
        Some(value) => value.clone(),
        None => digest(fields.get("record_set_digest")?)?,
    };
    Ok(RegistryCommandRecord::from_retained(
        ordinal,
        command,
        receipt,
        base,
        result,
        record_set_digest,
    ))
}

fn record_set(value: &CanonicalValue, observations: &Observations) -> Decoded<RegistryRecordSet> {
    let fields = Fields::new(
        value,
        &[
            "command",
            "new_observation",
            "project_replacement",
            "reservation_deletes",
            "reservation_inserts",
            "record_set_digest",
        ],
    )?;
    let expected = digest(fields.get("record_set_digest")?)?;
    let record = command_record(fields.get("command")?, observations, Some(&expected))?;
    let new_observation = optional(fields.get("new_observation")?, observation)?;
    if new_observation
        .as_ref()
        .is_some_and(|value| observations.get(value.digest().as_str()) != Some(value))
    {
        return Err(invalid());
    }
    let record_set = RegistryRecordSet {
        ordinal: record.ordinal,
        command: record.command,
        receipt: record.receipt,
        base_checkpoint: record.base_checkpoint,
        result_checkpoint: record.result_checkpoint,
        new_observation,
        project_replacement: optional(fields.get("project_replacement")?, |value| {
            project(value, observations)
        })?,
        reservation_deletes: reservations(fields.get("reservation_deletes")?, 6)?,
        reservation_inserts: reservations(fields.get("reservation_inserts")?, 6)?,
        record_set_digest: expected,
    };
    if registry_digest(
        "lattice.project-registry.record-set",
        &record_set_payload(&record_set),
    )? != record_set.record_set_digest
    {
        return Err(invalid());
    }
    Ok(record_set)
}

#[cfg(test)]
mod tests {
    use super::{Observations, receipt, record_set, record_set_payload, record_set_value};
    use crate::{
        CanonicalValue, CommandId, ContentDigest, GitRefIdentity, IdentityDimension, IdentityDrift,
        ProjectClass, ProjectId, ProjectLifecycle, ReconciliationDecision, RegistryCommand,
        RegistryCommandOutcome, RegistryCommandReceipt, RegistryDenial, RegistryError,
        RegistryRecordSet, RepositoryObservation, RuntimeKind, VerifiedRegistryState,
        command_result_digest, plan_command, registry_command_receipt_value, registry_digest,
    };
    use lattice_cjson::canonicalize;

    fn test_digest(byte: char) -> ContentDigest {
        ContentDigest::from_sha256(byte.to_string().repeat(64)).unwrap()
    }

    fn fixture() -> (RegistryRecordSet, Observations) {
        let observation = RepositoryObservation::new(
            r"C:\synthetic\epoch-codec",
            test_digest('1'),
            test_digest('2'),
            test_digest('3'),
            GitRefIdentity::new("refs/heads/main", test_digest('4')).unwrap(),
        )
        .unwrap();
        let observations = Observations::from([(
            observation.digest().as_str().to_owned(),
            observation.clone(),
        )]);
        let base = VerifiedRegistryState::vacant(RuntimeKind::Live).unwrap();
        let plan = plan_command(
            &base,
            RegistryCommand::register(
                CommandId::new("codec-register").unwrap(),
                ProjectId::new("codec-project").unwrap(),
                ProjectClass::UserProject,
                observation,
            ),
        )
        .unwrap();
        (plan.record_set().clone(), observations)
    }

    fn synthetic_receipt(
        seed: &RegistryCommandReceipt,
        outcome: RegistryCommandOutcome,
    ) -> RegistryCommandReceipt {
        let drift = vec![
            IdentityDrift::CanonicalRoot,
            IdentityDrift::Repository,
            IdentityDrift::File,
            IdentityDrift::PrimaryRefName,
            IdentityDrift::PrimaryRefStorage,
        ];
        let digest = command_result_digest(
            seed.command_id(),
            seed.request_digest(),
            seed.after(),
            seed.after(),
            &outcome,
            &drift,
            seed.authority(),
        )
        .unwrap();
        RegistryCommandReceipt::from_retained(
            seed.command_id().clone(),
            seed.request_digest().clone(),
            seed.after().cloned(),
            seed.after().cloned(),
            outcome,
            drift,
            seed.authority().cloned(),
            digest,
        )
    }

    fn field_mut<'a>(value: &'a mut CanonicalValue, path: &[&str]) -> &'a mut CanonicalValue {
        let Some((key, remaining)) = path.split_first() else {
            return value;
        };
        let CanonicalValue::Object(fields) = value else {
            panic!("fixture path is not an object: {key}");
        };
        let (_, field) = fields.iter_mut().find(|(name, _)| name == key).unwrap();
        field_mut(field, remaining)
    }

    #[test]
    fn synthetic_receipts_roundtrip_every_terminal_outcome_and_denial_variant() {
        let (record, _) = fixture();
        let mut denials = vec![
            RegistryDenial::UnknownProject,
            RegistryDenial::StaleHead,
            RegistryDenial::PendingObservationMismatch,
            RegistryDenial::RevisionOverflow,
        ];
        denials.extend(
            [
                IdentityDimension::ProjectId,
                IdentityDimension::CanonicalRoot,
                IdentityDimension::Repository,
                IdentityDimension::File,
            ]
            .map(|dimension| RegistryDenial::DuplicateIdentity {
                dimension,
                existing_project_id: ProjectId::new("existing-owner").unwrap(),
            }),
        );
        denials.extend(
            [
                ProjectLifecycle::Active,
                ProjectLifecycle::Suspended,
                ProjectLifecycle::ReconciliationRequired,
            ]
            .map(|lifecycle| RegistryDenial::LifecycleBlocked { lifecycle }),
        );
        let decisions = [
            ReconciliationDecision::AcceptMove,
            ReconciliationDecision::AcceptIdentityChange,
            ReconciliationDecision::Reactivate,
        ];
        for expected in decisions {
            for found in decisions {
                denials.push(RegistryDenial::ReconciliationDecisionMismatch { expected, found });
            }
        }
        let outcomes = std::iter::once(RegistryCommandOutcome::Applied).chain(
            denials.into_iter().flat_map(|reason| {
                [
                    RegistryCommandOutcome::Denied(reason.clone()),
                    RegistryCommandOutcome::Blocked(reason),
                ]
            }),
        );
        for outcome in outcomes {
            let expected = synthetic_receipt(record.receipt(), outcome);
            let value = registry_command_receipt_value(&expected);
            let decoded = receipt(&value).unwrap();
            assert_eq!(decoded, expected);
            assert_eq!(
                canonicalize(&registry_command_receipt_value(&decoded)).unwrap(),
                canonicalize(&value).unwrap()
            );
        }
    }

    #[test]
    fn record_set_roundtrip_keeps_original_receipt_heads_checkpoints_and_hash() {
        let (expected, observations) = fixture();
        let value = record_set_value(&expected);
        let decoded = record_set(&value, &observations).unwrap();
        assert_eq!(decoded, expected);
        assert_eq!(
            canonicalize(&record_set_value(&decoded)).unwrap(),
            canonicalize(&value).unwrap()
        );
    }

    #[test]
    fn nested_record_set_objects_reject_unknown_and_duplicate_keys() {
        let (record, observations) = fixture();
        let paths: &[&[&str]] = &[
            &["command"],
            &["command", "core"],
            &["command", "core", "request"],
            &["command", "core", "receipt"],
            &["command", "core", "receipt", "after"],
            &["command", "core", "receipt", "authority"],
            &["command", "base_checkpoint"],
            &["command", "result_checkpoint"],
            &["new_observation"],
            &["project_replacement"],
            &["project_replacement", "authority"],
        ];
        for path in paths {
            for duplicate in [false, true] {
                let mut value = record_set_value(&record);
                let CanonicalValue::Object(fields) = field_mut(&mut value, path) else {
                    panic!("fixture object expected");
                };
                // Keep object length unchanged to exercise key membership and
                // duplicate checks, independently of the collection-length guard.
                fields[1].0 = if duplicate {
                    fields[0].0.clone()
                } else {
                    "unexpected_field".to_owned()
                };
                assert_eq!(
                    record_set(&value, &observations),
                    Err(RegistryError::EpochBaselineInvalid),
                    "accepted path {path:?}, duplicate={duplicate}"
                );
            }
        }
    }

    #[test]
    fn receipt_rejects_unknown_enum_and_extra_applied_reason() {
        let (record, _) = fixture();
        let mut value = registry_command_receipt_value(record.receipt());
        *field_mut(&mut value, &["outcome", "status"]) =
            CanonicalValue::String("FUTURE_STATUS".to_owned());
        assert_eq!(receipt(&value), Err(RegistryError::EpochBaselineInvalid));

        let mut value = registry_command_receipt_value(record.receipt());
        let CanonicalValue::Object(fields) = field_mut(&mut value, &["outcome"]) else {
            panic!("outcome object");
        };
        fields.push((
            "reason".to_owned(),
            CanonicalValue::String("UNKNOWN_PROJECT".to_owned()),
        ));
        assert_eq!(receipt(&value), Err(RegistryError::EpochBaselineInvalid));

        let denied = synthetic_receipt(
            record.receipt(),
            RegistryCommandOutcome::Denied(RegistryDenial::RevisionOverflow),
        );
        let mut value = registry_command_receipt_value(&denied);
        *field_mut(&mut value, &["outcome", "reason"]) =
            CanonicalValue::String("FUTURE_REASON".to_owned());
        assert_eq!(receipt(&value), Err(RegistryError::EpochBaselineInvalid));
    }

    #[test]
    fn receipt_digest_tamper_is_rejected_even_when_record_set_is_rehashed() {
        let (mut record, observations) = fixture();
        record.receipt.result_digest = test_digest('f');
        assert_eq!(
            receipt(&registry_command_receipt_value(record.receipt())),
            Err(RegistryError::EpochBaselineInvalid)
        );
        record.record_set_digest = registry_digest(
            "lattice.project-registry.record-set",
            &record_set_payload(&record),
        )
        .unwrap();
        assert_eq!(
            record_set(&record_set_value(&record), &observations),
            Err(RegistryError::EpochBaselineInvalid)
        );
    }

    #[test]
    fn record_set_digest_tamper_is_rejected() {
        let (record, observations) = fixture();
        let mut value = record_set_value(&record);
        *field_mut(&mut value, &["record_set_digest"]) =
            CanonicalValue::String(test_digest('f').as_str().to_owned());
        assert_eq!(
            record_set(&value, &observations),
            Err(RegistryError::EpochBaselineInvalid)
        );
    }
}
