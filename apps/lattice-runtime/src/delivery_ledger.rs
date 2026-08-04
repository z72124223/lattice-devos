//! Durable TASK-032 intent and outcome flow over the verified `PostgreSQL` ledger.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::path::Path;
use std::time::Instant;

use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256};
use lattice_contracts::{
    ContentDigest, DaemonEpoch, ProjectId, ProjectSnapshotId, RuntimeAdmissionMode, RuntimeKind,
    StoreAuthorityHead, StoreAuthorityRevision, StoreDaemonInstanceId, TaskId,
    TaskLedgerStreamIdentity,
};
use lattice_postgres_store::{MigrationTarget, PostgresTaskLedger, PostgresTaskLedgerErrorKind};
use lattice_task_ledger::{
    ActionId, ActorId, AppendCommand, CommandId, CommandOutcome, CorrelationId, Diagnostic,
    LedgerEvent, LedgerEventKind, LedgerOutcome, ReasonCode, VerifiedStream,
};
use postgres::config::SslMode;
use postgres::{Config, NoTls};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

const PROJECT_ID: &str = "task032-delivery";
const SNAPSHOT_ID: &str = "task032-delivery:snapshot:1";
const TASK_ID: &str = "TASK-032";
const CORRELATION_ID: &str = "task032-delivery-001";
const ACTION_ID: &str = "codex-delivery";
const ACTOR_ID: &str = "lattice-runtime";
const INTENT_COMMAND_ID: &str = "task032-delivery-intent-001";
const OUTCOME_COMMAND_ID: &str = "task032-delivery-outcome-001";
const APPLICATION_NAME: &str = "lattice-devos-task019";

/// User-facing durable delivery projection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryStatus {
    NotStarted,
    ReconciliationRequired,
    Completed,
    Failed,
}

impl DeliveryStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotStarted => "NOT_STARTED",
            Self::ReconciliationRequired => "RECONCILIATION_REQUIRED",
            Self::Completed => "COMPLETED",
            Self::Failed => "FAILED",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Projection {
    NotStarted,
    Pending,
    Completed,
    Failed,
    Ambiguous,
}

/// Bounded database/ledger failure without retained SQL or credentials.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryLedgerErrorKind {
    InvalidBinding,
    ConnectFailed,
    SchemaRejected,
    PhysicalStateMismatch,
    CheckpointCorrupt,
    RetainedRowCorrupt,
    PersistedIntentCorrupt,
    OutcomeAppendCorrupt,
    LedgerRejected,
    CommitOutcomeUnknown,
    ReconciliationRequired,
    EvidenceInvalid,
    DeadlineExpired,
}

/// Static delivery-ledger error safe for CLI output.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeliveryLedgerError {
    kind: DeliveryLedgerErrorKind,
}

impl DeliveryLedgerError {
    #[must_use]
    pub const fn kind(self) -> DeliveryLedgerErrorKind {
        self.kind
    }
}

impl fmt::Display for DeliveryLedgerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "delivery ledger rejected: {:?}", self.kind)
    }
}

impl Error for DeliveryLedgerError {}

/// Exact local `PostgreSQL` binding. The password is supplied separately and is
/// never retained in this value or rendered in diagnostics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliveryDatabaseBinding {
    host: String,
    port: u16,
    run_id: String,
}

impl DeliveryDatabaseBinding {
    /// Accepts only the disposable TASK-019 loopback cluster identity.
    ///
    /// # Errors
    ///
    /// Rejects a non-loopback host, the installed-service port, or a malformed
    /// disposable run identity.
    pub fn new(
        host: impl Into<String>,
        port: u16,
        run_id: impl Into<String>,
    ) -> Result<Self, DeliveryLedgerError> {
        let host = host.into();
        let run_id = run_id.into();
        if host != "127.0.0.1"
            || port == 0
            || port == 5432
            || run_id.len() != 32
            || !run_id
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(delivery_error(DeliveryLedgerErrorKind::InvalidBinding));
        }
        Ok(Self { host, port, run_id })
    }

    fn database_name(&self) -> String {
        format!("lattice_task019_{}_base", &self.run_id[..8])
    }
}

/// One verified durable task stream connection.
pub struct DeliveryLedger {
    ledger: PostgresTaskLedger,
    identity: TaskLedgerStreamIdentity,
    authority: StoreAuthorityHead,
    deadline: Instant,
}

/// Expected delivery binding durably recorded before any Codex subprocess is
/// allowed to run. Post-effect identity evidence is intentionally absent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DeliveryIntentEvidence {
    launcher_path: String,
    version: String,
    launcher_sha256: ContentDigest,
    schema_directory: String,
    codex_home: String,
    repository_path: String,
}

