use lattice_contracts::{
    CodeSnapshotEvidence, ContentDigest, GitObjectId, GraphConfidence as ContractConfidence,
    GraphMemoryRunRequest, GraphSourceProvenance, GraphifyIdentity, GraphifyRawEdge,
    GraphifyRawEvidence, GraphifyRawNode, TrackedSource,
};
use lattice_ports::{
    CodeSnapshotPort, GraphMemoryFailureCertainty, GraphMemoryPortError, GraphMemoryPortResult,
    GraphMemoryStage, GraphifyAnalysisPort, PortErrorKind,
};

use crate::error::{GraphifyAdapterError, GraphifyAdapterErrorKind};
use crate::graph::GraphConfidence;
use crate::identity::GRAPHIFY_WSL_RUNTIME_MANIFEST_SHA256;
use crate::process::{GraphifyAnalysis, PinnedGraphifyAdapter};
use crate::snapshot::{ExactGitSnapshotMaterializer, MaterializedSnapshot, framed_digest};

impl CodeSnapshotPort for ExactGitSnapshotMaterializer {
    fn materialize_snapshot(
        &mut self,
        request: &GraphMemoryRunRequest,
    ) -> GraphMemoryPortResult<CodeSnapshotEvidence> {
        let materialized = self
            .materialize(request.commit_id().as_str())
            .map_err(|failure| map_snapshot_error(&failure))?;
        let evidence = snapshot_evidence(request, &materialized)?;
        self.bridge()
            .insert(snapshot_key_from_local(&materialized), materialized)
            .map_err(|failure| map_snapshot_error(&failure))?;
        Ok(evidence)
    }
}

impl GraphifyAnalysisPort for PinnedGraphifyAdapter {
    fn analyze(
        &mut self,
        request: &GraphMemoryRunRequest,
        snapshot: &CodeSnapshotEvidence,
    ) -> GraphMemoryPortResult<GraphifyRawEvidence> {
        if snapshot.request() != request {
            return Err(graph_port_error(
                PortErrorKind::Malformed,
                GraphMemoryFailureCertainty::Known,
                "GRAPHIFY_REQUEST_SNAPSHOT_BINDING_REJECTED",
            ));
        }
        let local = self
            .snapshot_for_key(&snapshot_key_from_contract(snapshot))
            .map_err(|failure| map_graph_error(&failure))?;
        verify_contract_matches_local(snapshot, &local)?;
        let analysis = self
            .analyze_materialized(&local)
            .map_err(|failure| map_graph_error(&failure))?;
        raw_evidence(request, snapshot, &analysis)
    }
}

fn snapshot_evidence(
    request: &GraphMemoryRunRequest,
    local: &MaterializedSnapshot,
) -> GraphMemoryPortResult<CodeSnapshotEvidence> {
    let tree_id = GitObjectId::new(local.tree_id().to_owned())
        .map_err(|_| snapshot_contract_error("GRAPHIFY_SNAPSHOT_TREE_CONTRACT_REJECTED"))?;
    let sources = local
        .sources()
        .iter()
        .map(|source| {
            let digest = digest(source.content_sha256(), "GRAPHIFY_SOURCE_DIGEST_REJECTED")?;
            TrackedSource::new(source.relative_path(), digest)
                .map_err(|_| snapshot_contract_error("GRAPHIFY_SOURCE_CONTRACT_REJECTED"))
        })
        .collect::<GraphMemoryPortResult<Vec<_>>>()?;
    CodeSnapshotEvidence::new(
        request,
        tree_id,
        sources,
        digest(local.manifest_sha256(), "GRAPHIFY_MANIFEST_DIGEST_REJECTED")?,
        digest(
            local.exclusion_sha256(),
            "GRAPHIFY_EXCLUSION_DIGEST_REJECTED",
        )?,
    )
    .map_err(|_| snapshot_contract_error("GRAPHIFY_SNAPSHOT_EVIDENCE_REJECTED"))
}

