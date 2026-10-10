//! Isolated synthetic acceptance only; parent executable checks fixture marker.
use lattice_codebase_memory::{digest_query_text, normalize_analysis, plan_retrieval};
use lattice_contracts::*;
use lattice_ports::{CodebaseMemoryPort, HermesReflectionMemoryPort};
use lattice_postgres_codebase_memory::{
    ExtensionTarget, PostgresCodebaseMemory, verify_store_v8_compatibility,
};
use lattice_postgres_store::MigrationTarget;
use postgres::{Client, Config, NoTls};
use serde_json::json;
use std::path::Path;

fn digest(c: char) -> ContentDigest {
    ContentDigest::from_sha256(c.to_string().repeat(64)).unwrap()
}

// Runtime bootstrap already installed and verified this exact profile.
pub fn install(admin: &mut Client, target: &MigrationTarget, run: &str) {
    verify_store_v8_compatibility(
        admin,
        &ExtensionTarget::new(target.database_name(), run).unwrap(),
    )
    .unwrap();
}
pub fn run(
    port: u16,
    run: &str,
    password: &str,
    target: &MigrationTarget,
    root: &Path,
    mode: &str,
) {
    let client = Config::new()
        .host("127.0.0.1")
        .port(port)
        .dbname(target.database_name())
        .user("lattice_runtime_login")
        .password(password)
        .application_name("lattice-devos-task019")
        .options("-c role=lattice_runtime -c search_path=pg_catalog")
        .connect(NoTls)
        .unwrap();
    let mut memory = PostgresCodebaseMemory::new(
        client,
        ExtensionTarget::new(target.database_name(), run).unwrap(),
    )
    .unwrap();
    let mut result = Vec::new();
    for (label, directory, byte) in [("target", "project-a", '1'), ("survivor", "project-b", '2')] {
        if mode == "graph-verify-survivor" && label == "target" {
            continue;
        }
        let source_root = if label == "target" && mode == "graph-seed-source" {
            std::path::PathBuf::from(std::env::var("LATTICE_GRAPHIFY_SOURCE_ROOT").unwrap())
        } else {
            root.join(directory)
        };
        let configs = lattice_runtime::composition::project_purge_graph_configuration_digests(
            source_root.to_str().unwrap(),
        )
        .unwrap();
        let config = ContentDigest::from_sha256(configs[0].clone()).unwrap();
        let invocation = Invocation::new(
            CONTRACT_VERSION,
            RequestId::new(format!("graph-{label}")).unwrap(),
            TaskId::new("purge-graph").unwrap(),
            AttemptId::new(format!("attempt-{label}")).unwrap(),
            ProjectSnapshotId::new(format!("snapshot-{label}")).unwrap(),
            digest(byte),
        )
        .unwrap();
        // The actual legacy Runtime namespace is shared; only source config owns it.
        let commit = if label == "target" && mode == "graph-seed-source" {
            let output =
                std::process::Command::new(std::env::var("LATTICE_DELIVERY_GIT_EXE").unwrap())
                    .args(["rev-parse", "HEAD"])
                    .current_dir(&source_root)
                    .output()
                    .unwrap();
            assert!(output.status.success());
            String::from_utf8(output.stdout).unwrap().trim().to_owned()
        } else {
            "1".repeat(40)
        };
        let request = GraphMemoryRunRequest::new(
            invocation,
            ProjectId::new("task032-delivery").unwrap(),
            GitObjectId::new(commit).unwrap(),
            digest_query_text("synthetic graph").unwrap(),
            config,
            5,
        )
        .unwrap();
        if mode == "graph-verify-survivor" {
            let receipt = memory.load_receipt(&request).unwrap();
            let reflection = memory.load_reflection(&request).unwrap();
            result.push(json!({"source":label,"receipt":receipt.receipt_digest().as_str(),"reflection":reflection.receipt_digest().as_str()}));
            continue;
        }
        let source = TrackedSource::new("src/lib.rs", digest('d')).unwrap();
        let snapshot = CodeSnapshotEvidence::new(
            &request,
            GitObjectId::new("2".repeat(40)).unwrap(),
            vec![source.clone()],
            digest('e'),
            digest('f'),
        )
        .unwrap();
        let raw = GraphifyRawEvidence::new(
            &request,
            &snapshot,
            GraphifyIdentity::task033(digest('1'), digest('2'), digest('3')).unwrap(),
            (0..match mode {
                "graph-seed-large" => 55_000,
                "graph-seed-compat" => 8,
                _ => 1,
            })
                .map(|index| {
                    GraphifyRawNode::new(
                        format!("synthetic-node-{index}"),
                        if mode == "graph-seed-compat" {
                            format!(
                                "synthetic graph {}",
                                [
                                    "a",
                                    "A",
                                    "a0",
                                    "a00",
                                    "繁體",
                                    "é",
                                    "0.1",
                                    "18446744073709551615"
                                ][index]
                            )
                        } else {
                            format!("synthetic graph {index:05}")
                        },
                        "trait",
                        GraphSourceProvenance::new(&source, Some(1), Some(2)).unwrap(),
                        GraphConfidence::Extracted,
                    )
                    .unwrap()
                })
                .collect(),
            vec![],
            digest('4'),
            digest('5'),
            digest('6'),
        )
        .unwrap();
        let analysis = normalize_analysis(&request, &snapshot, &raw).unwrap();
        let query = MemoryQuery::new(&request, "synthetic graph", 5).unwrap();
        let plan = plan_retrieval(&analysis, &query).unwrap();
        let persisted = memory.persist_analysis(&analysis).unwrap();
        let receipt = memory.retrieve(&persisted, plan).unwrap();
        let content = HermesReflectionContent::new(
            "Synthetic historical reflection",
            vec![HermesReflectionFinding::new("Synthetic graph observation", digest('7')).unwrap()],
            vec!["Synthetic fixture only".into()],
        )
        .unwrap();
        let candidate = HermesReflectionCandidate::new(
            &request,
            &receipt,
            content,
            digest('8'),
            digest('9'),
            digest('b'),
        )
        .unwrap();
        let reflection = memory.persist_reflection(&candidate).unwrap();
        result.push(json!({"source":label,"receipt":receipt.receipt_digest().as_str(),"reflection":reflection.receipt_digest().as_str(),"configuration":request.configuration_digest().as_str()}));
    }
    println!("{}", json!({"status":"VERIFIED","records":result}));
}
