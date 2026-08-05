//! Abstract I/O ports for LATTICE orchestration.

use std::error::Error;
use std::fmt;

use lattice_contracts::{
    CodeSnapshotEvidence, CodexDeliveryEvidence, CodexDeliveryRequest, CodexEvidence,
    CodexRunRequest, Component, DeliveryOutcomeEvidence, DeliveryOutcomeRequest, DeliveryReceipt,
    DeliveryRunRequest, DeliveryStage, DeliveryStatusRequest, DurableIntentEvidence,
    FixedTestEvidence, GatewayPeerContext, GatewayReply, GatewayRequest, GitCommitEvidence,
    GraphMemoryPersistenceEvidence, GraphMemoryReceipt, GraphMemoryRunRequest,
    GraphifyBuildRequest, GraphifyEvidence, GraphifyRawEvidence, HermesEvidence,
    HermesReflectionCandidate, HermesReflectionReceipt, HermesResearchRequest, MemoryRetrievalPlan,
    NormalizedGraphAnalysis, PreparedWorkspaceEvidence, RequestId, StorePhysicalHead, StoreScope,
    StoreTransactionReceipt, StoreTransactionRequest, WorkspaceChangeEvidence,
};

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
