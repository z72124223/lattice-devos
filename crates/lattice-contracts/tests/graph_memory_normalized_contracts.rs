use lattice_contracts::{
    AttemptId, CONTRACT_VERSION, CodeSnapshotEvidence, CodebaseMemoryPersistenceIdentity,
    ContentDigest, GitObjectId, GraphConfidence, GraphMemoryPersistenceEvidence,
    GraphMemoryReceipt, GraphMemoryRecord, GraphMemoryRunRequest, GraphRecordKind,
    GraphSourceProvenance, GraphifyIdentity, GraphifyRawEvidence, GraphifyRawNode,
    HERMES_REFLECTION_SCHEMA_VERSION, HermesReflectionContent, HermesReflectionFinding,
    HermesReflectionReceipt, HermesReflectionStatus, Invocation, MemoryQuery, MemoryRecordKind,
    MemoryRetrievalDisposition, MemoryRetrievalEvidence, MemoryRetrievalPlan, MemoryReviewState,
    NormalizedGraphAnalysis, ProjectId, ProjectSnapshotId, RankedMemoryRecord, RequestId, TaskId,
    TrackedSource,
};

fn digest(byte: char) -> ContentDigest {
    ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("valid digest")
}

fn persistence_identity() -> CodebaseMemoryPersistenceIdentity {
    CodebaseMemoryPersistenceIdentity::v2(digest('2'), digest('3'), digest('4'), digest('5'))
        .expect("persistence identity")
}

fn fixture() -> (
    GraphMemoryRunRequest,
    CodeSnapshotEvidence,
    GraphifyRawEvidence,
) {
    let invocation = Invocation::new(
        CONTRACT_VERSION,
        RequestId::new("graph-request-2").expect("request id"),
        TaskId::new("TASK-033").expect("task id"),
        AttemptId::new("attempt-2").expect("attempt id"),
        ProjectSnapshotId::new("snapshot-2").expect("snapshot id"),
        digest('a'),
    )
    .expect("invocation");
    let request = GraphMemoryRunRequest::new(
        invocation,
        ProjectId::new("fixture-project").expect("project"),
        GitObjectId::new("1".repeat(40)).expect("commit"),
        digest('b'),
        digest('c'),
        5,
    )
    .expect("request");
    let source = TrackedSource::new("src/lib.rs", digest('d')).expect("source");
    let snapshot = CodeSnapshotEvidence::new(
        &request,
        GitObjectId::new("2".repeat(40)).expect("tree"),
        vec![source.clone()],
        digest('e'),
        digest('f'),
    )
    .expect("snapshot");
    let provenance = GraphSourceProvenance::new(&source, Some(1), Some(3)).expect("provenance");
    let raw = GraphifyRawEvidence::new(
        &request,
        &snapshot,
        GraphifyIdentity::task033(digest('1'), digest('2'), digest('3')).expect("identity"),
        vec![
            GraphifyRawNode::new(
                "node-a",
                "CodebaseMemoryPort",
                "trait",
                provenance,
                GraphConfidence::Extracted,
            )
            .expect("node"),
        ],
        vec![],
        digest('4'),
        digest('5'),
        digest('6'),
    )
    .expect("raw");
    (request, snapshot, raw)
}

fn analysis() -> NormalizedGraphAnalysis {
    let (request, snapshot, raw) = fixture();
    let provenance = raw.nodes()[0].provenance().clone();
    let record = GraphMemoryRecord::candidate(
        1,
        GraphRecordKind::Node,
        "CodebaseMemoryPort",
        "trait",
        None,
        None,
        provenance,
        GraphConfidence::Extracted,
        digest('7'),
        digest('8'),
    )
    .expect("record");
    NormalizedGraphAnalysis::new(
        &request,
        &snapshot,
        &raw,
        vec![record],
        digest('9'),
        digest('a'),
        digest('b'),
    )
    .expect("analysis")
}

#[test]
fn normalized_records_are_candidate_observations_and_never_trusted() {
    let analysis = analysis();
    let record = &analysis.records()[0];

    assert_eq!(record.record_kind(), MemoryRecordKind::Observation);
    assert_eq!(record.review_state(), MemoryReviewState::Candidate);
    assert!(!record.trusted_context());
    assert_eq!(record.ordinal(), 1);
    assert_eq!(analysis.commit_id().as_str(), "1".repeat(40));
}

