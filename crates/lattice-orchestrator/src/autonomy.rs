//! Versioned, pure autonomy-control recommendations.
//!
//! This module recommends a bounded next step. It neither invokes a model nor
//! changes task state; Policy, Task Ledger, Writer Lease, and composition keep
//! authority for those effects.

use lattice_contracts::{
    ContentDigest, RuntimeAdmissionMode, RuntimeKind, SubjectBinding, WriterLeaseAuthorityHead,
    WriterLeaseStatus, sha256_content_digest,
};
use lattice_task_domain::{RiskClass, TaskState};

const AUTONOMY_RECEIPT_SCHEMA: &str = "lattice.autonomy-receipt/1.0";
const AUTONOMY_RECEIPT_DOMAIN: &str = "lattice.autonomy-receipt";
const AUTONOMY_AUTHORITY_DOMAIN: &str = "lattice.autonomy-authority";
const AUTONOMY_HASH_VERSION: &str = "1.0";
const P0_AUTHORITY_MODE: &str = "P0_PROCESS_START_PROFILE_V1";

#[derive(Clone, Debug, Eq, PartialEq)]
enum CanonicalValue {
    Null,
    Bool(bool),
    String(String),
    Object(Vec<(String, Self)>),
}

/// Fail-closed canonical autonomy contract errors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutonomyContractError {
    InvalidDigest,
    WriterAuthorityRequired,
    UnexpectedWriterAuthority,
    WriterAuthorityMismatch,
    Canonicalization,
}

/// Complete P0 authority evidence consumed by one canonical receipt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AutonomyAuthorityEvidence {
    process_start_authority_digest: ContentDigest,
    ingress_profile_adapter_commitment: ContentDigest,
    store_authority_head_digest: ContentDigest,
    writer_authority: Option<WriterLeaseAuthorityHead>,
}

impl AutonomyAuthorityEvidence {
    /// Constructs the only authority profile supported by TASK-050.
    ///
    /// # Errors
    ///
    /// Rejects a zero mandatory authority digest.
    pub fn new_p0_process_start_profile(
        process_start_authority_digest: ContentDigest,
        ingress_profile_adapter_commitment: ContentDigest,
        store_authority_head_digest: ContentDigest,
        writer_authority: Option<WriterLeaseAuthorityHead>,
    ) -> Result<Self, AutonomyContractError> {
        if [
            &process_start_authority_digest,
            &ingress_profile_adapter_commitment,
            &store_authority_head_digest,
        ]
        .into_iter()
        .any(is_zero_digest)
        {
            return Err(AutonomyContractError::InvalidDigest);
        }
        Ok(Self {
            process_start_authority_digest,
            ingress_profile_adapter_commitment,
            store_authority_head_digest,
            writer_authority,
        })
    }

    #[must_use]
    pub const fn process_start_authority_digest(&self) -> &ContentDigest {
        &self.process_start_authority_digest
    }

    #[must_use]
    pub const fn ingress_profile_adapter_commitment(&self) -> &ContentDigest {
        &self.ingress_profile_adapter_commitment
    }

    #[must_use]
    pub const fn store_authority_head_digest(&self) -> &ContentDigest {
        &self.store_authority_head_digest
    }

    #[must_use]
    pub const fn writer_authority(&self) -> Option<&WriterLeaseAuthorityHead> {
        self.writer_authority.as_ref()
    }
}

/// Canonical digest-bound autonomy receipt intended for Task Ledger persistence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalAutonomyReceipt {
    binding: SubjectBinding,
    intent: AutonomyIntent,
    observed_state: TaskState,
    decision: AutonomyDecision,
    authority_digest: ContentDigest,
    receipt_digest: ContentDigest,
    writer_fencing_token: Option<u64>,
    authority_evidence: AutonomyAuthorityEvidence,
    writer_lease_head_digest: Option<ContentDigest>,
}

