//! Canonical local verification descriptors, issued only by trusted maintenance.
use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256};
use lattice_contracts::{
    ContentDigest, ProjectId, ProjectSnapshotId, task_ingress_text_contains_recognized_secret,
};
use lattice_task_ledger::LocalVerifiedResultAdoption;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalVerifiedResultEvidence {
    project_id: ProjectId,
    project_snapshot_id: ProjectSnapshotId,
    artifact_sha256: ContentDigest,
    acceptance_sha256: ContentDigest,
    independent_verifier: String,
    descriptor_digest: ContentDigest,
}

impl LocalVerifiedResultEvidence {
    pub const RUNNER_PROFILE: &str = "NODE_TEST_V1";

    /// Reconstructing this type checks commitments; it does not execute tests.
    ///
    /// # Errors
    /// Rejects invalid hashes, verifier identity, runner profile, or canonical encoding.
    pub fn new(
        project_id: ProjectId,
        project_snapshot_id: ProjectSnapshotId,
        adoption: &LocalVerifiedResultAdoption,
        artifact_sha256: ContentDigest,
        acceptance_sha256: ContentDigest,
        independent_verifier: impl Into<String>,
        runner_profile: &str,
    ) -> Result<Self, &'static str> {
        let independent_verifier = independent_verifier.into();
        if artifact_sha256.as_str().bytes().all(|b| b == b'0')
            || acceptance_sha256.as_str().bytes().all(|b| b == b'0')
            || runner_profile != Self::RUNNER_PROFILE
            || independent_verifier.is_empty()
            || independent_verifier.len() > 128
            || !independent_verifier.as_bytes()[0].is_ascii_alphanumeric()
            || !independent_verifier
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
            || task_ingress_text_contains_recognized_secret(&independent_verifier)
        {
            return Err("LOCAL_VERIFIED_RESULT_EVIDENCE_REJECTED");
        }
        let value = CanonicalValue::Object(
            [
                (
                    "schema",
                    "lattice.artifact.local-verified-result-evidence/1.0",
                ),
                ("project_id", project_id.as_str()),
                ("project_snapshot_id", project_snapshot_id.as_str()),
                ("adoption_digest", adoption.result_digest().as_str()),
                ("artifact_sha256", artifact_sha256.as_str()),
                ("acceptance_sha256", acceptance_sha256.as_str()),
                ("independent_verifier", &independent_verifier),
                ("runner_profile", runner_profile),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), CanonicalValue::String(v.to_owned())))
            .collect(),
        );
        let domain = HashDomain::new("lattice.artifact.local-verified-result-evidence", "1.0")
            .map_err(|_| "LOCAL_VERIFIED_RESULT_EVIDENCE_REJECTED")?;
        let hash = canonical_sha256(&domain, &value)
            .map_err(|_| "LOCAL_VERIFIED_RESULT_EVIDENCE_REJECTED")?;
        let descriptor_digest = ContentDigest::from_sha256(hash.to_hex())
            .map_err(|_| "LOCAL_VERIFIED_RESULT_EVIDENCE_REJECTED")?;
        Ok(Self {
            project_id,
            project_snapshot_id,
            artifact_sha256,
            acceptance_sha256,
            independent_verifier,
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
    pub const fn artifact_sha256(&self) -> &ContentDigest {
        &self.artifact_sha256
    }
    #[must_use]
    pub const fn acceptance_sha256(&self) -> &ContentDigest {
        &self.acceptance_sha256
    }
    #[must_use]
    pub fn independent_verifier(&self) -> &str {
        &self.independent_verifier
    }
    #[must_use]
    pub const fn descriptor_digest(&self) -> &ContentDigest {
        &self.descriptor_digest
    }
}
