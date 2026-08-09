use std::cell::Cell;
use std::io::Cursor;
use std::rc::Rc;

use lattice_runtime::composition::fixed_gateway_submission;
use lattice_runtime::mcp::{
    DeliveryToolArguments, DeliveryToolService, MAX_STDIO_MESSAGE_BYTES,
    MAX_TOOL_INVOCATIONS_PER_SESSION, McpServer, ToolExecutionError, serve,
};
use serde_json::{Value, json};

#[derive(Clone)]
struct FakeService {
    run_calls: Rc<Cell<u32>>,
    status_calls: Rc<Cell<u32>>,
}

impl DeliveryToolService for FakeService {
    fn run(&mut self, arguments: &DeliveryToolArguments) -> Result<Value, ToolExecutionError> {
        assert_eq!(arguments.binding(), fixed_binding());
        self.run_calls.set(self.run_calls.get() + 1);
        Ok(json!({"status": "COMPLETED", "kind": "run"}))
    }

    fn status(&mut self, arguments: &DeliveryToolArguments) -> Result<Value, ToolExecutionError> {
        assert_eq!(arguments.binding(), fixed_binding());
        self.status_calls.set(self.status_calls.get() + 1);
        Ok(json!({"status": "COMPLETED", "kind": "status"}))
    }
}

fn fixed_binding() -> &'static lattice_contracts::SubjectBinding {
    static BINDING: std::sync::OnceLock<lattice_contracts::SubjectBinding> =
        std::sync::OnceLock::new();
    BINDING.get_or_init(|| {
        fixed_gateway_submission()
            .expect("fixed full-chain submission")
            .binding()
            .clone()
    })
}

fn server() -> (McpServer<FakeService>, Rc<Cell<u32>>, Rc<Cell<u32>>) {
    let run_calls = Rc::new(Cell::new(0));
    let status_calls = Rc::new(Cell::new(0));
    let service = FakeService {
        run_calls: run_calls.clone(),
        status_calls: status_calls.clone(),
    };
    (
        McpServer::new(service, fixed_binding().clone()),
        run_calls,
        status_calls,
    )
}

fn initialize(server: &mut McpServer<FakeService>) {
    let response = server
        .handle(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "test", "version": "1"}
            }
        }))
        .expect("initialize response");
    assert_eq!(response["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(response["result"]["capabilities"], json!({"tools": {}}));
    assert!(
        server
            .handle(json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized"
            }))
            .is_none()
    );
}

fn modern_request_meta() -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientInfo": {
            "name": "task038-chatgpt",
            "version": "1"
        },
        "io.modelcontextprotocol/clientCapabilities": {}
    })
}

#[test]
fn modern_discovery_advertises_the_stateless_tool_contract() {
    let (mut server, run_calls, status_calls) = server();
    let metadata = json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {}
    });

    let response = server
        .handle(json!({
            "jsonrpc": "2.0",
            "id": "discover",
            "method": "server/discover",
            "params": {"_meta": metadata}
        }))
        .expect("discovery response");

    assert_eq!(response["result"]["resultType"], "complete");
    assert_eq!(
        response["result"]["supportedVersions"],
        json!(["2026-07-28"])
    );
    assert_eq!(response["result"]["capabilities"], json!({"tools": {}}));
    assert_eq!(response["result"]["cacheScope"], "private");
    assert_eq!(response["result"]["ttlMs"], 0);
    assert_eq!(
        response["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "latticed"
    );
    assert_eq!(run_calls.get(), 0);
    assert_eq!(status_calls.get(), 0);
}

