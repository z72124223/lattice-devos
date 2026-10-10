//! Attested history boundary for an explicitly authorized Registry erasure.
use super::{
    CanonicalValue, CommandId, ContentDigest, IdentityDimension, MAX_REGISTRY_COMMANDS,
    MAX_REGISTRY_PROJECTS, MAX_REGISTRY_RETAINED_BYTES, ProjectAuthorityHead, ProjectId,
    RegistryCheckpoint, RegistryCommand, RegistryCommandOutcome, RegistryCommandRecord,
    RegistryDenial, RegistryError, RegistryIdentityReservation, RegistryProjectProjection,
    RegistryRecordSet, RepositoryObservation, UntrustedRegistrySnapshot, VerifiedRegistryState,
    apply_command_plan, build_registry_checkpoint, canonicalize, command_observation,
    command_project_id, command_result_digest, export_untrusted_registry_snapshot,
    identity_collision, plan_command, registry_digest, registry_logical_state_value,
    registry_record_set_value, registry_reservations, text_entry,
};
use std::collections::{BTreeMap, BTreeSet};

mod codec;

/// Exact permission to redact one survivor command and its complete old record set.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegistryRedactionAuthorization {
    command_id_digest: ContentDigest,
    record_set_digest: ContentDigest,
}

/// One unchanged historical command, with its original ordinal and persistence delta.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegistryHistoricalCommand {
    origin_epoch: u64,
    record: RegistryCommandRecord,
    record_set: RegistryRecordSet,
}

/// Untrusted persisted epoch payload. Only a separately trusted seal can attest it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegistryEpochBaseline {
    epoch: u64,
    source_checkpoint: RegistryCheckpoint,
    observations: BTreeMap<String, RepositoryObservation>,
    projects: BTreeMap<ProjectId, RegistryProjectProjection>,
    reservations: Vec<RegistryIdentityReservation>,
    history: BTreeMap<CommandId, RegistryHistoricalCommand>,
    redacted: BTreeSet<String>,
}

/// A new baseline made only from an already verified Registry state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegistryEpochPlan {
    baseline: RegistryEpochBaseline,
    baseline_digest: ContentDigest,
}

/// Exact command classification before any storage-side NEW-command admission.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegistryCommandLookup {
    /// This ID has never been retained or redacted in this verified lineage.
    New,
    /// The original command survives in attested history; return its old receipts.
    ExactHistorical(RegistryHistoricalCommand),
    /// The command belongs to the fully replayed current epoch tail.
    ExactTail(RegistryHistoricalCommand),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct VerifiedRegistryEpoch {
    pub(super) epoch: u64,
    pub(super) baseline_digest: ContentDigest,
    pub(super) history: BTreeMap<CommandId, RegistryHistoricalCommand>,
    pub(super) redacted: BTreeSet<String>,
}

impl RegistryRedactionAuthorization {
    /// Binds permission to one exact ID commitment and original record set.
    #[must_use]
    pub const fn new(command_id_digest: ContentDigest, record_set_digest: ContentDigest) -> Self {
        Self {
            command_id_digest,
            record_set_digest,
        }
    }

    /// Returns the exact command ID commitment covered by this permission.
    #[must_use]
    pub const fn command_id_digest(&self) -> &ContentDigest {
        &self.command_id_digest
    }

    /// Returns the original complete record-set digest covered by this permission.
    #[must_use]
    pub const fn record_set_digest(&self) -> &ContentDigest {
        &self.record_set_digest
    }
}

impl RegistryHistoricalCommand {
    /// Zero identifies a command from the original v1 history.
    #[must_use]
    pub const fn origin_epoch(&self) -> u64 {
        self.origin_epoch
    }
    /// Returns the original immutable command and semantic receipt.
    #[must_use]
    pub const fn record(&self) -> &RegistryCommandRecord {
        &self.record
    }
    /// Returns the original complete persistence delta, not a recomputed delta.
    #[must_use]
    pub const fn record_set(&self) -> &RegistryRecordSet {
        &self.record_set
    }
}

