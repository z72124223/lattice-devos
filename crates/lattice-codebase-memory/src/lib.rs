//! Pure deterministic normalization and retrieval for structural Codebase Memory.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

use lattice_cjson::{CanonicalError, CanonicalValue, HashDomain, canonical_sha256, normalize_nfc};
use lattice_contracts::{
    CodeSnapshotEvidence, ContentDigest, GRAPH_MEMORY_RETRIEVAL_ALGORITHM, GraphConfidence,
    GraphMemoryContractError, GraphMemoryRecord, GraphMemoryRunRequest, GraphRecordKind,
    GraphSourceProvenance, GraphifyIdentity, GraphifyRawEvidence, MemoryQuery, MemoryRetrievalPlan,
    NormalizedGraphAnalysis, RankedMemoryRecord,
};

const MAX_CHANGED_PATHS: usize = 4_096;

/// Pure graph-memory validation, canonicalization, or ranking failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CodebaseMemoryError {
    /// A shared typed contract rejected the constructed value.
    Contract(GraphMemoryContractError),
    /// Canonical byte framing or hashing failed.
    Canonical(CanonicalError),
    /// Two upstream graph values collapse to the same canonical record.
    DuplicateCanonicalRecord,
    /// Ephemeral query text disagrees with the request's committed query digest.
    QueryDigestMismatch,
    /// A bounded ordinal, rank, score, or collection cannot be represented.
    CapacityExceeded,
    /// A Git changed path is not a canonical repository-relative path.
    InvalidChangedPath,
}

impl fmt::Display for CodebaseMemoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contract(error) => write!(formatter, "graph-memory contract rejected: {error}"),
            Self::Canonical(error) => {
                write!(formatter, "graph-memory canonicalization failed: {error}")
            }
            Self::DuplicateCanonicalRecord => {
                formatter.write_str("duplicate canonical graph-memory record")
            }
            Self::QueryDigestMismatch => {
                formatter.write_str("memory query does not match request digest")
            }
            Self::CapacityExceeded => formatter.write_str("graph-memory capacity exceeded"),
            Self::InvalidChangedPath => {
                formatter.write_str("invalid repository-relative changed path")
            }
        }
    }
}

/// One existing Graphify/Codebase Memory node selected by an exact changed path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChangedPathNode {
    relative_path: String,
    record_id: ContentDigest,
    subject: String,
    category: String,
}

impl ChangedPathNode {
    #[must_use]
    pub fn relative_path(&self) -> &str {
        &self.relative_path
    }

    #[must_use]
    pub const fn record_id(&self) -> &ContentDigest {
        &self.record_id
    }

    #[must_use]
    pub fn subject(&self) -> &str {
        &self.subject
    }

    #[must_use]
    pub fn category(&self) -> &str {
        &self.category
    }
}

/// Closed, deterministic impact seeds derived only from exact changed paths.
///
/// This value deliberately contains nodes only. It does not traverse incoming
/// or outgoing graph edges, so later impact stages cannot mistake adjacency for
/// evidence that a node was directly changed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChangedPathImpactInput {
    analysis_digest: ContentDigest,
    changed_paths: Vec<String>,
    nodes: Vec<ChangedPathNode>,
}

impl ChangedPathImpactInput {
    /// Exact normalized analysis from which these closed impact seeds were selected.
    #[must_use]
    pub const fn analysis_digest(&self) -> &ContentDigest {
        &self.analysis_digest
    }

    #[must_use]
    pub fn changed_paths(&self) -> &[String] {
        &self.changed_paths
    }

    #[must_use]
    pub fn nodes(&self) -> &[ChangedPathNode] {
        &self.nodes
    }
}

