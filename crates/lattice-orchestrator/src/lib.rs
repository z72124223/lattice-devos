//! Pure injected-port delivery orchestration for LATTICE.

use std::error::Error;
use std::fmt;

use lattice_codebase_memory::{CodebaseMemoryError, normalize_analysis, plan_retrieval};
use lattice_contracts::{
    AttemptId, CodexDeliveryRequest, CompletedDeliveryEvidence, ContentDigest,
    DeliveryContractError, DeliveryOutcomeRequest, DeliveryReceipt, DeliveryRunRequest,
    DeliveryStage, DeliveryStatusRequest, DeliveryTerminalStatus, DurableIntentEvidence,
    GraphMemoryReceipt, GraphMemoryRunRequest, HolderProcessId, MemoryQuery, SubjectBinding,
    WriterLeaseAuthorityHead,
};
use lattice_ports::{
    CodeSnapshotPort, CodebaseMemoryPort, ControlledTaskExecutionError,
    ControlledTaskExecutionPort, DeliveryCodexPort, DeliveryFailureCertainty, DeliveryLedgerPort,
    DeliveryPortError, GraphMemoryPortError, GraphMemoryStage, GraphifyAnalysisPort, PortErrorKind,
    TaskLifecycleError, TaskLifecycleEvidence, TaskLifecyclePort, TestRunnerPort, WorkspaceGitPort,
    WriterAuthorityGuardPort,
};
use lattice_task_domain::TaskState;
use lattice_writer_lease::{
    CommandOutcome as WriterLeaseCommandOutcome, WriterLeaseAcquireRequest,
    WriterLeaseReleaseRequest, WriterLeaseRepository, WriterLeaseRepositoryCommand,
    WriterLeaseRepositoryError,
};

/// Pure graph-memory coordinator failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GraphMemoryOrchestratorError {
    /// Exact tracked snapshot materialization failed.
    Snapshot(GraphMemoryPortError),
    /// Pinned Graphify execution or strict output validation failed.
    Graphify(GraphMemoryPortError),
    /// Pure normalization, binding, or deterministic ranking failed.
    Normalize(CodebaseMemoryError),
    /// Repository analysis/record persistence failed or became ambiguous.
    Persistence(GraphMemoryPortError),
    /// Exact retrieval/audit persistence failed or became ambiguous.
    Retrieval(GraphMemoryPortError),
    /// Independent repository receipt readback failed.
    Receipt(GraphMemoryPortError),
    /// A port returned evidence belonging to another request or analysis.
    EvidenceMismatch(GraphMemoryStage),
}

impl fmt::Display for GraphMemoryOrchestratorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Snapshot(error) => write!(formatter, "graph snapshot rejected: {error}"),
            Self::Graphify(error) => write!(formatter, "Graphify analysis rejected: {error}"),
            Self::Normalize(error) => write!(formatter, "graph normalization rejected: {error}"),
            Self::Persistence(error) => write!(formatter, "graph persistence rejected: {error}"),
            Self::Retrieval(error) => write!(formatter, "memory retrieval rejected: {error}"),
            Self::Receipt(error) => write!(formatter, "graph-memory receipt rejected: {error}"),
            Self::EvidenceMismatch(stage) => {
                write!(formatter, "graph-memory evidence mismatch at {stage:?}")
            }
        }
    }
}

impl Error for GraphMemoryOrchestratorError {}

