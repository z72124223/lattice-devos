//! Deterministic, I/O-free command idempotency and strict history replay.
//!
//! This module owns command-history integrity only. A terminal projection
//! records digests supplied by the semantic owner; it never authorizes or
//! performs the state mutation represented by those digests.

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt;

use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256, canonicalize};
use lattice_contracts::{ArtifactCounter, ContentDigest, ProjectId};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

const HISTORY_VERSION: &str = "1";
const HASH_VERSION: &str = "1.0";
const MAX_COMMAND_ID_BYTES: usize = 256;
const MAX_DENIAL_CODE_BYTES: usize = 128;
const MAX_REQUEST_SOURCE_BYTES: usize = 65_536;
const MAX_REQUEST_SOURCE_COLLECTION_ENTRIES: usize = 1_024;
const MAX_REQUEST_SOURCE_NODES: usize = 4_096;
const MAX_REQUEST_SOURCE_FIELD_BYTES: usize = 256;
const MAX_REQUEST_SOURCE_STRING_BYTES: usize = 256;
const MAX_HISTORY_COMMAND_RECORDS: usize = 1_000_000;
const MAX_HISTORY_CANONICAL_BYTES: usize = 1_073_741_824;
const MAX_HISTORY_DEPTH: usize = 32;
const MAX_HISTORY_COLLECTION_ENTRIES: usize = 1_000_000;
const MAX_HISTORY_OBJECT_FIELDS: usize = 1_024;
const MAX_HISTORY_NODES: usize = 64_000_064;
const MAX_HISTORY_FIELD_BYTES: usize = 256;
const MAX_HISTORY_STRING_BYTES: usize = 65_536;

/// Every Artifact Store hash subject has a unique, frozen domain.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ArtifactHashDomain {
    /// Complete sanitized command request.
    CommandRequest,
    /// Terminal command record before its receipt envelope.
    CommandRecord,
    /// Current command-chain head.
    CommandHead,
    /// Immutable terminal command receipt.
    CommandReceipt,
    /// Independently retained trusted history checkpoint.
    HistoryCheckpoint,
    /// Read-only delete plan.
    DeletePlan,
    /// Exclusive delete claim.
    DeleteClaim,
    /// Terminal delete adapter result.
    DeleteResult,
}

impl ArtifactHashDomain {
    /// Complete closed hash-domain set.
    pub const ALL: [Self; 8] = [
        Self::CommandRequest,
        Self::CommandRecord,
        Self::CommandHead,
        Self::CommandReceipt,
        Self::HistoryCheckpoint,
        Self::DeletePlan,
        Self::DeleteClaim,
        Self::DeleteResult,
    ];

    /// Frozen schema identifier.
    #[must_use]
    pub const fn schema_id(self) -> &'static str {
        match self {
            Self::CommandRequest => "lattice.artifact.command-request",
            Self::CommandRecord => "lattice.artifact.command-record",
            Self::CommandHead => "lattice.artifact.command-head",
            Self::CommandReceipt => "lattice.artifact.command-receipt",
            Self::HistoryCheckpoint => "lattice.artifact.history-checkpoint",
            Self::DeletePlan => "lattice.artifact.delete-plan",
            Self::DeleteClaim => "lattice.artifact.delete-claim",
            Self::DeleteResult => "lattice.artifact.delete-result",
        }
    }

    /// Hashes a canonical subject under this exact domain.
    ///
    /// # Errors
    ///
    /// Returns a typed error if canonicalization or digest construction fails.
    pub fn digest(self, subject: &CanonicalValue) -> Result<ContentDigest, ArtifactHistoryError> {
        canonical_digest(self, subject)
    }
}

/// Closed set of terminal commands represented in Artifact Store history.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ArtifactCommandKind {
    /// Publish an object and its initial reference.
    Publish,
    /// Add an immutable reference.
    AddReference,
    /// Terminally release an immutable reference.
    ReleaseReference,
    /// Acquire a bounded read claim.
    AcquireRead,
    /// Terminally release a read claim.
    ReleaseRead,
    /// Mark an elapsed read claim as expired-suspect.
    ExpireRead,
    /// Reconcile an expired-suspect read.
    ReconcileRead,
    /// Change bounded staging state.
    Staging,
    /// Claim one exact object generation for deletion.
    DeleteClaim,
    /// Record a delete adapter result.
    DeleteResult,
    /// Reconcile an ambiguous delete result.
    DeleteReconcile,
}

impl ArtifactCommandKind {
    /// Complete closed command-kind set.
    pub const ALL: [Self; 11] = [
        Self::Publish,
        Self::AddReference,
        Self::ReleaseReference,
        Self::AcquireRead,
        Self::ReleaseRead,
        Self::ExpireRead,
        Self::ReconcileRead,
        Self::Staging,
        Self::DeleteClaim,
        Self::DeleteResult,
        Self::DeleteReconcile,
    ];

    /// Frozen serialized kind.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Publish => "PUBLISH",
            Self::AddReference => "ADD_REFERENCE",
            Self::ReleaseReference => "RELEASE_REFERENCE",
            Self::AcquireRead => "ACQUIRE_READ",
            Self::ReleaseRead => "RELEASE_READ",
            Self::ExpireRead => "EXPIRE_READ",
            Self::ReconcileRead => "RECONCILE_READ",
            Self::Staging => "STAGING",
            Self::DeleteClaim => "DELETE_CLAIM",
            Self::DeleteResult => "DELETE_RESULT",
            Self::DeleteReconcile => "DELETE_RECONCILE",
        }
    }

    fn parse(value: &str) -> Result<Self, ArtifactHistoryError> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.as_str() == value)
            .ok_or(ArtifactHistoryError::UnknownKind)
    }
}

/// Stable object scope for one independent command chain.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ArtifactCommandObjectScope {
    project_id: ProjectId,
    content_digest: ContentDigest,
}

impl ArtifactCommandObjectScope {
    /// Constructs a project-scoped SHA-256 object chain.
    #[must_use]
    pub const fn new(project_id: ProjectId, content_digest: ContentDigest) -> Self {
        Self {
            project_id,
            content_digest,
        }
    }

    /// Owning project.
    #[must_use]
    pub const fn project_id(&self) -> &ProjectId {
        &self.project_id
    }

    /// Frozen digest algorithm.
    #[must_use]
    #[allow(clippy::unused_self)]
    pub const fn algorithm(&self) -> &'static str {
        "sha256"
    }

    /// Exact content digest.
    #[must_use]
    pub const fn content_digest(&self) -> &ContentDigest {
        &self.content_digest
    }

    fn canonical_value(&self) -> CanonicalValue {
        object([
            string("project_id", self.project_id.as_str()),
            string("algorithm", self.algorithm()),
            string("content_digest", self.content_digest.as_str()),
        ])
    }
}

/// Exact idempotency storage key:
/// `(project_id, "sha256", content_digest, command_id)`.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ArtifactCommandStorageKey {
    scope: ArtifactCommandObjectScope,
    command_id: String,
}

impl ArtifactCommandStorageKey {
    /// Constructs an exact project/object/command key.
    ///
    /// # Errors
    ///
    /// Rejects empty, non-canonical, or oversized command identifiers.
    pub fn new(
        project_id: ProjectId,
        content_digest: ContentDigest,
        command_id: impl Into<String>,
    ) -> Result<Self, ArtifactHistoryError> {
        let command_id = command_id.into();
        validate_identifier(&command_id, MAX_COMMAND_ID_BYTES)
            .then_some(())
            .ok_or(ArtifactHistoryError::InvalidCommandId)?;
        Ok(Self {
            scope: ArtifactCommandObjectScope::new(project_id, content_digest),
            command_id,
        })
    }

    /// Owning project.
    #[must_use]
    pub const fn project_id(&self) -> &ProjectId {
        self.scope.project_id()
    }

    /// Frozen digest algorithm.
    #[must_use]
    pub const fn algorithm(&self) -> &'static str {
        self.scope.algorithm()
    }

    /// Exact content digest.
    #[must_use]
    pub const fn content_digest(&self) -> &ContentDigest {
        self.scope.content_digest()
    }

    /// Exact command identifier.
    #[must_use]
    pub fn command_id(&self) -> &str {
        &self.command_id
    }

    /// Independent object-chain scope.
    #[must_use]
    pub const fn scope(&self) -> &ArtifactCommandObjectScope {
        &self.scope
    }

    fn canonical_value(&self) -> CanonicalValue {
        object([
            string("project_id", self.project_id().as_str()),
            string("algorithm", self.algorithm()),
            string("content_digest", self.content_digest().as_str()),
            string("command_id", &self.command_id),
        ])
    }
}

