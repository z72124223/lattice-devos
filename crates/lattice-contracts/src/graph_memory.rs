//! Typed, I/O-free contracts for exact Git snapshot and graph-memory work.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

use crate::{ContentDigest, Invocation, ProjectId};

const MAX_SOURCE_PATH_BYTES: usize = 1_024;
const MAX_GRAPH_TEXT_BYTES: usize = 1_024;

/// Exact package identity pinned for the first executable graph-memory slice.
pub const GRAPHIFY_PACKAGE: &str = "graphifyy";
/// Exact upstream release pinned for the first executable graph-memory slice.
pub const GRAPHIFY_VERSION: &str = "0.9.33";
/// Immutable upstream commit behind [`GRAPHIFY_VERSION`].
pub const GRAPHIFY_UPSTREAM_COMMIT: &str = "4e7e6b1f7e0df10ed07d5f28f9189bbde42940f1";
/// SPDX license expression verified for the pinned package.
pub const GRAPHIFY_LICENSE: &str = "Apache-2.0";
/// Published wheel SHA-256 for `graphifyy==0.9.33`.
pub const GRAPHIFY_WHEEL_SHA256: &str =
    "c32b5792c783a6e66b1100b35bc65df3538e3f69b9df45fb098c9634c1b8eb01";
/// Semantic identity of the TASK-033 adapter boundary.
pub const GRAPHIFY_ADAPTER_VERSION: &str = "1.0";
/// First deterministic structural-memory ranking algorithm.
pub const GRAPH_MEMORY_RETRIEVAL_ALGORITHM: &str = "lattice-structural-retrieval-v1";
/// Maximum bounded records retained by one graph analysis.
pub const GRAPH_MEMORY_MAX_RECORDS: usize = 100_000;
/// Maximum results returned and audited for one process-owned query.
pub const GRAPH_MEMORY_MAX_RESULTS: u16 = 100;
/// Fixed identity of the independent same-database Memory extension.
pub const CODEBASE_MEMORY_EXTENSION_ID: &str = "lattice-codebase-memory";
/// First and only supported Memory extension schema version.
pub const CODEBASE_MEMORY_EXTENSION_SCHEMA_VERSION: u16 = 1;
/// Global Store schema profile required by the first Memory extension.
pub const CODEBASE_MEMORY_REQUIRED_GLOBAL_SCHEMA_VERSION: u16 = 3;

/// Structural construction failures for graph-memory boundary values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphMemoryContractError {
    /// A field is empty, malformed, unbounded, or carries a zero digest.
    InvalidValue { field: &'static str },
    /// The tracked source manifest is empty, duplicated, or not canonical.
    InvalidManifest,
    /// Evidence from different request/snapshot bindings was combined.
    CrossBinding { field: &'static str },
    /// A graph source is absent from the exact tracked manifest.
    UnknownSource,
    /// An upstream graph identifier is duplicated.
    DuplicateGraphId,
    /// An edge names a node absent from the same complete graph.
    DanglingGraphEdge,
}

impl fmt::Display for GraphMemoryContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidValue { field } => write!(formatter, "invalid graph-memory {field}"),
            Self::InvalidManifest => formatter.write_str("invalid graph-memory source manifest"),
            Self::CrossBinding { field } => write!(formatter, "cross-bound graph-memory {field}"),
            Self::UnknownSource => {
                formatter.write_str("graph source is absent from tracked manifest")
            }
            Self::DuplicateGraphId => formatter.write_str("duplicate graph identifier"),
            Self::DanglingGraphEdge => formatter.write_str("graph edge endpoint is absent"),
        }
    }
}

/// Closed confidence/provenance classification retained from Graphify output.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum GraphConfidence {
    /// Directly extracted from tracked source syntax.
    Extracted,
    /// Derived by the pinned analyzer and never authoritative by itself.
    Inferred,
    /// Analyzer output whose structural interpretation remains ambiguous.
    Ambiguous,
}

impl GraphConfidence {
    /// Returns the stable persistence-facing value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Extracted => "EXTRACTED",
            Self::Inferred => "INFERRED",
            Self::Ambiguous => "AMBIGUOUS",
        }
    }
}

impl Error for GraphMemoryContractError {}

/// Exact Git object identity accepted for a source commit or tree.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct GitObjectId(String);

impl GitObjectId {
    /// Validates a full lowercase SHA-1 or SHA-256 Git object identifier.
    ///
    /// # Errors
    ///
    /// Rejects abbreviated, uppercase, non-hexadecimal, or all-zero values.
    pub fn new(value: impl Into<String>) -> Result<Self, GraphMemoryContractError> {
        let value = value.into();
        let valid_length = matches!(value.len(), 40 | 64);
        let valid = valid_length
            && value
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
            && value.bytes().any(|byte| byte != b'0');
        if valid {
            Ok(Self(value))
        } else {
            Err(GraphMemoryContractError::InvalidValue {
                field: "git_object_id",
            })
        }
    }

    /// Returns the exact lowercase object identity.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Composition-created request for one exact graph-memory run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphMemoryRunRequest {
    invocation: Invocation,
    project_id: ProjectId,
    commit_id: GitObjectId,
    query_digest: ContentDigest,
    configuration_digest: ContentDigest,
    retrieval_limit: u16,
}

impl GraphMemoryRunRequest {
    /// Constructs a request from process-owned project/commit/query/configuration.
    ///
    /// # Errors
    ///
    /// Rejects zero query/configuration commitments or a retrieval limit
    /// outside `1..=GRAPH_MEMORY_MAX_RESULTS`.
    pub fn new(
        invocation: Invocation,
        project_id: ProjectId,
        commit_id: GitObjectId,
        query_digest: ContentDigest,
        configuration_digest: ContentDigest,
        retrieval_limit: u16,
    ) -> Result<Self, GraphMemoryContractError> {
        require_digest(&query_digest, "query_digest")?;
        require_digest(&configuration_digest, "configuration_digest")?;
        if retrieval_limit == 0 || retrieval_limit > GRAPH_MEMORY_MAX_RESULTS {
            return Err(GraphMemoryContractError::InvalidValue {
                field: "memory_retrieval_limit",
            });
        }
        Ok(Self {
            invocation,
            project_id,
            commit_id,
            query_digest,
            configuration_digest,
            retrieval_limit,
        })
    }

    #[must_use]
    pub const fn invocation(&self) -> &Invocation {
        &self.invocation
    }

    #[must_use]
    pub const fn project_id(&self) -> &ProjectId {
        &self.project_id
    }

