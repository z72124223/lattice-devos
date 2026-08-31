//! Pure Task Ledger V2 semantic owner, append planner, and non-durable fake.

mod foreman;
mod task_runtime;

pub use foreman::*;
pub use task_runtime::*;

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use lattice_cjson::{
    CanonicalError, CanonicalValue, HashDomain, canonical_sha256, canonicalize, normalize_nfc,
};
use lattice_contracts::{
    CONTRACT_VERSION, ContentDigest, ContractError, ResourceCounters, ResourceRequest,
    RuntimeAdmissionMode, RuntimeKind, STORE_PROJECT_SNAPSHOT_ID_MAX_BYTES,
    TASK_INGRESS_CLIENT_REQUEST_ID_MAX_BYTES, TASK_LEDGER_PRODUCER_ID,
    TASK_LEDGER_PRODUCER_VERSION, TaskLedgerResourceHead, TaskLedgerResourceReceipt,
    TaskLedgerStreamHead, TaskLedgerStreamIdentity, TaskLedgerSubjectKind,
    WriterLeaseAuthorityHead, WriterLeaseStatus, task_ingress_text_contains_recognized_secret,
    valid_task_ingress_client_request_id,
};
use time::format_description::well_known::Rfc3339;
use time::{OffsetDateTime, UtcOffset};

/// Task Ledger event and hash-subject schema version.
pub const LEDGER_SCHEMA_VERSION: &str = "2.0";
/// Immutable Task Ledger outbox-admission schema version.
pub const OUTBOX_ADMISSION_SCHEMA_VERSION: &str = "1.0";
/// Complete Task Ledger checkpoint schema version.
pub const LEDGER_CHECKPOINT_SCHEMA_VERSION: &str = "1.0";
/// Per-command persistence record-set schema version.
pub const LEDGER_RECORD_SET_SCHEMA_VERSION: &str = "1.0";
/// Authoritative general-task submission-envelope schema.
pub const TASK_SUBMISSION_ENVELOPE_SCHEMA: &str = "lattice.task-ledger.task-submission/1.0";
/// Authoritative cross-profile task-ingress idempotency-claim schema.
pub const TASK_INGRESS_CLAIM_SCHEMA: &str = "lattice.task-ledger.task-ingress-claim/1.0";
/// Immutable external-result adoption evidence schema.
pub const EXTERNAL_VERIFIED_RESULT_ADOPTION_SCHEMA: &str =
    "lattice.task-ledger.external-verified-result-adoption/1.0";

const TASK_SUBMISSION_HASH_VERSION: &str = "1.0";
const TASK_SUBMISSION_REF_DOMAIN: &str = "lattice.task-ledger.task-submission-ref";
const TASK_SUBMISSION_ENVELOPE_DOMAIN: &str = "lattice.task-ledger.task-submission-envelope";
const TASK_INGRESS_REQUEST_DOMAIN: &str = "lattice.task-ledger.task-ingress-request";
const MAX_SUBMISSION_INGRESS_ID_BYTES: usize = 64;
const MAX_SUBMISSION_CLIENT_REQUEST_ID_BYTES: usize = TASK_INGRESS_CLIENT_REQUEST_ID_MAX_BYTES;
const MAX_SUBMISSION_OBJECTIVE_CHARS: usize = 512;
const MAX_SUBMISSION_OBJECTIVE_BYTES: usize = 2_048;
const MAX_SUBMISSION_PROJECT_DISPLAY_NAME_CHARS: usize = 64;
const MAX_SUBMISSION_PROJECT_DISPLAY_NAME_BYTES: usize = 256;
const MAX_SUBMISSION_PROJECT_ID_BYTES: usize = 64;
const MAX_EXTERNAL_RESULT_APPROVAL_REFS: usize = 8;
const EXTERNAL_RESULT_ADOPTION_ACTION: &str = "ADOPT_VERIFIED_RESULT_V1";
const EXTERNAL_RESULT_ADOPTION_REASON: &str = "EXTERNAL_VERIFIED_RESULT_ADOPTED";
const GENERAL_TASK_INTAKE_CORRELATION_ID: &str = "general-task-intake-v1";
/// Maximum byte length of a Task Ledger project snapshot identifier.
///
/// This covers the Project Registry's maximum canonical authority snapshot:
/// a 64-byte project ID, `:registry:`, a 20-digit `u64` revision, one colon,
/// and a 64-byte SHA-256 digest.
pub const TASK_LEDGER_PROJECT_SNAPSHOT_ID_MAX_BYTES: usize = STORE_PROJECT_SNAPSHOT_ID_MAX_BYTES;

const ZERO_DIGEST_TEXT: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const MAX_DIAGNOSTIC_DEPTH: usize = 16;
const MAX_DIAGNOSTIC_NODES: usize = 1_024;
const MAX_DIAGNOSTIC_BYTES: usize = 16 * 1_024;

/// Failure at the pure Task Ledger contract boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LedgerError {
    /// One semantic identifier violates the bounded ASCII contract.
    InvalidIdentifier {
        /// Stable field name.
        field: &'static str,
    },
    /// Caller text is not already Unicode NFC.
    NonCanonicalText {
        /// Stable field name.
        field: &'static str,
    },
    /// A timestamp is not canonical UTC RFC 3339.
    InvalidTimestamp,
    /// A diagnostic value is structurally invalid.
    InvalidDiagnostic,
    /// A diagnostic exceeds depth, node, or canonical-byte limits.
    DiagnosticLimitExceeded,
    /// One authoritative submission-envelope field is malformed.
    InvalidSubmissionEnvelope {
        /// Stable field name; never includes submitted content.
        field: &'static str,
    },
    /// One authoritative submission-envelope field exceeds its fixed bound.
    SubmissionEnvelopeLimitExceeded {
        /// Stable field name; never includes submitted content.
        field: &'static str,
    },
    /// Submitted human text matched a closed secret-bearing shape.
    SubmissionSecretRejected,
    /// A retained submission envelope uses an unknown schema version.
    UnknownSubmissionEnvelopeVersion,
    /// A retained submission envelope disagrees with its canonical digest or reference.
    SubmissionEnvelopeMismatch,
    /// One externally verified-result adoption field is malformed.
    InvalidExternalVerifiedResultAdoption {
        /// Stable field name; never includes submitted content.
        field: &'static str,
    },
    /// An externally verified-result adoption exceeded a fixed collection bound.
    ExternalVerifiedResultAdoptionLimitExceeded {
        /// Stable field name; never includes submitted content.
        field: &'static str,
    },
    /// A retained external verified-result adoption diverged from its digest or event binding.
    ExternalVerifiedResultAdoptionMismatch,
    /// One authoritative task-ingress claim field is malformed.
    InvalidTaskIngressClaim {
        /// Stable field name; never includes submitted content.
        field: &'static str,
    },
    /// A retained task-ingress claim uses an unknown schema version.
    UnknownTaskIngressClaimVersion,
    /// A retained task-ingress claim differs from the expected semantic request.
    TaskIngressClaimMismatch,
    /// A supplied or reconstructed stream head is invalid.
    InvalidStreamHead,
    /// An event kind/resource snapshot combination is invalid.
    InvalidResourceSnapshot,
    /// An autonomy receipt event violates its fixed shape or exactly-once order.
    InvalidAutonomyReceipt,
    /// The supplied autonomy recommendation differs from Task Ledger policy.
    AutonomyRecommendationMismatch,
    /// A foreman event, child record, payload, linkage, or fixed stream is invalid.
    InvalidForemanSnapshot,
    /// A persisted foreman child-record or payload schema is unknown.
    UnknownForemanSnapshotVersion,
    /// A new foreman generation was not exactly the prior generation plus one.
    ForemanGenerationRollback,
    /// A managed-task lineage or runtime child record is malformed or cross-bound.
    InvalidTaskRuntimeRecord,
    /// A managed-task lineage or runtime child record schema is unknown.
    UnknownTaskRuntimeRecordVersion,
    /// An immutable managed-task lineage was reused with changed semantics.
    TaskRuntimeSubstitution,
    /// A new worker attempt number or Writer fence was not strictly monotonic.
    WorkerAttemptNotMonotonic,
    /// A repair attempt was claimed before its predecessor had an exact terminal.
    WorkerAttemptBeforeTerminal,
    /// A retained worker attempt changed its immutable provider thread or turn.
    WorkerIdentityDrift,
    /// A generic caller selected a reserved or unknown Task-created profile.
    UnknownTaskCreatedProfile,
    /// A pre-specification intake stream was asked to append executable work.
    GeneralTaskIntakeCreateOnly,
    /// Cumulative resource counters moved backwards.
    ResourceCounterRegression,
    /// A command ID was reused in one stream with another request digest.
    CommandIdReuse,
    /// A persisted event schema version is unknown.
    UnknownEventVersion,
    /// A persisted command-request schema version is unknown.
    UnknownRequestVersion,
    /// A persisted command-receipt schema version is unknown.
    UnknownReceiptVersion,
    /// A persisted event kind is outside the closed schema.
    UnknownEventKind,
    /// A persisted event outcome is outside the closed schema.
    UnknownEventOutcome,
    /// A persisted command receipt outcome/reason is outside the closed schema.
    UnknownReceiptOutcome,
    /// Event sequence is missing, duplicated, or non-contiguous.
    CorruptSequence,
    /// Event predecessor does not match the verified prior event.
    CorruptPredecessor,
    /// Event digest does not match its semantic subject.
    CorruptEventHash,
    /// An event request digest does not match the reconstructed request.
    RequestBindingMismatch,
    /// A command receipt does not match its event or digest.
    ReceiptBindingMismatch,
    /// Replayed state disagrees with the claimed stream head.
    HeadMismatch,
    /// Replayed resource counters disagree with the claimed projection.
    ResourceProjectionMismatch,
    /// An appended event has no matching command receipt.
    OrphanReceipt,
    /// A persisted outbox-admission schema version is unknown.
    UnknownOutboxVersion,
    /// A persisted outbox state is outside the closed schema.
    UnknownOutboxState,
    /// An outbox admission does not match its authoritative event.
    OutboxBindingMismatch,
    /// A complete stream checkpoint or append-plan base does not match.
    CheckpointMismatch,
    /// Canonical-byte mechanics failed.
    Canonical(CanonicalError),
    /// A shared immutable contract rejected a value.
    Contract(ContractError),
}

impl LedgerError {
    /// Returns a stable machine-facing failure code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidIdentifier { .. } => "LEDGER_INVALID_IDENTIFIER",
            Self::NonCanonicalText { .. } => "LEDGER_NON_CANONICAL_TEXT",
            Self::InvalidTimestamp => "LEDGER_INVALID_TIMESTAMP",
            Self::InvalidDiagnostic => "LEDGER_INVALID_DIAGNOSTIC",
            Self::DiagnosticLimitExceeded => "LEDGER_DIAGNOSTIC_LIMIT_EXCEEDED",
            Self::InvalidSubmissionEnvelope { .. } => "LEDGER_INVALID_SUBMISSION_ENVELOPE",
            Self::SubmissionEnvelopeLimitExceeded { .. } => {
                "LEDGER_SUBMISSION_ENVELOPE_LIMIT_EXCEEDED"
            }
            Self::SubmissionSecretRejected => "LEDGER_SUBMISSION_SECRET_REJECTED",
            Self::UnknownSubmissionEnvelopeVersion => "LEDGER_UNKNOWN_SUBMISSION_ENVELOPE_VERSION",
            Self::SubmissionEnvelopeMismatch => "LEDGER_SUBMISSION_ENVELOPE_MISMATCH",
            Self::InvalidExternalVerifiedResultAdoption { .. } => {
                "LEDGER_EXTERNAL_VERIFIED_RESULT_ADOPTION_INVALID"
            }
            Self::ExternalVerifiedResultAdoptionLimitExceeded { .. } => {
                "LEDGER_EXTERNAL_VERIFIED_RESULT_ADOPTION_LIMIT_EXCEEDED"
            }
            Self::ExternalVerifiedResultAdoptionMismatch => {
                "LEDGER_EXTERNAL_VERIFIED_RESULT_ADOPTION_MISMATCH"
            }
            Self::InvalidTaskIngressClaim { .. } => "LEDGER_INVALID_TASK_INGRESS_CLAIM",
            Self::UnknownTaskIngressClaimVersion => "LEDGER_UNKNOWN_TASK_INGRESS_CLAIM_VERSION",
            Self::TaskIngressClaimMismatch => "LEDGER_TASK_INGRESS_CLAIM_MISMATCH",
            Self::InvalidStreamHead => "LEDGER_INVALID_HEAD",
            Self::InvalidResourceSnapshot => "LEDGER_INVALID_RESOURCE_SNAPSHOT",
            Self::InvalidAutonomyReceipt => "LEDGER_INVALID_AUTONOMY_RECEIPT",
            Self::AutonomyRecommendationMismatch => "LEDGER_AUTONOMY_RECOMMENDATION_MISMATCH",
            Self::InvalidForemanSnapshot => "LEDGER_INVALID_FOREMAN_SNAPSHOT",
            Self::UnknownForemanSnapshotVersion => "LEDGER_UNKNOWN_FOREMAN_SNAPSHOT_VERSION",
            Self::ForemanGenerationRollback => "LEDGER_FOREMAN_GENERATION_ROLLBACK",
            Self::InvalidTaskRuntimeRecord => "LEDGER_INVALID_TASK_RUNTIME_RECORD",
            Self::UnknownTaskRuntimeRecordVersion => "LEDGER_UNKNOWN_TASK_RUNTIME_RECORD_VERSION",
            Self::TaskRuntimeSubstitution => "LEDGER_TASK_RUNTIME_SUBSTITUTION",
            Self::WorkerAttemptNotMonotonic => "LEDGER_WORKER_ATTEMPT_NOT_MONOTONIC",
            Self::WorkerAttemptBeforeTerminal => "LEDGER_WORKER_ATTEMPT_BEFORE_TERMINAL",
            Self::WorkerIdentityDrift => "LEDGER_WORKER_IDENTITY_DRIFT",
            Self::UnknownTaskCreatedProfile => "LEDGER_UNKNOWN_TASK_CREATED_PROFILE",
            Self::GeneralTaskIntakeCreateOnly => "LEDGER_GENERAL_TASK_INTAKE_CREATE_ONLY",
            Self::ResourceCounterRegression => "LEDGER_RESOURCE_COUNTER_REGRESSION",
            Self::CommandIdReuse => "LEDGER_COMMAND_ID_REUSE",
            Self::UnknownEventVersion => "LEDGER_UNKNOWN_EVENT_VERSION",
            Self::UnknownRequestVersion => "LEDGER_UNKNOWN_REQUEST_VERSION",
            Self::UnknownReceiptVersion => "LEDGER_UNKNOWN_RECEIPT_VERSION",
            Self::UnknownEventKind => "LEDGER_UNKNOWN_EVENT_KIND",
            Self::UnknownEventOutcome => "LEDGER_UNKNOWN_EVENT_OUTCOME",
            Self::UnknownReceiptOutcome => "LEDGER_UNKNOWN_RECEIPT_OUTCOME",
            Self::CorruptSequence => "LEDGER_CORRUPT_SEQUENCE",
            Self::CorruptPredecessor => "LEDGER_CORRUPT_PREDECESSOR",
            Self::CorruptEventHash => "LEDGER_EVENT_HASH_MISMATCH",
            Self::RequestBindingMismatch => "LEDGER_REQUEST_BINDING_MISMATCH",
            Self::ReceiptBindingMismatch => "LEDGER_RECEIPT_BINDING_MISMATCH",
            Self::HeadMismatch => "LEDGER_HEAD_MISMATCH",
            Self::ResourceProjectionMismatch => "LEDGER_RESOURCE_PROJECTION_MISMATCH",
            Self::OrphanReceipt => "LEDGER_ORPHAN_RECEIPT",
            Self::UnknownOutboxVersion => "LEDGER_UNKNOWN_OUTBOX_VERSION",
            Self::UnknownOutboxState => "LEDGER_UNKNOWN_OUTBOX_STATE",
            Self::OutboxBindingMismatch => "LEDGER_OUTBOX_BINDING_MISMATCH",
            Self::CheckpointMismatch => "LEDGER_CHECKPOINT_MISMATCH",
            Self::Canonical(_) => "LEDGER_CANONICAL_ENCODING_FAILED",
            Self::Contract(_) => "LEDGER_CONTRACT_INVALID",
        }
    }
}

impl fmt::Display for LedgerError {
    #[allow(clippy::too_many_lines)]
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIdentifier { field } => write!(formatter, "invalid Ledger {field}"),
            Self::NonCanonicalText { field } => {
                write!(formatter, "{field} must already be Unicode NFC")
            }
            Self::InvalidTimestamp => formatter.write_str("invalid canonical UTC timestamp"),
            Self::InvalidDiagnostic => formatter.write_str("invalid sanitized diagnostic"),
            Self::DiagnosticLimitExceeded => {
                formatter.write_str("diagnostic exceeds a bounded Ledger limit")
            }
            Self::InvalidSubmissionEnvelope { field } => {
                write!(formatter, "invalid submission-envelope {field}")
            }
            Self::SubmissionEnvelopeLimitExceeded { field } => {
                write!(formatter, "submission-envelope {field} exceeds its bound")
            }
            Self::SubmissionSecretRejected => {
                formatter.write_str("submission envelope contains prohibited secret material")
            }
            Self::UnknownSubmissionEnvelopeVersion => {
                formatter.write_str("unknown submission-envelope version")
            }
            Self::SubmissionEnvelopeMismatch => {
                formatter.write_str("submission envelope does not match its retained identity")
            }
            Self::InvalidExternalVerifiedResultAdoption { field } => {
                write!(
                    formatter,
                    "invalid external verified-result adoption {field}"
                )
            }
            Self::ExternalVerifiedResultAdoptionLimitExceeded { field } => {
                write!(
                    formatter,
                    "external verified-result adoption {field} exceeds its bound"
                )
            }
            Self::ExternalVerifiedResultAdoptionMismatch => {
                formatter.write_str("external verified-result adoption binding mismatch")
            }
            Self::InvalidTaskIngressClaim { field } => {
                write!(formatter, "invalid task-ingress claim {field}")
            }
            Self::UnknownTaskIngressClaimVersion => {
                formatter.write_str("unknown task-ingress claim version")
            }
            Self::TaskIngressClaimMismatch => {
                formatter.write_str("task-ingress claim does not match the expected request")
            }
            Self::InvalidStreamHead => formatter.write_str("invalid full stream head"),
            Self::InvalidResourceSnapshot => {
                formatter.write_str("invalid event/resource snapshot combination")
            }
            Self::InvalidAutonomyReceipt => {
                formatter.write_str("invalid autonomy receipt event shape or order")
            }
            Self::AutonomyRecommendationMismatch => {
                formatter.write_str("autonomy recommendation does not match Task Ledger policy")
            }
            Self::InvalidForemanSnapshot => {
                formatter.write_str("invalid foreman snapshot event or child record")
            }
            Self::UnknownForemanSnapshotVersion => {
                formatter.write_str("unknown foreman snapshot record or payload schema")
            }
            Self::ForemanGenerationRollback => {
                formatter.write_str("foreman generation was not exact-next")
            }
            Self::InvalidTaskRuntimeRecord => {
                formatter.write_str("invalid managed-task lineage or runtime child record")
            }
            Self::UnknownTaskRuntimeRecordVersion => {
                formatter.write_str("unknown managed-task runtime record version")
            }
            Self::TaskRuntimeSubstitution => {
                formatter.write_str("managed-task lineage was reused with changed semantics")
            }
            Self::WorkerAttemptNotMonotonic => {
                formatter.write_str("worker attempt number or Writer fence was not monotonic")
            }
            Self::WorkerAttemptBeforeTerminal => {
                formatter.write_str("worker retry was claimed before exact predecessor terminal")
            }
            Self::WorkerIdentityDrift => {
                formatter.write_str("worker provider thread or turn identity changed")
            }
            Self::UnknownTaskCreatedProfile => {
                formatter.write_str("unknown or caller-selected Task-created profile")
            }
            Self::GeneralTaskIntakeCreateOnly => {
                formatter.write_str("general-task intake streams are create-only")
            }
            Self::ResourceCounterRegression => {
                formatter.write_str("cumulative resource counter regressed")
            }
            Self::CommandIdReuse => {
                formatter.write_str("command_id was reused for another request")
            }
            Self::UnknownEventVersion => formatter.write_str("unknown Ledger event version"),
            Self::UnknownRequestVersion => formatter.write_str("unknown Ledger request version"),
            Self::UnknownReceiptVersion => formatter.write_str("unknown Ledger receipt version"),
            Self::UnknownEventKind => formatter.write_str("unknown Ledger event kind"),
            Self::UnknownEventOutcome => formatter.write_str("unknown Ledger event outcome"),
            Self::UnknownReceiptOutcome => formatter.write_str("unknown Ledger receipt outcome"),
            Self::CorruptSequence => formatter.write_str("corrupt Ledger event sequence"),
            Self::CorruptPredecessor => formatter.write_str("corrupt Ledger predecessor"),
            Self::CorruptEventHash => formatter.write_str("corrupt Ledger event digest"),
            Self::RequestBindingMismatch => formatter.write_str("event request binding mismatch"),
            Self::ReceiptBindingMismatch => formatter.write_str("command receipt binding mismatch"),
            Self::HeadMismatch => formatter.write_str("replayed Ledger head mismatch"),
            Self::ResourceProjectionMismatch => {
                formatter.write_str("replayed resource projection mismatch")
            }
            Self::OrphanReceipt => formatter.write_str("event has no matching command receipt"),
            Self::UnknownOutboxVersion => {
                formatter.write_str("unknown Ledger outbox-admission version")
            }
            Self::UnknownOutboxState => formatter.write_str("unknown Ledger outbox state"),
            Self::OutboxBindingMismatch => formatter.write_str("outbox admission binding mismatch"),
            Self::CheckpointMismatch => formatter.write_str("Ledger checkpoint mismatch"),
            Self::Canonical(error) => write!(formatter, "canonical encoding failed: {error}"),
            Self::Contract(error) => write!(formatter, "shared contract rejected value: {error}"),
        }
    }
}

impl Error for LedgerError {}

impl From<CanonicalError> for LedgerError {
    fn from(value: CanonicalError) -> Self {
        Self::Canonical(value)
    }
}

impl From<ContractError> for LedgerError {
    fn from(value: ContractError) -> Self {
        Self::Contract(value)
    }
}

macro_rules! ledger_identifier {
    ($name:ident, $field:literal) => {
        #[doc = concat!("Validated Task Ledger `", $field, "`.")]
        #[derive(Clone, Debug, Eq, Hash, PartialEq)]
        pub struct $name(String);

        impl $name {
            #[doc = concat!("Constructs one exact `", $field, "`.")]
            ///
            /// # Errors
            ///
            /// Rejects empty, padded, non-ASCII, NUL-bearing, or oversized
            /// values.
            pub fn new(value: impl Into<String>) -> Result<Self, LedgerError> {
                let value = value.into();
                if !valid_identifier(&value) || recognized_secret_text(&value) {
                    return Err(LedgerError::InvalidIdentifier { field: $field });
                }
                Ok(Self(value))
            }

            #[doc = concat!("Returns the exact `", $field, "`.")]
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

ledger_identifier!(CommandId, "command_id");
ledger_identifier!(CorrelationId, "correlation_id");
ledger_identifier!(ActorId, "actor_id");
ledger_identifier!(ActionId, "action");
ledger_identifier!(ReasonCode, "reason_code");
ledger_identifier!(EffectClaimId, "effect_claim_id");

/// Closed Task Ledger event kinds supported by schema 2.0.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LedgerEventKind {
    /// The immutable task subject was accepted.
    TaskCreated,
    /// One canonical autonomy decision receipt was recorded.
    AutonomyReceiptRecorded,
    /// One typed foreman coordination snapshot was recorded.
    ForemanSnapshotRecorded,
    /// A separately validated Task Domain transition was recorded.
    StateTransition,
    /// A pure Policy decision was recorded.
    PolicyDecision,
    /// Current resource counters were recorded.
    ResourceSnapshot,
    /// A future external effect intent was recorded.
    EffectIntent,
    /// A future external effect outcome was recorded.
    EffectOutcome,
    /// An immutable evidence digest was recorded.
    EvidenceRecorded,
    /// A server-verified externally completed result terminalized a create-only intake.
    ExternalVerifiedResultAdopted,
}

impl LedgerEventKind {
    /// Returns the stable wire value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TaskCreated => "TASK_CREATED",
            Self::AutonomyReceiptRecorded => "AUTONOMY_RECEIPT_RECORDED",
            Self::ForemanSnapshotRecorded => "FOREMAN_SNAPSHOT_RECORDED",
            Self::StateTransition => "STATE_TRANSITION",
            Self::PolicyDecision => "POLICY_DECISION",
            Self::ResourceSnapshot => "RESOURCE_SNAPSHOT",
            Self::EffectIntent => "EFFECT_INTENT",
            Self::EffectOutcome => "EFFECT_OUTCOME",
            Self::EvidenceRecorded => "EVIDENCE_RECORDED",
            Self::ExternalVerifiedResultAdopted => "EXTERNAL_VERIFIED_RESULT_ADOPTED",
        }
    }

    /// Parses one closed wire value.
    ///
    /// # Errors
    ///
    /// Rejects an unknown event kind.
    pub fn parse(value: &str) -> Result<Self, LedgerError> {
        match value {
            "TASK_CREATED" => Ok(Self::TaskCreated),
            "AUTONOMY_RECEIPT_RECORDED" => Ok(Self::AutonomyReceiptRecorded),
            "FOREMAN_SNAPSHOT_RECORDED" => Ok(Self::ForemanSnapshotRecorded),
            "STATE_TRANSITION" => Ok(Self::StateTransition),
            "POLICY_DECISION" => Ok(Self::PolicyDecision),
            "RESOURCE_SNAPSHOT" => Ok(Self::ResourceSnapshot),
            "EFFECT_INTENT" => Ok(Self::EffectIntent),
            "EFFECT_OUTCOME" => Ok(Self::EffectOutcome),
            "EVIDENCE_RECORDED" => Ok(Self::EvidenceRecorded),
            "EXTERNAL_VERIFIED_RESULT_ADOPTED" => Ok(Self::ExternalVerifiedResultAdopted),
            _ => Err(LedgerError::UnknownEventKind),
        }
    }
}

/// Closed task-control profiles carried by `TASK_CREATED.action`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskCreatedProfile {
    /// Frozen pre-TASK-050 profile; new admission cannot mint this marker.
    HistoricalAutonomyOptionalV1,
    /// Current profile requiring an exact sequence-two autonomy receipt.
    AutonomyReceiptRequiredV1,
    /// General natural-language intake retained in Draft without classifying
    /// risk, authority, model, or execution intent.
    GeneralTaskIntakeV1,
    /// Executable Task-Spec successor created only by the server-owned managed
    /// foreman after an exact general-intake promotion.
    ManagedGeneralTaskV1,
}

impl TaskCreatedProfile {
    /// Returns the exact hash-bound action marker.
    #[must_use]
    pub const fn action(self) -> &'static str {
        match self {
            Self::HistoricalAutonomyOptionalV1 => "CONTROLLED_CODEX_CANARY",
            Self::AutonomyReceiptRequiredV1 => "CONTROLLED_CODEX_CANARY_AUTONOMY_V1",
            Self::GeneralTaskIntakeV1 => "GENERAL_TASK_INTAKE_V1",
            Self::ManagedGeneralTaskV1 => "MANAGED_GENERAL_TASK_V1",
        }
    }

    /// Returns whether progress requires the exact sequence-two autonomy receipt.
    #[must_use]
    pub const fn requires_autonomy_receipt(self) -> bool {
        matches!(
            self,
            Self::AutonomyReceiptRequiredV1 | Self::ManagedGeneralTaskV1
        )
    }
}

/// Canonical authoritative intake envelope for one general task.
///
/// The envelope records task data only. It grants no execution, filesystem,
/// process, payment, merge, deployment, or external-effect authority.
#[derive(Clone, Eq, PartialEq)]
pub struct TaskSubmissionEnvelope {
    ingress_id: String,
    client_request_id: String,
    objective: String,
    project_display_name: String,
    project_authority_receipt_digest: ContentDigest,
    identity: TaskLedgerStreamIdentity,
    stream_id: ContentDigest,
    task_ref: ContentDigest,
    envelope_digest: ContentDigest,
}

impl fmt::Debug for TaskSubmissionEnvelope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TaskSubmissionEnvelope")
            .field("schema_version", &TASK_SUBMISSION_ENVELOPE_SCHEMA)
            .field("ingress_id", &self.ingress_id)
            .field("client_request_id", &self.client_request_id)
            .field("objective", &"[REDACTED]")
            .field("project_display_name", &"[REDACTED]")
            .field(
                "project_authority_receipt_digest",
                &self.project_authority_receipt_digest,
            )
            .field("identity", &self.identity)
            .field("stream_id", &self.stream_id)
            .field("task_ref", &self.task_ref)
            .field("envelope_digest", &self.envelope_digest)
            .finish()
    }
}

impl TaskSubmissionEnvelope {
    /// Constructs and hashes one exact general-task intake envelope.
    ///
    /// # Errors
    ///
    /// Rejects non-canonical, blank, control-bearing, oversized, or recognized
    /// secret-bearing input and every malformed stream identity.
    pub fn new(
        ingress_id: impl Into<String>,
        client_request_id: impl Into<String>,
        objective: impl Into<String>,
        project_display_name: impl Into<String>,
        identity: TaskLedgerStreamIdentity,
        project_authority_receipt_digest: ContentDigest,
    ) -> Result<Self, LedgerError> {
        let ingress_id = ingress_id.into();
        let client_request_id = client_request_id.into();
        let objective = objective.into();
        let project_display_name = project_display_name.into();
        validate_submission_control_id(&ingress_id, "ingress_id", MAX_SUBMISSION_INGRESS_ID_BYTES)?;
        validate_submission_control_id(
            &client_request_id,
            "client_request_id",
            MAX_SUBMISSION_CLIENT_REQUEST_ID_BYTES,
        )?;
        validate_submission_human_text(
            &objective,
            "objective",
            MAX_SUBMISSION_OBJECTIVE_CHARS,
            MAX_SUBMISSION_OBJECTIVE_BYTES,
        )?;
        validate_submission_human_text(
            &project_display_name,
            "project_display_name",
            MAX_SUBMISSION_PROJECT_DISPLAY_NAME_CHARS,
            MAX_SUBMISSION_PROJECT_DISPLAY_NAME_BYTES,
        )?;
        validate_submission_project_id(identity.project_id().as_str())?;
        validate_submission_project_snapshot_id(identity.project_snapshot_id().as_str())?;
        if is_zero_digest(&project_authority_receipt_digest) {
            return Err(LedgerError::InvalidSubmissionEnvelope {
                field: "project_authority_receipt_digest",
            });
        }
        validate_stream_identity(&identity)?;
        if identity.subject_kind() != TaskLedgerSubjectKind::GeneralTaskIntake
            || identity.general_task_intake_digest().is_none()
            || identity.task_spec_digest().is_some()
            || identity.accounting_currency().is_some()
        {
            return Err(LedgerError::InvalidSubmissionEnvelope {
                field: "stream_identity",
            });
        }
        let stream_id = hash_value("lattice.task-ledger.stream-id", &identity_value(&identity))?;
        let content = task_submission_content_value(
            &ingress_id,
            &client_request_id,
            &objective,
            &project_display_name,
            &project_authority_receipt_digest,
            &identity,
            &stream_id,
        );
        let task_ref = hash_value_at_version(
            TASK_SUBMISSION_REF_DOMAIN,
            TASK_SUBMISSION_HASH_VERSION,
            &content,
        )?;
        let envelope_digest = hash_value_at_version(
            TASK_SUBMISSION_ENVELOPE_DOMAIN,
            TASK_SUBMISSION_HASH_VERSION,
            &task_submission_envelope_value(&content, &task_ref),
        )?;
        Ok(Self {
            ingress_id,
            client_request_id,
            objective,
            project_display_name,
            project_authority_receipt_digest,
            identity,
            stream_id,
            task_ref,
            envelope_digest,
        })
    }