/// Complete sanitized request retained as the idempotency comparison source.
///
/// `CanonicalValue` deliberately has no byte-array variant. Callers must place
/// content bytes in the physical byte adapter and retain only bounded semantic
/// metadata here.
#[derive(Clone, Eq, PartialEq)]
pub struct ArtifactCommandRequest {
    key: ArtifactCommandStorageKey,
    kind: ArtifactCommandKind,
    source: CanonicalValue,
    canonical_source: Vec<u8>,
    request_digest: ContentDigest,
}

impl ArtifactCommandRequest {
    /// Validates and hashes one full sanitized request.
    ///
    /// # Errors
    ///
    /// Rejects non-canonical request objects, including normalized duplicate
    /// keys.
    pub(crate) fn new(
        key: ArtifactCommandStorageKey,
        kind: ArtifactCommandKind,
        source: CanonicalValue,
    ) -> Result<Self, ArtifactHistoryError> {
        if !matches!(source, CanonicalValue::Object(_)) {
            return Err(ArtifactHistoryError::InvalidRequestSource {
                field: "request_source",
            });
        }
        validate_request_source(&source)?;
        let canonical_source = canonicalize(&source)
            .map_err(|_| ArtifactHistoryError::Canonicalization)?
            .into_vec();
        if canonical_source.len() > MAX_REQUEST_SOURCE_BYTES {
            return Err(ArtifactHistoryError::RequestSourceLimit {
                field: "canonical_bytes",
            });
        }
        let request_digest = canonical_digest(
            ArtifactHashDomain::CommandRequest,
            &request_subject(&key, kind, &source),
        )?;
        Ok(Self {
            key,
            kind,
            source,
            canonical_source,
            request_digest,
        })
    }

    /// Exact storage key.
    #[must_use]
    pub const fn key(&self) -> &ArtifactCommandStorageKey {
        &self.key
    }

    /// Closed command kind.
    #[must_use]
    pub const fn kind(&self) -> ArtifactCommandKind {
        self.kind
    }

    /// Complete sanitized comparison source.
    #[must_use]
    pub const fn source(&self) -> &CanonicalValue {
        &self.source
    }

    /// Request-domain digest.
    #[must_use]
    pub const fn request_digest(&self) -> &ContentDigest {
        &self.request_digest
    }

    fn exactly_matches(&self, other: &Self) -> bool {
        self.key == other.key
            && self.kind == other.kind
            && self.canonical_source == other.canonical_source
            && self.request_digest == other.request_digest
    }
}

impl fmt::Debug for ArtifactCommandRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ArtifactCommandRequest")
            .field("key", &self.key)
            .field("kind", &self.kind)
            .field("source", &"[ELIDED]")
            .field("canonical_source_bytes", &self.canonical_source.len())
            .field("request_digest", &self.request_digest)
            .finish()
    }
}

/// Whether one terminal command was applied or denied.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtifactCommandOutcome {
    /// The semantic owner applied the transition.
    Applied,
    /// The semantic owner denied the transition without mutation.
    Denied,
}

impl ArtifactCommandOutcome {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Applied => "APPLIED",
            Self::Denied => "DENIED",
        }
    }

    fn parse(value: &str) -> Result<Self, ArtifactHistoryError> {
        match value {
            "APPLIED" => Ok(Self::Applied),
            "DENIED" => Ok(Self::Denied),
            _ => Err(ArtifactHistoryError::Malformed { field: "outcome" }),
        }
    }
}

/// Non-authorizing terminal projection returned by the semantic owner.
///
/// These typed digests are evidence inputs. Constructing this value does not
/// grant currentness, authority, or permission to mutate any state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactCommandTerminalProjection {
    outcome: ArtifactCommandOutcome,
    denial_code: Option<String>,
    before_state_digest: ContentDigest,
    after_state_digest: ContentDigest,
    result_digest: ContentDigest,
}

impl ArtifactCommandTerminalProjection {
    /// Records typed evidence for a transition already applied by its owner.
    ///
    /// # Errors
    ///
    /// Rejects zero evidence digests.
    pub fn applied(
        before_state_digest: ContentDigest,
        after_state_digest: ContentDigest,
        result_digest: ContentDigest,
    ) -> Result<Self, ArtifactHistoryError> {
        validate_evidence_digests(&before_state_digest, &after_state_digest, &result_digest)?;
        Ok(Self {
            outcome: ArtifactCommandOutcome::Applied,
            denial_code: None,
            before_state_digest,
            after_state_digest,
            result_digest,
        })
    }

    /// Records typed evidence for a terminal denial.
    ///
    /// # Errors
    ///
    /// Rejects a non-canonical denial code or zero evidence digest.
    pub fn denied(
        denial_code: impl Into<String>,
        before_state_digest: ContentDigest,
        after_state_digest: ContentDigest,
        result_digest: ContentDigest,
    ) -> Result<Self, ArtifactHistoryError> {
        let denial_code = denial_code.into();
        if !validate_denial_code(&denial_code) {
            return Err(ArtifactHistoryError::InvalidDenialCode);
        }
        validate_evidence_digests(&before_state_digest, &after_state_digest, &result_digest)?;
        if before_state_digest != after_state_digest {
            return Err(ArtifactHistoryError::DeniedStateChanged);
        }
        Ok(Self {
            outcome: ArtifactCommandOutcome::Denied,
            denial_code: Some(denial_code),
            before_state_digest,
            after_state_digest,
            result_digest,
        })
    }
}

/// Immutable terminal command receipt.
#[derive(Clone, Eq, PartialEq)]
pub struct ArtifactCommandReceipt {
    request: ArtifactCommandRequest,
    ordinal: ArtifactCounter,
    predecessor_digest: Option<ContentDigest>,
    terminal: ArtifactCommandTerminalProjection,
    record_digest: ContentDigest,
    receipt_digest: ContentDigest,
}

impl fmt::Debug for ArtifactCommandReceipt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ArtifactCommandReceipt")
            .field("request", &self.request)
            .field("ordinal", &self.ordinal)
            .field("predecessor_digest", &self.predecessor_digest)
            .field("terminal", &self.terminal)
            .field("record_digest", &self.record_digest)
            .field("receipt_digest", &self.receipt_digest)
            .finish()
    }
}

impl ArtifactCommandReceipt {
    /// Complete request retained for exact retry comparison.
    #[must_use]
    pub const fn request(&self) -> &ArtifactCommandRequest {
        &self.request
    }

    /// One-based non-wrapping command ordinal.
    #[must_use]
    pub const fn ordinal(&self) -> ArtifactCounter {
        self.ordinal
    }

    /// Previous receipt digest, or `None` for ordinal one.
    #[must_use]
    pub const fn predecessor_digest(&self) -> Option<&ContentDigest> {
        self.predecessor_digest.as_ref()
    }

    /// Applied or denied terminal status.
    #[must_use]
    pub const fn outcome(&self) -> ArtifactCommandOutcome {
        self.terminal.outcome
    }

    /// Stable denial reason for a denied terminal record.
    #[must_use]
    pub fn denial_code(&self) -> Option<&str> {
        self.terminal.denial_code.as_deref()
    }

    /// State digest observed before evaluation.
    #[must_use]
    pub const fn before_state_digest(&self) -> &ContentDigest {
        &self.terminal.before_state_digest
    }

    /// State digest observed after terminal evaluation.
    #[must_use]
    pub const fn after_state_digest(&self) -> &ContentDigest {
        &self.terminal.after_state_digest
    }

    /// Semantic result digest.
    #[must_use]
    pub const fn result_digest(&self) -> &ContentDigest {
        &self.terminal.result_digest
    }

    /// Command-record-domain digest.
    #[must_use]
    pub const fn record_digest(&self) -> &ContentDigest {
        &self.record_digest
    }

    /// Command-receipt-domain digest and chain link.
    #[must_use]
    pub const fn receipt_digest(&self) -> &ContentDigest {
        &self.receipt_digest
    }