#[test]
fn modern_tool_requests_are_stateless_and_preserve_the_server_binding() {
    let (mut server, run_calls, status_calls) = server();

    let list = server
        .handle(json!({
            "jsonrpc": "2.0",
            "id": "list",
            "method": "tools/list",
            "params": {"_meta": modern_request_meta()}
        }))
        .expect("modern tool list");
    assert_eq!(list["result"]["resultType"], "complete");
    assert_eq!(list["result"]["cacheScope"], "private");
    assert_eq!(list["result"]["ttlMs"], 0);
    assert_eq!(
        list["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "latticed"
    );
    assert_eq!(
        list["result"]["tools"]
            .as_array()
            .expect("tools")
            .iter()
            .map(|tool| tool["name"].as_str().expect("tool name"))
            .collect::<Vec<_>>(),
        ["lattice_delivery_run", "lattice_delivery_status"]
    );
    assert_eq!(
        list["result"]["tools"][0]["annotations"],
        json!({
            "readOnlyHint": false,
            "destructiveHint": true,
            "idempotentHint": false,
            "openWorldHint": false
        })
    );
    assert_eq!(
        list["result"]["tools"][1]["annotations"],
        json!({
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true,
            "openWorldHint": false
        })
    );

    for (id, name, arguments) in [
        ("run", "lattice_delivery_run", Some(json!({}))),
        ("status", "lattice_delivery_status", None),
    ] {
        let mut params = json!({"name": name, "_meta": modern_request_meta()});
        if let Some(arguments) = arguments {
            params["arguments"] = arguments;
        }
        let response = server
            .handle(json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "tools/call",
                "params": params
            }))
            .expect("modern tool response");
        assert_eq!(response["result"]["resultType"], "complete");
        assert_eq!(response["result"]["isError"], false);
        assert_eq!(
            response["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
            "latticed"
        );
        assert!(response["result"].get("ttlMs").is_none());
        assert!(response["result"].get("cacheScope").is_none());
    }

    assert_eq!(run_calls.get(), 1);
    assert_eq!(status_calls.get(), 1);
}

#[test]
fn modern_metadata_and_protocol_versions_fail_closed_before_dispatch() {
    for name in ["lattice_delivery_run", "lattice_delivery_status"] {
        let (mut server, run_calls, status_calls) = server();
        let missing_capabilities = server
            .handle(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": name,
                    "arguments": {},
                    "_meta": {
                        "io.modelcontextprotocol/protocolVersion": "2026-07-28"
                    }
                }
            }))
            .expect("invalid modern metadata response");
        assert_eq!(missing_capabilities["error"]["code"], -32602);

        let unsupported = server
            .handle(json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {
                    "name": name,
                    "arguments": {},
                    "_meta": {
                        "io.modelcontextprotocol/protocolVersion": "2027-01-01",
                        "io.modelcontextprotocol/clientCapabilities": {}
                    }
                }
            }))
            .expect("unsupported protocol response");
        assert_eq!(unsupported["error"]["code"], -32022);
        assert_eq!(
            unsupported["error"]["data"]["supported"],
            json!(["2026-07-28", "2025-11-25"])
        );
        assert_eq!(unsupported["error"]["data"]["requested"], "2027-01-01");
        assert_eq!(run_calls.get(), 0, "{name}");
        assert_eq!(status_calls.get(), 0, "{name}");
    }

    let (mut server, _, _) = server();
    let missing_metadata = server
        .handle(json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "server/discover",
            "params": {}
        }))
        .expect("missing discovery metadata response");
    assert_eq!(missing_metadata["error"]["code"], -32602);

    initialize(&mut server);
    let reserved_metadata_without_version = server
        .handle(json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "lattice_delivery_run",
                "arguments": {},
                "_meta": {
                    "io.modelcontextprotocol/logLevel": "warning"
                }
            }
        }))
        .expect("reserved metadata downgrade response");
    assert_eq!(reserved_metadata_without_version["error"]["code"], -32602);
}

#[test]
fn modern_known_metadata_fields_are_validated_before_dispatch() {
    for metadata in [
        json!({
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientCapabilities": {"roots": false}
        }),
        json!({
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientCapabilities": {},
            "io.modelcontextprotocol/clientInfo": {
                "name": "client",
                "version": "1",
                "title": 42
            }
        }),
        json!({
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientCapabilities": {},
            "io.modelcontextprotocol/logLevel": "verbose"
        }),
        json!({
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientCapabilities": {},
            "progressToken": {}
        }),
    ] {
        let (mut server, run_calls, status_calls) = server();
        let response = server
            .handle(json!({
                "jsonrpc": "2.0",
                "id": "malformed-known-metadata",
                "method": "tools/call",
                "params": {
                    "name": "lattice_delivery_run",
                    "arguments": {},
                    "_meta": metadata
                }
            }))
            .expect("malformed metadata response");
        assert_eq!(response["error"]["code"], -32602);
        assert_eq!(run_calls.get(), 0);
        assert_eq!(status_calls.get(), 0);
    }
}