    /// Returns the fixed envelope schema.
    #[must_use]
    pub const fn schema_version(&self) -> &'static str {
        TASK_SUBMISSION_ENVELOPE_SCHEMA
    }

    /// Returns the process-owned ingress identity.
    #[must_use]
    pub fn ingress_id(&self) -> &str {
        &self.ingress_id
    }

    /// Returns the caller's bounded idempotency key.
    #[must_use]
    pub fn client_request_id(&self) -> &str {
        &self.client_request_id
    }

    /// Returns the exact natural-language task objective.
    #[must_use]
    pub fn objective(&self) -> &str {
        &self.objective
    }

    /// Returns the exact registered project display name retained at intake.
    #[must_use]
    pub fn project_display_name(&self) -> &str {
        &self.project_display_name
    }

    /// Returns the formal Project Registry receipt bound at intake time.
    #[must_use]
    pub const fn project_authority_receipt_digest(&self) -> &ContentDigest {
        &self.project_authority_receipt_digest
    }

    /// Returns the complete formal Task Ledger stream identity.
    #[must_use]
    pub const fn identity(&self) -> &TaskLedgerStreamIdentity {
        &self.identity
    }

    /// Returns the canonical stream ID derived from the complete identity.
    #[must_use]
    pub const fn stream_id(&self) -> &ContentDigest {
        &self.stream_id
    }

    /// Returns the durable public task reference.
    #[must_use]
    pub const fn task_ref(&self) -> &ContentDigest {
        &self.task_ref
    }

    /// Returns the only Task-created action authorized for this envelope.
    #[must_use]
    pub const fn admission_action(&self) -> &'static str {
        TaskCreatedProfile::GeneralTaskIntakeV1.action()
    }

    /// Returns the authoritative digest bound into `TASK_CREATED.subject_digest`.
    #[must_use]
    pub const fn envelope_digest(&self) -> &ContentDigest {
        &self.envelope_digest
    }

    /// Exports an explicitly untrusted persistence representation.
    #[must_use]
    pub fn to_untrusted(&self) -> UntrustedTaskSubmissionEnvelope {
        UntrustedTaskSubmissionEnvelope {
            schema_version: TASK_SUBMISSION_ENVELOPE_SCHEMA.to_owned(),
            ingress_id: self.ingress_id.clone(),
            client_request_id: self.client_request_id.clone(),
            objective: self.objective.clone(),
            project_display_name: self.project_display_name.clone(),
            project_authority_receipt_digest: self.project_authority_receipt_digest.clone(),
            identity: self.identity.clone(),
            stream_id: self.stream_id.clone(),
            task_ref: self.task_ref.clone(),
            admission_action: self.admission_action().to_owned(),
            envelope_digest: self.envelope_digest.clone(),
        }
    }
}

/// Raw retained submission-envelope fields. Every field is untrusted until
/// verified by [`verify_untrusted_task_submission`].
#[derive(Clone, Eq, PartialEq)]
pub struct UntrustedTaskSubmissionEnvelope {
    /// Claimed envelope schema.
    pub schema_version: String,
    /// Claimed ingress identity.
    pub ingress_id: String,
    /// Claimed client idempotency key.
    pub client_request_id: String,
    /// Claimed exact objective.
    pub objective: String,
    /// Claimed registered project display name.
    pub project_display_name: String,
    /// Claimed formal Project Registry authority-receipt digest.
    pub project_authority_receipt_digest: ContentDigest,
    /// Claimed complete formal stream identity.
    pub identity: TaskLedgerStreamIdentity,
    /// Claimed canonical stream ID.
    pub stream_id: ContentDigest,
    /// Claimed public task reference.
    pub task_ref: ContentDigest,
    /// Claimed Task-created profile action.
    pub admission_action: String,
    /// Claimed canonical envelope digest.
    pub envelope_digest: ContentDigest,
}

/// Reconstructs and verifies every retained submission-envelope field.
///
/// # Errors
///
/// Rejects unknown versions, invalid input, or any changed digest, reference,
/// action, stream ID, or formal identity binding.
pub fn verify_untrusted_task_submission(
    raw: &UntrustedTaskSubmissionEnvelope,
) -> Result<TaskSubmissionEnvelope, LedgerError> {
    if raw.schema_version != TASK_SUBMISSION_ENVELOPE_SCHEMA {
        return Err(LedgerError::UnknownSubmissionEnvelopeVersion);
    }
    let verified = TaskSubmissionEnvelope::new(
        raw.ingress_id.clone(),
        raw.client_request_id.clone(),
        raw.objective.clone(),
        raw.project_display_name.clone(),
        raw.identity.clone(),
        raw.project_authority_receipt_digest.clone(),
    )?;
    if raw.stream_id != verified.stream_id
        || raw.task_ref != verified.task_ref
        || raw.admission_action != verified.admission_action()
        || raw.envelope_digest != verified.envelope_digest
    {
        return Err(LedgerError::SubmissionEnvelopeMismatch);
    }
    Ok(verified)
}

/// Immutable, digest-bound proof bundle for a server-verified external result.
///
/// This is not an execution, approval, or deployment command. It binds only
/// opaque receipt descriptors and Git identities; the repository adapter must
/// independently resolve and verify every referenced receipt before this type
/// can be committed as a terminal Ledger event.
#[derive(Clone, Eq, PartialEq)]
pub struct ExternalVerifiedResultAdoption {
    task_ref: ContentDigest,
    client_request_id: String,
    expected_ledger_head_digest: ContentDigest,
    source_sha: String,
    target_sha: String,
    push_merge_receipt_ref: String,
    deployment_receipt_ref: String,
    deployment_artifact_ref: String,
    independent_acceptance_ref: String,
    protected_action_approval_refs: Vec<String>,
    result_digest: ContentDigest,
}

impl fmt::Debug for ExternalVerifiedResultAdoption {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExternalVerifiedResultAdoption")
            .field("schema", &EXTERNAL_VERIFIED_RESULT_ADOPTION_SCHEMA)
            .field("task_ref", &self.task_ref)
            .field("client_request_id", &self.client_request_id)
            .field(
                "expected_ledger_head_digest",
                &self.expected_ledger_head_digest,
            )
            .field("source_sha", &self.source_sha)
            .field("target_sha", &self.target_sha)
            .field("receipt_refs", &"[DIGEST_BOUND]")
            .field(
                "approval_ref_count",
                &self.protected_action_approval_refs.len(),
            )
            .field("result_digest", &self.result_digest)
            .finish()
    }
}

impl ExternalVerifiedResultAdoption {
    /// Validates and canonically binds the complete external-result receipt set.
    ///
    /// # Errors
    ///
    /// Rejects malformed or secret-shaped identifiers, non-lowercase Git
    /// commits, zero digests, empty/duplicate approval references, and every
    /// mismatch between the caller's expected Ledger head and a later command.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        task_ref: ContentDigest,
        client_request_id: impl Into<String>,
        expected_ledger_head_digest: ContentDigest,
        source_sha: impl Into<String>,
        target_sha: impl Into<String>,
        push_merge_receipt_ref: impl Into<String>,
        deployment_receipt_ref: impl Into<String>,
        deployment_artifact_ref: impl Into<String>,
        independent_acceptance_ref: impl Into<String>,
        protected_action_approval_refs: Vec<String>,
    ) -> Result<Self, LedgerError> {
        let client_request_id = client_request_id.into();
        let source_sha = source_sha.into();
        let target_sha = target_sha.into();
        let push_merge_receipt_ref = push_merge_receipt_ref.into();
        let deployment_receipt_ref = deployment_receipt_ref.into();
        let deployment_artifact_ref = deployment_artifact_ref.into();
        let independent_acceptance_ref = independent_acceptance_ref.into();
        if is_zero_digest(&task_ref) {
            return Err(LedgerError::InvalidExternalVerifiedResultAdoption { field: "task_ref" });
        }
        if !valid_task_ingress_client_request_id(&client_request_id) {
            return Err(LedgerError::InvalidExternalVerifiedResultAdoption {
                field: "client_request_id",
            });
        }
        if is_zero_digest(&expected_ledger_head_digest) {
            return Err(LedgerError::InvalidExternalVerifiedResultAdoption {
                field: "expected_ledger_head_digest",
            });
        }
        if !valid_git_commit(&source_sha)
            || !valid_git_commit(&target_sha)
            || source_sha == target_sha
        {
            return Err(LedgerError::InvalidExternalVerifiedResultAdoption {
                field: "git_commit",
            });
        }
        for (field, value) in [
            ("push_merge_receipt_ref", &push_merge_receipt_ref),
            ("deployment_receipt_ref", &deployment_receipt_ref),
            ("deployment_artifact_ref", &deployment_artifact_ref),
            ("independent_acceptance_ref", &independent_acceptance_ref),
        ] {
            if !valid_evidence_reference(value) {
                return Err(LedgerError::InvalidExternalVerifiedResultAdoption { field });
            }
        }
        if !(1..=MAX_EXTERNAL_RESULT_APPROVAL_REFS).contains(&protected_action_approval_refs.len())
        {
            return Err(LedgerError::ExternalVerifiedResultAdoptionLimitExceeded {
                field: "protected_action_approval_refs",
            });
        }
        let mut normalized_approvals = BTreeMap::new();
        for reference in protected_action_approval_refs {
            if !valid_evidence_reference(&reference) {
                return Err(LedgerError::InvalidExternalVerifiedResultAdoption {
                    field: "protected_action_approval_refs",
                });
            }
            if normalized_approvals.insert(reference.clone(), ()).is_some() {
                return Err(LedgerError::InvalidExternalVerifiedResultAdoption {
                    field: "protected_action_approval_refs",
                });
            }
        }
        let protected_action_approval_refs = normalized_approvals.into_keys().collect::<Vec<_>>();
        let result_digest = hash_value_at_version(
            "lattice.task-ledger.external-verified-result-adoption",
            "1.0",
            &external_verified_result_adoption_value(
                &task_ref,
                &client_request_id,
                &expected_ledger_head_digest,
                &source_sha,
                &target_sha,
                &push_merge_receipt_ref,
                &deployment_receipt_ref,
                &deployment_artifact_ref,
                &independent_acceptance_ref,
                &protected_action_approval_refs,
            ),
        )?;
        Ok(Self {
            task_ref,
            client_request_id,
            expected_ledger_head_digest,
            source_sha,
            target_sha,
            push_merge_receipt_ref,
            deployment_receipt_ref,
            deployment_artifact_ref,
            independent_acceptance_ref,
            protected_action_approval_refs,
            result_digest,
        })
    }

    #[must_use]
    pub const fn task_ref(&self) -> &ContentDigest {
        &self.task_ref
    }
    #[must_use]
    pub fn client_request_id(&self) -> &str {
        &self.client_request_id
    }
    #[must_use]
    pub const fn expected_ledger_head_digest(&self) -> &ContentDigest {
        &self.expected_ledger_head_digest
    }
    #[must_use]
    pub fn source_sha(&self) -> &str {
        &self.source_sha
    }
    #[must_use]
    pub fn target_sha(&self) -> &str {
        &self.target_sha
    }
    #[must_use]
    pub fn push_merge_receipt_ref(&self) -> &str {
        &self.push_merge_receipt_ref
    }
    #[must_use]
    pub fn deployment_receipt_ref(&self) -> &str {
        &self.deployment_receipt_ref
    }
    #[must_use]
    pub fn deployment_artifact_ref(&self) -> &str {
        &self.deployment_artifact_ref
    }
    #[must_use]
    pub fn independent_acceptance_ref(&self) -> &str {
        &self.independent_acceptance_ref
    }
    #[must_use]
    pub fn protected_action_approval_refs(&self) -> &[String] {
        &self.protected_action_approval_refs
    }

    /// Stable command identity derived only from the MCP idempotency key.
    #[must_use]
    pub fn command_id(&self) -> String {
        format!("external-result-adoption:{}", self.client_request_id)
    }

    /// Complete immutable result identity bound into the terminal Ledger event.
    #[must_use]
    pub const fn result_digest(&self) -> &ContentDigest {
        &self.result_digest
    }

    /// Exports all retained fields as explicitly untrusted persistence data.
    #[must_use]
    pub fn to_untrusted(&self) -> UntrustedExternalVerifiedResultAdoption {
        UntrustedExternalVerifiedResultAdoption {
            schema_version: EXTERNAL_VERIFIED_RESULT_ADOPTION_SCHEMA.to_owned(),
            task_ref: self.task_ref.clone(),
            client_request_id: self.client_request_id.clone(),
            expected_ledger_head_digest: self.expected_ledger_head_digest.clone(),
            source_sha: self.source_sha.clone(),
            target_sha: self.target_sha.clone(),
            push_merge_receipt_ref: self.push_merge_receipt_ref.clone(),
            deployment_receipt_ref: self.deployment_receipt_ref.clone(),
            deployment_artifact_ref: self.deployment_artifact_ref.clone(),
            independent_acceptance_ref: self.independent_acceptance_ref.clone(),
            protected_action_approval_refs: self.protected_action_approval_refs.clone(),
            result_digest: self.result_digest.clone(),
        }
    }
}

/// Raw retained fields for one external verified-result adoption.
/// Every field remains untrusted until replayed through
/// [`verify_untrusted_external_verified_result_adoption`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UntrustedExternalVerifiedResultAdoption {
    pub schema_version: String,
    pub task_ref: ContentDigest,
    pub client_request_id: String,
    pub expected_ledger_head_digest: ContentDigest,
    pub source_sha: String,
    pub target_sha: String,
    pub push_merge_receipt_ref: String,
    pub deployment_receipt_ref: String,
    pub deployment_artifact_ref: String,
    pub independent_acceptance_ref: String,
    pub protected_action_approval_refs: Vec<String>,
    pub result_digest: ContentDigest,
}

/// Reconstructs and verifies one retained external adoption bundle.
///
/// # Errors
///
/// Rejects unknown schemas, malformed or secret-shaped values, and every
/// changed input or retained result digest.
pub fn verify_untrusted_external_verified_result_adoption(
    raw: &UntrustedExternalVerifiedResultAdoption,
) -> Result<ExternalVerifiedResultAdoption, LedgerError> {
    if raw.schema_version != EXTERNAL_VERIFIED_RESULT_ADOPTION_SCHEMA {
        return Err(LedgerError::ExternalVerifiedResultAdoptionMismatch);
    }
    let verified = ExternalVerifiedResultAdoption::new(
        raw.task_ref.clone(),
        raw.client_request_id.clone(),
        raw.expected_ledger_head_digest.clone(),
        raw.source_sha.clone(),
        raw.target_sha.clone(),
        raw.push_merge_receipt_ref.clone(),
        raw.deployment_receipt_ref.clone(),
        raw.deployment_artifact_ref.clone(),
        raw.independent_acceptance_ref.clone(),
        raw.protected_action_approval_refs.clone(),
    )?;
    if verified.result_digest() != &raw.result_digest {
        return Err(LedgerError::ExternalVerifiedResultAdoptionMismatch);
    }
    Ok(verified)
}

/// Closed semantic request families sharing one task-submission ingress keyspace.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskIngressRequestKind {
    /// The backwards-compatible controlled Codex canary request.
    ControlledCodexCanary,
    /// One natural-language task bound to a formal Project Registry identity.
    GeneralTask,
}

impl TaskIngressRequestKind {
    /// Returns the fixed persistence value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ControlledCodexCanary => "CONTROLLED_CODEX_CANARY",
            Self::GeneralTask => "GENERAL_TASK",
        }
    }

    fn parse(value: &str) -> Result<Self, LedgerError> {
        match value {
            "CONTROLLED_CODEX_CANARY" => Ok(Self::ControlledCodexCanary),
            "GENERAL_TASK" => Ok(Self::GeneralTask),
            _ => Err(LedgerError::InvalidTaskIngressClaim {
                field: "request_kind",
            }),
        }
    }
}

/// Canonical Task-Ledger-owned reservation for one client ingress key.
///
/// This claim is idempotency metadata only. It is linked to a real
/// `TASK_CREATED` event by persistence and grants no execution authority.
#[derive(Clone, Eq, PartialEq)]
pub struct TaskIngressClaim {
    ingress_id: String,
    client_request_id: String,
    request_kind: TaskIngressRequestKind,
    request_digest: ContentDigest,
    stream_id: ContentDigest,
}

impl fmt::Debug for TaskIngressClaim {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TaskIngressClaim")
            .field("schema_version", &TASK_INGRESS_CLAIM_SCHEMA)
            .field("ingress_id", &self.ingress_id)
            .field("client_request_id", &"[REDACTED]")
            .field("request_kind", &self.request_kind)
            .field("request_digest", &self.request_digest)
            .field("stream_id", &self.stream_id)
            .finish()
    }
}

impl TaskIngressClaim {
    /// Constructs the controlled-canary claim. Its canonical stream identity
    /// is already the complete semantic commitment for this fixed request kind.
    ///
    /// # Errors
    ///
    /// Rejects malformed ingress identifiers or a zero stream digest.
    pub fn controlled_canary(
        ingress_id: impl Into<String>,
        client_request_id: impl Into<String>,
        stream_id: ContentDigest,
    ) -> Result<Self, LedgerError> {
        let ingress_id = ingress_id.into();
        let client_request_id = client_request_id.into();
        validate_submission_control_id(&ingress_id, "ingress_id", MAX_SUBMISSION_INGRESS_ID_BYTES)?;
        validate_submission_control_id(
            &client_request_id,
            "client_request_id",
            MAX_SUBMISSION_CLIENT_REQUEST_ID_BYTES,
        )?;
        if is_zero_digest(&stream_id) {
            return Err(LedgerError::InvalidTaskIngressClaim { field: "stream_id" });
        }
        Ok(Self {
            ingress_id,
            client_request_id,
            request_kind: TaskIngressRequestKind::ControlledCodexCanary,
            request_digest: stream_id.clone(),
            stream_id,
        })
    }

    /// Constructs the semantic claim for one verified general submission.
    /// The request digest intentionally binds the exact objective and formal
    /// Project ID, while the separately retained stream binds the admitted
    /// Project Registry snapshot and complete Task Ledger identity.
    ///
    /// # Errors
    ///
    /// Returns a canonical-hash error only if the closed hash domain fails.
    pub fn general_submission(submission: &TaskSubmissionEnvelope) -> Result<Self, LedgerError> {
        let request_digest = hash_value_at_version(
            TASK_INGRESS_REQUEST_DOMAIN,
            TASK_SUBMISSION_HASH_VERSION,
            &object(vec![
                ("schema_version", text(TASK_INGRESS_CLAIM_SCHEMA)),
                ("ingress_id", text(submission.ingress_id())),
                ("client_request_id", text(submission.client_request_id())),
                (
                    "request_kind",
                    text(TaskIngressRequestKind::GeneralTask.as_str()),
                ),
                ("objective", text(submission.objective())),
                (
                    "project_id",
                    text(submission.identity().project_id().as_str()),
                ),
            ]),
        )?;
        Ok(Self {
            ingress_id: submission.ingress_id().to_owned(),
            client_request_id: submission.client_request_id().to_owned(),
            request_kind: TaskIngressRequestKind::GeneralTask,
            request_digest,
            stream_id: submission.stream_id().clone(),
        })
    }

    /// Returns the fixed claim schema.
    #[must_use]
    pub const fn schema_version(&self) -> &'static str {
        TASK_INGRESS_CLAIM_SCHEMA
    }

    /// Returns the process-owned ingress identity.
    #[must_use]
    pub fn ingress_id(&self) -> &str {
        &self.ingress_id
    }

    /// Returns the caller idempotency key without exposing it through `Debug`.
    #[must_use]
    pub fn client_request_id(&self) -> &str {
        &self.client_request_id
    }

    /// Returns the closed request family.
    #[must_use]
    pub const fn request_kind(&self) -> TaskIngressRequestKind {
        self.request_kind
    }

    /// Returns the canonical semantic request digest.
    #[must_use]
    pub const fn request_digest(&self) -> &ContentDigest {
        &self.request_digest
    }

    /// Returns the exact Task Ledger stream reserved by this request.
    #[must_use]
    pub const fn stream_id(&self) -> &ContentDigest {
        &self.stream_id
    }

    /// Exports explicitly untrusted retained claim fields.
    #[must_use]
    pub fn to_untrusted(&self) -> UntrustedTaskIngressClaim {
        UntrustedTaskIngressClaim {
            schema_version: TASK_INGRESS_CLAIM_SCHEMA.to_owned(),
            ingress_id: self.ingress_id.clone(),
            client_request_id: self.client_request_id.clone(),
            request_kind: self.request_kind.as_str().to_owned(),
            request_digest: self.request_digest.clone(),
            stream_id: self.stream_id.clone(),
        }
    }
}

/// Raw retained fields for one task-ingress claim.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UntrustedTaskIngressClaim {
    /// Claimed fixed schema.
    pub schema_version: String,
    /// Claimed ingress identity.
    pub ingress_id: String,
    /// Claimed client idempotency key.
    pub client_request_id: String,
    /// Claimed closed request family.
    pub request_kind: String,
    /// Claimed semantic request digest.
    pub request_digest: ContentDigest,
    /// Claimed reserved Task Ledger stream.
    pub stream_id: ContentDigest,
}

/// Verifies one retained claim against the expected pure semantic claim.
///
/// # Errors
///
/// Rejects an unknown schema, malformed fields, or any semantic substitution.
pub fn verify_untrusted_task_ingress_claim(
    raw: &UntrustedTaskIngressClaim,
    expected: &TaskIngressClaim,
) -> Result<TaskIngressClaim, LedgerError> {
    let retained = verify_untrusted_task_ingress_claim_structure(raw)?;
    if &retained != expected {
        return Err(LedgerError::TaskIngressClaimMismatch);
    }
    Ok(retained)
}

/// Verifies the closed structure of one retained ingress claim without
/// requiring caller-supplied request semantics.
///
/// This is intentionally narrower than exact-retry verification. It lets the
/// persistence adapter identify which request family already owns an ingress
/// key before resolving any new general-task project. A general request digest
/// remains opaque until its authoritative envelope is loaded; callers must not
/// treat this function as proof of an exact general-task retry.
///
/// # Errors
///
/// Rejects unknown schemas or kinds, malformed identifiers, zero digests, and
/// any controlled-canary claim whose fixed request digest is not its stream ID.
pub fn verify_untrusted_task_ingress_claim_structure(
    raw: &UntrustedTaskIngressClaim,
) -> Result<TaskIngressClaim, LedgerError> {
    if raw.schema_version != TASK_INGRESS_CLAIM_SCHEMA {
        return Err(LedgerError::UnknownTaskIngressClaimVersion);
    }
    validate_submission_control_id(
        &raw.ingress_id,
        "ingress_id",
        MAX_SUBMISSION_INGRESS_ID_BYTES,
    )?;
    validate_submission_control_id(
        &raw.client_request_id,
        "client_request_id",
        MAX_SUBMISSION_CLIENT_REQUEST_ID_BYTES,
    )?;
    let request_kind = TaskIngressRequestKind::parse(&raw.request_kind)?;
    if is_zero_digest(&raw.request_digest) || is_zero_digest(&raw.stream_id) {
        return Err(LedgerError::InvalidTaskIngressClaim { field: "digest" });
    }
    let retained = TaskIngressClaim {
        ingress_id: raw.ingress_id.clone(),
        client_request_id: raw.client_request_id.clone(),
        request_kind,
        request_digest: raw.request_digest.clone(),
        stream_id: raw.stream_id.clone(),
    };
    if retained.request_kind == TaskIngressRequestKind::ControlledCodexCanary
        && retained.request_digest != retained.stream_id
    {
        return Err(LedgerError::TaskIngressClaimMismatch);
    }
    Ok(retained)
}

const AUTONOMY_RECEIPT_SCHEMA: &str = "lattice.autonomy-receipt/1.0";
const AUTONOMY_RECEIPT_DOMAIN: &str = "lattice.autonomy-receipt";
const AUTONOMY_AUTHORITY_DOMAIN: &str = "lattice.autonomy-authority";
const AUTONOMY_HASH_VERSION: &str = "1.0";
const AUTONOMY_AUTHORITY_MODE: &str = "P0_PROCESS_START_PROFILE_V1";

/// Closed task kind stored in the canonical autonomy receipt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutonomyTaskKind {
    Feature,
    BugFix,
    Configuration,
    Research,
}

impl AutonomyTaskKind {
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

/// Closed risk class consumed by the canonical autonomy classifier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutonomyRiskClass {
    R0,
    R1,
    R2,
    R3,
}

impl AutonomyRiskClass {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::R0 => "R0",
            Self::R1 => "R1",
            Self::R2 => "R2",
            Self::R3 => "R3",
        }
    }
}

/// Closed task state supported by TASK-050 receipt recording.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutonomyObservedTaskState {
    Draft,
}

impl AutonomyObservedTaskState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "DRAFT",
        }
    }
}

/// Closed model value stored only for an accepted `PROCEED` recommendation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutonomyModel {
    GovernedCodexWriter,
    NoModel,
}

impl AutonomyModel {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::GovernedCodexWriter => "GOVERNED_CODEX_WRITER",
            Self::NoModel => "NO_MODEL",
        }
    }
}

/// Closed verification recommendation stored only for `PROCEED`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutonomyVerification {
    FocusedChecks,
    BuildAndFocusedChecks,
    ReadOnlyEvidence,
}

impl AutonomyVerification {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FocusedChecks => "FOCUSED_CHECKS",
            Self::BuildAndFocusedChecks => "BUILD_AND_FOCUSED_CHECKS",
            Self::ReadOnlyEvidence => "READ_ONLY_EVIDENCE",
        }
    }
}

/// Closed reason stored in the canonical autonomy decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutonomyDecisionReason {
    RoutineAuthorized,
    NewUserDecision,
    NewAuthority,
    HighRiskOrIrreversible,
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

/// Pure Orchestrator recommendation that Task Ledger independently verifies.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutonomyRecommendation {
    Proceed {
        model: AutonomyModel,
        verification: AutonomyVerification,
        reason: AutonomyDecisionReason,
    },
    AskUser {
        reason: AutonomyDecisionReason,
    },
}

impl AutonomyRecommendation {
    #[must_use]
    pub const fn disposition(self) -> &'static str {
        match self {
            Self::Proceed { .. } => "PROCEED",
            Self::AskUser { .. } => "ASK_USER",
        }
    }

    #[must_use]
    pub const fn reason(self) -> AutonomyDecisionReason {
        match self {
            Self::Proceed { reason, .. } | Self::AskUser { reason } => reason,
        }
    }

    #[must_use]
    pub const fn model(self) -> Option<AutonomyModel> {
        match self {
            Self::Proceed { model, .. } => Some(model),
            Self::AskUser { .. } => None,
        }
    }

    #[must_use]
    pub const fn verification(self) -> Option<AutonomyVerification> {
        match self {
            Self::Proceed { verification, .. } => Some(verification),
            Self::AskUser { .. } => None,
        }
    }
}

/// Typed autonomy intent plus the pure recommendation Task Ledger must verify.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AutonomyIntent {
    task_kind: AutonomyTaskKind,
    risk_class: AutonomyRiskClass,
    execution_preapproved: bool,
    requires_new_authority: bool,
    irreversible_or_high_risk: bool,
    observed_task_state: AutonomyObservedTaskState,
    recommendation: AutonomyRecommendation,
}

impl AutonomyIntent {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub const fn new(
        task_kind: AutonomyTaskKind,
        risk_class: AutonomyRiskClass,
        execution_preapproved: bool,
        requires_new_authority: bool,
        irreversible_or_high_risk: bool,
        observed_task_state: AutonomyObservedTaskState,
        recommendation: AutonomyRecommendation,
    ) -> Self {
        Self {
            task_kind,
            risk_class,
            execution_preapproved,
            requires_new_authority,
            irreversible_or_high_risk,
            observed_task_state,
            recommendation,
        }
    }

    #[must_use]
    pub const fn task_kind(self) -> AutonomyTaskKind {
        self.task_kind
    }

    #[must_use]
    pub const fn risk_class(self) -> AutonomyRiskClass {
        self.risk_class
    }

    #[must_use]
    pub const fn execution_preapproved(self) -> bool {
        self.execution_preapproved
    }

    #[must_use]
    pub const fn requires_new_authority(self) -> bool {
        self.requires_new_authority
    }

    #[must_use]
    pub const fn irreversible_or_high_risk(self) -> bool {
        self.irreversible_or_high_risk
    }

    #[must_use]
    pub const fn observed_task_state(self) -> AutonomyObservedTaskState {
        self.observed_task_state
    }

    #[must_use]
    pub const fn recommendation(self) -> AutonomyRecommendation {
        self.recommendation
    }
}

/// Fixed command metadata for one typed autonomy append.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AutonomyAppendMetadata {
    command_id: CommandId,
    correlation_id: CorrelationId,
    occurred_at: String,
    actor_id: ActorId,
}

impl AutonomyAppendMetadata {
    /// Constructs bounded metadata without reading a clock.
    ///
    /// # Errors
    ///
    /// Rejects a non-canonical UTC timestamp.
    pub fn new(
        command_id: CommandId,
        correlation_id: CorrelationId,
        occurred_at: impl Into<String>,
        actor_id: ActorId,
    ) -> Result<Self, LedgerError> {
        let occurred_at = occurred_at.into();
        validate_utc_timestamp(&occurred_at)?;
        Ok(Self {
            command_id,
            correlation_id,
            occurred_at,
            actor_id,
        })
    }
}

