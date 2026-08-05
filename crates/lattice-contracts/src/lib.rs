//! Versioned, I/O-free shared contracts for LATTICE ports and adapters.

mod delivery;
mod graph_memory;

pub use delivery::*;
pub use graph_memory::*;

use std::error::Error;
use std::fmt;

/// The only contract version supported by this initial boundary.
pub const CONTRACT_VERSION: u16 = 1;

/// The only Project Registry producer identity accepted by shared receipts.
pub const PROJECT_AUTHORITY_PRODUCER_ID: &str = "lattice-project-registry";

/// The Project Registry authority-receipt semantic version supported here.
pub const PROJECT_AUTHORITY_PRODUCER_VERSION: &str = "1.0";

/// The only Task Ledger producer identity accepted by shared receipts.
pub const TASK_LEDGER_PRODUCER_ID: &str = "lattice-task-ledger";

/// The Task Ledger semantic version supported by shared receipts.
pub const TASK_LEDGER_PRODUCER_VERSION: &str = "2.0";

/// The only Writer Lease producer identity accepted by shared receipts.
pub const WRITER_LEASE_PRODUCER_ID: &str = "lattice-writer-lease";

/// The Writer Lease authority-receipt semantic version supported here.
pub const WRITER_LEASE_PRODUCER_VERSION: &str = "1.0";

/// The only Approval Verifier producer identity accepted by shared receipts.
pub const APPROVAL_VERIFIER_PRODUCER_ID: &str = "lattice-approval-verifier";

/// The Approval Verifier authority-receipt semantic version supported here.
pub const APPROVAL_VERIFIER_PRODUCER_VERSION: &str = "1.0";

/// The only Artifact Store producer identity accepted by store receipts.
pub const ARTIFACT_STORE_PRODUCER_ID: &str = "lattice-artifact-store";

/// The Artifact Store semantic version supported by shared receipts.
pub const ARTIFACT_STORE_PRODUCER_VERSION: &str = "1.0";

/// The only producer identity accepted for typed artifact read-closure evidence.
pub const ARTIFACT_READ_CLOSURE_PRODUCER_ID: &str = "lattice-artifact-read-closure-verifier";

/// The read-closure evidence semantic version supported by shared receipts.
pub const ARTIFACT_READ_CLOSURE_PRODUCER_VERSION: &str = "1.0";

/// Stable local gateway protocol identity.
pub const GATEWAY_PROTOCOL_ID: &str = "lattice-gateway-ipc";
/// Initial local gateway protocol version.
pub const GATEWAY_PROTOCOL_VERSION: u16 = 1;
/// Task Spec schema accepted by the initial gateway submission envelope.
pub const GATEWAY_TASK_SPEC_SCHEMA_ID: &str = "lattice.task-spec";
/// Task Spec schema version accepted by the initial gateway submission envelope.
pub const GATEWAY_TASK_SPEC_SCHEMA_VERSION: &str = "2.1";
/// Maximum canonical Task Spec document retained by a gateway request.
pub const GATEWAY_TASK_SPEC_MAX_BYTES: usize = 786_432;
/// Maximum bounded gateway identifier length.
pub const GATEWAY_IDENTIFIER_MAX_BYTES: usize = 256;
/// Maximum opaque status cursor length.
pub const GATEWAY_CURSOR_MAX_BYTES: usize = 512;
/// Maximum bounded project-status page size.
pub const GATEWAY_STATUS_PAGE_MAX_ITEMS: u16 = 100;

/// Preserved fake-only physical Store transaction contract version.
pub const STORE_CONTRACT_VERSION_V1: u16 = 1;
/// Current physical Store transaction contract version with live durability.
pub const STORE_CONTRACT_VERSION: u16 = 2;
/// Fixed producer identity for typed Store terminal receipts.
pub const STORE_PRODUCER_ID: &str = "lattice-postgres-store";
/// Fixed semantic producer version for typed Store terminal receipts.
pub const STORE_PRODUCER_VERSION: &str = "1.0";
/// Maximum canonical ASCII length of Store transaction/daemon identifiers.
pub const STORE_IDENTIFIER_MAX_BYTES: usize = 128;

/// Failure to construct a valid shared contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContractError {
    /// A required opaque identifier contains no non-whitespace characters.
    EmptyIdentifier {
        /// Stable field name for diagnostic and test evidence.
        field: &'static str,
    },
    /// A SHA-256 reference is not exactly 64 lowercase hexadecimal characters.
    MalformedSha256,
    /// A canonical project identifier violates the shared ASCII contract.
    InvalidProjectId,
    /// A Git reference is not a fully qualified local branch.
    InvalidGitReference,
    /// A Registry authority receipt uses revision zero.
    ZeroRevision,
    /// A Registry authority receipt names an unknown semantic producer.
    UnsupportedProjectAuthorityProducer,
    /// A Registry authority receipt names an unsupported producer version.
    UnsupportedProjectAuthorityProducerVersion,
    /// A Task Ledger head or receipt names an unknown semantic producer.
    UnsupportedTaskLedgerProducer,
    /// A Task Ledger head or receipt names an unsupported producer version.
    UnsupportedTaskLedgerProducerVersion,
    /// A task revision is not a canonical positive unsigned integer string.
    InvalidTaskRevision,
    /// An accounting currency is not exactly three uppercase ASCII letters.
    InvalidAccountingCurrency,
    /// A Task Ledger identifier violates its exact bounded ASCII contract.
    InvalidTaskLedgerIdentifier {
        /// Stable field name for diagnostic and test evidence.
        field: &'static str,
    },
    /// A full Task Ledger stream head violates zero/non-zero invariants.
    InvalidTaskLedgerHead,
    /// Resource counters, request values, or canonical costs are malformed.
    InvalidResourceUsage,
    /// A resource observation revision is zero.
    ZeroObservationRevision,
    /// Receipt runtime disagrees with the represented stream runtime.
    TaskLedgerRuntimeMismatch,
    /// A Writer Lease receipt names an unknown semantic producer.
    UnsupportedWriterLeaseProducer,
    /// A Writer Lease receipt names an unsupported producer version.
    UnsupportedWriterLeaseProducerVersion,
    /// A Writer Lease identifier violates its exact bounded ASCII contract.
    InvalidWriterLeaseIdentifier {
        /// Stable field name for diagnostic and test evidence.
        field: &'static str,
    },
    /// A typed Writer Lease identity field violates a semantic invariant.
    InvalidWriterLeaseIdentity {
        /// Stable field name for diagnostic and test evidence.
        field: &'static str,
    },
    /// A persisted PostgreSQL-compatible positive signed BIGINT is out of range.
    InvalidPositiveSignedBigInt {
        /// Stable field name for diagnostic and test evidence.
        field: &'static str,
    },
    /// A Writer Lease authority receipt field is structurally invalid.
    InvalidWriterLeaseReceipt {
        /// Stable field name for diagnostic and test evidence.
        field: &'static str,
    },
    /// An Approval Verifier receipt names an unknown semantic producer.
    UnsupportedApprovalVerifierProducer,
    /// An Approval Verifier receipt names an unsupported producer version.
    UnsupportedApprovalVerifierProducerVersion,
    /// A typed approval subject field violates its structural contract.
    InvalidApprovalSubject {
        /// Stable field name for diagnostic and test evidence.
        field: &'static str,
    },
    /// Approval requester, approver, authority, trust, or lane identity is invalid.
    InvalidApprovalIdentity {
        /// Stable field name for diagnostic and test evidence.
        field: &'static str,
    },
    /// An Approval Verifier authority receipt/head field is invalid.
    InvalidApprovalReceipt {
        /// Stable field name for diagnostic and test evidence.
        field: &'static str,
    },
    /// An Artifact Store receipt names an unknown semantic producer.
    UnsupportedArtifactStoreProducer,
    /// An Artifact Store receipt names an unsupported producer version.
    UnsupportedArtifactStoreProducerVersion,
    /// A persisted non-negative signed-BIGINT-compatible value is out of range.
    InvalidNonNegativeSignedBigInt {
        /// Stable field name for diagnostic and test evidence.
        field: &'static str,
    },
    /// An immutable artifact value violates its structural contract.
    InvalidArtifactValue {
        /// Stable field name for diagnostic and test evidence.
        field: &'static str,
    },
    /// An artifact operation authority violates its typed scope.
    InvalidArtifactAuthority {
        /// Stable field name for diagnostic and test evidence.
        field: &'static str,
    },
    /// A receipt and independently queried artifact authority head disagree.
    ArtifactAuthorityHeadMismatch {
        /// Stable authority family for diagnostic and test evidence.
        field: &'static str,
    },
    /// An Artifact Store receipt or current head is structurally invalid.
    InvalidArtifactReceipt {
        /// Stable field name for diagnostic and test evidence.
        field: &'static str,
    },
    /// A bounded neutral gateway value violates its structural contract.
    InvalidGatewayValue {
        /// Stable field name for diagnostic and test evidence.
        field: &'static str,
    },
    /// The local gateway protocol version is not supported by this build.
    UnsupportedGatewayProtocolVersion,
    /// A typed gateway reply body does not correspond to its bound action.
    GatewayReplyActionMismatch,
    /// A neutral Store value violates its structural contract.
    InvalidStoreValue {
        /// Stable field name for diagnostic and test evidence.
        field: &'static str,
    },
    /// The typed physical Store contract version is unsupported.
    UnsupportedStoreContractVersion,
    /// Store request and expected physical-head scopes disagree.
    StoreScopeMismatch,
    /// Store authority, physical head, request, or receipt runtimes disagree.
    StoreRuntimeMismatch,
    /// A Store terminal receipt is inconsistent with its request/disposition.
    StoreReceiptMismatch,
    /// The caller requested a contract version this build does not understand.
    UnsupportedVersion {
        /// Version supported by this build.
        supported: u16,
        /// Version supplied by the caller.
        found: u16,
    },
}

impl fmt::Display for ContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyIdentifier { field } => {
                write!(formatter, "{field} must not be empty")
            }
            Self::MalformedSha256 => {
                formatter.write_str("SHA-256 reference must be 64 lowercase hex characters")
            }
            Self::InvalidProjectId => formatter.write_str(
                "project_id must be 2-64 lowercase ASCII letters, digits, dots, underscores, or hyphens and start with a letter or digit",
            ),
            Self::InvalidGitReference => {
                formatter.write_str("Git reference must be a valid fully qualified refs/heads/* local branch")
            }
            Self::ZeroRevision => {
                formatter.write_str("Registry authority revision must be greater than zero")
            }
            Self::UnsupportedProjectAuthorityProducer => formatter.write_str(
                "unsupported Project Registry authority producer identity",
            ),
            Self::UnsupportedProjectAuthorityProducerVersion => formatter.write_str(
                "unsupported Project Registry authority producer version",
            ),
            Self::UnsupportedTaskLedgerProducer => {
                formatter.write_str("unsupported Task Ledger producer identity")
            }
            Self::UnsupportedTaskLedgerProducerVersion => {
                formatter.write_str("unsupported Task Ledger producer version")
            }
            Self::InvalidTaskRevision => formatter.write_str(
                "task_revision must be a canonical positive unsigned integer string",
            ),
            Self::InvalidAccountingCurrency => {
                formatter.write_str("accounting_currency must be three uppercase ASCII letters")
            }
            Self::InvalidTaskLedgerIdentifier { field } => {
                write!(formatter, "invalid Task Ledger {field}")
            }
            Self::InvalidTaskLedgerHead => {
                formatter.write_str("invalid Task Ledger stream head")
            }
            Self::InvalidResourceUsage => {
                formatter.write_str("invalid Task Ledger resource usage")
            }
            Self::ZeroObservationRevision => {
                formatter.write_str("resource observation revision must be greater than zero")
            }
            Self::TaskLedgerRuntimeMismatch => {
                formatter.write_str("Task Ledger receipt and stream runtime differ")
            }
            Self::UnsupportedWriterLeaseProducer => {
                formatter.write_str("unsupported Writer Lease producer identity")
            }
            Self::UnsupportedWriterLeaseProducerVersion => {
                formatter.write_str("unsupported Writer Lease producer version")
            }
            Self::InvalidWriterLeaseIdentifier { field } => {
                write!(formatter, "invalid Writer Lease {field}")
            }
            Self::InvalidWriterLeaseIdentity { field } => {
                write!(formatter, "invalid Writer Lease identity {field}")
            }
            Self::InvalidPositiveSignedBigInt { field } => {
                write!(
                    formatter,
                    "{field} must be between 1 and signed BIGINT maximum"
                )
            }
            Self::InvalidWriterLeaseReceipt { field } => {
                write!(formatter, "invalid Writer Lease receipt {field}")
            }
            Self::UnsupportedApprovalVerifierProducer => {
                formatter.write_str("unsupported Approval Verifier producer identity")
            }
            Self::UnsupportedApprovalVerifierProducerVersion => {
                formatter.write_str("unsupported Approval Verifier producer version")
            }
            Self::InvalidApprovalSubject { field } => {
                write!(formatter, "invalid approval subject {field}")
            }
            Self::InvalidApprovalIdentity { field } => {
                write!(formatter, "invalid approval identity {field}")
            }
            Self::InvalidApprovalReceipt { field } => {
                write!(formatter, "invalid Approval Verifier receipt {field}")
            }
            Self::UnsupportedArtifactStoreProducer => {
                formatter.write_str("unsupported Artifact Store producer identity")
            }
            Self::UnsupportedArtifactStoreProducerVersion => {
                formatter.write_str("unsupported Artifact Store producer version")
            }
            Self::InvalidNonNegativeSignedBigInt { field } => {
                write!(
                    formatter,
                    "{field} must be between zero and signed BIGINT maximum"
                )
            }
            Self::InvalidArtifactValue { field } => {
                write!(formatter, "invalid artifact value {field}")
            }
            Self::InvalidArtifactAuthority { field } => {
                write!(formatter, "invalid artifact authority {field}")
            }
            Self::ArtifactAuthorityHeadMismatch { field } => {
                write!(formatter, "artifact authority head mismatch for {field}")
            }
            Self::InvalidArtifactReceipt { field } => {
                write!(formatter, "invalid Artifact Store receipt {field}")
            }
            Self::InvalidGatewayValue { field } => {
                write!(formatter, "invalid gateway value {field}")
            }
            Self::UnsupportedGatewayProtocolVersion => {
                formatter.write_str("unsupported gateway protocol version")
            }
            Self::GatewayReplyActionMismatch => {
                formatter.write_str("gateway reply body does not match action")
            }
            Self::InvalidStoreValue { field } => {
                write!(formatter, "invalid Store value {field}")
            }
            Self::UnsupportedStoreContractVersion => {
                formatter.write_str("unsupported Store contract version")
            }
            Self::StoreScopeMismatch => {
                formatter.write_str("Store request and physical-head scope mismatch")
            }
            Self::StoreRuntimeMismatch => {
                formatter.write_str("Store authority, head, request, or receipt runtime mismatch")
            }
            Self::StoreReceiptMismatch => {
                formatter.write_str("Store receipt does not match its request or disposition")
            }
            Self::UnsupportedVersion { supported, found } => {
                write!(
                    formatter,
                    "unsupported contract version {found}; supported version is {supported}"
                )
            }
        }
    }
}

impl Error for ContractError {}

/// A validated shared-contract version.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ContractVersion(u16);

impl ContractVersion {
    /// Validates and returns the current contract version.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::UnsupportedVersion`] for every version other
    /// than [`CONTRACT_VERSION`].
    pub fn new(found: u16) -> Result<Self, ContractError> {
        if found == CONTRACT_VERSION {
            Ok(Self(found))
        } else {
            Err(ContractError::UnsupportedVersion {
                supported: CONTRACT_VERSION,
                found,
            })
        }
    }

    /// Returns the numeric version.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

macro_rules! identifier {
    ($name:ident, $field:literal) => {
        #[doc = concat!("Opaque, non-empty `", $field, "` value.")]
        #[derive(Clone, Debug, Eq, Hash, PartialEq)]
        pub struct $name(String);

        impl $name {
            #[doc = concat!("Validates and owns a `", $field, "` value.")]
            ///
            /// # Errors
            ///
            #[doc = concat!("Returns [`ContractError::EmptyIdentifier`] when `", $field, "` contains no non-whitespace characters.")]
            pub fn new(value: impl Into<String>) -> Result<Self, ContractError> {
                let value = value.into();
                if value.trim().is_empty() {
                    Err(ContractError::EmptyIdentifier { field: $field })
                } else {
                    Ok(Self(value))
                }
            }

            #[doc = concat!("Returns the original `", $field, "` text.")]
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

identifier!(RequestId, "request_id");
identifier!(TaskId, "task_id");
identifier!(AttemptId, "attempt_id");
identifier!(ProjectSnapshotId, "project_snapshot_id");

/// One canonical project identifier shared by Task Domain, Registry, and Policy.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProjectId(String);

impl ProjectId {
    /// Validates an already canonical project identifier without normalizing it.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::InvalidProjectId`] unless the value is 2-64
    /// lowercase ASCII letters, digits, dots, underscores, or hyphens and
    /// starts with a letter or digit.
    pub fn new(value: impl Into<String>) -> Result<Self, ContractError> {
        let value = value.into();
        let valid = (2..=64).contains(&value.len())
            && value
                .bytes()
                .next()
                .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
            && value.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'.' | b'_' | b'-')
            });
        if valid {
            Ok(Self(value))
        } else {
            Err(ContractError::InvalidProjectId)
        }
    }

    /// Returns the canonical project identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A validated lowercase SHA-256 reference.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ContentDigest(String);

impl ContentDigest {
    /// Validates a lowercase hexadecimal SHA-256 reference.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::MalformedSha256`] unless the input is exactly
    /// 64 lowercase hexadecimal characters.
    pub fn from_sha256(value: impl Into<String>) -> Result<Self, ContractError> {
        let value = value.into();
        let valid = value.len() == 64
            && value
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));
        if valid {
            Ok(Self(value))
        } else {
            Err(ContractError::MalformedSha256)
        }
    }

    /// Returns the lowercase hexadecimal digest.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns the stable digest algorithm identifier.
    #[must_use]
    pub const fn algorithm(&self) -> &'static str {
        "sha256"
    }
}

/// Registry-owned project class represented at shared boundaries.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ProjectClass {
    /// A normal registered user project.
    UserProject,
    /// The protected LATTICE system project.
    LatticeSystem,
}

impl ProjectClass {
    /// Returns the stable receipt-facing value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UserProject => "USER_PROJECT",
            Self::LatticeSystem => "LATTICE_SYSTEM",
        }
    }
}

/// Closed Registry lifecycle represented by an authority receipt.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ProjectLifecycle {
    /// The exact current identity may be used by an authorized task.
    Active,
    /// Registry suspension blocks project authority.
    Suspended,
    /// A moved, replaced, or otherwise drifted identity requires reconciliation.
    ReconciliationRequired,
}

impl ProjectLifecycle {
    /// Returns the stable receipt-facing value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::Suspended => "SUSPENDED",
            Self::ReconciliationRequired => "RECONCILIATION_REQUIRED",
        }
    }
}

/// Owner-resolved physical identity of one fully qualified local Git ref.
///
/// The digest identifies the repository backend's physical ref identity, not
/// merely the reference text or pointed-to commit.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct GitRefIdentity {
    reference: String,
    storage_identity_digest: ContentDigest,
}

impl GitRefIdentity {
    /// Constructs one validated fully qualified local-branch identity.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::InvalidGitReference`] for pseudo-refs,
    /// revision DWIM, tags, remotes, shorthand, nested namespaces, or invalid
    /// Git ref forms.
    pub fn new(
        reference: impl Into<String>,
        storage_identity_digest: ContentDigest,
    ) -> Result<Self, ContractError> {
        let reference = reference.into();
        if canonical_local_branch(&reference).is_none() {
            return Err(ContractError::InvalidGitReference);
        }
        Ok(Self {
            reference,
            storage_identity_digest,
        })
    }

    /// Returns the case-preserving fully qualified local branch reference.
    #[must_use]
    pub fn reference(&self) -> &str {
        &self.reference
    }

    /// Returns the owner-supplied physical storage identity digest.
    #[must_use]
    pub const fn storage_identity_digest(&self) -> &ContentDigest {
        &self.storage_identity_digest
    }
}

fn valid_git_ref(value: &str) -> bool {
    if value.is_empty()
        || value.trim() != value
        || value == "@"
        || value.starts_with('-')
        || value.starts_with('/')
        || value.ends_with('/')
        || value.ends_with('.')
        || value.contains("..")
        || value.contains("//")
        || value.contains("@{")
        || value.contains('\\')
        || value.bytes().any(|byte| {
            byte.is_ascii_control()
                || matches!(byte, b' ' | b'~' | b'^' | b':' | b'?' | b'*' | b'[')
        })
    {
        return false;
    }
    value.split('/').all(|component| {
        !component.is_empty()
            && !component.starts_with('.')
            && !component
                .get(component.len().saturating_sub(5)..)
                .is_some_and(|suffix| suffix.eq_ignore_ascii_case(".lock"))
    })
}

fn canonical_local_branch(value: &str) -> Option<&str> {
    if !valid_git_ref(value) {
        return None;
    }
    let branch = value.strip_prefix("refs/heads/")?;
    if branch.is_empty()
        || looks_like_git_pseudoref(branch)
        || ["refs/", "heads/", "tags/", "remotes/"]
            .iter()
            .any(|prefix| branch.starts_with(prefix))
        || !valid_git_ref(branch)
    {
        return None;
    }
    Some(branch)
}

fn looks_like_git_pseudoref(value: &str) -> bool {
    !value.contains('/')
        && matches!(
            value,
            "HEAD"
                | "AUTO_MERGE"
                | "BISECT_EXPECTED_REV"
                | "BISECT_HEAD"
                | "BISECT_START"
                | "CHERRY_PICK_HEAD"
                | "FETCH_HEAD"
                | "MERGE_HEAD"
                | "ORIG_HEAD"
                | "REBASE_HEAD"
                | "REVERT_HEAD"
        )
}

/// One immutable request identity shared across adapter boundaries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Invocation {
    version: ContractVersion,
    request_id: RequestId,
    task_id: TaskId,
    attempt_id: AttemptId,
    project_snapshot_id: ProjectSnapshotId,
    subject_digest: ContentDigest,
}

impl Invocation {
    /// Constructs an invocation after validating the contract version.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::UnsupportedVersion`] for every version other
    /// than [`CONTRACT_VERSION`].
    pub fn new(
        version: u16,
        request_id: RequestId,
        task_id: TaskId,
        attempt_id: AttemptId,
        project_snapshot_id: ProjectSnapshotId,
        subject_digest: ContentDigest,
    ) -> Result<Self, ContractError> {
        Ok(Self {
            version: ContractVersion::new(version)?,
            request_id,
            task_id,
            attempt_id,
            project_snapshot_id,
            subject_digest,
        })
    }

    /// Returns the numeric contract version.
    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version.get()
    }

    /// Returns the request identifier.
    #[must_use]
    pub const fn request_id(&self) -> &RequestId {
        &self.request_id
    }

    /// Returns the task identifier.
    #[must_use]
    pub const fn task_id(&self) -> &TaskId {
        &self.task_id
    }

    /// Returns the task-attempt identifier.
    #[must_use]
    pub const fn attempt_id(&self) -> &AttemptId {
        &self.attempt_id
    }

    /// Returns the immutable project snapshot identifier.
    #[must_use]
    pub const fn project_snapshot_id(&self) -> &ProjectSnapshotId {
        &self.project_snapshot_id
    }

    /// Returns the digest of the immutable request subject.
    #[must_use]
    pub const fn subject_digest(&self) -> &ContentDigest {
        &self.subject_digest
    }
}

/// Stable component identity for normalized boundary evidence.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Component {
    OpenClaw,
    PostgreSql,
    Codex,
    Graphify,
    Hermes,
}

/// Authority/trust boundary represented by normalized evidence.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Boundary {
    Gateway,
    ControlStore,
    ProductCodeWriter,
    DerivedReadOnlyEvidence,
    UntrustedCandidate,
}

/// Whether evidence came from a test double or a live runtime.
///
/// This diagnostic marker never proves capability, durability, or authority by
/// itself; those claims require separately bound evidence.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RuntimeKind {
    Fake,
    Live,
}

impl RuntimeKind {
    /// Returns true only for a live runtime marker.
    #[must_use]
    pub const fn is_live(self) -> bool {
        matches!(self, Self::Live)
    }
}

/// Exact immutable task/project scope shared by approval subjects.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct SubjectBinding {
    project_id: ProjectId,
    project_snapshot_id: ProjectSnapshotId,
    task_id: TaskId,
    task_revision: String,
    task_spec_digest: ContentDigest,
}

impl SubjectBinding {
    /// Constructs one exact approval binding without hidden normalization.
    ///
    /// # Errors
    ///
    /// Rejects a non-canonical task revision or the all-zero digest sentinel.
    pub fn new(
        project_id: ProjectId,
        project_snapshot_id: ProjectSnapshotId,
        task_id: TaskId,
        task_revision: impl Into<String>,
        task_spec_digest: ContentDigest,
    ) -> Result<Self, ContractError> {
        let task_revision = task_revision.into();
        if !canonical_positive_u64(&task_revision) {
            return Err(ContractError::InvalidApprovalSubject {
                field: "task_revision",
            });
        }
        if is_zero_digest(&task_spec_digest) {
            return Err(ContractError::InvalidApprovalSubject {
                field: "task_spec_digest",
            });
        }
        Ok(Self {
            project_id,
            project_snapshot_id,
            task_id,
            task_revision,
            task_spec_digest,
        })
    }

    /// Returns the canonical project identifier.
    #[must_use]
    pub const fn project_id(&self) -> &ProjectId {
        &self.project_id
    }

    /// Returns the immutable project snapshot identifier.
    #[must_use]
    pub const fn project_snapshot_id(&self) -> &ProjectSnapshotId {
        &self.project_snapshot_id
    }

    /// Returns the immutable task identifier.
    #[must_use]
    pub const fn task_id(&self) -> &TaskId {
        &self.task_id
    }

    /// Returns the canonical positive task revision.
    #[must_use]
    pub fn task_revision(&self) -> &str {
        &self.task_revision
    }

    /// Returns the immutable Task Spec digest.
    #[must_use]
    pub const fn task_spec_digest(&self) -> &ContentDigest {
        &self.task_spec_digest
    }
}

/// Closed approval subject family.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ApprovalKind {
    /// Task execution, including an optional exact external-cost quote.
    Execution,
    /// One exact Git integration.
    Merge,
    /// One exact memory-preference candidate.
    Preference,
    /// One exact protected operation.
    ProtectedChange,
    /// One exact guarded release activation.
    ProtectedRelease,
}

impl ApprovalKind {
    /// Complete closed set.
    pub const ALL: [Self; 5] = [
        Self::Execution,
        Self::Merge,
        Self::Preference,
        Self::ProtectedChange,
        Self::ProtectedRelease,
    ];

    /// Returns the stable receipt-facing value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Execution => "EXECUTION",
            Self::Merge => "MERGE",
            Self::Preference => "PREFERENCE",
            Self::ProtectedChange => "PROTECTED_CHANGE",
            Self::ProtectedRelease => "PROTECTED_RELEASE",
        }
    }
}

/// One immutable external-cost quote bound into an execution approval.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ExternalCostSubject {
    amount: String,
    currency: String,
    provider_id: String,
    quote_digest: ContentDigest,
    pricing_digest: ContentDigest,
}

impl ExternalCostSubject {
    /// Constructs one structurally valid external-cost subject.
    ///
    /// # Errors
    ///
    /// Rejects a malformed canonical decimal/currency/provider or zero digest.
    pub fn new(
        amount: impl Into<String>,
        currency: impl Into<String>,
        provider_id: impl Into<String>,
        quote_digest: ContentDigest,
        pricing_digest: ContentDigest,
    ) -> Result<Self, ContractError> {
        let amount = amount.into();
        let currency = currency.into();
        let provider_id = provider_id.into();
        if !canonical_decimal(&amount) {
            return Err(ContractError::InvalidApprovalSubject { field: "amount" });
        }
        if !valid_accounting_currency(&currency) {
            return Err(ContractError::InvalidApprovalSubject { field: "currency" });
        }
        if !valid_approval_identifier(&provider_id) {
            return Err(ContractError::InvalidApprovalSubject {
                field: "provider_id",
            });
        }
        if is_zero_digest(&quote_digest) {
            return Err(ContractError::InvalidApprovalSubject {
                field: "quote_digest",
            });
        }
        if is_zero_digest(&pricing_digest) {
            return Err(ContractError::InvalidApprovalSubject {
                field: "pricing_digest",
            });
        }
        Ok(Self {
            amount,
            currency,
            provider_id,
            quote_digest,
            pricing_digest,
        })
    }

    /// Returns the canonical quote amount.
    #[must_use]
    pub fn amount(&self) -> &str {
        &self.amount
    }

    /// Returns the exact uppercase accounting currency.
    #[must_use]
    pub fn currency(&self) -> &str {
        &self.currency
    }

    /// Returns the provider identity.
    #[must_use]
    pub fn provider_id(&self) -> &str {
        &self.provider_id
    }

    /// Returns the exact quote digest.
    #[must_use]
    pub const fn quote_digest(&self) -> &ContentDigest {
        &self.quote_digest
    }

    /// Returns the exact pricing digest.
    #[must_use]
    pub const fn pricing_digest(&self) -> &ContentDigest {
        &self.pricing_digest
    }
}

/// Git integration target carried by a merge approval subject.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum MergeTarget {
    /// Invalid/unbound input retained for fail-closed Policy evaluation.
    Unbound,
    /// One non-primary fully qualified local branch.
    FeatureBranch(String),
    /// One primary fully qualified local branch.
    PrimaryBranch(String),
}

impl MergeTarget {
    /// Returns the supplied Git reference without trusting its classification.
    #[must_use]
    pub fn reference(&self) -> Option<&str> {
        match self {
            Self::Unbound => None,
            Self::FeatureBranch(reference) | Self::PrimaryBranch(reference) => Some(reference),
        }
    }
}

/// Exact Git integration approval subject.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct MergeSubject {
    target: MergeTarget,
    reviewed_commit: String,
    target_head: String,
    diff_digest: ContentDigest,
}