    #[must_use]
    pub const fn commit_id(&self) -> &GitObjectId {
        &self.commit_id
    }

    #[must_use]
    pub const fn query_digest(&self) -> &ContentDigest {
        &self.query_digest
    }

    #[must_use]
    pub const fn configuration_digest(&self) -> &ContentDigest {
        &self.configuration_digest
    }

    #[must_use]
    pub const fn retrieval_limit(&self) -> u16 {
        self.retrieval_limit
    }
}

/// One exact tracked file in a canonical Git-commit source manifest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrackedSource {
    relative_path: String,
    content_digest: ContentDigest,
}

impl TrackedSource {
    /// Validates one forward-slash relative path and content digest.
    ///
    /// # Errors
    ///
    /// Rejects absolute, drive-qualified, backslash, empty/dot/parent component,
    /// NUL, oversized, or zero-digest input.
    pub fn new(
        relative_path: impl Into<String>,
        content_digest: ContentDigest,
    ) -> Result<Self, GraphMemoryContractError> {
        let relative_path = relative_path.into();
        if !valid_relative_path(&relative_path) {
            return Err(GraphMemoryContractError::InvalidValue {
                field: "source_relative_path",
            });
        }
        require_digest(&content_digest, "source_content_digest")?;
        Ok(Self {
            relative_path,
            content_digest,
        })
    }

    #[must_use]
    pub fn relative_path(&self) -> &str {
        &self.relative_path
    }

    #[must_use]
    pub const fn content_digest(&self) -> &ContentDigest {
        &self.content_digest
    }
}

/// Proof that an exact commit was materialized as a canonical tracked-only tree.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodeSnapshotEvidence {
    request: GraphMemoryRunRequest,
    tree_id: GitObjectId,
    sources: Vec<TrackedSource>,
    manifest_digest: ContentDigest,
    exclusion_digest: ContentDigest,
}

impl CodeSnapshotEvidence {
    /// Constructs exact tracked-source evidence.
    ///
    /// # Errors
    ///
    /// Rejects an empty, duplicate, or non-lexicographic manifest and zero
    /// manifest/exclusion commitments.
    pub fn new(
        request: &GraphMemoryRunRequest,
        tree_id: GitObjectId,
        sources: Vec<TrackedSource>,
        manifest_digest: ContentDigest,
        exclusion_digest: ContentDigest,
    ) -> Result<Self, GraphMemoryContractError> {
        if sources.is_empty()
            || sources
                .windows(2)
                .any(|pair| pair[0].relative_path() >= pair[1].relative_path())
        {
            return Err(GraphMemoryContractError::InvalidManifest);
        }
        require_digest(&manifest_digest, "source_manifest_digest")?;
        require_digest(&exclusion_digest, "source_exclusion_digest")?;
        Ok(Self {
            request: request.clone(),
            tree_id,
            sources,
            manifest_digest,
            exclusion_digest,
        })
    }

    #[must_use]
    pub const fn request(&self) -> &GraphMemoryRunRequest {
        &self.request
    }

    #[must_use]
    pub const fn commit_id(&self) -> &GitObjectId {
        self.request.commit_id()
    }

    #[must_use]
    pub const fn tree_id(&self) -> &GitObjectId {
        &self.tree_id
    }

    #[must_use]
    pub fn sources(&self) -> &[TrackedSource] {
        &self.sources
    }

    #[must_use]
    pub const fn manifest_digest(&self) -> &ContentDigest {
        &self.manifest_digest
    }

    #[must_use]
    pub const fn exclusion_digest(&self) -> &ContentDigest {
        &self.exclusion_digest
    }
}

/// Exact pinned Graphify runtime identity without an executable path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphifyIdentity {
    executable: ContentDigest,
    cli_help: ContentDigest,
    capability: ContentDigest,
    wheel: ContentDigest,
}

impl GraphifyIdentity {
    /// Constructs the sole Graphify identity accepted by TASK-033.
    ///
    /// The package, version, upstream commit, license, wheel hash, and adapter
    /// version are compile-time fixed; composition supplies only observed
    /// executable/help/capability commitments.
    ///
    /// # Errors
    ///
    /// Rejects any all-zero runtime commitment.
    pub fn task033(
        executable_digest: ContentDigest,
        cli_help_digest: ContentDigest,
        capability_digest: ContentDigest,
    ) -> Result<Self, GraphMemoryContractError> {
        require_digest(&executable_digest, "graphify_executable_digest")?;
        require_digest(&cli_help_digest, "graphify_cli_help_digest")?;
        require_digest(&capability_digest, "graphify_capability_digest")?;
        let wheel_digest = ContentDigest::from_sha256(GRAPHIFY_WHEEL_SHA256).map_err(|_| {
            GraphMemoryContractError::InvalidValue {
                field: "graphify_wheel_digest",
            }
        })?;
        Ok(Self {
            executable: executable_digest,
            cli_help: cli_help_digest,
            capability: capability_digest,
            wheel: wheel_digest,
        })
    }

    #[must_use]
    pub const fn package(&self) -> &'static str {
        GRAPHIFY_PACKAGE
    }

    #[must_use]
    pub const fn version(&self) -> &'static str {
        GRAPHIFY_VERSION
    }

    #[must_use]
    pub const fn upstream_commit(&self) -> &'static str {
        GRAPHIFY_UPSTREAM_COMMIT
    }

    #[must_use]
    pub const fn license(&self) -> &'static str {
        GRAPHIFY_LICENSE
    }

    #[must_use]
    pub const fn adapter_version(&self) -> &'static str {
        GRAPHIFY_ADAPTER_VERSION
    }

    #[must_use]
    pub const fn wheel_digest(&self) -> &ContentDigest {
        &self.wheel
    }

    #[must_use]
    pub const fn executable_digest(&self) -> &ContentDigest {
        &self.executable
    }

    #[must_use]
    pub const fn cli_help_digest(&self) -> &ContentDigest {
        &self.cli_help
    }

    #[must_use]
    pub const fn capability_digest(&self) -> &ContentDigest {
        &self.capability
    }
}

/// Manifest-bound source provenance for one graph node or edge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphSourceProvenance {
    relative_path: String,
    content_digest: ContentDigest,
    line_start: Option<u32>,
    line_end: Option<u32>,
}

