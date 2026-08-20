use std::collections::BTreeSet;

use lattice_codebase_memory::{
    CodebaseMemoryError, digest_query_text, normalize_analysis, plan_retrieval,
};
use lattice_contracts::{
    AttemptId, CONTRACT_VERSION, CodeSnapshotEvidence, ContentDigest,
    GRAPH_MEMORY_RETRIEVAL_ALGORITHM, GitObjectId, GraphConfidence, GraphMemoryContractError,
    GraphMemoryRecord, GraphMemoryRunRequest, GraphSourceProvenance, GraphifyIdentity,
    GraphifyRawEvidence, GraphifyRawNode, Invocation, MemoryQuery, MemoryRetrievalDisposition,
    NormalizedGraphAnalysis, ProjectId, ProjectSnapshotId, RequestId, TaskId, TrackedSource,
};

const RETRIEVAL_LIMIT: u16 = 3;
const BASE_PROJECT: &str = "benchmark-project";
const BASE_SNAPSHOT: &str = "benchmark-snapshot-a";
const BASE_COMMIT_BYTE: char = '1';
const STABLE_RESULT_SET_DIGEST: &str =
    "17ee9a2ff916ec56f4a742b7a2b67c4eef379bd11ee31e1e3b5e04d4a89c66eb";

#[derive(Clone, Copy)]
struct Binding<'a> {
    project: &'a str,
    snapshot: &'a str,
    commit_byte: char,
    reverse_nodes: bool,
}

impl Default for Binding<'static> {
    fn default() -> Self {
        Self {
            project: BASE_PROJECT,
            snapshot: BASE_SNAPSHOT,
            commit_byte: BASE_COMMIT_BYTE,
            reverse_nodes: false,
        }
    }
}

fn digest(byte: char) -> ContentDigest {
    ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("valid digest")
}

fn source(path: &str, digest_byte: char) -> TrackedSource {
    TrackedSource::new(path, digest(digest_byte)).expect("tracked benchmark source")
}

