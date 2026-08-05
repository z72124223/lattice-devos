//! Typed, I/O-free contracts for one bounded delivery execution.

use std::error::Error;
use std::fmt;

use crate::{ContentDigest, Invocation, RequestId};

const MAX_EVIDENCE_TEXT_BYTES: usize = 1_024;
const MAX_FAILURE_CODE_BYTES: usize = 128;

/// The sole executable delivery profile activated by TASK-032.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DeliveryProfile {
    /// Official/scripted Codex plus `PostgreSQL`, fixed verification, and local Git.
    Task032CodexPostgres,
}

impl DeliveryProfile {
    /// Returns the stable profile identity.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Task032CodexPostgres => "task032-codex-postgres-v1",
        }
    }
}

/// Ordered delivery stages used by evidence and fail-closed errors.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DeliveryStage {
    Intent,
    WorkspacePrepare,
    Codex,
    ScopeVerification,
    FixedTest,
    GitCommit,
    Outcome,
    Receipt,
}

/// Evidence origin for the Codex stage.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DeliveryRuntime {
    /// Deterministic scripted app-server used only as a test harness.
    ScriptedAcceptance,
    /// The official Codex app-server executable.
    OfficialCodexAppServer,
}

/// Terminal durable delivery classification.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DeliveryTerminalStatus {
    Completed,
    Failed,
    ReconciliationRequired,
}

/// Construction failures for typed delivery evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryContractError {
    /// One field is empty, unbounded, malformed, or a zero digest.
    InvalidValue { field: &'static str },
    /// Evidence belongs to a different request, intent, or prior stage.
    CrossBinding { field: &'static str },
    /// Terminal details do not match their declared status.
    InvalidTerminal,
}

impl fmt::Display for DeliveryContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidValue { field } => write!(formatter, "invalid delivery {field}"),
            Self::CrossBinding { field } => write!(formatter, "delivery cross-binding: {field}"),
            Self::InvalidTerminal => formatter.write_str("invalid delivery terminal evidence"),
        }
    }
}

impl Error for DeliveryContractError {}

/// Immutable composition-created request; it carries no command, SQL, secret,
/// provider configuration, or caller-selected path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliveryRunRequest {
    invocation: Invocation,
    profile: DeliveryProfile,
    configuration_digest: ContentDigest,
}

impl DeliveryRunRequest {
    /// Constructs one request bound to LATTICE-owned configuration.
    ///
    /// # Errors
    ///
    /// Rejects a zero configuration digest.
    pub fn new(
        invocation: Invocation,
        profile: DeliveryProfile,
        configuration_digest: ContentDigest,
    ) -> Result<Self, DeliveryContractError> {
        require_digest(&configuration_digest, "configuration_digest")?;
        Ok(Self {
            invocation,
            profile,
            configuration_digest,
        })
    }

    #[must_use]
    pub const fn invocation(&self) -> &Invocation {
        &self.invocation
    }

    #[must_use]
    pub const fn profile(&self) -> DeliveryProfile {
        self.profile
    }

    #[must_use]
    pub const fn configuration_digest(&self) -> &ContentDigest {
        &self.configuration_digest
    }

    /// Returns the read-only status binding for this exact run.
    #[must_use]
    pub fn status_request(&self) -> DeliveryStatusRequest {
        DeliveryStatusRequest {
            invocation: self.invocation.clone(),
            profile: self.profile,
            configuration_digest: self.configuration_digest.clone(),
        }
    }
}

/// Read-only status/receipt lookup binding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliveryStatusRequest {
    invocation: Invocation,
    profile: DeliveryProfile,
    configuration_digest: ContentDigest,
}

impl DeliveryStatusRequest {
    #[must_use]
    pub const fn invocation(&self) -> &Invocation {
        &self.invocation
    }

    #[must_use]
    pub const fn profile(&self) -> DeliveryProfile {
        self.profile
    }

    #[must_use]
    pub const fn configuration_digest(&self) -> &ContentDigest {
        &self.configuration_digest
    }