impl RegistryEpochBaseline {
    /// Returns the exact successor epoch represented by this payload.
    #[must_use]
    pub const fn epoch(&self) -> u64 {
        self.epoch
    }
    /// Returns the source state commitment; its pre-epoch history is attested.
    #[must_use]
    pub const fn source_checkpoint(&self) -> &RegistryCheckpoint {
        &self.source_checkpoint
    }
    /// Returns unchanged historical commands indexed independently of tail ordinals.
    #[must_use]
    pub const fn normal_history(&self) -> &BTreeMap<CommandId, RegistryHistoricalCommand> {
        &self.history
    }
    /// Returns domain-separated commitments, never raw erased command IDs.
    #[must_use]
    pub const fn redacted_command_ids(&self) -> &BTreeSet<String> {
        &self.redacted
    }
    /// Encodes the closed retention envelope. The output is not a trusted seal.
    #[must_use]
    pub fn to_canonical_value(&self) -> CanonicalValue {
        codec::encode(self)
    }
    /// Decodes retained data without granting it verified-state authority.
    ///
    /// # Errors
    /// Rejects malformed, duplicate, unknown, or inconsistent retained fields.
    pub fn from_canonical_value(value: &CanonicalValue) -> Result<Self, RegistryError> {
        codec::decode(value)
    }
}

impl RegistryEpochPlan {
    /// Returns the proposed persistable payload.
    #[must_use]
    pub const fn baseline(&self) -> &RegistryEpochBaseline {
        &self.baseline
    }
    /// Returns the digest which the independently retained Store seal must cover.
    #[must_use]
    pub const fn baseline_digest(&self) -> &ContentDigest {
        &self.baseline_digest
    }
    /// Returns the exact verified state from which the boundary was created.
    #[must_use]
    pub const fn source_checkpoint(&self) -> &RegistryCheckpoint {
        &self.baseline.source_checkpoint
    }
}

/// Commits an exact canonical command ID. This is an identity guard, not anonymity.
///
/// # Errors
/// Returns a canonical encoding error.
pub fn registry_command_id_commitment(id: &CommandId) -> Result<ContentDigest, RegistryError> {
    registry_digest(
        "lattice.project-registry.command-id-commitment",
        &CanonicalValue::String(id.as_str().to_owned()),
    )
}

/// Classifies an ID before a persistence adapter admits a new command.
///
/// # Errors
/// Redacted IDs always fail closed; changed requests fail with `CommandIdReuse`.
pub fn lookup_registry_command(
    base: &VerifiedRegistryState,
    command_id: &CommandId,
    request_digest: &ContentDigest,
) -> Result<RegistryCommandLookup, RegistryError> {
    if let Some(epoch) = &base.epoch {
        let commitment = registry_command_id_commitment(command_id)?;
        if epoch.redacted.contains(commitment.as_str()) {
            return Err(RegistryError::CommandRedacted);
        }
        if let Some(historical) = epoch.history.get(command_id) {
            if historical.record.receipt.request_digest() != request_digest {
                return Err(RegistryError::CommandIdReuse);
            }
            return Ok(RegistryCommandLookup::ExactHistorical(historical.clone()));
        }
    }
    if let Some(record) = base
        .commands
        .values()
        .find(|record| record.command.command_id() == command_id)
    {
        if record.receipt.request_digest() != request_digest {
            return Err(RegistryError::CommandIdReuse);
        }
        let record_set = base
            .record_sets
            .get(&record.ordinal)
            .ok_or(RegistryError::CorruptSnapshot)?
            .clone();
        return Ok(RegistryCommandLookup::ExactTail(
            RegistryHistoricalCommand {
                origin_epoch: base.epoch(),
                record: record.clone(),
                record_set,
            },
        ));
    }
    Ok(RegistryCommandLookup::New)
}

