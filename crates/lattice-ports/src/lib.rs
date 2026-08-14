//! Abstract I/O ports for LATTICE orchestration.

use std::error::Error;
use std::fmt;

use lattice_contracts::{
    CodeSnapshotEvidence, CodexDeliveryEvidence, CodexDeliveryRequest, CodexEvidence,
    CodexRunRequest, Component, DeliveryOutcomeEvidence, DeliveryOutcomeRequest, DeliveryReceipt,
    DeliveryRunRequest, DeliveryStage, DeliveryStatusRequest, DurableIntentEvidence,
    FixedTestEvidence, GatewayActorId, GatewayCommandId, GatewayPeerContext, GatewayReply,
    GatewayRequest, GitCommitEvidence, GraphMemoryPersistenceEvidence, GraphMemoryReceipt,
    GraphMemoryRunRequest, GraphifyBuildRequest, GraphifyEvidence, GraphifyRawEvidence,
    HermesEvidence, HermesReflectionCandidate, HermesReflectionReceipt, HermesResearchRequest,
    MemoryRetrievalPlan, NormalizedGraphAnalysis, PreparedWorkspaceEvidence, ProjectId, RequestId,
    StorePhysicalHead, StoreScope, StoreTransactionReceipt, StoreTransactionRequest,
    WorkspaceChangeEvidence,
};
use lattice_task_domain::TaskState;

/// Result type returned by every LATTICE port.
pub type PortResult<T> = Result<T, PortError>;

/// Result returned by the inbound Rust-core gateway service.
///
/// This boundary is not an external adapter port, so its errors deliberately
/// do not carry a [`Component`] that could falsely attribute a core routing or
/// reply-binding failure to `OpenClaw` or another adapter.
pub type GatewayServiceResult<T> = Result<T, GatewayServiceError>;

/// Result returned by the typed physical control-store boundary.
pub type ControlStoreResult<T> = Result<T, ControlStoreError>;

/// Result returned by each typed delivery effect port.
pub type DeliveryPortResult<T> = Result<T, DeliveryPortError>;

/// Result returned by each exact graph-memory effect port.
pub type GraphMemoryPortResult<T> = Result<T, GraphMemoryPortError>;

/// Result returned by the authoritative Task lifecycle repository boundary.
pub type TaskLifecycleResult<T> = Result<T, TaskLifecycleError>;

/// Result returned by the single bounded task execution port.
pub type ControlledTaskExecutionResult<T> = Result<T, ControlledTaskExecutionError>;

/// Whether a failed controlled execution is known not to have completed or
/// requires reconciliation before any retry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlledTaskExecutionErrorKind {
    Known,
    Ambiguous,
}

/// Secret-free controlled execution failure returned to the orchestrator.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlledTaskExecutionError {
    kind: ControlledTaskExecutionErrorKind,
    code: &'static str,
}

impl ControlledTaskExecutionError {
    #[must_use]
    pub const fn new(kind: ControlledTaskExecutionErrorKind, code: &'static str) -> Self {
        Self { kind, code }
    }

    #[must_use]
    pub const fn kind(&self) -> ControlledTaskExecutionErrorKind {
        self.kind
    }

    #[must_use]
    pub const fn code(&self) -> &'static str {
        self.code
    }
}

impl fmt::Display for ControlledTaskExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code)
    }
}

impl Error for ControlledTaskExecutionError {}

/// Existing-orchestrator effect boundary for one server-owned bounded task.
/// The current Writer Lease head is supplied to the adapter but never exposed
/// through MCP or accepted from the GPT caller.
pub trait ControlledTaskExecutionPort {
    /// Executes the one server-owned task under the exact current writer head.
    ///
    /// # Errors
    ///
    /// Returns a bounded known or ambiguous execution failure.
    fn execute(
        &mut self,
        binding: &lattice_contracts::SubjectBinding,
        writer_authority: &lattice_contracts::WriterLeaseAuthorityHead,
        writer_guard: &mut dyn WriterAuthorityGuardPort,
    ) -> ControlledTaskExecutionResult<lattice_contracts::ContentDigest>;
}

/// Currentness assertion supplied by Orchestrator from the same injected
/// Writer Lease repository that allocated the fence. Execution adapters may
/// request checks but cannot acquire, release, or replace writer authority.
pub trait WriterAuthorityGuardPort {
    /// Proves the exact authority is still current at one mutation boundary.
    ///
    /// # Errors
    ///
    /// Returns a bounded known mismatch or ambiguous owner failure.
    fn assert_current(
        &mut self,
        expected: &lattice_contracts::WriterLeaseAuthorityHead,
    ) -> ControlledTaskExecutionResult<()>;
}

/// Closed durable Task lifecycle repository failure classes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskLifecycleErrorKind {
    Rejected,
    Unavailable,
    Ambiguous,
    Corrupt,
}

/// Bounded Task lifecycle failure without database or task-source contents.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskLifecycleError {
    kind: TaskLifecycleErrorKind,
    code: &'static str,
}

impl TaskLifecycleError {
    #[must_use]
    pub const fn new(kind: TaskLifecycleErrorKind, code: &'static str) -> Self {
        Self { kind, code }
    }

    #[must_use]
    pub const fn kind(&self) -> TaskLifecycleErrorKind {
        self.kind
    }

