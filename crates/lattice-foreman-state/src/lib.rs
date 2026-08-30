//! Secret-free foreman snapshot validation, replay projection, and watchdog logic.

use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256};
use std::collections::BTreeMap;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

const SNAPSHOT_SCHEMA: &str = "lattice.foreman-snapshot/1.0";
const EPISTEMIC_SCHEMA: &str = "lattice.foreman-epistemic/1.0";
const DEPENDENCY_BLOCKER_PREFIX: &str = "dependency:v1:";
const MAX_REFERENCE_BYTES: usize = 256;
const MAX_CHECKPOINT_ID_BYTES: usize = 64;
const MODEL_SELECTION_SCHEMA: &str = "lattice.foreman-model-selection";
const MODEL_SELECTION_VERSION: &str = "1.0";
const WORKER_BUDGET_SCHEMA: &str = "lattice.foreman-worker-budget";
const WORKER_BUDGET_VERSION: &str = "1.0";
const CONTINUATION_SCHEMA: &str = "lattice.foreman-continuation-summary";
const CONTINUATION_VERSION: &str = "1.0";
const ATTEMPT_PACKET_SCHEMA: &str = "lattice.foreman-attempt-packet";
const ATTEMPT_PACKET_VERSION: &str = "1.0";
const NATIVE_WINDOWS_EXECUTION_ENVIRONMENT_REF: &str =
    "execution-environment:sha256:0000000000000000000000000000000000000000000000000000000000000001";
const ATTEMPT_STATE_SCHEMA: &str = "lattice.foreman-attempt-state";
const ATTEMPT_STATE_VERSION: &str = "1.0";
const MEANINGFUL_PROGRESS_SCHEMA: &str = "lattice.foreman-meaningful-progress";
const MEANINGFUL_PROGRESS_VERSION: &str = "1.0";
/// Maximum retained repair-continuation text included in a worker packet.
pub const MAX_CONTINUATION_BYTES: usize = 512;

/// Product-wide active managed-worker capacity.
pub const MAX_GLOBAL_ACTIVE_ATTEMPTS: u8 = 4;
/// Default and currently admitted active attempts for one task.
pub const DEFAULT_PER_TASK_ACTIVE_ATTEMPTS: u8 = 1;
/// Maximum repair retries after the initial attempt.
pub const MAX_REPAIR_RETRIES: u8 = 2;
/// Initial attempt plus the maximum two repair retries.
pub const MAX_ATTEMPTS: u8 = MAX_REPAIR_RETRIES + 1;

/// The only durable foreman identity admitted to the product coordination
/// stream. Git evidence remains observed per checkpoint, but this identity is
/// fixed by the server and cannot be supplied by an MCP caller.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SoleForemanBinding;

impl SoleForemanBinding {
    pub const WORKER: &'static str = "sole-foreman-v1";
    pub const THREAD: &'static str = "lattice-devos-sole-foreman-v1";
    pub const TASK: &'static str = "TASK-FOREMAN-COORDINATION";

    /// Constructs one server-owned Git observation for the fixed identity.
    ///
    /// # Errors
    ///
    /// Rejects malformed Git evidence.
    pub fn observe_git(
        branch: impl Into<String>,
        worktree: impl Into<String>,
        head: impl Into<String>,
    ) -> Result<ForemanServerObservation, SnapshotError> {
        ForemanServerObservation::new(
            Self::WORKER,
            Self::THREAD,
            Self::TASK,
            branch,
            worktree,
            head,
        )
    }

    /// Verifies that a retained or proposed snapshot belongs to the sole
    /// product foreman rather than an arbitrary generic worker identity.
    #[must_use]
    pub fn matches(snapshot: &ForemanSnapshot) -> bool {
        snapshot.worker() == Self::WORKER
            && snapshot.thread() == Self::THREAD
            && snapshot.task() == Self::TASK
    }
}

/// Closed worker coordination state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ForemanState {
    Active,
    Blocked,
    Completed,
}

/// Stable failures for the pure managed-worker domain.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkerAttemptError {
    MalformedField,
    ForbiddenContent,
    InvalidModelReason,
    MissingEvidence,
    InvalidBudget,
    InvalidAttempt,
    InvalidPhase,
    DigestFailure,
}

/// The complete model allowlist for managed workers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerModel {
    Luna,
    Terra,
    Sol,
}

impl WorkerModel {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Luna => "gpt-5.6-luna",
            Self::Terra => "gpt-5.6-terra",
            Self::Sol => "gpt-5.6-sol",
        }
    }

    /// Parses only the product model allowlist.
    ///
    /// # Errors
    ///
    /// Rejects unavailable aliases, older model names, and case changes.
    pub fn from_persisted(value: &str) -> Result<Self, WorkerAttemptError> {
        match value {
            "gpt-5.6-luna" => Ok(Self::Luna),
            "gpt-5.6-terra" => Ok(Self::Terra),
            "gpt-5.6-sol" => Ok(Self::Sol),
            _ => Err(WorkerAttemptError::MalformedField),
        }
    }
}

/// Closed reasoning-effort values retained in a worker packet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReasoningEffort {
    Low,
    Medium,
    High,
    XHigh,
    Max,
    Ultra,
}

impl ReasoningEffort {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
            Self::Max => "max",
            Self::Ultra => "ultra",
        }
    }

    /// Parses one exact reasoning-effort spelling.
    ///
    /// # Errors
    ///
    /// Rejects arbitrary provider strings and case substitutions.
    pub fn from_persisted(value: &str) -> Result<Self, WorkerAttemptError> {
        match value {
            "low" => Ok(Self::Low),
            "medium" => Ok(Self::Medium),
            "high" => Ok(Self::High),
            "xhigh" => Ok(Self::XHigh),
            "max" => Ok(Self::Max),
            "ultra" => Ok(Self::Ultra),
            _ => Err(WorkerAttemptError::MalformedField),
        }
    }
}

/// Closed reason for selecting a managed-worker model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelReason {
    BoundedStateEvidenceDocumentation,
    RoutineEngineering,
    P0,
    Architecture,
    Security,
    HighRisk,
    TerraInsufficient,
}

impl ModelReason {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BoundedStateEvidenceDocumentation => "BOUNDED_STATE_EVIDENCE_DOCUMENTATION",
            Self::RoutineEngineering => "ROUTINE_ENGINEERING",
            Self::P0 => "P0",
            Self::Architecture => "ARCHITECTURE",
            Self::Security => "SECURITY",
            Self::HighRisk => "HIGH_RISK",
            Self::TerraInsufficient => "TERRA_INSUFFICIENT",
        }
    }

    /// Reconstructs one exact persisted routing reason.
    ///
    /// # Errors
    ///
    /// Rejects arbitrary provider text, aliases, and case substitutions.
    pub fn from_persisted(value: &str) -> Result<Self, WorkerAttemptError> {
        match value {
            "BOUNDED_STATE_EVIDENCE_DOCUMENTATION" => Ok(Self::BoundedStateEvidenceDocumentation),
            "ROUTINE_ENGINEERING" => Ok(Self::RoutineEngineering),
            "P0" => Ok(Self::P0),
            "ARCHITECTURE" => Ok(Self::Architecture),
            "SECURITY" => Ok(Self::Security),
            "HIGH_RISK" => Ok(Self::HighRisk),
            "TERRA_INSUFFICIENT" => Ok(Self::TerraInsufficient),
            _ => Err(WorkerAttemptError::MalformedField),
        }
    }

    /// Returns whether this closed reason is valid for the selected model.
    #[must_use]
    pub const fn is_allowed_for(self, model: WorkerModel) -> bool {
        match model {
            WorkerModel::Luna => matches!(self, Self::BoundedStateEvidenceDocumentation),
            WorkerModel::Terra => matches!(self, Self::RoutineEngineering),
            WorkerModel::Sol => matches!(
                self,
                Self::P0
                    | Self::Architecture
                    | Self::Security
                    | Self::HighRisk
                    | Self::TerraInsufficient
            ),
        }
    }
}

/// Validated model, effort, and routing rationale. Availability remains an
/// injected server observation; this type never performs provider I/O.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelSelection {
    model: WorkerModel,
    reasoning: ReasoningEffort,
    reason: ModelReason,
    evidence_ref: Option<String>,
    digest: String,
}

impl ModelSelection {
    /// Constructs one policy-compatible model selection.
    ///
    /// # Errors
    ///
    /// Rejects a reason outside the selected model's role or a retained
    /// Terra-insufficiency claim without exact evidence.
    pub fn new(
        model: WorkerModel,
        reasoning: ReasoningEffort,
        reason: ModelReason,
        evidence_ref: Option<&str>,
    ) -> Result<Self, WorkerAttemptError> {
        if !reason.is_allowed_for(model) {
            return Err(WorkerAttemptError::InvalidModelReason);
        }
        if reason == ModelReason::TerraInsufficient && evidence_ref.is_none() {
            return Err(WorkerAttemptError::MissingEvidence);
        }
        let evidence_ref = evidence_ref
            .map(|value| attempt_digest_pointer(value, "evidence"))
            .transpose()?;
        let value = CanonicalValue::Object(vec![
            (
                "model".to_owned(),
                CanonicalValue::String(model.as_str().to_owned()),
            ),
            (
                "reasoning".to_owned(),
                CanonicalValue::String(reasoning.as_str().to_owned()),
            ),
            (
                "reason".to_owned(),
                CanonicalValue::String(reason.as_str().to_owned()),
            ),
            (
                "evidence_ref".to_owned(),
                CanonicalValue::String(evidence_ref.clone().unwrap_or_default()),
            ),
        ]);
        let digest = canonical_digest_pointer(
            MODEL_SELECTION_SCHEMA,
            MODEL_SELECTION_VERSION,
            "model-selection",
            &value,
        )?;
        Ok(Self {
            model,
            reasoning,
            reason,
            evidence_ref,
            digest,
        })
    }

    #[must_use]
    pub const fn model(&self) -> WorkerModel {
        self.model
    }

    #[must_use]
    pub const fn reasoning(&self) -> ReasoningEffort {
        self.reasoning
    }

    #[must_use]
    pub const fn reason(&self) -> ModelReason {
        self.reason
    }

    #[must_use]
    pub fn evidence_ref(&self) -> Option<&str> {
        self.evidence_ref.as_deref()
    }

    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }
}

/// A quoted currency bound is structurally distinct from unavailable cost.
/// `Unavailable` must never be serialized as a numeric zero.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExternalCostBudget {
    Unavailable,
    LimitMicros(u64),
}

impl ExternalCostBudget {
    #[must_use]
    pub const fn status(self) -> &'static str {
        match self {
            Self::Unavailable => "UNAVAILABLE",
            Self::LimitMicros(_) => "LIMIT_MICROS",
        }
    }

    fn amount(self) -> String {
        match self {
            Self::Unavailable => String::new(),
            Self::LimitMicros(value) => value.to_string(),
        }
    }

    #[must_use]
    pub const fn limit_micros(self) -> Option<u64> {
        match self {
            Self::Unavailable => None,
            Self::LimitMicros(value) => Some(value),
        }
    }
}

/// Immutable server-owned capacity, retry, time, token, model-call, cost, and
/// deadline limits bound into each managed-worker packet.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkerBudget {
    global_active_limit: u8,
    per_task_active_limit: u8,
    repair_retry_limit: u8,
    max_duration_seconds: u64,
    max_total_tokens: u64,
    max_model_calls: u32,
    external_cost: ExternalCostBudget,
    deadline_at: String,
    digest: String,
}