/// Complete P0 authority evidence consumed by one Task-Ledger-owned receipt.
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
    ) -> Result<Self, LedgerError> {
        if [
            &process_start_authority_digest,
            &ingress_profile_adapter_commitment,
            &store_authority_head_digest,
        ]
        .into_iter()
        .any(is_zero_digest)
        {
            return Err(LedgerError::InvalidAutonomyReceipt);
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

/// Canonical autonomy subject and fixed scalars verified by Task Ledger.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedAutonomyReceipt {
    stream_id: ContentDigest,
    event_sequence: u64,
    event_digest: ContentDigest,
    intent: AutonomyIntent,
    process_start_authority_digest: ContentDigest,
    ingress_profile_adapter_commitment: ContentDigest,
    store_authority_head_digest: ContentDigest,
    writer_lease_receipt_digest: Option<ContentDigest>,
    writer_lease_head_digest: Option<ContentDigest>,
    writer_fencing_token: Option<u64>,
    authority_digest: ContentDigest,
    receipt_digest: ContentDigest,
}

impl VerifiedAutonomyReceipt {
    #[must_use]
    pub const fn stream_id(&self) -> &ContentDigest {
        &self.stream_id
    }

    #[must_use]
    pub const fn event_sequence(&self) -> u64 {
        self.event_sequence
    }

    #[must_use]
    pub const fn event_digest(&self) -> &ContentDigest {
        &self.event_digest
    }

    #[must_use]
    pub const fn receipt_schema_version(&self) -> &'static str {
        AUTONOMY_RECEIPT_SCHEMA
    }

    #[must_use]
    pub const fn intent_version(&self) -> &'static str {
        AUTONOMY_HASH_VERSION
    }

    #[must_use]
    pub const fn intent(&self) -> AutonomyIntent {
        self.intent
    }

    #[must_use]
    pub const fn authority_mode(&self) -> &'static str {
        AUTONOMY_AUTHORITY_MODE
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
    pub const fn writer_lease_receipt_digest(&self) -> Option<&ContentDigest> {
        self.writer_lease_receipt_digest.as_ref()
    }

    #[must_use]
    pub const fn writer_lease_head_digest(&self) -> Option<&ContentDigest> {
        self.writer_lease_head_digest.as_ref()
    }

    #[must_use]
    pub const fn writer_fencing_token(&self) -> Option<u64> {
        self.writer_fencing_token
    }

    #[must_use]
    pub const fn authority_digest(&self) -> &ContentDigest {
        &self.authority_digest
    }

    #[must_use]
    pub const fn receipt_digest(&self) -> &ContentDigest {
        &self.receipt_digest
    }

    /// Exports the already-verified receipt as untrusted persistence scalars.
    /// Re-import must pass through `verify_untrusted_autonomy_receipt_rows`.
    #[must_use]
    pub fn to_untrusted(&self) -> UntrustedAutonomyReceiptRow {
        UntrustedAutonomyReceiptRow {
            stream_id: self.stream_id.clone(),
            event_sequence: self.event_sequence,
            event_digest: self.event_digest.clone(),
            receipt_schema_version: AUTONOMY_RECEIPT_SCHEMA.to_owned(),
            intent_version: AUTONOMY_HASH_VERSION.to_owned(),
            task_kind: self.intent.task_kind.as_str().to_owned(),
            risk_class: self.intent.risk_class.as_str().to_owned(),
            execution_preapproved: self.intent.execution_preapproved,
            requires_new_authority: self.intent.requires_new_authority,
            irreversible_or_high_risk: self.intent.irreversible_or_high_risk,
            observed_task_state: self.intent.observed_task_state.as_str().to_owned(),
            disposition: self.intent.recommendation.disposition().to_owned(),
            decision_reason: self.intent.recommendation.reason().as_str().to_owned(),
            model: self
                .intent
                .recommendation
                .model()
                .map(|value| value.as_str().to_owned()),
            verification: self
                .intent
                .recommendation
                .verification()
                .map(|value| value.as_str().to_owned()),
            authority_mode: AUTONOMY_AUTHORITY_MODE.to_owned(),
            process_start_authority_digest: self.process_start_authority_digest.clone(),
            ingress_profile_adapter_commitment: self.ingress_profile_adapter_commitment.clone(),
            store_authority_head_digest: self.store_authority_head_digest.clone(),
            writer_lease_receipt_digest: self.writer_lease_receipt_digest.clone(),
            writer_lease_head_digest: self.writer_lease_head_digest.clone(),
            writer_fencing_token: self.writer_fencing_token,
            authority_digest: self.authority_digest.clone(),
            receipt_digest: self.receipt_digest.clone(),
        }
    }
}

/// Complete untrusted 24-scalar autonomy row decoded from persistence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UntrustedAutonomyReceiptRow {
    stream_id: ContentDigest,
    event_sequence: u64,
    event_digest: ContentDigest,
    receipt_schema_version: String,
    intent_version: String,
    task_kind: String,
    risk_class: String,
    execution_preapproved: bool,
    requires_new_authority: bool,
    irreversible_or_high_risk: bool,
    observed_task_state: String,
    disposition: String,
    decision_reason: String,
    model: Option<String>,
    verification: Option<String>,
    authority_mode: String,
    process_start_authority_digest: ContentDigest,
    ingress_profile_adapter_commitment: ContentDigest,
    store_authority_head_digest: ContentDigest,
    writer_lease_receipt_digest: Option<ContentDigest>,
    writer_lease_head_digest: Option<ContentDigest>,
    writer_fencing_token: Option<u64>,
    authority_digest: ContentDigest,
    receipt_digest: ContentDigest,
}

impl UntrustedAutonomyReceiptRow {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        stream_id: ContentDigest,
        event_sequence: u64,
        event_digest: ContentDigest,
        receipt_schema_version: impl Into<String>,
        intent_version: impl Into<String>,
        task_kind: impl Into<String>,
        risk_class: impl Into<String>,
        execution_preapproved: bool,
        requires_new_authority: bool,
        irreversible_or_high_risk: bool,
        observed_task_state: impl Into<String>,
        disposition: impl Into<String>,
        decision_reason: impl Into<String>,
        model: Option<String>,
        verification: Option<String>,
        authority_mode: impl Into<String>,
        process_start_authority_digest: ContentDigest,
        ingress_profile_adapter_commitment: ContentDigest,
        store_authority_head_digest: ContentDigest,
        writer_lease_receipt_digest: Option<ContentDigest>,
        writer_lease_head_digest: Option<ContentDigest>,
        writer_fencing_token: Option<u64>,
        authority_digest: ContentDigest,
        receipt_digest: ContentDigest,
    ) -> Self {
        Self {
            stream_id,
            event_sequence,
            event_digest,
            receipt_schema_version: receipt_schema_version.into(),
            intent_version: intent_version.into(),
            task_kind: task_kind.into(),
            risk_class: risk_class.into(),
            execution_preapproved,
            requires_new_authority,
            irreversible_or_high_risk,
            observed_task_state: observed_task_state.into(),
            disposition: disposition.into(),
            decision_reason: decision_reason.into(),
            model,
            verification,
            authority_mode: authority_mode.into(),
            process_start_authority_digest,
            ingress_profile_adapter_commitment,
            store_authority_head_digest,
            writer_lease_receipt_digest,
            writer_lease_head_digest,
            writer_fencing_token,
            authority_digest,
            receipt_digest,
        }
    }

    #[must_use]
    pub const fn stream_id(&self) -> &ContentDigest {
        &self.stream_id
    }

    #[must_use]
    pub const fn event_sequence(&self) -> u64 {
        self.event_sequence
    }

    #[must_use]
    pub const fn event_digest(&self) -> &ContentDigest {
        &self.event_digest
    }

    #[must_use]
    pub fn receipt_schema_version(&self) -> &str {
        &self.receipt_schema_version
    }

    #[must_use]
    pub fn intent_version(&self) -> &str {
        &self.intent_version
    }

    #[must_use]
    pub fn task_kind(&self) -> &str {
        &self.task_kind
    }

    #[must_use]
    pub fn risk_class(&self) -> &str {
        &self.risk_class
    }

    #[must_use]
    pub const fn execution_preapproved(&self) -> bool {
        self.execution_preapproved
    }

    #[must_use]
    pub const fn requires_new_authority(&self) -> bool {
        self.requires_new_authority
    }

    #[must_use]
    pub const fn irreversible_or_high_risk(&self) -> bool {
        self.irreversible_or_high_risk
    }

    #[must_use]
    pub fn observed_task_state(&self) -> &str {
        &self.observed_task_state
    }

    #[must_use]
    pub fn disposition(&self) -> &str {
        &self.disposition
    }

    #[must_use]
    pub fn decision_reason(&self) -> &str {
        &self.decision_reason
    }

    #[must_use]
    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    #[must_use]
    pub fn verification(&self) -> Option<&str> {
        self.verification.as_deref()
    }

    #[must_use]
    pub fn authority_mode(&self) -> &str {
        &self.authority_mode
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
    pub const fn writer_lease_receipt_digest(&self) -> Option<&ContentDigest> {
        self.writer_lease_receipt_digest.as_ref()
    }

    #[must_use]
    pub const fn writer_lease_head_digest(&self) -> Option<&ContentDigest> {
        self.writer_lease_head_digest.as_ref()
    }

    #[must_use]
    pub const fn writer_fencing_token(&self) -> Option<u64> {
        self.writer_fencing_token
    }

    #[must_use]
    pub const fn authority_digest(&self) -> &ContentDigest {
        &self.authority_digest
    }

    #[must_use]
    pub const fn receipt_digest(&self) -> &ContentDigest {
        &self.receipt_digest
    }
}

/// Closed Task-Ledger-owned autonomy replay state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VerifiedAutonomyReceiptState {
    NotApplicable,
    HistoricalOptional(Option<VerifiedAutonomyReceipt>),
    PendingRequiredReceipt,
    RequiredComplete(VerifiedAutonomyReceipt),
}

/// Typed Task Ledger append plan paired with its canonical autonomy scalars.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AutonomyReceiptAppendPlan {
    append_plan: LedgerAppendPlan,
    receipt: VerifiedAutonomyReceipt,
    writer_authority: Option<WriterLeaseAuthorityHead>,
}

impl AutonomyReceiptAppendPlan {
    #[must_use]
    pub const fn append_plan(&self) -> &LedgerAppendPlan {
        &self.append_plan
    }

    #[must_use]
    pub const fn receipt(&self) -> &VerifiedAutonomyReceipt {
        &self.receipt
    }

    #[must_use]
    pub const fn writer_authority(&self) -> Option<&WriterLeaseAuthorityHead> {
        self.writer_authority.as_ref()
    }
}

/// Closed audit outcomes supported by Task Ledger schema 2.0.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LedgerOutcome {
    Recorded,
    Allowed,
    Denied,
    Passed,
    Failed,
    Blocked,
    Cancelled,
}

impl LedgerOutcome {
    /// Returns the stable wire value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Recorded => "RECORDED",
            Self::Allowed => "ALLOWED",
            Self::Denied => "DENIED",
            Self::Passed => "PASSED",
            Self::Failed => "FAILED",
            Self::Blocked => "BLOCKED",
            Self::Cancelled => "CANCELLED",
        }
    }

    /// Parses one closed wire value.
    ///
    /// # Errors
    ///
    /// Rejects an unknown event outcome.
    pub fn parse(value: &str) -> Result<Self, LedgerError> {
        match value {
            "RECORDED" => Ok(Self::Recorded),
            "ALLOWED" => Ok(Self::Allowed),
            "DENIED" => Ok(Self::Denied),
            "PASSED" => Ok(Self::Passed),
            "FAILED" => Ok(Self::Failed),
            "BLOCKED" => Ok(Self::Blocked),
            "CANCELLED" => Ok(Self::Cancelled),
            _ => Err(LedgerError::UnknownEventOutcome),
        }
    }
}

/// Sanitized, bounded, non-authoritative diagnostic payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnostic(CanonicalValue);

impl Diagnostic {
    /// Sanitizes and bounds one diagnostic value before hashing.
    ///
    /// # Errors
    ///
    /// Rejects non-NFC keys, normalized-key collisions, or depth/node/byte
    /// limit violations.
    pub fn new(value: CanonicalValue) -> Result<Self, LedgerError> {
        let mut raw_nodes = 0_usize;
        validate_raw_diagnostic(&value, 0, &mut raw_nodes)?;
        let raw_canonical = canonicalize(&value).map_err(|_| LedgerError::InvalidDiagnostic)?;
        if raw_canonical.as_slice().len() > MAX_DIAGNOSTIC_BYTES {
            return Err(LedgerError::DiagnosticLimitExceeded);
        }
        let mut nodes = 0_usize;
        let sanitized = sanitize_diagnostic(value, 0, &mut nodes)?;
        if nodes > MAX_DIAGNOSTIC_NODES {
            return Err(LedgerError::DiagnosticLimitExceeded);
        }
        let canonical = canonicalize(&sanitized).map_err(|_| LedgerError::InvalidDiagnostic)?;
        if canonical.as_slice().len() > MAX_DIAGNOSTIC_BYTES {
            return Err(LedgerError::DiagnosticLimitExceeded);
        }
        Ok(Self(sanitized))
    }

    /// Returns the sanitized, bounded, non-authoritative value.
    #[must_use]
    pub const fn value(&self) -> &CanonicalValue {
        &self.0
    }
}

/// Typed current resource snapshot carried only by resource events.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceSnapshot(ResourceCounters);

impl ResourceSnapshot {
    /// Wraps already-validated current counters.
    #[must_use]
    pub const fn new(counters: ResourceCounters) -> Self {
        Self(counters)
    }

    /// Returns the represented current counters.
    #[must_use]
    pub const fn counters(&self) -> &ResourceCounters {
        &self.0
    }
}

/// One validated event append request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppendCommand {
    expected_head: TaskLedgerStreamHead,
    command_id: CommandId,
    correlation_id: CorrelationId,
    occurred_at: String,
    kind: LedgerEventKind,
    actor_id: ActorId,
    action: ActionId,
    outcome: LedgerOutcome,
    reason_code: ReasonCode,
    subject_digest: ContentDigest,
    diagnostic: Option<Diagnostic>,
    resource_snapshot: Option<ResourceSnapshot>,
}

#[derive(Clone, Copy)]
enum AppendConstruction {
    Generic,
    RequiredTaskCreated,
    ManagedTaskCreated,
    GeneralTaskCreated,
    VerifiedAutonomy,
    VerifiedForeman,
    VerifiedExternalResultAdoption,
    VerifiedReplay,
}

impl AppendCommand {
    /// Constructs one exact append request without reading a clock.
    ///
    /// # Errors
    ///
    /// Rejects a non-canonical timestamp or a resource snapshot on the wrong
    /// event kind.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        expected_head: TaskLedgerStreamHead,
        command_id: CommandId,
        correlation_id: CorrelationId,
        occurred_at: impl Into<String>,
        kind: LedgerEventKind,
        actor_id: ActorId,
        action: ActionId,
        outcome: LedgerOutcome,
        reason_code: ReasonCode,
        subject_digest: ContentDigest,
        diagnostic: Option<Diagnostic>,
        resource_snapshot: Option<ResourceSnapshot>,
    ) -> Result<Self, LedgerError> {
        Self::from_fields(
            expected_head,
            command_id,
            correlation_id,
            occurred_at,
            kind,
            actor_id,
            action,
            outcome,
            reason_code,
            subject_digest,
            diagnostic,
            resource_snapshot,
            AppendConstruction::Generic,
        )
    }

    /// Constructs the only Task-created profile that new task-control admission may mint.
    ///
    /// # Errors
    ///
    /// Rejects invalid identifiers, timestamp, diagnostic, or subject fields.
    #[allow(clippy::too_many_arguments)]
    pub fn new_autonomy_required_task_created(
        expected_head: TaskLedgerStreamHead,
        command_id: CommandId,
        correlation_id: CorrelationId,
        occurred_at: impl Into<String>,
        actor_id: ActorId,
        reason_code: ReasonCode,
        subject_digest: ContentDigest,
        diagnostic: Option<Diagnostic>,
    ) -> Result<Self, LedgerError> {
        if expected_head.sequence() != 0 {
            return Err(LedgerError::InvalidAutonomyReceipt);
        }
        Self::from_fields(
            expected_head,
            command_id,
            correlation_id,
            occurred_at,
            LedgerEventKind::TaskCreated,
            actor_id,
            ActionId::new(TaskCreatedProfile::AutonomyReceiptRequiredV1.action())?,
            LedgerOutcome::Recorded,
            reason_code,
            subject_digest,
            diagnostic,
            None,
            AppendConstruction::RequiredTaskCreated,
        )
    }

    /// Constructs the managed-foreman-only Task-Spec successor marker.
    ///
    /// Unlike the historical controlled canary, the Task-created subject is
    /// the exact Task Spec digest and no ingress diagnostic is retained.
    ///
    /// # Errors
    ///
    /// Rejects a non-vacant or non-Task-Spec stream and malformed metadata.
    #[allow(clippy::too_many_arguments)]
    pub fn new_managed_general_task_created(
        expected_head: TaskLedgerStreamHead,
        command_id: CommandId,
        correlation_id: CorrelationId,
        occurred_at: impl Into<String>,
        actor_id: ActorId,
        reason_code: ReasonCode,
    ) -> Result<Self, LedgerError> {
        let Some(task_spec_digest) = expected_head.identity().task_spec_digest().cloned() else {
            return Err(LedgerError::InvalidStreamHead);
        };
        if expected_head.sequence() != 0 {
            return Err(LedgerError::InvalidAutonomyReceipt);
        }
        Self::from_fields(
            expected_head,
            command_id,
            correlation_id,
            occurred_at,
            LedgerEventKind::TaskCreated,
            actor_id,
            ActionId::new(TaskCreatedProfile::ManagedGeneralTaskV1.action())?,
            LedgerOutcome::Recorded,
            reason_code,
            task_spec_digest,
            None,
            None,
            AppendConstruction::ManagedTaskCreated,
        )
    }

    /// Constructs the only `TASK_CREATED` command bound to a verified general
    /// task submission envelope.
    ///
    /// # Errors
    ///
    /// Rejects a non-vacant or differently bound stream head and any invalid
    /// command metadata. The objective is not copied into diagnostic data.
    pub fn new_general_task_created(
        expected_head: TaskLedgerStreamHead,
        command_id: CommandId,
        correlation_id: CorrelationId,
        occurred_at: impl Into<String>,
        actor_id: ActorId,
        submission: &TaskSubmissionEnvelope,
    ) -> Result<Self, LedgerError> {
        if expected_head.sequence() != 0
            || expected_head.identity() != submission.identity()
            || expected_head.stream_id() != submission.stream_id()
        {
            return Err(LedgerError::SubmissionEnvelopeMismatch);
        }
        Self::from_fields(
            expected_head,
            command_id,
            correlation_id,
            occurred_at,
            LedgerEventKind::TaskCreated,
            actor_id,
            ActionId::new(TaskCreatedProfile::GeneralTaskIntakeV1.action())?,
            LedgerOutcome::Recorded,
            ReasonCode::new("GENERAL_TASK_INTAKE_RECORDED")?,
            submission.envelope_digest().clone(),
            None,
            None,
            AppendConstruction::GeneralTaskCreated,
        )
    }

    /// Constructs the only terminal event that may close a create-only general
    /// intake without manufacturing execution or Writer-Lease authority.
    ///
    /// The repository adapter must independently verify all referenced
    /// receipts before constructing this command; this pure boundary binds the
    /// already-verified bundle to the exact DRAFT Ledger head.
    #[allow(clippy::too_many_arguments)]
    pub fn new_external_verified_result_adopted(
        expected_head: TaskLedgerStreamHead,
        command_id: CommandId,
        correlation_id: CorrelationId,
        occurred_at: impl Into<String>,
        actor_id: ActorId,
        adoption: &ExternalVerifiedResultAdoption,
    ) -> Result<Self, LedgerError> {
        if expected_head.identity().subject_kind() != TaskLedgerSubjectKind::GeneralTaskIntake
            || expected_head.sequence() != 1
            || expected_head.head_digest() != adoption.expected_ledger_head_digest()
            || command_id.as_str() != adoption.command_id()
        {
            return Err(LedgerError::ExternalVerifiedResultAdoptionMismatch);
        }
        Self::from_fields(
            expected_head,
            command_id,
            correlation_id,
            occurred_at,
            LedgerEventKind::ExternalVerifiedResultAdopted,
            actor_id,
            ActionId::new(EXTERNAL_RESULT_ADOPTION_ACTION)?,
            LedgerOutcome::Recorded,
            ReasonCode::new(EXTERNAL_RESULT_ADOPTION_REASON)?,
            adoption.result_digest().clone(),
            None,
            None,
            AppendConstruction::VerifiedExternalResultAdoption,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn from_fields(
        expected_head: TaskLedgerStreamHead,
        command_id: CommandId,
        correlation_id: CorrelationId,
        occurred_at: impl Into<String>,
        kind: LedgerEventKind,
        actor_id: ActorId,
        action: ActionId,
        outcome: LedgerOutcome,
        reason_code: ReasonCode,
        subject_digest: ContentDigest,
        diagnostic: Option<Diagnostic>,
        resource_snapshot: Option<ResourceSnapshot>,
        construction: AppendConstruction,
    ) -> Result<Self, LedgerError> {
        let occurred_at = occurred_at.into();
        validate_utc_timestamp(&occurred_at)?;
        let resource_shape = matches!(kind, LedgerEventKind::ResourceSnapshot);
        if resource_shape != resource_snapshot.is_some() {
            return Err(LedgerError::InvalidResourceSnapshot);
        }
        if matches!(kind, LedgerEventKind::AutonomyReceiptRecorded)
            && (!matches!(
                construction,
                AppendConstruction::VerifiedAutonomy | AppendConstruction::VerifiedReplay
            ) || action.as_str() != "RECORD_AUTONOMY_RECEIPT_V1"
                || outcome != LedgerOutcome::Recorded
                || reason_code.as_str() != "AUTONOMY_DECISION_RECORDED"
                || diagnostic.is_some()
                || resource_snapshot.is_some())
        {
            return Err(LedgerError::InvalidAutonomyReceipt);
        }
        if matches!(kind, LedgerEventKind::ForemanSnapshotRecorded)
            && (!matches!(
                construction,
                AppendConstruction::VerifiedForeman | AppendConstruction::VerifiedReplay
            ) || action.as_str() != "RECORD_FOREMAN_SNAPSHOT_V1"
                || outcome != LedgerOutcome::Recorded
                || reason_code.as_str() != "FOREMAN_SNAPSHOT_RECORDED"
                || diagnostic.is_some()
                || resource_snapshot.is_some())
        {
            return Err(LedgerError::InvalidForemanSnapshot);
        }
        if matches!(kind, LedgerEventKind::ExternalVerifiedResultAdopted)
            && (!matches!(
                construction,
                AppendConstruction::VerifiedExternalResultAdoption
                    | AppendConstruction::VerifiedReplay
            ) || action.as_str() != EXTERNAL_RESULT_ADOPTION_ACTION
                || outcome != LedgerOutcome::Recorded
                || reason_code.as_str() != EXTERNAL_RESULT_ADOPTION_REASON
                || diagnostic.is_some()
                || resource_snapshot.is_some())
        {
            return Err(LedgerError::ExternalVerifiedResultAdoptionMismatch);
        }
        if matches!(kind, LedgerEventKind::TaskCreated) {
            let profile = classify_task_created_action(action.as_str())?;
            match construction {
                AppendConstruction::Generic if profile.is_some() => {
                    return Err(LedgerError::UnknownTaskCreatedProfile);
                }
                AppendConstruction::RequiredTaskCreated
                    if !profile.is_some_and(TaskCreatedProfile::requires_autonomy_receipt) =>
                {
                    return Err(LedgerError::UnknownTaskCreatedProfile);
                }
                AppendConstruction::ManagedTaskCreated
                    if profile != Some(TaskCreatedProfile::ManagedGeneralTaskV1) =>
                {
                    return Err(LedgerError::UnknownTaskCreatedProfile);
                }
                AppendConstruction::GeneralTaskCreated
                    if profile != Some(TaskCreatedProfile::GeneralTaskIntakeV1) =>
                {
                    return Err(LedgerError::UnknownTaskCreatedProfile);
                }
                AppendConstruction::Generic
                | AppendConstruction::RequiredTaskCreated
                | AppendConstruction::ManagedTaskCreated
                | AppendConstruction::GeneralTaskCreated
                | AppendConstruction::VerifiedAutonomy
                | AppendConstruction::VerifiedForeman
                | AppendConstruction::VerifiedExternalResultAdoption
                | AppendConstruction::VerifiedReplay => {}
            }
        }
        Ok(Self {
            expected_head,
            command_id,
            correlation_id,
            occurred_at,
            kind,
            actor_id,
            action,
            outcome,
            reason_code,
            subject_digest,
            diagnostic,
            resource_snapshot,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn new_verified_foreman(
        expected_head: TaskLedgerStreamHead,
        command_id: CommandId,
        correlation_id: CorrelationId,
        occurred_at: impl Into<String>,
        subject_digest: ContentDigest,
    ) -> Result<Self, LedgerError> {
        Self::from_fields(
            expected_head,
            command_id,
            correlation_id,
            occurred_at,
            LedgerEventKind::ForemanSnapshotRecorded,
            ActorId::new("lattice-foreman")?,
            ActionId::new("RECORD_FOREMAN_SNAPSHOT_V1")?,
            LedgerOutcome::Recorded,
            ReasonCode::new("FOREMAN_SNAPSHOT_RECORDED")?,
            subject_digest,
            None,
            None,
            AppendConstruction::VerifiedForeman,
        )
    }

    /// Returns the caller-supplied full expected head.
    #[must_use]
    pub const fn expected_head(&self) -> &TaskLedgerStreamHead {
        &self.expected_head
    }

    /// Returns the stable command identifier.
    #[must_use]
    pub const fn command_id(&self) -> &CommandId {
        &self.command_id
    }

    /// Returns the correlation identifier.
    #[must_use]
    pub const fn correlation_id(&self) -> &CorrelationId {
        &self.correlation_id
    }

    /// Returns the caller-supplied canonical UTC occurrence time.
    #[must_use]
    pub fn occurred_at(&self) -> &str {
        &self.occurred_at
    }

    /// Returns the closed event kind.
    #[must_use]
    pub const fn kind(&self) -> LedgerEventKind {
        self.kind
    }

    /// Returns the actor identifier.
    #[must_use]
    pub const fn actor_id(&self) -> &ActorId {
        &self.actor_id
    }

    /// Returns the action identifier.
    #[must_use]
    pub const fn action(&self) -> &ActionId {
        &self.action
    }

    /// Returns the closed audit outcome.
    #[must_use]
    pub const fn outcome(&self) -> LedgerOutcome {
        self.outcome
    }

    /// Returns the reason identifier.
    #[must_use]
    pub const fn reason_code(&self) -> &ReasonCode {
        &self.reason_code
    }

    /// Returns the authoritative subject digest.
    #[must_use]
    pub const fn subject_digest(&self) -> &ContentDigest {
        &self.subject_digest
    }

    /// Returns the sanitized diagnostic, if any.
    #[must_use]
    pub const fn diagnostic(&self) -> Option<&Diagnostic> {
        self.diagnostic.as_ref()
    }

    /// Returns the typed resource snapshot, if any.
    #[must_use]
    pub const fn resource_snapshot(&self) -> Option<&ResourceSnapshot> {
        self.resource_snapshot.as_ref()
    }
}

/// One immutable Task Ledger event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LedgerEvent {
    schema_version: String,
    stream_identity: TaskLedgerStreamIdentity,
    stream_id: ContentDigest,
    sequence: u64,
    previous_event_digest: ContentDigest,
    command_id: CommandId,
    request_digest: ContentDigest,
    correlation_id: CorrelationId,
    occurred_at: String,
    kind: LedgerEventKind,
    actor_id: ActorId,
    action: ActionId,
    outcome: LedgerOutcome,
    reason_code: ReasonCode,
    subject_digest: ContentDigest,
    diagnostic: Option<Diagnostic>,
    resource_snapshot: Option<ResourceSnapshot>,
    resource_revision: u64,
    resource_projection_digest: ContentDigest,
    event_digest: ContentDigest,
}

impl LedgerEvent {
    /// Returns the event schema version.
    #[must_use]
    pub fn schema_version(&self) -> &str {
        &self.schema_version
    }

    /// Returns the complete stream identity.
    #[must_use]
    pub const fn stream_identity(&self) -> &TaskLedgerStreamIdentity {
        &self.stream_identity
    }

    /// Returns the stream digest.
    #[must_use]
    pub const fn stream_id(&self) -> &ContentDigest {
        &self.stream_id
    }

    /// Returns the exact sequence.
    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Returns the event digest.
    #[must_use]
    pub const fn event_digest(&self) -> &ContentDigest {
        &self.event_digest
    }

    /// Returns the predecessor event digest.
    #[must_use]
    pub const fn previous_event_digest(&self) -> &ContentDigest {
        &self.previous_event_digest
    }

    /// Returns the stable command identity.
    #[must_use]
    pub const fn command_id(&self) -> &CommandId {
        &self.command_id
    }

    /// Returns the canonical command request digest.
    #[must_use]
    pub const fn request_digest(&self) -> &ContentDigest {
        &self.request_digest
    }

    /// Returns the correlation identifier.
    #[must_use]
    pub const fn correlation_id(&self) -> &CorrelationId {
        &self.correlation_id
    }

    /// Returns the caller-supplied canonical UTC timestamp.
    #[must_use]
    pub fn occurred_at(&self) -> &str {
        &self.occurred_at
    }

    /// Returns the closed event kind.
    #[must_use]
    pub const fn kind(&self) -> LedgerEventKind {
        self.kind
    }

    /// Returns the actor identifier.
    #[must_use]
    pub const fn actor_id(&self) -> &ActorId {
        &self.actor_id
    }

    /// Returns the action identifier.
    #[must_use]
    pub const fn action(&self) -> &ActionId {
        &self.action
    }

    /// Returns the closed event outcome.
    #[must_use]
    pub const fn outcome(&self) -> LedgerOutcome {
        self.outcome
    }

    /// Returns the reason identifier.
    #[must_use]
    pub const fn reason_code(&self) -> &ReasonCode {
        &self.reason_code
    }

    /// Returns the authoritative event subject digest.
    #[must_use]
    pub const fn subject_digest(&self) -> &ContentDigest {
        &self.subject_digest
    }

    /// Returns the sanitized non-authoritative diagnostic, if any.
    #[must_use]
    pub const fn diagnostic(&self) -> Option<&Diagnostic> {
        self.diagnostic.as_ref()
    }

    /// Returns the typed resource snapshot, if this is a resource event.
    #[must_use]
    pub const fn resource_snapshot(&self) -> Option<&ResourceSnapshot> {
        self.resource_snapshot.as_ref()
    }

    /// Returns the resulting resource revision.
    #[must_use]
    pub const fn resource_revision(&self) -> u64 {
        self.resource_revision
    }

    /// Returns the resulting resource projection digest.
    #[must_use]
    pub const fn resource_projection_digest(&self) -> &ContentDigest {
        &self.resource_projection_digest
    }

    /// Exports the complete event as an explicitly untrusted persistence row.
    #[must_use]
    pub fn to_untrusted(&self) -> UntrustedLedgerEvent {
        untrusted_event(self)
    }
}

/// Classifies the Task-Ledger-owned profile carried by one verified event.
///
/// # Errors
///
/// Rejects unknown values in the reserved controlled-canary namespace.
pub fn classify_task_created_profile(
    event: &LedgerEvent,
) -> Result<Option<TaskCreatedProfile>, LedgerError> {
    if event.kind() != LedgerEventKind::TaskCreated {
        return Ok(None);
    }
    classify_task_created_action(event.action().as_str())
}

fn classify_task_created_action(action: &str) -> Result<Option<TaskCreatedProfile>, LedgerError> {
    match action {
        "CONTROLLED_CODEX_CANARY" => Ok(Some(TaskCreatedProfile::HistoricalAutonomyOptionalV1)),
        "CONTROLLED_CODEX_CANARY_AUTONOMY_V1" => {
            Ok(Some(TaskCreatedProfile::AutonomyReceiptRequiredV1))
        }
        "GENERAL_TASK_INTAKE_V1" => Ok(Some(TaskCreatedProfile::GeneralTaskIntakeV1)),
        "MANAGED_GENERAL_TASK_V1" => Ok(Some(TaskCreatedProfile::ManagedGeneralTaskV1)),
        value if value.starts_with("CONTROLLED_CODEX_CANARY") => {
            Err(LedgerError::UnknownTaskCreatedProfile)
        }
        value if value.starts_with("GENERAL_TASK_INTAKE") => {
            Err(LedgerError::UnknownTaskCreatedProfile)
        }
        value if value.starts_with("MANAGED_GENERAL_TASK") => {
            Err(LedgerError::UnknownTaskCreatedProfile)
        }
        _ => Ok(None),
    }
}

fn validate_task_created_profile_subject(
    identity: &TaskLedgerStreamIdentity,
    profile: Option<TaskCreatedProfile>,
    subject_digest: &ContentDigest,
) -> Result<(), LedgerError> {
    if profile == Some(TaskCreatedProfile::ManagedGeneralTaskV1)
        && (identity.subject_kind() != TaskLedgerSubjectKind::TaskSpec
            || identity.task_spec_digest() != Some(subject_digest))
    {
        return Err(LedgerError::InvalidStreamHead);
    }
    Ok(())
}

/// Stable terminal reason for a non-appending command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LedgerDenial {
    /// The supplied full expected head is no longer current.
    StaleHead,
    /// The current event sequence cannot advance without wrapping.
    SequenceOverflow,
}

impl LedgerDenial {
    /// Returns the stable denial wire value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::StaleHead => "STALE_HEAD",
            Self::SequenceOverflow => "SEQUENCE_OVERFLOW",
        }
    }

    /// Parses one closed denial wire value.
    ///
    /// # Errors
    ///
    /// Rejects an unknown denial value.
    pub fn parse(value: &str) -> Result<Self, LedgerError> {
        match value {
            "STALE_HEAD" => Ok(Self::StaleHead),
            "SEQUENCE_OVERFLOW" => Ok(Self::SequenceOverflow),
            _ => Err(LedgerError::UnknownReceiptOutcome),
        }
    }
}