impl GraphSourceProvenance {
    /// Binds an optional inclusive line range to one tracked source.
    ///
    /// # Errors
    ///
    /// Rejects zero, reversed, or only-half-present line ranges.
    pub fn new(
        source: &TrackedSource,
        line_start: Option<u32>,
        line_end: Option<u32>,
    ) -> Result<Self, GraphMemoryContractError> {
        if !matches!((line_start, line_end), (None, None))
            && !matches!((line_start, line_end), (Some(start), Some(end)) if start > 0 && end >= start)
        {
            return Err(GraphMemoryContractError::InvalidValue {
                field: "graph_source_line_range",
            });
        }
        Ok(Self {
            relative_path: source.relative_path.clone(),
            content_digest: source.content_digest.clone(),
            line_start,
            line_end,
        })
    }

    #[must_use]
    pub fn relative_path(&self) -> &str {
        &self.relative_path
    }

    #[must_use]
    pub const fn content_digest(&self) -> &ContentDigest {
        &self.content_digest
    }

    #[must_use]
    pub const fn line_start(&self) -> Option<u32> {
        self.line_start
    }

    #[must_use]
    pub const fn line_end(&self) -> Option<u32> {
        self.line_end
    }
}

/// One bounded raw node from a complete Graphify graph.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphifyRawNode {
    upstream_id: String,
    label: String,
    category: String,
    provenance: GraphSourceProvenance,
    confidence: GraphConfidence,
}

impl GraphifyRawNode {
    /// Constructs one raw node after bounded-text validation.
    ///
    /// # Errors
    ///
    /// Rejects empty, oversized, control-bearing identity/label/category text.
    pub fn new(
        upstream_id: impl Into<String>,
        label: impl Into<String>,
        category: impl Into<String>,
        provenance: GraphSourceProvenance,
        confidence: GraphConfidence,
    ) -> Result<Self, GraphMemoryContractError> {
        Ok(Self {
            upstream_id: graph_text(upstream_id, "graph_node_id")?,
            label: graph_text(label, "graph_node_label")?,
            category: graph_text(category, "graph_node_category")?,
            provenance,
            confidence,
        })
    }

    #[must_use]
    pub fn upstream_id(&self) -> &str {
        &self.upstream_id
    }

    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    #[must_use]
    pub fn category(&self) -> &str {
        &self.category
    }

    #[must_use]
    pub const fn provenance(&self) -> &GraphSourceProvenance {
        &self.provenance
    }

    #[must_use]
    pub const fn confidence(&self) -> GraphConfidence {
        self.confidence
    }
}

/// One bounded raw edge from a complete Graphify graph.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphifyRawEdge {
    upstream_id: String,
    source_node_id: String,
    target_node_id: String,
    relation: String,
    provenance: GraphSourceProvenance,
    confidence: GraphConfidence,
}

impl GraphifyRawEdge {
    /// Constructs one raw edge after bounded-text validation.
    ///
    /// # Errors
    ///
    /// Rejects empty, oversized, or control-bearing text. Endpoint membership
    /// is checked only when the complete graph evidence is constructed.
    pub fn new(
        upstream_id: impl Into<String>,
        source_node_id: impl Into<String>,
        target_node_id: impl Into<String>,
        relation: impl Into<String>,
        provenance: GraphSourceProvenance,
        confidence: GraphConfidence,
    ) -> Result<Self, GraphMemoryContractError> {
        Ok(Self {
            upstream_id: graph_text(upstream_id, "graph_edge_id")?,
            source_node_id: graph_text(source_node_id, "graph_edge_source")?,
            target_node_id: graph_text(target_node_id, "graph_edge_target")?,
            relation: graph_text(relation, "graph_edge_relation")?,
            provenance,
            confidence,
        })
    }

    #[must_use]
    pub fn upstream_id(&self) -> &str {
        &self.upstream_id
    }

    #[must_use]
    pub fn source_node_id(&self) -> &str {
        &self.source_node_id
    }

    #[must_use]
    pub fn target_node_id(&self) -> &str {
        &self.target_node_id
    }

    #[must_use]
    pub fn relation(&self) -> &str {
        &self.relation
    }

    #[must_use]
    pub const fn provenance(&self) -> &GraphSourceProvenance {
        &self.provenance
    }

    #[must_use]
    pub const fn confidence(&self) -> GraphConfidence {
        self.confidence
    }
}

/// Complete strictly parsed Graphify output bound to one exact snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphifyRawEvidence {
    request: GraphMemoryRunRequest,
    tree_id: GitObjectId,
    manifest_digest: ContentDigest,
    exclusion_digest: ContentDigest,
    identity: GraphifyIdentity,
    nodes: Vec<GraphifyRawNode>,
    edges: Vec<GraphifyRawEdge>,
    graph_artifact_digest: ContentDigest,
    raw_output_digest: ContentDigest,
    evidence_digest: ContentDigest,
}

/// Closed structural record shape.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum GraphRecordKind {
    /// A code symbol or structural entity.
    Node,
    /// A relationship between two code entities.
    Edge,
}

impl GraphRecordKind {
    /// Returns the stable persistence-facing value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Node => "NODE",
            Self::Edge => "EDGE",
        }
    }
}

/// Semantic memory kind fixed for TASK-033 structural evidence.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum MemoryRecordKind {
    /// Derived structural observation, never an authority fact.
    Observation,
}

impl MemoryRecordKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        "OBSERVATION"
    }
}

/// Review state fixed for TASK-033 structural evidence.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum MemoryReviewState {
    /// Untrusted candidate pending any separately authorized promotion lane.
    Candidate,
}

impl MemoryReviewState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        "CANDIDATE"
    }
}

/// One normalized, provenance-bound structural memory candidate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphMemoryRecord {
    ordinal: u32,
    graph_kind: GraphRecordKind,
    subject: String,
    category: String,
    relation: Option<String>,
    object: Option<String>,
    provenance: GraphSourceProvenance,
    confidence: GraphConfidence,
    content_digest: ContentDigest,
    record_id: ContentDigest,
}