fn raw_evidence(
    request: &GraphMemoryRunRequest,
    snapshot: &CodeSnapshotEvidence,
    analysis: &GraphifyAnalysis,
) -> GraphMemoryPortResult<GraphifyRawEvidence> {
    if analysis.payload_manifest_sha256() != Some(GRAPHIFY_WSL_RUNTIME_MANIFEST_SHA256) {
        return Err(graph_port_error(
            PortErrorKind::Denied,
            GraphMemoryFailureCertainty::Known,
            "GRAPHIFY_OFFICIAL_IDENTITY_WITHOUT_VERIFIED_PAYLOAD",
        ));
    }
    let identity = GraphifyIdentity::task033(
        digest(
            analysis.executable_sha256(),
            "GRAPHIFY_EXECUTABLE_DIGEST_REJECTED",
        )?,
        digest(analysis.help_sha256(), "GRAPHIFY_HELP_DIGEST_REJECTED")?,
        digest(
            analysis.capability_sha256(),
            "GRAPHIFY_CAPABILITY_DIGEST_REJECTED",
        )?,
    )
    .map_err(|_| graph_contract_error("GRAPHIFY_IDENTITY_CONTRACT_REJECTED"))?;
    let nodes = analysis
        .graph()
        .nodes()
        .iter()
        .map(|node| {
            let provenance = provenance(snapshot, node.source_file(), node.source_location())?;
            GraphifyRawNode::new(
                node.id(),
                node.label(),
                node.kind(),
                provenance,
                ContractConfidence::Extracted,
            )
            .map_err(|_| graph_contract_error("GRAPHIFY_NODE_CONTRACT_REJECTED"))
        })
        .collect::<GraphMemoryPortResult<Vec<_>>>()?;
    let edges = analysis
        .graph()
        .edges()
        .iter()
        .map(|edge| {
            let provenance = provenance(snapshot, edge.source_file(), edge.source_location())?;
            let edge_id = format!(
                "edge-{}",
                framed_digest(&[
                    edge.source().as_bytes(),
                    edge.target().as_bytes(),
                    edge.relation().as_bytes(),
                    edge.confidence().as_str().as_bytes(),
                    edge.source_file().as_bytes(),
                    edge.source_location().as_bytes(),
                ])
            );
            GraphifyRawEdge::new(
                edge_id,
                edge.source(),
                edge.target(),
                edge.relation(),
                provenance,
                contract_confidence(edge.confidence()),
            )
            .map_err(|_| graph_contract_error("GRAPHIFY_EDGE_CONTRACT_REJECTED"))
        })
        .collect::<GraphMemoryPortResult<Vec<_>>>()?;
    GraphifyRawEvidence::new(
        request,
        snapshot,
        identity,
        nodes,
        edges,
        digest(
            analysis.graph().raw_graph_sha256(),
            "GRAPHIFY_ARTIFACT_DIGEST_REJECTED",
        )?,
        digest(
            analysis.raw_process_sha256(),
            "GRAPHIFY_PROCESS_DIGEST_REJECTED",
        )?,
        digest(
            analysis.evidence_sha256(),
            "GRAPHIFY_EVIDENCE_DIGEST_REJECTED",
        )?,
    )
    .map_err(|_| graph_contract_error("GRAPHIFY_RAW_EVIDENCE_REJECTED"))
}

fn provenance(
    snapshot: &CodeSnapshotEvidence,
    relative_path: &str,
    source_location: &str,
) -> GraphMemoryPortResult<GraphSourceProvenance> {
    let source = snapshot
        .sources()
        .iter()
        .find(|source| source.relative_path() == relative_path)
        .ok_or_else(|| graph_contract_error("GRAPHIFY_PROVENANCE_SOURCE_MISSING"))?;
    let (line_start, line_end) = parse_line_range(source_location)?;
    GraphSourceProvenance::new(source, line_start, line_end)
        .map_err(|_| graph_contract_error("GRAPHIFY_PROVENANCE_CONTRACT_REJECTED"))
}

