use std::collections::{BTreeSet, HashSet};

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::error::{GraphifyAdapterError, GraphifyAdapterErrorKind, GraphifyAdapterResult};
use crate::snapshot::{MaterializedSnapshot, sha256_bytes, validate_relative_path};

/// Upstream confidence provenance retained on every accepted relation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum GraphConfidence {
    Extracted,
    Inferred,
    Ambiguous,
}

impl GraphConfidence {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Extracted => "EXTRACTED",
            Self::Inferred => "INFERRED",
            Self::Ambiguous => "AMBIGUOUS",
        }
    }
}

/// One bounded, manifest-provenanced Graphify node.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct NormalizedGraphNode {
    id: String,
    label: String,
    source_file: String,
    source_location: String,
    kind: String,
}

impl NormalizedGraphNode {
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    #[must_use]
    pub fn source_file(&self) -> &str {
        &self.source_file
    }

    #[must_use]
    pub fn source_location(&self) -> &str {
        &self.source_location
    }

    #[must_use]
    pub fn kind(&self) -> &str {
        &self.kind
    }
}

/// One bounded relation whose endpoints both survived provenance validation.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct NormalizedGraphEdge {
    source: String,
    target: String,
    relation: String,
    confidence: GraphConfidence,
    confidence_score: Option<String>,
    context: Option<String>,
    source_file: String,
    source_location: String,
}

impl NormalizedGraphEdge {
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    #[must_use]
    pub fn target(&self) -> &str {
        &self.target
    }

    #[must_use]
    pub fn relation(&self) -> &str {
        &self.relation
    }

    #[must_use]
    pub const fn confidence(&self) -> GraphConfidence {
        self.confidence
    }

    #[must_use]
    pub fn confidence_score(&self) -> Option<&str> {
        self.confidence_score.as_deref()
    }

    #[must_use]
    pub fn context(&self) -> Option<&str> {
        self.context.as_deref()
    }

    #[must_use]
    pub fn source_file(&self) -> &str {
        &self.source_file
    }

    #[must_use]
    pub fn source_location(&self) -> &str {
        &self.source_location
    }
}

/// Strictly parsed and canonically sorted code graph.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NormalizedGraph {
    nodes: Vec<NormalizedGraphNode>,
    edges: Vec<NormalizedGraphEdge>,
    raw_graph_sha256: String,
    record_set_sha256: String,
    raw_node_count: usize,
    raw_edge_count: usize,
    dropped_non_code_nodes: usize,
    dropped_source_less_nodes: usize,
    dropped_unbound_edges: usize,
}

impl NormalizedGraph {
    #[must_use]
    pub fn nodes(&self) -> &[NormalizedGraphNode] {
        &self.nodes
    }

    #[must_use]
    pub fn edges(&self) -> &[NormalizedGraphEdge] {
        &self.edges
    }

    #[must_use]
    pub fn raw_graph_sha256(&self) -> &str {
        &self.raw_graph_sha256
    }

    #[must_use]
    pub fn record_set_sha256(&self) -> &str {
        &self.record_set_sha256
    }

    #[must_use]
    pub const fn raw_node_count(&self) -> usize {
        self.raw_node_count
    }

    #[must_use]
    pub const fn raw_edge_count(&self) -> usize {
        self.raw_edge_count
    }

    #[must_use]
    pub const fn dropped_source_less_nodes(&self) -> usize {
        self.dropped_source_less_nodes
    }

    /// Number of explicit non-code nodes excluded from a code-only snapshot.
    #[must_use]
    pub const fn dropped_non_code_nodes(&self) -> usize {
        self.dropped_non_code_nodes
    }

