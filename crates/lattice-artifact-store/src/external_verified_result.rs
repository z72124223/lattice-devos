//! Immutable, secret-safe receipts for adopting externally verified results.

use std::error::Error;
use std::fmt;

use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256};
use lattice_contracts::{
    ContentDigest, ProjectId, ProjectSnapshotId, task_ingress_text_contains_recognized_secret,
};
use lattice_task_ledger::ExternalVerifiedResultAdoption;

pub const EXTERNAL_VERIFIED_RESULT_EVIDENCE_SCHEMA: &str =
    "lattice.artifact.external-verified-result-evidence/1.0";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExternalVerifiedResultEvidenceError {
    Malformed,
    Secret,
    Mismatch,
    Canonicalization,
}

impl fmt::Display for ExternalVerifiedResultEvidenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "EXTERNAL_VERIFIED_RESULT_EVIDENCE_{self:?}")
    }
}

impl Error for ExternalVerifiedResultEvidenceError {}

/// The independently retained facts which must agree with one typed adoption.
/// It carries descriptors only, never receipt bytes or credentials.
#[derive(Clone, Eq, PartialEq)]
pub struct ExternalVerifiedResultEvidence {
    project_id: ProjectId,
    project_snapshot_id: ProjectSnapshotId,
    adoption_digest: ContentDigest,
    remote_target_sha: String,
    deployment_artifact_sha256: ContentDigest,
    config_command_sha256: ContentDigest,
    independent_verifier: String,
    non_force_push_merge: bool,
    descriptor_digest: ContentDigest,
}

impl fmt::Debug for ExternalVerifiedResultEvidence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExternalVerifiedResultEvidence")
            .field("schema", &EXTERNAL_VERIFIED_RESULT_EVIDENCE_SCHEMA)
            .field("project_id", &self.project_id)
            .field("project_snapshot_id", &self.project_snapshot_id)
            .field("adoption_digest", &self.adoption_digest)
            .field("remote_target_sha", &self.remote_target_sha)
            .field(
                "deployment_artifact_sha256",
                &self.deployment_artifact_sha256,
            )
            .field("config_command_sha256", &self.config_command_sha256)
            .field("independent_verifier", &self.independent_verifier)
            .field("non_force_push_merge", &self.non_force_push_merge)
            .field("descriptor_digest", &self.descriptor_digest)
            .finish()
    }
}