impl GraphMemoryRecord {
    /// Constructs the only structural record class admitted by TASK-033.
    ///
    /// Nodes carry no relation/object. Edges carry both. Record kind, review
    /// state, and trust are compile-time fixed to observation/candidate/false.
    ///
    /// # Errors
    ///
    /// Rejects ordinal zero, malformed text/shape, or zero commitments.
    #[allow(clippy::too_many_arguments)]
    pub fn candidate(
        ordinal: u32,
        graph_kind: GraphRecordKind,
        subject: impl Into<String>,
        category: impl Into<String>,
        relation: Option<String>,
        object: Option<String>,
        provenance: GraphSourceProvenance,
        confidence: GraphConfidence,
        content_digest: ContentDigest,
        record_id: ContentDigest,
    ) -> Result<Self, GraphMemoryContractError> {
        if ordinal == 0 {
            return Err(GraphMemoryContractError::InvalidValue {
                field: "memory_record_ordinal",
            });
        }
        let subject = graph_text(subject, "memory_record_subject")?;
        let category = graph_text(category, "memory_record_category")?;
        let (relation, object) = match (graph_kind, relation, object) {
            (GraphRecordKind::Node, None, None) => (None, None),
            (GraphRecordKind::Edge, Some(relation), Some(object)) => (
                Some(graph_text(relation, "memory_record_relation")?),
                Some(graph_text(object, "memory_record_object")?),
            ),
            _ => {
                return Err(GraphMemoryContractError::InvalidValue {
                    field: "memory_record_shape",
                });
            }
        };
        require_digest(&content_digest, "memory_record_content_digest")?;
        require_digest(&record_id, "memory_record_id")?;
        Ok(Self {
            ordinal,
            graph_kind,
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

    #[must_use]
    pub const fn ordinal(&self) -> u32 {
        self.ordinal
    }

    #[must_use]
    pub const fn graph_kind(&self) -> GraphRecordKind {
        self.graph_kind
    }

    #[must_use]
    pub const fn record_kind(&self) -> MemoryRecordKind {
        MemoryRecordKind::Observation
    }

    #[must_use]
    pub const fn review_state(&self) -> MemoryReviewState {
        MemoryReviewState::Candidate
    }

    #[must_use]
    pub const fn trusted_context(&self) -> bool {
        false
    }

    #[must_use]
    pub fn subject(&self) -> &str {
        &self.subject
    }

    #[must_use]
    pub fn category(&self) -> &str {
        &self.category
    }

    #[must_use]
    pub fn relation(&self) -> Option<&str> {
        self.relation.as_deref()
    }

    #[must_use]
    pub fn object(&self) -> Option<&str> {
        self.object.as_deref()
    }

    #[must_use]
    pub const fn provenance(&self) -> &GraphSourceProvenance {
        &self.provenance
    }

    #[must_use]
    pub const fn confidence(&self) -> GraphConfidence {
        self.confidence
    }

    #[must_use]
    pub const fn content_digest(&self) -> &ContentDigest {
        &self.content_digest
    }

    #[must_use]
    pub const fn record_id(&self) -> &ContentDigest {
        &self.record_id
    }
}

/// Complete normalized graph analysis, ready for one atomic persistence call.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NormalizedGraphAnalysis {
    request: GraphMemoryRunRequest,
    tree_id: GitObjectId,
    manifest_digest: ContentDigest,
    exclusion_digest: ContentDigest,
    identity: GraphifyIdentity,
    identity_digest: ContentDigest,
    graph_artifact_digest: ContentDigest,
    raw_output_digest: ContentDigest,
    raw_evidence_digest: ContentDigest,
    records: Vec<GraphMemoryRecord>,
    record_set_digest: ContentDigest,
    analysis_digest: ContentDigest,
}

impl NormalizedGraphAnalysis {
    /// Constructs a fully cross-bound normalized graph analysis.
    ///
    /// # Errors
    ///
    /// Rejects substituted input, empty/overflowed or non-consecutive records,
    /// duplicate record identities, unknown provenance, or zero commitments.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        request: &GraphMemoryRunRequest,
        snapshot: &CodeSnapshotEvidence,
        raw: &GraphifyRawEvidence,
        records: Vec<GraphMemoryRecord>,
        identity_digest: ContentDigest,
        record_set_digest: ContentDigest,
        analysis_digest: ContentDigest,
    ) -> Result<Self, GraphMemoryContractError> {
        if snapshot.request() != request || raw.request() != request {
            return Err(GraphMemoryContractError::CrossBinding {
                field: "normalized_analysis_request",
            });
        }
        if raw.tree_id() != snapshot.tree_id()
            || raw.manifest_digest() != snapshot.manifest_digest()
            || raw.exclusion_digest() != snapshot.exclusion_digest()
        {
            return Err(GraphMemoryContractError::CrossBinding {
                field: "normalized_analysis_snapshot",
            });
        }
        if records.is_empty()
            || records.len() > GRAPH_MEMORY_MAX_RECORDS
            || records.len() != raw.nodes().len() + raw.edges().len()
        {
            return Err(GraphMemoryContractError::InvalidValue {
                field: "normalized_analysis_records",
            });
        }
        let mut ids = BTreeSet::new();
        for (index, record) in records.iter().enumerate() {
            let expected =
                u32::try_from(index + 1).map_err(|_| GraphMemoryContractError::InvalidValue {
                    field: "memory_record_ordinal",
                })?;
            if record.ordinal() != expected || !ids.insert(record.record_id().as_str()) {
                return Err(GraphMemoryContractError::InvalidValue {
                    field: "normalized_record_order",
                });
            }
            require_known_source(snapshot, record.provenance())?;
        }
        if records
            .windows(2)
            .any(|pair| pair[0].record_id().as_str() >= pair[1].record_id().as_str())
        {
            return Err(GraphMemoryContractError::InvalidValue {
                field: "normalized_record_order",
            });
        }
        require_digest(&identity_digest, "graphify_identity_digest")?;
        require_digest(&record_set_digest, "memory_record_set_digest")?;
        require_digest(&analysis_digest, "graph_analysis_digest")?;
        Ok(Self {
            request: request.clone(),
            tree_id: snapshot.tree_id.clone(),
            manifest_digest: snapshot.manifest_digest.clone(),
            exclusion_digest: snapshot.exclusion_digest.clone(),
            identity: raw.identity.clone(),
            identity_digest,
            graph_artifact_digest: raw.graph_artifact_digest.clone(),
            raw_output_digest: raw.raw_output_digest.clone(),
            raw_evidence_digest: raw.evidence_digest.clone(),
            records,
            record_set_digest,
            analysis_digest,
        })
    }

    #[must_use]
    pub const fn request(&self) -> &GraphMemoryRunRequest {
        &self.request
    }

    #[must_use]
    pub const fn project_id(&self) -> &ProjectId {
        self.request.project_id()
    }

    #[must_use]
    pub const fn commit_id(&self) -> &GitObjectId {
        self.request.commit_id()
    }

    #[must_use]
    pub const fn tree_id(&self) -> &GitObjectId {
        &self.tree_id
    }

    #[must_use]
    pub const fn manifest_digest(&self) -> &ContentDigest {
        &self.manifest_digest
    }

    #[must_use]
    pub const fn exclusion_digest(&self) -> &ContentDigest {
        &self.exclusion_digest
    }

    #[must_use]
    pub const fn identity(&self) -> &GraphifyIdentity {
        &self.identity
    }

    #[must_use]
    pub const fn identity_digest(&self) -> &ContentDigest {
        &self.identity_digest
    }

    #[must_use]
    pub const fn graph_artifact_digest(&self) -> &ContentDigest {
        &self.graph_artifact_digest
    }

    #[must_use]
    pub const fn raw_output_digest(&self) -> &ContentDigest {
        &self.raw_output_digest
    }

    #[must_use]
    pub const fn raw_evidence_digest(&self) -> &ContentDigest {
        &self.raw_evidence_digest
    }

    #[must_use]
    pub fn records(&self) -> &[GraphMemoryRecord] {
        &self.records
    }

    #[must_use]
    pub const fn record_set_digest(&self) -> &ContentDigest {
        &self.record_set_digest
    }

    #[must_use]
    pub const fn analysis_digest(&self) -> &ContentDigest {
        &self.analysis_digest
    }
}

/// Ephemeral process-owned query; raw text is never copied into durable plans.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryQuery {
    request: GraphMemoryRunRequest,
    text: String,
    limit: u16,
}

