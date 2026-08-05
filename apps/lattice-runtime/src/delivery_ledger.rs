//! Durable TASK-032 intent and outcome flow over the verified `PostgreSQL` ledger.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::path::Path;
use std::time::Instant;

use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256};
use lattice_contracts::{
    CodexDeliveryEvidence, CodexDeliveryRequest, CompletedDeliveryEvidence, ContentDigest,
    DaemonEpoch, DeliveryOutcomeEvidence as TypedDeliveryOutcomeEvidence,
    DeliveryOutcomeRequest as TypedDeliveryOutcomeRequest, DeliveryProfile,
    DeliveryReceipt as TypedDeliveryReceipt, DeliveryRunRequest, DeliveryRuntime, DeliveryStage,
    DeliveryStatusRequest, DeliveryTerminalStatus, DurableIntentEvidence, FixedTestEvidence,
    GitCommitEvidence, Invocation, PreparedWorkspaceEvidence, ProjectId, ProjectSnapshotId,
    RuntimeAdmissionMode, RuntimeKind, StoreAuthorityHead, StoreAuthorityRevision,
    StoreDaemonInstanceId, TaskId, TaskLedgerStreamIdentity, WorkspaceChangeEvidence,
};
use lattice_ports::{
    DeliveryFailureCertainty, DeliveryLedgerPort, DeliveryPortError, DeliveryPortResult,
    PortErrorKind,
};
use lattice_postgres_store::{MigrationTarget, PostgresTaskLedger, PostgresTaskLedgerErrorKind};
use lattice_task_ledger::{
    ActionId, ActorId, AppendCommand, CommandId, CommandOutcome, CorrelationId, Diagnostic,
    LedgerEvent, LedgerEventKind, LedgerOutcome, ReasonCode, VerifiedStream,
};
use postgres::config::SslMode;
use postgres::{Client, Config, NoTls};
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
/// Explicit format tag for the pre-typed 14-field completed result envelope.
pub(crate) const LEGACY_RECEIPT_FORMAT: &str = "legacy-delivery-result-v1";

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
    Reconciliation,
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
    MutationDeadlineAmbiguous,
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

    pub(crate) fn database_name(&self) -> String {
        format!("lattice_task019_{}_base", &self.run_id[..8])
    }

    /// Returns the marker-owned disposable run identity.
    #[must_use]
    pub fn run_id(&self) -> &str {
        &self.run_id
    }
}

fn fixed_runtime_config(
    binding: &DeliveryDatabaseBinding,
    password: &str,
    deadline: Instant,
) -> Result<Config, DeliveryLedgerError> {
    if password.is_empty() {
        return Err(delivery_error(DeliveryLedgerErrorKind::InvalidBinding));
    }
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
    let mut config = Config::new();
    config
        .host(&binding.host)
        .port(binding.port)
        .user("lattice_runtime_login")
        .password(password)
        .dbname(&binding.database_name())
        .application_name(APPLICATION_NAME)
        .connect_timeout(remaining)
        .options(&database_timeouts)
        .ssl_mode(SslMode::Disable);
    Ok(config)
}