    #[must_use]
    pub const fn code(&self) -> &'static str {
        self.code
    }
}

impl fmt::Display for TaskLifecycleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code)
    }
}

impl Error for TaskLifecycleError {}

/// Replay-derived authoritative Task lifecycle projection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutonomyDisposition {
    Proceed,
    AskUser,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutonomyReason {
    RoutineAuthorized,
    NewUserDecision,
    NewAuthority,
    HighRiskOrIrreversible,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutonomyModel {
    GovernedCodexWriter,
    NoModel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutonomyVerification {
    FocusedChecks,
    BuildAndFocusedChecks,
    ReadOnlyEvidence,
}

/// Internal-only replay projection of one verified autonomy receipt event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AutonomyReceiptProjection {
    receipt_digest: lattice_contracts::ContentDigest,
    authority_digest: lattice_contracts::ContentDigest,
    event_digest: lattice_contracts::ContentDigest,
    observed_state: TaskState,
    disposition: AutonomyDisposition,
    reason: AutonomyReason,
    model: Option<AutonomyModel>,
    verification: Option<AutonomyVerification>,
}

impl AutonomyReceiptProjection {
    /// Constructs one closed projection from Task-Ledger-verified values.
    ///
    /// # Errors
    ///
    /// Rejects non-Draft observations and impossible decision shapes.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        receipt_digest: lattice_contracts::ContentDigest,
        authority_digest: lattice_contracts::ContentDigest,
        event_digest: lattice_contracts::ContentDigest,
        observed_state: TaskState,
        disposition: AutonomyDisposition,
        reason: AutonomyReason,
        model: Option<AutonomyModel>,
        verification: Option<AutonomyVerification>,
    ) -> TaskLifecycleResult<Self> {
        let decision_shape_valid = matches!(
            (disposition, reason, model, verification),
            (
                AutonomyDisposition::Proceed,
                AutonomyReason::RoutineAuthorized,
                Some(_),
                Some(_)
            ) | (
                AutonomyDisposition::AskUser,
                AutonomyReason::NewUserDecision
                    | AutonomyReason::NewAuthority
                    | AutonomyReason::HighRiskOrIrreversible,
                None,
                None
            )
        );
        if observed_state != TaskState::Draft || !decision_shape_valid {
            return Err(TaskLifecycleError::new(
                TaskLifecycleErrorKind::Corrupt,
                "TASK_LIFECYCLE_AUTONOMY_PROJECTION_INVALID",
            ));
        }
        Ok(Self {
            receipt_digest,
            authority_digest,
            event_digest,
            observed_state,
            disposition,
            reason,
            model,
            verification,
        })
    }

    #[must_use]
    pub const fn receipt_digest(&self) -> &lattice_contracts::ContentDigest {
        &self.receipt_digest
    }
    #[must_use]
    pub const fn authority_digest(&self) -> &lattice_contracts::ContentDigest {
        &self.authority_digest
    }
    #[must_use]
    pub const fn event_digest(&self) -> &lattice_contracts::ContentDigest {
        &self.event_digest
    }
    #[must_use]
    pub const fn observed_state(&self) -> TaskState {
        self.observed_state
    }
    #[must_use]
    pub const fn disposition(&self) -> AutonomyDisposition {
        self.disposition
    }
    #[must_use]
    pub const fn reason(&self) -> AutonomyReason {
        self.reason
    }
    #[must_use]
    pub const fn model(&self) -> Option<AutonomyModel> {
        self.model
    }
    #[must_use]
    pub const fn verification(&self) -> Option<AutonomyVerification> {
        self.verification
    }
}

/// Closed lifecycle admission/receipt relationship exposed through Ports.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TaskLifecycleAutonomyEvidence {
    Unadmitted,
    HistoricalOptional(Option<AutonomyReceiptProjection>),
    RequiredComplete(AutonomyReceiptProjection),
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TaskLifecycleAdmissionState {
    PendingRequiredReceipt {
        binding: lattice_contracts::SubjectBinding,
        ledger_head_digest: lattice_contracts::ContentDigest,
    },
    Existing(TaskLifecycleEvidence),
}

/// Closed result of idempotent task admission. A fresh required profile may
/// exist only as reconciliation state until its sequence-2 receipt is durable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskLifecycleAdmission {
    state: TaskLifecycleAdmissionState,
}

impl TaskLifecycleAdmission {
    #[must_use]
    pub const fn pending_required_receipt(
        binding: lattice_contracts::SubjectBinding,
        ledger_head_digest: lattice_contracts::ContentDigest,
    ) -> Self {
        Self {
            state: TaskLifecycleAdmissionState::PendingRequiredReceipt {
                binding,
                ledger_head_digest,
            },
        }
    }

    /// Wraps replay-derived evidence only after it represents an admitted task.
    ///
    /// # Errors
    ///
    /// Rejects unadmitted/not-applicable lifecycle evidence.
    pub fn existing(evidence: TaskLifecycleEvidence) -> TaskLifecycleResult<Self> {
        if !evidence.admitted() {
            return Err(TaskLifecycleError::new(
                TaskLifecycleErrorKind::Corrupt,
                "LATTICE_TASK_ADMISSION_STATE_REJECTED",
            ));
        }
        Ok(Self {
            state: TaskLifecycleAdmissionState::Existing(evidence),
        })
    }