    /// Returns deterministic canonical receipt bytes for exact retry checks.
    ///
    /// # Errors
    ///
    /// Returns an error only if an internal canonical value cannot be encoded.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ArtifactHistoryError> {
        canonicalize(&self.raw_value())
            .map(lattice_cjson::CanonicalBytes::into_vec)
            .map_err(|_| ArtifactHistoryError::Canonicalization)
    }

    fn raw_value(&self) -> CanonicalValue {
        object([
            string("version", HISTORY_VERSION),
            ("key".to_owned(), self.request.key.canonical_value()),
            string("kind", self.request.kind.as_str()),
            ("request_source".to_owned(), self.request.source.clone()),
            string("request_digest", self.request.request_digest.as_str()),
            string("ordinal", self.ordinal.get().to_string()),
            (
                "predecessor_digest".to_owned(),
                optional_digest(self.predecessor_digest.as_ref()),
            ),
            string("outcome", self.terminal.outcome.as_str()),
            (
                "denial_code".to_owned(),
                optional_string(self.terminal.denial_code.as_deref()),
            ),
            string(
                "before_state_digest",
                self.terminal.before_state_digest.as_str(),
            ),
            string(
                "after_state_digest",
                self.terminal.after_state_digest.as_str(),
            ),
            string("result_digest", self.terminal.result_digest.as_str()),
            string("record_digest", self.record_digest.as_str()),
            string("receipt_digest", self.receipt_digest.as_str()),
        ])
    }
}

/// Whether execution appended a record or returned an exact prior receipt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtifactCommandExecutionDisposition {
    /// A new terminal record was appended.
    Recorded,
    /// The exact stored receipt was returned without evaluation.
    ExactRetry,
}

/// Result of one idempotent command-history execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactCommandExecution {
    disposition: ArtifactCommandExecutionDisposition,
    receipt: ArtifactCommandReceipt,
}

impl ArtifactCommandExecution {
    /// Retry or newly recorded disposition.
    #[must_use]
    pub const fn disposition(&self) -> ArtifactCommandExecutionDisposition {
        self.disposition
    }

    /// Immutable terminal receipt.
    #[must_use]
    pub const fn receipt(&self) -> &ArtifactCommandReceipt {
        &self.receipt
    }
}

/// Current integrity head of one project-scoped object command chain.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactCommandHistoryHead {
    scope: ArtifactCommandObjectScope,
    high_water: ArtifactCounter,
    tail_digest: Option<ContentDigest>,
    denial_count: ArtifactCounter,
    denial_tail_digest: Option<ContentDigest>,
    head_digest: ContentDigest,
}

impl ArtifactCommandHistoryHead {
    /// Independent object scope.
    #[must_use]
    pub const fn scope(&self) -> &ArtifactCommandObjectScope {
        &self.scope
    }

    /// Number of applied and denied terminal records.
    #[must_use]
    pub const fn high_water(&self) -> ArtifactCounter {
        self.high_water
    }

    /// Tail of the shared applied/denied chain.
    #[must_use]
    pub const fn tail_digest(&self) -> Option<&ContentDigest> {
        self.tail_digest.as_ref()
    }

    /// Number of denied records retained in the shared chain.
    #[must_use]
    pub const fn denial_count(&self) -> ArtifactCounter {
        self.denial_count
    }

    /// Most recent denied receipt digest.
    #[must_use]
    pub const fn denial_tail_digest(&self) -> Option<&ContentDigest> {
        self.denial_tail_digest.as_ref()
    }

    /// Command-head-domain digest.
    #[must_use]
    pub const fn head_digest(&self) -> &ContentDigest {
        &self.head_digest
    }

    fn canonical_subject(&self) -> CanonicalValue {
        head_subject(
            &self.scope,
            self.high_water,
            self.tail_digest.as_ref(),
            self.denial_count,
            self.denial_tail_digest.as_ref(),
        )
    }
}

/// Independently retained trusted checkpoint used to reject valid old prefixes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactHistoryCheckpoint {
    head: ArtifactCommandHistoryHead,
    checkpoint_digest: ContentDigest,
}

impl ArtifactHistoryCheckpoint {
    /// Constructs a checkpoint from independently trusted head fields.
    ///
    /// # Errors
    ///
    /// Rejects inconsistent zero/non-zero tail and denial fields.
    pub fn new_trusted(
        scope: ArtifactCommandObjectScope,
        high_water: ArtifactCounter,
        tail_digest: Option<ContentDigest>,
        denial_count: ArtifactCounter,
        denial_tail_digest: Option<ContentDigest>,
    ) -> Result<Self, ArtifactHistoryError> {
        validate_head_shape(
            high_water,
            tail_digest.as_ref(),
            denial_count,
            denial_tail_digest.as_ref(),
        )?;
        let head_digest = canonical_digest(
            ArtifactHashDomain::CommandHead,
            &head_subject(
                &scope,
                high_water,
                tail_digest.as_ref(),
                denial_count,
                denial_tail_digest.as_ref(),
            ),
        )?;
        let head = ArtifactCommandHistoryHead {
            scope,
            high_water,
            tail_digest,
            denial_count,
            denial_tail_digest,
            head_digest,
        };
        let checkpoint_digest = canonical_digest(
            ArtifactHashDomain::HistoryCheckpoint,
            &checkpoint_subject(&head),
        )?;
        Ok(Self {
            head,
            checkpoint_digest,
        })
    }

    /// Trusted current head.
    #[must_use]
    pub const fn head(&self) -> &ArtifactCommandHistoryHead {
        &self.head
    }

    /// Checkpoint-domain digest.
    #[must_use]
    pub const fn checkpoint_digest(&self) -> &ContentDigest {
        &self.checkpoint_digest
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct CommandStream {
    records: Vec<ArtifactCommandReceipt>,
    denial_count: u64,
    denial_tail_digest: Option<ContentDigest>,
}

/// Deterministic command history with exact-key idempotency.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ArtifactCommandHistory {
    streams: HashMap<ArtifactCommandObjectScope, CommandStream>,
    receipts: HashMap<ArtifactCommandStorageKey, ArtifactCommandReceipt>,
}

impl ArtifactCommandHistory {
    /// Constructs an empty history.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Looks up one request before any currentness, time, or semantic evaluation.
    ///
    /// An exact stored request returns its immutable receipt, a changed request
    /// under the same storage key fails permanently, and a new key returns
    /// `None`. This method never mutates history.
    ///
    /// # Errors
    ///
    /// Returns `CommandIdReuse` when the exact key already exists with a
    /// different kind or canonical request source.
    pub(crate) fn lookup_request(
        &self,
        request: &ArtifactCommandRequest,
    ) -> Result<Option<ArtifactCommandReceipt>, ArtifactHistoryError> {
        let Some(stored) = self.receipts.get(request.key()) else {
            return Ok(None);
        };
        if !stored.request.exactly_matches(request) {
            return Err(ArtifactHistoryError::CommandIdReuse);
        }
        Ok(Some(stored.clone()))
    }

    /// Returns every retained terminal receipt in stable storage-key order.
    ///
    /// This crate-private view lets the aggregate owner recompute command and
    /// canonical-history quota from its own immutable rows. It grants no
    /// mutation authority and never exposes artifact payload bytes.
    #[must_use]
    pub(crate) fn sorted_receipts(&self) -> Vec<&ArtifactCommandReceipt> {
        let mut receipts = self.receipts.values().collect::<Vec<_>>();
        receipts.sort_by(|left, right| {
            let left = left.request().key();
            let right = right.request().key();
            (
                left.project_id().as_str(),
                left.algorithm(),
                left.content_digest().as_str(),
                left.command_id(),
            )
                .cmp(&(
                    right.project_id().as_str(),
                    right.algorithm(),
                    right.content_digest().as_str(),
                    right.command_id(),
                ))
        });
        receipts
    }