/// Maps Git changed paths to existing graph-memory nodes without graph traversal.
///
/// Paths and nodes are sorted and deduplicated, making the result a closed,
/// deterministic input for a separately owned impact-analysis stage. Changed
/// paths with no existing node remain visible in `changed_paths` and add no node.
///
/// # Errors
///
/// Rejects an oversized collection or empty, absolute, colon, backslash, dot,
/// parent, control-character, padded, NUL, or oversized repository-relative path.
pub fn map_changed_paths_to_nodes(
    analysis: &NormalizedGraphAnalysis,
    changed_paths: impl IntoIterator<Item = impl AsRef<str>>,
) -> Result<ChangedPathImpactInput, CodebaseMemoryError> {
    let mut changed_paths = changed_paths
        .into_iter()
        .map(|path| path.as_ref().to_owned())
        .collect::<Vec<_>>();
    if changed_paths.len() > MAX_CHANGED_PATHS {
        return Err(CodebaseMemoryError::CapacityExceeded);
    }
    if changed_paths.iter().any(|path| !valid_changed_path(path)) {
        return Err(CodebaseMemoryError::InvalidChangedPath);
    }
    changed_paths.sort();
    changed_paths.dedup();
    let changed = changed_paths
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();

    let mut nodes = analysis
        .records()
        .iter()
        .filter(|record| {
            record.graph_kind() == GraphRecordKind::Node
                && changed.contains(record.provenance().relative_path())
        })
        .map(|record| ChangedPathNode {
            relative_path: record.provenance().relative_path().to_owned(),
            record_id: record.record_id().clone(),
            subject: record.subject().to_owned(),
            category: record.category().to_owned(),
        })
        .collect::<Vec<_>>();
    nodes.sort_by(|left, right| left.record_id.as_str().cmp(right.record_id.as_str()));

    Ok(ChangedPathImpactInput {
        analysis_digest: analysis.analysis_digest().clone(),
        changed_paths,
        nodes,
    })
}

fn valid_changed_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 1_024
        && path.trim() == path
        && !path.starts_with('/')
        && !path.starts_with('\\')
        && !path.contains('\\')
        && !path.contains(':')
        && !path.chars().any(char::is_control)
        && path
            .split('/')
            .all(|component| !component.is_empty() && component != "." && component != "..")
}

impl Error for CodebaseMemoryError {}

impl From<GraphMemoryContractError> for CodebaseMemoryError {
    fn from(value: GraphMemoryContractError) -> Self {
        Self::Contract(value)
    }
}

impl From<CanonicalError> for CodebaseMemoryError {
    fn from(value: CanonicalError) -> Self {
        Self::Canonical(value)
    }
}

#[derive(Clone)]
struct DraftRecord {
    kind: GraphRecordKind,
    subject: String,
    category: String,
    relation: Option<String>,
    object: Option<String>,
    provenance: GraphSourceProvenance,
    confidence: GraphConfidence,
    content_digest: ContentDigest,
    record_id: ContentDigest,
}

/// Computes the canonical digest committed by a graph-memory run request.
///
/// Raw query text is used only ephemerally; only this digest enters durable
/// analysis and retrieval values.
///
/// # Errors
///
/// Returns a canonicalization failure for unrepresentable input.
pub fn digest_query_text(query: &str) -> Result<ContentDigest, CodebaseMemoryError> {
    let normalized = normalize_query(query);
    let value = CanonicalValue::Object(vec![("query".to_owned(), string(normalized))]);
    hash_value("lattice.graph-memory.query", &value)
}