    #[must_use]
    pub const fn binding(&self) -> &lattice_contracts::SubjectBinding {
        match &self.state {
            TaskLifecycleAdmissionState::PendingRequiredReceipt { binding, .. } => binding,
            TaskLifecycleAdmissionState::Existing(evidence) => evidence.binding(),
        }
    }

    #[must_use]
    pub const fn ledger_head_digest(&self) -> &lattice_contracts::ContentDigest {
        match &self.state {
            TaskLifecycleAdmissionState::PendingRequiredReceipt {
                ledger_head_digest, ..
            } => ledger_head_digest,
            TaskLifecycleAdmissionState::Existing(evidence) => evidence.ledger_head_digest(),
        }
    }

    #[must_use]
    pub const fn existing_evidence(&self) -> Option<&TaskLifecycleEvidence> {
        match &self.state {
            TaskLifecycleAdmissionState::Existing(evidence) => Some(evidence),
            TaskLifecycleAdmissionState::PendingRequiredReceipt { .. } => None,
        }
    }

    #[must_use]
    pub fn into_existing(self) -> Option<TaskLifecycleEvidence> {
        match self.state {
            TaskLifecycleAdmissionState::Existing(evidence) => Some(evidence),
            TaskLifecycleAdmissionState::PendingRequiredReceipt { .. } => None,
        }
    }
}

/// Replay-derived authoritative Task lifecycle projection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskLifecycleEvidence {
    binding: lattice_contracts::SubjectBinding,
    autonomy_evidence: TaskLifecycleAutonomyEvidence,
    state: TaskState,
    ledger_head_digest: lattice_contracts::ContentDigest,
    result_digest: Option<lattice_contracts::ContentDigest>,
}

impl TaskLifecycleEvidence {
    #[must_use]
    pub const fn new(
        binding: lattice_contracts::SubjectBinding,
        autonomy_evidence: TaskLifecycleAutonomyEvidence,
        state: TaskState,
        ledger_head_digest: lattice_contracts::ContentDigest,
        result_digest: Option<lattice_contracts::ContentDigest>,
    ) -> Self {
        Self {
            binding,
            autonomy_evidence,
            state,
            ledger_head_digest,
            result_digest,
        }
    }

    #[must_use]
    pub const fn binding(&self) -> &lattice_contracts::SubjectBinding {
        &self.binding
    }

    #[must_use]
    pub const fn admitted(&self) -> bool {
        !matches!(
            self.autonomy_evidence,
            TaskLifecycleAutonomyEvidence::Unadmitted
        )
    }

    #[must_use]
    pub const fn state(&self) -> TaskState {
        self.state
    }

    #[must_use]
    pub const fn ledger_head_digest(&self) -> &lattice_contracts::ContentDigest {
        &self.ledger_head_digest
    }

    #[must_use]
    pub const fn result_digest(&self) -> Option<&lattice_contracts::ContentDigest> {
        self.result_digest.as_ref()
    }

    #[must_use]
    pub const fn autonomy_receipt(&self) -> Option<&AutonomyReceiptProjection> {
        match &self.autonomy_evidence {
            TaskLifecycleAutonomyEvidence::HistoricalOptional(receipt) => receipt.as_ref(),
            TaskLifecycleAutonomyEvidence::RequiredComplete(receipt) => Some(receipt),
            TaskLifecycleAutonomyEvidence::Unadmitted => None,
        }
    }

    #[must_use]
    pub const fn autonomy_evidence(&self) -> &TaskLifecycleAutonomyEvidence {
        &self.autonomy_evidence
    }
}

/// PostgreSQL-backed authoritative lifecycle boundary used by the sole
/// orchestrator. Implementations may persist and replay but never decide
/// Task Domain transition legality.
pub trait TaskLifecyclePort {
    /// Idempotently admits one exact caller retry key and Task binding.
    ///
    /// # Errors
    ///
    /// Returns a typed rejection, availability, ambiguity, or corruption error.
    fn admit(
        &mut self,
        binding: &lattice_contracts::SubjectBinding,
        client_request_id: &str,
    ) -> TaskLifecycleResult<TaskLifecycleAdmission>;

    /// Records the exactly-once autonomy receipt before the first writable task
    /// effect. `PROCEED` requires the supplied current writer authority;
    /// `ASK_USER` requires `None` and imports no ambient writer authority.
    ///
    /// # Errors
    ///
    /// Returns a typed rejection, availability, ambiguity, or corruption error.
    fn record_autonomy_receipt(
        &mut self,
        binding: &lattice_contracts::SubjectBinding,
        writer_authority: Option<&lattice_contracts::WriterLeaseAuthorityHead>,
    ) -> TaskLifecycleResult<TaskLifecycleEvidence>;

    /// Appends one Task Domain-approved state transition.
    ///
    /// # Errors
    ///
    /// Returns a typed rejection, availability, ambiguity, or corruption error.
    fn transition(
        &mut self,
        binding: &lattice_contracts::SubjectBinding,
        from: TaskState,
        to: TaskState,
        writer_authority: Option<&lattice_contracts::WriterLeaseAuthorityHead>,
    ) -> TaskLifecycleResult<TaskLifecycleEvidence>;