fn parse_line_range(location: &str) -> GraphMemoryPortResult<(Option<u32>, Option<u32>)> {
    let Some(body) = location.strip_prefix('L') else {
        return Err(graph_contract_error("GRAPHIFY_SOURCE_LOCATION_REJECTED"));
    };
    let (start, end) = if let Some((start, end)) = body.split_once("-L") {
        (start, end)
    } else if let Some((start, end)) = body.split_once('-') {
        (start, end)
    } else {
        (body, body)
    };
    let start = start
        .parse::<u32>()
        .map_err(|_| graph_contract_error("GRAPHIFY_SOURCE_LOCATION_REJECTED"))?;
    let end = end
        .parse::<u32>()
        .map_err(|_| graph_contract_error("GRAPHIFY_SOURCE_LOCATION_REJECTED"))?;
    if start == 0 || end < start {
        return Err(graph_contract_error("GRAPHIFY_SOURCE_LOCATION_REJECTED"));
    }
    Ok((Some(start), Some(end)))
}

fn verify_contract_matches_local(
    contract: &CodeSnapshotEvidence,
    local: &MaterializedSnapshot,
) -> GraphMemoryPortResult<()> {
    if contract.commit_id().as_str() != local.commit_id()
        || contract.tree_id().as_str() != local.tree_id()
        || contract.manifest_digest().as_str() != local.manifest_sha256()
        || contract.exclusion_digest().as_str() != local.exclusion_sha256()
        || contract.sources().len() != local.sources().len()
        || contract
            .sources()
            .iter()
            .zip(local.sources())
            .any(|(contract_source, local_source)| {
                contract_source.relative_path() != local_source.relative_path()
                    || contract_source.content_digest().as_str() != local_source.content_sha256()
            })
    {
        return Err(graph_port_error(
            PortErrorKind::Malformed,
            GraphMemoryFailureCertainty::Known,
            "GRAPHIFY_LOCAL_SNAPSHOT_BINDING_REJECTED",
        ));
    }
    Ok(())
}

fn snapshot_key_from_local(snapshot: &MaterializedSnapshot) -> String {
    framed_digest(&[
        snapshot.commit_id().as_bytes(),
        snapshot.tree_id().as_bytes(),
        snapshot.manifest_sha256().as_bytes(),
        snapshot.exclusion_sha256().as_bytes(),
    ])
}

fn snapshot_key_from_contract(snapshot: &CodeSnapshotEvidence) -> String {
    framed_digest(&[
        snapshot.commit_id().as_str().as_bytes(),
        snapshot.tree_id().as_str().as_bytes(),
        snapshot.manifest_digest().as_str().as_bytes(),
        snapshot.exclusion_digest().as_str().as_bytes(),
    ])
}

fn contract_confidence(confidence: GraphConfidence) -> ContractConfidence {
    match confidence {
        GraphConfidence::Extracted => ContractConfidence::Extracted,
        GraphConfidence::Inferred => ContractConfidence::Inferred,
        GraphConfidence::Ambiguous => ContractConfidence::Ambiguous,
    }
}

fn digest(value: &str, code: &'static str) -> GraphMemoryPortResult<ContentDigest> {
    ContentDigest::from_sha256(value.to_owned()).map_err(|_| graph_contract_error(code))
}

fn map_snapshot_error(failure: &GraphifyAdapterError) -> GraphMemoryPortError {
    GraphMemoryPortError::new(
        GraphMemoryStage::Snapshot,
        port_kind(failure.kind()),
        certainty(failure.kind()),
        failure.code(),
    )
}

fn map_graph_error(failure: &GraphifyAdapterError) -> GraphMemoryPortError {
    GraphMemoryPortError::new(
        GraphMemoryStage::Graphify,
        port_kind(failure.kind()),
        certainty(failure.kind()),
        failure.code(),
    )
}