impl MergeSubject {
    /// Constructs one structurally valid merge subject.
    ///
    /// # Errors
    ///
    /// Rejects malformed Git references, empty commit/head identities, or a
    /// zero diff digest.
    pub fn new(
        target: MergeTarget,
        reviewed_commit: impl Into<String>,
        target_head: impl Into<String>,
        diff_digest: ContentDigest,
    ) -> Result<Self, ContractError> {
        if target
            .reference()
            .is_some_and(|reference| canonical_local_branch(reference).is_none())
        {
            return Err(ContractError::InvalidApprovalSubject {
                field: "merge_target",
            });
        }
        let reviewed_commit = reviewed_commit.into();
        let target_head = target_head.into();
        if !valid_approval_text(&reviewed_commit) {
            return Err(ContractError::InvalidApprovalSubject {
                field: "reviewed_commit",
            });
        }
        if !valid_approval_text(&target_head) {
            return Err(ContractError::InvalidApprovalSubject {
                field: "target_head",
            });
        }
        if is_zero_digest(&diff_digest) {
            return Err(ContractError::InvalidApprovalSubject {
                field: "diff_digest",
            });
        }
        Ok(Self {
            target,
            reviewed_commit,
            target_head,
            diff_digest,
        })
    }

    /// Returns the exact merge target.
    #[must_use]
    pub const fn target(&self) -> &MergeTarget {
        &self.target
    }

    /// Returns the reviewed commit identity.
    #[must_use]
    pub fn reviewed_commit(&self) -> &str {
        &self.reviewed_commit
    }

    /// Returns the expected target head.
    #[must_use]
    pub fn target_head(&self) -> &str {
        &self.target_head
    }

    /// Returns the exact reviewed diff digest.
    #[must_use]
    pub const fn diff_digest(&self) -> &ContentDigest {
        &self.diff_digest
    }
}

/// Codebase Memory candidate class.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum MemoryKind {
    /// Accepted factual candidate.
    Fact,
    /// Direct observation candidate.
    Observation,
    /// Derived inference candidate.
    Inference,
    /// User preference candidate requiring exact approval.
    Preference,
}

impl MemoryKind {
    /// Returns the stable receipt-facing value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fact => "FACT",
            Self::Observation => "OBSERVATION",
            Self::Inference => "INFERENCE",
            Self::Preference => "PREFERENCE",
        }
    }
}

/// Exact Codebase Memory candidate approval subject.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct MemoryCandidateSubject {
    binding: SubjectBinding,
    candidate_digest: ContentDigest,
    kind: MemoryKind,
}

impl MemoryCandidateSubject {
    /// Constructs one exact memory candidate subject.
    ///
    /// # Errors
    ///
    /// Rejects the all-zero candidate digest sentinel.
    pub fn new(
        binding: SubjectBinding,
        candidate_digest: ContentDigest,
        kind: MemoryKind,
    ) -> Result<Self, ContractError> {
        if is_zero_digest(&candidate_digest) {
            return Err(ContractError::InvalidApprovalSubject {
                field: "candidate_digest",
            });
        }
        Ok(Self {
            binding,
            candidate_digest,
            kind,
        })
    }

    /// Returns the exact task/project binding.
    #[must_use]
    pub const fn binding(&self) -> &SubjectBinding {
        &self.binding
    }

    /// Returns the immutable candidate digest.
    #[must_use]
    pub const fn candidate_digest(&self) -> &ContentDigest {
        &self.candidate_digest
    }

    /// Returns the candidate class.
    #[must_use]
    pub const fn kind(&self) -> MemoryKind {
        self.kind
    }
}

/// Protected operation class.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ProtectedChangeClass {
    AccountOrCredential,
    PaymentOrPurchase,
    PublicExposure,
    ProductionDeployment,
    PermanentDelete,
    DisableSecurity,
    DestructiveMigration,
    Policy,
    Constitution,
    Supervisor,
    CapabilityExpansion,
    PrimaryBranchMerge,
    CoreReleaseActivation,
}

impl ProtectedChangeClass {
    /// Returns the stable receipt-facing value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AccountOrCredential => "ACCOUNT_OR_CREDENTIAL",
            Self::PaymentOrPurchase => "PAYMENT_OR_PURCHASE",
            Self::PublicExposure => "PUBLIC_EXPOSURE",
            Self::ProductionDeployment => "PRODUCTION_DEPLOYMENT",
            Self::PermanentDelete => "PERMANENT_DELETE",
            Self::DisableSecurity => "DISABLE_SECURITY",
            Self::DestructiveMigration => "DESTRUCTIVE_MIGRATION",
            Self::Policy => "POLICY",
            Self::Constitution => "CONSTITUTION",
            Self::Supervisor => "SUPERVISOR",
            Self::CapabilityExpansion => "CAPABILITY_EXPANSION",
            Self::PrimaryBranchMerge => "PRIMARY_BRANCH_MERGE",
            Self::CoreReleaseActivation => "CORE_RELEASE_ACTIVATION",
        }
    }
}

/// Exact protected-operation approval subject.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ProtectedChangeSubject {
    class: ProtectedChangeClass,
    operation_digest: ContentDigest,
}

impl ProtectedChangeSubject {
    /// Constructs one class-separated protected operation.
    ///
    /// # Errors
    ///
    /// Rejects the all-zero operation digest sentinel.
    pub fn new(
        class: ProtectedChangeClass,
        operation_digest: ContentDigest,
    ) -> Result<Self, ContractError> {
        if is_zero_digest(&operation_digest) {
            return Err(ContractError::InvalidApprovalSubject {
                field: "operation_digest",
            });
        }
        Ok(Self {
            class,
            operation_digest,
        })
    }

    /// Returns the protected operation class.
    #[must_use]
    pub const fn class(&self) -> ProtectedChangeClass {
        self.class
    }

    /// Returns the immutable operation digest.
    #[must_use]
    pub const fn operation_digest(&self) -> &ContentDigest {
        &self.operation_digest
    }
}

/// Immutable candidate-release delta classes.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct UpgradeDelta {
    schema_migration: bool,
    policy: bool,
    constitution: bool,
    supervisor: bool,
    credentials: bool,
    public_exposure: bool,
    destructive: bool,
    capability_expansion: bool,
}

impl UpgradeDelta {
    /// Constructs one exact release-delta classification.
    #[allow(clippy::fn_params_excessive_bools, clippy::too_many_arguments)]
    #[must_use]
    pub const fn new(
        schema_migration: bool,
        policy: bool,
        constitution: bool,
        supervisor: bool,
        credentials: bool,
        public_exposure: bool,
        destructive: bool,
        capability_expansion: bool,
    ) -> Self {
        Self {
            schema_migration,
            policy,
            constitution,
            supervisor,
            credentials,
            public_exposure,
            destructive,
            capability_expansion,
        }
    }

    #[must_use]
    pub const fn schema_migration(self) -> bool {
        self.schema_migration
    }

    #[must_use]
    pub const fn policy(self) -> bool {
        self.policy
    }

    #[must_use]
    pub const fn constitution(self) -> bool {
        self.constitution
    }

    #[must_use]
    pub const fn supervisor(self) -> bool {
        self.supervisor
    }

    #[must_use]
    pub const fn credentials(self) -> bool {
        self.credentials
    }

    #[must_use]
    pub const fn public_exposure(self) -> bool {
        self.public_exposure
    }

    #[must_use]
    pub const fn destructive(self) -> bool {
        self.destructive
    }

    #[must_use]
    pub const fn capability_expansion(self) -> bool {
        self.capability_expansion
    }
}

/// Immutable guarded-release approval scope.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ReleaseSubject {
    activation_id: String,
    saga_id: String,
    release_id: String,
    release_revision: String,
    manifest_digest: ContentDigest,
    source_commit: String,
    source_tree_digest: ContentDigest,
    dependency_lock_digest: ContentDigest,
    binary_digests: Vec<ContentDigest>,
    migration_digests: Vec<ContentDigest>,
    evidence_digest: ContentDigest,
    source_release_id: String,
    source_manifest_digest: ContentDigest,
    source_slot_id: String,
    target_slot_id: String,
    requested_epoch: DaemonEpoch,
    schema_compatible: bool,
    delta: UpgradeDelta,
}

impl ReleaseSubject {
    /// Constructs one complete guarded-release subject.
    ///
    /// # Errors
    ///
    /// Rejects malformed identifiers/revision or any required zero digest.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        activation_id: impl Into<String>,
        saga_id: impl Into<String>,
        release_id: impl Into<String>,
        release_revision: impl Into<String>,
        manifest_digest: ContentDigest,
        source_commit: impl Into<String>,
        source_tree_digest: ContentDigest,
        dependency_lock_digest: ContentDigest,
        binary_digests: Vec<ContentDigest>,
        migration_digests: Vec<ContentDigest>,
        evidence_digest: ContentDigest,
        source_release_id: impl Into<String>,
        source_manifest_digest: ContentDigest,
        source_slot_id: impl Into<String>,
        target_slot_id: impl Into<String>,
        requested_epoch: DaemonEpoch,
        schema_compatible: bool,
        delta: UpgradeDelta,
    ) -> Result<Self, ContractError> {
        let activation_id = activation_id.into();
        let saga_id = saga_id.into();
        let release_id = release_id.into();
        let release_revision = release_revision.into();
        let source_commit = source_commit.into();
        let source_release_id = source_release_id.into();
        let source_slot_id = source_slot_id.into();
        let target_slot_id = target_slot_id.into();
        for (field, value) in [
            ("activation_id", activation_id.as_str()),
            ("saga_id", saga_id.as_str()),
            ("release_id", release_id.as_str()),
            ("source_commit", source_commit.as_str()),
            ("source_release_id", source_release_id.as_str()),
            ("source_slot_id", source_slot_id.as_str()),
            ("target_slot_id", target_slot_id.as_str()),
        ] {
            if !valid_approval_identifier(value) {
                return Err(ContractError::InvalidApprovalSubject { field });
            }
        }
        if !canonical_positive_u64(&release_revision) {
            return Err(ContractError::InvalidApprovalSubject {
                field: "release_revision",
            });
        }
        for (field, value) in [
            ("manifest_digest", &manifest_digest),
            ("source_tree_digest", &source_tree_digest),
            ("dependency_lock_digest", &dependency_lock_digest),
            ("evidence_digest", &evidence_digest),
            ("source_manifest_digest", &source_manifest_digest),
        ] {
            if is_zero_digest(value) {
                return Err(ContractError::InvalidApprovalSubject { field });
            }
        }
        if binary_digests.is_empty() || binary_digests.iter().any(is_zero_digest) {
            return Err(ContractError::InvalidApprovalSubject {
                field: "binary_digests",
            });
        }
        if migration_digests.iter().any(is_zero_digest) {
            return Err(ContractError::InvalidApprovalSubject {
                field: "migration_digests",
            });
        }
        Ok(Self {
            activation_id,
            saga_id,
            release_id,
            release_revision,
            manifest_digest,
            source_commit,
            source_tree_digest,
            dependency_lock_digest,
            binary_digests,
            migration_digests,
            evidence_digest,
            source_release_id,
            source_manifest_digest,
            source_slot_id,
            target_slot_id,
            requested_epoch,
            schema_compatible,
            delta,
        })
    }

    #[must_use]
    pub fn activation_id(&self) -> &str {
        &self.activation_id
    }
    #[must_use]
    pub fn saga_id(&self) -> &str {
        &self.saga_id
    }
    #[must_use]
    pub fn release_id(&self) -> &str {
        &self.release_id
    }
    #[must_use]
    pub fn release_revision(&self) -> &str {
        &self.release_revision
    }
    #[must_use]
    pub const fn manifest_digest(&self) -> &ContentDigest {
        &self.manifest_digest
    }
    #[must_use]
    pub fn source_commit(&self) -> &str {
        &self.source_commit
    }
    #[must_use]
    pub const fn source_tree_digest(&self) -> &ContentDigest {
        &self.source_tree_digest
    }
    #[must_use]
    pub const fn dependency_lock_digest(&self) -> &ContentDigest {
        &self.dependency_lock_digest
    }
    #[must_use]
    pub fn binary_digests(&self) -> &[ContentDigest] {
        &self.binary_digests
    }
    #[must_use]
    pub fn migration_digests(&self) -> &[ContentDigest] {
        &self.migration_digests
    }
    #[must_use]
    pub const fn evidence_digest(&self) -> &ContentDigest {
        &self.evidence_digest
    }
    #[must_use]
    pub fn source_release_id(&self) -> &str {
        &self.source_release_id
    }
    #[must_use]
    pub const fn source_manifest_digest(&self) -> &ContentDigest {
        &self.source_manifest_digest
    }
    #[must_use]
    pub fn source_slot_id(&self) -> &str {
        &self.source_slot_id
    }
    #[must_use]
    pub fn target_slot_id(&self) -> &str {
        &self.target_slot_id
    }
    #[must_use]
    pub const fn requested_epoch(&self) -> DaemonEpoch {
        self.requested_epoch
    }
    #[must_use]
    pub const fn schema_compatible(&self) -> bool {
        self.schema_compatible
    }
    #[must_use]
    pub const fn delta(&self) -> UpgradeDelta {
        self.delta
    }
}

/// Exact Guardian process and trust-root identity for one release epoch.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct GuardianRuntimeSubject {
    guardian_id: String,
    trust_root_digest: ContentDigest,
    daemon_instance_id: String,
    observed_epoch: DaemonEpoch,
}

impl GuardianRuntimeSubject {
    /// Constructs one exact Guardian runtime identity.
    ///
    /// # Errors
    ///
    /// Rejects malformed identities or a zero trust-root digest.
    pub fn new(
        guardian_id: impl Into<String>,
        trust_root_digest: ContentDigest,
        daemon_instance_id: impl Into<String>,
        observed_epoch: DaemonEpoch,
    ) -> Result<Self, ContractError> {
        let guardian_id = guardian_id.into();
        let daemon_instance_id = daemon_instance_id.into();
        if !valid_approval_identifier(&guardian_id) {
            return Err(ContractError::InvalidApprovalSubject {
                field: "guardian_id",
            });
        }
        if !valid_approval_identifier(&daemon_instance_id) {
            return Err(ContractError::InvalidApprovalSubject {
                field: "guardian_daemon_instance_id",
            });
        }
        if is_zero_digest(&trust_root_digest) {
            return Err(ContractError::InvalidApprovalSubject {
                field: "trust_root_digest",
            });
        }
        Ok(Self {
            guardian_id,
            trust_root_digest,
            daemon_instance_id,
            observed_epoch,
        })
    }

    #[must_use]
    pub fn guardian_id(&self) -> &str {
        &self.guardian_id
    }
    #[must_use]
    pub const fn trust_root_digest(&self) -> &ContentDigest {
        &self.trust_root_digest
    }
    #[must_use]
    pub fn daemon_instance_id(&self) -> &str {
        &self.daemon_instance_id
    }
    #[must_use]
    pub const fn observed_epoch(&self) -> DaemonEpoch {
        self.observed_epoch
    }
}

/// Domain-separated protected release approval subject.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ProtectedReleaseSubject {
    release: ReleaseSubject,
    guardian: GuardianRuntimeSubject,
}

impl ProtectedReleaseSubject {
    /// Constructs one exact release plus Guardian subject.
    #[must_use]
    pub const fn new(release: ReleaseSubject, guardian: GuardianRuntimeSubject) -> Self {
        Self { release, guardian }
    }

    /// Returns the complete release subject.
    #[must_use]
    pub const fn release(&self) -> &ReleaseSubject {
        &self.release
    }

    /// Returns the exact Guardian runtime subject.
    #[must_use]
    pub const fn guardian(&self) -> &GuardianRuntimeSubject {
        &self.guardian
    }
}

/// Exact immutable approval scope. Kind is derived from the variant.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum ApprovalSubject {
    /// Task execution, optionally including one exact external-cost quote.
    Execution {
        task_spec_hash: ContentDigest,
        external_cost: Option<ExternalCostSubject>,
    },
    /// One exact non-conflicting Git integration.
    Merge(MergeSubject),
    /// One exact Codebase Memory preference candidate.
    Preference(MemoryCandidateSubject),
    /// One exact protected-change intent.
    ProtectedChange(ProtectedChangeSubject),
    /// One exact guarded release activation.
    ProtectedRelease(Box<ProtectedReleaseSubject>),
}

impl ApprovalSubject {
    /// Returns the subject kind derived from the closed typed variant.
    #[must_use]
    pub const fn kind(&self) -> ApprovalKind {
        match self {
            Self::Execution { .. } => ApprovalKind::Execution,
            Self::Merge(_) => ApprovalKind::Merge,
            Self::Preference(_) => ApprovalKind::Preference,
            Self::ProtectedChange(_) => ApprovalKind::ProtectedChange,
            Self::ProtectedRelease(_) => ApprovalKind::ProtectedRelease,
        }
    }

    /// Validates digest fields that remain directly constructible in the enum.
    ///
    /// # Errors
    ///
    /// Rejects the all-zero execution Task Spec digest.
    pub fn validate(&self) -> Result<(), ContractError> {
        if let Self::Execution { task_spec_hash, .. } = self
            && is_zero_digest(task_spec_hash)
        {
            return Err(ContractError::InvalidApprovalSubject {
                field: "execution_task_spec_hash",
            });
        }
        Ok(())
    }
}

/// Authority represented by a verified approval.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ApprovalAuthority {
    /// Internal Policy sufficiency, never accepted as an external receipt.
    InternalPolicy,
    /// Exact authenticated responsible user.
    ResponsibleUser,
    /// Separate protected Guardian trust root.
    ProtectedGuardian,
}

impl ApprovalAuthority {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InternalPolicy => "INTERNAL_POLICY",
            Self::ResponsibleUser => "RESPONSIBLE_USER",
            Self::ProtectedGuardian => "PROTECTED_GUARDIAN",
        }
    }
}

/// Trust surface represented by a verified approval.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ApprovalOrigin {
    PolicyEngine,
    OsAuthenticatedUser,
    GuardianTrustRoot,
    NormalGateway,
    ModelOrCandidate,
    ActiveDaemon,
}

impl ApprovalOrigin {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PolicyEngine => "POLICY_ENGINE",
            Self::OsAuthenticatedUser => "OS_AUTHENTICATED_USER",
            Self::GuardianTrustRoot => "GUARDIAN_TRUST_ROOT",
            Self::NormalGateway => "NORMAL_GATEWAY",
            Self::ModelOrCandidate => "MODEL_OR_CANDIDATE",
            Self::ActiveDaemon => "ACTIVE_DAEMON",
        }
    }
}

/// Closed normal-versus-protected approval trust lane.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ApprovalLane {
    /// Responsible-user, OS-authenticated approval.
    Normal,
    /// Guardian-trust-root protected approval.
    Protected,
}

impl ApprovalLane {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "NORMAL",
            Self::Protected => "PROTECTED",
        }
    }
}

/// Approval authority lifecycle status represented at shared boundaries.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ApprovalStatus {
    /// A normal approval is verified and remains available for one claim.
    Available,
    /// A protected approval awaits the Guardian-only atomic claim.
    ProtectedPendingClaim,
    /// A normal approval was already claimed.
    ClaimedNormal,
    /// Approval authority was explicitly revoked.
    Revoked,
}

impl ApprovalStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Available => "AVAILABLE",
            Self::ProtectedPendingClaim => "PROTECTED_PENDING_CLAIM",
            Self::ClaimedNormal => "CLAIMED_NORMAL",
            Self::Revoked => "REVOKED",
        }
    }
}

/// Closed runtime-admission mode shared by Policy and Writer Lease.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RuntimeAdmissionMode {
    /// The current daemon may perform policy-approved normal work.
    Active,
    /// New work is blocked while bounded drain and release actions complete.
    Draining,
    /// Only the guardian-reserved system health path is admitted.
    Canary,
    /// Normal daemon mutation and effects are stopped.
    Stopped,
    /// Ambiguous runtime state requires explicit reconciliation.
    ReconciliationRequired,
}

impl RuntimeAdmissionMode {
    /// Complete closed runtime-admission set.
    pub const ALL: [Self; 5] = [
        Self::Active,
        Self::Draining,
        Self::Canary,
        Self::Stopped,
        Self::ReconciliationRequired,
    ];

    /// Returns the stable receipt-facing value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::Draining => "DRAINING",
            Self::Canary => "CANARY",
            Self::Stopped => "STOPPED",
            Self::ReconciliationRequired => "RECONCILIATION_REQUIRED",
        }
    }
}

/// Closed state of a currently retained Writer Lease projection.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum WriterLeaseStatus {
    /// The exact current lease may support an otherwise authorized writer use.
    Active,
    /// The lease remains reserved but cannot authorize product mutation.
    Suspect,
}

impl WriterLeaseStatus {
    /// Returns the stable receipt-facing value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::Suspect => "SUSPECT",
        }
    }
}

const MAX_POSITIVE_SIGNED_BIGINT: u64 = 9_223_372_036_854_775_807;

macro_rules! positive_signed_bigint {
    ($name:ident, $field:literal, $description:literal) => {
        #[doc = $description]
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub struct $name(u64);

        impl $name {
            /// Constructs a positive value representable by `PostgreSQL`
            /// `BIGINT`.
            ///
            /// # Errors
            ///
            /// Rejects zero and values greater than signed `BIGINT` maximum.
            pub fn new(value: u64) -> Result<Self, ContractError> {
                if (1..=MAX_POSITIVE_SIGNED_BIGINT).contains(&value) {
                    Ok(Self(value))
                } else {
                    Err(ContractError::InvalidPositiveSignedBigInt { field: $field })
                }
            }

            /// Returns the checked positive value.
            #[must_use]
            pub const fn get(self) -> u64 {
                self.0
            }
        }
    };
}

positive_signed_bigint!(
    DaemonEpoch,
    "daemon_epoch",
    "A non-zero, non-wrapping daemon leadership epoch."
);
positive_signed_bigint!(
    FencingToken,
    "fencing_token",
    "A non-zero, non-wrapping project-writer fencing token."
);
positive_signed_bigint!(
    WriterLeaseRevision,
    "writer_lease_revision",
    "A non-zero revision of one project Writer Lease projection."
);
positive_signed_bigint!(
    ApprovalRevision,
    "approval_revision",
    "A non-zero revision of one Approval Verifier authority projection."
);
positive_signed_bigint!(
    StoreAuthorityRevision,
    "store_authority_revision",
    "A non-zero revision of the independently retained Store daemon authority."
);
positive_signed_bigint!(
    HolderProcessId,
    "holder_process_id",
    "A positive operating-system process identifier bound to a lease holder."
);
positive_signed_bigint!(
    ArtifactGeneration,
    "artifact_generation",
    "A positive non-wrapping generation of one project-scoped artifact object."
);
positive_signed_bigint!(
    ArtifactRevision,
    "artifact_revision",
    "A positive revision of an artifact object, reference, read, or authority projection."
);

macro_rules! non_negative_signed_bigint {
    ($name:ident, $field:literal, $description:literal) => {
        #[doc = $description]
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub struct $name(u64);

        impl $name {
            /// Constructs a non-negative value representable by `PostgreSQL`
            /// `BIGINT`.
            ///
            /// # Errors
            ///
            /// Rejects values greater than signed `BIGINT` maximum.
            pub fn new(value: u64) -> Result<Self, ContractError> {
                if value <= MAX_POSITIVE_SIGNED_BIGINT {
                    Ok(Self(value))
                } else {
                    Err(ContractError::InvalidNonNegativeSignedBigInt { field: $field })
                }
            }

            /// Returns the checked non-negative value.
            #[must_use]
            pub const fn get(self) -> u64 {
                self.0
            }
        }
    };
}

non_negative_signed_bigint!(
    ArtifactByteLength,
    "artifact_byte_length",
    "A non-negative artifact byte length."
);
non_negative_signed_bigint!(
    ArtifactCounter,
    "artifact_counter",
    "A non-negative artifact resource counter."
);
non_negative_signed_bigint!(
    StoreRevision,
    "store_revision",
    "A non-negative physical Store compare-and-swap revision."
);
non_negative_signed_bigint!(
    ArtifactQuotaValue,
    "artifact_quota_value",
    "A non-negative artifact quota or aggregate value."
);

/// Complete immutable identity bound by one verified approval.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ApprovalIdentity {
    approval_id: String,
    challenge_id: String,
    binding: SubjectBinding,
    subject: ApprovalSubject,
    requester_id: String,
    approver_id: String,
    authority: ApprovalAuthority,
    origin: ApprovalOrigin,
    lane: ApprovalLane,
    channel_id: String,
    session_id: String,
}

impl ApprovalIdentity {
    /// Constructs one exact approval identity.
    ///
    /// # Errors
    ///
    /// Rejects malformed identifiers, subject/binding disagreement,
    /// self-approval, unsupported authority/origin/lane pairs, or a protected
    /// release outside the protected Guardian lane.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        approval_id: impl Into<String>,
        challenge_id: impl Into<String>,
        binding: SubjectBinding,
        subject: ApprovalSubject,
        requester_id: impl Into<String>,
        approver_id: impl Into<String>,
        authority: ApprovalAuthority,
        origin: ApprovalOrigin,
        lane: ApprovalLane,
        channel_id: impl Into<String>,
        session_id: impl Into<String>,
    ) -> Result<Self, ContractError> {
        let approval_id = approval_id.into();
        let challenge_id = challenge_id.into();
        let requester_id = requester_id.into();
        let approver_id = approver_id.into();
        let channel_id = channel_id.into();
        let session_id = session_id.into();
        for (field, value) in [
            ("approval_id", approval_id.as_str()),
            ("challenge_id", challenge_id.as_str()),
            ("requester_id", requester_id.as_str()),
            ("approver_id", approver_id.as_str()),
            ("channel_id", channel_id.as_str()),
            ("session_id", session_id.as_str()),
        ] {
            if !valid_approval_identifier(value) {
                return Err(ContractError::InvalidApprovalIdentity { field });
            }
        }
        subject.validate()?;
        let subject_matches_binding = match &subject {
            ApprovalSubject::Execution { task_spec_hash, .. } => {
                task_spec_hash == binding.task_spec_digest()
            }
            ApprovalSubject::Preference(memory) => memory.binding() == &binding,
            ApprovalSubject::Merge(_)
            | ApprovalSubject::ProtectedChange(_)
            | ApprovalSubject::ProtectedRelease(_) => true,
        };
        if !subject_matches_binding {
            return Err(ContractError::InvalidApprovalIdentity {
                field: "subject_binding",
            });
        }
        if requester_id == approver_id {
            return Err(ContractError::InvalidApprovalIdentity {
                field: "self_approval",
            });
        }
        if !valid_approval_authority_pair(authority, origin, lane) {
            return Err(ContractError::InvalidApprovalIdentity {
                field: "authority_origin_lane",
            });
        }
        if matches!(&subject, ApprovalSubject::ProtectedRelease(_))
            && lane != ApprovalLane::Protected
        {
            return Err(ContractError::InvalidApprovalIdentity {
                field: "protected_release_lane",
            });
        }
        Ok(Self {
            approval_id,
            challenge_id,
            binding,
            subject,
            requester_id,
            approver_id,
            authority,
            origin,
            lane,
            channel_id,
            session_id,
        })
    }

    #[must_use]
    pub fn approval_id(&self) -> &str {
        &self.approval_id
    }
    #[must_use]
    pub fn challenge_id(&self) -> &str {
        &self.challenge_id
    }
    #[must_use]
    pub const fn binding(&self) -> &SubjectBinding {
        &self.binding
    }
    #[must_use]
    pub const fn subject(&self) -> &ApprovalSubject {
        &self.subject
    }
    #[must_use]
    pub fn requester_id(&self) -> &str {
        &self.requester_id
    }
    #[must_use]
    pub fn approver_id(&self) -> &str {
        &self.approver_id
    }
    #[must_use]
    pub const fn authority(&self) -> ApprovalAuthority {
        self.authority
    }
    #[must_use]
    pub const fn origin(&self) -> ApprovalOrigin {
        self.origin
    }
    #[must_use]
    pub const fn lane(&self) -> ApprovalLane {
        self.lane
    }
    #[must_use]
    pub fn channel_id(&self) -> &str {
        &self.channel_id
    }
    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
}

/// Immutable Approval-Verifier-owned authority receipt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovalAuthorityReceipt {
    version: ContractVersion,
    producer_id: String,
    producer_version: String,
    runtime: RuntimeKind,
    identity: ApprovalIdentity,
    revision: ApprovalRevision,
    status: ApprovalStatus,
    nonce_id: String,
    nonce_commitment: ContentDigest,
    issued_at: String,
    expires_at: String,
    subject_digest: ContentDigest,
    challenge_digest: ContentDigest,
    authenticator_id: String,
    key_id: String,
    proof_digest: ContentDigest,
    evidence_digest: ContentDigest,
    review_set_digest: Option<ContentDigest>,
    receipt_digest: ContentDigest,
}