    /// Persists the exact governed execution result under the current writer.
    ///
    /// # Errors
    ///
    /// Returns a typed rejection, availability, ambiguity, or corruption error.
    fn record_result(
        &mut self,
        binding: &lattice_contracts::SubjectBinding,
        result_digest: &lattice_contracts::ContentDigest,
        writer_authority: &lattice_contracts::WriterLeaseAuthorityHead,
    ) -> TaskLifecycleResult<TaskLifecycleEvidence>;

    /// Replays the authoritative lifecycle projection.
    ///
    /// # Errors
    ///
    /// Returns a typed rejection, availability, ambiguity, or corruption error.
    fn load(
        &mut self,
        binding: &lattice_contracts::SubjectBinding,
    ) -> TaskLifecycleResult<TaskLifecycleEvidence>;
}

/// Stable fail-closed categories shared across port and inbound-service boundaries.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PortErrorKind {
    Unavailable,
    VersionMismatch,
    CapabilityMismatch,
    Malformed,
    Timeout,
    Cancelled,
    Ambiguous,
    Denied,
}

/// A typed port failure with a stable machine-facing code.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortError {
    component: Component,
    kind: PortErrorKind,
    code: String,
}

impl PortError {
    /// Constructs a typed port failure.
    #[must_use]
    pub fn new(component: Component, kind: PortErrorKind, code: impl Into<String>) -> Self {
        Self {
            component,
            kind,
            code: code.into(),
        }
    }

    /// Returns the component that observed the failure.
    #[must_use]
    pub const fn component(&self) -> Component {
        self.component
    }

    /// Returns the fail-closed failure category.
    #[must_use]
    pub const fn kind(&self) -> PortErrorKind {
        self.kind
    }

    /// Returns the stable machine-facing failure code.
    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }
}

impl fmt::Display for PortError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{:?} {:?}: {}",
            self.component, self.kind, self.code
        )
    }
}

impl Error for PortError {}

/// Whether a failed delivery call is known not to have completed or has an
/// outcome that must be reconciled before retry.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DeliveryFailureCertainty {
    Known,
    Ambiguous,
}

/// Typed delivery failure with exact stage and effect certainty.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliveryPortError {
    stage: DeliveryStage,
    kind: PortErrorKind,
    certainty: DeliveryFailureCertainty,
    code: String,
}

impl DeliveryPortError {
    /// Constructs one stage-specific delivery failure.
    #[must_use]
    pub fn new(
        stage: DeliveryStage,
        kind: PortErrorKind,
        certainty: DeliveryFailureCertainty,
        code: impl Into<String>,
    ) -> Self {
        Self {
            stage,
            kind,
            certainty,
            code: code.into(),
        }
    }

    #[must_use]
    pub const fn stage(&self) -> DeliveryStage {
        self.stage
    }

    #[must_use]
    pub const fn kind(&self) -> PortErrorKind {
        self.kind
    }

    #[must_use]
    pub const fn certainty(&self) -> DeliveryFailureCertainty {
        self.certainty
    }

    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }
}

impl fmt::Display for DeliveryPortError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Delivery {:?} {:?}/{:?}: {}",
            self.stage, self.kind, self.certainty, self.code
        )
    }
}

impl Error for DeliveryPortError {}

/// Ordered effect stages for the executable graph-memory node.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GraphMemoryStage {
    /// Exact tracked-only Git snapshot materialization.
    Snapshot,
    /// Pinned Graphify child execution and strict output parsing.
    Graphify,
    /// Atomic analysis/record persistence.
    Persistence,
    /// Exact-snapshot deterministic retrieval and audit persistence.
    Retrieval,
    /// Restart-safe terminal receipt readback.
    Receipt,
    /// Atomic Hermes structured-reflection persistence.
    ReflectionPersistence,
    /// Restart-safe Hermes structured-reflection readback.
    ReflectionReceipt,
}

/// Whether a failed graph-memory effect is known not to have completed.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GraphMemoryFailureCertainty {
    /// The effect is proved not to have completed successfully.
    Known,
    /// The effect outcome cannot safely be inferred and requires reconciliation.
    Ambiguous,
}

/// Typed graph-memory failure with exact stage and outcome certainty.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphMemoryPortError {
    stage: GraphMemoryStage,
    kind: PortErrorKind,
    certainty: GraphMemoryFailureCertainty,
    code: String,
}

impl GraphMemoryPortError {
    /// Constructs one stage-specific graph-memory failure.
    #[must_use]
    pub fn new(
        stage: GraphMemoryStage,
        kind: PortErrorKind,
        certainty: GraphMemoryFailureCertainty,
        code: impl Into<String>,
    ) -> Self {
        Self {
            stage,
            kind,
            certainty,
            code: code.into(),
        }
    }

    #[must_use]
    pub const fn stage(&self) -> GraphMemoryStage {
        self.stage
    }

    #[must_use]
    pub const fn kind(&self) -> PortErrorKind {
        self.kind
    }

    #[must_use]
    pub const fn certainty(&self) -> GraphMemoryFailureCertainty {
        self.certainty
    }

    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }
}

impl fmt::Display for GraphMemoryPortError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "GraphMemory {:?} {:?}/{:?}: {}",
            self.stage, self.kind, self.certainty, self.code
        )
    }
}

impl Error for GraphMemoryPortError {}

/// A typed Rust-core gateway-service failure with no external component label.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GatewayServiceError {
    kind: PortErrorKind,
    code: String,
}