#[test]
fn modern_known_metadata_fields_accept_final_schema_shapes() {
    let (mut server, run_calls, status_calls) = server();
    let response = server
        .handle(json!({
            "jsonrpc": "2.0",
            "id": "valid-known-metadata",
            "method": "tools/call",
            "params": {
                "name": "lattice_delivery_status",
                "arguments": {},
                "_meta": {
                    "progressToken": 7,
                    "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                    "io.modelcontextprotocol/clientCapabilities": {
                        "experimental": {"feature": {}},
                        "roots": {},
                        "sampling": {"context": {}, "tools": {}},
                        "elicitation": {"form": {}, "url": {}},
                        "extensions": {"com.example/extension": {}}
                    },
                    "io.modelcontextprotocol/clientInfo": {
                        "name": "client",
                        "title": "Client",
                        "version": "1",
                        "description": "Compatibility probe",
                        "websiteUrl": "https://example.com",
                        "icons": [{
                            "src": "https://example.com/icon.png",
                            "mimeType": "image/png",
                            "sizes": ["48x48"],
                            "theme": "light"
                        }]
                    },
                    "io.modelcontextprotocol/logLevel": "warning"
                }
            }
        }))
        .expect("valid known metadata response");
    assert_eq!(response["result"]["resultType"], "complete");
    assert_eq!(response["result"]["isError"], false);
    assert_eq!(run_calls.get(), 0);
    assert_eq!(status_calls.get(), 1);
}

#[test]
fn modern_metadata_validator_matches_string_and_extension_key_schema() {
    let (mut accepting_server, run_calls, status_calls) = server();
    let accepted = accepting_server
        .handle(json!({
            "jsonrpc": "2.0",
            "id": "empty-schema-strings",
            "method": "tools/call",
            "params": {
                "name": "lattice_delivery_status",
                "arguments": {},
                "_meta": {
                    "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                    "io.modelcontextprotocol/clientCapabilities": {},
                    "io.modelcontextprotocol/clientInfo": {
                        "name": "",
                        "version": "",
                        "icons": [{"src": ""}]
                    }
                }
            }
        }))
        .expect("schema-valid empty string response");
    assert_eq!(accepted["result"]["resultType"], "complete");
    assert_eq!(run_calls.get(), 0);
    assert_eq!(status_calls.get(), 1);

    for extension_key in ["feature", "1example/feature", "com..example/feature"] {
        let (mut rejecting_server, run_calls, status_calls) = server();
        let rejected = rejecting_server
            .handle(json!({
                "jsonrpc": "2.0",
                "id": extension_key,
                "method": "tools/call",
                "params": {
                    "name": "lattice_delivery_run",
                    "arguments": {},
                    "_meta": {
                        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                        "io.modelcontextprotocol/clientCapabilities": {
                            "extensions": {extension_key: {}}
                        }
                    }
                }
            }))
            .expect("invalid extension key response");
        assert_eq!(rejected["error"]["code"], -32602, "{extension_key}");
        assert_eq!(run_calls.get(), 0, "{extension_key}");
        assert_eq!(status_calls.get(), 0, "{extension_key}");
    }
}

#[test]
fn modern_stateless_calls_do_not_exhaust_the_legacy_session_counter() {
    let (mut server, run_calls, status_calls) = server();
    for id in 0..=MAX_TOOL_INVOCATIONS_PER_SESSION {
        let response = server
            .handle(json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "tools/call",
                "params": {
                    "name": "lattice_delivery_status",
                    "arguments": {},
                    "_meta": modern_request_meta()
                }
            }))
            .expect("modern tool response");
        assert_eq!(response["result"]["resultType"], "complete", "{id}");
        assert_eq!(response["result"]["isError"], false, "{id}");
    }
    assert_eq!(run_calls.get(), 0);
    assert_eq!(
        status_calls.get() as usize,
        MAX_TOOL_INVOCATIONS_PER_SESSION + 1
    );
}

#[test]
fn modern_tool_calls_reject_caller_owned_arguments_before_dispatch() {
    for name in ["lattice_delivery_run", "lattice_delivery_status"] {
        let (mut server, run_calls, status_calls) = server();
        let response = server
            .handle(json!({
                "jsonrpc": "2.0",
                "id": name,
                "method": "tools/call",
                "params": {
                    "name": name,
                    "arguments": {"task_id": "caller-owned"},
                    "_meta": modern_request_meta()
                }
            }))
            .expect("closed argument response");
        assert_eq!(response["error"]["code"], -32602, "{name}");
        assert_eq!(run_calls.get(), 0, "{name}");
        assert_eq!(status_calls.get(), 0, "{name}");
    }
}