impl DeliveryIntentEvidence {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        launcher_path: impl Into<String>,
        version: impl Into<String>,
        launcher_sha256: impl Into<String>,
        schema_directory: impl Into<String>,
        codex_home: impl Into<String>,
        repository_path: impl Into<String>,
    ) -> Result<Self, DeliveryLedgerError> {
        let launcher_path = launcher_path.into();
        let version = version.into();
        let schema_directory = schema_directory.into();
        let codex_home = codex_home.into();
        let repository_path = repository_path.into();
        if !valid_absolute_path(&launcher_path)
            || version.is_empty()
            || !valid_absolute_path(&schema_directory)
            || !valid_absolute_path(&codex_home)
            || !valid_absolute_path(&repository_path)
        {
            return Err(evidence_error());
        }
        Ok(Self {
            launcher_path,
            version,
            launcher_sha256: ContentDigest::from_sha256(launcher_sha256.into())
                .map_err(|_| evidence_error())?,
            schema_directory,
            codex_home,
            repository_path,
        })
    }

    fn canonical_value(&self) -> CanonicalValue {
        CanonicalValue::Object(vec![
            (
                "changed_path".to_owned(),
                CanonicalValue::String("answer.txt".to_owned()),
            ),
            (
                "codex_home".to_owned(),
                CanonicalValue::String(self.codex_home.clone()),
            ),
            (
                "launcher_path".to_owned(),
                CanonicalValue::String(self.launcher_path.clone()),
            ),
            (
                "launcher_sha256".to_owned(),
                CanonicalValue::String(self.launcher_sha256.as_str().to_owned()),
            ),
            (
                "repository_path".to_owned(),
                CanonicalValue::String(self.repository_path.clone()),
            ),
            (
                "schema_directory".to_owned(),
                CanonicalValue::String(self.schema_directory.clone()),
            ),
            (
                "test_command_id".to_owned(),
                CanonicalValue::String("git-diff-no-index-exact-answer-v1".to_owned()),
            ),
            (
                "version".to_owned(),
                CanonicalValue::String(self.version.clone()),
            ),
        ])
    }
}

/// Restart-safe receipt reconstructed only from a fully validated ledger
/// intent/result pair.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliveryReceipt {
    intent_digest: ContentDigest,
    outcome_digest: ContentDigest,
    launcher_path: String,
    version: String,
    launcher_sha256: String,
    schema_bundle_sha256: String,
    schema_file_count: usize,
    thread_id: String,
    turn_id: String,
    repository_path: String,
    commit_sha: String,
    parent_sha: String,
}

impl DeliveryReceipt {
    #[must_use]
    pub fn intent_digest(&self) -> &str {
        self.intent_digest.as_str()
    }

    #[must_use]
    pub fn outcome_digest(&self) -> &str {
        self.outcome_digest.as_str()
    }

    #[must_use]
    pub fn launcher_path(&self) -> &str {
        &self.launcher_path
    }

    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    #[must_use]
    pub fn launcher_sha256(&self) -> &str {
        &self.launcher_sha256
    }

    #[must_use]
    pub fn schema_bundle_sha256(&self) -> &str {
        &self.schema_bundle_sha256
    }

    #[must_use]
    pub const fn schema_file_count(&self) -> usize {
        self.schema_file_count
    }

    #[must_use]
    pub fn thread_id(&self) -> &str {
        &self.thread_id
    }

    #[must_use]
    pub fn turn_id(&self) -> &str {
        &self.turn_id
    }

    #[must_use]
    pub fn repository_path(&self) -> &str {
        &self.repository_path
    }

    #[must_use]
    pub fn commit_sha(&self) -> &str {
        &self.commit_sha
    }

    #[must_use]
    pub fn parent_sha(&self) -> &str {
        &self.parent_sha
    }
}

/// Complete success evidence accepted by the durable outcome writer.
///
/// Its constructor is crate-private so only the composition path that already
/// verified Codex, scope, the fixed test, and Git can create this value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DeliverySuccessEvidence {
    intent_digest: ContentDigest,
    launcher_path: String,
    version: String,
    launcher_sha256: ContentDigest,
    schema_bundle_sha256: ContentDigest,
    schema_file_count: usize,
    thread_id: String,
    turn_id: String,
    repository_path: String,
    commit_sha: String,
    parent_sha: String,
}