    #[must_use]
    pub fn matches_run(&self, request: &DeliveryRunRequest) -> bool {
        self == &request.status_request()
    }
}

/// Proof that the exact effect intent was durably committed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DurableIntentEvidence {
    binding: DeliveryStatusRequest,
    intent_digest: ContentDigest,
}

impl DurableIntentEvidence {
    /// Constructs request-bound durable-intent evidence.
    ///
    /// # Errors
    ///
    /// Rejects a zero digest.
    pub fn new(
        request: &DeliveryRunRequest,
        intent_digest: ContentDigest,
    ) -> Result<Self, DeliveryContractError> {
        require_digest(&intent_digest, "intent_digest")?;
        Ok(Self {
            binding: request.status_request(),
            intent_digest,
        })
    }

    #[must_use]
    pub const fn binding(&self) -> &DeliveryStatusRequest {
        &self.binding
    }

    #[must_use]
    pub const fn intent_digest(&self) -> &ContentDigest {
        &self.intent_digest
    }

    #[must_use]
    pub fn matches_run(&self, request: &DeliveryRunRequest) -> bool {
        self.binding.matches_run(request)
    }
}

/// Adapter-produced locator and baseline for a bounded workspace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedWorkspaceEvidence {
    binding: DeliveryStatusRequest,
    intent_digest: ContentDigest,
    workspace_id: String,
    workspace_locator: String,
    baseline_commit: String,
    evidence_digest: ContentDigest,
}

impl PreparedWorkspaceEvidence {
    /// Constructs prepared-workspace evidence after durable intent.
    ///
    /// # Errors
    ///
    /// Rejects cross-request intent, malformed adapter locators/IDs, malformed
    /// commits, and zero evidence digests.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        request: &DeliveryRunRequest,
        intent: &DurableIntentEvidence,
        workspace_id: impl Into<String>,
        workspace_locator: impl Into<String>,
        baseline_commit: impl Into<String>,
        evidence_digest: ContentDigest,
    ) -> Result<Self, DeliveryContractError> {
        if !intent.matches_run(request) {
            return Err(cross("workspace_intent"));
        }
        let workspace_id = require_text(workspace_id, "workspace_id")?;
        let workspace_locator = require_text(workspace_locator, "workspace_locator")?;
        let baseline_commit = require_commit(baseline_commit, "baseline_commit")?;
        require_digest(&evidence_digest, "workspace_evidence_digest")?;
        Ok(Self {
            binding: request.status_request(),
            intent_digest: intent.intent_digest.clone(),
            workspace_id,
            workspace_locator,
            baseline_commit,
            evidence_digest,
        })
    }

    #[must_use]
    pub const fn binding(&self) -> &DeliveryStatusRequest {
        &self.binding
    }

    #[must_use]
    pub const fn intent_digest(&self) -> &ContentDigest {
        &self.intent_digest
    }

    #[must_use]
    pub fn workspace_id(&self) -> &str {
        &self.workspace_id
    }

    #[must_use]
    pub fn workspace_locator(&self) -> &str {
        &self.workspace_locator
    }

    #[must_use]
    pub fn baseline_commit(&self) -> &str {
        &self.baseline_commit
    }

    #[must_use]
    pub const fn evidence_digest(&self) -> &ContentDigest {
        &self.evidence_digest
    }

    #[must_use]
    pub fn matches(&self, request: &DeliveryRunRequest, intent: &DurableIntentEvidence) -> bool {
        self.binding.matches_run(request)
            && intent.matches_run(request)
            && self.intent_digest == *intent.intent_digest()
    }
}

/// Complete typed input to the sole delivery Codex lane.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexDeliveryRequest {
    request: DeliveryRunRequest,
    intent: DurableIntentEvidence,
    workspace: PreparedWorkspaceEvidence,
}

