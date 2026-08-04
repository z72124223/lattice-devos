//! Root-owned atomic quota-head set and checkpoint composition.
#![allow(
    dead_code,
    reason = "crate-private support is wired into the root facade in the next bounded slice"
)]

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt;

use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256};
use lattice_contracts::{
    ARTIFACT_STORE_PRODUCER_ID, ARTIFACT_STORE_PRODUCER_VERSION, ArtifactObjectIdentity,
    ContentDigest, ProjectId, RuntimeKind, TaskId,
};

use crate::{
    ArtifactQuotaError, ArtifactQuotaHead, ArtifactQuotaReport, ArtifactQuotaScope,
    ArtifactStoreIdentity,
};

/// Root-owned quota-head composition failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ArtifactQuotaOwnerError {
    /// The explicit scope set is empty, duplicated, or lacks one exact store.
    InvalidScopeSet,
    /// A previously owned scope was omitted or replaced.
    ScopeDrift,
    /// The recomputed report has no projection for an explicit scope.
    MissingScope(ArtifactQuotaScope),
    /// A requested current head is absent.
    MissingHead(ArtifactQuotaScope),
    /// The immutable limit snapshot changed inside one head set.
    LimitSnapshotDrift,
    /// The lower-level quota owner rejected head construction.
    Quota(ArtifactQuotaError),
    /// Canonical checkpoint framing failed.
    Canonicalization,
    /// A locally produced SHA-256 violated the shared digest contract.
    InvalidDigest,
}

impl ArtifactQuotaOwnerError {
    /// Stable machine-readable error code.
    #[must_use]
    pub(crate) const fn code(&self) -> &'static str {
        match self {
            Self::InvalidScopeSet => "ARTIFACT_QUOTA_OWNER_INVALID_SCOPE_SET",
            Self::ScopeDrift => "ARTIFACT_QUOTA_OWNER_SCOPE_DRIFT",
            Self::MissingScope(_) => "ARTIFACT_QUOTA_OWNER_MISSING_SCOPE",
            Self::MissingHead(_) => "ARTIFACT_QUOTA_OWNER_MISSING_HEAD",
            Self::LimitSnapshotDrift => "ARTIFACT_QUOTA_OWNER_LIMIT_SNAPSHOT_DRIFT",
            Self::Quota(_) => "ARTIFACT_QUOTA_OWNER_REJECTED",
            Self::Canonicalization => "ARTIFACT_QUOTA_OWNER_CANONICALIZATION_FAILED",
            Self::InvalidDigest => "ARTIFACT_QUOTA_OWNER_INVALID_DIGEST",
        }
    }
}

impl fmt::Display for ArtifactQuotaOwnerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidScopeSet => formatter.write_str("quota owner scope set is invalid"),
            Self::ScopeDrift => formatter.write_str("quota owner scope set drifted"),
            Self::MissingScope(_) => {
                formatter.write_str("quota report is missing an explicit scope")
            }
            Self::MissingHead(_) => formatter.write_str("quota owner head is absent"),
            Self::LimitSnapshotDrift => formatter.write_str("quota owner limit snapshot drifted"),
            Self::Quota(error) => write!(formatter, "quota owner rejected projection: {error}"),
            Self::Canonicalization => formatter.write_str("quota owner canonicalization failed"),
            Self::InvalidDigest => formatter.write_str("quota owner digest is invalid"),
        }
    }
}

impl Error for ArtifactQuotaOwnerError {}

impl From<ArtifactQuotaError> for ArtifactQuotaOwnerError {
    fn from(error: ArtifactQuotaError) -> Self {
        Self::Quota(error)
    }
}

/// Atomic fixed-owner current heads for every root-selected quota scope.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ArtifactQuotaHeadSet {
    limit_snapshot_digest: ContentDigest,
    heads: HashMap<ArtifactQuotaScope, ArtifactQuotaHead>,
    checkpoint_digest: ContentDigest,
}

impl ArtifactQuotaHeadSet {
    /// Creates initial revision-one heads from one internally recomputed report.
    pub(crate) fn from_report(
        report: &ArtifactQuotaReport,
        scopes: impl IntoIterator<Item = ArtifactQuotaScope>,
    ) -> Result<Self, ArtifactQuotaOwnerError> {
        let scopes = normalize_scopes(scopes)?;
        let mut heads = HashMap::with_capacity(scopes.len());
        for scope in scopes {
            let authoritative = report
                .authority_projection(scope.clone())
                .map_err(|error| map_projection_error(scope.clone(), error))?;
            heads.insert(scope, ArtifactQuotaHead::initial(authoritative)?);
        }
        let limit_snapshot_digest = report.limit_snapshot_digest().clone();
        let checkpoint_digest = checkpoint_digest(&limit_snapshot_digest, &heads)?;
        Ok(Self {
            limit_snapshot_digest,
            heads,
            checkpoint_digest,
        })
    }

