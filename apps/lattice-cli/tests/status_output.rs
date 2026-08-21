use lattice_cli::{dispatch, render_status, render_status_json, render_version};

#[test]
fn status_renders_the_inert_platform_manifest() {
    let output = render_status();

    assert!(output.starts_with("LATTICE DevOS\nstate: bootstrap-ready\n"));
    assert!(output.contains("openclaw: gateway [retired]\n"));
    assert!(output.contains("postgresql: durable-truth [core]\n"));
    assert!(output.contains("codex: sole-writer [core]\n"));
    assert!(output.contains("graphify: read-only-evidence [optional]\n"));
    assert!(output.contains("hermes: read-only-evidence [deferred]\n"));
    assert!(output.contains("codebase-memory: durable-memory [optional]\n"));
    assert!(output.contains("guardian: approval-gated [deferred]\n"));
}

#[test]
fn only_the_read_only_inspection_commands_are_accepted() {
    assert_eq!(dispatch(&["status".to_owned()]), Ok(render_status()));
    assert_eq!(dispatch(&["version".to_owned()]), Ok(render_version()));
    assert_eq!(dispatch(&[]), Err("usage: lattice <status|version>"));
    assert_eq!(
        dispatch(&["run".to_owned()]),
        Err("usage: lattice <status|version>")
    );
    assert_eq!(
        dispatch(&["status".to_owned(), "extra".to_owned()]),
        Err("usage: lattice <status|version>")
    );
}

#[test]
fn version_reports_the_alpha_release_identity() {
    assert_eq!(
        dispatch(&["version".to_owned()]),
        Ok("LATTICE DevOS 0.1.0-alpha.1\n".to_owned())
    );
}

#[test]
fn status_json_is_a_stable_read_only_manifest_projection() {
    let expected = concat!(
        "{\"platform\":\"LATTICE DevOS\",\"state\":\"bootstrap-ready\",\"components\":[",
        "{\"id\":\"rust-core\",\"mode\":\"control-core\",\"tier\":\"core\"},",
        "{\"id\":\"openclaw\",\"mode\":\"gateway\",\"tier\":\"retired\"},",
        "{\"id\":\"postgresql\",\"mode\":\"durable-truth\",\"tier\":\"core\"},",
        "{\"id\":\"codex\",\"mode\":\"sole-writer\",\"tier\":\"core\"},",
        "{\"id\":\"graphify\",\"mode\":\"read-only-evidence\",\"tier\":\"optional\"},",
        "{\"id\":\"hermes\",\"mode\":\"read-only-evidence\",\"tier\":\"deferred\"},",
        "{\"id\":\"codebase-memory\",\"mode\":\"durable-memory\",\"tier\":\"optional\"},",
        "{\"id\":\"guardian\",\"mode\":\"approval-gated\",\"tier\":\"deferred\"}]}",
        "\n"
    );

    assert_eq!(render_status_json(), expected);
    assert_eq!(
        dispatch(&["status".to_owned(), "--json".to_owned()]),
        Ok(expected.to_owned())
    );
}