/// Converts complete raw Graphify evidence into deterministic candidate records.
///
/// # Errors
///
/// Rejects cross-bound evidence, duplicate canonical records, capacity
/// overflow, or any shared contract/canonicalization failure.
pub fn normalize_analysis(
    request: &GraphMemoryRunRequest,
    snapshot: &CodeSnapshotEvidence,
    raw: &GraphifyRawEvidence,
) -> Result<NormalizedGraphAnalysis, CodebaseMemoryError> {
    if snapshot.request() != request || raw.request() != request {
        return Err(GraphMemoryContractError::CrossBinding {
            field: "memory_normalization_input",
        }
        .into());
    }

    let identity_digest = graphify_identity_digest(raw.identity())?;
    let node_labels = raw
        .nodes()
        .iter()
        .map(|node| {
            (
                node.upstream_id(),
                (
                    normalize_structural(node.label()),
                    normalize_structural(node.category()),
                ),
            )
        })
        .collect::<BTreeMap<_, _>>();

    let mut drafts = Vec::with_capacity(raw.nodes().len() + raw.edges().len());
    for node in raw.nodes() {
        drafts.push(draft_record(
            request,
            snapshot,
            GraphRecordKind::Node,
            normalize_structural(node.label()),
            normalize_structural(node.category()),
            None,
            None,
            node.provenance().clone(),
            node.confidence(),
        )?);
    }
    for edge in raw.edges() {
        let (subject, _) = node_labels
            .get(edge.source_node_id())
            .ok_or(GraphMemoryContractError::DanglingGraphEdge)?;
        let (object, _) = node_labels
            .get(edge.target_node_id())
            .ok_or(GraphMemoryContractError::DanglingGraphEdge)?;
        drafts.push(draft_record(
            request,
            snapshot,
            GraphRecordKind::Edge,
            subject.clone(),
            "relation".to_owned(),
            Some(normalize_structural(edge.relation())),
            Some(object.clone()),
            edge.provenance().clone(),
            edge.confidence(),
        )?);
    }

    drafts.sort_by(|left, right| left.record_id.as_str().cmp(right.record_id.as_str()));
    if drafts
        .windows(2)
        .any(|pair| pair[0].record_id == pair[1].record_id)
    {
        return Err(CodebaseMemoryError::DuplicateCanonicalRecord);
    }

    let mut records = Vec::with_capacity(drafts.len());
    for (index, draft) in drafts.into_iter().enumerate() {
        let ordinal =
            u32::try_from(index + 1).map_err(|_| CodebaseMemoryError::CapacityExceeded)?;
        records.push(GraphMemoryRecord::candidate(
            ordinal,
            draft.kind,
            draft.subject,
            draft.category,
            draft.relation,
            draft.object,
            draft.provenance,
            draft.confidence,
            draft.content_digest,
            draft.record_id,
        )?);
    }

    let record_set_digest = record_set_digest(&records)?;
    let analysis_digest =
        analysis_digest(request, snapshot, raw, &identity_digest, &record_set_digest)?;
    Ok(NormalizedGraphAnalysis::new(
        request,
        snapshot,
        raw,
        records,
        identity_digest,
        record_set_digest,
        analysis_digest,
    )?)
}

/// Produces a deterministic relevance-ranked retrieval/audit plan.
///
/// # Errors
///
/// Rejects a cross-bound analysis/query, a query digest mismatch, overflow, or
/// any canonical/shared-contract construction failure.
pub fn plan_retrieval(
    analysis: &NormalizedGraphAnalysis,
    query: &MemoryQuery,
) -> Result<MemoryRetrievalPlan, CodebaseMemoryError> {
    if query.request() != analysis.request() {
        return Err(GraphMemoryContractError::CrossBinding {
            field: "memory_retrieval_query",
        }
        .into());
    }
    if digest_query_text(query.text())? != *query.query_digest() {
        return Err(CodebaseMemoryError::QueryDigestMismatch);
    }

    let normalized_query = normalize_query(query.text());
    let query_tokens = tokens(&normalized_query);
    let mut scored = analysis
        .records()
        .iter()
        .filter_map(|record| {
            let score = relevance_score(record, &normalized_query, &query_tokens);
            (score > 0).then_some((score, record))
        })
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| {
        right.0.cmp(&left.0).then_with(|| {
            left.1
                .record_id()
                .as_str()
                .cmp(right.1.record_id().as_str())
        })
    });
    scored.truncate(usize::from(query.limit()));

    let mut results = Vec::with_capacity(scored.len());
    for (index, (score, record)) in scored.into_iter().enumerate() {
        let rank = u16::try_from(index + 1).map_err(|_| CodebaseMemoryError::CapacityExceeded)?;
        results.push(RankedMemoryRecord::new(record, rank, score)?);
    }
    let result_set_digest = retrieval_result_set_digest(analysis, query, &results)?;
    Ok(MemoryRetrievalPlan::new(
        analysis,
        query,
        results,
        result_set_digest,
    )?)
}