impl CodexDeliveryRequest {
    /// Binds Codex to the exact durable intent and prepared workspace.
    ///
    /// # Errors
    ///
    /// Rejects cross-bound prior-stage evidence.
    pub fn new(
        request: DeliveryRunRequest,
        intent: DurableIntentEvidence,
        workspace: PreparedWorkspaceEvidence,
    ) -> Result<Self, DeliveryContractError> {
        if !workspace.matches(&request, &intent) {
            return Err(cross("codex_workspace"));
        }
        Ok(Self {
            request,
            intent,
            workspace,
        })
    }

    #[must_use]
    pub const fn request(&self) -> &DeliveryRunRequest {
        &self.request
    }

    #[must_use]
    pub const fn intent(&self) -> &DurableIntentEvidence {
        &self.intent
    }

    #[must_use]
    pub const fn workspace(&self) -> &PreparedWorkspaceEvidence {
        &self.workspace
    }
}

/// Terminal, exact-identity evidence from one Codex delivery turn.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexDeliveryEvidence {
    binding: DeliveryStatusRequest,
    intent_digest: ContentDigest,
    workspace_evidence_digest: ContentDigest,
    runtime: DeliveryRuntime,
    launcher_locator: String,
    version: String,
    launcher_sha256: ContentDigest,
    schema_bundle_sha256: ContentDigest,
    schema_file_count: u32,
    thread_id: String,
    turn_id: String,
    output_digest: ContentDigest,
}

impl CodexDeliveryEvidence {
    /// Constructs terminal Codex evidence from the exact lane request.
    ///
    /// # Errors
    ///
    /// Rejects malformed identity/session fields or zero evidence digests.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        request: &CodexDeliveryRequest,
        runtime: DeliveryRuntime,
        launcher_locator: impl Into<String>,
        version: impl Into<String>,
        launcher_sha256: ContentDigest,
        schema_bundle_sha256: ContentDigest,
        schema_file_count: u32,
        thread_id: impl Into<String>,
        turn_id: impl Into<String>,
        output_digest: ContentDigest,
    ) -> Result<Self, DeliveryContractError> {
        let launcher_locator = require_text(launcher_locator, "codex_launcher_locator")?;
        let version = require_text(version, "codex_version")?;
        let thread_id = require_text(thread_id, "codex_thread_id")?;
        let turn_id = require_text(turn_id, "codex_turn_id")?;
        require_digest(&launcher_sha256, "codex_launcher_sha256")?;
        require_digest(&schema_bundle_sha256, "codex_schema_bundle_sha256")?;
        require_digest(&output_digest, "codex_output_digest")?;
        if schema_file_count == 0 {
            return Err(invalid("codex_schema_file_count"));
        }
        Ok(Self {
            binding: request.request.status_request(),
            intent_digest: request.intent.intent_digest.clone(),
            workspace_evidence_digest: request.workspace.evidence_digest.clone(),
            runtime,
            launcher_locator,
            version,
            launcher_sha256,
            schema_bundle_sha256,
            schema_file_count,
            thread_id,
            turn_id,
            output_digest,
        })
    }

    #[must_use]
    pub const fn binding(&self) -> &DeliveryStatusRequest {
        &self.binding
    }

    #[must_use]
    pub const fn intent_digest(&self) -> &ContentDigest {
        &self.intent_digest
    }

    #[must_use]
    pub const fn workspace_evidence_digest(&self) -> &ContentDigest {
        &self.workspace_evidence_digest
    }

    #[must_use]
    pub const fn runtime(&self) -> DeliveryRuntime {
        self.runtime
    }

    #[must_use]
    pub fn launcher_locator(&self) -> &str {
        &self.launcher_locator
    }

    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    #[must_use]
    pub const fn launcher_sha256(&self) -> &ContentDigest {
        &self.launcher_sha256
    }

    #[must_use]
    pub const fn schema_bundle_sha256(&self) -> &ContentDigest {
        &self.schema_bundle_sha256
    }

    #[must_use]
    pub const fn schema_file_count(&self) -> u32 {
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
    pub const fn output_digest(&self) -> &ContentDigest {
        &self.output_digest
    }

    #[must_use]
    pub fn matches(
        &self,
        request: &DeliveryRunRequest,
        intent: &DurableIntentEvidence,
        workspace: &PreparedWorkspaceEvidence,
    ) -> bool {
        self.binding.matches_run(request)
            && self.intent_digest == *intent.intent_digest()
            && self.workspace_evidence_digest == *workspace.evidence_digest()
    }
}