impl WorkerBudget {
    /// Constructs a closed budget that cannot exceed product capacity or the
    /// two-repair limit.
    ///
    /// # Errors
    ///
    /// Rejects zero resource bounds, excessive capacity/retries, and a
    /// non-canonical UTC deadline.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        global_active_limit: u8,
        per_task_active_limit: u8,
        repair_retry_limit: u8,
        max_duration_seconds: u64,
        max_total_tokens: u64,
        max_model_calls: u32,
        external_cost: ExternalCostBudget,
        deadline_at: impl Into<String>,
    ) -> Result<Self, WorkerAttemptError> {
        if !(1..=MAX_GLOBAL_ACTIVE_ATTEMPTS).contains(&global_active_limit)
            || !(1..=global_active_limit).contains(&per_task_active_limit)
            || repair_retry_limit > MAX_REPAIR_RETRIES
            || max_duration_seconds == 0
            || max_total_tokens == 0
            || max_model_calls == 0
        {
            return Err(WorkerAttemptError::InvalidBudget);
        }
        let deadline_at = attempt_timestamp(deadline_at.into())?;
        let value = CanonicalValue::Object(vec![
            text_canonical("global_active_limit", &global_active_limit),
            text_canonical("per_task_active_limit", &per_task_active_limit),
            text_canonical("repair_retry_limit", &repair_retry_limit),
            text_canonical("max_duration_seconds", &max_duration_seconds),
            text_canonical("max_total_tokens", &max_total_tokens),
            text_canonical("max_model_calls", &max_model_calls),
            (
                "external_cost_status".to_owned(),
                CanonicalValue::String(external_cost.status().to_owned()),
            ),
            (
                "external_cost_micros".to_owned(),
                CanonicalValue::String(external_cost.amount()),
            ),
            (
                "deadline_at".to_owned(),
                CanonicalValue::String(deadline_at.clone()),
            ),
        ]);
        let digest = canonical_digest_pointer(
            WORKER_BUDGET_SCHEMA,
            WORKER_BUDGET_VERSION,
            "budget",
            &value,
        )?;
        Ok(Self {
            global_active_limit,
            per_task_active_limit,
            repair_retry_limit,
            max_duration_seconds,
            max_total_tokens,
            max_model_calls,
            external_cost,
            deadline_at,
            digest,
        })
    }

    #[must_use]
    pub const fn global_active_limit(&self) -> u8 {
        self.global_active_limit
    }

    #[must_use]
    pub const fn per_task_active_limit(&self) -> u8 {
        self.per_task_active_limit
    }

    #[must_use]
    pub const fn repair_retry_limit(&self) -> u8 {
        self.repair_retry_limit
    }

    #[must_use]
    pub const fn max_attempts(&self) -> u8 {
        self.repair_retry_limit + 1
    }

    #[must_use]
    pub const fn max_duration_seconds(&self) -> u64 {
        self.max_duration_seconds
    }

    #[must_use]
    pub const fn max_total_tokens(&self) -> u64 {
        self.max_total_tokens
    }

    #[must_use]
    pub const fn max_model_calls(&self) -> u32 {
        self.max_model_calls
    }

    #[must_use]
    pub const fn external_cost(&self) -> ExternalCostBudget {
        self.external_cost
    }

    #[must_use]
    pub fn deadline_at(&self) -> &str {
        &self.deadline_at
    }

    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }

    #[must_use]
    pub const fn allows_attempt(&self, attempt: u8) -> bool {
        attempt > 0 && attempt <= self.max_attempts()
    }
}

/// A bounded continuation summary retained for a repair attempt. It is data,
/// never a command or an authority grant.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContinuationSummary {
    text: String,
    digest: String,
}

impl ContinuationSummary {
    /// # Errors
    ///
    /// Rejects empty, oversized, control-bearing, prompt-like, credential-like,
    /// or unredacted credential-bearing URL text.
    pub fn new(text: impl Into<String>) -> Result<Self, WorkerAttemptError> {
        let text = text.into();
        if text.is_empty()
            || text.len() > MAX_CONTINUATION_BYTES
            || text.trim() != text
            || text.chars().any(char::is_control)
        {
            return Err(WorkerAttemptError::MalformedField);
        }
        if looks_worker_secret_like(&text) {
            return Err(WorkerAttemptError::ForbiddenContent);
        }
        let digest = canonical_digest_pointer(
            CONTINUATION_SCHEMA,
            CONTINUATION_VERSION,
            "continuation",
            &CanonicalValue::String(text.clone()),
        )?;
        Ok(Self { text, digest })
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }
}

/// Immutable identity of one worker attempt. Resolved paths, prompts,
/// commands and credentials are deliberately absent. The stable execution
/// environment descriptor is digest-bound so retry/reconnect cannot cross an
/// operating-system domain while retaining the same packet identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttemptPacketIdentity {
    task_ref: String,
    attempt: u8,
    project_ref: String,
    spec_ref: String,
    approval_ref: String,
    budget_digest: String,
    global_active_limit: u8,
    per_task_active_limit: u8,
    repair_retry_limit: u8,
    max_duration_seconds: u64,
    max_total_tokens: u64,
    max_model_calls: u32,
    remaining_total_tokens: u64,
    remaining_model_calls: u32,
    external_cost: ExternalCostBudget,
    verification_ref: String,
    worktree_ref: String,
    execution_environment_ref: String,
    base_commit: String,
    model_selection: ModelSelection,
    deadline_at: String,
    writer_fence: u64,
    prior_terminal_evidence_ref: Option<String>,
    continuation: Option<ContinuationSummary>,
    digest: String,
}