/// Runs one exact graph-memory node using only injected effects and pure logic.
///
/// Effect order is fixed to snapshot -> Graphify -> normalize/rank -> persist
/// -> retrieve/audit -> independent repository receipt readback. The function
/// returns at the first failure; no later port is called.
///
/// # Errors
///
/// Returns a stage-specific error for the first failed/ambiguous effect, a
/// pure normalization/ranking error, or a cross-bound evidence mismatch.
pub fn run_graph_memory<S, G, M>(
    request: &GraphMemoryRunRequest,
    query: &MemoryQuery,
    snapshot_port: &mut S,
    graphify_port: &mut G,
    memory_port: &mut M,
) -> Result<GraphMemoryReceipt, GraphMemoryOrchestratorError>
where
    S: CodeSnapshotPort,
    G: GraphifyAnalysisPort,
    M: CodebaseMemoryPort,
{
    let snapshot = snapshot_port
        .materialize_snapshot(request)
        .map_err(GraphMemoryOrchestratorError::Snapshot)?;
    if snapshot.request() != request {
        return Err(GraphMemoryOrchestratorError::EvidenceMismatch(
            GraphMemoryStage::Snapshot,
        ));
    }

    let raw = graphify_port
        .analyze(request, &snapshot)
        .map_err(GraphMemoryOrchestratorError::Graphify)?;
    let analysis = normalize_analysis(request, &snapshot, &raw)
        .map_err(GraphMemoryOrchestratorError::Normalize)?;
    let retrieval_plan =
        plan_retrieval(&analysis, query).map_err(GraphMemoryOrchestratorError::Normalize)?;
    let expected_retrieval = retrieval_plan.clone();

    let persistence = memory_port
        .persist_analysis(&analysis)
        .map_err(GraphMemoryOrchestratorError::Persistence)?;
    if persistence.request() != request
        || persistence.analysis_digest() != analysis.analysis_digest()
        || persistence.record_set_digest() != analysis.record_set_digest()
        || usize::try_from(persistence.record_count()).ok() != Some(analysis.records().len())
    {
        return Err(GraphMemoryOrchestratorError::EvidenceMismatch(
            GraphMemoryStage::Persistence,
        ));
    }

    let retrieval_receipt = memory_port
        .retrieve(&persistence, retrieval_plan)
        .map_err(GraphMemoryOrchestratorError::Retrieval)?;
    let observed_retrieval = retrieval_receipt.retrieval();
    if !retrieval_receipt.matches_request(request)
        || retrieval_receipt.persistence() != &persistence
        || observed_retrieval.request() != expected_retrieval.request()
        || observed_retrieval.analysis_digest() != expected_retrieval.analysis_digest()
        || observed_retrieval.persistence_digest() != persistence.persistence_digest()
        || observed_retrieval.query_digest() != expected_retrieval.query_digest()
        || observed_retrieval.algorithm() != expected_retrieval.algorithm()
        || observed_retrieval.limit() != expected_retrieval.limit()
        || observed_retrieval.disposition() != expected_retrieval.disposition()
        || observed_retrieval.results() != expected_retrieval.results()
        || observed_retrieval.result_set_digest() != expected_retrieval.result_set_digest()
    {
        return Err(GraphMemoryOrchestratorError::EvidenceMismatch(
            GraphMemoryStage::Retrieval,
        ));
    }

    let receipt = memory_port
        .load_receipt(request)
        .map_err(GraphMemoryOrchestratorError::Receipt)?;
    if receipt != retrieval_receipt || !receipt.matches_request(request) {
        return Err(GraphMemoryOrchestratorError::EvidenceMismatch(
            GraphMemoryStage::Receipt,
        ));
    }
    Ok(receipt)
}

/// Loads one exact graph-memory receipt without invoking earlier effects.
///
/// # Errors
///
/// Returns a receipt-port failure or cross-binding mismatch.
pub fn graph_memory_status<M: CodebaseMemoryPort>(
    request: &GraphMemoryRunRequest,
    memory_port: &mut M,
) -> Result<GraphMemoryReceipt, GraphMemoryOrchestratorError> {
    let receipt = memory_port
        .load_receipt(request)
        .map_err(GraphMemoryOrchestratorError::Receipt)?;
    if !receipt.matches_request(request) {
        return Err(GraphMemoryOrchestratorError::EvidenceMismatch(
            GraphMemoryStage::Receipt,
        ));
    }
    Ok(receipt)
}

/// Server-owned execution identity for one bounded MCP task. GPT supplies none
/// of these writer fields.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlledTaskRequest {
    binding: SubjectBinding,
    client_request_id: String,
    attempt_id: AttemptId,
    lease_id: String,
    lease_holder_id: String,
    worktree_id: String,
    holder_process_id: HolderProcessId,
    holder_process_start_identity: ContentDigest,
}

impl ControlledTaskRequest {
    /// Constructs the exact request selected by the existing composition root.
    ///
    /// # Errors
    ///
    /// Rejects any unbounded or unsafe server-owned identifier.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        binding: SubjectBinding,
        client_request_id: impl Into<String>,
        attempt_id: AttemptId,
        lease_id: impl Into<String>,
        lease_holder_id: impl Into<String>,
        worktree_id: impl Into<String>,
        holder_process_id: HolderProcessId,
        holder_process_start_identity: ContentDigest,
    ) -> Result<Self, ControlledTaskOrchestratorError> {
        let client_request_id = client_request_id.into();
        let lease_id = lease_id.into();
        let lease_holder_id = lease_holder_id.into();
        let worktree_id = worktree_id.into();
        if !valid_control_identifier(&client_request_id)
            || !valid_control_identifier(&lease_id)
            || !valid_control_identifier(&lease_holder_id)
            || !valid_control_identifier(&worktree_id)
        {
            return Err(ControlledTaskOrchestratorError::RequestRejected);
        }
        Ok(Self {
            binding,
            client_request_id,
            attempt_id,
            lease_id,
            lease_holder_id,
            worktree_id,
            holder_process_id,
            holder_process_start_identity,
        })
    }

    #[must_use]
    pub const fn binding(&self) -> &SubjectBinding {
        &self.binding
    }
}