/// Opens the sole fixed runtime-role connection admitted by this process.
///
/// The caller supplies the already-validated marker binding, the service's
/// existing secret, and its absolute deadline. No DSN, role, schema, SQL, or
/// caller-selected connection option is accepted.
pub(crate) fn connect_fixed_runtime_client(
    binding: &DeliveryDatabaseBinding,
    password: &str,
    deadline: Instant,
) -> Result<Client, DeliveryLedgerError> {
    let mut client = fixed_runtime_config(binding, password, deadline)?
        .connect(NoTls)
        .map_err(|_| delivery_error(DeliveryLedgerErrorKind::ConnectFailed))?;
    client
        .batch_execute("SET ROLE lattice_runtime")
        .map_err(|_| delivery_error(DeliveryLedgerErrorKind::ConnectFailed))?;
    ensure_before_deadline(deadline)?;
    Ok(client)
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
    configuration_digest: ContentDigest,
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
        request: &DeliveryRunRequest,
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
            configuration_digest: request.configuration_digest().clone(),
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
                "configuration_digest".to_owned(),
                CanonicalValue::String(self.configuration_digest.as_str().to_owned()),
            ),
            (
                "envelope_version".to_owned(),
                CanonicalValue::String(TYPED_INTENT_ENVELOPE_VERSION.to_owned()),
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
#[cfg(test)]
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

#[cfg(test)]
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

/// Typed delivery-ledger port over the existing durable `PostgreSQL` stream.
///
/// One instance is bound to exactly one composition-created request and one
/// private legacy intent configuration. The wrapper adds no SQL surface and
/// does not order or invoke any other adapter.
pub struct PostgresDeliveryLedgerAdapter {
    ledger: DeliveryLedger,
    request: DeliveryRunRequest,
    intent_config: Option<DeliveryIntentEvidence>,
}

/// Restart status either returns the explicitly supported legacy receipt or a
/// typed v2 ledger adapter whose exact request was reconstructed from durable
/// intent/outcome evidence.
pub(crate) enum PostgresDeliveryStatusReplay {
    Legacy(Box<DeliveryReceipt>),
    Typed(Box<PostgresDeliveryLedgerAdapter>),
}

impl PostgresDeliveryLedgerAdapter {
    /// Resolves a status-only replay without accepting run configuration from
    /// the caller. Legacy 14-field receipts remain an explicit separate form;
    /// typed v1 or malformed streams are never promoted to typed v2.
    ///
    /// Write methods fail closed because no private intent configuration is
    /// present; [`DeliveryLedgerPort::load_receipt`] remains available.
    pub(crate) fn for_status(
        mut ledger: DeliveryLedger,
        expected_invocation: &Invocation,
        expected_profile: DeliveryProfile,
    ) -> Result<PostgresDeliveryStatusReplay, DeliveryLedgerError> {
        if let Some(receipt) = ledger.legacy_completed_receipt()? {
            return Ok(PostgresDeliveryStatusReplay::Legacy(Box::new(receipt)));
        }
        let request = ledger.typed_request(expected_invocation, expected_profile)?;
        Ok(PostgresDeliveryStatusReplay::Typed(Box::new(Self {
            ledger,
            request,
            intent_config: None,
        })))
    }

    /// Returns the exact request reconstructed for a typed v2 status replay.
    #[must_use]
    pub(crate) const fn request(&self) -> &DeliveryRunRequest {
        &self.request
    }

    /// Binds an already verified ledger connection and private fixed intent.
    #[must_use]
    pub(crate) const fn with_intent_config(
        ledger: DeliveryLedger,
        request: DeliveryRunRequest,
        intent_config: DeliveryIntentEvidence,
    ) -> Self {
        Self {
            ledger,
            request,
            intent_config: Some(intent_config),
        }
    }

    /// Constructs a writable adapter without exposing the private intent type.
    ///
    /// # Errors
    ///
    /// Rejects malformed fixed launcher, schema, Codex-home, or repository
    /// configuration before the adapter can append an intent.
    #[allow(clippy::too_many_arguments)]
    pub fn for_delivery(
        ledger: DeliveryLedger,
        request: DeliveryRunRequest,
        launcher_path: impl Into<String>,
        version: impl Into<String>,
        launcher_sha256: impl Into<String>,
        schema_directory: impl Into<String>,
        codex_home: impl Into<String>,
        repository_path: impl Into<String>,
    ) -> Result<Self, DeliveryLedgerError> {
        let intent_config = DeliveryIntentEvidence::new(
            &request,
            launcher_path,
            version,
            launcher_sha256,
            schema_directory,
            codex_home,
            repository_path,
        )?;
        Ok(Self::with_intent_config(ledger, request, intent_config))
    }
}

impl DeliveryLedgerPort for PostgresDeliveryLedgerAdapter {
    fn record_intent(
        &mut self,
        request: &DeliveryRunRequest,
    ) -> DeliveryPortResult<DurableIntentEvidence> {
        if request != &self.request {
            return Err(ledger_binding_error(DeliveryStage::Intent));
        }
        let intent_config = self.intent_config.as_ref().ok_or_else(|| {
            delivery_port_error(
                DeliveryStage::Intent,
                PortErrorKind::Denied,
                DeliveryFailureCertainty::Known,
                "DELIVERY_LEDGER_STATUS_ONLY",
            )
        })?;
        let intent_digest = self
            .ledger
            .record_intent(intent_config)
            .map_err(|error| map_port_ledger_error(DeliveryStage::Intent, error))?;
        DurableIntentEvidence::new(request, intent_digest).map_err(|_| {
            delivery_port_error(
                DeliveryStage::Intent,
                PortErrorKind::Ambiguous,
                DeliveryFailureCertainty::Ambiguous,
                "DELIVERY_LEDGER_INTENT_EVIDENCE_INVALID",
            )
        })
    }

    fn record_outcome(
        &mut self,
        request: &TypedDeliveryOutcomeRequest,
    ) -> DeliveryPortResult<TypedDeliveryOutcomeEvidence> {
        if !request.binding().matches_run(&self.request) {
            return Err(ledger_binding_error(DeliveryStage::Outcome));
        }
        let intent_config = self.intent_config.as_ref().ok_or_else(|| {
            delivery_port_error(
                DeliveryStage::Outcome,
                PortErrorKind::Denied,
                DeliveryFailureCertainty::Known,
                "DELIVERY_LEDGER_STATUS_ONLY",
            )
        })?;
        if !typed_outcome_matches_intent_config(request, intent_config) {
            return Err(ledger_binding_error(DeliveryStage::Outcome));
        }
        let outcome_digest = self
            .ledger
            .record_typed_outcome(request)
            .map_err(|error| map_port_ledger_error(DeliveryStage::Outcome, error))?;
        TypedDeliveryOutcomeEvidence::new(request.clone(), outcome_digest).map_err(|_| {
            delivery_port_error(
                DeliveryStage::Outcome,
                PortErrorKind::Ambiguous,
                DeliveryFailureCertainty::Ambiguous,
                "DELIVERY_LEDGER_OUTCOME_EVIDENCE_INVALID",
            )
        })
    }

    fn load_receipt(
        &mut self,
        request: &DeliveryStatusRequest,
    ) -> DeliveryPortResult<TypedDeliveryReceipt> {
        validate_status_binding(&self.request, request)?;
        self.ledger
            .typed_receipt(&self.request)
            .map_err(|error| map_port_ledger_error(DeliveryStage::Receipt, error))
    }
}

fn validate_status_binding(
    expected: &DeliveryRunRequest,
    request: &DeliveryStatusRequest,
) -> DeliveryPortResult<()> {
    if request == &expected.status_request() {
        Ok(())
    } else {
        Err(ledger_binding_error(DeliveryStage::Receipt))
    }
}

fn typed_outcome_matches_intent_config(
    request: &TypedDeliveryOutcomeRequest,
    intent: &DeliveryIntentEvidence,
) -> bool {
    if request.binding().configuration_digest() != &intent.configuration_digest {
        return false;
    }
    let Some(completed) = request.completed_evidence() else {
        return true;
    };
    completed.codex().launcher_locator() == intent.launcher_path
        && completed.codex().version() == intent.version
        && completed.codex().launcher_sha256() == &intent.launcher_sha256
        && completed.workspace().workspace_locator() == intent.repository_path
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
        let database_name = binding.database_name();
        let target = MigrationTarget::new(database_name.clone(), binding.run_id.clone())
            .map_err(|_| delivery_error(DeliveryLedgerErrorKind::InvalidBinding))?;
        let client = connect_fixed_runtime_client(binding, password, deadline)?;
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
        self.ensure_before_deadline()?;
        let deadline = self.deadline;
        let execution_result = self
            .ledger
            .execute(command, self.authority.clone())
            .map_err(map_ledger_error);
        let execution = finish_mutation_at(execution_result, deadline, Instant::now())?;
        if execution.receipt().outcome() != &CommandOutcome::Appended
            || execution
                .outbox_admission()
                .is_none_or(|outbox| outbox.intent_digest() != &subject_digest)
        {
            return finish_mutation_at(
                Err(delivery_error(DeliveryLedgerErrorKind::LedgerRejected)),
                deadline,
                Instant::now(),
            );
        }
        finish_mutation_at(Ok(subject_digest), deadline, Instant::now())
    }

    /// Appends one typed terminal envelope using the existing durable outcome
    /// event. No database schema or stream cardinality changes are required.
    fn record_typed_outcome(
        &mut self,
        request: &TypedDeliveryOutcomeRequest,
    ) -> Result<ContentDigest, DeliveryLedgerError> {
        self.ensure_before_deadline()?;
        let loaded = self
            .ledger
            .load_stream(self.identity.clone())
            .map_err(map_outcome_load_error)?;
        self.ensure_before_deadline()?;
        if inspect_stream(loaded.stream()).projection != Projection::Pending
            || loaded.stream().events()[0].subject_digest() != request.intent_digest()
        {
            return Err(delivery_error(
                DeliveryLedgerErrorKind::ReconciliationRequired,
            ));
        }
        let canonical = typed_outcome_value(request)?;
        let subject_digest = delivery_digest("lattice.runtime.typed-delivery-outcome", &canonical)?;
        let (outcome, reason) = match request.status() {
            DeliveryTerminalStatus::Completed => {
                (LedgerOutcome::Passed, "TASK032_DELIVERY_COMPLETED")
            }
            DeliveryTerminalStatus::Failed => (LedgerOutcome::Failed, "TASK032_DELIVERY_FAILED"),
            DeliveryTerminalStatus::ReconciliationRequired => {
                (LedgerOutcome::Cancelled, "TASK032_DELIVERY_RECONCILIATION")
            }
        };
        let command = append_command(
            loaded.stream().head().clone(),
            OUTCOME_COMMAND_ID,
            LedgerEventKind::EffectOutcome,
            outcome,
            reason,
            subject_digest.clone(),
            canonical,
        )?;
        self.ensure_before_deadline()?;
        let deadline = self.deadline;
        let execution_result = self
            .ledger
            .execute(command, self.authority.clone())
            .map_err(map_outcome_append_error);
        let execution = finish_mutation_at(execution_result, deadline, Instant::now())?;
        if execution.receipt().outcome() != &CommandOutcome::Appended {
            return finish_mutation_at(
                Err(delivery_error(DeliveryLedgerErrorKind::LedgerRejected)),
                deadline,
                Instant::now(),
            );
        }
        finish_mutation_at(Ok(subject_digest), deadline, Instant::now())
    }

    /// Reloads and reconstructs a full typed terminal chain after restart.
    fn typed_receipt(
        &mut self,
        request: &DeliveryRunRequest,
    ) -> Result<TypedDeliveryReceipt, DeliveryLedgerError> {
        self.ensure_before_deadline()?;
        let loaded = self
            .ledger
            .load_stream(self.identity.clone())
            .map_err(map_ledger_error)?;
        self.ensure_before_deadline()?;
        typed_receipt_from_stream(loaded.stream(), request)
    }

    /// Reconstructs the exact typed v2 run request from `PostgreSQL`. Only the
    /// deterministic run invocation/profile is supplied by composition.
    fn typed_request(
        &mut self,
        expected_invocation: &Invocation,
        expected_profile: DeliveryProfile,
    ) -> Result<DeliveryRunRequest, DeliveryLedgerError> {
        self.ensure_before_deadline()?;
        let loaded = self
            .ledger
            .load_stream(self.identity.clone())
            .map_err(map_ledger_error)?;
        self.ensure_before_deadline()?;
        typed_request_from_stream(loaded.stream(), expected_invocation, expected_profile)
    }

    /// Reloads the durable stream and returns a receipt only when its outcome
    /// is the exact pre-typed 14-field completed envelope.
    fn legacy_completed_receipt(&mut self) -> Result<Option<DeliveryReceipt>, DeliveryLedgerError> {
        self.ensure_before_deadline()?;
        let loaded = self
            .ledger
            .load_stream(self.identity.clone())
            .map_err(map_ledger_error)?;
        self.ensure_before_deadline()?;
        Ok(legacy_completed_receipt_from_stream(loaded.stream()))
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

const TYPED_INTENT_ENVELOPE_VERSION: &str = "typed-delivery-intent-v2";
const TYPED_ENVELOPE_VERSION: &str = "typed-delivery-v2";
const TYPED_ENVELOPE_FIELD_COUNT: usize = 31;
const ABSENT_TYPED_FIELD: &str = "NONE";

#[allow(clippy::too_many_lines)]
fn typed_outcome_value(
    outcome: &TypedDeliveryOutcomeRequest,
) -> Result<CanonicalValue, DeliveryLedgerError> {
    let request = outcome.binding();
    let invocation = request.invocation();
    let mut values = BTreeMap::new();
    values.insert("attempt_id", invocation.attempt_id().as_str().to_owned());
    values.insert("baseline_commit", ABSENT_TYPED_FIELD.to_owned());
    values.insert("change_evidence_digest", ABSENT_TYPED_FIELD.to_owned());
    values.insert("changed_paths_digest", ABSENT_TYPED_FIELD.to_owned());
    values.insert("codex_output_digest", ABSENT_TYPED_FIELD.to_owned());
    values.insert("codex_runtime", ABSENT_TYPED_FIELD.to_owned());
    values.insert("codex_version", ABSENT_TYPED_FIELD.to_owned());
    values.insert(
        "configuration_digest",
        request.configuration_digest().as_str().to_owned(),
    );
    values.insert("contract_version", invocation.version().to_string());
    values.insert("envelope_version", TYPED_ENVELOPE_VERSION.to_owned());
    values.insert(
        "failure_code",
        outcome
            .failure_code()
            .unwrap_or(ABSENT_TYPED_FIELD)
            .to_owned(),
    );
    values.insert(
        "failure_stage",
        outcome
            .failure_stage()
            .map_or(ABSENT_TYPED_FIELD, delivery_stage_name)
            .to_owned(),
    );
    values.insert("git_commit", ABSENT_TYPED_FIELD.to_owned());
    values.insert("git_evidence_digest", ABSENT_TYPED_FIELD.to_owned());
    values.insert("git_parent_commit", ABSENT_TYPED_FIELD.to_owned());
    values.insert("intent_digest", outcome.intent_digest().as_str().to_owned());
    values.insert("launcher_locator", ABSENT_TYPED_FIELD.to_owned());
    values.insert("launcher_sha256", ABSENT_TYPED_FIELD.to_owned());
    values.insert("profile", request.profile().as_str().to_owned());
    values.insert(
        "project_snapshot_id",
        invocation.project_snapshot_id().as_str().to_owned(),
    );
    values.insert("request_id", invocation.request_id().as_str().to_owned());
    values.insert("schema_bundle_sha256", ABSENT_TYPED_FIELD.to_owned());
    values.insert("schema_file_count", ABSENT_TYPED_FIELD.to_owned());
    values.insert(
        "subject_digest",
        invocation.subject_digest().as_str().to_owned(),
    );
    values.insert("task_id", invocation.task_id().as_str().to_owned());
    values.insert("test_evidence_digest", ABSENT_TYPED_FIELD.to_owned());
    values.insert("thread_id", ABSENT_TYPED_FIELD.to_owned());
    values.insert("turn_id", ABSENT_TYPED_FIELD.to_owned());
    values.insert("workspace_evidence_digest", ABSENT_TYPED_FIELD.to_owned());
    values.insert("workspace_id", ABSENT_TYPED_FIELD.to_owned());
    values.insert("workspace_locator", ABSENT_TYPED_FIELD.to_owned());

    if let Some(completed) = outcome.completed_evidence() {
        values.insert(
            "baseline_commit",
            completed.workspace().baseline_commit().to_owned(),
        );
        values.insert(
            "change_evidence_digest",
            completed.changes().evidence_digest().as_str().to_owned(),
        );
        values.insert(
            "changed_paths_digest",
            completed
                .changes()
                .changed_paths_digest()
                .as_str()
                .to_owned(),
        );
        values.insert(
            "codex_output_digest",
            completed.codex().output_digest().as_str().to_owned(),
        );
        values.insert(
            "codex_runtime",
            delivery_runtime_name(completed.codex().runtime()).to_owned(),
        );
        values.insert("codex_version", completed.codex().version().to_owned());
        values.insert("failure_code", ABSENT_TYPED_FIELD.to_owned());
        values.insert("failure_stage", ABSENT_TYPED_FIELD.to_owned());
        values.insert("git_commit", completed.git().commit().to_owned());
        values.insert(
            "git_evidence_digest",
            completed.git().evidence_digest().as_str().to_owned(),
        );
        values.insert(
            "git_parent_commit",
            completed.git().parent_commit().to_owned(),
        );
        values.insert(
            "launcher_locator",
            completed.codex().launcher_locator().to_owned(),
        );
        values.insert(
            "launcher_sha256",
            completed.codex().launcher_sha256().as_str().to_owned(),
        );
        values.insert(
            "schema_bundle_sha256",
            completed.codex().schema_bundle_sha256().as_str().to_owned(),
        );
        values.insert(
            "schema_file_count",
            completed.codex().schema_file_count().to_string(),
        );
        values.insert(
            "test_evidence_digest",
            completed.test().evidence_digest().as_str().to_owned(),
        );
        values.insert("thread_id", completed.codex().thread_id().to_owned());
        values.insert("turn_id", completed.codex().turn_id().to_owned());
        values.insert(
            "workspace_evidence_digest",
            completed.workspace().evidence_digest().as_str().to_owned(),
        );
        values.insert(
            "workspace_id",
            completed.workspace().workspace_id().to_owned(),
        );
        values.insert(
            "workspace_locator",
            completed.workspace().workspace_locator().to_owned(),
        );
    }

    values.insert("status", terminal_status_name(outcome.status()).to_owned());
    // `status` intentionally makes this a 32-field envelope. Keeping the
    // count assertion here prevents silent diagnostic drift.
    if values.len() != TYPED_ENVELOPE_FIELD_COUNT + 1 {
        return Err(evidence_error());
    }
    Ok(CanonicalValue::Object(
        values
            .into_iter()
            .map(|(key, value)| (key.to_owned(), CanonicalValue::String(value)))
            .collect(),
    ))
}

fn reconstruct_typed_receipt(
    request: &DeliveryRunRequest,
    intent_digest: &ContentDigest,
    outcome_digest: &ContentDigest,
    value: &CanonicalValue,
) -> Option<TypedDeliveryReceipt> {
    let fields = validate_typed_envelope(value, outcome_digest, intent_digest, Some(request))?;
    let intent = DurableIntentEvidence::new(request, intent_digest.clone()).ok()?;
    let outcome_request = match parse_terminal_status(fields.get("status")?)? {
        DeliveryTerminalStatus::Completed => {
            let workspace = PreparedWorkspaceEvidence::new(
                request,
                &intent,
                *fields.get("workspace_id")?,
                *fields.get("workspace_locator")?,
                *fields.get("baseline_commit")?,
                parse_digest(fields.get("workspace_evidence_digest")?)?,
            )
            .ok()?;
            let codex_request =
                CodexDeliveryRequest::new(request.clone(), intent.clone(), workspace.clone())
                    .ok()?;
            let codex = CodexDeliveryEvidence::new(
                &codex_request,
                parse_delivery_runtime(fields.get("codex_runtime")?)?,
                *fields.get("launcher_locator")?,
                *fields.get("codex_version")?,
                parse_digest(fields.get("launcher_sha256")?)?,
                parse_digest(fields.get("schema_bundle_sha256")?)?,
                fields.get("schema_file_count")?.parse().ok()?,
                *fields.get("thread_id")?,
                *fields.get("turn_id")?,
                parse_digest(fields.get("codex_output_digest")?)?,
            )
            .ok()?;
            let changes = WorkspaceChangeEvidence::new(
                request,
                &intent,
                &workspace,
                &codex,
                parse_digest(fields.get("changed_paths_digest")?)?,
                parse_digest(fields.get("change_evidence_digest")?)?,
            )
            .ok()?;
            let test = FixedTestEvidence::new(
                request,
                &changes,
                parse_digest(fields.get("test_evidence_digest")?)?,
            )
            .ok()?;
            let git = GitCommitEvidence::new(
                request,
                &changes,
                &test,
                *fields.get("git_parent_commit")?,
                *fields.get("git_commit")?,
                parse_digest(fields.get("git_evidence_digest")?)?,
            )
            .ok()?;
            let completed = CompletedDeliveryEvidence::new(
                request.clone(),
                intent.clone(),
                workspace,
                codex,
                changes,
                test,
                git,
            )
            .ok()?;
            TypedDeliveryOutcomeRequest::completed(request, completed).ok()?
        }
        DeliveryTerminalStatus::Failed => TypedDeliveryOutcomeRequest::failed(
            request,
            &intent,
            parse_delivery_stage(fields.get("failure_stage")?)?,
            *fields.get("failure_code")?,
        )
        .ok()?,
        DeliveryTerminalStatus::ReconciliationRequired => {
            TypedDeliveryOutcomeRequest::reconciliation_required(
                request,
                &intent,
                parse_delivery_stage(fields.get("failure_stage")?)?,
                *fields.get("failure_code")?,
            )
            .ok()?
        }
    };
    let outcome =
        TypedDeliveryOutcomeEvidence::new(outcome_request, outcome_digest.clone()).ok()?;
    let receipt_digest = delivery_digest(
        "lattice.runtime.typed-delivery-receipt",
        &CanonicalValue::Object(vec![
            ("outcome".to_owned(), value.clone()),
            (
                "outcome_digest".to_owned(),
                CanonicalValue::String(outcome_digest.as_str().to_owned()),
            ),
        ]),
    )
    .ok()?;
    TypedDeliveryReceipt::new(outcome, receipt_digest).ok()
}

#[allow(clippy::too_many_lines)]
fn validate_typed_envelope<'a>(
    value: &'a CanonicalValue,
    outcome_digest: &ContentDigest,
    intent_digest: &ContentDigest,
    request: Option<&DeliveryRunRequest>,
) -> Option<BTreeMap<&'a str, &'a str>> {
    let fields = string_fields(value, TYPED_ENVELOPE_FIELD_COUNT + 1)?;
    if fields.get("envelope_version")? != &TYPED_ENVELOPE_VERSION
        || fields.get("intent_digest")? != &intent_digest.as_str()
        || fields.get("profile")?
            != &lattice_contracts::DeliveryProfile::Task032CodexPostgres.as_str()
        || fields.get("contract_version")?.parse::<u16>().ok()?
            != lattice_contracts::CONTRACT_VERSION
        || !is_lower_hex(fields.get("configuration_digest")?, 64)
        || !is_lower_hex(fields.get("subject_digest")?, 64)
        || delivery_digest("lattice.runtime.typed-delivery-outcome", value)
            .ok()
            .as_ref()
            != Some(outcome_digest)
    {
        return None;
    }
    for field in ["attempt_id", "project_snapshot_id", "request_id", "task_id"] {
        if fields.get(field)?.is_empty() {
            return None;
        }
    }
    if let Some(request) = request {
        let invocation = request.invocation();
        if fields.get("attempt_id")? != &invocation.attempt_id().as_str()
            || fields.get("configuration_digest")? != &request.configuration_digest().as_str()
            || fields.get("contract_version")? != &invocation.version().to_string().as_str()
            || fields.get("profile")? != &request.profile().as_str()
            || fields.get("project_snapshot_id")? != &invocation.project_snapshot_id().as_str()
            || fields.get("request_id")? != &invocation.request_id().as_str()
            || fields.get("subject_digest")? != &invocation.subject_digest().as_str()
            || fields.get("task_id")? != &invocation.task_id().as_str()
        {
            return None;
        }
    }
    let status = parse_terminal_status(fields.get("status")?)?;
    match status {
        DeliveryTerminalStatus::Completed => {
            if fields.get("failure_code")? != &ABSENT_TYPED_FIELD
                || fields.get("failure_stage")? != &ABSENT_TYPED_FIELD
                || parse_delivery_runtime(fields.get("codex_runtime")?).is_none()
                || !is_lower_hex(fields.get("baseline_commit")?, 40)
                || !is_lower_hex(fields.get("git_commit")?, 40)
                || fields.get("baseline_commit")? == fields.get("git_commit")?
                || !canonical_positive_u32(fields.get("schema_file_count")?)
            {
                return None;
            }
            for field in [
                "change_evidence_digest",
                "changed_paths_digest",
                "codex_output_digest",
                "git_evidence_digest",
                "launcher_sha256",
                "schema_bundle_sha256",
                "test_evidence_digest",
                "workspace_evidence_digest",
            ] {
                if !is_lower_hex(fields.get(field)?, 64) {
                    return None;
                }
            }
            for field in [
                "codex_version",
                "launcher_locator",
                "thread_id",
                "turn_id",
                "workspace_id",
                "workspace_locator",
            ] {
                if fields.get(field)?.is_empty() || fields.get(field)? == &ABSENT_TYPED_FIELD {
                    return None;
                }
            }
        }
        DeliveryTerminalStatus::Failed | DeliveryTerminalStatus::ReconciliationRequired => {
            if parse_delivery_stage(fields.get("failure_stage")?).is_none()
                || !valid_failure_code(fields.get("failure_code")?)
            {
                return None;
            }
            for field in [
                "baseline_commit",
                "change_evidence_digest",
                "changed_paths_digest",
                "codex_output_digest",
                "codex_runtime",
                "codex_version",
                "git_commit",
                "git_evidence_digest",
                "git_parent_commit",
                "launcher_locator",
                "launcher_sha256",
                "schema_bundle_sha256",
                "schema_file_count",
                "test_evidence_digest",
                "thread_id",
                "turn_id",
                "workspace_evidence_digest",
                "workspace_id",
                "workspace_locator",
            ] {
                if fields.get(field)? != &ABSENT_TYPED_FIELD {
                    return None;
                }
            }
        }
    }
    Some(fields)
}

fn typed_legacy_receipt(
    value: Option<&CanonicalValue>,
    outcome_digest: &ContentDigest,
    intent_digest: &ContentDigest,
    intent: &IntentEvidenceProjection,
) -> Option<DeliveryReceipt> {
    let value = value?;
    let fields = validate_typed_envelope(value, outcome_digest, intent_digest, None)?;
    if intent.format != IntentEnvelopeFormat::TypedV2
        || fields.get("configuration_digest")? != &intent.configuration_digest.as_ref()?.as_str()
        || parse_terminal_status(fields.get("status")?)? != DeliveryTerminalStatus::Completed
        || fields.get("launcher_locator")? != &intent.launcher_path.as_str()
        || fields.get("launcher_sha256")? != &intent.launcher_sha256.as_str()
        || fields.get("workspace_locator")? != &intent.repository_path.as_str()
        || fields.get("codex_version")? != &intent.version.as_str()
    {
        return None;
    }
    Some(DeliveryReceipt {
        intent_digest: intent_digest.clone(),
        outcome_digest: outcome_digest.clone(),
        launcher_path: (*fields.get("launcher_locator")?).to_owned(),
        version: (*fields.get("codex_version")?).to_owned(),
        launcher_sha256: (*fields.get("launcher_sha256")?).to_owned(),
        schema_bundle_sha256: (*fields.get("schema_bundle_sha256")?).to_owned(),
        schema_file_count: fields.get("schema_file_count")?.parse().ok()?,
        thread_id: (*fields.get("thread_id")?).to_owned(),
        turn_id: (*fields.get("turn_id")?).to_owned(),
        repository_path: (*fields.get("workspace_locator")?).to_owned(),
        commit_sha: (*fields.get("git_commit")?).to_owned(),
        parent_sha: (*fields.get("git_parent_commit")?).to_owned(),
    })
}

fn legacy_completed_receipt_from_stream(stream: &VerifiedStream) -> Option<DeliveryReceipt> {
    if inspect_stream(stream).projection != Projection::Completed || stream.events().len() != 2 {
        return None;
    }
    let intent_event = &stream.events()[0];
    let outcome_event = &stream.events()[1];
    let intent = validate_intent_evidence(
        intent_event.diagnostic().map(Diagnostic::value),
        intent_event.subject_digest(),
    )?;
    validate_result_evidence(
        outcome_event.diagnostic().map(Diagnostic::value),
        outcome_event.subject_digest(),
        intent_event.subject_digest(),
        &intent,
    )
}

fn typed_receipt_from_stream(
    stream: &VerifiedStream,
    request: &DeliveryRunRequest,
) -> Result<TypedDeliveryReceipt, DeliveryLedgerError> {
    let inspected = inspect_stream(stream);
    if stream.events().len() != 2 {
        return Err(delivery_error(
            DeliveryLedgerErrorKind::ReconciliationRequired,
        ));
    }
    let intent_event = &stream.events()[0];
    let outcome_event = &stream.events()[1];
    let receipt = reconstruct_typed_receipt(
        request,
        intent_event.subject_digest(),
        outcome_event.subject_digest(),
        outcome_event
            .diagnostic()
            .map(Diagnostic::value)
            .ok_or_else(|| delivery_error(DeliveryLedgerErrorKind::EvidenceInvalid))?,
    )
    .ok_or_else(|| delivery_error(DeliveryLedgerErrorKind::EvidenceInvalid))?;
    let valid_projection = matches!(
        (inspected.projection, receipt.status()),
        (Projection::Completed, DeliveryTerminalStatus::Completed)
            | (Projection::Failed, DeliveryTerminalStatus::Failed)
            | (
                Projection::Reconciliation,
                DeliveryTerminalStatus::ReconciliationRequired
            )
    );
    if valid_projection {
        Ok(receipt)
    } else {
        Err(delivery_error(
            DeliveryLedgerErrorKind::ReconciliationRequired,
        ))
    }
}

fn typed_request_from_stream(
    stream: &VerifiedStream,
    expected_invocation: &Invocation,
    expected_profile: DeliveryProfile,
) -> Result<DeliveryRunRequest, DeliveryLedgerError> {
    if stream.events().len() != 2 {
        return Err(delivery_error(
            DeliveryLedgerErrorKind::ReconciliationRequired,
        ));
    }
    let intent_event = &stream.events()[0];
    let outcome_event = &stream.events()[1];
    let intent = validate_intent_evidence(
        intent_event.diagnostic().map(Diagnostic::value),
        intent_event.subject_digest(),
    )
    .filter(|intent| intent.format == IntentEnvelopeFormat::TypedV2)
    .ok_or_else(|| delivery_error(DeliveryLedgerErrorKind::ReconciliationRequired))?;
    let configuration_digest = intent
        .configuration_digest
        .ok_or_else(|| delivery_error(DeliveryLedgerErrorKind::EvidenceInvalid))?;
    let request = DeliveryRunRequest::new(
        expected_invocation.clone(),
        expected_profile,
        configuration_digest,
    )
    .map_err(|_| delivery_error(DeliveryLedgerErrorKind::EvidenceInvalid))?;
    let outcome_value = outcome_event
        .diagnostic()
        .map(Diagnostic::value)
        .ok_or_else(|| delivery_error(DeliveryLedgerErrorKind::EvidenceInvalid))?;
    validate_typed_envelope(
        outcome_value,
        outcome_event.subject_digest(),
        intent_event.subject_digest(),
        Some(&request),
    )
    .ok_or_else(|| delivery_error(DeliveryLedgerErrorKind::EvidenceInvalid))?;
    typed_receipt_from_stream(stream, &request)?;
    Ok(request)
}

fn parse_digest(value: &str) -> Option<ContentDigest> {
    ContentDigest::from_sha256(value.to_owned()).ok()
}

fn canonical_positive_u32(value: &str) -> bool {
    value
        .parse::<u32>()
        .ok()
        .is_some_and(|parsed| parsed > 0 && parsed.to_string() == value)
}

fn valid_failure_code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

const fn terminal_status_name(status: DeliveryTerminalStatus) -> &'static str {
    match status {
        DeliveryTerminalStatus::Completed => "COMPLETED",
        DeliveryTerminalStatus::Failed => "FAILED",
        DeliveryTerminalStatus::ReconciliationRequired => "RECONCILIATION_REQUIRED",
    }
}