impl CanonicalAutonomyReceipt {
    #[must_use]
    pub const fn schema_version(&self) -> &'static str {
        AUTONOMY_RECEIPT_SCHEMA
    }

    #[must_use]
    pub const fn authority_mode(&self) -> &'static str {
        P0_AUTHORITY_MODE
    }

    #[must_use]
    pub const fn binding(&self) -> &SubjectBinding {
        &self.binding
    }

    #[must_use]
    pub const fn intent(&self) -> AutonomyIntent {
        self.intent
    }

    #[must_use]
    pub const fn observed_state(&self) -> TaskState {
        self.observed_state
    }

    #[must_use]
    pub const fn decision(&self) -> AutonomyDecision {
        self.decision
    }

    #[must_use]
    pub const fn authority_digest(&self) -> &ContentDigest {
        &self.authority_digest
    }

    #[must_use]
    pub const fn receipt_digest(&self) -> &ContentDigest {
        &self.receipt_digest
    }

    #[must_use]
    pub const fn writer_fencing_token(&self) -> Option<u64> {
        self.writer_fencing_token
    }

    #[must_use]
    pub const fn authority_evidence(&self) -> &AutonomyAuthorityEvidence {
        &self.authority_evidence
    }

    #[must_use]
    pub const fn writer_lease_head_digest(&self) -> Option<&ContentDigest> {
        self.writer_lease_head_digest.as_ref()
    }
}

/// The only supported interpretation version for this minimal control slice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutonomyIntentVersion {
    V1,
}

/// Coarse task categories accepted by the local rule set.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskKind {
    Feature,
    BugFix,
    Configuration,
    Research,
}

/// Immutable, caller-reviewed task intent supplied to the pure classifier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AutonomyIntent {
    pub version: AutonomyIntentVersion,
    pub kind: TaskKind,
    pub risk: RiskClass,
    /// Whether the requested local execution boundary is already authorized.
    pub execution_preapproved: bool,
    /// Whether an effect needs authority not represented by the current task.
    pub requires_new_authority: bool,
    /// Whether the requested effect is high-risk or difficult to reverse.
    pub irreversible_or_high_risk: bool,
}

/// A recommendation, not a provider selection or model invocation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelRecommendation {
    GovernedCodexWriter,
    NoModel,
}

/// The minimum verification category recommended before ordinary progression.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerificationRecommendation {
    FocusedChecks,
    BuildAndFocusedChecks,
    ReadOnlyEvidence,
}

/// Why autonomous progression stopped or can continue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutonomyDecisionReason {
    RoutineAuthorized,
    NewUserDecision,
    NewAuthority,
    HighRiskOrIrreversible,
}

/// Closed, explainable control recommendation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutonomyDecision {
    Proceed {
        model: ModelRecommendation,
        verification: VerificationRecommendation,
        reason: AutonomyDecisionReason,
    },
    AskUser {
        reason: AutonomyDecisionReason,
    },
}

/// Non-durable status receipt; callers must bind it to the existing Ledger to persist it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AutonomyReceipt {
    pub version: AutonomyIntentVersion,
    pub observed_state: TaskState,
    pub decision: AutonomyDecision,
}

/// Classifies an already-understood intent with fail-closed priority.
///
/// It cannot select an unavailable model, scheduler, remote service, or a new
/// authority: the only writer recommendation is the existing governed Codex path.
#[must_use]
pub const fn classify_autonomy(
    intent: AutonomyIntent,
    observed_state: TaskState,
) -> AutonomyReceipt {
    let decision = if intent.requires_new_authority {
        AutonomyDecision::AskUser {
            reason: AutonomyDecisionReason::NewAuthority,
        }
    } else if intent.irreversible_or_high_risk || matches!(intent.risk, RiskClass::R3) {
        AutonomyDecision::AskUser {
            reason: AutonomyDecisionReason::HighRiskOrIrreversible,
        }
    } else if !intent.execution_preapproved {
        AutonomyDecision::AskUser {
            reason: AutonomyDecisionReason::NewUserDecision,
        }
    } else {
        let model = match intent.kind {
            TaskKind::Feature | TaskKind::BugFix => ModelRecommendation::GovernedCodexWriter,
            TaskKind::Configuration | TaskKind::Research => ModelRecommendation::NoModel,
        };
        let verification = match intent.kind {
            TaskKind::Research => VerificationRecommendation::ReadOnlyEvidence,
            TaskKind::Configuration | TaskKind::Feature | TaskKind::BugFix
                if matches!(intent.risk, RiskClass::R2) =>
            {
                VerificationRecommendation::BuildAndFocusedChecks
            }
            TaskKind::Configuration | TaskKind::Feature | TaskKind::BugFix => {
                VerificationRecommendation::FocusedChecks
            }
        };
        AutonomyDecision::Proceed {
            model,
            verification,
            reason: AutonomyDecisionReason::RoutineAuthorized,
        }
    };
    AutonomyReceipt {
        version: intent.version,
        observed_state,
        decision,
    }
}