/// Terminal command outcome stored in an immutable receipt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommandOutcome {
    /// Exactly one event was appended.
    Appended,
    /// The command was evaluated and appended no event.
    Denied(LedgerDenial),
}

/// Immutable command receipt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandReceipt {
    command_id: CommandId,
    request_digest: ContentDigest,
    before: TaskLedgerStreamHead,
    after: TaskLedgerStreamHead,
    outcome: CommandOutcome,
    event_digest: Option<ContentDigest>,
    receipt_digest: ContentDigest,
}

impl CommandReceipt {
    /// Returns the command identity.
    #[must_use]
    pub const fn command_id(&self) -> &CommandId {
        &self.command_id
    }

    /// Returns the request digest used for exact retry.
    #[must_use]
    pub const fn request_digest(&self) -> &ContentDigest {
        &self.request_digest
    }

    /// Returns the head observed when the command first terminated.
    #[must_use]
    pub const fn before(&self) -> &TaskLedgerStreamHead {
        &self.before
    }

    /// Returns the terminal resulting head.
    #[must_use]
    pub const fn after(&self) -> &TaskLedgerStreamHead {
        &self.after
    }

    /// Returns the typed terminal outcome.
    #[must_use]
    pub const fn outcome(&self) -> &CommandOutcome {
        &self.outcome
    }

    /// Returns the appended event digest, if any.
    #[must_use]
    pub const fn event_digest(&self) -> Option<&ContentDigest> {
        self.event_digest.as_ref()
    }

    /// Returns the complete receipt digest.
    #[must_use]
    pub const fn receipt_digest(&self) -> &ContentDigest {
        &self.receipt_digest
    }
}

/// Closed immutable state of a Task Ledger outbox admission.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutboxAdmissionState {
    /// The intent is durably eligible for a later, separately governed claim.
    Admitted,
}

impl OutboxAdmissionState {
    /// Returns the stable wire value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Admitted => "ADMITTED",
        }
    }

    /// Parses one closed wire value.
    ///
    /// # Errors
    ///
    /// Rejects any state other than `ADMITTED`.
    pub fn parse(value: &str) -> Result<Self, LedgerError> {
        match value {
            "ADMITTED" => Ok(Self::Admitted),
            _ => Err(LedgerError::UnknownOutboxState),
        }
    }
}

/// Immutable admission derived only from an appended `EFFECT_INTENT` whose
/// audit outcome is `RECORDED`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutboxAdmission {
    schema_version: String,
    stream_identity: TaskLedgerStreamIdentity,
    stream_id: ContentDigest,
    event_sequence: u64,
    event_digest: ContentDigest,
    command_id: CommandId,
    request_digest: ContentDigest,
    intent_digest: ContentDigest,
    occurred_at: String,
    state: OutboxAdmissionState,
    admission_digest: ContentDigest,
}

impl OutboxAdmission {
    /// Returns the immutable outbox-admission schema version.
    #[must_use]
    pub fn schema_version(&self) -> &str {
        &self.schema_version
    }

    /// Returns the complete stream identity.
    #[must_use]
    pub const fn stream_identity(&self) -> &TaskLedgerStreamIdentity {
        &self.stream_identity
    }

    /// Returns the complete stream digest.
    #[must_use]
    pub const fn stream_id(&self) -> &ContentDigest {
        &self.stream_id
    }

    /// Returns the authoritative event sequence.
    #[must_use]
    pub const fn event_sequence(&self) -> u64 {
        self.event_sequence
    }

    /// Returns the authoritative event digest.
    #[must_use]
    pub const fn event_digest(&self) -> &ContentDigest {
        &self.event_digest
    }

    /// Returns the originating command identifier.
    #[must_use]
    pub const fn command_id(&self) -> &CommandId {
        &self.command_id
    }

    /// Returns the originating command-request digest.
    #[must_use]
    pub const fn request_digest(&self) -> &ContentDigest {
        &self.request_digest
    }

    /// Returns the effect intent digest, equal to the event subject digest.
    #[must_use]
    pub const fn intent_digest(&self) -> &ContentDigest {
        &self.intent_digest
    }

    /// Returns the event's semantic occurrence time.
    #[must_use]
    pub fn occurred_at(&self) -> &str {
        &self.occurred_at
    }

    /// Returns the fixed immutable admission state.
    #[must_use]
    pub const fn state(&self) -> OutboxAdmissionState {
        self.state
    }

    /// Returns the complete outbox-admission digest.
    #[must_use]
    pub const fn admission_digest(&self) -> &ContentDigest {
        &self.admission_digest
    }

    /// Exports the complete admission as an explicitly untrusted row.
    #[must_use]
    pub fn to_untrusted(&self) -> UntrustedOutboxAdmission {
        untrusted_outbox(self)
    }
}

/// Complete immutable commitment to one verified Task Ledger stream snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LedgerCheckpoint {
    stream_id: ContentDigest,
    runtime: RuntimeKind,
    checkpoint_digest: ContentDigest,
}

impl LedgerCheckpoint {
    /// Reconstructs one independently retained checkpoint commitment.
    ///
    /// This constructor does not claim the checkpoint is current. Call
    /// [`verify_untrusted_snapshot_against_checkpoint`] to prove that an
    /// untrusted snapshot matches it.
    #[must_use]
    pub const fn from_retained(
        stream_id: ContentDigest,
        runtime: RuntimeKind,
        checkpoint_digest: ContentDigest,
    ) -> Self {
        Self {
            stream_id,
            runtime,
            checkpoint_digest,
        }
    }

    /// Returns the bound stream digest.
    #[must_use]
    pub const fn stream_id(&self) -> &ContentDigest {
        &self.stream_id
    }

    /// Returns the structural runtime marker bound by the checkpoint.
    #[must_use]
    pub const fn runtime(&self) -> RuntimeKind {
        self.runtime
    }

    /// Returns the complete checkpoint digest.
    #[must_use]
    pub const fn checkpoint_digest(&self) -> &ContentDigest {
        &self.checkpoint_digest
    }
}

/// Raw persisted append request. Every field is untrusted until the containing
/// snapshot passes [`verify_untrusted_snapshot`].
#[derive(Clone, Eq, PartialEq)]
pub struct UntrustedAppendRequest {
    /// Raw command-request schema version.
    pub schema_version: String,
    /// Caller-supplied full expected head.
    pub expected_head: TaskLedgerStreamHead,
    /// Raw command identifier.
    pub command_id: String,
    /// Raw correlation identifier.
    pub correlation_id: String,
    /// Raw caller-supplied timestamp.
    pub occurred_at: String,
    /// Raw closed event-kind wire value.
    pub kind: String,
    /// Raw actor identifier.
    pub actor_id: String,
    /// Raw action identifier.
    pub action: String,
    /// Raw closed event-outcome wire value.
    pub outcome: String,
    /// Raw reason identifier.
    pub reason_code: String,
    /// Authoritative subject digest.
    pub subject_digest: ContentDigest,
    /// Raw diagnostic value; verification sanitizes and bounds it again.
    pub diagnostic: Option<CanonicalValue>,
    /// Typed resource counters when the kind is `RESOURCE_SNAPSHOT`.
    pub resource_snapshot: Option<ResourceCounters>,
}

/// Raw persisted terminal command receipt.
#[derive(Clone, Eq, PartialEq)]
pub struct UntrustedCommandReceipt {
    /// Raw command-receipt schema version.
    pub schema_version: String,
    /// Raw command identifier stored inside the receipt.
    pub command_id: String,
    /// Claimed canonical request digest.
    pub request_digest: ContentDigest,
    /// Claimed head before the terminal result.
    pub before: TaskLedgerStreamHead,
    /// Claimed head after the terminal result.
    pub after: TaskLedgerStreamHead,
    /// `APPENDED` or `DENIED`.
    pub outcome: String,
    /// Stable denial reason, present only for `DENIED`.
    pub denial_reason: Option<String>,
    /// Claimed appended event digest, if any.
    pub event_digest: Option<ContentDigest>,
    /// Claimed complete receipt digest.
    pub receipt_digest: ContentDigest,
}

/// Raw persisted idempotency-key/request/receipt row.
#[derive(Clone, Eq, PartialEq)]
pub struct UntrustedCommandRecord {
    /// Stream-ID component of the durable unique key.
    pub stream_id: ContentDigest,
    /// Command-ID component of the durable unique key.
    pub command_id: String,
    /// Complete canonical request source.
    pub request: UntrustedAppendRequest,
    /// Complete terminal receipt.
    pub receipt: UntrustedCommandReceipt,
    /// Independently retained complete checkpoint before first evaluation.
    pub base_checkpoint: LedgerCheckpoint,
    /// Independently retained complete checkpoint after first evaluation.
    pub result_checkpoint: LedgerCheckpoint,
}

/// Raw persisted event row.
#[derive(Clone, Eq, PartialEq)]
pub struct UntrustedLedgerEvent {
    /// Raw event schema version.
    pub schema_version: String,
    /// Claimed complete stream identity.
    pub stream_identity: TaskLedgerStreamIdentity,
    /// Claimed stream digest.
    pub stream_id: ContentDigest,
    /// Claimed one-based sequence.
    pub sequence: u64,
    /// Claimed predecessor digest.
    pub previous_event_digest: ContentDigest,
    /// Raw command identifier.
    pub command_id: String,
    /// Claimed command-request digest.
    pub request_digest: ContentDigest,
    /// Raw correlation identifier.
    pub correlation_id: String,
    /// Raw caller-supplied timestamp.
    pub occurred_at: String,
    /// Raw closed event-kind wire value.
    pub kind: String,
    /// Raw actor identifier.
    pub actor_id: String,
    /// Raw action identifier.
    pub action: String,
    /// Raw closed event-outcome wire value.
    pub outcome: String,
    /// Raw reason identifier.
    pub reason_code: String,
    /// Authoritative subject digest.
    pub subject_digest: ContentDigest,
    /// Raw diagnostic value.
    pub diagnostic: Option<CanonicalValue>,
    /// Typed resource counters when present.
    pub resource_snapshot: Option<ResourceCounters>,
    /// Claimed resulting resource revision.
    pub resource_revision: u64,
    /// Claimed resulting resource projection digest.
    pub resource_projection_digest: ContentDigest,
    /// Claimed event digest.
    pub event_digest: ContentDigest,
}

/// Raw persisted immutable outbox-admission row.
#[derive(Clone, Eq, PartialEq)]
pub struct UntrustedOutboxAdmission {
    /// Raw outbox-admission schema version.
    pub schema_version: String,
    /// Claimed complete stream identity.
    pub stream_identity: TaskLedgerStreamIdentity,
    /// Claimed stream digest.
    pub stream_id: ContentDigest,
    /// Claimed authoritative event sequence.
    pub event_sequence: u64,
    /// Claimed authoritative event digest.
    pub event_digest: ContentDigest,
    /// Raw originating command identifier.
    pub command_id: String,
    /// Claimed originating command-request digest.
    pub request_digest: ContentDigest,
    /// Claimed effect intent digest.
    pub intent_digest: ContentDigest,
    /// Raw semantic occurrence time.
    pub occurred_at: String,
    /// Raw closed admission-state wire value.
    pub state: String,
    /// Claimed complete admission digest.
    pub admission_digest: ContentDigest,
}

/// Complete untrusted persistence snapshot for one task stream.
///
/// A persistence adapter may construct this representation, but no field is
/// authoritative until [`verify_untrusted_snapshot`] succeeds.
#[derive(Clone, Eq, PartialEq)]
pub struct UntrustedLedgerSnapshot {
    /// Claimed stream identity.
    pub identity: TaskLedgerStreamIdentity,
    /// Claimed current full head.
    pub claimed_head: TaskLedgerStreamHead,
    /// Raw persisted event rows.
    pub events: Vec<UntrustedLedgerEvent>,
    /// Raw persisted command rows, including terminal denials.
    pub commands: Vec<UntrustedCommandRecord>,
    /// Raw persisted immutable outbox-admission rows.
    pub outboxes: Vec<UntrustedOutboxAdmission>,
    /// Claimed current resource projection.
    pub claimed_counters: ResourceCounters,
}

impl fmt::Debug for UntrustedAppendRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UntrustedAppendRequest")
            .field("raw_fields", &"[ELIDED]")
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for UntrustedCommandReceipt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UntrustedCommandReceipt")
            .field("raw_fields", &"[ELIDED]")
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for UntrustedCommandRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UntrustedCommandRecord")
            .field("raw_fields", &"[ELIDED]")
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for UntrustedLedgerEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UntrustedLedgerEvent")
            .field("raw_fields", &"[ELIDED]")
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for UntrustedOutboxAdmission {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UntrustedOutboxAdmission")
            .field("raw_fields", &"[ELIDED]")
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for UntrustedLedgerSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UntrustedLedgerSnapshot")
            .field("event_count", &self.events.len())
            .field("command_count", &self.commands.len())
            .field("outbox_count", &self.outboxes.len())
            .field("raw_fields", &"[ELIDED]")
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug)]
struct StreamState {
    identity: TaskLedgerStreamIdentity,
    head: TaskLedgerStreamHead,
    events: Vec<LedgerEvent>,
    outboxes: Vec<OutboxAdmission>,
    counters: ResourceCounters,
    observation_revision: u64,
    latest_observation: Option<TaskLedgerResourceReceipt>,
}

/// One verified durable command request, terminal receipt, and original
/// checkpoint transition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedCommandRecord {
    request: AppendCommand,
    receipt: CommandReceipt,
    base_checkpoint: LedgerCheckpoint,
    result_checkpoint: LedgerCheckpoint,
}

impl VerifiedCommandRecord {
    /// Returns the complete canonical command request source.
    #[must_use]
    pub const fn request(&self) -> &AppendCommand {
        &self.request
    }

    /// Returns the typed terminal receipt.
    #[must_use]
    pub const fn receipt(&self) -> &CommandReceipt {
        &self.receipt
    }

    /// Returns the complete checkpoint before this command first terminated.
    #[must_use]
    pub const fn base_checkpoint(&self) -> &LedgerCheckpoint {
        &self.base_checkpoint
    }

    /// Returns the complete checkpoint after this command first terminated.
    #[must_use]
    pub const fn result_checkpoint(&self) -> &LedgerCheckpoint {
        &self.result_checkpoint
    }

    /// Exports the complete command as an explicitly untrusted persistence row.
    #[must_use]
    pub fn to_untrusted(&self) -> UntrustedCommandRecord {
        UntrustedCommandRecord {
            stream_id: self.request.expected_head.stream_id().clone(),
            command_id: self.request.command_id.as_str().to_owned(),
            request: untrusted_request(&self.request),
            receipt: untrusted_receipt(&self.receipt),
            base_checkpoint: self.base_checkpoint.clone(),
            result_checkpoint: self.result_checkpoint.clone(),
        }
    }
}

type StoredCommand = VerifiedCommandRecord;

/// Verified immutable view of one complete Task Ledger stream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedStream {
    identity: TaskLedgerStreamIdentity,
    head: TaskLedgerStreamHead,
    events: Vec<LedgerEvent>,
    commands: Vec<VerifiedCommandRecord>,
    outboxes: Vec<OutboxAdmission>,
    counters: ResourceCounters,
    checkpoint: LedgerCheckpoint,
}

impl VerifiedStream {
    /// Constructs a structural zero-position stream for the requested runtime.
    ///
    /// A `Live` marker remains structural only; it does not prove authority or
    /// durability.
    ///
    /// # Errors
    ///
    /// Rejects an invalid identity or canonical hashing failure.
    pub fn vacant(
        identity: TaskLedgerStreamIdentity,
        runtime: RuntimeKind,
    ) -> Result<Self, LedgerError> {
        let head = zero_head_for_runtime(identity.clone(), runtime)?;
        let events = Vec::new();
        let commands = Vec::new();
        let outboxes = Vec::new();
        let counters = zero_counters();
        let checkpoint = build_checkpoint(
            &identity, runtime, &head, &counters, &events, &commands, &outboxes,
        )?;
        Ok(Self {
            identity,
            head,
            events,
            commands,
            outboxes,
            counters,
            checkpoint,
        })
    }

    /// Returns the complete stream identity.
    #[must_use]
    pub const fn identity(&self) -> &TaskLedgerStreamIdentity {
        &self.identity
    }

    /// Returns the verified current full head.
    #[must_use]
    pub const fn head(&self) -> &TaskLedgerStreamHead {
        &self.head
    }

    /// Returns the structural runtime marker.
    #[must_use]
    pub const fn runtime(&self) -> RuntimeKind {
        self.head.runtime()
    }

    /// Returns all verified events.
    #[must_use]
    pub fn events(&self) -> &[LedgerEvent] {
        &self.events
    }

    /// Returns all terminal commands in canonical command-ID order.
    #[must_use]
    pub fn commands(&self) -> &[VerifiedCommandRecord] {
        &self.commands
    }

    /// Returns all immutable admissions in event-sequence order.
    #[must_use]
    pub fn outboxes(&self) -> &[OutboxAdmission] {
        &self.outboxes
    }

    /// Returns replay-derived current resource counters.
    #[must_use]
    pub const fn counters(&self) -> &ResourceCounters {
        &self.counters
    }

    /// Returns the complete verified snapshot commitment.
    #[must_use]
    pub const fn checkpoint(&self) -> &LedgerCheckpoint {
        &self.checkpoint
    }

    /// Looks up one retained typed terminal receipt by command identity.
    #[must_use]
    pub fn receipt(&self, command_id: &CommandId) -> Option<&CommandReceipt> {
        self.commands
            .iter()
            .find(|record| record.request.command_id == *command_id)
            .map(VerifiedCommandRecord::receipt)
    }
}

/// One immutable, non-mutating append decision produced from a complete
/// verified base stream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LedgerAppendPlan {
    base_checkpoint: LedgerCheckpoint,
    next_checkpoint: LedgerCheckpoint,
    command_record: VerifiedCommandRecord,
    exact_retry: bool,
    new_event: Option<LedgerEvent>,
    new_outbox: Option<OutboxAdmission>,
    record_set_digest: ContentDigest,
    next_state: VerifiedStream,
}

impl LedgerAppendPlan {
    /// Returns true only when this is a non-mutating exact retry.
    #[must_use]
    pub const fn is_exact_retry(&self) -> bool {
        self.exact_retry
    }

    /// Returns the typed terminal receipt for new work or exact retry.
    #[must_use]
    pub const fn receipt(&self) -> &CommandReceipt {
        self.command_record.receipt()
    }

    /// Returns the retained command record, including its original checkpoint
    /// transition, for both new work and exact retry.
    #[must_use]
    pub const fn command_record(&self) -> &VerifiedCommandRecord {
        &self.command_record
    }

    /// Returns the newly produced command row, or `None` for exact retry.
    #[must_use]
    pub const fn new_command(&self) -> Option<&VerifiedCommandRecord> {
        if self.exact_retry {
            None
        } else {
            Some(&self.command_record)
        }
    }

    /// Returns the newly appended event, if any.
    #[must_use]
    pub const fn new_event(&self) -> Option<&LedgerEvent> {
        self.new_event.as_ref()
    }

    /// Returns the newly derived outbox admission, if any.
    #[must_use]
    pub const fn new_outbox(&self) -> Option<&OutboxAdmission> {
        self.new_outbox.as_ref()
    }

    /// Returns the complete deterministic persistence record-set digest.
    #[must_use]
    pub const fn record_set_digest(&self) -> &ContentDigest {
        &self.record_set_digest
    }

    /// Returns the complete application-state checkpoint that must still hold.
    #[must_use]
    pub const fn base_checkpoint(&self) -> &LedgerCheckpoint {
        &self.base_checkpoint
    }

    /// Returns the resulting application-state checkpoint. For exact retry it
    /// equals the current base; the command record retains its historical pair.
    #[must_use]
    pub const fn next_checkpoint(&self) -> &LedgerCheckpoint {
        &self.next_checkpoint
    }

    /// Returns the complete planned next verified state.
    #[must_use]
    pub const fn next_state(&self) -> &VerifiedStream {
        &self.next_state
    }
}

/// Plans one terminal append without mutating the verified input stream.
///
/// Exact command retry is classified before stale-head evaluation.
///
/// # Errors
///
/// Rejects changed command-ID reuse, a cross-stream/runtime command, invalid
/// resource progression, checkpoint disagreement, or canonical hash failure.
#[allow(clippy::too_many_lines)]
pub fn plan_append(
    current: &VerifiedStream,
    command: AppendCommand,
) -> Result<LedgerAppendPlan, LedgerError> {
    validate_verified_checkpoint(current)?;
    validate_full_head(&command.expected_head, current.runtime())?;
    if command.expected_head.identity() != current.identity()
        || command.expected_head.stream_id() != current.head.stream_id()
    {
        return Err(LedgerError::InvalidStreamHead);
    }
    let computed_request_digest = request_digest(&command)?;
    if let Some(retained) = current
        .commands
        .iter()
        .find(|record| record.request.command_id == command.command_id)
    {
        if retained.receipt.request_digest() != &computed_request_digest {
            return Err(LedgerError::CommandIdReuse);
        }
        let retained_event = retained.receipt.event_digest().map(|event_digest| {
            current
                .events
                .iter()
                .find(|event| event.event_digest() == event_digest)
                .ok_or(LedgerError::OrphanReceipt)
        });
        let retained_event = retained_event.transpose()?;
        let retained_outbox = current
            .outboxes
            .iter()
            .find(|outbox| outbox.command_id() == retained.request.command_id());
        let record_set_digest = build_record_set_digest(retained, retained_event, retained_outbox)?;
        return Ok(LedgerAppendPlan {
            base_checkpoint: current.checkpoint.clone(),
            next_checkpoint: current.checkpoint.clone(),
            command_record: retained.clone(),
            exact_retry: true,
            new_event: None,
            new_outbox: None,
            record_set_digest,
            next_state: current.clone(),
        });
    }

    let requested_profile = if command.kind == LedgerEventKind::TaskCreated {
        classify_task_created_action(command.action.as_str())?
    } else {
        None
    };
    validate_task_created_profile_subject(
        current.identity(),
        requested_profile,
        &command.subject_digest,
    )?;
    let external_adoption = command.kind == LedgerEventKind::ExternalVerifiedResultAdopted;
    match current.identity().subject_kind() {
        TaskLedgerSubjectKind::TaskSpec
            if requested_profile == Some(TaskCreatedProfile::GeneralTaskIntakeV1) =>
        {
            return Err(LedgerError::InvalidStreamHead);
        }
        TaskLedgerSubjectKind::GeneralTaskIntake
            if requested_profile != Some(TaskCreatedProfile::GeneralTaskIntakeV1)
                && !external_adoption =>
        {
            return Err(LedgerError::GeneralTaskIntakeCreateOnly);
        }
        TaskLedgerSubjectKind::TaskSpec | TaskLedgerSubjectKind::GeneralTaskIntake => {}
    }

    if command.kind == LedgerEventKind::TaskCreated
        && requested_profile.is_some()
        && !current.events.is_empty()
    {
        return Err(LedgerError::InvalidAutonomyReceipt);
    }
    if current
        .events
        .first()
        .map(classify_task_created_profile)
        .transpose()?
        .flatten()
        == Some(TaskCreatedProfile::GeneralTaskIntakeV1)
        && !external_adoption
    {
        return Err(LedgerError::GeneralTaskIntakeCreateOnly);
    }
    if external_adoption {
        let valid_terminal = current.identity().subject_kind()
            == TaskLedgerSubjectKind::GeneralTaskIntake
            && current.events.len() == 1
            && classify_task_created_profile(&current.events[0])?
                == Some(TaskCreatedProfile::GeneralTaskIntakeV1)
            && command.expected_head.sequence() == 1
            && command.action.as_str() == EXTERNAL_RESULT_ADOPTION_ACTION
            && command.reason_code.as_str() == EXTERNAL_RESULT_ADOPTION_REASON
            && command.outcome == LedgerOutcome::Recorded
            && command.diagnostic.is_none()
            && command.resource_snapshot.is_none();
        if !valid_terminal {
            return Err(LedgerError::ExternalVerifiedResultAdoptionMismatch);
        }
    }
    if current.events.len() == 1
        && classify_task_created_profile(&current.events[0])?
            .is_some_and(TaskCreatedProfile::requires_autonomy_receipt)
        && command.kind != LedgerEventKind::AutonomyReceiptRecorded
    {
        return Err(LedgerError::InvalidAutonomyReceipt);
    }
    validate_autonomy_order(command.kind, &current.events)?;

    let before = current.head.clone();
    let (after, receipt, new_event, new_outbox, next_counters) = if before != command.expected_head
    {
        let receipt = command_receipt(
            command.command_id.clone(),
            computed_request_digest,
            before.clone(),
            before.clone(),
            CommandOutcome::Denied(LedgerDenial::StaleHead),
            None,
        )?;
        (
            before.clone(),
            receipt,
            None,
            None,
            current.counters.clone(),
        )
    } else if let Some(sequence) = before.sequence().checked_add(1) {
        let (next_counters, resource_revision, resource_projection_digest) =
            next_resource_projection(&before, &current.counters, &command)?;
        let event = build_event(
            &command,
            sequence,
            computed_request_digest,
            resource_revision,
            resource_projection_digest.clone(),
        )?;
        let after = build_head(
            current.runtime(),
            before.identity().clone(),
            before.stream_id().clone(),
            sequence,
            event.event_digest.clone(),
            resource_revision,
            resource_projection_digest,
        )?;
        let receipt = command_receipt(
            command.command_id.clone(),
            event.request_digest.clone(),
            before,
            after.clone(),
            CommandOutcome::Appended,
            Some(event.event_digest.clone()),
        )?;
        let outbox = derive_outbox_admission(&event)?;
        (after, receipt, Some(event), outbox, next_counters)
    } else {
        let receipt = command_receipt(
            command.command_id.clone(),
            computed_request_digest,
            before.clone(),
            before.clone(),
            CommandOutcome::Denied(LedgerDenial::SequenceOverflow),
            None,
        )?;
        (
            before.clone(),
            receipt,
            None,
            None,
            current.counters.clone(),
        )
    };

    let mut command_record = VerifiedCommandRecord {
        request: command,
        receipt,
        base_checkpoint: current.checkpoint.clone(),
        result_checkpoint: current.checkpoint.clone(),
    };
    let mut commands = current.commands.clone();
    commands.push(command_record.clone());
    canonicalize_commands(&mut commands);
    let mut events = current.events.clone();
    if let Some(event) = new_event.as_ref() {
        events.push(event.clone());
    }
    let mut outboxes = current.outboxes.clone();
    if let Some(outbox) = new_outbox.as_ref() {
        outboxes.push(outbox.clone());
        canonicalize_outboxes(&mut outboxes);
    }
    let next_checkpoint = build_checkpoint(
        &current.identity,
        current.runtime(),
        &after,
        &next_counters,
        &events,
        &commands,
        &outboxes,
    )?;
    command_record.result_checkpoint = next_checkpoint.clone();
    let position = commands
        .iter()
        .position(|record| record.request.command_id == command_record.request.command_id)
        .ok_or(LedgerError::CheckpointMismatch)?;
    commands[position] = command_record.clone();
    let next_state = VerifiedStream {
        identity: current.identity.clone(),
        head: after,
        events,
        commands,
        outboxes,
        counters: next_counters,
        checkpoint: next_checkpoint.clone(),
    };
    let record_set_digest =
        build_record_set_digest(&command_record, new_event.as_ref(), new_outbox.as_ref())?;
    Ok(LedgerAppendPlan {
        base_checkpoint: current.checkpoint.clone(),
        next_checkpoint,
        command_record,
        exact_retry: false,
        new_event,
        new_outbox,
        record_set_digest,
        next_state,
    })
}

/// Builds and plans the only canonical TASK-050 autonomy receipt append.
///
/// Task Ledger independently reclassifies the supplied recommendation, binds
/// authority to the verified stream, computes both canonical digests, and uses
/// a private event constructor. Generic append cannot supply the subject digest.
///
/// # Errors
///
/// Rejects a non-required profile, wrong event order, recommendation drift,
/// missing/unexpected/stale writer authority, or canonical hashing failure.
pub fn plan_autonomy_receipt_append(
    current: &VerifiedStream,
    metadata: AutonomyAppendMetadata,
    intent: AutonomyIntent,
    authority: AutonomyAuthorityEvidence,
) -> Result<AutonomyReceiptAppendPlan, LedgerError> {
    validate_verified_checkpoint(current)?;
    if current.events().len() != 1
        || !classify_task_created_profile(&current.events()[0])?
            .is_some_and(TaskCreatedProfile::requires_autonomy_receipt)
        || intent.risk_class != AutonomyRiskClass::R0
    {
        return Err(LedgerError::InvalidAutonomyReceipt);
    }
    if derive_autonomy_recommendation(intent) != intent.recommendation {
        return Err(LedgerError::AutonomyRecommendationMismatch);
    }
    validate_autonomy_authority(current.identity(), intent.recommendation, &authority)?;
    let writer_lease_head_digest =
        autonomy_writer_head_digest(authority.writer_authority.as_ref())?;
    let authority_value = autonomy_authority_value(
        current.identity(),
        &authority,
        writer_lease_head_digest.as_ref(),
    )?;
    let authority_digest = hash_value_at_version(
        AUTONOMY_AUTHORITY_DOMAIN,
        AUTONOMY_HASH_VERSION,
        &authority_value,
    )?;
    let receipt_value = autonomy_receipt_value(current.identity(), intent, &authority_digest)?;
    let receipt_digest = hash_value_at_version(
        AUTONOMY_RECEIPT_DOMAIN,
        AUTONOMY_HASH_VERSION,
        &receipt_value,
    )?;
    let writer_authority = authority.writer_authority.clone();
    let command = AppendCommand::from_fields(
        current.head().clone(),
        metadata.command_id,
        metadata.correlation_id,
        metadata.occurred_at,
        LedgerEventKind::AutonomyReceiptRecorded,
        metadata.actor_id,
        ActionId::new("RECORD_AUTONOMY_RECEIPT_V1")?,
        LedgerOutcome::Recorded,
        ReasonCode::new("AUTONOMY_DECISION_RECORDED")?,
        receipt_digest.clone(),
        None,
        None,
        AppendConstruction::VerifiedAutonomy,
    )?;
    let append_plan = plan_append(current, command)?;
    let event = append_plan
        .new_event()
        .ok_or(LedgerError::InvalidAutonomyReceipt)?;
    if event.sequence() != 2 || event.subject_digest() != &receipt_digest {
        return Err(LedgerError::InvalidAutonomyReceipt);
    }
    let receipt = VerifiedAutonomyReceipt {
        stream_id: current.head().stream_id().clone(),
        event_sequence: event.sequence(),
        event_digest: event.event_digest().clone(),
        intent,
        process_start_authority_digest: authority.process_start_authority_digest,
        ingress_profile_adapter_commitment: authority.ingress_profile_adapter_commitment,
        store_authority_head_digest: authority.store_authority_head_digest,
        writer_lease_receipt_digest: writer_authority
            .as_ref()
            .map(|writer| writer.receipt_digest().clone()),
        writer_lease_head_digest,
        writer_fencing_token: writer_authority
            .as_ref()
            .map(|writer| writer.identity().fencing_token().get()),
        authority_digest,
        receipt_digest,
    };
    Ok(AutonomyReceiptAppendPlan {
        append_plan,
        receipt,
        writer_authority,
    })
}