fn parse_terminal_status(value: &str) -> Option<DeliveryTerminalStatus> {
    match value {
        "COMPLETED" => Some(DeliveryTerminalStatus::Completed),
        "FAILED" => Some(DeliveryTerminalStatus::Failed),
        "RECONCILIATION_REQUIRED" => Some(DeliveryTerminalStatus::ReconciliationRequired),
        _ => None,
    }
}

const fn delivery_runtime_name(runtime: DeliveryRuntime) -> &'static str {
    match runtime {
        DeliveryRuntime::ScriptedAcceptance => "SCRIPTED_ACCEPTANCE",
        DeliveryRuntime::OfficialCodexAppServer => "OFFICIAL_CODEX_APP_SERVER",
    }
}

fn parse_delivery_runtime(value: &str) -> Option<DeliveryRuntime> {
    match value {
        "SCRIPTED_ACCEPTANCE" => Some(DeliveryRuntime::ScriptedAcceptance),
        "OFFICIAL_CODEX_APP_SERVER" => Some(DeliveryRuntime::OfficialCodexAppServer),
        _ => None,
    }
}

const fn delivery_stage_name(stage: DeliveryStage) -> &'static str {
    match stage {
        DeliveryStage::Intent => "INTENT",
        DeliveryStage::WorkspacePrepare => "WORKSPACE_PREPARE",
        DeliveryStage::Codex => "CODEX",
        DeliveryStage::ScopeVerification => "SCOPE_VERIFICATION",
        DeliveryStage::FixedTest => "FIXED_TEST",
        DeliveryStage::GitCommit => "GIT_COMMIT",
        DeliveryStage::Outcome => "OUTCOME",
        DeliveryStage::Receipt => "RECEIPT",
    }
}