/// Builds the exact canonical TASK-050 receipt after recomputing the decision.
///
/// # Errors
///
/// Rejects missing, unexpected, stale, or cross-bound writer authority and
/// any value that cannot be represented by the closed canonical contract.
pub fn build_autonomy_receipt(
    binding: SubjectBinding,
    intent: AutonomyIntent,
    observed_state: TaskState,
    authority: AutonomyAuthorityEvidence,
) -> Result<CanonicalAutonomyReceipt, AutonomyContractError> {
    let recommendation = classify_autonomy(intent, observed_state);
    let writer = authority.writer_authority.as_ref();
    match recommendation.decision {
        AutonomyDecision::Proceed { .. } => {
            let writer = writer.ok_or(AutonomyContractError::WriterAuthorityRequired)?;
            if writer.runtime() != RuntimeKind::Live
                || writer.status() != WriterLeaseStatus::Active
                || writer.runtime_admission() != RuntimeAdmissionMode::Active
                || !writer_binding_matches(writer, &binding)
            {
                return Err(AutonomyContractError::WriterAuthorityMismatch);
            }
        }
        AutonomyDecision::AskUser { .. } if writer.is_some() => {
            return Err(AutonomyContractError::UnexpectedWriterAuthority);
        }
        AutonomyDecision::AskUser { .. } => {}
    }

    let writer_lease_head_digest = writer_head_digest(authority.writer_authority.as_ref())?;
    let authority_value = authority_value(&binding, &authority)?;
    let authority_digest = canonical_digest(AUTONOMY_AUTHORITY_DOMAIN, &authority_value)?;
    let receipt_value = receipt_value(
        &binding,
        intent,
        observed_state,
        recommendation.decision,
        &authority_digest,
    );
    let receipt_digest = canonical_digest(AUTONOMY_RECEIPT_DOMAIN, &receipt_value)?;
    Ok(CanonicalAutonomyReceipt {
        binding,
        intent,
        observed_state,
        decision: recommendation.decision,
        authority_digest,
        receipt_digest,
        writer_fencing_token: writer.map(|head| head.identity().fencing_token().get()),
        authority_evidence: authority,
        writer_lease_head_digest,
    })
}

fn writer_head_digest(
    writer: Option<&WriterLeaseAuthorityHead>,
) -> Result<Option<ContentDigest>, AutonomyContractError> {
    writer
        .map(|writer| {
            canonical_digest(
                "lattice.autonomy-writer-lease-head",
                &CanonicalValue::Object(vec![
                    ("producer_id".into(), text(writer.producer_id())),
                    ("producer_version".into(), text(writer.producer_version())),
                    ("runtime".into(), text("LIVE")),
                    ("status".into(), text(writer.status().as_str())),
                    (
                        "runtime_admission".into(),
                        text(writer.runtime_admission().as_str()),
                    ),
                    ("revision".into(), text(writer.revision().get().to_string())),
                    (
                        "receipt_digest".into(),
                        text(writer.receipt_digest().as_str()),
                    ),
                    (
                        "transition_digest".into(),
                        text(writer.transition_digest().as_str()),
                    ),
                    (
                        "fencing_token".into(),
                        text(writer.identity().fencing_token().get().to_string()),
                    ),
                ]),
            )
        })
        .transpose()
}

fn writer_binding_matches(writer: &WriterLeaseAuthorityHead, binding: &SubjectBinding) -> bool {
    let identity = writer.identity();
    identity.project_id() == binding.project_id()
        && identity.project_snapshot_id() == binding.project_snapshot_id()
        && identity.task_id() == binding.task_id()
        && identity.task_revision() == binding.task_revision()
        && identity.task_spec_digest() == binding.task_spec_digest()
}