    #[must_use]
    pub const fn dropped_unbound_edges(&self) -> usize {
        self.dropped_unbound_edges
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
// The repeated prefix makes each security bound unambiguous at call sites.
#[allow(clippy::struct_field_names)]
pub(crate) struct GraphParseLimits {
    pub max_nodes: usize,
    pub max_edges: usize,
    pub max_text_bytes: usize,
}

// Keeping the schema checks in one linear fail-closed parser makes it possible
// to audit that no partially validated Graphify fact can escape.
#[allow(clippy::too_many_lines)]
pub(crate) fn parse_graph(
    bytes: &[u8],
    snapshot: &MaterializedSnapshot,
    limits: GraphParseLimits,
) -> GraphifyAdapterResult<NormalizedGraph> {
    let root: Value = serde_json::from_slice(bytes).map_err(|_| {
        error(
            GraphifyAdapterErrorKind::MalformedOutput,
            "GRAPHIFY_GRAPH_JSON_MALFORMED",
        )
    })?;
    let object = root.as_object().ok_or_else(|| {
        error(
            GraphifyAdapterErrorKind::MalformedOutput,
            "GRAPHIFY_GRAPH_ROOT_REJECTED",
        )
    })?;
    require_exact_keys(
        object,
        &[
            "nodes",
            "edges",
            "hyperedges",
            "input_tokens",
            "output_tokens",
        ],
        "GRAPHIFY_GRAPH_TOP_LEVEL_SCHEMA_REJECTED",
    )?;
    require_zero_u64(object, "input_tokens")?;
    require_zero_u64(object, "output_tokens")?;
    let hyperedges = object["hyperedges"].as_array().ok_or_else(|| {
        error(
            GraphifyAdapterErrorKind::MalformedOutput,
            "GRAPHIFY_GRAPH_HYPEREDGES_MALFORMED",
        )
    })?;
    if !hyperedges.is_empty() {
        return Err(error(
            GraphifyAdapterErrorKind::PartialOutput,
            "GRAPHIFY_GRAPH_HYPEREDGES_UNSUPPORTED",
        ));
    }
    let raw_nodes = object["nodes"].as_array().ok_or_else(|| {
        error(
            GraphifyAdapterErrorKind::MalformedOutput,
            "GRAPHIFY_GRAPH_NODES_MALFORMED",
        )
    })?;
    let raw_edges = object["edges"].as_array().ok_or_else(|| {
        error(
            GraphifyAdapterErrorKind::MalformedOutput,
            "GRAPHIFY_GRAPH_EDGES_MALFORMED",
        )
    })?;
    if raw_nodes.is_empty() {
        return Err(error(
            GraphifyAdapterErrorKind::EmptyAnalysis,
            "GRAPHIFY_GRAPH_ZERO_NODES_REJECTED",
        ));
    }
    if raw_nodes.len() > limits.max_nodes || raw_edges.len() > limits.max_edges {
        return Err(error(
            GraphifyAdapterErrorKind::OutputLimit,
            "GRAPHIFY_GRAPH_RECORD_LIMIT",
        ));
    }
    let manifest = snapshot.source_digests();
    let manifest_paths: BTreeSet<&str> = manifest.keys().copied().collect();
    let mut all_ids = HashSet::with_capacity(raw_nodes.len());
    let mut kept_ids = HashSet::with_capacity(raw_nodes.len());
    let mut nodes = Vec::with_capacity(raw_nodes.len());
    let mut dropped_non_code_nodes = 0_usize;
    let mut dropped_source_less_nodes = 0_usize;
    for raw_node in raw_nodes {
        let node = raw_node.as_object().ok_or_else(|| {
            error(
                GraphifyAdapterErrorKind::MalformedOutput,
                "GRAPHIFY_GRAPH_NODE_NOT_OBJECT",
            )
        })?;
        require_allowed_keys(
            node,
            &[
                "id",
                "label",
                "file_type",
                "source_file",
                "source_location",
                "_origin",
                "_callable",
                "_callable_class",
                "type",
                "kind",
                "language",
                "namespace",
                "visibility",
                "signature",
                "category",
                "subtype",
            ],
            "GRAPHIFY_GRAPH_NODE_SCHEMA_REJECTED",
        )?;
        let id = required_text(node, "id", limits.max_text_bytes)?;
        if !all_ids.insert(id.to_owned()) {
            return Err(error(
                GraphifyAdapterErrorKind::MalformedOutput,
                "GRAPHIFY_GRAPH_DUPLICATE_NODE_ID",
            ));
        }
        if required_text(node, "file_type", 32)? != "code" {
            dropped_non_code_nodes = dropped_non_code_nodes.checked_add(1).ok_or_else(|| {
                error(
                    GraphifyAdapterErrorKind::OutputLimit,
                    "GRAPHIFY_GRAPH_DROP_COUNT_OVERFLOW",
                )
            })?;
            continue;
        }
        let label = required_text(node, "label", limits.max_text_bytes)?;
        if required_text(node, "_origin", 32)? != "ast" {
            return Err(error(
                GraphifyAdapterErrorKind::PartialOutput,
                "GRAPHIFY_GRAPH_NODE_PROVENANCE_REJECTED",
            ));
        }
        validate_optional_scalar_fields(node, limits.max_text_bytes)?;
        let source_file = required_text_allow_empty(node, "source_file", limits.max_text_bytes)?;
        let source_location =
            required_text_allow_empty(node, "source_location", limits.max_text_bytes)?;
        if source_file.is_empty() {
            dropped_source_less_nodes =
                dropped_source_less_nodes.checked_add(1).ok_or_else(|| {
                    error(
                        GraphifyAdapterErrorKind::OutputLimit,
                        "GRAPHIFY_GRAPH_DROP_COUNT_OVERFLOW",
                    )
                })?;
            continue;
        }
        validate_manifest_path(source_file, &manifest_paths)?;
        if source_location.is_empty() {
            return Err(error(
                GraphifyAdapterErrorKind::MalformedOutput,
                "GRAPHIFY_GRAPH_NODE_LOCATION_MISSING",
            ));
        }
        let kind = node
            .get("type")
            .and_then(Value::as_str)
            .or_else(|| node.get("kind").and_then(Value::as_str))
            .unwrap_or("code");
        validate_text(kind, limits.max_text_bytes)?;
        kept_ids.insert(id.to_owned());
        nodes.push(NormalizedGraphNode {
            id: id.to_owned(),
            label: label.to_owned(),
            source_file: source_file.to_owned(),
            source_location: source_location.to_owned(),
            kind: kind.to_owned(),
        });
    }
    if nodes.is_empty() {
        return Err(error(
            GraphifyAdapterErrorKind::EmptyAnalysis,
            "GRAPHIFY_GRAPH_NO_PROVENANCED_NODES",
        ));
    }
    nodes.sort();

    let mut edges = Vec::with_capacity(raw_edges.len());
    let mut dropped_unbound_edges = 0_usize;
    let mut edge_keys = BTreeSet::new();
    for raw_edge in raw_edges {
        let edge = raw_edge.as_object().ok_or_else(|| {
            error(
                GraphifyAdapterErrorKind::MalformedOutput,
                "GRAPHIFY_GRAPH_EDGE_NOT_OBJECT",
            )
        })?;
        require_allowed_keys(
            edge,
            &[
                "source",
                "target",
                "relation",
                "confidence",
                "confidence_score",
                "context",
                "source_file",
                "source_location",
                "weight",
                "_origin",
            ],
            "GRAPHIFY_GRAPH_EDGE_SCHEMA_REJECTED",
        )?;
        let source = required_text(edge, "source", limits.max_text_bytes)?;
        let target = required_text(edge, "target", limits.max_text_bytes)?;
        let relation = required_text(edge, "relation", 128)?;
        if !is_known_relation(relation) {
            return Err(error(
                GraphifyAdapterErrorKind::MalformedOutput,
                "GRAPHIFY_GRAPH_EDGE_RELATION_UNKNOWN",
            ));
        }
        let confidence = match required_text(edge, "confidence", 32)? {
            "EXTRACTED" => GraphConfidence::Extracted,
            "INFERRED" => GraphConfidence::Inferred,
            "AMBIGUOUS" => GraphConfidence::Ambiguous,
            _ => {
                return Err(error(
                    GraphifyAdapterErrorKind::MalformedOutput,
                    "GRAPHIFY_GRAPH_EDGE_CONFIDENCE_UNKNOWN",
                ));
            }
        };
        if required_text(edge, "_origin", 32)? != "ast" {
            return Err(error(
                GraphifyAdapterErrorKind::PartialOutput,
                "GRAPHIFY_GRAPH_EDGE_PROVENANCE_REJECTED",
            ));
        }
        let source_file = required_text_allow_empty(edge, "source_file", limits.max_text_bytes)?;
        let source_location =
            required_text_allow_empty(edge, "source_location", limits.max_text_bytes)?;
        if !source_file.is_empty() {
            validate_manifest_path(source_file, &manifest_paths)?;
        }
        let confidence_score = optional_number(edge, "confidence_score", 0.0, 1.0)?;
        let weight = edge.get("weight").and_then(Value::as_f64).ok_or_else(|| {
            error(
                GraphifyAdapterErrorKind::MalformedOutput,
                "GRAPHIFY_GRAPH_EDGE_WEIGHT_MALFORMED",
            )
        })?;
        if !weight.is_finite() || !(0.0..=1_000.0).contains(&weight) {
            return Err(error(
                GraphifyAdapterErrorKind::MalformedOutput,
                "GRAPHIFY_GRAPH_EDGE_WEIGHT_REJECTED",
            ));
        }
        let context = optional_text(edge, "context", limits.max_text_bytes)?;
        if source_file.is_empty()
            || source_location.is_empty()
            || !kept_ids.contains(source)
            || !kept_ids.contains(target)
        {
            dropped_unbound_edges = dropped_unbound_edges.checked_add(1).ok_or_else(|| {
                error(
                    GraphifyAdapterErrorKind::OutputLimit,
                    "GRAPHIFY_GRAPH_DROP_COUNT_OVERFLOW",
                )
            })?;
            continue;
        }
        let normalized = NormalizedGraphEdge {
            source: source.to_owned(),
            target: target.to_owned(),
            relation: relation.to_owned(),
            confidence,
            confidence_score,
            context,
            source_file: source_file.to_owned(),
            source_location: source_location.to_owned(),
        };
        if !edge_keys.insert(normalized.clone()) {
            return Err(error(
                GraphifyAdapterErrorKind::MalformedOutput,
                "GRAPHIFY_GRAPH_DUPLICATE_EDGE",
            ));
        }
        edges.push(normalized);
    }
    edges.sort();
    let record_set_sha256 = record_set_digest(&nodes, &edges);
    Ok(NormalizedGraph {
        nodes,
        edges,
        raw_graph_sha256: sha256_bytes(bytes),
        record_set_sha256,
        raw_node_count: raw_nodes.len(),
        raw_edge_count: raw_edges.len(),
        dropped_non_code_nodes,
        dropped_source_less_nodes,
        dropped_unbound_edges,
    })
}

fn validate_optional_scalar_fields(
    node: &Map<String, Value>,
    max_text_bytes: usize,
) -> GraphifyAdapterResult<()> {
    for key in [
        "type",
        "kind",
        "language",
        "namespace",
        "visibility",
        "signature",
        "category",
        "subtype",
        "_callable_class",
    ] {
        let _ = optional_text(node, key, max_text_bytes)?;
    }
    if let Some(value) = node.get("_callable")
        && !value.is_boolean()
    {
        return Err(error(
            GraphifyAdapterErrorKind::MalformedOutput,
            "GRAPHIFY_GRAPH_NODE_CALLABLE_MALFORMED",
        ));
    }
    Ok(())
}

fn require_exact_keys(
    object: &Map<String, Value>,
    expected: &[&str],
    code: &'static str,
) -> GraphifyAdapterResult<()> {
    if object.len() != expected.len()
        || object
            .keys()
            .any(|key| !expected.iter().any(|expected_key| key == expected_key))
    {
        return Err(error(GraphifyAdapterErrorKind::PartialOutput, code));
    }
    Ok(())
}

fn require_allowed_keys(
    object: &Map<String, Value>,
    allowed: &[&str],
    code: &'static str,
) -> GraphifyAdapterResult<()> {
    if object
        .keys()
        .any(|key| !allowed.iter().any(|allowed_key| key == allowed_key))
    {
        return Err(error(GraphifyAdapterErrorKind::MalformedOutput, code));
    }
    Ok(())
}

fn require_zero_u64(object: &Map<String, Value>, key: &str) -> GraphifyAdapterResult<()> {
    if object.get(key).and_then(Value::as_u64) != Some(0) {
        return Err(error(
            GraphifyAdapterErrorKind::PartialOutput,
            "GRAPHIFY_GRAPH_CODE_ONLY_TOKEN_COUNT_REJECTED",
        ));
    }
    Ok(())
}

fn required_text<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    max_bytes: usize,
) -> GraphifyAdapterResult<&'a str> {
    let value = object.get(key).and_then(Value::as_str).ok_or_else(|| {
        error(
            GraphifyAdapterErrorKind::MalformedOutput,
            "GRAPHIFY_GRAPH_REQUIRED_TEXT_MISSING",
        )
    })?;
    validate_text(value, max_bytes)?;
    if value.is_empty() {
        return Err(error(
            GraphifyAdapterErrorKind::MalformedOutput,
            "GRAPHIFY_GRAPH_REQUIRED_TEXT_EMPTY",
        ));
    }
    Ok(value)
}