fn complete_history(
    base: &VerifiedRegistryState,
) -> Result<BTreeMap<CommandId, RegistryHistoricalCommand>, RegistryError> {
    let mut history = base
        .epoch
        .as_ref()
        .map_or_else(BTreeMap::new, |epoch| epoch.history.clone());
    for record in base.commands.values() {
        let record_set = base
            .record_sets
            .get(&record.ordinal)
            .ok_or(RegistryError::CorruptSnapshot)?
            .clone();
        let historical = RegistryHistoricalCommand {
            origin_epoch: base.epoch(),
            record: record.clone(),
            record_set,
        };
        if history
            .insert(record.command.command_id().clone(), historical)
            .is_some()
        {
            return Err(RegistryError::CorruptSnapshot);
        }
    }
    Ok(history)
}

struct EpochRemoval<'a> {
    history: BTreeMap<CommandId, RegistryHistoricalCommand>,
    scope: RemovedScope<'a>,
    projects: BTreeMap<ProjectId, RegistryProjectProjection>,
    reservations: Vec<RegistryIdentityReservation>,
}

fn prepare_epoch_removal<'a>(
    base: &VerifiedRegistryState,
    removed_project: &'a ProjectId,
) -> Result<EpochRemoval<'a>, RegistryError> {
    let target = base
        .project(removed_project)
        .ok_or(RegistryError::EpochProjectNotFound)?;
    let history = complete_history(base)?;
    let scope = RemovedScope::new(removed_project, target, &history, &base.observations);
    let projects = base
        .projects
        .iter()
        .filter(|(id, _)| *id != removed_project)
        .map(|(id, projection)| (id.clone(), projection.clone()))
        .collect::<BTreeMap<_, _>>();
    if projects
        .iter()
        .any(|(id, projection)| scope.text(id.as_str()) || scope.projection(projection))
    {
        return Err(RegistryError::EpochSurvivorReference);
    }
    let reservations = registry_reservations(&projects);
    if reservations
        .iter()
        .any(|reservation| scope.reservation(reservation))
    {
        return Err(RegistryError::EpochSurvivorReference);
    }
    Ok(EpochRemoval {
        history,
        scope,
        projects,
        reservations,
    })
}

/// Lists the exact survivor-command permissions required by an epoch removal.
/// Uses the same complete history and reference classification as the plan.
/// Target-owned commands and unrelated survivor commands are omitted. The
/// result is a read-only preview; the caller must obtain authorization before
/// supplying these entries to `plan_registry_epoch`.
///
/// # Errors
/// Rejects a missing target, corrupt retained history, surviving current
/// dependencies, or a command commitment encoding failure.
pub fn preview_required_redactions(
    base: &VerifiedRegistryState,
    target: &ProjectId,
) -> Result<Vec<RegistryRedactionAuthorization>, RegistryError> {
    let removal = prepare_epoch_removal(base, target)?;
    removal
        .history
        .iter()
        .filter(|(_, historical)| removal.scope.requires_redaction(historical))
        .map(|(id, historical)| {
            Ok(RegistryRedactionAuthorization::new(
                registry_command_id_commitment(id)?,
                historical.record_set.record_set_digest.clone(),
            ))
        })
        .collect()
}

