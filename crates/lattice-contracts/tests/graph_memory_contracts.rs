use lattice_contracts::{
    AttemptId, CONTRACT_VERSION, CodeSnapshotEvidence, ContentDigest, GitObjectId, GraphConfidence,
    GraphMemoryRunRequest, GraphSourceProvenance, GraphifyIdentity, GraphifyRawEdge,
    GraphifyRawEvidence, GraphifyRawNode, Invocation, MemoryQuery, ProjectId, ProjectSnapshotId,
    RequestId, TaskId, TrackedSource,
};

fn digest(byte: char) -> ContentDigest {
    ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("valid digest")
}

fn request() -> GraphMemoryRunRequest {
    let invocation = Invocation::new(
        CONTRACT_VERSION,
        RequestId::new("graph-request-1").expect("request id"),
        TaskId::new("TASK-033").expect("task id"),
        AttemptId::new("attempt-1").expect("attempt id"),
        ProjectSnapshotId::new("snapshot-1").expect("snapshot id"),
        digest('a'),
    )
    .expect("invocation");
    GraphMemoryRunRequest::new(
        invocation,
        ProjectId::new("fixture-project").expect("project"),
        GitObjectId::new("1".repeat(40)).expect("commit"),
        digest('b'),
        digest('c'),
        5,
    )
    .expect("graph-memory request")
}

#[test]
fn graph_memory_request_binds_one_bounded_retrieval_limit() {
    let request = request();
    assert_eq!(request.retrieval_limit(), 5);

    assert!(
        GraphMemoryRunRequest::new(
            request.invocation().clone(),
            request.project_id().clone(),
            request.commit_id().clone(),
            request.query_digest().clone(),
            request.configuration_digest().clone(),
            0,
        )
        .is_err()
    );
    assert!(
        GraphMemoryRunRequest::new(
            request.invocation().clone(),
            request.project_id().clone(),
            request.commit_id().clone(),
            request.query_digest().clone(),
            request.configuration_digest().clone(),
            lattice_contracts::GRAPH_MEMORY_MAX_RESULTS + 1,
        )
        .is_err()
    );
    assert!(MemoryQuery::new(&request, "fixed-query", 4).is_err());
    assert!(MemoryQuery::new(&request, "fixed-query", 5).is_ok());
}

#[test]
fn exact_snapshot_binds_sorted_tracked_sources_to_commit_and_tree() {
    let request = request();
    let source = TrackedSource::new("src/lib.rs", digest('d')).expect("tracked source");
    let snapshot = CodeSnapshotEvidence::new(
        &request,
        GitObjectId::new("2".repeat(40)).expect("tree"),
        vec![source],
        digest('e'),
        digest('f'),
    )
    .expect("snapshot evidence");

    assert_eq!(snapshot.commit_id(), request.commit_id());
    assert_eq!(snapshot.sources()[0].relative_path(), "src/lib.rs");
}

#[test]
fn snapshot_rejects_escape_and_nondeterministic_manifest_order() {
    assert!(TrackedSource::new("../secret.txt", digest('d')).is_err());

    let request = request();
    let sources = vec![
        TrackedSource::new("src/z.rs", digest('d')).expect("z"),
        TrackedSource::new("src/a.rs", digest('e')).expect("a"),
    ];
    assert!(
        CodeSnapshotEvidence::new(
            &request,
            GitObjectId::new("2".repeat(40)).expect("tree"),
            sources,
            digest('f'),
            digest('1'),
        )
        .is_err()
    );
}

fn snapshot() -> CodeSnapshotEvidence {
    let request = request();
    CodeSnapshotEvidence::new(
        &request,
        GitObjectId::new("2".repeat(40)).expect("tree"),
        vec![
            TrackedSource::new("src/a.rs", digest('d')).expect("a"),
            TrackedSource::new("src/lib.rs", digest('e')).expect("lib"),
        ],
        digest('f'),
        digest('1'),
    )
    .expect("snapshot")
}

#[test]
fn graphify_raw_evidence_is_exactly_snapshot_bound_and_provenance_checked() {
    let snapshot = snapshot();
    let identity =
        GraphifyIdentity::task033(digest('2'), digest('3'), digest('4')).expect("pinned identity");
    let source = GraphSourceProvenance::new(&snapshot.sources()[1], Some(3), Some(9))
        .expect("source provenance");
    let nodes = vec![
        GraphifyRawNode::new(
            "node-a",
            "run_graph_memory",
            "function",
            source.clone(),
            GraphConfidence::Extracted,
        )
        .expect("node a"),
        GraphifyRawNode::new(
            "node-b",
            "CodebaseMemoryPort",
            "trait",
            source.clone(),
            GraphConfidence::Inferred,
        )
        .expect("node b"),
    ];
    let edges = vec![
        GraphifyRawEdge::new(
            "edge-a",
            "node-a",
            "node-b",
            "uses",
            source,
            GraphConfidence::Ambiguous,
        )
        .expect("edge"),
    ];

    let evidence = GraphifyRawEvidence::new(
        snapshot.request(),
        &snapshot,
        identity,
        nodes,
        edges,
        digest('5'),
        digest('6'),
        digest('7'),
    )
    .expect("raw evidence");

    assert_eq!(evidence.commit_id(), snapshot.commit_id());
    assert_eq!(evidence.nodes().len(), 2);
    assert_eq!(GraphConfidence::Extracted.as_str(), "EXTRACTED");
    assert_eq!(GraphConfidence::Inferred.as_str(), "INFERRED");
    assert_eq!(GraphConfidence::Ambiguous.as_str(), "AMBIGUOUS");
}

#[test]
fn graphify_raw_evidence_rejects_foreign_source_and_dangling_edges() {
    let snapshot = snapshot();
    let identity =
        GraphifyIdentity::task033(digest('2'), digest('3'), digest('4')).expect("pinned identity");
    let foreign = TrackedSource::new("src/foreign.rs", digest('8')).expect("foreign source");
    let provenance = GraphSourceProvenance::new(&foreign, None, None).expect("provenance");
    let node = GraphifyRawNode::new(
        "node-a",
        "foreign",
        "function",
        provenance,
        GraphConfidence::Extracted,
    )
    .expect("raw node");

    assert!(
        GraphifyRawEvidence::new(
            snapshot.request(),
            &snapshot,
            identity.clone(),
            vec![node],
            vec![],
            digest('5'),
            digest('6'),
            digest('7'),
        )
        .is_err()
    );

    let source =
        GraphSourceProvenance::new(&snapshot.sources()[0], None, None).expect("source provenance");
    let node = GraphifyRawNode::new(
        "node-a",
        "valid",
        "function",
        source.clone(),
        GraphConfidence::Extracted,
    )
    .expect("node");
    let dangling = GraphifyRawEdge::new(
        "edge-a",
        "node-a",
        "node-missing",
        "uses",
        source,
        GraphConfidence::Extracted,
    )
    .expect("edge shape");
    assert!(
        GraphifyRawEvidence::new(
            snapshot.request(),
            &snapshot,
            identity,
            vec![node],
            vec![dangling],
            digest('5'),
            digest('6'),
            digest('7'),
        )
        .is_err()
    );
}