#[test]
fn modern_discovery_does_not_replace_the_legacy_lifecycle() {
    let (mut legacy_server, _, _) = server();
    let discover = legacy_server
        .handle(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "server/discover",
            "params": {"_meta": modern_request_meta()}
        }))
        .expect("discover response");
    assert_eq!(discover["result"]["resultType"], "complete");

    initialize(&mut legacy_server);
    let legacy_list = legacy_server
        .handle(json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}))
        .expect("legacy tool list");
    assert_eq!(
        legacy_list["result"]["tools"].as_array().map(Vec::len),
        Some(2)
    );

    for method in ["initialize", "ping"] {
        let (mut modern_server, _, _) = server();
        let response = modern_server
            .handle(json!({
                "jsonrpc": "2.0",
                "id": method,
                "method": method,
                "params": {"_meta": modern_request_meta()}
            }))
            .expect("removed modern method response");
        assert_eq!(response["error"]["code"], -32601, "{method}");
    }
}

#[test]
fn tool_list_is_exactly_two_closed_zero_argument_tools() {
    let (mut server, _, _) = server();
    initialize(&mut server);

    let response = server
        .handle(json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}))
        .expect("tool list");
    let tools = response["result"]["tools"].as_array().expect("tools");

    assert_eq!(tools.len(), 2);
    assert_eq!(
        tools
            .iter()
            .map(|tool| tool["name"].as_str().expect("name"))
            .collect::<Vec<_>>(),
        ["lattice_delivery_run", "lattice_delivery_status"]
    );
    for tool in tools {
        assert_eq!(
            tool["inputSchema"],
            json!({"type":"object","additionalProperties":false})
        );
        assert!(tool.get("annotations").is_none());
    }
}

#[test]
fn empty_or_omitted_arguments_dispatch_each_fixed_tool_with_server_binding() {
    let (mut server, run_calls, status_calls) = server();
    initialize(&mut server);

    for (id, name, arguments) in [
        (2, "lattice_delivery_run", Some(json!({}))),
        (3, "lattice_delivery_status", None),
    ] {
        let mut params = json!({"name":name});
        if let Some(arguments) = arguments {
            params["arguments"] = arguments;
        }
        let response = server
            .handle(json!({
                "jsonrpc":"2.0",
                "id":id,
                "method":"tools/call",
                "params":params
            }))
            .expect("tool response");
        assert_eq!(response["result"]["isError"], false);
        assert!(response["result"]["structuredContent"].is_object());
    }

    assert_eq!(run_calls.get(), 1);
    assert_eq!(status_calls.get(), 1);
}

#[test]
fn request_metadata_is_allowed_without_widening_tool_arguments() {
    let (mut server, run_calls, status_calls) = server();
    initialize(&mut server);

    let list = server
        .handle(json!({
            "jsonrpc":"2.0",
            "id":"list-with-meta",
            "method":"tools/list",
            "params":{"_meta":{"progressToken":"list-progress"}}
        }))
        .expect("tool list");
    assert_eq!(list["result"]["tools"].as_array().map(Vec::len), Some(2));

    let call = server
        .handle(json!({
            "jsonrpc":"2.0",
            "id":"call-with-meta",
            "method":"tools/call",
            "params":{
                "name":"lattice_delivery_run",
                "arguments":{},
                "_meta":{"progressToken":"run-progress"}
            }
        }))
        .expect("tool response");
    assert_eq!(call["result"]["isError"], false);
    assert_eq!(run_calls.get(), 1);
    assert_eq!(status_calls.get(), 0);

    let invalid_meta = server
        .handle(json!({
            "jsonrpc":"2.0",
            "id":3,
            "method":"tools/call",
            "params":{
                "name":"lattice_delivery_status",
                "arguments":{},
                "_meta":"not-an-object"
            }
        }))
        .expect("invalid metadata response");
    assert_eq!(invalid_meta["error"]["code"], -32602);
    assert_eq!(status_calls.get(), 0);
}