    /// Executes exact retry lookup before invoking semantic evaluation.
    ///
    /// `evaluate` is the only place a caller should perform currentness or
    /// clock-sensitive work. It is never invoked for an exact retry or a
    /// reused command identifier.
    ///
    /// # Errors
    ///
    /// Returns `CommandIdReuse` when the exact key already exists with a
    /// different kind or full canonical request source. Other errors indicate
    /// invalid evidence or a non-wrapping counter failure.
    pub fn execute<F>(
        &mut self,
        request: ArtifactCommandRequest,
        evaluate: F,
    ) -> Result<ArtifactCommandExecution, ArtifactHistoryError>
    where
        F: FnOnce() -> Result<ArtifactCommandTerminalProjection, ArtifactHistoryError>,
    {
        if let Some(stored) = self.lookup_request(&request)? {
            return Ok(ArtifactCommandExecution {
                disposition: ArtifactCommandExecutionDisposition::ExactRetry,
                receipt: stored,
            });
        }

        let scope = request.key.scope.clone();
        let stream = self.streams.get(&scope);
        let current_record_count = stream.map_or(0, |stream| stream.records.len());
        let next_ordinal = u64::try_from(current_record_count)
            .ok()
            .and_then(|value| value.checked_add(1))
            .filter(|value| i64::try_from(*value).is_ok())
            .ok_or(ArtifactHistoryError::CounterExhausted)?;
        let ordinal = ArtifactCounter::new(next_ordinal)
            .map_err(|_| ArtifactHistoryError::CounterExhausted)?;
        let predecessor_digest = stream
            .and_then(|stream| stream.records.last())
            .map(|receipt| receipt.receipt_digest.clone());

        let terminal = evaluate()?;
        let receipt = build_receipt(request, ordinal, predecessor_digest, terminal)?;
        let mut next = self.clone();
        let stream = next.streams.entry(scope).or_default();
        if receipt.outcome() == ArtifactCommandOutcome::Denied {
            stream.denial_count = stream
                .denial_count
                .checked_add(1)
                .filter(|value| i64::try_from(*value).is_ok())
                .ok_or(ArtifactHistoryError::CounterExhausted)?;
            stream.denial_tail_digest = Some(receipt.receipt_digest.clone());
        }
        stream.records.push(receipt.clone());
        next.receipts
            .insert(receipt.request.key.clone(), receipt.clone());
        *self = next;

        Ok(ArtifactCommandExecution {
            disposition: ArtifactCommandExecutionDisposition::Recorded,
            receipt,
        })
    }

    /// Builds the current head for one exact object scope.
    ///
    /// # Errors
    ///
    /// Returns a typed counter or hashing error.
    pub fn head(
        &self,
        scope: &ArtifactCommandObjectScope,
    ) -> Result<ArtifactCommandHistoryHead, ArtifactHistoryError> {
        let stream = self.streams.get(scope);
        let high_water_value = stream.map_or(0, |value| value.records.len());
        let high_water_value =
            u64::try_from(high_water_value).map_err(|_| ArtifactHistoryError::CounterExhausted)?;
        let high_water = ArtifactCounter::new(high_water_value)
            .map_err(|_| ArtifactHistoryError::CounterExhausted)?;
        let tail_digest = stream
            .and_then(|value| value.records.last())
            .map(|receipt| receipt.receipt_digest.clone());
        let denial_count_value = stream.map_or(0, |value| value.denial_count);
        let denial_count = ArtifactCounter::new(denial_count_value)
            .map_err(|_| ArtifactHistoryError::CounterExhausted)?;
        let denial_tail_digest = stream.and_then(|value| value.denial_tail_digest.clone());
        let head_digest = canonical_digest(
            ArtifactHashDomain::CommandHead,
            &head_subject(
                scope,
                high_water,
                tail_digest.as_ref(),
                denial_count,
                denial_tail_digest.as_ref(),
            ),
        )?;
        Ok(ArtifactCommandHistoryHead {
            scope: scope.clone(),
            high_water,
            tail_digest,
            denial_count,
            denial_tail_digest,
            head_digest,
        })
    }

    /// Creates a trusted checkpoint from this independently owned history.
    ///
    /// # Errors
    ///
    /// Returns a typed hashing or counter error.
    pub fn checkpoint(
        &self,
        scope: &ArtifactCommandObjectScope,
    ) -> Result<ArtifactHistoryCheckpoint, ArtifactHistoryError> {
        let head = self.head(scope)?;
        ArtifactHistoryCheckpoint::new_trusted(
            head.scope.clone(),
            head.high_water,
            head.tail_digest,
            head.denial_count,
            head.denial_tail_digest,
        )
    }

    /// Exports a strict raw canonical history document for durable storage.
    ///
    /// The returned fields are intentionally replayed as untrusted data.
    ///
    /// # Errors
    ///
    /// Returns a typed hashing or counter error.
    pub fn export_untrusted(
        &self,
        scope: &ArtifactCommandObjectScope,
    ) -> Result<CanonicalValue, ArtifactHistoryError> {
        let head = self.head(scope)?;
        let records = self.streams.get(scope).map_or_else(Vec::new, |stream| {
            stream
                .records
                .iter()
                .map(ArtifactCommandReceipt::raw_value)
                .collect()
        });
        Ok(object([
            string("version", HISTORY_VERSION),
            ("scope".to_owned(), scope.canonical_value()),
            (
                "head".to_owned(),
                object([
                    string("high_water", head.high_water.get().to_string()),
                    (
                        "tail_digest".to_owned(),
                        optional_digest(head.tail_digest.as_ref()),
                    ),
                    string("denial_count", head.denial_count.get().to_string()),
                    (
                        "denial_tail_digest".to_owned(),
                        optional_digest(head.denial_tail_digest.as_ref()),
                    ),
                    string("head_digest", head.head_digest.as_str()),
                ]),
            ),
            ("records".to_owned(), CanonicalValue::Array(records)),
        ]))
    }

    /// Strictly replays untrusted raw history against an independent checkpoint.
    ///
    /// # Errors
    ///
    /// Rejects unknown versions, kinds, or fields; malformed/tampered records;
    /// reordered, truncated, or duplicate chains; scope substitution; head or
    /// denial-tail disagreement; and a cryptographically coherent old prefix.
    pub fn replay_untrusted(
        raw: &CanonicalValue,
        trusted: &ArtifactHistoryCheckpoint,
    ) -> Result<Self, ArtifactHistoryError> {
        Self::replay_untrusted_with_bounds(
            raw,
            trusted,
            MAX_HISTORY_COMMAND_RECORDS,
            MAX_HISTORY_CANONICAL_BYTES,
        )
    }

    /// Replays with an independently selected lower-or-equal safety bound.
    ///
    /// This crate-private entry point lets the future aggregate owner apply its
    /// immutable limit snapshot without weakening the frozen absolute maxima.
    pub(crate) fn replay_untrusted_with_bounds(
        raw: &CanonicalValue,
        trusted: &ArtifactHistoryCheckpoint,
        max_records: usize,
        max_canonical_bytes: usize,
    ) -> Result<Self, ArtifactHistoryError> {
        let (history, scope) =
            Self::restore_untrusted_with_bounds(raw, max_records, max_canonical_bytes)?;
        if &scope != trusted.head.scope() {
            return Err(ArtifactHistoryError::ScopeSubstitution);
        }
        let replayed_checkpoint = history.checkpoint(&scope)?;
        if &replayed_checkpoint != trusted {
            return Err(ArtifactHistoryError::CheckpointMismatch);
        }
        Ok(history)
    }