/// Failure from the sole governed-task coordinator.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ControlledTaskOrchestratorError {
    RequestRejected,
    Lifecycle(TaskLifecycleError),
    Lease(WriterLeaseRepositoryError),
    Execution(ControlledTaskExecutionError),
    StateMismatch,
    LeaseMismatch,
    ReconciliationRequired,
}

impl fmt::Display for ControlledTaskOrchestratorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RequestRejected => formatter.write_str("controlled task request rejected"),
            Self::Lifecycle(error) => write!(formatter, "task lifecycle rejected: {error}"),
            Self::Lease(error) => write!(formatter, "writer lease rejected: {error}"),
            Self::Execution(error) => write!(formatter, "controlled execution rejected: {error}"),
            Self::StateMismatch => formatter.write_str("task lifecycle state mismatch"),
            Self::LeaseMismatch => formatter.write_str("writer lease evidence mismatch"),
            Self::ReconciliationRequired => {
                formatter.write_str("controlled task requires reconciliation")
            }
        }
    }
}

impl Error for ControlledTaskOrchestratorError {}

/// Runs one bounded task through Task Domain state, durable Writer Lease,
/// fenced execution, result persistence, exact release, and terminal state.
/// No MCP field can select a command, path, database, Git operation, or lease.
///
/// # Errors
///
/// Returns the first lifecycle, lease, execution, evidence, or reconciliation
/// failure without creating a second coordinator or writer path.
#[allow(clippy::too_many_lines)]
pub fn run_controlled_task<T, W, E>(
    request: &ControlledTaskRequest,
    lifecycle: &mut T,
    writer_lease: &mut W,
    execution: &mut E,
) -> Result<TaskLifecycleEvidence, ControlledTaskOrchestratorError>
where
    T: TaskLifecyclePort,
    W: WriterLeaseRepository,
    E: ControlledTaskExecutionPort,
{
    let admitted = lifecycle
        .admit(&request.binding, &request.client_request_id)
        .map_err(ControlledTaskOrchestratorError::Lifecycle)?;
    ensure_evidence(&admitted, &request.binding, admitted.state())?;
    if !admitted.admitted() {
        return Err(ControlledTaskOrchestratorError::StateMismatch);
    }
    if admitted.state() == TaskState::Completed && admitted.result_digest().is_some() {
        return Ok(admitted);
    }
    if admitted.state() == TaskState::Merging && admitted.result_digest().is_some() {
        return finish_merging_task(request, lifecycle, writer_lease, &admitted);
    }
    if admitted.state() != TaskState::Draft || admitted.result_digest().is_some() {
        return Err(ControlledTaskOrchestratorError::ReconciliationRequired);
    }

    advance(
        lifecycle,
        &request.binding,
        TaskState::Draft,
        TaskState::AwaitingExecutionApproval,
        None,
    )?;
    advance(
        lifecycle,
        &request.binding,
        TaskState::AwaitingExecutionApproval,
        TaskState::Preparing,
        None,
    )?;
    if writer_lease
        .current_authority(request.binding.project_id())
        .map_err(ControlledTaskOrchestratorError::Lease)?
        .is_some()
    {
        return Err(ControlledTaskOrchestratorError::ReconciliationRequired);
    }
    let acquired = writer_lease
        .execute(WriterLeaseRepositoryCommand::Acquire(
            WriterLeaseAcquireRequest {
                command_id: "task038-writer-acquire".to_owned(),
                expected_head: None,
                project_id: request.binding.project_id().clone(),
                project_snapshot_id: request.binding.project_snapshot_id().clone(),
                task_id: request.binding.task_id().clone(),
                task_revision: request.binding.task_revision().to_owned(),
                task_spec_digest: request.binding.task_spec_digest().clone(),
                attempt_id: request.attempt_id.clone(),
                lease_id: request.lease_id.clone(),
                lease_holder_id: request.lease_holder_id.clone(),
                worktree_id: request.worktree_id.clone(),
                holder_process_id: request.holder_process_id,
                holder_process_start_identity: request.holder_process_start_identity.clone(),
            },
        ))
        .map_err(ControlledTaskOrchestratorError::Lease)?;
    let authority = acquired
        .after
        .filter(|_| acquired.outcome == WriterLeaseCommandOutcome::Applied)
        .ok_or(ControlledTaskOrchestratorError::LeaseMismatch)?;
    ensure_writer_binding(&authority, &request.binding)?;
    let current = writer_lease
        .current_authority(request.binding.project_id())
        .map_err(ControlledTaskOrchestratorError::Lease)?
        .ok_or(ControlledTaskOrchestratorError::LeaseMismatch)?;
    if current.independent_head() != &authority {
        return Err(ControlledTaskOrchestratorError::LeaseMismatch);
    }
    writer_lease
        .assert_current(&authority)
        .map_err(ControlledTaskOrchestratorError::Lease)?;
    advance(
        lifecycle,
        &request.binding,
        TaskState::Preparing,
        TaskState::Executing,
        Some(&authority),
    )?;

    let execution_result = {
        let mut writer_guard = RepositoryWriterGuard {
            repository: writer_lease,
            binding: &request.binding,
        };
        execution.execute(&request.binding, &authority, &mut writer_guard)
    };
    let result_digest = match execution_result {
        Ok(digest) => digest,
        Err(error) => {
            match error.kind() {
                lattice_ports::ControlledTaskExecutionErrorKind::Known => {
                    stop_failed_execution(request, lifecycle, writer_lease, &authority)?;
                }
                lattice_ports::ControlledTaskExecutionErrorKind::Ambiguous => {
                    begin_execution_reconciliation(lifecycle, &request.binding, &authority)?;
                }
            }
            return Err(ControlledTaskOrchestratorError::Execution(error));
        }
    };
    writer_lease
        .assert_current(&authority)
        .map_err(ControlledTaskOrchestratorError::Lease)?;
    for (from, to) in [
        (TaskState::Executing, TaskState::Verifying),
        (TaskState::Verifying, TaskState::Reviewing),
        (TaskState::Reviewing, TaskState::AwaitingMergeApproval),
        (TaskState::AwaitingMergeApproval, TaskState::Merging),
    ] {
        advance(lifecycle, &request.binding, from, to, Some(&authority))?;
    }
    let result = lifecycle
        .record_result(&request.binding, &result_digest, &authority)
        .map_err(ControlledTaskOrchestratorError::Lifecycle)?;
    ensure_evidence(&result, &request.binding, TaskState::Merging)?;
    release_writer(request, writer_lease, &authority)?;
    advance(
        lifecycle,
        &request.binding,
        TaskState::Merging,
        TaskState::Completed,
        None,
    )
}

