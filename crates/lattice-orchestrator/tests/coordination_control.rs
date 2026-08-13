use lattice_orchestrator::{
    ArchiveDisposition, CompletionReport, CompletionState, CoordinationBlocker,
    CoordinationWorkState, DispatchBoundary, DispatchCandidate, EvidenceRecord, EvidenceState,
    ProjectedCompletion, WorkItem, decide_coordination_round, project_work_item_status,
};

fn work(id: &str, dependencies: &[&str], resources: &[&str]) -> WorkItem {
    WorkItem::new(
        id,
        CoordinationWorkState::Ready,
        dependencies.iter().copied(),
        resources.iter().copied(),
    )
}

fn done(id: &str) -> CompletionReport {
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
fn projects_only_referenced_verified_done_evidence_as_complete() {
    let item = work("prepare", &[], &["workspace:prepare"]);
    let incomplete = CompletionReport::new(
        "prepare",
        CompletionState::Done,
        [EvidenceRecord::new(
            EvidenceState::Unknown,
            Some("test:prepare"),
        )],
    );

    let projection = project_work_item_status(&item, Some(&incomplete));

    assert_eq!(projection.completion(), ProjectedCompletion::NotComplete);
    assert_eq!(projection.verified_evidence(), 0);
    assert_eq!(projection.total_evidence(), 1);
    assert_eq!(
        projection.blockers(),
        &[CoordinationBlocker::EvidenceNotVerified {
            work_item_id: "prepare".into(),
            evidence_index: 0,
            status: EvidenceState::Unknown,
        }]
    );

    let complete = project_work_item_status(&item, Some(&done("prepare")));
    assert_eq!(complete.completion(), ProjectedCompletion::VerifiedDone);
    assert!(complete.blockers().is_empty());
}

#[test]
fn blocked_or_unknown_work_cannot_be_completed_or_archived_by_a_done_report() {
    for state in [
        CoordinationWorkState::Blocked,
        CoordinationWorkState::Unknown,
    ] {
        let item = WorkItem::new("unsafe", state, Vec::<&str>::new(), ["workspace:unsafe"]);
        let report = done("unsafe");

        let projection = project_work_item_status(&item, Some(&report));
        assert_eq!(projection.completion(), ProjectedCompletion::NotComplete);

        let round = decide_coordination_round(&[item], &[report]);
        assert!(round.dispatchable().is_empty());
        assert!(round.archive().is_empty());
    }
}

#[test]
fn dispatches_only_ready_dependency_complete_conflict_free_work() {
    let items = vec![
        work("prepare", &[], &["workspace:prepare"]),
        work("publish", &["prepare"], &["workspace:publish"]),
        WorkItem::new(
            "blocked",
            CoordinationWorkState::Blocked,
            Vec::<&str>::new(),
            ["workspace:blocked"],
        ),
        WorkItem::new(
            "unknown",
            CoordinationWorkState::Unknown,
            Vec::<&str>::new(),
            ["workspace:unknown"],
        ),
    ];

    let round = decide_coordination_round(&items, &[]);

    assert!(round.structural_blockers().is_empty());
    assert_eq!(round.dispatchable().len(), 1);
    assert_eq!(round.dispatchable()[0].work_item_id(), "prepare");
    assert_eq!(
        round.dispatchable()[0].boundary(),
        DispatchBoundary::GovernedLatticeExecutionOnly
    );
    assert!(round.blocked().iter().any(|entry| {
        entry.work_item_id() == "publish"
            && entry
                .blockers()
                .contains(&CoordinationBlocker::DependencyNotVerifiedDone {
                    work_item_id: "publish".into(),
                    dependency_id: "prepare".into(),
                })
    }));
    assert!(round.blocked().iter().any(|entry| {
        entry.work_item_id() == "blocked"
            && entry
                .blockers()
                .contains(&CoordinationBlocker::WorkBlocked {
                    work_item_id: "blocked".into(),
                })
    }));
    assert!(round.blocked().iter().any(|entry| {
        entry.work_item_id() == "unknown"
            && entry
                .blockers()
                .contains(&CoordinationBlocker::WorkStateUnknown {
                    work_item_id: "unknown".into(),
                })
    }));
}

#[test]
fn completion_registration_opens_next_round_and_controls_archive_decisions() {
    let items = vec![
        work("prepare", &[], &["workspace:prepare"]),
        work("publish", &["prepare"], &["workspace:publish"]),
    ];

    let second_round = decide_coordination_round(&items, &[done("prepare")]);
    assert_eq!(
        second_round
            .dispatchable()
            .iter()
            .map(DispatchCandidate::work_item_id)
            .collect::<Vec<_>>(),
        vec!["publish"]
    );
    assert_eq!(second_round.archive().len(), 1);
    assert_eq!(second_round.archive()[0].work_item_id(), "prepare");
    assert_eq!(
        second_round.archive()[0].disposition(),
        ArchiveDisposition::Retain
    );

    let terminal_round = decide_coordination_round(&items, &[done("prepare"), done("publish")]);
    assert!(terminal_round.dispatchable().is_empty());
    assert_eq!(
        terminal_round
            .archive()
            .iter()
            .map(|decision| (decision.work_item_id(), decision.disposition()))
            .collect::<Vec<_>>(),
        vec![
            ("prepare", ArchiveDisposition::Archive),
            ("publish", ArchiveDisposition::Archive),
        ]
    );
}

#[test]
fn structural_ambiguity_rejects_dispatch_and_archive_for_the_round() {
    let cases = vec![
        (
            vec![
                work("duplicate", &[], &["workspace:a"]),
                work("duplicate", &[], &["workspace:b"]),
            ],
            vec![],
            CoordinationBlocker::DuplicateWorkId("duplicate".into()),
        ),
        (
            vec![work("dependent", &["missing"], &["workspace:d"])],
            vec![],
            CoordinationBlocker::UndeclaredDependency {
                work_item_id: "dependent".into(),
                dependency_id: "missing".into(),
            },
        ),
        (
            vec![work("self", &["self"], &["workspace:self"])],
            vec![],
            CoordinationBlocker::SelfDependency("self".into()),
        ),
        (
            vec![
                work("a", &[], &["workspace:a"]),
                work("b", &[], &["workspace:b"]),
            ],
            vec![done("a"), done("a")],
            CoordinationBlocker::DuplicateCompletionReportId("a".into()),
        ),
    ];

    for (items, reports, expected) in cases {
        let round = decide_coordination_round(&items, &reports);
        assert!(round.dispatchable().is_empty());
        assert!(round.archive().is_empty());
        assert!(round.structural_blockers().contains(&expected));
    }
}

#[test]
fn resource_conflicts_block_every_conflicting_candidate_without_first_winner() {
    let items = vec![
        work("a", &[], &["workspace:shared"]),
        work("b", &[], &["workspace:shared"]),
        work("safe", &[], &["workspace:safe"]),
    ];

    let round = decide_coordination_round(&items, &[]);

    assert_eq!(
        round
            .dispatchable()
            .iter()
            .map(DispatchCandidate::work_item_id)
            .collect::<Vec<_>>(),
        vec!["safe"]
    );
    for id in ["a", "b"] {
        assert!(round.blocked().iter().any(|entry| {
            entry.work_item_id() == id
                && entry
                    .blockers()
                    .contains(&CoordinationBlocker::ResourceConflict {
                        work_item_id: id.into(),
                        resource_id: "workspace:shared".into(),
                    })
        }));
    }
}