    /// Rebuilds one complete strict stream from raw data without using a
    /// rollback checkpoint as a source of metadata.
    ///
    /// The aggregate replay layer binds the independently retained checkpoint
    /// after all raw streams have been rebuilt and cross-owner invariants have
    /// been checked.
    pub(crate) fn restore_untrusted_with_bounds(
        raw: &CanonicalValue,
        max_records: usize,
        max_canonical_bytes: usize,
    ) -> Result<(Self, ArtifactCommandObjectScope), ArtifactHistoryError> {
        if max_records == 0 || max_records > MAX_HISTORY_COMMAND_RECORDS {
            return Err(ArtifactHistoryError::ReplayLimit {
                field: "record_bound",
            });
        }
        if max_canonical_bytes == 0 || max_canonical_bytes > MAX_HISTORY_CANONICAL_BYTES {
            return Err(ArtifactHistoryError::ReplayLimit {
                field: "canonical_byte_bound",
            });
        }
        preflight_replay_value(raw, max_canonical_bytes)?;

        let root = StrictObject::new(raw, &["version", "scope", "head", "records"])?;
        if root.string("version")? != HISTORY_VERSION {
            return Err(ArtifactHistoryError::UnknownVersion);
        }
        let scope = parse_scope(root.get("scope")?)?;

        let records_value = root.get("records")?;
        let CanonicalValue::Array(raw_records) = records_value else {
            return Err(ArtifactHistoryError::Malformed { field: "records" });
        };
        if raw_records.len() > max_records {
            return Err(ArtifactHistoryError::ReplayLimit {
                field: "record_count",
            });
        }

        let mut history = Self::new();
        let mut stream = CommandStream::default();
        let mut command_ids = HashSet::new();
        let mut predecessor: Option<ContentDigest> = None;

        for (index, raw_record) in raw_records.iter().enumerate() {
            let receipt = parse_receipt(raw_record)?;
            if receipt.request.key.scope() != &scope {
                return Err(ArtifactHistoryError::ScopeSubstitution);
            }
            if !command_ids.insert(receipt.request.key.command_id.clone()) {
                return Err(ArtifactHistoryError::DuplicateCommand);
            }
            let expected_ordinal = u64::try_from(index)
                .ok()
                .and_then(|value| value.checked_add(1))
                .ok_or(ArtifactHistoryError::CounterExhausted)?;
            if receipt.ordinal.get() != expected_ordinal
                || receipt.predecessor_digest != predecessor
            {
                return Err(ArtifactHistoryError::Reordered);
            }
            predecessor = Some(receipt.receipt_digest.clone());
            if receipt.outcome() == ArtifactCommandOutcome::Denied {
                stream.denial_count = stream
                    .denial_count
                    .checked_add(1)
                    .ok_or(ArtifactHistoryError::CounterExhausted)?;
                stream.denial_tail_digest = Some(receipt.receipt_digest.clone());
            }
            history
                .receipts
                .insert(receipt.request.key.clone(), receipt.clone());
            stream.records.push(receipt);
        }
        history.streams.insert(scope.clone(), stream);

        let raw_head = parse_raw_head(root.get("head")?, scope.clone())?;
        let replayed_head = history.head(&scope)?;
        let record_count =
            u64::try_from(raw_records.len()).map_err(|_| ArtifactHistoryError::CounterExhausted)?;
        if raw_head.high_water.get() > record_count {
            return Err(ArtifactHistoryError::Truncated);
        }
        if raw_head.high_water.get() != record_count
            || raw_head.tail_digest != replayed_head.tail_digest
        {
            return Err(ArtifactHistoryError::HeadMismatch);
        }
        if raw_head.denial_count != replayed_head.denial_count
            || raw_head.denial_tail_digest != replayed_head.denial_tail_digest
        {
            return Err(ArtifactHistoryError::DenialTailMismatch);
        }
        if raw_head.head_digest != replayed_head.head_digest {
            return Err(ArtifactHistoryError::HeadMismatch);
        }

        Ok((history, scope))
    }

    /// Rebuilds one strict stream under the frozen absolute replay bounds.
    pub(crate) fn restore_untrusted(
        raw: &CanonicalValue,
    ) -> Result<(Self, ArtifactCommandObjectScope), ArtifactHistoryError> {
        Self::restore_untrusted_with_bounds(
            raw,
            MAX_HISTORY_COMMAND_RECORDS,
            MAX_HISTORY_CANONICAL_BYTES,
        )
    }

    /// Atomically combines one independently restored strict stream.
    #[allow(
        dead_code,
        reason = "aggregate strict replay wires this helper in the same bounded slice"
    )]
    pub(crate) fn merge_restored(&mut self, restored: Self) -> Result<(), ArtifactHistoryError> {
        let mut next = self.clone();
        for (scope, stream) in restored.streams {
            if next.streams.insert(scope, stream).is_some() {
                return Err(ArtifactHistoryError::DuplicateCommand);
            }
        }
        for (key, receipt) in restored.receipts {
            if next.receipts.insert(key, receipt).is_some() {
                return Err(ArtifactHistoryError::DuplicateCommand);
            }
        }
        *self = next;
        Ok(())
    }
}

/// Fail-closed command-history error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArtifactHistoryError {
    /// Command identifier is empty, non-canonical, or oversized.
    InvalidCommandId,
    /// Denial code is not stable uppercase ASCII.
    InvalidDenialCode,
    /// A denied terminal projection attempted to change the state digest.
    DeniedStateChanged,
    /// Sanitized request source is not a bounded object tree.
    InvalidRequestSource {
        /// Stable offending field.
        field: &'static str,
    },
    /// Sanitized request source exceeded a frozen structural bound.
    RequestSourceLimit {
        /// Stable exceeded bound.
        field: &'static str,
    },
    /// Untrusted raw history exceeded a pre-canonicalization safety bound.
    ReplayLimit {
        /// Stable exceeded bound.
        field: &'static str,
    },
    /// Sanitized request source named a plaintext byte/content field.
    ForbiddenRequestField,
    /// A caller attempted to reuse an exact key with a different full request.
    CommandIdReuse,
    /// A signed-BIGINT-compatible counter was exhausted.
    CounterExhausted,
    /// Canonical framing failed.
    Canonicalization,
    /// A digest field is malformed or zero where evidence is required.
    InvalidDigest {
        /// Stable offending field.
        field: &'static str,
    },
    /// Raw history declared an unsupported version.
    UnknownVersion,
    /// Raw history declared an unsupported command kind.
    UnknownKind,
    /// Raw history included a field outside the frozen schema.
    UnknownField,
    /// Raw history omitted or malformed a frozen field.
    Malformed {
        /// Stable offending field.
        field: &'static str,
    },
    /// A record or receipt digest does not match its full source.
    Tampered,
    /// Ordinal or predecessor order was changed.
    Reordered,
    /// Records were removed without matching the declared high-water.
    Truncated,
    /// An exact command identifier occurs twice in one object chain.
    DuplicateCommand,
    /// Project, algorithm, or object scope was substituted.
    ScopeSubstitution,
    /// Raw high-water, tail, or head digest disagrees with replay.
    HeadMismatch,
    /// Denial count or denial tail was lost or changed.
    DenialTailMismatch,
    /// Replay is internally coherent but older/different than trusted state.
    CheckpointMismatch,
}

impl ArtifactHistoryError {
    /// Stable machine-readable error code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidCommandId => "ARTIFACT_COMMAND_ID_INVALID",
            Self::InvalidDenialCode => "ARTIFACT_DENIAL_CODE_INVALID",
            Self::DeniedStateChanged => "ARTIFACT_DENIAL_STATE_CHANGED",
            Self::InvalidRequestSource { .. } => "ARTIFACT_REQUEST_SOURCE_INVALID",
            Self::RequestSourceLimit { .. } => "ARTIFACT_REQUEST_SOURCE_LIMIT",
            Self::ReplayLimit { .. } => "ARTIFACT_HISTORY_REPLAY_LIMIT",
            Self::ForbiddenRequestField => "ARTIFACT_REQUEST_SOURCE_FORBIDDEN_FIELD",
            Self::CommandIdReuse => "ARTIFACT_COMMAND_ID_REUSE",
            Self::CounterExhausted => "ARTIFACT_COMMAND_COUNTER_EXHAUSTED",
            Self::Canonicalization => "ARTIFACT_HISTORY_CANONICALIZATION_FAILED",
            Self::InvalidDigest { .. } => "ARTIFACT_HISTORY_DIGEST_INVALID",
            Self::UnknownVersion => "ARTIFACT_HISTORY_UNKNOWN_VERSION",
            Self::UnknownKind => "ARTIFACT_HISTORY_UNKNOWN_KIND",
            Self::UnknownField => "ARTIFACT_HISTORY_UNKNOWN_FIELD",
            Self::Malformed { .. } => "ARTIFACT_HISTORY_MALFORMED",
            Self::Tampered => "ARTIFACT_HISTORY_TAMPERED",
            Self::Reordered => "ARTIFACT_HISTORY_REORDERED",
            Self::Truncated => "ARTIFACT_HISTORY_TRUNCATED",
            Self::DuplicateCommand => "ARTIFACT_HISTORY_DUPLICATE_COMMAND",
            Self::ScopeSubstitution => "ARTIFACT_HISTORY_SCOPE_SUBSTITUTION",
            Self::HeadMismatch => "ARTIFACT_HISTORY_HEAD_MISMATCH",
            Self::DenialTailMismatch => "ARTIFACT_HISTORY_DENIAL_TAIL_MISMATCH",
            Self::CheckpointMismatch => "ARTIFACT_HISTORY_CHECKPOINT_MISMATCH",
        }
    }
}

impl fmt::Display for ArtifactHistoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl Error for ArtifactHistoryError {}

