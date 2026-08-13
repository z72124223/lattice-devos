//! Read-only recovery CLI helpers for `LATTICE DevOS`.

use std::fmt::Write;

use lattice_core::{bootstrap_manifest, platform_name};

const USAGE: &str = "usage: lattice status";

/// Renders the inert bootstrap manifest without performing external I/O.
#[must_use]
pub fn render_status() -> String {
    let mut output = format!("{}\nstate: bootstrap-ready\n", platform_name());

    for component in bootstrap_manifest() {
        writeln!(
            output,
            "{}: {}",
            component.id.as_str(),
            component.mode.as_str()
        )
        .expect("writing to a String cannot fail");
    }

    output
}

/// Renders the inert bootstrap manifest as stable JSON without external I/O.
#[must_use]
pub fn render_status_json() -> String {
    let mut output = format!(
        "{{\"platform\":\"{}\",\"state\":\"bootstrap-ready\",\"components\":[",
        platform_name()
    );

    for (index, component) in bootstrap_manifest().iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        write!(
            output,
            "{{\"id\":\"{}\",\"mode\":\"{}\"}}",
            component.id.as_str(),
            component.mode.as_str()
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