#[allow(clippy::too_many_lines)]
fn benchmark_fixture(
    query: &str,
    binding: Binding<'_>,
) -> (
    GraphMemoryRunRequest,
    CodeSnapshotEvidence,
    GraphifyRawEvidence,
) {
    let invocation = Invocation::new(
        CONTRACT_VERSION,
        RequestId::new("task043-benchmark-request").expect("request id"),
        TaskId::new("TASK-043").expect("task id"),
        AttemptId::new("attempt-1").expect("attempt id"),
        ProjectSnapshotId::new(binding.snapshot).expect("snapshot id"),
        digest('a'),
    )
    .expect("invocation");
    let request = GraphMemoryRunRequest::new(
        invocation,
        ProjectId::new(binding.project).expect("project id"),
        GitObjectId::new(binding.commit_byte.to_string().repeat(40)).expect("commit id"),
        digest_query_text(query).expect("query digest"),
        digest('c'),
        RETRIEVAL_LIMIT,
    )
    .expect("graph-memory request");

    let lib = source("crates/lattice-codebase-memory/src/lib.rs", '1');
    let retrieval_test = source(
        "crates/lattice-codebase-memory/tests/normalization_and_retrieval.rs",
        '2',
    );
    let gateway = source("src/gateway.rs", '3');
    let chinese_only = source("src/memory/chinese_only.rs", '9');
    let english_only = source("src/memory/english_only.rs", 'd');
    let mixed = source("src/memory/mixed_retrieval.rs", '4');
    let chinese = source("src/memory/retrieval_zh.rs", '5');
    let tie_alpha = source("tests/fixtures/tie_alpha.rs", '6');
    let tie_beta = source("tests/fixtures/tie_beta.rs", '7');
    let error = source("tests/ui/e0425.rs", '8');
    let snapshot = CodeSnapshotEvidence::new(
        &request,
        GitObjectId::new(
            if binding.commit_byte == BASE_COMMIT_BYTE {
                'a'
            } else {
                'b'
            }
            .to_string()
            .repeat(40),
        )
        .expect("tree id"),
        vec![
            lib.clone(),
            retrieval_test.clone(),
            gateway.clone(),
            chinese_only.clone(),
            english_only.clone(),
            mixed.clone(),
            chinese.clone(),
            tie_alpha.clone(),
            tie_beta.clone(),
            error.clone(),
        ],
        if binding.commit_byte == BASE_COMMIT_BYTE {
            digest('9')
        } else {
            digest('e')
        },
        digest('f'),
    )
    .expect("snapshot evidence");

    let mut nodes = vec![
        GraphifyRawNode::new(
            "zh-retrieval-flow",
            "記憶體查詢流程",
            "函式",
            GraphSourceProvenance::new(&chinese, Some(10), Some(24)).expect("provenance"),
            GraphConfidence::Extracted,
        )
        .expect("Chinese retrieval node"),
        GraphifyRawNode::new(
            "mixed-retrieval-pipeline",
            "記憶體 Retrieval Pipeline",
            "module",
            GraphSourceProvenance::new(&mixed, Some(1), Some(18)).expect("provenance"),
            GraphConfidence::Extracted,
        )
        .expect("mixed retrieval node"),
        GraphifyRawNode::new(
            "chinese-memory-only",
            "記憶體",
            "contrast",
            GraphSourceProvenance::new(&chinese_only, Some(1), Some(4)).expect("provenance"),
            GraphConfidence::Extracted,
        )
        .expect("Chinese-only contrast node"),
        GraphifyRawNode::new(
            "english-retrieval-only",
            "retrieval",
            "contrast",
            GraphSourceProvenance::new(&english_only, Some(1), Some(4)).expect("provenance"),
            GraphConfidence::Extracted,
        )
        .expect("English-only contrast node"),
        GraphifyRawNode::new(
            "rust-symbol-plan-retrieval",
            "plan_retrieval",
            "function",
            GraphSourceProvenance::new(&lib, Some(202), Some(248)).expect("provenance"),
            GraphConfidence::Extracted,
        )
        .expect("Rust symbol node"),
        GraphifyRawNode::new(
            "error-e0425",
            "E0425 unresolved name",
            "compiler_error",
            GraphSourceProvenance::new(&error, Some(1), Some(6)).expect("provenance"),
            GraphConfidence::Extracted,
        )
        .expect("error-code node"),
        GraphifyRawNode::new(
            "exact-filename-test",
            "retrieval_prioritizes_exact_identifier",
            "test",
            GraphSourceProvenance::new(&retrieval_test, Some(135), Some(150)).expect("provenance"),
            GraphConfidence::Extracted,
        )
        .expect("exact-filename node"),
        GraphifyRawNode::new(
            "stable-tie-alpha",
            "stable tie alpha",
            "benchmark",
            GraphSourceProvenance::new(&tie_alpha, Some(1), Some(2)).expect("provenance"),
            GraphConfidence::Extracted,
        )
        .expect("tie alpha node"),
        GraphifyRawNode::new(
            "stable-tie-beta",
            "stable tie beta",
            "benchmark",
            GraphSourceProvenance::new(&tie_beta, Some(1), Some(2)).expect("provenance"),
            GraphConfidence::Extracted,
        )
        .expect("tie beta node"),
        GraphifyRawNode::new(
            "irrelevant-gateway",
            "GatewayService",
            "struct",
            GraphSourceProvenance::new(&gateway, Some(1), Some(12)).expect("provenance"),
            GraphConfidence::Extracted,
        )
        .expect("irrelevant node"),
    ];
    if binding.reverse_nodes {
        nodes.reverse();
    }
    let raw = GraphifyRawEvidence::new(
        &request,
        &snapshot,
        GraphifyIdentity::task033(digest('3'), digest('4'), digest('5')).expect("identity"),
        nodes,
        vec![],
        digest('6'),
        digest('7'),
        digest('8'),
    )
    .expect("raw graph evidence");
    (request, snapshot, raw)
}

fn analysis_for(
    query: &str,
    binding: Binding<'_>,
) -> (GraphMemoryRunRequest, NormalizedGraphAnalysis) {
    let (request, snapshot, raw) = benchmark_fixture(query, binding);
    let analysis = normalize_analysis(&request, &snapshot, &raw).expect("normalized analysis");
    (request, analysis)
}

