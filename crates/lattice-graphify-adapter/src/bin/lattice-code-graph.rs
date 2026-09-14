//! Control-owned analysis helper. Reads one committed source snapshot; no database writes.
use lattice_graphify_adapter::{
    ExactGitSnapshotMaterializer, GRAPHIFY_VERSION, GitSnapshotConfig, GraphOutputLimits,
    GraphifyRuntimeConfig, PinnedGraphifyAdapter, SnapshotBridge, SnapshotLimits,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    io::{self, Write},
    path::PathBuf,
    time::Duration,
};

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.len() != 6 {
        return Err("CODE_GRAPH_ARGUMENTS_REJECTED".into());
    }
    let [repository, commit, git, runtime, wsl, work]: [std::ffi::OsString; 6] = args
        .try_into()
        .map_err(|_| "CODE_GRAPH_ARGUMENTS_REJECTED")?;
    let commit = commit.to_str().ok_or("CODE_GRAPH_COMMIT_REJECTED")?;
    let work = PathBuf::from(work);
    let git = PathBuf::from(git);
    let git_sha: String = Sha256::digest(fs::read(&git)?)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let snapshot = ExactGitSnapshotMaterializer::new(GitSnapshotConfig::new(
        git,
        git_sha,
        PathBuf::from(repository),
        work.join("snapshots"),
        SnapshotLimits::default(),
    )?)
    .materialize(commit)?;
    let mut adapter = PinnedGraphifyAdapter::new(
        GraphifyRuntimeConfig::new(
            PathBuf::from(wsl),
            PathBuf::from(runtime),
            work.join("staging"),
            Duration::from_secs(300),
            GraphOutputLimits::default(),
        )?,
        SnapshotBridge::new(),
    );
    let analysis = adapter.analyze_for_display(&snapshot)?;
    let graph = analysis.graph();
    let packet = json!({
        "schema_version": "lattice.control.code-graph.v1", "source": "GRAPHIFY",
        "authority": "DERIVED", "version": GRAPHIFY_VERSION,
        "commit": snapshot.commit_id(), "manifest_digest": snapshot.manifest_sha256(),
        "graph_digest": graph.raw_graph_sha256(), "evidence_digest": analysis.evidence_sha256(),
        "files": snapshot.sources().len(),
        "coverage_warnings": analysis.coverage_warnings().iter().map(|(code, count)|
            json!({"code":code,"files":count})).collect::<Vec<_>>(),
        "coverage": {"raw_nodes":graph.raw_node_count(), "raw_edges":graph.raw_edge_count(),
            "excluded_nodes":graph.dropped_non_code_nodes()+graph.dropped_source_less_nodes(),
            "excluded_edges":graph.dropped_unbound_edges()},
        "nodes": graph.nodes().iter().map(|n|json!({"id":n.id(),"label":n.label(),
            "file":n.source_file(),"location":n.source_location(),"kind":n.kind()})).collect::<Vec<_>>(),
        "edges": graph.edges().iter().map(|e|json!({"source":e.source(),"target":e.target(),
            "relation":e.relation(),"confidence":e.confidence().as_str(),
            "file":e.source_file(),"location":e.source_location()})).collect::<Vec<_>>()
    });
    serde_json::to_writer(io::stdout().lock(), &packet)?;
    io::stdout().flush()?;
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