/// Plans an attested boundary only after complete verification of its source.
/// Current survivor references cannot be redacted through command permissions.
///
/// # Errors
/// Rejects a missing target, non-successor epoch, surviving target dependencies,
/// absent or stale redaction permissions, or excessive retained evidence.
pub fn plan_registry_epoch(
    base: &VerifiedRegistryState,
    removed_project: &ProjectId,
    next_epoch: u64,
    authorized_redactions: &[RegistryRedactionAuthorization],
) -> Result<RegistryEpochPlan, RegistryError> {
    if base.epoch().checked_add(1) != Some(next_epoch) {
        return Err(RegistryError::EpochSequenceInvalid);
    }
    let EpochRemoval {
        history,
        scope,
        projects,
        reservations,
    } = prepare_epoch_removal(base, removed_project)?;
    let mut permissions = BTreeMap::new();
    for permission in authorized_redactions {
        if permissions
            .insert(
                permission.command_id_digest.as_str().to_owned(),
                permission.record_set_digest.clone(),
            )
            .is_some()
        {
            return Err(RegistryError::EpochRedactionInvalid);
        }
    }
    let mut redacted = base
        .epoch
        .as_ref()
        .map_or_else(BTreeSet::new, |epoch| epoch.redacted.clone());
    let mut retained = BTreeMap::new();
    for (id, historical) in history {
        let commitment = registry_command_id_commitment(&id)?;
        let owned = command_project_id(&historical.record.command) == removed_project;
        if owned || scope.requires_redaction(&historical) {
            if !owned {
                match permissions.remove(commitment.as_str()) {
                    None => return Err(RegistryError::EpochRedactionRequired),
                    Some(digest) if digest != historical.record_set.record_set_digest => {
                        return Err(RegistryError::EpochRedactionInvalid);
                    }
                    Some(_) => {}
                }
            }
            redacted.insert(commitment.as_str().to_owned());
        } else {
            retained.insert(id, historical);
        }
    }
    if !permissions.is_empty() {
        return Err(RegistryError::EpochRedactionInvalid);
    }
    let observations = base
        .observations
        .iter()
        .filter(|(_, observation)| !scope.observation(observation))
        .map(|(digest, observation)| (digest.clone(), observation.clone()))
        .collect();
    let baseline = RegistryEpochBaseline {
        epoch: next_epoch,
        source_checkpoint: base.checkpoint.clone(),
        observations,
        projects,
        reservations,
        history: retained,
        redacted,
    };
    validate_baseline(&baseline)?;
    let baseline_digest = baseline_digest(&baseline)?;
    Ok(RegistryEpochPlan {
        baseline,
        baseline_digest,
    })
}

fn baseline_digest(baseline: &RegistryEpochBaseline) -> Result<ContentDigest, RegistryError> {
    let value = baseline.to_canonical_value();
    if u64::try_from(canonicalize(&value)?.as_slice().len()).unwrap_or(u64::MAX)
        > MAX_REGISTRY_RETAINED_BYTES
    {
        return Err(RegistryError::CapacityExceeded);
    }
    registry_digest("lattice.project-registry.epoch-baseline", &value)
}

/// Verifies an attested baseline against an independently trusted Store seal.
///
/// The caller MUST first verify its external anchor, database identity, and
/// complete Store seal, then pass that seal's baseline commitment here. A digest
/// read beside the payload in the same database is not independent trust. This
/// attests pre-epoch history; it does not replay it or prove tail anti-rollback.
///
/// # Errors
/// Rejects wrong/missing trust, malformed content, and structural corruption.
pub fn verify_registry_epoch_baseline(
    baseline: &RegistryEpochBaseline,
    expected_epoch: u64,
    trusted_baseline_digest: &ContentDigest,
) -> Result<VerifiedRegistryState, RegistryError> {
    if expected_epoch == 0
        || baseline.epoch != expected_epoch
        || baseline_digest(baseline)? != *trusted_baseline_digest
    {
        return Err(RegistryError::EpochTrustMismatch);
    }
    validate_baseline(baseline)?;
    let epoch = VerifiedRegistryEpoch {
        epoch: baseline.epoch,
        baseline_digest: trusted_baseline_digest.clone(),
        history: baseline.history.clone(),
        redacted: baseline.redacted.clone(),
    };
    let logical = bind_logical_state(
        Some(&epoch),
        registry_logical_state_value(
            baseline.source_checkpoint.runtime,
            &baseline.observations,
            &baseline.projects,
            &[],
            &baseline.reservations,
        ),
    );
    let bytes = u64::try_from(canonicalize(&logical)?.as_slice().len())
        .map_err(|_| RegistryError::CapacityExceeded)?;
    let checkpoint = build_registry_checkpoint(
        baseline.source_checkpoint.runtime,
        0,
        baseline.observations.len(),
        baseline.projects.len(),
        0,
        baseline.reservations.len(),
        bytes,
        logical,
    )?;
    Ok(VerifiedRegistryState {
        checkpoint,
        observations: baseline.observations.clone(),
        projects: baseline.projects.clone(),
        commands: BTreeMap::new(),
        record_sets: BTreeMap::new(),
        reservations: baseline.reservations.clone(),
        epoch: Some(epoch),
    })
}