    /// Restores an exact compact current-head set after every individual head
    /// has independently recomputed its digest chain.
    pub(crate) fn restore_exact(
        limit_snapshot_digest: ContentDigest,
        input: impl IntoIterator<Item = ArtifactQuotaHead>,
        stored_checkpoint_digest: &ContentDigest,
    ) -> Result<Self, ArtifactQuotaOwnerError> {
        let mut heads = HashMap::new();
        for head in input {
            if head.limit_snapshot_digest() != &limit_snapshot_digest
                || heads.insert(head.scope().clone(), head).is_some()
            {
                return Err(ArtifactQuotaOwnerError::InvalidScopeSet);
            }
        }
        if heads.is_empty()
            || heads
                .keys()
                .filter(|scope| matches!(scope, ArtifactQuotaScope::Store(_)))
                .count()
                != 1
        {
            return Err(ArtifactQuotaOwnerError::InvalidScopeSet);
        }
        let expected = checkpoint_digest(&limit_snapshot_digest, &heads)?;
        if &expected != stored_checkpoint_digest {
            return Err(ArtifactQuotaOwnerError::InvalidScopeSet);
        }
        Ok(Self {
            limit_snapshot_digest,
            heads,
            checkpoint_digest: expected,
        })
    }

    /// Atomically advances only changed projections and adds exact new scopes.
    ///
    /// Existing scopes may not disappear. Any failure leaves this set byte-for-
    /// byte unchanged.
    pub(crate) fn apply_report(
        &mut self,
        report: &ArtifactQuotaReport,
        scopes: impl IntoIterator<Item = ArtifactQuotaScope>,
    ) -> Result<(), ArtifactQuotaOwnerError> {
        self.apply_report_with_retired(report, scopes, std::iter::empty())
    }

    /// Atomically advances active scopes while retaining explicitly retired
    /// object-generation heads at their last authoritative value.
    ///
    /// Every retired scope must already exist, must be an object scope, and
    /// must not also be active. Any failure leaves this set byte-for-byte
    /// unchanged.
    pub(crate) fn apply_report_with_retired(
        &mut self,
        report: &ArtifactQuotaReport,
        active_scopes: impl IntoIterator<Item = ArtifactQuotaScope>,
        retired_object_scopes: impl IntoIterator<Item = ArtifactQuotaScope>,
    ) -> Result<(), ArtifactQuotaOwnerError> {
        let candidate = self.next(report, active_scopes, retired_object_scopes)?;
        *self = candidate;
        Ok(())
    }

    fn next(
        &self,
        report: &ArtifactQuotaReport,
        active_scopes: impl IntoIterator<Item = ArtifactQuotaScope>,
        retired_object_scopes: impl IntoIterator<Item = ArtifactQuotaScope>,
    ) -> Result<Self, ArtifactQuotaOwnerError> {
        if report.limit_snapshot_digest() != &self.limit_snapshot_digest {
            return Err(ArtifactQuotaOwnerError::LimitSnapshotDrift);
        }
        let active_scopes = normalize_scopes(active_scopes)?;
        let active_scope_set = active_scopes.iter().cloned().collect::<HashSet<_>>();
        let retired_scope_set =
            normalize_retired_object_scopes(retired_object_scopes, &self.heads, &active_scope_set)?;
        if self
            .heads
            .keys()
            .any(|scope| !active_scope_set.contains(scope) && !retired_scope_set.contains(scope))
        {
            return Err(ArtifactQuotaOwnerError::ScopeDrift);
        }

        let mut heads =
            HashMap::with_capacity(active_scopes.len().saturating_add(retired_scope_set.len()));
        for scope in active_scopes {
            let authoritative = report
                .authority_projection(scope.clone())
                .map_err(|error| map_projection_error(scope.clone(), error))?;
            let next_head = if let Some(current) = self.heads.get(&scope) {
                let successor = current.successor(authoritative)?;
                if successor.projection() == current.projection() {
                    current.clone()
                } else {
                    successor
                }
            } else {
                ArtifactQuotaHead::initial(authoritative)?
            };
            heads.insert(scope, next_head);
        }
        for scope in retired_scope_set {
            let head = self
                .heads
                .get(&scope)
                .ok_or(ArtifactQuotaOwnerError::InvalidScopeSet)?
                .clone();
            heads.insert(scope, head);
        }
        let checkpoint_digest = checkpoint_digest(&self.limit_snapshot_digest, &heads)?;
        Ok(Self {
            limit_snapshot_digest: self.limit_snapshot_digest.clone(),
            heads,
            checkpoint_digest,
        })
    }

    /// Returns one exact current head.
    #[must_use]
    pub(crate) fn head(&self, scope: &ArtifactQuotaScope) -> Option<&ArtifactQuotaHead> {
        self.heads.get(scope)
    }

    /// Returns every scope/head pair in stable canonical scope order.
    #[must_use]
    pub(crate) fn sorted_heads(&self) -> Vec<(&ArtifactQuotaScope, &ArtifactQuotaHead)> {
        let mut heads = self.heads.iter().collect::<Vec<_>>();
        heads.sort_by_key(|(scope, _)| scope_sort_key(scope));
        heads
    }

    /// Returns the immutable configured limit snapshot.
    #[must_use]
    pub(crate) const fn limit_snapshot_digest(&self) -> &ContentDigest {
        &self.limit_snapshot_digest
    }

    /// Returns the full fixed-owner checkpoint of every current scope head.
    #[must_use]
    pub(crate) const fn checkpoint_digest(&self) -> &ContentDigest {
        &self.checkpoint_digest
    }

    /// Returns one exact store-head digest.
    pub(crate) fn store_head_digest(
        &self,
        store: &ArtifactStoreIdentity,
    ) -> Result<&ContentDigest, ArtifactQuotaOwnerError> {
        self.head_digest(&ArtifactQuotaScope::Store(store.clone()))
    }