/// Verifies that a candidate is the exact canonical retry of one retained
/// autonomy receipt.
///
/// Task Ledger owns this comparison because only it owns the canonical Writer
/// head, authority, receipt, and stream-binding commitments. Adapters must not
/// approximate exact retry by comparing a subset of persistence scalars.
///
/// # Errors
///
/// Rejects binding drift, recommendation drift, missing or unexpected Writer
/// authority, any substitution in the Store-asserted Writer tuple, or any
/// canonical authority/receipt commitment mismatch.
pub fn verify_exact_autonomy_receipt_retry(
    identity: &TaskLedgerStreamIdentity,
    existing: &VerifiedAutonomyReceipt,
    candidate_intent: AutonomyIntent,
    candidate_authority: &AutonomyAuthorityEvidence,
) -> Result<(), LedgerError> {
    validate_stream_identity(identity)?;
    if candidate_intent.risk_class != AutonomyRiskClass::R0
        || derive_autonomy_recommendation(candidate_intent) != candidate_intent.recommendation
    {
        return Err(LedgerError::InvalidAutonomyReceipt);
    }
    validate_autonomy_authority(
        identity,
        candidate_intent.recommendation,
        candidate_authority,
    )?;

    let stream_id = hash_value("lattice.task-ledger.stream-id", &identity_value(identity))?;
    let writer_lease_head_digest =
        autonomy_writer_head_digest(candidate_authority.writer_authority.as_ref())?;
    let authority_digest = hash_value_at_version(
        AUTONOMY_AUTHORITY_DOMAIN,
        AUTONOMY_HASH_VERSION,
        &autonomy_authority_value(
            identity,
            candidate_authority,
            writer_lease_head_digest.as_ref(),
        )?,
    )?;
    let receipt_digest = hash_value_at_version(
        AUTONOMY_RECEIPT_DOMAIN,
        AUTONOMY_HASH_VERSION,
        &autonomy_receipt_value(identity, candidate_intent, &authority_digest)?,
    )?;
    let writer = candidate_authority.writer_authority.as_ref();

    if existing.stream_id != stream_id
        || existing.event_sequence != 2
        || existing.intent != candidate_intent
        || existing.process_start_authority_digest
            != candidate_authority.process_start_authority_digest
        || existing.ingress_profile_adapter_commitment
            != candidate_authority.ingress_profile_adapter_commitment
        || existing.store_authority_head_digest != candidate_authority.store_authority_head_digest
        || existing.writer_lease_receipt_digest.as_ref()
            != writer.map(WriterLeaseAuthorityHead::receipt_digest)
        || existing.writer_lease_head_digest != writer_lease_head_digest
        || existing.writer_fencing_token
            != writer.map(|authority| authority.identity().fencing_token().get())
        || existing.authority_digest != authority_digest
        || existing.receipt_digest != receipt_digest
    {
        return Err(LedgerError::InvalidAutonomyReceipt);
    }
    Ok(())
}

/// Verifies complete untrusted persistence rows and classifies the stream's
/// closed autonomy profile state.
///
/// # Errors
///
/// Rejects row cardinality drift, profile/event disagreement, substituted
/// scalars, non-canonical decisions, incomplete writer tuples, or hash drift.
pub fn verify_untrusted_autonomy_receipt_rows(
    stream: &VerifiedStream,
    rows: &[UntrustedAutonomyReceiptRow],
) -> Result<VerifiedAutonomyReceiptState, LedgerError> {
    validate_verified_checkpoint(stream)?;
    if rows.len() > 1 {
        return Err(LedgerError::InvalidAutonomyReceipt);
    }
    let autonomy_events = stream
        .events()
        .iter()
        .filter(|event| event.kind() == LedgerEventKind::AutonomyReceiptRecorded)
        .collect::<Vec<_>>();
    let profile = stream
        .events()
        .first()
        .map(classify_task_created_profile)
        .transpose()?
        .flatten();
    match profile {
        None => {
            if rows.is_empty() && autonomy_events.is_empty() {
                Ok(VerifiedAutonomyReceiptState::NotApplicable)
            } else {
                Err(LedgerError::InvalidAutonomyReceipt)
            }
        }
        Some(TaskCreatedProfile::HistoricalAutonomyOptionalV1) => match rows {
            [] if autonomy_events.is_empty() => {
                Ok(VerifiedAutonomyReceiptState::HistoricalOptional(None))
            }
            [row] => verify_one_untrusted_autonomy_receipt(stream, row)
                .map(|receipt| VerifiedAutonomyReceiptState::HistoricalOptional(Some(receipt))),
            _ => Err(LedgerError::InvalidAutonomyReceipt),
        },
        Some(
            TaskCreatedProfile::AutonomyReceiptRequiredV1
            | TaskCreatedProfile::ManagedGeneralTaskV1,
        ) => match rows {
            [] if stream.events().len() == 1 && autonomy_events.is_empty() => {
                Ok(VerifiedAutonomyReceiptState::PendingRequiredReceipt)
            }
            [row] => verify_one_untrusted_autonomy_receipt(stream, row)
                .map(VerifiedAutonomyReceiptState::RequiredComplete),
            _ => Err(LedgerError::InvalidAutonomyReceipt),
        },
        Some(TaskCreatedProfile::GeneralTaskIntakeV1) => {
            if rows.is_empty()
                && autonomy_events.is_empty()
                && (stream.events().len() == 1
                    || valid_general_external_adoption_events(stream.events()))
            {
                Ok(VerifiedAutonomyReceiptState::NotApplicable)
            } else {
                Err(LedgerError::GeneralTaskIntakeCreateOnly)
            }
        }
    }
}

fn verify_one_untrusted_autonomy_receipt(
    stream: &VerifiedStream,
    row: &UntrustedAutonomyReceiptRow,
) -> Result<VerifiedAutonomyReceipt, LedgerError> {
    let events = stream.events();
    if events.len() < 2
        || events
            .iter()
            .filter(|event| event.kind() == LedgerEventKind::AutonomyReceiptRecorded)
            .count()
            != 1
        || classify_task_created_profile(&events[0])?.is_none()
        || events[1].kind() != LedgerEventKind::AutonomyReceiptRecorded
        || row.stream_id != *stream.head().stream_id()
        || row.event_sequence != 2
        || row.event_sequence != events[1].sequence()
        || row.event_digest != *events[1].event_digest()
        || row.receipt_digest != *events[1].subject_digest()
        || row.receipt_schema_version != AUTONOMY_RECEIPT_SCHEMA
        || row.intent_version != AUTONOMY_HASH_VERSION
        || row.observed_task_state != AutonomyObservedTaskState::Draft.as_str()
        || row.authority_mode != AUTONOMY_AUTHORITY_MODE
        || [
            &row.process_start_authority_digest,
            &row.ingress_profile_adapter_commitment,
            &row.store_authority_head_digest,
        ]
        .into_iter()
        .any(is_zero_digest)
    {
        return Err(LedgerError::InvalidAutonomyReceipt);
    }
    let recommendation = parse_autonomy_recommendation(row)?;
    let intent = AutonomyIntent {
        task_kind: parse_autonomy_task_kind(&row.task_kind)?,
        risk_class: parse_autonomy_risk_class(&row.risk_class)?,
        execution_preapproved: row.execution_preapproved,
        requires_new_authority: row.requires_new_authority,
        irreversible_or_high_risk: row.irreversible_or_high_risk,
        observed_task_state: AutonomyObservedTaskState::Draft,
        recommendation,
    };
    if intent.risk_class != AutonomyRiskClass::R0
        || derive_autonomy_recommendation(intent) != recommendation
    {
        return Err(LedgerError::InvalidAutonomyReceipt);
    }
    let writer_complete = row.writer_lease_receipt_digest.is_some()
        && row.writer_lease_head_digest.is_some()
        && row.writer_fencing_token.is_some();
    let writer_empty = row.writer_lease_receipt_digest.is_none()
        && row.writer_lease_head_digest.is_none()
        && row.writer_fencing_token.is_none();
    if !valid_autonomy_writer_scalar_tuple(
        row.writer_lease_receipt_digest.as_ref(),
        row.writer_lease_head_digest.as_ref(),
        row.writer_fencing_token,
    ) || !matches!(
        (recommendation, writer_complete, writer_empty),
        (AutonomyRecommendation::Proceed { .. }, true, false)
            | (AutonomyRecommendation::AskUser { .. }, false, true)
    ) {
        return Err(LedgerError::InvalidAutonomyReceipt);
    }
    let authority_value = autonomy_authority_value_from_scalars(
        stream.identity(),
        &row.process_start_authority_digest,
        &row.ingress_profile_adapter_commitment,
        &row.store_authority_head_digest,
        row.writer_lease_receipt_digest.as_ref(),
        row.writer_lease_head_digest.as_ref(),
        row.writer_fencing_token,
    )?;
    let authority_digest = hash_value_at_version(
        AUTONOMY_AUTHORITY_DOMAIN,
        AUTONOMY_HASH_VERSION,
        &authority_value,
    )?;
    if authority_digest != row.authority_digest {
        return Err(LedgerError::InvalidAutonomyReceipt);
    }
    let receipt_value = autonomy_receipt_value(stream.identity(), intent, &authority_digest)?;
    let receipt_digest = hash_value_at_version(
        AUTONOMY_RECEIPT_DOMAIN,
        AUTONOMY_HASH_VERSION,
        &receipt_value,
    )?;
    if receipt_digest != row.receipt_digest {
        return Err(LedgerError::InvalidAutonomyReceipt);
    }
    Ok(VerifiedAutonomyReceipt {
        stream_id: row.stream_id.clone(),
        event_sequence: row.event_sequence,
        event_digest: row.event_digest.clone(),
        intent,
        process_start_authority_digest: row.process_start_authority_digest.clone(),
        ingress_profile_adapter_commitment: row.ingress_profile_adapter_commitment.clone(),
        store_authority_head_digest: row.store_authority_head_digest.clone(),
        writer_lease_receipt_digest: row.writer_lease_receipt_digest.clone(),
        writer_lease_head_digest: row.writer_lease_head_digest.clone(),
        writer_fencing_token: row.writer_fencing_token,
        authority_digest,
        receipt_digest,
    })
}

fn valid_autonomy_writer_scalar_tuple(
    receipt_digest: Option<&ContentDigest>,
    head_digest: Option<&ContentDigest>,
    fencing_token: Option<u64>,
) -> bool {
    match (receipt_digest, head_digest, fencing_token) {
        (None, None, None) => true,
        (Some(receipt), Some(head), Some(token)) => {
            !is_zero_digest(receipt)
                && !is_zero_digest(head)
                && lattice_contracts::FencingToken::new(token).is_ok()
        }
        _ => false,
    }
}

fn parse_autonomy_task_kind(value: &str) -> Result<AutonomyTaskKind, LedgerError> {
    match value {
        "FEATURE" => Ok(AutonomyTaskKind::Feature),
        "BUG_FIX" => Ok(AutonomyTaskKind::BugFix),
        "CONFIGURATION" => Ok(AutonomyTaskKind::Configuration),
        "RESEARCH" => Ok(AutonomyTaskKind::Research),
        _ => Err(LedgerError::InvalidAutonomyReceipt),
    }
}

fn parse_autonomy_risk_class(value: &str) -> Result<AutonomyRiskClass, LedgerError> {
    match value {
        "R0" => Ok(AutonomyRiskClass::R0),
        "R1" => Ok(AutonomyRiskClass::R1),
        "R2" => Ok(AutonomyRiskClass::R2),
        "R3" => Ok(AutonomyRiskClass::R3),
        _ => Err(LedgerError::InvalidAutonomyReceipt),
    }
}

fn parse_autonomy_reason(value: &str) -> Result<AutonomyDecisionReason, LedgerError> {
    match value {
        "ROUTINE_AUTHORIZED" => Ok(AutonomyDecisionReason::RoutineAuthorized),
        "NEW_USER_DECISION" => Ok(AutonomyDecisionReason::NewUserDecision),
        "NEW_AUTHORITY" => Ok(AutonomyDecisionReason::NewAuthority),
        "HIGH_RISK_OR_IRREVERSIBLE" => Ok(AutonomyDecisionReason::HighRiskOrIrreversible),
        _ => Err(LedgerError::InvalidAutonomyReceipt),
    }
}

fn parse_autonomy_recommendation(
    row: &UntrustedAutonomyReceiptRow,
) -> Result<AutonomyRecommendation, LedgerError> {
    let reason = parse_autonomy_reason(&row.decision_reason)?;
    match (
        row.disposition.as_str(),
        row.model.as_deref(),
        row.verification.as_deref(),
    ) {
        ("PROCEED", Some(model), Some(verification)) => Ok(AutonomyRecommendation::Proceed {
            model: match model {
                "GOVERNED_CODEX_WRITER" => AutonomyModel::GovernedCodexWriter,
                "NO_MODEL" => AutonomyModel::NoModel,
                _ => return Err(LedgerError::InvalidAutonomyReceipt),
            },
            verification: match verification {
                "FOCUSED_CHECKS" => AutonomyVerification::FocusedChecks,
                "BUILD_AND_FOCUSED_CHECKS" => AutonomyVerification::BuildAndFocusedChecks,
                "READ_ONLY_EVIDENCE" => AutonomyVerification::ReadOnlyEvidence,
                _ => return Err(LedgerError::InvalidAutonomyReceipt),
            },
            reason,
        }),
        ("ASK_USER", None, None) => Ok(AutonomyRecommendation::AskUser { reason }),
        _ => Err(LedgerError::InvalidAutonomyReceipt),
    }
}

fn derive_autonomy_recommendation(intent: AutonomyIntent) -> AutonomyRecommendation {
    if intent.requires_new_authority {
        AutonomyRecommendation::AskUser {
            reason: AutonomyDecisionReason::NewAuthority,
        }
    } else if intent.irreversible_or_high_risk || intent.risk_class == AutonomyRiskClass::R3 {
        AutonomyRecommendation::AskUser {
            reason: AutonomyDecisionReason::HighRiskOrIrreversible,
        }
    } else if !intent.execution_preapproved {
        AutonomyRecommendation::AskUser {
            reason: AutonomyDecisionReason::NewUserDecision,
        }
    } else {
        let model = match intent.task_kind {
            AutonomyTaskKind::Feature | AutonomyTaskKind::BugFix => {
                AutonomyModel::GovernedCodexWriter
            }
            AutonomyTaskKind::Configuration | AutonomyTaskKind::Research => AutonomyModel::NoModel,
        };
        let verification = if intent.task_kind == AutonomyTaskKind::Research {
            AutonomyVerification::ReadOnlyEvidence
        } else if intent.risk_class == AutonomyRiskClass::R2 {
            AutonomyVerification::BuildAndFocusedChecks
        } else {
            AutonomyVerification::FocusedChecks
        };
        AutonomyRecommendation::Proceed {
            model,
            verification,
            reason: AutonomyDecisionReason::RoutineAuthorized,
        }
    }
}

fn validate_autonomy_authority(
    identity: &TaskLedgerStreamIdentity,
    recommendation: AutonomyRecommendation,
    authority: &AutonomyAuthorityEvidence,
) -> Result<(), LedgerError> {
    match (recommendation, authority.writer_authority.as_ref()) {
        (AutonomyRecommendation::Proceed { .. }, Some(writer))
            if writer.runtime() == RuntimeKind::Live
                && writer.status() == WriterLeaseStatus::Active
                && writer.runtime_admission() == RuntimeAdmissionMode::Active
                && writer_binding_matches(identity, writer) =>
        {
            Ok(())
        }
        (AutonomyRecommendation::AskUser { .. }, None) => Ok(()),
        _ => Err(LedgerError::InvalidAutonomyReceipt),
    }
}

fn writer_binding_matches(
    identity: &TaskLedgerStreamIdentity,
    writer: &WriterLeaseAuthorityHead,
) -> bool {
    let writer = writer.identity();
    writer.project_id() == identity.project_id()
        && writer.project_snapshot_id() == identity.project_snapshot_id()
        && writer.task_id() == identity.task_id()
        && writer.task_revision() == identity.task_revision()
        && identity
            .task_spec_digest()
            .is_some_and(|digest| writer.task_spec_digest() == digest)
}

fn autonomy_writer_head_digest(
    writer: Option<&WriterLeaseAuthorityHead>,
) -> Result<Option<ContentDigest>, LedgerError> {
    writer
        .map(|writer| {
            let identity = writer.identity();
            hash_value_at_version(
                "lattice.autonomy-writer-lease-head",
                AUTONOMY_HASH_VERSION,
                &object(vec![
                    ("project_id", text(identity.project_id().as_str())),
                    (
                        "project_snapshot_id",
                        text(identity.project_snapshot_id().as_str()),
                    ),
                    ("task_id", text(identity.task_id().as_str())),
                    ("task_revision", text(identity.task_revision())),
                    (
                        "task_spec_digest",
                        text(identity.task_spec_digest().as_str()),
                    ),
                    ("attempt_id", text(identity.attempt_id().as_str())),
                    ("lease_id", text(identity.lease_id())),
                    ("lease_holder_id", text(identity.lease_holder_id())),
                    ("worktree_id", text(identity.worktree_id())),
                    (
                        "holder_process_id",
                        text(identity.holder_process_id().get().to_string()),
                    ),
                    (
                        "holder_process_start_identity",
                        text(identity.holder_process_start_identity().as_str()),
                    ),
                    ("daemon_instance_id", text(identity.daemon_instance_id())),
                    (
                        "daemon_epoch",
                        text(identity.daemon_epoch().get().to_string()),
                    ),
                    (
                        "fencing_token",
                        text(identity.fencing_token().get().to_string()),
                    ),
                    ("receipt_digest", text(writer.receipt_digest().as_str())),
                ]),
            )
        })
        .transpose()
}

fn autonomy_binding_value(
    identity: &TaskLedgerStreamIdentity,
) -> Result<CanonicalValue, LedgerError> {
    let task_spec_digest = identity
        .task_spec_digest()
        .ok_or(LedgerError::InvalidAutonomyReceipt)?;
    Ok(object(vec![
        ("project_id", text(identity.project_id().as_str())),
        (
            "project_snapshot_id",
            text(identity.project_snapshot_id().as_str()),
        ),
        ("task_id", text(identity.task_id().as_str())),
        ("task_revision", text(identity.task_revision())),
        ("task_spec_digest", text(task_spec_digest.as_str())),
    ]))
}

fn autonomy_authority_value(
    identity: &TaskLedgerStreamIdentity,
    authority: &AutonomyAuthorityEvidence,
    writer_head_digest: Option<&ContentDigest>,
) -> Result<CanonicalValue, LedgerError> {
    let writer = authority.writer_authority.as_ref();
    autonomy_authority_value_from_scalars(
        identity,
        &authority.process_start_authority_digest,
        &authority.ingress_profile_adapter_commitment,
        &authority.store_authority_head_digest,
        writer.map(WriterLeaseAuthorityHead::receipt_digest),
        writer_head_digest,
        writer.map(|writer| writer.identity().fencing_token().get()),
    )
}

fn autonomy_authority_value_from_scalars(
    identity: &TaskLedgerStreamIdentity,
    process_start_authority_digest: &ContentDigest,
    ingress_profile_adapter_commitment: &ContentDigest,
    store_authority_head_digest: &ContentDigest,
    writer_lease_receipt_digest: Option<&ContentDigest>,
    writer_lease_head_digest: Option<&ContentDigest>,
    writer_fencing_token: Option<u64>,
) -> Result<CanonicalValue, LedgerError> {
    Ok(object(vec![
        ("binding", autonomy_binding_value(identity)?),
        ("authority_mode", text(AUTONOMY_AUTHORITY_MODE)),
        (
            "process_start_authority_digest",
            text(process_start_authority_digest.as_str()),
        ),
        (
            "ingress_profile_adapter_commitment",
            text(ingress_profile_adapter_commitment.as_str()),
        ),
        (
            "store_authority_head_digest",
            text(store_authority_head_digest.as_str()),
        ),
        ("policy_decision_receipt_digest", CanonicalValue::Null),
        ("policy_owner_head_digest", CanonicalValue::Null),
        ("approval_receipt_digest", CanonicalValue::Null),
        ("approval_owner_head_digest", CanonicalValue::Null),
        (
            "writer_lease_receipt_digest",
            optional(writer_lease_receipt_digest.map(|digest| text(digest.as_str()))),
        ),
        (
            "writer_lease_head_digest",
            optional(writer_lease_head_digest.map(|digest| text(digest.as_str()))),
        ),
        (
            "writer_fencing_token",
            optional(writer_fencing_token.map(|token| text(token.to_string()))),
        ),
    ]))
}

fn autonomy_receipt_value(
    identity: &TaskLedgerStreamIdentity,
    intent: AutonomyIntent,
    authority_digest: &ContentDigest,
) -> Result<CanonicalValue, LedgerError> {
    Ok(object(vec![
        ("schema_version", text(AUTONOMY_RECEIPT_SCHEMA)),
        ("binding", autonomy_binding_value(identity)?),
        (
            "intent",
            object(vec![
                ("version", text(AUTONOMY_HASH_VERSION)),
                ("task_kind", text(intent.task_kind.as_str())),
                ("risk_class", text(intent.risk_class.as_str())),
                (
                    "execution_preapproved",
                    CanonicalValue::Bool(intent.execution_preapproved),
                ),
                (
                    "requires_new_authority",
                    CanonicalValue::Bool(intent.requires_new_authority),
                ),
                (
                    "irreversible_or_high_risk",
                    CanonicalValue::Bool(intent.irreversible_or_high_risk),
                ),
            ]),
        ),
        (
            "observed_task_state",
            text(intent.observed_task_state.as_str()),
        ),
        (
            "decision",
            object(vec![
                ("disposition", text(intent.recommendation.disposition())),
                ("reason", text(intent.recommendation.reason().as_str())),
                (
                    "model",
                    optional(
                        intent
                            .recommendation
                            .model()
                            .map(|model| text(model.as_str())),
                    ),
                ),
                (
                    "verification",
                    optional(
                        intent
                            .recommendation
                            .verification()
                            .map(|value| text(value.as_str())),
                    ),
                ),
            ]),
        ),
        ("authority_digest", text(authority_digest.as_str())),
    ]))
}

/// Applies one indivisible pure plan only while its complete base checkpoint
/// remains current.
///
/// # Errors
///
/// Rejects a stale/tampered base or an internally inconsistent planned state.
pub fn apply_append_plan(
    current: &VerifiedStream,
    plan: &LedgerAppendPlan,
) -> Result<VerifiedStream, LedgerError> {
    validate_verified_checkpoint(current)?;
    validate_verified_checkpoint(&plan.next_state)?;
    if current.checkpoint != plan.base_checkpoint
        || plan.next_state.checkpoint != plan.next_checkpoint
    {
        return Err(LedgerError::CheckpointMismatch);
    }
    Ok(plan.next_state.clone())
}

/// Exports one complete verified stream through the untrusted persistence
/// boundary.
#[must_use]
pub fn export_untrusted_snapshot(stream: &VerifiedStream) -> UntrustedLedgerSnapshot {
    UntrustedLedgerSnapshot {
        identity: stream.identity.clone(),
        claimed_head: stream.head.clone(),
        events: stream
            .events
            .iter()
            .map(LedgerEvent::to_untrusted)
            .collect(),
        commands: stream
            .commands
            .iter()
            .map(VerifiedCommandRecord::to_untrusted)
            .collect(),
        outboxes: stream
            .outboxes
            .iter()
            .map(OutboxAdmission::to_untrusted)
            .collect(),
        claimed_counters: stream.counters.clone(),
    }
}

/// Verifies untrusted persistence rows against an independently retained
/// complete checkpoint.
///
/// # Errors
///
/// Returns the underlying replay error or [`LedgerError::CheckpointMismatch`].
pub fn verify_untrusted_snapshot_against_checkpoint(
    snapshot: &UntrustedLedgerSnapshot,
    retained_checkpoint: &LedgerCheckpoint,
) -> Result<VerifiedStream, LedgerError> {
    let verified = verify_untrusted_snapshot(snapshot)?;
    if verified.checkpoint() != retained_checkpoint {
        return Err(LedgerError::CheckpointMismatch);
    }
    Ok(verified)
}

/// Deterministic in-memory Task Ledger used only for characterization.
#[derive(Debug, Default)]
pub struct FakeTaskLedger {
    streams: BTreeMap<String, StreamState>,
    commands: BTreeMap<(String, String), StoredCommand>,
}

impl FakeTaskLedger {
    /// Constructs an empty, visibly non-durable fake.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            streams: BTreeMap::new(),
            commands: BTreeMap::new(),
        }
    }

    /// Computes the exact zero head for one complete stream identity.
    ///
    /// # Errors
    ///
    /// Rejects an invalid Task ID or snapshot identity and canonical failures.
    pub fn zero_head(
        identity: TaskLedgerStreamIdentity,
    ) -> Result<TaskLedgerStreamHead, LedgerError> {
        zero_head_for_runtime(identity, RuntimeKind::Fake)
    }

    /// Executes one command with exact retry-before-stale semantics.
    ///
    /// # Errors
    ///
    /// Returns [`LedgerError::CommandIdReuse`] for a changed retry and a typed
    /// validation/hash error before any mutation.
    pub fn execute(&mut self, command: AppendCommand) -> Result<CommandReceipt, LedgerError> {
        validate_full_head(&command.expected_head, RuntimeKind::Fake)?;
        let stream_id = command.expected_head.stream_id().as_str().to_owned();
        let receipt_key = (stream_id.clone(), command.command_id.as_str().to_owned());
        let current = if self.commands.contains_key(&receipt_key) {
            verify_untrusted_snapshot(&self.retry_snapshot(
                command.expected_head.stream_id(),
                command.expected_head.identity(),
            )?)?
        } else {
            self.planning_stream(
                command.expected_head.stream_id(),
                command.expected_head.identity(),
            )?
        };
        let plan = plan_append(&current, command)?;
        let receipt = plan.receipt().clone();
        if plan.is_exact_retry() {
            return Ok(receipt);
        }
        let next = apply_append_plan(&current, &plan)?;
        let command_record = plan
            .new_command()
            .ok_or(LedgerError::CheckpointMismatch)?
            .clone();
        self.commands.insert(receipt_key, command_record);
        if plan.new_event().is_some() {
            let state = self
                .streams
                .entry(stream_id)
                .or_insert_with(|| StreamState {
                    identity: next.identity.clone(),
                    head: current.head.clone(),
                    events: Vec::new(),
                    outboxes: Vec::new(),
                    counters: current.counters.clone(),
                    observation_revision: 0,
                    latest_observation: None,
                });
            state.identity = next.identity.clone();
            state.head = next.head.clone();
            state.events.clone_from(&next.events);
            state.outboxes.clone_from(&next.outboxes);
            state.counters = next.counters.clone();
            state.latest_observation = None;
        }
        Ok(receipt)
    }

    /// Returns the current fake head for a stream.
    #[must_use]
    pub fn current_head(&self, stream_id: &ContentDigest) -> Option<TaskLedgerStreamHead> {
        self.streams
            .get(stream_id.as_str())
            .map(|state| state.head.clone())
    }

    fn planning_stream(
        &self,
        stream_id: &ContentDigest,
        identity: &TaskLedgerStreamIdentity,
    ) -> Result<VerifiedStream, LedgerError> {
        let vacant = VerifiedStream::vacant(identity.clone(), RuntimeKind::Fake)?;
        if vacant.head.stream_id() != stream_id {
            return Err(LedgerError::InvalidStreamHead);
        }
        let (head, events, outboxes, counters) = self.streams.get(stream_id.as_str()).map_or_else(
            || (vacant.head.clone(), Vec::new(), Vec::new(), zero_counters()),
            |state| {
                (
                    state.head.clone(),
                    state.events.clone(),
                    state.outboxes.clone(),
                    state.counters.clone(),
                )
            },
        );
        let mut commands = self
            .commands
            .iter()
            .filter(|((key_stream_id, _), _)| key_stream_id == stream_id.as_str())
            .map(|(_, stored)| stored.clone())
            .collect::<Vec<_>>();
        canonicalize_commands(&mut commands);
        let checkpoint = build_checkpoint(
            identity,
            RuntimeKind::Fake,
            &head,
            &counters,
            &events,
            &commands,
            &outboxes,
        )?;
        Ok(VerifiedStream {
            identity: identity.clone(),
            head,
            events,
            commands,
            outboxes,
            counters,
            checkpoint,
        })
    }

    /// Exports one complete fake stream through the same untrusted persistence
    /// boundary that every persistence adapter must use.
    ///
    /// # Errors
    ///
    /// Rejects an unknown stream or impossible internal storage-key shape.
    pub fn untrusted_snapshot(
        &self,
        stream_id: &ContentDigest,
    ) -> Result<UntrustedLedgerSnapshot, LedgerError> {
        if let Some(state) = self.streams.get(stream_id.as_str()) {
            let events = state.events.iter().map(untrusted_event).collect();
            let commands = self.untrusted_commands_for_stream(stream_id)?;
            return Ok(UntrustedLedgerSnapshot {
                identity: state.identity.clone(),
                claimed_head: state.head.clone(),
                events,
                commands,
                outboxes: state
                    .outboxes
                    .iter()
                    .map(OutboxAdmission::to_untrusted)
                    .collect(),
                claimed_counters: state.counters.clone(),
            });
        }

        let identity = self
            .commands
            .iter()
            .find(|((key_stream_id, _), _)| key_stream_id == stream_id.as_str())
            .map(|(_, stored)| stored.request.expected_head.identity().clone())
            .ok_or(LedgerError::InvalidStreamHead)?;
        let claimed_head = Self::zero_head(identity.clone())?;
        if claimed_head.stream_id() != stream_id {
            return Err(LedgerError::InvalidStreamHead);
        }
        Ok(UntrustedLedgerSnapshot {
            identity,
            claimed_head,
            events: Vec::new(),
            commands: self.untrusted_commands_for_stream(stream_id)?,
            outboxes: Vec::new(),
            claimed_counters: zero_counters(),
        })
    }

    fn untrusted_commands_for_stream(
        &self,
        stream_id: &ContentDigest,
    ) -> Result<Vec<UntrustedCommandRecord>, LedgerError> {
        let commands = self
            .commands
            .iter()
            .filter(|((key_stream_id, _), _)| key_stream_id == stream_id.as_str())
            .map(|((key_stream_id, key_command_id), stored)| {
                let mut untrusted = stored.to_untrusted();
                untrusted.stream_id = ContentDigest::from_sha256(key_stream_id.clone())?;
                untrusted.command_id.clone_from(key_command_id);
                Ok(untrusted)
            })
            .collect::<Result<Vec<_>, LedgerError>>()?;
        Ok(commands)
    }

    fn retry_snapshot(
        &self,
        stream_id: &ContentDigest,
        identity: &TaskLedgerStreamIdentity,
    ) -> Result<UntrustedLedgerSnapshot, LedgerError> {
        if self.streams.contains_key(stream_id.as_str()) {
            return self.untrusted_snapshot(stream_id);
        }
        let claimed_head = Self::zero_head(identity.clone())?;
        if claimed_head.stream_id() != stream_id {
            return Err(LedgerError::InvalidStreamHead);
        }
        Ok(UntrustedLedgerSnapshot {
            identity: identity.clone(),
            claimed_head,
            events: Vec::new(),
            commands: self.untrusted_commands_for_stream(stream_id)?,
            outboxes: Vec::new(),
            claimed_counters: zero_counters(),
        })
    }

    /// Replays and verifies one complete fake stream.
    ///
    /// # Errors
    ///
    /// Rejects event/request/receipt/hash/projection/head disagreement.
    pub fn verified_stream(
        &self,
        stream_id: &ContentDigest,
    ) -> Result<VerifiedStream, LedgerError> {
        verify_untrusted_snapshot(&self.untrusted_snapshot(stream_id)?)
    }

    /// Issues a fake Task-Ledger-owned resource observation.
    ///
    /// # Errors
    ///
    /// Rejects a stale full expected head or canonical/hash failure.
    pub fn issue_resource_observation(
        &mut self,
        expected_head: TaskLedgerStreamHead,
        effect_claim_id: &EffectClaimId,
        effect_subject_digest: ContentDigest,
        request: ResourceRequest,
    ) -> Result<TaskLedgerResourceReceipt, LedgerError> {
        validate_full_head(&expected_head, RuntimeKind::Fake)?;
        let stream_key = expected_head.stream_id().as_str().to_owned();
        let state = self
            .streams
            .get_mut(&stream_key)
            .ok_or(LedgerError::InvalidStreamHead)?;
        if state.head != expected_head || state.head.sequence() == 0 {
            return Err(LedgerError::InvalidStreamHead);
        }
        if let Some(previous) = state.latest_observation.as_ref()
            && previous.stream_head() == &expected_head
            && previous.effect_claim_id() == effect_claim_id.as_str()
            && previous.effect_subject_digest() == &effect_subject_digest
            && previous.counters() == &state.counters
            && previous.request() == &request
        {
            return Ok(previous.clone());
        }
        let observation_revision = state
            .observation_revision
            .checked_add(1)
            .ok_or(LedgerError::InvalidResourceSnapshot)?;
        let accounting_currency = state
            .identity
            .accounting_currency()
            .ok_or(LedgerError::GeneralTaskIntakeCreateOnly)?;
        let observation_value = resource_observation_value(
            &expected_head,
            observation_revision,
            effect_claim_id.as_str(),
            &effect_subject_digest,
            &state.counters,
            &request,
            accounting_currency,
        );
        let observation_digest = hash_value(
            "lattice.task-ledger.resource-observation",
            &observation_value,
        )?;
        let receipt_digest = hash_value(
            "lattice.task-ledger.resource-receipt",
            &object(vec![
                ("producer_id", text(TASK_LEDGER_PRODUCER_ID)),
                ("producer_version", text(TASK_LEDGER_PRODUCER_VERSION)),
                ("runtime", text(runtime_text(RuntimeKind::Fake))),
                ("observation", observation_value),
                ("observation_digest", digest_value(&observation_digest)),
            ]),
        )?;
        let receipt = TaskLedgerResourceReceipt::new(
            CONTRACT_VERSION,
            TASK_LEDGER_PRODUCER_ID,
            TASK_LEDGER_PRODUCER_VERSION,
            RuntimeKind::Fake,
            expected_head,
            observation_revision,
            effect_claim_id.as_str(),
            effect_subject_digest,
            state.counters.clone(),
            request,
            accounting_currency,
            observation_digest,
            receipt_digest,
        )?;
        state.observation_revision = observation_revision;
        state.latest_observation = Some(receipt.clone());
        Ok(receipt)
    }

    /// Returns an independent fake current resource head only while exact.
    #[must_use]
    pub fn current_resource_head(
        &self,
        receipt: &TaskLedgerResourceReceipt,
    ) -> Option<TaskLedgerResourceHead> {
        let state = self
            .streams
            .get(receipt.stream_head().stream_id().as_str())?;
        let current = state.latest_observation.as_ref()?;
        (current == receipt
            && receipt.stream_head() == &state.head
            && receipt.counters() == &state.counters)
            .then(|| receipt.head())
    }
}