#[test]
fn request_ids_are_limited_to_strings_and_integers() {
    for invalid_id in [
        Value::Null,
        json!({"nested":"id"}),
        json!([1]),
        json!(true),
        json!(1.5),
    ] {
        let (mut server, _, _) = server();
        let response = server
            .handle(json!({"jsonrpc":"2.0","id":invalid_id,"method":"ping"}))
            .expect("invalid request response");
        assert_eq!(response["id"], Value::Null);
        assert_eq!(response["error"]["code"], -32600);
    }

    for valid_id in [json!("request-id"), json!(-1), json!(u64::MAX)] {
        let (mut server, _, _) = server();
        let response = server
            .handle(json!({"jsonrpc":"2.0","id":valid_id.clone(),"method":"ping"}))
            .expect("ping response");
        assert_eq!(response["id"], valid_id);
        assert!(response.get("result").is_some());
    }
}

#[test]
fn tool_invocations_are_bounded_per_session() {
    let (mut server, run_calls, status_calls) = server();
    initialize(&mut server);

    for id in 0..MAX_TOOL_INVOCATIONS_PER_SESSION {
        let response = server
            .handle(json!({
                "jsonrpc":"2.0",
                "id":id + 10,
                "method":"tools/call",
                "params":{"name":"lattice_delivery_status","arguments":{}}
            }))
            .expect("tool response within limit");
        assert_eq!(response["result"]["isError"], false);
    }

    let rejected = server
        .handle(json!({
            "jsonrpc":"2.0",
            "id":999,
            "method":"tools/call",
            "params":{"name":"lattice_delivery_run","arguments":{}}
        }))
        .expect("rate limit response");
    assert_eq!(rejected["error"]["code"], -32029);
    assert_eq!(run_calls.get(), 0);
    assert_eq!(
        status_calls.get() as usize,
        MAX_TOOL_INVOCATIONS_PER_SESSION
    );
}

#[test]
fn every_unknown_argument_field_is_rejected_before_dispatch() {
    let dangerous = [
        "project_id",
        "project_snapshot_id",
        "task_id",
        "revision",
        "task_spec_digest",
        "shell",
        "sql",
        "path",
        "credential",
        "provider",
        "task",
        "extra",
    ];

    for property in dangerous {
        for name in ["lattice_delivery_run", "lattice_delivery_status"] {
            let (mut server, run_calls, status_calls) = server();
            initialize(&mut server);
            let mut arguments = serde_json::Map::new();
            arguments.insert(property.to_owned(), json!("forbidden"));
            let response = server
                .handle(json!({
                    "jsonrpc":"2.0",
                    "id":2,
                    "method":"tools/call",
                    "params":{
                        "name":name,
                        "arguments":Value::Object(arguments)
                    }
                }))
                .expect("error response");

            assert_eq!(response["error"]["code"], -32602);
            assert_eq!(run_calls.get(), 0, "{name}:{property}");
            assert_eq!(status_calls.get(), 0, "{name}:{property}");
        }
    }
}

#[test]
fn every_non_object_argument_shape_is_rejected_before_dispatch() {
    for arguments in [Value::Null, json!(false), json!(0), json!(""), json!([])] {
        for name in ["lattice_delivery_run", "lattice_delivery_status"] {
            let (mut server, run_calls, status_calls) = server();
            initialize(&mut server);
            let response = server
                .handle(json!({
                    "jsonrpc":"2.0",
                    "id":2,
                    "method":"tools/call",
                    "params":{
                        "name":name,
                        "arguments":arguments.clone()
                    }
                }))
                .expect("error response");

            assert_eq!(response["error"]["code"], -32602);
            assert_eq!(run_calls.get(), 0, "{name}");
            assert_eq!(status_calls.get(), 0, "{name}");
        }
    }
}

#[test]
fn tools_are_unavailable_before_initialized_notification() {
    let (mut server, run_calls, _) = server();
    let response = server
        .handle(json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}))
        .expect("error response");

    assert_eq!(response["error"]["code"], -32002);
    assert_eq!(run_calls.get(), 0);
}