impl MemoryQuery {
    /// Constructs one bounded ephemeral query for the exact run request.
    ///
    /// # Errors
    ///
    /// Rejects empty/control-bearing/oversized text or a limit that differs
    /// from the exact run request binding.
    pub fn new(
        request: &GraphMemoryRunRequest,
        text: impl Into<String>,
        limit: u16,
    ) -> Result<Self, GraphMemoryContractError> {
        let text = graph_text(text, "memory_query")?;
        if limit != request.retrieval_limit() {
            return Err(GraphMemoryContractError::InvalidValue {
                field: "memory_query_limit",
            });
        }
        Ok(Self {
            request: request.clone(),
            text,
            limit,
        })
    }

    #[must_use]
    pub const fn request(&self) -> &GraphMemoryRunRequest {
        &self.request
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub const fn query_digest(&self) -> &ContentDigest {
        self.request.query_digest()
    }

    #[must_use]
    pub const fn limit(&self) -> u16 {
        self.limit
    }
}

/// One deterministic ranked result containing no raw source/query text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RankedMemoryRecord {
    record_id: ContentDigest,
    record_digest: ContentDigest,
    rank: u16,
    score: u32,
}

impl RankedMemoryRecord {
    /// Binds a positive rank and relevance score to one exact record.
    ///
    /// # Errors
    ///
    /// Rejects rank/score zero.
    pub fn new(
        record: &GraphMemoryRecord,
        rank: u16,
        score: u32,
    ) -> Result<Self, GraphMemoryContractError> {
        if rank == 0 || score == 0 {
            return Err(GraphMemoryContractError::InvalidValue {
                field: "memory_result_rank_score",
            });
        }
        Ok(Self {
            record_id: record.record_id.clone(),
            record_digest: record.content_digest.clone(),
            rank,
            score,
        })
    }

    /// Reconstructs one ranked result after a fixed repository function has
    /// proved its binding to the persisted analysis.
    ///
    /// # Errors
    ///
    /// Rejects zero digests, rank, or score.
    pub fn replay(
        record_id: ContentDigest,
        record_digest: ContentDigest,
        rank: u16,
        score: u32,
    ) -> Result<Self, GraphMemoryContractError> {
        require_digest(&record_id, "memory_result_record_id")?;
        require_digest(&record_digest, "memory_result_record_digest")?;
        if rank == 0 || score == 0 {
            return Err(GraphMemoryContractError::InvalidValue {
                field: "memory_result_rank_score",
            });
        }
        Ok(Self {
            record_id,
            record_digest,
            rank,
            score,
        })
    }

    #[must_use]
    pub const fn record_id(&self) -> &ContentDigest {
        &self.record_id
    }

    #[must_use]
    pub const fn record_digest(&self) -> &ContentDigest {
        &self.record_digest
    }

    #[must_use]
    pub const fn rank(&self) -> u16 {
        self.rank
    }

    #[must_use]
    pub const fn score(&self) -> u32 {
        self.score
    }
}

/// Closed retrieval terminal disposition.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum MemoryRetrievalDisposition {
    /// At least one relevant, exact-bound record was returned.
    Results,
    /// No relevant exact-bound record passed the deterministic scorer.
    NoAnswer,
}

impl MemoryRetrievalDisposition {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Results => "RESULTS",
            Self::NoAnswer => "NO_ANSWER",
        }
    }
}

/// Pure deterministic retrieval/audit plan; contains no raw query text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryRetrievalPlan {
    request: GraphMemoryRunRequest,
    analysis_digest: ContentDigest,
    limit: u16,
    disposition: MemoryRetrievalDisposition,
    results: Vec<RankedMemoryRecord>,
    result_set_digest: ContentDigest,
}

/// Typed database and extension identity required by durable Memory evidence.
///
/// This value is representation only. It performs no I/O and grants no
/// migration, database, memory, policy, or release authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodebaseMemoryPersistenceIdentity {
    database_identity_digest: ContentDigest,
    global_schema_version: u16,
    global_manifest_digest: ContentDigest,
    extension_id: &'static str,
    extension_schema_version: u16,
    extension_sql_digest: ContentDigest,
    extension_manifest_digest: ContentDigest,
}

impl CodebaseMemoryPersistenceIdentity {
    /// Constructs the exact v1 same-database extension identity.
    ///
    /// # Errors
    ///
    /// Rejects any zero database, global-manifest, SQL, or extension-manifest
    /// commitment.
    pub fn v1(
        database_identity_digest: ContentDigest,
        global_manifest_digest: ContentDigest,
        extension_sql_digest: ContentDigest,
        extension_manifest_digest: ContentDigest,
    ) -> Result<Self, GraphMemoryContractError> {
        require_digest(&database_identity_digest, "memory_database_identity_digest")?;
        require_digest(&global_manifest_digest, "memory_global_manifest_digest")?;
        require_digest(&extension_sql_digest, "memory_extension_sql_digest")?;
        require_digest(
            &extension_manifest_digest,
            "memory_extension_manifest_digest",
        )?;
        Ok(Self {
            database_identity_digest,
            global_schema_version: CODEBASE_MEMORY_REQUIRED_GLOBAL_SCHEMA_VERSION,
            global_manifest_digest,
            extension_id: CODEBASE_MEMORY_EXTENSION_ID,
            extension_schema_version: CODEBASE_MEMORY_EXTENSION_SCHEMA_VERSION,
            extension_sql_digest,
            extension_manifest_digest,
        })
    }

