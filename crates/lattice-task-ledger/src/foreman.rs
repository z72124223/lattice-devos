use std::collections::{BTreeMap, BTreeSet};

use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256};
use lattice_contracts::{
    ContentDigest, ProjectId, ProjectSnapshotId, TaskId, TaskLedgerStreamHead,
    TaskLedgerStreamIdentity,
};
use lattice_foreman_state::{EpistemicReferences, ForemanSnapshot};

use super::{
    AppendCommand, CommandId, CorrelationId, LedgerAppendPlan, LedgerError, LedgerEventKind,
    VerifiedStream, plan_append,
};

pub const FOREMAN_COORDINATION_STREAM: &str = "FOREMAN_COORDINATION";
pub const FOREMAN_SNAPSHOT_EVENT: &str = "FOREMAN_SNAPSHOT_RECORDED";
pub const FOREMAN_RECORD_SCHEMA: &str = "lattice.task-ledger.foreman-record/1.0";

const FOREMAN_SPEC_DIGEST: &str =
    "7979797979797979797979797979797979797979797979797979797979797979";

/// Returns the one reserved Task Ledger identity for foreman coordination.
///
/// # Errors
///
/// Returns a shared-contract error only if the compile-time identity drifts.
pub fn foreman_coordination_identity() -> Result<TaskLedgerStreamIdentity, LedgerError> {
    Ok(TaskLedgerStreamIdentity::new(
        ProjectId::new("lattice-control")?,
        ProjectSnapshotId::new("foreman-coordination-v1")?,
        TaskId::new("TASK-FOREMAN-COORDINATION")?,
        "1",
        ContentDigest::from_sha256(FOREMAN_SPEC_DIGEST)?,
        "USD",
    )?)
}

/// Caller-supplied stable command/time metadata for one foreman append.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForemanAppendMetadata {
    command_id: CommandId,
    correlation_id: CorrelationId,
    occurred_at: String,
}

impl ForemanAppendMetadata {
    /// # Errors
    ///
    /// Rejects a non-canonical UTC timestamp.
    pub fn new(
        command_id: CommandId,
        correlation_id: CorrelationId,
        occurred_at: impl Into<String>,
    ) -> Result<Self, LedgerError> {
        let occurred_at = occurred_at.into();
        super::validate_utc_timestamp(&occurred_at)?;
        Ok(Self {
            command_id,
            correlation_id,
            occurred_at,
        })
    }

    #[must_use]
    pub const fn command_id(&self) -> &CommandId {
        &self.command_id
    }
}

/// Verified fixed-scalar child record bound to one Ledger event and command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedForemanSnapshotRecord {
    expected_head: TaskLedgerStreamHead,
    stream_id: ContentDigest,
    event_digest: ContentDigest,
    command_id: CommandId,
    request_digest: ContentDigest,
    payload_digest: ContentDigest,
    snapshot: ForemanSnapshot,
}

impl VerifiedForemanSnapshotRecord {
    #[must_use]
    pub const fn expected_head(&self) -> &TaskLedgerStreamHead {
        &self.expected_head
    }

    #[must_use]
    pub const fn event_sequence(&self) -> u64 {
        self.expected_head.sequence() + 1
    }

    #[must_use]
    pub const fn stream_id(&self) -> &ContentDigest {
        &self.stream_id
    }

    #[must_use]
    pub const fn event_digest(&self) -> &ContentDigest {
        &self.event_digest
    }

    #[must_use]
    pub const fn command_id(&self) -> &CommandId {
        &self.command_id
    }

    #[must_use]
    pub const fn request_digest(&self) -> &ContentDigest {
        &self.request_digest
    }

    #[must_use]
    pub const fn payload_digest(&self) -> &ContentDigest {
        &self.payload_digest
    }

    #[must_use]
    pub const fn snapshot(&self) -> &ForemanSnapshot {
        &self.snapshot
    }

    #[must_use]
    pub fn to_untrusted(&self) -> UntrustedForemanSnapshotRow {
        UntrustedForemanSnapshotRow {
            record_schema: FOREMAN_RECORD_SCHEMA.to_owned(),
            stream_id: self.stream_id.clone(),
            event_digest: self.event_digest.clone(),
            command_id: self.command_id.clone(),
            request_digest: self.request_digest.clone(),
            payload_digest: self.payload_digest.clone(),
            snapshot: self.snapshot.clone(),
            expected_head: self.expected_head.clone(),
        }
    }
}

/// Explicitly untrusted row returned by a persistence adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UntrustedForemanSnapshotRow {
    record_schema: String,
    stream_id: ContentDigest,
    event_digest: ContentDigest,
    command_id: CommandId,
    request_digest: ContentDigest,
    payload_digest: ContentDigest,
    snapshot: ForemanSnapshot,
    expected_head: TaskLedgerStreamHead,
}

