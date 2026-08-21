//! Read-only recovery CLI helpers for `LATTICE DevOS`.

use std::{
    fmt::Write,
    io::{self, BufRead, Write as IoWrite},
};

use lattice_core::{bootstrap_manifest, platform_name};
use serde_json::{Value, json};

const USAGE: &str = "usage: lattice <status|version|mcp>";

/// Renders the exact packaged command-line version.
#[must_use]
pub fn render_version() -> String {
    format!("{} {}\n", platform_name(), env!("CARGO_PKG_VERSION"))
}

/// Renders the inert bootstrap manifest without performing external I/O.
#[must_use]
pub fn render_status() -> String {
    let mut output = format!("{}\nstate: bootstrap-ready\n", platform_name());

    for component in bootstrap_manifest() {
        writeln!(
            output,
            "{}: {} [{}]",
            component.id.as_str(),
            component.mode.as_str(),
            component.tier.as_str()
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
            "{{\"id\":\"{}\",\"mode\":\"{}\",\"tier\":\"{}\"}}",
            component.id.as_str(),
            component.mode.as_str(),
            component.tier.as_str()
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
        [command] if command == "version" => Ok(render_version()),
        _ => Err(USAGE),
    }
}

/// Serves the small, read-only LATTICE MCP surface over standard I/O.
///
/// The server intentionally exposes only the bootstrap status. Codex already
/// owns task execution, approvals, and the agent loop.
///
/// # Errors
///
/// Returns an I/O error when its standard input or output cannot be read or
/// written.
pub fn serve_mcp_stdio() -> io::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let Ok(request) = serde_json::from_str::<Value>(&line) else {
            write_mcp_message(&mut stdout, &mcp_error(&Value::Null, -32700, "parse error"))?;
            continue;
        };

        if let Some(response) = mcp_response(&request) {
            write_mcp_message(&mut stdout, &response)?;
        }
    }

    Ok(())
}

fn write_mcp_message(output: &mut impl IoWrite, message: &Value) -> io::Result<()> {
    writeln!(output, "{message}")?;
    output.flush()
}

fn mcp_response(request: &Value) -> Option<Value> {
    let id = request.get("id").cloned();
    let Some(method) = request.get("method").and_then(Value::as_str) else {
        return id.map(|id| mcp_error(&id, -32600, "invalid request"));
    };
    let id = id?;
    let response = match method {
        "initialize" => {
            let result = json!({
                "protocolVersion": "2025-11-25",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "lattice", "version": env!("CARGO_PKG_VERSION")}
            });
            mcp_result(&id, &result)
        }
        "tools/list" => {
            let result = json!({"tools": [status_tool_definition()]});
            mcp_result(&id, &result)
        }
        "tools/call" => match status_tool_call(request.get("params")) {
            Ok(result) => mcp_result(&id, &result),
            Err(message) => mcp_error(&id, -32602, message),
        },
        _ => mcp_error(&id, -32601, "method not found"),
    };

    Some(response)
}

fn status_tool_definition() -> Value {
    json!({
        "name": "lattice_status",
        "description": "Read the local LATTICE module status. It starts no services and changes nothing.",
        "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}
    })
}

fn status_tool_call(params: Option<&Value>) -> Result<Value, &'static str> {
    let params = params
        .and_then(Value::as_object)
        .ok_or("invalid tool parameters")?;
    if params.get("name").and_then(Value::as_str) != Some("lattice_status") {
        return Err("unknown tool");
    }
    if !matches!(params.get("arguments"), Some(Value::Object(arguments)) if arguments.is_empty()) {
        return Err("lattice_status accepts no arguments");
    }

    Ok(json!({
        "content": [{"type": "text", "text": render_status_json()}],
        "structuredContent": serde_json::from_str::<Value>(&render_status_json())
            .expect("the built-in manifest is valid JSON"),
        "isError": false
    }))
}

fn mcp_result(id: &Value, result: &Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn mcp_error(id: &Value, code: i64, message: &'static str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}