impl DeliverySuccessEvidence {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        intent_digest: ContentDigest,
        launcher_path: impl Into<String>,
        version: impl Into<String>,
        launcher_sha256: impl Into<String>,
        schema_bundle_sha256: impl Into<String>,
        schema_file_count: usize,
        thread_id: impl Into<String>,
        turn_id: impl Into<String>,
        repository_path: impl Into<String>,
        commit_sha: impl Into<String>,
        parent_sha: impl Into<String>,
    ) -> Result<Self, DeliveryLedgerError> {
        let launcher_path = launcher_path.into();
        let version = version.into();
        let thread_id = thread_id.into();
        let turn_id = turn_id.into();
        let repository_path = repository_path.into();
        let commit_sha = commit_sha.into();
        let parent_sha = parent_sha.into();
        if !Path::new(&launcher_path).is_absolute()
            || version.is_empty()
            || schema_file_count == 0
            || thread_id.is_empty()
            || turn_id.is_empty()
            || !Path::new(&repository_path).is_absolute()
            || !is_lower_hex(&commit_sha, 40)
            || !is_lower_hex(&parent_sha, 40)
            || commit_sha == parent_sha
        {
            return Err(evidence_error());
        }
        Ok(Self {
            intent_digest,
            launcher_path,
            version,
            launcher_sha256: ContentDigest::from_sha256(launcher_sha256.into())
                .map_err(|_| evidence_error())?,
            schema_bundle_sha256: ContentDigest::from_sha256(schema_bundle_sha256.into())
                .map_err(|_| evidence_error())?,
            schema_file_count,
            thread_id,
            turn_id,
            repository_path,
            commit_sha,
            parent_sha,
        })
    }

    fn canonical_value(&self) -> CanonicalValue {
        CanonicalValue::Object(vec![
            (
                "changed_path".to_owned(),
                CanonicalValue::String("answer.txt".to_owned()),
            ),
            (
                "commit_sha".to_owned(),
                CanonicalValue::String(self.commit_sha.clone()),
            ),
            (
                "intent_digest".to_owned(),
                CanonicalValue::String(self.intent_digest.as_str().to_owned()),
            ),
            (
                "launcher_path".to_owned(),
                CanonicalValue::String(self.launcher_path.clone()),
            ),
            (
                "launcher_sha256".to_owned(),
                CanonicalValue::String(self.launcher_sha256.as_str().to_owned()),
            ),
            (
                "parent_sha".to_owned(),
                CanonicalValue::String(self.parent_sha.clone()),
            ),
            (
                "repository_path".to_owned(),
                CanonicalValue::String(self.repository_path.clone()),
            ),
            (
                "schema_bundle_sha256".to_owned(),
                CanonicalValue::String(self.schema_bundle_sha256.as_str().to_owned()),
            ),
            (
                "schema_file_count".to_owned(),
                CanonicalValue::String(self.schema_file_count.to_string()),
            ),
            (
                "test".to_owned(),
                CanonicalValue::String("FIXED_TEST_PASSED".to_owned()),
            ),
            (
                "test_command_id".to_owned(),
                CanonicalValue::String("git-diff-no-index-exact-answer-v1".to_owned()),
            ),
            (
                "thread_id".to_owned(),
                CanonicalValue::String(self.thread_id.clone()),
            ),
            (
                "turn_id".to_owned(),
                CanonicalValue::String(self.turn_id.clone()),
            ),
            (
                "version".to_owned(),
                CanonicalValue::String(self.version.clone()),
            ),
        ])
    }
}

impl DeliveryLedger {
    /// Opens a runtime-role connection and verifies the exact marker-owned schema.
    ///
    /// # Errors
    ///
    /// Rejects missing credentials, failed role binding, or any schema,
    /// authority, or retained-ledger mismatch.
    pub fn connect(
        binding: &DeliveryDatabaseBinding,
        password: &str,
        deadline: Instant,
    ) -> Result<Self, DeliveryLedgerError> {
        if password.is_empty() {
            return Err(delivery_error(DeliveryLedgerErrorKind::InvalidBinding));
        }
        let database_name = binding.database_name();
        let target = MigrationTarget::new(database_name.clone(), binding.run_id.clone())
            .map_err(|_| delivery_error(DeliveryLedgerErrorKind::InvalidBinding))?;
        let mut config = Config::new();
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(deadline_error)?;
        let statement_timeout_ms = u64::try_from(remaining.as_millis())
            .unwrap_or(u64::MAX)
            .clamp(1, u64::from(u32::MAX));
        let database_timeouts = format!(
            "-c statement_timeout={statement_timeout_ms} -c lock_timeout={statement_timeout_ms} -c idle_in_transaction_session_timeout={statement_timeout_ms}"
        );
        config
            .host(&binding.host)
            .port(binding.port)
            .user("lattice_runtime_login")
            .password(password)
            .dbname(&database_name)
            .application_name(APPLICATION_NAME)
            .connect_timeout(remaining)
            .options(&database_timeouts)
            .ssl_mode(SslMode::Disable);
        let mut client = config
            .connect(NoTls)
            .map_err(|_| delivery_error(DeliveryLedgerErrorKind::ConnectFailed))?;
        client
            .batch_execute("SET ROLE lattice_runtime")
            .map_err(|_| delivery_error(DeliveryLedgerErrorKind::ConnectFailed))?;
        let ledger = PostgresTaskLedger::new(client, &target).map_err(map_ledger_error)?;
        ensure_before_deadline(deadline)?;
        Ok(Self {
            ledger,
            identity: delivery_identity()?,
            authority: delivery_authority()?,
            deadline,
        })
    }

    /// Reloads and projects the durable stream without trusting caller state.
    ///
    /// # Errors
    ///
    /// Returns a bounded error when the durable stream cannot be verified.
    pub fn status(&mut self) -> Result<DeliveryStatus, DeliveryLedgerError> {
        self.ensure_before_deadline()?;
        let loaded = self
            .ledger
            .load_stream(self.identity.clone())
            .map_err(map_ledger_error)?;
        self.ensure_before_deadline()?;
        Ok(public_status(inspect_stream(loaded.stream()).projection))
    }

