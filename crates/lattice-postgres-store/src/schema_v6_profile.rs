//! Offline, fail-closed compatibility contract for the schema-v6
//! foreman-coordination migration. Installation remains an explicit coordinated gate.

use std::ops::RangeInclusive;

use sha2::{Digest, Sha256};

use crate::migrations::{
    CURRENT_V5_MANIFEST_SHA256, MigrationDescriptor, migration_manifest, verify_v5_manifest_prefix,
};

const MANIFEST_HASH_DOMAIN: &[u8] = b"LATTICE_POSTGRES_MIGRATION_MANIFEST_V1\0";

pub const FOREMAN_COORDINATION_MIGRATION_ID: &str = "0007_foreman_coordination";
pub const FOREMAN_COORDINATION_MIGRATION_PATH: &str = "db/migrations/0007_foreman_coordination.sql";
pub const FOREMAN_COORDINATION_STREAM_IDENTITY: &str = "FOREMAN_COORDINATION";
pub const FOREMAN_COORDINATION_EVENT_IDENTITY: &str = "FOREMAN_SNAPSHOT_RECORDED";
pub const FOREMAN_COORDINATION_TABLE: &str = "task_ledger_foreman_snapshots";
pub const FOREMAN_COORDINATION_RECORD_FUNCTION: &str = "task_ledger_record_foreman_snapshot_v1";
pub const FOREMAN_COORDINATION_READ_FUNCTION: &str = "task_ledger_read_foreman_snapshots_v1";
pub const WRITER_LEASE_ASSERT_CURRENT_FUNCTION: &str = "writer_lease_assert_current_v1";
pub const FOREMAN_COORDINATION_SCHEMA_VERSION: u16 = 6;
pub const FOREMAN_COORDINATION_MIGRATION_ORDINAL: u16 = 7;