/// Exact changed-path inspection evidence for the fixed delivery profile.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceChangeEvidence {
    binding: DeliveryStatusRequest,
    workspace_evidence_digest: ContentDigest,
    codex_output_digest: ContentDigest,
    changed_paths_digest: ContentDigest,
    evidence_digest: ContentDigest,
}

impl WorkspaceChangeEvidence {
    /// Constructs passing scope evidence.
    ///
    /// # Errors
    ///
    /// Rejects cross-bound Codex/workspace evidence or zero digests.
    pub fn new(
        request: &DeliveryRunRequest,
        intent: &DurableIntentEvidence,
        workspace: &PreparedWorkspaceEvidence,
        codex: &CodexDeliveryEvidence,
        changed_paths_digest: ContentDigest,
        evidence_digest: ContentDigest,
    ) -> Result<Self, DeliveryContractError> {
        if !workspace.matches(request, intent) || !codex.matches(request, intent, workspace) {
            return Err(cross("workspace_changes"));
        }
        require_digest(&changed_paths_digest, "changed_paths_digest")?;
        require_digest(&evidence_digest, "change_evidence_digest")?;
        Ok(Self {
            binding: request.status_request(),
            workspace_evidence_digest: workspace.evidence_digest.clone(),
            codex_output_digest: codex.output_digest.clone(),
            changed_paths_digest,
            evidence_digest,
        })
    }

    #[must_use]
    pub const fn binding(&self) -> &DeliveryStatusRequest {
        &self.binding
    }

    #[must_use]
    pub const fn workspace_evidence_digest(&self) -> &ContentDigest {
        &self.workspace_evidence_digest
    }

    #[must_use]
    pub const fn codex_output_digest(&self) -> &ContentDigest {
        &self.codex_output_digest
    }

    #[must_use]
    pub const fn changed_paths_digest(&self) -> &ContentDigest {
        &self.changed_paths_digest
    }

    #[must_use]
    pub const fn evidence_digest(&self) -> &ContentDigest {
        &self.evidence_digest
    }

    #[must_use]
    pub fn matches(
        &self,
        request: &DeliveryRunRequest,
        workspace: &PreparedWorkspaceEvidence,
        codex: &CodexDeliveryEvidence,
    ) -> bool {
        self.binding.matches_run(request)
            && self.workspace_evidence_digest == *workspace.evidence_digest()
            && self.codex_output_digest == *codex.output_digest()
    }
}

/// Passing evidence from the sole fixed TASK-032 test profile.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FixedTestEvidence {
    binding: DeliveryStatusRequest,
    change_evidence_digest: ContentDigest,
    evidence_digest: ContentDigest,
}

impl FixedTestEvidence {
    /// Constructs passing fixed-test evidence.
    ///
    /// # Errors
    ///
    /// Rejects cross-bound change evidence or a zero digest.
    pub fn new(
        request: &DeliveryRunRequest,
        changes: &WorkspaceChangeEvidence,
        evidence_digest: ContentDigest,
    ) -> Result<Self, DeliveryContractError> {
        if !changes.binding.matches_run(request) {
            return Err(cross("fixed_test_changes"));
        }
        require_digest(&evidence_digest, "test_evidence_digest")?;
        Ok(Self {
            binding: request.status_request(),
            change_evidence_digest: changes.evidence_digest.clone(),
            evidence_digest,
        })
    }

    #[must_use]
    pub const fn binding(&self) -> &DeliveryStatusRequest {
        &self.binding
    }