fn build_receipt(
    request: ArtifactCommandRequest,
    ordinal: ArtifactCounter,
    predecessor_digest: Option<ContentDigest>,
    terminal: ArtifactCommandTerminalProjection,
) -> Result<ArtifactCommandReceipt, ArtifactHistoryError> {
    let record_subject = record_subject(&request, ordinal, predecessor_digest.as_ref(), &terminal);
    let record_digest = canonical_digest(ArtifactHashDomain::CommandRecord, &record_subject)?;
    let receipt_subject = object([
        string("version", HISTORY_VERSION),
        ("key".to_owned(), request.key.canonical_value()),
        string("ordinal", ordinal.get().to_string()),
        (
            "predecessor_digest".to_owned(),
            optional_digest(predecessor_digest.as_ref()),
        ),
        string("record_digest", record_digest.as_str()),
        string("result_digest", terminal.result_digest.as_str()),
    ]);
    let receipt_digest = canonical_digest(ArtifactHashDomain::CommandReceipt, &receipt_subject)?;
    Ok(ArtifactCommandReceipt {
        request,
        ordinal,
        predecessor_digest,
        terminal,
        record_digest,
        receipt_digest,
    })
}

fn parse_receipt(raw: &CanonicalValue) -> Result<ArtifactCommandReceipt, ArtifactHistoryError> {
    let record = StrictObject::new(
        raw,
        &[
            "version",
            "key",
            "kind",
            "request_source",
            "request_digest",
            "ordinal",
            "predecessor_digest",
            "outcome",
            "denial_code",
            "before_state_digest",
            "after_state_digest",
            "result_digest",
            "record_digest",
            "receipt_digest",
        ],
    )?;
    if record.string("version")? != HISTORY_VERSION {
        return Err(ArtifactHistoryError::UnknownVersion);
    }
    let key = parse_storage_key(record.get("key")?)?;
    let kind = ArtifactCommandKind::parse(record.string("kind")?)?;
    let source = record.get("request_source")?.clone();
    let request = ArtifactCommandRequest::new(key, kind, source)?;
    let stored_request_digest = parse_digest(record.string("request_digest")?, "request_digest")?;
    if request.request_digest != stored_request_digest {
        return Err(ArtifactHistoryError::Tampered);
    }
    let ordinal = parse_counter(record.string("ordinal")?, "ordinal")?;
    if ordinal.get() == 0 {
        return Err(ArtifactHistoryError::Malformed { field: "ordinal" });
    }
    let predecessor_digest =
        parse_optional_digest(record.get("predecessor_digest")?, "predecessor_digest")?;
    let outcome = ArtifactCommandOutcome::parse(record.string("outcome")?)?;
    let denial_code = parse_optional_string(record.get("denial_code")?, "denial_code")?;
    match (outcome, denial_code.as_deref()) {
        (ArtifactCommandOutcome::Applied, None) => {}
        (ArtifactCommandOutcome::Denied, Some(code)) if validate_denial_code(code) => {}
        _ => {
            return Err(ArtifactHistoryError::Malformed {
                field: "denial_code",
            });
        }
    }
    let before_state_digest =
        parse_evidence_digest(record.string("before_state_digest")?, "before_state_digest")?;
    let after_state_digest =
        parse_evidence_digest(record.string("after_state_digest")?, "after_state_digest")?;
    let result_digest = parse_evidence_digest(record.string("result_digest")?, "result_digest")?;
    if outcome == ArtifactCommandOutcome::Denied && before_state_digest != after_state_digest {
        return Err(ArtifactHistoryError::DeniedStateChanged);
    }
    let terminal = ArtifactCommandTerminalProjection {
        outcome,
        denial_code,
        before_state_digest,
        after_state_digest,
        result_digest,
    };
    let rebuilt = build_receipt(request, ordinal, predecessor_digest, terminal)?;
    let stored_record_digest = parse_digest(record.string("record_digest")?, "record_digest")?;
    let stored_receipt_digest = parse_digest(record.string("receipt_digest")?, "receipt_digest")?;
    if rebuilt.record_digest != stored_record_digest
        || rebuilt.receipt_digest != stored_receipt_digest
    {
        return Err(ArtifactHistoryError::Tampered);
    }
    Ok(rebuilt)
}

fn parse_scope(raw: &CanonicalValue) -> Result<ArtifactCommandObjectScope, ArtifactHistoryError> {
    let scope = StrictObject::new(raw, &["project_id", "algorithm", "content_digest"])?;
    if scope.string("algorithm")? != "sha256" {
        return Err(ArtifactHistoryError::ScopeSubstitution);
    }
    let project_id = ProjectId::new(scope.string("project_id")?.to_owned()).map_err(|_| {
        ArtifactHistoryError::Malformed {
            field: "project_id",
        }
    })?;
    let content_digest = parse_digest(scope.string("content_digest")?, "content_digest")?;
    Ok(ArtifactCommandObjectScope::new(project_id, content_digest))
}

fn parse_storage_key(
    raw: &CanonicalValue,
) -> Result<ArtifactCommandStorageKey, ArtifactHistoryError> {
    let key = StrictObject::new(
        raw,
        &["project_id", "algorithm", "content_digest", "command_id"],
    )?;
    if key.string("algorithm")? != "sha256" {
        return Err(ArtifactHistoryError::ScopeSubstitution);
    }
    let project_id = ProjectId::new(key.string("project_id")?.to_owned()).map_err(|_| {
        ArtifactHistoryError::Malformed {
            field: "project_id",
        }
    })?;
    let content_digest = parse_digest(key.string("content_digest")?, "content_digest")?;
    ArtifactCommandStorageKey::new(
        project_id,
        content_digest,
        key.string("command_id")?.to_owned(),
    )
}

fn parse_raw_head(
    raw: &CanonicalValue,
    scope: ArtifactCommandObjectScope,
) -> Result<ArtifactCommandHistoryHead, ArtifactHistoryError> {
    let head = StrictObject::new(
        raw,
        &[
            "high_water",
            "tail_digest",
            "denial_count",
            "denial_tail_digest",
            "head_digest",
        ],
    )?;
    let high_water = parse_counter(head.string("high_water")?, "high_water")?;
    let tail_digest = parse_optional_digest(head.get("tail_digest")?, "tail_digest")?;
    let denial_count = parse_counter(head.string("denial_count")?, "denial_count")?;
    let denial_tail_digest =
        parse_optional_digest(head.get("denial_tail_digest")?, "denial_tail_digest")?;
    validate_head_shape(
        high_water,
        tail_digest.as_ref(),
        denial_count,
        denial_tail_digest.as_ref(),
    )?;
    let head_digest = parse_digest(head.string("head_digest")?, "head_digest")?;
    Ok(ArtifactCommandHistoryHead {
        scope,
        high_water,
        tail_digest,
        denial_count,
        denial_tail_digest,
        head_digest,
    })
}

fn request_subject(
    key: &ArtifactCommandStorageKey,
    kind: ArtifactCommandKind,
    source: &CanonicalValue,
) -> CanonicalValue {
    object([
        string("version", HISTORY_VERSION),
        ("key".to_owned(), key.canonical_value()),
        string("kind", kind.as_str()),
        ("request_source".to_owned(), source.clone()),
    ])
}

fn record_subject(
    request: &ArtifactCommandRequest,
    ordinal: ArtifactCounter,
    predecessor_digest: Option<&ContentDigest>,
    terminal: &ArtifactCommandTerminalProjection,
) -> CanonicalValue {
    object([
        string("version", HISTORY_VERSION),
        ("key".to_owned(), request.key.canonical_value()),
        string("kind", request.kind.as_str()),
        ("request_source".to_owned(), request.source.clone()),
        string("request_digest", request.request_digest.as_str()),
        string("ordinal", ordinal.get().to_string()),
        (
            "predecessor_digest".to_owned(),
            optional_digest(predecessor_digest),
        ),
        string("outcome", terminal.outcome.as_str()),
        (
            "denial_code".to_owned(),
            optional_string(terminal.denial_code.as_deref()),
        ),
        string("before_state_digest", terminal.before_state_digest.as_str()),
        string("after_state_digest", terminal.after_state_digest.as_str()),
        string("result_digest", terminal.result_digest.as_str()),
    ])
}

fn head_subject(
    scope: &ArtifactCommandObjectScope,
    high_water: ArtifactCounter,
    tail_digest: Option<&ContentDigest>,
    denial_count: ArtifactCounter,
    denial_tail_digest: Option<&ContentDigest>,
) -> CanonicalValue {
    object([
        string("version", HISTORY_VERSION),
        ("scope".to_owned(), scope.canonical_value()),
        string("high_water", high_water.get().to_string()),
        ("tail_digest".to_owned(), optional_digest(tail_digest)),
        string("denial_count", denial_count.get().to_string()),
        (
            "denial_tail_digest".to_owned(),
            optional_digest(denial_tail_digest),
        ),
    ])
}