    /// Returns one exact project-head digest.
    pub(crate) fn project_head_digest(
        &self,
        project_id: &ProjectId,
    ) -> Result<&ContentDigest, ArtifactQuotaOwnerError> {
        self.head_digest(&ArtifactQuotaScope::Project(project_id.clone()))
    }

    /// Returns one exact object-generation-head digest.
    pub(crate) fn object_head_digest(
        &self,
        object: &ArtifactObjectIdentity,
    ) -> Result<&ContentDigest, ArtifactQuotaOwnerError> {
        self.head_digest(&ArtifactQuotaScope::Object(object.clone()))
    }

    fn head_digest(
        &self,
        scope: &ArtifactQuotaScope,
    ) -> Result<&ContentDigest, ArtifactQuotaOwnerError> {
        self.head(scope)
            .map(ArtifactQuotaHead::head_digest)
            .ok_or_else(|| ArtifactQuotaOwnerError::MissingHead(scope.clone()))
    }

    /// Hashes the sorted task heads relevant to one exact object generation.
    pub(crate) fn combined_task_head_digest(
        &self,
        object: &ArtifactObjectIdentity,
        task_ids: &[TaskId],
    ) -> Result<ContentDigest, ArtifactQuotaOwnerError> {
        let mut task_ids = task_ids.to_vec();
        task_ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        task_ids.dedup();
        let mut rows = Vec::with_capacity(task_ids.len());
        for task_id in task_ids {
            let scope = ArtifactQuotaScope::Task {
                project_id: object.key().project_id().clone(),
                task_id,
            };
            let head = self
                .head(&scope)
                .ok_or_else(|| ArtifactQuotaOwnerError::MissingHead(scope.clone()))?;
            rows.push(head_row(&scope, head));
        }
        owner_digest(
            "lattice.artifact.quota-owner.object-task-head-set",
            &CanonicalValue::Object(vec![
                (
                    "algorithm".to_owned(),
                    CanonicalValue::String(object.key().algorithm().to_owned()),
                ),
                (
                    "content_digest".to_owned(),
                    CanonicalValue::String(object.key().content_digest().as_str().to_owned()),
                ),
                (
                    "generation".to_owned(),
                    CanonicalValue::String(object.generation().get().to_string()),
                ),
                (
                    "limit_snapshot_digest".to_owned(),
                    CanonicalValue::String(self.limit_snapshot_digest.as_str().to_owned()),
                ),
                (
                    "producer_id".to_owned(),
                    CanonicalValue::String(ARTIFACT_STORE_PRODUCER_ID.to_owned()),
                ),
                (
                    "producer_version".to_owned(),
                    CanonicalValue::String(ARTIFACT_STORE_PRODUCER_VERSION.to_owned()),
                ),
                (
                    "project_id".to_owned(),
                    CanonicalValue::String(object.key().project_id().as_str().to_owned()),
                ),
                (
                    "runtime".to_owned(),
                    CanonicalValue::String("FAKE".to_owned()),
                ),
                ("task_heads".to_owned(), CanonicalValue::Array(rows)),
            ]),
        )
    }

    /// Hashes relevant sorted task heads together with the exact store head.
    pub(crate) fn staging_quota_digest(
        &self,
        tasks: &[(ProjectId, TaskId)],
    ) -> Result<ContentDigest, ArtifactQuotaOwnerError> {
        let store_heads = self
            .sorted_heads()
            .into_iter()
            .filter(|(scope, _)| matches!(scope, ArtifactQuotaScope::Store(_)))
            .collect::<Vec<_>>();
        if store_heads.len() != 1 {
            return Err(ArtifactQuotaOwnerError::InvalidScopeSet);
        }
        let (store_scope, store_head) = store_heads[0];
        let mut tasks = tasks.to_vec();
        tasks.sort_by(|left, right| {
            (left.0.as_str(), left.1.as_str()).cmp(&(right.0.as_str(), right.1.as_str()))
        });
        tasks.dedup();
        let mut rows = Vec::with_capacity(tasks.len());
        for (project_id, task_id) in tasks {
            let scope = ArtifactQuotaScope::Task {
                project_id,
                task_id,
            };
            let head = self
                .head(&scope)
                .ok_or_else(|| ArtifactQuotaOwnerError::MissingHead(scope.clone()))?;
            rows.push(head_row(&scope, head));
        }
        owner_digest(
            "lattice.artifact.quota-owner.staging-head-set",
            &CanonicalValue::Object(vec![
                (
                    "limit_snapshot_digest".to_owned(),
                    CanonicalValue::String(self.limit_snapshot_digest.as_str().to_owned()),
                ),
                (
                    "producer_id".to_owned(),
                    CanonicalValue::String(ARTIFACT_STORE_PRODUCER_ID.to_owned()),
                ),
                (
                    "producer_version".to_owned(),
                    CanonicalValue::String(ARTIFACT_STORE_PRODUCER_VERSION.to_owned()),
                ),
                (
                    "runtime".to_owned(),
                    CanonicalValue::String("FAKE".to_owned()),
                ),
                ("store_head".to_owned(), head_row(store_scope, store_head)),
                ("task_heads".to_owned(), CanonicalValue::Array(rows)),
            ]),
        )
    }
}

fn map_projection_error(
    scope: ArtifactQuotaScope,
    error: ArtifactQuotaError,
) -> ArtifactQuotaOwnerError {
    if matches!(error, ArtifactQuotaError::MissingScope) {
        ArtifactQuotaOwnerError::MissingScope(scope)
    } else {
        ArtifactQuotaOwnerError::Quota(error)
    }
}