fn port_kind(kind: GraphifyAdapterErrorKind) -> PortErrorKind {
    match kind {
        GraphifyAdapterErrorKind::Configuration => PortErrorKind::Denied,
        GraphifyAdapterErrorKind::GitIdentity | GraphifyAdapterErrorKind::GraphifyIdentity => {
            PortErrorKind::VersionMismatch
        }
        GraphifyAdapterErrorKind::Spawn
        | GraphifyAdapterErrorKind::SnapshotIo
        | GraphifyAdapterErrorKind::MissingOutput
        | GraphifyAdapterErrorKind::NonZeroExit => PortErrorKind::Unavailable,
        GraphifyAdapterErrorKind::Timeout => PortErrorKind::Timeout,
        GraphifyAdapterErrorKind::TeardownAmbiguous => PortErrorKind::Ambiguous,
        GraphifyAdapterErrorKind::UnsafeSnapshot | GraphifyAdapterErrorKind::ForeignSource => {
            PortErrorKind::Denied
        }
        GraphifyAdapterErrorKind::GitObject
        | GraphifyAdapterErrorKind::SnapshotLimit
        | GraphifyAdapterErrorKind::SnapshotChanged
        | GraphifyAdapterErrorKind::OutputLimit
        | GraphifyAdapterErrorKind::MalformedOutput
        | GraphifyAdapterErrorKind::PartialOutput
        | GraphifyAdapterErrorKind::EmptyAnalysis => PortErrorKind::Malformed,
    }
}

const fn certainty(kind: GraphifyAdapterErrorKind) -> GraphMemoryFailureCertainty {
    if matches!(kind, GraphifyAdapterErrorKind::TeardownAmbiguous) {
        GraphMemoryFailureCertainty::Ambiguous
    } else {
        GraphMemoryFailureCertainty::Known
    }
}

fn snapshot_contract_error(code: &'static str) -> GraphMemoryPortError {
    GraphMemoryPortError::new(
        GraphMemoryStage::Snapshot,
        PortErrorKind::Malformed,
        GraphMemoryFailureCertainty::Known,
        code,
    )
}

fn graph_contract_error(code: &'static str) -> GraphMemoryPortError {
    graph_port_error(
        PortErrorKind::Malformed,
        GraphMemoryFailureCertainty::Known,
        code,
    )
}

fn graph_port_error(
    kind: PortErrorKind,
    certainty: GraphMemoryFailureCertainty,
    code: &'static str,
) -> GraphMemoryPortError {
    GraphMemoryPortError::new(GraphMemoryStage::Graphify, kind, certainty, code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_error_mapping_preserves_timeout_and_teardown_certainty() {
        let timeout = map_graph_error(&GraphifyAdapterError::new(
            GraphifyAdapterErrorKind::Timeout,
            "GRAPHIFY_TIMEOUT_REAP_CONFIRMED",
        ));
        assert_eq!(timeout.kind(), PortErrorKind::Timeout);
        assert_eq!(timeout.certainty(), GraphMemoryFailureCertainty::Known);

        let ambiguous = map_graph_error(&GraphifyAdapterError::new(
            GraphifyAdapterErrorKind::TeardownAmbiguous,
            "GRAPHIFY_TIMEOUT_REAP_UNKNOWN",
        ));
        assert_eq!(ambiguous.kind(), PortErrorKind::Ambiguous);
        assert_eq!(
            ambiguous.certainty(),
            GraphMemoryFailureCertainty::Ambiguous
        );
    }

    #[test]
    fn line_range_accepts_real_graphify_shapes_only() {
        assert_eq!(parse_line_range("L7").expect("single"), (Some(7), Some(7)));
        assert_eq!(
            parse_line_range("L7-L9").expect("range"),
            (Some(7), Some(9))
        );
        assert!(parse_line_range("line 7").is_err());
        assert!(parse_line_range("L0").is_err());
    }
}
