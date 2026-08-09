use lattice_scope_check::{PathPattern, PatternKind, ScopeError};

#[test]
fn exact_prefix_and_language_neutral_glob_have_segment_semantics() {
    let exact = PathPattern::new(PatternKind::Exact, "README.md").expect("exact pattern");
    assert!(exact.matches_path("README.md").expect("normalized path"));
    assert!(
        !exact
            .matches_path("docs/README.md")
            .expect("normalized path")
    );

    let prefix = PathPattern::new(PatternKind::Prefix, "src").expect("prefix pattern");
    assert!(prefix.matches_path("src").expect("normalized path"));
    assert!(prefix.matches_path("src/lib.rs").expect("normalized path"));
    assert!(
        !prefix
            .matches_path("src-old/lib.rs")
            .expect("normalized path")
    );

    let glob =
        PathPattern::new(PatternKind::Glob, "crates/**/src/*.rs").expect("language-neutral glob");
    assert!(
        glob.matches_path("crates/a/src/lib.rs")
            .expect("normalized path")
    );
    assert!(
        glob.matches_path("crates/a/b/src/main.rs")
            .expect("normalized path")
    );
    assert!(
        !glob
            .matches_path("crates/a/src/nested/lib.rs")
            .expect("normalized path")
    );

    let one_character =
        PathPattern::new(PatternKind::Glob, "docs/v?/guide.*").expect("question glob");
    assert!(
        one_character
            .matches_path("docs/v2/guide.md")
            .expect("normalized path")
    );
    assert!(
        !one_character
            .matches_path("docs/v12/guide.md")
            .expect("normalized path")
    );
}

#[test]
fn malformed_patterns_and_candidate_paths_fail_closed() {
    for invalid in [
        "",
        "/absolute",
        "../escape",
        "src/../escape",
        "src\\windows",
        ".git/config",
        "src/.git/config",
        "C:/drive",
        "src//double",
        "src/trailing/",
        "src/**suffix/file",
        "e\u{301}.rs",
        ".git./config",
        ".git /config",
        "src/name./file",
        "src/name /file",
        "NUL",
        "src/CON.txt",
        "src/COM1.rs",
        "src/LPT9",
        "src/COM¹.txt",
        "src/CONIN$",
    ] {
        assert_eq!(
            PathPattern::new(PatternKind::Glob, invalid),
            Err(ScopeError::InvalidPattern),
            "invalid pattern {invalid:?}"
        );
    }

    let pattern = PathPattern::new(PatternKind::Glob, "src/**").expect("valid pattern");
    for invalid in ["", "../escape", "src\\windows", ".git/config", "C:/drive"] {
        assert_eq!(
            pattern.matches_path(invalid),
            Err(ScopeError::InvalidEvidence),
            "invalid evidence {invalid:?}"
        );
    }
}