fn finish_merging_task<T: TaskLifecyclePort, W: WriterLeaseRepository>(
    request: &ControlledTaskRequest,
    lifecycle: &mut T,
    writer_lease: &mut W,
    evidence: &TaskLifecycleEvidence,
) -> Result<TaskLifecycleEvidence, ControlledTaskOrchestratorError> {
    if let Some(current) = writer_lease
        .current_authority(request.binding.project_id())
        .map_err(ControlledTaskOrchestratorError::Lease)?
    {
        ensure_writer_binding(current.independent_head(), &request.binding)?;
        release_writer(request, writer_lease, current.independent_head())?;
    }
    ensure_evidence(evidence, &request.binding, TaskState::Merging)?;
    advance(
        lifecycle,
        &request.binding,
        TaskState::Merging,
        TaskState::Completed,
        None,
    )
}

fn stop_failed_execution<T: TaskLifecyclePort, W: WriterLeaseRepository>(
    request: &ControlledTaskRequest,
    lifecycle: &mut T,
    writer_lease: &mut W,
    authority: &WriterLeaseAuthorityHead,
) -> Result<(), ControlledTaskOrchestratorError> {
    advance(
        lifecycle,
        &request.binding,
        TaskState::Executing,
        TaskState::Stopping,
        Some(authority),
    )?;
    release_writer(request, writer_lease, authority)?;
    advance(
        lifecycle,
        &request.binding,
        TaskState::Stopping,
        TaskState::Failed,
        None,
    )?;
    Ok(())
}