fn parse_delivery_stage(value: &str) -> Option<DeliveryStage> {
    match value {
        "INTENT" => Some(DeliveryStage::Intent),
        "WORKSPACE_PREPARE" => Some(DeliveryStage::WorkspacePrepare),
        "CODEX" => Some(DeliveryStage::Codex),
        "SCOPE_VERIFICATION" => Some(DeliveryStage::ScopeVerification),
        "FIXED_TEST" => Some(DeliveryStage::FixedTest),
        "GIT_COMMIT" => Some(DeliveryStage::GitCommit),
        "OUTCOME" => Some(DeliveryStage::Outcome),
        "RECEIPT" => Some(DeliveryStage::Receipt),
        _ => None,
    }
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

#[allow(clippy::too_many_lines)]
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
    match (
        outcome_event.outcome(),
        outcome_event.reason_code().as_str(),
    ) {
        (LedgerOutcome::Passed, "TASK032_DELIVERY_COMPLETED") => {
            let receipt = validate_result_evidence(
                outcome_event.diagnostic().map(Diagnostic::value),
                outcome_event.subject_digest(),
                intent_event.subject_digest(),
                &intent_evidence,
            )
            .or_else(|| {
                typed_legacy_receipt(
                    outcome_event.diagnostic().map(Diagnostic::value),
                    outcome_event.subject_digest(),
                    intent_event.subject_digest(),
                    &intent_evidence,
                )
            });
            receipt.map_or_else(
                || inspection(Projection::Ambiguous),
                |receipt| StreamInspection {
                    projection: Projection::Completed,
                    receipt: Some(receipt),
                },
            )
        }
        (LedgerOutcome::Failed, "TASK032_DELIVERY_FAILED")
            if typed_terminal_status(
                outcome_event.diagnostic().map(Diagnostic::value),
                outcome_event.subject_digest(),
                intent_event.subject_digest(),
                &intent_evidence,
            ) == Some(DeliveryTerminalStatus::Failed) =>
        {
            inspection(Projection::Failed)
        }
        (LedgerOutcome::Cancelled, "TASK032_DELIVERY_RECONCILIATION")
            if typed_terminal_status(
                outcome_event.diagnostic().map(Diagnostic::value),
                outcome_event.subject_digest(),
                intent_event.subject_digest(),
                &intent_evidence,
            ) == Some(DeliveryTerminalStatus::ReconciliationRequired) =>
        {
            inspection(Projection::Reconciliation)
        }
        _ => inspection(Projection::Ambiguous),
    }
}

