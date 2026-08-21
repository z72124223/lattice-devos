//! Read-only recovery CLI helpers for `LATTICE DevOS`.

use std::{
    collections::BTreeMap,
    fmt::Write,
    io::{self, BufRead, Write as IoWrite},
    path::{Path, PathBuf},
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

/// Reads the local engineering map and returns a compact, read-only summary.
///
/// The map is an observation index. It is useful for finding work that needs
/// attention, but it does not replace live Git or test evidence.
///
/// # Errors
///
/// Returns an error when the local engineering map is unavailable or malformed.
pub fn render_engineering_status() -> Result<Value, String> {
    engineering_status_summary(&engineering_status_path())
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
            let result =
                json!({"tools": [status_tool_definition(), engineering_status_tool_definition()]});
            mcp_result(&id, &result)
        }
        "tools/call" => match lattice_tool_call(request.get("params")) {
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

fn engineering_status_tool_definition() -> Value {
    json!({
        "name": "lattice_engineering_status",
        "description": "Read the local LATTICE engineering map: status counts and work needing attention. It changes nothing.",
        "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}
    })
}

fn lattice_tool_call(params: Option<&Value>) -> Result<Value, &'static str> {
    let params = params
        .and_then(Value::as_object)
        .ok_or("invalid tool parameters")?;
    if !matches!(params.get("arguments"), Some(Value::Object(arguments)) if arguments.is_empty()) {
        return Err("this LATTICE tool accepts no arguments");
    }

    match params.get("name").and_then(Value::as_str) {
        Some("lattice_status") => {
            let status = serde_json::from_str::<Value>(&render_status_json())
                .expect("the built-in manifest is valid JSON");
            Ok(tool_success(&status))
        }
        Some("lattice_engineering_status") => match render_engineering_status() {
            Ok(status) => Ok(tool_success(&status)),
            Err(message) => Ok(tool_error(&message)),
        },
        _ => Err("unknown tool"),
    }
}

fn engineering_status_path() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map_or_else(std::env::temp_dir, PathBuf::from)
        .join("LATTICE")
        .join("engineering-status")
        .join("status.json")
}

fn engineering_status_summary(path: &Path) -> Result<Value, String> {
    let source = std::fs::read_to_string(path)
        .map_err(|_| "local engineering map is unavailable".to_owned())?;
    let snapshot: Value = serde_json::from_str(&source)
        .map_err(|_| "local engineering map is malformed".to_owned())?;
    let items = snapshot
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| "local engineering map has no item list".to_owned())?;

    let mut outcomes = BTreeMap::<String, usize>::new();
    for outcome in items
        .iter()
        .filter_map(|item| item.get("outcome").and_then(Value::as_str))
    {
        *outcomes.entry(outcome.to_owned()).or_default() += 1;
    }

    let mut attention_items = items
        .iter()
        .filter_map(|item| {
            let outcome = item.get("outcome").and_then(Value::as_str)?;
            (outcome != "COMPLETE").then(|| {
                json!({
                    "id": item.get("id"),
                    "title": item.get("displayNameZh").or_else(|| item.get("title")),
                    "outcome": outcome,
                    "nextStep": item.get("nextStep"),
                    "branch": item.get("branch"),
                    "evidenceState": item.get("evidenceState")
                })
            })
        })
        .collect::<Vec<_>>();
    attention_items.sort_by_key(|item| match item.get("outcome").and_then(Value::as_str) {
        Some("IN_PROGRESS") => 0,
        Some("PARTIAL") => 1,
        _ => 2,
    });
    let attention_item_count = attention_items.len();
    attention_items.truncate(12);

    Ok(json!({
        "schema": "lattice.mcp.engineering-status/1.0",
        "source": {
            "generatedAt": snapshot.get("generatedAt"),
            "freshness": snapshot.get("freshness"),
            "completeness": snapshot.get("completeness"),
            "currentItemId": snapshot.get("currentItemId")
        },
        "totalItems": items.len(),
        "outcomes": outcomes,
        "attentionItemCount": attention_item_count,
        "attentionItems": attention_items,
        "notice": "工程地圖是狀態索引；執行前仍需核對即時 Git、測試與工作目錄。"
    }))
}

fn tool_success(content: &Value) -> Value {
    json!({
        "content": [{"type": "text", "text": content.to_string()}],
        "structuredContent": content,
        "isError": false
    })
}

fn tool_error(message: &str) -> Value {
    json!({
        "content": [{"type": "text", "text": message}],
        "isError": true
    })
}

fn mcp_result(id: &Value, result: &Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn mcp_error(id: &Value, code: i64, message: &'static str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

#[cfg(test)]
mod tests {
    use super::engineering_status_summary;
    use std::io::Write;

    #[test]
    fn engineering_summary_marks_non_complete_items_for_attention() {
        let path = std::env::temp_dir().join(format!("lattice-status-{}.json", std::process::id()));
        let mut file = std::fs::File::create(&path).expect("temporary map");
        writeln!(
            file,
            r#"{{"generatedAt":"now","items":[{{"id":"TASK-1","outcome":"COMPLETE"}},{{"id":"TASK-2","outcome":"BLOCKED","nextStep":"fix it"}}]}}"#
        )
        .expect("fixture");

        let summary = engineering_status_summary(&path).expect("summary");
        assert_eq!(summary["totalItems"], 2);
        assert_eq!(summary["outcomes"]["COMPLETE"], 1);
        assert_eq!(summary["outcomes"]["BLOCKED"], 1);
        assert_eq!(summary["attentionItems"][0]["id"], "TASK-2");

        std::fs::remove_file(path).expect("remove owned fixture");
    }
}