fn normalize_scopes(
    input: impl IntoIterator<Item = ArtifactQuotaScope>,
) -> Result<Vec<ArtifactQuotaScope>, ArtifactQuotaOwnerError> {
    let mut unique = HashSet::new();
    let mut scopes = Vec::new();
    for scope in input {
        if !unique.insert(scope.clone()) {
            return Err(ArtifactQuotaOwnerError::InvalidScopeSet);
        }
        scopes.push(scope);
    }
    if scopes.is_empty()
        || scopes
            .iter()
            .filter(|scope| matches!(scope, ArtifactQuotaScope::Store(_)))
            .count()
            != 1
    {
        return Err(ArtifactQuotaOwnerError::InvalidScopeSet);
    }
    scopes.sort_by_key(scope_sort_key);
    Ok(scopes)
}

fn normalize_retired_object_scopes(
    input: impl IntoIterator<Item = ArtifactQuotaScope>,
    existing_heads: &HashMap<ArtifactQuotaScope, ArtifactQuotaHead>,
    active_scopes: &HashSet<ArtifactQuotaScope>,
) -> Result<HashSet<ArtifactQuotaScope>, ArtifactQuotaOwnerError> {
    let mut retired = HashSet::new();
    for scope in input {
        if !matches!(scope, ArtifactQuotaScope::Object(_))
            || !existing_heads.contains_key(&scope)
            || active_scopes.contains(&scope)
            || !retired.insert(scope)
        {
            return Err(ArtifactQuotaOwnerError::InvalidScopeSet);
        }
    }
    Ok(retired)
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ScopeSortKey(u8, String, String, String, u64);

fn scope_sort_key(scope: &ArtifactQuotaScope) -> ScopeSortKey {
    match scope {
        ArtifactQuotaScope::Store(store) => ScopeSortKey(
            0,
            store.as_str().to_owned(),
            String::new(),
            String::new(),
            0,
        ),
        ArtifactQuotaScope::Project(project_id) => ScopeSortKey(
            1,
            project_id.as_str().to_owned(),
            String::new(),
            String::new(),
            0,
        ),
        ArtifactQuotaScope::Task {
            project_id,
            task_id,
        } => ScopeSortKey(
            2,
            project_id.as_str().to_owned(),
            task_id.as_str().to_owned(),
            String::new(),
            0,
        ),
        ArtifactQuotaScope::Object(object) => ScopeSortKey(
            3,
            object.key().project_id().as_str().to_owned(),
            object.key().algorithm().to_owned(),
            object.key().content_digest().as_str().to_owned(),
            object.generation().get(),
        ),
    }
}

fn checkpoint_digest(
    limit_snapshot_digest: &ContentDigest,
    heads: &HashMap<ArtifactQuotaScope, ArtifactQuotaHead>,
) -> Result<ContentDigest, ArtifactQuotaOwnerError> {
    let mut sorted = heads.iter().collect::<Vec<_>>();
    sorted.sort_by_key(|(scope, _)| scope_sort_key(scope));
    let rows = sorted
        .into_iter()
        .map(|(scope, head)| head_row(scope, head))
        .collect();
    owner_digest(
        "lattice.artifact.quota-owner.checkpoint",
        &CanonicalValue::Object(vec![
            ("heads".to_owned(), CanonicalValue::Array(rows)),
            (
                "limit_snapshot_digest".to_owned(),
                CanonicalValue::String(limit_snapshot_digest.as_str().to_owned()),
            ),
            (
                "producer_id".to_owned(),
                CanonicalValue::String(ARTIFACT_STORE_PRODUCER_ID.to_owned()),
            ),
            (
                "producer_version".to_owned(),
                CanonicalValue::String(ARTIFACT_STORE_PRODUCER_VERSION.to_owned()),
            ),
            (
                "runtime".to_owned(),
                CanonicalValue::String(runtime_label(RuntimeKind::Fake).to_owned()),
            ),
        ]),
    )
}

fn head_row(scope: &ArtifactQuotaScope, head: &ArtifactQuotaHead) -> CanonicalValue {
    CanonicalValue::Object(vec![
        (
            "head_digest".to_owned(),
            CanonicalValue::String(head.head_digest().as_str().to_owned()),
        ),
        (
            "limit_snapshot_digest".to_owned(),
            CanonicalValue::String(head.limit_snapshot_digest().as_str().to_owned()),
        ),
        (
            "predecessor_head_digest".to_owned(),
            CanonicalValue::String(head.predecessor_head_digest().as_str().to_owned()),
        ),
        (
            "revision".to_owned(),
            CanonicalValue::String(head.revision().get().to_string()),
        ),
        ("scope".to_owned(), scope_value(scope)),
        (
            "transition_tail_digest".to_owned(),
            CanonicalValue::String(head.transition_tail_digest().as_str().to_owned()),
        ),
    ])
}

fn scope_value(scope: &ArtifactQuotaScope) -> CanonicalValue {
    match scope {
        ArtifactQuotaScope::Store(store) => CanonicalValue::Object(vec![
            (
                "scope_type".to_owned(),
                CanonicalValue::String("store".to_owned()),
            ),
            (
                "store_id".to_owned(),
                CanonicalValue::String(store.as_str().to_owned()),
            ),
        ]),
        ArtifactQuotaScope::Project(project_id) => CanonicalValue::Object(vec![
            (
                "project_id".to_owned(),
                CanonicalValue::String(project_id.as_str().to_owned()),
            ),
            (
                "scope_type".to_owned(),
                CanonicalValue::String("project".to_owned()),
            ),
        ]),
        ArtifactQuotaScope::Task {
            project_id,
            task_id,
        } => CanonicalValue::Object(vec![
            (
                "project_id".to_owned(),
                CanonicalValue::String(project_id.as_str().to_owned()),
            ),
            (
                "scope_type".to_owned(),
                CanonicalValue::String("task".to_owned()),
            ),
            (
                "task_id".to_owned(),
                CanonicalValue::String(task_id.as_str().to_owned()),
            ),
        ]),
        ArtifactQuotaScope::Object(object) => CanonicalValue::Object(vec![
            (
                "algorithm".to_owned(),
                CanonicalValue::String(object.key().algorithm().to_owned()),
            ),
            (
                "content_digest".to_owned(),
                CanonicalValue::String(object.key().content_digest().as_str().to_owned()),
            ),
            (
                "generation".to_owned(),
                CanonicalValue::String(object.generation().get().to_string()),
            ),
            (
                "project_id".to_owned(),
                CanonicalValue::String(object.key().project_id().as_str().to_owned()),
            ),
            (
                "scope_type".to_owned(),
                CanonicalValue::String("object".to_owned()),
            ),
        ]),
    }
}

fn owner_digest(
    domain: &'static str,
    value: &CanonicalValue,
) -> Result<ContentDigest, ArtifactQuotaOwnerError> {
    let domain =
        HashDomain::new(domain, "1.0").map_err(|_| ArtifactQuotaOwnerError::Canonicalization)?;
    let digest =
        canonical_sha256(&domain, value).map_err(|_| ArtifactQuotaOwnerError::Canonicalization)?;
    ContentDigest::from_sha256(digest.to_hex()).map_err(|_| ArtifactQuotaOwnerError::InvalidDigest)
}

const fn runtime_label(runtime: RuntimeKind) -> &'static str {
    match runtime {
        RuntimeKind::Fake => "FAKE",
        RuntimeKind::Live => "LIVE",
    }
}