    /// Reconstructs the completed receipt from validated `PostgreSQL` evidence.
    ///
    /// # Errors
    ///
    /// Rejects pending, failed, incomplete, or structurally ambiguous streams.
    pub fn receipt(&mut self) -> Result<DeliveryReceipt, DeliveryLedgerError> {
        self.ensure_before_deadline()?;
        let loaded = self
            .ledger
            .load_stream(self.identity.clone())
            .map_err(map_ledger_error)?;
        self.ensure_before_deadline()?;
        let inspected = inspect_stream(loaded.stream());
        if inspected.projection != Projection::Completed {
            return Err(delivery_error(
                DeliveryLedgerErrorKind::ReconciliationRequired,
            ));
        }
        inspected
            .receipt
            .ok_or_else(|| delivery_error(DeliveryLedgerErrorKind::ReconciliationRequired))
    }

    /// Commits the exact effect intent before Codex or Git can mutate a repo.
    ///
    /// # Errors
    ///
    /// Rejects non-canonical evidence, repeated work, authority drift, or an
    /// intent commit that cannot be proved durable.
    pub(crate) fn record_intent(
        &mut self,
        evidence: &DeliveryIntentEvidence,
    ) -> Result<ContentDigest, DeliveryLedgerError> {
        self.ensure_before_deadline()?;
        let loaded = self
            .ledger
            .load_stream(self.identity.clone())
            .map_err(map_ledger_error)?;
        self.ensure_before_deadline()?;
        if inspect_stream(loaded.stream()).projection != Projection::NotStarted {
            return Err(delivery_error(
                DeliveryLedgerErrorKind::ReconciliationRequired,
            ));
        }
        let canonical = evidence.canonical_value();
        let subject_digest = delivery_digest("lattice.runtime.codex-delivery-intent", &canonical)?;
        let command = append_command(
            loaded.stream().head().clone(),
            INTENT_COMMAND_ID,
            LedgerEventKind::EffectIntent,
            LedgerOutcome::Recorded,
            "TASK032_CODEX_INTENT",
            subject_digest.clone(),
            canonical,
        )?;
        let execution = self
            .ledger
            .execute(command, self.authority.clone())
            .map_err(map_ledger_error)?;
        self.ensure_before_deadline()?;
        if execution.receipt().outcome() != &CommandOutcome::Appended
            || execution
                .outbox_admission()
                .is_none_or(|outbox| outbox.intent_digest() != &subject_digest)
        {
            return Err(delivery_error(DeliveryLedgerErrorKind::LedgerRejected));
        }
        Ok(subject_digest)
    }

    /// Records a verified success. Unknown or incomplete effects remain pending.
    pub(crate) fn record_success(
        &mut self,
        evidence: &DeliverySuccessEvidence,
    ) -> Result<ContentDigest, DeliveryLedgerError> {
        self.ensure_before_deadline()?;
        let loaded = self
            .ledger
            .load_stream(self.identity.clone())
            .map_err(map_outcome_load_error)?;
        self.ensure_before_deadline()?;
        if inspect_stream(loaded.stream()).projection != Projection::Pending
            || loaded.stream().events()[0].subject_digest() != &evidence.intent_digest
        {
            return Err(delivery_error(
                DeliveryLedgerErrorKind::ReconciliationRequired,
            ));
        }
        let canonical = evidence.canonical_value();
        let subject_digest = delivery_digest("lattice.runtime.codex-delivery-result", &canonical)?;
        let command = append_command(
            loaded.stream().head().clone(),
            OUTCOME_COMMAND_ID,
            LedgerEventKind::EffectOutcome,
            LedgerOutcome::Passed,
            "TASK032_DELIVERY_COMPLETED",
            subject_digest.clone(),
            canonical,
        )?;
        let execution = self
            .ledger
            .execute(command, self.authority.clone())
            .map_err(map_outcome_append_error)?;
        self.ensure_before_deadline()?;
        if execution.receipt().outcome() != &CommandOutcome::Appended {
            return Err(delivery_error(DeliveryLedgerErrorKind::LedgerRejected));
        }
        Ok(subject_digest)
    }

    fn ensure_before_deadline(&self) -> Result<(), DeliveryLedgerError> {
        ensure_before_deadline(self.deadline)
    }
}

fn append_command(
    head: lattice_contracts::TaskLedgerStreamHead,
    command_id: &str,
    kind: LedgerEventKind,
    outcome: LedgerOutcome,
    reason: &str,
    subject_digest: ContentDigest,
    diagnostic: CanonicalValue,
) -> Result<AppendCommand, DeliveryLedgerError> {
    AppendCommand::new(
        head,
        CommandId::new(command_id).map_err(|_| evidence_error())?,
        CorrelationId::new(CORRELATION_ID).map_err(|_| evidence_error())?,
        now_timestamp()?,
        kind,
        ActorId::new(ACTOR_ID).map_err(|_| evidence_error())?,
        ActionId::new(ACTION_ID).map_err(|_| evidence_error())?,
        outcome,
        ReasonCode::new(reason).map_err(|_| evidence_error())?,
        subject_digest,
        Some(Diagnostic::new(diagnostic).map_err(|_| evidence_error())?),
        None,
    )
    .map_err(|_| evidence_error())
}