fn begin_execution_reconciliation<T: TaskLifecyclePort>(
    lifecycle: &mut T,
    binding: &SubjectBinding,
    authority: &WriterLeaseAuthorityHead,
) -> Result<(), ControlledTaskOrchestratorError> {
    advance(
        lifecycle,
        binding,
        TaskState::Executing,
        TaskState::Stopping,
        Some(authority),
    )?;
    // The effect may still exist. Keep the exact lease/fence current so no
    // replacement writer can overlap it; recovery must resolve or revoke it.
    Ok(())
}

fn release_writer<W: WriterLeaseRepository>(
    request: &ControlledTaskRequest,
    writer_lease: &mut W,
    authority: &WriterLeaseAuthorityHead,
) -> Result<(), ControlledTaskOrchestratorError> {
    writer_lease
        .assert_current(authority)
        .map_err(ControlledTaskOrchestratorError::Lease)?;
    let release = writer_lease
        .execute(WriterLeaseRepositoryCommand::Release(
            WriterLeaseReleaseRequest {
                command_id: "task038-writer-release".to_owned(),
                project_id: request.binding.project_id().clone(),
                expected_head: authority.clone(),
            },
        ))
        .map_err(ControlledTaskOrchestratorError::Lease)?;
    if release.outcome != WriterLeaseCommandOutcome::Applied || release.after.is_some() {
        return Err(ControlledTaskOrchestratorError::LeaseMismatch);
    }
    if writer_lease
        .current_authority(request.binding.project_id())
        .map_err(ControlledTaskOrchestratorError::Lease)?
        .is_some()
    {
        return Err(ControlledTaskOrchestratorError::LeaseMismatch);
    }
    Ok(())
}

fn advance<T: TaskLifecyclePort>(
    lifecycle: &mut T,
    binding: &SubjectBinding,
    from: TaskState,
    to: TaskState,
    writer_authority: Option<&WriterLeaseAuthorityHead>,
) -> Result<TaskLifecycleEvidence, ControlledTaskOrchestratorError> {
    let evidence = lifecycle
        .transition(binding, from, to, writer_authority)
        .map_err(ControlledTaskOrchestratorError::Lifecycle)?;
    ensure_evidence(&evidence, binding, to)?;
    Ok(evidence)
}

fn ensure_evidence(
    evidence: &TaskLifecycleEvidence,
    binding: &SubjectBinding,
    state: TaskState,
) -> Result<(), ControlledTaskOrchestratorError> {
    if evidence.binding() != binding || evidence.state() != state || !evidence.admitted() {
        return Err(ControlledTaskOrchestratorError::StateMismatch);
    }
    Ok(())
}

fn ensure_writer_binding(
    authority: &WriterLeaseAuthorityHead,
    binding: &SubjectBinding,
) -> Result<(), ControlledTaskOrchestratorError> {
    let identity = authority.identity();
    if identity.project_id() != binding.project_id()
        || identity.project_snapshot_id() != binding.project_snapshot_id()
        || identity.task_id() != binding.task_id()
        || identity.task_revision() != binding.task_revision()
        || identity.task_spec_digest() != binding.task_spec_digest()
    {
        return Err(ControlledTaskOrchestratorError::LeaseMismatch);
    }
    Ok(())
}

struct RepositoryWriterGuard<'a, W> {
    repository: &'a mut W,
    binding: &'a SubjectBinding,
}

impl<W: WriterLeaseRepository> WriterAuthorityGuardPort for RepositoryWriterGuard<'_, W> {
    fn assert_current(
        &mut self,
        expected: &WriterLeaseAuthorityHead,
    ) -> Result<(), ControlledTaskExecutionError> {
        if ensure_writer_binding(expected, self.binding).is_err() {
            return Err(ControlledTaskExecutionError::new(
                lattice_ports::ControlledTaskExecutionErrorKind::Known,
                "LATTICE_WRITER_AUTHORITY_BINDING_REJECTED",
            ));
        }
        self.repository.assert_current(expected).map_err(|error| {
            let kind = match error.kind() {
                lattice_writer_lease::WriterLeaseRepositoryErrorKind::Domain
                | lattice_writer_lease::WriterLeaseRepositoryErrorKind::AuthorityMismatch => {
                    lattice_ports::ControlledTaskExecutionErrorKind::Known
                }
                lattice_writer_lease::WriterLeaseRepositoryErrorKind::Unavailable
                | lattice_writer_lease::WriterLeaseRepositoryErrorKind::SerializationExhausted
                | lattice_writer_lease::WriterLeaseRepositoryErrorKind::CommitOutcomeUnknown
                | lattice_writer_lease::WriterLeaseRepositoryErrorKind::Corrupt => {
                    lattice_ports::ControlledTaskExecutionErrorKind::Ambiguous
                }
            };
            ControlledTaskExecutionError::new(kind, error.code())
        })
    }
}