impl AttemptPacketIdentity {
    /// Constructs one exact initial or repair-attempt identity.
    ///
    /// # Errors
    ///
    /// Rejects malformed bindings, an attempt outside its immutable budget,
    /// or a repair packet without prior terminal evidence and a bounded
    /// continuation summary.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        task_ref: impl Into<String>,
        attempt: u8,
        project_ref: &str,
        spec_ref: &str,
        approval_ref: &str,
        budget: &WorkerBudget,
        verification_ref: &str,
        worktree_ref: &str,
        base_commit: impl Into<String>,
        model_selection: ModelSelection,
        writer_fence: u64,
        prior_terminal_evidence_ref: Option<&str>,
        continuation: Option<ContinuationSummary>,
    ) -> Result<Self, WorkerAttemptError> {
        if !budget.allows_attempt(attempt) || writer_fence == 0 {
            return Err(WorkerAttemptError::InvalidAttempt);
        }
        match (attempt, prior_terminal_evidence_ref, continuation.as_ref()) {
            (1, None, None) | (2.., Some(_), Some(_)) => {}
            _ => return Err(WorkerAttemptError::InvalidAttempt),
        }
        let task_ref = attempt_identifier(task_ref.into())?;
        let project_ref = attempt_digest_pointer(project_ref, "project")?;
        let spec_ref = attempt_digest_pointer(spec_ref, "spec")?;
        let approval_ref = attempt_digest_pointer(approval_ref, "approval")?;
        let verification_ref = attempt_digest_pointer(verification_ref, "verification")?;
        let worktree_ref = attempt_digest_pointer(worktree_ref, "worktree")?;
        let prior_terminal_evidence_ref = prior_terminal_evidence_ref
            .map(|value| attempt_digest_pointer(value, "evidence"))
            .transpose()?;
        let base_commit = base_commit.into();
        if !is_lower_hex(&base_commit, 40) {
            return Err(WorkerAttemptError::MalformedField);
        }
        let budget_digest = budget.digest().to_owned();
        let deadline_at = budget.deadline_at().to_owned();
        let mut packet = Self {
            task_ref,
            attempt,
            project_ref,
            spec_ref,
            approval_ref,
            budget_digest,
            global_active_limit: budget.global_active_limit(),
            per_task_active_limit: budget.per_task_active_limit(),
            repair_retry_limit: budget.repair_retry_limit(),
            max_duration_seconds: budget.max_duration_seconds(),
            max_total_tokens: budget.max_total_tokens(),
            max_model_calls: budget.max_model_calls(),
            remaining_total_tokens: budget.max_total_tokens(),
            remaining_model_calls: budget.max_model_calls(),
            external_cost: budget.external_cost(),
            verification_ref,
            worktree_ref,
            execution_environment_ref: NATIVE_WINDOWS_EXECUTION_ENVIRONMENT_REF.to_owned(),
            base_commit,
            model_selection,
            deadline_at,
            writer_fence,
            prior_terminal_evidence_ref,
            continuation,
            digest: String::new(),
        };
        packet.digest = canonical_digest_pointer(
            ATTEMPT_PACKET_SCHEMA,
            ATTEMPT_PACKET_VERSION,
            "attempt-packet",
            &packet.canonical_value(),
        )?;
        Ok(packet)
    }

    fn canonical_value(&self) -> CanonicalValue {
        CanonicalValue::Object(vec![
            (
                "task_ref".to_owned(),
                CanonicalValue::String(self.task_ref.clone()),
            ),
            text_canonical("attempt", &self.attempt),
            (
                "project_ref".to_owned(),
                CanonicalValue::String(self.project_ref.clone()),
            ),
            (
                "spec_ref".to_owned(),
                CanonicalValue::String(self.spec_ref.clone()),
            ),
            (
                "approval_ref".to_owned(),
                CanonicalValue::String(self.approval_ref.clone()),
            ),
            (
                "budget_digest".to_owned(),
                CanonicalValue::String(self.budget_digest.clone()),
            ),
            text_canonical("global_active_limit", &self.global_active_limit),
            text_canonical("per_task_active_limit", &self.per_task_active_limit),
            text_canonical("repair_retry_limit", &self.repair_retry_limit),
            text_canonical("max_duration_seconds", &self.max_duration_seconds),
            text_canonical("max_total_tokens", &self.max_total_tokens),
            text_canonical("max_model_calls", &self.max_model_calls),
            text_canonical("remaining_total_tokens", &self.remaining_total_tokens),
            text_canonical("remaining_model_calls", &self.remaining_model_calls),
            (
                "external_cost_status".to_owned(),
                CanonicalValue::String(self.external_cost.status().to_owned()),
            ),
            (
                "external_cost_micros".to_owned(),
                CanonicalValue::String(self.external_cost.amount()),
            ),
            (
                "non_model_external_spend_allowed".to_owned(),
                CanonicalValue::Bool(false),
            ),
            (
                "verification_ref".to_owned(),
                CanonicalValue::String(self.verification_ref.clone()),
            ),
            (
                "worktree_ref".to_owned(),
                CanonicalValue::String(self.worktree_ref.clone()),
            ),
            (
                "execution_environment_ref".to_owned(),
                CanonicalValue::String(self.execution_environment_ref.clone()),
            ),
            (
                "base_commit".to_owned(),
                CanonicalValue::String(self.base_commit.clone()),
            ),
            (
                "model_selection_digest".to_owned(),
                CanonicalValue::String(self.model_selection.digest().to_owned()),
            ),
            (
                "deadline_at".to_owned(),
                CanonicalValue::String(self.deadline_at.clone()),
            ),
            text_canonical("writer_fence", &self.writer_fence),
            (
                "prior_terminal_evidence_ref".to_owned(),
                CanonicalValue::String(
                    self.prior_terminal_evidence_ref.clone().unwrap_or_default(),
                ),
            ),
            (
                "continuation_digest".to_owned(),
                CanonicalValue::String(
                    self.continuation
                        .as_ref()
                        .map_or_else(String::new, |value| value.digest().to_owned()),
                ),
            ),
        ])
    }

    #[must_use]
    pub const fn schema(&self) -> &'static str {
        "lattice.foreman-attempt-packet/1.0"
    }

    #[must_use]
    pub fn task_ref(&self) -> &str {
        &self.task_ref
    }

    #[must_use]
    pub const fn attempt(&self) -> u8 {
        self.attempt
    }

    #[must_use]
    pub fn project_ref(&self) -> &str {
        &self.project_ref
    }

    #[must_use]
    pub fn spec_ref(&self) -> &str {
        &self.spec_ref
    }

    #[must_use]
    pub fn approval_ref(&self) -> &str {
        &self.approval_ref
    }

    #[must_use]
    pub fn budget_digest(&self) -> &str {
        &self.budget_digest
    }

    #[must_use]
    pub const fn global_active_limit(&self) -> u8 {
        self.global_active_limit
    }

    #[must_use]
    pub const fn per_task_active_limit(&self) -> u8 {
        self.per_task_active_limit
    }

    #[must_use]
    pub const fn repair_retry_limit(&self) -> u8 {
        self.repair_retry_limit
    }

    #[must_use]
    pub const fn max_duration_seconds(&self) -> u64 {
        self.max_duration_seconds
    }

    #[must_use]
    pub const fn max_total_tokens(&self) -> u64 {
        self.max_total_tokens
    }

    #[must_use]
    pub const fn max_model_calls(&self) -> u32 {
        self.max_model_calls
    }

    #[must_use]
    pub const fn remaining_total_tokens(&self) -> u64 {
        self.remaining_total_tokens
    }

    #[must_use]
    pub const fn remaining_model_calls(&self) -> u32 {
        self.remaining_model_calls
    }

    /// Narrows this exact attempt to the replay-derived cumulative budget
    /// remaining before its first model call and rebinds the packet digest.
    ///
    /// # Errors
    ///
    /// Rejects zero or expanding limits.
    pub fn with_remaining_budget(
        mut self,
        remaining_total_tokens: u64,
        remaining_model_calls: u32,
    ) -> Result<Self, WorkerAttemptError> {
        if remaining_total_tokens == 0
            || remaining_total_tokens > self.max_total_tokens
            || remaining_model_calls == 0
            || remaining_model_calls > self.max_model_calls
        {
            return Err(WorkerAttemptError::InvalidBudget);
        }
        self.remaining_total_tokens = remaining_total_tokens;
        self.remaining_model_calls = remaining_model_calls;
        self.digest = canonical_digest_pointer(
            ATTEMPT_PACKET_SCHEMA,
            ATTEMPT_PACKET_VERSION,
            "attempt-packet",
            &self.canonical_value(),
        )?;
        Ok(self)
    }

    #[must_use]
    pub const fn external_cost(&self) -> ExternalCostBudget {
        self.external_cost
    }

    #[must_use]
    pub const fn non_model_external_spend_allowed(&self) -> bool {
        false
    }

    #[must_use]
    pub fn verification_ref(&self) -> &str {
        &self.verification_ref
    }

    #[must_use]
    pub fn worktree_ref(&self) -> &str {
        &self.worktree_ref
    }

    #[must_use]
    pub fn execution_environment_ref(&self) -> &str {
        &self.execution_environment_ref
    }

    /// Reports whether this packet uses the legacy process-owned native
    /// Windows domain rather than a durable external execution descriptor.
    #[must_use]
    pub fn is_native_windows_execution_environment(&self) -> bool {
        self.execution_environment_ref == NATIVE_WINDOWS_EXECUTION_ENVIRONMENT_REF
    }

    /// Rebinds this immutable packet to one verified execution-environment
    /// descriptor before claim/provider effects.
    ///
    /// # Errors
    ///
    /// Rejects anything other than an exact secret-free environment digest.
    pub fn with_execution_environment_ref(
        mut self,
        execution_environment_ref: &str,
    ) -> Result<Self, WorkerAttemptError> {
        self.execution_environment_ref =
            attempt_digest_pointer(execution_environment_ref, "execution-environment")?;
        self.digest = canonical_digest_pointer(
            ATTEMPT_PACKET_SCHEMA,
            ATTEMPT_PACKET_VERSION,
            "attempt-packet",
            &self.canonical_value(),
        )?;
        Ok(self)
    }

    #[must_use]
    pub fn base_commit(&self) -> &str {
        &self.base_commit
    }

    #[must_use]
    pub const fn model_selection(&self) -> &ModelSelection {
        &self.model_selection
    }

    #[must_use]
    pub const fn writer_fence(&self) -> u64 {
        self.writer_fence
    }

    #[must_use]
    pub fn deadline_at(&self) -> &str {
        &self.deadline_at
    }

    #[must_use]
    pub fn prior_terminal_evidence_ref(&self) -> Option<&str> {
        self.prior_terminal_evidence_ref.as_deref()
    }

    #[must_use]
    pub const fn continuation(&self) -> Option<&ContinuationSummary> {
        self.continuation.as_ref()
    }

    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// Checks the immutable lineage plus exact next attempt and strictly newer
    /// Writer fence. A repair may select a different allowed model and base
    /// commit, but cannot change task/spec/approval/budget/profile/worktree.
    ///
    /// # Errors
    ///
    /// Rejects every lineage substitution or non-incrementing attempt/fence.
    pub fn validate_repair_successor(
        &self,
        previous: &WorkerAttemptState,
    ) -> Result<(), WorkerAttemptError> {
        let previous_packet = previous.packet();
        if previous.phase() != WorkerAttemptPhase::Terminal
            || previous.terminal_evidence_ref() != self.prior_terminal_evidence_ref()
            || previous_packet.attempt.checked_add(1) != Some(self.attempt)
            || self.writer_fence <= previous_packet.writer_fence
            || self.task_ref != previous_packet.task_ref
            || self.project_ref != previous_packet.project_ref
            || self.spec_ref != previous_packet.spec_ref
            || self.approval_ref != previous_packet.approval_ref
            || self.budget_digest != previous_packet.budget_digest
            || self.verification_ref != previous_packet.verification_ref
            || self.worktree_ref != previous_packet.worktree_ref
            || self.execution_environment_ref != previous_packet.execution_environment_ref
            || self.prior_terminal_evidence_ref.is_none()
            || self.continuation.is_none()
            || (self.model_selection.reason() == ModelReason::TerraInsufficient
                && (previous_packet.model_selection().model() != WorkerModel::Terra
                    || self.model_selection.evidence_ref() != self.prior_terminal_evidence_ref()))
        {
            return Err(WorkerAttemptError::InvalidAttempt);
        }
        Ok(())
    }

    /// Checks a repair successor whose predecessor ended before an exact
    /// provider terminal and was instead closed by a durable exact no-effect
    /// reconciliation proof. The proof reference is persisted separately from
    /// the immutable original blocker and becomes the repair evidence anchor.
    ///
    /// # Errors
    ///
    /// Rejects a malformed or substituted closure proof, lineage drift,
    /// non-consecutive attempt, stale Writer fence, or missing continuation.
    pub fn validate_closed_prestart_repair_successor(
        &self,
        previous: &Self,
        closure_proof_evidence_ref: &str,
    ) -> Result<(), WorkerAttemptError> {
        let proof = attempt_digest_pointer(closure_proof_evidence_ref, "evidence")?;
        if self.prior_terminal_evidence_ref.as_deref() != Some(proof.as_str())
            || previous.attempt.checked_add(1) != Some(self.attempt)
            || self.writer_fence <= previous.writer_fence
            || self.task_ref != previous.task_ref
            || self.project_ref != previous.project_ref
            || self.spec_ref != previous.spec_ref
            || self.approval_ref != previous.approval_ref
            || self.budget_digest != previous.budget_digest
            || self.verification_ref != previous.verification_ref
            || self.worktree_ref != previous.worktree_ref
            || self.execution_environment_ref != previous.execution_environment_ref
            || self.continuation.is_none()
            || (self.model_selection.reason() == ModelReason::TerraInsufficient
                && (previous.model_selection().model() != WorkerModel::Terra
                    || self.model_selection.evidence_ref() != self.prior_terminal_evidence_ref()))
        {
            return Err(WorkerAttemptError::InvalidAttempt);
        }
        Ok(())
    }
}

/// Lifecycle phase of one Task-Ledger-owned worker-attempt child record. This
/// is not a second Task Domain state machine.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerAttemptPhase {
    Claimed,
    Dispatching,
    Accepted,
    Starting,
    Executing,
    Reconciling,
    Interrupting,
    Terminal,
}

impl WorkerAttemptPhase {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Claimed => "CLAIMED",
            Self::Dispatching => "DISPATCHING",
            Self::Accepted => "ACCEPTED",
            Self::Starting => "STARTING",
            Self::Executing => "EXECUTING",
            Self::Reconciling => "RECONCILING",
            Self::Interrupting => "INTERRUPTING",
            Self::Terminal => "TERMINAL",
        }
    }
}

/// Exact App Server terminal observation. Completion remains only a candidate
/// for independent verification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerTerminal {
    Completed,
    Interrupted,
    Failed,
}

impl WorkerTerminal {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "COMPLETED",
            Self::Interrupted => "INTERRUPTED",
            Self::Failed => "FAILED",
        }
    }
}

/// Status carried by a `turn/started` observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TurnStartedStatus {
    InProgress,
    NotInProgress,
}

/// Closed exact-start observations. RPC acceptance never implies execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StartObservation {
    ThreadStartAccepted {
        thread_id: String,
    },
    ThreadStarted {
        thread_id: String,
    },
    TurnStartAccepted {
        thread_id: String,
        turn_id: String,
    },
    TurnStarted {
        thread_id: String,
        turn_id: String,
        status: TurnStartedStatus,
        observed_at: String,
    },
}

/// Result of applying one exact-start observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartGateDecision {
    Applied(WorkerAttemptPhase),
    Ignored,
}

/// Pure retained state for one exact worker attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkerAttemptState {
    packet: AttemptPacketIdentity,
    phase: WorkerAttemptPhase,
    thread_id: Option<String>,
    turn_id: Option<String>,
    attempt_started_at: Option<String>,
    attempt_deadline_at: Option<String>,
    terminal: Option<WorkerTerminal>,
    terminal_evidence_ref: Option<String>,
    digest: String,
}

impl WorkerAttemptState {
    /// Creates one claimed but not yet dispatched attempt.
    ///
    /// # Errors
    ///
    /// Fails closed if its canonical state digest cannot be constructed.
    pub fn new(packet: AttemptPacketIdentity) -> Result<Self, WorkerAttemptError> {
        let mut state = Self {
            packet,
            phase: WorkerAttemptPhase::Claimed,
            thread_id: None,
            turn_id: None,
            attempt_started_at: None,
            attempt_deadline_at: None,
            terminal: None,
            terminal_evidence_ref: None,
            digest: String::new(),
        };
        state.refresh_digest()?;
        Ok(state)
    }

    #[must_use]
    pub const fn packet(&self) -> &AttemptPacketIdentity {
        &self.packet
    }

    #[must_use]
    pub const fn phase(&self) -> WorkerAttemptPhase {
        self.phase
    }

    #[must_use]
    pub fn thread_id(&self) -> Option<&str> {
        self.thread_id.as_deref()
    }

    #[must_use]
    pub fn turn_id(&self) -> Option<&str> {
        self.turn_id.as_deref()
    }

    /// Returns the exact provider-observed `turn/started` time, if execution
    /// was durably established.
    #[must_use]
    pub fn attempt_started_at(&self) -> Option<&str> {
        self.attempt_started_at.as_deref()
    }

    /// Returns the immutable execution deadline bounded by both exact start
    /// plus the packet's maximum duration and the task-level budget deadline.
    #[must_use]
    pub fn attempt_deadline_at(&self) -> Option<&str> {
        self.attempt_deadline_at.as_deref()
    }

    #[must_use]
    pub const fn terminal(&self) -> Option<WorkerTerminal> {
        self.terminal
    }

