use lattice_cli::{dispatch, render_status, render_status_json};

#[test]
fn status_renders_the_runtime_contract() {
    let output = render_status();

    assert!(output.starts_with("LATTICE Runtime\nstate: contract-ready\n"));
    assert!(output.contains(
        "lattice: control-core; failure=runtime-unavailable; recovery=repair-control-core\n"
    ));
    assert!(output.contains(
        "postgresql: durable-truth; failure=runtime-unavailable; recovery=restore-durable-facts\n"
    ));
    assert!(output.contains("graphify: derived-relationship-memory; failure=degraded; recovery=rebuild-from-postgresql\n"));
    assert!(output.contains(
        "hermes: reflective-advisor; failure=degraded; recovery=recompute-from-facts-and-graph\n"
    ));
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

#[test]
fn status_json_is_a_stable_read_only_manifest_projection() {
    let expected = concat!(
        "{\"platform\":\"LATTICE Runtime\",\"state\":\"contract-ready\",\"components\":[",
        "{\"id\":\"lattice\",\"mode\":\"control-core\",\"failure_policy\":\"runtime-unavailable\",\"recovery_action\":\"repair-control-core\"},",
        "{\"id\":\"postgresql\",\"mode\":\"durable-truth\",\"failure_policy\":\"runtime-unavailable\",\"recovery_action\":\"restore-durable-facts\"},",
        "{\"id\":\"graphify\",\"mode\":\"derived-relationship-memory\",\"failure_policy\":\"degraded\",\"recovery_action\":\"rebuild-from-postgresql\"},",
        "{\"id\":\"hermes\",\"mode\":\"reflective-advisor\",\"failure_policy\":\"degraded\",\"recovery_action\":\"recompute-from-facts-and-graph\"}]}",
        "\n"
    );

    assert_eq!(render_status_json(), expected);
    assert_eq!(
        dispatch(&["status".to_owned(), "--json".to_owned()]),
        Ok(expected.to_owned())
    );
}
