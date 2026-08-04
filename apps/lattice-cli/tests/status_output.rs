use lattice_cli::{dispatch, render_status};

#[test]
fn status_renders_the_inert_platform_manifest() {
    let output = render_status();

    assert!(output.starts_with("LATTICE DevOS\nstate: bootstrap-ready\n"));
    assert!(output.contains("openclaw: gateway\n"));
    assert!(output.contains("postgresql: durable-truth\n"));
    assert!(output.contains("codex: sole-writer\n"));
    assert!(output.contains("graphify: read-only-evidence\n"));
    assert!(output.contains("hermes: read-only-evidence\n"));
    assert!(output.contains("codebase-memory: durable-memory\n"));
    assert!(output.contains("guardian: approval-gated\n"));
}

#[test]
fn only_the_read_only_status_command_is_accepted() {
    assert_eq!(dispatch(&["status".to_owned()]), Ok(render_status()));
    assert_eq!(dispatch(&[]), Err("usage: lattice status"));
    assert_eq!(dispatch(&["run".to_owned()]), Err("usage: lattice status"));
    assert_eq!(
        dispatch(&["status".to_owned(), "extra".to_owned()]),
        Err("usage: lattice status")
    );
}