    #[must_use]
    pub fn terminal_evidence_ref(&self) -> Option<&str> {
        self.terminal_evidence_ref.as_deref()
    }

    /// True only after an exact matching in-progress `turn/started` and before
    /// an exact terminal. Reconciliation/interrupt retain the same active turn.
    #[must_use]
    pub const fn is_real_running(&self) -> bool {
        matches!(
            self.phase,
            WorkerAttemptPhase::Executing
                | WorkerAttemptPhase::Reconciling
                | WorkerAttemptPhase::Interrupting
        )
    }

    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// Records the durable effect intent before a `thread/start` RPC may be
    /// sent. A restart in this phase blocks as uncertain rather than opening a
    /// duplicate thread.
    ///
    /// # Errors
    ///
    /// Rejects dispatch intent after an RPC result or lifecycle observation.
    pub fn begin_dispatch(&mut self) -> Result<(), WorkerAttemptError> {
        if !matches!(
            self.phase,
            WorkerAttemptPhase::Claimed | WorkerAttemptPhase::Dispatching
        ) {
            return Err(WorkerAttemptError::InvalidPhase);
        }
        self.phase = WorkerAttemptPhase::Dispatching;
        self.refresh_digest()
    }

    /// Applies one exact-start observation without I/O.
    ///
    /// # Errors
    ///
    /// Rejects secret-bearing/malformed IDs or an impossible local phase.
    pub fn apply_start(
        &mut self,
        observation: StartObservation,
    ) -> Result<StartGateDecision, WorkerAttemptError> {
        let decision = match observation {
            StartObservation::ThreadStartAccepted { thread_id } => {
                let thread_id = attempt_identifier(thread_id)?;
                if self.phase == WorkerAttemptPhase::Dispatching {
                    self.thread_id = Some(thread_id);
                    self.phase = WorkerAttemptPhase::Accepted;
                    StartGateDecision::Applied(self.phase)
                } else if self.thread_id.as_deref() == Some(thread_id.as_str()) {
                    StartGateDecision::Ignored
                } else {
                    return Err(WorkerAttemptError::InvalidPhase);
                }
            }
            StartObservation::ThreadStarted { thread_id } => {
                let thread_id = attempt_identifier(thread_id)?;
                if self.thread_id.as_deref() != Some(thread_id.as_str()) {
                    StartGateDecision::Ignored
                } else if matches!(
                    self.phase,
                    WorkerAttemptPhase::Accepted | WorkerAttemptPhase::Starting
                ) {
                    self.phase = WorkerAttemptPhase::Starting;
                    StartGateDecision::Applied(self.phase)
                } else {
                    StartGateDecision::Ignored
                }
            }
            StartObservation::TurnStartAccepted { thread_id, turn_id } => {
                let thread_id = attempt_identifier(thread_id)?;
                let turn_id = attempt_identifier(turn_id)?;
                if self.thread_id.as_deref() != Some(thread_id.as_str()) {
                    StartGateDecision::Ignored
                } else if matches!(
                    self.phase,
                    WorkerAttemptPhase::Accepted | WorkerAttemptPhase::Starting
                ) && self
                    .turn_id
                    .as_deref()
                    .is_none_or(|retained| retained == turn_id)
                {
                    self.turn_id = Some(turn_id);
                    self.phase = WorkerAttemptPhase::Starting;
                    StartGateDecision::Applied(self.phase)
                } else {
                    StartGateDecision::Ignored
                }
            }
            StartObservation::TurnStarted {
                thread_id,
                turn_id,
                status,
                observed_at,
            } => {
                let thread_id = attempt_identifier(thread_id)?;
                let turn_id = attempt_identifier(turn_id)?;
                let observed_at = attempt_event_timestamp(observed_at)?;
                let attempt_deadline_at = derive_attempt_deadline(
                    &observed_at,
                    self.packet.max_duration_seconds(),
                    self.packet.deadline_at(),
                )?;
                if self.phase == WorkerAttemptPhase::Starting
                    && status == TurnStartedStatus::InProgress
                    && self.thread_id.as_deref() == Some(thread_id.as_str())
                    && self.turn_id.as_deref() == Some(turn_id.as_str())
                {
                    self.attempt_started_at = Some(observed_at);
                    self.attempt_deadline_at = Some(attempt_deadline_at);
                    self.phase = WorkerAttemptPhase::Executing;
                    StartGateDecision::Applied(self.phase)
                } else {
                    StartGateDecision::Ignored
                }
            }
        };
        if matches!(decision, StartGateDecision::Applied(_)) {
            self.refresh_digest()?;
        }
        Ok(decision)
    }

    /// Marks reconciliation of the already retained exact turn.
    ///
    /// # Errors
    ///
    /// Rejects a phase without a real exact active turn.
    pub fn begin_reconciliation(&mut self) -> Result<(), WorkerAttemptError> {
        if !self.is_real_running() || self.thread_id.is_none() || self.turn_id.is_none() {
            return Err(WorkerAttemptError::InvalidPhase);
        }
        self.phase = WorkerAttemptPhase::Reconciling;
        self.refresh_digest()
    }

    /// Marks an exact-turn interrupt only after reconciliation.
    ///
    /// # Errors
    ///
    /// Rejects interrupt-before-reconcile.
    pub fn begin_interrupt(&mut self) -> Result<(), WorkerAttemptError> {
        if self.phase != WorkerAttemptPhase::Reconciling {
            return Err(WorkerAttemptError::InvalidPhase);
        }
        self.phase = WorkerAttemptPhase::Interrupting;
        self.refresh_digest()
    }

    /// Records one exact terminal; it never marks the parent task complete.
    ///
    /// # Errors
    ///
    /// Rejects mismatched IDs, terminal-before-exact-start, or malformed
    /// evidence.
    pub fn record_terminal(
        &mut self,
        thread_id: &str,
        turn_id: &str,
        terminal: WorkerTerminal,
        evidence_ref: &str,
    ) -> Result<(), WorkerAttemptError> {
        let thread_id = attempt_identifier(thread_id.to_owned())?;
        let turn_id = attempt_identifier(turn_id.to_owned())?;
        if !self.is_real_running()
            || self.thread_id.as_deref() != Some(thread_id.as_str())
            || self.turn_id.as_deref() != Some(turn_id.as_str())
        {
            return Err(WorkerAttemptError::InvalidPhase);
        }
        self.terminal_evidence_ref = Some(attempt_digest_pointer(evidence_ref, "evidence")?);
        self.terminal = Some(terminal);
        self.phase = WorkerAttemptPhase::Terminal;
        self.refresh_digest()
    }

    /// Records the sole terminal allowed before exact `turn/started`: a
    /// recovered failed start for the already accepted exact thread/turn.
    /// The attempt never becomes real-running and retains no start/deadline.
    ///
    /// # Errors
    ///
    /// Rejects a non-starting phase, mismatched IDs, any prior exact start,
    /// or malformed evidence.
    pub fn record_prestart_terminal_failed(
        &mut self,
        thread_id: &str,
        turn_id: &str,
        evidence_ref: &str,
    ) -> Result<(), WorkerAttemptError> {
        let thread_id = attempt_identifier(thread_id.to_owned())?;
        let turn_id = attempt_identifier(turn_id.to_owned())?;
        if self.phase != WorkerAttemptPhase::Starting
            || self.attempt_started_at.is_some()
            || self.attempt_deadline_at.is_some()
            || self.thread_id.as_deref() != Some(thread_id.as_str())
            || self.turn_id.as_deref() != Some(turn_id.as_str())
        {
            return Err(WorkerAttemptError::InvalidPhase);
        }
        self.terminal_evidence_ref = Some(attempt_digest_pointer(evidence_ref, "evidence")?);
        self.terminal = Some(WorkerTerminal::Failed);
        self.phase = WorkerAttemptPhase::Terminal;
        self.refresh_digest()
    }

    fn refresh_digest(&mut self) -> Result<(), WorkerAttemptError> {
        let value = CanonicalValue::Object(vec![
            (
                "packet_digest".to_owned(),
                CanonicalValue::String(self.packet.digest().to_owned()),
            ),
            (
                "phase".to_owned(),
                CanonicalValue::String(self.phase.as_str().to_owned()),
            ),
            (
                "thread_id".to_owned(),
                CanonicalValue::String(self.thread_id.clone().unwrap_or_default()),
            ),
            (
                "turn_id".to_owned(),
                CanonicalValue::String(self.turn_id.clone().unwrap_or_default()),
            ),
            (
                "attempt_started_at".to_owned(),
                CanonicalValue::String(self.attempt_started_at.clone().unwrap_or_default()),
            ),
            (
                "attempt_deadline_at".to_owned(),
                CanonicalValue::String(self.attempt_deadline_at.clone().unwrap_or_default()),
            ),
            (
                "terminal".to_owned(),
                CanonicalValue::String(
                    self.terminal
                        .map_or_else(String::new, |value| value.as_str().to_owned()),
                ),
            ),
            (
                "terminal_evidence_ref".to_owned(),
                CanonicalValue::String(self.terminal_evidence_ref.clone().unwrap_or_default()),
            ),
        ]);
        self.digest = canonical_digest_pointer(
            ATTEMPT_STATE_SCHEMA,
            ATTEMPT_STATE_VERSION,
            "attempt-state",
            &value,
        )?;
        Ok(())
    }
}

/// Closed inputs that may advance the meaningful-progress clock.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MeaningfulProgressKind {
    ExactLifecycleNotification,
    ProcessObservation,
    TerminalObservation,
    VerifiedWorkChange,
    VerifiedEvidenceChange,
}

impl MeaningfulProgressKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::ExactLifecycleNotification => "EXACT_LIFECYCLE_NOTIFICATION",
            Self::ProcessObservation => "PROCESS_OBSERVATION",
            Self::TerminalObservation => "TERMINAL_OBSERVATION",
            Self::VerifiedWorkChange => "VERIFIED_WORK_CHANGE",
            Self::VerifiedEvidenceChange => "VERIFIED_EVIDENCE_CHANGE",
        }
    }
}

/// One bounded, digest-only meaningful-progress heartbeat.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MeaningfulProgress {
    packet_digest: String,
    kind: MeaningfulProgressKind,
    occurred_at: String,
    evidence_ref: String,
    digest: String,
}

impl MeaningfulProgress {
    /// # Errors
    ///
    /// Rejects malformed time/evidence; free-form heartbeat text is absent.
    pub fn new(
        state: &WorkerAttemptState,
        kind: MeaningfulProgressKind,
        occurred_at: impl Into<String>,
        evidence_ref: &str,
    ) -> Result<Self, WorkerAttemptError> {
        let occurred_at = attempt_timestamp(occurred_at.into())?;
        let evidence_ref = attempt_digest_pointer(evidence_ref, "evidence")?;
        let packet_digest = state.packet().digest().to_owned();
        let value = CanonicalValue::Object(vec![
            (
                "packet_digest".to_owned(),
                CanonicalValue::String(packet_digest.clone()),
            ),
            (
                "kind".to_owned(),
                CanonicalValue::String(kind.as_str().to_owned()),
            ),
            (
                "occurred_at".to_owned(),
                CanonicalValue::String(occurred_at.clone()),
            ),
            (
                "evidence_ref".to_owned(),
                CanonicalValue::String(evidence_ref.clone()),
            ),
        ]);
        let digest = canonical_digest_pointer(
            MEANINGFUL_PROGRESS_SCHEMA,
            MEANINGFUL_PROGRESS_VERSION,
            "progress",
            &value,
        )?;
        Ok(Self {
            packet_digest,
            kind,
            occurred_at,
            evidence_ref,
            digest,
        })
    }