    #[must_use]
    pub const fn database_identity_digest(&self) -> &ContentDigest {
        &self.database_identity_digest
    }

    #[must_use]
    pub const fn global_schema_version(&self) -> u16 {
        self.global_schema_version
    }

    #[must_use]
    pub const fn global_manifest_digest(&self) -> &ContentDigest {
        &self.global_manifest_digest
    }

    #[must_use]
    pub const fn extension_id(&self) -> &'static str {
        self.extension_id
    }

    #[must_use]
    pub const fn extension_schema_version(&self) -> u16 {
        self.extension_schema_version
    }

    #[must_use]
    pub const fn extension_sql_digest(&self) -> &ContentDigest {
        &self.extension_sql_digest
    }

    #[must_use]
    pub const fn extension_manifest_digest(&self) -> &ContentDigest {
        &self.extension_manifest_digest
    }
}

impl MemoryRetrievalPlan {
    /// Constructs a deterministic exact-analysis retrieval plan.
    ///
    /// # Errors
    ///
    /// Rejects cross-binding, unknown/duplicate records, non-consecutive rank,
    /// non-descending score/tie order, result overflow, or zero digest.
    pub fn new(
        analysis: &NormalizedGraphAnalysis,
        query: &MemoryQuery,
        results: Vec<RankedMemoryRecord>,
        result_set_digest: ContentDigest,
    ) -> Result<Self, GraphMemoryContractError> {
        if query.request() != analysis.request() {
            return Err(GraphMemoryContractError::CrossBinding {
                field: "memory_query_analysis",
            });
        }
        if results.len() > usize::from(query.limit()) {
            return Err(GraphMemoryContractError::InvalidValue {
                field: "memory_result_limit",
            });
        }
        require_digest(&result_set_digest, "memory_result_set_digest")?;
        let mut seen = BTreeSet::new();
        for (index, result) in results.iter().enumerate() {
            let expected =
                u16::try_from(index + 1).map_err(|_| GraphMemoryContractError::InvalidValue {
                    field: "memory_result_rank",
                })?;
            let record = analysis
                .records()
                .iter()
                .find(|record| record.record_id() == result.record_id())
                .ok_or(GraphMemoryContractError::CrossBinding {
                    field: "memory_result_record",
                })?;
            if result.rank() != expected
                || record.content_digest() != result.record_digest()
                || !seen.insert(result.record_id().as_str())
            {
                return Err(GraphMemoryContractError::InvalidValue {
                    field: "memory_result_order",
                });
            }
        }
        if results.windows(2).any(|pair| {
            pair[0].score() < pair[1].score()
                || (pair[0].score() == pair[1].score()
                    && pair[0].record_id().as_str() > pair[1].record_id().as_str())
        }) {
            return Err(GraphMemoryContractError::InvalidValue {
                field: "memory_result_score_order",
            });
        }
        let disposition = if results.is_empty() {
            MemoryRetrievalDisposition::NoAnswer
        } else {
            MemoryRetrievalDisposition::Results
        };
        Ok(Self {
            request: analysis.request.clone(),
            analysis_digest: analysis.analysis_digest.clone(),
            limit: query.limit,
            disposition,
            results,
            result_set_digest,
        })
    }

    #[must_use]
    pub const fn request(&self) -> &GraphMemoryRunRequest {
        &self.request
    }

    #[must_use]
    pub const fn analysis_digest(&self) -> &ContentDigest {
        &self.analysis_digest
    }

    #[must_use]
    pub const fn query_digest(&self) -> &ContentDigest {
        self.request.query_digest()
    }

    #[must_use]
    pub const fn algorithm(&self) -> &'static str {
        GRAPH_MEMORY_RETRIEVAL_ALGORITHM
    }

    #[must_use]
    pub const fn limit(&self) -> u16 {
        self.limit
    }

    #[must_use]
    pub const fn disposition(&self) -> MemoryRetrievalDisposition {
        self.disposition
    }

    #[must_use]
    pub fn results(&self) -> &[RankedMemoryRecord] {
        &self.results
    }

    #[must_use]
    pub const fn result_set_digest(&self) -> &ContentDigest {
        &self.result_set_digest
    }
}

/// Durable repository evidence for one exact analysis and record set.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphMemoryPersistenceEvidence {
    request: GraphMemoryRunRequest,
    identity: CodebaseMemoryPersistenceIdentity,
    analysis_digest: ContentDigest,
    record_set_digest: ContentDigest,
    record_count: u32,
    persistence_digest: ContentDigest,
}

impl GraphMemoryPersistenceEvidence {
    /// Constructs exact-analysis persistence evidence.
    ///
    /// # Errors
    ///
    /// Rejects count overflow or a zero persistence commitment.
    pub fn new(
        analysis: &NormalizedGraphAnalysis,
        identity: CodebaseMemoryPersistenceIdentity,
        persistence_digest: ContentDigest,
    ) -> Result<Self, GraphMemoryContractError> {
        require_digest(&persistence_digest, "memory_persistence_digest")?;
        let record_count = u32::try_from(analysis.records.len()).map_err(|_| {
            GraphMemoryContractError::InvalidValue {
                field: "memory_record_count",
            }
        })?;
        Ok(Self {
            request: analysis.request.clone(),
            identity,
            analysis_digest: analysis.analysis_digest.clone(),
            record_set_digest: analysis.record_set_digest.clone(),
            record_count,
            persistence_digest,
        })
    }

    /// Reconstructs exact persistence evidence returned by the fixed
    /// same-database repository profile.
    ///
    /// # Errors
    ///
    /// Rejects an empty/overflowed record set or any zero commitment.
    #[allow(clippy::too_many_arguments)]
    pub fn replay(
        request: GraphMemoryRunRequest,
        identity: CodebaseMemoryPersistenceIdentity,
        analysis_digest: ContentDigest,
        record_set_digest: ContentDigest,
        record_count: u32,
        persistence_digest: ContentDigest,
    ) -> Result<Self, GraphMemoryContractError> {
        require_digest(&analysis_digest, "graph_analysis_digest")?;
        require_digest(&record_set_digest, "memory_record_set_digest")?;
        require_digest(&persistence_digest, "memory_persistence_digest")?;
        if record_count == 0
            || usize::try_from(record_count).map_or(true, |count| count > GRAPH_MEMORY_MAX_RECORDS)
        {
            return Err(GraphMemoryContractError::InvalidValue {
                field: "memory_record_count",
            });
        }
        Ok(Self {
            request,
            identity,
            analysis_digest,
            record_set_digest,
            record_count,
            persistence_digest,
        })
    }