/// Replays only current-epoch commands from an independently verified baseline.
///
/// # Errors
/// Rejects a non-baseline starting state, gaps, historical replays in the tail,
/// or any retained row/checkpoint which differs from the complete replay.
pub fn verify_untrusted_registry_snapshot_from_baseline(
    base: &VerifiedRegistryState,
    snapshot: &UntrustedRegistrySnapshot,
) -> Result<VerifiedRegistryState, RegistryError> {
    if base.epoch.is_none()
        || !base.commands.is_empty()
        || !base.record_sets.is_empty()
        || base.checkpoint.command_ordinal != 0
        || base.checkpoint.command_count != 0
        || base.checkpoint.runtime != snapshot.claimed_checkpoint.runtime
    {
        return Err(RegistryError::EpochBaselineInvalid);
    }
    let mut verified = base.clone();
    for (index, record) in snapshot.rows.commands.iter().enumerate() {
        if u64::try_from(index)
            .ok()
            .and_then(|value| value.checked_add(1))
            != Some(record.ordinal)
        {
            return Err(RegistryError::CorruptSnapshot);
        }
        let plan = plan_command(&verified, record.command.clone())
            .map_err(|_| RegistryError::CorruptSnapshot)?;
        if plan.is_replay() || plan.record != *record {
            return Err(RegistryError::CorruptSnapshot);
        }
        verified = apply_command_plan(&verified, &plan)?.state;
    }
    if export_untrusted_registry_snapshot(&verified) != *snapshot {
        return Err(RegistryError::CorruptSnapshot);
    }
    Ok(verified)
}

pub(super) fn retained_command_count(base: &VerifiedRegistryState) -> usize {
    base.commands.len()
        + base
            .epoch
            .as_ref()
            .map_or(0, |epoch| epoch.history.len() + epoch.redacted.len())
}

pub(super) fn bind_logical_state(
    epoch: Option<&VerifiedRegistryEpoch>,
    logical: CanonicalValue,
) -> CanonicalValue {
    match epoch {
        None => logical,
        Some(epoch) => CanonicalValue::Object(vec![
            text_entry("schema_version", "lattice.project-registry.epoch-state.v1"),
            text_entry("epoch", &epoch.epoch.to_string()),
            text_entry("baseline_digest", epoch.baseline_digest.as_str()),
            ("logical_state".to_owned(), logical),
        ]),
    }
}