fn typed_terminal_status(
    value: Option<&CanonicalValue>,
    outcome_digest: &ContentDigest,
    intent_digest: &ContentDigest,
    intent: &IntentEvidenceProjection,
) -> Option<DeliveryTerminalStatus> {
    if intent.format != IntentEnvelopeFormat::TypedV2 {
        return None;
    }
    let fields = validate_typed_envelope(value?, outcome_digest, intent_digest, None)?;
    if fields.get("configuration_digest")? != &intent.configuration_digest.as_ref()?.as_str() {
        return None;
    }
    parse_terminal_status(fields.get("status")?)
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
    format: IntentEnvelopeFormat,
    configuration_digest: Option<ContentDigest>,
    launcher_path: String,
    launcher_sha256: String,
    repository_path: String,
    test_command_id: String,
    version: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IntentEnvelopeFormat {
    LegacyV1,
    TypedV2,
}

fn validate_intent_evidence(
    value: Option<&CanonicalValue>,
    subject_digest: &ContentDigest,
) -> Option<IntentEvidenceProjection> {
    let value = value?;
    validate_typed_intent_evidence(value, subject_digest)
        .or_else(|| validate_legacy_intent_evidence(value, subject_digest))
}

fn validate_typed_intent_evidence(
    value: &CanonicalValue,
    subject_digest: &ContentDigest,
) -> Option<IntentEvidenceProjection> {
    let fields = string_fields(value, 10)?;
    if fields.get("envelope_version")? != &TYPED_INTENT_ENVELOPE_VERSION
        || !is_lower_hex(fields.get("configuration_digest")?, 64)
        || fields.get("changed_path")? != &"answer.txt"
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
        format: IntentEnvelopeFormat::TypedV2,
        configuration_digest: parse_digest(fields.get("configuration_digest")?),
        launcher_path: (*fields.get("launcher_path")?).to_owned(),
        launcher_sha256: (*fields.get("launcher_sha256")?).to_owned(),
        repository_path: (*fields.get("repository_path")?).to_owned(),
        test_command_id: (*fields.get("test_command_id")?).to_owned(),
        version: (*fields.get("version")?).to_owned(),
    })
}