    #[must_use]
    pub const fn request(&self) -> &GraphMemoryRunRequest {
        &self.request
    }

    #[must_use]
    pub const fn identity(&self) -> &CodebaseMemoryPersistenceIdentity {
        &self.identity
    }

    #[must_use]
    pub const fn analysis_digest(&self) -> &ContentDigest {
        &self.analysis_digest
    }

    #[must_use]
    pub const fn record_set_digest(&self) -> &ContentDigest {
        &self.record_set_digest
    }

    #[must_use]
    pub const fn record_count(&self) -> u32 {
        self.record_count
    }

    #[must_use]
    pub const fn persistence_digest(&self) -> &ContentDigest {
        &self.persistence_digest
    }
}

/// Durable repository evidence for one exact-query retrieval/audit operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryRetrievalEvidence {
    request: GraphMemoryRunRequest,
    identity: CodebaseMemoryPersistenceIdentity,
    analysis_digest: ContentDigest,
    persistence_digest: ContentDigest,
    limit: u16,
    disposition: MemoryRetrievalDisposition,
    results: Vec<RankedMemoryRecord>,
    result_set_digest: ContentDigest,
    retrieval_digest: ContentDigest,
}

impl MemoryRetrievalEvidence {
    /// Constructs repository retrieval evidence from the exact persistence/plan.
    ///
    /// # Errors
    ///
    /// Rejects substituted analysis/request or zero retrieval commitment.
    pub fn new(
        persisted: &GraphMemoryPersistenceEvidence,
        plan: MemoryRetrievalPlan,
        retrieval_digest: ContentDigest,
    ) -> Result<Self, GraphMemoryContractError> {
        if persisted.request() != plan.request()
            || persisted.analysis_digest() != plan.analysis_digest()
        {
            return Err(GraphMemoryContractError::CrossBinding {
                field: "memory_retrieval_persistence",
            });
        }
        require_digest(&retrieval_digest, "memory_retrieval_digest")?;
        Ok(Self {
            request: plan.request,
            identity: persisted.identity.clone(),
            analysis_digest: plan.analysis_digest,
            persistence_digest: persisted.persistence_digest.clone(),
            limit: plan.limit,
            disposition: plan.disposition,
            results: plan.results,
            result_set_digest: plan.result_set_digest,
            retrieval_digest,
        })
    }

    /// Reconstructs exact retrieval evidence returned by the fixed repository
    /// receipt loader.
    ///
    /// # Errors
    ///
    /// Rejects a changed request limit, inconsistent disposition, duplicate or
    /// non-canonical ranked results, overflow, or zero commitments.
    #[allow(clippy::too_many_arguments)]
    pub fn replay(
        persisted: &GraphMemoryPersistenceEvidence,
        limit: u16,
        disposition: MemoryRetrievalDisposition,
        results: Vec<RankedMemoryRecord>,
        result_set_digest: ContentDigest,
        retrieval_digest: ContentDigest,
    ) -> Result<Self, GraphMemoryContractError> {
        if limit != persisted.request().retrieval_limit()
            || results.len() > usize::from(limit)
            || results.len() > usize::try_from(persisted.record_count()).unwrap_or(usize::MAX)
            || matches!(disposition, MemoryRetrievalDisposition::NoAnswer) != results.is_empty()
        {
            return Err(GraphMemoryContractError::InvalidValue {
                field: "memory_retrieval_replay",
            });
        }
        require_digest(&result_set_digest, "memory_result_set_digest")?;
        require_digest(&retrieval_digest, "memory_retrieval_digest")?;
        let mut seen = BTreeSet::new();
        for (index, result) in results.iter().enumerate() {
            let expected =
                u16::try_from(index + 1).map_err(|_| GraphMemoryContractError::InvalidValue {
                    field: "memory_result_rank",
                })?;
            if result.rank() != expected || !seen.insert(result.record_id().as_str()) {
                return Err(GraphMemoryContractError::InvalidValue {
                    field: "memory_result_order",
                });
            }
        }
        if results.windows(2).any(|pair| {
            pair[0].score() < pair[1].score()
                || (pair[0].score() == pair[1].score()
                    && pair[0].record_id().as_str() > pair[1].record_id().as_str())
        }) {
            return Err(GraphMemoryContractError::InvalidValue {
                field: "memory_result_score_order",
            });
        }
        Ok(Self {
            request: persisted.request.clone(),
            identity: persisted.identity.clone(),
            analysis_digest: persisted.analysis_digest.clone(),
            persistence_digest: persisted.persistence_digest.clone(),
            limit,
            disposition,
            results,
            result_set_digest,
            retrieval_digest,
        })
    }

    #[must_use]
    pub const fn request(&self) -> &GraphMemoryRunRequest {
        &self.request
    }

    #[must_use]
    pub const fn identity(&self) -> &CodebaseMemoryPersistenceIdentity {
        &self.identity
    }

    #[must_use]
    pub const fn analysis_digest(&self) -> &ContentDigest {
        &self.analysis_digest
    }

    #[must_use]
    pub const fn persistence_digest(&self) -> &ContentDigest {
        &self.persistence_digest
    }

    #[must_use]
    pub const fn query_digest(&self) -> &ContentDigest {
        self.request.query_digest()
    }

    #[must_use]
    pub const fn algorithm(&self) -> &'static str {
        GRAPH_MEMORY_RETRIEVAL_ALGORITHM
    }

    #[must_use]
    pub const fn limit(&self) -> u16 {
        self.limit
    }

    #[must_use]
    pub const fn disposition(&self) -> MemoryRetrievalDisposition {
        self.disposition
    }

    #[must_use]
    pub fn results(&self) -> &[RankedMemoryRecord] {
        &self.results
    }

    #[must_use]
    pub const fn result_set_digest(&self) -> &ContentDigest {
        &self.result_set_digest
    }

    #[must_use]
    pub const fn retrieval_digest(&self) -> &ContentDigest {
        &self.retrieval_digest
    }
}

/// Restart-safe terminal graph-memory receipt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphMemoryReceipt {
    persistence: GraphMemoryPersistenceEvidence,
    retrieval: MemoryRetrievalEvidence,
    receipt_digest: ContentDigest,
}