impl UntrustedForemanSnapshotRow {
    /// Constructs one explicitly untrusted persistence row for verification.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        record_schema: impl Into<String>,
        stream_id: ContentDigest,
        event_digest: ContentDigest,
        command_id: CommandId,
        request_digest: ContentDigest,
        payload_digest: ContentDigest,
        snapshot: ForemanSnapshot,
        expected_head: TaskLedgerStreamHead,
    ) -> Self {
        Self {
            record_schema: record_schema.into(),
            stream_id,
            event_digest,
            command_id,
            request_digest,
            payload_digest,
            snapshot,
            expected_head,
        }
    }

    #[must_use]
    pub fn with_record_schema(mut self, schema: impl Into<String>) -> Self {
        self.record_schema = schema.into();
        self
    }
}

/// One combined Ledger plan plus its child row. Exact retry emits no new row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForemanSnapshotAppendPlan {
    ledger_plan: LedgerAppendPlan,
    new_record: Option<VerifiedForemanSnapshotRecord>,
}

impl ForemanSnapshotAppendPlan {
    #[must_use]
    pub const fn ledger_plan(&self) -> &LedgerAppendPlan {
        &self.ledger_plan
    }

    #[must_use]
    pub const fn new_record(&self) -> Option<&VerifiedForemanSnapshotRecord> {
        self.new_record.as_ref()
    }
}

/// Plans one reserved foreman snapshot append against verified Ledger and
/// fixed-scalar child records.
///
/// # Errors
///
/// Rejects a non-fixed stream, corrupt/missing child record, changed command,
/// unknown payload, or a generation that does not strictly advance.
pub fn plan_foreman_snapshot_append(
    current: &VerifiedStream,
    existing_records: &[VerifiedForemanSnapshotRecord],
    metadata: ForemanAppendMetadata,
    snapshot: ForemanSnapshot,
) -> Result<ForemanSnapshotAppendPlan, LedgerError> {
    ensure_fixed_stream(current)?;
    let untrusted = existing_records
        .iter()
        .map(VerifiedForemanSnapshotRecord::to_untrusted)
        .collect::<Vec<_>>();
    verify_untrusted_foreman_snapshot_rows(current, &untrusted)?;

    let payload_digest = foreman_snapshot_payload_digest(&snapshot)?;
    let retained = existing_records
        .iter()
        .find(|record| record.command_id == metadata.command_id);
    let expected_head = retained.map_or_else(
        || current.head().clone(),
        |record| record.expected_head.clone(),
    );
    let command = AppendCommand::new_verified_foreman(
        expected_head.clone(),
        metadata.command_id.clone(),
        metadata.correlation_id,
        metadata.occurred_at,
        payload_digest.clone(),
    )?;
    let ledger_plan = plan_append(current, command)?;
    if ledger_plan.is_exact_retry() {
        let retained = retained.ok_or(LedgerError::InvalidForemanSnapshot)?;
        if retained.payload_digest != payload_digest || retained.snapshot != snapshot {
            return Err(LedgerError::CommandIdReuse);
        }
        return Ok(ForemanSnapshotAppendPlan {
            ledger_plan,
            new_record: None,
        });
    }

    let latest_generation = existing_records
        .iter()
        .filter(|record| record.snapshot.worker() == snapshot.worker())
        .map(|record| record.snapshot.generation())
        .max()
        .unwrap_or(0);
    if latest_generation.checked_add(1) != Some(snapshot.generation()) {
        return Err(LedgerError::ForemanGenerationRollback);
    }
    let event = ledger_plan
        .new_event()
        .ok_or(LedgerError::InvalidForemanSnapshot)?;
    if event.kind() != LedgerEventKind::ForemanSnapshotRecorded
        || event.subject_digest() != &payload_digest
    {
        return Err(LedgerError::InvalidForemanSnapshot);
    }
    let new_record = VerifiedForemanSnapshotRecord {
        expected_head,
        stream_id: event.stream_id().clone(),
        event_digest: event.event_digest().clone(),
        command_id: event.command_id().clone(),
        request_digest: event.request_digest().clone(),
        payload_digest,
        snapshot,
    };
    Ok(ForemanSnapshotAppendPlan {
        ledger_plan,
        new_record: Some(new_record),
    })
}