fn required_text_allow_empty<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    max_bytes: usize,
) -> GraphifyAdapterResult<&'a str> {
    let value = object.get(key).and_then(Value::as_str).ok_or_else(|| {
        error(
            GraphifyAdapterErrorKind::MalformedOutput,
            "GRAPHIFY_GRAPH_REQUIRED_TEXT_MISSING",
        )
    })?;
    validate_text_allow_empty(value, max_bytes)?;
    Ok(value)
}

fn optional_text(
    object: &Map<String, Value>,
    key: &str,
    max_bytes: usize,
) -> GraphifyAdapterResult<Option<String>> {
    let Some(value) = object.get(key) else {
        return Ok(None);
    };
    let value = value.as_str().ok_or_else(|| {
        error(
            GraphifyAdapterErrorKind::MalformedOutput,
            "GRAPHIFY_GRAPH_OPTIONAL_TEXT_MALFORMED",
        )
    })?;
    validate_text(value, max_bytes)?;
    Ok(Some(value.to_owned()))
}

fn optional_number(
    object: &Map<String, Value>,
    key: &str,
    minimum: f64,
    maximum: f64,
) -> GraphifyAdapterResult<Option<String>> {
    let Some(value) = object.get(key) else {
        return Ok(None);
    };
    let number = value.as_f64().ok_or_else(|| {
        error(
            GraphifyAdapterErrorKind::MalformedOutput,
            "GRAPHIFY_GRAPH_OPTIONAL_NUMBER_MALFORMED",
        )
    })?;
    if !number.is_finite() || !(minimum..=maximum).contains(&number) {
        return Err(error(
            GraphifyAdapterErrorKind::MalformedOutput,
            "GRAPHIFY_GRAPH_OPTIONAL_NUMBER_REJECTED",
        ));
    }
    Ok(Some(value.to_string()))
}