fn checkpoint_subject(head: &ArtifactCommandHistoryHead) -> CanonicalValue {
    object([
        string("version", HISTORY_VERSION),
        ("head".to_owned(), head.canonical_subject()),
        string("head_digest", head.head_digest.as_str()),
    ])
}

fn validate_head_shape(
    high_water: ArtifactCounter,
    tail_digest: Option<&ContentDigest>,
    denial_count: ArtifactCounter,
    denial_tail_digest: Option<&ContentDigest>,
) -> Result<(), ArtifactHistoryError> {
    if (high_water.get() == 0) != tail_digest.is_none()
        || denial_count.get() > high_water.get()
        || (denial_count.get() == 0) != denial_tail_digest.is_none()
        || tail_digest.is_some_and(is_zero_digest)
        || denial_tail_digest.is_some_and(is_zero_digest)
    {
        return Err(ArtifactHistoryError::HeadMismatch);
    }
    Ok(())
}

fn validate_evidence_digests(
    before: &ContentDigest,
    after: &ContentDigest,
    result: &ContentDigest,
) -> Result<(), ArtifactHistoryError> {
    for (field, digest) in [
        ("before_state_digest", before),
        ("after_state_digest", after),
        ("result_digest", result),
    ] {
        if is_zero_digest(digest) {
            return Err(ArtifactHistoryError::InvalidDigest { field });
        }
    }
    Ok(())
}

fn canonical_digest(
    domain: ArtifactHashDomain,
    subject: &CanonicalValue,
) -> Result<ContentDigest, ArtifactHistoryError> {
    let domain = HashDomain::new(domain.schema_id(), HASH_VERSION)
        .map_err(|_| ArtifactHistoryError::Canonicalization)?;
    let digest =
        canonical_sha256(&domain, subject).map_err(|_| ArtifactHistoryError::Canonicalization)?;
    ContentDigest::from_sha256(digest.to_hex()).map_err(|_| ArtifactHistoryError::Canonicalization)
}

fn parse_counter(
    value: &str,
    field: &'static str,
) -> Result<ArtifactCounter, ArtifactHistoryError> {
    let canonical = value == "0"
        || (!value.starts_with('0') && value.bytes().all(|byte| byte.is_ascii_digit()));
    if !canonical {
        return Err(ArtifactHistoryError::Malformed { field });
    }
    let parsed = value
        .parse::<u64>()
        .map_err(|_| ArtifactHistoryError::Malformed { field })?;
    ArtifactCounter::new(parsed).map_err(|_| ArtifactHistoryError::Malformed { field })
}

fn parse_digest(value: &str, field: &'static str) -> Result<ContentDigest, ArtifactHistoryError> {
    ContentDigest::from_sha256(value.to_owned())
        .map_err(|_| ArtifactHistoryError::InvalidDigest { field })
}

fn parse_evidence_digest(
    value: &str,
    field: &'static str,
) -> Result<ContentDigest, ArtifactHistoryError> {
    let digest = parse_digest(value, field)?;
    if is_zero_digest(&digest) {
        return Err(ArtifactHistoryError::InvalidDigest { field });
    }
    Ok(digest)
}

fn parse_optional_digest(
    value: &CanonicalValue,
    field: &'static str,
) -> Result<Option<ContentDigest>, ArtifactHistoryError> {
    match value {
        CanonicalValue::Null => Ok(None),
        CanonicalValue::String(value) => parse_digest(value, field).map(Some),
        _ => Err(ArtifactHistoryError::Malformed { field }),
    }
}

fn parse_optional_string(
    value: &CanonicalValue,
    field: &'static str,
) -> Result<Option<String>, ArtifactHistoryError> {
    match value {
        CanonicalValue::Null => Ok(None),
        CanonicalValue::String(value) => Ok(Some(value.clone())),
        _ => Err(ArtifactHistoryError::Malformed { field }),
    }
}

fn is_zero_digest(digest: &ContentDigest) -> bool {
    digest.as_str().bytes().all(|byte| byte == b'0')
}

fn preflight_replay_value(
    value: &CanonicalValue,
    max_canonical_bytes: usize,
) -> Result<usize, ArtifactHistoryError> {
    let mut stack = vec![(value, 0_usize)];
    let mut node_count = 0_usize;
    let mut canonical_bytes = 0_usize;

    while let Some((value, depth)) = stack.pop() {
        if depth > MAX_HISTORY_DEPTH {
            return Err(ArtifactHistoryError::ReplayLimit { field: "depth" });
        }
        node_count = node_count
            .checked_add(1)
            .ok_or(ArtifactHistoryError::ReplayLimit {
                field: "node_count",
            })?;
        if node_count > MAX_HISTORY_NODES {
            return Err(ArtifactHistoryError::ReplayLimit {
                field: "node_count",
            });
        }

        match value {
            CanonicalValue::Null => {
                add_replay_bytes(&mut canonical_bytes, 4, max_canonical_bytes)?;
            }
            CanonicalValue::Bool(value) => {
                add_replay_bytes(
                    &mut canonical_bytes,
                    if *value { 4 } else { 5 },
                    max_canonical_bytes,
                )?;
            }
            CanonicalValue::String(value) => {
                if value.len() > MAX_HISTORY_STRING_BYTES {
                    return Err(ArtifactHistoryError::ReplayLimit {
                        field: "string_leaf",
                    });
                }
                let encoded = ascii_json_string_bytes(value, "string_leaf")?;
                add_replay_bytes(&mut canonical_bytes, encoded, max_canonical_bytes)?;
            }
            CanonicalValue::Array(values) => {
                if values.len() > MAX_HISTORY_COLLECTION_ENTRIES {
                    return Err(ArtifactHistoryError::ReplayLimit {
                        field: "array_entries",
                    });
                }
                add_replay_bytes(
                    &mut canonical_bytes,
                    2_usize.saturating_add(values.len().saturating_sub(1)),
                    max_canonical_bytes,
                )?;
                let next_depth = depth
                    .checked_add(1)
                    .ok_or(ArtifactHistoryError::ReplayLimit { field: "depth" })?;
                stack.extend(values.iter().rev().map(|value| (value, next_depth)));
            }
            CanonicalValue::Object(fields) => {
                if fields.len() > MAX_HISTORY_OBJECT_FIELDS {
                    return Err(ArtifactHistoryError::ReplayLimit {
                        field: "object_fields",
                    });
                }
                add_replay_bytes(
                    &mut canonical_bytes,
                    2_usize.saturating_add(fields.len().saturating_sub(1)),
                    max_canonical_bytes,
                )?;
                let next_depth = depth
                    .checked_add(1)
                    .ok_or(ArtifactHistoryError::ReplayLimit { field: "depth" })?;
                for (name, value) in fields.iter().rev() {
                    if !(1..=MAX_HISTORY_FIELD_BYTES).contains(&name.len()) {
                        return Err(ArtifactHistoryError::ReplayLimit {
                            field: "field_name",
                        });
                    }
                    let encoded_name = ascii_json_string_bytes(name, "field_name")?;
                    add_replay_bytes(
                        &mut canonical_bytes,
                        encoded_name.saturating_add(1),
                        max_canonical_bytes,
                    )?;
                    stack.push((value, next_depth));
                }
            }
        }
    }
    Ok(canonical_bytes)
}

fn add_replay_bytes(
    total: &mut usize,
    additional: usize,
    maximum: usize,
) -> Result<(), ArtifactHistoryError> {
    *total = total
        .checked_add(additional)
        .ok_or(ArtifactHistoryError::ReplayLimit {
            field: "canonical_bytes",
        })?;
    if *total > maximum {
        return Err(ArtifactHistoryError::ReplayLimit {
            field: "canonical_bytes",
        });
    }
    Ok(())
}