    #[must_use]
    pub const fn kind(&self) -> MeaningfulProgressKind {
        self.kind
    }

    #[must_use]
    pub fn occurred_at(&self) -> &str {
        &self.occurred_at
    }

    #[must_use]
    pub fn evidence_ref(&self) -> &str {
        &self.evidence_ref
    }

    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }
}

/// Independently observed worker-process state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessObservation {
    Unknown,
    Alive,
    Exited,
}

/// Exact active-turn observation supplied by the App Server connector. An
/// unbound boolean cannot suppress or create a heartbeat stall.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TurnActivityObservation {
    Unknown,
    ExactInProgress { thread_id: String, turn_id: String },
}

/// Progress of the mandatory read/resume/reconcile-first action.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReconciliationState {
    NotAttempted,
    Pending,
    Recovered,
    Exhausted,
}

/// Pure inputs for one watchdog classification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttemptWatchdogObservation {
    now: String,
    heartbeat_timeout_seconds: u64,
    process: ProcessObservation,
    turn_activity: TurnActivityObservation,
    reconciliation: ReconciliationState,
}

impl AttemptWatchdogObservation {
    /// # Errors
    ///
    /// Rejects zero/unrepresentable timeout and malformed current time.
    pub fn new(
        now: impl Into<String>,
        heartbeat_timeout_seconds: u64,
        process: ProcessObservation,
        turn_activity: TurnActivityObservation,
        reconciliation: ReconciliationState,
    ) -> Result<Self, WorkerAttemptError> {
        if heartbeat_timeout_seconds == 0 || heartbeat_timeout_seconds > i64::MAX as u64 {
            return Err(WorkerAttemptError::InvalidBudget);
        }
        let turn_activity = match turn_activity {
            TurnActivityObservation::Unknown => TurnActivityObservation::Unknown,
            TurnActivityObservation::ExactInProgress { thread_id, turn_id } => {
                TurnActivityObservation::ExactInProgress {
                    thread_id: attempt_identifier(thread_id)?,
                    turn_id: attempt_identifier(turn_id)?,
                }
            }
        };
        Ok(Self {
            now: attempt_timestamp(now.into())?,
            heartbeat_timeout_seconds,
            process,
            turn_activity,
            reconciliation,
        })
    }
}

/// Complete, replayable stall reason allowlist.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StallReason {
    HeartbeatTimeoutActiveTurn,
    ProcessExitWithoutTerminal,
    ReconciliationExhausted,
    DeadlineExceeded,
}

/// Watchdog outcome. Repairable raw observations always require
/// reconciliation before interruption or retry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StallClassification {
    Healthy,
    ReconcileFirst(StallReason),
    Stalled(StallReason),
}

/// Classifies one worker without filesystem, process, database, or network I/O.
/// Elapsed time is a heartbeat stall only for the retained exact active turn.
///
/// # Errors
///
/// Rejects substituted packet/budget identity and malformed time ordering.
pub fn classify_attempt_stall(
    state: &WorkerAttemptState,
    budget: &WorkerBudget,
    last_progress: &MeaningfulProgress,
    observation: &AttemptWatchdogObservation,
) -> Result<StallClassification, WorkerAttemptError> {
    if state.packet().budget_digest() != budget.digest()
        || last_progress.packet_digest != state.packet().digest()
        || observation.heartbeat_timeout_seconds > budget.max_duration_seconds()
    {
        return Err(WorkerAttemptError::InvalidBudget);
    }
    let now = parse_attempt_time(&observation.now)?;
    let progress_at = parse_attempt_time(last_progress.occurred_at())?;
    if now < progress_at {
        return Err(WorkerAttemptError::MalformedField);
    }
    let exact_turn_in_progress = match &observation.turn_activity {
        TurnActivityObservation::Unknown => false,
        TurnActivityObservation::ExactInProgress { thread_id, turn_id } => {
            if state.thread_id() != Some(thread_id.as_str())
                || state.turn_id() != Some(turn_id.as_str())
            {
                return Err(WorkerAttemptError::InvalidPhase);
            }
            true
        }
    };
    if state.phase() == WorkerAttemptPhase::Terminal {
        return Ok(StallClassification::Healthy);
    }
    if observation.reconciliation == ReconciliationState::Exhausted {
        return Ok(StallClassification::Stalled(
            StallReason::ReconciliationExhausted,
        ));
    }
    if observation.process == ProcessObservation::Exited {
        return Ok(StallClassification::ReconcileFirst(
            StallReason::ProcessExitWithoutTerminal,
        ));
    }
    if state.is_real_running() {
        let deadline = state
            .attempt_deadline_at()
            .ok_or(WorkerAttemptError::InvalidPhase)
            .and_then(parse_attempt_time)?;
        if now >= deadline {
            return Ok(StallClassification::ReconcileFirst(
                StallReason::DeadlineExceeded,
            ));
        }
    }
    let elapsed_seconds = (now - progress_at).whole_seconds();
    let heartbeat_timeout = i64::try_from(observation.heartbeat_timeout_seconds)
        .map_err(|_| WorkerAttemptError::InvalidBudget)?;
    if state.phase() == WorkerAttemptPhase::Executing
        && exact_turn_in_progress
        && observation.reconciliation == ReconciliationState::NotAttempted
        && elapsed_seconds >= heartbeat_timeout
    {
        return Ok(StallClassification::ReconcileFirst(
            StallReason::HeartbeatTimeoutActiveTurn,
        ));
    }
    Ok(StallClassification::Healthy)
}

/// Fresh-process action for a retained worker attempt. Only an attempt that
/// was durably claimed but never dispatched may start; all retained IDs are
/// reconciled exactly before any later action.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RestartDecision {
    DispatchUnsentAttempt,
    BlockUncertainDispatch,
    ReadExactThread { thread_id: String },
    ReadResumeExactTurn { thread_id: String, turn_id: String },
    PreserveTerminal,
}

/// # Errors
///
/// Rejects a retained phase missing the exact IDs required by that phase.
pub fn restart_reconciliation_decision(
    state: &WorkerAttemptState,
) -> Result<RestartDecision, WorkerAttemptError> {
    Ok(match state.phase() {
        WorkerAttemptPhase::Claimed => RestartDecision::DispatchUnsentAttempt,
        WorkerAttemptPhase::Dispatching => RestartDecision::BlockUncertainDispatch,
        WorkerAttemptPhase::Accepted | WorkerAttemptPhase::Starting => {
            if let (Some(thread_id), Some(turn_id)) = (state.thread_id(), state.turn_id()) {
                RestartDecision::ReadResumeExactTurn {
                    thread_id: thread_id.to_owned(),
                    turn_id: turn_id.to_owned(),
                }
            } else {
                let thread_id = state.thread_id().ok_or(WorkerAttemptError::InvalidPhase)?;
                RestartDecision::ReadExactThread {
                    thread_id: thread_id.to_owned(),
                }
            }
        }
        WorkerAttemptPhase::Executing
        | WorkerAttemptPhase::Reconciling
        | WorkerAttemptPhase::Interrupting => {
            let thread_id = state.thread_id().ok_or(WorkerAttemptError::InvalidPhase)?;
            let turn_id = state.turn_id().ok_or(WorkerAttemptError::InvalidPhase)?;
            RestartDecision::ReadResumeExactTurn {
                thread_id: thread_id.to_owned(),
                turn_id: turn_id.to_owned(),
            }
        }
        WorkerAttemptPhase::Terminal => RestartDecision::PreserveTerminal,
    })
}

/// Closed repair decision. A non-terminal attempt can only reconcile its exact
/// turn; it can never consume retry budget yet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetryDecision {
    ReconcileExactTurn,
    Retry { next_attempt: u8 },
    BlockedNonRepairable,
    BlockedRetryBudgetExhausted,
}

/// Applies the immutable repair budget after an exact terminal.
///
/// # Errors
///
/// Rejects a substituted budget digest.
pub fn decide_repair_retry(
    state: &WorkerAttemptState,
    budget: &WorkerBudget,
    repairable: bool,
) -> Result<RetryDecision, WorkerAttemptError> {
    if state.packet().budget_digest() != budget.digest() {
        return Err(WorkerAttemptError::InvalidBudget);
    }
    if state.phase() != WorkerAttemptPhase::Terminal {
        return Ok(RetryDecision::ReconcileExactTurn);
    }
    if !repairable {
        return Ok(RetryDecision::BlockedNonRepairable);
    }
    let attempt = state.packet().attempt();
    if attempt >= budget.max_attempts() {
        return Ok(RetryDecision::BlockedRetryBudgetExhausted);
    }
    Ok(RetryDecision::Retry {
        next_attempt: attempt + 1,
    })
}

/// The explicit confidence of a non-authoritative epistemic record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Confidence {
    Unknown,
    Low,
    Medium,
    High,
}

/// A closed reason that forces an epistemic record to be checked again.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefreshTrigger {
    Expiry,
    NewEvidence,
    Counterevidence,
    DependencyChange,
}

/// Bounded references for provisional facts and hypotheses. The text of a
/// hypothesis is deliberately absent: its pointer cannot become task truth.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EpistemicReferences {
    observed_facts: Vec<String>,
    hypotheses: Vec<String>,
    confidence: Confidence,
    unknowns: Vec<String>,
    evidence: Vec<String>,
    counterevidence: Vec<String>,
    checked_at: String,
    expires_at: String,
    refresh_trigger: RefreshTrigger,
    decision: String,
    probe: String,
    falsifier: String,
}

impl EpistemicReferences {
    /// # Errors
    ///
    /// Rejects non-pointer content, malformed timestamps, and an expiry that
    /// does not follow its check time.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        observed_facts: Vec<String>,
        hypotheses: Vec<String>,
        confidence: Confidence,
        unknowns: Vec<String>,
        evidence: Vec<String>,
        counterevidence: Vec<String>,
        checked_at: impl Into<String>,
        expires_at: impl Into<String>,
        refresh_trigger: RefreshTrigger,
        decision: impl Into<String>,
        probe: impl Into<String>,
        falsifier: impl Into<String>,
    ) -> Result<Self, SnapshotError> {
        let checked_at = timestamp(checked_at.into())?;
        let expires_at = timestamp(expires_at.into())?;
        if expires_at <= checked_at {
            return Err(SnapshotError::MalformedReference);
        }
        Ok(Self {
            observed_facts: pointer_list(observed_facts, "fact")?,
            hypotheses: pointer_list(hypotheses, "hypothesis")?,
            confidence,
            unknowns: pointer_list(unknowns, "unknown")?,
            evidence: pointer_list(evidence, "evidence")?,
            counterevidence: pointer_list(counterevidence, "counterevidence")?,
            checked_at,
            expires_at,
            refresh_trigger,
            decision: digest_pointer(decision.into(), "decision")?,
            probe: digest_pointer(probe.into(), "probe")?,
            falsifier: digest_pointer(falsifier.into(), "falsifier")?,
        })
    }

    /// Versioned schema for non-authoritative epistemic pointers only.
    #[must_use]
    pub const fn schema(&self) -> &'static str {
        EPISTEMIC_SCHEMA
    }

    #[must_use]
    pub const fn confidence(&self) -> Confidence {
        self.confidence
    }

    /// Observed-fact digest pointers; callers must resolve and assess them
    /// independently rather than treating them as lifecycle state.
    #[must_use]
    pub fn observed_facts(&self) -> &[String] {
        &self.observed_facts
    }

    /// Hypothesis digest pointers; they remain provisional by contract.
    #[must_use]
    pub fn hypotheses(&self) -> &[String] {
        &self.hypotheses
    }

    /// Unknowns that must remain explicit in any later decision.
    #[must_use]
    pub fn unknowns(&self) -> &[String] {
        &self.unknowns
    }

    /// Supporting evidence digest pointers.
    #[must_use]
    pub fn evidence(&self) -> &[String] {
        &self.evidence
    }

    /// Counterevidence digest pointers.
    #[must_use]
    pub fn counterevidence(&self) -> &[String] {
        &self.counterevidence
    }

    /// Time at which the epistemic record was checked.
    #[must_use]
    pub fn checked_at(&self) -> &str {
        &self.checked_at
    }

    /// Time at which the record must be refreshed before reuse.
    #[must_use]
    pub fn expires_at(&self) -> &str {
        &self.expires_at
    }

    /// Closed condition that requires reassessment.
    #[must_use]
    pub const fn refresh_trigger(&self) -> RefreshTrigger {
        self.refresh_trigger
    }

    /// Digest pointer to the decision under examination.
    #[must_use]
    pub fn decision(&self) -> &str {
        &self.decision
    }

    /// Digest pointer to the probe that can reduce the uncertainty.
    #[must_use]
    pub fn probe(&self) -> &str {
        &self.probe
    }

    /// Digest pointer to the record that can falsify the hypothesis.
    #[must_use]
    pub fn falsifier(&self) -> &str {
        &self.falsifier
    }
}