#[cfg(test)]
mod tests {
    use lattice_contracts::{
        ArtifactGeneration, ArtifactObjectIdentity, ArtifactObjectKey, ContentDigest, ProjectId,
        TaskId,
    };

    use super::{ArtifactQuotaHeadSet, ArtifactQuotaOwnerError};
    use crate::{
        ArtifactLimitKind, ArtifactObjectQuotaRecord, ArtifactObjectQuotaState, ArtifactQuotaScope,
        ArtifactQuotaSnapshot, ArtifactReferenceIdentity, ArtifactReferenceQuotaRecord,
        ArtifactReferenceQuotaState, ArtifactStoreIdentity, ArtifactStoreLimits,
    };

    fn project(value: &str) -> ProjectId {
        ProjectId::new(value).expect("project")
    }

    fn task(value: &str) -> TaskId {
        TaskId::new(value).expect("task")
    }

    fn object(project: &ProjectId, digit: char) -> ArtifactObjectIdentity {
        ArtifactObjectIdentity::new(
            ArtifactObjectKey::new(
                project.clone(),
                ContentDigest::from_sha256(digit.to_string().repeat(64)).expect("digest"),
            ),
            ArtifactGeneration::new(1).expect("generation"),
        )
    }

    fn next_generation(object: &ArtifactObjectIdentity, generation: u64) -> ArtifactObjectIdentity {
        ArtifactObjectIdentity::new(
            object.key().clone(),
            ArtifactGeneration::new(generation).expect("generation"),
        )
    }

    fn report(
        store: &ArtifactStoreIdentity,
        entries: &[(ArtifactObjectIdentity, TaskId, &str)],
        limits: ArtifactStoreLimits,
    ) -> crate::ArtifactQuotaReport {
        let objects = entries
            .iter()
            .map(|(identity, _, _)| {
                ArtifactObjectQuotaRecord::new(
                    identity.clone(),
                    10,
                    16,
                    0,
                    0,
                    ArtifactObjectQuotaState::Available,
                )
                .expect("object")
            })
            .collect();
        let references = entries
            .iter()
            .map(|(identity, task_id, reference_id)| {
                ArtifactReferenceQuotaRecord::new(
                    ArtifactReferenceIdentity::new(
                        task_id.clone(),
                        identity.key().clone(),
                        *reference_id,
                    )
                    .expect("reference identity"),
                    identity.clone(),
                    32,
                    ArtifactReferenceQuotaState::Active,
                )
                .expect("reference")
            })
            .collect();
        ArtifactQuotaSnapshot::new(store.clone(), objects, references, vec![], vec![], vec![])
            .recompute(limits)
            .expect("quota report")
    }

    fn scopes(
        store: &ArtifactStoreIdentity,
        entries: &[(ArtifactObjectIdentity, TaskId, &str)],
    ) -> Vec<ArtifactQuotaScope> {
        let mut scopes = vec![ArtifactQuotaScope::Store(store.clone())];
        for (object, task, _) in entries {
            let project_id = object.key().project_id().clone();
            let project_scope = ArtifactQuotaScope::Project(project_id.clone());
            if !scopes.contains(&project_scope) {
                scopes.push(project_scope);
            }
            let task_scope = ArtifactQuotaScope::Task {
                project_id,
                task_id: task.clone(),
            };
            if !scopes.contains(&task_scope) {
                scopes.push(task_scope);
            }
            scopes.push(ArtifactQuotaScope::Object(object.clone()));
        }
        scopes
    }

