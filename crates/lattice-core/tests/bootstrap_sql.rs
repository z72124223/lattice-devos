const BOOTSTRAP_SQL: &str = include_str!("../../../db/migrations/0001_bootstrap.sql");

#[test]
fn sql_draft_declares_only_the_three_approved_namespaces() {
    assert_eq!(BOOTSTRAP_SQL.matches("CREATE SCHEMA").count(), 3);
    assert!(BOOTSTRAP_SQL.contains("CREATE SCHEMA IF NOT EXISTS control;"));
    assert!(BOOTSTRAP_SQL.contains("CREATE SCHEMA IF NOT EXISTS memory;"));
    assert!(BOOTSTRAP_SQL.contains("CREATE SCHEMA IF NOT EXISTS readmodel;"));

    for forbidden in [
        "CREATE TABLE",
        "CREATE ROLE",
        "CREATE DATABASE",
        "CREATE EXTENSION",
        "GRANT ",
    ] {
        assert!(
            !BOOTSTRAP_SQL.contains(forbidden),
            "bootstrap draft must not contain {forbidden}"
        );
    }
}