fn delivery_identity() -> Result<TaskLedgerStreamIdentity, DeliveryLedgerError> {
    let spec_digest = delivery_digest(
        "lattice.runtime.codex-delivery-task",
        &CanonicalValue::Object(vec![
            (
                "project_id".to_owned(),
                CanonicalValue::String(PROJECT_ID.to_owned()),
            ),
            (
                "snapshot_id".to_owned(),
                CanonicalValue::String(SNAPSHOT_ID.to_owned()),
            ),
            (
                "task_id".to_owned(),
                CanonicalValue::String(TASK_ID.to_owned()),
            ),
        ]),
    )?;
    TaskLedgerStreamIdentity::new(
        ProjectId::new(PROJECT_ID).map_err(|_| evidence_error())?,
        ProjectSnapshotId::new(SNAPSHOT_ID).map_err(|_| evidence_error())?,
        TaskId::new(TASK_ID).map_err(|_| evidence_error())?,
        "1",
        spec_digest,
        "TWD",
    )
    .map_err(|_| evidence_error())
}

fn delivery_authority() -> Result<StoreAuthorityHead, DeliveryLedgerError> {
    StoreAuthorityHead::new(
        RuntimeKind::Live,
        StoreDaemonInstanceId::new("daemon-live-1").map_err(|_| evidence_error())?,
        DaemonEpoch::new(7).map_err(|_| evidence_error())?,
        RuntimeAdmissionMode::Active,
        StoreAuthorityRevision::new(3).map_err(|_| evidence_error())?,
        ContentDigest::from_sha256("a".repeat(64)).map_err(|_| evidence_error())?,
        ContentDigest::from_sha256("b".repeat(64)).map_err(|_| evidence_error())?,
    )
    .map_err(|_| evidence_error())
}

fn delivery_digest(
    schema_id: &str,
    value: &CanonicalValue,
) -> Result<ContentDigest, DeliveryLedgerError> {
    let domain = HashDomain::new(schema_id, "1.0").map_err(|_| evidence_error())?;
    let digest = canonical_sha256(&domain, value).map_err(|_| evidence_error())?;
    ContentDigest::from_sha256(digest.to_hex()).map_err(|_| evidence_error())
}

fn now_timestamp() -> Result<String, DeliveryLedgerError> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|_| evidence_error())
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

struct StreamInspection {
    projection: Projection,
    receipt: Option<DeliveryReceipt>,
}

const fn inspection(projection: Projection) -> StreamInspection {
    StreamInspection {
        projection,
        receipt: None,
    }
}

fn inspect_stream(stream: &VerifiedStream) -> StreamInspection {
    if stream.events().is_empty() && stream.commands().is_empty() && stream.outboxes().is_empty() {
        return inspection(Projection::NotStarted);
    }
    if !matches!(stream.events().len(), 1 | 2)
        || stream.commands().len() != stream.events().len()
        || stream.outboxes().len() != 1
    {
        return inspection(Projection::Ambiguous);
    }

    let intent_event = &stream.events()[0];
    let Some(intent_command) = stream
        .commands()
        .iter()
        .find(|command| command.request().command_id().as_str() == INTENT_COMMAND_ID)
    else {
        return inspection(Projection::Ambiguous);
    };
    let outbox = &stream.outboxes()[0];
    let Some(intent_evidence) = validate_intent_evidence(
        intent_event.diagnostic().map(Diagnostic::value),
        intent_event.subject_digest(),
    ) else {
        return inspection(Projection::Ambiguous);
    };
    if !valid_command_event(
        intent_command.request(),
        intent_command.receipt(),
        intent_event,
        INTENT_COMMAND_ID,
        LedgerEventKind::EffectIntent,
        LedgerOutcome::Recorded,
        "TASK032_CODEX_INTENT",
        1,
    ) || outbox.command_id().as_str() != INTENT_COMMAND_ID
        || outbox.event_sequence() != intent_event.sequence()
        || outbox.event_digest() != intent_event.event_digest()
        || outbox.request_digest() != intent_event.request_digest()
        || outbox.intent_digest() != intent_event.subject_digest()
    {
        return inspection(Projection::Ambiguous);
    }
    if stream.events().len() == 1 {
        return inspection(Projection::Pending);
    }

    let outcome_event = &stream.events()[1];
    let Some(outcome_command) = stream
        .commands()
        .iter()
        .find(|command| command.request().command_id().as_str() == OUTCOME_COMMAND_ID)
    else {
        return inspection(Projection::Ambiguous);
    };
    if outcome_command.receipt().before() != intent_command.receipt().after()
        || outcome_command.receipt().after() != stream.head()
        || !valid_command_event(
            outcome_command.request(),
            outcome_command.receipt(),
            outcome_event,
            OUTCOME_COMMAND_ID,
            LedgerEventKind::EffectOutcome,
            outcome_event.outcome(),
            outcome_event.reason_code().as_str(),
            2,
        )
    {
        return inspection(Projection::Ambiguous);
    }
    let Some(receipt) = validate_result_evidence(
        outcome_event.diagnostic().map(Diagnostic::value),
        outcome_event.subject_digest(),
        intent_event.subject_digest(),
        &intent_evidence,
    ) else {
        return inspection(Projection::Ambiguous);
    };
    match (
        outcome_event.outcome(),
        outcome_event.reason_code().as_str(),
    ) {
        (LedgerOutcome::Passed, "TASK032_DELIVERY_COMPLETED") => StreamInspection {
            projection: Projection::Completed,
            receipt: Some(receipt),
        },
        (LedgerOutcome::Failed, "TASK032_DELIVERY_FAILED")
        | (LedgerOutcome::Cancelled, "TASK032_DELIVERY_CANCELLED") => {
            inspection(Projection::Failed)
        }
        _ => inspection(Projection::Ambiguous),
    }
}