impl GatewayServiceError {
    /// Constructs a typed gateway-service failure.
    #[must_use]
    pub fn new(kind: PortErrorKind, code: impl Into<String>) -> Self {
        Self {
            kind,
            code: code.into(),
        }
    }

    /// Returns the fail-closed failure category.
    #[must_use]
    pub const fn kind(&self) -> PortErrorKind {
        self.kind
    }

    /// Returns the stable machine-facing failure code.
    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }
}

impl fmt::Display for GatewayServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "GatewayService {:?}: {}", self.kind, self.code)
    }
}

impl Error for GatewayServiceError {}

/// Exact logical-session command scope used by durable `OpenClaw` idempotency.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct OpenClawCommandScope {
    project: ProjectId,
    actor: GatewayActorId,
    session_epoch: u64,
    command: GatewayCommandId,
}

impl OpenClawCommandScope {
    /// Constructs one server-derived scope. All identifiers are already typed;
    /// the positive logical session epoch is checked again at this boundary.
    ///
    /// # Errors
    ///
    /// Rejects the zero epoch sentinel.
    pub fn new(
        project: ProjectId,
        actor: GatewayActorId,
        session_epoch: u64,
        command: GatewayCommandId,
    ) -> Result<Self, OpenClawIdempotencyError> {
        if session_epoch == 0 {
            return Err(OpenClawIdempotencyError::Malformed);
        }
        Ok(Self {
            project,
            actor,
            session_epoch,
            command,
        })
    }

    /// Returns the command's exact project.
    #[must_use]
    pub const fn project_id(&self) -> &ProjectId {
        &self.project
    }

    /// Returns the server-derived actor identity.
    #[must_use]
    pub const fn actor_id(&self) -> &GatewayActorId {
        &self.actor
    }

    /// Returns the verified logical `OpenClaw` session epoch.
    #[must_use]
    pub const fn session_epoch(&self) -> u64 {
        self.session_epoch
    }

    /// Returns the idempotent semantic command identity.
    #[must_use]
    pub const fn command_id(&self) -> &GatewayCommandId {
        &self.command
    }
}

/// One typed terminal command record suitable for durable receipt storage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenClawTerminalCommandRecord {
    scope: OpenClawCommandScope,
    request: GatewayRequest,
    reply: GatewayReply,
}

impl OpenClawTerminalCommandRecord {
    /// Constructs one exact terminal record after checking request, reply, and
    /// logical command-scope binding.
    ///
    /// # Errors
    ///
    /// Rejects command, project, request-digest, or reply binding drift.
    pub fn new(
        scope: OpenClawCommandScope,
        request: GatewayRequest,
        reply: GatewayReply,
    ) -> Result<Self, OpenClawIdempotencyError> {
        if request.project_id() != scope.project_id()
            || request.command_id() != scope.command_id()
            || reply.command_id() != request.command_id()
            || reply.correlation_id() != request.correlation_id()
            || reply.action() != request.action()
            || reply.request_digest() != request.request_digest()
        {
            return Err(OpenClawIdempotencyError::Malformed);
        }
        Ok(Self {
            scope,
            request,
            reply,
        })
    }

    /// Returns the exact project/actor/session/command scope.
    #[must_use]
    pub const fn scope(&self) -> &OpenClawCommandScope {
        &self.scope
    }

    /// Returns the exact mechanically verified gateway request.
    #[must_use]
    pub const fn request(&self) -> &GatewayRequest {
        &self.request
    }

    /// Returns the exact reconstructed gateway request digest.
    #[must_use]
    pub const fn request_digest(&self) -> &lattice_contracts::ContentDigest {
        self.request.request_digest()
    }

    /// Returns the validated terminal gateway reply.
    #[must_use]
    pub const fn reply(&self) -> &GatewayReply {
        &self.reply
    }
}

/// Persistence claim exposed by a terminal-idempotency provider.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpenClawIdempotencyDurability {
    /// Process-local checkpoint storage; never restart-safe.
    ProcessMemory,
    /// Durable terminal command receipts reconciled across process starts.
    DurableTerminalReceipts,
}

/// Typed result of reconciling one command before dispatch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OpenClawIdempotencyDecision {
    /// The exact request now owns a bounded pre-dispatch claim.
    Claimed,
    /// The exact request is already claimed but has no terminal reply yet.
    InFlight,
    /// The exact request already has a terminal reply.
    Exact(Box<GatewayReply>),
    /// The command scope exists under a different request digest.
    CommandSubstitution,
}

/// Closed idempotency-provider failure without backend details.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpenClawIdempotencyError {
    /// The provider could not be reached.
    Unavailable,
    /// The provider's bounded capacity was exhausted.
    Capacity,
    /// The provider returned a malformed or contradictory record.
    Malformed,
}

/// Typed terminal-command idempotency port implemented by process-memory test
/// checkpoints or the `PostgreSQL` truth owner.
pub trait OpenClawIdempotencyStore: Send {
    /// States whether records survive a process restart.
    fn durability(&self) -> OpenClawIdempotencyDurability;

    /// Atomically reconciles or claims one command before dispatch.
    ///
    /// # Errors
    ///
    /// Returns a closed provider failure without backend detail.
    fn reconcile_and_claim(
        &mut self,
        scope: &OpenClawCommandScope,
        request: &GatewayRequest,
    ) -> Result<OpenClawIdempotencyDecision, OpenClawIdempotencyError>;

