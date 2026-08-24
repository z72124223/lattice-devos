//! Typed Store conformance fake plus exact live `PostgreSQL` adapter for LATTICE.

mod foreman_coordination;
mod live;
mod migrations;
mod postgres_setup;
mod project_registry;
mod schema_v6_profile;
mod task_ledger;

pub use foreman_coordination::PostgresForemanCoordination;
pub use live::PostgresControlStore;
pub use migrations::{
    DatabaseRole, ManifestEvidence, MigrationDescriptor, MigrationStatus, MigrationTarget,
    MigrationTransactionMode, POSTGRES_DRIVER_VERSION, POSTGRES_SCHEMA_VERSION,
    PostgresStoreSetupError, PostgresStoreSetupErrorKind, SUPPORTED_POSTGRES_MAJOR, Sha256Hex,
    migration_manifest, verify_embedded_manifest,
};
pub use postgres_setup::{
    BootstrapAdmission, MigrationApplyOutcome, MigrationBootstrapProfile, PostgresSchemaEvidence,
    apply_migrations, inspect_migration_profile, verify_postgres_schema,
};
pub use project_registry::{
    PostgresProjectRegistry, PostgresProjectRegistryError, PostgresProjectRegistryErrorKind,
    PostgresProjectRegistryExecution, PostgresProjectRegistryLoad,
    PostgresProjectRegistryPersistenceEvidence, PostgresProjectRegistryPersistenceReceipt,
    PostgresProjectRegistryResult,
};
pub use schema_v6_profile::{
    FOREMAN_COORDINATION_EVENT_IDENTITY, FOREMAN_COORDINATION_MIGRATION_ID,
    FOREMAN_COORDINATION_MIGRATION_ORDINAL, FOREMAN_COORDINATION_MIGRATION_PATH,
    FOREMAN_COORDINATION_READ_FUNCTION, FOREMAN_COORDINATION_RECORD_FUNCTION,
    FOREMAN_COORDINATION_SCHEMA_VERSION, FOREMAN_COORDINATION_STREAM_IDENTITY,
    FOREMAN_COORDINATION_TABLE, ForemanSchemaV6Candidate, ForemanSchemaV6CatalogAcl,
    SchemaV6ProfileError, VerifiedForemanSchemaV6Profile, WRITER_LEASE_ASSERT_CURRENT_FUNCTION,
    WriterLeaseV3Profile, verify_foreman_schema_v6_profile,
};
pub use task_ledger::{
    PostgresForemanReplay, PostgresTaskLedger, PostgresTaskLedgerError,
    PostgresTaskLedgerErrorKind, PostgresTaskLedgerExecution, PostgresTaskLedgerLoad,
    PostgresTaskLedgerPersistenceEvidence, PostgresTaskLedgerResult,
};

use std::collections::BTreeMap;

use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256};
use lattice_contracts::{
    ContentDigest, RuntimeAdmissionMode, RuntimeKind, STORE_PRODUCER_ID, STORE_PRODUCER_VERSION,
    StoreAuthorityHead, StoreDurability, StorePhysicalHead, StoreReceiptDisposition,
    StoreRepositoryOwner, StoreRevision, StoreScope, StoreTransactionId, StoreTransactionReceipt,
    StoreTransactionRequest,
};
use lattice_ports::{ControlStore, ControlStoreError, ControlStoreErrorKind, ControlStoreResult};

/// Maximum terminal transaction records retained by one fake.
pub const MAX_FAKE_TRANSACTIONS: usize = 1_024;
/// Maximum independently materialized physical scopes retained by one fake.
pub const MAX_FAKE_SCOPES: usize = 256;
/// Maximum total attempts represented by deterministic serialization faults.
pub const MAX_SERIALIZATION_ATTEMPTS: u8 = 3;

const STORE_SCHEMA_VERSION: &str = "1.0";