impl ApprovalAuthorityReceipt {
    /// Constructs one complete immutable approval authority receipt.
    ///
    /// # Errors
    ///
    /// Rejects unknown producer/version, malformed identifiers/timestamps,
    /// unavailable receipt statuses, lane/status disagreement, or zero
    /// security digests.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        version: u16,
        producer_id: impl Into<String>,
        producer_version: impl Into<String>,
        runtime: RuntimeKind,
        identity: ApprovalIdentity,
        revision: ApprovalRevision,
        status: ApprovalStatus,
        nonce_id: impl Into<String>,
        nonce_commitment: ContentDigest,
        issued_at: impl Into<String>,
        expires_at: impl Into<String>,
        subject_digest: ContentDigest,
        challenge_digest: ContentDigest,
        authenticator_id: impl Into<String>,
        key_id: impl Into<String>,
        proof_digest: ContentDigest,
        evidence_digest: ContentDigest,
        review_set_digest: Option<ContentDigest>,
        receipt_digest: ContentDigest,
    ) -> Result<Self, ContractError> {
        let version = ContractVersion::new(version)?;
        let producer_id = producer_id.into();
        validate_approval_verifier_producer(&producer_id)?;
        let producer_version = producer_version.into();
        validate_approval_verifier_producer_version(&producer_version)?;
        let nonce_id = nonce_id.into();
        let issued_at = issued_at.into();
        let expires_at = expires_at.into();
        let authenticator_id = authenticator_id.into();
        let key_id = key_id.into();
        validate_approval_authority_fields(
            &identity,
            status,
            &nonce_id,
            &nonce_commitment,
            &issued_at,
            &expires_at,
            &subject_digest,
            &challenge_digest,
            &authenticator_id,
            &key_id,
            &proof_digest,
            &evidence_digest,
            review_set_digest.as_ref(),
            &receipt_digest,
        )?;
        if !matches!(
            (identity.lane(), status),
            (ApprovalLane::Normal, ApprovalStatus::Available)
                | (
                    ApprovalLane::Protected,
                    ApprovalStatus::ProtectedPendingClaim
                )
        ) {
            return Err(ContractError::InvalidApprovalReceipt {
                field: "receipt_status",
            });
        }
        Ok(Self {
            version,
            producer_id,
            producer_version,
            runtime,
            identity,
            revision,
            status,
            nonce_id,
            nonce_commitment,
            issued_at,
            expires_at,
            subject_digest,
            challenge_digest,
            authenticator_id,
            key_id,
            proof_digest,
            evidence_digest,
            review_set_digest,
            receipt_digest,
        })
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version.get()
    }
    #[must_use]
    pub fn producer_id(&self) -> &str {
        &self.producer_id
    }
    #[must_use]
    pub fn producer_version(&self) -> &str {
        &self.producer_version
    }
    #[must_use]
    pub const fn runtime(&self) -> RuntimeKind {
        self.runtime
    }
    #[must_use]
    pub const fn identity(&self) -> &ApprovalIdentity {
        &self.identity
    }
    #[must_use]
    pub const fn revision(&self) -> ApprovalRevision {
        self.revision
    }
    #[must_use]
    pub const fn status(&self) -> ApprovalStatus {
        self.status
    }
    #[must_use]
    pub fn nonce_id(&self) -> &str {
        &self.nonce_id
    }
    #[must_use]
    pub const fn nonce_commitment(&self) -> &ContentDigest {
        &self.nonce_commitment
    }
    #[must_use]
    pub fn issued_at(&self) -> &str {
        &self.issued_at
    }
    #[must_use]
    pub fn expires_at(&self) -> &str {
        &self.expires_at
    }
    #[must_use]
    pub const fn subject_digest(&self) -> &ContentDigest {
        &self.subject_digest
    }
    #[must_use]
    pub const fn challenge_digest(&self) -> &ContentDigest {
        &self.challenge_digest
    }
    #[must_use]
    pub fn authenticator_id(&self) -> &str {
        &self.authenticator_id
    }
    #[must_use]
    pub fn key_id(&self) -> &str {
        &self.key_id
    }
    #[must_use]
    pub const fn proof_digest(&self) -> &ContentDigest {
        &self.proof_digest
    }
    #[must_use]
    pub const fn evidence_digest(&self) -> &ContentDigest {
        &self.evidence_digest
    }
    #[must_use]
    pub const fn review_set_digest(&self) -> Option<&ContentDigest> {
        self.review_set_digest.as_ref()
    }
    #[must_use]
    pub const fn receipt_digest(&self) -> &ContentDigest {
        &self.receipt_digest
    }

    /// Projects every security-relevant field into a structural authority head.
    ///
    /// This projection does not establish independent currentness.
    #[must_use]
    pub fn head(&self) -> ApprovalAuthorityHead {
        ApprovalAuthorityHead {
            version: self.version,
            producer_id: self.producer_id.clone(),
            producer_version: self.producer_version.clone(),
            runtime: self.runtime,
            identity: self.identity.clone(),
            revision: self.revision,
            status: self.status,
            nonce_id: self.nonce_id.clone(),
            nonce_commitment: self.nonce_commitment.clone(),
            issued_at: self.issued_at.clone(),
            expires_at: self.expires_at.clone(),
            subject_digest: self.subject_digest.clone(),
            challenge_digest: self.challenge_digest.clone(),
            authenticator_id: self.authenticator_id.clone(),
            key_id: self.key_id.clone(),
            proof_digest: self.proof_digest.clone(),
            evidence_digest: self.evidence_digest.clone(),
            review_set_digest: self.review_set_digest.clone(),
            receipt_digest: self.receipt_digest.clone(),
        }
    }
}

/// Full Approval Verifier authority head returned by an independent owner lookup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovalAuthorityHead {
    version: ContractVersion,
    producer_id: String,
    producer_version: String,
    runtime: RuntimeKind,
    identity: ApprovalIdentity,
    revision: ApprovalRevision,
    status: ApprovalStatus,
    nonce_id: String,
    nonce_commitment: ContentDigest,
    issued_at: String,
    expires_at: String,
    subject_digest: ContentDigest,
    challenge_digest: ContentDigest,
    authenticator_id: String,
    key_id: String,
    proof_digest: ContentDigest,
    evidence_digest: ContentDigest,
    review_set_digest: Option<ContentDigest>,
    receipt_digest: ContentDigest,
}

impl ApprovalAuthorityHead {
    /// Constructs one complete authority head supplied by an owner lookup.
    ///
    /// # Errors
    ///
    /// Applies the same producer, identity, lane, text, and digest validation
    /// as the receipt representation.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        version: u16,
        producer_id: impl Into<String>,
        producer_version: impl Into<String>,
        runtime: RuntimeKind,
        identity: ApprovalIdentity,
        revision: ApprovalRevision,
        status: ApprovalStatus,
        nonce_id: impl Into<String>,
        nonce_commitment: ContentDigest,
        issued_at: impl Into<String>,
        expires_at: impl Into<String>,
        subject_digest: ContentDigest,
        challenge_digest: ContentDigest,
        authenticator_id: impl Into<String>,
        key_id: impl Into<String>,
        proof_digest: ContentDigest,
        evidence_digest: ContentDigest,
        review_set_digest: Option<ContentDigest>,
        receipt_digest: ContentDigest,
    ) -> Result<Self, ContractError> {
        let version = ContractVersion::new(version)?;
        let producer_id = producer_id.into();
        validate_approval_verifier_producer(&producer_id)?;
        let producer_version = producer_version.into();
        validate_approval_verifier_producer_version(&producer_version)?;
        let nonce_id = nonce_id.into();
        let issued_at = issued_at.into();
        let expires_at = expires_at.into();
        let authenticator_id = authenticator_id.into();
        let key_id = key_id.into();
        validate_approval_authority_fields(
            &identity,
            status,
            &nonce_id,
            &nonce_commitment,
            &issued_at,
            &expires_at,
            &subject_digest,
            &challenge_digest,
            &authenticator_id,
            &key_id,
            &proof_digest,
            &evidence_digest,
            review_set_digest.as_ref(),
            &receipt_digest,
        )?;
        Ok(Self {
            version,
            producer_id,
            producer_version,
            runtime,
            identity,
            revision,
            status,
            nonce_id,
            nonce_commitment,
            issued_at,
            expires_at,
            subject_digest,
            challenge_digest,
            authenticator_id,
            key_id,
            proof_digest,
            evidence_digest,
            review_set_digest,
            receipt_digest,
        })
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version.get()
    }
    #[must_use]
    pub fn producer_id(&self) -> &str {
        &self.producer_id
    }
    #[must_use]
    pub fn producer_version(&self) -> &str {
        &self.producer_version
    }
    #[must_use]
    pub const fn runtime(&self) -> RuntimeKind {
        self.runtime
    }
    #[must_use]
    pub const fn identity(&self) -> &ApprovalIdentity {
        &self.identity
    }
    #[must_use]
    pub const fn revision(&self) -> ApprovalRevision {
        self.revision
    }
    #[must_use]
    pub const fn status(&self) -> ApprovalStatus {
        self.status
    }
    #[must_use]
    pub fn nonce_id(&self) -> &str {
        &self.nonce_id
    }
    #[must_use]
    pub const fn nonce_commitment(&self) -> &ContentDigest {
        &self.nonce_commitment
    }
    #[must_use]
    pub fn issued_at(&self) -> &str {
        &self.issued_at
    }
    #[must_use]
    pub fn expires_at(&self) -> &str {
        &self.expires_at
    }
    #[must_use]
    pub const fn subject_digest(&self) -> &ContentDigest {
        &self.subject_digest
    }
    #[must_use]
    pub const fn challenge_digest(&self) -> &ContentDigest {
        &self.challenge_digest
    }
    #[must_use]
    pub fn authenticator_id(&self) -> &str {
        &self.authenticator_id
    }
    #[must_use]
    pub fn key_id(&self) -> &str {
        &self.key_id
    }
    #[must_use]
    pub const fn proof_digest(&self) -> &ContentDigest {
        &self.proof_digest
    }
    #[must_use]
    pub const fn evidence_digest(&self) -> &ContentDigest {
        &self.evidence_digest
    }
    #[must_use]
    pub const fn review_set_digest(&self) -> Option<&ContentDigest> {
        self.review_set_digest.as_ref()
    }
    #[must_use]
    pub const fn receipt_digest(&self) -> &ContentDigest {
        &self.receipt_digest
    }
}

/// Complete immutable identity bound by one current Writer Lease.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct WriterLeaseIdentity {
    project_id: ProjectId,
    project_snapshot_id: ProjectSnapshotId,
    task_id: TaskId,
    task_revision: String,
    task_spec_digest: ContentDigest,
    attempt_id: AttemptId,
    lease_id: String,
    lease_holder_id: String,
    worktree_id: String,
    holder_process_id: HolderProcessId,
    holder_process_start_identity: ContentDigest,
    daemon_instance_id: String,
    daemon_epoch: DaemonEpoch,
    fencing_token: FencingToken,
}

impl WriterLeaseIdentity {
    /// Constructs one exact Writer Lease identity without normalization.
    ///
    /// # Errors
    ///
    /// Rejects malformed shared identifiers, a non-canonical positive task
    /// revision, or any raw identifier outside the bounded ASCII contract.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        project_id: ProjectId,
        project_snapshot_id: ProjectSnapshotId,
        task_id: TaskId,
        task_revision: impl Into<String>,
        task_spec_digest: ContentDigest,
        attempt_id: AttemptId,
        lease_id: impl Into<String>,
        lease_holder_id: impl Into<String>,
        worktree_id: impl Into<String>,
        holder_process_id: HolderProcessId,
        holder_process_start_identity: ContentDigest,
        daemon_instance_id: impl Into<String>,
        daemon_epoch: DaemonEpoch,
        fencing_token: FencingToken,
    ) -> Result<Self, ContractError> {
        let task_revision = task_revision.into();
        if !canonical_positive_u64(&task_revision) {
            return Err(ContractError::InvalidTaskRevision);
        }

        let lease_id = lease_id.into();
        let lease_holder_id = lease_holder_id.into();
        let worktree_id = worktree_id.into();
        let daemon_instance_id = daemon_instance_id.into();
        for (field, value) in [
            ("project_snapshot_id", project_snapshot_id.as_str()),
            ("task_id", task_id.as_str()),
            ("attempt_id", attempt_id.as_str()),
            ("lease_id", lease_id.as_str()),
            ("lease_holder_id", lease_holder_id.as_str()),
            ("worktree_id", worktree_id.as_str()),
            ("daemon_instance_id", daemon_instance_id.as_str()),
        ] {
            if !valid_writer_lease_identifier(value) {
                return Err(ContractError::InvalidWriterLeaseIdentifier { field });
            }
        }
        if is_zero_digest(&holder_process_start_identity) {
            return Err(ContractError::InvalidWriterLeaseIdentity {
                field: "holder_process_start_identity",
            });
        }

        Ok(Self {
            project_id,
            project_snapshot_id,
            task_id,
            task_revision,
            task_spec_digest,
            attempt_id,
            lease_id,
            lease_holder_id,
            worktree_id,
            holder_process_id,
            holder_process_start_identity,
            daemon_instance_id,
            daemon_epoch,
            fencing_token,
        })
    }

    /// Returns the canonical registered project identity.
    #[must_use]
    pub const fn project_id(&self) -> &ProjectId {
        &self.project_id
    }

    /// Returns the immutable Project Registry snapshot identity.
    #[must_use]
    pub const fn project_snapshot_id(&self) -> &ProjectSnapshotId {
        &self.project_snapshot_id
    }

    /// Returns the immutable task identity.
    #[must_use]
    pub const fn task_id(&self) -> &TaskId {
        &self.task_id
    }

    /// Returns the canonical positive Task Spec revision.
    #[must_use]
    pub fn task_revision(&self) -> &str {
        &self.task_revision
    }

    /// Returns the immutable Task Spec digest.
    #[must_use]
    pub const fn task_spec_digest(&self) -> &ContentDigest {
        &self.task_spec_digest
    }

    /// Returns the exact task-attempt identity.
    #[must_use]
    pub const fn attempt_id(&self) -> &AttemptId {
        &self.attempt_id
    }

    /// Returns the exact lease identifier.
    #[must_use]
    pub fn lease_id(&self) -> &str {
        &self.lease_id
    }

    /// Returns the logical Implementer holder identity.
    #[must_use]
    pub fn lease_holder_id(&self) -> &str {
        &self.lease_holder_id
    }

    /// Returns the LATTICE-owned worktree identity.
    #[must_use]
    pub fn worktree_id(&self) -> &str {
        &self.worktree_id
    }

    /// Returns the exact holder process identifier.
    #[must_use]
    pub const fn holder_process_id(&self) -> HolderProcessId {
        self.holder_process_id
    }

    /// Returns the holder process-start identity used to detect PID reuse.
    #[must_use]
    pub const fn holder_process_start_identity(&self) -> &ContentDigest {
        &self.holder_process_start_identity
    }

    /// Returns the daemon instance that owns the writer attempt.
    #[must_use]
    pub fn daemon_instance_id(&self) -> &str {
        &self.daemon_instance_id
    }

    /// Returns the non-wrapping daemon epoch.
    #[must_use]
    pub const fn daemon_epoch(&self) -> DaemonEpoch {
        self.daemon_epoch
    }

    /// Returns the non-wrapping project fencing token.
    #[must_use]
    pub const fn fencing_token(&self) -> FencingToken {
        self.fencing_token
    }
}

/// Immutable Writer-Lease-owned authority observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriterLeaseAuthorityReceipt {
    version: ContractVersion,
    producer_id: String,
    producer_version: String,
    runtime: RuntimeKind,
    identity: WriterLeaseIdentity,
    status: WriterLeaseStatus,
    revision: WriterLeaseRevision,
    runtime_admission: RuntimeAdmissionMode,
    acquired_at: String,
    heartbeat_at: String,
    expires_at: String,
    time_observation_digest: ContentDigest,
    admission_observation_digest: ContentDigest,
    transition_digest: ContentDigest,
    receipt_digest: ContentDigest,
}

impl WriterLeaseAuthorityReceipt {
    /// Constructs a validated immutable Writer Lease authority receipt.
    ///
    /// Timestamp syntax and ordering remain Writer Lease owner semantics; this
    /// shared representation rejects only empty, padded, oversized, or
    /// NUL-bearing timestamp text.
    ///
    /// # Errors
    ///
    /// Rejects unsupported contract/producer versions, structurally invalid
    /// timestamp text, or zero authority digests.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        version: u16,
        producer_id: impl Into<String>,
        producer_version: impl Into<String>,
        runtime: RuntimeKind,
        identity: WriterLeaseIdentity,
        status: WriterLeaseStatus,
        revision: WriterLeaseRevision,
        runtime_admission: RuntimeAdmissionMode,
        acquired_at: impl Into<String>,
        heartbeat_at: impl Into<String>,
        expires_at: impl Into<String>,
        time_observation_digest: ContentDigest,
        admission_observation_digest: ContentDigest,
        transition_digest: ContentDigest,
        receipt_digest: ContentDigest,
    ) -> Result<Self, ContractError> {
        let version = ContractVersion::new(version)?;
        let producer_id = producer_id.into();
        validate_writer_lease_producer(&producer_id)?;
        let producer_version = producer_version.into();
        validate_writer_lease_producer_version(&producer_version)?;

        let acquired_at = acquired_at.into();
        let heartbeat_at = heartbeat_at.into();
        let expires_at = expires_at.into();
        for (field, value) in [
            ("acquired_at", acquired_at.as_str()),
            ("heartbeat_at", heartbeat_at.as_str()),
            ("expires_at", expires_at.as_str()),
        ] {
            if !valid_writer_lease_receipt_text(value) {
                return Err(ContractError::InvalidWriterLeaseReceipt { field });
            }
        }
        for (field, value) in [
            ("time_observation_digest", &time_observation_digest),
            (
                "admission_observation_digest",
                &admission_observation_digest,
            ),
            ("transition_digest", &transition_digest),
            ("receipt_digest", &receipt_digest),
        ] {
            if is_zero_digest(value) {
                return Err(ContractError::InvalidWriterLeaseReceipt { field });
            }
        }

        Ok(Self {
            version,
            producer_id,
            producer_version,
            runtime,
            identity,
            status,
            revision,
            runtime_admission,
            acquired_at,
            heartbeat_at,
            expires_at,
            time_observation_digest,
            admission_observation_digest,
            transition_digest,
            receipt_digest,
        })
    }

    /// Returns the shared contract version.
    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version.get()
    }

    /// Returns the fixed semantic producer identity.
    #[must_use]
    pub fn producer_id(&self) -> &str {
        &self.producer_id
    }

    /// Returns the fixed semantic producer version.
    #[must_use]
    pub fn producer_version(&self) -> &str {
        &self.producer_version
    }

    /// Returns whether the receipt came from a fake or live owner.
    #[must_use]
    pub const fn runtime(&self) -> RuntimeKind {
        self.runtime
    }

    /// Returns the complete current lease identity.
    #[must_use]
    pub const fn identity(&self) -> &WriterLeaseIdentity {
        &self.identity
    }

    /// Returns the exact lease status represented by this receipt.
    #[must_use]
    pub const fn status(&self) -> WriterLeaseStatus {
        self.status
    }

    /// Returns the current lease projection revision.
    #[must_use]
    pub const fn revision(&self) -> WriterLeaseRevision {
        self.revision
    }

    /// Returns the runtime-admission observation bound to this receipt.
    #[must_use]
    pub const fn runtime_admission(&self) -> RuntimeAdmissionMode {
        self.runtime_admission
    }

    /// Returns the caller-supplied canonical acquisition timestamp text.
    #[must_use]
    pub fn acquired_at(&self) -> &str {
        &self.acquired_at
    }

    /// Returns the caller-supplied canonical heartbeat timestamp text.
    #[must_use]
    pub fn heartbeat_at(&self) -> &str {
        &self.heartbeat_at
    }

    /// Returns the caller-supplied canonical expiry timestamp text.
    #[must_use]
    pub fn expires_at(&self) -> &str {
        &self.expires_at
    }

    /// Returns the digest of exact owner time evidence.
    #[must_use]
    pub const fn time_observation_digest(&self) -> &ContentDigest {
        &self.time_observation_digest
    }

    /// Returns the digest of exact daemon/runtime-admission evidence.
    #[must_use]
    pub const fn admission_observation_digest(&self) -> &ContentDigest {
        &self.admission_observation_digest
    }

    /// Returns the digest of the exact Writer Lease transition.
    #[must_use]
    pub const fn transition_digest(&self) -> &ContentDigest {
        &self.transition_digest
    }

    /// Returns the digest of this complete immutable receipt.
    #[must_use]
    pub const fn receipt_digest(&self) -> &ContentDigest {
        &self.receipt_digest
    }

    /// Projects every security field into a structural authority head.
    ///
    /// This projection does not prove that the receipt remains current.
    #[must_use]
    pub fn head(&self) -> WriterLeaseAuthorityHead {
        WriterLeaseAuthorityHead {
            version: self.version,
            producer_id: self.producer_id.clone(),
            producer_version: self.producer_version.clone(),
            runtime: self.runtime,
            identity: self.identity.clone(),
            status: self.status,
            revision: self.revision,
            runtime_admission: self.runtime_admission,
            acquired_at: self.acquired_at.clone(),
            heartbeat_at: self.heartbeat_at.clone(),
            expires_at: self.expires_at.clone(),
            time_observation_digest: self.time_observation_digest.clone(),
            admission_observation_digest: self.admission_observation_digest.clone(),
            transition_digest: self.transition_digest.clone(),
            receipt_digest: self.receipt_digest.clone(),
        }
    }
}

/// Full Writer Lease authority head returned by an independent owner lookup.
///
/// A receipt can project this shape structurally, but only the Writer Lease
/// owner can establish that a head is current.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriterLeaseAuthorityHead {
    version: ContractVersion,
    producer_id: String,
    producer_version: String,
    runtime: RuntimeKind,
    identity: WriterLeaseIdentity,
    status: WriterLeaseStatus,
    revision: WriterLeaseRevision,
    runtime_admission: RuntimeAdmissionMode,
    acquired_at: String,
    heartbeat_at: String,
    expires_at: String,
    time_observation_digest: ContentDigest,
    admission_observation_digest: ContentDigest,
    transition_digest: ContentDigest,
    receipt_digest: ContentDigest,
}

impl WriterLeaseAuthorityHead {
    /// Returns the shared contract version.
    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version.get()
    }

    /// Returns the fixed semantic producer identity.
    #[must_use]
    pub fn producer_id(&self) -> &str {
        &self.producer_id
    }

    /// Returns the fixed semantic producer version.
    #[must_use]
    pub fn producer_version(&self) -> &str {
        &self.producer_version
    }

    /// Returns whether the represented authority is fake or live.
    #[must_use]
    pub const fn runtime(&self) -> RuntimeKind {
        self.runtime
    }

    /// Returns the complete current lease identity.
    #[must_use]
    pub const fn identity(&self) -> &WriterLeaseIdentity {
        &self.identity
    }

    /// Returns the exact current lease status.
    #[must_use]
    pub const fn status(&self) -> WriterLeaseStatus {
        self.status
    }

    /// Returns the current lease projection revision.
    #[must_use]
    pub const fn revision(&self) -> WriterLeaseRevision {
        self.revision
    }

    /// Returns the runtime-admission mode bound to the owner observation.
    #[must_use]
    pub const fn runtime_admission(&self) -> RuntimeAdmissionMode {
        self.runtime_admission
    }

    /// Returns the exact acquisition timestamp text.
    #[must_use]
    pub fn acquired_at(&self) -> &str {
        &self.acquired_at
    }

    /// Returns the exact heartbeat timestamp text.
    #[must_use]
    pub fn heartbeat_at(&self) -> &str {
        &self.heartbeat_at
    }

    /// Returns the exact expiry timestamp text.
    #[must_use]
    pub fn expires_at(&self) -> &str {
        &self.expires_at
    }

    /// Returns the digest of exact owner time evidence.
    #[must_use]
    pub const fn time_observation_digest(&self) -> &ContentDigest {
        &self.time_observation_digest
    }

    /// Returns the digest of exact daemon/runtime-admission evidence.
    #[must_use]
    pub const fn admission_observation_digest(&self) -> &ContentDigest {
        &self.admission_observation_digest
    }

    /// Returns the digest of the exact Writer Lease transition.
    #[must_use]
    pub const fn transition_digest(&self) -> &ContentDigest {
        &self.transition_digest
    }

    /// Returns the digest of the current authority receipt.
    #[must_use]
    pub const fn receipt_digest(&self) -> &ContentDigest {
        &self.receipt_digest
    }
}

/// Immutable task-agnostic authority receipt issued by Project Registry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectAuthorityReceipt {
    version: ContractVersion,
    producer_id: String,
    producer_version: String,
    runtime: RuntimeKind,
    project_id: ProjectId,
    project_snapshot_id: ProjectSnapshotId,
    registry_revision: u64,
    lifecycle: ProjectLifecycle,
    project_class: ProjectClass,
    primary_branch: GitRefIdentity,
    observation_digest: ContentDigest,
    receipt_digest: ContentDigest,
}

impl ProjectAuthorityReceipt {
    /// Constructs a validated immutable Project Registry authority receipt.
    ///
    /// # Errors
    ///
    /// Rejects an unsupported contract version, empty producer identity or
    /// version, and revision zero.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        version: u16,
        producer_id: impl Into<String>,
        producer_version: impl Into<String>,
        runtime: RuntimeKind,
        project_id: ProjectId,
        project_snapshot_id: ProjectSnapshotId,
        registry_revision: u64,
        lifecycle: ProjectLifecycle,
        project_class: ProjectClass,
        primary_branch: GitRefIdentity,
        observation_digest: ContentDigest,
        receipt_digest: ContentDigest,
    ) -> Result<Self, ContractError> {
        let version = ContractVersion::new(version)?;
        let producer_id = producer_id.into();
        if producer_id.trim().is_empty() {
            return Err(ContractError::EmptyIdentifier {
                field: "project_authority_producer_id",
            });
        }
        if producer_id != PROJECT_AUTHORITY_PRODUCER_ID {
            return Err(ContractError::UnsupportedProjectAuthorityProducer);
        }
        let producer_version = producer_version.into();
        if producer_version.trim().is_empty() {
            return Err(ContractError::EmptyIdentifier {
                field: "project_authority_producer_version",
            });
        }
        if producer_version != PROJECT_AUTHORITY_PRODUCER_VERSION {
            return Err(ContractError::UnsupportedProjectAuthorityProducerVersion);
        }
        if registry_revision == 0 {
            return Err(ContractError::ZeroRevision);
        }
        Ok(Self {
            version,
            producer_id,
            producer_version,
            runtime,
            project_id,
            project_snapshot_id,
            registry_revision,
            lifecycle,
            project_class,
            primary_branch,
            observation_digest,
            receipt_digest,
        })
    }

    /// Returns the shared contract version.
    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version.get()
    }

    /// Returns the Registry implementation identity.
    #[must_use]
    pub fn producer_id(&self) -> &str {
        &self.producer_id
    }

    /// Returns the Registry implementation version.
    #[must_use]
    pub fn producer_version(&self) -> &str {
        &self.producer_version
    }

    /// Returns whether this receipt came from a fake or live owner.
    #[must_use]
    pub const fn runtime(&self) -> RuntimeKind {
        self.runtime
    }

    /// Returns the registered project identity.
    #[must_use]
    pub const fn project_id(&self) -> &ProjectId {
        &self.project_id
    }

    /// Returns the immutable Registry snapshot identity.
    #[must_use]
    pub const fn project_snapshot_id(&self) -> &ProjectSnapshotId {
        &self.project_snapshot_id
    }

    /// Returns the non-zero Registry revision.
    #[must_use]
    pub const fn registry_revision(&self) -> u64 {
        self.registry_revision
    }

    /// Returns the closed Registry lifecycle.
    #[must_use]
    pub const fn lifecycle(&self) -> ProjectLifecycle {
        self.lifecycle
    }

    /// Returns the immutable project class.
    #[must_use]
    pub const fn project_class(&self) -> ProjectClass {
        self.project_class
    }

    /// Returns the canonical primary local-ref identity.
    #[must_use]
    pub const fn primary_branch(&self) -> &GitRefIdentity {
        &self.primary_branch
    }

    /// Returns the digest of the complete owner observation.
    #[must_use]
    pub const fn observation_digest(&self) -> &ContentDigest {
        &self.observation_digest
    }

    /// Returns the digest of the complete immutable authority receipt.
    #[must_use]
    pub const fn receipt_digest(&self) -> &ContentDigest {
        &self.receipt_digest
    }

    /// Projects the complete head represented by this receipt.
    ///
    /// This is a structural projection, not proof that the receipt remains
    /// current. Consumers must compare it with an independent Registry lookup.
    #[must_use]
    pub fn head(&self) -> ProjectAuthorityHead {
        ProjectAuthorityHead {
            producer_id: self.producer_id.clone(),
            producer_version: self.producer_version.clone(),
            runtime: self.runtime,
            project_id: self.project_id.clone(),
            project_snapshot_id: self.project_snapshot_id.clone(),
            registry_revision: self.registry_revision,
            lifecycle: self.lifecycle,
            project_class: self.project_class,
            primary_branch: self.primary_branch.clone(),
            observation_digest: self.observation_digest.clone(),
            receipt_digest: self.receipt_digest.clone(),
        }
    }
}

/// Exact Registry head used by an independent owner lookup to compare every
/// security-relevant authority field.
///
/// A receipt can project the head it represents, but that projection alone
/// does not prove currentness. Policy callers must supply the current head
/// returned by the Registry owner.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectAuthorityHead {
    producer_id: String,
    producer_version: String,
    runtime: RuntimeKind,
    project_id: ProjectId,
    project_snapshot_id: ProjectSnapshotId,
    registry_revision: u64,
    lifecycle: ProjectLifecycle,
    project_class: ProjectClass,
    primary_branch: GitRefIdentity,
    observation_digest: ContentDigest,
    receipt_digest: ContentDigest,
}

impl ProjectAuthorityHead {
    /// Returns the Registry semantic producer identity.
    #[must_use]
    pub fn producer_id(&self) -> &str {
        &self.producer_id
    }

    /// Returns the Registry semantic producer version.
    #[must_use]
    pub fn producer_version(&self) -> &str {
        &self.producer_version
    }

    /// Returns whether the represented authority is fake or live.
    #[must_use]
    pub const fn runtime(&self) -> RuntimeKind {
        self.runtime
    }

    /// Returns the registered project identity.
    #[must_use]
    pub const fn project_id(&self) -> &ProjectId {
        &self.project_id
    }

    /// Returns the current immutable snapshot identity.
    #[must_use]
    pub const fn project_snapshot_id(&self) -> &ProjectSnapshotId {
        &self.project_snapshot_id
    }

    /// Returns the current non-zero Registry revision.
    #[must_use]
    pub const fn registry_revision(&self) -> u64 {
        self.registry_revision
    }

    /// Returns the exact lifecycle represented by the current owner head.
    #[must_use]
    pub const fn lifecycle(&self) -> ProjectLifecycle {
        self.lifecycle
    }

    /// Returns the immutable project class represented by the owner head.
    #[must_use]
    pub const fn project_class(&self) -> ProjectClass {
        self.project_class
    }

    /// Returns the exact primary local-ref identity represented by the head.
    #[must_use]
    pub const fn primary_branch(&self) -> &GitRefIdentity {
        &self.primary_branch
    }

    /// Returns the complete owner-observation digest represented by the head.
    #[must_use]
    pub const fn observation_digest(&self) -> &ContentDigest {
        &self.observation_digest
    }

    /// Returns the exact current authority-receipt digest.
    #[must_use]
    pub const fn receipt_digest(&self) -> &ContentDigest {
        &self.receipt_digest
    }
}

/// Complete immutable identity of one Task Ledger task stream.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct TaskLedgerStreamIdentity {
    project_id: ProjectId,
    project_snapshot_id: ProjectSnapshotId,
    task_id: TaskId,
    task_revision: String,
    task_spec_digest: ContentDigest,
    accounting_currency: String,
}