impl ExternalVerifiedResultEvidence {
    /// Validates and binds independently retained deployment evidence to one
    /// typed external-result adoption.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the target commit, evidence digests,
    /// verifier identity, non-force merge proof, or canonical descriptor is
    /// malformed, secret-shaped, or inconsistent with the adoption.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        project_id: ProjectId,
        project_snapshot_id: ProjectSnapshotId,
        adoption: &ExternalVerifiedResultAdoption,
        remote_target_sha: impl Into<String>,
        deployment_artifact_sha256: ContentDigest,
        config_command_sha256: ContentDigest,
        independent_verifier: impl Into<String>,
        non_force_push_merge: bool,
    ) -> Result<Self, ExternalVerifiedResultEvidenceError> {
        let remote_target_sha = remote_target_sha.into();
        let independent_verifier = independent_verifier.into();
        if !valid_sha(&remote_target_sha)
            || remote_target_sha != adoption.target_sha()
            || !non_force_push_merge
        {
            return Err(ExternalVerifiedResultEvidenceError::Mismatch);
        }
        if is_zero(&deployment_artifact_sha256)
            || is_zero(&config_command_sha256)
            || !valid_identifier(&independent_verifier)
        {
            return Err(ExternalVerifiedResultEvidenceError::Malformed);
        }
        if [
            project_id.as_str(),
            project_snapshot_id.as_str(),
            remote_target_sha.as_str(),
            independent_verifier.as_str(),
        ]
        .into_iter()
        .any(task_ingress_text_contains_recognized_secret)
        {
            return Err(ExternalVerifiedResultEvidenceError::Secret);
        }
        let adoption_digest = adoption.result_digest().clone();
        let descriptor_digest = digest(
            &project_id,
            &project_snapshot_id,
            &adoption_digest,
            &remote_target_sha,
            &deployment_artifact_sha256,
            &config_command_sha256,
            &independent_verifier,
            non_force_push_merge,
        )?;
        Ok(Self {
            project_id,
            project_snapshot_id,
            adoption_digest,
            remote_target_sha,
            deployment_artifact_sha256,
            config_command_sha256,
            independent_verifier,
            non_force_push_merge,
            descriptor_digest,
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
    pub const fn adoption_digest(&self) -> &ContentDigest {
        &self.adoption_digest
    }
    #[must_use]
    pub fn remote_target_sha(&self) -> &str {
        &self.remote_target_sha
    }
    #[must_use]
    pub const fn deployment_artifact_sha256(&self) -> &ContentDigest {
        &self.deployment_artifact_sha256
    }
    #[must_use]
    pub const fn config_command_sha256(&self) -> &ContentDigest {
        &self.config_command_sha256
    }
    #[must_use]
    pub fn independent_verifier(&self) -> &str {
        &self.independent_verifier
    }
    #[must_use]
    pub const fn non_force_push_merge(&self) -> bool {
        self.non_force_push_merge
    }
    #[must_use]
    pub const fn descriptor_digest(&self) -> &ContentDigest {
        &self.descriptor_digest
    }
}

#[allow(clippy::too_many_arguments)]
fn digest(
    project_id: &ProjectId,
    project_snapshot_id: &ProjectSnapshotId,
    adoption_digest: &ContentDigest,
    remote_target_sha: &str,
    deployment_artifact_sha256: &ContentDigest,
    config_command_sha256: &ContentDigest,
    independent_verifier: &str,
    non_force_push_merge: bool,
) -> Result<ContentDigest, ExternalVerifiedResultEvidenceError> {
    let value = CanonicalValue::Object(vec![
        (
            "schema".to_owned(),
            CanonicalValue::String(EXTERNAL_VERIFIED_RESULT_EVIDENCE_SCHEMA.to_owned()),
        ),
        (
            "project_id".to_owned(),
            CanonicalValue::String(project_id.as_str().to_owned()),
        ),
        (
            "project_snapshot_id".to_owned(),
            CanonicalValue::String(project_snapshot_id.as_str().to_owned()),
        ),
        (
            "adoption_digest".to_owned(),
            CanonicalValue::String(adoption_digest.as_str().to_owned()),
        ),
        (
            "remote_target_sha".to_owned(),
            CanonicalValue::String(remote_target_sha.to_owned()),
        ),
        (
            "deployment_artifact_sha256".to_owned(),
            CanonicalValue::String(deployment_artifact_sha256.as_str().to_owned()),
        ),
        (
            "config_command_sha256".to_owned(),
            CanonicalValue::String(config_command_sha256.as_str().to_owned()),
        ),
        (
            "independent_verifier".to_owned(),
            CanonicalValue::String(independent_verifier.to_owned()),
        ),
        (
            "non_force_push_merge".to_owned(),
            CanonicalValue::Bool(non_force_push_merge),
        ),
    ]);
    let domain = HashDomain::new("lattice.artifact.external-verified-result-evidence", "1.0")
        .map_err(|_| ExternalVerifiedResultEvidenceError::Canonicalization)?;
    let digest = canonical_sha256(&domain, &value)
        .map_err(|_| ExternalVerifiedResultEvidenceError::Canonicalization)?;
    ContentDigest::from_sha256(digest.to_hex())
        .map_err(|_| ExternalVerifiedResultEvidenceError::Canonicalization)
}

fn valid_sha(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':'))
}

fn is_zero(value: &ContentDigest) -> bool {
    value.as_str().bytes().all(|byte| byte == b'0')
}
