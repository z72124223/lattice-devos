use lattice_scope_check::{
    ChangeSide, EvidenceLevel, PathPattern, PatternKind, ScopeOutcome, ScopeRules, ViolationReason,
    evaluate,
};
use lattice_workspace_git::{
    ChangeManifest, ChangeOperation, ChangeRecord, ChangeState, PathEvidence, PathKind,
};

fn commit(character: char) -> String {
    character.to_string().repeat(40)
}

fn create(path: &str, kind: PathKind) -> ChangeRecord {
    ChangeRecord::new(
        ChangeOperation::Create,
        None,
        Some(PathEvidence::new(path, kind).expect("path evidence")),
        vec![ChangeState::Untracked],
        "??",
    )
    .expect("create change")
}

fn rename(source: &str, destination: &str) -> ChangeRecord {
    ChangeRecord::new(
        ChangeOperation::Rename,
        Some(PathEvidence::new(source, PathKind::Regular).expect("rename source")),
        Some(PathEvidence::new(destination, PathKind::Regular).expect("rename destination")),
        vec![ChangeState::Staged],
        "R100",
    )
    .expect("rename change")
}

fn manifest(changes: Vec<ChangeRecord>) -> ChangeManifest {
    ChangeManifest::from_normalized(&commit('a'), &commit('b'), changes)
        .expect("normalized manifest")
}

fn pattern(kind: PatternKind, value: &str) -> PathPattern {
    PathPattern::new(kind, value).expect("scope pattern")
}

#[test]
fn pass_is_explicitly_detection_only() {
    let rules = ScopeRules::new(
        vec![pattern(PatternKind::Prefix, "src")],
        Vec::new(),
        vec![ChangeOperation::Modify],
    )
    .expect("rules");
    let change = ChangeRecord::new(
        ChangeOperation::Modify,
        Some(PathEvidence::new("src/lib.rs", PathKind::Regular).expect("source")),
        Some(PathEvidence::new("src/lib.rs", PathKind::Regular).expect("destination")),
        vec![ChangeState::Unstaged],
        "M",
    )
    .expect("modify change");

    let report = evaluate(&rules, &manifest(vec![change])).expect("scope report");

    assert_eq!(report.outcome(), ScopeOutcome::Pass);
    assert_eq!(report.evidence_level(), EvidenceLevel::DetectionOnly);
    assert!(report.violations().is_empty());
}

#[test]
fn rename_checks_both_sides_and_forbidden_overrides_broad_allow() {
    let rules = ScopeRules::new(
        vec![pattern(PatternKind::Glob, "src/**")],
        vec![pattern(PatternKind::Glob, "src/private/**")],
        vec![ChangeOperation::Rename],
    )
    .expect("rules");

    let report = evaluate(
        &rules,
        &manifest(vec![rename("src/public.rs", "src/private/secret.rs")]),
    )
    .expect("scope report");

    assert_eq!(report.outcome(), ScopeOutcome::Violation);
    assert_eq!(report.violations().len(), 1);
    let violation = &report.violations()[0];
    assert_eq!(violation.side(), ChangeSide::Destination);
    assert_eq!(violation.path(), "src/private/secret.rs");
    assert_eq!(violation.reason(), ViolationReason::ForbiddenPath);
}

#[test]
fn forbidden_reason_precedes_disallowed_operation_and_out_of_scope() {
    let rules = ScopeRules::new(
        vec![pattern(PatternKind::Glob, "src/**")],
        vec![pattern(PatternKind::Exact, "src/blocked.rs")],
        vec![ChangeOperation::Modify],
    )
    .expect("rules");
    let report = evaluate(
        &rules,
        &manifest(vec![
            create("src/blocked.rs", PathKind::Symlink),
            create("elsewhere/file.rs", PathKind::Regular),
        ]),
    )
    .expect("scope report");

    assert_eq!(report.violations().len(), 2);
    assert!(report.violations().iter().any(|violation| {
        violation.path() == "src/blocked.rs" && violation.reason() == ViolationReason::ForbiddenPath
    }));
    assert!(report.violations().iter().any(|violation| {
        violation.path() == "elsewhere/file.rs"
            && violation.reason() == ViolationReason::PathNotAllowed
    }));
}

#[test]
fn disallowed_and_unknown_operations_fail_closed_inside_allowed_paths() {
    let rules = ScopeRules::new(
        vec![pattern(PatternKind::Glob, "src/**")],
        Vec::new(),
        vec![ChangeOperation::Modify],
    )
    .expect("rules");
    let unknown = ChangeRecord::new(
        ChangeOperation::Unknown,
        None,
        Some(PathEvidence::new("src/unknown.rs", PathKind::Regular).expect("unknown path")),
        vec![ChangeState::Unstaged],
        "X",
    )
    .expect("unknown evidence remains classifiable");

    let report = evaluate(
        &rules,
        &manifest(vec![create("src/new.rs", PathKind::Regular), unknown]),
    )
    .expect("scope report");

    assert_eq!(report.violations().len(), 2);
    assert!(
        report
            .violations()
            .iter()
            .all(|violation| violation.reason() == ViolationReason::OperationNotAllowed)
    );
}

#[test]
fn link_reparse_directory_and_unknown_kinds_fail_closed() {
    let rules = ScopeRules::new(
        vec![pattern(PatternKind::Glob, "**")],
        Vec::new(),
        vec![ChangeOperation::Create],
    )
    .expect("rules");
    let changes = [
        ("symlink", PathKind::Symlink),
        ("gitlink", PathKind::GitLink),
        ("junction/child", PathKind::ReparsePoint),
        ("directory", PathKind::Directory),
        ("unknown", PathKind::Unknown),
    ]
    .into_iter()
    .map(|(path, kind)| create(path, kind))
    .collect();

    let report = evaluate(&rules, &manifest(changes)).expect("scope report");

    assert_eq!(report.outcome(), ScopeOutcome::Violation);
    assert_eq!(report.violations().len(), 5);
    assert!(
        report
            .violations()
            .iter()
            .all(|violation| violation.reason() == ViolationReason::UnsafePathKind)
    );
}
