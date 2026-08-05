use std::cell::RefCell;
use std::rc::Rc;

use lattice_codebase_memory::digest_query_text;
use lattice_contracts::{
    AttemptId, CONTRACT_VERSION, CodeSnapshotEvidence, CodebaseMemoryPersistenceIdentity,
    ContentDigest, GitObjectId, GraphConfidence, GraphMemoryPersistenceEvidence,
    GraphMemoryReceipt, GraphMemoryRunRequest, GraphSourceProvenance, GraphifyIdentity,
    GraphifyRawEdge, GraphifyRawEvidence, GraphifyRawNode, Invocation, MemoryQuery,
    MemoryRetrievalEvidence, MemoryRetrievalPlan, NormalizedGraphAnalysis, ProjectId,
    ProjectSnapshotId, RequestId, TaskId, TrackedSource,
};
use lattice_orchestrator::{GraphMemoryOrchestratorError, graph_memory_status, run_graph_memory};
use lattice_ports::{
    CodeSnapshotPort, CodebaseMemoryPort, GraphMemoryFailureCertainty, GraphMemoryPortError,
    GraphMemoryPortResult, GraphMemoryStage, GraphifyAnalysisPort, PortErrorKind,
};

fn digest(byte: char) -> ContentDigest {
    ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("valid digest")
}

fn persistence_identity() -> CodebaseMemoryPersistenceIdentity {
    CodebaseMemoryPersistenceIdentity::v2(digest('a'), digest('b'), digest('c'), digest('d'))
        .expect("persistence identity")
}

fn request(query: &str) -> GraphMemoryRunRequest {
    GraphMemoryRunRequest::new(
        Invocation::new(
            CONTRACT_VERSION,
            RequestId::new("graph-order-request").expect("request id"),
            TaskId::new("TASK-033").expect("task id"),
            AttemptId::new("graph-order-attempt").expect("attempt id"),
            ProjectSnapshotId::new("graph-order-snapshot").expect("snapshot id"),
            digest('a'),
        )
        .expect("invocation"),
        ProjectId::new("fixture-project").expect("project"),
        GitObjectId::new("1".repeat(40)).expect("commit"),
        digest_query_text(query).expect("query digest"),
        digest('b'),
        10,
    )
    .expect("request")
}

fn snapshot(request: &GraphMemoryRunRequest) -> CodeSnapshotEvidence {
    CodeSnapshotEvidence::new(
        request,
        GitObjectId::new("2".repeat(40)).expect("tree"),
        vec![TrackedSource::new("src/lib.rs", digest('c')).expect("source")],
        digest('d'),
        digest('e'),
    )
    .expect("snapshot")
}