fn validate_legacy_intent_evidence(
    value: &CanonicalValue,
    subject_digest: &ContentDigest,
) -> Option<IntentEvidenceProjection> {
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
        format: IntentEnvelopeFormat::LegacyV1,
        configuration_digest: None,
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
    let valid = intent.format == IntentEnvelopeFormat::LegacyV1
        && fields.get("changed_path") == Some(&"answer.txt")
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
        Projection::Pending | Projection::Reconciliation | Projection::Ambiguous => {
            DeliveryStatus::ReconciliationRequired
        }
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

fn map_port_ledger_error(stage: DeliveryStage, error: DeliveryLedgerError) -> DeliveryPortError {
    let (kind, certainty) = match error.kind() {
        DeliveryLedgerErrorKind::CommitOutcomeUnknown
        | DeliveryLedgerErrorKind::MutationDeadlineAmbiguous
        | DeliveryLedgerErrorKind::ReconciliationRequired
        | DeliveryLedgerErrorKind::CheckpointCorrupt
        | DeliveryLedgerErrorKind::RetainedRowCorrupt
        | DeliveryLedgerErrorKind::PersistedIntentCorrupt
        | DeliveryLedgerErrorKind::OutcomeAppendCorrupt
        | DeliveryLedgerErrorKind::PhysicalStateMismatch => (
            PortErrorKind::Ambiguous,
            DeliveryFailureCertainty::Ambiguous,
        ),
        DeliveryLedgerErrorKind::InvalidBinding | DeliveryLedgerErrorKind::EvidenceInvalid => {
            (PortErrorKind::Malformed, DeliveryFailureCertainty::Known)
        }
        DeliveryLedgerErrorKind::SchemaRejected => (
            PortErrorKind::CapabilityMismatch,
            DeliveryFailureCertainty::Known,
        ),
        DeliveryLedgerErrorKind::DeadlineExpired if stage == DeliveryStage::Outcome => (
            PortErrorKind::Ambiguous,
            DeliveryFailureCertainty::Ambiguous,
        ),
        DeliveryLedgerErrorKind::DeadlineExpired => {
            (PortErrorKind::Timeout, DeliveryFailureCertainty::Known)
        }
        DeliveryLedgerErrorKind::ConnectFailed | DeliveryLedgerErrorKind::LedgerRejected => {
            (PortErrorKind::Unavailable, DeliveryFailureCertainty::Known)
        }
    };
    delivery_port_error(stage, kind, certainty, ledger_error_code(error.kind()))
}

const fn ledger_error_code(kind: DeliveryLedgerErrorKind) -> &'static str {
    match kind {
        DeliveryLedgerErrorKind::InvalidBinding => "DELIVERY_LEDGER_INVALID_BINDING",
        DeliveryLedgerErrorKind::ConnectFailed => "DELIVERY_LEDGER_CONNECT_FAILED",
        DeliveryLedgerErrorKind::SchemaRejected => "DELIVERY_LEDGER_SCHEMA_REJECTED",
        DeliveryLedgerErrorKind::PhysicalStateMismatch => "DELIVERY_LEDGER_PHYSICAL_STATE_MISMATCH",
        DeliveryLedgerErrorKind::CheckpointCorrupt => "DELIVERY_LEDGER_CHECKPOINT_CORRUPT",
        DeliveryLedgerErrorKind::RetainedRowCorrupt => "DELIVERY_LEDGER_RETAINED_ROW_CORRUPT",
        DeliveryLedgerErrorKind::PersistedIntentCorrupt => {
            "DELIVERY_LEDGER_PERSISTED_INTENT_CORRUPT"
        }
        DeliveryLedgerErrorKind::OutcomeAppendCorrupt => "DELIVERY_LEDGER_OUTCOME_APPEND_CORRUPT",
        DeliveryLedgerErrorKind::LedgerRejected => "DELIVERY_LEDGER_REJECTED",
        DeliveryLedgerErrorKind::CommitOutcomeUnknown => "DELIVERY_LEDGER_COMMIT_OUTCOME_UNKNOWN",
        DeliveryLedgerErrorKind::MutationDeadlineAmbiguous => {
            "DELIVERY_LEDGER_MUTATION_DEADLINE_AMBIGUOUS"
        }
        DeliveryLedgerErrorKind::ReconciliationRequired => {
            "DELIVERY_LEDGER_RECONCILIATION_REQUIRED"
        }
        DeliveryLedgerErrorKind::EvidenceInvalid => "DELIVERY_LEDGER_EVIDENCE_INVALID",
        DeliveryLedgerErrorKind::DeadlineExpired => "DELIVERY_LEDGER_DEADLINE_EXPIRED",
    }
}

fn ledger_binding_error(stage: DeliveryStage) -> DeliveryPortError {
    delivery_port_error(
        stage,
        PortErrorKind::Denied,
        DeliveryFailureCertainty::Known,
        "DELIVERY_LEDGER_BINDING_MISMATCH",
    )
}