fn validate_text(value: &str, max_bytes: usize) -> GraphifyAdapterResult<()> {
    if value.is_empty() {
        return Err(error(
            GraphifyAdapterErrorKind::MalformedOutput,
            "GRAPHIFY_GRAPH_TEXT_EMPTY",
        ));
    }
    validate_text_allow_empty(value, max_bytes)
}

fn validate_text_allow_empty(value: &str, max_bytes: usize) -> GraphifyAdapterResult<()> {
    if value.len() > max_bytes || value.chars().any(char::is_control) {
        return Err(error(
            GraphifyAdapterErrorKind::MalformedOutput,
            "GRAPHIFY_GRAPH_TEXT_REJECTED",
        ));
    }
    Ok(())
}

fn validate_manifest_path(
    source_file: &str,
    manifest_paths: &BTreeSet<&str>,
) -> GraphifyAdapterResult<()> {
    validate_relative_path(source_file).map_err(|_| {
        error(
            GraphifyAdapterErrorKind::ForeignSource,
            "GRAPHIFY_GRAPH_SOURCE_PATH_REJECTED",
        )
    })?;
    if !manifest_paths.contains(source_file) {
        return Err(error(
            GraphifyAdapterErrorKind::ForeignSource,
            "GRAPHIFY_GRAPH_FOREIGN_SOURCE",
        ));
    }
    Ok(())
}