impl TaskLedgerStreamIdentity {
    /// Constructs one exact stream identity without hidden normalization.
    ///
    /// # Errors
    ///
    /// Rejects a non-canonical positive revision or a currency other than
    /// exactly three uppercase ASCII letters.
    pub fn new(
        project_id: ProjectId,
        project_snapshot_id: ProjectSnapshotId,
        task_id: TaskId,
        task_revision: impl Into<String>,
        task_spec_digest: ContentDigest,
        accounting_currency: impl Into<String>,
    ) -> Result<Self, ContractError> {
        let task_revision = task_revision.into();
        if !canonical_positive_u64(&task_revision) {
            return Err(ContractError::InvalidTaskRevision);
        }
        let accounting_currency = accounting_currency.into();
        if !valid_accounting_currency(&accounting_currency) {
            return Err(ContractError::InvalidAccountingCurrency);
        }
        Ok(Self {
            project_id,
            project_snapshot_id,
            task_id,
            task_revision,
            task_spec_digest,
            accounting_currency,
        })
    }

    /// Returns the canonical project identifier.
    #[must_use]
    pub const fn project_id(&self) -> &ProjectId {
        &self.project_id
    }

    /// Returns the immutable Registry snapshot identifier.
    #[must_use]
    pub const fn project_snapshot_id(&self) -> &ProjectSnapshotId {
        &self.project_snapshot_id
    }

    /// Returns the immutable task identifier.
    #[must_use]
    pub const fn task_id(&self) -> &TaskId {
        &self.task_id
    }

    /// Returns the canonical positive Task Spec revision.
    #[must_use]
    pub fn task_revision(&self) -> &str {
        &self.task_revision
    }

    /// Returns the Task Spec SHA-256 digest.
    #[must_use]
    pub const fn task_spec_digest(&self) -> &ContentDigest {
        &self.task_spec_digest
    }

    /// Returns the exact accounting currency.
    #[must_use]
    pub fn accounting_currency(&self) -> &str {
        &self.accounting_currency
    }
}

/// Exact full Task Ledger stream head shared across owner boundaries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskLedgerStreamHead {
    version: ContractVersion,
    producer_id: String,
    producer_version: String,
    runtime: RuntimeKind,
    identity: TaskLedgerStreamIdentity,
    stream_id: ContentDigest,
    sequence: u64,
    last_event_digest: ContentDigest,
    resource_revision: u64,
    resource_projection_digest: ContentDigest,
    head_digest: ContentDigest,
}

impl TaskLedgerStreamHead {
    /// Constructs one validated full Task Ledger stream head.
    ///
    /// # Errors
    ///
    /// Rejects unknown producer/version, a zero stream/head digest, or
    /// inconsistent zero/non-zero event and resource positions.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        version: u16,
        producer_id: impl Into<String>,
        producer_version: impl Into<String>,
        runtime: RuntimeKind,
        identity: TaskLedgerStreamIdentity,
        stream_id: ContentDigest,
        sequence: u64,
        last_event_digest: ContentDigest,
        resource_revision: u64,
        resource_projection_digest: ContentDigest,
        head_digest: ContentDigest,
    ) -> Result<Self, ContractError> {
        let version = ContractVersion::new(version)?;
        let producer_id = producer_id.into();
        validate_task_ledger_producer(&producer_id)?;
        let producer_version = producer_version.into();
        validate_task_ledger_producer_version(&producer_version)?;
        let stream_zero = is_zero_digest(&stream_id);
        let event_zero = is_zero_digest(&last_event_digest);
        let resource_zero = is_zero_digest(&resource_projection_digest);
        let head_zero = is_zero_digest(&head_digest);
        let valid_position = if sequence == 0 {
            event_zero && resource_revision == 0 && resource_zero
        } else {
            !event_zero
                && resource_revision <= sequence
                && ((resource_revision == 0 && resource_zero)
                    || (resource_revision > 0 && !resource_zero))
        };
        if stream_zero || head_zero || !valid_position {
            return Err(ContractError::InvalidTaskLedgerHead);
        }
        Ok(Self {
            version,
            producer_id,
            producer_version,
            runtime,
            identity,
            stream_id,
            sequence,
            last_event_digest,
            resource_revision,
            resource_projection_digest,
            head_digest,
        })
    }

    /// Returns the shared contract version.
    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version.get()
    }

    /// Returns the fixed semantic producer identity.
    #[must_use]
    pub fn producer_id(&self) -> &str {
        &self.producer_id
    }

    /// Returns the fixed producer semantic version.
    #[must_use]
    pub fn producer_version(&self) -> &str {
        &self.producer_version
    }

    /// Returns fake/live runtime identity.
    #[must_use]
    pub const fn runtime(&self) -> RuntimeKind {
        self.runtime
    }

    /// Returns the complete task-stream identity.
    #[must_use]
    pub const fn identity(&self) -> &TaskLedgerStreamIdentity {
        &self.identity
    }

    /// Returns the domain-separated task-stream identifier.
    #[must_use]
    pub const fn stream_id(&self) -> &ContentDigest {
        &self.stream_id
    }

    /// Returns the current event sequence.
    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Returns the current last-event digest or the zero digest.
    #[must_use]
    pub const fn last_event_digest(&self) -> &ContentDigest {
        &self.last_event_digest
    }

    /// Returns the current resource-projection revision.
    #[must_use]
    pub const fn resource_revision(&self) -> u64 {
        self.resource_revision
    }

    /// Returns the current resource-projection digest or the zero digest.
    #[must_use]
    pub const fn resource_projection_digest(&self) -> &ContentDigest {
        &self.resource_projection_digest
    }

    /// Returns the digest of the complete full head.
    #[must_use]
    pub const fn head_digest(&self) -> &ContentDigest {
        &self.head_digest
    }

    /// Returns true only for the exact zero-position head.
    #[must_use]
    pub const fn is_zero(&self) -> bool {
        self.sequence == 0
    }
}

/// Current resource counters derived by Task Ledger replay.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceCounters {
    active_agents: u64,
    active_implementers: u64,
    elapsed_seconds: u64,
    attempt_number: u64,
    used_model_calls: u64,
    used_external_cost: String,
}

impl ResourceCounters {
    /// Constructs checked current resource counters.
    ///
    /// # Errors
    ///
    /// Rejects more active Implementers than agents or a non-canonical cost.
    pub fn new(
        active_agents: u64,
        active_implementers: u64,
        elapsed_seconds: u64,
        attempt_number: u64,
        used_model_calls: u64,
        used_external_cost: impl Into<String>,
    ) -> Result<Self, ContractError> {
        let used_external_cost = used_external_cost.into();
        if active_implementers > active_agents || !canonical_decimal(&used_external_cost) {
            return Err(ContractError::InvalidResourceUsage);
        }
        Ok(Self {
            active_agents,
            active_implementers,
            elapsed_seconds,
            attempt_number,
            used_model_calls,
            used_external_cost,
        })
    }

    /// Returns the active agent gauge.
    #[must_use]
    pub const fn active_agents(&self) -> u64 {
        self.active_agents
    }

    /// Returns the active Implementer gauge.
    #[must_use]
    pub const fn active_implementers(&self) -> u64 {
        self.active_implementers
    }

    /// Returns elapsed task seconds.
    #[must_use]
    pub const fn elapsed_seconds(&self) -> u64 {
        self.elapsed_seconds
    }

    /// Returns the current attempt number.
    #[must_use]
    pub const fn attempt_number(&self) -> u64 {
        self.attempt_number
    }

    /// Returns consumed model calls.
    #[must_use]
    pub const fn used_model_calls(&self) -> u64 {
        self.used_model_calls
    }

    /// Returns the canonical consumed external cost.
    #[must_use]
    pub fn used_external_cost(&self) -> &str {
        &self.used_external_cost
    }
}

/// Resource increments requested by one exact effect claim.
#[allow(clippy::struct_field_names)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceRequest {
    requested_agents: u64,
    requested_implementers: u64,
    requested_duration_seconds: u64,
    requested_attempts: u64,
    requested_model_calls: u64,
    requested_external_cost: Option<String>,
}

impl ResourceRequest {
    /// Constructs checked requested resource increments.
    ///
    /// # Errors
    ///
    /// Rejects more requested Implementers than agents or a non-canonical
    /// known external cost.
    pub fn new(
        requested_agents: u64,
        requested_implementers: u64,
        requested_duration_seconds: u64,
        requested_attempts: u64,
        requested_model_calls: u64,
        requested_external_cost: Option<impl Into<String>>,
    ) -> Result<Self, ContractError> {
        let requested_external_cost = requested_external_cost.map(Into::into);
        if requested_implementers > requested_agents
            || requested_external_cost
                .as_deref()
                .is_some_and(|value| !canonical_decimal(value))
        {
            return Err(ContractError::InvalidResourceUsage);
        }
        Ok(Self {
            requested_agents,
            requested_implementers,
            requested_duration_seconds,
            requested_attempts,
            requested_model_calls,
            requested_external_cost,
        })
    }

    /// Returns the requested agent increment.
    #[must_use]
    pub const fn requested_agents(&self) -> u64 {
        self.requested_agents
    }

    /// Returns the requested Implementer increment.
    #[must_use]
    pub const fn requested_implementers(&self) -> u64 {
        self.requested_implementers
    }

    /// Returns the requested duration increment.
    #[must_use]
    pub const fn requested_duration_seconds(&self) -> u64 {
        self.requested_duration_seconds
    }

    /// Returns the requested attempt increment.
    #[must_use]
    pub const fn requested_attempts(&self) -> u64 {
        self.requested_attempts
    }

    /// Returns the requested model-call increment.
    #[must_use]
    pub const fn requested_model_calls(&self) -> u64 {
        self.requested_model_calls
    }

    /// Returns the canonical requested cost, or `None` when unknown.
    #[must_use]
    pub fn requested_external_cost(&self) -> Option<&str> {
        self.requested_external_cost.as_deref()
    }
}

/// Immutable Task-Ledger-owned resource observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskLedgerResourceReceipt {
    version: ContractVersion,
    producer_id: String,
    producer_version: String,
    runtime: RuntimeKind,
    stream_head: TaskLedgerStreamHead,
    observation_revision: u64,
    effect_claim_id: String,
    effect_subject_digest: ContentDigest,
    counters: ResourceCounters,
    request: ResourceRequest,
    accounting_currency: String,
    observation_digest: ContentDigest,
    receipt_digest: ContentDigest,
}

impl TaskLedgerResourceReceipt {
    /// Constructs one immutable Task Ledger resource observation receipt.
    ///
    /// # Errors
    ///
    /// Rejects unknown producer/version, runtime or currency disagreement,
    /// zero revision/digests, or an invalid effect-claim identifier.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        version: u16,
        producer_id: impl Into<String>,
        producer_version: impl Into<String>,
        runtime: RuntimeKind,
        stream_head: TaskLedgerStreamHead,
        observation_revision: u64,
        effect_claim_id: impl Into<String>,
        effect_subject_digest: ContentDigest,
        counters: ResourceCounters,
        request: ResourceRequest,
        accounting_currency: impl Into<String>,
        observation_digest: ContentDigest,
        receipt_digest: ContentDigest,
    ) -> Result<Self, ContractError> {
        let version = ContractVersion::new(version)?;
        let producer_id = producer_id.into();
        validate_task_ledger_producer(&producer_id)?;
        let producer_version = producer_version.into();
        validate_task_ledger_producer_version(&producer_version)?;
        if runtime != stream_head.runtime() {
            return Err(ContractError::TaskLedgerRuntimeMismatch);
        }
        if observation_revision == 0 {
            return Err(ContractError::ZeroObservationRevision);
        }
        let effect_claim_id = effect_claim_id.into();
        if !valid_task_ledger_identifier(&effect_claim_id) {
            return Err(ContractError::InvalidTaskLedgerIdentifier {
                field: "effect_claim_id",
            });
        }
        let accounting_currency = accounting_currency.into();
        if !valid_accounting_currency(&accounting_currency)
            || accounting_currency != stream_head.identity().accounting_currency()
        {
            return Err(ContractError::InvalidAccountingCurrency);
        }
        if is_zero_digest(&effect_subject_digest)
            || is_zero_digest(&observation_digest)
            || is_zero_digest(&receipt_digest)
        {
            return Err(ContractError::InvalidResourceUsage);
        }
        Ok(Self {
            version,
            producer_id,
            producer_version,
            runtime,
            stream_head,
            observation_revision,
            effect_claim_id,
            effect_subject_digest,
            counters,
            request,
            accounting_currency,
            observation_digest,
            receipt_digest,
        })
    }

    /// Returns the shared contract version.
    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version.get()
    }

    /// Returns the fixed producer identity.
    #[must_use]
    pub fn producer_id(&self) -> &str {
        &self.producer_id
    }

    /// Returns the fixed producer semantic version.
    #[must_use]
    pub fn producer_version(&self) -> &str {
        &self.producer_version
    }

    /// Returns fake/live owner identity.
    #[must_use]
    pub const fn runtime(&self) -> RuntimeKind {
        self.runtime
    }

    /// Returns the full observed stream head.
    #[must_use]
    pub const fn stream_head(&self) -> &TaskLedgerStreamHead {
        &self.stream_head
    }

    /// Returns the non-zero observation revision.
    #[must_use]
    pub const fn observation_revision(&self) -> u64 {
        self.observation_revision
    }

    /// Returns the exact effect-claim identifier.
    #[must_use]
    pub fn effect_claim_id(&self) -> &str {
        &self.effect_claim_id
    }

    /// Returns the exact effect-subject digest.
    #[must_use]
    pub const fn effect_subject_digest(&self) -> &ContentDigest {
        &self.effect_subject_digest
    }

    /// Returns current replay-derived counters.
    #[must_use]
    pub const fn counters(&self) -> &ResourceCounters {
        &self.counters
    }

    /// Returns the exact requested resource increments.
    #[must_use]
    pub const fn request(&self) -> &ResourceRequest {
        &self.request
    }

    /// Returns the exact accounting currency.
    #[must_use]
    pub fn accounting_currency(&self) -> &str {
        &self.accounting_currency
    }

    /// Returns the complete observation-subject digest.
    #[must_use]
    pub const fn observation_digest(&self) -> &ContentDigest {
        &self.observation_digest
    }

    /// Returns the complete immutable receipt digest.
    #[must_use]
    pub const fn receipt_digest(&self) -> &ContentDigest {
        &self.receipt_digest
    }

    /// Projects every security-relevant field into a structural head.
    ///
    /// This projection does not prove independent currentness.
    #[must_use]
    pub fn head(&self) -> TaskLedgerResourceHead {
        TaskLedgerResourceHead {
            producer_id: self.producer_id.clone(),
            producer_version: self.producer_version.clone(),
            runtime: self.runtime,
            stream_head: self.stream_head.clone(),
            observation_revision: self.observation_revision,
            effect_claim_id: self.effect_claim_id.clone(),
            effect_subject_digest: self.effect_subject_digest.clone(),
            counters: self.counters.clone(),
            request: self.request.clone(),
            accounting_currency: self.accounting_currency.clone(),
            observation_digest: self.observation_digest.clone(),
            receipt_digest: self.receipt_digest.clone(),
        }
    }
}

/// Full resource observation head returned by an independent Ledger lookup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskLedgerResourceHead {
    producer_id: String,
    producer_version: String,
    runtime: RuntimeKind,
    stream_head: TaskLedgerStreamHead,
    observation_revision: u64,
    effect_claim_id: String,
    effect_subject_digest: ContentDigest,
    counters: ResourceCounters,
    request: ResourceRequest,
    accounting_currency: String,
    observation_digest: ContentDigest,
    receipt_digest: ContentDigest,
}

impl TaskLedgerResourceHead {
    /// Returns the fixed producer identity.
    #[must_use]
    pub fn producer_id(&self) -> &str {
        &self.producer_id
    }

    /// Returns the fixed producer semantic version.
    #[must_use]
    pub fn producer_version(&self) -> &str {
        &self.producer_version
    }

    /// Returns fake/live owner identity.
    #[must_use]
    pub const fn runtime(&self) -> RuntimeKind {
        self.runtime
    }

    /// Returns the full current stream head.
    #[must_use]
    pub const fn stream_head(&self) -> &TaskLedgerStreamHead {
        &self.stream_head
    }

    /// Returns the current observation revision.
    #[must_use]
    pub const fn observation_revision(&self) -> u64 {
        self.observation_revision
    }

    /// Returns the exact effect-claim identifier.
    #[must_use]
    pub fn effect_claim_id(&self) -> &str {
        &self.effect_claim_id
    }

    /// Returns the exact effect-subject digest.
    #[must_use]
    pub const fn effect_subject_digest(&self) -> &ContentDigest {
        &self.effect_subject_digest
    }

    /// Returns current replay-derived counters.
    #[must_use]
    pub const fn counters(&self) -> &ResourceCounters {
        &self.counters
    }

    /// Returns the exact requested resource increments.
    #[must_use]
    pub const fn request(&self) -> &ResourceRequest {
        &self.request
    }

    /// Returns the exact accounting currency.
    #[must_use]
    pub fn accounting_currency(&self) -> &str {
        &self.accounting_currency
    }

    /// Returns the complete observation-subject digest.
    #[must_use]
    pub const fn observation_digest(&self) -> &ContentDigest {
        &self.observation_digest
    }

    /// Returns the exact receipt digest.
    #[must_use]
    pub const fn receipt_digest(&self) -> &ContentDigest {
        &self.receipt_digest
    }
}

fn validate_task_ledger_producer(value: &str) -> Result<(), ContractError> {
    if value.trim().is_empty() {
        return Err(ContractError::EmptyIdentifier {
            field: "task_ledger_producer_id",
        });
    }
    if value != TASK_LEDGER_PRODUCER_ID {
        return Err(ContractError::UnsupportedTaskLedgerProducer);
    }
    Ok(())
}

fn validate_task_ledger_producer_version(value: &str) -> Result<(), ContractError> {
    if value.trim().is_empty() {
        return Err(ContractError::EmptyIdentifier {
            field: "task_ledger_producer_version",
        });
    }
    if value != TASK_LEDGER_PRODUCER_VERSION {
        return Err(ContractError::UnsupportedTaskLedgerProducerVersion);
    }
    Ok(())
}

fn validate_writer_lease_producer(value: &str) -> Result<(), ContractError> {
    if value.trim().is_empty() {
        return Err(ContractError::EmptyIdentifier {
            field: "writer_lease_producer_id",
        });
    }
    if value != WRITER_LEASE_PRODUCER_ID {
        return Err(ContractError::UnsupportedWriterLeaseProducer);
    }
    Ok(())
}

fn validate_writer_lease_producer_version(value: &str) -> Result<(), ContractError> {
    if value.trim().is_empty() {
        return Err(ContractError::EmptyIdentifier {
            field: "writer_lease_producer_version",
        });
    }
    if value != WRITER_LEASE_PRODUCER_VERSION {
        return Err(ContractError::UnsupportedWriterLeaseProducerVersion);
    }
    Ok(())
}

fn validate_approval_verifier_producer(value: &str) -> Result<(), ContractError> {
    if value.trim().is_empty() {
        return Err(ContractError::EmptyIdentifier {
            field: "approval_verifier_producer_id",
        });
    }
    if value != APPROVAL_VERIFIER_PRODUCER_ID {
        return Err(ContractError::UnsupportedApprovalVerifierProducer);
    }
    Ok(())
}

fn validate_approval_verifier_producer_version(value: &str) -> Result<(), ContractError> {
    if value.trim().is_empty() {
        return Err(ContractError::EmptyIdentifier {
            field: "approval_verifier_producer_version",
        });
    }
    if value != APPROVAL_VERIFIER_PRODUCER_VERSION {
        return Err(ContractError::UnsupportedApprovalVerifierProducerVersion);
    }
    Ok(())
}

fn valid_approval_authority_pair(
    authority: ApprovalAuthority,
    origin: ApprovalOrigin,
    lane: ApprovalLane,
) -> bool {
    matches!(
        (authority, origin, lane),
        (
            ApprovalAuthority::ResponsibleUser,
            ApprovalOrigin::OsAuthenticatedUser,
            ApprovalLane::Normal
        ) | (
            ApprovalAuthority::ProtectedGuardian,
            ApprovalOrigin::GuardianTrustRoot,
            ApprovalLane::Protected
        )
    )
}

#[allow(clippy::too_many_arguments)]
fn validate_approval_authority_fields(
    identity: &ApprovalIdentity,
    status: ApprovalStatus,
    nonce_id: &str,
    nonce_commitment: &ContentDigest,
    issued_at: &str,
    expires_at: &str,
    subject_digest: &ContentDigest,
    challenge_digest: &ContentDigest,
    authenticator_id: &str,
    key_id: &str,
    proof_digest: &ContentDigest,
    evidence_digest: &ContentDigest,
    review_set_digest: Option<&ContentDigest>,
    receipt_digest: &ContentDigest,
) -> Result<(), ContractError> {
    let status_matches_lane = match identity.lane() {
        ApprovalLane::Normal => matches!(
            status,
            ApprovalStatus::Available | ApprovalStatus::ClaimedNormal | ApprovalStatus::Revoked
        ),
        ApprovalLane::Protected => matches!(
            status,
            ApprovalStatus::ProtectedPendingClaim | ApprovalStatus::Revoked
        ),
    };
    if !status_matches_lane {
        return Err(ContractError::InvalidApprovalReceipt { field: "status" });
    }
    for (field, value) in [
        ("nonce_id", nonce_id),
        ("authenticator_id", authenticator_id),
        ("key_id", key_id),
    ] {
        if !valid_approval_identifier(value) {
            return Err(ContractError::InvalidApprovalReceipt { field });
        }
    }
    for (field, value) in [("issued_at", issued_at), ("expires_at", expires_at)] {
        if !valid_approval_receipt_text(value) {
            return Err(ContractError::InvalidApprovalReceipt { field });
        }
    }
    for (field, value) in [
        ("nonce_commitment", nonce_commitment),
        ("subject_digest", subject_digest),
        ("challenge_digest", challenge_digest),
        ("proof_digest", proof_digest),
        ("evidence_digest", evidence_digest),
        ("receipt_digest", receipt_digest),
    ] {
        if is_zero_digest(value) {
            return Err(ContractError::InvalidApprovalReceipt { field });
        }
    }
    if review_set_digest.is_some_and(is_zero_digest) {
        return Err(ContractError::InvalidApprovalReceipt {
            field: "review_set_digest",
        });
    }
    Ok(())
}

fn canonical_positive_u64(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('0')
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && value.parse::<u64>().is_ok_and(|parsed| parsed > 0)
}

fn valid_approval_identifier(value: &str) -> bool {
    (1..=128).contains(&value.len())
        && value.trim() == value
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'/' | b'-')
        })
}

fn valid_approval_text(value: &str) -> bool {
    (1..=256).contains(&value.len()) && value.trim() == value && !value.contains('\0')
}

fn valid_approval_receipt_text(value: &str) -> bool {
    (1..=128).contains(&value.len()) && value.trim() == value && !value.contains('\0')
}

fn valid_accounting_currency(value: &str) -> bool {
    value.len() == 3 && value.bytes().all(|byte| byte.is_ascii_uppercase())
}

fn valid_task_ledger_identifier(value: &str) -> bool {
    (1..=128).contains(&value.len())
        && value.trim() == value
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'/' | b'-')
        })
}

fn valid_writer_lease_identifier(value: &str) -> bool {
    (1..=128).contains(&value.len())
        && value.trim() == value
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'/' | b'-')
        })
}

fn valid_writer_lease_receipt_text(value: &str) -> bool {
    (1..=128).contains(&value.len()) && value.trim() == value && !value.contains('\0')
}

fn canonical_decimal(value: &str) -> bool {
    const MAX_BYTES: usize = 256;
    const MAX_INTEGER_DIGITS: usize = 127;
    const MAX_SCALE: usize = 128;
    let (integer, fraction) = match value.split_once('.') {
        Some((integer, fraction)) => (integer, Some(fraction)),
        None => (value, None),
    };
    !value.is_empty()
        && value.len() <= MAX_BYTES
        && integer.len() <= MAX_INTEGER_DIGITS
        && !integer.is_empty()
        && integer.bytes().all(|byte| byte.is_ascii_digit())
        && (integer == "0" || !integer.starts_with('0'))
        && fraction.is_none_or(|fraction| {
            !fraction.is_empty()
                && fraction.len() <= MAX_SCALE
                && fraction.bytes().all(|byte| byte.is_ascii_digit())
                && !fraction.ends_with('0')
        })
}

fn is_zero_digest(value: &ContentDigest) -> bool {
    value.as_str().bytes().all(|byte| byte == b'0')
}

mod artifact_contracts {
    use super::{
        ARTIFACT_READ_CLOSURE_PRODUCER_ID, ARTIFACT_READ_CLOSURE_PRODUCER_VERSION,
        ARTIFACT_STORE_PRODUCER_ID, ARTIFACT_STORE_PRODUCER_VERSION, ArtifactByteLength,
        ArtifactCounter, ArtifactGeneration, ArtifactRevision, AttemptId, ContentDigest,
        ContractError, ContractVersion, DaemonEpoch, ProjectId, RequestId, RuntimeAdmissionMode,
        RuntimeKind, SubjectBinding, TASK_LEDGER_PRODUCER_ID, TASK_LEDGER_PRODUCER_VERSION, TaskId,
        is_zero_digest,
    };

    const MAX_BUNDLE_ENTRIES: u64 = 100_000;
    const MAX_BUNDLE_DEPTH: u64 = 64;
    const MAX_ARTIFACT_TEXT_BYTES: usize = 256;

    /// A project-scoped SHA-256 content-object key.
    #[derive(Clone, Debug, Eq, Hash, PartialEq)]
    pub struct ArtifactObjectKey {
        project_id: ProjectId,
        content_digest: ContentDigest,
    }

    impl ArtifactObjectKey {
        /// Constructs a key whose project namespace cannot be omitted.
        #[must_use]
        pub const fn new(project_id: ProjectId, content_digest: ContentDigest) -> Self {
            Self {
                project_id,
                content_digest,
            }
        }

        /// Returns the owning project namespace.
        #[must_use]
        pub const fn project_id(&self) -> &ProjectId {
            &self.project_id
        }

        /// Returns the exact byte-content digest.
        #[must_use]
        pub const fn content_digest(&self) -> &ContentDigest {
            &self.content_digest
        }