    #[test]
    fn same_project_b_updates_only_affected_heads() {
        let store = ArtifactStoreIdentity::new("root-store").expect("store");
        let p1 = project("project-a");
        let a = (object(&p1, 'a'), task("task-a"), "ref-a");
        let b = (object(&p1, 'b'), task("task-b"), "ref-b");
        let limits = ArtifactStoreLimits::hard_maximums();
        let report_a = report(&store, std::slice::from_ref(&a), limits);
        let scopes_a = scopes(&store, std::slice::from_ref(&a));
        let mut heads =
            ArtifactQuotaHeadSet::from_report(&report_a, scopes_a).expect("initial heads");
        let store_before = heads
            .head(&ArtifactQuotaScope::Store(store.clone()))
            .expect("store head")
            .clone();
        let project_before = heads
            .head(&ArtifactQuotaScope::Project(p1.clone()))
            .expect("project head")
            .clone();
        let task_a_scope = ArtifactQuotaScope::Task {
            project_id: p1.clone(),
            task_id: a.1.clone(),
        };
        let task_before = heads.head(&task_a_scope).expect("task head").clone();
        let object_before = heads
            .head(&ArtifactQuotaScope::Object(a.0.clone()))
            .expect("object head")
            .clone();

        let entries = vec![a.clone(), b.clone()];
        let report_b = report(&store, &entries, limits);
        heads
            .apply_report(&report_b, scopes(&store, &entries))
            .expect("apply B");

        let store_after = heads
            .head(&ArtifactQuotaScope::Store(store))
            .expect("store head");
        assert_eq!(
            store_after.revision().get(),
            store_before.revision().get() + 1
        );
        assert_eq!(
            store_after.predecessor_head_digest(),
            store_before.head_digest()
        );
        assert_ne!(store_after.head_digest(), store_before.head_digest());
        let project_after = heads
            .head(&ArtifactQuotaScope::Project(p1.clone()))
            .expect("project head");
        assert_eq!(
            project_after.revision().get(),
            project_before.revision().get() + 1
        );
        assert_eq!(
            project_after.predecessor_head_digest(),
            project_before.head_digest()
        );
        assert_ne!(project_after.head_digest(), project_before.head_digest());
        assert_eq!(heads.head(&task_a_scope), Some(&task_before));
        assert_eq!(
            heads.head(&ArtifactQuotaScope::Object(a.0)),
            Some(&object_before)
        );
        assert_eq!(
            heads
                .head(&ArtifactQuotaScope::Task {
                    project_id: p1,
                    task_id: b.1,
                })
                .expect("new task head")
                .revision()
                .get(),
            1
        );
        assert_eq!(
            heads
                .head(&ArtifactQuotaScope::Object(b.0))
                .expect("new object head")
                .revision()
                .get(),
            1
        );
    }