/// Verifies persistence rows against the independently replayed Ledger stream.
///
/// # Errors
///
/// Unknown schemas, missing/duplicate rows, linkage/payload drift, or generation
/// rollback fail closed.
pub fn verify_untrusted_foreman_snapshot_rows(
    stream: &VerifiedStream,
    rows: &[UntrustedForemanSnapshotRow],
) -> Result<Vec<VerifiedForemanSnapshotRecord>, LedgerError> {
    ensure_fixed_stream(stream)?;
    let events = stream
        .events()
        .iter()
        .filter(|event| event.kind() == LedgerEventKind::ForemanSnapshotRecorded)
        .collect::<Vec<_>>();
    if events.len() != rows.len() {
        return Err(LedgerError::InvalidForemanSnapshot);
    }
    let mut seen_events = BTreeSet::new();
    let mut generations = BTreeMap::<String, u64>::new();
    let mut verified = Vec::with_capacity(rows.len());
    for event in events {
        let row = rows
            .iter()
            .find(|row| row.event_digest == *event.event_digest())
            .ok_or(LedgerError::InvalidForemanSnapshot)?;
        let command = stream
            .commands()
            .iter()
            .find(|record| record.request().command_id() == event.command_id())
            .ok_or(LedgerError::InvalidForemanSnapshot)?;
        if row.record_schema != FOREMAN_RECORD_SCHEMA
            || row.snapshot.schema() != "lattice.foreman-snapshot/1.0"
        {
            return Err(LedgerError::UnknownForemanSnapshotVersion);
        }
        if !seen_events.insert(row.event_digest.as_str().to_owned())
            || row.stream_id != *event.stream_id()
            || row.command_id != *event.command_id()
            || row.request_digest != *event.request_digest()
            || row.expected_head != *command.request().expected_head()
            || row.expected_head.sequence().checked_add(1) != Some(event.sequence())
            || row.payload_digest != *event.subject_digest()
            || foreman_snapshot_payload_digest(&row.snapshot)? != row.payload_digest
        {
            return Err(LedgerError::InvalidForemanSnapshot);
        }
        let prior = generations
            .entry(row.snapshot.worker().to_owned())
            .or_default();
        if prior.checked_add(1) != Some(row.snapshot.generation()) {
            return Err(LedgerError::ForemanGenerationRollback);
        }
        *prior = row.snapshot.generation();
        verified.push(VerifiedForemanSnapshotRecord {
            expected_head: row.expected_head.clone(),
            stream_id: row.stream_id.clone(),
            event_digest: row.event_digest.clone(),
            command_id: row.command_id.clone(),
            request_digest: row.request_digest.clone(),
            payload_digest: row.payload_digest.clone(),
            snapshot: row.snapshot.clone(),
        });
    }
    Ok(verified)
}

fn ensure_fixed_stream(stream: &VerifiedStream) -> Result<(), LedgerError> {
    if stream.identity() != &foreman_coordination_identity()? {
        return Err(LedgerError::InvalidForemanSnapshot);
    }
    Ok(())
}

fn foreman_snapshot_payload_digest(
    snapshot: &ForemanSnapshot,
) -> Result<ContentDigest, LedgerError> {
    let epistemic = snapshot
        .epistemic()
        .map_or(CanonicalValue::Null, epistemic_value);
    let subject = CanonicalValue::Object(vec![
        ("schema".to_owned(), text(snapshot.schema())),
        ("worker".to_owned(), text(snapshot.worker())),
        ("thread".to_owned(), text(snapshot.thread())),
        ("task".to_owned(), text(snapshot.task())),
        ("branch".to_owned(), text(snapshot.branch())),
        ("worktree".to_owned(), text(snapshot.worktree())),
        ("head".to_owned(), text(snapshot.head())),
        ("state".to_owned(), text(snapshot.state().as_str())),
        (
            "blocker".to_owned(),
            snapshot.blocker().map_or(CanonicalValue::Null, text),
        ),
        ("heartbeat".to_owned(), text(snapshot.heartbeat())),
        ("authority".to_owned(), text(snapshot.authority())),
        ("evidence".to_owned(), text(snapshot.evidence())),
        (
            "generation".to_owned(),
            text(snapshot.generation().to_string()),
        ),
        ("epistemic".to_owned(), epistemic),
    ]);
    let domain = HashDomain::new("lattice.foreman-snapshot", "1.0")?;
    Ok(ContentDigest::from_sha256(
        canonical_sha256(&domain, &subject)?.to_hex(),
    )?)
}

fn epistemic_value(value: &EpistemicReferences) -> CanonicalValue {
    CanonicalValue::Object(vec![
        ("schema".to_owned(), text(value.schema())),
        ("observedFacts".to_owned(), list(value.observed_facts())),
        ("hypotheses".to_owned(), list(value.hypotheses())),
        ("confidence".to_owned(), text(value.confidence().as_str())),
        ("unknowns".to_owned(), list(value.unknowns())),
        ("evidence".to_owned(), list(value.evidence())),
        ("counterevidence".to_owned(), list(value.counterevidence())),
        ("checkedAt".to_owned(), text(value.checked_at())),
        ("expiresAt".to_owned(), text(value.expires_at())),
        (
            "refreshTrigger".to_owned(),
            text(value.refresh_trigger().as_str()),
        ),
        ("decision".to_owned(), text(value.decision())),
        ("probe".to_owned(), text(value.probe())),
        ("falsifier".to_owned(), text(value.falsifier())),
    ])
}

fn list(values: &[String]) -> CanonicalValue {
    CanonicalValue::Array(values.iter().map(text).collect())
}

fn text(value: impl Into<String>) -> CanonicalValue {
    CanonicalValue::String(value.into())
}