fn is_known_relation(relation: &str) -> bool {
    matches!(
        relation,
        "binds_method"
            | "bound_to"
            | "calls"
            | "cites"
            | "contains"
            | "crate_depends_on"
            | "defines"
            | "depends_on"
            | "extends"
            | "implements"
            | "imports"
            | "imports_from"
            | "includes"
            | "indirect_call"
            | "inherits"
            | "instantiates"
            | "listened_by"
            | "method"
            | "overrides"
            | "rationale_for"
            | "re_exports"
            | "references"
            | "references_constant"
            | "uses"
            | "uses_component"
            | "uses_static_prop"
    )
}

fn record_set_digest(nodes: &[NormalizedGraphNode], edges: &[NormalizedGraphEdge]) -> String {
    let mut hasher = Sha256::new();
    hash_field(&mut hasher, b"graphify-record-set-v1");
    for node in nodes {
        hash_field(&mut hasher, b"node");
        for field in [
            node.id.as_bytes(),
            node.label.as_bytes(),
            node.source_file.as_bytes(),
            node.source_location.as_bytes(),
            node.kind.as_bytes(),
        ] {
            hash_field(&mut hasher, field);
        }
    }
    for edge in edges {
        hash_field(&mut hasher, b"edge");
        for field in [
            edge.source.as_bytes(),
            edge.target.as_bytes(),
            edge.relation.as_bytes(),
            edge.confidence.as_str().as_bytes(),
            edge.confidence_score.as_deref().unwrap_or("").as_bytes(),
            edge.context.as_deref().unwrap_or("").as_bytes(),
            edge.source_file.as_bytes(),
            edge.source_location.as_bytes(),
        ] {
            hash_field(&mut hasher, field);
        }
    }
    hex_digest(&hasher.finalize())
}