fn validate_baseline(baseline: &RegistryEpochBaseline) -> Result<(), RegistryError> {
    if baseline.epoch == 0
        || baseline.projects.len() > MAX_REGISTRY_PROJECTS
        || baseline.history.len() + baseline.redacted.len() > MAX_REGISTRY_COMMANDS
        || baseline.reservations != registry_reservations(&baseline.projects)
    {
        return Err(RegistryError::EpochBaselineInvalid);
    }
    for (digest, observation) in &baseline.observations {
        if digest != observation.digest.as_str() {
            return Err(RegistryError::EpochBaselineInvalid);
        }
    }
    let require_observation = |observation: &RepositoryObservation| -> Result<(), RegistryError> {
        if baseline.observations.get(observation.digest.as_str()) != Some(observation) {
            return Err(RegistryError::EpochBaselineInvalid);
        }
        Ok(())
    };
    let require_projection =
        |id: &ProjectId, projection: &RegistryProjectProjection| -> Result<(), RegistryError> {
            if projection.authority.project_id() != id
                || projection.authority.runtime() != baseline.source_checkpoint.runtime
                || !baseline
                    .observations
                    .contains_key(projection.authority.observation_digest().as_str())
            {
                return Err(RegistryError::EpochBaselineInvalid);
            }
            require_observation(&projection.observation)?;
            if let Some(pending) = &projection.pending_observation {
                require_observation(pending)?;
            }
            Ok(())
        };
    for (id, projection) in &baseline.projects {
        require_projection(id, projection)?;
    }
    for commitment in &baseline.redacted {
        ContentDigest::from_sha256(commitment.clone())
            .map_err(|_| RegistryError::EpochBaselineInvalid)?;
    }
    for (id, historical) in &baseline.history {
        let record = &historical.record;
        let set = &historical.record_set;
        if historical.origin_epoch >= baseline.epoch
            || id != record.command.command_id()
            || baseline
                .redacted
                .contains(registry_command_id_commitment(id)?.as_str())
            || record.ordinal == 0
            || record.command != set.command
            || record.receipt != set.receipt
            || record.ordinal != set.ordinal
            || record.base_checkpoint != set.base_checkpoint
            || record.result_checkpoint != set.result_checkpoint
            || record.record_set_digest != set.record_set_digest
            || record.command.request_digest()? != *record.receipt.request_digest()
            || record.command.command_id() != record.receipt.command_id()
        {
            return Err(RegistryError::EpochBaselineInvalid);
        }
        let receipt = &record.receipt;
        if command_result_digest(
            &receipt.command_id,
            &receipt.request_digest,
            receipt.before.as_ref(),
            receipt.after.as_ref(),
            &receipt.outcome,
            &receipt.drift,
            receipt.authority.as_ref(),
        )? != receipt.result_digest
        {
            return Err(RegistryError::EpochBaselineInvalid);
        }
        let value = registry_record_set_value(
            set.ordinal,
            &set.command,
            &set.receipt,
            &set.base_checkpoint,
            &set.result_checkpoint,
            set.new_observation.as_ref(),
            set.project_replacement.as_ref(),
            &set.reservation_deletes,
            &set.reservation_inserts,
        );
        if registry_digest("lattice.project-registry.record-set", &value)? != set.record_set_digest
        {
            return Err(RegistryError::EpochBaselineInvalid);
        }
        if let Some(observation) = command_observation(&record.command) {
            require_observation(observation)?;
        }
        if let Some(observation) = &set.new_observation {
            require_observation(observation)?;
        }
        if let Some((id, projection)) = &set.project_replacement {
            require_projection(id, projection)?;
        }
    }
    Ok(())
}

struct RemovedScope<'a> {
    project_id: &'a ProjectId,
    observations: Vec<RepositoryObservation>,
}

impl<'a> RemovedScope<'a> {
    fn new(
        id: &'a ProjectId,
        target: &RegistryProjectProjection,
        history: &BTreeMap<CommandId, RegistryHistoricalCommand>,
        observations: &BTreeMap<String, RepositoryObservation>,
    ) -> Self {
        let mut owned = vec![target.observation.clone()];
        if let Some(pending) = &target.pending_observation {
            owned.push(pending.clone());
        }
        if let Some(authority) = observations.get(target.authority.observation_digest().as_str()) {
            owned.push(authority.clone());
        }
        for historical in history
            .values()
            .filter(|item| command_project_id(&item.record.command) == id)
        {
            if let Some(observation) = command_observation(&historical.record.command) {
                owned.push(observation.clone());
            }
            if let Some(observation) = &historical.record_set.new_observation {
                owned.push(observation.clone());
            }
            if let Some((_, projection)) = &historical.record_set.project_replacement {
                owned.push(projection.observation.clone());
                if let Some(pending) = &projection.pending_observation {
                    owned.push(pending.clone());
                }
            }
        }
        Self {
            project_id: id,
            observations: owned,
        }
    }

    fn text(&self, value: &str) -> bool {
        if value.contains(self.project_id.as_str()) {
            return true;
        }
        let path = value.replace('\\', "/").to_lowercase();
        self.observations.iter().any(|observation| {
            path.contains(&observation.canonical_root.replace('\\', "/").to_lowercase())
        })
    }