fn ascii_json_string_bytes(
    value: &str,
    field: &'static str,
) -> Result<usize, ArtifactHistoryError> {
    if !value.is_ascii() {
        return Err(ArtifactHistoryError::Malformed { field });
    }
    value
        .bytes()
        .try_fold(2_usize, |length, byte| {
            let encoded = match byte {
                b'"' | b'\\' => 2,
                0..=0x1f => 6,
                _ => 1,
            };
            length.checked_add(encoded)
        })
        .ok_or(ArtifactHistoryError::ReplayLimit {
            field: "canonical_bytes",
        })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RequestMetadataKind {
    Digest,
    Counter,
    Timestamp,
    Identifier,
    Token,
    MediaType,
}

impl RequestMetadataKind {
    const fn error_field(self) -> &'static str {
        match self {
            Self::Digest => "digest",
            Self::Counter => "counter",
            Self::Timestamp => "timestamp",
            Self::Identifier => "identifier",
            Self::Token => "token",
            Self::MediaType => "media_type",
        }
    }
}

fn validate_request_source(value: &CanonicalValue) -> Result<(), ArtifactHistoryError> {
    let CanonicalValue::Object(fields) = value else {
        return Err(ArtifactHistoryError::InvalidRequestSource {
            field: "request_source",
        });
    };
    if fields.len() > MAX_REQUEST_SOURCE_COLLECTION_ENTRIES {
        return Err(ArtifactHistoryError::RequestSourceLimit {
            field: "object_fields",
        });
    }

    let mut names = HashSet::with_capacity(fields.len());
    let mut node_count = 1_usize;
    let mut structural_bytes = 2_usize;
    for (name, value) in fields {
        node_count = node_count
            .checked_add(1)
            .ok_or(ArtifactHistoryError::RequestSourceLimit {
                field: "node_count",
            })?;
        if node_count > MAX_REQUEST_SOURCE_NODES {
            return Err(ArtifactHistoryError::RequestSourceLimit {
                field: "node_count",
            });
        }
        if !valid_request_field_name(name) || !names.insert(name.as_str()) {
            return Err(ArtifactHistoryError::InvalidRequestSource {
                field: "field_name",
            });
        }
        let kind =
            request_metadata_kind(name).ok_or(ArtifactHistoryError::ForbiddenRequestField)?;
        let CanonicalValue::String(value) = value else {
            return Err(ArtifactHistoryError::InvalidRequestSource {
                field: "scalar_value",
            });
        };
        if value.len() > MAX_REQUEST_SOURCE_STRING_BYTES
            || value.contains('\0')
            || !value.is_ascii()
        {
            return Err(ArtifactHistoryError::RequestSourceLimit {
                field: "string_leaf",
            });
        }
        if !validate_request_metadata_value(kind, value) {
            return Err(ArtifactHistoryError::InvalidRequestSource {
                field: kind.error_field(),
            });
        }
        structural_bytes = structural_bytes
            .checked_add(name.len())
            .and_then(|bytes| bytes.checked_add(value.len()))
            .and_then(|bytes| bytes.checked_add(6))
            .ok_or(ArtifactHistoryError::RequestSourceLimit {
                field: "canonical_bytes",
            })?;
        if structural_bytes > MAX_REQUEST_SOURCE_BYTES {
            return Err(ArtifactHistoryError::RequestSourceLimit {
                field: "canonical_bytes",
            });
        }
    }
    Ok(())
}

fn request_metadata_kind(name: &str) -> Option<RequestMetadataKind> {
    if name == "content_digest" || name.ends_with("_digest") {
        Some(RequestMetadataKind::Digest)
    } else if name == "byte_length"
        || name == "attempt"
        || name == "sequence"
        || name == "ordinal"
        || matches!(
            name,
            "active_bytes" | "bundle_bytes" | "history_bytes" | "staging_bytes" | "unique_bytes"
        )
        || name.ends_with("_byte_length")
        || name.ends_with("_count")
        || name.ends_with("_depth")
        || name.ends_with("_revision")
        || name.ends_with("_generation")
        || name.ends_with("_epoch")
        || name.ends_with("_sequence")
        || name.ends_with("_streams")
    {
        Some(RequestMetadataKind::Counter)
    } else if name.ends_with("_at")
        || name.ends_with("_time")
        || name.ends_with("_deadline")
        || name.ends_with("_until")
        || name.ends_with("_not_before")
    {
        Some(RequestMetadataKind::Timestamp)
    } else if name.ends_with("_id") {
        Some(RequestMetadataKind::Identifier)
    } else if name == "media_type" {
        Some(RequestMetadataKind::MediaType)
    } else if name.ends_with("_token")
        || matches!(
            name,
            "action"
                | "algorithm"
                | "availability"
                | "kind"
                | "purpose"
                | "runtime"
                | "schema_version"
                | "producer_version"
                | "adapter_version"
                | "status"
        )
    {
        Some(RequestMetadataKind::Token)
    } else {
        None
    }
}

fn validate_request_metadata_value(kind: RequestMetadataKind, value: &str) -> bool {
    match kind {
        RequestMetadataKind::Digest => ContentDigest::from_sha256(value.to_owned()).is_ok(),
        RequestMetadataKind::Counter => {
            let canonical = value == "0"
                || (!value.starts_with('0') && value.bytes().all(|byte| byte.is_ascii_digit()));
            canonical
                && value
                    .parse::<u64>()
                    .ok()
                    .and_then(|value| ArtifactCounter::new(value).ok())
                    .is_some()
        }
        RequestMetadataKind::Timestamp => OffsetDateTime::parse(value, &Rfc3339).is_ok(),
        RequestMetadataKind::Identifier | RequestMetadataKind::Token => {
            validate_safe_metadata_token(value)
        }
        RequestMetadataKind::MediaType => {
            (1..=MAX_REQUEST_SOURCE_STRING_BYTES).contains(&value.len())
                && value.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric()
                        || matches!(byte, b'/' | b'+' | b'.' | b'_' | b':' | b'-')
                })
        }
    }
}

fn valid_request_field_name(value: &str) -> bool {
    (1..=MAX_REQUEST_SOURCE_FIELD_BYTES).contains(&value.len())
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'.' | b'-')
        })
}

fn validate_safe_metadata_token(value: &str) -> bool {
    (1..=MAX_REQUEST_SOURCE_STRING_BYTES).contains(&value.len())
        && value.trim() == value
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
}

fn validate_identifier(value: &str, max_bytes: usize) -> bool {
    (1..=max_bytes).contains(&value.len())
        && value.trim() == value
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
}

fn validate_denial_code(value: &str) -> bool {
    (1..=MAX_DENIAL_CODE_BYTES).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

fn object<const N: usize>(fields: [(String, CanonicalValue); N]) -> CanonicalValue {
    CanonicalValue::Object(Vec::from(fields))
}

fn string(name: &str, value: impl Into<String>) -> (String, CanonicalValue) {
    (name.to_owned(), CanonicalValue::String(value.into()))
}

fn optional_string(value: Option<&str>) -> CanonicalValue {
    value.map_or(CanonicalValue::Null, |value| {
        CanonicalValue::String(value.to_owned())
    })
}

fn optional_digest(value: Option<&ContentDigest>) -> CanonicalValue {
    optional_string(value.map(ContentDigest::as_str))
}

struct StrictObject<'a> {
    fields: HashMap<&'a str, &'a CanonicalValue>,
}

impl<'a> StrictObject<'a> {
    fn new(value: &'a CanonicalValue, expected: &[&str]) -> Result<Self, ArtifactHistoryError> {
        let CanonicalValue::Object(entries) = value else {
            return Err(ArtifactHistoryError::Malformed { field: "object" });
        };
        let mut fields = HashMap::with_capacity(entries.len());
        for (name, value) in entries {
            if !expected.contains(&name.as_str()) {
                return Err(ArtifactHistoryError::UnknownField);
            }
            if fields.insert(name.as_str(), value).is_some() {
                return Err(ArtifactHistoryError::Malformed { field: "object" });
            }
        }
        if fields.len() != expected.len() || expected.iter().any(|name| !fields.contains_key(name))
        {
            return Err(ArtifactHistoryError::Malformed { field: "object" });
        }
        Ok(Self { fields })
    }

    fn get(&self, name: &'static str) -> Result<&'a CanonicalValue, ArtifactHistoryError> {
        self.fields
            .get(name)
            .copied()
            .ok_or(ArtifactHistoryError::Malformed { field: name })
    }

    fn string(&self, name: &'static str) -> Result<&'a str, ArtifactHistoryError> {
        match self.get(name)? {
            CanonicalValue::String(value) => Ok(value),
            _ => Err(ArtifactHistoryError::Malformed { field: name }),
        }
    }
}
