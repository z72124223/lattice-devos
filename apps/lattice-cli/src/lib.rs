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

/// Dispatches the deliberately narrow bootstrap command surface.
///
/// # Errors
///
/// Returns the stable usage message when arguments are absent or unsupported.
pub fn dispatch(arguments: &[String]) -> Result<String, &'static str> {
    match arguments {
        [command] if command == "status" => Ok(render_status()),
        _ => Err(USAGE),
    }
}