    #[must_use]
    pub const fn change_evidence_digest(&self) -> &ContentDigest {
        &self.change_evidence_digest
    }

    #[must_use]
    pub const fn evidence_digest(&self) -> &ContentDigest {
        &self.evidence_digest
    }

    #[must_use]
    pub fn matches(&self, request: &DeliveryRunRequest, changes: &WorkspaceChangeEvidence) -> bool {
        self.binding.matches_run(request)
            && self.change_evidence_digest == *changes.evidence_digest()
    }
}

/// Verified local Git commit evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitCommitEvidence {
    binding: DeliveryStatusRequest,
    test_evidence_digest: ContentDigest,
    parent_commit: String,
    commit: String,
    evidence_digest: ContentDigest,
}

impl GitCommitEvidence {
    /// Constructs commit evidence after scope and fixed-test success.
    ///
    /// # Errors
    ///
    /// Rejects cross-bound evidence, malformed/equal commits, or a zero digest.
    pub fn new(
        request: &DeliveryRunRequest,
        changes: &WorkspaceChangeEvidence,
        test: &FixedTestEvidence,
        parent_commit: impl Into<String>,
        commit: impl Into<String>,
        evidence_digest: ContentDigest,
    ) -> Result<Self, DeliveryContractError> {
        if !test.matches(request, changes) {
            return Err(cross("git_test"));
        }
        let parent_commit = require_commit(parent_commit, "git_parent_commit")?;
        let commit = require_commit(commit, "git_commit")?;
        if parent_commit == commit {
            return Err(invalid("git_commit_transition"));
        }
        require_digest(&evidence_digest, "git_evidence_digest")?;
        Ok(Self {
            binding: request.status_request(),
            test_evidence_digest: test.evidence_digest.clone(),
            parent_commit,
            commit,
            evidence_digest,
        })
    }

    #[must_use]
    pub const fn binding(&self) -> &DeliveryStatusRequest {
        &self.binding
    }

    #[must_use]
    pub const fn test_evidence_digest(&self) -> &ContentDigest {
        &self.test_evidence_digest
    }

    #[must_use]
    pub fn parent_commit(&self) -> &str {
        &self.parent_commit
    }

    #[must_use]
    pub fn commit(&self) -> &str {
        &self.commit
    }

    #[must_use]
    pub const fn evidence_digest(&self) -> &ContentDigest {
        &self.evidence_digest
    }
}

/// Complete ordered success bundle supplied to the durable ledger.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletedDeliveryEvidence {
    request: DeliveryRunRequest,
    intent: DurableIntentEvidence,
    workspace: PreparedWorkspaceEvidence,
    codex: CodexDeliveryEvidence,
    changes: WorkspaceChangeEvidence,
    test: FixedTestEvidence,
    git: GitCommitEvidence,
}

impl CompletedDeliveryEvidence {
    /// Constructs the complete success chain.
    ///
    /// # Errors
    ///
    /// Rejects any substituted or out-of-order stage evidence.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        request: DeliveryRunRequest,
        intent: DurableIntentEvidence,
        workspace: PreparedWorkspaceEvidence,
        codex: CodexDeliveryEvidence,
        changes: WorkspaceChangeEvidence,
        test: FixedTestEvidence,
        git: GitCommitEvidence,
    ) -> Result<Self, DeliveryContractError> {
        if !workspace.matches(&request, &intent)
            || !codex.matches(&request, &intent, &workspace)
            || !changes.matches(&request, &workspace, &codex)
            || !test.matches(&request, &changes)
            || !git.binding.matches_run(&request)
            || git.test_evidence_digest != *test.evidence_digest()
        {
            return Err(cross("completed_chain"));
        }
        Ok(Self {
            request,
            intent,
            workspace,
            codex,
            changes,
            test,
            git,
        })
    }

    #[must_use]
    pub const fn request(&self) -> &DeliveryRunRequest {
        &self.request
    }

    #[must_use]
    pub const fn intent(&self) -> &DurableIntentEvidence {
        &self.intent
    }

    #[must_use]
    pub const fn workspace(&self) -> &PreparedWorkspaceEvidence {
        &self.workspace
    }

    #[must_use]
    pub const fn codex(&self) -> &CodexDeliveryEvidence {
        &self.codex
    }

    #[must_use]
    pub const fn changes(&self) -> &WorkspaceChangeEvidence {
        &self.changes
    }

    #[must_use]
    pub const fn test(&self) -> &FixedTestEvidence {
        &self.test
    }

    #[must_use]
    pub const fn git(&self) -> &GitCommitEvidence {
        &self.git
    }
}