#[test]
fn record_shape_and_analysis_order_fail_closed() {
    let (_, _, raw) = fixture();
    let provenance = raw.nodes()[0].provenance().clone();
    assert!(
        GraphMemoryRecord::candidate(
            1,
            GraphRecordKind::Node,
            "node",
            "function",
            Some("uses".to_owned()),
            None,
            provenance.clone(),
            GraphConfidence::Extracted,
            digest('7'),
            digest('8'),
        )
        .is_err()
    );

    let (request, snapshot, raw) = fixture();
    let record = GraphMemoryRecord::candidate(
        2,
        GraphRecordKind::Node,
        "node",
        "function",
        None,
        None,
        provenance,
        GraphConfidence::Extracted,
        digest('7'),
        digest('8'),
    )
    .expect("record shape");
    assert!(
        NormalizedGraphAnalysis::new(
            &request,
            &snapshot,
            &raw,
            vec![record],
            digest('9'),
            digest('a'),
            digest('b'),
        )
        .is_err()
    );
}

#[test]
fn normalized_analysis_rejects_noncanonical_record_identity_order() {
    let (request, snapshot, raw) = fixture();
    let provenance = raw.nodes()[0].provenance().clone();
    let two_node_raw = GraphifyRawEvidence::new(
        &request,
        &snapshot,
        raw.identity().clone(),
        vec![
            GraphifyRawNode::new(
                "node-a",
                "alpha",
                "function",
                provenance.clone(),
                GraphConfidence::Extracted,
            )
            .expect("alpha"),
            GraphifyRawNode::new(
                "node-b",
                "beta",
                "function",
                provenance.clone(),
                GraphConfidence::Extracted,
            )
            .expect("beta"),
        ],
        vec![],
        digest('4'),
        digest('5'),
        digest('6'),
    )
    .expect("two node graph");
    let records = vec![
        GraphMemoryRecord::candidate(
            1,
            GraphRecordKind::Node,
            "alpha",
            "function",
            None,
            None,
            provenance.clone(),
            GraphConfidence::Extracted,
            digest('7'),
            digest('9'),
        )
        .expect("alpha record"),
        GraphMemoryRecord::candidate(
            2,
            GraphRecordKind::Node,
            "beta",
            "function",
            None,
            None,
            provenance,
            GraphConfidence::Extracted,
            digest('8'),
            digest('8'),
        )
        .expect("beta record"),
    ];

    assert!(
        NormalizedGraphAnalysis::new(
            &request,
            &snapshot,
            &two_node_raw,
            records,
            digest('9'),
            digest('a'),
            digest('b'),
        )
        .is_err()
    );
}

#[test]
fn retrieval_plan_and_receipt_preserve_exact_binding_and_no_answer() {
    let analysis = analysis();
    let query = MemoryQuery::new(analysis.request(), "unrelated-token", 5).expect("query");
    let no_answer =
        MemoryRetrievalPlan::new(&analysis, &query, vec![], digest('c')).expect("no answer plan");
    assert_eq!(
        no_answer.disposition(),
        MemoryRetrievalDisposition::NoAnswer
    );

    let query = MemoryQuery::new(analysis.request(), "CodebaseMemoryPort", 5).expect("query");
    let ranked = RankedMemoryRecord::new(&analysis.records()[0], 1, 1_000).expect("ranked");
    let plan = MemoryRetrievalPlan::new(&analysis, &query, vec![ranked], digest('d'))
        .expect("result plan");
    let identity = persistence_identity();
    let persisted = GraphMemoryPersistenceEvidence::new(&analysis, identity.clone(), digest('e'))
        .expect("persistence evidence");
    let retrieval =
        MemoryRetrievalEvidence::new(&persisted, plan, digest('f')).expect("retrieval evidence");
    let receipt = GraphMemoryReceipt::new(persisted, retrieval, digest('1')).expect("receipt");

    assert!(receipt.matches_request(analysis.request()));
    assert_eq!(receipt.persistence().identity(), &identity);
    let different_limit = GraphMemoryRunRequest::new(
        analysis.request().invocation().clone(),
        analysis.request().project_id().clone(),
        analysis.request().commit_id().clone(),
        analysis.request().query_digest().clone(),
        analysis.request().configuration_digest().clone(),
        6,
    )
    .expect("different request-bound limit");
    assert!(!receipt.matches_request(&different_limit));
    assert_eq!(receipt.retrieval().results().len(), 1);
    assert_eq!(
        receipt.retrieval().disposition(),
        MemoryRetrievalDisposition::Results
    );
}