#[test]
fn execution_failures_are_tool_errors_without_sensitive_messages() {
    struct FailingService;
    impl DeliveryToolService for FailingService {
        fn run(&mut self, _arguments: &DeliveryToolArguments) -> Result<Value, ToolExecutionError> {
            Err(ToolExecutionError::new("LATTICE_DELIVERY_REJECTED"))
        }

        fn status(
            &mut self,
            _arguments: &DeliveryToolArguments,
        ) -> Result<Value, ToolExecutionError> {
            Err(ToolExecutionError::new("LATTICE_STATUS_REJECTED"))
        }
    }
    let mut server = McpServer::new(FailingService, fixed_binding().clone());
    let response = server
        .handle(json!({
            "jsonrpc":"2.0","id":1,"method":"initialize",
            "params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{}}
        }))
        .expect("initialize");
    assert!(response.get("result").is_some());
    assert!(
        server
            .handle(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
            .is_none()
    );

    let response = server
        .handle(json!({
            "jsonrpc":"2.0","id":2,"method":"tools/call",
            "params":{"name":"lattice_delivery_run","arguments":{}}
        }))
        .expect("tool result");

    assert_eq!(response["result"]["isError"], true);
    assert_eq!(
        response["result"]["structuredContent"],
        json!({"status":"ERROR","code":"LATTICE_DELIVERY_REJECTED"})
    );

    let mut modern_server = McpServer::new(FailingService, fixed_binding().clone());
    let modern_response = modern_server
        .handle(json!({
            "jsonrpc": "2.0",
            "id": "modern-failure",
            "method": "tools/call",
            "params": {
                "name": "lattice_delivery_status",
                "arguments": {},
                "_meta": modern_request_meta()
            }
        }))
        .expect("modern tool result");
    assert_eq!(modern_response["result"]["resultType"], "complete");
    assert_eq!(modern_response["result"]["isError"], true);
    assert_eq!(
        modern_response["result"]["structuredContent"],
        json!({"status":"ERROR","code":"LATTICE_STATUS_REJECTED"})
    );
    assert_eq!(
        modern_response["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "latticed"
    );
    assert!(modern_response["result"].get("ttlMs").is_none());
    assert!(modern_response["result"].get("cacheScope").is_none());
}

#[test]
fn stdio_transport_emits_only_jsonrpc_responses_and_parse_errors() {
    let run_calls = Rc::new(Cell::new(0));
    let status_calls = Rc::new(Cell::new(0));
    let service = FakeService {
        run_calls,
        status_calls,
    };
    let input = concat!(
        "{not-json}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-11-25\",\"capabilities\":{},\"clientInfo\":{}}}\n",
        "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"params\":{}}\n"
    );
    let mut output = Vec::new();

    serve(
        service,
        fixed_binding().clone(),
        Cursor::new(input.as_bytes()),
        &mut output,
    )
    .expect("serve");

    let responses = String::from_utf8(output)
        .expect("utf8")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("json response"))
        .collect::<Vec<_>>();
    assert_eq!(responses.len(), 3);
    assert_eq!(responses[0]["error"]["code"], -32700);
    assert_eq!(responses[1]["id"], 1);
    assert_eq!(responses[2]["id"], 2);
}

#[test]
fn stdio_transport_drains_oversized_frames_before_next_message() {
    let run_calls = Rc::new(Cell::new(0));
    let status_calls = Rc::new(Cell::new(0));
    let service = FakeService {
        run_calls,
        status_calls,
    };
    let mut input = vec![b'x'; MAX_STDIO_MESSAGE_BYTES + 1];
    input.push(b'\n');
    input.extend_from_slice(
        b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-11-25\",\"capabilities\":{},\"clientInfo\":{}}}\n",
    );
    let mut output = Vec::new();

    serve(
        service,
        fixed_binding().clone(),
        Cursor::new(input),
        &mut output,
    )
    .expect("serve");

    let responses = String::from_utf8(output)
        .expect("utf8")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("json response"))
        .collect::<Vec<_>>();
    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0]["error"]["code"], -32600);
    assert_eq!(responses[0]["error"]["message"], "Message too large");
    assert_eq!(responses[1]["id"], 1);
    assert!(responses[1].get("result").is_some());
}

#[test]
fn stdio_transport_rejects_unterminated_frames_without_dispatch() {
    let run_calls = Rc::new(Cell::new(0));
    let status_calls = Rc::new(Cell::new(0));
    let service = FakeService {
        run_calls: run_calls.clone(),
        status_calls: status_calls.clone(),
    };
    let input = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"lattice_delivery_run\",\"arguments\":{}}}";
    let mut output = Vec::new();

    serve(
        service,
        fixed_binding().clone(),
        Cursor::new(input),
        &mut output,
    )
    .expect("serve");

    let response = serde_json::from_slice::<Value>(&output).expect("json response");
    assert_eq!(response["id"], Value::Null);
    assert_eq!(response["error"]["code"], -32600);
    assert_eq!(response["error"]["message"], "Unterminated message");
    assert_eq!(run_calls.get(), 0);
    assert_eq!(status_calls.get(), 0);
}