#[derive(Clone, Debug)]
struct ParsedCommandRecord {
    request: AppendCommand,
    receipt: CommandReceipt,
    base_checkpoint: LedgerCheckpoint,
    result_checkpoint: LedgerCheckpoint,
}

fn untrusted_request(command: &AppendCommand) -> UntrustedAppendRequest {
    UntrustedAppendRequest {
        schema_version: LEDGER_SCHEMA_VERSION.to_owned(),
        expected_head: command.expected_head.clone(),
        command_id: command.command_id.as_str().to_owned(),
        correlation_id: command.correlation_id.as_str().to_owned(),
        occurred_at: command.occurred_at.clone(),
        kind: command.kind.as_str().to_owned(),
        actor_id: command.actor_id.as_str().to_owned(),
        action: command.action.as_str().to_owned(),
        outcome: command.outcome.as_str().to_owned(),
        reason_code: command.reason_code.as_str().to_owned(),
        subject_digest: command.subject_digest.clone(),
        diagnostic: command
            .diagnostic
            .as_ref()
            .map(|diagnostic| diagnostic.value().clone()),
        resource_snapshot: command
            .resource_snapshot
            .as_ref()
            .map(|snapshot| snapshot.counters().clone()),
    }
}

fn untrusted_receipt(receipt: &CommandReceipt) -> UntrustedCommandReceipt {
    let (outcome, denial_reason) = match receipt.outcome() {
        CommandOutcome::Appended => ("APPENDED".to_owned(), None),
        CommandOutcome::Denied(reason) => ("DENIED".to_owned(), Some(reason.as_str().to_owned())),
    };
    UntrustedCommandReceipt {
        schema_version: LEDGER_SCHEMA_VERSION.to_owned(),
        command_id: receipt.command_id().as_str().to_owned(),
        request_digest: receipt.request_digest().clone(),
        before: receipt.before().clone(),
        after: receipt.after().clone(),
        outcome,
        denial_reason,
        event_digest: receipt.event_digest().cloned(),
        receipt_digest: receipt.receipt_digest().clone(),
    }
}

fn untrusted_event(event: &LedgerEvent) -> UntrustedLedgerEvent {
    UntrustedLedgerEvent {
        schema_version: event.schema_version.clone(),
        stream_identity: event.stream_identity.clone(),
        stream_id: event.stream_id.clone(),
        sequence: event.sequence,
        previous_event_digest: event.previous_event_digest.clone(),
        command_id: event.command_id.as_str().to_owned(),
        request_digest: event.request_digest.clone(),
        correlation_id: event.correlation_id.as_str().to_owned(),
        occurred_at: event.occurred_at.clone(),
        kind: event.kind.as_str().to_owned(),
        actor_id: event.actor_id.as_str().to_owned(),
        action: event.action.as_str().to_owned(),
        outcome: event.outcome.as_str().to_owned(),
        reason_code: event.reason_code.as_str().to_owned(),
        subject_digest: event.subject_digest.clone(),
        diagnostic: event
            .diagnostic
            .as_ref()
            .map(|diagnostic| diagnostic.value().clone()),
        resource_snapshot: event
            .resource_snapshot
            .as_ref()
            .map(|snapshot| snapshot.counters().clone()),
        resource_revision: event.resource_revision,
        resource_projection_digest: event.resource_projection_digest.clone(),
        event_digest: event.event_digest.clone(),
    }
}

fn untrusted_outbox(outbox: &OutboxAdmission) -> UntrustedOutboxAdmission {
    UntrustedOutboxAdmission {
        schema_version: outbox.schema_version.clone(),
        stream_identity: outbox.stream_identity.clone(),
        stream_id: outbox.stream_id.clone(),
        event_sequence: outbox.event_sequence,
        event_digest: outbox.event_digest.clone(),
        command_id: outbox.command_id.as_str().to_owned(),
        request_digest: outbox.request_digest.clone(),
        intent_digest: outbox.intent_digest.clone(),
        occurred_at: outbox.occurred_at.clone(),
        state: outbox.state.as_str().to_owned(),
        admission_digest: outbox.admission_digest.clone(),
    }
}

fn diagnostic_from_persisted(
    value: Option<&CanonicalValue>,
) -> Result<Option<Diagnostic>, LedgerError> {
    value
        .map(|value| {
            let diagnostic = Diagnostic::new(value.clone())?;
            if diagnostic.value() != value {
                return Err(LedgerError::InvalidDiagnostic);
            }
            Ok(diagnostic)
        })
        .transpose()
}

fn command_from_untrusted_request(
    raw: &UntrustedAppendRequest,
) -> Result<AppendCommand, LedgerError> {
    if raw.schema_version != LEDGER_SCHEMA_VERSION {
        return Err(LedgerError::UnknownRequestVersion);
    }
    AppendCommand::from_fields(
        raw.expected_head.clone(),
        CommandId::new(raw.command_id.clone())?,
        CorrelationId::new(raw.correlation_id.clone())?,
        raw.occurred_at.clone(),
        LedgerEventKind::parse(&raw.kind)?,
        ActorId::new(raw.actor_id.clone())?,
        ActionId::new(raw.action.clone())?,
        LedgerOutcome::parse(&raw.outcome)?,
        ReasonCode::new(raw.reason_code.clone())?,
        raw.subject_digest.clone(),
        diagnostic_from_persisted(raw.diagnostic.as_ref())?,
        raw.resource_snapshot
            .as_ref()
            .map(|counters| ResourceSnapshot::new(counters.clone())),
        AppendConstruction::VerifiedReplay,
    )
}

fn command_from_untrusted_event(
    before: &TaskLedgerStreamHead,
    raw: &UntrustedLedgerEvent,
) -> Result<AppendCommand, LedgerError> {
    command_from_untrusted_request(&UntrustedAppendRequest {
        schema_version: LEDGER_SCHEMA_VERSION.to_owned(),
        expected_head: before.clone(),
        command_id: raw.command_id.clone(),
        correlation_id: raw.correlation_id.clone(),
        occurred_at: raw.occurred_at.clone(),
        kind: raw.kind.clone(),
        actor_id: raw.actor_id.clone(),
        action: raw.action.clone(),
        outcome: raw.outcome.clone(),
        reason_code: raw.reason_code.clone(),
        subject_digest: raw.subject_digest.clone(),
        diagnostic: raw.diagnostic.clone(),
        resource_snapshot: raw.resource_snapshot.clone(),
    })
}

fn receipt_from_untrusted(raw: &UntrustedCommandReceipt) -> Result<CommandReceipt, LedgerError> {
    if raw.schema_version != LEDGER_SCHEMA_VERSION {
        return Err(LedgerError::UnknownReceiptVersion);
    }
    let outcome = match (raw.outcome.as_str(), raw.denial_reason.as_deref()) {
        ("APPENDED", None) => CommandOutcome::Appended,
        ("DENIED", Some("STALE_HEAD")) => CommandOutcome::Denied(LedgerDenial::StaleHead),
        ("DENIED", Some("SEQUENCE_OVERFLOW")) => {
            CommandOutcome::Denied(LedgerDenial::SequenceOverflow)
        }
        ("APPENDED" | "DENIED", _) => return Err(LedgerError::ReceiptBindingMismatch),
        _ => return Err(LedgerError::UnknownReceiptOutcome),
    };
    Ok(CommandReceipt {
        command_id: CommandId::new(raw.command_id.clone())?,
        request_digest: raw.request_digest.clone(),
        before: raw.before.clone(),
        after: raw.after.clone(),
        outcome,
        event_digest: raw.event_digest.clone(),
        receipt_digest: raw.receipt_digest.clone(),
    })
}

fn parse_command_record(
    raw: &UntrustedCommandRecord,
    identity: &TaskLedgerStreamIdentity,
    stream_id: &ContentDigest,
    runtime: RuntimeKind,
) -> Result<ParsedCommandRecord, LedgerError> {
    if &raw.stream_id != stream_id {
        return Err(LedgerError::InvalidStreamHead);
    }
    for checkpoint in [&raw.base_checkpoint, &raw.result_checkpoint] {
        if checkpoint.stream_id() != stream_id || checkpoint.runtime() != runtime {
            return Err(LedgerError::CheckpointMismatch);
        }
    }
    let key_command_id = CommandId::new(raw.command_id.clone())?;
    let request = command_from_untrusted_request(&raw.request)?;
    validate_full_head(&request.expected_head, runtime)?;
    if request.expected_head.identity() != identity
        || request.expected_head.stream_id() != stream_id
        || request.command_id != key_command_id
    {
        return Err(LedgerError::RequestBindingMismatch);
    }
    let computed_request_digest = request_digest(&request)?;
    let receipt = receipt_from_untrusted(&raw.receipt)?;
    validate_full_head(receipt.before(), runtime)?;
    validate_full_head(receipt.after(), runtime)?;
    if receipt.command_id() != &key_command_id
        || receipt.request_digest() != &computed_request_digest
        || receipt.before().identity() != identity
        || receipt.after().identity() != identity
        || receipt.before().stream_id() != stream_id
        || receipt.after().stream_id() != stream_id
    {
        return Err(LedgerError::ReceiptBindingMismatch);
    }
    let rebuilt = command_receipt(
        receipt.command_id.clone(),
        receipt.request_digest.clone(),
        receipt.before.clone(),
        receipt.after.clone(),
        receipt.outcome.clone(),
        receipt.event_digest.clone(),
    )?;
    if rebuilt.receipt_digest != receipt.receipt_digest {
        return Err(LedgerError::ReceiptBindingMismatch);
    }
    match receipt.outcome() {
        CommandOutcome::Appended => {
            if request.expected_head != *receipt.before() || receipt.event_digest().is_none() {
                return Err(LedgerError::ReceiptBindingMismatch);
            }
        }
        CommandOutcome::Denied(LedgerDenial::StaleHead) => {
            if receipt.before() != receipt.after()
                || receipt.event_digest().is_some()
                || request.expected_head == *receipt.before()
            {
                return Err(LedgerError::ReceiptBindingMismatch);
            }
        }
        CommandOutcome::Denied(LedgerDenial::SequenceOverflow) => {
            if receipt.before() != receipt.after()
                || receipt.event_digest().is_some()
                || request.expected_head != *receipt.before()
                || receipt.before().sequence() != u64::MAX
            {
                return Err(LedgerError::ReceiptBindingMismatch);
            }
        }
    }
    Ok(ParsedCommandRecord {
        request,
        receipt,
        base_checkpoint: raw.base_checkpoint.clone(),
        result_checkpoint: raw.result_checkpoint.clone(),
    })
}

/// Verifies one complete untrusted task-stream persistence snapshot.
///
/// This is the reusable pure replay boundary for the fake and a future
/// persistence adapter. It performs no I/O and returns one complete typed
/// stream only after every raw command, event, outbox, checkpoint link, head,
/// and resource projection agrees.
///
/// # Errors
///
/// Rejects unknown schema/kind/outcome values, invalid identifiers or
/// diagnostics, duplicate/missing/extra command records, receipt disagreement,
/// corruption, reorder, truncation, outbox disagreement, and claimed
/// head/projection/checkpoint-chain mismatch.
#[allow(clippy::too_many_lines)]
pub fn verify_untrusted_snapshot(
    snapshot: &UntrustedLedgerSnapshot,
) -> Result<VerifiedStream, LedgerError> {
    validate_stream_identity(&snapshot.identity)?;
    let runtime = snapshot.claimed_head.runtime();
    validate_full_head(&snapshot.claimed_head, runtime)?;
    if snapshot.claimed_head.identity() != &snapshot.identity {
        return Err(LedgerError::InvalidStreamHead);
    }
    let stream_id = snapshot.claimed_head.stream_id();
    let mut commands = BTreeMap::new();
    for raw in &snapshot.commands {
        let key = (raw.stream_id.as_str().to_owned(), raw.command_id.clone());
        let parsed = parse_command_record(raw, &snapshot.identity, stream_id, runtime)?;
        if commands.insert(key, parsed).is_some() {
            return Err(LedgerError::ReceiptBindingMismatch);
        }
    }
    let mut unmatched_commands = commands.clone();

    let mut head = zero_head_for_runtime(snapshot.identity.clone(), runtime)?;
    let mut known_heads = vec![head.clone()];
    let mut counters = zero_counters();
    let mut events = Vec::with_capacity(snapshot.events.len());
    for raw in &snapshot.events {
        if raw.schema_version != LEDGER_SCHEMA_VERSION {
            return Err(LedgerError::UnknownEventVersion);
        }
        let expected_sequence = head
            .sequence()
            .checked_add(1)
            .ok_or(LedgerError::CorruptSequence)?;
        if raw.sequence != expected_sequence {
            return Err(LedgerError::CorruptSequence);
        }
        if raw.previous_event_digest != *head.last_event_digest() {
            return Err(LedgerError::CorruptPredecessor);
        }
        if raw.stream_id != *head.stream_id() || raw.stream_identity != snapshot.identity {
            return Err(LedgerError::InvalidStreamHead);
        }
        let command = command_from_untrusted_event(&head, raw)?;
        validate_autonomy_order(command.kind, &events)?;
        let reconstructed_request = request_digest(&command)?;
        if reconstructed_request != raw.request_digest {
            return Err(LedgerError::RequestBindingMismatch);
        }
        let key = (stream_id.as_str().to_owned(), raw.command_id.clone());
        let stored = unmatched_commands
            .remove(&key)
            .ok_or(LedgerError::OrphanReceipt)?;
        if stored.request != command {
            return Err(LedgerError::RequestBindingMismatch);
        }
        let (next_counters, resource_revision, resource_projection_digest) =
            next_resource_projection(&head, &counters, &command)?;
        if resource_revision != raw.resource_revision
            || resource_projection_digest != raw.resource_projection_digest
        {
            return Err(LedgerError::ResourceProjectionMismatch);
        }
        let reconstructed = build_event(
            &command,
            raw.sequence,
            reconstructed_request,
            resource_revision,
            resource_projection_digest.clone(),
        )?;
        if reconstructed.event_digest != raw.event_digest {
            return Err(LedgerError::CorruptEventHash);
        }
        let after = build_head(
            runtime,
            snapshot.identity.clone(),
            head.stream_id().clone(),
            raw.sequence,
            raw.event_digest.clone(),
            resource_revision,
            resource_projection_digest,
        )?;
        verify_receipt(
            &stored.receipt,
            &reconstructed.command_id,
            &raw.request_digest,
            &head,
            &after,
            Some(&raw.event_digest),
        )?;
        head = after;
        known_heads.push(head.clone());
        counters = next_counters;
        events.push(reconstructed);
    }
    let retained_profile = events
        .first()
        .map(classify_task_created_profile)
        .transpose()?
        .flatten();
    match snapshot.identity.subject_kind() {
        TaskLedgerSubjectKind::TaskSpec
            if retained_profile == Some(TaskCreatedProfile::GeneralTaskIntakeV1) =>
        {
            return Err(LedgerError::InvalidStreamHead);
        }
        TaskLedgerSubjectKind::GeneralTaskIntake
            if !events.is_empty()
                && (retained_profile != Some(TaskCreatedProfile::GeneralTaskIntakeV1)
                    || !(events.len() == 1 || valid_general_external_adoption_events(&events))) =>
        {
            return Err(LedgerError::GeneralTaskIntakeCreateOnly);
        }
        TaskLedgerSubjectKind::TaskSpec | TaskLedgerSubjectKind::GeneralTaskIntake => {}
    }
    if head != snapshot.claimed_head {
        return Err(LedgerError::HeadMismatch);
    }
    if counters != snapshot.claimed_counters {
        return Err(LedgerError::ResourceProjectionMismatch);
    }
    for stored in unmatched_commands.into_values() {
        if stored.receipt.outcome() == &CommandOutcome::Appended {
            return Err(LedgerError::OrphanReceipt);
        }
        if !known_heads.contains(stored.receipt.before()) {
            return Err(LedgerError::ReceiptBindingMismatch);
        }
    }

    let mut verified_commands = commands
        .into_values()
        .map(|stored| VerifiedCommandRecord {
            request: stored.request,
            receipt: stored.receipt,
            base_checkpoint: stored.base_checkpoint,
            result_checkpoint: stored.result_checkpoint,
        })
        .collect::<Vec<_>>();
    canonicalize_commands(&mut verified_commands);

    let mut expected_outboxes = events
        .iter()
        .map(derive_outbox_admission)
        .collect::<Result<Vec<_>, LedgerError>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    canonicalize_outboxes(&mut expected_outboxes);
    let mut outboxes = Vec::with_capacity(snapshot.outboxes.len());
    for raw in &snapshot.outboxes {
        if raw.schema_version != OUTBOX_ADMISSION_SCHEMA_VERSION {
            return Err(LedgerError::UnknownOutboxVersion);
        }
        let state = OutboxAdmissionState::parse(&raw.state)?;
        validate_utc_timestamp(&raw.occurred_at)?;
        let parsed = OutboxAdmission {
            schema_version: raw.schema_version.clone(),
            stream_identity: raw.stream_identity.clone(),
            stream_id: raw.stream_id.clone(),
            event_sequence: raw.event_sequence,
            event_digest: raw.event_digest.clone(),
            command_id: CommandId::new(raw.command_id.clone())?,
            request_digest: raw.request_digest.clone(),
            intent_digest: raw.intent_digest.clone(),
            occurred_at: raw.occurred_at.clone(),
            state,
            admission_digest: raw.admission_digest.clone(),
        };
        if parsed.stream_identity != snapshot.identity || parsed.stream_id != *stream_id {
            return Err(LedgerError::OutboxBindingMismatch);
        }
        let event = events
            .iter()
            .find(|event| event.sequence() == parsed.event_sequence)
            .ok_or(LedgerError::OutboxBindingMismatch)?;
        let expected = derive_outbox_admission(event)?.ok_or(LedgerError::OutboxBindingMismatch)?;
        if parsed != expected {
            return Err(LedgerError::OutboxBindingMismatch);
        }
        outboxes.push(parsed);
    }
    canonicalize_outboxes(&mut outboxes);
    if outboxes != expected_outboxes {
        return Err(LedgerError::OutboxBindingMismatch);
    }

    let checkpoint = build_checkpoint(
        &snapshot.identity,
        runtime,
        &head,
        &counters,
        &events,
        &verified_commands,
        &outboxes,
    )?;
    let verified = VerifiedStream {
        identity: snapshot.identity.clone(),
        head,
        events,
        commands: verified_commands,
        outboxes,
        counters,
        checkpoint,
    };
    validate_command_checkpoint_chain(&verified)?;
    Ok(verified)
}

fn validate_autonomy_order(
    kind: LedgerEventKind,
    preceding_events: &[LedgerEvent],
) -> Result<(), LedgerError> {
    if kind == LedgerEventKind::AutonomyReceiptRecorded
        && (preceding_events.len() != 1
            || preceding_events[0].kind() != LedgerEventKind::TaskCreated)
    {
        return Err(LedgerError::InvalidAutonomyReceipt);
    }
    Ok(())
}

fn valid_general_external_adoption_events(events: &[LedgerEvent]) -> bool {
    let [created, terminal] = events else {
        return false;
    };
    let Some(client_request_id) = created.command_id().as_str().strip_prefix("mcp-submit:") else {
        return false;
    };
    classify_task_created_profile(created).ok()
        == Some(Some(TaskCreatedProfile::GeneralTaskIntakeV1))
        && valid_task_ingress_client_request_id(client_request_id)
        && terminal.kind() == LedgerEventKind::ExternalVerifiedResultAdopted
        && terminal.actor_id() == created.actor_id()
        && terminal.correlation_id().as_str() == GENERAL_TASK_INTAKE_CORRELATION_ID
        && terminal.action().as_str() == EXTERNAL_RESULT_ADOPTION_ACTION
        && terminal.outcome() == LedgerOutcome::Recorded
        && terminal.reason_code().as_str() == EXTERNAL_RESULT_ADOPTION_REASON
        && terminal.command_id().as_str() == format!("external-result-adoption:{client_request_id}")
        && !is_zero_digest(terminal.subject_digest())
        && terminal.diagnostic().is_none()
        && terminal.resource_snapshot().is_none()
}

fn validate_submission_control_id(
    value: &str,
    field: &'static str,
    max_bytes: usize,
) -> Result<(), LedgerError> {
    if value.len() > max_bytes {
        return Err(LedgerError::SubmissionEnvelopeLimitExceeded { field });
    }
    let bytes = value.as_bytes();
    if bytes.is_empty()
        || !bytes.first().is_some_and(u8::is_ascii_alphanumeric)
        || !bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'.' | b'_' | b':' | b'-'))
    {
        return Err(LedgerError::InvalidSubmissionEnvelope { field });
    }
    if field == "client_request_id" && !valid_task_ingress_client_request_id(value) {
        return Err(if task_submission_text_contains_secret(value) {
            LedgerError::SubmissionSecretRejected
        } else {
            LedgerError::InvalidSubmissionEnvelope { field }
        });
    }
    if task_submission_text_contains_secret(value) {
        return Err(LedgerError::SubmissionSecretRejected);
    }
    Ok(())
}

fn validate_submission_human_text(
    value: &str,
    field: &'static str,
    max_chars: usize,
    max_bytes: usize,
) -> Result<(), LedgerError> {
    if value.len() > max_bytes || value.chars().count() > max_chars {
        return Err(LedgerError::SubmissionEnvelopeLimitExceeded { field });
    }
    if normalize_nfc(value) != value {
        return Err(LedgerError::NonCanonicalText { field });
    }
    if value.is_empty() || value.trim() != value || value.chars().any(char::is_control) {
        return Err(LedgerError::InvalidSubmissionEnvelope { field });
    }
    if task_submission_text_contains_secret(value) {
        return Err(LedgerError::SubmissionSecretRejected);
    }
    Ok(())
}

fn validate_submission_project_id(value: &str) -> Result<(), LedgerError> {
    if value.len() > MAX_SUBMISSION_PROJECT_ID_BYTES {
        return Err(LedgerError::SubmissionEnvelopeLimitExceeded {
            field: "project_id",
        });
    }
    if task_submission_text_contains_secret(value) {
        return Err(LedgerError::SubmissionSecretRejected);
    }
    let bytes = value.as_bytes();
    if bytes.len() < 2
        || !bytes
            .first()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        || !bytes.iter().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(*byte, b'.' | b'_' | b'-')
        })
    {
        return Err(LedgerError::InvalidSubmissionEnvelope {
            field: "project_id",
        });
    }
    Ok(())
}

fn validate_submission_project_snapshot_id(value: &str) -> Result<(), LedgerError> {
    if value.len() > TASK_LEDGER_PROJECT_SNAPSHOT_ID_MAX_BYTES {
        return Err(LedgerError::SubmissionEnvelopeLimitExceeded {
            field: "project_snapshot_id",
        });
    }
    if task_submission_text_contains_secret(value) {
        return Err(LedgerError::SubmissionSecretRejected);
    }
    let bytes = value.as_bytes();
    if bytes.is_empty()
        || !bytes
            .first()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        || !bytes.iter().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(*byte, b'.' | b'_' | b':' | b'-')
        })
    {
        return Err(LedgerError::InvalidSubmissionEnvelope {
            field: "project_snapshot_id",
        });
    }
    Ok(())
}

/// Returns true when untrusted task-intake text contains recognizable secret
/// material or a sensitive-key assignment/header shape.
///
/// This validator is intentionally shared by the MCP boundary and the Task
/// Ledger owner so transport validation cannot drift from durable validation.
/// It rejects the whole request; callers must never redact and persist a
/// content-mutated objective under the original idempotency key.
#[must_use]
pub fn task_submission_text_contains_secret(value: &str) -> bool {
    task_ingress_text_contains_recognized_secret(value)
}

fn valid_identifier(value: &str) -> bool {
    (1..=128).contains(&value.len())
        && value.trim() == value
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'/' | b'-')
        })
}

fn valid_git_commit(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_evidence_reference(value: &str) -> bool {
    value
        .strip_prefix("evidence:sha256:")
        .is_some_and(|digest| {
            digest.len() == 64
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
        && !recognized_secret_text(value)
}

fn valid_project_snapshot_identifier(value: &str) -> bool {
    (1..=TASK_LEDGER_PROJECT_SNAPSHOT_ID_MAX_BYTES).contains(&value.len())
        && value.trim() == value
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'/' | b'-')
        })
}