        /// Returns the only supported content algorithm.
        #[must_use]
        pub const fn algorithm(&self) -> &'static str {
            "sha256"
        }
    }

    /// One physical generation of a project-scoped content object.
    #[derive(Clone, Debug, Eq, Hash, PartialEq)]
    pub struct ArtifactObjectIdentity {
        key: ArtifactObjectKey,
        generation: ArtifactGeneration,
    }

    impl ArtifactObjectIdentity {
        /// Constructs one non-zero generation of an object key.
        #[must_use]
        pub const fn new(key: ArtifactObjectKey, generation: ArtifactGeneration) -> Self {
            Self { key, generation }
        }

        /// Returns the project-scoped logical key.
        #[must_use]
        pub const fn key(&self) -> &ArtifactObjectKey {
            &self.key
        }

        /// Returns the exact non-wrapping physical generation.
        #[must_use]
        pub const fn generation(&self) -> ArtifactGeneration {
            self.generation
        }
    }

    /// Bounds committed by a bundle reference.
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub struct ArtifactBundleBounds {
        entry_count: ArtifactCounter,
        max_depth: ArtifactCounter,
        total_declared_bytes: ArtifactByteLength,
    }

    impl ArtifactBundleBounds {
        /// Constructs bundle bounds within the frozen 1.0 hard maxima.
        ///
        /// # Errors
        ///
        /// Rejects entry counts above 100,000 or depth above 64.
        pub fn new(
            entry_count: ArtifactCounter,
            max_depth: ArtifactCounter,
            total_declared_bytes: ArtifactByteLength,
        ) -> Result<Self, ContractError> {
            if entry_count.get() > MAX_BUNDLE_ENTRIES {
                return Err(ContractError::InvalidArtifactValue {
                    field: "bundle_entry_count",
                });
            }
            if max_depth.get() > MAX_BUNDLE_DEPTH {
                return Err(ContractError::InvalidArtifactValue {
                    field: "bundle_max_depth",
                });
            }
            if (entry_count.get() == 0) != (max_depth.get() == 0) {
                return Err(ContractError::InvalidArtifactValue {
                    field: "bundle_shape",
                });
            }
            Ok(Self {
                entry_count,
                max_depth,
                total_declared_bytes,
            })
        }

        #[must_use]
        pub const fn entry_count(self) -> ArtifactCounter {
            self.entry_count
        }

        #[must_use]
        pub const fn max_depth(self) -> ArtifactCounter {
            self.max_depth
        }

        #[must_use]
        pub const fn total_declared_bytes(self) -> ArtifactByteLength {
            self.total_declared_bytes
        }
    }

    /// Closed source-use purpose carried by an artifact reference.
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub enum ArtifactPurpose {
        GraphifyGraph,
        HermesCandidate,
        CodexEvidence,
        ReviewBundle,
        CodebaseMemorySource,
        UpgradeCandidate,
        TaskOutput,
    }

    impl ArtifactPurpose {
        #[must_use]
        pub const fn as_str(self) -> &'static str {
            match self {
                Self::GraphifyGraph => "GRAPHIFY_GRAPH",
                Self::HermesCandidate => "HERMES_CANDIDATE",
                Self::CodexEvidence => "CODEX_EVIDENCE",
                Self::ReviewBundle => "REVIEW_BUNDLE",
                Self::CodebaseMemorySource => "CODEBASE_MEMORY_SOURCE",
                Self::UpgradeCandidate => "UPGRADE_CANDIDATE",
                Self::TaskOutput => "TASK_OUTPUT",
            }
        }
    }

    /// Availability of one artifact object generation.
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub enum ArtifactAvailability {
        Available,
        DeleteClaimed,
        Deleted,
        ReconciliationRequired,
    }

    impl ArtifactAvailability {
        #[must_use]
        pub const fn as_str(self) -> &'static str {
            match self {
                Self::Available => "AVAILABLE",
                Self::DeleteClaimed => "DELETE_CLAIMED",
                Self::Deleted => "DELETED",
                Self::ReconciliationRequired => "RECONCILIATION_REQUIRED",
            }
        }
    }

    /// Delete-claim projection retained beside availability.
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub enum ArtifactDeleteStatus {
        NotClaimed,
        Claimed,
        VerifiedNoEffect,
        VerifiedDeleted,
        ReconciliationRequired,
    }

    impl ArtifactDeleteStatus {
        #[must_use]
        pub const fn as_str(self) -> &'static str {
            match self {
                Self::NotClaimed => "NOT_CLAIMED",
                Self::Claimed => "CLAIMED",
                Self::VerifiedNoEffect => "VERIFIED_NO_EFFECT",
                Self::VerifiedDeleted => "VERIFIED_DELETED",
                Self::ReconciliationRequired => "RECONCILIATION_REQUIRED",
            }
        }
    }

    /// Lifecycle of an immutable artifact reference.
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub enum ArtifactReferenceStatus {
        Active,
        Released,
    }

    impl ArtifactReferenceStatus {
        #[must_use]
        pub const fn as_str(self) -> &'static str {
            match self {
                Self::Active => "ACTIVE",
                Self::Released => "RELEASED",
            }
        }
    }

    /// Lifecycle of an object-scoped active-read claim.
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub enum ArtifactReadStatus {
        Active,
        ExpiredSuspect,
        Released,
    }

    impl ArtifactReadStatus {
        #[must_use]
        pub const fn as_str(self) -> &'static str {
            match self {
                Self::Active => "ACTIVE",
                Self::ExpiredSuspect => "EXPIRED_SUSPECT",
                Self::Released => "RELEASED",
            }
        }
    }

    /// Closed owner family allowed to appear in typed artifact authority.
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub enum ArtifactAuthorityOwnerKind {
        TaskLedger,
        CodebaseMemory,
        ReviewRuntime,
        Guardian,
        ArtifactStore,
    }

    impl ArtifactAuthorityOwnerKind {
        #[must_use]
        pub const fn as_str(self) -> &'static str {
            match self {
                Self::TaskLedger => "TASK_LEDGER",
                Self::CodebaseMemory => "CODEBASE_MEMORY",
                Self::ReviewRuntime => "REVIEW_RUNTIME",
                Self::Guardian => "GUARDIAN",
                Self::ArtifactStore => "ARTIFACT_STORE",
            }
        }

        #[must_use]
        pub const fn producer_id(self) -> &'static str {
            match self {
                Self::TaskLedger => TASK_LEDGER_PRODUCER_ID,
                Self::CodebaseMemory => "lattice-codebase-memory",
                Self::ReviewRuntime => "lattice-review-runtime",
                Self::Guardian => "lattice-guardian",
                Self::ArtifactStore => ARTIFACT_STORE_PRODUCER_ID,
            }
        }

        #[must_use]
        pub const fn producer_version(self) -> &'static str {
            match self {
                Self::TaskLedger => TASK_LEDGER_PRODUCER_VERSION,
                Self::CodebaseMemory
                | Self::ReviewRuntime
                | Self::Guardian
                | Self::ArtifactStore => "1.0",
            }
        }
    }

    /// Availability of a typed operation authority record.
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub enum ArtifactAuthorityStatus {
        Available,
        Consumed,
        Revoked,
    }

    impl ArtifactAuthorityStatus {
        #[must_use]
        pub const fn as_str(self) -> &'static str {
            match self {
                Self::Available => "AVAILABLE",
                Self::Consumed => "CONSUMED",
                Self::Revoked => "REVOKED",
            }
        }
    }

    /// Complete source provenance bound into an immutable artifact reference.
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct ArtifactProvenance {
        source_producer_id: String,
        source_producer_version: String,
        source_runtime: RuntimeKind,
        producer_binary_digest: ContentDigest,
        adapter_id: String,
        adapter_version: String,
        adapter_binary_digest: ContentDigest,
        invocation_id: String,
        correlation_id: String,
        run_id: String,
        sequence: ArtifactCounter,
        produced_at: String,
        payload_digest: ContentDigest,
        capability_id: String,
        input_set_digest: ContentDigest,
        configuration_digest: ContentDigest,
        evidence_digest: ContentDigest,
        registry_authority_receipt_digest: ContentDigest,
        registry_current_head_digest: ContentDigest,
        effect_claim_id: String,
        effect_claim_digest: ContentDigest,
        daemon_instance_id: String,
        daemon_epoch: DaemonEpoch,
        runtime_admission: RuntimeAdmissionMode,
        capability_owner_receipt_digest: ContentDigest,
        capability_owner_current_head_digest: ContentDigest,
        limit_snapshot_digest: ContentDigest,
    }

    impl ArtifactProvenance {
        /// Constructs a complete immutable provenance envelope without
        /// authenticating its external claims.
        ///
        /// # Errors
        ///
        /// Rejects missing, over-bound, path-like identity fields, malformed
        /// canonical time text, or zero authority/evidence digests.
        #[allow(clippy::too_many_arguments)]
        pub fn new(
            source_producer_id: impl Into<String>,
            source_producer_version: impl Into<String>,
            source_runtime: RuntimeKind,
            producer_binary_digest: ContentDigest,
            adapter_id: impl Into<String>,
            adapter_version: impl Into<String>,
            adapter_binary_digest: ContentDigest,
            invocation_id: impl Into<String>,
            correlation_id: impl Into<String>,
            run_id: impl Into<String>,
            sequence: ArtifactCounter,
            produced_at: impl Into<String>,
            payload_digest: ContentDigest,
            capability_id: impl Into<String>,
            input_set_digest: ContentDigest,
            configuration_digest: ContentDigest,
            evidence_digest: ContentDigest,
            registry_authority_receipt_digest: ContentDigest,
            registry_current_head_digest: ContentDigest,
            effect_claim_id: impl Into<String>,
            effect_claim_digest: ContentDigest,
            daemon_instance_id: impl Into<String>,
            daemon_epoch: DaemonEpoch,
            runtime_admission: RuntimeAdmissionMode,
            capability_owner_receipt_digest: ContentDigest,
            capability_owner_current_head_digest: ContentDigest,
            limit_snapshot_digest: ContentDigest,
        ) -> Result<Self, ContractError> {
            let source_producer_id = source_producer_id.into();
            let source_producer_version = source_producer_version.into();
            let adapter_id = adapter_id.into();
            let adapter_version = adapter_version.into();
            let invocation_id = invocation_id.into();
            let correlation_id = correlation_id.into();
            let run_id = run_id.into();
            let produced_at = produced_at.into();
            let capability_id = capability_id.into();
            let effect_claim_id = effect_claim_id.into();
            let daemon_instance_id = daemon_instance_id.into();
            for (field, value) in [
                ("source_producer_id", source_producer_id.as_str()),
                ("source_producer_version", source_producer_version.as_str()),
                ("adapter_id", adapter_id.as_str()),
                ("adapter_version", adapter_version.as_str()),
                ("invocation_id", invocation_id.as_str()),
                ("correlation_id", correlation_id.as_str()),
                ("run_id", run_id.as_str()),
                ("capability_id", capability_id.as_str()),
                ("effect_claim_id", effect_claim_id.as_str()),
                ("daemon_instance_id", daemon_instance_id.as_str()),
            ] {
                validate_artifact_identifier(field, value)?;
            }
            validate_artifact_time("produced_at", &produced_at)?;
            for (field, value) in [
                ("producer_binary_digest", &producer_binary_digest),
                ("adapter_binary_digest", &adapter_binary_digest),
                ("payload_digest", &payload_digest),
                ("input_set_digest", &input_set_digest),
                ("configuration_digest", &configuration_digest),
                ("evidence_digest", &evidence_digest),
                (
                    "registry_authority_receipt_digest",
                    &registry_authority_receipt_digest,
                ),
                (
                    "registry_current_head_digest",
                    &registry_current_head_digest,
                ),
                ("effect_claim_digest", &effect_claim_digest),
                (
                    "capability_owner_receipt_digest",
                    &capability_owner_receipt_digest,
                ),
                (
                    "capability_owner_current_head_digest",
                    &capability_owner_current_head_digest,
                ),
                ("limit_snapshot_digest", &limit_snapshot_digest),
            ] {
                validate_artifact_digest(field, value)?;
            }
            Ok(Self {
                source_producer_id,
                source_producer_version,
                source_runtime,
                producer_binary_digest,
                adapter_id,
                adapter_version,
                adapter_binary_digest,
                invocation_id,
                correlation_id,
                run_id,
                sequence,
                produced_at,
                payload_digest,
                capability_id,
                input_set_digest,
                configuration_digest,
                evidence_digest,
                registry_authority_receipt_digest,
                registry_current_head_digest,
                effect_claim_id,
                effect_claim_digest,
                daemon_instance_id,
                daemon_epoch,
                runtime_admission,
                capability_owner_receipt_digest,
                capability_owner_current_head_digest,
                limit_snapshot_digest,
            })
        }

        #[must_use]
        pub fn source_producer_id(&self) -> &str {
            &self.source_producer_id
        }
        #[must_use]
        pub fn source_producer_version(&self) -> &str {
            &self.source_producer_version
        }
        #[must_use]
        pub const fn source_runtime(&self) -> RuntimeKind {
            self.source_runtime
        }
        #[must_use]
        pub const fn producer_binary_digest(&self) -> &ContentDigest {
            &self.producer_binary_digest
        }
        #[must_use]
        pub fn adapter_id(&self) -> &str {
            &self.adapter_id
        }
        #[must_use]
        pub fn adapter_version(&self) -> &str {
            &self.adapter_version
        }
        #[must_use]
        pub const fn adapter_binary_digest(&self) -> &ContentDigest {
            &self.adapter_binary_digest
        }
        #[must_use]
        pub fn invocation_id(&self) -> &str {
            &self.invocation_id
        }
        #[must_use]
        pub fn correlation_id(&self) -> &str {
            &self.correlation_id
        }
        #[must_use]
        pub fn run_id(&self) -> &str {
            &self.run_id
        }
        #[must_use]
        pub const fn sequence(&self) -> ArtifactCounter {
            self.sequence
        }
        #[must_use]
        pub fn produced_at(&self) -> &str {
            &self.produced_at
        }
        #[must_use]
        pub const fn payload_digest(&self) -> &ContentDigest {
            &self.payload_digest
        }
        #[must_use]
        pub fn capability_id(&self) -> &str {
            &self.capability_id
        }
        #[must_use]
        pub const fn input_set_digest(&self) -> &ContentDigest {
            &self.input_set_digest
        }
        #[must_use]
        pub const fn configuration_digest(&self) -> &ContentDigest {
            &self.configuration_digest
        }
        #[must_use]
        pub const fn evidence_digest(&self) -> &ContentDigest {
            &self.evidence_digest
        }
        #[must_use]
        pub const fn registry_authority_receipt_digest(&self) -> &ContentDigest {
            &self.registry_authority_receipt_digest
        }
        #[must_use]
        pub const fn registry_current_head_digest(&self) -> &ContentDigest {
            &self.registry_current_head_digest
        }
        #[must_use]
        pub fn effect_claim_id(&self) -> &str {
            &self.effect_claim_id
        }
        #[must_use]
        pub const fn effect_claim_digest(&self) -> &ContentDigest {
            &self.effect_claim_digest
        }
        #[must_use]
        pub fn daemon_instance_id(&self) -> &str {
            &self.daemon_instance_id
        }
        #[must_use]
        pub const fn daemon_epoch(&self) -> DaemonEpoch {
            self.daemon_epoch
        }
        #[must_use]
        pub const fn runtime_admission(&self) -> RuntimeAdmissionMode {
            self.runtime_admission
        }
        #[must_use]
        pub const fn capability_owner_receipt_digest(&self) -> &ContentDigest {
            &self.capability_owner_receipt_digest
        }
        #[must_use]
        pub const fn capability_owner_current_head_digest(&self) -> &ContentDigest {
            &self.capability_owner_current_head_digest
        }
        #[must_use]
        pub const fn limit_snapshot_digest(&self) -> &ContentDigest {
            &self.limit_snapshot_digest
        }
    }

    /// Typed reference-owner operation.
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub enum ArtifactReferenceAuthorityAction {
        PublishInitialReference,
        AddReference,
        ReleaseReference,
    }

    impl ArtifactReferenceAuthorityAction {
        #[must_use]
        pub const fn as_str(self) -> &'static str {
            match self {
                Self::PublishInitialReference => "PUBLISH_INITIAL_REFERENCE",
                Self::AddReference => "ADD_REFERENCE",
                Self::ReleaseReference => "RELEASE_REFERENCE",
            }
        }
    }

    /// Complete exact scope asserted by a reference-owner authority.
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct ArtifactReferenceAuthorityBinding {
        owner_kind: ArtifactAuthorityOwnerKind,
        producer_id: String,
        producer_version: String,
        runtime: RuntimeKind,
        owner_record_id: String,
        owner_revision: ArtifactRevision,
        status: ArtifactAuthorityStatus,
        action: ArtifactReferenceAuthorityAction,
        project_id: ProjectId,
        task_id: TaskId,
        object: ArtifactObjectIdentity,
        reference_id: String,
        observation_digest: ContentDigest,
    }

    impl ArtifactReferenceAuthorityBinding {
        /// Constructs an exact typed reference-owner authority binding.
        ///
        /// # Errors
        ///
        /// Rejects the Artifact Store as reference owner, unavailable owner
        /// state, malformed identifiers/digests, or project disagreement.
        #[allow(clippy::too_many_arguments)]
        pub fn new(
            owner_kind: ArtifactAuthorityOwnerKind,
            runtime: RuntimeKind,
            owner_record_id: impl Into<String>,
            owner_revision: ArtifactRevision,
            status: ArtifactAuthorityStatus,
            action: ArtifactReferenceAuthorityAction,
            project_id: ProjectId,
            task_id: TaskId,
            object: ArtifactObjectIdentity,
            reference_id: impl Into<String>,
            observation_digest: ContentDigest,
        ) -> Result<Self, ContractError> {
            if owner_kind == ArtifactAuthorityOwnerKind::ArtifactStore {
                return Err(ContractError::InvalidArtifactAuthority {
                    field: "reference_owner_kind",
                });
            }
            let owner_record_id = owner_record_id.into();
            let reference_id = reference_id.into();
            validate_artifact_identifier("owner_record_id", &owner_record_id)?;
            validate_artifact_identifier("task_id", task_id.as_str())?;
            validate_artifact_identifier("reference_id", &reference_id)?;
            validate_artifact_digest("authority_observation_digest", &observation_digest)?;
            if status != ArtifactAuthorityStatus::Available {
                return Err(ContractError::InvalidArtifactAuthority {
                    field: "reference_owner_status",
                });
            }
            if &project_id != object.key().project_id() {
                return Err(ContractError::InvalidArtifactAuthority {
                    field: "reference_project_scope",
                });
            }
            Ok(Self {
                owner_kind,
                producer_id: owner_kind.producer_id().to_owned(),
                producer_version: owner_kind.producer_version().to_owned(),
                runtime,
                owner_record_id,
                owner_revision,
                status,
                action,
                project_id,
                task_id,
                object,
                reference_id,
                observation_digest,
            })
        }

        #[must_use]
        pub const fn owner_kind(&self) -> ArtifactAuthorityOwnerKind {
            self.owner_kind
        }
        #[must_use]
        pub fn producer_id(&self) -> &str {
            &self.producer_id
        }
        #[must_use]
        pub fn producer_version(&self) -> &str {
            &self.producer_version
        }
        #[must_use]
        pub const fn runtime(&self) -> RuntimeKind {
            self.runtime
        }
        #[must_use]
        pub fn owner_record_id(&self) -> &str {
            &self.owner_record_id
        }
        #[must_use]
        pub const fn owner_revision(&self) -> ArtifactRevision {
            self.owner_revision
        }
        #[must_use]
        pub const fn status(&self) -> ArtifactAuthorityStatus {
            self.status
        }
        #[must_use]
        pub const fn action(&self) -> ArtifactReferenceAuthorityAction {
            self.action
        }
        #[must_use]
        pub const fn project_id(&self) -> &ProjectId {
            &self.project_id
        }
        #[must_use]
        pub const fn task_id(&self) -> &TaskId {
            &self.task_id
        }
        #[must_use]
        pub const fn object(&self) -> &ArtifactObjectIdentity {
            &self.object
        }
        #[must_use]
        pub fn reference_id(&self) -> &str {
            &self.reference_id
        }
        #[must_use]
        pub const fn observation_digest(&self) -> &ContentDigest {
            &self.observation_digest
        }
    }

    /// Typed read-owner operation.
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub enum ArtifactReadAuthorityAction {
        AcquireRead,
        ReleaseRead,
    }

    impl ArtifactReadAuthorityAction {
        #[must_use]
        pub const fn as_str(self) -> &'static str {
            match self {
                Self::AcquireRead => "ACQUIRE_READ",
                Self::ReleaseRead => "RELEASE_READ",
            }
        }
    }

    /// Complete exact scope asserted by an active-read owner.
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct ArtifactReadAuthorityBinding {
        owner_kind: ArtifactAuthorityOwnerKind,
        producer_id: String,
        producer_version: String,
        runtime: RuntimeKind,
        owner_record_id: String,
        owner_revision: ArtifactRevision,
        status: ArtifactAuthorityStatus,
        action: ArtifactReadAuthorityAction,
        project_id: ProjectId,
        task_id: TaskId,
        object: ArtifactObjectIdentity,
        read_claim_id: String,
        observation_digest: ContentDigest,
    }

    impl ArtifactReadAuthorityBinding {
        /// Constructs an exact typed active-read-owner authority binding.
        ///
        /// # Errors
        ///
        /// Rejects the Artifact Store as read owner, unavailable owner state,
        /// malformed identifiers/digests, or project disagreement.
        #[allow(clippy::too_many_arguments)]
        pub fn new(
            owner_kind: ArtifactAuthorityOwnerKind,
            runtime: RuntimeKind,
            owner_record_id: impl Into<String>,
            owner_revision: ArtifactRevision,
            status: ArtifactAuthorityStatus,
            action: ArtifactReadAuthorityAction,
            project_id: ProjectId,
            task_id: TaskId,
            object: ArtifactObjectIdentity,
            read_claim_id: impl Into<String>,
            observation_digest: ContentDigest,
        ) -> Result<Self, ContractError> {
            if owner_kind == ArtifactAuthorityOwnerKind::ArtifactStore {
                return Err(ContractError::InvalidArtifactAuthority {
                    field: "read_owner_kind",
                });
            }
            let owner_record_id = owner_record_id.into();
            let read_claim_id = read_claim_id.into();
            validate_artifact_identifier("owner_record_id", &owner_record_id)?;
            validate_artifact_identifier("task_id", task_id.as_str())?;
            validate_artifact_identifier("read_claim_id", &read_claim_id)?;
            validate_artifact_digest("authority_observation_digest", &observation_digest)?;
            if status != ArtifactAuthorityStatus::Available {
                return Err(ContractError::InvalidArtifactAuthority {
                    field: "read_owner_status",
                });
            }
            if &project_id != object.key().project_id() {
                return Err(ContractError::InvalidArtifactAuthority {
                    field: "read_project_scope",
                });
            }
            Ok(Self {
                owner_kind,
                producer_id: owner_kind.producer_id().to_owned(),
                producer_version: owner_kind.producer_version().to_owned(),
                runtime,
                owner_record_id,
                owner_revision,
                status,
                action,
                project_id,
                task_id,
                object,
                read_claim_id,
                observation_digest,
            })
        }

        #[must_use]
        pub const fn owner_kind(&self) -> ArtifactAuthorityOwnerKind {
            self.owner_kind
        }
        #[must_use]
        pub fn producer_id(&self) -> &str {
            &self.producer_id
        }
        #[must_use]
        pub fn producer_version(&self) -> &str {
            &self.producer_version
        }
        #[must_use]
        pub const fn runtime(&self) -> RuntimeKind {
            self.runtime
        }
        #[must_use]
        pub fn owner_record_id(&self) -> &str {
            &self.owner_record_id
        }
        #[must_use]
        pub const fn owner_revision(&self) -> ArtifactRevision {
            self.owner_revision
        }
        #[must_use]
        pub const fn status(&self) -> ArtifactAuthorityStatus {
            self.status
        }
        #[must_use]
        pub const fn action(&self) -> ArtifactReadAuthorityAction {
            self.action
        }
        #[must_use]
        pub const fn project_id(&self) -> &ProjectId {
            &self.project_id
        }
        #[must_use]
        pub const fn task_id(&self) -> &TaskId {
            &self.task_id
        }
        #[must_use]
        pub const fn object(&self) -> &ArtifactObjectIdentity {
            &self.object
        }
        #[must_use]
        pub fn read_claim_id(&self) -> &str {
            &self.read_claim_id
        }
        #[must_use]
        pub const fn observation_digest(&self) -> &ContentDigest {
            &self.observation_digest
        }
    }

    /// Closed proof kind accepted for terminal reconciliation of a suspect read.
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub enum ArtifactReadClosureEvidenceKind {
        /// The exact holder process is authoritatively verified dead.
        HolderDeath,
        /// The exact owned byte handle is authoritatively verified closed.
        HandleClosed,
    }

    impl ArtifactReadClosureEvidenceKind {
        #[must_use]
        pub const fn as_str(self) -> &'static str {
            match self {
                Self::HolderDeath => "HOLDER_DEATH",
                Self::HandleClosed => "HANDLE_CLOSED",
            }
        }
    }

    /// Complete fixed-owner scope of one read-holder closure observation.
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct ArtifactReadClosureEvidenceBinding {
        producer_id: String,
        producer_version: String,
        runtime: RuntimeKind,
        evidence_record_id: String,
        evidence_revision: ArtifactRevision,
        status: ArtifactAuthorityStatus,
        kind: ArtifactReadClosureEvidenceKind,
        project_id: ProjectId,
        task_id: TaskId,
        object: ArtifactObjectIdentity,
        read_claim_id: String,
        holder_id: String,
        daemon_instance_id: String,
        daemon_epoch: DaemonEpoch,
        observed_at: String,
        observation_digest: ContentDigest,
    }

    impl ArtifactReadClosureEvidenceBinding {
        /// Constructs one immutable read-closure verifier observation.
        ///
        /// This representation does not authenticate the verifier. The
        /// Artifact Store fake additionally requires an independently queried
        /// full current head and accepts only `RuntimeKind::Fake`.
        ///
        /// # Errors
        ///
        /// Rejects unavailable state, malformed exact scope, malformed
        /// canonical time, or a zero observation digest.
        #[allow(clippy::too_many_arguments)]
        pub fn new(
            runtime: RuntimeKind,
            evidence_record_id: impl Into<String>,
            evidence_revision: ArtifactRevision,
            status: ArtifactAuthorityStatus,
            kind: ArtifactReadClosureEvidenceKind,
            object: ArtifactObjectIdentity,
            task_id: TaskId,
            read_claim_id: impl Into<String>,
            holder_id: impl Into<String>,
            daemon_instance_id: impl Into<String>,
            daemon_epoch: DaemonEpoch,
            observed_at: impl Into<String>,
            observation_digest: ContentDigest,
        ) -> Result<Self, ContractError> {
            let evidence_record_id = evidence_record_id.into();
            let read_claim_id = read_claim_id.into();
            let holder_id = holder_id.into();
            let daemon_instance_id = daemon_instance_id.into();
            let observed_at = observed_at.into();
            for (field, value) in [
                ("closure_evidence_record_id", evidence_record_id.as_str()),
                ("closure_task_id", task_id.as_str()),
                ("closure_read_claim_id", read_claim_id.as_str()),
                ("closure_holder_id", holder_id.as_str()),
                ("closure_daemon_instance_id", daemon_instance_id.as_str()),
            ] {
                validate_artifact_identifier(field, value)?;
            }
            validate_artifact_time("closure_observed_at", &observed_at)?;
            validate_artifact_digest("closure_observation_digest", &observation_digest)?;
            if status != ArtifactAuthorityStatus::Available {
                return Err(ContractError::InvalidArtifactAuthority {
                    field: "closure_evidence_status",
                });
            }
            Ok(Self {
                producer_id: ARTIFACT_READ_CLOSURE_PRODUCER_ID.to_owned(),
                producer_version: ARTIFACT_READ_CLOSURE_PRODUCER_VERSION.to_owned(),
                runtime,
                evidence_record_id,
                evidence_revision,
                status,
                kind,
                project_id: object.key().project_id().clone(),
                task_id,
                object,
                read_claim_id,
                holder_id,
                daemon_instance_id,
                daemon_epoch,
                observed_at,
                observation_digest,
            })
        }

        #[must_use]
        pub fn producer_id(&self) -> &str {
            &self.producer_id
        }
        #[must_use]
        pub fn producer_version(&self) -> &str {
            &self.producer_version
        }
        #[must_use]
        pub const fn runtime(&self) -> RuntimeKind {
            self.runtime
        }
        #[must_use]
        pub fn evidence_record_id(&self) -> &str {
            &self.evidence_record_id
        }
        #[must_use]
        pub const fn evidence_revision(&self) -> ArtifactRevision {
            self.evidence_revision
        }
        #[must_use]
        pub const fn status(&self) -> ArtifactAuthorityStatus {
            self.status
        }
        #[must_use]
        pub const fn kind(&self) -> ArtifactReadClosureEvidenceKind {
            self.kind
        }
        #[must_use]
        pub const fn project_id(&self) -> &ProjectId {
            &self.project_id
        }
        #[must_use]
        pub const fn task_id(&self) -> &TaskId {
            &self.task_id
        }
        #[must_use]
        pub const fn object(&self) -> &ArtifactObjectIdentity {
            &self.object
        }
        #[must_use]
        pub fn read_claim_id(&self) -> &str {
            &self.read_claim_id
        }
        #[must_use]
        pub fn holder_id(&self) -> &str {
            &self.holder_id
        }
        #[must_use]
        pub fn daemon_instance_id(&self) -> &str {
            &self.daemon_instance_id
        }
        #[must_use]
        pub const fn daemon_epoch(&self) -> DaemonEpoch {
            self.daemon_epoch
        }
        #[must_use]
        pub fn observed_at(&self) -> &str {
            &self.observed_at
        }
        #[must_use]
        pub const fn observation_digest(&self) -> &ContentDigest {
            &self.observation_digest
        }
    }

    /// Typed sweep-owner operation.
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub enum ArtifactSweepAuthorityAction {
        ClaimDelete,
    }

    impl ArtifactSweepAuthorityAction {
        #[must_use]
        pub const fn as_str(self) -> &'static str {
            "CLAIM_DELETE"
        }
    }

    /// Complete exact scope asserted by the Artifact Store sweep owner.
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct ArtifactSweepAuthorityBinding {
        producer_id: String,
        producer_version: String,
        runtime: RuntimeKind,
        owner_record_id: String,
        owner_revision: ArtifactRevision,
        status: ArtifactAuthorityStatus,
        action: ArtifactSweepAuthorityAction,
        object: ArtifactObjectIdentity,
        zero_reference_set_digest: ContentDigest,
        zero_read_set_digest: ContentDigest,
        quota_projection_digest: ContentDigest,
        retention_observed_at: String,
        grace_until: String,
        root_identity_digest: ContentDigest,
        daemon_instance_id: String,
        daemon_epoch: DaemonEpoch,
        runtime_admission: RuntimeAdmissionMode,
        observation_digest: ContentDigest,
    }

    impl ArtifactSweepAuthorityBinding {
        /// Constructs an exact fixed-owner sweep authority binding.
        ///
        /// # Errors
        ///
        /// Rejects unavailable authority, malformed identifiers/times, or
        /// zero authority, root, quota, reference, or read-set digests.
        #[allow(clippy::too_many_arguments)]
        pub fn new(
            runtime: RuntimeKind,
            owner_record_id: impl Into<String>,
            owner_revision: ArtifactRevision,
            status: ArtifactAuthorityStatus,
            action: ArtifactSweepAuthorityAction,
            object: ArtifactObjectIdentity,
            zero_reference_set_digest: ContentDigest,
            zero_read_set_digest: ContentDigest,
            quota_projection_digest: ContentDigest,
            retention_observed_at: impl Into<String>,
            grace_until: impl Into<String>,
            root_identity_digest: ContentDigest,
            daemon_instance_id: impl Into<String>,
            daemon_epoch: DaemonEpoch,
            runtime_admission: RuntimeAdmissionMode,
            observation_digest: ContentDigest,
        ) -> Result<Self, ContractError> {
            let owner_record_id = owner_record_id.into();
            let retention_observed_at = retention_observed_at.into();
            let grace_until = grace_until.into();
            let daemon_instance_id = daemon_instance_id.into();
            validate_artifact_identifier("owner_record_id", &owner_record_id)?;
            validate_artifact_identifier("daemon_instance_id", &daemon_instance_id)?;
            validate_artifact_time("retention_observed_at", &retention_observed_at)?;
            validate_artifact_time("grace_until", &grace_until)?;
            for (field, value) in [
                ("zero_reference_set_digest", &zero_reference_set_digest),
                ("zero_read_set_digest", &zero_read_set_digest),
                ("quota_projection_digest", &quota_projection_digest),
                ("root_identity_digest", &root_identity_digest),
                ("authority_observation_digest", &observation_digest),
            ] {
                validate_artifact_digest(field, value)?;
            }
            if status != ArtifactAuthorityStatus::Available {
                return Err(ContractError::InvalidArtifactAuthority {
                    field: "sweep_owner_status",
                });
            }
            Ok(Self {
                producer_id: ARTIFACT_STORE_PRODUCER_ID.to_owned(),
                producer_version: ARTIFACT_STORE_PRODUCER_VERSION.to_owned(),
                runtime,
                owner_record_id,
                owner_revision,
                status,
                action,
                object,
                zero_reference_set_digest,
                zero_read_set_digest,
                quota_projection_digest,
                retention_observed_at,
                grace_until,
                root_identity_digest,
                daemon_instance_id,
                daemon_epoch,
                runtime_admission,
                observation_digest,
            })
        }

        #[must_use]
        pub fn producer_id(&self) -> &str {
            &self.producer_id
        }
        #[must_use]
        pub fn producer_version(&self) -> &str {
            &self.producer_version
        }
        #[must_use]
        pub const fn runtime(&self) -> RuntimeKind {
            self.runtime
        }
        #[must_use]
        pub fn owner_record_id(&self) -> &str {
            &self.owner_record_id
        }
        #[must_use]
        pub const fn owner_revision(&self) -> ArtifactRevision {
            self.owner_revision
        }
        #[must_use]
        pub const fn status(&self) -> ArtifactAuthorityStatus {
            self.status
        }
        #[must_use]
        pub const fn action(&self) -> ArtifactSweepAuthorityAction {
            self.action
        }
        #[must_use]
        pub const fn object(&self) -> &ArtifactObjectIdentity {
            &self.object
        }
        #[must_use]
        pub const fn zero_reference_set_digest(&self) -> &ContentDigest {
            &self.zero_reference_set_digest
        }
        #[must_use]
        pub const fn zero_read_set_digest(&self) -> &ContentDigest {
            &self.zero_read_set_digest
        }
        #[must_use]
        pub const fn quota_projection_digest(&self) -> &ContentDigest {
            &self.quota_projection_digest
        }
        #[must_use]
        pub fn retention_observed_at(&self) -> &str {
            &self.retention_observed_at
        }
        #[must_use]
        pub fn grace_until(&self) -> &str {
            &self.grace_until
        }
        #[must_use]
        pub const fn root_identity_digest(&self) -> &ContentDigest {
            &self.root_identity_digest
        }
        #[must_use]
        pub fn daemon_instance_id(&self) -> &str {
            &self.daemon_instance_id
        }
        #[must_use]
        pub const fn daemon_epoch(&self) -> DaemonEpoch {
            self.daemon_epoch
        }
        #[must_use]
        pub const fn runtime_admission(&self) -> RuntimeAdmissionMode {
            self.runtime_admission
        }
        #[must_use]
        pub const fn observation_digest(&self) -> &ContentDigest {
            &self.observation_digest
        }
    }

    macro_rules! artifact_authority_family {
        (
            $receipt:ident,
            $head:ident,
            $pair:ident,
            $binding:ident,
            $mismatch:literal,
            $receipt_doc:literal,
            $head_doc:literal,
            $pair_doc:literal
        ) => {
            #[doc = $receipt_doc]
            #[derive(Clone, Debug, Eq, PartialEq)]
            pub struct $receipt {
                version: ContractVersion,
                binding: $binding,
                receipt_digest: ContentDigest,
            }

            impl $receipt {
                /// Constructs one typed immutable authority receipt.
                ///
                /// # Errors
                ///
                /// Rejects an unknown contract version or zero receipt digest.
                pub fn new(
                    version: u16,
                    binding: $binding,
                    receipt_digest: ContentDigest,
                ) -> Result<Self, ContractError> {
                    let version = ContractVersion::new(version)?;
                    validate_artifact_digest("authority_receipt_digest", &receipt_digest)?;
                    Ok(Self {
                        version,
                        binding,
                        receipt_digest,
                    })
                }

                #[must_use]
                pub const fn version(&self) -> u16 {
                    self.version.get()
                }
                #[must_use]
                pub const fn binding(&self) -> &$binding {
                    &self.binding
                }
                #[must_use]
                pub const fn receipt_digest(&self) -> &ContentDigest {
                    &self.receipt_digest
                }
                #[must_use]
                pub fn head(&self) -> $head {
                    $head {
                        version: self.version,
                        binding: self.binding.clone(),
                        receipt_digest: self.receipt_digest.clone(),
                    }
                }
            }

            #[doc = $head_doc]
            #[derive(Clone, Debug, Eq, PartialEq)]
            pub struct $head {
                version: ContractVersion,
                binding: $binding,
                receipt_digest: ContentDigest,
            }

            impl $head {
                /// Constructs one independently queried authority head.
                ///
                /// # Errors
                ///
                /// Rejects an unknown contract version or zero receipt digest.
                pub fn new(
                    version: u16,
                    binding: $binding,
                    receipt_digest: ContentDigest,
                ) -> Result<Self, ContractError> {
                    let version = ContractVersion::new(version)?;
                    validate_artifact_digest("authority_receipt_digest", &receipt_digest)?;
                    Ok(Self {
                        version,
                        binding,
                        receipt_digest,
                    })
                }

                #[must_use]
                pub const fn version(&self) -> u16 {
                    self.version.get()
                }
                #[must_use]
                pub const fn binding(&self) -> &$binding {
                    &self.binding
                }
                #[must_use]
                pub const fn receipt_digest(&self) -> &ContentDigest {
                    &self.receipt_digest
                }
            }

            #[doc = $pair_doc]
            #[derive(Clone, Debug, Eq, PartialEq)]
            pub struct $pair {
                receipt: $receipt,
                current_head: $head,
            }

            impl $pair {
                /// Matches a receipt with an independently queried full head.
                ///
                /// # Errors
                ///
                /// Rejects any structural field disagreement.
                pub fn new(receipt: $receipt, current_head: $head) -> Result<Self, ContractError> {
                    if receipt.head() != current_head {
                        return Err(ContractError::ArtifactAuthorityHeadMismatch {
                            field: $mismatch,
                        });
                    }
                    Ok(Self {
                        receipt,
                        current_head,
                    })
                }

                #[must_use]
                pub const fn receipt(&self) -> &$receipt {
                    &self.receipt
                }
                #[must_use]
                pub const fn current_head(&self) -> &$head {
                    &self.current_head
                }
            }
        };
    }

    artifact_authority_family!(
        ArtifactReferenceAuthorityReceipt,
        ArtifactReferenceAuthorityHead,
        ArtifactReferenceAuthorityPair,
        ArtifactReferenceAuthorityBinding,
        "reference_authority",
        "Typed immutable reference-owner authority receipt.",
        "Independently queried complete reference-owner current head.",
        "Exact matching reference-owner receipt/current-head pair."
    );
    artifact_authority_family!(
        ArtifactReadAuthorityReceipt,
        ArtifactReadAuthorityHead,
        ArtifactReadAuthorityPair,
        ArtifactReadAuthorityBinding,
        "read_authority",
        "Typed immutable active-read-owner authority receipt.",
        "Independently queried complete active-read-owner current head.",
        "Exact matching active-read-owner receipt/current-head pair."
    );
    artifact_authority_family!(
        ArtifactReadClosureEvidenceReceipt,
        ArtifactReadClosureEvidenceHead,
        ArtifactReadClosureEvidencePair,
        ArtifactReadClosureEvidenceBinding,
        "read_closure_evidence",
        "Typed immutable fixed-owner read-closure evidence receipt.",
        "Independently queried complete read-closure verifier current head.",
        "Exact matching read-closure evidence receipt/current-head pair."
    );
    artifact_authority_family!(
        ArtifactSweepAuthorityReceipt,
        ArtifactSweepAuthorityHead,
        ArtifactSweepAuthorityPair,
        ArtifactSweepAuthorityBinding,
        "sweep_authority",
        "Typed immutable Artifact Store sweep-authority receipt.",
        "Independently queried complete Artifact Store sweep current head.",
        "Exact matching sweep receipt/current-head pair."
    );

    /// Complete immutable artifact reference and provenance manifest.
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct ArtifactReferenceManifest {
        binding: SubjectBinding,
        attempt_id: AttemptId,
        request_id: RequestId,
        reference_id: String,
        object: ArtifactObjectIdentity,
        byte_length: ArtifactByteLength,
        media_type: String,
        payload_schema_id: String,
        payload_schema_version: String,
        bundle: Option<ArtifactBundleBounds>,
        provenance: ArtifactProvenance,
        creation_authority: ArtifactReferenceAuthorityPair,
        purpose: ArtifactPurpose,
        retention_until: String,
        manifest_digest: ContentDigest,
    }

    impl ArtifactReferenceManifest {
        /// Constructs one complete immutable use of an artifact object.
        ///
        /// # Errors
        ///
        /// Rejects project/task/object/reference disagreement, release
        /// authority as creation authority, unbounded text, malformed time, or
        /// a zero manifest digest.
        #[allow(clippy::too_many_arguments)]
        pub fn new(
            binding: SubjectBinding,
            attempt_id: AttemptId,
            request_id: RequestId,
            reference_id: impl Into<String>,
            object: ArtifactObjectIdentity,
            byte_length: ArtifactByteLength,
            media_type: impl Into<String>,
            payload_schema_id: impl Into<String>,
            payload_schema_version: impl Into<String>,
            bundle: Option<ArtifactBundleBounds>,
            provenance: ArtifactProvenance,
            creation_authority: ArtifactReferenceAuthorityPair,
            purpose: ArtifactPurpose,
            retention_until: impl Into<String>,
            manifest_digest: ContentDigest,
        ) -> Result<Self, ContractError> {
            let reference_id = reference_id.into();
            let media_type = media_type.into();
            let payload_schema_id = payload_schema_id.into();
            let payload_schema_version = payload_schema_version.into();
            let retention_until = retention_until.into();
            for (field, value) in [
                (
                    "project_snapshot_id",
                    binding.project_snapshot_id().as_str(),
                ),
                ("task_id", binding.task_id().as_str()),
                ("attempt_id", attempt_id.as_str()),
                ("request_id", request_id.as_str()),
                ("reference_id", reference_id.as_str()),
            ] {
                validate_artifact_identifier(field, value)?;
            }
            for (field, value) in [
                ("media_type", media_type.as_str()),
                ("payload_schema_id", payload_schema_id.as_str()),
                ("payload_schema_version", payload_schema_version.as_str()),
            ] {
                validate_artifact_text(field, value)?;
            }
            validate_artifact_time("retention_until", &retention_until)?;
            validate_artifact_digest("manifest_digest", &manifest_digest)?;
            if binding.project_id() != object.key().project_id() {
                return Err(ContractError::InvalidArtifactValue {
                    field: "project_scope",
                });
            }
            let owner = creation_authority.receipt().binding();
            if owner.project_id() != binding.project_id()
                || owner.task_id() != binding.task_id()
                || owner.object() != &object
                || owner.reference_id() != reference_id
            {
                return Err(ContractError::InvalidArtifactAuthority {
                    field: "creation_scope",
                });
            }
            if owner.action() == ArtifactReferenceAuthorityAction::ReleaseReference {
                return Err(ContractError::InvalidArtifactAuthority {
                    field: "creation_action",
                });
            }
            Ok(Self {
                binding,
                attempt_id,
                request_id,
                reference_id,
                object,
                byte_length,
                media_type,
                payload_schema_id,
                payload_schema_version,
                bundle,
                provenance,
                creation_authority,
                purpose,
                retention_until,
                manifest_digest,
            })
        }

        #[must_use]
        pub const fn binding(&self) -> &SubjectBinding {
            &self.binding
        }
        #[must_use]
        pub const fn attempt_id(&self) -> &AttemptId {
            &self.attempt_id
        }
        #[must_use]
        pub const fn request_id(&self) -> &RequestId {
            &self.request_id
        }
        #[must_use]
        pub fn reference_id(&self) -> &str {
            &self.reference_id
        }
        #[must_use]
        pub const fn object(&self) -> &ArtifactObjectIdentity {
            &self.object
        }
        #[must_use]
        pub const fn byte_length(&self) -> ArtifactByteLength {
            self.byte_length
        }
        #[must_use]
        pub fn media_type(&self) -> &str {
            &self.media_type
        }
        #[must_use]
        pub fn payload_schema_id(&self) -> &str {
            &self.payload_schema_id
        }
        #[must_use]
        pub fn payload_schema_version(&self) -> &str {
            &self.payload_schema_version
        }
        #[must_use]
        pub const fn bundle(&self) -> Option<ArtifactBundleBounds> {
            self.bundle
        }
        #[must_use]
        pub const fn provenance(&self) -> &ArtifactProvenance {
            &self.provenance
        }
        #[must_use]
        pub const fn creation_authority(&self) -> &ArtifactReferenceAuthorityPair {
            &self.creation_authority
        }
        #[must_use]
        pub const fn purpose(&self) -> ArtifactPurpose {
            self.purpose
        }
        #[must_use]
        pub fn retention_until(&self) -> &str {
            &self.retention_until
        }
        #[must_use]
        pub const fn manifest_digest(&self) -> &ContentDigest {
            &self.manifest_digest
        }
    }

    /// Complete object-state projection carried by an Artifact Store head.
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct ArtifactObjectHead {
        object: ArtifactObjectIdentity,
        revision: ArtifactRevision,
        availability: ArtifactAvailability,
        byte_length: ArtifactByteLength,
        active_reference_count: ArtifactCounter,
        active_reference_set_digest: ContentDigest,
        sweep_not_before: String,
        active_read_count: ArtifactCounter,
        active_read_set_digest: ContentDigest,
        delete_status: ArtifactDeleteStatus,
        delete_claim_token: Option<String>,
        task_quota_projection_digest: ContentDigest,
        project_quota_projection_digest: ContentDigest,
        store_quota_projection_digest: ContentDigest,
        staging_quota_projection_digest: ContentDigest,
        command_high_water: ArtifactCounter,
        command_tail_digest: ContentDigest,
        transition_digest: ContentDigest,
    }

    impl ArtifactObjectHead {
        /// Constructs one structurally consistent object-state projection.
        ///
        /// # Errors
        ///
        /// Rejects malformed times/digests/tokens, an empty command chain,
        /// incompatible availability/delete state, or active delete blockers.
        #[allow(clippy::too_many_arguments)]
        pub fn new(
            object: ArtifactObjectIdentity,
            revision: ArtifactRevision,
            availability: ArtifactAvailability,
            byte_length: ArtifactByteLength,
            active_reference_count: ArtifactCounter,
            active_reference_set_digest: ContentDigest,
            sweep_not_before: impl Into<String>,
            active_read_count: ArtifactCounter,
            active_read_set_digest: ContentDigest,
            delete_status: ArtifactDeleteStatus,
            delete_claim_token: Option<String>,
            task_quota_projection_digest: ContentDigest,
            project_quota_projection_digest: ContentDigest,
            store_quota_projection_digest: ContentDigest,
            staging_quota_projection_digest: ContentDigest,
            command_high_water: ArtifactCounter,
            command_tail_digest: ContentDigest,
            transition_digest: ContentDigest,
        ) -> Result<Self, ContractError> {
            let sweep_not_before = sweep_not_before.into();
            validate_artifact_time("sweep_not_before", &sweep_not_before)?;
            for (field, value) in [
                ("active_reference_set_digest", &active_reference_set_digest),
                ("active_read_set_digest", &active_read_set_digest),
                (
                    "task_quota_projection_digest",
                    &task_quota_projection_digest,
                ),
                (
                    "project_quota_projection_digest",
                    &project_quota_projection_digest,
                ),
                (
                    "store_quota_projection_digest",
                    &store_quota_projection_digest,
                ),
                (
                    "staging_quota_projection_digest",
                    &staging_quota_projection_digest,
                ),
                ("command_tail_digest", &command_tail_digest),
                ("transition_digest", &transition_digest),
            ] {
                validate_artifact_digest(field, value)?;
            }
            if command_high_water.get() == 0 {
                return Err(ContractError::InvalidArtifactValue {
                    field: "command_high_water",
                });
            }
            if let Some(token) = delete_claim_token.as_deref() {
                validate_artifact_identifier("delete_claim_token", token)?;
            }
            let valid_delete_state = matches!(
                (availability, delete_status, delete_claim_token.is_some()),
                (
                    ArtifactAvailability::Available,
                    ArtifactDeleteStatus::NotClaimed,
                    false
                ) | (
                    ArtifactAvailability::Available,
                    ArtifactDeleteStatus::VerifiedNoEffect,
                    true
                ) | (
                    ArtifactAvailability::DeleteClaimed,
                    ArtifactDeleteStatus::Claimed,
                    true
                ) | (
                    ArtifactAvailability::Deleted,
                    ArtifactDeleteStatus::VerifiedDeleted,
                    true
                ) | (
                    ArtifactAvailability::ReconciliationRequired,
                    ArtifactDeleteStatus::ReconciliationRequired,
                    true
                )
            );
            if !valid_delete_state {
                return Err(ContractError::InvalidArtifactValue {
                    field: "delete_state",
                });
            }
            if availability != ArtifactAvailability::Available
                && (active_reference_count.get() != 0 || active_read_count.get() != 0)
            {
                return Err(ContractError::InvalidArtifactValue {
                    field: "delete_blockers",
                });
            }
            Ok(Self {
                object,
                revision,
                availability,
                byte_length,
                active_reference_count,
                active_reference_set_digest,
                sweep_not_before,
                active_read_count,
                active_read_set_digest,
                delete_status,
                delete_claim_token,
                task_quota_projection_digest,
                project_quota_projection_digest,
                store_quota_projection_digest,
                staging_quota_projection_digest,
                command_high_water,
                command_tail_digest,
                transition_digest,
            })
        }

        #[must_use]
        pub const fn object(&self) -> &ArtifactObjectIdentity {
            &self.object
        }
        #[must_use]
        pub const fn revision(&self) -> ArtifactRevision {
            self.revision
        }
        #[must_use]
        pub const fn availability(&self) -> ArtifactAvailability {
            self.availability
        }
        #[must_use]
        pub const fn byte_length(&self) -> ArtifactByteLength {
            self.byte_length
        }
        #[must_use]
        pub const fn active_reference_count(&self) -> ArtifactCounter {
            self.active_reference_count
        }
        #[must_use]
        pub const fn active_reference_set_digest(&self) -> &ContentDigest {
            &self.active_reference_set_digest
        }
        #[must_use]
        pub fn sweep_not_before(&self) -> &str {
            &self.sweep_not_before
        }
        #[must_use]
        pub const fn active_read_count(&self) -> ArtifactCounter {
            self.active_read_count
        }
        #[must_use]
        pub const fn active_read_set_digest(&self) -> &ContentDigest {
            &self.active_read_set_digest
        }
        #[must_use]
        pub const fn delete_status(&self) -> ArtifactDeleteStatus {
            self.delete_status
        }
        #[must_use]
        pub fn delete_claim_token(&self) -> Option<&str> {
            self.delete_claim_token.as_deref()
        }
        #[must_use]
        pub const fn task_quota_projection_digest(&self) -> &ContentDigest {
            &self.task_quota_projection_digest
        }
        #[must_use]
        pub const fn project_quota_projection_digest(&self) -> &ContentDigest {
            &self.project_quota_projection_digest
        }
        #[must_use]
        pub const fn store_quota_projection_digest(&self) -> &ContentDigest {
            &self.store_quota_projection_digest
        }
        #[must_use]
        pub const fn staging_quota_projection_digest(&self) -> &ContentDigest {
            &self.staging_quota_projection_digest
        }
        #[must_use]
        pub const fn command_high_water(&self) -> ArtifactCounter {
            self.command_high_water
        }
        #[must_use]
        pub const fn command_tail_digest(&self) -> &ContentDigest {
            &self.command_tail_digest
        }
        #[must_use]
        pub const fn transition_digest(&self) -> &ContentDigest {
            &self.transition_digest
        }
    }

    /// Complete current projection of one immutable artifact reference.
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct ArtifactReferenceHead {
        manifest: ArtifactReferenceManifest,
        transition_authority: ArtifactReferenceAuthorityPair,
        revision: ArtifactRevision,
        status: ArtifactReferenceStatus,
        transition_digest: ContentDigest,
    }

    impl ArtifactReferenceHead {
        /// Constructs a reference head bound to the exact transition authority.
        ///
        /// # Errors
        ///
        /// Rejects a zero transition digest, scope disagreement, or an action
        /// that cannot produce the supplied reference status.
        pub fn new(
            manifest: ArtifactReferenceManifest,
            transition_authority: ArtifactReferenceAuthorityPair,
            revision: ArtifactRevision,
            status: ArtifactReferenceStatus,
            transition_digest: ContentDigest,
        ) -> Result<Self, ContractError> {
            validate_artifact_digest("reference_transition_digest", &transition_digest)?;
            let authority = transition_authority.receipt().binding();
            if authority.project_id() != manifest.binding().project_id()
                || authority.task_id() != manifest.binding().task_id()
                || authority.object() != manifest.object()
                || authority.reference_id() != manifest.reference_id()
            {
                return Err(ContractError::InvalidArtifactAuthority {
                    field: "reference_transition_scope",
                });
            }
            let action_matches_status = matches!(
                (status, authority.action()),
                (
                    ArtifactReferenceStatus::Active,
                    ArtifactReferenceAuthorityAction::PublishInitialReference
                        | ArtifactReferenceAuthorityAction::AddReference
                ) | (
                    ArtifactReferenceStatus::Released,
                    ArtifactReferenceAuthorityAction::ReleaseReference
                )
            );
            if !action_matches_status {
                return Err(ContractError::InvalidArtifactAuthority {
                    field: "reference_transition_action",
                });
            }
            Ok(Self {
                manifest,
                transition_authority,
                revision,
                status,
                transition_digest,
            })
        }

        #[must_use]
        pub const fn manifest(&self) -> &ArtifactReferenceManifest {
            &self.manifest
        }
        #[must_use]
        pub const fn transition_authority(&self) -> &ArtifactReferenceAuthorityPair {
            &self.transition_authority
        }
        #[must_use]
        pub const fn revision(&self) -> ArtifactRevision {
            self.revision
        }
        #[must_use]
        pub const fn status(&self) -> ArtifactReferenceStatus {
            self.status
        }
        #[must_use]
        pub const fn transition_digest(&self) -> &ContentDigest {
            &self.transition_digest
        }
    }

    /// Complete current projection of one artifact read claim.
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct ArtifactReadHead {
        authority: ArtifactReadAuthorityPair,
        revision: ArtifactRevision,
        status: ArtifactReadStatus,
        holder_id: String,
        acquired_at: String,
        expires_at: String,
        transition_digest: ContentDigest,
    }

    impl ArtifactReadHead {
        /// Constructs a read head bound to the exact read-owner authority.
        ///
        /// # Errors
        ///
        /// Rejects malformed holder/time/digest values or an action that
        /// cannot produce the supplied read status.
        pub fn new(
            authority: ArtifactReadAuthorityPair,
            revision: ArtifactRevision,
            status: ArtifactReadStatus,
            holder_id: impl Into<String>,
            acquired_at: impl Into<String>,
            expires_at: impl Into<String>,
            transition_digest: ContentDigest,
        ) -> Result<Self, ContractError> {
            let holder_id = holder_id.into();
            let acquired_at = acquired_at.into();
            let expires_at = expires_at.into();
            validate_artifact_identifier("read_holder_id", &holder_id)?;
            validate_artifact_time("read_acquired_at", &acquired_at)?;
            validate_artifact_time("read_expires_at", &expires_at)?;
            validate_artifact_digest("read_transition_digest", &transition_digest)?;
            let action_matches_status = matches!(
                (status, authority.receipt().binding().action()),
                (
                    ArtifactReadStatus::Active | ArtifactReadStatus::ExpiredSuspect,
                    ArtifactReadAuthorityAction::AcquireRead
                ) | (
                    ArtifactReadStatus::Released,
                    ArtifactReadAuthorityAction::ReleaseRead
                )
            );
            if !action_matches_status {
                return Err(ContractError::InvalidArtifactAuthority {
                    field: "read_transition_action",
                });
            }
            Ok(Self {
                authority,
                revision,
                status,
                holder_id,
                acquired_at,
                expires_at,
                transition_digest,
            })
        }

        #[must_use]
        pub const fn authority(&self) -> &ArtifactReadAuthorityPair {
            &self.authority
        }
        #[must_use]
        pub const fn revision(&self) -> ArtifactRevision {
            self.revision
        }
        #[must_use]
        pub const fn status(&self) -> ArtifactReadStatus {
            self.status
        }
        #[must_use]
        pub fn holder_id(&self) -> &str {
            &self.holder_id
        }
        #[must_use]
        pub fn acquired_at(&self) -> &str {
            &self.acquired_at
        }
        #[must_use]
        pub fn expires_at(&self) -> &str {
            &self.expires_at
        }
        #[must_use]
        pub const fn transition_digest(&self) -> &ContentDigest {
            &self.transition_digest
        }
    }

    fn validate_artifact_store_projection(
        runtime: RuntimeKind,
        object: &ArtifactObjectHead,
        reference: Option<&ArtifactReferenceHead>,
        read: Option<&ArtifactReadHead>,
        observation_digest: &ContentDigest,
        receipt_digest: &ContentDigest,
    ) -> Result<(), ContractError> {
        validate_artifact_digest("artifact_observation_digest", observation_digest)?;
        validate_artifact_digest("artifact_receipt_digest", receipt_digest)?;
        if let Some(reference) = reference
            && (reference.manifest().object() != object.object()
                || reference
                    .transition_authority()
                    .receipt()
                    .binding()
                    .runtime()
                    != runtime)
        {
            return Err(ContractError::InvalidArtifactReceipt {
                field: "reference_projection",
            });
        }
        if let Some(read) = read
            && (read.authority().receipt().binding().object() != object.object()
                || read.authority().receipt().binding().runtime() != runtime)
        {
            return Err(ContractError::InvalidArtifactReceipt {
                field: "read_projection",
            });
        }
        Ok(())
    }

    fn validate_artifact_store_producer(value: &str) -> Result<(), ContractError> {
        if value != ARTIFACT_STORE_PRODUCER_ID {
            return Err(ContractError::UnsupportedArtifactStoreProducer);
        }
        Ok(())
    }

    fn validate_artifact_store_producer_version(value: &str) -> Result<(), ContractError> {
        if value != ARTIFACT_STORE_PRODUCER_VERSION {
            return Err(ContractError::UnsupportedArtifactStoreProducerVersion);
        }
        Ok(())
    }

    /// Immutable fixed-owner Artifact Store transition receipt.
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct ArtifactAuthorityReceipt {
        version: ContractVersion,
        producer_id: String,
        producer_version: String,
        runtime: RuntimeKind,
        object: ArtifactObjectHead,
        reference: Option<ArtifactReferenceHead>,
        read: Option<ArtifactReadHead>,
        observation_digest: ContentDigest,
        receipt_digest: ContentDigest,
    }

    impl ArtifactAuthorityReceipt {
        /// Constructs one complete fixed-owner store receipt.
        ///
        /// # Errors
        ///
        /// Rejects unknown versions/owners, zero digests, or nested
        /// object/reference/read scope and runtime disagreement.
        #[allow(clippy::too_many_arguments)]
        pub fn new(
            version: u16,
            producer_id: impl Into<String>,
            producer_version: impl Into<String>,
            runtime: RuntimeKind,
            object: ArtifactObjectHead,
            reference: Option<ArtifactReferenceHead>,
            read: Option<ArtifactReadHead>,
            observation_digest: ContentDigest,
            receipt_digest: ContentDigest,
        ) -> Result<Self, ContractError> {
            let version = ContractVersion::new(version)?;
            let producer_id = producer_id.into();
            let producer_version = producer_version.into();
            validate_artifact_store_producer(&producer_id)?;
            validate_artifact_store_producer_version(&producer_version)?;
            validate_artifact_store_projection(
                runtime,
                &object,
                reference.as_ref(),
                read.as_ref(),
                &observation_digest,
                &receipt_digest,
            )?;
            Ok(Self {
                version,
                producer_id,
                producer_version,
                runtime,
                object,
                reference,
                read,
                observation_digest,
                receipt_digest,
            })
        }

        #[must_use]
        pub const fn version(&self) -> u16 {
            self.version.get()
        }
        #[must_use]
        pub fn producer_id(&self) -> &str {
            &self.producer_id
        }
        #[must_use]
        pub fn producer_version(&self) -> &str {
            &self.producer_version
        }
        #[must_use]
        pub const fn runtime(&self) -> RuntimeKind {
            self.runtime
        }
        #[must_use]
        pub const fn object(&self) -> &ArtifactObjectHead {
            &self.object
        }
        #[must_use]
        pub const fn reference(&self) -> Option<&ArtifactReferenceHead> {
            self.reference.as_ref()
        }
        #[must_use]
        pub const fn read(&self) -> Option<&ArtifactReadHead> {
            self.read.as_ref()
        }
        #[must_use]
        pub const fn observation_digest(&self) -> &ContentDigest {
            &self.observation_digest
        }
        #[must_use]
        pub const fn receipt_digest(&self) -> &ContentDigest {
            &self.receipt_digest
        }

        /// Projects every nested security field into a structural current head.
        #[must_use]
        pub fn head(&self) -> ArtifactAuthorityHead {
            ArtifactAuthorityHead {
                version: self.version,
                producer_id: self.producer_id.clone(),
                producer_version: self.producer_version.clone(),
                runtime: self.runtime,
                object: self.object.clone(),
                reference: self.reference.clone(),
                read: self.read.clone(),
                observation_digest: self.observation_digest.clone(),
                receipt_digest: self.receipt_digest.clone(),
            }
        }
    }

    /// Complete Artifact Store current head from an independent owner lookup.
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct ArtifactAuthorityHead {
        version: ContractVersion,
        producer_id: String,
        producer_version: String,
        runtime: RuntimeKind,
        object: ArtifactObjectHead,
        reference: Option<ArtifactReferenceHead>,
        read: Option<ArtifactReadHead>,
        observation_digest: ContentDigest,
        receipt_digest: ContentDigest,
    }

    impl ArtifactAuthorityHead {
        /// Constructs one complete structural Artifact Store current head.
        ///
        /// # Errors
        ///
        /// Rejects unknown versions/owners, zero digests, or nested
        /// object/reference/read scope and runtime disagreement.
        #[allow(clippy::too_many_arguments)]
        pub fn new(
            version: u16,
            producer_id: impl Into<String>,
            producer_version: impl Into<String>,
            runtime: RuntimeKind,
            object: ArtifactObjectHead,
            reference: Option<ArtifactReferenceHead>,
            read: Option<ArtifactReadHead>,
            observation_digest: ContentDigest,
            receipt_digest: ContentDigest,
        ) -> Result<Self, ContractError> {
            let version = ContractVersion::new(version)?;
            let producer_id = producer_id.into();
            let producer_version = producer_version.into();
            validate_artifact_store_producer(&producer_id)?;
            validate_artifact_store_producer_version(&producer_version)?;
            validate_artifact_store_projection(
                runtime,
                &object,
                reference.as_ref(),
                read.as_ref(),
                &observation_digest,
                &receipt_digest,
            )?;
            Ok(Self {
                version,
                producer_id,
                producer_version,
                runtime,
                object,
                reference,
                read,
                observation_digest,
                receipt_digest,
            })
        }

        #[must_use]
        pub const fn version(&self) -> u16 {
            self.version.get()
        }
        #[must_use]
        pub fn producer_id(&self) -> &str {
            &self.producer_id
        }
        #[must_use]
        pub fn producer_version(&self) -> &str {
            &self.producer_version
        }
        #[must_use]
        pub const fn runtime(&self) -> RuntimeKind {
            self.runtime
        }
        #[must_use]
        pub const fn object(&self) -> &ArtifactObjectHead {
            &self.object
        }
        #[must_use]
        pub const fn reference(&self) -> Option<&ArtifactReferenceHead> {
            self.reference.as_ref()
        }
        #[must_use]
        pub const fn read(&self) -> Option<&ArtifactReadHead> {
            self.read.as_ref()
        }
        #[must_use]
        pub const fn observation_digest(&self) -> &ContentDigest {
            &self.observation_digest
        }
        #[must_use]
        pub const fn receipt_digest(&self) -> &ContentDigest {
            &self.receipt_digest
        }
    }

    /// Alias emphasizing that an Artifact Store authority head is the complete
    /// structural current-head representation.
    pub type ArtifactCurrentHead = ArtifactAuthorityHead;

    fn validate_artifact_identifier(field: &'static str, value: &str) -> Result<(), ContractError> {
        let valid = (1..=MAX_ARTIFACT_TEXT_BYTES).contains(&value.len())
            && value.trim() == value
            && value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-')
            });
        if valid {
            Ok(())
        } else {
            Err(ContractError::InvalidArtifactValue { field })
        }
    }

    fn validate_artifact_text(field: &'static str, value: &str) -> Result<(), ContractError> {
        let valid = (1..=MAX_ARTIFACT_TEXT_BYTES).contains(&value.len())
            && value.trim() == value
            && !value.contains('\0')
            && !value.contains('\\');
        if valid {
            Ok(())
        } else {
            Err(ContractError::InvalidArtifactValue { field })
        }
    }

    fn validate_artifact_time(field: &'static str, value: &str) -> Result<(), ContractError> {
        let bytes = value.as_bytes();
        let fixed_shape = bytes.len() == 20
            && bytes[4] == b'-'
            && bytes[7] == b'-'
            && bytes[10] == b'T'
            && bytes[13] == b':'
            && bytes[16] == b':'
            && bytes[19] == b'Z'
            && bytes.iter().enumerate().all(|(index, byte)| {
                matches!(index, 4 | 7 | 10 | 13 | 16 | 19) || byte.is_ascii_digit()
            });
        if fixed_shape {
            let year = parse_ascii_digits(&bytes[0..4]);
            let month = parse_ascii_digits(&bytes[5..7]);
            let day = parse_ascii_digits(&bytes[8..10]);
            let hour = parse_ascii_digits(&bytes[11..13]);
            let minute = parse_ascii_digits(&bytes[14..16]);
            let second = parse_ascii_digits(&bytes[17..19]);
            let leap_year =
                year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
            let days_in_month = match month {
                1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
                4 | 6 | 9 | 11 => 30,
                2 if leap_year => 29,
                2 => 28,
                _ => 0,
            };
            if year > 0
                && day > 0
                && day <= days_in_month
                && hour <= 23
                && minute <= 59
                && second <= 59
            {
                return Ok(());
            }
        }
        Err(ContractError::InvalidArtifactValue { field })
    }

    fn parse_ascii_digits(bytes: &[u8]) -> u32 {
        bytes
            .iter()
            .fold(0, |value, byte| value * 10 + u32::from(byte - b'0'))
    }

    fn validate_artifact_digest(
        field: &'static str,
        value: &ContentDigest,
    ) -> Result<(), ContractError> {
        if is_zero_digest(value) {
            Err(ContractError::InvalidArtifactValue { field })
        } else {
            Ok(())
        }
    }
}