fn authority_value(
    binding: &SubjectBinding,
    authority: &AutonomyAuthorityEvidence,
) -> Result<CanonicalValue, AutonomyContractError> {
    let (writer_receipt, writer_head, writer_fence) = match authority.writer_authority.as_ref() {
        Some(writer) => {
            let head_digest =
                writer_head_digest(Some(writer))?.ok_or(AutonomyContractError::Canonicalization)?;
            (
                text(writer.receipt_digest().as_str()),
                text(head_digest.as_str()),
                text(writer.identity().fencing_token().get().to_string()),
            )
        }
        None => (
            CanonicalValue::Null,
            CanonicalValue::Null,
            CanonicalValue::Null,
        ),
    };
    Ok(CanonicalValue::Object(vec![
        ("binding".into(), binding_value(binding)),
        ("authority_mode".into(), text(P0_AUTHORITY_MODE)),
        (
            "process_start_authority_digest".into(),
            text(authority.process_start_authority_digest.as_str()),
        ),
        (
            "ingress_profile_adapter_commitment".into(),
            text(authority.ingress_profile_adapter_commitment.as_str()),
        ),
        (
            "store_authority_head_digest".into(),
            text(authority.store_authority_head_digest.as_str()),
        ),
        (
            "policy_decision_receipt_digest".into(),
            CanonicalValue::Null,
        ),
        ("policy_owner_head_digest".into(), CanonicalValue::Null),
        ("approval_receipt_digest".into(), CanonicalValue::Null),
        ("approval_owner_head_digest".into(), CanonicalValue::Null),
        ("writer_lease_receipt_digest".into(), writer_receipt),
        ("writer_lease_head_digest".into(), writer_head),
        ("writer_fencing_token".into(), writer_fence),
    ]))
}

fn receipt_value(
    binding: &SubjectBinding,
    intent: AutonomyIntent,
    observed_state: TaskState,
    decision: AutonomyDecision,
    authority_digest: &ContentDigest,
) -> CanonicalValue {
    CanonicalValue::Object(vec![
        ("schema_version".into(), text(AUTONOMY_RECEIPT_SCHEMA)),
        ("binding".into(), binding_value(binding)),
        (
            "intent".into(),
            CanonicalValue::Object(vec![
                ("version".into(), text(intent.version.as_str())),
                ("task_kind".into(), text(intent.kind.as_str())),
                ("risk_class".into(), text(intent.risk.as_str())),
                (
                    "execution_preapproved".into(),
                    CanonicalValue::Bool(intent.execution_preapproved),
                ),
                (
                    "requires_new_authority".into(),
                    CanonicalValue::Bool(intent.requires_new_authority),
                ),
                (
                    "irreversible_or_high_risk".into(),
                    CanonicalValue::Bool(intent.irreversible_or_high_risk),
                ),
            ]),
        ),
        ("observed_task_state".into(), text(observed_state.as_str())),
        ("decision".into(), decision_value(decision)),
        ("authority_digest".into(), text(authority_digest.as_str())),
    ])
}

fn binding_value(binding: &SubjectBinding) -> CanonicalValue {
    CanonicalValue::Object(vec![
        ("project_id".into(), text(binding.project_id().as_str())),
        (
            "project_snapshot_id".into(),
            text(binding.project_snapshot_id().as_str()),
        ),
        ("task_id".into(), text(binding.task_id().as_str())),
        ("task_revision".into(), text(binding.task_revision())),
        (
            "task_spec_digest".into(),
            text(binding.task_spec_digest().as_str()),
        ),
    ])
}

fn decision_value(decision: AutonomyDecision) -> CanonicalValue {
    let (disposition, reason, model, verification) = match decision {
        AutonomyDecision::Proceed {
            model,
            verification,
            reason,
        } => (
            "PROCEED",
            reason.as_str(),
            text(model.as_str()),
            text(verification.as_str()),
        ),
        AutonomyDecision::AskUser { reason } => (
            "ASK_USER",
            reason.as_str(),
            CanonicalValue::Null,
            CanonicalValue::Null,
        ),
    };
    CanonicalValue::Object(vec![
        ("disposition".into(), text(disposition)),
        ("reason".into(), text(reason)),
        ("model".into(), model),
        ("verification".into(), verification),
    ])
}

