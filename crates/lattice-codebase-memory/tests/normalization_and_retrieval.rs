use lattice_codebase_memory::{digest_query_text, normalize_analysis, plan_retrieval};
use lattice_contracts::{
    AttemptId, CONTRACT_VERSION, CodeSnapshotEvidence, ContentDigest, GitObjectId, GraphConfidence,
    GraphMemoryRunRequest, GraphSourceProvenance, GraphifyIdentity, GraphifyRawEdge,
    GraphifyRawEvidence, GraphifyRawNode, Invocation, MemoryQuery, MemoryRetrievalDisposition,
    ProjectId, ProjectSnapshotId, RequestId, TaskId, TrackedSource,
};

fn digest(byte: char) -> ContentDigest {
    ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("valid digest")
}

fn fixture(
    query: &str,
    commit_byte: char,
    reverse_nodes: bool,
) -> (
    GraphMemoryRunRequest,
    CodeSnapshotEvidence,
    GraphifyRawEvidence,
) {
    let invocation = Invocation::new(
        CONTRACT_VERSION,
        RequestId::new(format!("graph-request-{commit_byte}")).expect("request id"),
        TaskId::new("TASK-033").expect("task id"),
        AttemptId::new("attempt-1").expect("attempt id"),
        ProjectSnapshotId::new(format!("snapshot-{commit_byte}")).expect("snapshot id"),
        digest('a'),
    )
    .expect("invocation");
    let request = GraphMemoryRunRequest::new(
        invocation,
        ProjectId::new("fixture-project").expect("project"),
        GitObjectId::new(commit_byte.to_string().repeat(40)).expect("commit"),
        digest_query_text(query).expect("query digest"),
        digest('c'),
        10,
    )
    .expect("request");
    let lib = TrackedSource::new("src/lib.rs", digest('d')).expect("lib");
    let port = TrackedSource::new("src/port.rs", digest('e')).expect("port");
    let snapshot = CodeSnapshotEvidence::new(
        &request,
        GitObjectId::new("f".repeat(40)).expect("tree"),
        vec![lib.clone(), port.clone()],
        digest('1'),
        digest('2'),
    )
    .expect("snapshot");
    let mut nodes = vec![
        GraphifyRawNode::new(
            "node-run",
            "run_graph_memory",
            "function",
            GraphSourceProvenance::new(&lib, Some(10), Some(20)).expect("lib provenance"),
            GraphConfidence::Extracted,
        )
        .expect("run node"),
        GraphifyRawNode::new(
            "node-port",
            "CodebaseMemoryPort",
            "trait",
            GraphSourceProvenance::new(&port, Some(1), Some(8)).expect("port provenance"),
            GraphConfidence::Inferred,
        )
        .expect("port node"),
    ];
    if reverse_nodes {
        nodes.reverse();
    }
    let edges = vec![
        GraphifyRawEdge::new(
            "edge-uses",
            "node-run",
            "node-port",
            "uses",
            GraphSourceProvenance::new(&lib, Some(15), Some(15)).expect("edge provenance"),
            GraphConfidence::Ambiguous,
        )
        .expect("edge"),
    ];
    let raw = GraphifyRawEvidence::new(
        &request,
        &snapshot,
        GraphifyIdentity::task033(digest('3'), digest('4'), digest('5')).expect("identity"),
        nodes,
        edges,
        digest('6'),
        digest('7'),
        digest('8'),
    )
    .expect("raw evidence");
    (request, snapshot, raw)
}

#[test]
fn normalization_is_deterministic_and_preserves_untrusted_confidence() {
    let (request_a, snapshot_a, raw_a) = fixture("CodebaseMemoryPort", '1', false);
    let (request_b, snapshot_b, raw_b) = fixture("CodebaseMemoryPort", '1', true);

    let analysis_a = normalize_analysis(&request_a, &snapshot_a, &raw_a).expect("analysis a");
    let analysis_b = normalize_analysis(&request_b, &snapshot_b, &raw_b).expect("analysis b");

    assert_eq!(analysis_a, analysis_b);
    assert_eq!(analysis_a.records().len(), 3);
    assert!(
        analysis_a
            .records()
            .iter()
            .any(|record| record.confidence() == GraphConfidence::Ambiguous)
    );
    assert!(
        analysis_a
            .records()
            .iter()
            .all(|record| !record.trusted_context())
    );
}

#[test]
fn exact_commit_changes_record_and_analysis_identity() {
    let (request_a, snapshot_a, raw_a) = fixture("run_graph_memory", '1', false);
    let (request_b, snapshot_b, raw_b) = fixture("run_graph_memory", '2', false);
    let analysis_a = normalize_analysis(&request_a, &snapshot_a, &raw_a).expect("analysis a");
    let analysis_b = normalize_analysis(&request_b, &snapshot_b, &raw_b).expect("analysis b");

    assert_ne!(analysis_a.analysis_digest(), analysis_b.analysis_digest());
    assert_ne!(
        analysis_a.records()[0].record_id(),
        analysis_b.records()[0].record_id()
    );
}

#[test]
fn retrieval_prioritizes_exact_identifier_then_uses_stable_ties() {
    let (request, snapshot, raw) = fixture("CodebaseMemoryPort", '1', true);
    let analysis = normalize_analysis(&request, &snapshot, &raw).expect("analysis");
    let query = MemoryQuery::new(&request, "CodebaseMemoryPort", 10).expect("query");
    let first = plan_retrieval(&analysis, &query).expect("first plan");
    let second = plan_retrieval(&analysis, &query).expect("second plan");

    assert_eq!(first, second);
    assert_eq!(first.disposition(), MemoryRetrievalDisposition::Results);
    let top = analysis
        .records()
        .iter()
        .find(|record| record.record_id() == first.results()[0].record_id())
        .expect("top record");
    assert_eq!(top.subject(), "CodebaseMemoryPort");
}

#[test]
fn irrelevant_query_returns_no_answer_and_digest_mismatch_is_rejected() {
    let (request, snapshot, raw) = fixture("totally-unrelated-token", '1', false);
    let analysis = normalize_analysis(&request, &snapshot, &raw).expect("analysis");
    let query = MemoryQuery::new(&request, "totally-unrelated-token", 10).expect("query");
    let plan = plan_retrieval(&analysis, &query).expect("no answer");
    assert_eq!(plan.disposition(), MemoryRetrievalDisposition::NoAnswer);
    assert!(plan.results().is_empty());

    let wrong = MemoryQuery::new(&request, "CodebaseMemoryPort", 10).expect("wrong query");
    assert!(plan_retrieval(&analysis, &wrong).is_err());
}

#[test]
fn duplicate_canonical_records_are_rejected() {
    let (request, snapshot, raw) = fixture("duplicate", '1', false);
    let source = raw.nodes()[0].provenance().clone();
    let duplicate_raw = GraphifyRawEvidence::new(
        &request,
        &snapshot,
        raw.identity().clone(),
        vec![
            GraphifyRawNode::new(
                "node-one",
                "duplicate",
                "function",
                source.clone(),
                GraphConfidence::Extracted,
            )
            .expect("one"),
            GraphifyRawNode::new(
                "node-two",
                "duplicate",
                "function",
                source,
                GraphConfidence::Extracted,
            )
            .expect("two"),
        ],
        vec![],
        digest('6'),
        digest('7'),
        digest('8'),
    )
    .expect("duplicate raw graph is structurally complete");

    assert!(normalize_analysis(&request, &snapshot, &duplicate_raw).is_err());
}