#[test]
fn durable_receipt_can_be_reconstructed_from_exact_typed_rows() {
    let analysis = analysis();
    let query = MemoryQuery::new(analysis.request(), "CodebaseMemoryPort", 5).expect("query");
    let ranked = RankedMemoryRecord::new(&analysis.records()[0], 1, 1_000).expect("ranked");
    let plan = MemoryRetrievalPlan::new(&analysis, &query, vec![ranked], digest('d'))
        .expect("result plan");
    let identity = persistence_identity();
    let persisted = GraphMemoryPersistenceEvidence::replay(
        analysis.request().clone(),
        identity,
        analysis.analysis_digest().clone(),
        analysis.record_set_digest().clone(),
        1,
        digest('e'),
    )
    .expect("replayed persistence");
    let replayed_result = RankedMemoryRecord::replay(
        analysis.records()[0].record_id().clone(),
        analysis.records()[0].content_digest().clone(),
        1,
        1_000,
    )
    .expect("replayed result");
    let retrieval = MemoryRetrievalEvidence::replay(
        &persisted,
        plan.limit(),
        plan.disposition(),
        vec![replayed_result],
        plan.result_set_digest().clone(),
        digest('f'),
    )
    .expect("replayed retrieval");
    let receipt = GraphMemoryReceipt::new(persisted, retrieval, digest('1')).expect("receipt");

    assert!(receipt.matches_request(analysis.request()));
    assert_eq!(receipt.retrieval().results()[0].rank(), 1);
    assert_eq!(receipt.retrieval().results()[0].score(), 1_000);
}

#[test]
fn hermes_reflection_receipt_is_inference_candidate_bound_to_exact_graph_receipt() {
    let analysis = analysis();
    let query = MemoryQuery::new(analysis.request(), "CodebaseMemoryPort", 5).expect("query");
    let ranked = RankedMemoryRecord::new(&analysis.records()[0], 1, 1_000).expect("ranked");
    let plan = MemoryRetrievalPlan::new(&analysis, &query, vec![ranked], digest('d'))
        .expect("result plan");
    let persisted =
        GraphMemoryPersistenceEvidence::new(&analysis, persistence_identity(), digest('e'))
            .expect("persistence evidence");
    let retrieval =
        MemoryRetrievalEvidence::new(&persisted, plan, digest('f')).expect("retrieval evidence");
    let graph_receipt =
        GraphMemoryReceipt::new(persisted, retrieval, digest('1')).expect("graph receipt");
    let content = HermesReflectionContent::new(
        "The graph receipt supports one bounded integration finding.",
        vec![
            HermesReflectionFinding::new(
                "Persist reflection only after the exact graph receipt.",
                digest('a'),
            )
            .expect("finding"),
        ],
        vec!["Load the typed reflection from PostgreSQL status.".to_owned()],
    )
    .expect("structured reflection");

    let reflection = HermesReflectionReceipt::new(
        analysis.request(),
        &graph_receipt,
        content,
        digest('2'),
        digest('3'),
        digest('4'),
        digest('5'),
    )
    .expect("reflection receipt");

    assert_eq!(
        reflection.schema_version(),
        HERMES_REFLECTION_SCHEMA_VERSION
    );
    assert_eq!(
        reflection.status(),
        HermesReflectionStatus::InferenceCandidate
    );
    assert_eq!(reflection.project_id(), analysis.request().project_id());
    assert_eq!(reflection.commit_id(), analysis.request().commit_id());
    assert_eq!(
        reflection.graph_receipt_digest(),
        graph_receipt.receipt_digest()
    );
    assert_eq!(
        reflection.content().summary(),
        "The graph receipt supports one bounded integration finding."
    );
    assert_eq!(reflection.content().findings().len(), 1);
    assert_eq!(reflection.content().next_actions().len(), 1);
    assert!(reflection.matches_request(analysis.request()));

    let different_limit = GraphMemoryRunRequest::new(
        analysis.request().invocation().clone(),
        analysis.request().project_id().clone(),
        analysis.request().commit_id().clone(),
        analysis.request().query_digest().clone(),
        analysis.request().configuration_digest().clone(),
        6,
    )
    .expect("different request");
    assert!(
        HermesReflectionReceipt::new(
            &different_limit,
            &graph_receipt,
            HermesReflectionContent::new("bounded", vec![], vec![]).expect("content"),
            digest('2'),
            digest('3'),
            digest('4'),
            digest('5'),
        )
        .is_err()
    );
}