fn validate_stream_identity(identity: &TaskLedgerStreamIdentity) -> Result<(), LedgerError> {
    if recognized_secret_text(identity.project_id().as_str()) {
        return Err(LedgerError::InvalidIdentifier {
            field: "project_id",
        });
    }
    let task_id = identity.task_id().as_str();
    let task_suffix = task_id
        .strip_prefix("TASK-")
        .ok_or(LedgerError::InvalidIdentifier { field: "task_id" })?;
    if !(3..=64).contains(&task_suffix.len())
        || !task_suffix.bytes().all(|byte| {
            byte.is_ascii_uppercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
        || !task_suffix
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
    {
        return Err(LedgerError::InvalidIdentifier { field: "task_id" });
    }
    let snapshot = identity.project_snapshot_id().as_str();
    if !valid_project_snapshot_identifier(snapshot) || recognized_secret_text(snapshot) {
        return Err(LedgerError::InvalidIdentifier {
            field: "project_snapshot_id",
        });
    }
    Ok(())
}

fn zero_head_for_runtime(
    identity: TaskLedgerStreamIdentity,
    runtime: RuntimeKind,
) -> Result<TaskLedgerStreamHead, LedgerError> {
    validate_stream_identity(&identity)?;
    let stream_id = hash_value("lattice.task-ledger.stream-id", &identity_value(&identity))?;
    build_head(
        runtime,
        identity,
        stream_id,
        0,
        zero_digest(),
        0,
        zero_digest(),
    )
}

fn validate_full_head(
    head: &TaskLedgerStreamHead,
    expected_runtime: RuntimeKind,
) -> Result<(), LedgerError> {
    validate_stream_identity(head.identity())?;
    if head.runtime() != expected_runtime {
        return Err(LedgerError::InvalidStreamHead);
    }
    let stream_id = hash_value(
        "lattice.task-ledger.stream-id",
        &identity_value(head.identity()),
    )?;
    if &stream_id != head.stream_id() {
        return Err(LedgerError::InvalidStreamHead);
    }
    let rebuilt = build_head(
        expected_runtime,
        head.identity().clone(),
        stream_id,
        head.sequence(),
        head.last_event_digest().clone(),
        head.resource_revision(),
        head.resource_projection_digest().clone(),
    )?;
    if &rebuilt != head {
        return Err(LedgerError::InvalidStreamHead);
    }
    Ok(())
}

fn validate_utc_timestamp(value: &str) -> Result<(), LedgerError> {
    let parsed =
        OffsetDateTime::parse(value, &Rfc3339).map_err(|_| LedgerError::InvalidTimestamp)?;
    if parsed.offset() != UtcOffset::UTC {
        return Err(LedgerError::InvalidTimestamp);
    }
    let formatted = parsed
        .format(&Rfc3339)
        .map_err(|_| LedgerError::InvalidTimestamp)?;
    if formatted != value || !value.ends_with('Z') {
        return Err(LedgerError::InvalidTimestamp);
    }
    Ok(())
}

fn validate_raw_diagnostic(
    value: &CanonicalValue,
    depth: usize,
    nodes: &mut usize,
) -> Result<(), LedgerError> {
    if depth > MAX_DIAGNOSTIC_DEPTH {
        return Err(LedgerError::DiagnosticLimitExceeded);
    }
    *nodes = nodes
        .checked_add(1)
        .ok_or(LedgerError::DiagnosticLimitExceeded)?;
    if *nodes > MAX_DIAGNOSTIC_NODES {
        return Err(LedgerError::DiagnosticLimitExceeded);
    }
    match value {
        CanonicalValue::Null | CanonicalValue::Bool(_) => Ok(()),
        CanonicalValue::String(value) => {
            if normalize_nfc(value) != *value || value.contains('\0') {
                return Err(LedgerError::NonCanonicalText {
                    field: "diagnostic_value",
                });
            }
            Ok(())
        }
        CanonicalValue::Array(values) => {
            for value in values {
                validate_raw_diagnostic(value, depth + 1, nodes)?;
            }
            Ok(())
        }
        CanonicalValue::Object(entries) => {
            for (key, value) in entries {
                if normalize_nfc(key) != *key || key.contains('\0') || key.trim() != key {
                    return Err(LedgerError::NonCanonicalText {
                        field: "diagnostic_key",
                    });
                }
                if sanitize_string(key) == "[REDACTED]" {
                    return Err(LedgerError::InvalidDiagnostic);
                }
                validate_raw_diagnostic(value, depth + 1, nodes)?;
            }
            Ok(())
        }
    }
}

fn sanitize_diagnostic(
    value: CanonicalValue,
    depth: usize,
    nodes: &mut usize,
) -> Result<CanonicalValue, LedgerError> {
    if depth > MAX_DIAGNOSTIC_DEPTH {
        return Err(LedgerError::DiagnosticLimitExceeded);
    }
    *nodes = nodes
        .checked_add(1)
        .ok_or(LedgerError::DiagnosticLimitExceeded)?;
    if *nodes > MAX_DIAGNOSTIC_NODES {
        return Err(LedgerError::DiagnosticLimitExceeded);
    }
    match value {
        CanonicalValue::Null | CanonicalValue::Bool(_) => Ok(value),
        CanonicalValue::String(value) => {
            if normalize_nfc(&value) != value || value.contains('\0') {
                return Err(LedgerError::NonCanonicalText {
                    field: "diagnostic_value",
                });
            }
            Ok(CanonicalValue::String(sanitize_string(&value)))
        }
        CanonicalValue::Array(values) => values
            .into_iter()
            .map(|value| sanitize_diagnostic(value, depth + 1, nodes))
            .collect::<Result<Vec<_>, _>>()
            .map(CanonicalValue::Array),
        CanonicalValue::Object(entries) => {
            let mut sanitized = Vec::with_capacity(entries.len());
            for (key, value) in entries {
                if normalize_nfc(&key) != key || key.contains('\0') || key.trim() != key {
                    return Err(LedgerError::NonCanonicalText {
                        field: "diagnostic_key",
                    });
                }
                let value = if sensitive_key(&key) {
                    *nodes = nodes
                        .checked_add(1)
                        .ok_or(LedgerError::DiagnosticLimitExceeded)?;
                    CanonicalValue::String("[REDACTED]".to_owned())
                } else {
                    sanitize_diagnostic(value, depth + 1, nodes)?
                };
                sanitized.push((key, value));
            }
            canonicalize(&CanonicalValue::Object(sanitized.clone()))
                .map_err(|_| LedgerError::InvalidDiagnostic)?;
            Ok(CanonicalValue::Object(sanitized))
        }
    }
}

fn sensitive_key(key: &str) -> bool {
    let normalized = key
        .bytes()
        .filter(u8::is_ascii_alphanumeric)
        .map(|byte| byte.to_ascii_lowercase() as char)
        .collect::<String>();
    [
        "apikey",
        "accesskey",
        "clientsecret",
        "refreshtoken",
        "token",
        "secret",
        "password",
        "credential",
        "authorization",
        "cookie",
        "privatekey",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

fn sanitize_string(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    if lower.contains("bearer ")
        || (lower.contains("-----begin ") && lower.contains("private key-----"))
        || contains_secret_prefix(&lower, "sk-")
        || [
            "ghp_",
            "gho_",
            "ghu_",
            "ghs_",
            "ghr_",
            "github_pat_",
            "glpat-",
            "npm_",
            "pypi-",
            "xoxa-",
            "xoxb-",
            "xoxp-",
            "xoxr-",
            "xoxs-",
        ]
        .iter()
        .any(|prefix| lower.contains(prefix))
    {
        "[REDACTED]".to_owned()
    } else {
        value.to_owned()
    }
}

fn contains_secret_prefix(value: &str, prefix: &str) -> bool {
    value
        .match_indices(prefix)
        .any(|(index, _)| index == 0 || !value.as_bytes()[index - 1].is_ascii_alphanumeric())
}

fn recognized_secret_text(value: &str) -> bool {
    sanitize_string(value) == "[REDACTED]"
}

fn zero_digest() -> ContentDigest {
    ContentDigest::from_sha256(ZERO_DIGEST_TEXT).expect("constant zero SHA-256 shape")
}

fn zero_counters() -> ResourceCounters {
    ResourceCounters::new(0, 0, 0, 0, 0, "0").expect("constant zero resource counters")
}

fn is_zero_digest(value: &ContentDigest) -> bool {
    value.as_str().bytes().all(|byte| byte == b'0')
}

fn hash_value(schema_id: &str, value: &CanonicalValue) -> Result<ContentDigest, LedgerError> {
    hash_value_at_version(schema_id, LEDGER_SCHEMA_VERSION, value)
}

fn hash_value_at_version(
    schema_id: &str,
    schema_version: &str,
    value: &CanonicalValue,
) -> Result<ContentDigest, LedgerError> {
    let domain = HashDomain::new(schema_id, schema_version)?;
    let digest = canonical_sha256(&domain, value)?;
    ContentDigest::from_sha256(digest.to_hex()).map_err(LedgerError::from)
}

fn text(value: impl Into<String>) -> CanonicalValue {
    CanonicalValue::String(value.into())
}

fn object(entries: Vec<(&str, CanonicalValue)>) -> CanonicalValue {
    CanonicalValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

fn optional(value: Option<CanonicalValue>) -> CanonicalValue {
    value.unwrap_or(CanonicalValue::Null)
}

fn unsigned(value: u64) -> CanonicalValue {
    text(value.to_string())
}

fn digest_value(value: &ContentDigest) -> CanonicalValue {
    text(value.as_str())
}

fn runtime_text(runtime: RuntimeKind) -> &'static str {
    match runtime {
        RuntimeKind::Fake => "FAKE",
        RuntimeKind::Live => "LIVE",
    }
}

fn canonicalize_commands(commands: &mut [VerifiedCommandRecord]) {
    commands.sort_by(|left, right| {
        (
            left.request.expected_head.stream_id().as_str(),
            left.request.command_id.as_str(),
        )
            .cmp(&(
                right.request.expected_head.stream_id().as_str(),
                right.request.command_id.as_str(),
            ))
    });
}

fn canonicalize_outboxes(outboxes: &mut [OutboxAdmission]) {
    outboxes.sort_by(|left, right| {
        (left.event_sequence, left.admission_digest.as_str())
            .cmp(&(right.event_sequence, right.admission_digest.as_str()))
    });
}

fn identity_value(identity: &TaskLedgerStreamIdentity) -> CanonicalValue {
    let common = || {
        vec![
            ("stream_kind", text("TASK")),
            ("project_id", text(identity.project_id().as_str())),
            (
                "project_snapshot_id",
                text(identity.project_snapshot_id().as_str()),
            ),
            ("task_id", text(identity.task_id().as_str())),
            ("task_revision", text(identity.task_revision())),
        ]
    };
    match identity.subject_kind() {
        TaskLedgerSubjectKind::TaskSpec => {
            let mut fields = common();
            fields.push((
                "task_spec_digest",
                digest_value(
                    identity
                        .task_spec_digest()
                        .expect("TaskSpec subject always carries its digest"),
                ),
            ));
            fields.push((
                "accounting_currency",
                text(
                    identity
                        .accounting_currency()
                        .expect("TaskSpec subject always carries its currency"),
                ),
            ));
            object(fields)
        }
        TaskLedgerSubjectKind::GeneralTaskIntake => {
            let mut fields = common();
            fields.push((
                "task_subject_kind",
                text(TaskLedgerSubjectKind::GeneralTaskIntake.as_str()),
            ));
            fields.push((
                "intake_digest",
                digest_value(
                    identity
                        .general_task_intake_digest()
                        .expect("general intake subject always carries its digest"),
                ),
            ));
            object(fields)
        }
    }
}

fn task_submission_content_value(
    ingress_id: &str,
    client_request_id: &str,
    objective: &str,
    project_display_name: &str,
    project_authority_receipt_digest: &ContentDigest,
    identity: &TaskLedgerStreamIdentity,
    stream_id: &ContentDigest,
) -> CanonicalValue {
    object(vec![
        ("schema_version", text(TASK_SUBMISSION_ENVELOPE_SCHEMA)),
        (
            "admission_action",
            text(TaskCreatedProfile::GeneralTaskIntakeV1.action()),
        ),
        ("ingress_id", text(ingress_id)),
        ("client_request_id", text(client_request_id)),
        ("objective", text(objective)),
        ("project_display_name", text(project_display_name)),
        (
            "project_authority_receipt_digest",
            digest_value(project_authority_receipt_digest),
        ),
        ("stream_identity", identity_value(identity)),
        ("stream_id", digest_value(stream_id)),
    ])
}

#[allow(clippy::too_many_arguments)]
fn external_verified_result_adoption_value(
    task_ref: &ContentDigest,
    client_request_id: &str,
    expected_ledger_head_digest: &ContentDigest,
    source_sha: &str,
    target_sha: &str,
    push_merge_receipt_ref: &str,
    deployment_receipt_ref: &str,
    deployment_artifact_ref: &str,
    independent_acceptance_ref: &str,
    protected_action_approval_refs: &[String],
) -> CanonicalValue {
    object(vec![
        ("schema", text(EXTERNAL_VERIFIED_RESULT_ADOPTION_SCHEMA)),
        ("task_ref", digest_value(task_ref)),
        ("client_request_id", text(client_request_id)),
        (
            "expected_ledger_head_digest",
            digest_value(expected_ledger_head_digest),
        ),
        ("source_sha", text(source_sha)),
        ("target_sha", text(target_sha)),
        ("push_merge_receipt_ref", text(push_merge_receipt_ref)),
        ("deployment_receipt_ref", text(deployment_receipt_ref)),
        ("deployment_artifact_ref", text(deployment_artifact_ref)),
        (
            "independent_acceptance_ref",
            text(independent_acceptance_ref),
        ),
        (
            "protected_action_approval_refs",
            CanonicalValue::Array(
                protected_action_approval_refs
                    .iter()
                    .cloned()
                    .map(CanonicalValue::String)
                    .collect(),
            ),
        ),
    ])
}

fn task_submission_envelope_value(
    content: &CanonicalValue,
    task_ref: &ContentDigest,
) -> CanonicalValue {
    object(vec![
        ("content", content.clone()),
        ("task_ref", digest_value(task_ref)),
    ])
}

fn head_position_value(
    runtime: RuntimeKind,
    identity: &TaskLedgerStreamIdentity,
    stream_id: &ContentDigest,
    sequence: u64,
    last_event_digest: &ContentDigest,
    resource_revision: u64,
    resource_projection_digest: &ContentDigest,
) -> CanonicalValue {
    object(vec![
        ("producer_id", text(TASK_LEDGER_PRODUCER_ID)),
        ("producer_version", text(TASK_LEDGER_PRODUCER_VERSION)),
        ("runtime", text(runtime_text(runtime))),
        ("identity", identity_value(identity)),
        ("stream_id", digest_value(stream_id)),
        ("sequence", unsigned(sequence)),
        ("last_event_digest", digest_value(last_event_digest)),
        ("resource_revision", unsigned(resource_revision)),
        (
            "resource_projection_digest",
            digest_value(resource_projection_digest),
        ),
    ])
}

fn full_head_value(head: &TaskLedgerStreamHead) -> CanonicalValue {
    object(vec![
        (
            "position",
            head_position_value(
                head.runtime(),
                head.identity(),
                head.stream_id(),
                head.sequence(),
                head.last_event_digest(),
                head.resource_revision(),
                head.resource_projection_digest(),
            ),
        ),
        ("head_digest", digest_value(head.head_digest())),
    ])
}

fn build_head(
    runtime: RuntimeKind,
    identity: TaskLedgerStreamIdentity,
    stream_id: ContentDigest,
    sequence: u64,
    last_event_digest: ContentDigest,
    resource_revision: u64,
    resource_projection_digest: ContentDigest,
) -> Result<TaskLedgerStreamHead, LedgerError> {
    let head_digest = hash_value(
        "lattice.task-ledger.stream-head",
        &head_position_value(
            runtime,
            &identity,
            &stream_id,
            sequence,
            &last_event_digest,
            resource_revision,
            &resource_projection_digest,
        ),
    )?;
    TaskLedgerStreamHead::new(
        CONTRACT_VERSION,
        TASK_LEDGER_PRODUCER_ID,
        TASK_LEDGER_PRODUCER_VERSION,
        runtime,
        identity,
        stream_id,
        sequence,
        last_event_digest,
        resource_revision,
        resource_projection_digest,
        head_digest,
    )
    .map_err(LedgerError::from)
}

fn counters_value(counters: &ResourceCounters) -> CanonicalValue {
    object(vec![
        ("active_agents", unsigned(counters.active_agents())),
        (
            "active_implementers",
            unsigned(counters.active_implementers()),
        ),
        ("elapsed_seconds", unsigned(counters.elapsed_seconds())),
        ("attempt_number", unsigned(counters.attempt_number())),
        ("used_model_calls", unsigned(counters.used_model_calls())),
        ("used_external_cost", text(counters.used_external_cost())),
    ])
}

fn request_value(request: &ResourceRequest) -> CanonicalValue {
    object(vec![
        ("requested_agents", unsigned(request.requested_agents())),
        (
            "requested_implementers",
            unsigned(request.requested_implementers()),
        ),
        (
            "requested_duration_seconds",
            unsigned(request.requested_duration_seconds()),
        ),
        ("requested_attempts", unsigned(request.requested_attempts())),
        (
            "requested_model_calls",
            unsigned(request.requested_model_calls()),
        ),
        (
            "requested_external_cost",
            optional(request.requested_external_cost().map(text)),
        ),
    ])
}

fn snapshot_value(snapshot: &ResourceSnapshot) -> CanonicalValue {
    counters_value(snapshot.counters())
}

fn request_subject(command: &AppendCommand) -> CanonicalValue {
    object(vec![
        ("expected_head", full_head_value(&command.expected_head)),
        ("command_id", text(command.command_id.as_str())),
        ("correlation_id", text(command.correlation_id.as_str())),
        ("occurred_at", text(&command.occurred_at)),
        ("kind", text(command.kind.as_str())),
        ("actor_id", text(command.actor_id.as_str())),
        ("action", text(command.action.as_str())),
        ("outcome", text(command.outcome.as_str())),
        ("reason_code", text(command.reason_code.as_str())),
        ("subject_digest", digest_value(&command.subject_digest)),
        (
            "diagnostic",
            optional(
                command
                    .diagnostic
                    .as_ref()
                    .map(|diagnostic| diagnostic.value().clone()),
            ),
        ),
        (
            "resource_snapshot",
            optional(command.resource_snapshot.as_ref().map(snapshot_value)),
        ),
    ])
}

fn request_digest(command: &AppendCommand) -> Result<ContentDigest, LedgerError> {
    hash_value(
        "lattice.task-ledger.command-request",
        &request_subject(command),
    )
}

fn command_outcome_value(outcome: &CommandOutcome) -> CanonicalValue {
    match outcome {
        CommandOutcome::Appended => object(vec![
            ("kind", text("APPENDED")),
            ("reason", CanonicalValue::Null),
        ]),
        CommandOutcome::Denied(reason) => object(vec![
            ("kind", text("DENIED")),
            ("reason", text(reason.as_str())),
        ]),
    }
}

fn receipt_record_value(receipt: &CommandReceipt) -> CanonicalValue {
    object(vec![
        ("schema_version", text(LEDGER_SCHEMA_VERSION)),
        ("command_id", text(receipt.command_id.as_str())),
        ("request_digest", digest_value(&receipt.request_digest)),
        ("before", full_head_value(&receipt.before)),
        ("after", full_head_value(&receipt.after)),
        ("outcome", command_outcome_value(&receipt.outcome)),
        (
            "event_digest",
            optional(receipt.event_digest.as_ref().map(digest_value)),
        ),
        ("receipt_digest", digest_value(&receipt.receipt_digest)),
    ])
}

fn command_record_core_value(record: &VerifiedCommandRecord) -> CanonicalValue {
    object(vec![
        (
            "stream_id",
            digest_value(record.request.expected_head.stream_id()),
        ),
        ("command_id", text(record.request.command_id.as_str())),
        ("request_schema_version", text(LEDGER_SCHEMA_VERSION)),
        ("request", request_subject(&record.request)),
        (
            "request_digest",
            digest_value(record.receipt.request_digest()),
        ),
        ("receipt", receipt_record_value(&record.receipt)),
    ])
}

fn checkpoint_reference_value(checkpoint: &LedgerCheckpoint) -> CanonicalValue {
    object(vec![
        ("stream_id", digest_value(checkpoint.stream_id())),
        ("runtime", text(runtime_text(checkpoint.runtime()))),
        ("digest", digest_value(checkpoint.checkpoint_digest())),
    ])
}

fn command_record_persistence_value(record: &VerifiedCommandRecord) -> CanonicalValue {
    object(vec![
        ("command", command_record_core_value(record)),
        (
            "base_checkpoint",
            checkpoint_reference_value(&record.base_checkpoint),
        ),
        (
            "result_checkpoint",
            checkpoint_reference_value(&record.result_checkpoint),
        ),
    ])
}

fn event_record_value(event: &LedgerEvent) -> CanonicalValue {
    object(vec![
        ("schema_version", text(&event.schema_version)),
        ("stream_identity", identity_value(&event.stream_identity)),
        ("stream_id", digest_value(&event.stream_id)),
        ("sequence", unsigned(event.sequence)),
        (
            "previous_event_digest",
            digest_value(&event.previous_event_digest),
        ),
        ("command_id", text(event.command_id.as_str())),
        ("request_digest", digest_value(&event.request_digest)),
        ("correlation_id", text(event.correlation_id.as_str())),
        ("occurred_at", text(&event.occurred_at)),
        ("kind", text(event.kind.as_str())),
        ("actor_id", text(event.actor_id.as_str())),
        ("action", text(event.action.as_str())),
        ("outcome", text(event.outcome.as_str())),
        ("reason_code", text(event.reason_code.as_str())),
        ("subject_digest", digest_value(&event.subject_digest)),
        (
            "diagnostic",
            optional(
                event
                    .diagnostic
                    .as_ref()
                    .map(|diagnostic| diagnostic.value().clone()),
            ),
        ),
        (
            "resource_snapshot",
            optional(event.resource_snapshot.as_ref().map(snapshot_value)),
        ),
        ("resource_revision", unsigned(event.resource_revision)),
        (
            "resource_projection_digest",
            digest_value(&event.resource_projection_digest),
        ),
        ("event_digest", digest_value(&event.event_digest)),
    ])
}

#[allow(clippy::too_many_arguments)]
fn outbox_subject_value(
    stream_identity: &TaskLedgerStreamIdentity,
    stream_id: &ContentDigest,
    event_sequence: u64,
    event_digest: &ContentDigest,
    command_id: &CommandId,
    request_digest: &ContentDigest,
    intent_digest: &ContentDigest,
    occurred_at: &str,
    state: OutboxAdmissionState,
) -> CanonicalValue {
    object(vec![
        ("schema_version", text(OUTBOX_ADMISSION_SCHEMA_VERSION)),
        ("stream_identity", identity_value(stream_identity)),
        ("stream_id", digest_value(stream_id)),
        ("event_sequence", unsigned(event_sequence)),
        ("event_digest", digest_value(event_digest)),
        ("command_id", text(command_id.as_str())),
        ("request_digest", digest_value(request_digest)),
        ("intent_digest", digest_value(intent_digest)),
        ("occurred_at", text(occurred_at)),
        ("state", text(state.as_str())),
    ])
}

fn outbox_record_value(outbox: &OutboxAdmission) -> CanonicalValue {
    object(vec![
        (
            "admission",
            outbox_subject_value(
                &outbox.stream_identity,
                &outbox.stream_id,
                outbox.event_sequence,
                &outbox.event_digest,
                &outbox.command_id,
                &outbox.request_digest,
                &outbox.intent_digest,
                &outbox.occurred_at,
                outbox.state,
            ),
        ),
        ("admission_digest", digest_value(&outbox.admission_digest)),
    ])
}

fn derive_outbox_admission(event: &LedgerEvent) -> Result<Option<OutboxAdmission>, LedgerError> {
    if event.kind != LedgerEventKind::EffectIntent || event.outcome != LedgerOutcome::Recorded {
        return Ok(None);
    }
    let state = OutboxAdmissionState::Admitted;
    let subject = outbox_subject_value(
        &event.stream_identity,
        &event.stream_id,
        event.sequence,
        &event.event_digest,
        &event.command_id,
        &event.request_digest,
        &event.subject_digest,
        &event.occurred_at,
        state,
    );
    let admission_digest = hash_value_at_version(
        "lattice.task-ledger.outbox-admission",
        OUTBOX_ADMISSION_SCHEMA_VERSION,
        &subject,
    )?;
    Ok(Some(OutboxAdmission {
        schema_version: OUTBOX_ADMISSION_SCHEMA_VERSION.to_owned(),
        stream_identity: event.stream_identity.clone(),
        stream_id: event.stream_id.clone(),
        event_sequence: event.sequence,
        event_digest: event.event_digest.clone(),
        command_id: event.command_id.clone(),
        request_digest: event.request_digest.clone(),
        intent_digest: event.subject_digest.clone(),
        occurred_at: event.occurred_at.clone(),
        state,
        admission_digest,
    }))
}

fn build_record_set_digest(
    command: &VerifiedCommandRecord,
    event: Option<&LedgerEvent>,
    outbox: Option<&OutboxAdmission>,
) -> Result<ContentDigest, LedgerError> {
    hash_value_at_version(
        "lattice.task-ledger.record-set",
        LEDGER_RECORD_SET_SCHEMA_VERSION,
        &object(vec![
            ("command", command_record_persistence_value(command)),
            ("event", optional(event.map(event_record_value))),
            ("outbox", optional(outbox.map(outbox_record_value))),
        ]),
    )
}

#[allow(clippy::too_many_arguments)]
fn build_checkpoint(
    identity: &TaskLedgerStreamIdentity,
    runtime: RuntimeKind,
    head: &TaskLedgerStreamHead,
    counters: &ResourceCounters,
    events: &[LedgerEvent],
    commands: &[VerifiedCommandRecord],
    outboxes: &[OutboxAdmission],
) -> Result<LedgerCheckpoint, LedgerError> {
    let mut ordered_events = events.iter().collect::<Vec<_>>();
    ordered_events.sort_by_key(|event| event.sequence);
    let mut ordered_commands = commands.iter().collect::<Vec<_>>();
    ordered_commands.sort_by(|left, right| {
        (
            left.request.expected_head.stream_id().as_str(),
            left.request.command_id.as_str(),
        )
            .cmp(&(
                right.request.expected_head.stream_id().as_str(),
                right.request.command_id.as_str(),
            ))
    });
    let mut ordered_outboxes = outboxes.iter().collect::<Vec<_>>();
    ordered_outboxes.sort_by(|left, right| {
        (left.event_sequence, left.admission_digest.as_str())
            .cmp(&(right.event_sequence, right.admission_digest.as_str()))
    });
    let subject = object(vec![
        ("schema_version", text(LEDGER_CHECKPOINT_SCHEMA_VERSION)),
        ("identity", identity_value(identity)),
        ("runtime", text(runtime_text(runtime))),
        ("head", full_head_value(head)),
        (
            "resource_projection",
            object(vec![
                ("revision", unsigned(head.resource_revision())),
                ("digest", digest_value(head.resource_projection_digest())),
                ("counters", counters_value(counters)),
            ]),
        ),
        (
            "events",
            CanonicalValue::Array(ordered_events.into_iter().map(event_record_value).collect()),
        ),
        (
            "commands",
            // The complete request and receipt are checkpoint inputs. Each
            // command's base/result checkpoint pair is instead verified as a
            // unique chain and bound by its record-set digest; including the
            // result checkpoint here would make the digest self-referential.
            CanonicalValue::Array(
                ordered_commands
                    .into_iter()
                    .map(command_record_core_value)
                    .collect(),
            ),
        ),
        (
            "outboxes",
            CanonicalValue::Array(
                ordered_outboxes
                    .into_iter()
                    .map(outbox_record_value)
                    .collect(),
            ),
        ),
    ]);
    let checkpoint_digest = hash_value_at_version(
        "lattice.task-ledger.checkpoint",
        LEDGER_CHECKPOINT_SCHEMA_VERSION,
        &subject,
    )?;
    Ok(LedgerCheckpoint {
        stream_id: head.stream_id().clone(),
        runtime,
        checkpoint_digest,
    })
}

fn validate_verified_checkpoint(stream: &VerifiedStream) -> Result<(), LedgerError> {
    validate_full_head(&stream.head, stream.runtime())?;
    if stream.head.identity() != &stream.identity {
        return Err(LedgerError::InvalidStreamHead);
    }
    let rebuilt = build_checkpoint(
        &stream.identity,
        stream.runtime(),
        &stream.head,
        &stream.counters,
        &stream.events,
        &stream.commands,
        &stream.outboxes,
    )?;
    if rebuilt != stream.checkpoint {
        return Err(LedgerError::CheckpointMismatch);
    }
    Ok(())
}

fn validate_command_checkpoint_chain(expected: &VerifiedStream) -> Result<(), LedgerError> {
    let mut current = VerifiedStream::vacant(expected.identity.clone(), expected.runtime())?;
    let mut remaining = expected.commands.clone();
    while !remaining.is_empty() {
        let matches = remaining
            .iter()
            .enumerate()
            .filter(|(_, record)| record.base_checkpoint == current.checkpoint)
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err(LedgerError::CheckpointMismatch);
        }
        let record = remaining.remove(matches[0]);
        let plan = plan_append(&current, record.request.clone())?;
        if plan.is_exact_retry()
            || plan.receipt() != &record.receipt
            || plan.command_record().base_checkpoint != record.base_checkpoint
            || plan.command_record().result_checkpoint != record.result_checkpoint
        {
            return Err(LedgerError::CheckpointMismatch);
        }
        current = apply_append_plan(&current, &plan)?;
    }
    if &current != expected {
        return Err(LedgerError::CheckpointMismatch);
    }
    Ok(())
}

fn event_subject(
    command: &AppendCommand,
    sequence: u64,
    request_digest: &ContentDigest,
    resource_revision: u64,
    resource_projection_digest: &ContentDigest,
) -> CanonicalValue {
    object(vec![
        ("schema_version", text(LEDGER_SCHEMA_VERSION)),
        (
            "stream_identity",
            identity_value(command.expected_head.identity()),
        ),
        ("stream_id", digest_value(command.expected_head.stream_id())),
        ("sequence", unsigned(sequence)),
        (
            "previous_event_digest",
            digest_value(command.expected_head.last_event_digest()),
        ),
        ("command_id", text(command.command_id.as_str())),
        ("request_digest", digest_value(request_digest)),
        ("correlation_id", text(command.correlation_id.as_str())),
        ("occurred_at", text(&command.occurred_at)),
        ("kind", text(command.kind.as_str())),
        ("actor_id", text(command.actor_id.as_str())),
        ("action", text(command.action.as_str())),
        ("outcome", text(command.outcome.as_str())),
        ("reason_code", text(command.reason_code.as_str())),
        ("subject_digest", digest_value(&command.subject_digest)),
        (
            "diagnostic",
            optional(
                command
                    .diagnostic
                    .as_ref()
                    .map(|diagnostic| diagnostic.value().clone()),
            ),
        ),
        (
            "resource_snapshot",
            optional(command.resource_snapshot.as_ref().map(snapshot_value)),
        ),
        ("resource_revision", unsigned(resource_revision)),
        (
            "resource_projection_digest",
            digest_value(resource_projection_digest),
        ),
    ])
}

fn build_event(
    command: &AppendCommand,
    sequence: u64,
    request_digest: ContentDigest,
    resource_revision: u64,
    resource_projection_digest: ContentDigest,
) -> Result<LedgerEvent, LedgerError> {
    let subject = event_subject(
        command,
        sequence,
        &request_digest,
        resource_revision,
        &resource_projection_digest,
    );
    let event_digest = hash_value("lattice.task-ledger.event", &subject)?;
    Ok(LedgerEvent {
        schema_version: LEDGER_SCHEMA_VERSION.to_owned(),
        stream_identity: command.expected_head.identity().clone(),
        stream_id: command.expected_head.stream_id().clone(),
        sequence,
        previous_event_digest: command.expected_head.last_event_digest().clone(),
        command_id: command.command_id.clone(),
        request_digest,
        correlation_id: command.correlation_id.clone(),
        occurred_at: command.occurred_at.clone(),
        kind: command.kind,
        actor_id: command.actor_id.clone(),
        action: command.action.clone(),
        outcome: command.outcome,
        reason_code: command.reason_code.clone(),
        subject_digest: command.subject_digest.clone(),
        diagnostic: command.diagnostic.clone(),
        resource_snapshot: command.resource_snapshot.clone(),
        resource_revision,
        resource_projection_digest,
        event_digest,
    })
}

fn command_receipt(
    command_id: CommandId,
    request_digest: ContentDigest,
    before: TaskLedgerStreamHead,
    after: TaskLedgerStreamHead,
    outcome: CommandOutcome,
    event_digest: Option<ContentDigest>,
) -> Result<CommandReceipt, LedgerError> {
    if before.runtime() != after.runtime() {
        return Err(LedgerError::InvalidStreamHead);
    }
    let runtime = before.runtime();
    let outcome_value = match &outcome {
        CommandOutcome::Appended => object(vec![
            ("kind", text("APPENDED")),
            ("reason", CanonicalValue::Null),
        ]),
        CommandOutcome::Denied(reason) => object(vec![
            ("kind", text("DENIED")),
            ("reason", text(reason.as_str())),
        ]),
    };
    let subject = object(vec![
        ("producer_id", text(TASK_LEDGER_PRODUCER_ID)),
        ("producer_version", text(TASK_LEDGER_PRODUCER_VERSION)),
        ("runtime", text(runtime_text(runtime))),
        ("command_id", text(command_id.as_str())),
        ("request_digest", digest_value(&request_digest)),
        ("before", full_head_value(&before)),
        ("after", full_head_value(&after)),
        ("outcome", outcome_value),
        (
            "event_digest",
            optional(event_digest.as_ref().map(digest_value)),
        ),
    ]);
    let receipt_digest = hash_value("lattice.task-ledger.command-receipt", &subject)?;
    Ok(CommandReceipt {
        command_id,
        request_digest,
        before,
        after,
        outcome,
        event_digest,
        receipt_digest,
    })
}

fn verify_receipt(
    receipt: &CommandReceipt,
    command_id: &CommandId,
    request_digest: &ContentDigest,
    before: &TaskLedgerStreamHead,
    after: &TaskLedgerStreamHead,
    event_digest: Option<&ContentDigest>,
) -> Result<(), LedgerError> {
    if receipt.command_id() != command_id
        || receipt.request_digest() != request_digest
        || receipt.before() != before
        || receipt.after() != after
        || receipt.outcome() != &CommandOutcome::Appended
        || receipt.event_digest() != event_digest
    {
        return Err(LedgerError::ReceiptBindingMismatch);
    }
    let rebuilt = command_receipt(
        receipt.command_id.clone(),
        receipt.request_digest.clone(),
        receipt.before.clone(),
        receipt.after.clone(),
        receipt.outcome.clone(),
        receipt.event_digest.clone(),
    )?;
    if rebuilt.receipt_digest != receipt.receipt_digest {
        return Err(LedgerError::ReceiptBindingMismatch);
    }
    Ok(())
}

fn next_resource_projection(
    head: &TaskLedgerStreamHead,
    current: &ResourceCounters,
    command: &AppendCommand,
) -> Result<(ResourceCounters, u64, ContentDigest), LedgerError> {
    let Some(snapshot) = command.resource_snapshot.as_ref() else {
        return Ok((
            current.clone(),
            head.resource_revision(),
            head.resource_projection_digest().clone(),
        ));
    };
    let next = snapshot.counters();
    if next.elapsed_seconds() < current.elapsed_seconds()
        || next.attempt_number() < current.attempt_number()
        || next.used_model_calls() < current.used_model_calls()
        || decimal_less(next.used_external_cost(), current.used_external_cost())
    {
        return Err(LedgerError::ResourceCounterRegression);
    }
    let revision = head
        .resource_revision()
        .checked_add(1)
        .ok_or(LedgerError::InvalidResourceSnapshot)?;
    let accounting_currency = head
        .identity()
        .accounting_currency()
        .ok_or(LedgerError::GeneralTaskIntakeCreateOnly)?;
    let projection_digest = hash_value(
        "lattice.task-ledger.resource-projection",
        &object(vec![
            ("stream_id", digest_value(head.stream_id())),
            ("revision", unsigned(revision)),
            ("counters", counters_value(next)),
            ("accounting_currency", text(accounting_currency)),
        ]),
    )?;
    Ok((next.clone(), revision, projection_digest))
}

fn decimal_less(left: &str, right: &str) -> bool {
    let (left_integer, left_fraction) = decimal_parts(left);
    let (right_integer, right_fraction) = decimal_parts(right);
    match left_integer.len().cmp(&right_integer.len()) {
        std::cmp::Ordering::Less => true,
        std::cmp::Ordering::Greater => false,
        std::cmp::Ordering::Equal => match left_integer.cmp(right_integer) {
            std::cmp::Ordering::Less => true,
            std::cmp::Ordering::Greater => false,
            std::cmp::Ordering::Equal => {
                let width = left_fraction.len().max(right_fraction.len());
                (0..width).any(|index| {
                    let left_digit = left_fraction.as_bytes().get(index).copied().unwrap_or(b'0');
                    let right_digit = right_fraction
                        .as_bytes()
                        .get(index)
                        .copied()
                        .unwrap_or(b'0');
                    left_digit != right_digit
                        && left_digit < right_digit
                        && left_fraction
                            .as_bytes()
                            .iter()
                            .zip(right_fraction.as_bytes())
                            .take(index)
                            .all(|(left, right)| left == right)
                })
            }
        },
    }
}

fn decimal_parts(value: &str) -> (&str, &str) {
    value.split_once('.').unwrap_or((value, ""))
}

fn resource_observation_value(
    head: &TaskLedgerStreamHead,
    observation_revision: u64,
    effect_claim_id: &str,
    effect_subject_digest: &ContentDigest,
    counters: &ResourceCounters,
    request: &ResourceRequest,
    accounting_currency: &str,
) -> CanonicalValue {
    object(vec![
        ("stream_head", full_head_value(head)),
        ("observation_revision", unsigned(observation_revision)),
        ("effect_claim_id", text(effect_claim_id)),
        ("effect_subject_digest", digest_value(effect_subject_digest)),
        ("counters", counters_value(counters)),
        ("request", request_value(request)),
        ("accounting_currency", text(accounting_currency)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use lattice_contracts::{ProjectId, ProjectSnapshotId, TaskId};

    fn digest(byte: char) -> ContentDigest {
        ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("valid digest")
    }

    fn identity() -> TaskLedgerStreamIdentity {
        TaskLedgerStreamIdentity::new(
            ProjectId::new("project-1").expect("project"),
            ProjectSnapshotId::new("project-1:snapshot:1").expect("snapshot"),
            TaskId::new("TASK-013").expect("task"),
            "1",
            digest('a'),
            "TWD",
        )
        .expect("identity")
    }

    fn append_command(
        head: TaskLedgerStreamHead,
        command_id: &str,
        subject: char,
    ) -> AppendCommand {
        AppendCommand::new(
            head,
            CommandId::new(command_id).expect("command"),
            CorrelationId::new("correlation-1").expect("correlation"),
            "2026-07-29T00:00:00Z",
            LedgerEventKind::TaskCreated,
            ActorId::new("lattice-pm").expect("actor"),
            ActionId::new("record-task").expect("action"),
            LedgerOutcome::Recorded,
            ReasonCode::new("TASK_ACCEPTED").expect("reason"),
            digest(subject),
            None,
            None,
        )
        .expect("append command")
    }

    #[test]
    fn unpersistable_ingress_handoff_kind_is_not_part_of_schema_v5() {
        assert_eq!(
            LedgerEventKind::parse("INGRESS_RECEIPT_HANDOFF"),
            Err(LedgerError::UnknownEventKind)
        );
    }

    #[test]
    fn persisted_writer_tuple_rejects_zero_and_out_of_range_fences() {
        let zero = zero_digest();
        let nonzero = digest('a');
        assert!(valid_autonomy_writer_scalar_tuple(None, None, None));
        assert!(valid_autonomy_writer_scalar_tuple(
            Some(&nonzero),
            Some(&nonzero),
            Some(1)
        ));
        assert!(!valid_autonomy_writer_scalar_tuple(
            Some(&zero),
            Some(&nonzero),
            Some(1)
        ));
        assert!(!valid_autonomy_writer_scalar_tuple(
            Some(&nonzero),
            Some(&zero),
            Some(1)
        ));
        assert!(!valid_autonomy_writer_scalar_tuple(
            Some(&nonzero),
            Some(&nonzero),
            Some(0)
        ));
        assert!(!valid_autonomy_writer_scalar_tuple(
            Some(&nonzero),
            Some(&nonzero),
            Some(u64::MAX)
        ));
    }

    #[test]
    fn autonomy_order_validator_is_shared_by_append_and_untrusted_replay() {
        assert_eq!(
            validate_autonomy_order(LedgerEventKind::AutonomyReceiptRecorded, &[]),
            Err(LedgerError::InvalidAutonomyReceipt)
        );

        let identity = identity();
        let zero = FakeTaskLedger::zero_head(identity.clone()).expect("zero");
        let mut ledger = FakeTaskLedger::new();
        ledger
            .execute(append_command(zero.clone(), "create", 'b'))
            .expect("created");
        let created = ledger
            .planning_stream(zero.stream_id(), &identity)
            .expect("created stream");
        validate_autonomy_order(LedgerEventKind::AutonomyReceiptRecorded, created.events())
            .expect("exact sequence two");

        let duplicated = vec![created.events()[0].clone(), created.events()[0].clone()];
        assert_eq!(
            validate_autonomy_order(
                LedgerEventKind::AutonomyReceiptRecorded,
                duplicated.as_slice()
            ),
            Err(LedgerError::InvalidAutonomyReceipt)
        );
    }

    #[test]
    fn managed_profile_planner_rejects_a_non_spec_subject() {
        let vacant = VerifiedStream::vacant(identity(), RuntimeKind::Fake).expect("vacant");
        let forged = AppendCommand::from_fields(
            vacant.head().clone(),
            CommandId::new("forged-managed-subject").expect("command"),
            CorrelationId::new("managed-general-task-v1").expect("correlation"),
            "2026-08-26T00:00:00Z",
            LedgerEventKind::TaskCreated,
            ActorId::new("lattice-foreman").expect("actor"),
            ActionId::new(TaskCreatedProfile::ManagedGeneralTaskV1.action()).expect("action"),
            LedgerOutcome::Recorded,
            ReasonCode::new("MANAGED_GENERAL_TASK_ACCEPTED").expect("reason"),
            digest('b'),
            None,
            None,
            AppendConstruction::VerifiedReplay,
        )
        .expect("shape-valid forged retained command");

        assert_eq!(
            plan_append(&vacant, forged),
            Err(LedgerError::InvalidStreamHead)
        );
    }

    fn self_consistent_managed_profile_stream(subject: char) -> VerifiedStream {
        let vacant = VerifiedStream::vacant(identity(), RuntimeKind::Fake).expect("vacant");
        let forged_command = AppendCommand::from_fields(
            vacant.head().clone(),
            CommandId::new("retained-managed-subject").expect("command"),
            CorrelationId::new("managed-general-task-v1").expect("correlation"),
            "2026-08-26T00:00:00Z",
            LedgerEventKind::TaskCreated,
            ActorId::new("lattice-foreman").expect("actor"),
            ActionId::new(TaskCreatedProfile::ManagedGeneralTaskV1.action()).expect("action"),
            LedgerOutcome::Recorded,
            ReasonCode::new("MANAGED_GENERAL_TASK_ACCEPTED").expect("reason"),
            digest(subject),
            None,
            None,
            AppendConstruction::VerifiedReplay,
        )
        .expect("shape-valid retained command");
        let request_digest = request_digest(&forged_command).expect("request digest");
        let (counters, revision, projection_digest) =
            next_resource_projection(vacant.head(), &vacant.counters, &forged_command)
                .expect("projection");
        let forged_event = build_event(
            &forged_command,
            1,
            request_digest.clone(),
            revision,
            projection_digest.clone(),
        )
        .expect("self-consistent event");
        let head = build_head(
            RuntimeKind::Fake,
            vacant.identity.clone(),
            vacant.head.stream_id().clone(),
            1,
            forged_event.event_digest().clone(),
            revision,
            projection_digest,
        )
        .expect("self-consistent head");
        let receipt = command_receipt(
            forged_command.command_id.clone(),
            request_digest,
            vacant.head.clone(),
            head.clone(),
            CommandOutcome::Appended,
            Some(forged_event.event_digest().clone()),
        )
        .expect("self-consistent receipt");
        let events = vec![forged_event];
        let mut commands = vec![VerifiedCommandRecord {
            request: forged_command,
            receipt,
            base_checkpoint: vacant.checkpoint.clone(),
            result_checkpoint: vacant.checkpoint.clone(),
        }];
        let checkpoint = build_checkpoint(
            &vacant.identity,
            RuntimeKind::Fake,
            &head,
            &counters,
            &events,
            &commands,
            &[],
        )
        .expect("self-consistent checkpoint");
        commands[0].result_checkpoint = checkpoint.clone();
        VerifiedStream {
            identity: vacant.identity,
            head,
            events,
            commands,
            outboxes: Vec::new(),
            counters,
            checkpoint,
        }
    }

    #[test]
    fn managed_profile_replay_accepts_the_exact_spec_subject() {
        let retained = self_consistent_managed_profile_stream('a');
        verify_untrusted_snapshot(&export_untrusted_snapshot(&retained))
            .expect("exact Task Spec subject replays");
    }

    #[test]
    fn managed_profile_replay_rejects_a_self_consistent_non_spec_subject() {
        let retained = self_consistent_managed_profile_stream('b');
        assert_eq!(
            verify_untrusted_snapshot(&export_untrusted_snapshot(&retained)),
            Err(LedgerError::InvalidStreamHead)
        );
    }

    #[test]
    fn untrusted_replay_rejects_a_self_consistent_late_required_profile() {
        let vacant = VerifiedStream::vacant(identity(), RuntimeKind::Fake).expect("vacant");
        let first_plan = plan_append(
            &vacant,
            append_command(vacant.head().clone(), "ordinary-first", 'b'),
        )
        .expect("ordinary first plan");
        let first = apply_append_plan(&vacant, &first_plan).expect("ordinary first stream");
        let late_command = AppendCommand::from_fields(
            first.head().clone(),
            CommandId::new("late-required").expect("command"),
            CorrelationId::new("correlation-1").expect("correlation"),
            "2026-07-29T00:00:01Z",
            LedgerEventKind::TaskCreated,
            ActorId::new("lattice-runtime").expect("actor"),
            ActionId::new(TaskCreatedProfile::AutonomyReceiptRequiredV1.action()).expect("action"),
            LedgerOutcome::Recorded,
            ReasonCode::new("TASK038_TASK_ACCEPTED").expect("reason"),
            digest('c'),
            None,
            None,
            AppendConstruction::VerifiedReplay,
        )
        .expect("shape-valid late required command");
        let request_digest = request_digest(&late_command).expect("request digest");
        let (counters, revision, projection_digest) =
            next_resource_projection(first.head(), &first.counters, &late_command)
                .expect("projection");
        let late_event = build_event(
            &late_command,
            2,
            request_digest.clone(),
            revision,
            projection_digest.clone(),
        )
        .expect("late event");
        let head = build_head(
            RuntimeKind::Fake,
            first.identity.clone(),
            first.head.stream_id().clone(),
            2,
            late_event.event_digest().clone(),
            revision,
            projection_digest,
        )
        .expect("head");
        let receipt = command_receipt(
            late_command.command_id.clone(),
            request_digest,
            first.head.clone(),
            head.clone(),
            CommandOutcome::Appended,
            Some(late_event.event_digest().clone()),
        )
        .expect("receipt");
        let mut events = first.events.clone();
        events.push(late_event);
        let mut commands = first.commands.clone();
        commands.push(VerifiedCommandRecord {
            request: late_command,
            receipt,
            base_checkpoint: first.checkpoint.clone(),
            result_checkpoint: first.checkpoint.clone(),
        });
        canonicalize_commands(&mut commands);
        let checkpoint = build_checkpoint(
            &first.identity,
            RuntimeKind::Fake,
            &head,
            &counters,
            &events,
            &commands,
            &first.outboxes,
        )
        .expect("checkpoint");
        commands
            .iter_mut()
            .find(|record| record.request.command_id.as_str() == "late-required")
            .expect("late command")
            .result_checkpoint = checkpoint.clone();
        let late = VerifiedStream {
            identity: first.identity,
            head,
            events,
            commands,
            outboxes: first.outboxes,
            counters,
            checkpoint,
        };

        assert_eq!(
            verify_untrusted_snapshot(&export_untrusted_snapshot(&late)),
            Err(LedgerError::InvalidAutonomyReceipt)
        );
    }

    fn resource_command(
        head: TaskLedgerStreamHead,
        command_id: &str,
        counters: ResourceCounters,
    ) -> AppendCommand {
        AppendCommand::new(
            head,
            CommandId::new(command_id).expect("command"),
            CorrelationId::new("correlation-1").expect("correlation"),
            "2026-07-29T00:01:00Z",
            LedgerEventKind::ResourceSnapshot,
            ActorId::new("runtime-supervisor").expect("actor"),
            ActionId::new("record-resources").expect("action"),
            LedgerOutcome::Recorded,
            ReasonCode::new("RESOURCE_SNAPSHOT").expect("reason"),
            digest('d'),
            None,
            Some(ResourceSnapshot::new(counters)),
        )
        .expect("resource command")
    }

    fn two_event_ledger() -> (FakeTaskLedger, ContentDigest) {
        let mut ledger = FakeTaskLedger::new();
        let zero = FakeTaskLedger::zero_head(identity()).expect("zero");
        let first = ledger
            .execute(append_command(zero, "command-1", 'b'))
            .expect("first");
        let second = ledger
            .execute(append_command(first.after().clone(), "command-2", 'c'))
            .expect("second");
        (ledger, second.after().stream_id().clone())
    }

    fn resource_ledger(counters: ResourceCounters) -> (FakeTaskLedger, TaskLedgerStreamHead) {
        let mut ledger = FakeTaskLedger::new();
        let zero = FakeTaskLedger::zero_head(identity()).expect("zero");
        let first = ledger
            .execute(append_command(zero, "command-1", 'b'))
            .expect("first");
        let resource = ledger
            .execute(resource_command(
                first.after().clone(),
                "resource-1",
                counters,
            ))
            .expect("resource");
        (ledger, resource.after().clone())
    }

    #[test]
    fn decimal_comparison_uses_the_first_different_digit() {
        for (left, right, expected) in [
            ("0", "0", false),
            ("0.001", "0.01", true),
            ("0.010", "0.001", false),
            ("1.2", "1.19", false),
            ("1.019", "1.02", true),
            ("9.999", "10", true),
            ("10.0001", "10", false),
        ] {
            assert_eq!(
                decimal_less(left, right),
                expected,
                "unexpected ordering for {left} < {right}"
            );
        }
    }

    #[test]
    fn request_digest_changes_for_every_semantic_command_field() {
        let zero = FakeTaskLedger::zero_head(identity()).expect("zero");
        let baseline = append_command(zero.clone(), "command-1", 'b');
        let baseline_digest = request_digest(&baseline).expect("digest");
        let mut variants = Vec::new();

        let mut changed = baseline.clone();
        changed.expected_head = FakeTaskLedger::zero_head(
            TaskLedgerStreamIdentity::new(
                ProjectId::new("project-2").expect("project"),
                ProjectSnapshotId::new("project-2:snapshot:1").expect("snapshot"),
                TaskId::new("TASK-013").expect("task"),
                "1",
                digest('a'),
                "TWD",
            )
            .expect("identity"),
        )
        .expect("head");
        variants.push(("expected_head", changed));
        let mut changed = baseline.clone();
        changed.command_id = CommandId::new("command-2").expect("command");
        variants.push(("command_id", changed));
        let mut changed = baseline.clone();
        changed.correlation_id = CorrelationId::new("correlation-2").expect("correlation");
        variants.push(("correlation_id", changed));
        let mut changed = baseline.clone();
        changed.occurred_at = "2026-07-29T00:00:01Z".to_owned();
        variants.push(("occurred_at", changed));
        let mut changed = baseline.clone();
        changed.kind = LedgerEventKind::EvidenceRecorded;
        variants.push(("kind", changed));
        let mut changed = baseline.clone();
        changed.actor_id = ActorId::new("lattice-pm-2").expect("actor");
        variants.push(("actor_id", changed));
        let mut changed = baseline.clone();
        changed.action = ActionId::new("record-evidence").expect("action");
        variants.push(("action", changed));
        let mut changed = baseline.clone();
        changed.outcome = LedgerOutcome::Passed;
        variants.push(("outcome", changed));
        let mut changed = baseline.clone();
        changed.reason_code = ReasonCode::new("EVIDENCE_ACCEPTED").expect("reason");
        variants.push(("reason_code", changed));
        let mut changed = baseline.clone();
        changed.subject_digest = digest('c');
        variants.push(("subject_digest", changed));
        let mut changed = baseline.clone();
        changed.diagnostic =
            Some(Diagnostic::new(text("sanitized diagnostic")).expect("diagnostic"));
        variants.push(("diagnostic", changed));
        let mut changed = baseline;
        changed.resource_snapshot = Some(ResourceSnapshot::new(zero_counters()));
        variants.push(("resource_snapshot", changed));

        for (field, command) in variants {
            assert_ne!(
                request_digest(&command).expect("digest"),
                baseline_digest,
                "{field} must be bound by the request digest"
            );
        }
    }

    #[test]
    fn replay_rejects_unknown_reorder_duplicate_truncate_and_field_corruption() {
        let (mut unknown, stream_id) = two_event_ledger();
        unknown
            .streams
            .get_mut(stream_id.as_str())
            .expect("stream")
            .events[0]
            .schema_version = "9.0".to_owned();
        assert_eq!(
            unknown.verified_stream(&stream_id),
            Err(LedgerError::UnknownEventVersion)
        );

        let (mut reordered, stream_id) = two_event_ledger();
        reordered
            .streams
            .get_mut(stream_id.as_str())
            .expect("stream")
            .events
            .swap(0, 1);
        assert_eq!(
            reordered.verified_stream(&stream_id),
            Err(LedgerError::CorruptSequence)
        );

        let (mut duplicated, stream_id) = two_event_ledger();
        let duplicate = duplicated
            .streams
            .get(stream_id.as_str())
            .expect("stream")
            .events[0]
            .clone();
        duplicated
            .streams
            .get_mut(stream_id.as_str())
            .expect("stream")
            .events
            .push(duplicate);
        assert_eq!(
            duplicated.verified_stream(&stream_id),
            Err(LedgerError::CorruptSequence)
        );

        let (mut truncated, stream_id) = two_event_ledger();
        truncated
            .streams
            .get_mut(stream_id.as_str())
            .expect("stream")
            .events
            .pop();
        assert_eq!(
            truncated.verified_stream(&stream_id),
            Err(LedgerError::HeadMismatch)
        );

        let (mut predecessor, stream_id) = two_event_ledger();
        predecessor
            .streams
            .get_mut(stream_id.as_str())
            .expect("stream")
            .events[1]
            .previous_event_digest = digest('f');
        assert_eq!(
            predecessor.verified_stream(&stream_id),
            Err(LedgerError::CorruptPredecessor)
        );

        let (mut request, stream_id) = two_event_ledger();
        request
            .streams
            .get_mut(stream_id.as_str())
            .expect("stream")
            .events[0]
            .request_digest = digest('f');
        assert_eq!(
            request.verified_stream(&stream_id),
            Err(LedgerError::RequestBindingMismatch)
        );

        let (mut event, stream_id) = two_event_ledger();
        event
            .streams
            .get_mut(stream_id.as_str())
            .expect("stream")
            .events[0]
            .event_digest = digest('f');
        assert_eq!(
            event.verified_stream(&stream_id),
            Err(LedgerError::CorruptEventHash)
        );

        let (mut projection, stream_id) = two_event_ledger();
        projection
            .streams
            .get_mut(stream_id.as_str())
            .expect("stream")
            .events[0]
            .resource_projection_digest = digest('f');
        assert_eq!(
            projection.verified_stream(&stream_id),
            Err(LedgerError::ResourceProjectionMismatch)
        );
    }

    #[test]
    fn replay_rejects_orphan_receipt_claimed_head_and_counter_disagreement() {
        let (mut orphan, stream_id) = two_event_ledger();
        orphan
            .commands
            .remove(&(stream_id.as_str().to_owned(), "command-1".to_owned()));
        assert_eq!(
            orphan.verified_stream(&stream_id),
            Err(LedgerError::OrphanReceipt)
        );

        let (mut wrong_head, stream_id) = two_event_ledger();
        let identity = wrong_head
            .streams
            .get(stream_id.as_str())
            .expect("stream")
            .identity
            .clone();
        wrong_head
            .streams
            .get_mut(stream_id.as_str())
            .expect("stream")
            .head = FakeTaskLedger::zero_head(identity).expect("zero");
        assert_eq!(
            wrong_head.verified_stream(&stream_id),
            Err(LedgerError::HeadMismatch)
        );

        let (mut wrong_counters, stream_id) = two_event_ledger();
        wrong_counters
            .streams
            .get_mut(stream_id.as_str())
            .expect("stream")
            .counters = ResourceCounters::new(1, 0, 0, 0, 0, "0").expect("counters");
        assert_eq!(
            wrong_counters.verified_stream(&stream_id),
            Err(LedgerError::ResourceProjectionMismatch)
        );
    }

    #[test]
    fn replay_rejects_self_consistent_receipt_command_substitution() {
        let (mut ledger, stream_id) = two_event_ledger();
        let key = (stream_id.as_str().to_owned(), "command-1".to_owned());
        let original = ledger.commands.get(&key).expect("command").receipt.clone();
        let rewritten = command_receipt(
            CommandId::new("substituted-command").expect("command"),
            original.request_digest.clone(),
            original.before.clone(),
            original.after.clone(),
            original.outcome.clone(),
            original.event_digest.clone(),
        )
        .expect("self-consistent receipt");
        ledger.commands.get_mut(&key).expect("command").receipt = rewritten;

        assert_eq!(
            ledger.verified_stream(&stream_id),
            Err(LedgerError::ReceiptBindingMismatch)
        );
    }

    #[test]
    fn exact_retry_rejects_a_corrupt_stored_terminal_record() {
        let (mut ledger, stream_id) = two_event_ledger();
        let key = (stream_id.as_str().to_owned(), "command-1".to_owned());
        let original = ledger.commands.get(&key).expect("command").receipt.clone();
        ledger.commands.get_mut(&key).expect("command").receipt = command_receipt(
            CommandId::new("substituted-command").expect("command"),
            original.request_digest.clone(),
            original.before.clone(),
            original.after.clone(),
            original.outcome.clone(),
            original.event_digest.clone(),
        )
        .expect("self-consistent receipt");

        assert_eq!(
            ledger.execute(append_command(
                FakeTaskLedger::zero_head(identity()).expect("zero"),
                "command-1",
                'b',
            )),
            Err(LedgerError::ReceiptBindingMismatch)
        );
    }

    #[test]
    fn exact_retry_rejects_self_consistent_receipt_event_substitution() {
        let (mut ledger, stream_id) = two_event_ledger();
        let key = (stream_id.as_str().to_owned(), "command-1".to_owned());
        let original = ledger.commands.get(&key).expect("command").receipt.clone();
        ledger.commands.get_mut(&key).expect("command").receipt = command_receipt(
            original.command_id.clone(),
            original.request_digest.clone(),
            original.before.clone(),
            original.after.clone(),
            original.outcome.clone(),
            Some(digest('f')),
        )
        .expect("self-consistent receipt");

        assert_eq!(
            ledger.execute(append_command(
                FakeTaskLedger::zero_head(identity()).expect("zero"),
                "command-1",
                'b',
            )),
            Err(LedgerError::ReceiptBindingMismatch)
        );
    }

    #[test]
    fn sequence_overflow_is_a_denial_without_stream_mutation() {
        let mut ledger = FakeTaskLedger::new();
        let zero = FakeTaskLedger::zero_head(identity()).expect("zero");
        let max_head = build_head(
            RuntimeKind::Fake,
            zero.identity().clone(),
            zero.stream_id().clone(),
            u64::MAX,
            digest('b'),
            0,
            zero_digest(),
        )
        .expect("max head");
        ledger.streams.insert(
            max_head.stream_id().as_str().to_owned(),
            StreamState {
                identity: max_head.identity().clone(),
                head: max_head.clone(),
                events: Vec::new(),
                outboxes: Vec::new(),
                counters: zero_counters(),
                observation_revision: 0,
                latest_observation: None,
            },
        );

        let receipt = ledger
            .execute(append_command(max_head.clone(), "overflow-command", 'c'))
            .expect("terminal denial");
        assert_eq!(
            receipt.outcome(),
            &CommandOutcome::Denied(LedgerDenial::SequenceOverflow)
        );
        assert_eq!(receipt.before(), &max_head);
        assert_eq!(receipt.after(), &max_head);
        assert_eq!(ledger.current_head(max_head.stream_id()), Some(max_head));
    }

    #[test]
    fn cumulative_resource_regression_fails_without_partial_mutation() {
        let baseline = ResourceCounters::new(3, 1, 20, 2, 4, "2.01").expect("counters");
        for regressed in [
            ResourceCounters::new(3, 1, 19, 2, 4, "2.01").expect("elapsed"),
            ResourceCounters::new(3, 1, 20, 1, 4, "2.01").expect("attempt"),
            ResourceCounters::new(3, 1, 20, 2, 3, "2.01").expect("model calls"),
            ResourceCounters::new(3, 1, 20, 2, 4, "2.001").expect("cost"),
        ] {
            let (mut ledger, head) = resource_ledger(baseline.clone());
            let before = ledger.verified_stream(head.stream_id()).expect("valid");
            assert_eq!(
                ledger.execute(resource_command(
                    head.clone(),
                    "resource-regression",
                    regressed,
                )),
                Err(LedgerError::ResourceCounterRegression)
            );
            assert_eq!(ledger.current_head(head.stream_id()), Some(head.clone()));
            assert_eq!(
                ledger.verified_stream(head.stream_id()).expect("valid"),
                before
            );
        }
        assert!(ResourceCounters::new(1, 2, 0, 0, 0, "0").is_err());
    }

    #[test]
    fn resource_observation_retry_and_replacement_currentness_are_exact() {
        let counters = ResourceCounters::new(2, 1, 20, 1, 2, "1").expect("counters");
        let (mut ledger, head) = resource_ledger(counters);
        let request = ResourceRequest::new(1, 0, 5, 0, 1, Some("0.1")).expect("request");
        let first = ledger
            .issue_resource_observation(
                head.clone(),
                &EffectClaimId::new("effect-claim-1").expect("claim"),
                digest('e'),
                request.clone(),
            )
            .expect("observation");
        let retry = ledger
            .issue_resource_observation(
                head.clone(),
                &EffectClaimId::new("effect-claim-1").expect("claim"),
                digest('e'),
                request.clone(),
            )
            .expect("retry");
        assert_eq!(retry, first);

        let replacement = ledger
            .issue_resource_observation(
                head,
                &EffectClaimId::new("effect-claim-2").expect("claim"),
                digest('f'),
                request,
            )
            .expect("replacement");
        assert_eq!(
            replacement.observation_revision(),
            first.observation_revision() + 1
        );
        assert_eq!(ledger.current_resource_head(&first), None);
        assert_eq!(
            ledger.current_resource_head(&replacement),
            Some(replacement.head())
        );
    }

    #[test]
    fn forged_cross_identity_head_cannot_poison_another_stream_command_key() {
        let identity_a = identity();
        let identity_b = TaskLedgerStreamIdentity::new(
            ProjectId::new("project-2").expect("project"),
            ProjectSnapshotId::new("project-2:snapshot:1").expect("snapshot"),
            TaskId::new("TASK-013").expect("task"),
            "1",
            digest('a'),
            "TWD",
        )
        .expect("identity");
        let zero_b = FakeTaskLedger::zero_head(identity_b).expect("zero B");
        let forged = build_head(
            RuntimeKind::Fake,
            identity_a,
            zero_b.stream_id().clone(),
            0,
            zero_digest(),
            0,
            zero_digest(),
        )
        .expect("shape-valid forged head");
        let mut ledger = FakeTaskLedger::new();

        assert_eq!(
            ledger.execute(append_command(forged, "shared-command", 'b',)),
            Err(LedgerError::InvalidStreamHead)
        );
        assert_eq!(
            ledger
                .execute(append_command(zero_b, "shared-command", 'c'))
                .expect("legitimate stream B command")
                .outcome(),
            &CommandOutcome::Appended
        );
    }

    #[test]
    fn stale_denial_for_an_uncreated_valid_stream_retries_identically() {
        let mut ledger = FakeTaskLedger::new();
        let zero = FakeTaskLedger::zero_head(identity()).expect("zero");
        let uncreated_nonzero = build_head(
            RuntimeKind::Fake,
            zero.identity().clone(),
            zero.stream_id().clone(),
            7,
            digest('b'),
            0,
            zero_digest(),
        )
        .expect("valid nonzero head");
        let first = ledger
            .execute(append_command(
                uncreated_nonzero.clone(),
                "stale-uncreated",
                'c',
            ))
            .expect("terminal stale denial");
        assert_eq!(
            first.outcome(),
            &CommandOutcome::Denied(LedgerDenial::StaleHead)
        );
        assert_eq!(ledger.current_head(zero.stream_id()), None);
        assert_eq!(
            ledger
                .execute(append_command(uncreated_nonzero, "stale-uncreated", 'c',))
                .expect("exact retry"),
            first
        );
        assert_eq!(ledger.current_head(zero.stream_id()), None);
    }
}