/// One deterministic fault injected into the next new transaction only.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FakeStoreFault {
    /// Availability fails before any physical fake mutation.
    BeforeApplyUnavailable,
    /// The fake applies atomically but loses the response.
    AfterApplyOutcomeUnknown,
    /// A fixed number of serialization attempts conflict before apply.
    SerializationConflicts(u8),
}

/// Explicit corruption target for retained fake replay validation tests.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FakeReplayCorruption {
    RequestDigest,
    ReceiptDigest,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ScopeKey {
    project_id: String,
    project_snapshot_id: String,
    owner: StoreRepositoryOwner,
    aggregate_key_digest: String,
}

impl From<&StoreScope> for ScopeKey {
    fn from(scope: &StoreScope) -> Self {
        Self {
            project_id: scope.project_id().as_str().to_owned(),
            project_snapshot_id: scope.project_snapshot_id().as_str().to_owned(),
            owner: scope.owner(),
            aggregate_key_digest: scope.aggregate_key_digest().as_str().to_owned(),
        }
    }
}

#[derive(Clone, Debug)]
struct ReplayEntry {
    request_digest: ContentDigest,
    receipt: StoreTransactionReceipt,
    receipt_digest: ContentDigest,
}

/// Deterministic in-memory conformance fake; never a `PostgreSQL` durability claim.
#[derive(Clone, Debug)]
pub struct FakePostgresStore {
    current_authority: StoreAuthorityHead,
    heads: BTreeMap<ScopeKey, StorePhysicalHead>,
    replay: BTreeMap<StoreTransactionId, ReplayEntry>,
    transaction_capacity: usize,
    next_fault: Option<FakeStoreFault>,
}

impl FakePostgresStore {
    /// Constructs an empty fake under one independently retained fake authority.
    ///
    /// # Errors
    ///
    /// Rejects live authority and zero/oversized transaction capacity.
    pub fn new(
        current_authority: StoreAuthorityHead,
        transaction_capacity: usize,
    ) -> ControlStoreResult<Self> {
        Self::with_heads(current_authority, transaction_capacity, [])
    }

    /// Constructs a fake with explicit physical fixture heads.
    ///
    /// # Errors
    ///
    /// Rejects live authority/heads, duplicate scopes, too many scopes, and
    /// zero/oversized transaction capacity.
    pub fn with_heads<I>(
        current_authority: StoreAuthorityHead,
        transaction_capacity: usize,
        heads: I,
    ) -> ControlStoreResult<Self>
    where
        I: IntoIterator<Item = StorePhysicalHead>,
    {
        if current_authority.runtime() != RuntimeKind::Fake
            || !(1..=MAX_FAKE_TRANSACTIONS).contains(&transaction_capacity)
        {
            return Err(store_error(
                ControlStoreErrorKind::Malformed,
                "STORE_FAKE_CONFIG_INVALID",
            ));
        }
        let mut retained = BTreeMap::new();
        for head in heads {
            if head.runtime() != RuntimeKind::Fake || retained.len() >= MAX_FAKE_SCOPES {
                return Err(store_error(
                    ControlStoreErrorKind::Malformed,
                    "STORE_FAKE_HEAD_INVALID",
                ));
            }
            validate_physical_head(&head)?;
            let key = ScopeKey::from(head.scope());
            if retained.insert(key, head).is_some() {
                return Err(store_error(
                    ControlStoreErrorKind::Malformed,
                    "STORE_FAKE_HEAD_DUPLICATE",
                ));
            }
        }
        Ok(Self {
            current_authority,
            heads: retained,
            replay: BTreeMap::new(),
            transaction_capacity,
            next_fault: None,
        })
    }

    /// Derives a canonical fake physical head for explicit test fixtures.
    ///
    /// # Errors
    ///
    /// Returns a typed corruption failure if canonical hashing cannot produce
    /// the structurally valid fake head.
    pub fn derive_head_for_fixture(
        scope: StoreScope,
        revision: StoreRevision,
        state_digest: ContentDigest,
    ) -> ControlStoreResult<StorePhysicalHead> {
        let head = physical_head(RuntimeKind::Fake, scope, revision, state_digest)?;
        validate_physical_head(&head)?;
        Ok(head)
    }