/// Terminal record requested from the durable delivery ledger.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliveryOutcomeRequest {
    binding: DeliveryStatusRequest,
    intent_digest: ContentDigest,
    status: DeliveryTerminalStatus,
    completed: Option<CompletedDeliveryEvidence>,
    failure_stage: Option<DeliveryStage>,
    failure_code: Option<String>,
}

impl DeliveryOutcomeRequest {
    /// Constructs a completed terminal record.
    ///
    /// # Errors
    ///
    /// Rejects a success chain belonging to another request.
    pub fn completed(
        request: &DeliveryRunRequest,
        completed: CompletedDeliveryEvidence,
    ) -> Result<Self, DeliveryContractError> {
        if completed.request != *request {
            return Err(cross("completed_request"));
        }
        Ok(Self {
            binding: request.status_request(),
            intent_digest: completed.intent.intent_digest.clone(),
            status: DeliveryTerminalStatus::Completed,
            completed: Some(completed),
            failure_stage: None,
            failure_code: None,
        })
    }

    /// Constructs a known failed terminal record.
    ///
    /// # Errors
    ///
    /// Rejects cross-bound intent or an invalid stable code.
    pub fn failed(
        request: &DeliveryRunRequest,
        intent: &DurableIntentEvidence,
        stage: DeliveryStage,
        code: impl Into<String>,
    ) -> Result<Self, DeliveryContractError> {
        Self::failure(request, intent, DeliveryTerminalStatus::Failed, stage, code)
    }

    /// Constructs a reconciliation-required terminal record.
    ///
    /// # Errors
    ///
    /// Rejects cross-bound intent or an invalid stable code.
    pub fn reconciliation_required(
        request: &DeliveryRunRequest,
        intent: &DurableIntentEvidence,
        stage: DeliveryStage,
        code: impl Into<String>,
    ) -> Result<Self, DeliveryContractError> {
        Self::failure(
            request,
            intent,
            DeliveryTerminalStatus::ReconciliationRequired,
            stage,
            code,
        )
    }

    fn failure(
        request: &DeliveryRunRequest,
        intent: &DurableIntentEvidence,
        status: DeliveryTerminalStatus,
        stage: DeliveryStage,
        code: impl Into<String>,
    ) -> Result<Self, DeliveryContractError> {
        if !intent.matches_run(request) || status == DeliveryTerminalStatus::Completed {
            return Err(DeliveryContractError::InvalidTerminal);
        }
        let code = require_failure_code(code)?;
        Ok(Self {
            binding: request.status_request(),
            intent_digest: intent.intent_digest.clone(),
            status,
            completed: None,
            failure_stage: Some(stage),
            failure_code: Some(code),
        })
    }

    #[must_use]
    pub const fn binding(&self) -> &DeliveryStatusRequest {
        &self.binding
    }

    #[must_use]
    pub const fn intent_digest(&self) -> &ContentDigest {
        &self.intent_digest
    }

    #[must_use]
    pub const fn status(&self) -> DeliveryTerminalStatus {
        self.status
    }

    #[must_use]
    pub const fn completed_evidence(&self) -> Option<&CompletedDeliveryEvidence> {
        self.completed.as_ref()
    }

    #[must_use]
    pub const fn failure_stage(&self) -> Option<DeliveryStage> {
        self.failure_stage
    }

    #[must_use]
    pub fn failure_code(&self) -> Option<&str> {
        self.failure_code.as_deref()
    }
}