fn delivery_port_error(
    stage: DeliveryStage,
    kind: PortErrorKind,
    certainty: DeliveryFailureCertainty,
    code: impl Into<String>,
) -> DeliveryPortError {
    DeliveryPortError::new(stage, kind, certainty, code)
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

fn finish_mutation_at<T>(
    result: Result<T, DeliveryLedgerError>,
    deadline: Instant,
    observed_at: Instant,
) -> Result<T, DeliveryLedgerError> {
    if observed_at >= deadline {
        Err(delivery_error(
            DeliveryLedgerErrorKind::MutationDeadlineAmbiguous,
        ))
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lattice_contracts::{AttemptId, CONTRACT_VERSION, DeliveryProfile, Invocation, RequestId};
    use lattice_task_ledger::{apply_append_plan, plan_append};

    fn digest(byte: char) -> ContentDigest {
        ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("valid digest")
    }

    fn fixture_request(label: &str) -> DeliveryRunRequest {
        let invocation = Invocation::new(
            CONTRACT_VERSION,
            RequestId::new(format!("legacy-replay-{label}")).expect("request id"),
            TaskId::new("TASK-032").expect("task id"),
            AttemptId::new(format!("legacy-attempt-{label}")).expect("attempt id"),
            ProjectSnapshotId::new("task032-delivery:snapshot:1").expect("snapshot id"),
            digest('8'),
        )
        .expect("invocation");
        DeliveryRunRequest::new(
            invocation,
            DeliveryProfile::Task032CodexPostgres,
            digest('9'),
        )
        .expect("request")
    }

    fn legacy_intent_value() -> CanonicalValue {
        CanonicalValue::Object(vec![
            (
                "changed_path".to_owned(),
                CanonicalValue::String("answer.txt".to_owned()),
            ),
            (
                "codex_home".to_owned(),
                CanonicalValue::String(r"C:\delivery\codex-home".to_owned()),
            ),
            (
                "launcher_path".to_owned(),
                CanonicalValue::String(r"C:\tools\codex.exe".to_owned()),
            ),
            (
                "launcher_sha256".to_owned(),
                CanonicalValue::String("a".repeat(64)),
            ),
            (
                "repository_path".to_owned(),
                CanonicalValue::String(r"C:\delivery\repo".to_owned()),
            ),
            (
                "schema_directory".to_owned(),
                CanonicalValue::String(r"C:\delivery\schema".to_owned()),
            ),
            (
                "test_command_id".to_owned(),
                CanonicalValue::String("git-diff-no-index-exact-answer-v1".to_owned()),
            ),
            (
                "version".to_owned(),
                CanonicalValue::String("codex-cli 0.144.6".to_owned()),
            ),
        ])
    }

    fn apply_fixture_command(
        stream: &VerifiedStream,
        command_id: &str,
        kind: LedgerEventKind,
        outcome: LedgerOutcome,
        reason: &str,
        subject_digest: ContentDigest,
        diagnostic: CanonicalValue,
    ) -> VerifiedStream {
        let command = AppendCommand::new(
            stream.head().clone(),
            CommandId::new(command_id).expect("command id"),
            CorrelationId::new(CORRELATION_ID).expect("correlation id"),
            "2026-08-05T00:00:00Z",
            kind,
            ActorId::new(ACTOR_ID).expect("actor id"),
            ActionId::new(ACTION_ID).expect("action id"),
            outcome,
            ReasonCode::new(reason).expect("reason code"),
            subject_digest,
            Some(Diagnostic::new(diagnostic).expect("diagnostic")),
            None,
        )
        .expect("append command");
        let plan = plan_append(stream, command).expect("append plan");
        apply_append_plan(stream, &plan).expect("apply append plan")
    }

    fn replace_string_field(value: &mut CanonicalValue, key: &str, replacement: String) {
        let CanonicalValue::Object(fields) = value else {
            panic!("canonical object");
        };
        let (_, field) = fields
            .iter_mut()
            .find(|(name, _)| name == key)
            .expect("field");
        *field = CanonicalValue::String(replacement);
    }

    fn typed_failed_stream(
        request: &DeliveryRunRequest,
        mutate_outcome: impl FnOnce(&mut CanonicalValue),
    ) -> VerifiedStream {
        let intent_value = DeliveryIntentEvidence::new(
            request,
            r"C:\tools\codex.exe",
            "codex-cli 0.144.6",
            "a".repeat(64),
            r"C:\delivery\schema",
            r"C:\delivery\codex-home",
            r"C:\delivery\repo",
        )
        .expect("typed intent")
        .canonical_value();
        let intent_digest = delivery_digest("lattice.runtime.codex-delivery-intent", &intent_value)
            .expect("intent digest");
        let vacant = VerifiedStream::vacant(
            delivery_identity().expect("delivery identity"),
            RuntimeKind::Live,
        )
        .expect("vacant stream");
        let pending = apply_fixture_command(
            &vacant,
            INTENT_COMMAND_ID,
            LedgerEventKind::EffectIntent,
            LedgerOutcome::Recorded,
            "TASK032_CODEX_INTENT",
            intent_digest.clone(),
            intent_value,
        );
        let intent = DurableIntentEvidence::new(request, intent_digest).expect("intent contract");
        let outcome = TypedDeliveryOutcomeRequest::failed(
            request,
            &intent,
            DeliveryStage::FixedTest,
            "FIXED_TEST_FAILED",
        )
        .expect("failed outcome");
        let mut outcome_value = typed_outcome_value(&outcome).expect("typed outcome");
        mutate_outcome(&mut outcome_value);
        let outcome_digest =
            delivery_digest("lattice.runtime.typed-delivery-outcome", &outcome_value)
                .expect("outcome digest");
        apply_fixture_command(
            &pending,
            OUTCOME_COMMAND_ID,
            LedgerEventKind::EffectOutcome,
            LedgerOutcome::Failed,
            "TASK032_DELIVERY_FAILED",
            outcome_digest,
            outcome_value,
        )
    }

    fn legacy_completed_stream_with(
        mutate_result: impl FnOnce(&mut CanonicalValue),
    ) -> VerifiedStream {
        let intent_value = legacy_intent_value();
        let intent_digest = delivery_digest("lattice.runtime.codex-delivery-intent", &intent_value)
            .expect("intent digest");
        let vacant = VerifiedStream::vacant(
            delivery_identity().expect("delivery identity"),
            RuntimeKind::Live,
        )
        .expect("vacant stream");
        let pending = apply_fixture_command(
            &vacant,
            INTENT_COMMAND_ID,
            LedgerEventKind::EffectIntent,
            LedgerOutcome::Recorded,
            "TASK032_CODEX_INTENT",
            intent_digest.clone(),
            intent_value,
        );
        let success = DeliverySuccessEvidence::new(
            intent_digest,
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
        let mut result_value = success.canonical_value();
        mutate_result(&mut result_value);
        let result_digest = delivery_digest("lattice.runtime.codex-delivery-result", &result_value)
            .expect("result digest");
        apply_fixture_command(
            &pending,
            OUTCOME_COMMAND_ID,
            LedgerEventKind::EffectOutcome,
            LedgerOutcome::Passed,
            "TASK032_DELIVERY_COMPLETED",
            result_digest,
            result_value,
        )
    }

    #[test]
    fn restart_replay_keeps_legacy_and_typed_receipts_distinct() {
        let stream = legacy_completed_stream_with(|_| {});
        let legacy = legacy_completed_receipt_from_stream(&stream).expect("legacy receipt");

        assert_eq!(LEGACY_RECEIPT_FORMAT, "legacy-delivery-result-v1");
        assert_eq!(legacy.commit_sha(), "d".repeat(40));
        assert!(typed_receipt_from_stream(&stream, &fixture_request("exact")).is_err());
    }

    #[test]
    fn new_typed_terminal_envelopes_use_v2_semantics() {
        let request = fixture_request("typed-v2");
        let intent = DurableIntentEvidence::new(&request, digest('c')).expect("intent");
        let outcome = TypedDeliveryOutcomeRequest::failed(
            &request,
            &intent,
            DeliveryStage::FixedTest,
            "FIXED_TEST_FAILED",
        )
        .expect("failed outcome");
        let value = typed_outcome_value(&outcome).expect("typed outcome");
        let CanonicalValue::Object(fields) = value else {
            panic!("typed outcome object");
        };
        let version = fields
            .iter()
            .find(|(key, _)| key == "envelope_version")
            .map(|(_, value)| value);

        assert_eq!(
            version,
            Some(&CanonicalValue::String("typed-delivery-v2".to_owned()))
        );
    }

    #[test]
    fn status_reconstructs_v2_request_and_rejects_rehashed_configuration_substitution() {
        let request = fixture_request("status-v2");
        let stream = typed_failed_stream(&request, |_| {});
        let reconstructed = typed_request_from_stream(
            &stream,
            request.invocation(),
            DeliveryProfile::Task032CodexPostgres,
        )
        .expect("typed v2 request");
        assert_eq!(reconstructed, request);

        let substituted = typed_failed_stream(&request, |value| {
            replace_string_field(value, "configuration_digest", "f".repeat(64));
        });
        assert!(
            typed_request_from_stream(
                &substituted,
                request.invocation(),
                DeliveryProfile::Task032CodexPostgres,
            )
            .is_err(),
            "a rehashed outcome must not override the pre-effect configuration binding"
        );
    }

    #[test]
    fn weak_typed_v1_is_not_promoted_to_exact_v2_status() {
        let request = fixture_request("weak-v1");
        let intent_value = legacy_intent_value();
        let intent_digest = delivery_digest("lattice.runtime.codex-delivery-intent", &intent_value)
            .expect("legacy intent digest");
        let vacant = VerifiedStream::vacant(
            delivery_identity().expect("delivery identity"),
            RuntimeKind::Live,
        )
        .expect("vacant stream");
        let pending = apply_fixture_command(
            &vacant,
            INTENT_COMMAND_ID,
            LedgerEventKind::EffectIntent,
            LedgerOutcome::Recorded,
            "TASK032_CODEX_INTENT",
            intent_digest.clone(),
            intent_value,
        );
        let intent = DurableIntentEvidence::new(&request, intent_digest).expect("intent contract");
        let outcome = TypedDeliveryOutcomeRequest::failed(
            &request,
            &intent,
            DeliveryStage::FixedTest,
            "FIXED_TEST_FAILED",
        )
        .expect("failed outcome");
        let mut value = typed_outcome_value(&outcome).expect("typed outcome");
        replace_string_field(
            &mut value,
            "envelope_version",
            "typed-delivery-v1".to_owned(),
        );
        let outcome_digest =
            delivery_digest("lattice.runtime.typed-delivery-outcome", &value).expect("digest");
        let stream = apply_fixture_command(
            &pending,
            OUTCOME_COMMAND_ID,
            LedgerEventKind::EffectOutcome,
            LedgerOutcome::Failed,
            "TASK032_DELIVERY_FAILED",
            outcome_digest,
            value,
        );

        assert!(
            typed_request_from_stream(
                &stream,
                request.invocation(),
                DeliveryProfile::Task032CodexPostgres,
            )
            .is_err()
        );
    }

    #[test]
    fn legacy_restart_replay_rejects_cross_bound_and_tampered_evidence() {
        let expected = fixture_request("expected");
        let foreign = fixture_request("foreign");
        assert!(validate_status_binding(&expected, &expected.status_request()).is_ok());
        assert!(validate_status_binding(&expected, &foreign.status_request()).is_err());

        let stream = legacy_completed_stream_with(|value| {
            let CanonicalValue::Object(fields) = value else {
                panic!("result object");
            };
            let (_, repository_path) = fields
                .iter_mut()
                .find(|(key, _)| key == "repository_path")
                .expect("repository path");
            *repository_path = CanonicalValue::String(r"C:\foreign\repo".to_owned());
        });
        assert!(legacy_completed_receipt_from_stream(&stream).is_none());
    }

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
        let intent_value = legacy_intent_value();
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
    #[allow(clippy::too_many_lines)]
    fn typed_postgres_wrapper_reconstructs_the_complete_success_chain() {
        use lattice_contracts::{
            AttemptId, CONTRACT_VERSION, CodexDeliveryEvidence, CodexDeliveryRequest,
            CompletedDeliveryEvidence, DeliveryOutcomeRequest, DeliveryProfile, DeliveryRunRequest,
            DeliveryRuntime, DurableIntentEvidence, FixedTestEvidence, GitCommitEvidence,
            Invocation, PreparedWorkspaceEvidence, RequestId, WorkspaceChangeEvidence,
        };
        use lattice_ports::DeliveryLedgerPort;

        fn assert_port<T: DeliveryLedgerPort>() {}

        fn digest(byte: char) -> ContentDigest {
            ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("valid digest")
        }

        assert_port::<PostgresDeliveryLedgerAdapter>();

        let invocation = Invocation::new(
            CONTRACT_VERSION,
            RequestId::new("typed-ledger-request").expect("request id"),
            TaskId::new("TASK-032").expect("task id"),
            AttemptId::new("attempt-1").expect("attempt id"),
            ProjectSnapshotId::new("snapshot-1").expect("snapshot id"),
            digest('a'),
        )
        .expect("invocation");
        let request = DeliveryRunRequest::new(
            invocation,
            DeliveryProfile::Task032CodexPostgres,
            digest('b'),
        )
        .expect("request");
        let intent = DurableIntentEvidence::new(&request, digest('c')).expect("intent");
        let workspace = PreparedWorkspaceEvidence::new(
            &request,
            &intent,
            "workspace-1",
            r"C:\delivery\repo",
            "1".repeat(40),
            digest('d'),
        )
        .expect("workspace");
        let codex_request =
            CodexDeliveryRequest::new(request.clone(), intent.clone(), workspace.clone())
                .expect("codex request");
        let codex = CodexDeliveryEvidence::new(
            &codex_request,
            DeliveryRuntime::OfficialCodexAppServer,
            r"C:\tools\codex.exe",
            "codex-cli 0.144.6",
            digest('e'),
            digest('f'),
            2,
            "thread-1",
            "turn-1",
            digest('7'),
        )
        .expect("codex");
        let changes = WorkspaceChangeEvidence::new(
            &request,
            &intent,
            &workspace,
            &codex,
            digest('8'),
            digest('9'),
        )
        .expect("changes");
        let test = FixedTestEvidence::new(&request, &changes, digest('a')).expect("test");
        let git = GitCommitEvidence::new(
            &request,
            &changes,
            &test,
            "1".repeat(40),
            "2".repeat(40),
            digest('b'),
        )
        .expect("git");
        let completed = CompletedDeliveryEvidence::new(
            request.clone(),
            intent.clone(),
            workspace,
            codex,
            changes,
            test,
            git,
        )
        .expect("completed");
        let outcome = DeliveryOutcomeRequest::completed(&request, completed).expect("outcome");
        let value = typed_outcome_value(&outcome).expect("typed envelope");
        let outcome_digest = delivery_digest("lattice.runtime.typed-delivery-outcome", &value)
            .expect("outcome digest");

        let receipt =
            reconstruct_typed_receipt(&request, intent.intent_digest(), &outcome_digest, &value)
                .expect("restart receipt");

        assert_eq!(
            receipt.status(),
            lattice_contracts::DeliveryTerminalStatus::Completed
        );
        let completed = receipt
            .outcome()
            .request()
            .completed_evidence()
            .expect("completed chain");
        assert_eq!(
            completed.codex().runtime(),
            DeliveryRuntime::OfficialCodexAppServer
        );
        assert_eq!(completed.git().commit(), "2".repeat(40));
    }

    #[test]
    fn typed_failure_and_reconciliation_envelopes_remain_distinct_and_never_success() {
        use lattice_contracts::{
            AttemptId, CONTRACT_VERSION, DeliveryOutcomeRequest, DeliveryProfile,
            DeliveryRunRequest, DeliveryStage, DeliveryTerminalStatus, DurableIntentEvidence,
            Invocation, RequestId,
        };

        fn digest(byte: char) -> ContentDigest {
            ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("valid digest")
        }

        let invocation = Invocation::new(
            CONTRACT_VERSION,
            RequestId::new("typed-ledger-failure").expect("request id"),
            TaskId::new("TASK-032").expect("task id"),
            AttemptId::new("attempt-1").expect("attempt id"),
            ProjectSnapshotId::new("snapshot-1").expect("snapshot id"),
            digest('a'),
        )
        .expect("invocation");
        let request = DeliveryRunRequest::new(
            invocation,
            DeliveryProfile::Task032CodexPostgres,
            digest('b'),
        )
        .expect("request");
        let intent = DurableIntentEvidence::new(&request, digest('c')).expect("intent");
        let failed = DeliveryOutcomeRequest::failed(
            &request,
            &intent,
            DeliveryStage::FixedTest,
            "FIXED_TEST_FAILED",
        )
        .expect("failed outcome");
        let reconciliation = DeliveryOutcomeRequest::reconciliation_required(
            &request,
            &intent,
            DeliveryStage::GitCommit,
            "COMMIT_OUTCOME_UNKNOWN",
        )
        .expect("reconciliation outcome");

        let failed_value = typed_outcome_value(&failed).expect("failed envelope");
        let reconciliation_value =
            typed_outcome_value(&reconciliation).expect("reconciliation envelope");
        let failed_digest =
            delivery_digest("lattice.runtime.typed-delivery-outcome", &failed_value)
                .expect("failed digest");
        let reconciliation_digest = delivery_digest(
            "lattice.runtime.typed-delivery-outcome",
            &reconciliation_value,
        )
        .expect("reconciliation digest");
        let failed_receipt = reconstruct_typed_receipt(
            &request,
            intent.intent_digest(),
            &failed_digest,
            &failed_value,
        )
        .expect("failed receipt");
        let reconciliation_receipt = reconstruct_typed_receipt(
            &request,
            intent.intent_digest(),
            &reconciliation_digest,
            &reconciliation_value,
        )
        .expect("reconciliation receipt");

        assert_eq!(failed_receipt.status(), DeliveryTerminalStatus::Failed);
        assert_eq!(
            reconciliation_receipt.status(),
            DeliveryTerminalStatus::ReconciliationRequired
        );
        assert_ne!(failed_digest, reconciliation_digest);
        assert!(
            failed_receipt
                .outcome()
                .request()
                .completed_evidence()
                .is_none()
        );
        assert!(
            reconciliation_receipt
                .outcome()
                .request()
                .completed_evidence()
                .is_none()
        );
    }

    #[test]
    fn post_mutation_deadline_is_ambiguous_even_when_execute_returned_success() {
        let deadline = Instant::now();
        let error = finish_mutation_at(Ok(()), deadline, deadline)
            .expect_err("a completed mutation at the deadline is not a timely success");
        assert_eq!(
            error.kind(),
            DeliveryLedgerErrorKind::MutationDeadlineAmbiguous
        );

        let port_error = map_port_ledger_error(DeliveryStage::Intent, error);
        assert_eq!(port_error.kind(), PortErrorKind::Ambiguous);
        assert_eq!(port_error.certainty(), DeliveryFailureCertainty::Ambiguous);
        assert_eq!(
            port_error.code(),
            "DELIVERY_LEDGER_MUTATION_DEADLINE_AMBIGUOUS"
        );
    }

    #[test]
    fn post_mutation_deadline_overrides_a_known_inner_error_but_read_deadline_stays_known() {
        let deadline = Instant::now();
        let inner = Err::<(), _>(delivery_error(DeliveryLedgerErrorKind::LedgerRejected));
        let mutation_error = finish_mutation_at(inner, deadline, deadline)
            .expect_err("post-mutation deadline must dominate inner certainty");
        let mutation_port = map_port_ledger_error(DeliveryStage::Outcome, mutation_error);
        assert_eq!(mutation_port.kind(), PortErrorKind::Ambiguous);
        assert_eq!(
            mutation_port.certainty(),
            DeliveryFailureCertainty::Ambiguous
        );

        let pre_mutation_port = map_port_ledger_error(DeliveryStage::Intent, deadline_error());
        assert_eq!(pre_mutation_port.kind(), PortErrorKind::Timeout);
        assert_eq!(
            pre_mutation_port.certainty(),
            DeliveryFailureCertainty::Known
        );

        let outcome_port = map_port_ledger_error(DeliveryStage::Outcome, deadline_error());
        assert_eq!(outcome_port.kind(), PortErrorKind::Ambiguous);
        assert_eq!(
            outcome_port.certainty(),
            DeliveryFailureCertainty::Ambiguous
        );

        let read_port = map_port_ledger_error(DeliveryStage::Receipt, deadline_error());
        assert_eq!(read_port.kind(), PortErrorKind::Timeout);
        assert_eq!(read_port.certainty(), DeliveryFailureCertainty::Known);
    }

    #[test]
    fn task019_application_name_is_used_for_schema_compatibility() {
        assert_eq!(APPLICATION_NAME, "lattice-devos-task019");
    }

    #[test]
    fn fixed_runtime_connection_factory_binds_one_validated_target() {
        use postgres::config::Host;

        let binding =
            DeliveryDatabaseBinding::new("127.0.0.1", 55_432, "0123456789abcdef0123456789abcdef")
                .expect("fixed database binding");
        let deadline = Instant::now()
            .checked_add(std::time::Duration::from_secs(30))
            .expect("deadline");

        let config = fixed_runtime_config(&binding, "test-password", deadline)
            .expect("fixed runtime config");

        assert!(matches!(
            config.get_hosts(),
            [Host::Tcp(host)] if host == "127.0.0.1"
        ));
        assert_eq!(config.get_ports(), [55_432]);
        assert_eq!(config.get_user(), Some("lattice_runtime_login"));
        assert_eq!(config.get_dbname(), Some("lattice_task019_01234567_base"));
        assert_eq!(config.get_application_name(), Some(APPLICATION_NAME));
        assert_eq!(config.get_ssl_mode(), SslMode::Disable);
        let options = config.get_options().expect("fixed timeout options");
        for option in [
            "statement_timeout=",
            "lock_timeout=",
            "idle_in_transaction_session_timeout=",
        ] {
            assert!(options.contains(option), "missing {option}");
        }
        let _: fn(
            &DeliveryDatabaseBinding,
            &str,
            Instant,
        ) -> Result<postgres::Client, DeliveryLedgerError> = connect_fixed_runtime_client;
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