    /// Replaces the fake's independently retained authority observation.
    ///
    /// # Errors
    ///
    /// Rejects a live authority because this module cannot authenticate it.
    pub fn set_current_authority(
        &mut self,
        current_authority: StoreAuthorityHead,
    ) -> ControlStoreResult<()> {
        if current_authority.runtime() != RuntimeKind::Fake {
            return Err(store_error(
                ControlStoreErrorKind::Malformed,
                "STORE_FAKE_LIVE_AUTHORITY_FORBIDDEN",
            ));
        }
        self.current_authority = current_authority;
        Ok(())
    }

    /// Injects one deterministic fault into the next new transaction.
    pub const fn inject_next_fault(&mut self, fault: FakeStoreFault) {
        self.next_fault = Some(fault);
    }

    /// Corrupts one retained replay commitment for fail-closed test evidence.
    ///
    /// Returns false when the transaction is not retained.
    pub fn inject_replay_corruption(
        &mut self,
        transaction_id: &StoreTransactionId,
        corruption: FakeReplayCorruption,
    ) -> bool {
        let Some(entry) = self.replay.get_mut(transaction_id) else {
            return false;
        };
        match corruption {
            FakeReplayCorruption::RequestDigest => {
                entry.request_digest = different_digest(&entry.request_digest);
            }
            FakeReplayCorruption::ReceiptDigest => {
                entry.receipt_digest = different_digest(&entry.receipt_digest);
            }
        }
        true
    }

    /// Corrupts one materialized physical head while preserving its old digest.
    ///
    /// Returns false when the exact scope has not been materialized.
    pub fn inject_head_corruption(&mut self, scope: &StoreScope) -> bool {
        let Some(head) = self.heads.get_mut(&ScopeKey::from(scope)) else {
            return false;
        };
        let Ok(corrupt) = StorePhysicalHead::new(
            RuntimeKind::Fake,
            head.scope().clone(),
            head.revision(),
            different_digest(head.state_digest()),
            head.head_digest().clone(),
        ) else {
            return false;
        };
        *head = corrupt;
        true
    }

    /// Returns the number of retained terminal transaction records.
    #[must_use]
    pub fn transaction_count(&self) -> usize {
        self.replay.len()
    }

    /// Returns the number of physical heads materialized by fixtures/applies.
    #[must_use]
    pub fn materialized_scope_count(&self) -> usize {
        self.heads.len()
    }

    fn replay(
        &self,
        request: &StoreTransactionRequest,
        request_digest: &ContentDigest,
    ) -> ControlStoreResult<Option<StoreTransactionReceipt>> {
        let Some(entry) = self.replay.get(request.transaction_id()) else {
            return Ok(None);
        };
        validate_replay(entry, request, request_digest)?;
        self.validate_scope_state(entry.receipt.request().scope())?;
        Ok(Some(entry.receipt.clone()))
    }

    fn validate_scope_state(&self, scope: &StoreScope) -> ControlStoreResult<()> {
        let key = ScopeKey::from(scope);
        if let Some(head) = self.heads.get(&key) {
            return validate_physical_head(head);
        }
        if self.replay.values().any(|entry| {
            entry.receipt.request().scope() == scope
                && entry.receipt.disposition() == StoreReceiptDisposition::Applied
        }) {
            return Err(store_error(
                ControlStoreErrorKind::CorruptState,
                "STORE_APPLIED_HEAD_MISSING",
            ));
        }
        Ok(())
    }

    fn retain(&mut self, receipt: StoreTransactionReceipt) {
        self.replay.insert(
            receipt.request().transaction_id().clone(),
            ReplayEntry {
                request_digest: receipt.request_digest().clone(),
                receipt_digest: receipt.receipt_digest().clone(),
                receipt,
            },
        );
    }
}