pub use artifact_contracts::*;

/// A normalized evidence reference returned through a LATTICE port.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Evidence {
    invocation: Invocation,
    component: Component,
    boundary: Boundary,
    runtime: RuntimeKind,
    output_digest: ContentDigest,
}

impl Evidence {
    const fn new(
        invocation: Invocation,
        component: Component,
        boundary: Boundary,
        runtime: RuntimeKind,
        output_digest: ContentDigest,
    ) -> Self {
        Self {
            invocation,
            component,
            boundary,
            runtime,
            output_digest,
        }
    }

    /// Returns the immutable request identity.
    #[must_use]
    pub const fn invocation(&self) -> &Invocation {
        &self.invocation
    }

    /// Returns the producing component.
    #[must_use]
    pub const fn component(&self) -> Component {
        self.component
    }

    /// Returns the authority/trust boundary.
    #[must_use]
    pub const fn boundary(&self) -> Boundary {
        self.boundary
    }

    /// Returns the runtime classification.
    #[must_use]
    pub const fn runtime(&self) -> RuntimeKind {
        self.runtime
    }

    /// Returns the content-addressed output reference.
    #[must_use]
    pub const fn output_digest(&self) -> &ContentDigest {
        &self.output_digest
    }
}

macro_rules! lane_evidence {
    ($name:ident, $component:path, $boundary:path, $description:literal) => {
        #[doc = $description]
        #[derive(Clone, Debug, Eq, PartialEq)]
        pub struct $name(Evidence);

        impl $name {
            /// Constructs evidence with a compile-time fixed component and
            /// authority boundary.
            #[must_use]
            pub const fn new(
                invocation: Invocation,
                runtime: RuntimeKind,
                output_digest: ContentDigest,
            ) -> Self {
                Self(Evidence::new(
                    invocation,
                    $component,
                    $boundary,
                    runtime,
                    output_digest,
                ))
            }

            /// Returns the immutable request identity.
            #[must_use]
            pub const fn invocation(&self) -> &Invocation {
                self.0.invocation()
            }

            /// Returns the compile-time fixed component.
            #[must_use]
            pub const fn component(&self) -> Component {
                self.0.component()
            }

            /// Returns the compile-time fixed authority/trust boundary.
            #[must_use]
            pub const fn boundary(&self) -> Boundary {
                self.0.boundary()
            }

            /// Returns the runtime classification.
            #[must_use]
            pub const fn runtime(&self) -> RuntimeKind {
                self.0.runtime()
            }

            /// Returns the content-addressed output reference.
            #[must_use]
            pub const fn output_digest(&self) -> &ContentDigest {
                self.0.output_digest()
            }

            /// Converts the lane-specific value to normalized evidence after
            /// the type system has fixed its component/boundary pair.
            #[must_use]
            pub fn into_normalized(self) -> Evidence {
                self.0
            }
        }
    };
}