    /// Finalizes one existing claim with a validated terminal reply.
    ///
    /// # Errors
    ///
    /// Returns a closed provider failure without backend detail.
    fn finalize_terminal(
        &mut self,
        record: OpenClawTerminalCommandRecord,
    ) -> Result<(), OpenClawIdempotencyError>;
}

/// Stable fail-closed Store failure categories.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ControlStoreErrorKind {
    Malformed,
    UnsupportedVersion,
    CommandSubstitution,
    AuthorityMismatch,
    AdmissionDenied,
    RevisionOverflow,
    CapacityExceeded,
    Unavailable,
    SerializationExhausted,
    CommitOutcomeUnknown,
    CorruptState,
}

impl ControlStoreErrorKind {
    /// Complete closed Store error set; no variant represents success.
    pub const ALL: [Self; 11] = [
        Self::Malformed,
        Self::UnsupportedVersion,
        Self::CommandSubstitution,
        Self::AuthorityMismatch,
        Self::AdmissionDenied,
        Self::RevisionOverflow,
        Self::CapacityExceeded,
        Self::Unavailable,
        Self::SerializationExhausted,
        Self::CommitOutcomeUnknown,
        Self::CorruptState,
    ];
}

/// Typed Store failure with one stable bounded machine-facing code.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlStoreError {
    kind: ControlStoreErrorKind,
    code: &'static str,
}

impl ControlStoreError {
    /// Constructs one Store-specific failure.
    #[must_use]
    pub const fn new(kind: ControlStoreErrorKind, code: &'static str) -> Self {
        Self { kind, code }
    }

    /// Returns the fail-closed category.
    #[must_use]
    pub const fn kind(&self) -> ControlStoreErrorKind {
        self.kind
    }

    /// Returns the stable machine-facing code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        self.code
    }
}

impl fmt::Display for ControlStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "ControlStore {:?}: {}", self.kind, self.code)
    }
}

impl Error for ControlStoreError {}

/// Inbound service implemented by the Rust core for the normal `OpenClaw` gateway.
pub trait GatewayService {
    /// Handles one mechanically verified request under server-derived peer context.
    ///
    /// # Errors
    ///
    /// Returns a typed failure when the request cannot be safely accepted or
    /// observed.
    fn handle(
        &mut self,
        peer: GatewayPeerContext,
        request: GatewayRequest,
    ) -> GatewayServiceResult<GatewayReply>;
}

/// Typed physical control-store boundary implemented by fake or live adapters.
pub trait ControlStore {
    /// Applies one complete domain-committed physical transaction.
    ///
    /// # Errors
    ///
    /// Returns a Store-specific failure for malformed/substituted authority,
    /// capacity, transport, serialization, corruption, or unknown outcomes.
    /// A terminal receipt may classify proven physical durability, but it
    /// never defines domain legality, freshness, or effect delivery.
    fn transact(
        &mut self,
        request: StoreTransactionRequest,
    ) -> ControlStoreResult<StoreTransactionReceipt>;

    /// Returns the independently retained physical head for one exact scope.
    ///
    /// # Errors
    ///
    /// Returns a Store-specific failure when the head cannot be safely
    /// observed. The result is not a domain-owner current head.
    fn current_head(&mut self, scope: &StoreScope) -> ControlStoreResult<StorePhysicalHead>;
}

/// Durable delivery-ledger boundary. Implementations own persistence and
/// canonical receipt reconstruction; callers never receive a database client.
pub trait DeliveryLedgerPort {
    /// Commits intent before any workspace or provider effect.
    ///
    /// # Errors
    ///
    /// Returns a typed failure when durability cannot be proved.
    fn record_intent(
        &mut self,
        request: &DeliveryRunRequest,
    ) -> DeliveryPortResult<DurableIntentEvidence>;

    /// Records one completed, failed, or reconciliation-required outcome.
    ///
    /// # Errors
    ///
    /// Returns a typed failure when terminal persistence is rejected or
    /// unknown.
    fn record_outcome(
        &mut self,
        request: &DeliveryOutcomeRequest,
    ) -> DeliveryPortResult<DeliveryOutcomeEvidence>;

    /// Reconstructs the terminal receipt from durable state.
    ///
    /// # Errors
    ///
    /// Returns a typed failure for missing, incomplete, corrupt, or ambiguous
    /// durable state.
    fn load_receipt(
        &mut self,
        request: &DeliveryStatusRequest,
    ) -> DeliveryPortResult<DeliveryReceipt>;
}

/// Sole production Codex writer boundary for typed delivery. The legacy
/// generic [`CodexPort`] remains source compatible for earlier consumers but
/// is frozen outside the production delivery composition.
pub trait DeliveryCodexPort {
    /// Runs one request bound to durable intent and a prepared workspace.
    ///
    /// # Errors
    ///
    /// Returns a typed known or ambiguous Codex-stage failure.
    fn run_delivery(
        &mut self,
        request: CodexDeliveryRequest,
    ) -> DeliveryPortResult<CodexDeliveryEvidence>;

