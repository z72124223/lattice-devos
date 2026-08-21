use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

fn read_response(reader: &mut BufReader<impl std::io::Read>) -> Value {
    let mut line = String::new();
    reader.read_line(&mut line).expect("server response");
    serde_json::from_str(&line).expect("valid JSON-RPC response")
}

#[test]
fn mcp_stdio_discovers_and_calls_the_read_only_status_tool() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_lattice"))
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("start lattice MCP server");
    let mut input = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut output = BufReader::new(stdout);

    writeln!(input, "{}", json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}
    }))
    .expect("write initialize");
    let initialized = read_response(&mut output);
    assert_eq!(initialized["id"], 1);
    assert_eq!(initialized["result"]["serverInfo"]["name"], "lattice");

    writeln!(
        input,
        "{}",
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}})
    )
    .expect("write tools/list");
    let tools = read_response(&mut output);
    assert_eq!(tools["result"]["tools"][0]["name"], "lattice_status");
    assert_eq!(
        tools["result"]["tools"][1]["name"],
        "lattice_engineering_status"
    );

    writeln!(
        input,
        "{}",
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {"name": "lattice_status", "arguments": {}}
        })
    )
    .expect("write tools/call");
    let call = read_response(&mut output);
    assert_eq!(call["id"], 3);
    assert_eq!(call["result"]["isError"], false);
    let manifest: Value = serde_json::from_str(
        call["result"]["content"][0]["text"]
            .as_str()
            .expect("tool text"),
    )
    .expect("status JSON");
    assert_eq!(manifest["platform"], "LATTICE DevOS");
    assert_eq!(manifest["components"][1]["id"], "openclaw");

    writeln!(
        input,
        "{}",
        json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {"name": "lattice_status", "arguments": {"unexpected": true}}
        })
    )
    .expect("write rejected tools/call");
    let rejected = read_response(&mut output);
    assert_eq!(rejected["id"], 4);
    assert_eq!(rejected["error"]["code"], -32602);

    drop(input);
    assert!(child.wait().expect("server exit").success());
}
