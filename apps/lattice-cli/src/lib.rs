//! Read-only runtime-contract CLI helpers for LATTICE.

use std::fmt::Write;

use lattice_core::{bootstrap_manifest, platform_name};

const USAGE: &str = "usage: lattice status";

/// Renders the runtime contract without performing external I/O.
#[must_use]
pub fn render_status() -> String {
    let mut output = format!("{}\nstate: contract-ready\n", platform_name());

    for component in bootstrap_manifest() {
        writeln!(
            output,
            "{}: {}; failure={}; recovery={}",
            component.id.as_str(),
            component.mode.as_str(),
            component.failure_policy.as_str(),
            component.recovery_action.as_str(),
        )
        .expect("writing to a String cannot fail");
    }

    output
}

/// Renders the runtime contract as stable JSON without external I/O.
#[must_use]
pub fn render_status_json() -> String {
    let mut output = format!(
        "{{\"platform\":\"{}\",\"state\":\"contract-ready\",\"components\":[",
        platform_name()
    );

    for (index, component) in bootstrap_manifest().iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        write!(
            output,
            "{{\"id\":\"{}\",\"mode\":\"{}\",\"failure_policy\":\"{}\",\"recovery_action\":\"{}\"}}",
            component.id.as_str(),
            component.mode.as_str(),
            component.failure_policy.as_str(),
            component.recovery_action.as_str(),
        )
        .expect("writing to a String cannot fail");
    }

    output.push_str("]}\n");
    output
}

/// Dispatches the deliberately narrow bootstrap command surface.
///
/// # Errors
///
/// Returns the stable usage message when arguments are absent or unsupported.
pub fn dispatch(arguments: &[String]) -> Result<String, &'static str> {
    match arguments {
        [command] if command == "status" => Ok(render_status()),
        [command, format] if command == "status" && format == "--json" => Ok(render_status_json()),
        _ => Err(USAGE),
    }
}