#[allow(clippy::too_many_arguments)]
fn draft_record(
    request: &GraphMemoryRunRequest,
    snapshot: &CodeSnapshotEvidence,
    kind: GraphRecordKind,
    subject: String,
    category: String,
    relation: Option<String>,
    object: Option<String>,
    provenance: GraphSourceProvenance,
    confidence: GraphConfidence,
) -> Result<DraftRecord, CodebaseMemoryError> {
    let semantic = CanonicalValue::Object(vec![
        ("category".to_owned(), string(&category)),
        ("confidence".to_owned(), string(confidence.as_str())),
        ("kind".to_owned(), string(kind.as_str())),
        ("object".to_owned(), optional_string(object.as_deref())),
        ("path".to_owned(), string(provenance.relative_path())),
        ("relation".to_owned(), optional_string(relation.as_deref())),
        (
            "source_digest".to_owned(),
            string(provenance.content_digest().as_str()),
        ),
        (
            "line_start".to_owned(),
            optional_u32(provenance.line_start()),
        ),
        ("line_end".to_owned(), optional_u32(provenance.line_end())),
        ("subject".to_owned(), string(&subject)),
    ]);
    let content_digest = hash_value("lattice.graph-memory.record-content", &semantic)?;
    let record_identity = CanonicalValue::Object(vec![
        ("commit".to_owned(), string(request.commit_id().as_str())),
        ("content".to_owned(), string(content_digest.as_str())),
        ("project".to_owned(), string(request.project_id().as_str())),
        (
            "snapshot".to_owned(),
            string(request.invocation().project_snapshot_id().as_str()),
        ),
        ("tree".to_owned(), string(snapshot.tree_id().as_str())),
    ]);
    let record_id = hash_value("lattice.graph-memory.record-id", &record_identity)?;
    Ok(DraftRecord {
        kind,
        subject,
        category,
        relation,
        object,
        provenance,
        confidence,
        content_digest,
        record_id,
    })
}

fn graphify_identity_digest(
    identity: &GraphifyIdentity,
) -> Result<ContentDigest, CodebaseMemoryError> {
    let value = CanonicalValue::Object(vec![
        ("adapter".to_owned(), string(identity.adapter_version())),
        (
            "capability".to_owned(),
            string(identity.capability_digest().as_str()),
        ),
        (
            "cli_help".to_owned(),
            string(identity.cli_help_digest().as_str()),
        ),
        (
            "executable".to_owned(),
            string(identity.executable_digest().as_str()),
        ),
        ("license".to_owned(), string(identity.license())),
        ("package".to_owned(), string(identity.package())),
        (
            "upstream_commit".to_owned(),
            string(identity.upstream_commit()),
        ),
        ("version".to_owned(), string(identity.version())),
        ("wheel".to_owned(), string(identity.wheel_digest().as_str())),
    ]);
    hash_value("lattice.graph-memory.graphify-identity", &value)
}

fn record_set_digest(records: &[GraphMemoryRecord]) -> Result<ContentDigest, CodebaseMemoryError> {
    let values = records
        .iter()
        .map(|record| {
            CanonicalValue::Object(vec![
                (
                    "content".to_owned(),
                    string(record.content_digest().as_str()),
                ),
                ("id".to_owned(), string(record.record_id().as_str())),
                ("ordinal".to_owned(), string(record.ordinal().to_string())),
            ])
        })
        .collect();
    let value = CanonicalValue::Array(values);
    hash_value("lattice.graph-memory.record-set", &value)
}

fn analysis_digest(
    request: &GraphMemoryRunRequest,
    snapshot: &CodeSnapshotEvidence,
    raw: &GraphifyRawEvidence,
    identity_digest: &ContentDigest,
    record_set_digest: &ContentDigest,
) -> Result<ContentDigest, CodebaseMemoryError> {
    let value = CanonicalValue::Object(vec![
        ("commit".to_owned(), string(request.commit_id().as_str())),
        (
            "configuration".to_owned(),
            string(request.configuration_digest().as_str()),
        ),
        (
            "exclusion".to_owned(),
            string(snapshot.exclusion_digest().as_str()),
        ),
        (
            "graph_artifact".to_owned(),
            string(raw.graph_artifact_digest().as_str()),
        ),
        (
            "graph_raw".to_owned(),
            string(raw.raw_output_digest().as_str()),
        ),
        (
            "graph_evidence".to_owned(),
            string(raw.evidence_digest().as_str()),
        ),
        (
            "graphify_identity".to_owned(),
            string(identity_digest.as_str()),
        ),
        (
            "manifest".to_owned(),
            string(snapshot.manifest_digest().as_str()),
        ),
        ("project".to_owned(), string(request.project_id().as_str())),
        ("record_set".to_owned(), string(record_set_digest.as_str())),
        (
            "snapshot".to_owned(),
            string(request.invocation().project_snapshot_id().as_str()),
        ),
        ("tree".to_owned(), string(snapshot.tree_id().as_str())),
    ]);
    hash_value("lattice.graph-memory.analysis", &value)
}