fn fixture_record<'a>(
    analysis: &'a NormalizedGraphAnalysis,
    fixture_id: &str,
) -> &'a GraphMemoryRecord {
    let expected_subject = match fixture_id {
        "zh_retrieval_flow" => "記憶體查詢流程",
        "mixed_retrieval_pipeline" => "記憶體 Retrieval Pipeline",
        "chinese_memory_only" => "記憶體",
        "english_retrieval_only" => "retrieval",
        "rust_symbol_plan_retrieval" => "plan_retrieval",
        "error_e0425" => "E0425 unresolved name",
        "exact_filename_test" => "retrieval_prioritizes_exact_identifier",
        "stable_tie_alpha" => "stable tie alpha",
        "stable_tie_beta" => "stable tie beta",
        "irrelevant_gateway" => "GatewayService",
        unknown => panic!("unknown benchmark fixture id: {unknown}"),
    };
    analysis
        .records()
        .iter()
        .find(|record| record.subject() == expected_subject)
        .expect("fixture record must exist")
}

fn assert_cross_binding(error: &CodebaseMemoryError) {
    assert_eq!(
        error,
        &CodebaseMemoryError::Contract(GraphMemoryContractError::CrossBinding {
            field: "memory_retrieval_query",
        })
    );
}

#[test]
fn retrieval_quality_matrix_meets_locked_hit_at_one_and_no_answer_thresholds() {
    let positive_cases = [
        (
            "Traditional Chinese",
            "記憶體查詢 查詢流程",
            "zh_retrieval_flow",
        ),
        (
            "mixed language",
            "retrieval 記憶體",
            "mixed_retrieval_pipeline",
        ),
        (
            "Rust symbol",
            "plan_retrieval",
            "rust_symbol_plan_retrieval",
        ),
        (
            "Rust path",
            "crates/lattice-codebase-memory/src/lib.rs",
            "rust_symbol_plan_retrieval",
        ),
        ("error code", "E0425", "error_e0425"),
        (
            "exact filename",
            "normalization_and_retrieval.rs",
            "exact_filename_test",
        ),
    ];
    let mut hit_at_one = 0_usize;
    let mut reciprocal_rank_sixths = 0_u32;

    for (case_name, text, expected_fixture_id) in positive_cases {
        let (request, analysis) = analysis_for(text, Binding::default());
        let query = MemoryQuery::new(&request, text, RETRIEVAL_LIMIT).expect("query");
        let plan = plan_retrieval(&analysis, &query).expect("retrieval plan");
        let expected_id = fixture_record(&analysis, expected_fixture_id).record_id();
        let rank = plan
            .results()
            .iter()
            .position(|result| result.record_id() == expected_id)
            .map_or(0, |index| index + 1);

        assert_eq!(
            plan.disposition(),
            MemoryRetrievalDisposition::Results,
            "{case_name} must return results"
        );
        assert_eq!(rank, 1, "{case_name} must return the expected ID at rank 1");
        hit_at_one += usize::from(rank == 1);
        reciprocal_rank_sixths += match rank {
            1 => 6,
            2 => 3,
            3 => 2,
            _ => 0,
        };
    }

    assert_eq!(hit_at_one, 6, "locked Hit@1 threshold is 6/6");
    assert_eq!(reciprocal_rank_sixths, 36, "locked MRR is 36/36 = 1.0");

    for (text, expected_fixture_id) in [
        ("retrieval", "english_retrieval_only"),
        ("記憶體", "chinese_memory_only"),
    ] {
        let (request, analysis) = analysis_for(text, Binding::default());
        let query = MemoryQuery::new(&request, text, RETRIEVAL_LIMIT).expect("ablation query");
        let plan = plan_retrieval(&analysis, &query).expect("ablation plan");
        assert_eq!(
            plan.results()[0].record_id(),
            fixture_record(&analysis, expected_fixture_id).record_id(),
            "each single-language ablation must select its contrast record"
        );
    }

    let mut no_answer_count = 0_usize;
    for text in ["支付 發票", "HERMES_RUN_FAILED_QUOTA"] {
        let (request, analysis) = analysis_for(text, Binding::default());
        let query = MemoryQuery::new(&request, text, RETRIEVAL_LIMIT).expect("query");
        let plan = plan_retrieval(&analysis, &query).expect("retrieval plan");
        assert_eq!(plan.disposition(), MemoryRetrievalDisposition::NoAnswer);
        assert!(plan.results().is_empty());
        no_answer_count += 1;
    }
    assert_eq!(no_answer_count, 2, "locked no-answer threshold is 2/2");
    println!(
        "TASK043_RETRIEVAL_QUALITY hit_at_one={hit_at_one}/6 mrr=1.0 no_answer={no_answer_count}/2"
    );
}