fn valid_control_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value.starts_with(char::is_whitespace)
        && !value.ends_with(char::is_whitespace)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

/// Pure coordinator failure. A terminal stage failure may still carry its
/// independently reloaded durable receipt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeliveryOrchestratorError {
    /// Intent was not proved durable, so no later effect was attempted.
    Intent(DeliveryPortError),
    /// Trusted stage evidence violated the shared contract.
    Contract(DeliveryContractError),
    /// A terminal record could not be durably written.
    OutcomePersistence(DeliveryPortError),
    /// Durable receipt readback failed.
    ReceiptRead(DeliveryPortError),
    /// Durable readback did not match the exact request/outcome just written.
    ReceiptMismatch,
    /// A known or ambiguous stage failed and its terminal receipt was verified.
    Terminal {
        cause: DeliveryPortError,
        receipt: Box<DeliveryReceipt>,
    },
}

impl fmt::Display for DeliveryOrchestratorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Intent(error) => write!(formatter, "delivery intent rejected: {error}"),
            Self::Contract(error) => write!(formatter, "delivery contract rejected: {error}"),
            Self::OutcomePersistence(error) => {
                write!(formatter, "delivery outcome persistence rejected: {error}")
            }
            Self::ReceiptRead(error) => write!(formatter, "delivery receipt rejected: {error}"),
            Self::ReceiptMismatch => formatter.write_str("delivery receipt cross-binding"),
            Self::Terminal { cause, .. } => write!(formatter, "delivery stage failed: {cause}"),
        }
    }
}

impl Error for DeliveryOrchestratorError {}

/// Runs one delivery using only injected abstract ports.
///
/// The fixed effect order is intent -> workspace preparation -> Codex ->
/// changed-path inspection -> fixed test -> Git commit -> terminal outcome ->
/// independent receipt readback.
///
/// # Errors
///
/// Returns immediately after the first failed gate. Failures after durable
/// intent are themselves recorded as failed or reconciliation-required before
/// the function returns, unless terminal persistence/readback is unavailable.
pub fn run_delivery<L, C, W>(
    request: &DeliveryRunRequest,
    ledger: &mut L,
    workspace_git: &mut W,
    codex: &mut C,
) -> Result<DeliveryReceipt, DeliveryOrchestratorError>
where
    L: DeliveryLedgerPort,
    C: DeliveryCodexPort,
    W: WorkspaceGitPort + TestRunnerPort,
{
    run_delivery_inner(request, ledger, workspace_git, codex, None)
}

struct GovernedWriter<'a> {
    authority: &'a WriterLeaseAuthorityHead,
    guard: &'a mut dyn WriterAuthorityGuardPort,
}

/// Runs the same delivery with an exact server-owned Writer Lease bound into
/// the Codex request. The caller remains the existing LATTICE orchestrator.
///
/// # Errors
///
/// Returns the same fail-closed delivery error classes as [`run_delivery`],
/// including a contract rejection for a cross-bound writer authority.
pub fn run_delivery_governed<L, C, W>(
    request: &DeliveryRunRequest,
    writer_authority: &WriterLeaseAuthorityHead,
    writer_guard: &mut dyn WriterAuthorityGuardPort,
    ledger: &mut L,
    workspace_git: &mut W,
    codex: &mut C,
) -> Result<DeliveryReceipt, DeliveryOrchestratorError>
where
    L: DeliveryLedgerPort,
    C: DeliveryCodexPort,
    W: WorkspaceGitPort + TestRunnerPort,
{
    run_delivery_inner(
        request,
        ledger,
        workspace_git,
        codex,
        Some(GovernedWriter {
            authority: writer_authority,
            guard: writer_guard,
        }),
    )
}