fn canonical_digest(
    domain: &str,
    value: &CanonicalValue,
) -> Result<ContentDigest, AutonomyContractError> {
    let canonical = canonical_json(value)?;
    let mut frame = b"lattice-hash-1\0".to_vec();
    for field in [
        b"sha256".as_slice(),
        b"lattice-cjson-1",
        domain.as_bytes(),
        AUTONOMY_HASH_VERSION.as_bytes(),
    ] {
        let length =
            u16::try_from(field.len()).map_err(|_| AutonomyContractError::Canonicalization)?;
        frame.extend_from_slice(&length.to_be_bytes());
        frame.extend_from_slice(field);
    }
    let length =
        u64::try_from(canonical.len()).map_err(|_| AutonomyContractError::Canonicalization)?;
    frame.extend_from_slice(&length.to_be_bytes());
    frame.extend_from_slice(canonical.as_bytes());
    Ok(sha256_content_digest(&frame))
}

fn text(value: impl Into<String>) -> CanonicalValue {
    CanonicalValue::String(value.into())
}

fn canonical_json(value: &CanonicalValue) -> Result<String, AutonomyContractError> {
    fn write_value(
        value: &CanonicalValue,
        output: &mut String,
    ) -> Result<(), AutonomyContractError> {
        match value {
            CanonicalValue::Null => output.push_str("null"),
            CanonicalValue::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
            CanonicalValue::String(value) => write_string(value, output),
            CanonicalValue::Object(entries) => {
                let mut entries = entries.iter().collect::<Vec<_>>();
                entries.sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
                if entries.windows(2).any(|pair| pair[0].0 == pair[1].0) {
                    return Err(AutonomyContractError::Canonicalization);
                }
                output.push('{');
                for (index, (key, value)) in entries.into_iter().enumerate() {
                    if index > 0 {
                        output.push(',');
                    }
                    write_string(key, output);
                    output.push(':');
                    write_value(value, output)?;
                }
                output.push('}');
            }
        }
        Ok(())
    }
    fn write_string(value: &str, output: &mut String) {
        output.push('"');
        for character in value.chars() {
            match character {
                '"' => output.push_str("\\\""),
                '\\' => output.push_str("\\\\"),
                '\u{8}' => output.push_str("\\b"),
                '\t' => output.push_str("\\t"),
                '\n' => output.push_str("\\n"),
                '\u{c}' => output.push_str("\\f"),
                '\r' => output.push_str("\\r"),
                control if control <= '\u{1f}' => {
                    use std::fmt::Write as _;
                    write!(output, "\\u{:04x}", u32::from(control)).expect("string write");
                }
                other => output.push(other),
            }
        }
        output.push('"');
    }
    let mut output = String::new();
    write_value(value, &mut output)?;
    Ok(output)
}

fn is_zero_digest(value: &ContentDigest) -> bool {
    value.as_str().bytes().all(|byte| byte == b'0')
}

impl AutonomyIntentVersion {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::V1 => "1.0",
        }
    }
}

impl TaskKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Feature => "FEATURE",
            Self::BugFix => "BUG_FIX",
            Self::Configuration => "CONFIGURATION",
            Self::Research => "RESEARCH",
        }
    }
}

impl ModelRecommendation {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::GovernedCodexWriter => "GOVERNED_CODEX_WRITER",
            Self::NoModel => "NO_MODEL",
        }
    }
}

impl VerificationRecommendation {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FocusedChecks => "FOCUSED_CHECKS",
            Self::BuildAndFocusedChecks => "BUILD_AND_FOCUSED_CHECKS",
            Self::ReadOnlyEvidence => "READ_ONLY_EVIDENCE",
        }
    }
}

impl AutonomyDecisionReason {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RoutineAuthorized => "ROUTINE_AUTHORIZED",
            Self::NewUserDecision => "NEW_USER_DECISION",
            Self::NewAuthority => "NEW_AUTHORITY",
            Self::HighRiskOrIrreversible => "HIGH_RISK_OR_IRREVERSIBLE",
        }
    }
}