    #[test]
    fn unchanged_report_is_stable_and_different_project_b_is_isolated() {
        let store = ArtifactStoreIdentity::new("root-store-isolation").expect("store");
        let p1 = project("project-one");
        let p2 = project("project-two");
        let a = (object(&p1, '1'), task("task-one"), "ref-one");
        let b = (object(&p2, '2'), task("task-two"), "ref-two");
        let limits = ArtifactStoreLimits::hard_maximums();
        let report_a = report(&store, std::slice::from_ref(&a), limits);
        let scopes_a = scopes(&store, std::slice::from_ref(&a));
        let mut heads =
            ArtifactQuotaHeadSet::from_report(&report_a, scopes_a.clone()).expect("initial");
        let unchanged = heads.clone();
        let mut reversed = scopes_a;
        reversed.reverse();
        heads
            .apply_report(&report_a, reversed)
            .expect("unchanged report");
        assert_eq!(heads, unchanged);

        let store_before = heads
            .head(&ArtifactQuotaScope::Store(store.clone()))
            .expect("store")
            .clone();
        let project_before = heads
            .head(&ArtifactQuotaScope::Project(p1.clone()))
            .expect("project")
            .clone();
        let task_scope = ArtifactQuotaScope::Task {
            project_id: p1.clone(),
            task_id: a.1.clone(),
        };
        let task_before = heads.head(&task_scope).expect("task").clone();
        let object_scope = ArtifactQuotaScope::Object(a.0.clone());
        let object_before = heads.head(&object_scope).expect("object").clone();
        let checkpoint_before = heads.checkpoint_digest().clone();

        let entries = vec![a, b.clone()];
        let report_b = report(&store, &entries, limits);
        heads
            .apply_report(&report_b, scopes(&store, &entries))
            .expect("different-project B");

        assert_eq!(
            heads
                .head(&ArtifactQuotaScope::Store(store))
                .expect("store")
                .revision()
                .get(),
            store_before.revision().get() + 1
        );
        assert_eq!(
            heads.head(&ArtifactQuotaScope::Project(p1)),
            Some(&project_before)
        );
        assert_eq!(heads.head(&task_scope), Some(&task_before));
        assert_eq!(heads.head(&object_scope), Some(&object_before));
        assert_ne!(heads.checkpoint_digest(), &checkpoint_before);
        assert_eq!(
            heads
                .head(&ArtifactQuotaScope::Project(p2.clone()))
                .expect("new project")
                .revision()
                .get(),
            1
        );
        assert_eq!(
            heads
                .head(&ArtifactQuotaScope::Task {
                    project_id: p2,
                    task_id: b.1,
                })
                .expect("new task")
                .revision()
                .get(),
            1
        );
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn ordering_accessors_and_composed_digests_are_exact_and_deterministic() {
        let store = ArtifactStoreIdentity::new("root-store-digests").expect("store");
        let project = project("project-digests");
        let a = (object(&project, '3'), task("task-alpha"), "ref-alpha");
        let b = (object(&project, '4'), task("task-beta"), "ref-beta");
        let entries = vec![a.clone(), b.clone()];
        let limits = ArtifactStoreLimits::hard_maximums();
        let report = report(&store, &entries, limits);
        let ordered_scopes = scopes(&store, &entries);
        let mut reversed_scopes = ordered_scopes.clone();
        reversed_scopes.reverse();
        let ordered =
            ArtifactQuotaHeadSet::from_report(&report, ordered_scopes).expect("ordered heads");
        let reversed =
            ArtifactQuotaHeadSet::from_report(&report, reversed_scopes).expect("reversed heads");

        assert_eq!(ordered, reversed);
        assert_eq!(
            ordered.limit_snapshot_digest(),
            report.limit_snapshot_digest()
        );
        let ordered_rows = ordered
            .sorted_heads()
            .into_iter()
            .map(|(scope, head)| (scope.clone(), head.head_digest().clone()))
            .collect::<Vec<_>>();
        let reversed_rows = reversed
            .sorted_heads()
            .into_iter()
            .map(|(scope, head)| (scope.clone(), head.head_digest().clone()))
            .collect::<Vec<_>>();
        assert_eq!(ordered_rows, reversed_rows);
        assert_eq!(
            ordered_rows
                .iter()
                .map(|(scope, _)| scope.clone())
                .collect::<Vec<_>>(),
            vec![
                ArtifactQuotaScope::Store(store.clone()),
                ArtifactQuotaScope::Project(project.clone()),
                ArtifactQuotaScope::Task {
                    project_id: project.clone(),
                    task_id: a.1.clone(),
                },
                ArtifactQuotaScope::Task {
                    project_id: project.clone(),
                    task_id: b.1.clone(),
                },
                ArtifactQuotaScope::Object(a.0.clone()),
                ArtifactQuotaScope::Object(b.0.clone()),
            ]
        );

        assert_eq!(
            ordered.store_head_digest(&store).expect("store digest"),
            ordered
                .head(&ArtifactQuotaScope::Store(store.clone()))
                .expect("store head")
                .head_digest()
        );
        assert_eq!(
            ordered
                .project_head_digest(&project)
                .expect("project digest"),
            ordered
                .head(&ArtifactQuotaScope::Project(project.clone()))
                .expect("project head")
                .head_digest()
        );
        assert_eq!(
            ordered.object_head_digest(&a.0).expect("object digest"),
            ordered
                .head(&ArtifactQuotaScope::Object(a.0.clone()))
                .expect("object head")
                .head_digest()
        );

        let combined_ab = ordered
            .combined_task_head_digest(&a.0, &[b.1.clone(), a.1.clone(), a.1.clone()])
            .expect("combined task heads");
        let combined_reordered = ordered
            .combined_task_head_digest(&a.0, &[a.1.clone(), b.1.clone()])
            .expect("reordered task heads");
        let combined_single = ordered
            .combined_task_head_digest(&a.0, std::slice::from_ref(&a.1))
            .expect("one task head");
        assert_eq!(combined_ab, combined_reordered);
        assert_ne!(combined_ab, combined_single);

        let tasks_ab = vec![
            (project.clone(), b.1.clone()),
            (project.clone(), a.1.clone()),
            (project.clone(), a.1.clone()),
        ];
        let tasks_ba = vec![
            (project.clone(), a.1.clone()),
            (project.clone(), b.1.clone()),
        ];
        let staging_ab = ordered
            .staging_quota_digest(&tasks_ab)
            .expect("staging digest");
        let staging_reordered = ordered
            .staging_quota_digest(&tasks_ba)
            .expect("reordered staging digest");
        let staging_single = ordered
            .staging_quota_digest(&[(project, a.1)])
            .expect("one-task staging digest");
        assert_eq!(staging_ab, staging_reordered);
        assert_ne!(staging_ab, staging_single);
        assert_eq!(ordered.checkpoint_digest(), reversed.checkpoint_digest());
    }

    #[test]
    fn limit_scope_and_missing_head_failures_are_atomic() {
        let store = ArtifactStoreIdentity::new("root-store-failures").expect("store");
        let project = project("project-failures");
        let a = (object(&project, '5'), task("task-failure"), "ref-failure");
        let limits = ArtifactStoreLimits::hard_maximums();
        let report_a = report(&store, std::slice::from_ref(&a), limits);
        let scopes_a = scopes(&store, std::slice::from_ref(&a));
        let mut heads =
            ArtifactQuotaHeadSet::from_report(&report_a, scopes_a.clone()).expect("initial");
        let initial = heads.clone();

        let tightened_limits = limits
            .tighten(ArtifactLimitKind::ReferencesPerStore, 10)
            .expect("tightened limits");
        let tightened_report = report(&store, std::slice::from_ref(&a), tightened_limits);
        let error = heads
            .apply_report(&tightened_report, scopes_a.clone())
            .expect_err("limit drift");
        assert_eq!(error, ArtifactQuotaOwnerError::LimitSnapshotDrift);
        assert_eq!(error.code(), "ARTIFACT_QUOTA_OWNER_LIMIT_SNAPSHOT_DRIFT");
        assert_eq!(heads, initial);

        let scopes_without_object = scopes_a
            .iter()
            .filter(|scope| !matches!(scope, ArtifactQuotaScope::Object(_)))
            .cloned()
            .collect::<Vec<_>>();
        let error = heads
            .apply_report(&report_a, scopes_without_object)
            .expect_err("scope drift");
        assert_eq!(error, ArtifactQuotaOwnerError::ScopeDrift);
        assert_eq!(heads, initial);

        let missing_object = object(&project, '6');
        let missing_scope = ArtifactQuotaScope::Object(missing_object.clone());
        let mut scopes_with_missing = scopes_a.clone();
        scopes_with_missing.push(missing_scope.clone());
        let error = heads
            .apply_report(&report_a, scopes_with_missing)
            .expect_err("missing projection");
        assert_eq!(
            error,
            ArtifactQuotaOwnerError::MissingScope(missing_scope.clone())
        );
        assert_eq!(error.code(), "ARTIFACT_QUOTA_OWNER_MISSING_SCOPE");
        assert_eq!(heads, initial);

        let mut duplicate_scopes = scopes_a.clone();
        duplicate_scopes.push(ArtifactQuotaScope::Store(store.clone()));
        let error = heads
            .apply_report(&report_a, duplicate_scopes)
            .expect_err("duplicate scope");
        assert_eq!(error, ArtifactQuotaOwnerError::InvalidScopeSet);
        assert_eq!(heads, initial);

        let error = heads
            .object_head_digest(&missing_object)
            .expect_err("missing head");
        assert_eq!(error, ArtifactQuotaOwnerError::MissingHead(missing_scope));
        assert_eq!(error.code(), "ARTIFACT_QUOTA_OWNER_MISSING_HEAD");
        assert_eq!(heads, initial);
    }

    #[test]
    fn retired_object_head_is_preserved_when_a_new_generation_becomes_active() {
        let store = ArtifactStoreIdentity::new("root-store-retired").expect("store");
        let project = project("project-retired");
        let old = (object(&project, '7'), task("task-retired"), "ref-old");
        let limits = ArtifactStoreLimits::hard_maximums();
        let old_report = report(&store, std::slice::from_ref(&old), limits);
        let old_scopes = scopes(&store, std::slice::from_ref(&old));
        let mut heads =
            ArtifactQuotaHeadSet::from_report(&old_report, old_scopes).expect("old generation");
        let old_scope = ArtifactQuotaScope::Object(old.0.clone());
        let old_head = heads.head(&old_scope).expect("old object head").clone();
        let checkpoint_before = heads.checkpoint_digest().clone();

        let new = (
            next_generation(&old.0, 2),
            old.1.clone(),
            "ref-new-generation",
        );
        let new_report = report(&store, std::slice::from_ref(&new), limits);
        let new_scopes = scopes(&store, std::slice::from_ref(&new));
        let new_scope = ArtifactQuotaScope::Object(new.0.clone());
        heads
            .apply_report_with_retired(&new_report, new_scopes, [old_scope.clone()])
            .expect("retire old and activate new generation");

        assert_eq!(heads.head(&old_scope), Some(&old_head));
        assert_eq!(
            heads
                .head(&new_scope)
                .expect("new generation head")
                .revision()
                .get(),
            1
        );
        assert_ne!(heads.checkpoint_digest(), &checkpoint_before);
        assert_eq!(
            heads
                .sorted_heads()
                .into_iter()
                .filter(|(scope, _)| matches!(scope, ArtifactQuotaScope::Object(_)))
                .count(),
            2
        );
    }

    #[test]
    fn illegal_retired_scopes_are_rejected_without_mutation() {
        let store = ArtifactStoreIdentity::new("root-store-invalid-retired").expect("store");
        let project = project("project-invalid-retired");
        let old = (
            object(&project, '8'),
            task("task-invalid-retired"),
            "ref-old",
        );
        let limits = ArtifactStoreLimits::hard_maximums();
        let current_report = report(&store, std::slice::from_ref(&old), limits);
        let current_scopes = scopes(&store, std::slice::from_ref(&old));
        let mut heads = ArtifactQuotaHeadSet::from_report(&current_report, current_scopes.clone())
            .expect("current heads");
        let unchanged = heads.clone();
        let object_scope = ArtifactQuotaScope::Object(old.0.clone());
        let task_scope = ArtifactQuotaScope::Task {
            project_id: project.clone(),
            task_id: old.1.clone(),
        };
        let absent_object_scope =
            ArtifactQuotaScope::Object(next_generation(&object(&project, '9'), 2));

        for retired in [
            vec![task_scope],
            vec![absent_object_scope],
            vec![object_scope.clone()],
        ] {
            let active_scopes = if retired == vec![object_scope.clone()] {
                current_scopes.clone()
            } else {
                current_scopes
                    .iter()
                    .filter(|scope| !retired.contains(scope))
                    .cloned()
                    .collect()
            };
            assert_eq!(
                heads.apply_report_with_retired(&current_report, active_scopes, retired,),
                Err(ArtifactQuotaOwnerError::InvalidScopeSet)
            );
            assert_eq!(heads, unchanged);
        }
    }
}