#[allow(clippy::too_many_lines)]
fn run_delivery_inner<L, C, W>(
    request: &DeliveryRunRequest,
    ledger: &mut L,
    workspace_git: &mut W,
    codex: &mut C,
    mut writer: Option<GovernedWriter<'_>>,
) -> Result<DeliveryReceipt, DeliveryOrchestratorError>
where
    L: DeliveryLedgerPort,
    C: DeliveryCodexPort,
    W: WorkspaceGitPort + TestRunnerPort,
{
    let intent = ledger
        .record_intent(request)
        .map_err(DeliveryOrchestratorError::Intent)?;

    assert_writer_current(
        &mut writer,
        ledger,
        request,
        &intent,
        DeliveryStage::WorkspacePrepare,
        false,
    )?;

    let workspace = match workspace_git.prepare(request, &intent) {
        Ok(evidence) => evidence,
        Err(error) => return finish_failure(ledger, request, &intent, error),
    };
    assert_writer_current(
        &mut writer,
        ledger,
        request,
        &intent,
        DeliveryStage::WorkspacePrepare,
        false,
    )?;
    let codex_request = match writer
        .as_ref()
        .map(|current| current.authority)
        .map_or_else(
            || CodexDeliveryRequest::new(request.clone(), intent.clone(), workspace.clone()),
            |authority| {
                CodexDeliveryRequest::new_governed(
                    request.clone(),
                    intent.clone(),
                    workspace.clone(),
                    authority.clone(),
                )
            },
        ) {
        Ok(request) => request,
        Err(error) => {
            return finish_contract_failure(
                ledger,
                request,
                &intent,
                DeliveryStage::WorkspacePrepare,
                error,
            );
        }
    };
    let codex_evidence = match codex.run_delivery(codex_request) {
        Ok(evidence) => evidence,
        Err(error) => return finish_failure(ledger, request, &intent, error),
    };
    assert_writer_current(
        &mut writer,
        ledger,
        request,
        &intent,
        DeliveryStage::Codex,
        true,
    )?;
    let changes = match workspace_git.inspect_changes(request, &intent, &workspace, &codex_evidence)
    {
        Ok(evidence) => evidence,
        Err(error) => return finish_failure(ledger, request, &intent, error),
    };
    assert_writer_current(
        &mut writer,
        ledger,
        request,
        &intent,
        DeliveryStage::ScopeVerification,
        true,
    )?;
    let test = match workspace_git.run_fixed(request, &workspace, &changes) {
        Ok(evidence) => evidence,
        Err(error) => return finish_failure(ledger, request, &intent, error),
    };
    assert_writer_current(
        &mut writer,
        ledger,
        request,
        &intent,
        DeliveryStage::FixedTest,
        true,
    )?;
    let git = match workspace_git.commit(request, &workspace, &changes, &test) {
        Ok(evidence) => evidence,
        Err(error) => return finish_failure(ledger, request, &intent, error),
    };
    assert_writer_current(
        &mut writer,
        ledger,
        request,
        &intent,
        DeliveryStage::GitCommit,
        true,
    )?;

    let completed = match CompletedDeliveryEvidence::new(
        request.clone(),
        intent.clone(),
        workspace,
        codex_evidence,
        changes,
        test,
        git,
    ) {
        Ok(evidence) => evidence,
        Err(error) => {
            return finish_post_commit_contract_failure(ledger, request, &intent, error);
        }
    };
    let outcome_request = match DeliveryOutcomeRequest::completed(request, completed) {
        Ok(outcome) => outcome,
        Err(error) => {
            return finish_post_commit_contract_failure(ledger, request, &intent, error);
        }
    };
    let outcome = ledger
        .record_outcome(&outcome_request)
        .map_err(outcome_persistence_after_durable_intent)?;
    let receipt = ledger
        .load_receipt(&request.status_request())
        .map_err(DeliveryOrchestratorError::ReceiptRead)?;
    if !receipt.matches_run(request)
        || receipt.status() != DeliveryTerminalStatus::Completed
        || receipt.outcome() != &outcome
    {
        return Err(DeliveryOrchestratorError::ReceiptMismatch);
    }
    Ok(receipt)
}

fn assert_writer_current<L: DeliveryLedgerPort>(
    writer: &mut Option<GovernedWriter<'_>>,
    ledger: &mut L,
    request: &DeliveryRunRequest,
    intent: &DurableIntentEvidence,
    stage: DeliveryStage,
    after_writer_effect: bool,
) -> Result<(), DeliveryOrchestratorError> {
    let Some(writer) = writer.as_mut() else {
        return Ok(());
    };
    let Err(error) = writer.guard.assert_current(writer.authority) else {
        return Ok(());
    };
    let ambiguous = after_writer_effect
        || error.kind() == lattice_ports::ControlledTaskExecutionErrorKind::Ambiguous;
    let failure = DeliveryPortError::new(
        stage,
        if ambiguous {
            PortErrorKind::Ambiguous
        } else {
            PortErrorKind::Denied
        },
        if ambiguous {
            DeliveryFailureCertainty::Ambiguous
        } else {
            DeliveryFailureCertainty::Known
        },
        error.code(),
    );
    finish_failure(ledger, request, intent, failure).map(|_| ())
}