fn raw(request: &GraphMemoryRunRequest, snapshot: &CodeSnapshotEvidence) -> GraphifyRawEvidence {
    let source =
        GraphSourceProvenance::new(&snapshot.sources()[0], Some(1), Some(2)).expect("provenance");
    GraphifyRawEvidence::new(
        request,
        snapshot,
        GraphifyIdentity::task033(digest('f'), digest('1'), digest('2')).expect("identity"),
        vec![
            GraphifyRawNode::new(
                "node-run",
                "run_graph_memory",
                "function",
                source.clone(),
                GraphConfidence::Extracted,
            )
            .expect("run node"),
            GraphifyRawNode::new(
                "node-port",
                "CodebaseMemoryPort",
                "trait",
                source.clone(),
                GraphConfidence::Inferred,
            )
            .expect("port node"),
        ],
        vec![
            GraphifyRawEdge::new(
                "edge-uses",
                "node-run",
                "node-port",
                "uses",
                source,
                GraphConfidence::Ambiguous,
            )
            .expect("edge"),
        ],
        digest('3'),
        digest('4'),
        digest('5'),
    )
    .expect("raw evidence")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FailAt {
    Never,
    Snapshot,
    Graphify,
    Persistence,
    Retrieval,
    Receipt,
    SubstitutedRetrieval,
}

fn failure(stage: GraphMemoryStage) -> GraphMemoryPortError {
    GraphMemoryPortError::new(
        stage,
        PortErrorKind::Timeout,
        GraphMemoryFailureCertainty::Ambiguous,
        "INJECTED_AMBIGUOUS_FAILURE",
    )
}

struct SnapshotFake {
    calls: Rc<RefCell<Vec<&'static str>>>,
    evidence: CodeSnapshotEvidence,
    fail_at: FailAt,
}

impl CodeSnapshotPort for SnapshotFake {
    fn materialize_snapshot(
        &mut self,
        _request: &GraphMemoryRunRequest,
    ) -> GraphMemoryPortResult<CodeSnapshotEvidence> {
        self.calls.borrow_mut().push("snapshot");
        if self.fail_at == FailAt::Snapshot {
            Err(failure(GraphMemoryStage::Snapshot))
        } else {
            Ok(self.evidence.clone())
        }
    }
}

struct GraphifyFake {
    calls: Rc<RefCell<Vec<&'static str>>>,
    evidence: GraphifyRawEvidence,
    fail_at: FailAt,
}

impl GraphifyAnalysisPort for GraphifyFake {
    fn analyze(
        &mut self,
        _request: &GraphMemoryRunRequest,
        _snapshot: &CodeSnapshotEvidence,
    ) -> GraphMemoryPortResult<GraphifyRawEvidence> {
        self.calls.borrow_mut().push("graphify");
        if self.fail_at == FailAt::Graphify {
            Err(failure(GraphMemoryStage::Graphify))
        } else {
            Ok(self.evidence.clone())
        }
    }
}

struct MemoryFake {
    calls: Rc<RefCell<Vec<&'static str>>>,
    fail_at: FailAt,
    analysis: Option<NormalizedGraphAnalysis>,
    receipt: Option<GraphMemoryReceipt>,
}

impl CodebaseMemoryPort for MemoryFake {
    fn persist_analysis(
        &mut self,
        analysis: &NormalizedGraphAnalysis,
    ) -> GraphMemoryPortResult<GraphMemoryPersistenceEvidence> {
        self.calls.borrow_mut().push("persist");
        if self.fail_at == FailAt::Persistence {
            Err(failure(GraphMemoryStage::Persistence))
        } else {
            self.analysis = Some(analysis.clone());
            GraphMemoryPersistenceEvidence::new(analysis, persistence_identity(), digest('6'))
                .map_err(|_| {
                    GraphMemoryPortError::new(
                        GraphMemoryStage::Persistence,
                        PortErrorKind::Malformed,
                        GraphMemoryFailureCertainty::Known,
                        "FAKE_PERSISTENCE_CONTRACT",
                    )
                })
        }
    }

    fn retrieve(
        &mut self,
        persistence: &GraphMemoryPersistenceEvidence,
        plan: MemoryRetrievalPlan,
    ) -> GraphMemoryPortResult<GraphMemoryReceipt> {
        self.calls.borrow_mut().push("retrieve");
        if self.fail_at == FailAt::Retrieval {
            return Err(failure(GraphMemoryStage::Retrieval));
        }
        let plan = if self.fail_at == FailAt::SubstitutedRetrieval {
            let analysis = self.analysis.as_ref().expect("persisted analysis");
            let query = MemoryQuery::new(
                analysis.request(),
                "CodebaseMemoryPort",
                analysis.request().retrieval_limit(),
            )
            .expect("replacement query");
            MemoryRetrievalPlan::new(analysis, &query, vec![], digest('9'))
                .expect("same-request substituted plan")
        } else {
            plan
        };
        let retrieval =
            MemoryRetrievalEvidence::new(persistence, plan, digest('7')).map_err(|_| {
                GraphMemoryPortError::new(
                    GraphMemoryStage::Retrieval,
                    PortErrorKind::Malformed,
                    GraphMemoryFailureCertainty::Known,
                    "FAKE_RETRIEVAL_CONTRACT",
                )
            })?;
        let receipt = GraphMemoryReceipt::new(persistence.clone(), retrieval, digest('8'))
            .map_err(|_| {
                GraphMemoryPortError::new(
                    GraphMemoryStage::Retrieval,
                    PortErrorKind::Malformed,
                    GraphMemoryFailureCertainty::Known,
                    "FAKE_RECEIPT_CONTRACT",
                )
            })?;
        self.receipt = Some(receipt.clone());
        Ok(receipt)
    }

    fn load_receipt(
        &mut self,
        _request: &GraphMemoryRunRequest,
    ) -> GraphMemoryPortResult<GraphMemoryReceipt> {
        self.calls.borrow_mut().push("receipt");
        if self.fail_at == FailAt::Receipt {
            Err(failure(GraphMemoryStage::Receipt))
        } else {
            self.receipt.clone().ok_or_else(|| {
                GraphMemoryPortError::new(
                    GraphMemoryStage::Receipt,
                    PortErrorKind::Malformed,
                    GraphMemoryFailureCertainty::Known,
                    "FAKE_RECEIPT_MISSING",
                )
            })
        }
    }
}

fn run(
    fail_at: FailAt,
) -> (
    Vec<&'static str>,
    Result<GraphMemoryReceipt, GraphMemoryOrchestratorError>,
) {
    let request = request("CodebaseMemoryPort");
    let snapshot_evidence = snapshot(&request);
    let raw_evidence = raw(&request, &snapshot_evidence);
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut snapshot = SnapshotFake {
        calls: Rc::clone(&calls),
        evidence: snapshot_evidence,
        fail_at,
    };
    let mut graphify = GraphifyFake {
        calls: Rc::clone(&calls),
        evidence: raw_evidence,
        fail_at,
    };
    let mut memory = MemoryFake {
        calls: Rc::clone(&calls),
        fail_at,
        analysis: None,
        receipt: None,
    };
    let query = MemoryQuery::new(&request, "CodebaseMemoryPort", 10).expect("query");
    let result = run_graph_memory(&request, &query, &mut snapshot, &mut graphify, &mut memory);
    let observed = calls.borrow().clone();
    (observed, result)
}

#[test]
fn graph_memory_effect_order_is_fixed_and_receipt_is_reloaded() {
    let (calls, result) = run(FailAt::Never);
    let receipt = result.expect("terminal receipt");
    assert!(receipt.matches_request(receipt.persistence().request()));
    assert_eq!(
        calls,
        vec!["snapshot", "graphify", "persist", "retrieve", "receipt"]
    );
}

#[test]
fn every_effect_failure_stops_all_later_calls() {
    for (fail_at, expected) in [
        (FailAt::Snapshot, vec!["snapshot"]),
        (FailAt::Graphify, vec!["snapshot", "graphify"]),
        (FailAt::Persistence, vec!["snapshot", "graphify", "persist"]),
        (
            FailAt::Retrieval,
            vec!["snapshot", "graphify", "persist", "retrieve"],
        ),
        (
            FailAt::Receipt,
            vec!["snapshot", "graphify", "persist", "retrieve", "receipt"],
        ),
    ] {
        let (calls, result) = run(fail_at);
        assert!(result.is_err(), "{fail_at:?} unexpectedly succeeded");
        assert_eq!(calls, expected, "{fail_at:?} called a later effect");
    }
}

#[test]
fn same_request_substituted_retrieval_plan_is_rejected_before_receipt_readback() {
    let (calls, result) = run(FailAt::SubstitutedRetrieval);
    assert!(matches!(
        result,
        Err(GraphMemoryOrchestratorError::EvidenceMismatch(
            GraphMemoryStage::Retrieval
        ))
    ));
    assert_eq!(calls, vec!["snapshot", "graphify", "persist", "retrieve"]);
}

#[test]
fn query_digest_rejection_has_zero_memory_side_effects() {
    let request = request("different-query");
    let snapshot_evidence = snapshot(&request);
    let raw_evidence = raw(&request, &snapshot_evidence);
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut snapshot = SnapshotFake {
        calls: Rc::clone(&calls),
        evidence: snapshot_evidence,
        fail_at: FailAt::Never,
    };
    let mut graphify = GraphifyFake {
        calls: Rc::clone(&calls),
        evidence: raw_evidence,
        fail_at: FailAt::Never,
    };
    let mut memory = MemoryFake {
        calls: Rc::clone(&calls),
        fail_at: FailAt::Never,
        analysis: None,
        receipt: None,
    };
    let query = MemoryQuery::new(&request, "CodebaseMemoryPort", 10).expect("query");

    assert!(
        run_graph_memory(&request, &query, &mut snapshot, &mut graphify, &mut memory,).is_err()
    );
    assert_eq!(*calls.borrow(), vec!["snapshot", "graphify"]);
}

struct StatusOnlyFake {
    calls: Vec<&'static str>,
    receipt: GraphMemoryReceipt,
}

impl CodebaseMemoryPort for StatusOnlyFake {
    fn persist_analysis(
        &mut self,
        _analysis: &NormalizedGraphAnalysis,
    ) -> GraphMemoryPortResult<GraphMemoryPersistenceEvidence> {
        self.calls.push("persist");
        panic!("status must not persist")
    }

    fn retrieve(
        &mut self,
        _persistence: &GraphMemoryPersistenceEvidence,
        _plan: MemoryRetrievalPlan,
    ) -> GraphMemoryPortResult<GraphMemoryReceipt> {
        self.calls.push("retrieve");
        panic!("status must not retrieve")
    }

    fn load_receipt(
        &mut self,
        _request: &GraphMemoryRunRequest,
    ) -> GraphMemoryPortResult<GraphMemoryReceipt> {
        self.calls.push("receipt");
        Ok(self.receipt.clone())
    }
}

#[test]
fn graph_memory_status_loads_only_receipt_and_rejects_cross_binding() {
    let (_, completed) = run(FailAt::Never);
    let receipt = completed.expect("completed receipt");
    let exact_request = receipt.persistence().request().clone();
    let mut exact = StatusOnlyFake {
        calls: vec![],
        receipt: receipt.clone(),
    };

    assert_eq!(
        graph_memory_status(&exact_request, &mut exact).expect("exact status"),
        receipt
    );
    assert_eq!(exact.calls, vec!["receipt"]);

    let different_request = request("different-query");
    let mut cross_bound = StatusOnlyFake {
        calls: vec![],
        receipt,
    };
    assert!(graph_memory_status(&different_request, &mut cross_bound).is_err());
    assert_eq!(cross_bound.calls, vec!["receipt"]);
}