impl GraphMemoryReceipt {
    /// Constructs a cross-bound terminal graph-memory receipt.
    ///
    /// # Errors
    ///
    /// Rejects substituted retrieval evidence or a zero receipt commitment.
    pub fn new(
        persistence: GraphMemoryPersistenceEvidence,
        retrieval: MemoryRetrievalEvidence,
        receipt_digest: ContentDigest,
    ) -> Result<Self, GraphMemoryContractError> {
        if persistence.request() != retrieval.request()
            || persistence.identity() != retrieval.identity()
            || persistence.analysis_digest() != retrieval.analysis_digest()
            || persistence.persistence_digest() != retrieval.persistence_digest()
        {
            return Err(GraphMemoryContractError::CrossBinding {
                field: "graph_memory_receipt",
            });
        }
        require_digest(&receipt_digest, "graph_memory_receipt_digest")?;
        Ok(Self {
            persistence,
            retrieval,
            receipt_digest,
        })
    }

    #[must_use]
    pub const fn persistence(&self) -> &GraphMemoryPersistenceEvidence {
        &self.persistence
    }

    #[must_use]
    pub const fn retrieval(&self) -> &MemoryRetrievalEvidence {
        &self.retrieval
    }

    #[must_use]
    pub const fn receipt_digest(&self) -> &ContentDigest {
        &self.receipt_digest
    }

    #[must_use]
    pub fn matches_request(&self, request: &GraphMemoryRunRequest) -> bool {
        self.persistence.request() == request && self.retrieval.request() == request
    }
}

impl GraphifyRawEvidence {
    /// Constructs complete graph evidence after exact source/endpoint checks.
    ///
    /// # Errors
    ///
    /// Rejects cross-binding, empty/duplicate nodes, duplicate edges, foreign
    /// sources, dangling endpoints, or zero output commitments.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        request: &GraphMemoryRunRequest,
        snapshot: &CodeSnapshotEvidence,
        identity: GraphifyIdentity,
        nodes: Vec<GraphifyRawNode>,
        edges: Vec<GraphifyRawEdge>,
        graph_artifact_digest: ContentDigest,
        raw_output_digest: ContentDigest,
        evidence_digest: ContentDigest,
    ) -> Result<Self, GraphMemoryContractError> {
        if snapshot.request() != request {
            return Err(GraphMemoryContractError::CrossBinding {
                field: "graph_snapshot_request",
            });
        }
        if nodes.is_empty() {
            return Err(GraphMemoryContractError::InvalidValue {
                field: "graph_nodes",
            });
        }
        require_digest(&graph_artifact_digest, "graph_artifact_digest")?;
        require_digest(&raw_output_digest, "graph_raw_output_digest")?;
        require_digest(&evidence_digest, "graph_evidence_digest")?;

        let mut node_ids = BTreeSet::new();
        for node in &nodes {
            if !node_ids.insert(node.upstream_id()) {
                return Err(GraphMemoryContractError::DuplicateGraphId);
            }
            require_known_source(snapshot, node.provenance())?;
        }
        let mut edge_ids = BTreeSet::new();
        for edge in &edges {
            if !edge_ids.insert(edge.upstream_id()) {
                return Err(GraphMemoryContractError::DuplicateGraphId);
            }
            if !node_ids.contains(edge.source_node_id())
                || !node_ids.contains(edge.target_node_id())
            {
                return Err(GraphMemoryContractError::DanglingGraphEdge);
            }
            require_known_source(snapshot, edge.provenance())?;
        }

        Ok(Self {
            request: request.clone(),
            tree_id: snapshot.tree_id.clone(),
            manifest_digest: snapshot.manifest_digest.clone(),
            exclusion_digest: snapshot.exclusion_digest.clone(),
            identity,
            nodes,
            edges,
            graph_artifact_digest,
            raw_output_digest,
            evidence_digest,
        })
    }

    #[must_use]
    pub const fn request(&self) -> &GraphMemoryRunRequest {
        &self.request
    }

    #[must_use]
    pub const fn commit_id(&self) -> &GitObjectId {
        self.request.commit_id()
    }

    #[must_use]
    pub const fn tree_id(&self) -> &GitObjectId {
        &self.tree_id
    }

    #[must_use]
    pub const fn manifest_digest(&self) -> &ContentDigest {
        &self.manifest_digest
    }

    #[must_use]
    pub const fn exclusion_digest(&self) -> &ContentDigest {
        &self.exclusion_digest
    }

    #[must_use]
    pub const fn identity(&self) -> &GraphifyIdentity {
        &self.identity
    }

    #[must_use]
    pub fn nodes(&self) -> &[GraphifyRawNode] {
        &self.nodes
    }

    #[must_use]
    pub fn edges(&self) -> &[GraphifyRawEdge] {
        &self.edges
    }

    #[must_use]
    pub const fn graph_artifact_digest(&self) -> &ContentDigest {
        &self.graph_artifact_digest
    }

    #[must_use]
    pub const fn raw_output_digest(&self) -> &ContentDigest {
        &self.raw_output_digest
    }

    #[must_use]
    pub const fn evidence_digest(&self) -> &ContentDigest {
        &self.evidence_digest
    }
}

fn valid_relative_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_SOURCE_PATH_BYTES
        && value.trim() == value
        && !value.starts_with('/')
        && !value.contains(['\\', ':', '\0'])
        && value
            .split('/')
            .all(|component| !component.is_empty() && !matches!(component, "." | ".."))
}

fn require_digest(
    digest: &ContentDigest,
    field: &'static str,
) -> Result<(), GraphMemoryContractError> {
    if digest.as_str().bytes().all(|byte| byte == b'0') {
        Err(GraphMemoryContractError::InvalidValue { field })
    } else {
        Ok(())
    }
}

fn graph_text(
    value: impl Into<String>,
    field: &'static str,
) -> Result<String, GraphMemoryContractError> {
    let value = value.into();
    if value.trim().is_empty()
        || value.len() > MAX_GRAPH_TEXT_BYTES
        || value.bytes().any(|byte| byte.is_ascii_control())
    {
        Err(GraphMemoryContractError::InvalidValue { field })
    } else {
        Ok(value)
    }
}

fn require_known_source(
    snapshot: &CodeSnapshotEvidence,
    provenance: &GraphSourceProvenance,
) -> Result<(), GraphMemoryContractError> {
    if snapshot.sources().iter().any(|source| {
        source.relative_path() == provenance.relative_path()
            && source.content_digest() == provenance.content_digest()
    }) {
        Ok(())
    } else {
        Err(GraphMemoryContractError::UnknownSource)
    }
}