impl Confidence {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "UNKNOWN",
            Self::Low => "LOW",
            Self::Medium => "MEDIUM",
            Self::High => "HIGH",
        }
    }

    /// Parses the closed persistence spelling.
    ///
    /// # Errors
    ///
    /// Rejects every unknown or case-substituted value.
    pub fn from_persisted(value: &str) -> Result<Self, SnapshotError> {
        match value {
            "UNKNOWN" => Ok(Self::Unknown),
            "LOW" => Ok(Self::Low),
            "MEDIUM" => Ok(Self::Medium),
            "HIGH" => Ok(Self::High),
            _ => Err(SnapshotError::MalformedReference),
        }
    }
}

impl RefreshTrigger {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Expiry => "EXPIRY",
            Self::NewEvidence => "NEW_EVIDENCE",
            Self::Counterevidence => "COUNTEREVIDENCE",
            Self::DependencyChange => "DEPENDENCY_CHANGE",
        }
    }

    /// Parses the closed persistence spelling.
    ///
    /// # Errors
    ///
    /// Rejects every unknown or case-substituted value.
    pub fn from_persisted(value: &str) -> Result<Self, SnapshotError> {
        match value {
            "EXPIRY" => Ok(Self::Expiry),
            "NEW_EVIDENCE" => Ok(Self::NewEvidence),
            "COUNTEREVIDENCE" => Ok(Self::Counterevidence),
            "DEPENDENCY_CHANGE" => Ok(Self::DependencyChange),
            _ => Err(SnapshotError::MalformedReference),
        }
    }
}

/// Stable rejection and replay failures.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SnapshotError {
    MalformedReference,
    ForbiddenContent,
    MissingBlocker,
    UnexpectedBlocker,
    GenerationRollback,
    DuplicateWorkerIdentity,
}

/// One closed dependency identity stored inside the existing bounded blocker
/// scalar. Branch and next action are redundant inputs at the wire boundary so
/// substitution is rejected, but only their canonical derivation is retained.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DependencyBinding {
    parent_task_id: String,
    dependency_task_id: String,
    dependency_worktree_id: String,
    dependency_branch: String,
    base_sha: String,
    blocker_ref: String,
    evidence_ref: String,
}

impl DependencyBinding {
    /// Constructs one exact dependency binding.
    ///
    /// # Errors
    ///
    /// Rejects malformed identifiers, a substituted branch/base/next action,
    /// or a value that cannot fit the existing durable 256-byte scalar.
    pub fn new(
        parent_task_id: impl Into<String>,
        dependency_task_id: impl Into<String>,
        dependency_worktree_id: impl Into<String>,
        dependency_branch: impl Into<String>,
        base_sha: impl Into<String>,
        next_action: &str,
    ) -> Result<Self, SnapshotError> {
        let parent_task_id = dependency_task_identifier(parent_task_id.into())?;
        let dependency_task_id = dependency_task_identifier(dependency_task_id.into())?;
        if parent_task_id == dependency_task_id {
            return Err(SnapshotError::MalformedReference);
        }
        let dependency_worktree_id = dependency_worktree_identifier(dependency_worktree_id.into())?;
        let expected_branch = format!("lattice/{}", dependency_task_id.to_ascii_lowercase());
        if dependency_branch.into() != expected_branch || next_action != "COMPLETE_DEPENDENCY" {
            return Err(SnapshotError::MalformedReference);
        }
        let base_sha = base_sha.into();
        if !is_lower_hex(&base_sha, 40) {
            return Err(SnapshotError::MalformedReference);
        }
        let blocker_ref = format!(
            "{DEPENDENCY_BLOCKER_PREFIX}{parent_task_id}:{dependency_task_id}:{dependency_worktree_id}:{base_sha}"
        );
        bounded_reference(blocker_ref.clone())?;
        let domain = HashDomain::new("lattice.foreman-dependency-binding", "1.0")
            .map_err(|_| SnapshotError::MalformedReference)?;
        let evidence_ref = format!(
            "evidence:sha256:{}",
            canonical_sha256(&domain, &CanonicalValue::String(blocker_ref.clone()))
                .map_err(|_| SnapshotError::MalformedReference)?
                .to_hex()
        );
        Ok(Self {
            parent_task_id,
            dependency_task_id,
            dependency_worktree_id,
            dependency_branch: expected_branch,
            base_sha,
            blocker_ref,
            evidence_ref,
        })
    }

    /// Parses only the versioned dependency namespace. Legacy blocker strings
    /// remain opaque and return `None`.
    ///
    /// # Errors
    ///
    /// Only the complete canonical v1 encoding is promoted. A colliding
    /// historical free-form string remains an opaque legacy blocker.
    pub fn from_blocker_ref(value: &str) -> Result<Option<Self>, SnapshotError> {
        if !value.starts_with(DEPENDENCY_BLOCKER_PREFIX) {
            return Ok(None);
        }
        let fields = value.split(':').collect::<Vec<_>>();
        if fields.len() != 6 || fields[0] != "dependency" || fields[1] != "v1" {
            return Ok(None);
        }
        let Ok(binding) = Self::new(
            fields[2],
            fields[3],
            fields[4],
            format!("lattice/{}", fields[3].to_ascii_lowercase()),
            fields[5],
            "COMPLETE_DEPENDENCY",
        ) else {
            return Ok(None);
        };
        if binding.as_blocker_ref() != value {
            return Ok(None);
        }
        Ok(Some(binding))
    }

    #[must_use]
    pub fn parent_task_id(&self) -> &str {
        &self.parent_task_id
    }

    #[must_use]
    pub fn dependency_task_id(&self) -> &str {
        &self.dependency_task_id
    }

    #[must_use]
    pub fn dependency_worktree_id(&self) -> &str {
        &self.dependency_worktree_id
    }

    #[must_use]
    pub fn dependency_branch(&self) -> &str {
        &self.dependency_branch
    }

    #[must_use]
    pub fn base_sha(&self) -> &str {
        &self.base_sha
    }

    #[must_use]
    pub const fn next_action(&self) -> &'static str {
        "COMPLETE_DEPENDENCY"
    }

    #[must_use]
    pub fn as_blocker_ref(&self) -> &str {
        &self.blocker_ref
    }

    #[must_use]
    pub fn evidence_ref(&self) -> &str {
        &self.evidence_ref
    }
}

/// Restart-restored phase for the most recent structured dependency.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DependencyContinuationState {
    Blocked,
    Resumed,
}

/// Pure projection of one dependency relation from verified snapshot history.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DependencyContinuation {
    binding: DependencyBinding,
    parent_branch: String,
    parent_worktree: String,
    state: DependencyContinuationState,
}

impl DependencyContinuation {
    #[must_use]
    pub const fn state(&self) -> DependencyContinuationState {
        self.state
    }

    #[must_use]
    pub fn parent_task_id(&self) -> &str {
        self.binding.parent_task_id()
    }

    #[must_use]
    pub fn dependency_task_id(&self) -> &str {
        self.binding.dependency_task_id()
    }

    #[must_use]
    pub fn parent_branch(&self) -> &str {
        &self.parent_branch
    }

    #[must_use]
    pub fn parent_worktree(&self) -> &str {
        &self.parent_worktree
    }

    #[must_use]
    pub fn dependency_branch(&self) -> &str {
        self.binding.dependency_branch()
    }

    #[must_use]
    pub fn dependency_worktree_id(&self) -> &str {
        self.binding.dependency_worktree_id()
    }

    #[must_use]
    pub fn base_sha(&self) -> &str {
        self.binding.base_sha()
    }

    #[must_use]
    pub const fn next_action(&self) -> &'static str {
        match self.state {
            DependencyContinuationState::Blocked => "COMPLETE_DEPENDENCY",
            DependencyContinuationState::Resumed => "CONTINUE_PARENT",
        }
    }
}

/// Caller-owned, closed checkpoint intent. Server identity, Git and Writer
/// authority are deliberately absent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForemanCheckpointIntent {
    checkpoint_id: String,
    generation: u64,
    occurred_at: String,
    state: ForemanState,
    blocker_ref: Option<String>,
    heartbeat_ref: String,
    evidence_ref: String,
}

impl ForemanCheckpointIntent {
    /// # Errors
    ///
    /// Rejects unsafe IDs, zero generation, non-canonical time, malformed
    /// lowercase digest pointers, and state/blocker mismatch.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        checkpoint_id: impl Into<String>,
        generation: u64,
        occurred_at: impl Into<String>,
        state: ForemanState,
        blocker_ref: Option<String>,
        heartbeat_ref: impl Into<String>,
        evidence_ref: impl Into<String>,
    ) -> Result<Self, SnapshotError> {
        let checkpoint_id = checkpoint_identifier(checkpoint_id.into())?;
        if generation == 0 {
            return Err(SnapshotError::GenerationRollback);
        }
        let occurred_at = timestamp(occurred_at.into())?;
        let heartbeat_ref = lowercase_digest_pointer(heartbeat_ref.into(), "heartbeat")?;
        let evidence_ref = lowercase_digest_pointer(evidence_ref.into(), "evidence")?;
        let blocker_ref = blocker_ref.map(bounded_reference).transpose()?;
        if let Some(blocker) = blocker_ref.as_deref()
            && let Some(binding) = DependencyBinding::from_blocker_ref(blocker)?
            && binding.evidence_ref() != evidence_ref
        {
            return Err(SnapshotError::MalformedReference);
        }
        match (state, blocker_ref.is_some()) {
            (ForemanState::Blocked, false) => return Err(SnapshotError::MissingBlocker),
            (ForemanState::Active | ForemanState::Completed, true) => {
                return Err(SnapshotError::UnexpectedBlocker);
            }
            _ => {}
        }
        Ok(Self {
            checkpoint_id,
            generation,
            occurred_at,
            state,
            blocker_ref,
            heartbeat_ref,
            evidence_ref,
        })
    }

    #[must_use]
    pub fn checkpoint_id(&self) -> &str {
        &self.checkpoint_id
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub fn occurred_at(&self) -> &str {
        &self.occurred_at
    }

    #[must_use]
    pub const fn state(&self) -> ForemanState {
        self.state
    }

    #[must_use]
    pub fn blocker_ref(&self) -> Option<&str> {
        self.blocker_ref.as_deref()
    }

    #[must_use]
    pub fn heartbeat_ref(&self) -> &str {
        &self.heartbeat_ref
    }

    #[must_use]
    pub fn evidence_ref(&self) -> &str {
        &self.evidence_ref
    }

    /// Matches only caller-owned fields against one retained server snapshot.
    #[must_use]
    pub fn matches_snapshot(&self, snapshot: &ForemanSnapshot) -> bool {
        self.generation == snapshot.generation()
            && self.state == snapshot.state()
            && self.blocker_ref() == snapshot.blocker()
            && self.heartbeat_ref == snapshot.heartbeat()
            && self.evidence_ref == snapshot.evidence()
    }
}

