use lattice_orchestrator::{
    ArchiveDisposition, CompletionReport, CompletionState, CoordinationWorkState, EvidenceRecord,
    EvidenceState, WorkItem,
};
use lattice_runtime::coordination::evaluate_coordination_round;

fn item(id: &str, dependencies: &[&str], resource: &str) -> WorkItem {
    WorkItem::new(
        id,
        CoordinationWorkState::Ready,
        dependencies.iter().copied(),
        [resource],
    )
}

fn completion(id: &str) -> CompletionReport {
    CompletionReport::new(
        id,
        CompletionState::Done,
        [EvidenceRecord::new(
            EvidenceState::Verified,
            Some(format!("test:{id}")),
        )],
    )
}

#[test]
fn normal_runtime_gate_recomputes_dispatch_and_terminal_archive_rounds() {
    let work = vec![
        item("prepare", &[], "workspace:prepare"),
        item("verify", &["prepare"], "workspace:verify"),
    ];

    let first = evaluate_coordination_round(&work, &[]);
    assert_eq!(first.dispatchable()[0].work_item_id(), "prepare");

    let next = evaluate_coordination_round(&work, &[completion("prepare")]);
    assert_eq!(next.dispatchable()[0].work_item_id(), "verify");
    assert_eq!(next.archive()[0].disposition(), ArchiveDisposition::Retain);

    let terminal =
        evaluate_coordination_round(&work, &[completion("prepare"), completion("verify")]);
    assert!(terminal.dispatchable().is_empty());
    assert!(
        terminal
            .archive()
            .iter()
            .all(|decision| decision.disposition() == ArchiveDisposition::Archive)
    );
}
