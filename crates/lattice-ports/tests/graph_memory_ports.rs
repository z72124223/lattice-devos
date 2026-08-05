use lattice_contracts::{
    CodeSnapshotEvidence, GraphMemoryPersistenceEvidence, GraphMemoryReceipt,
    GraphMemoryRunRequest, GraphifyRawEvidence, MemoryRetrievalPlan, NormalizedGraphAnalysis,
};
use lattice_ports::{
    CodeSnapshotPort, CodebaseMemoryPort, GraphMemoryFailureCertainty, GraphMemoryPortError,
    GraphMemoryPortResult, GraphMemoryStage, GraphifyAnalysisPort, PortErrorKind,
};

struct CompileOnlyPort;

impl CodeSnapshotPort for CompileOnlyPort {
    fn materialize_snapshot(
        &mut self,
        _request: &GraphMemoryRunRequest,
    ) -> GraphMemoryPortResult<CodeSnapshotEvidence> {
        Err(GraphMemoryPortError::new(
            GraphMemoryStage::Snapshot,
            PortErrorKind::Unavailable,
            GraphMemoryFailureCertainty::Known,
            "SNAPSHOT_UNAVAILABLE",
        ))
    }
}

impl GraphifyAnalysisPort for CompileOnlyPort {
    fn analyze(
        &mut self,
        _request: &GraphMemoryRunRequest,
        _snapshot: &CodeSnapshotEvidence,
    ) -> GraphMemoryPortResult<GraphifyRawEvidence> {
        Err(GraphMemoryPortError::new(
            GraphMemoryStage::Graphify,
            PortErrorKind::Timeout,
            GraphMemoryFailureCertainty::Ambiguous,
            "GRAPHIFY_TIMEOUT_REAP_UNKNOWN",
        ))
    }
}

impl CodebaseMemoryPort for CompileOnlyPort {
    fn persist_analysis(
        &mut self,
        _analysis: &NormalizedGraphAnalysis,
    ) -> GraphMemoryPortResult<GraphMemoryPersistenceEvidence> {
        Err(GraphMemoryPortError::new(
            GraphMemoryStage::Persistence,
            PortErrorKind::Unavailable,
            GraphMemoryFailureCertainty::Known,
            "MEMORY_UNAVAILABLE",
        ))
    }

    fn retrieve(
        &mut self,
        _persistence: &GraphMemoryPersistenceEvidence,
        _plan: MemoryRetrievalPlan,
    ) -> GraphMemoryPortResult<GraphMemoryReceipt> {
        Err(GraphMemoryPortError::new(
            GraphMemoryStage::Retrieval,
            PortErrorKind::Malformed,
            GraphMemoryFailureCertainty::Known,
            "MEMORY_RETRIEVAL_REJECTED",
        ))
    }

    fn load_receipt(
        &mut self,
        _request: &GraphMemoryRunRequest,
    ) -> GraphMemoryPortResult<GraphMemoryReceipt> {
        Err(GraphMemoryPortError::new(
            GraphMemoryStage::Receipt,
            PortErrorKind::Unavailable,
            GraphMemoryFailureCertainty::Known,
            "MEMORY_RECEIPT_UNAVAILABLE",
        ))
    }
}

#[test]
fn graph_memory_error_preserves_stage_and_ambiguity() {
    let error = GraphMemoryPortError::new(
        GraphMemoryStage::Graphify,
        PortErrorKind::Timeout,
        GraphMemoryFailureCertainty::Ambiguous,
        "GRAPHIFY_TIMEOUT_REAP_UNKNOWN",
    );
    assert_eq!(error.stage(), GraphMemoryStage::Graphify);
    assert_eq!(error.kind(), PortErrorKind::Timeout);
    assert_eq!(error.certainty(), GraphMemoryFailureCertainty::Ambiguous);
    assert_eq!(error.code(), "GRAPHIFY_TIMEOUT_REAP_UNKNOWN");

    let _: CompileOnlyPort = CompileOnlyPort;
}
