use lattice_scope_check::{PathPattern, PatternKind, ScopeRules, evaluate};
use lattice_workspace_git::{
    ChangeManifest, ChangeOperation, ChangeRecord, ChangeState, PathEvidence, PathKind,
};

fn commit(character: char) -> String {
    character.to_string().repeat(40)
}

fn create(path: &str) -> ChangeRecord {
    ChangeRecord::new(
        ChangeOperation::Create,
        None,
        Some(PathEvidence::new(path, PathKind::Regular).expect("path evidence")),
        vec![ChangeState::Untracked],
        "??",
    )
    .expect("create evidence")
}

fn exact(value: &str) -> PathPattern {
    PathPattern::new(PatternKind::Exact, value).expect("exact pattern")
}

fn glob(value: &str) -> PathPattern {
    PathPattern::new(PatternKind::Glob, value).expect("glob pattern")
}

#[test]
fn rules_evidence_violations_and_hashes_are_permutation_stable_and_frozen() {
    let first_rules = ScopeRules::new(
        vec![glob("src/**"), exact("README.md")],
        vec![glob("src/private/**"), glob("src/private/**")],
        vec![ChangeOperation::Modify, ChangeOperation::Create],
    )
    .expect("first rules");
    let second_rules = ScopeRules::new(
        vec![exact("README.md"), glob("src/**")],
        vec![glob("src/private/**")],
        vec![ChangeOperation::Create, ChangeOperation::Modify],
    )
    .expect("second rules");
    assert_eq!(first_rules, second_rules);

    let first_manifest = ChangeManifest::from_normalized(
        &commit('a'),
        &commit('b'),
        vec![create("outside.txt"), create("src/private/key.rs")],
    )
    .expect("first manifest");
    let second_manifest = ChangeManifest::from_normalized(
        &commit('a'),
        &commit('b'),
        vec![create("src/private/key.rs"), create("outside.txt")],
    )
    .expect("second manifest");
    assert_eq!(first_manifest, second_manifest);

    let first = evaluate(&first_rules, &first_manifest).expect("first report");
    let second = evaluate(&second_rules, &second_manifest).expect("second report");

    assert_eq!(first, second);
    assert!(first.violations().windows(2).all(|pair| pair[0] <= pair[1]));
    assert_eq!(
        (
            first.rules_digest(),
            first.evidence_digest(),
            first.report_digest(),
        ),
        (
            "601eb21757b05d12e975148d693c63b7618c4684de15b96fc84db872bf670d93",
            "01e44bd8c517dcbe96dd60390c2b9cd86bbf4fdc1327b5df9e60ba3ff47d8531",
            "3b4f55aaccfcccce7d5317e2e1f84ae52fc32e6ed60d4e9cf08b06cf618e5996",
        )
    );
}