/// Server-owned binding and Git observation made only after replay proves that
/// a checkpoint is new. Writer authority is attached later by Orchestrator.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForemanServerObservation {
    worker: String,
    thread: String,
    task: String,
    branch: String,
    worktree: String,
    head: String,
}

impl ForemanServerObservation {
    /// # Errors
    ///
    /// Rejects malformed fixed identity or Git evidence.
    pub fn new(
        worker: impl Into<String>,
        thread: impl Into<String>,
        task: impl Into<String>,
        branch: impl Into<String>,
        worktree: impl Into<String>,
        head: impl Into<String>,
    ) -> Result<Self, SnapshotError> {
        let head = head.into();
        if !is_hex(&head, 40) {
            return Err(SnapshotError::MalformedReference);
        }
        Ok(Self {
            worker: bounded_reference(worker.into())?,
            thread: bounded_reference(thread.into())?,
            task: bounded_reference(task.into())?,
            branch: bounded_reference(branch.into())?,
            worktree: bounded_reference(worktree.into())?,
            head,
        })
    }

    /// Binds caller intent to the newly acquired Writer authority.
    ///
    /// # Errors
    ///
    /// Rejects malformed authority evidence or any impossible snapshot shape.
    pub fn bind(
        self,
        intent: &ForemanCheckpointIntent,
        authority_ref: impl Into<String>,
    ) -> Result<ForemanSnapshot, SnapshotError> {
        ForemanSnapshot::new(
            self.worker,
            self.thread,
            self.task,
            self.branch,
            self.worktree,
            self.head,
            intent.state(),
            intent.blocker_ref().map(str::to_owned),
            intent.heartbeat_ref(),
            lowercase_digest_pointer(authority_ref.into(), "authority")?,
            intent.evidence_ref(),
            intent.generation(),
        )
    }
}

/// One versioned, bounded coordination record. It deliberately has no free-form
/// transcript, command, path, environment, or credential field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForemanSnapshot {
    worker: String,
    thread: String,
    task: String,
    branch: String,
    worktree: String,
    head: String,
    state: ForemanState,
    blocker: Option<String>,
    heartbeat: String,
    authority: String,
    evidence: String,
    generation: u64,
    epistemic: Option<EpistemicReferences>,
}

impl ForemanSnapshot {
    /// # Errors
    ///
    /// Returns a typed rejection for malformed, secret-bearing, or
    /// state-incompatible fields.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        worker: impl Into<String>,
        thread: impl Into<String>,
        task: impl Into<String>,
        branch: impl Into<String>,
        worktree: impl Into<String>,
        head: impl Into<String>,
        state: ForemanState,
        blocker: Option<String>,
        heartbeat: impl Into<String>,
        authority: impl Into<String>,
        evidence: impl Into<String>,
        generation: u64,
    ) -> Result<Self, SnapshotError> {
        let worker = bounded_reference(worker.into())?;
        let thread = bounded_reference(thread.into())?;
        let task = bounded_reference(task.into())?;
        let branch = bounded_reference(branch.into())?;
        let worktree = bounded_reference(worktree.into())?;
        let head = head.into();
        if !is_hex(&head, 40) {
            return Err(SnapshotError::MalformedReference);
        }
        let heartbeat = digest_pointer(heartbeat.into(), "heartbeat")?;
        let authority = digest_pointer(authority.into(), "authority")?;
        let evidence = digest_pointer(evidence.into(), "evidence")?;
        if generation == 0 {
            return Err(SnapshotError::GenerationRollback);
        }
        let blocker = blocker.map(bounded_reference).transpose()?;
        if let Some(blocker) = blocker.as_deref() {
            DependencyBinding::from_blocker_ref(blocker)?;
        }
        match (state, blocker.is_some()) {
            (ForemanState::Blocked, false) => return Err(SnapshotError::MissingBlocker),
            (ForemanState::Active | ForemanState::Completed, true) => {
                return Err(SnapshotError::UnexpectedBlocker);
            }
            _ => {}
        }
        Ok(Self {
            worker,
            thread,
            task,
            branch,
            worktree,
            head,
            state,
            blocker,
            heartbeat,
            authority,
            evidence,
            generation,
            epistemic: None,
        })
    }

    /// Attaches only expiring, non-authoritative pointers to this snapshot.
    ///
    /// # Errors
    ///
    /// Rejects an epistemic record whose lifetime has already expired.
    pub fn with_epistemic(mut self, epistemic: EpistemicReferences) -> Result<Self, SnapshotError> {
        if epistemic.expires_at <= epistemic.checked_at {
            return Err(SnapshotError::MalformedReference);
        }
        self.epistemic = Some(epistemic);
        Ok(self)
    }

    #[must_use]
    pub const fn schema(&self) -> &'static str {
        SNAPSHOT_SCHEMA
    }

    #[must_use]
    pub fn worker(&self) -> &str {
        &self.worker
    }

    #[must_use]
    pub fn thread(&self) -> &str {
        &self.thread
    }

    #[must_use]
    pub fn task(&self) -> &str {
        &self.task
    }

    #[must_use]
    pub fn branch(&self) -> &str {
        &self.branch
    }

    #[must_use]
    pub fn worktree(&self) -> &str {
        &self.worktree
    }

    #[must_use]
    pub fn head(&self) -> &str {
        &self.head
    }

    #[must_use]
    pub const fn state(&self) -> ForemanState {
        self.state
    }

    #[must_use]
    pub fn blocker(&self) -> Option<&str> {
        self.blocker.as_deref()
    }

    #[must_use]
    pub fn heartbeat(&self) -> &str {
        &self.heartbeat
    }

    /// Digest pointer to the authority receipt/head used for this report.
    #[must_use]
    pub fn authority(&self) -> &str {
        &self.authority
    }

    #[must_use]
    pub fn evidence(&self) -> &str {
        &self.evidence
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Returns provisional pointers only; callers must not use them as state.
    #[must_use]
    pub const fn epistemic(&self) -> Option<&EpistemicReferences> {
        self.epistemic.as_ref()
    }
}

/// One reconstructed blocked record. Blocked coordination never permits archive.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockedWorker {
    snapshot: ForemanSnapshot,
}

impl BlockedWorker {
    #[must_use]
    pub const fn archive_ready(&self) -> bool {
        false
    }
}

/// Fresh-reader projection over verified ordered snapshot events.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForemanProjection {
    active: Vec<ForemanSnapshot>,
    blocked: Vec<BlockedWorker>,
    completed: Vec<ForemanSnapshot>,
    latest_generation: u64,
    next_action: String,
    dependency: Option<DependencyContinuation>,
}

impl ForemanProjection {
    #[must_use]
    pub fn active(&self) -> &[ForemanSnapshot] {
        &self.active
    }

    #[must_use]
    pub fn blocked(&self) -> &[BlockedWorker] {
        &self.blocked
    }

    #[must_use]
    pub fn completed(&self) -> &[ForemanSnapshot] {
        &self.completed
    }

    #[must_use]
    pub const fn latest_generation(&self) -> u64 {
        self.latest_generation
    }

    #[must_use]
    pub const fn runtime_next_action(&self) -> &'static str {
        if !self.blocked.is_empty() {
            "RESOLVE_BLOCKERS"
        } else if !self.active.is_empty() {
            "CONTINUE"
        } else if !self.completed.is_empty() {
            "ALL_COMPLETED"
        } else {
            "NO_DURABLE_SNAPSHOT"
        }
    }

    #[must_use]
    pub fn next_action(&self) -> &str {
        &self.next_action
    }

    #[must_use]
    pub const fn dependency(&self) -> Option<&DependencyContinuation> {
        self.dependency.as_ref()
    }
}

/// Reconstructs the current worker projection from append order without I/O.
///
/// # Errors
///
/// Rejects duplicate worker ownership and non-monotonic generations.
pub fn reconstruct(
    snapshots: impl IntoIterator<Item = ForemanSnapshot>,
) -> Result<ForemanProjection, SnapshotError> {
    let mut by_worker = BTreeMap::<String, ForemanSnapshot>::new();
    let mut dependency = None::<(String, DependencyContinuation)>;
    for snapshot in snapshots {
        if let Some(previous) = by_worker.get(snapshot.worker()) {
            if previous.thread() != snapshot.thread() {
                return Err(SnapshotError::DuplicateWorkerIdentity);
            }
            if !is_exact_next_generation(Some(previous.generation()), snapshot.generation()) {
                return Err(SnapshotError::GenerationRollback);
            }
        } else if !is_exact_next_generation(None, snapshot.generation()) {
            return Err(SnapshotError::GenerationRollback);
        }
        if snapshot.state() == ForemanState::Blocked {
            if let Some(blocker) = snapshot.blocker()
                && let Some(binding) = DependencyBinding::from_blocker_ref(blocker)?
                && binding.evidence_ref() == snapshot.evidence()
            {
                if binding.base_sha() != snapshot.head() {
                    return Err(SnapshotError::MalformedReference);
                }
                if let Some((worker, current)) = dependency.as_ref()
                    && (worker != snapshot.worker()
                        || (current.state == DependencyContinuationState::Blocked
                            && current.binding != binding))
                {
                    return Err(SnapshotError::DuplicateWorkerIdentity);
                }
                dependency = Some((
                    snapshot.worker().to_owned(),
                    DependencyContinuation {
                        binding,
                        parent_branch: snapshot.branch().to_owned(),
                        parent_worktree: snapshot.worktree().to_owned(),
                        state: DependencyContinuationState::Blocked,
                    },
                ));
            }
        } else if let Some((worker, current)) = dependency.as_mut()
            && worker == snapshot.worker()
        {
            match (snapshot.state(), current.state) {
                (ForemanState::Active, DependencyContinuationState::Blocked) => {
                    current.state = DependencyContinuationState::Resumed;
                }
                (ForemanState::Completed, DependencyContinuationState::Blocked) => {
                    return Err(SnapshotError::MalformedReference);
                }
                _ => {}
            }
        }
        by_worker.insert(snapshot.worker().to_owned(), snapshot);
    }
    let mut active = Vec::new();
    let mut blocked = Vec::new();
    let mut completed = Vec::new();
    let mut latest_generation = 0;
    for snapshot in by_worker.into_values() {
        latest_generation = latest_generation.max(snapshot.generation());
        match snapshot.state() {
            ForemanState::Active => active.push(snapshot),
            ForemanState::Blocked => blocked.push(BlockedWorker { snapshot }),
            ForemanState::Completed => completed.push(snapshot),
        }
    }
    let next_action = if let Some(blocked_worker) = blocked.first() {
        format!(
            "unblock {}: {}",
            blocked_worker.snapshot.worker(),
            blocked_worker.snapshot.blocker().unwrap_or_default(),
        )
    } else if let Some(active_worker) = active.first() {
        format!("await {}", active_worker.worker())
    } else {
        "no active worker".to_owned()
    };
    Ok(ForemanProjection {
        active,
        blocked,
        completed,
        latest_generation,
        next_action,
        dependency: dependency.map(|(_, continuation)| continuation),
    })
}