fn hash_field(hasher: &mut Sha256, field: &[u8]) {
    hasher.update((field.len() as u64).to_be_bytes());
    hasher.update(field);
}

fn hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

const fn error(kind: GraphifyAdapterErrorKind, code: &'static str) -> GraphifyAdapterError {
    GraphifyAdapterError::new(kind, code)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::snapshot::MaterializedSnapshot;

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    fn snapshot() -> MaterializedSnapshot {
        let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "lattice-graph-parse-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(root.join(".git")).expect("snapshot root");
        fs::create_dir_all(root.join("src")).expect("src");
        fs::write(root.join("src/lib.rs"), b"fn main() {}\n").expect("source");
        MaterializedSnapshot::for_test(root, vec![("src/lib.rs", b"fn main() {}\n")])
    }

    fn valid_graph() -> Vec<u8> {
        br#"{
          "nodes": [
            {"id":"src_lib","label":"lib.rs","file_type":"code","source_file":"src/lib.rs","source_location":"L1","_origin":"ast"},
            {"id":"src_lib_main","label":"main()","file_type":"code","source_file":"src/lib.rs","source_location":"L1","_origin":"ast"},
            {"id":"src_lib_string","label":"String","file_type":"code","source_file":"","source_location":"","_origin":"ast"}
          ],
          "edges": [
            {"source":"src_lib","target":"src_lib_main","relation":"contains","confidence":"EXTRACTED","source_file":"src/lib.rs","source_location":"L1","weight":1.0,"_origin":"ast"},
            {"source":"src_lib_main","target":"src_lib_string","relation":"references","confidence":"EXTRACTED","source_file":"src/lib.rs","source_location":"L1","weight":1.0,"context":"return_type","_origin":"ast"},
            {"source":"src_lib","target":"missing","relation":"imports_from","confidence":"EXTRACTED","source_file":"src/lib.rs","source_location":"L1","weight":1.0,"context":"import","_origin":"ast"}
          ],
          "hyperedges": [],
          "input_tokens": 0,
          "output_tokens": 0
        }"#
        .to_vec()
    }

    fn limits() -> GraphParseLimits {
        GraphParseLimits {
            max_nodes: 100,
            max_edges: 100,
            max_text_bytes: 1_024,
        }
    }

    #[test]
    fn parser_keeps_manifest_nodes_and_drops_source_less_or_dangling_facts() {
        let graph = parse_graph(&valid_graph(), &snapshot(), limits()).expect("valid graph");
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.dropped_source_less_nodes, 1);
        assert_eq!(graph.dropped_unbound_edges, 2);
    }

    #[test]
    fn parser_drops_explicit_non_code_nodes_without_weakening_ast_checks() {
        let source = String::from_utf8(valid_graph()).expect("fixture utf8");
        let graph = source.replacen(
            "\"file_type\":\"code\",\"source_file\":\"src/lib.rs\",\"source_location\":\"L1\",\"_origin\":\"ast\"",
            "\"file_type\":\"resource\",\"source_file\":\"src/lib.rs\",\"source_location\":\"L1\",\"_origin\":\"ast\"",
            1,
        );
        let graph = parse_graph(graph.as_bytes(), &snapshot(), limits()).expect("code-only graph");

        assert_eq!(graph.nodes.len(), 1);
        assert_eq!(graph.edges.len(), 0);
        assert_eq!(graph.dropped_non_code_nodes(), 1);
    }

    #[test]
    fn parser_rejects_foreign_sources_zero_nodes_and_partial_schema() {
        let foreign = String::from_utf8(valid_graph())
            .expect("utf8")
            .replace("src/lib.rs", "src/foreign.rs");
        assert_eq!(
            parse_graph(foreign.as_bytes(), &snapshot(), limits())
                .expect_err("foreign")
                .kind(),
            GraphifyAdapterErrorKind::ForeignSource
        );
        let zero = br#"{"nodes":[],"edges":[],"hyperedges":[],"input_tokens":0,"output_tokens":0}"#;
        assert_eq!(
            parse_graph(zero, &snapshot(), limits())
                .expect_err("zero")
                .kind(),
            GraphifyAdapterErrorKind::EmptyAnalysis
        );
        let partial = br#"{"nodes":[],"edges":[],"input_tokens":0,"output_tokens":0}"#;
        assert_eq!(
            parse_graph(partial, &snapshot(), limits())
                .expect_err("partial")
                .kind(),
            GraphifyAdapterErrorKind::PartialOutput
        );
    }

    #[test]
    fn parser_is_deterministic_for_identical_raw_graphs() {
        let snapshot = snapshot();
        let first = parse_graph(&valid_graph(), &snapshot, limits()).expect("first");
        let second = parse_graph(&valid_graph(), &snapshot, limits()).expect("second");
        assert_eq!(first.record_set_sha256, second.record_set_sha256);
        assert_eq!(first.raw_graph_sha256, second.raw_graph_sha256);
    }
}