#[test]
fn tie_order_and_digests_are_stable_across_input_order_and_repeated_runs() {
    let text = "stable tie";
    let (request, expected_analysis) = analysis_for(text, Binding::default());
    let query = MemoryQuery::new(&request, text, RETRIEVAL_LIMIT).expect("query");
    let expected_plan = plan_retrieval(&expected_analysis, &query).expect("expected plan");
    assert_eq!(expected_plan.algorithm(), GRAPH_MEMORY_RETRIEVAL_ALGORITHM);
    assert_eq!(
        expected_plan.result_set_digest().as_str(),
        STABLE_RESULT_SET_DIGEST
    );
    let mut expected_tie_ids = [
        fixture_record(&expected_analysis, "stable_tie_alpha")
            .record_id()
            .clone(),
        fixture_record(&expected_analysis, "stable_tie_beta")
            .record_id()
            .clone(),
    ];
    expected_tie_ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));

    assert_eq!(expected_plan.results().len(), 2);
    assert_eq!(expected_plan.results()[0].record_id(), &expected_tie_ids[0]);
    assert_eq!(expected_plan.results()[1].record_id(), &expected_tie_ids[1]);
    assert_eq!(
        expected_plan.results()[0].score(),
        expected_plan.results()[1].score()
    );

    for iteration in 0..32 {
        let binding = Binding {
            reverse_nodes: iteration % 2 == 0,
            ..Binding::default()
        };
        let (actual_request, actual_analysis) = analysis_for(text, binding);
        let actual_query = MemoryQuery::new(&actual_request, text, RETRIEVAL_LIMIT).expect("query");
        let actual_plan = plan_retrieval(&actual_analysis, &actual_query).expect("actual plan");

        assert_eq!(actual_analysis, expected_analysis, "iteration {iteration}");
        assert_eq!(actual_plan, expected_plan, "iteration {iteration}");
        assert_eq!(
            actual_plan.result_set_digest(),
            expected_plan.result_set_digest(),
            "iteration {iteration}"
        );
    }
    println!(
        "TASK043_RETRIEVAL_DETERMINISM stable_runs=32/32 tie_order=2/2 result_set_digest={}",
        expected_plan.result_set_digest().as_str()
    );
}

#[test]
fn project_snapshot_and_changed_commit_are_exactly_isolated() {
    let text = "plan_retrieval";
    let (base_request, base_analysis) = analysis_for(text, Binding::default());
    let base_query = MemoryQuery::new(&base_request, text, RETRIEVAL_LIMIT).expect("base query");
    let base_plan = plan_retrieval(&base_analysis, &base_query).expect("base plan");

    let (_, other_project) = analysis_for(
        text,
        Binding {
            project: "other-project",
            ..Binding::default()
        },
    );
    assert_cross_binding(
        &plan_retrieval(&other_project, &base_query).expect_err("cross-project query must reject"),
    );

    let (_, other_snapshot) = analysis_for(
        text,
        Binding {
            snapshot: "benchmark-snapshot-b",
            ..Binding::default()
        },
    );
    assert_cross_binding(
        &plan_retrieval(&other_snapshot, &base_query)
            .expect_err("cross-snapshot query must reject"),
    );

    let changed_binding = Binding {
        commit_byte: '2',
        ..Binding::default()
    };
    let (changed_request, changed_analysis) = analysis_for(text, changed_binding);
    assert_cross_binding(
        &plan_retrieval(&changed_analysis, &base_query)
            .expect_err("changed-commit query must reject"),
    );

    let changed_query =
        MemoryQuery::new(&changed_request, text, RETRIEVAL_LIMIT).expect("changed query");
    let changed_plan =
        plan_retrieval(&changed_analysis, &changed_query).expect("changed exact-bound plan");
    assert_ne!(
        base_analysis.analysis_digest(),
        changed_analysis.analysis_digest()
    );
    assert_ne!(
        base_plan.result_set_digest(),
        changed_plan.result_set_digest()
    );

    let base_ids = base_analysis
        .records()
        .iter()
        .map(|record| record.record_id().as_str())
        .collect::<BTreeSet<_>>();
    let changed_ids = changed_analysis
        .records()
        .iter()
        .map(|record| record.record_id().as_str())
        .collect::<BTreeSet<_>>();
    assert_ne!(base_ids, changed_ids);
    assert!(base_ids.is_disjoint(&changed_ids));
    assert_ne!(base_plan.results(), changed_plan.results());
    println!("TASK043_RETRIEVAL_ISOLATION project=1/1 snapshot=1/1 changed_commit=1/1");
}