#[allow(clippy::too_many_arguments)]
fn valid_command_event(
    request: &AppendCommand,
    receipt: &lattice_task_ledger::CommandReceipt,
    event: &LedgerEvent,
    command_id: &str,
    kind: LedgerEventKind,
    outcome: LedgerOutcome,
    reason: &str,
    sequence: u64,
) -> bool {
    request.command_id().as_str() == command_id
        && request.correlation_id().as_str() == CORRELATION_ID
        && request.actor_id().as_str() == ACTOR_ID
        && request.action().as_str() == ACTION_ID
        && request.kind() == kind
        && request.outcome() == outcome
        && request.reason_code().as_str() == reason
        && receipt.outcome() == &CommandOutcome::Appended
        && receipt.event_digest() == Some(event.event_digest())
        && event.sequence() == sequence
        && event.command_id().as_str() == command_id
        && event.correlation_id().as_str() == CORRELATION_ID
        && event.actor_id().as_str() == ACTOR_ID
        && event.action().as_str() == ACTION_ID
        && event.kind() == kind
        && event.outcome() == outcome
        && event.reason_code().as_str() == reason
        && event.request_digest() == receipt.request_digest()
        && event.subject_digest() == request.subject_digest()
        && request.diagnostic().map(Diagnostic::value) == event.diagnostic().map(Diagnostic::value)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct IntentEvidenceProjection {
    launcher_path: String,
    launcher_sha256: String,
    repository_path: String,
    test_command_id: String,
    version: String,
}

fn validate_intent_evidence(
    value: Option<&CanonicalValue>,
    subject_digest: &ContentDigest,
) -> Option<IntentEvidenceProjection> {
    let value = value?;
    let fields = string_fields(value, 8)?;
    if fields.get("changed_path")? != &"answer.txt"
        || fields.get("test_command_id")? != &"git-diff-no-index-exact-answer-v1"
        || !valid_absolute_path(fields.get("codex_home")?)
        || !valid_absolute_path(fields.get("launcher_path")?)
        || !valid_absolute_path(fields.get("repository_path")?)
        || !valid_absolute_path(fields.get("schema_directory")?)
        || !is_lower_hex(fields.get("launcher_sha256")?, 64)
        || fields.get("version")?.is_empty()
        || delivery_digest("lattice.runtime.codex-delivery-intent", value)
            .ok()
            .as_ref()
            != Some(subject_digest)
    {
        return None;
    }
    Some(IntentEvidenceProjection {
        launcher_path: (*fields.get("launcher_path")?).to_owned(),
        launcher_sha256: (*fields.get("launcher_sha256")?).to_owned(),
        repository_path: (*fields.get("repository_path")?).to_owned(),
        test_command_id: (*fields.get("test_command_id")?).to_owned(),
        version: (*fields.get("version")?).to_owned(),
    })
}

fn validate_result_evidence(
    value: Option<&CanonicalValue>,
    subject_digest: &ContentDigest,
    intent_digest: &ContentDigest,
    intent: &IntentEvidenceProjection,
) -> Option<DeliveryReceipt> {
    let value = value?;
    let fields = string_fields(value, 14)?;
    let valid = fields.get("changed_path") == Some(&"answer.txt")
        && fields.get("test") == Some(&"FIXED_TEST_PASSED")
        && fields.get("test_command_id") == Some(&intent.test_command_id.as_str())
        && fields.get("intent_digest") == Some(&intent_digest.as_str())
        && fields.get("launcher_path") == Some(&intent.launcher_path.as_str())
        && fields.get("launcher_sha256") == Some(&intent.launcher_sha256.as_str())
        && fields.get("repository_path") == Some(&intent.repository_path.as_str())
        && fields.get("version") == Some(&intent.version.as_str())
        && fields
            .get("schema_bundle_sha256")
            .is_some_and(|value| is_lower_hex(value, 64))
        && fields
            .get("schema_file_count")
            .is_some_and(|value| canonical_positive_usize(value))
        && fields
            .get("thread_id")
            .is_some_and(|value| !value.is_empty())
        && fields.get("turn_id").is_some_and(|value| !value.is_empty())
        && fields
            .get("commit_sha")
            .is_some_and(|value| is_lower_hex(value, 40))
        && fields
            .get("parent_sha")
            .is_some_and(|value| is_lower_hex(value, 40))
        && fields.get("commit_sha") != fields.get("parent_sha")
        && delivery_digest("lattice.runtime.codex-delivery-result", value)
            .ok()
            .as_ref()
            == Some(subject_digest);
    if !valid {
        return None;
    }
    Some(DeliveryReceipt {
        intent_digest: intent_digest.clone(),
        outcome_digest: subject_digest.clone(),
        launcher_path: (*fields.get("launcher_path")?).to_owned(),
        version: (*fields.get("version")?).to_owned(),
        launcher_sha256: (*fields.get("launcher_sha256")?).to_owned(),
        schema_bundle_sha256: (*fields.get("schema_bundle_sha256")?).to_owned(),
        schema_file_count: fields.get("schema_file_count")?.parse().ok()?,
        thread_id: (*fields.get("thread_id")?).to_owned(),
        turn_id: (*fields.get("turn_id")?).to_owned(),
        repository_path: (*fields.get("repository_path")?).to_owned(),
        commit_sha: (*fields.get("commit_sha")?).to_owned(),
        parent_sha: (*fields.get("parent_sha")?).to_owned(),
    })
}

fn string_fields(value: &CanonicalValue, expected: usize) -> Option<BTreeMap<&str, &str>> {
    let CanonicalValue::Object(entries) = value else {
        return None;
    };
    if entries.len() != expected {
        return None;
    }
    let mut fields = BTreeMap::new();
    for (key, value) in entries {
        let CanonicalValue::String(value) = value else {
            return None;
        };
        if fields.insert(key.as_str(), value.as_str()).is_some() {
            return None;
        }
    }
    Some(fields)
}

fn valid_absolute_path(value: &str) -> bool {
    !value.is_empty() && Path::new(value).is_absolute()
}

fn canonical_positive_usize(value: &str) -> bool {
    value
        .parse::<usize>()
        .ok()
        .is_some_and(|parsed| parsed > 0 && parsed.to_string() == value)
}

const fn public_status(projection: Projection) -> DeliveryStatus {
    match projection {
        Projection::NotStarted => DeliveryStatus::NotStarted,
        Projection::Pending | Projection::Ambiguous => DeliveryStatus::ReconciliationRequired,
        Projection::Completed => DeliveryStatus::Completed,
        Projection::Failed => DeliveryStatus::Failed,
    }
}

fn map_ledger_error(error: lattice_postgres_store::PostgresTaskLedgerError) -> DeliveryLedgerError {
    let kind = match error.kind() {
        PostgresTaskLedgerErrorKind::CommitOutcomeUnknown => {
            DeliveryLedgerErrorKind::CommitOutcomeUnknown
        }
        PostgresTaskLedgerErrorKind::PhysicalStateMismatch => {
            DeliveryLedgerErrorKind::PhysicalStateMismatch
        }
        PostgresTaskLedgerErrorKind::CheckpointCorrupt => {
            DeliveryLedgerErrorKind::CheckpointCorrupt
        }
        PostgresTaskLedgerErrorKind::RetainedRowCorrupt => {
            DeliveryLedgerErrorKind::RetainedRowCorrupt
        }
        _ => DeliveryLedgerErrorKind::LedgerRejected,
    };
    delivery_error(kind)
}

fn map_outcome_load_error(
    error: lattice_postgres_store::PostgresTaskLedgerError,
) -> DeliveryLedgerError {
    if error.kind() == PostgresTaskLedgerErrorKind::RetainedRowCorrupt {
        delivery_error(DeliveryLedgerErrorKind::PersistedIntentCorrupt)
    } else {
        map_ledger_error(error)
    }
}

fn map_outcome_append_error(
    error: lattice_postgres_store::PostgresTaskLedgerError,
) -> DeliveryLedgerError {
    if error.kind() == PostgresTaskLedgerErrorKind::RetainedRowCorrupt {
        delivery_error(DeliveryLedgerErrorKind::OutcomeAppendCorrupt)
    } else {
        map_ledger_error(error)
    }
}

const fn delivery_error(kind: DeliveryLedgerErrorKind) -> DeliveryLedgerError {
    DeliveryLedgerError { kind }
}

const fn evidence_error() -> DeliveryLedgerError {
    delivery_error(DeliveryLedgerErrorKind::EvidenceInvalid)
}

const fn deadline_error() -> DeliveryLedgerError {
    delivery_error(DeliveryLedgerErrorKind::DeadlineExpired)
}

fn ensure_before_deadline(deadline: Instant) -> Result<(), DeliveryLedgerError> {
    if Instant::now() < deadline {
        Ok(())
    } else {
        Err(deadline_error())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_only_the_marker_owned_loopback_binding() {
        assert!(DeliveryDatabaseBinding::new("127.0.0.1", 55432, "a".repeat(32)).is_ok());
        for (host, port, run_id) in [
            ("localhost", 55432, "a".repeat(32)),
            ("127.0.0.1", 5432, "a".repeat(32)),
            ("127.0.0.1", 55432, "A".repeat(32)),
        ] {
            assert_eq!(
                DeliveryDatabaseBinding::new(host, port, run_id)
                    .expect_err("binding must fail")
                    .kind(),
                DeliveryLedgerErrorKind::InvalidBinding
            );
        }
    }

    #[test]
    fn public_projection_fails_closed_for_pending_or_ambiguous_work() {
        assert_eq!(
            public_status(Projection::NotStarted),
            DeliveryStatus::NotStarted
        );
        assert_eq!(
            public_status(Projection::Pending),
            DeliveryStatus::ReconciliationRequired
        );
        assert_eq!(
            public_status(Projection::Ambiguous),
            DeliveryStatus::ReconciliationRequired
        );
        assert_eq!(
            public_status(Projection::Completed),
            DeliveryStatus::Completed
        );
        assert_eq!(public_status(Projection::Failed), DeliveryStatus::Failed);
    }

    #[test]
    fn success_evidence_rejects_empty_or_noncanonical_fields() {
        let intent = ContentDigest::from_sha256("c".repeat(64)).expect("intent");
        assert!(
            DeliverySuccessEvidence::new(
                intent.clone(),
                r"C:\tools\codex.exe",
                "codex-cli 0.144.6",
                "a".repeat(64),
                "b".repeat(64),
                1,
                "thread-1",
                "turn-1",
                r"C:\delivery\repo",
                "d".repeat(40),
                "e".repeat(40),
            )
            .is_ok()
        );
        assert!(
            DeliverySuccessEvidence::new(
                intent,
                r"C:\tools\codex.exe",
                "codex-cli 0.144.6",
                "a".repeat(64),
                "b".repeat(64),
                0,
                "thread-1",
                "turn-1",
                r"C:\delivery\repo",
                "d".repeat(40),
                "e".repeat(40),
            )
            .is_err()
        );
    }

    #[test]
    fn authoritative_envelopes_bind_result_to_intent_and_reject_valid_json_drift() {
        let intent_value = DeliveryIntentEvidence::new(
            r"C:\tools\codex.exe",
            "codex-cli 0.144.6",
            "a".repeat(64),
            r"C:\delivery\schema",
            r"C:\delivery\codex-home",
            r"C:\delivery\repo",
        )
        .expect("intent evidence")
        .canonical_value();
        let intent_digest = delivery_digest("lattice.runtime.codex-delivery-intent", &intent_value)
            .expect("intent digest");
        let intent =
            validate_intent_evidence(Some(&intent_value), &intent_digest).expect("intent envelope");
        let success = DeliverySuccessEvidence::new(
            intent_digest.clone(),
            r"C:\tools\codex.exe",
            "codex-cli 0.144.6",
            "a".repeat(64),
            "b".repeat(64),
            1,
            "thread-1",
            "turn-1",
            r"C:\delivery\repo",
            "d".repeat(40),
            "e".repeat(40),
        )
        .expect("success evidence");
        let result = success.canonical_value();
        let CanonicalValue::Object(result_entries) = &result else {
            panic!("result object");
        };
        assert!(
            result_entries.windows(2).all(|pair| pair[0].0 < pair[1].0),
            "result evidence keys must already be canonical before jsonb persistence"
        );
        let result_digest = delivery_digest("lattice.runtime.codex-delivery-result", &result)
            .expect("result digest");
        let receipt =
            validate_result_evidence(Some(&result), &result_digest, &intent_digest, &intent)
                .expect("validated receipt");
        assert_eq!(receipt.intent_digest(), intent_digest.as_str());
        assert_eq!(receipt.outcome_digest(), result_digest.as_str());
        assert_eq!(receipt.repository_path(), r"C:\delivery\repo");
        assert_eq!(receipt.commit_sha(), "d".repeat(40));
        assert_eq!(receipt.parent_sha(), "e".repeat(40));

        let mut drifted = result;
        let CanonicalValue::Object(entries) = &mut drifted else {
            panic!("result object");
        };
        let (_, changed_path) = entries
            .iter_mut()
            .find(|(key, _)| key == "changed_path")
            .expect("changed path");
        *changed_path = CanonicalValue::String("foreign.txt".to_owned());
        let drifted_digest = delivery_digest("lattice.runtime.codex-delivery-result", &drifted)
            .expect("drifted digest");
        assert!(
            validate_result_evidence(Some(&drifted), &drifted_digest, &intent_digest, &intent,)
                .is_none()
        );
    }

    #[test]
    fn task019_application_name_is_used_for_schema_compatibility() {
        assert_eq!(APPLICATION_NAME, "lattice-devos-task019");
    }

    #[test]
    fn task_identity_and_authority_are_constructible() {
        assert_eq!(
            delivery_identity().expect("identity").task_id().as_str(),
            TASK_ID
        );
        assert_eq!(
            delivery_authority().expect("authority").runtime(),
            RuntimeKind::Live
        );
    }
}