    /// Interrupts the delivery associated with one request.
    ///
    /// # Errors
    ///
    /// Returns a typed failure when interruption or final outcome is unknown.
    fn interrupt_delivery(&mut self, request_id: &RequestId) -> DeliveryPortResult<()>;
}

/// Bounded workspace and Git lane. It exposes no command text or caller path.
pub trait WorkspaceGitPort {
    /// Prepares the fixed delivery workspace after durable intent.
    ///
    /// # Errors
    ///
    /// Returns a typed failure for unsafe or ambiguous preparation.
    fn prepare(
        &mut self,
        request: &DeliveryRunRequest,
        intent: &DurableIntentEvidence,
    ) -> DeliveryPortResult<PreparedWorkspaceEvidence>;

    /// Inspects the fixed changed-path scope after Codex completes.
    ///
    /// # Errors
    ///
    /// Returns a typed failure when scope does not match or cannot be proved.
    fn inspect_changes(
        &mut self,
        request: &DeliveryRunRequest,
        intent: &DurableIntentEvidence,
        workspace: &PreparedWorkspaceEvidence,
        codex: &CodexDeliveryEvidence,
    ) -> DeliveryPortResult<WorkspaceChangeEvidence>;

    /// Creates one local commit after passing scope and fixed-test evidence.
    ///
    /// # Errors
    ///
    /// Returns a typed known or ambiguous Git-stage failure.
    fn commit(
        &mut self,
        request: &DeliveryRunRequest,
        workspace: &PreparedWorkspaceEvidence,
        changes: &WorkspaceChangeEvidence,
        test: &FixedTestEvidence,
    ) -> DeliveryPortResult<GitCommitEvidence>;
}

/// Sole fixed verification profile used by the bounded delivery node.
pub trait TestRunnerPort {
    /// Runs the profile-selected fixed test; no command text is accepted.
    ///
    /// # Errors
    ///
    /// Returns a typed failure for a failed or unobservable test.
    fn run_fixed(
        &mut self,
        request: &DeliveryRunRequest,
        workspace: &PreparedWorkspaceEvidence,
        changes: &WorkspaceChangeEvidence,
    ) -> DeliveryPortResult<FixedTestEvidence>;
}

/// Frozen generic product-code writer boundary retained for pre-delivery
/// consumers. Production delivery uses [`DeliveryCodexPort`] and must not wire
/// both interfaces as separate runtime writers.
pub trait CodexPort {
    /// Runs one approved implementation request.
    ///
    /// # Errors
    ///
    /// Returns a typed failure when capability, version, permission, runtime,
    /// completion, or outcome evidence is unsafe or unknown.
    fn run(&mut self, request: CodexRunRequest) -> PortResult<CodexEvidence>;

    /// Interrupts the run associated with a request.
    ///
    /// # Errors
    ///
    /// Returns a typed failure when interruption or final outcome is unknown.
    fn interrupt(&mut self, request_id: &RequestId) -> PortResult<()>;
}

/// Exact tracked-only snapshot boundary for the production graph-memory node.
///
/// The implementation owns its process-configured repository and staging
/// location; callers can select neither a path nor a Git command.
pub trait CodeSnapshotPort {
    /// Materializes one immutable tracked snapshot for the exact request commit.
    ///
    /// # Errors
    ///
    /// Returns a typed known or ambiguous failure for missing/substituted Git
    /// objects, unsafe tracked paths, secret/untracked leakage, or teardown.
    fn materialize_snapshot(
        &mut self,
        request: &GraphMemoryRunRequest,
    ) -> GraphMemoryPortResult<CodeSnapshotEvidence>;
}

/// Pinned Graphify analysis boundary for production graph-memory work.
///
/// This interface is separate from the frozen generic [`GraphifyPort`] and
/// accepts no command, environment, path, credential, or backend selection.
pub trait GraphifyAnalysisPort {
    /// Runs the fixed headless code-only analysis over one exact snapshot.
    ///
    /// # Errors
    ///
    /// Returns a typed known or ambiguous failure for identity/capability
    /// mismatch, timeout, malformed/partial output, or unsafe provenance.
    fn analyze(
        &mut self,
        request: &GraphMemoryRunRequest,
        snapshot: &CodeSnapshotEvidence,
    ) -> GraphMemoryPortResult<GraphifyRawEvidence>;
}

/// Sole Codebase Memory repository boundary.
///
/// Implementations persist only typed analysis/plan values through fixed
/// repository operations and never expose SQL, credentials, or a database
/// client. Effect ordering remains owned by the orchestrator.
pub trait CodebaseMemoryPort {
    /// Atomically writes one complete normalized analysis and candidate set.
    ///
    /// # Errors
    ///
    /// Returns a typed failure when exact persistence is rejected or unknown.
    fn persist_analysis(
        &mut self,
        analysis: &NormalizedGraphAnalysis,
    ) -> GraphMemoryPortResult<GraphMemoryPersistenceEvidence>;

    /// Executes and audits one precomputed deterministic retrieval plan.
    ///
    /// # Errors
    ///
    /// Returns a typed failure for missing/substituted analysis, invalidation,
    /// corrupt ordering, unavailable storage, or ambiguous commit outcome.
    fn retrieve(
        &mut self,
        persistence: &GraphMemoryPersistenceEvidence,
        plan: MemoryRetrievalPlan,
    ) -> GraphMemoryPortResult<GraphMemoryReceipt>;