/// Loads one terminal delivery receipt without invoking any other port.
///
/// # Errors
///
/// Returns a receipt error or cross-binding error when durable state cannot be
/// safely associated with the exact status request.
pub fn delivery_status<L: DeliveryLedgerPort>(
    request: &DeliveryStatusRequest,
    ledger: &mut L,
) -> Result<DeliveryReceipt, DeliveryOrchestratorError> {
    let receipt = ledger
        .load_receipt(request)
        .map_err(DeliveryOrchestratorError::ReceiptRead)?;
    if !receipt.matches_status_request(request) {
        return Err(DeliveryOrchestratorError::ReceiptMismatch);
    }
    Ok(receipt)
}

fn finish_contract_failure<L: DeliveryLedgerPort>(
    ledger: &mut L,
    request: &DeliveryRunRequest,
    intent: &DurableIntentEvidence,
    stage: DeliveryStage,
    error: DeliveryContractError,
) -> Result<DeliveryReceipt, DeliveryOrchestratorError> {
    let port_error = DeliveryPortError::new(
        stage,
        PortErrorKind::Malformed,
        DeliveryFailureCertainty::Known,
        "CONTRACT_EVIDENCE_REJECTED",
    );
    finish_failure(ledger, request, intent, port_error).map_err(|terminal| match terminal {
        DeliveryOrchestratorError::Terminal { .. } => DeliveryOrchestratorError::Contract(error),
        other => other,
    })
}

fn finish_post_commit_contract_failure<L: DeliveryLedgerPort>(
    ledger: &mut L,
    request: &DeliveryRunRequest,
    intent: &DurableIntentEvidence,
    _error: DeliveryContractError,
) -> Result<DeliveryReceipt, DeliveryOrchestratorError> {
    let port_error = DeliveryPortError::new(
        DeliveryStage::GitCommit,
        PortErrorKind::Ambiguous,
        DeliveryFailureCertainty::Ambiguous,
        "POST_COMMIT_EVIDENCE_REJECTED",
    );
    finish_failure(ledger, request, intent, port_error)
}

fn finish_failure<L: DeliveryLedgerPort>(
    ledger: &mut L,
    request: &DeliveryRunRequest,
    intent: &DurableIntentEvidence,
    cause: DeliveryPortError,
) -> Result<DeliveryReceipt, DeliveryOrchestratorError> {
    let ambiguous = cause.certainty() == DeliveryFailureCertainty::Ambiguous
        || cause.kind() == PortErrorKind::Ambiguous;
    let outcome_request = if ambiguous {
        DeliveryOutcomeRequest::reconciliation_required(
            request,
            intent,
            cause.stage(),
            cause.code(),
        )
    } else {
        DeliveryOutcomeRequest::failed(request, intent, cause.stage(), cause.code())
    }
    .map_err(DeliveryOrchestratorError::Contract)?;
    let expected_status = outcome_request.status();
    let outcome = ledger
        .record_outcome(&outcome_request)
        .map_err(outcome_persistence_after_durable_intent)?;
    let receipt = ledger
        .load_receipt(&request.status_request())
        .map_err(DeliveryOrchestratorError::ReceiptRead)?;
    if !receipt.matches_run(request)
        || receipt.status() != expected_status
        || receipt.outcome() != &outcome
    {
        return Err(DeliveryOrchestratorError::ReceiptMismatch);
    }
    Err(DeliveryOrchestratorError::Terminal {
        cause,
        receipt: Box::new(receipt),
    })
}

fn outcome_persistence_after_durable_intent(
    _error: DeliveryPortError,
) -> DeliveryOrchestratorError {
    DeliveryOrchestratorError::OutcomePersistence(DeliveryPortError::new(
        DeliveryStage::Outcome,
        PortErrorKind::Ambiguous,
        DeliveryFailureCertainty::Ambiguous,
        "OUTCOME_PERSISTENCE_AFTER_DURABLE_INTENT_UNKNOWN",
    ))
}
mod autonomy;

pub use autonomy::{
    AutonomyDecision, AutonomyDecisionReason, AutonomyIntent, AutonomyIntentVersion,
    AutonomyReceipt, ModelRecommendation, TaskKind, VerificationRecommendation, classify_autonomy,
};
mod window_closure;

pub use window_closure::{
    DurableHandoffReceipt, HandoffFieldStatus, KeepWindowOpenReason, WindowClosureDecision,
    WindowKind, classify_window_closure,
};