lane_evidence!(
    GatewayEvidence,
    Component::OpenClaw,
    Boundary::Gateway,
    "Normalized evidence attributed to the `OpenClaw` gateway boundary."
);
lane_evidence!(
    CodexEvidence,
    Component::Codex,
    Boundary::ProductCodeWriter,
    "Evidence returned only by the sole product-code writer lane."
);
lane_evidence!(
    GraphifyEvidence,
    Component::Graphify,
    Boundary::DerivedReadOnlyEvidence,
    "Derived read-only evidence returned by the knowledge lane."
);
lane_evidence!(
    HermesEvidence,
    Component::Hermes,
    Boundary::UntrustedCandidate,
    "Untrusted candidate evidence returned by the research lane."
);

/// The complete typed action set accepted from the normal `OpenClaw` gateway.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GatewayAction {
    Submit,
    Plan,
    Status,
    Approve,
    Reject,
    Stop,
}

impl GatewayAction {
    /// Exhaustive initial action set; no arbitrary shell or SQL action exists.
    pub const ALL: [Self; 6] = [
        Self::Submit,
        Self::Plan,
        Self::Status,
        Self::Approve,
        Self::Reject,
        Self::Stop,
    ];

    /// Returns the stable wire-facing action name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Submit => "submit",
            Self::Plan => "plan",
            Self::Status => "status",
            Self::Approve => "approve",
            Self::Reject => "reject",
            Self::Stop => "stop",
        }
    }
}

macro_rules! gateway_identifier {
    ($name:ident, $field:literal, $description:literal) => {
        #[doc = $description]
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);

        impl $name {
            /// Constructs one bounded canonical ASCII gateway identifier.
            ///
            /// # Errors
            ///
            /// Rejects empty, oversized, non-ASCII, whitespace, control, or
            /// punctuation outside the closed safe identifier alphabet.
            pub fn new(value: impl Into<String>) -> Result<Self, ContractError> {
                let value = value.into();
                if !valid_gateway_identifier(&value) {
                    return Err(ContractError::InvalidGatewayValue { field: $field });
                }
                Ok(Self(value))
            }

            /// Returns the canonical identifier text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

fn valid_gateway_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= GATEWAY_IDENTIFIER_MAX_BYTES
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':' | b'/' | b'@')
        })
}

fn validate_gateway_subject_binding(binding: &SubjectBinding) -> Result<(), ContractError> {
    for (field, value) in [
        (
            "project_snapshot_id",
            binding.project_snapshot_id().as_str(),
        ),
        ("task_id", binding.task_id().as_str()),
    ] {
        if value.len() > GATEWAY_IDENTIFIER_MAX_BYTES {
            return Err(ContractError::InvalidGatewayValue { field });
        }
    }
    Ok(())
}

gateway_identifier!(
    GatewayCommandId,
    "command_id",
    "A client-selected idempotent semantic gateway command identity."
);
gateway_identifier!(
    GatewayCorrelationId,
    "correlation_id",
    "A safe gateway correlation identity."
);
gateway_identifier!(
    GatewayActorId,
    "actor_id",
    "A server-derived gateway actor identity."
);
gateway_identifier!(
    GatewayInstanceId,
    "gateway_instance_id",
    "A server-derived gateway instance identity."
);
gateway_identifier!(
    GatewayAdapterId,
    "adapter_id",
    "A gateway adapter implementation identity."
);
gateway_identifier!(
    GatewayChannelId,
    "channel_id",
    "A server-derived gateway channel identity."
);
gateway_identifier!(
    GatewaySessionId,
    "session_id",
    "A server-derived gateway session identity."
);
gateway_identifier!(
    GatewayApprovalId,
    "approval_id",
    "An Approval-Verifier-owned approval identity reference."
);
gateway_identifier!(
    GatewayChallengeId,
    "challenge_id",
    "An Approval-Verifier-owned challenge identity reference."
);

/// Closed kind of server-observed gateway client.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GatewayClientKind {
    /// The sole normal human gateway.
    OpenClaw,
    /// Bounded recovery/test client; never a second normal gateway.
    RecoveryCli,
    /// Visibly fake contract-test client.
    TestFake,
}

impl GatewayClientKind {
    /// Returns the stable wire-facing value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OpenClaw => "OPENCLAW",
            Self::RecoveryCli => "RECOVERY_CLI",
            Self::TestFake => "TEST_FAKE",
        }
    }
}

/// Closed actor classification derived by a trusted ingress surface.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GatewayActorKind {
    /// Normal responsible user represented by the `OpenClaw` test boundary.
    ResponsibleUser,
    /// Recovery-only local operator.
    RecoveryOperator,
    /// Visibly fake test fixture.
    TestFixture,
}

impl GatewayActorKind {
    /// Returns the stable representation value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ResponsibleUser => "RESPONSIBLE_USER",
            Self::RecoveryOperator => "RECOVERY_OPERATOR",
            Self::TestFixture => "TEST_FIXTURE",
        }
    }
}

/// Server-derived fake peer context kept outside untrusted gateway bytes.
///
/// This value is deliberately fake-only in TASK-017. It is representation of
/// a test boundary, not proof of operating-system authentication.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GatewayPeerContext {
    client_kind: GatewayClientKind,
    gateway_instance_id: GatewayInstanceId,
    adapter_id: GatewayAdapterId,
    adapter_version: String,
    adapter_binary_digest: ContentDigest,
    schema_digest: ContentDigest,
    actor_id: GatewayActorId,
    actor_kind: GatewayActorKind,
    channel_id: GatewayChannelId,
    session_id: GatewaySessionId,
    session_epoch: u64,
    session_receipt_digest: ContentDigest,
    current_session_head_digest: ContentDigest,
}

impl GatewayPeerContext {
    /// Constructs one visibly fake, currently matching peer context.
    ///
    /// # Errors
    ///
    /// Rejects unsupported client/actor pairs, invalid version text, a zero or
    /// signed-BIGINT-incompatible epoch, zero digests, or receipt/head drift.
    #[allow(clippy::too_many_arguments)]
    pub fn new_fake(
        client_kind: GatewayClientKind,
        gateway_instance_id: GatewayInstanceId,
        adapter_id: GatewayAdapterId,
        adapter_version: impl Into<String>,
        adapter_binary_digest: ContentDigest,
        schema_digest: ContentDigest,
        actor_id: GatewayActorId,
        actor_kind: GatewayActorKind,
        channel_id: GatewayChannelId,
        session_id: GatewaySessionId,
        session_epoch: u64,
        session_receipt_digest: ContentDigest,
        current_session_head_digest: ContentDigest,
    ) -> Result<Self, ContractError> {
        let adapter_version = adapter_version.into();
        let pair_valid = matches!(
            (client_kind, actor_kind),
            (
                GatewayClientKind::OpenClaw,
                GatewayActorKind::ResponsibleUser
            ) | (
                GatewayClientKind::RecoveryCli,
                GatewayActorKind::RecoveryOperator
            ) | (GatewayClientKind::TestFake, GatewayActorKind::TestFixture)
        );
        if !valid_gateway_identifier(&adapter_version) {
            return Err(ContractError::InvalidGatewayValue {
                field: "adapter_version",
            });
        }
        if !pair_valid {
            return Err(ContractError::InvalidGatewayValue {
                field: "client_actor_pair",
            });
        }
        if !(1..=MAX_POSITIVE_SIGNED_BIGINT).contains(&session_epoch) {
            return Err(ContractError::InvalidGatewayValue {
                field: "session_epoch",
            });
        }
        if is_zero_digest(&adapter_binary_digest)
            || is_zero_digest(&schema_digest)
            || is_zero_digest(&session_receipt_digest)
            || session_receipt_digest != current_session_head_digest
        {
            return Err(ContractError::InvalidGatewayValue {
                field: "session_authority",
            });
        }
        Ok(Self {
            client_kind,
            gateway_instance_id,
            adapter_id,
            adapter_version,
            adapter_binary_digest,
            schema_digest,
            actor_id,
            actor_kind,
            channel_id,
            session_id,
            session_epoch,
            session_receipt_digest,
            current_session_head_digest,
        })
    }

    #[must_use]
    pub const fn runtime(&self) -> RuntimeKind {
        RuntimeKind::Fake
    }
    #[must_use]
    pub const fn client_kind(&self) -> GatewayClientKind {
        self.client_kind
    }
    #[must_use]
    pub const fn gateway_instance_id(&self) -> &GatewayInstanceId {
        &self.gateway_instance_id
    }
    #[must_use]
    pub const fn adapter_id(&self) -> &GatewayAdapterId {
        &self.adapter_id
    }
    #[must_use]
    pub fn adapter_version(&self) -> &str {
        &self.adapter_version
    }
    #[must_use]
    pub const fn adapter_binary_digest(&self) -> &ContentDigest {
        &self.adapter_binary_digest
    }
    #[must_use]
    pub const fn schema_digest(&self) -> &ContentDigest {
        &self.schema_digest
    }
    #[must_use]
    pub const fn actor_id(&self) -> &GatewayActorId {
        &self.actor_id
    }
    #[must_use]
    pub const fn actor_kind(&self) -> GatewayActorKind {
        self.actor_kind
    }
    #[must_use]
    pub const fn channel_id(&self) -> &GatewayChannelId {
        &self.channel_id
    }
    #[must_use]
    pub const fn session_id(&self) -> &GatewaySessionId {
        &self.session_id
    }
    #[must_use]
    pub const fn session_epoch(&self) -> u64 {
        self.session_epoch
    }
    #[must_use]
    pub const fn session_receipt_digest(&self) -> &ContentDigest {
        &self.session_receipt_digest
    }
    #[must_use]
    pub const fn current_session_head_digest(&self) -> &ContentDigest {
        &self.current_session_head_digest
    }
}

/// Redacted, bounded canonical Task Spec document carried by Submit.
#[derive(Clone, Eq, PartialEq)]
pub struct TaskSpecSubmission {
    binding: SubjectBinding,
    canonical_document: Vec<u8>,
    claimed_spec_digest: ContentDigest,
}

impl TaskSpecSubmission {
    /// Constructs a bounded representation without claiming domain validity.
    ///
    /// # Errors
    ///
    /// Rejects an empty/oversized document or a claimed digest that differs
    /// from the exact subject binding. The IPC codec performs the hash check.
    pub fn new(
        binding: SubjectBinding,
        canonical_document: Vec<u8>,
        claimed_spec_digest: ContentDigest,
    ) -> Result<Self, ContractError> {
        validate_gateway_subject_binding(&binding)?;
        if canonical_document.is_empty() || canonical_document.len() > GATEWAY_TASK_SPEC_MAX_BYTES {
            return Err(ContractError::InvalidGatewayValue {
                field: "canonical_task_spec_document",
            });
        }
        if binding.task_spec_digest() != &claimed_spec_digest {
            return Err(ContractError::InvalidGatewayValue {
                field: "claimed_spec_digest",
            });
        }
        Ok(Self {
            binding,
            canonical_document,
            claimed_spec_digest,
        })
    }

    #[must_use]
    pub const fn binding(&self) -> &SubjectBinding {
        &self.binding
    }
    #[must_use]
    pub fn canonical_document(&self) -> &[u8] {
        &self.canonical_document
    }
    #[must_use]
    pub const fn claimed_spec_digest(&self) -> &ContentDigest {
        &self.claimed_spec_digest
    }
}

impl fmt::Debug for TaskSpecSubmission {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TaskSpecSubmission")
            .field("binding", &self.binding)
            .field("canonical_document_bytes", &self.canonical_document.len())
            .field("claimed_spec_digest", &self.claimed_spec_digest)
            .finish()
    }
}

/// Exact task subject plus expected Task Ledger head used by read/routing calls.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GatewayTaskTarget {
    binding: SubjectBinding,
    expected_ledger_head_digest: ContentDigest,
}

impl GatewayTaskTarget {
    /// Constructs one bounded target with a non-sentinel owner head.
    ///
    /// # Errors
    ///
    /// Rejects oversized reused identifiers or an all-zero expected head.
    pub fn new(
        binding: SubjectBinding,
        expected_ledger_head_digest: ContentDigest,
    ) -> Result<Self, ContractError> {
        validate_gateway_subject_binding(&binding)?;
        if is_zero_digest(&expected_ledger_head_digest) {
            return Err(ContractError::InvalidGatewayValue {
                field: "expected_ledger_head_digest",
            });
        }
        Ok(Self {
            binding,
            expected_ledger_head_digest,
        })
    }
    #[must_use]
    pub const fn binding(&self) -> &SubjectBinding {
        &self.binding
    }
    #[must_use]
    pub const fn expected_ledger_head_digest(&self) -> &ContentDigest {
        &self.expected_ledger_head_digest
    }
}

/// Bounded project-status page request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GatewayProjectStatusTarget {
    project_id: ProjectId,
    page_size: u16,
    cursor: Option<String>,
}

impl GatewayProjectStatusTarget {
    /// Constructs a bounded project-status request.
    ///
    /// # Errors
    ///
    /// Rejects a zero/oversized page or malformed bounded cursor.
    pub fn new(
        project_id: ProjectId,
        page_size: u16,
        cursor: Option<String>,
    ) -> Result<Self, ContractError> {
        if page_size == 0 || page_size > GATEWAY_STATUS_PAGE_MAX_ITEMS {
            return Err(ContractError::InvalidGatewayValue { field: "page_size" });
        }
        if cursor.as_ref().is_some_and(|value| {
            value.is_empty()
                || value.len() > GATEWAY_CURSOR_MAX_BYTES
                || !value.bytes().all(|byte| byte.is_ascii_graphic())
        }) {
            return Err(ContractError::InvalidGatewayValue { field: "cursor" });
        }
        Ok(Self {
            project_id,
            page_size,
            cursor,
        })
    }
    #[must_use]
    pub const fn project_id(&self) -> &ProjectId {
        &self.project_id
    }
    #[must_use]
    pub const fn page_size(&self) -> u16 {
        self.page_size
    }
    #[must_use]
    pub fn cursor(&self) -> Option<&str> {
        self.cursor.as_deref()
    }
}

/// Closed status target; no arbitrary query language is accepted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GatewayStatusTarget {
    Project(GatewayProjectStatusTarget),
    Task(GatewayTaskTarget),
    Command {
        project_id: ProjectId,
        original_command_id: GatewayCommandId,
    },
}

impl GatewayStatusTarget {
    #[must_use]
    pub const fn project_id(&self) -> &ProjectId {
        match self {
            Self::Project(target) => target.project_id(),
            Self::Task(target) => target.binding().project_id(),
            Self::Command { project_id, .. } => project_id,
        }
    }
}

/// Normal approval kinds available to the normal gateway.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GatewayNormalApprovalKind {
    Execution,
    Merge,
    Preference,
    ProtectedChange,
}

impl GatewayNormalApprovalKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Execution => "EXECUTION",
            Self::Merge => "MERGE",
            Self::Preference => "PREFERENCE",
            Self::ProtectedChange => "PROTECTED_CHANGE",
        }
    }
}

/// Exact normal approval challenge/presentation routing reference.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GatewayApprovalRoute {
    binding: SubjectBinding,
    kind: GatewayNormalApprovalKind,
    approval_id: GatewayApprovalId,
    challenge_id: GatewayChallengeId,
    subject_digest: ContentDigest,
    challenge_digest: ContentDigest,
    presentation_digest: ContentDigest,
}

impl GatewayApprovalRoute {
    #[allow(clippy::too_many_arguments)]
    /// Constructs one bounded normal approval routing reference.
    ///
    /// # Errors
    ///
    /// Rejects oversized reused identifiers or zero authority digests.
    pub fn new(
        binding: SubjectBinding,
        kind: GatewayNormalApprovalKind,
        approval_id: GatewayApprovalId,
        challenge_id: GatewayChallengeId,
        subject_digest: ContentDigest,
        challenge_digest: ContentDigest,
        presentation_digest: ContentDigest,
    ) -> Result<Self, ContractError> {
        validate_gateway_subject_binding(&binding)?;
        for (field, digest) in [
            ("subject_digest", &subject_digest),
            ("challenge_digest", &challenge_digest),
            ("presentation_digest", &presentation_digest),
        ] {
            if is_zero_digest(digest) {
                return Err(ContractError::InvalidGatewayValue { field });
            }
        }
        Ok(Self {
            binding,
            kind,
            approval_id,
            challenge_id,
            subject_digest,
            challenge_digest,
            presentation_digest,
        })
    }
    #[must_use]
    pub const fn binding(&self) -> &SubjectBinding {
        &self.binding
    }
    #[must_use]
    pub const fn kind(&self) -> GatewayNormalApprovalKind {
        self.kind
    }
    #[must_use]
    pub const fn approval_id(&self) -> &GatewayApprovalId {
        &self.approval_id
    }
    #[must_use]
    pub const fn challenge_id(&self) -> &GatewayChallengeId {
        &self.challenge_id
    }
    #[must_use]
    pub const fn subject_digest(&self) -> &ContentDigest {
        &self.subject_digest
    }
    #[must_use]
    pub const fn challenge_digest(&self) -> &ContentDigest {
        &self.challenge_digest
    }
    #[must_use]
    pub const fn presentation_digest(&self) -> &ContentDigest {
        &self.presentation_digest
    }
}

/// Closed reason for requesting one exact task stop.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GatewayStopReason {
    UserRequested,
    Superseded,
    SafetyConcern,
}

impl GatewayStopReason {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UserRequested => "USER_REQUESTED",
            Self::Superseded => "SUPERSEDED",
            Self::SafetyConcern => "SAFETY_CONCERN",
        }
    }
}

/// Exact task/attempt stop target. It grants no process or lease authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GatewayStopTarget {
    target: GatewayTaskTarget,
    attempt_id: AttemptId,
    reason: GatewayStopReason,
}

impl GatewayStopTarget {
    /// Constructs one bounded task-stop target.
    ///
    /// # Errors
    ///
    /// Rejects an oversized attempt identity.
    pub fn new(
        target: GatewayTaskTarget,
        attempt_id: AttemptId,
        reason: GatewayStopReason,
    ) -> Result<Self, ContractError> {
        if attempt_id.as_str().len() > GATEWAY_IDENTIFIER_MAX_BYTES {
            return Err(ContractError::InvalidGatewayValue {
                field: "attempt_id",
            });
        }
        Ok(Self {
            target,
            attempt_id,
            reason,
        })
    }
    #[must_use]
    pub const fn target(&self) -> &GatewayTaskTarget {
        &self.target
    }
    #[must_use]
    pub const fn attempt_id(&self) -> &AttemptId {
        &self.attempt_id
    }
    #[must_use]
    pub const fn reason(&self) -> GatewayStopReason {
        self.reason
    }
}

/// Six sealed action-specific normal gateway requests.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GatewayRequestBody {
    Submit(TaskSpecSubmission),
    Plan(GatewayTaskTarget),
    Status(GatewayStatusTarget),
    Approve(GatewayApprovalRoute),
    Reject(GatewayApprovalRoute),
    Stop(GatewayStopTarget),
}

impl GatewayRequestBody {
    #[must_use]
    pub const fn action(&self) -> GatewayAction {
        match self {
            Self::Submit(_) => GatewayAction::Submit,
            Self::Plan(_) => GatewayAction::Plan,
            Self::Status(_) => GatewayAction::Status,
            Self::Approve(_) => GatewayAction::Approve,
            Self::Reject(_) => GatewayAction::Reject,
            Self::Stop(_) => GatewayAction::Stop,
        }
    }

    #[must_use]
    pub const fn project_id(&self) -> &ProjectId {
        match self {
            Self::Submit(value) => value.binding().project_id(),
            Self::Plan(value) => value.binding().project_id(),
            Self::Status(value) => value.project_id(),
            Self::Approve(value) | Self::Reject(value) => value.binding().project_id(),
            Self::Stop(value) => value.target().binding().project_id(),
        }
    }
}

/// Complete immutable typed request after mechanical codec verification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GatewayRequest {
    version: u16,
    command_id: GatewayCommandId,
    correlation_id: GatewayCorrelationId,
    body: GatewayRequestBody,
    request_digest: ContentDigest,
}

impl GatewayRequest {
    /// Constructs one represented request; the codec must verify its digest.
    ///
    /// # Errors
    ///
    /// Rejects an unsupported protocol version or the zero digest sentinel.
    pub fn new(
        version: u16,
        command_id: GatewayCommandId,
        correlation_id: GatewayCorrelationId,
        body: GatewayRequestBody,
        request_digest: ContentDigest,
    ) -> Result<Self, ContractError> {
        if version != GATEWAY_PROTOCOL_VERSION {
            return Err(ContractError::UnsupportedGatewayProtocolVersion);
        }
        if is_zero_digest(&request_digest) {
            return Err(ContractError::InvalidGatewayValue {
                field: "request_digest",
            });
        }
        Ok(Self {
            version,
            command_id,
            correlation_id,
            body,
            request_digest,
        })
    }
    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }
    #[must_use]
    pub const fn command_id(&self) -> &GatewayCommandId {
        &self.command_id
    }
    #[must_use]
    pub const fn correlation_id(&self) -> &GatewayCorrelationId {
        &self.correlation_id
    }
    #[must_use]
    pub const fn action(&self) -> GatewayAction {
        self.body.action()
    }
    #[must_use]
    pub const fn project_id(&self) -> &ProjectId {
        self.body.project_id()
    }
    #[must_use]
    pub const fn body(&self) -> &GatewayRequestBody {
        &self.body
    }
    #[must_use]
    pub const fn request_digest(&self) -> &ContentDigest {
        &self.request_digest
    }
}

/// Closed gateway-facing task state projection; it owns no transition rules.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GatewayTaskState {
    Draft,
    AwaitingExecutionApproval,
    Preparing,
    Executing,
    Verifying,
    Reviewing,
    AwaitingMergeApproval,
    Merging,
    Completed,
    Rejected,
    Blocked,
    Failed,
    Stopping,
    Cancelled,
}

impl GatewayTaskState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "DRAFT",
            Self::AwaitingExecutionApproval => "AWAITING_EXECUTION_APPROVAL",
            Self::Preparing => "PREPARING",
            Self::Executing => "EXECUTING",
            Self::Verifying => "VERIFYING",
            Self::Reviewing => "REVIEWING",
            Self::AwaitingMergeApproval => "AWAITING_MERGE_APPROVAL",
            Self::Merging => "MERGING",
            Self::Completed => "COMPLETED",
            Self::Rejected => "REJECTED",
            Self::Blocked => "BLOCKED",
            Self::Failed => "FAILED",
            Self::Stopping => "STOPPING",
            Self::Cancelled => "CANCELLED",
        }
    }
}

/// One bounded status observation returned by Orchestrator through the gateway.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GatewayTaskProjection {
    binding: SubjectBinding,
    state: GatewayTaskState,
    ledger_head_digest: ContentDigest,
    observation_receipt_digest: ContentDigest,
}

impl GatewayTaskProjection {
    /// Constructs one bounded owner-evidence projection.
    ///
    /// # Errors
    ///
    /// Rejects oversized reused identifiers or zero evidence digests.
    pub fn new(
        binding: SubjectBinding,
        state: GatewayTaskState,
        ledger_head_digest: ContentDigest,
        observation_receipt_digest: ContentDigest,
    ) -> Result<Self, ContractError> {
        validate_gateway_subject_binding(&binding)?;
        if is_zero_digest(&ledger_head_digest) || is_zero_digest(&observation_receipt_digest) {
            return Err(ContractError::InvalidGatewayValue {
                field: "task_projection_digest",
            });
        }
        Ok(Self {
            binding,
            state,
            ledger_head_digest,
            observation_receipt_digest,
        })
    }
    #[must_use]
    pub const fn binding(&self) -> &SubjectBinding {
        &self.binding
    }
    #[must_use]
    pub const fn state(&self) -> GatewayTaskState {
        self.state
    }
    #[must_use]
    pub const fn ledger_head_digest(&self) -> &ContentDigest {
        &self.ledger_head_digest
    }
    #[must_use]
    pub const fn observation_receipt_digest(&self) -> &ContentDigest {
        &self.observation_receipt_digest
    }
}

/// Closed status observation form.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GatewayStatusObservation {
    Project {
        project_id: ProjectId,
        tasks: Vec<GatewayTaskProjection>,
        next_cursor: Option<String>,
    },
    Task(GatewayTaskProjection),
    Command {
        project_id: ProjectId,
        original_command_id: GatewayCommandId,
        terminal_reply_digest: ContentDigest,
    },
}

/// Safe routing result for a normal approval/rejection request.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GatewayApprovalDisposition {
    RoutedForVerification,
    RejectionRecorded,
}

impl GatewayApprovalDisposition {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RoutedForVerification => "ROUTED_FOR_VERIFICATION",
            Self::RejectionRecorded => "REJECTION_RECORDED",
        }
    }
}

/// Safe task-stop routing result.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GatewayStopDisposition {
    Requested,
    AlreadyTerminal,
    ReconciliationRequired,
}

impl GatewayStopDisposition {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Requested => "REQUESTED",
            Self::AlreadyTerminal => "ALREADY_TERMINAL",
            Self::ReconciliationRequired => "RECONCILIATION_REQUIRED",
        }
    }
}

/// Stable fail-closed business denial returned for a valid frame.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GatewayDenialCode {
    ScopeDenied,
    SessionNotCurrent,
    RoleDenied,
    ProtectedSurfaceRequired,
    CommandSubstitution,
    MalformedSubject,
    DownstreamDenied,
}