impl ControlStore for FakePostgresStore {
    fn transact(
        &mut self,
        request: StoreTransactionRequest,
    ) -> ControlStoreResult<StoreTransactionReceipt> {
        let request_digest = request_digest(&request)?;
        if let Some(receipt) = self.replay(&request, &request_digest)? {
            return Ok(receipt);
        }

        if request.expected_authority() != &self.current_authority {
            return Err(store_error(
                ControlStoreErrorKind::AuthorityMismatch,
                "STORE_AUTHORITY_MISMATCH",
            ));
        }
        if self.current_authority.admission() != RuntimeAdmissionMode::Active {
            return Err(store_error(
                ControlStoreErrorKind::AdmissionDenied,
                "STORE_ADMISSION_DENIED",
            ));
        }
        if self.replay.len() >= self.transaction_capacity {
            return Err(store_error(
                ControlStoreErrorKind::CapacityExceeded,
                "STORE_TRANSACTION_CAPACITY_EXCEEDED",
            ));
        }

        let current = self.current_head(request.scope())?;
        if current != *request.expected_head() {
            let receipt = build_fake_receipt(
                request,
                request_digest,
                current.clone(),
                current,
                StoreReceiptDisposition::StalePhysicalHead,
            )?;
            self.retain(receipt.clone());
            return Ok(receipt);
        }

        let next_revision = current
            .revision()
            .get()
            .checked_add(1)
            .and_then(|value| StoreRevision::new(value).ok())
            .ok_or_else(|| {
                store_error(
                    ControlStoreErrorKind::RevisionOverflow,
                    "STORE_REVISION_OVERFLOW",
                )
            })?;

        let fault = self.next_fault.take();
        if matches!(fault, Some(FakeStoreFault::BeforeApplyUnavailable)) {
            return Err(store_error(
                ControlStoreErrorKind::Unavailable,
                "STORE_BEFORE_APPLY_UNAVAILABLE",
            ));
        }
        if let Some(FakeStoreFault::SerializationConflicts(conflicts)) = fault
            && conflicts >= MAX_SERIALIZATION_ATTEMPTS
        {
            return Err(store_error(
                ControlStoreErrorKind::SerializationExhausted,
                "STORE_SERIALIZATION_RETRIES_EXHAUSTED",
            ));
        }

        let key = ScopeKey::from(request.scope());
        if !self.heads.contains_key(&key) && self.heads.len() >= MAX_FAKE_SCOPES {
            return Err(store_error(
                ControlStoreErrorKind::CapacityExceeded,
                "STORE_SCOPE_CAPACITY_EXCEEDED",
            ));
        }
        let after = physical_head(
            RuntimeKind::Fake,
            request.scope().clone(),
            next_revision,
            request.mutation().next_state_digest().clone(),
        )?;
        let receipt = build_fake_receipt(
            request,
            request_digest,
            current,
            after.clone(),
            StoreReceiptDisposition::Applied,
        )?;

        self.heads.insert(key, after);
        self.retain(receipt.clone());

        if matches!(fault, Some(FakeStoreFault::AfterApplyOutcomeUnknown)) {
            Err(store_error(
                ControlStoreErrorKind::CommitOutcomeUnknown,
                "STORE_COMMIT_OUTCOME_UNKNOWN",
            ))
        } else {
            Ok(receipt)
        }
    }

    fn current_head(&mut self, scope: &StoreScope) -> ControlStoreResult<StorePhysicalHead> {
        self.validate_scope_state(scope)?;
        self.heads
            .get(&ScopeKey::from(scope))
            .cloned()
            .map_or_else(|| genesis_head(RuntimeKind::Fake, scope.clone()), Ok)
    }
}

fn store_error(kind: ControlStoreErrorKind, code: &'static str) -> ControlStoreError {
    ControlStoreError::new(kind, code)
}