const EXPECTED_TABLE_COUNT: u16 = 17;
const EXPECTED_RETAINED_FUNCTION_COUNT: u16 = 49;
const EXPECTED_RUNTIME_FUNCTION_COUNT: u16 = 21;
const EXPECTED_HISTORICAL_FUNCTION_COUNT: u16 = 28;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SchemaV6ProfileError {
    MigrationMissing,
    MigrationIdentity,
    CatalogAcl,
    WriterProfile,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[must_use]
pub struct ForemanSchemaV6Candidate {
    migration_sql_sha256: String,
    manifest_sha256: String,
    byte_length: usize,
}

impl ForemanSchemaV6Candidate {
    /// Constructs an exact offline schema-v6 candidate over the frozen v5 predecessor.
    ///
    /// # Errors
    ///
    /// Returns a typed failure when migration bytes are missing or any ordinal,
    /// identity, compatibility range, stream, event, or predecessor identity drifts.
    #[allow(clippy::too_many_arguments)]
    pub fn from_migration_bytes(
        ordinal: u16,
        id: &str,
        path: &str,
        schema_version: u16,
        reader_compatibility: RangeInclusive<u16>,
        writer_compatibility: RangeInclusive<u16>,
        stream_identity: &str,
        event_identity: &str,
        bytes: &[u8],
    ) -> Result<Self, SchemaV6ProfileError> {
        if bytes.is_empty() {
            return Err(SchemaV6ProfileError::MigrationMissing);
        }
        let predecessor =
            verify_v5_manifest_prefix().map_err(|_| SchemaV6ProfileError::MigrationIdentity)?;
        if ordinal != FOREMAN_COORDINATION_MIGRATION_ORDINAL
            || id != FOREMAN_COORDINATION_MIGRATION_ID
            || path != FOREMAN_COORDINATION_MIGRATION_PATH
            || schema_version != FOREMAN_COORDINATION_SCHEMA_VERSION
            || reader_compatibility != (6..=6)
            || writer_compatibility != (6..=6)
            || stream_identity != FOREMAN_COORDINATION_STREAM_IDENTITY
            || event_identity != FOREMAN_COORDINATION_EVENT_IDENTITY
            || migration_manifest().len() < 7
            || migration_manifest()
                .get(5)
                .map(MigrationDescriptor::schema_version)
                != Some(5)
            || migration_manifest().get(6).is_none_or(|entry| {
                entry.id() != FOREMAN_COORDINATION_MIGRATION_ID
                    || entry.path() != FOREMAN_COORDINATION_MIGRATION_PATH
                    || entry.bytes() != bytes
            })
            || predecessor.manifest_sha256().as_str() != CURRENT_V5_MANIFEST_SHA256
        {
            return Err(SchemaV6ProfileError::MigrationIdentity);
        }

        let migration_sql_sha256 = sha256_hex(bytes);
        let manifest_sha256 = successor_manifest_sha256(bytes, &migration_sql_sha256);
        Ok(Self {
            migration_sql_sha256,
            manifest_sha256,
            byte_length: bytes.len(),
        })
    }

    #[must_use]
    pub fn migration_sql_sha256(&self) -> &str {
        &self.migration_sql_sha256
    }

    #[must_use]
    pub fn manifest_sha256(&self) -> &str {
        &self.manifest_sha256
    }

    #[must_use]
    pub const fn byte_length(&self) -> usize {
        self.byte_length
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use]
#[allow(clippy::struct_excessive_bools)]
pub struct ForemanSchemaV6CatalogAcl {
    table_count: u16,
    retained_function_count: u16,
    runtime_function_count: u16,
    historical_function_count: u16,
    foreman_table: bool,
    record_function: bool,
    read_function: bool,
    runtime_record_execute: bool,
    runtime_read_execute: bool,
    direct_table_privilege: bool,
    unexpected_object_count: u16,
    writer_assertion_present: bool,
    writer_assertion_before_append: bool,
    atomic_foreman_finalize: bool,
}

impl ForemanSchemaV6CatalogAcl {
    pub const fn exact_foreman_coordination() -> Self {
        Self {
            table_count: EXPECTED_TABLE_COUNT,
            retained_function_count: EXPECTED_RETAINED_FUNCTION_COUNT,
            runtime_function_count: EXPECTED_RUNTIME_FUNCTION_COUNT,
            historical_function_count: EXPECTED_HISTORICAL_FUNCTION_COUNT,
            foreman_table: true,
            record_function: true,
            read_function: true,
            runtime_record_execute: true,
            runtime_read_execute: true,
            direct_table_privilege: false,
            unexpected_object_count: 0,
            writer_assertion_present: true,
            writer_assertion_before_append: true,
            atomic_foreman_finalize: true,
        }
    }

    pub const fn with_table_count(mut self, value: u16) -> Self {
        self.table_count = value;
        self
    }
    pub const fn with_retained_function_count(mut self, value: u16) -> Self {
        self.retained_function_count = value;
        self
    }
    pub const fn with_runtime_function_count(mut self, value: u16) -> Self {
        self.runtime_function_count = value;
        self
    }
    pub const fn with_foreman_table(mut self, value: bool) -> Self {
        self.foreman_table = value;
        self
    }
    pub const fn with_record_function(mut self, value: bool) -> Self {
        self.record_function = value;
        self
    }
    pub const fn with_read_function(mut self, value: bool) -> Self {
        self.read_function = value;
        self
    }
    pub const fn with_runtime_record_execute(mut self, value: bool) -> Self {
        self.runtime_record_execute = value;
        self
    }
    pub const fn with_runtime_read_execute(mut self, value: bool) -> Self {
        self.runtime_read_execute = value;
        self
    }
    pub const fn with_direct_table_privilege(mut self, value: bool) -> Self {
        self.direct_table_privilege = value;
        self
    }
    pub const fn with_unexpected_object_count(mut self, value: u16) -> Self {
        self.unexpected_object_count = value;
        self
    }
    pub const fn with_writer_assertion_present(mut self, value: bool) -> Self {
        self.writer_assertion_present = value;
        self
    }
    pub const fn with_writer_assertion_before_append(mut self, value: bool) -> Self {
        self.writer_assertion_before_append = value;
        self
    }
    pub const fn with_atomic_foreman_finalize(mut self, value: bool) -> Self {
        self.atomic_foreman_finalize = value;
        self
    }

    fn is_exact(self) -> bool {
        self == Self::exact_foreman_coordination()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WriterLeaseV3Profile {
    V2Current,
    Bridge,
    BridgePending,
    Current,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[must_use]
pub struct VerifiedForemanSchemaV6Profile {
    candidate: ForemanSchemaV6Candidate,
    runtime_writer_functions: u8,
}

impl VerifiedForemanSchemaV6Profile {
    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        FOREMAN_COORDINATION_SCHEMA_VERSION
    }
    #[must_use]
    pub const fn migration_ordinal(&self) -> u16 {
        FOREMAN_COORDINATION_MIGRATION_ORDINAL
    }
    #[must_use]
    pub const fn stream_identity(&self) -> &'static str {
        FOREMAN_COORDINATION_STREAM_IDENTITY
    }
    #[must_use]
    pub const fn event_identity(&self) -> &'static str {
        FOREMAN_COORDINATION_EVENT_IDENTITY
    }
    #[must_use]
    pub const fn runtime_writer_functions(&self) -> u8 {
        self.runtime_writer_functions
    }
    #[must_use]
    pub fn manifest_sha256(&self) -> &str {
        self.candidate.manifest_sha256()
    }
}

/// Verifies the exact schema-v6 migration, catalog/ACL, and Writer v3 phase.
///
/// # Errors
///
/// Returns a typed failure for missing migration evidence, catalog/ACL drift, or
/// any Writer profile that could expose runtime functions before the v3 rebind.
pub fn verify_foreman_schema_v6_profile(
    candidate: &ForemanSchemaV6Candidate,
    catalog_acl: &ForemanSchemaV6CatalogAcl,
    writer_profile: WriterLeaseV3Profile,
) -> Result<VerifiedForemanSchemaV6Profile, SchemaV6ProfileError> {
    if candidate.byte_length == 0
        || candidate.migration_sql_sha256.len() != 64
        || candidate.manifest_sha256.len() != 64
    {
        return Err(SchemaV6ProfileError::MigrationMissing);
    }
    if !catalog_acl.is_exact() {
        return Err(SchemaV6ProfileError::CatalogAcl);
    }
    let runtime_writer_functions = match writer_profile {
        WriterLeaseV3Profile::V2Current => return Err(SchemaV6ProfileError::WriterProfile),
        WriterLeaseV3Profile::Bridge | WriterLeaseV3Profile::BridgePending => 0,
        WriterLeaseV3Profile::Current => 7,
    };
    Ok(VerifiedForemanSchemaV6Profile {
        candidate: candidate.clone(),
        runtime_writer_functions,
    })
}

fn successor_manifest_sha256(bytes: &[u8], sql_sha256: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(MANIFEST_HASH_DOMAIN);
    for entry in &migration_manifest()[..6] {
        update_field(&mut hasher, &entry.ordinal().to_be_bytes());
        update_field(&mut hasher, entry.id().as_bytes());
        update_field(&mut hasher, entry.path().as_bytes());
        update_field(
            &mut hasher,
            &u64::try_from(entry.byte_length())
                .expect("migration length fits u64")
                .to_be_bytes(),
        );
        update_field(&mut hasher, entry.sha256().as_bytes());
        update_field(&mut hasher, entry.status().as_str().as_bytes());
        update_field(&mut hasher, entry.transaction_mode().as_str().as_bytes());
        for value in [
            entry.schema_version(),
            *entry.reader_compatibility().start(),
            *entry.reader_compatibility().end(),
            *entry.writer_compatibility().start(),
            *entry.writer_compatibility().end(),
        ] {
            update_field(&mut hasher, &value.to_be_bytes());
        }
    }
    update_field(
        &mut hasher,
        &FOREMAN_COORDINATION_MIGRATION_ORDINAL.to_be_bytes(),
    );
    update_field(&mut hasher, FOREMAN_COORDINATION_MIGRATION_ID.as_bytes());
    update_field(&mut hasher, FOREMAN_COORDINATION_MIGRATION_PATH.as_bytes());
    update_field(
        &mut hasher,
        &u64::try_from(bytes.len())
            .expect("migration length fits u64")
            .to_be_bytes(),
    );
    update_field(&mut hasher, sql_sha256.as_bytes());
    update_field(&mut hasher, b"EXECUTABLE");
    update_field(&mut hasher, b"RUNNER_OWNED");
    for value in [6_u16, 6, 6, 6, 6] {
        update_field(&mut hasher, &value.to_be_bytes());
    }
    bytes_to_hex(hasher.finalize().as_ref())
}

fn update_field(hasher: &mut Sha256, value: &[u8]) {
    hasher.update(
        u64::try_from(value.len())
            .expect("manifest field length fits u64")
            .to_be_bytes(),
    );
    hasher.update(value);
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    bytes_to_hex(digest.as_ref())
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(64);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("String writes cannot fail");
    }
    output
}