/// Returns whether `candidate` is the only allowed generation after `previous`.
/// An empty identity starts at one, and overflow never wraps to a valid value.
#[must_use]
pub const fn is_exact_next_generation(previous: Option<u64>, candidate: u64) -> bool {
    match previous {
        None => candidate == 1,
        Some(previous) => match previous.checked_add(1) {
            Some(expected) => candidate == expected,
            None => false,
        },
    }
}

/// Read-only dashboard metadata; it is never a durable authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DashboardIndex {
    generated_at: String,
    branch: String,
    head: String,
    outcome: String,
}

impl DashboardIndex {
    /// # Errors
    ///
    /// Rejects malformed bounded dashboard index values.
    pub fn new(
        generated_at: impl Into<String>,
        branch: impl Into<String>,
        head: impl Into<String>,
        outcome: impl Into<String>,
    ) -> Result<Self, SnapshotError> {
        let generated_at = bounded_reference(generated_at.into())?;
        let branch = bounded_reference(branch.into())?;
        let head = head.into();
        if !is_hex(&head, 40) {
            return Err(SnapshotError::MalformedReference);
        }
        let outcome = bounded_reference(outcome.into())?;
        Ok(Self {
            generated_at,
            branch,
            head,
            outcome,
        })
    }
}

/// Independently collected current worktree facts, injected by a later adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiveWorktree {
    worker: String,
    branch: String,
    head: String,
    heartbeat_fresh: bool,
}

impl LiveWorktree {
    /// # Errors
    ///
    /// Rejects malformed bounded live worktree values.
    pub fn new(
        worker: impl Into<String>,
        branch: impl Into<String>,
        head: impl Into<String>,
        heartbeat_fresh: bool,
    ) -> Result<Self, SnapshotError> {
        let worker = bounded_reference(worker.into())?;
        let branch = bounded_reference(branch.into())?;
        let head = head.into();
        if !is_hex(&head, 40) {
            return Err(SnapshotError::MalformedReference);
        }
        Ok(Self {
            worker,
            branch,
            head,
            heartbeat_fresh,
        })
    }
}

/// Fail-closed watchdog results.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WatchdogFinding {
    AllWorkersMissedHeartbeat,
    OldHead { worker: String },
    DashboardDrift,
}

/// Compares untrusted dashboard metadata with injected live observations.
///
/// # Errors
///
/// Rejects a snapshot with no exact independently supplied live worker.
pub fn watchdog(
    snapshots: &[ForemanSnapshot],
    dashboard: &DashboardIndex,
    live: &[LiveWorktree],
) -> Result<Vec<WatchdogFinding>, SnapshotError> {
    let mut findings = Vec::new();
    if !live.is_empty() && live.iter().all(|item| !item.heartbeat_fresh) {
        findings.push(WatchdogFinding::AllWorkersMissedHeartbeat);
    }
    for snapshot in snapshots {
        let item = live
            .iter()
            .find(|candidate| candidate.worker == snapshot.worker());
        let Some(item) = item else {
            return Err(SnapshotError::DuplicateWorkerIdentity);
        };
        if item.branch != snapshot.branch() || item.head != snapshot.head() {
            findings.push(WatchdogFinding::OldHead {
                worker: snapshot.worker().to_owned(),
            });
        }
        if (dashboard.branch != item.branch
            || dashboard.head != item.head
            || dashboard.outcome != snapshot.state().as_str())
            && !findings.contains(&WatchdogFinding::DashboardDrift)
        {
            findings.push(WatchdogFinding::DashboardDrift);
        }
    }
    Ok(findings)
}

impl ForemanState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::Blocked => "BLOCKED",
            Self::Completed => "COMPLETED",
        }
    }

    /// Parses the closed persistence spelling.
    ///
    /// # Errors
    ///
    /// Rejects every unknown or case-substituted value.
    pub fn from_persisted(value: &str) -> Result<Self, SnapshotError> {
        match value {
            "ACTIVE" => Ok(Self::Active),
            "BLOCKED" => Ok(Self::Blocked),
            "COMPLETED" => Ok(Self::Completed),
            _ => Err(SnapshotError::MalformedReference),
        }
    }
}

fn bounded_reference(value: String) -> Result<String, SnapshotError> {
    if value.is_empty()
        || value.len() > MAX_REFERENCE_BYTES
        || !value.is_ascii()
        || value.contains(char::is_whitespace)
        || looks_secret_like(&value)
    {
        return Err(if looks_secret_like(&value) {
            SnapshotError::ForbiddenContent
        } else {
            SnapshotError::MalformedReference
        });
    }
    Ok(value)
}

fn digest_pointer(value: String, prefix: &str) -> Result<String, SnapshotError> {
    let expected_prefix = format!("{prefix}:sha256:");
    if !value.starts_with(&expected_prefix) || !is_hex(&value[expected_prefix.len()..], 64) {
        return Err(if looks_secret_like(&value) {
            SnapshotError::ForbiddenContent
        } else {
            SnapshotError::MalformedReference
        });
    }
    Ok(value)
}

fn lowercase_digest_pointer(value: String, prefix: &str) -> Result<String, SnapshotError> {
    let expected_prefix = format!("{prefix}:sha256:");
    let digest = value
        .strip_prefix(&expected_prefix)
        .ok_or(SnapshotError::MalformedReference)?;
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(SnapshotError::MalformedReference);
    }
    Ok(value)
}

fn checkpoint_identifier(value: String) -> Result<String, SnapshotError> {
    let mut bytes = value.bytes();
    if value.len() > MAX_CHECKPOINT_ID_BYTES
        || !bytes
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        || !bytes
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(SnapshotError::MalformedReference);
    }
    Ok(value)
}

fn dependency_task_identifier(value: String) -> Result<String, SnapshotError> {
    let suffix = value
        .strip_prefix("TASK-")
        .ok_or(SnapshotError::MalformedReference)?;
    let mut bytes = suffix.bytes();
    if value.len() > 64
        || suffix.len() < 3
        || !bytes
            .next()
            .is_some_and(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        || !bytes.all(|byte| {
            byte.is_ascii_uppercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
    {
        return Err(SnapshotError::MalformedReference);
    }
    Ok(value)
}

fn dependency_worktree_identifier(value: String) -> Result<String, SnapshotError> {
    let mut bytes = value.bytes();
    if !(3..=64).contains(&value.len())
        || !bytes
            .next()
            .is_some_and(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        || !bytes.all(|byte| {
            byte.is_ascii_uppercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
    {
        return Err(SnapshotError::MalformedReference);
    }
    Ok(value)
}

fn pointer_list(values: Vec<String>, prefix: &str) -> Result<Vec<String>, SnapshotError> {
    if values.len() > 64 {
        return Err(SnapshotError::MalformedReference);
    }
    values
        .into_iter()
        .map(|value| digest_pointer(value, prefix))
        .collect()
}

fn timestamp(value: String) -> Result<String, SnapshotError> {
    let bytes = value.as_bytes();
    if bytes.len() != 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'Z'
        || !bytes
            .iter()
            .enumerate()
            .filter(|(index, _)| !matches!(index, 4 | 7 | 10 | 13 | 16 | 19))
            .all(|(_, byte)| byte.is_ascii_digit())
    {
        return Err(SnapshotError::MalformedReference);
    }
    let parsed =
        OffsetDateTime::parse(&value, &Rfc3339).map_err(|_| SnapshotError::MalformedReference)?;
    if parsed
        .format(&Rfc3339)
        .map_err(|_| SnapshotError::MalformedReference)?
        != value
    {
        return Err(SnapshotError::MalformedReference);
    }
    Ok(value)
}

fn is_hex(value: &str, expected_len: usize) -> bool {
    value.len() == expected_len && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_lower_hex(value: &str, expected_len: usize) -> bool {
    value.len() == expected_len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn attempt_digest_pointer(value: &str, prefix: &str) -> Result<String, WorkerAttemptError> {
    let expected_prefix = format!("{prefix}:sha256:");
    let Some(digest) = value.strip_prefix(&expected_prefix) else {
        return Err(if looks_secret_like(value) {
            WorkerAttemptError::ForbiddenContent
        } else {
            WorkerAttemptError::MalformedField
        });
    };
    if !is_lower_hex(digest, 64) {
        return Err(WorkerAttemptError::MalformedField);
    }
    Ok(value.to_owned())
}

fn canonical_digest_pointer(
    schema: &str,
    version: &str,
    prefix: &str,
    value: &CanonicalValue,
) -> Result<String, WorkerAttemptError> {
    let domain = HashDomain::new(schema, version).map_err(|_| WorkerAttemptError::DigestFailure)?;
    let digest = canonical_sha256(&domain, value).map_err(|_| WorkerAttemptError::DigestFailure)?;
    Ok(format!("{prefix}:sha256:{}", digest.to_hex()))
}

fn text_canonical(label: &str, value: &impl ToString) -> (String, CanonicalValue) {
    (label.to_owned(), CanonicalValue::String(value.to_string()))
}

fn attempt_timestamp(value: String) -> Result<String, WorkerAttemptError> {
    timestamp(value).map_err(|_| WorkerAttemptError::MalformedField)
}

fn parse_attempt_time(value: &str) -> Result<OffsetDateTime, WorkerAttemptError> {
    OffsetDateTime::parse(value, &Rfc3339).map_err(|_| WorkerAttemptError::MalformedField)
}

fn attempt_event_timestamp(value: String) -> Result<String, WorkerAttemptError> {
    let parsed = parse_attempt_time(&value)?;
    let canonical = parsed
        .format(&Rfc3339)
        .map_err(|_| WorkerAttemptError::MalformedField)?;
    if !value.ends_with('Z') || canonical != value {
        return Err(WorkerAttemptError::MalformedField);
    }
    Ok(value)
}

fn derive_attempt_deadline(
    started_at: &str,
    max_duration_seconds: u64,
    task_deadline_at: &str,
) -> Result<String, WorkerAttemptError> {
    let seconds =
        i64::try_from(max_duration_seconds).map_err(|_| WorkerAttemptError::InvalidBudget)?;
    let attempt_deadline = parse_attempt_time(started_at)?
        .checked_add(time::Duration::seconds(seconds))
        .ok_or(WorkerAttemptError::InvalidBudget)?;
    let task_deadline = parse_attempt_time(task_deadline_at)?;
    let deadline = attempt_deadline.min(task_deadline);
    deadline
        .format(&Rfc3339)
        .map_err(|_| WorkerAttemptError::MalformedField)
}

fn attempt_identifier(value: String) -> Result<String, WorkerAttemptError> {
    let mut bytes = value.bytes();
    if !(3..=128).contains(&value.len())
        || !bytes
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        || !bytes
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(if looks_worker_secret_like(&value) {
            WorkerAttemptError::ForbiddenContent
        } else {
            WorkerAttemptError::MalformedField
        });
    }
    if looks_worker_secret_like(&value) {
        return Err(WorkerAttemptError::ForbiddenContent);
    }
    Ok(value)
}

fn looks_worker_secret_like(value: &str) -> bool {
    let lowercase = value.to_ascii_lowercase();
    looks_secret_like(value)
        || lowercase.starts_with("bearer-")
        || lowercase.contains("authorization:")
        || lowercase.contains("api_key")
        || lowercase.contains("api-key")
        || lowercase.contains("token=")
        || lowercase.contains("client_secret")
        || lowercase.contains("private key")
        || lowercase.contains("full prompt")
        || lowercase.contains("system prompt")
        || (lowercase.contains("://") && lowercase.contains('@'))
}

fn looks_secret_like(value: &str) -> bool {
    let lowercase = value.to_ascii_lowercase();
    lowercase.starts_with("sk-")
        || lowercase.starts_with("bearer ")
        || lowercase.contains("password")
        || lowercase.contains("full chat")
        || lowercase.contains("begin private")
}