fn retrieval_result_set_digest(
    analysis: &NormalizedGraphAnalysis,
    query: &MemoryQuery,
    results: &[RankedMemoryRecord],
) -> Result<ContentDigest, CodebaseMemoryError> {
    let values = results
        .iter()
        .map(|result| {
            CanonicalValue::Object(vec![
                ("id".to_owned(), string(result.record_id().as_str())),
                ("rank".to_owned(), string(result.rank().to_string())),
                ("score".to_owned(), string(result.score().to_string())),
            ])
        })
        .collect::<Vec<_>>();
    let value = CanonicalValue::Object(vec![
        (
            "algorithm".to_owned(),
            string(GRAPH_MEMORY_RETRIEVAL_ALGORITHM),
        ),
        (
            "analysis".to_owned(),
            string(analysis.analysis_digest().as_str()),
        ),
        ("limit".to_owned(), string(query.limit().to_string())),
        ("query".to_owned(), string(query.query_digest().as_str())),
        ("results".to_owned(), CanonicalValue::Array(values)),
    ]);
    hash_value("lattice.graph-memory.retrieval-result-set", &value)
}

fn relevance_score(
    record: &GraphMemoryRecord,
    query: &str,
    query_tokens: &BTreeSet<String>,
) -> u32 {
    let subject = normalize_query(record.subject());
    let category = normalize_query(record.category());
    let relation = record.relation().map(normalize_query);
    let object = record.object().map(normalize_query);
    let path = normalize_query(record.provenance().relative_path());
    let mut score = 0_u32;

    if subject == query {
        score += 1_200;
    }
    if object.as_deref() == Some(query) {
        score += 1_100;
    }
    if path == query {
        score += 1_000;
    }
    if category == query || relation.as_deref() == Some(query) {
        score += 800;
    }

    let searchable = [
        subject.as_str(),
        category.as_str(),
        relation.as_deref().unwrap_or(""),
        object.as_deref().unwrap_or(""),
        path.as_str(),
    ];
    let record_tokens = searchable
        .iter()
        .flat_map(|value| tokens(value))
        .collect::<BTreeSet<_>>();
    for token in query_tokens {
        if record_tokens.contains(token) {
            score += 100;
        } else if token.chars().count() >= 3 && searchable.iter().any(|value| value.contains(token))
        {
            score += 10;
        }
    }
    score
}

fn tokens(value: &str) -> BTreeSet<String> {
    value
        .split(|character: char| !(character.is_alphanumeric() || character == '_'))
        .filter(|token| !token.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn normalize_query(value: &str) -> String {
    normalize_structural(value).to_lowercase()
}

fn normalize_structural(value: &str) -> String {
    normalize_nfc(value)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn hash_value(
    schema_id: &str,
    value: &CanonicalValue,
) -> Result<ContentDigest, CodebaseMemoryError> {
    let domain = HashDomain::new(schema_id, "1")?;
    let digest = canonical_sha256(&domain, value)?.to_hex();
    ContentDigest::from_sha256(digest).map_err(|_| {
        CodebaseMemoryError::Contract(GraphMemoryContractError::InvalidValue {
            field: "canonical_digest",
        })
    })
}

fn string(value: impl Into<String>) -> CanonicalValue {
    CanonicalValue::String(value.into())
}

fn optional_string(value: Option<&str>) -> CanonicalValue {
    value.map_or(CanonicalValue::Null, string)
}

fn optional_u32(value: Option<u32>) -> CanonicalValue {
    value.map_or(CanonicalValue::Null, |value| string(value.to_string()))
}