fn hash_value(schema_id: &str, value: &CanonicalValue) -> ControlStoreResult<ContentDigest> {
    let domain = HashDomain::new(schema_id, STORE_SCHEMA_VERSION).map_err(|_| {
        store_error(
            ControlStoreErrorKind::CorruptState,
            "STORE_HASH_DOMAIN_INVALID",
        )
    })?;
    let digest = canonical_sha256(&domain, value).map_err(|_| {
        store_error(
            ControlStoreErrorKind::CorruptState,
            "STORE_CANONICAL_HASH_FAILED",
        )
    })?;
    ContentDigest::from_sha256(digest.to_hex())
        .map_err(|_| store_error(ControlStoreErrorKind::CorruptState, "STORE_DIGEST_INVALID"))
}

fn string(value: impl Into<String>) -> CanonicalValue {
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

fn optional_digest(value: Option<&ContentDigest>) -> CanonicalValue {
    value.map_or(CanonicalValue::Null, |digest| string(digest.as_str()))
}

fn runtime_value(runtime: RuntimeKind) -> CanonicalValue {
    string(match runtime {
        RuntimeKind::Fake => "FAKE",
        RuntimeKind::Live => "LIVE",
    })
}

fn scope_value(scope: &StoreScope) -> CanonicalValue {
    object(vec![
        ("project_id", string(scope.project_id().as_str())),
        (
            "project_snapshot_id",
            string(scope.project_snapshot_id().as_str()),
        ),
        ("repository_owner", string(scope.owner().as_str())),
        (
            "aggregate_key_digest",
            string(scope.aggregate_key_digest().as_str()),
        ),
    ])
}

fn authority_value(authority: &StoreAuthorityHead) -> CanonicalValue {
    object(vec![
        ("runtime", runtime_value(authority.runtime())),
        (
            "daemon_instance_id",
            string(authority.daemon_instance_id().as_str()),
        ),
        (
            "daemon_epoch",
            string(authority.daemon_epoch().get().to_string()),
        ),
        ("admission", string(authority.admission().as_str())),
        ("revision", string(authority.revision().get().to_string())),
        (
            "observation_digest",
            string(authority.observation_digest().as_str()),
        ),
        ("head_digest", string(authority.head_digest().as_str())),
    ])
}

fn physical_head_value(head: &StorePhysicalHead) -> CanonicalValue {
    object(vec![
        ("runtime", runtime_value(head.runtime())),
        ("scope", scope_value(head.scope())),
        ("revision", string(head.revision().get().to_string())),
        ("state_digest", string(head.state_digest().as_str())),
        ("head_digest", string(head.head_digest().as_str())),
    ])
}

fn mutation_value(request: &StoreTransactionRequest) -> CanonicalValue {
    let mutation = request.mutation();
    object(vec![
        (
            "domain_command_digest",
            string(mutation.domain_command_digest().as_str()),
        ),
        (
            "record_set_digest",
            string(mutation.record_set_digest().as_str()),
        ),
        (
            "next_state_digest",
            string(mutation.next_state_digest().as_str()),
        ),
        (
            "domain_receipt_digest",
            string(mutation.domain_receipt_digest().as_str()),
        ),
        (
            "checkpoint_digest",
            optional_digest(mutation.checkpoint_digest()),
        ),
        (
            "outbox_intent_digest",
            optional_digest(mutation.outbox_intent_digest()),
        ),
    ])
}

fn request_value(request: &StoreTransactionRequest) -> CanonicalValue {
    object(vec![
        ("version", string(request.version().to_string())),
        ("transaction_id", string(request.transaction_id().as_str())),
        ("scope", scope_value(request.scope())),
        (
            "expected_authority",
            authority_value(request.expected_authority()),
        ),
        (
            "expected_head",
            physical_head_value(request.expected_head()),
        ),
        ("mutation", mutation_value(request)),
    ])
}

pub(crate) fn request_digest(
    request: &StoreTransactionRequest,
) -> ControlStoreResult<ContentDigest> {
    hash_value(
        "lattice.postgres-store.transaction-request",
        &request_value(request),
    )
}

pub(crate) fn genesis_head(
    runtime: RuntimeKind,
    scope: StoreScope,
) -> ControlStoreResult<StorePhysicalHead> {
    let state_digest = hash_value("lattice.postgres-store.genesis-state", &scope_value(&scope))?;
    physical_head(
        runtime,
        scope,
        StoreRevision::new(0).map_err(|_| {
            store_error(
                ControlStoreErrorKind::CorruptState,
                "STORE_GENESIS_REVISION_INVALID",
            )
        })?,
        state_digest,
    )
}

pub(crate) fn physical_head(
    runtime: RuntimeKind,
    scope: StoreScope,
    revision: StoreRevision,
    state_digest: ContentDigest,
) -> ControlStoreResult<StorePhysicalHead> {
    let head_subject = object(vec![
        ("runtime", runtime_value(runtime)),
        ("scope", scope_value(&scope)),
        ("revision", string(revision.get().to_string())),
        ("state_digest", string(state_digest.as_str())),
    ]);
    let head_digest = hash_value("lattice.postgres-store.physical-head", &head_subject)?;
    StorePhysicalHead::new(runtime, scope, revision, state_digest, head_digest).map_err(|_| {
        store_error(
            ControlStoreErrorKind::CorruptState,
            "STORE_PHYSICAL_HEAD_INVALID",
        )
    })
}

pub(crate) fn validate_physical_head(head: &StorePhysicalHead) -> ControlStoreResult<()> {
    let expected = if head.revision().get() == 0 {
        genesis_head(head.runtime(), head.scope().clone())?
    } else {
        physical_head(
            head.runtime(),
            head.scope().clone(),
            head.revision(),
            head.state_digest().clone(),
        )?
    };
    if &expected != head {
        return Err(store_error(
            ControlStoreErrorKind::CorruptState,
            "STORE_PHYSICAL_HEAD_CORRUPT",
        ));
    }
    Ok(())
}

pub(crate) fn transaction_digest(
    request_digest: &ContentDigest,
    before: &StorePhysicalHead,
    after: &StorePhysicalHead,
    disposition: StoreReceiptDisposition,
) -> ControlStoreResult<ContentDigest> {
    hash_value(
        "lattice.postgres-store.transaction",
        &object(vec![
            ("request_digest", string(request_digest.as_str())),
            ("before_head", physical_head_value(before)),
            ("after_head", physical_head_value(after)),
            ("disposition", string(disposition.as_str())),
        ]),
    )
}

#[allow(clippy::too_many_arguments)]
fn receipt_digest(
    request: &StoreTransactionRequest,
    request_digest: &ContentDigest,
    before: &StorePhysicalHead,
    after: &StorePhysicalHead,
    disposition: StoreReceiptDisposition,
    transaction_digest: &ContentDigest,
    runtime: RuntimeKind,
    durability: StoreDurability,
    persistence: Option<&lattice_contracts::StorePersistenceEvidence>,
) -> ControlStoreResult<ContentDigest> {
    let mut entries = vec![
        ("producer_id", string(STORE_PRODUCER_ID)),
        ("producer_version", string(STORE_PRODUCER_VERSION)),
        ("runtime", runtime_value(runtime)),
        ("durability", string(durability.as_str())),
        ("transaction_id", string(request.transaction_id().as_str())),
        ("scope", scope_value(request.scope())),
        ("request_digest", string(request_digest.as_str())),
        ("before_head", physical_head_value(before)),
        ("after_head", physical_head_value(after)),
        ("disposition", string(disposition.as_str())),
        ("transaction_digest", string(transaction_digest.as_str())),
    ];
    if let Some(persistence) = persistence {
        entries.extend([
            (
                "database_identity_digest",
                string(persistence.database_identity_digest().as_str()),
            ),
            (
                "schema_version",
                string(persistence.schema_version().to_string()),
            ),
            (
                "manifest_digest",
                string(persistence.manifest_digest().as_str()),
            ),
        ]);
    }
    hash_value(
        "lattice.postgres-store.transaction-receipt",
        &object(entries),
    )
}

fn build_fake_receipt(
    request: StoreTransactionRequest,
    request_digest: ContentDigest,
    before: StorePhysicalHead,
    after: StorePhysicalHead,
    disposition: StoreReceiptDisposition,
) -> ControlStoreResult<StoreTransactionReceipt> {
    let transaction_digest = transaction_digest(&request_digest, &before, &after, disposition)?;
    let receipt_digest = receipt_digest(
        &request,
        &request_digest,
        &before,
        &after,
        disposition,
        &transaction_digest,
        RuntimeKind::Fake,
        StoreDurability::NonDurableFake,
        None,
    )?;
    StoreTransactionReceipt::new_non_durable_fake(
        request,
        request_digest,
        before,
        after,
        disposition,
        transaction_digest,
        receipt_digest,
    )
    .map_err(|_| {
        store_error(
            ControlStoreErrorKind::CorruptState,
            "STORE_RECEIPT_CONSTRUCTION_INVALID",
        )
    })
}

pub(crate) fn build_live_receipt(
    request: StoreTransactionRequest,
    persistence: lattice_contracts::StorePersistenceEvidence,
    request_digest: ContentDigest,
    before: StorePhysicalHead,
    after: StorePhysicalHead,
    disposition: StoreReceiptDisposition,
) -> ControlStoreResult<StoreTransactionReceipt> {
    let transaction_digest = transaction_digest(&request_digest, &before, &after, disposition)?;
    let receipt_digest = receipt_digest(
        &request,
        &request_digest,
        &before,
        &after,
        disposition,
        &transaction_digest,
        RuntimeKind::Live,
        StoreDurability::DurablePostgres,
        Some(&persistence),
    )?;
    StoreTransactionReceipt::new_durable_postgres(
        request,
        persistence,
        request_digest,
        before,
        after,
        disposition,
        transaction_digest,
        receipt_digest,
    )
    .map_err(|_| {
        store_error(
            ControlStoreErrorKind::CorruptState,
            "STORE_LIVE_RECEIPT_CONSTRUCTION_INVALID",
        )
    })
}

fn validate_replay(
    entry: &ReplayEntry,
    request: &StoreTransactionRequest,
    incoming_request_digest: &ContentDigest,
) -> ControlStoreResult<()> {
    let receipt = &entry.receipt;
    let recomputed_request = request_digest(receipt.request())?;
    let recomputed_transaction = transaction_digest(
        receipt.request_digest(),
        receipt.before_head(),
        receipt.after_head(),
        receipt.disposition(),
    )?;
    let recomputed_receipt = receipt_digest(
        receipt.request(),
        receipt.request_digest(),
        receipt.before_head(),
        receipt.after_head(),
        receipt.disposition(),
        receipt.transaction_digest(),
        receipt.runtime(),
        receipt.durability(),
        receipt.persistence(),
    )?;
    if entry.request_digest != *receipt.request_digest()
        || recomputed_request != *receipt.request_digest()
        || recomputed_transaction != *receipt.transaction_digest()
        || entry.receipt_digest != *receipt.receipt_digest()
        || recomputed_receipt != *receipt.receipt_digest()
    {
        return Err(store_error(
            ControlStoreErrorKind::CorruptState,
            "STORE_REPLAY_CORRUPT",
        ));
    }
    if receipt.request() != request || &entry.request_digest != incoming_request_digest {
        return Err(store_error(
            ControlStoreErrorKind::CommandSubstitution,
            "STORE_TRANSACTION_ID_SUBSTITUTION",
        ));
    }
    Ok(())
}

fn different_digest(current: &ContentDigest) -> ContentDigest {
    let replacement = if current.as_str().starts_with('f') {
        "e".repeat(64)
    } else {
        "f".repeat(64)
    };
    ContentDigest::from_sha256(replacement).expect("fixed valid corruption digest")
}