    /// Replays one exact terminal receipt without invoking earlier effects.
    ///
    /// # Errors
    ///
    /// Returns a typed failure for missing, incomplete, cross-bound, corrupt,
    /// or unavailable repository state. This receipt alone is not live
    /// database-identity or restart-replay proof.
    fn load_receipt(
        &mut self,
        request: &GraphMemoryRunRequest,
    ) -> GraphMemoryPortResult<GraphMemoryReceipt>;
}

/// Sole durable repository boundary for structured Hermes reflection content.
///
/// The implementation is owned by the same `PostgreSQL` Codebase Memory adapter
/// as graph receipts. Hermes itself never receives this port or a database
/// client, and fresh status uses only [`Self::load_reflection`].
pub trait HermesReflectionMemoryPort {
    /// Atomically persists one exact-graph-bound inference candidate.
    ///
    /// # Errors
    ///
    /// Returns a typed failure for missing/substituted graph truth, invalid
    /// structured content, unavailable storage, or ambiguous commit outcome.
    fn persist_reflection(
        &mut self,
        reflection: &HermesReflectionCandidate,
    ) -> GraphMemoryPortResult<HermesReflectionReceipt>;

    /// Loads one exact structured reflection without invoking Hermes.
    ///
    /// # Errors
    ///
    /// Returns a typed failure for missing, incomplete, cross-bound, corrupt,
    /// or unavailable durable state.
    fn load_reflection(
        &mut self,
        request: &GraphMemoryRunRequest,
    ) -> GraphMemoryPortResult<HermesReflectionReceipt>;
}

/// Read-only derived-knowledge boundary implemented by the `Graphify` adapter.
pub trait GraphifyPort {
    /// Builds a code graph for an immutable project snapshot.
    ///
    /// # Errors
    ///
    /// Returns a typed failure for unsafe source/output boundaries, unavailable
    /// capabilities, malformed output, timeout, or ambiguous completion.
    fn build_code_graph(&mut self, request: GraphifyBuildRequest) -> PortResult<GraphifyEvidence>;
}

/// Untrusted research-candidate boundary implemented by the `Hermes` adapter.
pub trait HermesPort {
    /// Runs one bounded research request.
    ///
    /// # Errors
    ///
    /// Returns a typed failure for unavailable capabilities, malformed or
    /// provenance-free output, timeout, cancellation, or ambiguity.
    fn research(&mut self, request: HermesResearchRequest) -> PortResult<HermesEvidence>;

    /// Interrupts the run associated with a request.
    ///
    /// # Errors
    ///
    /// Returns a typed failure when interruption or final outcome is unknown.
    fn interrupt(&mut self, request_id: &RequestId) -> PortResult<()>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use lattice_contracts::{ContentDigest, ProjectId, ProjectSnapshotId, SubjectBinding, TaskId};

    fn digest(byte: char) -> ContentDigest {
        ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("valid digest")
    }

    #[test]
    fn task_lifecycle_autonomy_evidence_is_closed() {
        let binding = SubjectBinding::new(
            ProjectId::new("project-1").expect("project"),
            ProjectSnapshotId::new("snapshot-1").expect("snapshot"),
            TaskId::new("TASK-050").expect("task"),
            "1",
            digest('a'),
        )
        .expect("binding");
        let receipt = AutonomyReceiptProjection::new(
            digest('b'),
            digest('c'),
            digest('d'),
            TaskState::Draft,
            AutonomyDisposition::Proceed,
            AutonomyReason::RoutineAuthorized,
            Some(AutonomyModel::GovernedCodexWriter),
            Some(AutonomyVerification::FocusedChecks),
        )
        .expect("closed receipt projection");
        let evidence = TaskLifecycleEvidence::new(
            binding,
            TaskLifecycleAutonomyEvidence::RequiredComplete(receipt.clone()),
            TaskState::Draft,
            digest('e'),
            None,
        );
        assert!(evidence.admitted());
        assert_eq!(evidence.autonomy_receipt(), Some(&receipt));
        assert_eq!(
            evidence.autonomy_evidence(),
            &TaskLifecycleAutonomyEvidence::RequiredComplete(receipt)
        );
        assert!(TaskLifecycleAdmission::existing(evidence).is_ok());
        let unadmitted = TaskLifecycleEvidence::new(
            SubjectBinding::new(
                ProjectId::new("project-1").expect("project"),
                ProjectSnapshotId::new("snapshot-1").expect("snapshot"),
                TaskId::new("TASK-050").expect("task"),
                "1",
                digest('a'),
            )
            .expect("binding"),
            TaskLifecycleAutonomyEvidence::Unadmitted,
            TaskState::Draft,
            digest('e'),
            None,
        );
        assert_eq!(
            TaskLifecycleAdmission::existing(unadmitted)
                .expect_err("unadmitted evidence cannot become existing admission")
                .code(),
            "LATTICE_TASK_ADMISSION_STATE_REJECTED"
        );
        assert!(
            AutonomyReceiptProjection::new(
                digest('b'),
                digest('c'),
                digest('d'),
                TaskState::Draft,
                AutonomyDisposition::AskUser,
                AutonomyReason::NewUserDecision,
                Some(AutonomyModel::GovernedCodexWriter),
                None,
            )
            .is_err()
        );
    }
}