/// Durable terminal outcome evidence returned after persistence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliveryOutcomeEvidence {
    request: DeliveryOutcomeRequest,
    outcome_digest: ContentDigest,
}

impl DeliveryOutcomeEvidence {
    /// Constructs durable outcome evidence.
    ///
    /// # Errors
    ///
    /// Rejects a zero outcome digest.
    pub fn new(
        request: DeliveryOutcomeRequest,
        outcome_digest: ContentDigest,
    ) -> Result<Self, DeliveryContractError> {
        require_digest(&outcome_digest, "outcome_digest")?;
        Ok(Self {
            request,
            outcome_digest,
        })
    }

    #[must_use]
    pub const fn request(&self) -> &DeliveryOutcomeRequest {
        &self.request
    }

    #[must_use]
    pub const fn outcome_digest(&self) -> &ContentDigest {
        &self.outcome_digest
    }
}

/// Restart-safe, cross-bound terminal delivery receipt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliveryReceipt {
    outcome: DeliveryOutcomeEvidence,
    receipt_digest: ContentDigest,
}

impl DeliveryReceipt {
    /// Constructs a terminal receipt from one durable outcome.
    ///
    /// # Errors
    ///
    /// Rejects a zero receipt digest.
    pub fn new(
        outcome: DeliveryOutcomeEvidence,
        receipt_digest: ContentDigest,
    ) -> Result<Self, DeliveryContractError> {
        require_digest(&receipt_digest, "receipt_digest")?;
        Ok(Self {
            outcome,
            receipt_digest,
        })
    }

    #[must_use]
    pub const fn outcome(&self) -> &DeliveryOutcomeEvidence {
        &self.outcome
    }

    #[must_use]
    pub const fn status(&self) -> DeliveryTerminalStatus {
        self.outcome.request.status
    }

    #[must_use]
    pub const fn receipt_digest(&self) -> &ContentDigest {
        &self.receipt_digest
    }

    #[must_use]
    pub fn matches_status_request(&self, request: &DeliveryStatusRequest) -> bool {
        self.outcome.request.binding == *request
    }

    #[must_use]
    pub fn matches_run(&self, request: &DeliveryRunRequest) -> bool {
        self.matches_status_request(&request.status_request())
    }
}

fn require_digest(
    digest: &ContentDigest,
    field: &'static str,
) -> Result<(), DeliveryContractError> {
    if digest.as_str().bytes().all(|byte| byte == b'0') {
        Err(invalid(field))
    } else {
        Ok(())
    }
}

fn require_text(
    value: impl Into<String>,
    field: &'static str,
) -> Result<String, DeliveryContractError> {
    let value = value.into();
    if value.trim().is_empty()
        || value.len() > MAX_EVIDENCE_TEXT_BYTES
        || value.bytes().any(|byte| matches!(byte, 0 | b'\r' | b'\n'))
    {
        Err(invalid(field))
    } else {
        Ok(value)
    }
}

fn require_failure_code(value: impl Into<String>) -> Result<String, DeliveryContractError> {
    let value = value.into();
    if value.is_empty()
        || value.len() > MAX_FAILURE_CODE_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        Err(invalid("failure_code"))
    } else {
        Ok(value)
    }
}

fn require_commit(
    value: impl Into<String>,
    field: &'static str,
) -> Result<String, DeliveryContractError> {
    let value = value.into();
    if !matches!(value.len(), 40 | 64)
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Err(invalid(field))
    } else {
        Ok(value)
    }
}

const fn invalid(field: &'static str) -> DeliveryContractError {
    DeliveryContractError::InvalidValue { field }
}

const fn cross(field: &'static str) -> DeliveryContractError {
    DeliveryContractError::CrossBinding { field }
}

/// Returns the request ID without exposing the rest of the run binding.
#[must_use]
pub const fn delivery_request_id(request: &DeliveryStatusRequest) -> &RequestId {
    request.invocation.request_id()
}