    fn observation(&self, value: &RepositoryObservation) -> bool {
        self.text(value.canonical_root())
            || self.text(value.primary_branch.reference())
            || self.observations.iter().any(|target| {
                target.digest == value.digest || identity_collision(target, value).is_some()
            })
    }

    fn head(&self, value: &ProjectAuthorityHead) -> bool {
        value.project_id() == self.project_id
            || self.text(value.project_id().as_str())
            || self.text(value.primary_branch().reference())
            || self
                .observations
                .iter()
                .any(|observation| observation.digest() == value.observation_digest())
    }

    fn projection(&self, value: &RegistryProjectProjection) -> bool {
        self.observation(&value.observation)
            || value
                .pending_observation
                .as_ref()
                .is_some_and(|item| self.observation(item))
            || self.head(&value.authority.head())
    }

    fn reservation(&self, value: &RegistryIdentityReservation) -> bool {
        value.project_id == *self.project_id
            || self.text(value.project_id.as_str())
            || self
                .observations
                .iter()
                .any(|observation| match value.dimension {
                    IdentityDimension::ProjectId => false,
                    IdentityDimension::CanonicalRoot => {
                        value.identity_digest == observation.canonical_root_identity_digest
                    }
                    IdentityDimension::Repository => {
                        value.identity_digest == observation.repository_identity_digest
                    }
                    IdentityDimension::File => {
                        value.identity_digest == observation.file_identity_digest
                    }
                })
    }

    fn command(&self, command: &RegistryCommand) -> bool {
        if self.text(command.command_id().as_str()) {
            return true;
        }
        match command {
            RegistryCommand::Register {
                project_id,
                observation,
                ..
            } => self.text(project_id.as_str()) || self.observation(observation),
            RegistryCommand::Observe {
                project_id,
                expected_head,
                observation,
                ..
            }
            | RegistryCommand::Reconcile {
                project_id,
                expected_head,
                observation,
                ..
            } => {
                self.text(project_id.as_str())
                    || self.head(expected_head)
                    || self.observation(observation)
            }
            RegistryCommand::Suspend {
                project_id,
                expected_head,
                ..
            } => self.text(project_id.as_str()) || self.head(expected_head),
        }
    }

    fn outcome(&self, value: &RegistryCommandOutcome) -> bool {
        let denial = match value {
            RegistryCommandOutcome::Applied => return false,
            RegistryCommandOutcome::Denied(denial) | RegistryCommandOutcome::Blocked(denial) => {
                denial
            }
        };
        match denial {
            RegistryDenial::DuplicateIdentity {
                existing_project_id,
                ..
            } => self.text(existing_project_id.as_str()),
            RegistryDenial::UnknownProject
            | RegistryDenial::StaleHead
            | RegistryDenial::LifecycleBlocked { .. }
            | RegistryDenial::ReconciliationDecisionMismatch { .. }
            | RegistryDenial::PendingObservationMismatch
            | RegistryDenial::RevisionOverflow => false,
        }
    }

    fn historical(&self, historical: &RegistryHistoricalCommand) -> bool {
        let receipt = &historical.record.receipt;
        let set = &historical.record_set;
        self.command(&historical.record.command)
            || self.outcome(&receipt.outcome)
            || receipt.before.as_ref().is_some_and(|head| self.head(head))
            || receipt.after.as_ref().is_some_and(|head| self.head(head))
            || receipt
                .authority
                .as_ref()
                .is_some_and(|authority| self.head(&authority.head()))
            || set
                .new_observation
                .as_ref()
                .is_some_and(|observation| self.observation(observation))
            || set
                .project_replacement
                .as_ref()
                .is_some_and(|(id, projection)| {
                    self.text(id.as_str()) || self.projection(projection)
                })
            || set
                .reservation_deletes
                .iter()
                .chain(&set.reservation_inserts)
                .any(|reservation| self.reservation(reservation))
    }

    fn requires_redaction(&self, historical: &RegistryHistoricalCommand) -> bool {
        command_project_id(&historical.record.command) != self.project_id
            && self.historical(historical)
    }
}