impl GatewayDenialCode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ScopeDenied => "SCOPE_DENIED",
            Self::SessionNotCurrent => "SESSION_NOT_CURRENT",
            Self::RoleDenied => "ROLE_DENIED",
            Self::ProtectedSurfaceRequired => "PROTECTED_SURFACE_REQUIRED",
            Self::CommandSubstitution => "COMMAND_SUBSTITUTION",
            Self::MalformedSubject => "MALFORMED_SUBJECT",
            Self::DownstreamDenied => "DOWNSTREAM_DENIED",
        }
    }
}

/// Stable unknown-outcome classification; it never implies success.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GatewayUnknownCode {
    DownstreamAmbiguous,
    ReconciliationRequired,
}

impl GatewayUnknownCode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DownstreamAmbiguous => "DOWNSTREAM_AMBIGUOUS",
            Self::ReconciliationRequired => "RECONCILIATION_REQUIRED",
        }
    }
}

/// Closed typed reply body returned by the Rust gateway service.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GatewayReplyBody {
    SubmitAccepted {
        binding: SubjectBinding,
        command_receipt_digest: ContentDigest,
    },
    PlanRouted {
        binding: SubjectBinding,
        command_receipt_digest: ContentDigest,
    },
    StatusObserved(GatewayStatusObservation),
    ApprovalRouted {
        binding: SubjectBinding,
        approval_id: GatewayApprovalId,
        challenge_id: GatewayChallengeId,
        challenge_digest: ContentDigest,
        disposition: GatewayApprovalDisposition,
        routing_receipt_digest: ContentDigest,
    },
    StopRouted {
        target: GatewayStopTarget,
        disposition: GatewayStopDisposition,
        routing_receipt_digest: ContentDigest,
    },
    Denied(GatewayDenialCode),
    UnknownOutcome(GatewayUnknownCode),
}

impl GatewayReplyBody {
    fn accepted_action(&self) -> Option<GatewayAction> {
        match self {
            Self::SubmitAccepted { .. } => Some(GatewayAction::Submit),
            Self::PlanRouted { .. } => Some(GatewayAction::Plan),
            Self::StatusObserved(_) => Some(GatewayAction::Status),
            Self::ApprovalRouted { disposition, .. } => Some(match disposition {
                GatewayApprovalDisposition::RoutedForVerification => GatewayAction::Approve,
                GatewayApprovalDisposition::RejectionRecorded => GatewayAction::Reject,
            }),
            Self::StopRouted { .. } => Some(GatewayAction::Stop),
            Self::Denied(_) | Self::UnknownOutcome(_) => None,
        }
    }
}

fn gateway_reply_matches_request(request: &GatewayRequestBody, reply: &GatewayReplyBody) -> bool {
    match (request, reply) {
        (GatewayRequestBody::Submit(request), GatewayReplyBody::SubmitAccepted { binding, .. }) => {
            request.binding() == binding
        }
        (GatewayRequestBody::Plan(request), GatewayReplyBody::PlanRouted { binding, .. }) => {
            request.binding() == binding
        }
        (
            GatewayRequestBody::Status(GatewayStatusTarget::Task(request)),
            GatewayReplyBody::StatusObserved(GatewayStatusObservation::Task(reply)),
        ) => request.binding() == reply.binding(),
        (
            GatewayRequestBody::Status(GatewayStatusTarget::Project(request)),
            GatewayReplyBody::StatusObserved(GatewayStatusObservation::Project {
                project_id,
                tasks,
                next_cursor,
            }),
        ) => {
            request.project_id() == project_id
                && tasks.len() <= usize::from(request.page_size())
                && tasks
                    .iter()
                    .all(|task| task.binding().project_id() == project_id)
                && next_cursor.as_ref().is_none_or(|cursor| {
                    !cursor.is_empty()
                        && cursor.len() <= GATEWAY_CURSOR_MAX_BYTES
                        && cursor.bytes().all(|byte| byte.is_ascii_graphic())
                })
        }
        (
            GatewayRequestBody::Status(GatewayStatusTarget::Command {
                project_id: request_project,
                original_command_id: request_command,
            }),
            GatewayReplyBody::StatusObserved(GatewayStatusObservation::Command {
                project_id: reply_project,
                original_command_id: reply_command,
                ..
            }),
        ) => request_project == reply_project && request_command == reply_command,
        (
            GatewayRequestBody::Approve(request),
            GatewayReplyBody::ApprovalRouted {
                binding,
                approval_id,
                challenge_id,
                challenge_digest,
                disposition: GatewayApprovalDisposition::RoutedForVerification,
                ..
            },
        )
        | (
            GatewayRequestBody::Reject(request),
            GatewayReplyBody::ApprovalRouted {
                binding,
                approval_id,
                challenge_id,
                challenge_digest,
                disposition: GatewayApprovalDisposition::RejectionRecorded,
                ..
            },
        ) => {
            request.binding() == binding
                && request.approval_id() == approval_id
                && request.challenge_id() == challenge_id
                && request.challenge_digest() == challenge_digest
        }
        (GatewayRequestBody::Stop(request), GatewayReplyBody::StopRouted { target, .. }) => {
            request == target
        }
        (_, GatewayReplyBody::Denied(_) | GatewayReplyBody::UnknownOutcome(_)) => true,
        _ => false,
    }
}

/// Complete reply bound to the exact request and a mechanically checked digest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GatewayReply {
    version: u16,
    command_id: GatewayCommandId,
    correlation_id: GatewayCorrelationId,
    action: GatewayAction,
    request_digest: ContentDigest,
    body: GatewayReplyBody,
    reply_digest: ContentDigest,
}

impl GatewayReply {
    /// Validates an action-specific reply body before hashing or allocation.
    ///
    /// # Errors
    ///
    /// Rejects action/subject drift, oversized pages, or zero evidence digests.
    pub fn validate_body(
        request: &GatewayRequest,
        body: &GatewayReplyBody,
    ) -> Result<(), ContractError> {
        if body
            .accepted_action()
            .is_some_and(|action| action != request.action())
            || !gateway_reply_matches_request(request.body(), body)
        {
            return Err(ContractError::GatewayReplyActionMismatch);
        }
        let zero = match body {
            GatewayReplyBody::SubmitAccepted {
                command_receipt_digest,
                ..
            }
            | GatewayReplyBody::PlanRouted {
                command_receipt_digest,
                ..
            } => is_zero_digest(command_receipt_digest),
            GatewayReplyBody::StatusObserved(GatewayStatusObservation::Command {
                terminal_reply_digest,
                ..
            }) => is_zero_digest(terminal_reply_digest),
            GatewayReplyBody::ApprovalRouted {
                challenge_digest,
                routing_receipt_digest,
                ..
            } => is_zero_digest(challenge_digest) || is_zero_digest(routing_receipt_digest),
            GatewayReplyBody::StopRouted {
                routing_receipt_digest,
                ..
            } => is_zero_digest(routing_receipt_digest),
            GatewayReplyBody::StatusObserved(_)
            | GatewayReplyBody::Denied(_)
            | GatewayReplyBody::UnknownOutcome(_) => false,
        };
        if zero {
            return Err(ContractError::InvalidGatewayValue {
                field: "reply_evidence_digest",
            });
        }
        Ok(())
    }

    /// Constructs a typed reply bound to one request.
    ///
    /// # Errors
    ///
    /// Rejects an action/subject mismatch or the zero digest sentinel.
    pub fn new(
        request: &GatewayRequest,
        body: GatewayReplyBody,
        reply_digest: ContentDigest,
    ) -> Result<Self, ContractError> {
        Self::validate_body(request, &body)?;
        if is_zero_digest(&reply_digest) {
            return Err(ContractError::InvalidGatewayValue {
                field: "reply_digest",
            });
        }
        Ok(Self {
            version: request.version,
            command_id: request.command_id.clone(),
            correlation_id: request.correlation_id.clone(),
            action: request.action(),
            request_digest: request.request_digest.clone(),
            body,
            reply_digest,
        })
    }
    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }
    #[must_use]
    pub const fn command_id(&self) -> &GatewayCommandId {
        &self.command_id
    }
    #[must_use]
    pub const fn correlation_id(&self) -> &GatewayCorrelationId {
        &self.correlation_id
    }
    #[must_use]
    pub const fn action(&self) -> GatewayAction {
        self.action
    }
    #[must_use]
    pub const fn request_digest(&self) -> &ContentDigest {
        &self.request_digest
    }
    #[must_use]
    pub const fn body(&self) -> &GatewayReplyBody {
        &self.body
    }
    #[must_use]
    pub const fn reply_digest(&self) -> &ContentDigest {
        &self.reply_digest
    }
}

fn valid_store_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= STORE_IDENTIFIER_MAX_BYTES
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'.' | b'_' | b'-' | b':')
        })
}

fn valid_store_daemon_identifier(value: &str) -> bool {
    valid_store_identifier(value)
        && value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
}

macro_rules! store_identifier {
    ($name:ident, $field:literal, $description:literal, $validator:ident) => {
        #[doc = $description]
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);

        impl $name {
            /// Constructs one bounded canonical ASCII Store identifier.
            ///
            /// # Errors
            ///
            /// Rejects empty, oversized, uppercase, non-ASCII, whitespace, or
            /// punctuation outside the closed safe identifier alphabet.
            pub fn new(value: impl Into<String>) -> Result<Self, ContractError> {
                let value = value.into();
                if !$validator(&value) {
                    return Err(ContractError::InvalidStoreValue { field: $field });
                }
                Ok(Self(value))
            }

            /// Returns the canonical identifier text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

store_identifier!(
    StoreTransactionId,
    "store_transaction_id",
    "Globally unique idempotency identity of one physical Store transaction.",
    valid_store_identifier
);
store_identifier!(
    StoreDaemonInstanceId,
    "store_daemon_instance_id",
    "Bounded identity of the daemon represented by a Store authority head.",
    valid_store_identifier
);

/// Closed semantic owner whose approved commitments may be physically stored.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum StoreRepositoryOwner {
    ProjectRegistry,
    TaskLedger,
    WriterLease,
    ApprovalVerifier,
    ArtifactStore,
}

impl StoreRepositoryOwner {
    /// Complete initial owner set; callers cannot supply an arbitrary table.
    pub const ALL: [Self; 5] = [
        Self::ProjectRegistry,
        Self::TaskLedger,
        Self::WriterLease,
        Self::ApprovalVerifier,
        Self::ArtifactStore,
    ];

    /// Returns the stable hash/receipt-facing value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProjectRegistry => "PROJECT_REGISTRY",
            Self::TaskLedger => "TASK_LEDGER",
            Self::WriterLease => "WRITER_LEASE",
            Self::ApprovalVerifier => "APPROVAL_VERIFIER",
            Self::ArtifactStore => "ARTIFACT_STORE",
        }
    }
}

/// Exact project/snapshot/owner/aggregate address for one physical head.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct StoreScope {
    project_id: ProjectId,
    project_snapshot_id: ProjectSnapshotId,
    owner: StoreRepositoryOwner,
    aggregate_key_digest: ContentDigest,
}

impl StoreScope {
    /// Constructs one closed physical Store scope.
    ///
    /// # Errors
    ///
    /// Rejects a snapshot outside the canonical 128-byte Store identifier
    /// alphabet or an all-zero aggregate-key commitment.
    pub fn new(
        project_id: ProjectId,
        project_snapshot_id: ProjectSnapshotId,
        owner: StoreRepositoryOwner,
        aggregate_key_digest: ContentDigest,
    ) -> Result<Self, ContractError> {
        if !valid_store_identifier(project_snapshot_id.as_str()) {
            return Err(ContractError::InvalidStoreValue {
                field: "project_snapshot_id",
            });
        }
        if is_zero_digest(&aggregate_key_digest) {
            return Err(ContractError::InvalidStoreValue {
                field: "aggregate_key_digest",
            });
        }
        Ok(Self {
            project_id,
            project_snapshot_id,
            owner,
            aggregate_key_digest,
        })
    }

    #[must_use]
    pub const fn project_id(&self) -> &ProjectId {
        &self.project_id
    }

    #[must_use]
    pub const fn project_snapshot_id(&self) -> &ProjectSnapshotId {
        &self.project_snapshot_id
    }

    #[must_use]
    pub const fn owner(&self) -> StoreRepositoryOwner {
        self.owner
    }

    #[must_use]
    pub const fn aggregate_key_digest(&self) -> &ContentDigest {
        &self.aggregate_key_digest
    }
}

/// Complete independently retained daemon authority expected by a transaction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoreAuthorityHead {
    runtime: RuntimeKind,
    daemon_instance_id: StoreDaemonInstanceId,
    daemon_epoch: DaemonEpoch,
    admission: RuntimeAdmissionMode,
    revision: StoreAuthorityRevision,
    observation_digest: ContentDigest,
    head_digest: ContentDigest,
}

impl StoreAuthorityHead {
    /// Constructs one complete neutral daemon authority head.
    ///
    /// # Errors
    ///
    /// Rejects zero observation or head commitments. Live authorities also
    /// require a daemon identifier whose first byte is ASCII alphanumeric so
    /// it is representable by the `PostgreSQL` v2 schema without narrowing the
    /// pre-existing fake-runtime identifier contract.
    pub fn new(
        runtime: RuntimeKind,
        daemon_instance_id: StoreDaemonInstanceId,
        daemon_epoch: DaemonEpoch,
        admission: RuntimeAdmissionMode,
        revision: StoreAuthorityRevision,
        observation_digest: ContentDigest,
        head_digest: ContentDigest,
    ) -> Result<Self, ContractError> {
        if runtime == RuntimeKind::Live
            && !valid_store_daemon_identifier(daemon_instance_id.as_str())
        {
            return Err(ContractError::InvalidStoreValue {
                field: "store_daemon_instance_id",
            });
        }
        if is_zero_digest(&observation_digest) {
            return Err(ContractError::InvalidStoreValue {
                field: "store_authority_observation_digest",
            });
        }
        if is_zero_digest(&head_digest) {
            return Err(ContractError::InvalidStoreValue {
                field: "store_authority_head_digest",
            });
        }
        Ok(Self {
            runtime,
            daemon_instance_id,
            daemon_epoch,
            admission,
            revision,
            observation_digest,
            head_digest,
        })
    }

    #[must_use]
    pub const fn runtime(&self) -> RuntimeKind {
        self.runtime
    }

    #[must_use]
    pub const fn daemon_instance_id(&self) -> &StoreDaemonInstanceId {
        &self.daemon_instance_id
    }

    #[must_use]
    pub const fn daemon_epoch(&self) -> DaemonEpoch {
        self.daemon_epoch
    }

    #[must_use]
    pub const fn admission(&self) -> RuntimeAdmissionMode {
        self.admission
    }

    #[must_use]
    pub const fn revision(&self) -> StoreAuthorityRevision {
        self.revision
    }

    #[must_use]
    pub const fn observation_digest(&self) -> &ContentDigest {
        &self.observation_digest
    }

    #[must_use]
    pub const fn head_digest(&self) -> &ContentDigest {
        &self.head_digest
    }
}

/// Complete physical compare-and-swap head for one Store scope.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StorePhysicalHead {
    runtime: RuntimeKind,
    scope: StoreScope,
    revision: StoreRevision,
    state_digest: ContentDigest,
    head_digest: ContentDigest,
}

impl StorePhysicalHead {
    /// Constructs one neutral physical head without claiming currentness.
    ///
    /// # Errors
    ///
    /// Rejects zero state or head commitments.
    pub fn new(
        runtime: RuntimeKind,
        scope: StoreScope,
        revision: StoreRevision,
        state_digest: ContentDigest,
        head_digest: ContentDigest,
    ) -> Result<Self, ContractError> {
        if is_zero_digest(&state_digest) {
            return Err(ContractError::InvalidStoreValue {
                field: "store_state_digest",
            });
        }
        if is_zero_digest(&head_digest) {
            return Err(ContractError::InvalidStoreValue {
                field: "store_physical_head_digest",
            });
        }
        Ok(Self {
            runtime,
            scope,
            revision,
            state_digest,
            head_digest,
        })
    }

    #[must_use]
    pub const fn runtime(&self) -> RuntimeKind {
        self.runtime
    }

    #[must_use]
    pub const fn scope(&self) -> &StoreScope {
        &self.scope
    }

    #[must_use]
    pub const fn revision(&self) -> StoreRevision {
        self.revision
    }

    #[must_use]
    pub const fn state_digest(&self) -> &ContentDigest {
        &self.state_digest
    }

    #[must_use]
    pub const fn head_digest(&self) -> &ContentDigest {
        &self.head_digest
    }
}

/// Opaque commitments produced by one domain-approved physical mutation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoreMutationCommitment {
    domain_command: ContentDigest,
    record_set: ContentDigest,
    next_state: ContentDigest,
    domain_receipt: ContentDigest,
    checkpoint: Option<ContentDigest>,
    outbox_intent: Option<ContentDigest>,
}

impl StoreMutationCommitment {
    /// Constructs one payload-free mutation commitment.
    ///
    /// # Errors
    ///
    /// Rejects any all-zero mandatory or optional commitment.
    pub fn new(
        domain_command_digest: ContentDigest,
        record_set_digest: ContentDigest,
        next_state_digest: ContentDigest,
        domain_receipt_digest: ContentDigest,
        checkpoint_digest: Option<ContentDigest>,
        outbox_intent_digest: Option<ContentDigest>,
    ) -> Result<Self, ContractError> {
        for (field, digest) in [
            ("domain_command_digest", &domain_command_digest),
            ("record_set_digest", &record_set_digest),
            ("next_state_digest", &next_state_digest),
            ("domain_receipt_digest", &domain_receipt_digest),
        ] {
            if is_zero_digest(digest) {
                return Err(ContractError::InvalidStoreValue { field });
            }
        }
        if checkpoint_digest.as_ref().is_some_and(is_zero_digest) {
            return Err(ContractError::InvalidStoreValue {
                field: "checkpoint_digest",
            });
        }
        if outbox_intent_digest.as_ref().is_some_and(is_zero_digest) {
            return Err(ContractError::InvalidStoreValue {
                field: "outbox_intent_digest",
            });
        }
        Ok(Self {
            domain_command: domain_command_digest,
            record_set: record_set_digest,
            next_state: next_state_digest,
            domain_receipt: domain_receipt_digest,
            checkpoint: checkpoint_digest,
            outbox_intent: outbox_intent_digest,
        })
    }

    #[must_use]
    pub const fn domain_command_digest(&self) -> &ContentDigest {
        &self.domain_command
    }

    #[must_use]
    pub const fn record_set_digest(&self) -> &ContentDigest {
        &self.record_set
    }

    #[must_use]
    pub const fn next_state_digest(&self) -> &ContentDigest {
        &self.next_state
    }

    #[must_use]
    pub const fn domain_receipt_digest(&self) -> &ContentDigest {
        &self.domain_receipt
    }

    #[must_use]
    pub const fn checkpoint_digest(&self) -> Option<&ContentDigest> {
        self.checkpoint.as_ref()
    }

    #[must_use]
    pub const fn outbox_intent_digest(&self) -> Option<&ContentDigest> {
        self.outbox_intent.as_ref()
    }
}

/// Complete typed request for one physical Store transaction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoreTransactionRequest {
    version: u16,
    transaction_id: StoreTransactionId,
    scope: StoreScope,
    expected_authority: StoreAuthorityHead,
    expected_head: StorePhysicalHead,
    mutation: StoreMutationCommitment,
}

impl StoreTransactionRequest {
    /// Constructs one complete request after scope/runtime consistency checks.
    ///
    /// # Errors
    ///
    /// Rejects unknown versions, mixed scope, or mixed runtime evidence.
    pub fn new(
        version: u16,
        transaction_id: StoreTransactionId,
        scope: StoreScope,
        expected_authority: StoreAuthorityHead,
        expected_head: StorePhysicalHead,
        mutation: StoreMutationCommitment,
    ) -> Result<Self, ContractError> {
        if !matches!(version, STORE_CONTRACT_VERSION_V1 | STORE_CONTRACT_VERSION) {
            return Err(ContractError::UnsupportedStoreContractVersion);
        }
        if expected_head.scope() != &scope {
            return Err(ContractError::StoreScopeMismatch);
        }
        if expected_authority.runtime() != expected_head.runtime() {
            return Err(ContractError::StoreRuntimeMismatch);
        }
        if version == STORE_CONTRACT_VERSION_V1 && expected_authority.runtime() != RuntimeKind::Fake
        {
            return Err(ContractError::UnsupportedStoreContractVersion);
        }
        Ok(Self {
            version,
            transaction_id,
            scope,
            expected_authority,
            expected_head,
            mutation,
        })
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }

    #[must_use]
    pub const fn transaction_id(&self) -> &StoreTransactionId {
        &self.transaction_id
    }

    #[must_use]
    pub const fn scope(&self) -> &StoreScope {
        &self.scope
    }

    #[must_use]
    pub const fn expected_authority(&self) -> &StoreAuthorityHead {
        &self.expected_authority
    }

    #[must_use]
    pub const fn expected_head(&self) -> &StorePhysicalHead {
        &self.expected_head
    }

    #[must_use]
    pub const fn mutation(&self) -> &StoreMutationCommitment {
        &self.mutation
    }
}

/// Immutable database/schema evidence required by a durable Store receipt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StorePersistenceEvidence {
    database_identity_digest: ContentDigest,
    schema_version: u16,
    manifest_digest: ContentDigest,
}

impl StorePersistenceEvidence {
    /// Constructs complete `PostgreSQL` persistence evidence.
    ///
    /// # Errors
    ///
    /// Rejects zero commitments or a schema version outside positive signed
    /// `PostgreSQL` `SMALLINT`.
    pub fn new(
        database_identity_digest: ContentDigest,
        schema_version: u16,
        manifest_digest: ContentDigest,
    ) -> Result<Self, ContractError> {
        if is_zero_digest(&database_identity_digest) {
            return Err(ContractError::InvalidStoreValue {
                field: "store_database_identity_digest",
            });
        }
        if schema_version == 0 || schema_version > i16::MAX as u16 {
            return Err(ContractError::InvalidStoreValue {
                field: "store_schema_version",
            });
        }
        if is_zero_digest(&manifest_digest) {
            return Err(ContractError::InvalidStoreValue {
                field: "store_manifest_digest",
            });
        }
        Ok(Self {
            database_identity_digest,
            schema_version,
            manifest_digest,
        })
    }

    #[must_use]
    pub const fn database_identity_digest(&self) -> &ContentDigest {
        &self.database_identity_digest
    }

    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    #[must_use]
    pub const fn manifest_digest(&self) -> &ContentDigest {
        &self.manifest_digest
    }
}

/// Terminal physical Store disposition retained for exact replay.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum StoreReceiptDisposition {
    Applied,
    StalePhysicalHead,
}

impl StoreReceiptDisposition {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Applied => "APPLIED",
            Self::StalePhysicalHead => "STALE_PHYSICAL_HEAD",
        }
    }
}

/// Explicit physical Store durability classification.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum StoreDurability {
    NonDurableFake,
    DurablePostgres,
}

impl StoreDurability {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NonDurableFake => "NON_DURABLE_FAKE",
            Self::DurablePostgres => "DURABLE_POSTGRES",
        }
    }
}

/// Complete terminal physical Store receipt; never domain authority by itself.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoreTransactionReceipt {
    runtime: RuntimeKind,
    durability: StoreDurability,
    persistence: Option<StorePersistenceEvidence>,
    request: StoreTransactionRequest,
    request_digest: ContentDigest,
    before_head: StorePhysicalHead,
    after_head: StorePhysicalHead,
    disposition: StoreReceiptDisposition,
    transaction_digest: ContentDigest,
    receipt_digest: ContentDigest,
}

impl StoreTransactionReceipt {
    /// Constructs a visibly non-durable fake terminal receipt.
    ///
    /// # Errors
    ///
    /// Rejects live/mixed runtime, scope drift, zero digests, or a disposition
    /// whose before/after heads do not describe exactly one apply or no stale
    /// mutation.
    pub fn new_non_durable_fake(
        request: StoreTransactionRequest,
        request_digest: ContentDigest,
        before_head: StorePhysicalHead,
        after_head: StorePhysicalHead,
        disposition: StoreReceiptDisposition,
        transaction_digest: ContentDigest,
        receipt_digest: ContentDigest,
    ) -> Result<Self, ContractError> {
        Self::new_terminal(
            RuntimeKind::Fake,
            StoreDurability::NonDurableFake,
            None,
            request,
            request_digest,
            before_head,
            after_head,
            disposition,
            transaction_digest,
            receipt_digest,
        )
    }

    /// Constructs a structurally complete live `PostgreSQL` terminal receipt.
    ///
    /// # Errors
    ///
    /// Rejects Store v1, fake/mixed runtime, missing persistence evidence,
    /// scope drift, zero digests, or an invalid applied/stale transition.
    #[allow(clippy::too_many_arguments)]
    pub fn new_durable_postgres(
        request: StoreTransactionRequest,
        persistence: StorePersistenceEvidence,
        request_digest: ContentDigest,
        before_head: StorePhysicalHead,
        after_head: StorePhysicalHead,
        disposition: StoreReceiptDisposition,
        transaction_digest: ContentDigest,
        receipt_digest: ContentDigest,
    ) -> Result<Self, ContractError> {
        if request.version() != STORE_CONTRACT_VERSION {
            return Err(ContractError::UnsupportedStoreContractVersion);
        }
        Self::new_terminal(
            RuntimeKind::Live,
            StoreDurability::DurablePostgres,
            Some(persistence),
            request,
            request_digest,
            before_head,
            after_head,
            disposition,
            transaction_digest,
            receipt_digest,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn new_terminal(
        runtime: RuntimeKind,
        durability: StoreDurability,
        persistence: Option<StorePersistenceEvidence>,
        request: StoreTransactionRequest,
        request_digest: ContentDigest,
        before_head: StorePhysicalHead,
        after_head: StorePhysicalHead,
        disposition: StoreReceiptDisposition,
        transaction_digest: ContentDigest,
        receipt_digest: ContentDigest,
    ) -> Result<Self, ContractError> {
        let valid_evidence = matches!(
            (runtime, durability, persistence.as_ref()),
            (RuntimeKind::Fake, StoreDurability::NonDurableFake, None)
                | (RuntimeKind::Live, StoreDurability::DurablePostgres, Some(_))
        );
        if !valid_evidence
            || request.expected_authority().runtime() != runtime
            || request.expected_head().runtime() != runtime
            || before_head.runtime() != runtime
            || after_head.runtime() != runtime
        {
            return Err(ContractError::StoreRuntimeMismatch);
        }
        if before_head.scope() != request.scope() || after_head.scope() != request.scope() {
            return Err(ContractError::StoreScopeMismatch);
        }
        for (field, digest) in [
            ("store_request_digest", &request_digest),
            ("store_transaction_digest", &transaction_digest),
            ("store_receipt_digest", &receipt_digest),
        ] {
            if is_zero_digest(digest) {
                return Err(ContractError::InvalidStoreValue { field });
            }
        }
        let valid_disposition = match disposition {
            StoreReceiptDisposition::Applied => {
                let next_revision = before_head.revision().get().checked_add(1);
                before_head == *request.expected_head()
                    && next_revision == Some(after_head.revision().get())
                    && after_head.state_digest() == request.mutation().next_state_digest()
            }
            StoreReceiptDisposition::StalePhysicalHead => {
                before_head != *request.expected_head() && before_head == after_head
            }
        };
        if !valid_disposition {
            return Err(ContractError::StoreReceiptMismatch);
        }
        Ok(Self {
            runtime,
            durability,
            persistence,
            request,
            request_digest,
            before_head,
            after_head,
            disposition,
            transaction_digest,
            receipt_digest,
        })
    }

    #[must_use]
    pub const fn producer_id(&self) -> &'static str {
        STORE_PRODUCER_ID
    }

    #[must_use]
    pub const fn producer_version(&self) -> &'static str {
        STORE_PRODUCER_VERSION
    }

    #[must_use]
    pub const fn runtime(&self) -> RuntimeKind {
        self.runtime
    }

    #[must_use]
    pub const fn durability(&self) -> StoreDurability {
        self.durability
    }

    #[must_use]
    pub const fn persistence(&self) -> Option<&StorePersistenceEvidence> {
        self.persistence.as_ref()
    }

    #[must_use]
    pub const fn request(&self) -> &StoreTransactionRequest {
        &self.request
    }

    #[must_use]
    pub const fn request_digest(&self) -> &ContentDigest {
        &self.request_digest
    }

    #[must_use]
    pub const fn before_head(&self) -> &StorePhysicalHead {
        &self.before_head
    }

    #[must_use]
    pub const fn after_head(&self) -> &StorePhysicalHead {
        &self.after_head
    }

    #[must_use]
    pub const fn disposition(&self) -> StoreReceiptDisposition {
        self.disposition
    }

    #[must_use]
    pub const fn transaction_digest(&self) -> &ContentDigest {
        &self.transaction_digest
    }

    #[must_use]
    pub const fn receipt_digest(&self) -> &ContentDigest {
        &self.receipt_digest
    }
}

macro_rules! lane_request {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        #[derive(Clone, Debug, Eq, PartialEq)]
        pub struct $name {
            invocation: Invocation,
        }

        impl $name {
            /// Constructs the typed lane request.
            #[must_use]
            pub const fn new(invocation: Invocation) -> Self {
                Self { invocation }
            }

            /// Returns the immutable request identity.
            #[must_use]
            pub const fn invocation(&self) -> &Invocation {
                &self.invocation
            }

            /// Consumes the request and returns its immutable identity.
            #[must_use]
            pub fn into_invocation(self) -> Invocation {
                self.invocation
            }
        }
    };
}

/// Typed request for the sole product-code writer lane.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexRunRequest {
    invocation: Invocation,
    writer_claim_digest: ContentDigest,
}

impl CodexRunRequest {
    /// Constructs a code-writer request bound to exact writer-claim evidence.
    #[must_use]
    pub const fn new(invocation: Invocation, writer_claim_digest: ContentDigest) -> Self {
        Self {
            invocation,
            writer_claim_digest,
        }
    }

    /// Returns the immutable request identity.
    #[must_use]
    pub const fn invocation(&self) -> &Invocation {
        &self.invocation
    }

    /// Returns the digest of the exact lease/fencing/worktree claim.
    #[must_use]
    pub const fn writer_claim_digest(&self) -> &ContentDigest {
        &self.writer_claim_digest
    }

    /// Consumes the request and returns its immutable identity.
    #[must_use]
    pub fn into_invocation(self) -> Invocation {
        self.invocation
    }
}

lane_request!(
    GraphifyBuildRequest,
    "Typed request for a derived, read-only code-graph build."
);
lane_request!(
    HermesResearchRequest,
    "Typed request for an untrusted research-candidate lane."
);
